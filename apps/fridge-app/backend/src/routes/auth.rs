use axum::{
    Json,
    extract::{FromRef, FromRequestParts, Query, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::auth::{
    self, AuthError, GOOGLE_STATE_COOKIE_NAME, GoogleOAuthConfig, MAX_PASSWORD_LENGTH,
    MIN_PASSWORD_LENGTH, SESSION_COOKIE_NAME, SESSION_DURATION_DAYS,
};
use crate::models::{LoginRequest, RegisterRequest, User, normalize_email};

/// Cookie needed for Google authentication. 10 minutes is plenty of time for a
/// user to sign in using Google auth.
const OAUTH_STATE_COOKIE_MINUTES: i64 = 10;

/// Represents a user in the database
#[derive(Debug, Clone, Serialize)]
pub struct AuthenticatedUser {
    pub id: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
    pub is_admin: bool,
}

impl From<&User> for AuthenticatedUser {
    fn from(user: &User) -> Self {
        Self {
            id: user.id.clone(),
            email: user.email.clone(),
            created_at: user.created_at,
            is_admin: user.is_admin,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CurrentUser(pub User);

impl CurrentUser {
    pub fn viewer(&self) -> Option<&str> {
        Some(self.0.id.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct MaybeUser(pub Option<User>);

/// Like `CurrentUser`, but also requires `auth::require_admin` to pass. A route taking this
/// instead of `CurrentUser` is, by its signature alone, admin-only — same reasoning as why
/// every data route already takes `CurrentUser` rather than relying on a middleware list.
#[derive(Debug, Clone)]
pub struct RequireAdmin(pub User);

/// Uses the current cookie to validate the session
async fn user_from_jar(pool: &SqlitePool, jar: &CookieJar) -> Result<Option<User>, AuthError> {
    let Some(cookie) = jar.get(SESSION_COOKIE_NAME) else {
        return Ok(None);
    };

    auth::validate_session(pool, cookie.value()).await
}

// Builds a user using header data in the cookie
impl<S> FromRequestParts<S> for MaybeUser
where
    SqlitePool: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let pool = SqlitePool::from_ref(state);
        let jar = CookieJar::from_headers(&parts.headers);

        if let Some(user) = user_from_jar(&pool, &jar).await? {
            return Ok(MaybeUser(Some(user)));
        }

        // Then a bearer token. The cookie is still the primary credential and is tried first;
        // this exists because it cannot reach the Firefox extension — `SameSite=Lax` means a
        // request from a `moz-extension://` page never carries it. See `hunt::tokens`.
        //
        // Adding it HERE rather than on the hunt routes is what keeps it one auth system: a
        // route's signature still says `CurrentUser` and no route knows tokens exist. It also
        // means the token is exactly as powerful as a session and no more, which is the
        // property to preserve if it ever grows a scope.
        Ok(MaybeUser(user_from_bearer(&pool, parts).await))
    }
}

/// The user named by an `Authorization: Bearer` header, if there is a live token for it.
///
/// Returns `None` for every failure — malformed header, unknown token, revoked token, or a
/// database error. A credential that does not check out is an anonymous request, not a 500,
/// and the distinction between "no token" and "bad token" is not one we owe the caller.
async fn user_from_bearer(pool: &SqlitePool, parts: &Parts) -> Option<User> {
    let header = parts.headers.get(axum::http::header::AUTHORIZATION)?;
    let value = header.to_str().ok()?;
    let secret = value.strip_prefix("Bearer ").or_else(|| value.strip_prefix("bearer "))?;

    match crate::hunt::tokens::validate(pool, secret.trim(), Utc::now()).await {
        Ok(user) => user,
        Err(err) => {
            eprintln!("auth: validating a hunt token failed: {err:?}");
            None
        }
    }
}

// Error rather None version of MaybeUser
impl<S> FromRequestParts<S> for CurrentUser
where
    SqlitePool: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let MaybeUser(user) = MaybeUser::from_request_parts(parts, state).await?;

        user.map(CurrentUser).ok_or(AuthError::InvalidCredentials)
    }
}

impl<S> FromRequestParts<S> for RequireAdmin
where
    SqlitePool: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let CurrentUser(user) = CurrentUser::from_request_parts(parts, state).await?;
        auth::require_admin(&user)?;
        Ok(RequireAdmin(user))
    }
}

/// Checks if the cookie is secure
fn cookie_secure() -> bool {
    std::env::var("COOKIE_SECURE").is_ok_and(|v| v == "true" || v == "1")
}

/// Builds the cookie for your session
fn session_cookie(token: String) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE_NAME, token))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(cookie_secure())
        .path("/")
        .max_age(time::Duration::days(SESSION_DURATION_DAYS))
        .build()
}

