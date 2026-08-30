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

use crate::hunt::events::{self, AckOutcome, EventQuery, HuntEvent};
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
    use crate::internships::dedup::{ats_identity, canonical_url};

    let wanted_identity = ats_identity(&params.url);
    let wanted_canonical = canonical_url(&params.url);

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
