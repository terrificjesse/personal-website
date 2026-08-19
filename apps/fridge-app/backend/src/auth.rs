use serde::Deserialize;

use argon2::{
    PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{self, SaltString, rand_core::OsRng},
};
use chrono::{DateTime, Duration, Utc};
use rand::fill;
use reqwest::Url;
use sha2::Digest;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::models::User;

/// Cookie for creating a session
pub const SESSION_COOKIE_NAME: &str = "fridge_session";

/// Cookie for communicating with the Google auth
pub const GOOGLE_STATE_COOKIE_NAME: &str = "fridge_oauth_state";

pub const SESSION_DURATION_DAYS: i64 = 30;

pub const MIN_PASSWORD_LENGTH: usize = 12;

pub const MAX_PASSWORD_LENGTH: usize = 1_024;

/// AuthError potential errors
#[derive(Debug)]
pub enum AuthError {
    InvalidCredentials,
    EmailAlreadyRegistered,
    InvalidInput(&'static str),
    OAuthStateMismatch,
    OAuthExchangeFailed(String),
    OAuthNotConfigured,
    Database(sqlx::Error),
    Hashing(String),
    NotImplemented(&'static str),
    /// The requester is authenticated but not allowed to do this — see `require_admin`.
    Forbidden,
}

impl From<sqlx::Error> for AuthError {
    fn from(err: sqlx::Error) -> Self {
        AuthError::Database(err)
    }
}

/// Hashing Error
impl From<password_hash::Error> for AuthError {
    fn from(err: password_hash::Error) -> Self {
        AuthError::Hashing(err.to_string())
    }
}
/// Uses the argon2 hash to generate a hash from the input password. This hashing
/// function is fairly computationally heavy to prevent bruteforcing the password
pub fn hash_password(plaintext: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(argon2::Argon2::default()
        .hash_password(plaintext.as_bytes(), &salt)?
        .to_string())
}

/// Checks that the hashed input password is equivalent to existing hashed password
pub fn verify_password(plaintext: &str, phc_hash: &str) -> Result<bool, AuthError> {
    let parsed = PasswordHash::new(phc_hash)?;
    match argon2::Argon2::default().verify_password(plaintext.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(password_hash::Error::Password) => Ok(false),
        Err(err) => Err(err.into()),
    }
}

/// Decides whether `user` may access admin-only routes (currently: the blog editor).
/// Called by the `RequireAdmin` extractor (`routes/auth.rs`) on every admin route, so this is
/// the single place that decision is made.
pub fn require_admin(user: &User) -> Result<(), AuthError> {
    if !user.is_admin {
        return Err(AuthError::Forbidden);
    }
    Ok(())
}

/// Issues a session to a user
#[derive(Debug, Clone)]
pub struct IssuedSession {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Generates a random session token. Repeats are not a concern since the sample
/// space is so large. They will be caught in a different function regardless
pub fn generate_session_token() -> String {
    let mut random: [u8; 32] = [0; 32];
    fill(&mut random);
    hex::encode(random)
}

/// Hashes the input random token. Uses Sha256, a fast hashing function since
/// only the backend issues the session tokens.
pub fn session_token_hash(token: &str) -> String {
    let mut hash = sha2::Sha256::default();
    hash.update(token.as_bytes());
    hex::encode(hash.finalize())
}

/// Generates a new entry in the session table for a user
pub async fn issue_session(pool: &SqlitePool, user_id: &str) -> Result<IssuedSession, AuthError> {
    let now = Utc::now();
    let expires_at = session_expiry_from(now);
    let token = generate_session_token();
    let hash = session_token_hash(&token);
    sqlx::query("INSERT INTO sessions (id, user_id, token_hash, created_at, expires_at) VALUES (?, ?, ?, ?, ?)")
        .bind(Uuid::new_v4().to_string())
        .bind(user_id)
        .bind(hash)
        .bind(now)
        .bind(expires_at)
        .execute(pool)
        .await?;
    Ok(IssuedSession { token, expires_at })
}

/// Checks the current session under the user and token_hash both exists and
/// isn't expired
pub async fn validate_session(pool: &SqlitePool, token: &str) -> Result<Option<User>, AuthError> {
    let now = Utc::now();
    let token_hash = session_token_hash(token);
    Ok(sqlx::query_as::<_, User>(
        "SELECT u.id, u.email, u.password_hash, u.created_at, u.is_admin
    FROM sessions s JOIN users u ON u.id = s.user_id
    WHERE s.token_hash = ? AND s.expires_at > ?",
    )
    .bind(token_hash)
    .bind(now)
    .fetch_optional(pool)
    .await?)
}

/// Deletes a session entry from the session table
pub async fn revoke_session(pool: &SqlitePool, token: &str) -> Result<(), AuthError> {
    let token_hash = session_token_hash(token);
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(token_hash)
        .execute(pool)
        .await?;
    Ok(())
}

/// Purges all of the sections that have expired from the session table
pub async fn purge_expired_sessions(pool: &SqlitePool) -> Result<u64, AuthError> {
    let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(Utc::now())
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Generates the expiration time
pub fn session_expiry_from(now: DateTime<Utc>) -> DateTime<Utc> {
    now + Duration::days(SESSION_DURATION_DAYS)
}

/// Struct with the connection requirements for GoogleOAuth
#[derive(Debug, Clone)]
pub struct GoogleOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

impl GoogleOAuthConfig {
    /// Draws the info from the .env file
    pub fn from_env() -> Option<Self> {
        Some(Self {
            client_id: std::env::var("GOOGLE_CLIENT_ID").ok()?,
            client_secret: std::env::var("GOOGLE_CLIENT_SECRET").ok()?,
            redirect_uri: std::env::var("GOOGLE_REDIRECT_URI").ok()?,
        })
    }
}

/// Struct for the Google Identity
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleIdentity {
    pub subject: String,
    pub email: String,
    pub email_verified: bool,
}

/// Constructs the url to redirect the user towards so that they can sign in
pub fn google_authorize_url(config: &GoogleOAuthConfig, state: &str) -> String {
    let base = String::from("https://accounts.google.com/o/oauth2/v2/auth");
    let url = Url::parse_with_params(
        &base,
        [
            ("client_id", &config.client_id),
            ("redirect_uri", &config.redirect_uri),
            ("response_type", &"code".to_string()),
            ("scope", &"openid email".to_string()),
            ("state", &state.to_string()),
        ],
    );
    url.expect("Bad URL").to_string()
}

impl From<reqwest::Error> for AuthError {
    fn from(err: reqwest::Error) -> Self {
        AuthError::OAuthExchangeFailed(err.to_string())
    }
}

#[derive(Deserialize)]
struct TokenData {
    access_token: String,
}
#[derive(Deserialize)]
struct UserData {
    sub: String,
    email: String,
    email_verified: bool,
}

/// First query the Google access token API with a post request to receive a Google
/// access token. Use this access token to query the user API to receive user data.
pub async fn exchange_google_code(
    config: &GoogleOAuthConfig,
    code: &str,
) -> Result<GoogleIdentity, AuthError> {
    let client = reqwest::Client::new();

    let params = [
        ("client_id", &config.client_id),
        ("redirect_uri", &config.redirect_uri),
        ("client_secret", &config.client_secret),
        ("grant_type", &"authorization_code".to_string()),
        ("code", &code.to_string()),
    ];

    let res = client
        .post("https://oauth2.googleapis.com/token")
        .form(&params)
        .send()
        .await?;
    if !res.status().is_success() {
        let msg = res.text().await?;
        return Err(AuthError::OAuthExchangeFailed(msg));
    };

    let data: TokenData = res.json().await?;

    let res = client
        .get("https://openidconnect.googleapis.com/v1/userinfo")
        .bearer_auth(data.access_token)
        .send()
        .await?;

    if !res.status().is_success() {
        let msg = res.text().await?;
        return Err(AuthError::OAuthExchangeFailed(msg));
    };

    let user_data = res.json::<UserData>().await?;

    Ok(GoogleIdentity {
        subject: user_data.sub,
        email: user_data.email,
        email_verified: user_data.email_verified,
    })
}

pub fn generate_oauth_state() -> String {
    let mut random = [0; 32];
    fill(&mut random);
    hex::encode(random)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "correct horse battery staple";

    #[test]
    fn a_password_hash_is_not_the_password() {
        let hash = hash_password(PASSWORD).expect("hashing should succeed");

        assert_ne!(hash, PASSWORD);
        assert!(
            !hash.contains(PASSWORD),
            "the plaintext must not be recoverable from the stored hash"
        );
    }

    #[test]
    fn the_same_password_hashes_differently_every_time() {
        let first = hash_password(PASSWORD).expect("hashing should succeed");
        let second = hash_password(PASSWORD).expect("hashing should succeed");

        assert_ne!(
            first, second,
            "each hash needs its own random salt, so two hashes of one password must differ"
        );
    }

    #[test]
    fn a_correct_password_verifies_against_its_own_hash() {
        let hash = hash_password(PASSWORD).expect("hashing should succeed");

        assert!(verify_password(PASSWORD, &hash).expect("verification should not error"));
    }

    #[test]
    fn a_wrong_password_does_not_verify() {
        let hash = hash_password(PASSWORD).expect("hashing should succeed");

        assert!(!verify_password("wrong password entirely", &hash).unwrap());
        assert!(!verify_password("correct horse battery stapl", &hash).unwrap());
        assert!(!verify_password("", &hash).unwrap());
    }

    #[test]
    fn verifying_against_a_hash_of_a_different_password_fails() {
        let hash = hash_password("a completely different password").unwrap();

        assert!(!verify_password(PASSWORD, &hash).unwrap());
    }

    #[test]
    fn a_wrong_password_is_a_false_not_an_error() {
        let hash = hash_password(PASSWORD).unwrap();

        assert!(matches!(verify_password("nope", &hash), Ok(false)));
    }

    #[test]
    fn a_corrupt_stored_hash_is_an_error_not_a_silent_false() {
        assert!(verify_password(PASSWORD, "not a PHC string at all").is_err());
    }

    #[test]
    fn session_tokens_are_unique_across_calls() {
        let first = generate_session_token();
        let second = generate_session_token();

        assert_ne!(first, second);
    }

    #[test]
    fn a_session_token_is_long_enough_to_be_unguessable() {
        let token = generate_session_token();

        assert!(
            token.len() >= 32,
            "session token {token:?} is only {} chars — see the module doc on entropy",
            token.len()
        );
    }

    #[test]
    fn a_session_token_survives_a_cookie_value_unescaped() {
        let token = generate_session_token();

        assert!(
            token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "token {token:?} contains characters that need escaping in a cookie value"
        );
    }

    #[test]
    fn hashing_a_session_token_is_deterministic() {
        let token = "a-fixed-token-value-for-this-test";

        assert_eq!(session_token_hash(token), session_token_hash(token));
    }

    #[test]
    fn different_session_tokens_hash_differently() {
        assert_ne!(
            session_token_hash("token-one"),
            session_token_hash("token-two")
        );
    }

    #[test]
    fn a_session_token_hash_is_not_the_token() {
        let token = "a-fixed-token-value-for-this-test";

        assert_ne!(session_token_hash(token), token);
    }

    #[test]
    fn session_expiry_is_in_the_future_and_matches_the_configured_duration() {
        let now = Utc::now();

        let expiry = session_expiry_from(now);

        assert!(expiry > now);
        assert_eq!((expiry - now).num_days(), SESSION_DURATION_DAYS);
    }

    #[test]
    fn oauth_state_values_are_unique_and_unguessable() {
        let first = generate_oauth_state();
        let second = generate_oauth_state();

        assert_ne!(first, second);
        assert!(
            first.len() >= 32,
            "oauth state {first:?} is too short to be unguessable"
        );
    }

    #[test]
    fn the_authorize_url_points_at_google_and_carries_the_state() {
        let config = GoogleOAuthConfig {
            client_id: "test-client-id".to_string(),
            client_secret: "test-client-secret".to_string(),
            redirect_uri: "http://127.0.0.1:8080/auth/google/callback".to_string(),
        };

        let url = google_authorize_url(&config, "test-state-value");

        assert!(
            url.starts_with("https://"),
            "the authorize URL must be https"
        );
        assert!(url.contains("accounts.google.com"));
        assert!(url.contains("test-client-id"));
        assert!(
            url.contains("test-state-value"),
            "state must reach Google or CSRF checking is theatre"
        );
        assert!(url.contains("response_type=code"));
        assert!(
            !url.contains("test-client-secret"),
            "the client secret must never appear in a URL the browser visits"
        );
    }

    #[test]
    fn the_authorize_url_percent_encodes_its_parameters() {
        let config = GoogleOAuthConfig {
            client_id: "test-client-id".to_string(),
            client_secret: "test-client-secret".to_string(),
            redirect_uri: "http://127.0.0.1:8080/auth/google/callback".to_string(),
        };

        let url = google_authorize_url(&config, "state");

        assert!(
            !url.contains("redirect_uri=http://"),
            "redirect_uri must be percent-encoded, not spliced in raw"
        );
    }

    use sqlx::sqlite::SqlitePoolOptions;

    const TEST_USER_ID: &str = "test-user-1";

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory database should open");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations should apply");

        sqlx::query("INSERT INTO users (id, email, password_hash, created_at) VALUES (?, ?, ?, ?)")
            .bind(TEST_USER_ID)
            .bind("jesse@example.com")
            .bind("$argon2id$not-a-real-hash")
            .bind(Utc::now())
            .execute(&pool)
            .await
            .expect("test user should insert");

        pool
    }

    async fn insert_session_expiring(pool: &SqlitePool, token: &str, expires_at: DateTime<Utc>) {
        sqlx::query(
            "INSERT INTO sessions (id, user_id, token_hash, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(TEST_USER_ID)
        .bind(session_token_hash(token))
        .bind(Utc::now())
        .bind(expires_at)
        .execute(pool)
        .await
        .expect("session row should insert");
    }

    #[tokio::test]
    async fn a_freshly_issued_session_validates_to_its_own_user() {
        let pool = test_pool().await;

        let session = issue_session(&pool, TEST_USER_ID).await.expect("issue");
        let user = validate_session(&pool, &session.token)
            .await
            .expect("validate");

        let user = user.expect("a session issued one line ago must validate");
        assert_eq!(user.id, TEST_USER_ID);
        assert_eq!(user.email, "jesse@example.com");
    }

    #[tokio::test]
    async fn validate_returns_the_user_not_the_session() {
        let pool = test_pool().await;

        let session = issue_session(&pool, TEST_USER_ID).await.expect("issue");
        let session_id: String = sqlx::query_scalar("SELECT id FROM sessions LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("session row should exist");

        let user = validate_session(&pool, &session.token)
            .await
            .expect("validate")
            .expect("session should be live");

        assert_eq!(
            user.id, TEST_USER_ID,
            "id must come from users, not sessions"
        );
        assert_ne!(
            user.id, session_id,
            "this is the session's id, not the user's"
        );
    }

    #[tokio::test]
    async fn the_plaintext_token_is_never_stored() {
        let pool = test_pool().await;

        let session = issue_session(&pool, TEST_USER_ID).await.expect("issue");

        let matches: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sessions WHERE id = ?1 OR user_id = ?1 OR token_hash = ?1",
        )
        .bind(&session.token)
        .fetch_one(&pool)
        .await
        .expect("count query");

        assert_eq!(
            matches, 0,
            "the plaintext session token must not appear anywhere in the sessions table"
        );
    }

    #[tokio::test]
    async fn an_expired_session_does_not_validate() {
        let pool = test_pool().await;
        let token = generate_session_token();

        insert_session_expiring(&pool, &token, Utc::now() - Duration::days(1)).await;

        let user = validate_session(&pool, &token).await.expect("validate");

        assert!(
            user.is_none(),
            "an expired session must not authenticate anyone"
        );
    }

    #[tokio::test]
    async fn a_session_expiring_in_the_future_still_validates() {
        let pool = test_pool().await;
        let token = generate_session_token();

        insert_session_expiring(&pool, &token, Utc::now() + Duration::minutes(1)).await;

        let user = validate_session(&pool, &token).await.expect("validate");

        assert!(
            user.is_some(),
            "a session with time left must still authenticate"
        );
    }

    #[tokio::test]
    async fn an_unknown_token_is_none_not_an_error() {
        let pool = test_pool().await;

        let result = validate_session(&pool, "not-a-real-token-at-all").await;

        assert!(
            matches!(result, Ok(None)),
            "an unrecognised token is the ordinary signed-out case, not a failure"
        );
    }

    #[tokio::test]
    async fn a_revoked_session_stops_validating() {
        let pool = test_pool().await;

        let session = issue_session(&pool, TEST_USER_ID).await.expect("issue");
        assert!(
            validate_session(&pool, &session.token)
                .await
                .expect("validate")
                .is_some(),
            "precondition: the session must be live before we revoke it"
        );

        revoke_session(&pool, &session.token).await.expect("revoke");

        let user = validate_session(&pool, &session.token)
            .await
            .expect("validate");
        assert!(user.is_none(), "a revoked session must stop authenticating");
    }

    #[tokio::test]
    async fn revoking_one_session_leaves_the_others_alive() {
        let pool = test_pool().await;

        let laptop = issue_session(&pool, TEST_USER_ID).await.expect("issue");
        let phone = issue_session(&pool, TEST_USER_ID).await.expect("issue");

        revoke_session(&pool, &laptop.token).await.expect("revoke");

        assert!(
            validate_session(&pool, &laptop.token)
                .await
                .expect("validate")
                .is_none(),
            "the revoked session should be gone"
        );
        assert!(
            validate_session(&pool, &phone.token)
                .await
                .expect("validate")
                .is_some(),
            "revoking one session must not end the user's other sessions"
        );
    }

    #[tokio::test]
    async fn revoking_an_unknown_token_succeeds() {
        let pool = test_pool().await;

        assert!(revoke_session(&pool, "never-existed").await.is_ok());
    }

    #[tokio::test]
    async fn purging_removes_expired_sessions_but_not_live_ones() {
        let pool = test_pool().await;
        let stale = generate_session_token();
        let live = generate_session_token();

        insert_session_expiring(&pool, &stale, Utc::now() - Duration::days(1)).await;
        insert_session_expiring(&pool, &live, Utc::now() + Duration::days(1)).await;

        let purged = purge_expired_sessions(&pool).await.expect("purge");

        assert_eq!(
            purged, 1,
            "exactly the expired row should have been deleted"
        );
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(remaining, 1, "the live session must survive the purge");
    }
}