/// Deletes a cookie by setting the exact identity to an expired time
fn expired_session_cookie() -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE_NAME, ""))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(cookie_secure())
        .path("/")
        .max_age(time::Duration::ZERO)
        .build()
}

/// Builds the cookie for the Google flow
fn oauth_state_cookie(state: String) -> Cookie<'static> {
    Cookie::build((GOOGLE_STATE_COOKIE_NAME, state))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(cookie_secure())
        .path("/")
        .max_age(time::Duration::minutes(OAUTH_STATE_COOKIE_MINUTES))
        .build()
}

/// Deletes the cookie for the Google flow
fn expired_oauth_state_cookie() -> Cookie<'static> {
    Cookie::build((GOOGLE_STATE_COOKIE_NAME, ""))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(cookie_secure())
        .path("/")
        .max_age(time::Duration::ZERO)
        .build()
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
}

/// Outlines the different possible responses for an AuthError
impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, error) = match &self {
            AuthError::InvalidCredentials => {
                (StatusCode::UNAUTHORIZED, "Invalid email or password")
            }
            AuthError::EmailAlreadyRegistered => (
                StatusCode::CONFLICT,
                "An account with that email already exists",
            ),
            AuthError::InvalidInput(message) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorBody { error: message }))
                    .into_response();
            }
            AuthError::OAuthStateMismatch => (
                StatusCode::BAD_REQUEST,
                "Sign-in request could not be verified. Please start again.",
            ),
            AuthError::OAuthExchangeFailed(detail) => {
                eprintln!("google oauth exchange failed: {detail}");
                (StatusCode::BAD_GATEWAY, "Could not complete Google sign-in")
            }
            AuthError::OAuthNotConfigured => (
                StatusCode::NOT_IMPLEMENTED,
                "Google sign-in is not configured on this server",
            ),
            AuthError::Database(err) => {
                eprintln!("auth database error: {err}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong")
            }
            AuthError::Hashing(detail) => {
                eprintln!("auth hashing error: {detail}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong")
            }
            AuthError::NotImplemented(what) => {
                eprintln!("auth: {what} is not implemented yet (see src/auth.rs)");
                (StatusCode::NOT_IMPLEMENTED, "This is not implemented yet")
            }
            AuthError::Forbidden => (
                StatusCode::FORBIDDEN,
                "You don't have permission to do that",
            ),
        };

        (status, Json(ErrorBody { error })).into_response()
    }
}

/// Sanity checking to make sure the password is valid
fn validate_password(password: &str) -> Result<(), AuthError> {
    if password.len() < MIN_PASSWORD_LENGTH {
        return Err(AuthError::InvalidInput("Password is too short"));
    }
    if password.len() > MAX_PASSWORD_LENGTH {
        return Err(AuthError::InvalidInput("Password is too long"));
    }
    Ok(())
}

/// Makes sure that the email looks like a valid email address
fn validate_email(email: &str) -> Result<(), AuthError> {
    let mut parts = email.split('@');
    let valid = matches!((parts.next(), parts.next(), parts.next()), (Some(local), Some(domain), None)
        if !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.'));

    if valid {
        Ok(())
    } else {
        Err(AuthError::InvalidInput(
            "That doesn't look like an email address",
        ))
    }
}

/// This is the sign in function. Given a register request, the email and password
/// are verified to be valid and normalized. Then the password is hashed and the
/// pair is assigne to a database entry with a unique id.
pub async fn register(
    State(pool): State<SqlitePool>,
    jar: CookieJar,
    Json(req): Json<RegisterRequest>,
) -> Result<(CookieJar, (StatusCode, Json<AuthenticatedUser>)), AuthError> {
    let email = normalize_email(&req.email);
    validate_email(&email)?;
    validate_password(&req.password)?;

    let password_hash = auth::hash_password(&req.password)?;

    let id = Uuid::new_v4().to_string();
    let created_at = Utc::now();

    let mut tx = pool.begin().await?;

    let insert =
        sqlx::query("INSERT INTO users (id, email, password_hash, created_at) VALUES (?, ?, ?, ?)")
            .bind(&id)
            .bind(&email)
            .bind(&password_hash)
            .bind(created_at)
            .execute(&mut *tx)
            .await;

    if let Err(err) = insert {
        return Err(match &err {
            sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
                AuthError::EmailAlreadyRegistered
            }
            _ => AuthError::Database(err),
        });
    }

    claim_if_first_account(&mut tx, &id, &email).await?;

    tx.commit().await?;

    let session = auth::issue_session(&pool, &id).await?;

    let user = User {
        id,
        email,
        password_hash: Some(password_hash),
        created_at,
        is_admin: false,
    };

    Ok((
        jar.add(session_cookie(session.token)),
        (StatusCode::CREATED, Json(AuthenticatedUser::from(&user))),
    ))
}

