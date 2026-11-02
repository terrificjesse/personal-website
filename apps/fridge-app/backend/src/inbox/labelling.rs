//! Hand-labelling the classifier's held-out mail, one message at a time (Phase 13b).
//!
//! # Why this exists
//!
//! Phase 13's measurement needs a human to say what each email actually is, and it has been
//! blocked on that for a month. The tool for it was a CSV — export, open in a spreadsheet, fill a
//! column, save, score — and a CSV carries only Gmail's ~200-character snippet, which is not
//! enough to label from: nine Microsoft rejections read "Thank you for your application!" and
//! refuse you four hundred characters in. A labeller working from the snippet would label them
//! confirmations, and the measurement would then grade the classifier against a mistake.
//!
//! So this serves one message at a time, **with its body**, and records one click.
//!
//! # What it deliberately does not show
//!
//! **The classifier's verdict.** Not in the response, not in the table. A person shown the
//! machine's answer agrees with it, and the measurement then grades the classifier against
//! itself — the "anchoring" argument the labelling sheet was built on, in `labelset.rs`. A test
//! serialises the response and asserts the word is absent.
//!
//! # Which mail
//!
//! Only mail that arrived **after the classifier's rules last changed**. Earlier mail was
//! available while the rules were being tuned, and grading on it measures the tuning — the
//! reason 13b is the owner's task at all. [`HELD_OUT_SINCE`] is that boundary, and
//! [`HELD_OUT_RULES`] is the rules it was set against: a test fails the moment `classify.rs`
//! changes, so the window cannot quietly go stale after a revision.
//!
//! The order is a fixed shuffle (SHA-256 of the Gmail id), so **any prefix is a random sample**.
//! Stop after ten or after a hundred, and what has been labelled is still unbiased — there is no
//! "first forty" to pick, and nothing about the order leaks the verdict either.
//!
//! # The body is fetched on a click, never on a read
//!
//! `routes/inbox.rs`: "Nothing here fetches from Gmail on a GET." The next message, and its body,
//! comes back from a POST the labeller triggers; the progress counter is the only GET. And the
//! body is returned and dropped — `gmail.rs`'s rule that a body is "decoded, stripped, truncated,
//! classified, and dropped" is unchanged; nothing here writes one.

use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use sha2::Digest;
use sqlx::SqlitePool;

use super::classify::Category;

/// Mail received at or after this instant has never been seen by the rules being measured.
///
/// The rules last changed on 2026-11-02, revised against the owner's first 90 labels, all of
/// which are therefore in-sample (`labelsets/fresh-2026-10.csv`, ledger beside it). Set to the
/// minute of that change rather than the next midnight: every message received before it had
/// been read while tuning. Before that, 2026-09-22 (`f454449`).
///
/// **Move this whenever `classify.rs` changes**, to the moment of the change. [`HELD_OUT_RULES`]
/// and its test make that impossible to forget.
pub const HELD_OUT_SINCE: &str = "2026-11-02T20:46:00+00:00";

/// The rules fingerprint ([`super::labelset::rules_fingerprint`]) [`HELD_OUT_SINCE`] belongs to.
pub const HELD_OUT_RULES: &str = "1959b41978bc";

/// How many labels make a first honest measurement. A suggestion shown to the labeller, not a
/// cap: the shuffle means stopping anywhere is still a random sample.
pub const TARGET: i64 = 40;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Progress {
    pub labelled: i64,
    pub skipped: i64,
    pub remaining: i64,
    pub total: i64,
    pub target: i64,
    pub since: &'static str,
}

/// One message to label. **No category, by construction** — see the module docs.
#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub message_id: String,
    pub received_at: Option<String>,
    pub from: Option<String>,
    pub subject: Option<String>,
    /// The body, when it could be read. `None` falls back to the snippet below, and the page
    /// says so — labelling from a snippet is how a polite rejection gets called a confirmation.
    pub body: Option<String>,
    pub snippet: Option<String>,
    /// The message in Gmail, for anything the body here does not settle.
    pub gmail_url: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct Candidate {
    pub id: String,
    pub gmail_message_id: String,
    pub received_at: Option<String>,
    pub from_address: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
}

/// The fixed shuffle: SHA-256 of the Gmail id. Deterministic, so a reload shows the same
/// message, and unrelated to arrival order or to anything the classifier decided.
pub fn shuffle_key(gmail_message_id: &str) -> [u8; 32] {
    sha2::Sha256::digest(gmail_message_id.as_bytes()).into()
}

