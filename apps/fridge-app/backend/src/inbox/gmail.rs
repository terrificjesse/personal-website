//! The Gmail API surface this agent uses — **read-only, by construction.**
//!
//! Every function here is a GET. There is no `modify`, no `trash`, no `send`, and no
//! `batchModify` in this file, and 8a is the phase where that is a property of the code rather
//! than a promise. Labels arrive in 8c, in their own function, so "when did this gain write
//! access" is answerable from a diff.
//!
//! Not routed through `internships::http::PoliteClient`: that exists to be polite to *other
//! people's* servers — robots.txt, per-host rate limits, honest identification while scraping.
//! This is an authenticated API we are a first-party client of, with its own quota rules, and
//! borrowing the scraper's manners would only obscure that difference.

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

const API: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

/// One message, with only the headers this agent needs.
///
/// **The body is not fetched.** It is a burner account, but it is still someone's mail, and
/// 8a has no use for it — the classifier that will is 8b's, and it can ask for what it needs
/// then. Storing the minimum is cheaper to get right than deleting it later.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub id: String,
    pub thread_id: Option<String>,
    pub from: Option<String>,
    pub subject: Option<String>,
    pub received_at: Option<String>,
    pub snippet: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListResponse {
    #[serde(default)]
    messages: Vec<MessageRef>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessageRef {
    id: String,
}

#[derive(Debug, Deserialize)]
struct MessageResponse {
    id: String,
    #[serde(rename = "threadId")]
    thread_id: Option<String>,
    snippet: Option<String>,
    payload: Option<Payload>,
    #[serde(rename = "internalDate")]
    internal_date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Payload {
    #[serde(default)]
    headers: Vec<Header>,
}

#[derive(Debug, Deserialize)]
struct Header {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct ProfileResponse {
    #[serde(rename = "historyId")]
    history_id: Option<String>,
}

async fn get_json<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    token: &str,
    url: &str,
) -> Result<T> {
    let response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Gmail answered {status} for {url}: {body}"));
    }

    response.json::<T>().await.context("parsing a Gmail response")
}

/// The mailbox's current `historyId`, for watermarking the next incremental pass.
pub async fn current_history_id(client: &reqwest::Client, token: &str) -> Result<Option<String>> {
    Ok(get_json::<ProfileResponse>(client, token, &format!("{API}/profile"))
        .await?
        .history_id)
}

/// Message ids, newest first, capped.
///
/// A cap rather than "everything": the first sync of a real burner inbox is thousands of
/// messages, and a first pass that runs for ten minutes before recording anything is one you
/// cannot tell from a hung one. Paginating to a limit makes the first run finish and the run
/// record appear; the watermark then carries the rest.
pub async fn list_message_ids(
    client: &reqwest::Client,
    token: &str,
    max: usize,
) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    let mut page: Option<String> = None;

    while ids.len() < max {
        let page_size = (max - ids.len()).min(500);
        let mut url = format!("{API}/messages?maxResults={page_size}");
        if let Some(token) = &page {
            url.push_str(&format!("&pageToken={token}"));
        }

        let response: ListResponse = get_json(client, token, &url).await?;
        if response.messages.is_empty() {
            break;
        }
        ids.extend(response.messages.into_iter().map(|m| m.id));

        match response.next_page_token {
            Some(next) => page = Some(next),
            None => break,
        }
    }

    ids.truncate(max);
    Ok(ids)
}

/// One message's metadata. `format=metadata` so the body is never transferred at all.
pub async fn fetch_message(
    client: &reqwest::Client,
    token: &str,
    id: &str,
) -> Result<Message> {
    let url = format!(
        "{API}/messages/{id}?format=metadata\
         &metadataHeaders=From&metadataHeaders=Subject&metadataHeaders=Date"
    );
    let raw: MessageResponse = get_json(client, token, &url).await?;

    let header = |name: &str| {
        raw.payload.as_ref().and_then(|p| {
            p.headers
                .iter()
                .find(|h| h.name.eq_ignore_ascii_case(name))
                .map(|h| h.value.clone())
        })
    };

    Ok(Message {
        id: raw.id,
        thread_id: raw.thread_id,
        from: header("From"),
        subject: header("Subject"),
        // Gmail's `internalDate` is epoch milliseconds as a string, and is the arrival time
        // Gmail itself sorts by. The `Date` header is written by the sender and can say
        // anything at all — including a time that makes an email look older than the reply
        // to it, which is exactly the out-of-order trap rule 3 is about.
        received_at: raw
            .internal_date
            .as_deref()
            .and_then(|ms| ms.parse::<i64>().ok())
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|dt| dt.to_rfc3339())
            .or_else(|| header("Date")),
        snippet: raw.snippet,
    })
}

#[cfg(test)]
mod tests {
    /// The one property worth asserting about this file mechanically: it performs no writes.
    ///
    /// 8a is read-only, and "read-only" is only true while nobody adds a `modify` call in
    /// passing. Reading the source is crude and it is also exactly what
    /// `sources::adapters_do_not_build_their_own_http_client` does one subsystem over, for the
    /// same reason — a rule the compiler cannot state is better checked than trusted.
    #[test]
    fn this_module_makes_no_write_calls_to_gmail() {
        let source = include_str!("gmail.rs");
        let body = source
            .split("mod tests")
            .next()
            .expect("the module above its tests");

        for forbidden in [".post(", ".put(", ".patch(", ".delete(", "/modify", "/trash", "/send"] {
            assert!(
                !body.contains(forbidden),
                "gmail.rs contains {forbidden:?} — 8a is read-only, and write access belongs \
                 in its own function in 8c so a diff can show when it arrived"
            );
        }
    }
}
