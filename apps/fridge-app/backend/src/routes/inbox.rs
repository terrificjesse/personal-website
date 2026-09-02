//! Connecting the burner Gmail, and looking at what the sync did (Phase 8a).
//!
//! # Nothing here fetches from Gmail on a GET
//!
//! The root `CLAUDE.md` cache rule applies to Gmail as much as to a job board: a sync crosses
//! the network to someone else's service, so it runs on an explicit trigger, never inline in a
//! read. `GET /hunt/inbox/status` reports what the last run recorded and makes no network call
//! at all.
//!
//! # Proposal audit contract
//!
//! `GET /hunt/proposals` returns the triggering email's `from_address` and `subject` as
//! separate nullable fields. The reviewer needs both to check a proposed status change; the
//! subject is evidence about what happened, but it is never a substitute for who sent it.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Redirect,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::auth::GoogleOAuthConfig;
use crate::inbox::{oauth, sync};
use crate::internships::application_events::{self, Actor, Cause, NewApplicationEvent};
use crate::internships::models::ApplicationStatus;
use crate::routes::auth::CurrentUser;

/// Guards the callback against a request the user did not start. Its own cookie, separate from
/// sign-in's, so connecting Gmail cannot consume or invalidate a sign-in in flight.
const GMAIL_STATE_COOKIE: &str = "gmail_oauth_state";