pub async fn progress(pool: &SqlitePool, user_id: &str) -> Result<Progress> {
    let (total, labelled, skipped): (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*),
                COALESCE(SUM(l.message_id IS NOT NULL AND l.label IS NOT NULL), 0),
                COALESCE(SUM(l.message_id IS NOT NULL AND l.label IS NULL), 0)
           FROM email_messages m
           LEFT JOIN email_labels l ON l.message_id = m.id
          WHERE m.user_id = ?1 AND m.received_at >= ?2",
    )
    .bind(user_id)
    .bind(HELD_OUT_SINCE)
    .fetch_one(pool)
    .await?;
    Ok(Progress {
        labelled,
        skipped,
        remaining: total - labelled - skipped,
        total,
        target: TARGET,
        since: HELD_OUT_SINCE,
    })
}

/// The next unlabelled message in the window, in shuffle order.
///
/// Refuses outright when the window belongs to other rules. The test of the same name catches
/// this in CI; this catches a binary built without running it, which would otherwise serve mail
/// the current rules may have been tuned on and call it held out.
pub async fn next_candidate(pool: &SqlitePool, user_id: &str) -> Result<Option<Candidate>> {
    let rules = super::labelset::rules_fingerprint();
    if rules != HELD_OUT_RULES {
        anyhow::bail!(
            "the held-out window was set for rules {HELD_OUT_RULES}, and these are {rules}: \
             move HELD_OUT_SINCE and HELD_OUT_RULES in inbox/labelling.rs before labelling more"
        );
    }
    let mut candidates: Vec<Candidate> = sqlx::query_as(
        "SELECT m.id, m.gmail_message_id, m.received_at, m.from_address, m.subject, m.snippet
           FROM email_messages m
          WHERE m.user_id = ?1 AND m.received_at >= ?2
            AND NOT EXISTS (SELECT 1 FROM email_labels l WHERE l.message_id = m.id)",
    )
    .bind(user_id)
    .bind(HELD_OUT_SINCE)
    .fetch_all(pool)
    .await?;
    candidates.sort_by_key(|c| shuffle_key(&c.gmail_message_id));
    Ok(candidates.into_iter().next())
}

