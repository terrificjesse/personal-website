//! The hunt alert channel's HTTP surface (Phase 8e).
//!
//! Two endpoints, consumed by the Firefox extension's background poll:
//!
//! ```text
//! GET  /hunt/events?since=&include_acked=&limit=   undelivered events, newest first
//! POST /hunt/events/{id}/ack                       the extension took delivery
//! ```
//!
//! # Auth is the session cookie that already exists
//!
//! Both take `CurrentUser`, exactly like `/internships/applications`, so a route's signature
//! says whether it is protected — the Phase 5 pattern. The extension holds `host_permissions`
//! for the backend origin and fetches with `credentials: "include"`, which puts the ordinary
//! `fridge_session` cookie on the request. There is no extension token and no second auth
//! path to keep in sync.
//!
//! # Why the poller does not send `since`
//!
//! It is tempting to have the client remember where it got to. It cannot: an MV3 background
//! page is killed and restarted at the browser's convenience, and an event that arrived while
//! Firefox was closed would then sit behind a watermark that had already moved past it.
//! `acked_at` on the server is the record. `since` exists for the popup, which is drawing a
//! recent-activity list rather than deciding what to notify about.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::hunt::variants;
use crate::hunt::events::{self, AckOutcome, EventQuery, HuntEvent};
use crate::hunt::answers::{self, Answer, NewAnswer, Suggestion};
use crate::hunt::profile::{self, CvProfile};
use crate::hunt::tokens::{self, HuntToken, MAX_LABEL_LENGTH, MintedToken};
use crate::routes::auth::CurrentUser;

/// How many events one poll returns when the caller doesn't say.
const DEFAULT_EVENT_LIMIT: u32 = 50;
/// Clamped rather than rejected, matching `RunHealthQuery` — an oversized `limit` is a client
/// bug, not a reason to fail the request.
const MAX_EVENT_LIMIT: u32 = 200;

