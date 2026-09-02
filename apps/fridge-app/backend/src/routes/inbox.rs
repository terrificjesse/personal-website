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
    let row: Option<(String, String, i64)> = sqlx::query_as(
        "SELECT p.application_id, p.to_status, p.applied_automatically
           FROM status_proposals p
           JOIN internship_applications a ON a.id = p.application_id
          WHERE p.id = ? AND a.user_id = ? AND p.reviewed_at IS NULL",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| {
        eprintln!("inbox: reading a proposal failed: {err:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let Some((application_id, to_status, was_auto)) = row else {
        return Err(StatusCode::NOT_FOUND);
    };
    let now = Utc::now();

    if accept {
        // Applying an already-auto-applied proposal again is a no-op on the status, which is
        // why this is safe to press twice.
        if ApplicationStatus::parse(&to_status).is_none() {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
        let _ = sqlx::query(
            "UPDATE internship_applications
                SET status = ?1, status_changed_at = ?2, updated_at = ?2
              WHERE id = ?3 AND user_id = ?4",
        )
        .bind(&to_status)
        .bind(now.to_rfc3339())
        .bind(&application_id)
        .bind(user_id)
        .execute(pool)
        .await;
    } else if was_auto == 1 {
        // Rejecting a proposal that was already applied has to UNDO it. Without this, "reject"
        // would only mean "stop showing me this", and the tracker would keep the change the
        // user just said was wrong — which is the whole failure rule 2 exists to prevent.
        let previous: Option<String> = sqlx::query_scalar(
            "SELECT from_status FROM status_proposals WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        if let Some(previous) = previous {
            let _ = sqlx::query(
                "UPDATE internship_applications
                    SET status = ?1, status_changed_at = ?2, updated_at = ?2
                  WHERE id = ?3 AND user_id = ?4",
            )
            .bind(previous)
            .bind(now.to_rfc3339())
            .bind(&application_id)
            .bind(user_id)
            .execute(pool)
            .await;
        }
    }

    sqlx::query("UPDATE status_proposals SET reviewed_at = ?1, accepted = ?2 WHERE id = ?3")
        .bind(now.to_rfc3339())
        .bind(i64::from(accept))
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| {
            eprintln!("inbox: recording a review failed: {err:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(StatusCode::NO_CONTENT)
}
