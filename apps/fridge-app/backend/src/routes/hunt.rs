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

/// Log the cause, tell the client nothing. Mirrors `routes::internships::internal`.
fn internal(context: &'static str) -> impl Fn(anyhow::Error) -> StatusCode {
    move |err| {
        eprintln!("hunt: {context} failed: {err:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    }
}