/// Ensures that the first account associated with this app claims all of the
/// miscellaneous testing data already present in the database.
async fn claim_if_first_account(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: &str,
    email: &str,
) -> Result<(), AuthError> {
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&mut **tx)
        .await?;

    if user_count == 1 {
        let claimed = claim_unowned_rows(tx, user_id).await?;
        println!("first account created — claimed {claimed} pre-auth rows for {email}");
    }

    Ok(())
}

/// Updates all the current unclaimed rows in Reviews, Fridge Items, Shopping List,
/// and Purchase history to belong to the input user.
async fn claim_unowned_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: &str,
) -> Result<u64, AuthError> {
    let mut claimed = 0;

    for table in [
        "reviews",
        "fridge_items",
        "shopping_list_items",
        "purchase_history",
    ] {
        let result = sqlx::query(&format!(
            "UPDATE {table} SET user_id = ? WHERE user_id IS NULL"
        ))
        .bind(user_id)
        .execute(&mut **tx)
        .await?;

        claimed += result.rows_affected();
    }

    Ok(claimed)
}

/// An attempted login attempt. Attempts to match the email to an existing entry.
/// If found, hashes the input password and compares it to the associated hashed
/// password on record. Finally issues a session when the verification is complete.
pub async fn login(
    State(pool): State<SqlitePool>,
    jar: CookieJar,
    Json(req): Json<LoginRequest>,
) -> Result<(CookieJar, Json<AuthenticatedUser>), AuthError> {
    let email = normalize_email(&req.email);

    let user: Option<User> = sqlx::query_as::<_, User>(
        "SELECT id, email, password_hash, created_at, is_admin FROM users WHERE email = ?",
    )
    .bind(&email)
    .fetch_optional(&pool)
    .await?;

    let user = user.ok_or(AuthError::InvalidCredentials)?;
    let stored_hash = user
        .password_hash
        .as_deref()
        .ok_or(AuthError::InvalidCredentials)?;

    if !auth::verify_password(&req.password, stored_hash)? {
        return Err(AuthError::InvalidCredentials);
    }

    let session = auth::issue_session(&pool, &user.id).await?;

    Ok((
        jar.add(session_cookie(session.token)),
        Json(AuthenticatedUser::from(&user)),
    ))
}

/// The session is revoked and the entry in the session table is removed.
pub async fn logout(
    State(pool): State<SqlitePool>,
    jar: CookieJar,
) -> Result<(CookieJar, StatusCode), AuthError> {
    if let Some(cookie) = jar.get(SESSION_COOKIE_NAME) {
        auth::revoke_session(&pool, cookie.value()).await?;
    }

    Ok((jar.add(expired_session_cookie()), StatusCode::NO_CONTENT))
}

pub async fn me(MaybeUser(user): MaybeUser) -> Json<Option<AuthenticatedUser>> {
    Json(user.as_ref().map(AuthenticatedUser::from))
}

fn frontend_origin() -> String {
    std::env::var("FRONTEND_ORIGIN").unwrap_or_else(|_| "http://localhost:3000".to_string())
}

/// Sets up the config to connect with Google auth
pub async fn google_start(
    State(config): State<Option<GoogleOAuthConfig>>,
    jar: CookieJar,
) -> Result<(CookieJar, Redirect), AuthError> {
    let config = config.ok_or(AuthError::OAuthNotConfigured)?;

    let state = auth::generate_oauth_state();
    let url = auth::google_authorize_url(&config, &state);

    Ok((jar.add(oauth_state_cookie(state)), Redirect::to(&url)))
}

// Fields for the redirect
#[derive(Debug, Deserialize)]
pub struct GoogleCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// Handles the redirect that Google gives
pub async fn google_callback(
    State(pool): State<SqlitePool>,
    State(config): State<Option<GoogleOAuthConfig>>,
    jar: CookieJar,
    Query(query): Query<GoogleCallbackQuery>,
    MaybeUser(current_user): MaybeUser,
) -> Result<(CookieJar, Redirect), AuthError> {
    let config = config.ok_or(AuthError::OAuthNotConfigured)?;

    // Record the cookie
    let expected = jar
        .get(GOOGLE_STATE_COOKIE_NAME)
        .map(|c| c.value().to_string());

    // Delete the cookie
    let jar = jar.add(expired_oauth_state_cookie());

    if query.error.is_some() {
        return Ok((
            jar,
            Redirect::to(&format!("{}/login?error=google", frontend_origin())),
        ));
    }

    // Checks that both the sent state and the received state are the same
    match (query.state.as_deref(), expected.as_deref()) {
        (Some(received), Some(stored)) if received == stored && !stored.is_empty() => {}
        _ => return Err(AuthError::OAuthStateMismatch),
    }

    // Ensures the code exists
    let code = query.code.ok_or(AuthError::OAuthStateMismatch)?;
    let identity = auth::exchange_google_code(&config, &code).await?;

    let user_id = resolve_google_identity(&pool, &identity, current_user.as_ref()).await?;
    let session = auth::issue_session(&pool, &user_id).await?;

    Ok((
        jar.add(session_cookie(session.token)),
        Redirect::to(&format!("{}/fridge", frontend_origin())),
    ))
}

