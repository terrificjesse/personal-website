//! `/hunt/labelling` — the labelling page's four endpoints. Logic lives in `inbox::labelling`.
//!
//! Only `GET` reads, and it reads the database alone: the next message and its body come back
//! from a `POST` the labeller triggers, because nothing in this backend fetches from Gmail on a
//! read (`routes/inbox.rs`). Every response is scoped to the signed-in user, and none carries
//! the classifier's verdict.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::auth::GoogleOAuthConfig;
use crate::inbox::classify::Category;
use crate::inbox::labelling::{self, Item, Progress};
use crate::routes::auth::CurrentUser;

fn internal(err: anyhow::Error) -> StatusCode {
    eprintln!("labelling: {err:?}");
    StatusCode::INTERNAL_SERVER_ERROR
}

/// How far through the held-out mail the labeller is. Database only.
pub async fn progress(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Progress>, StatusCode> {
    labelling::progress(&pool, &user.id).await.map(Json).map_err(internal)
}

#[derive(Serialize)]
pub struct Next {
    /// `None` when every held-out message has been labelled or skipped.
    item: Option<Item>,
    progress: Progress,
}

/// The next message, with its body. A `POST` because it reaches Gmail.
pub async fn next(
    State(pool): State<SqlitePool>,
    State(config): State<Option<GoogleOAuthConfig>>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Next>, StatusCode> {
    let progress = labelling::progress(&pool, &user.id).await.map_err(internal)?;
    let Some(candidate) = labelling::next_candidate(&pool, &user.id).await.map_err(internal)? else {
        return Ok(Json(Next { item: None, progress }));
    };

    let account: Option<String> =
        sqlx::query_scalar("SELECT email FROM gmail_accounts WHERE user_id = ?")
            .bind(&user.id)
            .fetch_optional(&pool)
            .await
            .map_err(|err| internal(err.into()))?;

    // Best effort. Without a connected, working account the page shows the snippet and says so
    // — it does not refuse to work, because the snippet plus the Gmail link still lets you label.
    let body = match (&config, &account) {
        (Some(config), Some(_)) => {
            labelling::body_for(
                &pool,
                &user.id,
                &config.client_id,
                &config.client_secret,
                &candidate.gmail_message_id,
            )
            .await
        }
        _ => None,
    };

    Ok(Json(Next {
        item: Some(Item {
            gmail_url: Some(labelling::gmail_url(account.as_deref(), &candidate.gmail_message_id)),
            message_id: candidate.id,
            received_at: candidate.received_at,
            from: candidate.from_address,
            subject: candidate.subject,
            body,
            snippet: candidate.snippet,
        }),
        progress,
    }))
}

#[derive(Deserialize)]
pub struct LabelRequest {
    /// One of the seven categories, or `null` for "could not tell".
    label: Option<String>,
}

/// Record one label. 400 for a word that is not a category, 404 for a message that is not
/// yours or is outside the held-out window.
pub async fn label(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(message_id): Path<String>,
    Json(request): Json<LabelRequest>,
) -> Result<StatusCode, StatusCode> {
    let label = match request.label.as_deref() {
        None => None,
        Some(raw) => Some(Category::parse(raw).ok_or(StatusCode::BAD_REQUEST)?),
    };
    match labelling::record(&pool, &user.id, &message_id, label).await.map_err(internal)? {
        true => Ok(StatusCode::NO_CONTENT),
        false => Err(StatusCode::NOT_FOUND),
    }
}

/// Undo a label; the message goes back into the queue.
pub async fn undo(
    State(pool): State<SqlitePool>,
    CurrentUser(user): CurrentUser,
    Path(message_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    match labelling::undo(&pool, &user.id, &message_id).await.map_err(internal)? {
        true => Ok(StatusCode::NO_CONTENT),
        false => Err(StatusCode::NOT_FOUND),
    }
}
