//! Authentication primitives — **flagged as a learning area**, see CLAUDE.md and PLAN.md
//! Phase 5. Every function marked `[learn]` below is yours to implement; this file provides
//! the signatures, the contracts, and the tests that describe them.
//!
//! The request plumbing that *calls* these — route handlers, the session-cookie extractor,
//! per-user query scoping — is scaffolding and lives in `routes/auth.rs`. The split is the
//! same one every other phase used: the decisions are here, the wiring is next door.
//!
//! ## What the placeholders do, and why they differ
//!
//! Placeholder bodies in this file are not uniform, on purpose. Auth is the one subsystem
//! where a placeholder's *failure mode* is itself a security property:
//!
//! - Anything that **grants** access returns the denying value (`Ok(false)`, `Ok(None)`).
//!   Never a panic, never an error that a caller might paper over with `unwrap_or(true)`.
//!   An unimplemented authenticator must reject everyone, not admit everyone.
//! - Anything that **mints** a credential is `todo!()`. Loud and impossible to miss, because
//!   the alternative — a placeholder that returns some fixed string — is a hardcoded
//!   credential, and those have a way of surviving to production.
//!
//! Keep that distinction when you replace them; it is the same reasoning that makes
//! `verify_password` return `Ok(false)` rather than `Err(..)` for a wrong password.
//!
//! ## Reading, before you start
//!
//! **Password hashing.** You want a memory-hard KDF, not a general-purpose hash: Argon2id
//! (the `argon2` crate, already a dependency), scrypt, or bcrypt. The things worth
//! understanding before writing the two functions: why a per-password salt is mandatory and
//! why it's stored *in* the PHC hash string rather than a separate column; why cost
//! parameters live in the hash string too (so they can be raised later without invalidating
//! existing hashes); and why comparing hashes needs a constant-time equality check — which
//! `PasswordVerifier::verify_password` already does for you, so the lesson is knowing that it
//! matters, not hand-rolling it.
//!
//! **Sessions.** The cookie carries an opaque random token; `sessions.token_hash` carries a
//! hash of it (see `migrations/0007`). Worth thinking through: how many bits of entropy make
//! a token unguessable, why the token must come from a CSPRNG (`rand::rngs::OsRng`) rather
//! than the `rand::rng()` used for favorite selection in `rerank.rs`, and why a *fast* hash
//! is correct here even though a slow one is correct for passwords — a 256-bit random token
//! has no dictionary to attack, so the reason to hash it is leak containment, not brute-force
//! resistance.
//!
//! **Google OAuth.** The authorization-code flow, in five steps: redirect the user to
//! Google's authorize endpoint with a `state` you generated; Google redirects back to your
//! `redirect_uri` with `code` and the same `state`; you verify the `state` matches (this is
//! the CSRF defense — skipping it is the classic bug); you exchange `code` server-side for
//! tokens; you read the user's stable id from the result. `state` is carried in a short-lived
//! cookie by `routes/auth.rs` — see `GOOGLE_STATE_COOKIE_NAME`.
//!
//! The one identity trap worth naming in advance: link accounts on Google's `sub` claim, not
//! on email. Emails change and can be reassigned; `sub` is stable and unique forever. Linking
//! on email is how OAuth integrations grow account-takeover bugs.

use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;

use crate::models::User;

/// Name of the cookie carrying the opaque session token.
pub const SESSION_COOKIE_NAME: &str = "fridge_session";

/// Name of the short-lived cookie carrying the OAuth `state` value across the redirect to
/// Google and back. Separate from the session cookie because it is meaningful for one
/// round-trip only and must be cleared as soon as the callback consumes it.
pub const GOOGLE_STATE_COOKIE_NAME: &str = "fridge_oauth_state";

/// How long a newly issued session stays valid.
///
/// Written into `sessions.expires_at` at issue time rather than computed at validation time,
/// so changing this value later affects new sessions only — shortening it must not
/// retroactively kill sessions already issued, and lengthening it must not resurrect expired
/// ones.
pub const SESSION_DURATION_DAYS: i64 = 30;

