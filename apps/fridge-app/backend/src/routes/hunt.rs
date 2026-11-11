//! The hunt alert channel's HTTP surface (Phase 8e).
//!
//! Two endpoints, consumed by the Firefox extension's background poll:
//!
//! ```text
//! GET  /hunt/events?since=&include_acked=&limit=&cursor=   events, newest first
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
use crate::models::{decode_cursor, encode_cursor};
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
    /// A previous page's `next_cursor`: only events older than the last one it returned. The
    /// popup scrolls back with it; the background poll never sends one.
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventsResponse {
    pub events: Vec<HuntEvent>,
    /// Every undelivered event, not just the ones that fit under `limit`. Without it a
    /// truncated page is indistinguishable from a complete one — the same reasoning as
    /// keeping `filtered` and `rejected` separate on `source_runs`.
    pub unacked_total: i64,
    /// Set when there are older events than this page holds. Pass it back as `cursor`.
    pub next_cursor: Option<String>,
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

    // A cursor that does not decode is a caller error, not a first page: answering it with the
    // newest events would send a scrolling popup back to the top and repeat everything.
    let older_than = match params.cursor.as_deref() {
        None => None,
        Some(raw) => Some(decode_cursor(raw).ok_or(StatusCode::BAD_REQUEST)?),
    };

    let limit = params
        .limit
        .unwrap_or(DEFAULT_EVENT_LIMIT)
        .min(MAX_EVENT_LIMIT);

    // One more than asked for: a full page cannot say on its own whether anything is older.
    let query = EventQuery {
        viewer: &user.id,
        since,
        include_acked: params.include_acked.unwrap_or(false),
        older_than,
        limit: i64::from(limit) + 1,
    };

    let mut events = events::list(&pool, &query)
        .await
        .map_err(internal("listing hunt events"))?;
    let has_more = events.len() > limit as usize;
    events.truncate(limit as usize);
    let next_cursor = has_more
        .then(|| events.last())
        .flatten()
        .map(|event| encode_cursor(event.created_at, &event.id));
    let unacked_total = events::unacked_total(&pool, &user.id)
        .await
        .map_err(internal("counting unacked hunt events"))?;

    Ok(Json(EventsResponse {
        events,
        unacked_total,
        next_cursor,
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

    let minted = tokens::mint_within_limit(&pool, &user.id, &label, Utc::now())
        .await
        .map_err(internal("minting a hunt token"))?
        .ok_or(StatusCode::FORBIDDEN)?;

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

    // A linear scan over every row, expired ones included, keeps the matching rule in one place.
    // Measured 2026-11-11 at 7,937 rows (S13, `docs/PUBLIC_API_STRESS_TEST.md`): 16 ms for a URL
    // that matches nothing, and 246 req/s under 16 clients, more than the listing manages. It was
    // written at ~1,400 rows; it grows with the table, so measure again before it matters. Indexing a derived key would mean storing it, and storing it would
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
    match answers::save(&pool, &user.id, body, Utc::now())
        .await
        .map_err(internal("saving an answer"))?
    {
        Ok(answer) => Ok((StatusCode::CREATED, Json(answer))),
        Err(answers::Refused::Invalid) => Err(StatusCode::BAD_REQUEST),
        // 403, not 409: the client already reads 409 as "duplicate" elsewhere on this API.
        Err(answers::Refused::LibraryFull) => Err(StatusCode::FORBIDDEN),
    }
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
        variants::Refused::LimitReached => StatusCode::FORBIDDEN,
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

#[cfg(test)]
mod event_paging_tests {
    use super::*;
    use crate::hunt::events::{EventKind, NewHuntEvent, emit};
    use crate::models::User;
    use axum::http::Uri;
    use uuid::Uuid;

    async fn pool() -> SqlitePool {
        let path = std::env::temp_dir().join(format!("hunt-paging-{}.db", Uuid::new_v4()));
        crate::db::init_pool(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("migrations")
    }

    fn viewer() -> CurrentUser {
        // Posting alerts are visible to every user, so the viewer needs no row of its own.
        CurrentUser(User {
            id: Uuid::new_v4().to_string(),
            email: "who@example.com".into(),
            password_hash: None,
            created_at: Utc::now(),
            is_admin: false,
        })
    }

    async fn posting_alert(pool: &SqlitePool, subject: &str, at: DateTime<Utc>) {
        let event = NewHuntEvent {
            kind: EventKind::Posting,
            user_id: None,
            subject_id: subject.to_string(),
            title: subject.to_string(),
            body: "body".to_string(),
            url: None,
            payload: serde_json::json!({}),
        };
        emit(pool, &event, at).await.expect("emit");
    }

    /// Exactly the URL `popup.js` builds: `include_acked`, its page size, and the cursor it was
    /// handed, if any.
    async fn popup_page(pool: &SqlitePool, who: &CurrentUser, cursor: Option<&str>) -> Result<EventsResponse, StatusCode> {
        let mut url = "/hunt/events?include_acked=true&limit=3".to_string();
        if let Some(cursor) = cursor {
            url.push_str(&format!("&cursor={cursor}"));
        }
        let query = Query::try_from_uri(&url.parse::<Uri>().unwrap()).expect("the popup's URL must parse");
        list_events(State(pool.clone()), CurrentUser(who.0.clone()), query).await.map(|json| json.0)
    }

    #[tokio::test]
    async fn following_next_cursor_reaches_every_alert_once_and_then_stops() {
        let pool = pool().await;
        let who = viewer();
        let batch = Utc::now() - chrono::Duration::days(3);
        for n in 0..5 {
            posting_alert(&pool, &format!("batch-{n}"), batch).await;
        }
        for n in 0..3 {
            posting_alert(&pool, &format!("older-{n}"), batch - chrono::Duration::days(n + 1)).await;
        }

        let mut seen = Vec::new();
        let mut cursor: Option<String> = None;
        let mut pages = 0;
        loop {
            let page = popup_page(&pool, &who, cursor.as_deref()).await.expect("page");
            pages += 1;
            assert!(page.events.len() <= 3);
            seen.extend(page.events.iter().map(|event| event.subject_id.clone()));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
            assert!(pages < 10, "next_cursor never ran out");
        }

        assert_eq!(pages, 3, "8 alerts at 3 a page is 3 pages, and the last says so");
        let mut unique = seen.clone();
        unique.sort();
        unique.dedup();
        assert_eq!((seen.len(), unique.len()), (8, 8), "every alert once: {seen:?}");
        assert_eq!(&seen[5..], ["older-0", "older-1", "older-2"], "oldest last");
    }

    #[tokio::test]
    async fn a_page_that_holds_the_last_alert_offers_no_cursor() {
        // Exactly a page's worth: the extra row fetched to look ahead finds nothing.
        let pool = pool().await;
        let who = viewer();
        for n in 0..3 {
            posting_alert(&pool, &format!("p{n}"), Utc::now() - chrono::Duration::hours(n)).await;
        }
        let page = popup_page(&pool, &who, None).await.expect("page");
        assert_eq!(page.events.len(), 3);
        assert_eq!(page.next_cursor, None);
    }

    #[tokio::test]
    async fn a_cursor_that_does_not_decode_is_a_400_not_the_first_page() {
        let pool = pool().await;
        let who = viewer();
        posting_alert(&pool, "p0", Utc::now()).await;
        for bad in ["zz", "6e6f2d706970652d686572", ""] {
            let result = popup_page(&pool, &who, Some(bad)).await;
            assert_eq!(result.err(), Some(StatusCode::BAD_REQUEST), "cursor={bad:?}");
        }
    }
}

#[cfg(test)]
mod quota_tests {
    //! S14 (docs/PUBLIC_API_STRESS_TEST.md, second pass). One signed-in account wrote 45,410
    //! maximum-length answers in 10 seconds, growing the database from 49 MB to 1,128 MB; listing
    //! them built a 1 GB response, and four of those took the server to 2.8 GiB. Every collection
    //! an account can append to has to stop somewhere. Driven through the real router, so a cap
    //! the route does not enforce cannot pass.
    use axum::http::StatusCode;
    use uuid::Uuid;

    /// A router on a fresh database, and a session cookie for one registered account.
    async fn signed_in() -> (String, String, sqlx::SqlitePool) {
        let path = std::env::temp_dir().join(format!("quota-{}.db", Uuid::new_v4()));
        let pool = crate::db::init_pool(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("pool");
        let app = crate::routes::build_router(crate::routes::AppState {
            pool: pool.clone(),
            catalog: std::sync::Arc::new(crate::foodkeeper::Catalog::load().expect("catalog")),
            recipe_catalog: std::sync::Arc::new(
                crate::themealdb::Catalog::load().expect("recipe catalog"),
            ),
            google_oauth: None,
            rate_limits: crate::rate_limit::RateLimits::default(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .expect("serve");
        });
        let base = format!("http://{address}");
        let response = reqwest::Client::new()
            .post(format!("{base}/auth/register"))
            .json(&serde_json::json!({
                "email": "quota@example.com",
                "password": "a-long-enough-password",
            }))
            .send()
            .await
            .expect("register");
        assert_eq!(response.status(), StatusCode::CREATED);
        let cookie = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find(|value| value.starts_with("fridge_session="))
            .and_then(|value| value.split(';').next())
            .expect("session cookie")
            .to_string();
        (base, cookie, pool)
    }

    async fn post(base: &str, cookie: &str, path: &str, body: serde_json::Value) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{base}{path}"))
            .header("cookie", cookie)
            .json(&body)
            .send()
            .await
            .expect("request")
    }

    #[tokio::test]
    async fn the_answer_library_stops_at_its_ceiling() {
        let (base, cookie, _) = signed_in().await;
        for n in 0..500 {
            let response = post(&base, &cookie, "/hunt/answers", serde_json::json!({
                "question_text": format!("Question {n}"),
                "answer_text": "An answer.",
            }))
            .await;
            assert_eq!(response.status(), StatusCode::CREATED, "answer {n} is within the ceiling");
        }
        let response = post(&base, &cookie, "/hunt/answers", serde_json::json!({
            "question_text": "One too many",
            "answer_text": "An answer.",
        }))
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "the 501st answer is refused");
    }

    #[tokio::test]
    async fn resume_variants_stop_at_their_ceiling() {
        let (base, cookie, _) = signed_in().await;
        for n in 0..100 {
            let response = post(&base, &cookie, "/hunt/resume-variants", serde_json::json!({
                "label": format!("Variant {n}"),
            }))
            .await;
            assert_eq!(response.status(), StatusCode::CREATED, "variant {n} is within the ceiling");
        }
        let response = post(&base, &cookie, "/hunt/resume-variants", serde_json::json!({
            "label": "One too many",
        }))
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "the 101st variant is refused");
    }

    #[tokio::test]
    async fn live_extension_tokens_stop_at_their_ceiling_and_revoking_frees_a_slot() {
        let (base, cookie, _) = signed_in().await;
        let mut first_id = String::new();
        for n in 0..20 {
            let response = post(&base, &cookie, "/hunt/tokens", serde_json::json!({})).await;
            assert_eq!(response.status(), StatusCode::CREATED, "token {n} is within the ceiling");
            if n == 0 {
                let minted: serde_json::Value = response.json().await.expect("json");
                first_id = minted["id"].as_str().expect("id").to_string();
            }
        }
        let response = post(&base, &cookie, "/hunt/tokens", serde_json::json!({})).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "the 21st live token is refused");

        let revoked = reqwest::Client::new()
            .delete(format!("{base}/hunt/tokens/{first_id}"))
            .header("cookie", &cookie)
            .send()
            .await
            .expect("revoke");
        assert!(revoked.status().is_success(), "revoke: {}", revoked.status());
        let response = post(&base, &cookie, "/hunt/tokens", serde_json::json!({})).await;
        assert_eq!(response.status(), StatusCode::CREATED, "revoking one frees a slot");
    }

    /// Revoked tokens stay as tombstones, so minting and revoking in a loop was the way round a
    /// cap on live tokens.
    #[tokio::test]
    async fn revoked_token_tombstones_do_not_accumulate_without_limit() {
        let (base, cookie, pool) = signed_in().await;
        for _ in 0..130 {
            let minted: serde_json::Value = post(&base, &cookie, "/hunt/tokens", serde_json::json!({}))
                .await
                .json()
                .await
                .expect("json");
            let id = minted["id"].as_str().expect("id").to_string();
            reqwest::Client::new()
                .delete(format!("{base}/hunt/tokens/{id}"))
                .header("cookie", &cookie)
                .send()
                .await
                .expect("revoke");
        }
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM hunt_tokens")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(rows, 100, "the newest 100 revoked tokens are kept as tombstones");
    }
}
