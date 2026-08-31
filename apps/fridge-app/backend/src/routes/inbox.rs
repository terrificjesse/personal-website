//! Connecting the burner Gmail, and looking at what the sync did (Phase 8a).
//!
//! # Nothing here fetches from Gmail on a GET
//!
//! The root `CLAUDE.md` cache rule applies to Gmail as much as to a job board: a sync crosses
//! the network to someone else's service, so it runs on an explicit trigger, never inline in a
//! read. `GET /hunt/inbox/status` reports what the last run recorded and makes no network call
//! at all.

use axum::{
    Json,
    extract::{Query, State},
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
    let jar = jar.remove(Cookie::from(GMAIL_STATE_COOKIE));

    // A user who declined is not an error to investigate.
    if params.error.is_some() {
        return Ok((jar, Redirect::to(&front_end("gmail=declined"))));
    }

    let expected = jar.get(GMAIL_STATE_COOKIE).map(|c| c.value().to_string());
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

    let last_run = sync::last_run(&pool, &user.id)
        .await
        .map_err(internal("reading the last inbox run"))?
        .map(|(started_at, finished_at, outcome, error, fetched, classified)| LastRun {
            started_at,
            finished_at,
            outcome,
            error,
            fetched,
            classified,
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