/// Minimum accepted password length.
///
/// A length floor is the one password rule with good evidence behind it; composition rules
/// ("one number, one symbol") mostly push users toward predictable substitutions. NIST
/// SP 800-63B is the reference if you want to read the reasoning.
pub const MIN_PASSWORD_LENGTH: usize = 12;

/// Upper bound on password length, so an enormous input can't be used to make the KDF burn
/// server CPU on demand. Argon2's cost is dominated by its memory parameter rather than input
/// length, which makes this a cheap belt-and-braces limit rather than the primary defense.
pub const MAX_PASSWORD_LENGTH: usize = 1_024;

/// Everything that can go wrong in this module.
///
/// Note what is deliberately *absent*: there is no `UnknownEmail` variant distinct from
/// `InvalidCredentials`. A login response that distinguishes "no such account" from "wrong
/// password" is an account-enumeration oracle — it tells an attacker which addresses are
/// registered. `routes/auth::login` returns the same error for both, and this enum is shaped
/// so that returning the wrong one is not an option a caller has.
// `dead_code` here and on the items below is scaffolding, not intent: nothing constructs
// these until the `[learn]` bodies exist. Delete each attribute as you start using the thing
// it covers — if any is still needed when you're finished, that's a real signal, not noise.
// Same convention `rerank.rs` used through Phase 4.
#[allow(dead_code)]
#[derive(Debug)]
pub enum AuthError {
    /// Wrong password, unknown email, or an account with no password set (a Google-only
    /// account). One variant for all three, on purpose — see above.
    InvalidCredentials,
    /// Registration hit the `users.email` UNIQUE constraint.
    EmailAlreadyRegistered,
    /// Password failed `MIN_PASSWORD_LENGTH` / `MAX_PASSWORD_LENGTH`, or the email didn't
    /// parse. Carries a message safe to show the user.
    InvalidInput(&'static str),
    /// The OAuth callback's `state` didn't match the one in the cookie, or was missing.
    /// Treated as hostile rather than as a retryable error.
    OAuthStateMismatch,
    /// Google's endpoints were unreachable or returned something unexpected.
    OAuthExchangeFailed(String),
    /// Google OAuth env vars aren't set — see `GoogleOAuthConfig::from_env`.
    OAuthNotConfigured,
    Database(sqlx::Error),
    /// The `argon2` crate's error type, stringified — its `password_hash::Error` isn't
    /// `std::error::Error` in a way that composes cleanly here.
    Hashing(String),
    /// A `[learn]` function in this module hasn't been implemented yet.
    NotImplemented(&'static str),
}

impl From<sqlx::Error> for AuthError {
    fn from(err: sqlx::Error) -> Self {
        AuthError::Database(err)
    }
}

// ---------------------------------------------------------------------------------------
// Passwords — [learn]
// ---------------------------------------------------------------------------------------

/// **[learn]** Hashes a plaintext password for storage in `users.password_hash`.
///
/// Return the full PHC string (`$argon2id$v=19$m=...,t=...,p=...$<salt>$<hash>`), not raw
/// bytes — it carries the algorithm, the cost parameters and the salt alongside the digest,
/// which is what lets `verify_password` work without a second column and lets you raise the
/// cost later without invalidating existing hashes.
///
/// Generate a fresh random salt per call. Reusing one salt across users would let a single
/// precomputed table attack every password at once, which is the entire reason salts exist.
///
/// Validation of length bounds belongs to the caller (`routes/auth::register`), not here —
/// this function's job is the hash, and a hasher that silently enforces policy is a hasher
/// you can't reuse for a password-change flow with different rules.
///
/// Placeholder returns `Err(NotImplemented)`: this mints a credential, so it fails loudly
/// rather than producing something that looks like a hash.
pub fn hash_password(_plaintext: &str) -> Result<String, AuthError> {
    Err(AuthError::NotImplemented("auth::hash_password"))
}

/// **[learn]** Checks `plaintext` against a stored PHC hash.
///
/// Returns `Ok(false)` for a wrong password — that's an expected outcome, not an error.
/// Reserve `Err` for a hash string that doesn't parse, which means the *stored data* is
/// corrupt and is a genuinely different situation from a user mistyping.
///
/// Do not compare with `==`. `PasswordVerifier::verify_password` does a constant-time
/// comparison; a naive equality check on the digest leaks information through timing.
///
/// Placeholder returns `Ok(false)`: unimplemented means nobody gets in.
pub fn verify_password(_plaintext: &str, _phc_hash: &str) -> Result<bool, AuthError> {
    Ok(false)
}

// ---------------------------------------------------------------------------------------
// Sessions — [learn]
// ---------------------------------------------------------------------------------------

/// A freshly issued session: the token to hand the browser, plus when it stops working.
///
/// The plaintext token exists only here and in the `Set-Cookie` header — it is deliberately
/// never returned by any query, because the database stores only its hash.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct IssuedSession {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// **[learn]** Generates a new opaque session token.
///
/// Must come from the OS CSPRNG (`rand::rngs::OsRng`), **not** from `rand::rng()` — the
/// generator `rerank.rs` uses for favorite selection is seeded for reproducibility, which is
/// exactly the property a session token must not have. Aim for at least 128 bits of entropy,
/// 256 is cheap; encode as hex or URL-safe base64 so it survives a cookie value unescaped.
///
/// Placeholder is `todo!()`: this mints a credential, and a placeholder that returned a fixed
/// string would be a hardcoded session token.
#[allow(dead_code)]
pub fn generate_session_token() -> String {
    todo!("auth::generate_session_token — see the module doc on CSPRNG choice and entropy")
}

/// **[learn]** Hashes a session token for storage and lookup in `sessions.token_hash`.
///
/// A **fast** hash (SHA-256 is the obvious pick) is correct here, which is the opposite of
/// the advice for passwords, and worth being able to explain: a 256-bit random token has no
/// dictionary and no structure to guess at, so slowing an attacker down buys nothing. The
/// reason to hash at all is that a database leak then yields hashes rather than a pile of
/// live sessions.
///
/// Must be deterministic — unlike `hash_password`, no salt. The lookup is
/// `WHERE token_hash = ?`, so the same token has to produce the same output every time.
///
/// Placeholder is `todo!()`: a fixed return value would collapse every session onto one row.
#[allow(dead_code)]
pub fn session_token_hash(_token: &str) -> String {
    todo!("auth::session_token_hash — deterministic, unsalted, fast; see the module doc")
}

/// **[learn]** Creates a session row for `user_id` and returns the token to put in the cookie.
///
/// Store `session_token_hash(&token)`, never the token. Set `expires_at` to
/// `now + SESSION_DURATION_DAYS`, computed once here rather than at validation time — see
/// that constant's doc for why.
///
/// Placeholder returns `Err(NotImplemented)`: minting again.
pub async fn issue_session(
    _pool: &SqlitePool,
    _user_id: &str,
) -> Result<IssuedSession, AuthError> {
    Err(AuthError::NotImplemented("auth::issue_session"))
}

/// **[learn]** Resolves a session token to its user, or `None` if there's no live session.
///
/// This is the function the extractor in `routes/auth.rs` calls on **every request**, and
/// therefore the one place that decides who the caller is. Three things it has to get right:
///
/// 1. Look up by `session_token_hash(token)`, not by the token.
/// 2. Reject expired sessions — `expires_at <= now` is not a session, whether you filter it
///    in SQL or check it after loading the row. A row existing is not the same as it being
///    valid, and this is the check that's easy to leave out because nothing visibly breaks
///    when you do.
/// 3. Return `Ok(None)` for "no valid session". Reserve `Err` for the database actually
///    failing — a missing or expired session is the normal signed-out case, and mapping it to
///    an error tends to turn into a 500 where a 401 belongs.
///
/// Placeholder returns `Ok(None)`: unimplemented means every request is unauthenticated.
pub async fn validate_session(
    _pool: &SqlitePool,
    _token: &str,
) -> Result<Option<User>, AuthError> {
    Ok(None)
}

/// **[learn]** Ends the session identified by `token`.
///
/// Deleting the row is what makes logout real, and it's the reason this app uses server-side
/// sessions rather than a signed JWT: clearing the cookie only asks the browser to forget the
/// token, and a token already copied elsewhere would keep working until it expired.
///
/// Idempotent — logging out twice, or with a token that was never valid, is a success, not a
/// 404. The caller has no way to tell the difference and nothing useful to do with it.
///
/// Placeholder returns `Ok(())`: it revokes nothing, but a logout that *reports* failure
/// would push callers toward retry logic that doesn't help.
pub async fn revoke_session(_pool: &SqlitePool, _token: &str) -> Result<(), AuthError> {
    Ok(())
}

/// Deletes every expired session row. Housekeeping, not security — `validate_session` must
/// reject expired sessions on its own regardless of whether this has ever run.
///
/// `[gen]`: plain maintenance SQL with no decision in it. Called from `main` at startup.
pub async fn purge_expired_sessions(pool: &SqlitePool) -> Result<u64, AuthError> {
    let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(Utc::now())
        .execute(pool)
        .await?;

    Ok(result.rows_affected())
}

/// When a session issued `now` should expire. Shared by `issue_session` and its tests so the
/// policy is stated once.
#[allow(dead_code)]
pub fn session_expiry_from(now: DateTime<Utc>) -> DateTime<Utc> {
    now + Duration::days(SESSION_DURATION_DAYS)
}

// ---------------------------------------------------------------------------------------
// Google OAuth — [learn]
// ---------------------------------------------------------------------------------------

/// Google OAuth client credentials, read from the environment.
///
/// `[you]` per PLAN.md: register the client in Google Cloud Console yourself and put the
/// values in `apps/fridge-app/backend/.env` (gitignored). `.env.example` lists the names.
#[derive(Debug, Clone)]
pub struct GoogleOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    /// Must match a redirect URI registered on the Google client *exactly* — Google compares
    /// the full string including scheme, host, port and path.
    pub redirect_uri: String,
}

impl GoogleOAuthConfig {
    /// Reads the config from the environment, or `None` if it isn't fully configured.
    ///
    /// `None` rather than an error so the app runs perfectly well without Google set up —
    /// password login is the primary method and OAuth is explicitly the *alternate* one.
    /// `routes/auth.rs` turns a `None` here into a 501 on the two Google routes only.
    ///
    /// `[gen]`: reading three env vars is not the learning content.
    pub fn from_env() -> Option<Self> {
        Some(Self {
            client_id: std::env::var("GOOGLE_CLIENT_ID").ok()?,
            client_secret: std::env::var("GOOGLE_CLIENT_SECRET").ok()?,
            redirect_uri: std::env::var("GOOGLE_REDIRECT_URI").ok()?,
        })
    }
}

/// Who Google says the user is.
///
/// `subject` is Google's `sub` claim and the **only** field safe to link an account on — see
/// the module doc. `email` is here for display and for first-time account creation; treat it
/// as a mutable attribute of the identity, never as the identity itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleIdentity {
    pub subject: String,
    pub email: String,
    pub email_verified: bool,
}

