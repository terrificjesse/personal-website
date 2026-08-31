//! Connecting the burner Gmail, and keeping an access token available.
//!
//! Separate from `auth::GoogleOAuthConfig`'s sign-in flow on purpose. They share a client id
//! and a secret, and nothing else: sign-in asks for `openid email` and wants an identity,
//! this asks for `gmail.modify` and wants a *refresh token* it can use later without the user
//! present. Folding them together would mean one code path with two meanings, and the one
//! that matters here — `access_type=offline` — is silently absent from the other.

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::SqlitePool;

/// The maximum this agent may ever request. Read, label, archive — withholding exactly the two
/// irreversible powers: permanent delete, and send-as.
///
/// **Binding.** `mail.google.com`, `gmail.send` and any settings scope are out without asking
/// first. The agent must never send mail, never permanently delete, and never create filters
/// or forwarding rules.
pub const GMAIL_SCOPE: &str = "https://www.googleapis.com/auth/gmail.modify";

/// Where Google sends the user back. Its own callback, not sign-in's.
pub fn redirect_uri() -> String {
    std::env::var("GMAIL_REDIRECT_URI")
        .unwrap_or_else(|_| "http://localhost:8080/auth/gmail/callback".to_string())
}

/// The consent URL.
///
/// Two parameters here are load-bearing and easy to omit:
///
/// - `access_type=offline` is what makes Google return a **refresh token** at all. Without it
///   you get an access token that dies in an hour and an agent that only works while you are
///   watching.
/// - `prompt=consent` forces the refresh token to be re-issued. Google returns one only on the
///   *first* consent otherwise, so a re-connect after any token loss silently yields none —
///   and the failure appears later, as an agent that cannot refresh.
pub fn consent_url(client_id: &str, state: &str) -> String {
    let params = [
        ("client_id", client_id),
        ("redirect_uri", &redirect_uri()),
        ("response_type", "code"),
        ("scope", GMAIL_SCOPE),
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("include_granted_scopes", "true"),
        ("state", state),
    ];
    let query = params
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("https://accounts.google.com/o/oauth2/v2/auth?{query}")
}

/// Percent-encode a query value. Small and local rather than a new dependency.
fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    #[allow(dead_code)]
    expires_in: Option<i64>,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GmailProfile {
    #[serde(rename = "emailAddress")]
    email_address: String,
}

/// Trade the consent code for a refresh token, and record the account.
///
/// Rejects a grant that did not include `gmail.modify`: Google's consent screen lets a user
/// untick scopes, and an account stored with a scope it does not have fails later, in the
/// sync, as an authorization error nobody expects.
pub async fn connect(
    pool: &SqlitePool,
    user_id: &str,
    client_id: &str,
    client_secret: &str,
    code: &str,
    now: DateTime<Utc>,
) -> Result<String> {
    let client = reqwest::Client::new();
    let redirect = redirect_uri();

    let response = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("redirect_uri", redirect.as_str()),
            ("grant_type", "authorization_code"),
            ("code", code),
        ])
        .send()
        .await
        .context("requesting a Gmail token")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Google refused the code ({status}): {body}"));
    }

    let token: TokenResponse = response.json().await.context("reading the token response")?;

    if token.scope.as_deref().is_some_and(|scope| !scope.contains(GMAIL_SCOPE)) {
        return Err(anyhow!(
            "the grant does not include {GMAIL_SCOPE} — it was probably unticked on the \
             consent screen; reconnect and leave it checked"
        ));
    }

    // `access_type=offline` plus `prompt=consent` should always produce one. If it did not,
    // say so now: an account stored without one cannot sync tomorrow, and the error would
    // otherwise surface days later as an unexplained failure.
    let refresh_token = token.refresh_token.ok_or_else(|| {
        anyhow!(
            "Google returned no refresh token. Revoke this app's access at \
             myaccount.google.com/permissions and connect again."
        )
    })?;

    let email = fetch_profile_email(&client, &token.access_token).await?;

    sqlx::query(
        "INSERT INTO gmail_accounts (user_id, email, refresh_token, connected_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT (user_id) DO UPDATE SET
             email = excluded.email,
             refresh_token = excluded.refresh_token,
             updated_at = excluded.updated_at",
    )
    .bind(user_id)
    .bind(&email)
    .bind(&refresh_token)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await
    .context("storing the Gmail account")?;

    Ok(email)
}

