//! Auth routes and the session extractor — **scaffolding**, not a learning area.
//!
//! The security decisions live next door in `src/auth.rs` and are yours to implement. This
//! file is the plumbing that calls them: request shapes, cookie attributes, status codes,
//! and the `FromRequestParts` impls that turn a cookie into a user.
//!
//! ## This is where `reviews::current_viewer()` went
//!
//! Phase 4 left a seam: `current_viewer()` returned `None` unconditionally, and every read
//! path threaded its result through. `CurrentUser` below is the real extractor that replaces
//! it. The `Option<&str>` viewer parameter still runs through
//! `reviews::fetch_for_viewer` / `fetch_visible_to`, `recipes::liked_recipe_ids` and
//! `rerank::rerank_recommendations` exactly as before — the only change is that handlers now
//! pass `Some(&user.id)` instead of the result of a function that always said `None`.
//!
//! Those functions keep accepting `Option<&str>` rather than `&str` even though every handler
//! now passes `Some`. `Review::is_by(None)` — "no accounts exist, so every review is mine" —
//! is still the documented pre-auth semantics, still unit-tested in `models.rs`, and still
//! what the `rerank.rs` fixtures exercise. Narrowing the type would delete that distinction
//! from the type system for no gain at the call sites.
//!
//! ## Cookies across two origins
//!
//! The frontend (`:3000`) and this backend (`:8080`) are different *origins* but the same
//! *site* — a site is scheme plus registrable domain, and ports are not part of it. That's
//! why `SameSite=Lax` is sufficient here and why the LAN setup works unchanged: both servers
//! sit on one host. It would stop being true if the two were deployed to different domains,
//! which would force `SameSite=None; Secure` and therefore HTTPS on both.
//!
//! CORS still applies, because origins *do* differ. `routes::build_router` sends an explicit
//! allow-list with `allow_credentials(true)`; the wildcard it used through Phase 4 is
//! rejected by browsers for credentialed requests, so cookies would silently never be sent.

use axum::{
    extract::{FromRef, FromRequestParts, Query, State},
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::auth::{
    self, AuthError, GoogleOAuthConfig, GOOGLE_STATE_COOKIE_NAME, MAX_PASSWORD_LENGTH,
    MIN_PASSWORD_LENGTH, SESSION_COOKIE_NAME, SESSION_DURATION_DAYS,
};
use crate::models::{normalize_email, LoginRequest, RegisterRequest, User};

/// How long the OAuth `state` cookie lives. Long enough to sign in at Google, short enough
/// that an abandoned attempt doesn't leave a usable value lying around.
const OAUTH_STATE_COOKIE_MINUTES: i64 = 10;

// ---------------------------------------------------------------------------------------
// Public view of an account
// ---------------------------------------------------------------------------------------

/// What the frontend is allowed to know about the signed-in account.
///
/// A separate type rather than `Serialize` on `User`, so `password_hash` cannot reach a
/// response body by someone adding a field or returning the wrong struct. `User` is not
/// serializable at all — see its doc in `models.rs`.
#[derive(Debug, Clone, Serialize)]
pub struct AuthenticatedUser {
    pub id: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
}

impl From<&User> for AuthenticatedUser {
    fn from(user: &User) -> Self {
        Self {
            id: user.id.clone(),
            email: user.email.clone(),
            created_at: user.created_at,
        }
    }
}

// ---------------------------------------------------------------------------------------
// Extractors — the replacement for `reviews::current_viewer()`
// ---------------------------------------------------------------------------------------

/// A request from a signed-in account. Rejects with 401 when there's no valid session.
///
/// Put this in a handler's argument list and the route is protected — there is no separate
/// middleware to keep in sync, and no way to read a route's signature and be wrong about
/// whether it requires auth.
#[derive(Debug, Clone)]
pub struct CurrentUser(pub User);

impl CurrentUser {
    /// The viewer id to hand the review/ranking functions, which all take `Option<&str>`.
    pub fn viewer(&self) -> Option<&str> {
        Some(self.0.id.as_str())
    }
}

/// A request that may or may not be signed in. For routes that must answer either way —
/// `GET /auth/me` is the only one today, since it's how the frontend asks "am I logged in?"
/// and a 401 there would be an answer, not an error.
#[derive(Debug, Clone)]
pub struct MaybeUser(pub Option<User>);

/// Pulls the session token out of the cookie jar and resolves it.
///
/// One place, so every extractor agrees on what counts as a session. `Ok(None)` covers both
/// "no cookie" and "cookie present but not a live session" — the client can't tell those
/// apart and shouldn't need to.
async fn user_from_jar(pool: &SqlitePool, jar: &CookieJar) -> Result<Option<User>, AuthError> {
    let Some(cookie) = jar.get(SESSION_COOKIE_NAME) else {
        return Ok(None);
    };

    auth::validate_session(pool, cookie.value()).await
}

impl<S> FromRequestParts<S> for MaybeUser
where
    SqlitePool: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let pool = SqlitePool::from_ref(state);
        let jar = CookieJar::from_headers(&parts.headers);

        Ok(MaybeUser(user_from_jar(&pool, &jar).await?))
    }
}

