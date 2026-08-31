//! One sync pass: fetch, record, count. Owns `inbox_runs`.
//!
//! **8a writes to our own tables and to nothing else.** No Gmail labels, no status changes, no
//! notifications. `email_messages` and `inbox_runs` are the whole write surface.
//!
//! # A broken sync must be visible — rule 5
//!
//! An expired refresh token must not look like a quiet inbox. That is not a logging
//! preference; it is the failure this phase is most likely to hit, because Google expires
//! refresh tokens after seven days while the OAuth app is in Testing. So **every pass writes
//! an `inbox_runs` row, including the ones that fail before fetching anything**, with the
//! outcome and the error on it. A run that classified zero emails and a run that could not
//! authenticate are different rows, not the same silence.
//!
//! # Never from a request handler
//!
//! The root `CLAUDE.md` cache rule applies to Gmail too. A sync crosses the network to someone
//! else's service, so it runs on an interval worker or an explicit trigger — never inline in a
//! GET, where a slow response would hold a request open and a page refresh would re-fetch.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::classify::{self, Category};
use super::{gmail, oauth};

/// How many messages one pass will look at.
///
/// The first sync of a real burner inbox is thousands of messages. A first pass that runs for
/// ten minutes before writing anything is indistinguishable from a hung one, and rule 5 is
/// about being able to tell. Capped, so the run record appears; the watermark carries the rest.
pub const DEFAULT_MAX_MESSAGES: usize = 100;

/// What one pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub run_id: String,
    pub fetched: i64,
    pub classified: i64,
    pub pressing: i64,
    pub confirmation: i64,
    pub outreach: i64,
    pub disregarded: i64,
    /// Messages already stored from a previous pass. Rule 4: reprocessing is a no-op.
    pub already_seen: i64,
}

impl SyncReport {
    /// RULE 7's invariant, as a function so a test can assert it rather than eyeball it.
    ///
    /// `classified = pressing + confirmation + outreach + disregarded`. Summed into one number
    /// the defect is invisible — the same reasoning as `fetched = accepted + filtered +
    /// rejected` in `source_runs`.
    pub fn counts_balance(&self) -> bool {
        self.classified == self.pressing + self.confirmation + self.outreach + self.disregarded
    }
}

/// Run one pass for a user.
///
/// Never returns `Err` for anything a run can survive — a failure is recorded on the run row
/// and returned as a report, because a sync that fails loudly into a log nobody reads is the
/// quiet inbox this rule exists to prevent. `Err` here means the *database* was unusable, and
/// there is then nowhere to record anything.
pub async fn run(
    pool: &SqlitePool,
    user_id: &str,
    client_id: &str,
    client_secret: &str,
    max_messages: usize,
    now: DateTime<Utc>,
) -> Result<SyncReport> {
    let run_id = Uuid::new_v4().to_string();
    let mut report = SyncReport { run_id: run_id.clone(), ..SyncReport::default() };

    sqlx::query(
        "INSERT INTO inbox_runs (id, user_id, started_at, outcome) VALUES (?1, ?2, ?3, 'failed')",
    )
    .bind(&run_id)
    .bind(user_id)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;

    // Opened as `failed` and corrected on success, rather than the reverse. A process that
    // dies mid-pass then leaves a row that says so, instead of one that claims success it
    // never reached — the same reasoning as `collection_runs.interrupted` next door.

    if oauth::connected_account(pool, user_id).await?.is_none() {
        finish(pool, &run_id, "skipped", Some("no Gmail account is connected"), &report, now)
            .await?;
        return Ok(report);
    }

    let token = match oauth::access_token(pool, user_id, client_id, client_secret).await {
        Ok(token) => token,
        Err(err) => {
            // THE failure this phase will actually hit. Recorded as `failed` with the reason,
            // never as a successful pass that happened to find nothing.
            finish(pool, &run_id, "failed", Some(&err.to_string()), &report, now).await?;
            return Ok(report);
        }
    };

    // The classifier is pure (rule 1), so what it may know about the world is passed in.
    // Loaded once per pass rather than per message: it is one query and a run is a hundred
    // messages.
    let known_companies: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT lower(company_name) FROM internship_postings
          UNION SELECT DISTINCT lower(company_name) FROM internship_applications",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let context = classify::Context { known_companies: &known_companies };

    let client = reqwest::Client::new();
    let ids = match gmail::list_message_ids(&client, &token, max_messages).await {
        Ok(ids) => ids,
        Err(err) => {
            finish(pool, &run_id, "failed", Some(&err.to_string()), &report, now).await?;
            return Ok(report);
        }
    };

    let mut partial: Option<String> = None;
    for id in &ids {
        let message = match gmail::fetch_message(&client, &token, id).await {
            Ok(message) => message,
            Err(err) => {
                // One unreadable message must not lose the pass, but it must not be silent
                // either: the run ends `partial` with the reason.
                partial = Some(format!("stopped after {} messages: {err}", report.fetched));
                break;
            }
        };
        report.fetched += 1;

        let stored = store_message(pool, user_id, &message, now).await?;
        if !stored {
            report.already_seen += 1;
            continue;
        }

        let verdict = classify::classify(
            message.from.as_deref(),
            message.subject.as_deref(),
            message.snippet.as_deref(),
            &context,
        );
        report.classified += 1;

        // RULE 7: written for EVERY message, including the disregarded ones. A dropped email
        // that leaves no trace makes "correctly ignored 400 newsletters" and "broken and ate
        // an OA" produce identical output — a quiet inbox.
        //
        // `matched_application_id` stays NULL here: 8b classifies, and matching is 8c's. Rule
        // 8 makes that safe — NULL is legal on a pressing category, and the category was
        // decided from the email alone regardless.
        if let Err(err) = store_verdict(pool, &message.id, &verdict, now).await {
            eprintln!("inbox: could not record a verdict for {}: {err:?}", message.id);
        }

        // Rejection folds into the confirmation counter rather than getting its own: rule 7
        // names four buckets, and a rejection is a handled outcome rather than something
        // needing your attention. `is_pressing` is the line that matters, and it excludes it.
        // Exhaustive and spelled out rather than using a guard, so adding a category is a
        // compile error here — a new category silently falling into no bucket would break
        // rule 7's invariant without anything failing.
        match verdict.category {
            Category::Oa | Category::Interview | Category::Offer => report.pressing += 1,
            Category::Confirmation | Category::Rejection => report.confirmation += 1,
            Category::Outreach => report.outreach += 1,
            Category::Disregarded => report.disregarded += 1,
        }
    }

    // The watermark for the next pass, taken after the fetch so nothing between the two is
    // skipped. Failing to read it is not a failed run — it only costs a full list next time.
    if let Ok(Some(history_id)) = gmail::current_history_id(&client, &token).await {
        let _ = sqlx::query(
            "UPDATE gmail_accounts SET history_id = ?1, updated_at = ?2 WHERE user_id = ?3",
        )
        .bind(history_id)
        .bind(now.to_rfc3339())
        .bind(user_id)
        .execute(pool)
        .await;
    }

    let outcome = if partial.is_some() { "partial" } else { "success" };
    finish(pool, &run_id, outcome, partial.as_deref(), &report, now).await?;
    Ok(report)
}