/// Record a label, or `None` for "could not tell". Re-labelling overwrites — the page's undo is
/// a delete, and a corrected click is a second record.
///
/// `false` when the message is not this user's, or is outside the held-out window: labelling
/// mail the rules were tuned on would put an in-sample row into the measurement.
pub async fn record(
    pool: &SqlitePool,
    user_id: &str,
    message_id: &str,
    label: Option<Category>,
) -> Result<bool> {
    let in_window: Option<String> = sqlx::query_scalar(
        "SELECT id FROM email_messages WHERE id = ?1 AND user_id = ?2 AND received_at >= ?3",
    )
    .bind(message_id)
    .bind(user_id)
    .bind(HELD_OUT_SINCE)
    .fetch_optional(pool)
    .await?;
    if in_window.is_none() {
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO email_labels (message_id, user_id, label, labelled_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (message_id) DO UPDATE SET label = excluded.label,
                                                labelled_at = excluded.labelled_at",
    )
    .bind(message_id)
    .bind(user_id)
    .bind(label.map(Category::as_str))
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(true)
}

/// Undo: the message goes back into the queue.
pub async fn undo(pool: &SqlitePool, user_id: &str, message_id: &str) -> Result<bool> {
    let removed = sqlx::query("DELETE FROM email_labels WHERE message_id = ?1 AND user_id = ?2")
        .bind(message_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(removed.rows_affected() == 1)
}

/// The body of one message, or `None` when it cannot be read. Never stored.
///
/// A failure is not an error here: the page falls back to the snippet and says it did.
pub async fn body_for(
    pool: &SqlitePool,
    user_id: &str,
    client_id: &str,
    client_secret: &str,
    gmail_message_id: &str,
) -> Option<String> {
    let token = super::oauth::access_token(pool, user_id, client_id, client_secret)
        .await
        .ok()?;
    let client = reqwest::Client::new();
    super::reclassify::fetch_body_politely(&client, &token, gmail_message_id)
        .await
        .ok()?
        .body
}

/// A link to the message in Gmail, opened in the right account when more than one is signed in.
pub fn gmail_url(account: Option<&str>, gmail_message_id: &str) -> String {
    match account {
        Some(account) => format!(
            "https://mail.google.com/mail/?authuser={}#all/{gmail_message_id}",
            account.replace('@', "%40")
        ),
        None => format!("https://mail.google.com/mail/#all/{gmail_message_id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    /// The tripwire. `HELD_OUT_SINCE` is only honest for the rules it was set against; change
    /// `classify.rs` and every message in the window becomes something those rules may have been
    /// tuned on. When this fails: move `HELD_OUT_SINCE` to the day of the change, and set
    /// `HELD_OUT_RULES` to the new fingerprint. Labels already collected stay in the table and
    /// remain exportable — they are simply in-sample for the new rules, which the ledger records.
    #[test]
    fn the_held_out_window_belongs_to_the_current_rules() {
        assert_eq!(
            super::super::labelset::rules_fingerprint(),
            HELD_OUT_RULES,
            "classify.rs changed: move HELD_OUT_SINCE to the day of the change and update \\
             HELD_OUT_RULES, or this page serves mail the new rules were tuned on"
        );
    }

    /// The verdict must never reach the labeller. Asserted on the serialised response rather
    /// than the struct, because what matters is what the browser receives.
    #[test]
    fn an_item_never_carries_the_classifier_verdict() {
        let item = Item {
            message_id: "m1".into(),
            received_at: None,
            from: Some("careers@example.com".into()),
            subject: Some("Thank you for your application!".into()),
            body: Some("We have decided not to move forward.".into()),
            snippet: None,
            gmail_url: None,
        };
        let json = serde_json::to_string(&item).unwrap().to_lowercase();
        for leak in ["category", "verdict", "confidence", "evidence", "classified"] {
            assert!(!json.contains(leak), "the item leaks `{leak}`: {json}");
        }
    }

    #[test]
    fn the_shuffle_is_fixed_and_is_not_arrival_order() {
        let ids: Vec<String> = (0..50).map(|i| format!("{i:016x}")).collect();
        let mut once = ids.clone();
        once.sort_by_key(|id| shuffle_key(id));
        let mut twice = ids.clone();
        twice.reverse();
        twice.sort_by_key(|id| shuffle_key(id));
        assert_eq!(once, twice, "the order must not depend on how the rows came back");
        assert_ne!(once, ids, "fifty ids in arrival order would mean no shuffle at all");
    }

    async fn pool() -> SqlitePool {
        let path = std::env::temp_dir().join(format!("labelling-{}.db", Uuid::new_v4()));
        let pool = crate::db::init_pool(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("migrations");
        sqlx::query("INSERT INTO users (id, email, created_at) VALUES ('u1','u1@example.com','2026-09-01')")
            .execute(&pool)
            .await
            .unwrap();
        // Relative to the window, not fixed dates. These were "2026-09-10" and "2026-10-01/02",
        // which silently became pre-window the first time the window moved (2026-11-02).
        let since = chrono::DateTime::parse_from_rfc3339(HELD_OUT_SINCE).expect("since").with_timezone(&Utc);
        for (id, offset_days) in [("old", -1), ("a", 1), ("b", 2)] {
            let received = (since + chrono::Duration::days(offset_days)).to_rfc3339();
            sqlx::query(
                "INSERT INTO email_messages (id, user_id, gmail_message_id, received_at, created_at)
                 VALUES (?1, 'u1', ?1, ?2, ?2)",
            )
            .bind(id)
            .bind(&received)
            .execute(&pool)
            .await
            .unwrap();
        }
        pool
    }

    /// Mail the rules were tuned on is never offered, and cannot be labelled by asking directly.
    #[tokio::test]
    async fn only_held_out_mail_is_offered_or_accepted() {
        let pool = pool().await;
        let progress = progress(&pool, "u1").await.unwrap();
        assert_eq!(progress.total, 2, "the message from before the window predates the rules");

        assert!(!record(&pool, "u1", "old", Some(Category::Rejection)).await.unwrap());
        assert!(!record(&pool, "someone-else", "a", Some(Category::Rejection)).await.unwrap());
    }

    /// Label, skip, undo: the queue drains, a skip is counted apart from a label, and undo puts
    /// the message back.
    #[tokio::test]
    async fn labelling_drains_the_queue_and_undo_refills_it() {
        let pool = pool().await;
        let first = next_candidate(&pool, "u1").await.unwrap().expect("one to label");
        assert!(record(&pool, "u1", &first.id, Some(Category::Oa)).await.unwrap());

        let second = next_candidate(&pool, "u1").await.unwrap().expect("one more");
        assert_ne!(second.id, first.id);
        assert!(record(&pool, "u1", &second.id, None).await.unwrap(), "a skip is recorded");

        assert!(next_candidate(&pool, "u1").await.unwrap().is_none());
        let p = progress(&pool, "u1").await.unwrap();
        assert_eq!((p.labelled, p.skipped, p.remaining), (1, 1, 0));

        assert!(undo(&pool, "u1", &first.id).await.unwrap());
        assert_eq!(next_candidate(&pool, "u1").await.unwrap().map(|c| c.id), Some(first.id));
    }
}