impl<S> FromRequestParts<S> for CurrentUser
where
    SqlitePool: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let MaybeUser(user) = MaybeUser::from_request_parts(parts, state).await?;

        // `InvalidCredentials` maps to 401, which is what the frontend keys off to redirect
        // to the login page. Deliberately not 403: the request isn't forbidden, it's
        // unauthenticated, and the difference is what tells the client to offer a login
        // rather than an apology.
        user.map(CurrentUser).ok_or(AuthError::InvalidCredentials)
    }
}

// ---------------------------------------------------------------------------------------
// Cookies
// ---------------------------------------------------------------------------------------

/// Whether to set `Secure` on the session cookie.
///
/// Env-driven because the honest answer differs by deployment: this app currently serves
/// plain HTTP on a LAN, where a `Secure` cookie would simply never be sent and login would
/// fail with no visible error. Defaults to off for that reason — but it **must** be on
/// anywhere reachable over the internet, which is what the note in `.env.example` says.
fn cookie_secure() -> bool {
    std::env::var("COOKIE_SECURE").is_ok_and(|v| v == "true" || v == "1")
}

/// Builds the session cookie for a freshly issued token.
///
/// - `HttpOnly` — JavaScript can't read it, so an XSS bug can't exfiltrate the session.
/// - `SameSite=Lax` — see the module doc on why Lax is sufficient across `:3000`/`:8080`.
/// - `Path=/` — the fridge endpoints live at the router root, not under a prefix.
/// - `Max-Age` mirrors `sessions.expires_at`. The server-side expiry is the one that counts;
///   this only stops the browser from sending a token that's already dead.
fn session_cookie(token: String) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE_NAME, token))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(cookie_secure())
        .path("/")
        .max_age(time::Duration::days(SESSION_DURATION_DAYS))
        .build()
}

/// A removal cookie for logout: same name and path, empty value, immediate expiry.
///
/// The `Path` has to match the one the cookie was set with or the browser treats it as a
/// different cookie and quietly keeps the original.
fn expired_session_cookie() -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE_NAME, ""))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(cookie_secure())
        .path("/")
        .max_age(time::Duration::ZERO)
        .build()
}

/// The short-lived cookie carrying the OAuth `state` across the redirect to Google.
///
/// `SameSite=Lax` is load-bearing here rather than merely sufficient: Google's callback is a
/// top-level GET navigation from another site, which Lax allows and `Strict` would not. Under
/// `Strict` this cookie would be absent from the very request that needs it, and every OAuth
/// attempt would fail the state check.
fn oauth_state_cookie(state: String) -> Cookie<'static> {
    Cookie::build((GOOGLE_STATE_COOKIE_NAME, state))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(cookie_secure())
        .path("/")
        .max_age(time::Duration::minutes(OAUTH_STATE_COOKIE_MINUTES))
        .build()
}

fn expired_oauth_state_cookie() -> Cookie<'static> {
    Cookie::build((GOOGLE_STATE_COOKIE_NAME, ""))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(cookie_secure())
        .path("/")
        .max_age(time::Duration::ZERO)
        .build()
}

// ---------------------------------------------------------------------------------------
// Error responses
// ---------------------------------------------------------------------------------------

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        // The message is what the user sees. Nothing here distinguishes "unknown email" from
        // "wrong password" — see `AuthError`'s doc on account enumeration. Internal detail
        // (sqlx errors, argon2 errors, which function is unimplemented) is logged, never
        // returned.
        let (status, error) = match &self {
            AuthError::InvalidCredentials => (StatusCode::UNAUTHORIZED, "Invalid email or password"),
            AuthError::EmailAlreadyRegistered => {
                (StatusCode::CONFLICT, "An account with that email already exists")
            }
            AuthError::InvalidInput(message) => return (StatusCode::BAD_REQUEST, Json(ErrorBody { error: message })).into_response(),
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
        };

        (status, Json(ErrorBody { error })).into_response()
    }
}

