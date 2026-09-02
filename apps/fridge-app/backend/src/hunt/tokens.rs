//! Bearer tokens for the extension, because the session cookie cannot reach it.
//!
//! # Why this exists at all
//!
//! `apps/hunt-extension/CLAUDE.md` says to reuse the site's `fridge_session` cookie and to
//! fall back to a dedicated token only if that proves awkward in Firefox. It was tried first,
//! properly, and it is not awkward — it is blocked. The cookie is `SameSite=Lax`, a request
//! from a `moz-extension://` page is cross-site, and Firefox therefore never attaches it. The
//! backend saw an anonymous request and answered 401 while the user was demonstrably signed in
//! on the site. Three other causes wore that same symptom on the way here; this is the last of
//! them and the only one that was not a bug on our side.
//!
//! # This is a second credential, not a second auth system
//!
//! The distinction is what keeps the 8e constraint honest. Everything below reuses what
//! already exists:
//!
//! - Hashing is [`auth::session_token_hash`] and generation is [`auth::generate_session_token`],
//!   **called, never modified** — `src/auth.rs` is a `[learn]` file. One definition of "hash a
//!   bearer credential" in the codebase, not two that can drift.
//! - Validation returns the same [`User`] the cookie path returns, so every route keeps its
//!   existing `CurrentUser` signature and no route learns that tokens exist.
//! - Minting requires a live *cookie* session. A token can only be created by someone already
//!   signed in on the site, so this widens how you prove who you are, never who you can be.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::auth;
use crate::models::User;

/// The longest label we will store. Same reasoning as the blog's body cap: an unbounded text
/// column filled by a client is a denial-of-service vector, not a feature.
pub const MAX_LABEL_LENGTH: usize = 100;

/// A token as the API lists it. **Never carries the token itself** — that exists exactly once,
/// in the response to the call that minted it.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct HuntToken {
    pub id: String,
    pub label: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

/// What minting returns: the record, plus the one and only sight of the secret.
#[derive(Debug, Clone, Serialize)]
pub struct MintedToken {
    #[serde(flatten)]
    pub token: HuntToken,
    /// Shown once. We store only its hash, so this cannot be recovered — losing it means
    /// minting another and revoking this one.
    pub secret: String,
}