#[derive(Debug, Clone, Deserialize)]
pub struct EventsQuery {
    /// RFC3339. Only events created strictly after this.
    pub since: Option<String>,
    /// Include events a client has already taken delivery of. The popup's recent-alerts list
    /// passes `true`; the background poll leaves it off.
    pub include_acked: Option<bool>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventsResponse {
    pub events: Vec<HuntEvent>,
    /// Every undelivered event, not just the ones that fit under `limit`. Without it a
    /// truncated page is indistinguishable from a complete one — the same reasoning as
    /// keeping `filtered` and `rejected` separate on `source_runs`.
    pub unacked_total: i64,
}

/// Events visible to this user, newest first.
pub async fn list_events(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Query(params): Query<EventsQuery>,
) -> Result<Json<EventsResponse>, StatusCode> {
    // A malformed `since` is rejected rather than ignored. Silently dropping it would widen
    // the result set, which is the safe direction for a notifier but hides a client bug.
    let since = match params.since.as_deref() {
        None => None,
        Some(raw) => Some(
            DateTime::parse_from_rfc3339(raw)
                .map_err(|_| StatusCode::BAD_REQUEST)?
                .with_timezone(&Utc),
        ),
    };

    let limit = params
        .limit
        .unwrap_or(DEFAULT_EVENT_LIMIT)
        .min(MAX_EVENT_LIMIT);

    let query = EventQuery {
        viewer: &user.id,
        since,
        include_acked: params.include_acked.unwrap_or(false),
        limit: i64::from(limit),
    };

    let events = events::list(&pool, &query)
        .await
        .map_err(internal("listing hunt events"))?;
    let unacked_total = events::unacked_total(&pool, &user.id)
        .await
        .map_err(internal("counting unacked hunt events"))?;

    Ok(Json(EventsResponse {
        events,
        unacked_total,
    }))
}

/// Record that this client has raised a notification for the event.
///
/// Idempotent: acking an already-acked event is `204`, not an error. The extension retrying an
/// ack it isn't sure landed is correct behaviour and must not look like a failure — the
/// alternative is a client that gives up and re-notifies instead.
pub async fn ack_event(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    match events::ack(&pool, &id, &user.id, Utc::now()).await {
        Ok(AckOutcome::Acked | AckOutcome::AlreadyAcked) => Ok(StatusCode::NO_CONTENT),
        Ok(AckOutcome::NotFound) => Err(StatusCode::NOT_FOUND),
        Err(err) => Err(internal("acking a hunt event")(err)),
    }
}

/// Same as [`internal`], for the handlers that talk to sqlx directly rather than through a
/// `hunt::` function. Two helpers because the two error types are genuinely different, not
/// because anyone enjoys having two.
fn sql(context: &'static str) -> impl Fn(sqlx::Error) -> StatusCode {
    move |err| {
        eprintln!("hunt: {context} failed: {err:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

/// Log the cause, tell the client nothing. Mirrors `routes::internships::internal`.
fn internal(context: &'static str) -> impl Fn(anyhow::Error) -> StatusCode {
    move |err| {
        eprintln!("hunt: {context} failed: {err:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

// ------------------------------------------------------------------------------------------
// Extension tokens
// ------------------------------------------------------------------------------------------

/// Minting requires a **cookie** session by construction.
///
/// `CurrentUser` accepts a bearer token too, so in principle a token could mint another token.
/// That is fine and deliberately not prevented: it grants nothing the caller does not already
/// have, and blocking it would mean a second notion of "how did you authenticate" leaking into
/// route signatures — the thing keeping this one auth system rather than two.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateTokenRequest {
    #[serde(default)]
    pub label: Option<String>,
}

/// Mint a token. **The secret is in this response and nowhere else, ever.**
pub async fn create_token(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<CreateTokenRequest>,
) -> Result<(StatusCode, Json<MintedToken>), StatusCode> {
    let label = body.label.unwrap_or_default();
    if label.len() > MAX_LABEL_LENGTH {
        return Err(StatusCode::BAD_REQUEST);
    }

    let minted = tokens::mint(&pool, &user.id, &label, Utc::now())
        .await
        .map_err(internal("minting a hunt token"))?;

    Ok((StatusCode::CREATED, Json(minted)))
}

/// This user's live tokens. Carries labels and usage, never secrets.
pub async fn list_tokens(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<HuntToken>>, StatusCode> {
    tokens::list(&pool, &user.id)
        .await
        .map(Json)
        .map_err(internal("listing hunt tokens"))
}

/// Revoke a token. Idempotent in effect: revoking an already-revoked or unknown token is 404,
/// and either way that token no longer authenticates anything.
pub async fn revoke_token(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let revoked = tokens::revoke(&pool, &id, &user.id, Utc::now())
        .await
        .map_err(internal("revoking a hunt token"))?;

    if revoked {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

// ------------------------------------------------------------------------------------------
// CV profile (Phase 8f)
// ------------------------------------------------------------------------------------------

/// This user's CV details, for the extension's autofill and the site's editor.
///
/// Always 200 with a profile — an empty one for a user who has never saved. "You have not
/// filled this in" is a profile with nothing in it, not a 404, and making the client handle
/// both shapes would buy nothing.
pub async fn get_profile(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<CvProfile>, StatusCode> {
    profile::get(&pool, &user.id)
        .await
        .map(Json)
        .map_err(internal("reading the CV profile"))
}

/// Replace this user's CV details.
///
/// A full replace, not a patch: the editor sends the whole form, and a partial update could
/// not distinguish "I cleared this field" from "I did not send this field".
pub async fn put_profile(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<CvProfile>,
) -> Result<Json<CvProfile>, StatusCode> {
    let saved = profile::put(&pool, &user.id, body, Utc::now())
        .await
        .map_err(internal("saving the CV profile"))?;

    // `None` means a field was over the length cap — a client bug, so 400 rather than a
    // silent truncation that would put a half-written answer on a real application.
    saved.map(Json).ok_or(StatusCode::BAD_REQUEST)
}

// ------------------------------------------------------------------------------------------
// "Track this application" (Phase 8f)
// ------------------------------------------------------------------------------------------

/// What we know about the page the user is applying on.
#[derive(Debug, Clone, Serialize)]
pub struct PostingForPage {
    /// Present when this page is a posting we already collected. Passing it to
    /// `POST /internships/applications` gets the full snapshot for free.
    pub posting_id: Option<String>,
    pub company_name: Option<String>,
    pub title: Option<String>,
    /// Set when the user has already tracked this posting, so the extension can say
    /// "already tracked" rather than offering a button that will 409.
    pub already_tracked: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PostingForPageQuery {
    pub url: String,
}

/// Match a page URL against the collected corpus.
///
/// # Why this reuses `dedup` rather than comparing strings
///
/// The URL in the address bar is never the one we stored: Lever appends `/apply`, Ashby
/// `/application`, and real links carry `?gh_jid=`, `?mobile=` and `?ats=` tracking noise —
/// all measured in `docs/INTERNSHIP_SCRAPING.md` § C, and all of it enough to make an equality
/// test miss. `dedup::ats_identity` already knows how to reduce these to `(ats, board, job id)`,
/// and using it here means one definition of "the same posting" instead of a second one that
/// drifts.
///
/// A miss is a normal answer, not an error: Phase 7 found company-owned careers pages are the
/// majority of the corpus, and none of them are in it.
pub async fn posting_for_page(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Query(params): Query<PostingForPageQuery>,
) -> Result<Json<PostingForPage>, StatusCode> {
    use crate::internships::dedup::{ats_identity, canonical_url, greenhouse_job_id};

    let wanted_identity = ats_identity(&params.url);
    let wanted_canonical = canonical_url(&params.url);
    // A company careers page can host a Greenhouse job — jumptrading.com/hr/job?gh_jid=… is
    // the same posting as job-boards.greenhouse.io/jumptrading/jobs/…, and neither the ATS
    // identity nor the canonical URL can tell. See `dedup::greenhouse_job_id` for why this is
    // a lookup concern and deliberately not part of the merge key.
    let wanted_greenhouse = greenhouse_job_id(&params.url);

    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id, canonical_url, company_name FROM internship_postings")
            .fetch_all(&pool)
            .await
            .map_err(sql("scanning postings for this page"))?;

    // A linear scan over a corpus this size (~1,400 rows) costs nothing and keeps the matching
    // rule in one place. Indexing a derived key would mean storing it, and storing it would
    // mean a migration whose only job is to speed up a query nobody has measured.
    let mut matched = None;
    for (id, url, _company) in &rows {
        let same = match (&wanted_identity, ats_identity(url)) {
            (Some(wanted), Some(found)) => *wanted == found,
            _ => canonical_url(url) == wanted_canonical,
        };
        // Then the Greenhouse job id, which sees through the company-page/ATS-page split the
        // two checks above cannot. Last, so it never overrides a stronger match.
        let same = same
            || match (&wanted_greenhouse, greenhouse_job_id(url)) {
                (Some(wanted), Some(found)) => *wanted == found,
                _ => false,
            };
        if same {
            matched = Some(id.clone());
            break;
        }
    }

    let Some(posting_id) = matched else {
        return Ok(Json(PostingForPage {
            posting_id: None,
            company_name: None,
            title: None,
            already_tracked: false,
        }));
    };

    let details: Option<(String, String)> =
        sqlx::query_as("SELECT company_name, title FROM internship_postings WHERE id = ?")
            .bind(&posting_id)
            .fetch_optional(&pool)
            .await
            .map_err(sql("reading the matched posting"))?;

    let already_tracked: Option<String> = sqlx::query_scalar(
        "SELECT id FROM internship_applications WHERE user_id = ? AND posting_id = ?",
    )
    .bind(&user.id)
    .bind(&posting_id)
    .fetch_optional(&pool)
    .await
    .map_err(sql("checking whether this is already tracked"))?;

    Ok(Json(PostingForPage {
        posting_id: Some(posting_id),
        company_name: details.as_ref().map(|d| d.0.clone()),
        title: details.map(|d| d.1),
        already_tracked: already_tracked.is_some(),
    }))
}

// ------------------------------------------------------------------------------------------
// The answer library (Phase 8g)
// ------------------------------------------------------------------------------------------

/// How many suggestions a lookup returns when the caller doesn't say. Small on purpose: this
/// is a list you read, and a long one turns choosing into skimming — which is the behaviour
/// the whole company-specific guard exists to protect you from.
const DEFAULT_SUGGESTION_LIMIT: usize = 5;
const MAX_SUGGESTION_LIMIT: usize = 20;

#[derive(Debug, Clone, Deserialize)]
pub struct AnswersQuery {
    /// The question being asked. Omit to list everything instead of ranking.
    pub q: Option<String>,
    /// The employer whose form this is. **Materially changes the result**: without it, an
    /// answer written for a specific company is never offered, because we cannot tell whether
    /// this is that company.
    pub company: Option<String>,
    pub limit: Option<usize>,
}

/// Two shapes from one route: ranked suggestions when `q` is given, the whole library when not.
///
/// Deliberately not two endpoints. The caller's question is "what do I have for this?", and
/// "everything" is the answer when there is no particular question — splitting it would make
/// the client decide which to call before it knows.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum AnswersResponse {
    Ranked { suggestions: Vec<Suggestion> },
    All { answers: Vec<Answer> },
}

pub async fn list_answers(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Query(params): Query<AnswersQuery>,
) -> Result<Json<AnswersResponse>, StatusCode> {
    let limit = params
        .limit
        .unwrap_or(DEFAULT_SUGGESTION_LIMIT)
        .min(MAX_SUGGESTION_LIMIT);

    match params.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        Some(question) => {
            let suggestions = answers::suggest(
                &pool,
                &user.id,
                question,
                params.company.as_deref(),
                limit,
            )
            .await
            .map_err(internal("suggesting answers"))?;
            Ok(Json(AnswersResponse::Ranked { suggestions }))
        }
        None => {
            let all = answers::list(&pool, &user.id)
                .await
                .map_err(internal("listing answers"))?;
            Ok(Json(AnswersResponse::All { answers: all }))
        }
    }
}

/// Save an answer. 400 if a field is empty or over its cap.
pub async fn create_answer(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<NewAnswer>,
) -> Result<(StatusCode, Json<Answer>), StatusCode> {
    answers::save(&pool, &user.id, body, Utc::now())
        .await
        .map_err(internal("saving an answer"))?
        .map(|answer| (StatusCode::CREATED, Json(answer)))
        .ok_or(StatusCode::BAD_REQUEST)
}

#[derive(Debug, Clone, Deserialize)]
pub struct EditAnswerRequest {
    pub answer_text: String,
}

/// Replace an answer's text, keeping the previous version.
pub async fn edit_answer(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<EditAnswerRequest>,
) -> Result<Json<Answer>, StatusCode> {
    // `None` covers both "no such answer" and "the text was empty or too long". They are
    // different, so ask which before answering.
    match answers::edit(&pool, &id, &user.id, &body.answer_text, Utc::now())
        .await
        .map_err(internal("editing an answer"))?
    {
        Some(answer) => Ok(Json(answer)),
        None => {
            let exists = answers::get(&pool, &id, &user.id)
                .await
                .map_err(internal("reading an answer"))?;
            Err(if exists.is_some() {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::NOT_FOUND
            })
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RevisionsResponse {
    pub revisions: Vec<AnswerRevision>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnswerRevision {
    pub replaced_at: DateTime<Utc>,
    pub answer_text: String,
}

/// Previous versions, newest first — so a rewrite you regret is recoverable.
pub async fn answer_revisions(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<RevisionsResponse>, StatusCode> {
    let rows = answers::revisions(&pool, &id, &user.id)
        .await
        .map_err(internal("reading answer revisions"))?;

    Ok(Json(RevisionsResponse {
        revisions: rows
            .into_iter()
            .map(|(replaced_at, answer_text)| AnswerRevision { replaced_at, answer_text })
            .collect(),
    }))
}

/// Record that an answer was actually used. Separate from being suggested, deliberately: a
/// suggestion you ignored is not evidence the answer is any good.
pub async fn use_answer(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    if answers::mark_used(&pool, &id, &user.id, Utc::now())
        .await
        .map_err(internal("recording answer use"))?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

pub async fn delete_answer(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    if answers::delete(&pool, &id, &user.id)
        .await
        .map_err(internal("deleting an answer"))?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

// ------------------------------------------------------------------------------------------
// Résumé variants (Phase 12f) — which résumé went with which application
// ------------------------------------------------------------------------------------------

/// Turn a refusal into the status that says what happened.
///
/// A 409 on delete is the load-bearing one: it is what stops a tidy-up from deleting the
/// attribution, and a 500 there would read as a bug rather than as a rule.
fn refused(refusal: variants::Refused) -> StatusCode {
    match refusal {
        variants::Refused::BadLabel => StatusCode::BAD_REQUEST,
        variants::Refused::DuplicateLabel | variants::Refused::InUse => StatusCode::CONFLICT,
    }
}

pub async fn list_variants(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<variants::ResumeVariant>>, StatusCode> {
    variants::list(&pool, &user.id)
        .await
        .map(Json)
        .map_err(internal("listing resume variants"))
}

pub async fn create_variant(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<variants::NewVariant>,
) -> Result<(StatusCode, Json<variants::ResumeVariant>), StatusCode> {
    variants::create(&pool, &user.id, body, Utc::now())
        .await
        .map_err(internal("creating a resume variant"))?
        .map(|variant| (StatusCode::CREATED, Json(variant)))
        .map_err(refused)
}

pub async fn edit_variant(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<variants::EditVariant>,
) -> Result<Json<variants::ResumeVariant>, StatusCode> {
    variants::edit(&pool, &user.id, &id, body, Utc::now())
        .await
        .map_err(internal("editing a resume variant"))?
        .ok_or(StatusCode::NOT_FOUND)?
        .map(Json)
        .map_err(refused)
}

pub async fn delete_variant(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    variants::delete(&pool, &user.id, &id)
        .await
        .map_err(internal("deleting a resume variant"))?
        .ok_or(StatusCode::NOT_FOUND)?
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(refused)
}

/// The seam between `popup.js` and these handlers — 8g's loop, at the HTTP layer.
///
/// `hunt/answers.rs` already tests the retrieval rules thoroughly. What nothing tested is the
/// *contract the extension actually depends on*: the query parameter names it builds, the JSON
/// keys it reads back, and the exact body it posts. Those are invisible to a unit test and to
/// the compiler, live in two different languages, and are precisely what "the loop was never
/// closed by hand" would have discovered — a renamed parameter fails silently as "no
/// suggestions", which is indistinguishable from an empty library.
#[cfg(test)]
mod answer_loop_tests {
    use super::*;
    use crate::models::User;
    use axum::http::Uri;
    use uuid::Uuid;

    async fn pool() -> SqlitePool {
        let path = std::env::temp_dir().join(format!("hunt-routes-{}.db", Uuid::new_v4()));
        crate::db::init_pool(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("migrations")
    }

    async fn user(pool: &SqlitePool) -> CurrentUser {
        let id = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO users (id, email, created_at) VALUES (?1, ?2, ?3)")
            .bind(&id)
            .bind(format!("{id}@example.com"))
            .bind(Utc::now().to_rfc3339())
            .execute(pool)
            .await
            .unwrap();
        CurrentUser(User {
            id,
            email: "who@example.com".into(),
            password_hash: None,
            created_at: Utc::now(),
            is_admin: false,
        })
    }

    /// Exactly the body `popup.js` builds on Save: three fields, and no `tags`.
    fn popup_save_body(question: &str, answer: &str, company: Option<&str>) -> NewAnswer {
        serde_json::from_value(serde_json::json!({
            "question_text": question,
            "answer_text": answer,
            "company_name": company,
        }))
        .expect("the popup's save body must deserialize; a required field here breaks Save")
    }

    /// Percent-encoding for the characters these questions actually contain.
    fn urlencoded(text: &str) -> String {
        let mut out = String::new();
        for c in text.chars() {
            match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
                other => {
                    let mut buf = [0u8; 4];
                    for byte in other.encode_utf8(&mut buf).as_bytes() {
                        out.push_str(&format!("%{byte:02X}"));
                    }
                }
            }
        }
        out
    }

    /// Exactly the URL `popup.js` builds on Suggest.
    fn popup_suggest_query(question: &str, company: Option<&str>) -> Query<AnswersQuery> {
        let mut url = format!("/hunt/answers?q={}", urlencoded(question));
        if let Some(company) = company {
            url.push_str(&format!("&company={}", urlencoded(company)));
        }
        let uri: Uri = url.parse().unwrap();
        Query::<AnswersQuery>::try_from_uri(&uri)
            .expect("the popup's query string must parse into AnswersQuery")
    }

    fn suggestions_of(response: &AnswersResponse) -> Vec<serde_json::Value> {
        let json = serde_json::to_value(response).unwrap();
        json.get("suggestions")
            .and_then(|s| s.as_array())
            .cloned()
            .unwrap_or_default()
    }

    /// The loop itself: save an answer off one company's form, meet the question on another's.
    #[tokio::test]
    async fn a_generic_answer_saved_on_one_form_is_offered_on_another_companys() {
        let pool = pool().await;
        let me = user(&pool).await;

        let (status, _) = create_answer(
            State(pool.clone()),
            me.clone(),
            axum::Json(popup_save_body(
                "Tell us about a project you are proud of.",
                "I built a fridge app that estimates expiration dates from a FoodKeeper snapshot.",
                Some("Acme Corp"),
            )),
        )
        .await
        .expect("save should succeed");
        assert_eq!(status, StatusCode::CREATED);

        // The same question, on a different employer's form.
        let response = list_answers(
            State(pool.clone()),
            me,
            popup_suggest_query("Tell us about a project you are proud of.", Some("Globex")),
        )
        .await
        .expect("suggest should succeed");

        let suggestions = suggestions_of(&response.0);
        assert_eq!(suggestions.len(), 1, "a generic answer must cross companies");

        // The keys `popup.js` reads off each suggestion. Renaming any of them turns the
        // feature into a silent no-op.
        let first = &suggestions[0];
        assert!(first.get("id").and_then(|v| v.as_str()).is_some(), "popup reads suggestion.id");
        assert!(
            first.get("answer_text").and_then(|v| v.as_str()).is_some(),
            "popup reads suggestion.answer_text"
        );
        assert!(first.get("similarity").is_some(), "the score is published");
    }

    /// The other half, and the one that costs an application if it is wrong.
    #[tokio::test]
    async fn an_employer_question_answered_for_one_company_is_withheld_from_another() {
        let pool = pool().await;
        let me = user(&pool).await;

        let _saved = create_answer(
            State(pool.clone()),
            me.clone(),
            axum::Json(popup_save_body(
                "Why do you want to work at Stripe?",
                "Stripe's payments infrastructure is the part of the internet I find most interesting.",
                Some("Stripe"),
            )),
        )
        .await
        .expect("save should succeed");

        let elsewhere = list_answers(
            State(pool.clone()),
            me.clone(),
            popup_suggest_query("Why do you want to work at Datadog?", Some("Datadog")),
        )
        .await
        .unwrap();
        assert!(
            suggestions_of(&elsewhere.0).is_empty(),
            "one employer's answer must never reach another's form"
        );

        let same = list_answers(
            State(pool.clone()),
            me,
            popup_suggest_query("Why do you want to work at Stripe?", Some("Stripe")),
        )
        .await
        .unwrap();
        assert_eq!(
            suggestions_of(&same.0).len(),
            1,
            "the same employer must still get its own answer back"
        );
    }

    /// With no `q`, the popup's "whole library" call must come back under `answers`, not
    /// `suggestions` — the untagged enum makes this a silent shape change if it ever flips.
    #[tokio::test]
    async fn the_library_listing_and_the_ranked_listing_are_different_json_keys() {
        let pool = pool().await;
        let me = user(&pool).await;

        let _saved = create_answer(
            State(pool.clone()),
            me.clone(),
            axum::Json(popup_save_body("Anything else?", "Not really.", None)),
        )
        .await
        .unwrap();

        let uri: Uri = "/hunt/answers".parse().unwrap();
        let all = list_answers(
            State(pool.clone()),
            me,
            Query::<AnswersQuery>::try_from_uri(&uri).unwrap(),
        )
        .await
        .unwrap();

        let json = serde_json::to_value(&all.0).unwrap();
        assert!(json.get("answers").is_some(), "no q -> the whole library, under `answers`");
        assert!(json.get("suggestions").is_none());
    }
}