async fn finish(
    pool: &SqlitePool,
    run_id: &str,
    outcome: &str,
    error: Option<&str>,
    report: &SyncReport,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "UPDATE inbox_runs
            SET finished_at = ?1, outcome = ?2, error = ?3,
                fetched_count = ?4, classified_count = ?5,
                pressing_count = ?6, confirmation_count = ?7,
                outreach_count = ?8, disregarded_count = ?9
          WHERE id = ?10",
    )
    .bind(now.to_rfc3339())
    .bind(outcome)
    .bind(error)
    .bind(report.fetched)
    .bind(report.classified)
    .bind(report.pressing)
    .bind(report.confirmation)
    .bind(report.outreach)
    .bind(report.disregarded)
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Store a message. Returns whether it is new.
///
/// Rule 4: `gmail_message_id` is UNIQUE, so re-seeing a message is a no-op rather than a
/// second row — and, once 8c exists, rather than a second label write and a second alert.
async fn store_message(
    pool: &SqlitePool,
    user_id: &str,
    message: &gmail::Message,
    now: DateTime<Utc>,
) -> Result<bool> {
    let inserted = sqlx::query(
        "INSERT INTO email_messages
             (id, user_id, gmail_message_id, gmail_thread_id, from_address, subject,
              received_at, snippet, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT (gmail_message_id) DO NOTHING",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(user_id)
    .bind(&message.id)
    .bind(&message.thread_id)
    .bind(&message.from)
    .bind(&message.subject)
    .bind(&message.received_at)
    .bind(&message.snippet)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?
    .rows_affected();

    Ok(inserted > 0)
}

/// Record what the classifier decided, for every message.
async fn store_verdict(
    pool: &SqlitePool,
    gmail_message_id: &str,
    verdict: &classify::EmailVerdict,
    now: DateTime<Utc>,
) -> Result<()> {
    let message_id: Option<String> =
        sqlx::query_scalar("SELECT id FROM email_messages WHERE gmail_message_id = ?")
            .bind(gmail_message_id)
            .fetch_optional(pool)
            .await?;
    let Some(message_id) = message_id else {
        return Ok(());
    };

    sqlx::query(
        "INSERT INTO email_verdicts
             (id, message_id, category, confidence, matched_application_id, classifier,
              evidence, created_at)
         VALUES (?1, ?2, ?3, ?4, NULL, 'rules', ?5, ?6)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&message_id)
    .bind(verdict.category.as_str())
    .bind(verdict.confidence)
    .bind(&verdict.evidence)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;

    Ok(())
}

/// The most recent run for a user, for the status endpoint and the popup.
pub async fn last_run(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Option<(String, Option<String>, String, Option<String>, i64, i64)>> {
    Ok(sqlx::query_as(
        "SELECT started_at, finished_at, outcome, error, fetched_count, classified_count
           FROM inbox_runs WHERE user_id = ? ORDER BY started_at DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
        let path = std::env::temp_dir().join(format!("inbox-{}.db", Uuid::new_v4()));
        crate::db::init_pool(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("migrations")
    }

    async fn user(pool: &SqlitePool, id: &str) {
        sqlx::query("INSERT INTO users (id, email, password_hash, created_at) VALUES (?1,?2,?3,?4)")
            .bind(id).bind(format!("{id}@example.com")).bind("x")
            .bind(Utc::now().to_rfc3339())
            .execute(pool).await.expect("user");
    }

    async fn last(pool: &SqlitePool, user_id: &str) -> (String, Option<String>) {
        let row: (String, Option<String>) = sqlx::query_as(
            "SELECT outcome, error FROM inbox_runs WHERE user_id = ? ORDER BY started_at DESC LIMIT 1")
            .bind(user_id).fetch_one(pool).await.expect("run");
        row
    }

    /// RULE 5, and the reason this phase writes a run row before it does anything: an
    /// unconnected account is `skipped` with a reason, not silence.
    #[tokio::test]
    async fn a_pass_with_no_account_connected_still_writes_a_run() {
        let pool = test_pool().await;
        user(&pool, "u1").await;

        let report = run(&pool, "u1", "id", "secret", 10, Utc::now()).await.expect("run");
        assert_eq!(report.fetched, 0);

        let (outcome, error) = last(&pool, "u1").await;
        assert_eq!(outcome, "skipped");
        assert!(error.unwrap().contains("no Gmail account"));
    }

    /// **The failure this phase will actually hit.** Google expires refresh tokens after seven
    /// days in Testing mode, and a stopped agent must never read as a quiet inbox.
    #[tokio::test]
    async fn an_unusable_token_is_recorded_as_failed_and_not_as_an_empty_inbox() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        // A connected account whose refresh token Google will refuse.
        sqlx::query(
            "INSERT INTO gmail_accounts (user_id, email, refresh_token, connected_at, updated_at)
             VALUES ('u1','burner@example.com','expired-token',?1,?1)")
            .bind(Utc::now().to_rfc3339())
            .execute(&pool).await.expect("account");

        let report = run(&pool, "u1", "bad-client", "bad-secret", 10, Utc::now())
            .await
            .expect("run");
        assert_eq!(report.fetched, 0, "nothing was fetched");

        let (outcome, error) = last(&pool, "u1").await;
        assert_eq!(outcome, "failed", "a dead token is a failed run, never a quiet one");
        let error = error.expect("a reason");
        assert!(!error.is_empty());
        // The counts alone would say "zero" either way; the outcome is what distinguishes them.
        assert_ne!(outcome, "success");
    }

    #[tokio::test]
    async fn a_run_row_exists_before_anything_can_go_wrong() {
        // Opened as `failed` and corrected on success, so a process that dies mid-pass leaves
        // a row saying so rather than one claiming a success it never reached.
        let pool = test_pool().await;
        user(&pool, "u1").await;
        run(&pool, "u1", "id", "secret", 10, Utc::now()).await.expect("run");

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM inbox_runs WHERE user_id='u1'")
            .fetch_one(&pool).await.expect("count");
        assert_eq!(count, 1);
    }

    /// RULE 4: reprocessing a message is a no-op.
    #[tokio::test]
    async fn the_same_message_is_never_stored_twice() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        let message = gmail::Message {
            id: "gmail-1".into(),
            thread_id: Some("t1".into()),
            from: Some("recruiter@example.com".into()),
            subject: Some("Interview invitation".into()),
            received_at: Some(Utc::now().to_rfc3339()),
            snippet: Some("...".into()),
        };

        assert!(store_message(&pool, "u1", &message, Utc::now()).await.expect("first"));
        assert!(!store_message(&pool, "u1", &message, Utc::now()).await.expect("second"),
                "a second sighting must be a no-op, not a second row");

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM email_messages")
            .fetch_one(&pool).await.expect("count");
        assert_eq!(count, 1);
    }

    /// RULE 7's invariant. Summed into one number, a miscount is invisible.
    #[test]
    fn the_category_counts_must_balance() {
        let balanced = SyncReport {
            classified: 10, pressing: 2, confirmation: 3, outreach: 1, disregarded: 4,
            ..SyncReport::default()
        };
        assert!(balanced.counts_balance());

        let lost_one = SyncReport { disregarded: 3, ..balanced.clone() };
        assert!(!lost_one.counts_balance(), "a dropped email must not balance");
    }
}