/// **[learn]** Builds the URL to redirect the user to, to start the Google flow.
///
/// Query parameters you'll need: `client_id`, `redirect_uri`, `response_type=code`, a `scope`
/// covering at least `openid email`, and the `state` you were handed. Percent-encode the
/// values — `redirect_uri` and `scope` both contain characters that will silently break the
/// request otherwise.
///
/// `state` is generated and stored in a cookie by `routes/auth::google_start`; your job here
/// is only to put it on the URL. It exists so the callback can prove the request it's
/// handling is one this server started.
///
/// Placeholder is `todo!()`: returning a wrong-but-plausible URL would fail at Google with an
/// error that's much harder to trace back to here.
pub fn google_authorize_url(_config: &GoogleOAuthConfig, _state: &str) -> String {
    todo!("auth::google_authorize_url — see the module doc for the five-step flow")
}

/// **[learn]** Exchanges an authorization `code` for the user's Google identity.
///
/// Server-side POST to Google's token endpoint with `code`, `client_id`, `client_secret`,
/// `redirect_uri` and `grant_type=authorization_code`, form-encoded (`reqwest`'s `.form()`,
/// already a dependency). The `client_secret` is why this half must never touch the browser.
///
/// Getting the identity out of the response is a real fork in the road, and both branches are
/// defensible:
///
/// - **Read the `id_token`.** It's a JWT; doing it properly means fetching Google's JWKS,
///   verifying the signature, and checking `iss`/`aud`/`exp`. More to build, and the version
///   worth learning, since it's how you'd validate a token that arrived from anywhere else.
/// - **Call the `userinfo` endpoint** with the returned access token. No JWT verification at
///   all, and it's sound here for a specific reason worth being able to state: this response
///   came to you directly from Google over TLS, so there's no untrusted intermediary whose
///   tampering a signature would be protecting you against. That reasoning stops holding the
///   moment a token reaches you via the browser.
///
/// If you take the JWT branch, do not skip signature verification because the token "came
/// from Google" — write that as a deliberate choice with the userinfo endpoint, or verify it
/// properly. Decoding a JWT without checking its signature is the single most common OAuth
/// implementation bug.
///
/// Placeholder returns `Err(NotImplemented)`: it establishes an identity, so it fails loudly.
pub async fn exchange_google_code(
    _config: &GoogleOAuthConfig,
    _code: &str,
) -> Result<GoogleIdentity, AuthError> {
    Err(AuthError::NotImplemented("auth::exchange_google_code"))
}