/// Links a Google account with an account on the app.
async fn resolve_google_identity(
    pool: &SqlitePool,
    identity: &auth::GoogleIdentity,
    current_user: Option<&User>,
) -> Result<String, AuthError> {
    // Google account already linked
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT user_id FROM oauth_identities WHERE provider = 'google' AND provider_account_id = ?",
    )
    .bind(&identity.subject)
    .fetch_optional(pool)
    .await?;

    if let Some(user_id) = existing {
        return Ok(user_id);
    }

    // Checks if the user is already logged in and links the Google account to it
    if let Some(user) = current_user {
        link_google_identity(pool, &user.id, identity).await?;
        return Ok(user.id.clone());
    }

    let email = normalize_email(&identity.email);
    // Verify that Google has verified the email identity
    if !identity.email_verified {
        return Err(AuthError::InvalidCredentials);
    }

    let existing_account: Option<String> =
        sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
            .bind(&email)
            .fetch_optional(pool)
            .await?;

    // If a Google linking is attempted with an already existent account, error
    if existing_account.is_some() {
        return Err(AuthError::InvalidCredentials);
    }

    // Create the account in the database

    let user_id = Uuid::new_v4().to_string();
    let created_at = Utc::now();

    let mut tx = pool.begin().await?;

    sqlx::query("INSERT INTO users (id, email, password_hash, created_at) VALUES (?, ?, NULL, ?)")
        .bind(&user_id)
        .bind(&email)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "INSERT INTO oauth_identities (id, user_id, provider, provider_account_id, created_at) \
         VALUES (?, ?, 'google', ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&user_id)
    .bind(&identity.subject)
    .bind(created_at)
    .execute(&mut *tx)
    .await?;

    claim_if_first_account(&mut tx, &user_id, &email).await?;

    tx.commit().await?;

    Ok(user_id)
}

// Links the oauth identity to the present account
async fn link_google_identity(
    pool: &SqlitePool,
    user_id: &str,
    identity: &auth::GoogleIdentity,
) -> Result<(), AuthError> {
    sqlx::query(
        "INSERT INTO oauth_identities (id, user_id, provider, provider_account_id, created_at) \
         VALUES (?, ?, 'google', ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(user_id)
    .bind(&identity.subject)
    .bind(Utc::now())
    .execute(pool)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_password_is_rejected() {
        let short = "a".repeat(MIN_PASSWORD_LENGTH - 1);

        assert!(matches!(
            validate_password(&short),
            Err(AuthError::InvalidInput(_))
        ));
    }

    #[test]
    fn a_password_at_the_minimum_length_is_accepted() {
        let exact = "a".repeat(MIN_PASSWORD_LENGTH);

        assert!(validate_password(&exact).is_ok());
    }

    #[test]
    fn an_absurdly_long_password_is_rejected() {
        let long = "a".repeat(MAX_PASSWORD_LENGTH + 1);

        assert!(validate_password(&long).is_err());
    }

    #[test]
    fn obvious_email_typos_are_rejected() {
        for bad in [
            "",
            "no-at-sign",
            "@nolocal.com",
            "trailing@dot.",
            "nodot@domain",
            "two@at@signs.com",
        ] {
            assert!(
                validate_email(bad).is_err(),
                "{bad:?} should have been rejected"
            );
        }
    }

    #[test]
    fn ordinary_addresses_are_accepted() {
        for good in [
            "jesse@example.com",
            "first.last+tag@sub.domain.co.uk",
            "x@y.io",
        ] {
            assert!(
                validate_email(good).is_ok(),
                "{good:?} should have been accepted"
            );
        }
    }

    #[test]
    fn the_session_cookie_is_not_readable_by_javascript() {
        let cookie = session_cookie("token".to_string());

        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.path(), Some("/"));
    }

    #[test]
    fn the_logout_cookie_matches_the_session_cookie_it_replaces() {
        let session = session_cookie("token".to_string());
        let expired = expired_session_cookie();

        assert_eq!(session.name(), expired.name());
        assert_eq!(session.path(), expired.path());
        assert_eq!(expired.value(), "");
        assert_eq!(expired.max_age(), Some(time::Duration::ZERO));
    }

    #[test]
    fn the_oauth_state_cookie_is_lax_so_googles_redirect_carries_it() {
        let cookie = oauth_state_cookie("state".to_string());

        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.http_only(), Some(true));
    }
}