async fn fetch_profile_email(client: &reqwest::Client, access_token: &str) -> Result<String> {
    let response = client
        .get("https://gmail.googleapis.com/gmail/v1/users/me/profile")
        .bearer_auth(access_token)
        .send()
        .await
        .context("reading the Gmail profile")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Gmail refused the profile request ({status}): {body}"));
    }

    Ok(response
        .json::<GmailProfile>()
        .await
        .context("parsing the Gmail profile")?
        .email_address)
}

/// A usable access token for this user, minted from the stored refresh token.
///
/// Not cached: access tokens live an hour, a sync runs for seconds, and a cache would be one
/// more piece of state to invalidate for no measurable gain.
pub async fn access_token(
    pool: &SqlitePool,
    user_id: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<String> {
    let refresh_token: Option<String> =
        sqlx::query_scalar("SELECT refresh_token FROM gmail_accounts WHERE user_id = ?")
            .bind(user_id)
            .fetch_optional(pool)
            .await?
            .flatten();

    let refresh_token = refresh_token.ok_or_else(|| anyhow!("no Gmail account is connected"))?;

    let response = reqwest::Client::new()
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.as_str()),
        ])
        .send()
        .await
        .context("refreshing the Gmail token")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        // Named explicitly, because this is the failure that WILL happen: Google expires
        // refresh tokens after seven days while the OAuth app is in Testing. A sync that
        // stops for this reason must never read as a quiet inbox — see `sync`'s run record.
        return Err(anyhow!(
            "the stored Gmail token no longer works ({status}): {body} — if the app is in \
             Testing, Google expires refresh tokens after 7 days; reconnect the account"
        ));
    }

    Ok(response
        .json::<TokenResponse>()
        .await
        .context("reading the refreshed token")?
        .access_token)
}

/// Whether an account is connected, and which address.
pub async fn connected_account(pool: &SqlitePool, user_id: &str) -> Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT email FROM gmail_accounts WHERE user_id = ?")
            .bind(user_id)
            .fetch_optional(pool)
            .await?,
    )
}

/// Forget the account. Local only — it does not revoke the grant at Google, and says so.
pub async fn disconnect(pool: &SqlitePool, user_id: &str) -> Result<bool> {
    let affected = sqlx::query("DELETE FROM gmail_accounts WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(affected > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_consent_url_asks_for_offline_access_and_forces_a_fresh_refresh_token() {
        // Both are easy to omit and both fail later rather than now: without `offline` there
        // is no refresh token at all, and without `prompt=consent` a re-connect silently
        // returns none because Google only issues one on first consent.
        let url = consent_url("client-123", "state-abc");
        assert!(url.contains("access_type=offline"), "{url}");
        assert!(url.contains("prompt=consent"), "{url}");
        assert!(url.contains("state=state-abc"), "{url}");
    }

    #[test]
    fn the_consent_url_requests_exactly_the_permitted_scope() {
        // The scope ceiling is binding. If this test has to change, that is a decision to
        // take deliberately, not a diff to wave through.
        let url = consent_url("client-123", "s");
        assert!(url.contains(&urlencode(GMAIL_SCOPE)), "{url}");
        for forbidden in ["mail.google.com", "gmail.send", "gmail.settings"] {
            assert!(!url.contains(forbidden), "{url} must not request {forbidden}");
        }
    }

    #[test]
    fn query_values_are_encoded() {
        assert_eq!(urlencode("a b/c:d"), "a%20b%2Fc%3Ad");
        assert!(consent_url("c", "a b").contains("state=a%20b"));
    }
}
