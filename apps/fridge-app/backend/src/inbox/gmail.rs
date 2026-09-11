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
    /// The message text, stripped and truncated to [`BODY_LIMIT`].
    ///
    /// **Never stored.** `email_messages` has no column for it and must not gain one: this
    /// exists to be classified and dropped. The snippet is the durable record.
    pub body: Option<String>,
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
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
    body: Option<PartBody>,
    #[serde(default)]
    parts: Vec<Payload>,
}

#[derive(Debug, Deserialize)]
struct PartBody {
    data: Option<String>,
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

/// How much message text the classifier gets.
///
/// Rule 1 says to truncate the body, and the reason is not bandwidth: the whole body is the
/// prompt-injection surface, and a decision sentence that has not appeared in four thousand
/// characters is not going to. Polite rejections bury the refusal one or two paragraphs in,
/// which is the case this limit has to clear — the snippet's ~200 characters did not.
pub const BODY_LIMIT: usize = 4000;

/// One message: headers, snippet, and enough body text to classify it.
///
/// **`format=full` since 2026-09-11.** It was `format=metadata`, with a comment saying the body
/// was never transferred at all — a deliberate minimisation that turned out to cost real
/// verdicts. Gmail's snippet is capped near 200 characters, and a polite rejection reads
/// "Thank you so much for taking the time to apply… we know a lot of thought went into your
/// application, and…" for longer than that before it says no. Epic Games' rejection was
/// unclassifiable for exactly this reason.
///
/// What did not change: **the body is never stored.** It is decoded, stripped, truncated,
/// classified, and dropped. `email_messages` holds the subject and the snippet, as before.
pub async fn fetch_message(
    client: &reqwest::Client,
    token: &str,
    id: &str,
) -> Result<Message> {
    let url = format!("{API}/messages/{id}?format=full");
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
        body: raw.payload.as_ref().and_then(body_text),
    })
}

/// The best text/plain the payload offers, else text/html stripped to text.
///
/// Prefers plain over html wherever a multipart/alternative offers both: stripping tags is a
/// lossy guess, and the plain part is what the sender wrote.
fn body_text(payload: &Payload) -> Option<String> {
    let plain = find_part(payload, "text/plain").map(|raw| clean(&raw, false));
    let text = plain
        .filter(|text| !text.trim().is_empty())
        .or_else(|| find_part(payload, "text/html").map(|raw| clean(&raw, true)))?;

    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(BODY_LIMIT).collect())
}

/// Depth-first search for the first part of a given MIME type, decoded.
fn find_part(payload: &Payload, want: &str) -> Option<String> {
    if payload.mime_type.as_deref() == Some(want)
        && let Some(data) = payload.body.as_ref().and_then(|b| b.data.as_deref())
    {
        return decode_base64url(data);
    }
    payload.parts.iter().find_map(|part| find_part(part, want))
}