/// Create a token for a user. The caller must already be authenticated.
pub async fn mint(pool: &SqlitePool, user_id: &str, label: &str, now: DateTime<Utc>)
    -> Result<MintedToken>
{
    let label = label.trim();
    let label = if label.is_empty() { "Firefox extension" } else { label };

    let id = Uuid::new_v4().to_string();
    let secret = auth::generate_session_token();

    sqlx::query(
        "INSERT INTO hunt_tokens (id, user_id, token_hash, label, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(auth::session_token_hash(&secret))
    .bind(label)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;

    Ok(MintedToken {
        token: HuntToken {
            id,
            label: label.to_string(),
            created_at: now,
            last_used_at: None,
        },
        secret,
    })
}

/// The user this token belongs to, or `None` if it is unknown or revoked.
///
/// Bumps `last_used_at` on success. That write is best-effort: failing to record a use must
/// never fail the request it was recording, or a busy database turns into a logged-out user.
pub async fn validate(pool: &SqlitePool, secret: &str, now: DateTime<Utc>) -> Result<Option<User>> {
    let hash = auth::session_token_hash(secret);

    // Two queries rather than one join: `User` is a `FromRow` struct, so it cannot ride along
    // in a tuple, and spelling its columns out here would couple this file to a shape that
    // belongs to `models`. Neither query is on a hot path.
    let found: Option<(String, String)> = sqlx::query_as(
        "SELECT id, user_id FROM hunt_tokens WHERE token_hash = ? AND revoked_at IS NULL",
    )
    .bind(&hash)
    .fetch_optional(pool)
    .await?;

    let Some((token_id, user_id)) = found else {
        return Ok(None);
    };

    let user: Option<User> = sqlx::query_as(
        "SELECT id, email, password_hash, created_at, is_admin FROM users WHERE id = ?",
    )
    .bind(&user_id)
    .fetch_optional(pool)
    .await?;

    // A token whose user is gone authenticates nobody. Foreign keys are enforced through the
    // application, but the `sqlite3` CLI does not enforce them, so a hand-deleted user is
    // reachable — the same dangling-reference case `internship_applications` documents.
    let Some(user) = user else {
        return Ok(None);
    };

    if let Err(err) = sqlx::query("UPDATE hunt_tokens SET last_used_at = ?1 WHERE id = ?2")
        .bind(now.to_rfc3339())
        .bind(&token_id)
        .execute(pool)
        .await
    {
        eprintln!("hunt: could not record use of token {token_id}: {err:?}");
    }

    Ok(Some(user))
}

/// A user's live tokens, newest first.
pub async fn list(pool: &SqlitePool, user_id: &str) -> Result<Vec<HuntToken>> {
    Ok(sqlx::query_as::<_, HuntToken>(
        "SELECT id, label, created_at, last_used_at FROM hunt_tokens
          WHERE user_id = ? AND revoked_at IS NULL
          ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

/// Revoke one of this user's tokens. Returns whether anything was revoked.
///
/// A tombstone rather than a delete: "this token was revoked on the 3rd" is a different and
/// more useful answer than the row simply not being there.
pub async fn revoke(pool: &SqlitePool, id: &str, user_id: &str, now: DateTime<Utc>) -> Result<bool> {
    let affected = sqlx::query(
        "UPDATE hunt_tokens SET revoked_at = ?1
          WHERE id = ?2 AND user_id = ?3 AND revoked_at IS NULL",
    )
    .bind(now.to_rfc3339())
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?
    .rows_affected();

    Ok(affected > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
        let path = std::env::temp_dir().join(format!("hunt-tokens-{}.db", Uuid::new_v4()));
        crate::db::init_pool(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("migrations")
    }

    async fn user(pool: &SqlitePool, id: &str) {
        sqlx::query("INSERT INTO users (id, email, password_hash, created_at) VALUES (?1,?2,?3,?4)")
            .bind(id)
            .bind(format!("{id}@example.com"))
            .bind("x")
            .bind(Utc::now().to_rfc3339())
            .execute(pool)
            .await
            .expect("user");
    }

    #[tokio::test]
    async fn a_minted_token_authenticates_its_owner() {
        let pool = test_pool().await;
        user(&pool, "u1").await;

        let minted = mint(&pool, "u1", "Firefox", Utc::now()).await.expect("mint");
        let who = validate(&pool, &minted.secret, Utc::now()).await.expect("validate");

        assert_eq!(who.expect("a user").id, "u1");
    }

    #[tokio::test]
    async fn the_secret_is_never_stored() {
        // The whole point of hashing. If this ever fails, a database read is a credential leak.
        let pool = test_pool().await;
        user(&pool, "u1").await;
        let minted = mint(&pool, "u1", "Firefox", Utc::now()).await.expect("mint");

        let stored: String = sqlx::query_scalar("SELECT token_hash FROM hunt_tokens")
            .fetch_one(&pool)
            .await
            .expect("row");

        assert_ne!(stored, minted.secret);
        assert_eq!(stored, crate::auth::session_token_hash(&minted.secret));
    }

    #[tokio::test]
    async fn a_revoked_token_authenticates_nobody() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        let minted = mint(&pool, "u1", "Firefox", Utc::now()).await.expect("mint");

        assert!(revoke(&pool, &minted.token.id, "u1", Utc::now()).await.expect("revoke"));
        assert!(validate(&pool, &minted.secret, Utc::now()).await.expect("validate").is_none());
        // And revoking twice is not a second success.
        assert!(!revoke(&pool, &minted.token.id, "u1", Utc::now()).await.expect("again"));
    }

    #[tokio::test]
    async fn one_user_cannot_revoke_anothers_token() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        user(&pool, "u2").await;
        let minted = mint(&pool, "u1", "Firefox", Utc::now()).await.expect("mint");

        assert!(!revoke(&pool, &minted.token.id, "u2", Utc::now()).await.expect("revoke"));
        assert!(
            validate(&pool, &minted.secret, Utc::now()).await.expect("validate").is_some(),
            "the token must still work for its owner"
        );
    }

    #[tokio::test]
    async fn an_unknown_token_is_refused_rather_than_erroring() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        mint(&pool, "u1", "Firefox", Utc::now()).await.expect("mint");

        let who = validate(&pool, "not-a-real-token", Utc::now()).await.expect("validate");
        assert!(who.is_none());
    }

    #[tokio::test]
    async fn use_is_recorded_so_an_unused_token_is_visibly_unused() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        let minted = mint(&pool, "u1", "Firefox", Utc::now()).await.expect("mint");

        assert!(list(&pool, "u1").await.expect("list")[0].last_used_at.is_none());
        validate(&pool, &minted.secret, Utc::now()).await.expect("validate");
        assert!(list(&pool, "u1").await.expect("list")[0].last_used_at.is_some());
    }

    #[tokio::test]
    async fn a_revoked_token_leaves_the_list() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        let a = mint(&pool, "u1", "laptop", Utc::now()).await.expect("mint");
        mint(&pool, "u1", "desktop", Utc::now()).await.expect("mint");

        assert_eq!(list(&pool, "u1").await.expect("list").len(), 2);
        revoke(&pool, &a.token.id, "u1", Utc::now()).await.expect("revoke");

        let live = list(&pool, "u1").await.expect("list");
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].label, "desktop");
    }

    #[tokio::test]
    async fn two_tokens_never_collide() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        let a = mint(&pool, "u1", "one", Utc::now()).await.expect("mint");
        let b = mint(&pool, "u1", "two", Utc::now()).await.expect("mint");

        assert_ne!(a.secret, b.secret);
        assert_eq!(
            validate(&pool, &a.secret, Utc::now()).await.expect("a").map(|u| u.id),
            Some("u1".to_string())
        );
    }
}