// ---------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------

/// Validates a submitted password against the length bounds.
///
/// Kept out of `auth::hash_password` so the hasher stays reusable for flows with different
/// rules, and kept out of the handler so it's unit-testable.
fn validate_password(password: &str) -> Result<(), AuthError> {
    if password.len() < MIN_PASSWORD_LENGTH {
        return Err(AuthError::InvalidInput("Password is too short"));
    }
    if password.len() > MAX_PASSWORD_LENGTH {
        return Err(AuthError::InvalidInput("Password is too long"));
    }
    Ok(())
}

/// The laziest email check that catches real typos without rejecting valid addresses.
///
/// Deliberately not a regex: the grammar in RFC 5322 admits addresses that every "email
/// regex" on the internet rejects, and the only real proof an address works is sending mail
/// to it. This exists to catch a missing `@`, nothing more.
fn validate_email(email: &str) -> Result<(), AuthError> {
    let mut parts = email.split('@');
    let valid = matches!((parts.next(), parts.next(), parts.next()), (Some(local), Some(domain), None)
        if !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.'));

    if valid {
        Ok(())
    } else {
        Err(AuthError::InvalidInput("That doesn't look like an email address"))
    }
}

/// `POST /auth/register` — creates an account and signs it in.
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

    // One transaction so that "create the account" and "claim the pre-auth rows" either both
    // happen or neither does. A half-applied registration would leave the first account
    // staring at an empty fridge with no way to re-run the claim.
    let mut tx = pool.begin().await?;

    let insert = sqlx::query(
        "INSERT INTO users (id, email, password_hash, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&email)
    .bind(&password_hash)
    .bind(created_at)
    .execute(&mut *tx)
    .await;

    // The UNIQUE constraint on `users.email` is what actually decides this, rather than a
    // SELECT beforehand — two simultaneous registrations for one address would both pass a
    // pre-check and only the constraint catches the second.
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
    };

    Ok((
        jar.add(session_cookie(session.token)),
        (StatusCode::CREATED, Json(AuthenticatedUser::from(&user))),
    ))
}

/// Runs the pre-auth backfill if `user_id` is the account that just became the *first* one.
///
/// **Call this from every path that creates an account.** There are two — password
/// registration and Google sign-in for an unrecognised identity — and the backfill originally
/// lived only in the first. The consequence was silent and permanent: signing in with Google
/// on a fresh database created the first account, skipped the claim, and stranded every
/// pre-auth row as unclaimed forever, with no UI able to see or recover them.
///
/// Hence one helper called from both sites rather than the check inlined at each. A third
/// account-creation path added later should call this too; that's much easier to notice with
/// a named function than with a `SELECT COUNT(*)` copied into one handler.
///
/// Must run inside the same transaction as the `INSERT INTO users`, so "account created" and
/// "rows claimed" either both happen or neither does.
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

/// Assigns every pre-auth row to `user_id`.
///
/// Rows written before Phase 5 carry `user_id IS NULL` — "unclaimed", not "public" (see
/// `migrations/0006` and `0008`). Every scoped read filters them out, so until this runs the
/// first account sees an empty fridge and an empty review history. Running it at first
/// registration, inside that registration's transaction, is what makes the migration
/// invisible rather than a manual step someone has to remember.
///
/// Only ever runs once: it's gated on there being exactly one account, and it leaves no NULLs
/// behind for a second run to find.
///
/// The `reviews` update is the one PLAN.md calls out by name — without it, `Review::is_by`
/// reports every seeded review as belonging to nobody (`a_pre_auth_review_belongs_to_nobody_once_accounts_exist`
/// in `models.rs` pins that deliberately), and the "Recipes you liked" section would come
/// back empty right after the first login.
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
        // Table names are from this fixed literal array, never from input.
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

// ---------------------------------------------------------------------------------------
// Password login / logout
// ---------------------------------------------------------------------------------------