fn internal(context: &'static str) -> impl Fn(anyhow::Error) -> StatusCode {
    move |err| {
        eprintln!("inbox: {context} failed: {err:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

/// Begin the consent flow.
pub async fn start(
    CurrentUser(_user): CurrentUser,
    State(config): State<Option<GoogleOAuthConfig>>,
    jar: CookieJar,
) -> Result<(CookieJar, Redirect), StatusCode> {
    let config = config.ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let state = Uuid::new_v4().to_string();

    // `SameSite=Lax` and host-scoped, exactly like the sign-in state cookie. The lesson from
    // Phase 5 applies unchanged: the state cookie is set on whichever host `/start` was
    // reached on, and if the redirect URI names a different host the cookie is never sent back
    // and the callback fails its own check. Keep GMAIL_REDIRECT_URI on the same host you browse.
    let cookie = Cookie::build((GMAIL_STATE_COOKIE, state.clone()))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(time::Duration::minutes(10))
        .build();

    Ok((
        jar.add(cookie),
        Redirect::to(&oauth::consent_url(&config.client_id, &state)),
    ))
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// Google sends the user back here.
pub async fn callback(
    State(pool): State<SqlitePool>,
    State(config): State<Option<GoogleOAuthConfig>>,
    CurrentUser(user): CurrentUser,
    jar: CookieJar,
    Query(params): Query<CallbackQuery>,
) -> Result<(CookieJar, Redirect), StatusCode> {
    let config = config.ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    // READ THE STATE BEFORE CLEARING IT. `remove` returns a jar without that cookie, so a
    // `get` afterwards yields None, `expected` is always None, and every callback 400s — the
    // flow could not have worked once. Written the other way round first, and it was invisible
    // to the whole suite because nothing exercises this path without Google on the other end:
    // the Phase 5 lesson in apps/fridge-app/CLAUDE.md, in the same function it was learned in.
    let expected = jar.get(GMAIL_STATE_COOKIE).map(|c| c.value().to_string());
    let jar = jar.remove(Cookie::from(GMAIL_STATE_COOKIE));

    // A user who declined is not an error to investigate.
    if params.error.is_some() {
        return Ok((jar, Redirect::to(&front_end("gmail=declined"))));
    }

    let (Some(code), Some(state), Some(expected)) = (params.code, params.state, expected) else {
        return Err(StatusCode::BAD_REQUEST);
    };
    if state != expected {
        return Err(StatusCode::BAD_REQUEST);
    }

    match oauth::connect(
        &pool, &user.id, &config.client_id, &config.client_secret, &code, Utc::now(),
    )
    .await
    {
        Ok(_) => Ok((jar, Redirect::to(&front_end("gmail=connected")))),
        Err(err) => {
            // The reasons are specific and actionable — an unticked scope, no refresh token —
            // so they go to the log rather than being flattened into a 500 page.
            eprintln!("inbox: connecting Gmail failed: {err:?}");
            Ok((jar, Redirect::to(&front_end("gmail=failed"))))
        }
    }
}

fn front_end(query: &str) -> String {
    let base = std::env::var("FRONTEND_ORIGIN")
        .unwrap_or_else(|_| "http://localhost:3000".to_string());
    format!("{base}/internships?{query}")
}

#[derive(Debug, Serialize)]
pub struct InboxStatus {
    /// The connected address, or `None`. The extension shows this so a disconnected agent is
    /// visibly disconnected rather than merely quiet.
    pub account: Option<String>,
    pub last_run: Option<LastRun>,
}

#[derive(Debug, Serialize)]
pub struct LastRun {
    pub started_at: String,
    pub finished_at: Option<String>,
    pub outcome: String,
    /// Populated on a failed or partial run. **This is rule 5's whole point**: without it, a
    /// run that could not authenticate and a run that found nothing are the same empty row.
    pub error: Option<String>,
    /// True when the account was reconnected *after* this run finished.
    ///
    /// Rule 5 says a broken sync must be visible — but a failure whose cause has already been
    /// fixed is not visibility, it is a wrong answer. Reconnecting expires weekly while the
    /// OAuth app is in Testing, so without this the interface tells you the token is dead for
    /// up to fifteen minutes after you have replaced it, and the obvious conclusion is that
    /// the reconnect did not work.
    pub superseded_by_reconnect: bool,
    pub fetched: i64,
    pub classified: i64,
}

/// What the last pass did. Reads the record; makes no network call.
pub async fn status(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<InboxStatus>, StatusCode> {
    let account = oauth::connected_account(&pool, &user.id)
        .await
        .map_err(internal("reading the connected account"))?;

    // When the stored credential was last replaced. A failure older than this has already
    // been addressed.
    let reconnected_at: Option<String> =
        sqlx::query_scalar("SELECT updated_at FROM gmail_accounts WHERE user_id = ?")
            .bind(&user.id)
            .fetch_optional(&pool)
            .await
            .map_err(|err| {
                eprintln!("inbox: reading the account timestamp failed: {err:?}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

    let last_run = sync::last_run(&pool, &user.id)
        .await
        .map_err(internal("reading the last inbox run"))?
        .map(|(started_at, finished_at, outcome, error, fetched, classified)| {
            // RFC3339 strings from the same source sort lexicographically in timestamp order,
            // which is why every timestamp in this schema is stored that way.
            let superseded = reconnected_at
                .as_deref()
                .is_some_and(|reconnected| reconnected > started_at.as_str());
            LastRun {
                started_at,
                finished_at,
                outcome,
                error,
                superseded_by_reconnect: superseded,
                fetched,
                classified,
            }
        });

    Ok(Json(InboxStatus { account, last_run }))
}

/// Run a pass now. The explicit trigger; there is no automatic one in 8a.
pub async fn sync_now(
    State(pool): State<SqlitePool>,
    State(config): State<Option<GoogleOAuthConfig>>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<SyncSummary>, StatusCode> {
    let config = config.ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let report = sync::run(
        &pool,
        &user.id,
        &config.client_id,
        &config.client_secret,
        sync::DEFAULT_MAX_MESSAGES,
        Utc::now(),
    )
    .await
    .map_err(internal("running an inbox sync"))?;

    Ok(Json(SyncSummary {
        run_id: report.run_id,
        fetched: report.fetched,
        classified: report.classified,
        already_seen: report.already_seen,
        pressing: report.pressing,
        confirmation: report.confirmation,
        outreach: report.outreach,
        disregarded: report.disregarded,
    }))
}

#[derive(Debug, Serialize)]
pub struct SyncSummary {
    pub run_id: String,
    pub fetched: i64,
    pub classified: i64,
    pub already_seen: i64,
    pub pressing: i64,
    pub confirmation: i64,
    pub outreach: i64,
    pub disregarded: i64,
}

/// Forget the stored account. Local only.
pub async fn disconnect(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
) -> Result<StatusCode, StatusCode> {
    // Deliberately does NOT revoke the grant at Google. Doing that silently would be a
    // side effect on an account this app does not own; the user revokes at
    // myaccount.google.com/permissions if they want it gone there too.
    oauth::disconnect(&pool, &user.id)
        .await
        .map_err(internal("disconnecting Gmail"))?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// The bug this file shipped with, pinned as behaviour rather than as a comment.
    ///
    /// `CookieJar::remove` returns a jar with the cookie gone, so anything that reads after
    /// removing reads nothing. The callback did exactly that and rejected every consent it was
    /// given. No test in the suite could have caught it, because the path needs Google at the
    /// other end — which is precisely why the ordering is worth asserting directly.
    #[test]
    fn a_jar_read_after_removal_yields_nothing() {
        let jar = CookieJar::new().add(Cookie::new(GMAIL_STATE_COOKIE, "abc123"));
        assert_eq!(
            jar.get(GMAIL_STATE_COOKIE).map(|c| c.value().to_string()),
            Some("abc123".to_string()),
            "reading first works"
        );

        let emptied = jar.remove(Cookie::from(GMAIL_STATE_COOKIE));
        assert_eq!(
            emptied.get(GMAIL_STATE_COOKIE).map(|c| c.value().to_string()),
            None,
            "reading after removing does not — capture the value before clearing it"
        );
    }

    #[test]
    fn the_state_cookie_is_not_the_session_cookie() {
        // Its own name on purpose: connecting Gmail must not consume or invalidate a sign-in
        // in flight, and sharing a cookie name is how that happens by accident.
        assert_ne!(GMAIL_STATE_COOKIE, "fridge_session");
        assert_ne!(GMAIL_STATE_COOKIE, "oauth_state");
    }

    #[tokio::test]
    async fn proposals_keep_the_sender_and_subject_separate() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");

        sqlx::query(
            "INSERT INTO users (id, email, created_at)
             VALUES ('user-1', 'reviewer@example.com', '2026-09-02T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("user");
        sqlx::query(
            "INSERT INTO internship_applications
                (id, user_id, company_name, title, url, snapshot_json, snapshot_at, status,
                 applied_at, status_changed_at, created_at, updated_at)
             VALUES
                ('application-1', 'user-1', 'Example Co', 'Engineer', 'https://example.com/job',
                 '{}', '2026-09-02T00:00:00Z', 'applied', '2026-09-02T00:00:00Z',
                 '2026-09-02T00:00:00Z', '2026-09-02T00:00:00Z', '2026-09-02T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("application");
        sqlx::query(
            "INSERT INTO email_messages
                (id, user_id, gmail_message_id, from_address, subject, created_at)
             VALUES
                ('message-1', 'user-1', 'gmail-1', 'recruiter@example.com',
                 'Your interview', '2026-09-02T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("message");
        sqlx::query(
            "INSERT INTO email_verdicts
                (id, message_id, category, confidence, matched_application_id, classifier,
                 evidence, created_at)
             VALUES
                ('verdict-1', 'message-1', 'interview', 0.9, 'application-1', 'rules',
                 'interview', '2026-09-02T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("verdict");
        sqlx::query(
            "INSERT INTO status_proposals
                (id, application_id, verdict_id, from_status, to_status, created_at)
             VALUES
                ('proposal-1', 'application-1', 'verdict-1', 'applied', 'interview',
                 '2026-09-02T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("proposal");

        let proposals = fetch_proposals(&pool, "user-1")
            .await
            .expect("proposal query");

        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].from_address.as_deref(), Some("recruiter@example.com"));
        assert_eq!(proposals[0].subject.as_deref(), Some("Your interview"));
    }
}

// ------------------------------------------------------------------------------------------
// Status proposals (Phase 8c, the reversible half)
// ------------------------------------------------------------------------------------------

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Proposal {
    pub id: String,
    pub application_id: String,
    pub company_name: String,
    pub title: String,
    pub from_status: String,
    pub to_status: String,
    pub applied_automatically: bool,
    /// The email that caused it. **This link is what makes a bad call reversible** — rule 2.
    pub from_address: Option<String>,
    pub subject: Option<String>,
    pub evidence: Option<String>,
    pub confidence: Option<f64>,
    pub created_at: String,
}

/// Proposals still awaiting a decision, newest first.
pub async fn proposals(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<Proposal>>, StatusCode> {
    fetch_proposals(&pool, &user.id)
        .await
        .map(Json)
        .map_err(|err| {
            eprintln!("inbox: listing proposals failed: {err:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn fetch_proposals(pool: &SqlitePool, user_id: &str) -> Result<Vec<Proposal>, sqlx::Error> {
    sqlx::query_as::<_, Proposal>(
        "SELECT p.id, p.application_id, a.company_name, a.title,
                p.from_status, p.to_status, p.applied_automatically,
                m.from_address, m.subject, v.evidence, v.confidence, p.created_at
           FROM status_proposals p
           JOIN internship_applications a ON a.id = p.application_id
           JOIN email_verdicts v ON v.id = p.verdict_id
           JOIN email_messages m ON m.id = v.message_id
          WHERE a.user_id = ? AND p.reviewed_at IS NULL
          ORDER BY p.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Accept a proposal: apply the status change and mark it reviewed.
pub async fn accept_proposal(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    decide(&pool, &user.id, &id, true).await
}

/// Reject a proposal: mark it reviewed and change nothing.
pub async fn reject_proposal(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    decide(&pool, &user.id, &id, false).await
}

async fn decide(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
    accept: bool,
) -> Result<StatusCode, StatusCode> {
    // **One transaction, because this decision is two writes.** The tracker moves and the
    // proposal is marked reviewed, and a half of that is worse than neither: a reviewed
    // proposal whose status change never landed reads as settled while the application sits
    // at the old stage, and nothing anywhere records that the two disagree. Rule 2 says an
    // email-driven change must stay reversible, which it cannot be if the audit row and the
    // change it describes can come apart.
    let mut tx = crate::db::begin_write(pool).await.map_err(|err| {
        eprintln!("inbox: opening a transaction failed: {err:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // `from_status` is read here rather than in a second query on the undo path below. It is
    // needed inside this transaction, and the read it replaces ended in `.ok().flatten()` —
    // so a failing lookup was indistinguishable from a proposal that had no previous status,
    // and the undo silently did nothing.
    let row: Option<(String, String, String, i64)> = sqlx::query_as(
        "SELECT p.application_id, p.from_status, p.to_status, p.applied_automatically
           FROM status_proposals p
           JOIN internship_applications a ON a.id = p.application_id
          WHERE p.id = ? AND a.user_id = ? AND p.reviewed_at IS NULL",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| {
        eprintln!("inbox: reading a proposal failed: {err:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let Some((application_id, from_status, to_status, was_auto)) = row else {
        return Err(StatusCode::NOT_FOUND);
    };
    let now = Utc::now();

    // What this decision leaves the application at, if it moves it at all. Accepting applies
    // the proposal — a no-op on the status when it was already auto-applied, which is why
    // pressing accept twice is safe. Rejecting undoes it, but only if it was auto-applied:
    // otherwise "reject" would only mean "stop showing me this", and the tracker would keep
    // the change the user just said was wrong.
    // `(where it ends up, where it came from)` — the second is what the event records as
    // `from_status`. Accepting moves it to the proposal's `to_status`; undoing puts it back,
    // so the pair is reversed.
    let next = if accept {
        if ApplicationStatus::parse(&to_status).is_none() {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
        Some((to_status, from_status))
    } else if was_auto == 1 {
        Some((from_status, to_status))
    } else {
        None
    };

    if let Some((next, previous)) = next {
        let moved = sqlx::query(
            "UPDATE internship_applications
                SET status = ?1, status_changed_at = ?2, updated_at = ?2
              WHERE id = ?3 AND user_id = ?4",
        )
        .bind(&next)
        .bind(now.to_rfc3339())
        .bind(&application_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| {
            eprintln!("inbox: applying a proposal to the application failed: {err:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        // The SELECT above joined on this row, so it exists and belongs to this user. Zero
        // rows here therefore means something changed underneath us — worth failing on rather
        // than reviewing a proposal whose change did not happen.
        if moved.rows_affected() != 1 {
            eprintln!(
                "inbox: applying proposal {id} matched {} application rows, expected 1",
                moved.rows_affected()
            );
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }

        // Parsed *after* the write, deliberately. The applications table's CHECK has just
        // accepted this value, so it is in the enum by construction — but "cannot fail" is a
        // claim about a constraint in another file, and the undo path reaches here with a
        // status this code never validated. A 500 rolls the whole decision back; an `unwrap`
        // would take the process with it.
        let (Some(to_status), from_status) = (
            ApplicationStatus::parse(&next),
            ApplicationStatus::parse(&previous),
        ) else {
            eprintln!("inbox: proposal {id} applied status {next:?}, which does not parse");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        };

        // Actor `manual`, not `email`: the cause is an email and `cause_id` records it, but a
        // person clicked. Collapsing the two would make "how often do I accept what the
        // classifier proposes" unanswerable, which is the number 13e needs.
        //
        // Accepting a proposal that was ALREADY auto-applied hits the same
        // (application, cause, to_status) key the email actor wrote and returns
        // `AlreadyRecorded` — a normal outcome, and the reason `to_status` is in that key is
        // the undo below, which differs from it.
        application_events::record(
            &mut tx,
            NewApplicationEvent {
                application_id: &application_id,
                from_status,
                to_status,
                actor: Actor::Manual,
                cause: Some(Cause::StatusProposal(id)),
                at: now,
                note: None,
            },
        )
        .await
        .map_err(|err| {
            eprintln!("inbox: recording a reviewed proposal failed: {err:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }

    sqlx::query("UPDATE status_proposals SET reviewed_at = ?1, accepted = ?2 WHERE id = ?3")
        .bind(now.to_rfc3339())
        .bind(i64::from(accept))
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|err| {
            eprintln!("inbox: recording a review failed: {err:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    tx.commit().await.map_err(|err| {
        eprintln!("inbox: committing a proposal decision failed: {err:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod decide_tests {
    //! The proposal decision is two writes, and these hold them together.
    //!
    //! Before Phase 10 this path updated the application and the proposal as separate
    //! statements outside any transaction, and discarded the `Result` of the first with
    //! `let _ =`. A failed status change therefore left the proposal marked reviewed and
    //! accepted while the tracker never moved — the proposal claiming a change that did not
    //! happen, which is exactly the state rule 2 exists to make impossible.

    use super::*;
    use uuid::Uuid;

    async fn pool() -> SqlitePool {
        let path = std::env::temp_dir().join(format!("inbox-decide-{}.db", Uuid::new_v4()));
        crate::db::init_pool(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("migrations")
    }

    async fn user(pool: &SqlitePool) -> String {
        let id = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO users (id, email, created_at) VALUES (?1, ?2, ?3)")
            .bind(&id)
            .bind(format!("{id}@example.com"))
            .bind(Utc::now().to_rfc3339())
            .execute(pool)
            .await
            .unwrap();
        id
    }

    async fn application(pool: &SqlitePool, user_id: &str, status: &str) -> String {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO internship_applications
                (id, user_id, company_name, title, url, snapshot_json, snapshot_at,
                 status, applied_at, status_changed_at, created_at, updated_at)
             VALUES (?1, ?2, 'Jump Trading', 'SWE Intern', 'https://example.com/j',
                     '{}', ?3, ?4, ?3, ?3, ?3, ?3)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(&now)
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    /// A proposal needs a verdict, which needs a message: `status_proposals.verdict_id` is a
    /// real foreign key and sqlx turns `PRAGMA foreign_keys` on per connection.
    async fn proposal(
        pool: &SqlitePool,
        user_id: &str,
        application_id: &str,
        from_status: &str,
        to_status: &str,
        auto: i64,
    ) -> String {
        let now = Utc::now().to_rfc3339();
        let message_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO email_messages (id, user_id, gmail_message_id, created_at)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(&message_id)
        .bind(user_id)
        .bind(Uuid::new_v4().to_string())
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();

        let verdict_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO email_verdicts (id, message_id, category, classifier, created_at)
             VALUES (?1, ?2, 'oa', 'rules', ?3)",
        )
        .bind(&verdict_id)
        .bind(&message_id)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();

        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO status_proposals
                (id, application_id, verdict_id, from_status, to_status,
                 applied_automatically, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(&id)
        .bind(application_id)
        .bind(&verdict_id)
        .bind(from_status)
        .bind(to_status)
        .bind(auto)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    async fn status_of(pool: &SqlitePool, application_id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM internship_applications WHERE id = ?")
            .bind(application_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn reviewed_at(pool: &SqlitePool, proposal_id: &str) -> Option<String> {
        sqlx::query_scalar("SELECT reviewed_at FROM status_proposals WHERE id = ?")
            .bind(proposal_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// The regression this transaction exists for.
    ///
    /// `from_status` has no CHECK on `status_proposals` but `status` does on
    /// `internship_applications`, so a proposal carrying a status the applications table will
    /// not accept makes the undo fail for real — a database error on the first of the two
    /// writes, which is the shape the old code swallowed.
    #[tokio::test]
    async fn a_failed_undo_leaves_the_proposal_unreviewed() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        let app_id = application(&pool, &user_id, "oa").await;
        let proposal_id = proposal(&pool, &user_id, &app_id, "hired", "oa", 1).await;

        let outcome = decide(&pool, &user_id, &proposal_id, false).await;

        assert_eq!(outcome, Err(StatusCode::INTERNAL_SERVER_ERROR));
        assert_eq!(
            reviewed_at(&pool, &proposal_id).await,
            None,
            "the proposal must not read as settled when its status change did not land"
        );
        assert_eq!(status_of(&pool, &app_id).await, "oa", "nothing moved");
    }

    #[tokio::test]
    async fn rejecting_an_auto_applied_proposal_restores_the_previous_status() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        let app_id = application(&pool, &user_id, "oa").await;
        let proposal_id = proposal(&pool, &user_id, &app_id, "applied", "oa", 1).await;

        assert_eq!(
            decide(&pool, &user_id, &proposal_id, false).await,
            Ok(StatusCode::NO_CONTENT)
        );
        assert_eq!(status_of(&pool, &app_id).await, "applied", "the undo landed");
        assert!(reviewed_at(&pool, &proposal_id).await.is_some());
    }

    #[tokio::test]
    async fn accepting_the_same_proposal_twice_changes_nothing_the_second_time() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        let app_id = application(&pool, &user_id, "applied").await;
        let proposal_id = proposal(&pool, &user_id, &app_id, "applied", "oa", 0).await;

        assert_eq!(
            decide(&pool, &user_id, &proposal_id, true).await,
            Ok(StatusCode::NO_CONTENT)
        );
        assert_eq!(status_of(&pool, &app_id).await, "oa");

        // Reviewed proposals are invisible to the SELECT, so the second press is a 404 rather
        // than a second application of the same change.
        assert_eq!(
            decide(&pool, &user_id, &proposal_id, true).await,
            Err(StatusCode::NOT_FOUND)
        );
        assert_eq!(status_of(&pool, &app_id).await, "oa");
    }

    /// The pool settings the transactions above depend on.
    ///
    /// Holding a write transaction across two statements is only safe because SQLite is in WAL
    /// mode with a busy timeout — readers do not block, and a competing writer waits instead of
    /// failing instantly with SQLITE_BUSY. Both are sqlx defaults rather than anything this
    /// project sets, which is exactly why they are worth pinning: a future `init_pool` that
    /// builds `SqliteConnectOptions` by hand could drop either one, and the symptom would be
    /// intermittent 500s under concurrency rather than a compile error.
    #[tokio::test]
    async fn the_pool_is_wal_with_a_busy_timeout() {
        let pool = pool().await;

        let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(journal.to_lowercase(), "wal");

        let timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(timeout > 0, "a zero busy timeout fails instantly under contention");
    }

    /// Eight decisions at once, on one pool, all of which now open write transactions.
    #[tokio::test]
    async fn concurrent_decisions_do_not_collide() {
        let pool = pool().await;
        let user_id = user(&pool).await;

        let mut proposals = Vec::new();
        for _ in 0..8 {
            let app_id = application(&pool, &user_id, "applied").await;
            proposals.push((
                proposal(&pool, &user_id, &app_id, "applied", "oa", 0).await,
                app_id,
            ));
        }

        let mut handles = Vec::new();
        for (proposal_id, app_id) in proposals {
            let pool = pool.clone();
            let user_id = user_id.clone();
            handles.push(tokio::spawn(async move {
                let outcome = decide(&pool, &user_id, &proposal_id, true).await;
                (outcome, app_id)
            }));
        }

        for handle in handles {
            let (outcome, app_id) = handle.await.unwrap();
            assert_eq!(outcome, Ok(StatusCode::NO_CONTENT));
            assert_eq!(status_of(&pool, &app_id).await, "oa");
        }
    }

    // ---- 10e part 2: what each decision records ----

    #[derive(Debug, sqlx::FromRow, PartialEq, Eq)]
    struct EventRow {
        from_status: Option<String>,
        to_status: String,
        actor: String,
        cause_kind: Option<String>,
        cause_id: Option<String>,
    }

    async fn events_for(pool: &SqlitePool, application_id: &str) -> Vec<EventRow> {
        sqlx::query_as::<_, EventRow>(
            "SELECT from_status, to_status, actor, cause_kind, cause_id
               FROM application_events WHERE application_id = ?
              ORDER BY at ASC, created_at ASC, id ASC",
        )
        .bind(application_id)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// The email actor's row, written the way `propose_status` writes it when it auto-applies.
    async fn record_auto_apply(
        pool: &SqlitePool,
        application_id: &str,
        proposal_id: &str,
        from: ApplicationStatus,
        to: ApplicationStatus,
    ) {
        let mut tx = crate::db::begin_write(pool).await.unwrap();
        application_events::record(
            &mut tx,
            NewApplicationEvent {
                application_id,
                from_status: Some(from),
                to_status: to,
                actor: Actor::Email,
                cause: Some(Cause::StatusProposal(proposal_id)),
                at: Utc::now(),
                note: None,
            },
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }

    #[tokio::test]
    async fn accepting_a_proposal_records_one_manual_event() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        let app_id = application(&pool, &user_id, "applied").await;
        let proposal_id = proposal(&pool, &user_id, &app_id, "applied", "oa", 0).await;

        decide(&pool, &user_id, &proposal_id, true).await.unwrap();

        assert_eq!(
            events_for(&pool, &app_id).await,
            vec![EventRow {
                from_status: Some("applied".into()),
                to_status: "oa".into(),
                // The cause is an email; the actor is the person who clicked.
                actor: "manual".into(),
                cause_kind: Some("status_proposal".into()),
                cause_id: Some(proposal_id),
            }]
        );
    }

    /// Where the UNIQUE key earns its place.
    ///
    /// The email actor already recorded (application, status_proposal, p, oa) when it
    /// auto-applied. A human accepting the same proposal afterwards produces the identical
    /// key, and `record` returns `AlreadyRecorded` — which is a normal outcome, not an error.
    /// Asserting the COUNT is the point: asserting the call succeeded would pass either way.
    #[tokio::test]
    async fn accepting_an_already_auto_applied_proposal_records_no_second_event() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        let app_id = application(&pool, &user_id, "oa").await;
        let proposal_id = proposal(&pool, &user_id, &app_id, "applied", "oa", 1).await;
        record_auto_apply(
            &pool,
            &app_id,
            &proposal_id,
            ApplicationStatus::Applied,
            ApplicationStatus::Oa,
        )
        .await;

        decide(&pool, &user_id, &proposal_id, true).await.unwrap();

        let events = events_for(&pool, &app_id).await;
        assert_eq!(events.len(), 1, "the email actor's row, and nothing on top of it");
        assert_eq!(events[0].actor, "email");
    }

    /// Rejecting something that was never applied changes no status, so there is no transition
    /// to record. An event here would make the fold disagree with the column immediately.
    #[tokio::test]
    async fn rejecting_a_proposal_that_never_applied_records_nothing() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        let app_id = application(&pool, &user_id, "applied").await;
        let proposal_id = proposal(&pool, &user_id, &app_id, "applied", "oa", 0).await;

        decide(&pool, &user_id, &proposal_id, false).await.unwrap();

        assert!(events_for(&pool, &app_id).await.is_empty());
        assert_eq!(status_of(&pool, &app_id).await, "applied");
    }

    /// The undo shares its cause with the event that applied it and differs only in
    /// `to_status` — which is exactly why `to_status` is in the UNIQUE key.
    #[tokio::test]
    async fn undoing_an_auto_applied_proposal_records_the_reverse_transition() {
        let pool = pool().await;
        let user_id = user(&pool).await;
        let app_id = application(&pool, &user_id, "oa").await;
        let proposal_id = proposal(&pool, &user_id, &app_id, "applied", "oa", 1).await;
        record_auto_apply(
            &pool,
            &app_id,
            &proposal_id,
            ApplicationStatus::Applied,
            ApplicationStatus::Oa,
        )
        .await;

        decide(&pool, &user_id, &proposal_id, false).await.unwrap();

        let events = events_for(&pool, &app_id).await;
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].to_status, "applied", "back where it came from");
        assert_eq!(events[1].from_status.as_deref(), Some("oa"));
        assert_eq!(events[1].actor, "manual");
        assert_eq!(events[1].cause_id.as_deref(), Some(proposal_id.as_str()));
        assert_eq!(status_of(&pool, &app_id).await, "applied");
    }
}