/// Collapse a message part into classifiable text.
///
/// `strip_tags` also drops `<script>` and `<style>` bodies wholesale — their contents are not
/// prose, and leaving them in gives the marker lists a haystack full of CSS.
fn clean(raw: &str, strip_tags: bool) -> String {
    let mut out = String::with_capacity(raw.len());
    if strip_tags {
        let mut in_tag = false;
        let mut skip_until: Option<&str> = None;
        let lower = raw.to_lowercase();
        let bytes: Vec<char> = raw.chars().collect();
        let lower_chars: Vec<char> = lower.chars().collect();
        let mut i = 0;
        while i < bytes.len() {
            if let Some(end) = skip_until {
                if lower_chars[i..].starts_with(&end.chars().collect::<Vec<_>>()[..]) {
                    i += end.chars().count();
                    skip_until = None;
                    continue;
                }
                i += 1;
                continue;
            }
            if lower_chars[i..].starts_with(&['<', 's', 'c', 'r', 'i', 'p', 't']) {
                skip_until = Some("</script>");
                i += 7;
                continue;
            }
            if lower_chars[i..].starts_with(&['<', 's', 't', 'y', 'l', 'e']) {
                skip_until = Some("</style>");
                i += 6;
                continue;
            }
            match bytes[i] {
                '<' => in_tag = true,
                '>' => {
                    in_tag = false;
                    out.push(' ');
                }
                c if !in_tag => out.push(c),
                _ => {}
            }
            i += 1;
        }
    } else {
        out.push_str(raw);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Gmail returns part bodies base64url-encoded without padding.
///
/// Hand-rolled rather than pulling in a crate: this repo's conventions say to ask before adding
/// a dependency, and the alphabet is twenty lines. Invalid input yields `None`, which reads as
/// "no body" and falls back to the snippet — the safe direction.
fn decode_base64url(data: &str) -> Option<String> {
    fn value(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        })
    }

    let mut bytes = Vec::with_capacity(data.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for c in data.bytes() {
        if c == b'=' || c == b'\n' || c == b'\r' {
            continue;
        }
        acc = (acc << 6) | value(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    // Lossy on purpose: a mailbox contains every encoding anyone has ever used, and refusing a
    // message because one byte is not UTF-8 would drop the whole verdict for a mojibake glyph.
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    /// The one property worth asserting about this file mechanically: it performs no writes.
    ///
    /// 8a is read-only, and "read-only" is only true while nobody adds a `modify` call in
    /// passing. Reading the source is crude and it is also exactly what
    /// `sources::adapters_do_not_build_their_own_http_client` does one subsystem over, for the
    /// same reason — a rule the compiler cannot state is better checked than trusted.
    /// The module's code with comment lines removed.
///
/// Scanning raw source made a doc comment explaining what this module does *not* do fail the
/// check that it does not do it. Prose about a forbidden call is not the call.
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

    #[test]
    fn this_module_makes_no_write_calls_to_gmail() {
        let source = include_str!("gmail.rs");
        // Comments stripped for the same reason as in `labels.rs`: this file's own doc says
        // it contains no `modify`, and prose about a call is not the call.
        let body = code_only(
            source.split("mod tests").next().expect("the module above its tests"),
        );

        for forbidden in [".post(", ".put(", ".patch(", ".delete(", "/modify", "/trash", "/send"] {
            assert!(
                !body.contains(forbidden),
                "gmail.rs contains {forbidden:?} — 8a is read-only, and write access belongs \
                 in its own function in 8c so a diff can show when it arrived"
            );
        }
    }

    use super::*;

    fn part(mime: &str, data: &str) -> Payload {
        Payload {
            headers: Vec::new(),
            mime_type: Some(mime.to_string()),
            body: Some(PartBody { data: Some(data.to_string()) }),
            parts: Vec::new(),
        }
    }

    /// base64url, unpadded, as Gmail emits it.
    fn b64(text: &str) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let bytes = text.as_bytes();
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            let take = chunk.len() + 1;
            for i in 0..take {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
        }
        out
    }

    #[test]
    fn a_plain_part_round_trips() {
        let payload = part("text/plain", &b64("Unfortunately we will not be moving forward."));
        assert_eq!(
            body_text(&payload).as_deref(),
            Some("Unfortunately we will not be moving forward.")
        );
    }

    #[test]
    fn plain_wins_over_html_in_a_multipart_alternative() {
        // Stripping tags is a lossy guess; the plain part is what the sender wrote.
        let payload = Payload {
            headers: Vec::new(),
            mime_type: Some("multipart/alternative".to_string()),
            body: None,
            parts: vec![
                part("text/html", &b64("<p>html version</p>")),
                part("text/plain", &b64("plain version")),
            ],
        };
        assert_eq!(body_text(&payload).as_deref(), Some("plain version"));
    }

    #[test]
    fn html_is_stripped_to_its_text_when_that_is_all_there_is() {
        let payload = part(
            "text/html",
            &b64("<html><style>p{color:red}</style><body><p>We regret to inform you</p>\
                  <script>var x=1</script></body></html>"),
        );
        let text = body_text(&payload).expect("text");
        assert!(text.contains("We regret to inform you"), "got {text:?}");
        assert!(!text.contains("color:red"), "style contents are not prose: {text:?}");
        assert!(!text.contains("var x"), "script contents are not prose: {text:?}");
    }

    #[test]
    fn a_body_is_truncated_rather_than_transferred_whole() {
        let long = "a".repeat(BODY_LIMIT * 2);
        let payload = part("text/plain", &b64(&long));
        assert_eq!(body_text(&payload).map(|t| t.len()), Some(BODY_LIMIT));
    }

    #[test]
    fn an_undecodable_body_reads_as_no_body_rather_than_an_error() {
        // Falls back to the snippet, which is the safe direction — a mailbox contains every
        // encoding anyone has ever used.
        let payload = part("text/plain", "!!!not base64!!!");
        assert_eq!(body_text(&payload), None);
    }

    #[test]
    fn a_nested_multipart_still_yields_its_text() {
        // multipart/mixed wrapping multipart/alternative is the common real shape.
        let inner = Payload {
            headers: Vec::new(),
            mime_type: Some("multipart/alternative".to_string()),
            body: None,
            parts: vec![part("text/plain", &b64("buried but findable"))],
        };
        let outer = Payload {
            headers: Vec::new(),
            mime_type: Some("multipart/mixed".to_string()),
            body: None,
            parts: vec![inner],
        };
        assert_eq!(body_text(&outer).as_deref(), Some("buried but findable"));
    }
}