/// `POST /auth/login`.
pub async fn login(
    State(pool): State<SqlitePool>,
    jar: CookieJar,
    Json(req): Json<LoginRequest>,
) -> Result<(CookieJar, Json<AuthenticatedUser>), AuthError> {
    let email = normalize_email(&req.email);

    let user: Option<User> = sqlx::query_as::<_, User>(
        "SELECT id, email, password_hash, created_at FROM users WHERE email = ?",
    )
    .bind(&email)
    .fetch_optional(&pool)
    .await?;

    // Every failure below returns the same `InvalidCredentials`: unknown email, no password
    // set (a Google-only account), and wrong password are indistinguishable to the client.
    //
    // Note this still leaks a little through *timing* — the unknown-email path skips the
    // Argon2 verification and returns measurably faster. Closing that means verifying against
    // a dummy hash when the user doesn't exist, so both paths do the same work. Worth
    // deciding on deliberately when you implement `verify_password`; it's a real technique
    // with a real cost, not an obvious win.
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

/// `POST /auth/logout`.
///
/// Always succeeds, including when there was no session to begin with — a logout that can
/// fail is a logout the client has to write retry logic around, for no benefit.
pub async fn logout(
    State(pool): State<SqlitePool>,
    jar: CookieJar,
) -> Result<(CookieJar, StatusCode), AuthError> {
    if let Some(cookie) = jar.get(SESSION_COOKIE_NAME) {
        // Server-side revocation first. Clearing the cookie only asks the browser to forget
        // the token; deleting the row is what stops a copied token from working.
        auth::revoke_session(&pool, cookie.value()).await?;
    }

    Ok((jar.add(expired_session_cookie()), StatusCode::NO_CONTENT))
}

/// `GET /auth/me` — who am I, if anyone.
///
/// Uses `MaybeUser`, so being signed out is a 200 with `null` rather than a 401. The frontend
/// calls this on load to decide what to render; an error status for the ordinary
/// not-signed-in case would make every page load look like a failure.
pub async fn me(MaybeUser(user): MaybeUser) -> Json<Option<AuthenticatedUser>> {
    Json(user.as_ref().map(AuthenticatedUser::from))
}

// ---------------------------------------------------------------------------------------
// Google OAuth
// ---------------------------------------------------------------------------------------

/// Where to send the browser after the OAuth callback finishes. The frontend's origin, since
/// the callback lands on the *backend* and the user needs to end up back in the app.
fn frontend_origin() -> String {
    std::env::var("FRONTEND_ORIGIN").unwrap_or_else(|_| "http://localhost:3000".to_string())
}

/// `GET /auth/google/start` — kicks off the flow.
///
/// Works signed-in or signed-out, and the difference decides what the callback does: signed
/// in, it *links* Google to the account you're already using; signed out, it logs you in as
/// whoever the Google identity is already linked to.
pub async fn google_start(
    State(config): State<Option<GoogleOAuthConfig>>,
    jar: CookieJar,
) -> Result<(CookieJar, Redirect), AuthError> {
    let config = config.ok_or(AuthError::OAuthNotConfigured)?;

    let state = auth::generate_oauth_state();
    let url = auth::google_authorize_url(&config, &state);

    // The state goes in a cookie, and the callback compares the two. That comparison is the
    // whole CSRF defense: without it, anyone can feed this server a `code` of their choosing.
    Ok((jar.add(oauth_state_cookie(state)), Redirect::to(&url)))
}

#[derive(Debug, Deserialize)]
pub struct GoogleCallbackQuery {
    /// Absent when the user declined at Google's consent screen, which arrives here as
    /// `?error=access_denied` instead.
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// `GET /auth/google/callback` — Google redirects the browser here.
///
/// Redirects back into the frontend rather than returning JSON: this is a top-level browser
/// navigation, not a fetch, so the user has to end up looking at a page.
pub async fn google_callback(
    State(pool): State<SqlitePool>,
    State(config): State<Option<GoogleOAuthConfig>>,
    jar: CookieJar,
    Query(query): Query<GoogleCallbackQuery>,
    MaybeUser(current_user): MaybeUser,
) -> Result<(CookieJar, Redirect), AuthError> {
    let config = config.ok_or(AuthError::OAuthNotConfigured)?;

    // Read the incoming state **before** clearing it. `CookieJar::add` inserts into the same
    // map `get` reads from, so adding the removal cookie first would shadow the browser's
    // value with the empty one — `get` would return `Some("")` and every callback would fail
    // the check below. Capture, then clear.
    let expected = jar
        .get(GOOGLE_STATE_COOKIE_NAME)
        .map(|c| c.value().to_string());

    // The state cookie is spent either way — success, failure, or the user backing out.
    let jar = jar.add(expired_oauth_state_cookie());

    if query.error.is_some() {
        return Ok((jar, Redirect::to(&format!("{}/login?error=google", frontend_origin()))));
    }

    // Compare before touching anything else. A callback whose state doesn't match the cookie
    // is not a failed sign-in to retry — it's a request this server never started.
    match (query.state.as_deref(), expected.as_deref()) {
        (Some(received), Some(stored)) if received == stored && !stored.is_empty() => {}
        _ => return Err(AuthError::OAuthStateMismatch),
    }

    let code = query.code.ok_or(AuthError::OAuthStateMismatch)?;
    let identity = auth::exchange_google_code(&config, &code).await?;

    let user_id = resolve_google_identity(&pool, &identity, current_user.as_ref()).await?;
    let session = auth::issue_session(&pool, &user_id).await?;

    Ok((
        jar.add(session_cookie(session.token)),
        Redirect::to(&format!("{}/fridge", frontend_origin())),
    ))
}

/// Maps a verified Google identity onto a local account, creating or linking as needed.
///
/// The three cases, and the one refusal:
///
/// 1. **Identity already linked** → that account. (If someone is signed in as a *different*
///    account, the link wins; the identity is the stronger claim.)
/// 2. **Unknown identity, signed in** → link it to the current account. This is the
///    "connect Google as an alternate login method" path from PLAN.md's checkpoint.
/// 3. **Unknown identity, signed out, email not registered** → create a new account with no
///    password (`password_hash` NULL). They can set one later.
/// 4. **Unknown identity, signed out, email already registered** → **refuse**. This is the
///    one that matters: auto-linking here would let anyone who can get Google to assert an
///    address take over the local account holding it. Requiring the user to sign in with
///    their password first, *then* connect Google, means the link is authorized by someone
///    who already controls the account.
///
/// The refusal in (4) is a security policy rather than plumbing, so it's worth your review
/// even though it's scaffolding — the alternative (link on matching email) is the standard
/// shape of an OAuth account-takeover bug.
async fn resolve_google_identity(
    pool: &SqlitePool,
    identity: &auth::GoogleIdentity,
    current_user: Option<&User>,
) -> Result<String, AuthError> {
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT user_id FROM oauth_identities WHERE provider = 'google' AND provider_account_id = ?",
    )
    .bind(&identity.subject)
    .fetch_optional(pool)
    .await?;

    if let Some(user_id) = existing {
        return Ok(user_id);
    }

    if let Some(user) = current_user {
        link_google_identity(pool, &user.id, identity).await?;
        return Ok(user.id.clone());
    }

    let email = normalize_email(&identity.email);

    // An unverified Google email tells you nothing about who controls the address, so it
    // can't be the basis for creating an account keyed on it.
    if !identity.email_verified {
        return Err(AuthError::InvalidCredentials);
    }

    let existing_account: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind(&email)
        .fetch_optional(pool)
        .await?;

    if existing_account.is_some() {
        // Case (4). Same opaque error as any other failed sign-in — saying "that address has
        // a password account, log in and link it" would confirm the address is registered.
        return Err(AuthError::InvalidCredentials);
    }

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

    // Google sign-in can be what creates the very first account, in which case it owns the
    // pre-auth backfill exactly as password registration would.
    claim_if_first_account(&mut tx, &user_id, &email).await?;

    tx.commit().await?;

    Ok(user_id)
}

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

    // These test the scaffolding's own decisions — input validation and the cookie
    // attributes — not the `[learn]` bodies in `auth.rs`. They pass today.

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
        // Boundary, inclusive — the `>` vs `>=` class of bug that has bitten this project
        // before (see the working-patterns section of apps/fridge-app/CLAUDE.md).
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
        // The property that keeps an XSS bug from turning into a stolen session.
        let cookie = session_cookie("token".to_string());

        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.path(), Some("/"));
    }

    #[test]
    fn the_logout_cookie_matches_the_session_cookie_it_replaces() {
        // Name and path have to match or the browser stores a second cookie and keeps
        // sending the original — a logout that appears to work and doesn't.
        let session = session_cookie("token".to_string());
        let expired = expired_session_cookie();

        assert_eq!(session.name(), expired.name());
        assert_eq!(session.path(), expired.path());
        assert_eq!(expired.value(), "");
        assert_eq!(expired.max_age(), Some(time::Duration::ZERO));
    }

    #[test]
    fn the_oauth_state_cookie_is_lax_so_googles_redirect_carries_it() {
        // `Strict` would withhold this cookie from Google's top-level redirect back here —
        // the one request that needs it — and every sign-in would fail the state check.
        let cookie = oauth_state_cookie("state".to_string());

        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.http_only(), Some(true));
    }
}