/// **[learn]** Generates the OAuth `state` value.
///
/// Same requirements as `generate_session_token` — CSPRNG, enough entropy that it can't be
/// guessed — because it serves the same purpose: an attacker who can predict it can forge a
/// callback. You may well implement this by delegating to that function; the separate name is
/// so that changing session-token policy later doesn't silently change CSRF policy too.
///
/// Placeholder is `todo!()`, matching `generate_session_token`.
pub fn generate_oauth_state() -> String {
    todo!("auth::generate_oauth_state — CSPRNG, same requirements as generate_session_token")
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests describe the required behavior and will fail against the placeholders above
    // until you implement them — the same arrangement `rerank.rs` shipped with in Phase 4.
    // They deliberately assert on *properties* rather than exact outputs, so they constrain
    // correctness without dictating your choice of algorithm, encoding, or parameters.

    const PASSWORD: &str = "correct horse battery staple";

    #[test]
    fn a_password_hash_is_not_the_password() {
        // The one failure mode that matters most and is easiest to introduce by accident —
        // a "hash" function that returns its input, or that encodes rather than hashes.
        let hash = hash_password(PASSWORD).expect("hashing should succeed");

        assert_ne!(hash, PASSWORD);
        assert!(
            !hash.contains(PASSWORD),
            "the plaintext must not be recoverable from the stored hash"
        );
    }

    #[test]
    fn the_same_password_hashes_differently_every_time() {
        // This is the salt test. Two users who pick the same password must not end up with
        // the same row value, or one precomputed table breaks both at once.
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
        // A near-miss, since a truncating or prefix-comparing implementation passes the test
        // above but fails this one.
        assert!(!verify_password("correct horse battery stapl", &hash).unwrap());
        assert!(!verify_password("", &hash).unwrap());
    }

    #[test]
    fn verifying_against_a_hash_of_a_different_password_fails() {
        // Guards the case where verification ignores the stored hash entirely and just
        // re-hashes the input — which passes every test above.
        let hash = hash_password("a completely different password").unwrap();

        assert!(!verify_password(PASSWORD, &hash).unwrap());
    }

    #[test]
    fn a_wrong_password_is_a_false_not_an_error() {
        // The contract `routes/auth::login` depends on: `Ok(false)` is "wrong password",
        // `Err` means the stored hash is unreadable. Collapsing the two turns a failed login
        // into a 500.
        let hash = hash_password(PASSWORD).unwrap();

        assert!(matches!(verify_password("nope", &hash), Ok(false)));
    }

    #[test]
    fn a_corrupt_stored_hash_is_an_error_not_a_silent_false() {
        // The other half of that contract. A hash string that doesn't parse means the row is
        // damaged, which is a genuinely different situation from a user mistyping and should
        // not be reported as "wrong password".
        assert!(verify_password(PASSWORD, "not a PHC string at all").is_err());
    }

    #[test]
    fn session_tokens_are_unique_across_calls() {
        // Cheap smoke test for "did you actually use a random generator". A counter or a
        // constant fails here immediately.
        let first = generate_session_token();
        let second = generate_session_token();

        assert_ne!(first, second);
    }

    #[test]
    fn a_session_token_is_long_enough_to_be_unguessable() {
        // 32 characters is a deliberately loose floor — it admits hex, base64 and anything
        // else sensible at 128 bits or more, while rejecting a token short enough to be
        // brute-forced. Tighten it if you want to pin your encoding.
        let token = generate_session_token();

        assert!(
            token.len() >= 32,
            "session token {token:?} is only {} chars — see the module doc on entropy",
            token.len()
        );
    }

    #[test]
    fn a_session_token_survives_a_cookie_value_unescaped() {
        // Whatever encoding you pick has to round-trip through a Set-Cookie header without
        // quoting. Rules out raw bytes and standard (non-URL-safe) base64, whose `+` and `/`
        // cause trouble in practice.
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
        // Unlike passwords: the lookup is `WHERE token_hash = ?`, so a salted (non-repeatable)
        // hash here would make every session unfindable the moment it was stored.
        let token = "a-fixed-token-value-for-this-test";

        assert_eq!(session_token_hash(token), session_token_hash(token));
    }

    #[test]
    fn different_session_tokens_hash_differently() {
        assert_ne!(session_token_hash("token-one"), session_token_hash("token-two"));
    }

    #[test]
    fn a_session_token_hash_is_not_the_token() {
        // Same shape as the password test, and the same reason: the point of the column is
        // that a database leak doesn't hand over live sessions.
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
        // Same requirements as session tokens — a predictable `state` is a forgeable
        // callback, which is the whole thing `state` exists to prevent.
        let first = generate_oauth_state();
        let second = generate_oauth_state();

        assert_ne!(first, second);
        assert!(first.len() >= 32, "oauth state {first:?} is too short to be unguessable");
    }

    #[test]
    fn the_authorize_url_points_at_google_and_carries_the_state() {
        let config = GoogleOAuthConfig {
            client_id: "test-client-id".to_string(),
            client_secret: "test-client-secret".to_string(),
            redirect_uri: "http://127.0.0.1:8080/auth/google/callback".to_string(),
        };

        let url = google_authorize_url(&config, "test-state-value");

        assert!(url.starts_with("https://"), "the authorize URL must be https");
        assert!(url.contains("accounts.google.com"));
        assert!(url.contains("test-client-id"));
        assert!(url.contains("test-state-value"), "state must reach Google or CSRF checking is theatre");
        assert!(url.contains("response_type=code"));
        // The secret is for the server-side token exchange only. Putting it on a URL the
        // browser follows would publish it to history, logs and the Referer header.
        assert!(
            !url.contains("test-client-secret"),
            "the client secret must never appear in a URL the browser visits"
        );
    }

    #[test]
    fn the_authorize_url_percent_encodes_its_parameters() {
        // `redirect_uri` contains `:` and `/`, which have to survive as an encoded value
        // rather than splitting the query string. This is the bug that shows up as an opaque
        // "redirect_uri_mismatch" from Google.
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
}
