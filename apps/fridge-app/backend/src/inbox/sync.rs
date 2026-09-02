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
use std::time::Duration;
use uuid::Uuid;

use crate::hunt::events::{self, EventKind, NewHuntEvent};

use crate::internships::models::ApplicationStatus;

use super::classify::{self, Category};
use super::{advance, gmail, labels, oauth};

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

    let client = reqwest::Client::new();

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

    // The applications an email can be matched to. Loaded once, like the company list — the
    // matcher is a pure function and everything it needs arrives as an argument.
    let applications: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, company_name FROM internship_applications WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let threshold = auto_apply_threshold();

    // Labels resolved once per pass, creating any that do not exist yet. Two API calls at
    // most against a hundred messages — and a first run on a fresh mailbox is where the six
    // `Hunt/` labels get created.
    let label_ids = if labelling_enabled() {
        match labels::ensure_all(&client, &token).await {
            Ok(ids) => Some(ids),
            Err(err) => {
                // Not fatal. Labelling is a projection of what we already recorded, so failing
                // to write it loses nothing that is not still in the database.
                eprintln!("inbox: could not prepare Gmail labels: {err:?}");
                None
            }
        }
    } else {
        None
    };

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

            // Seen before, but possibly never labelled: everything synced before labelling
            // existed, and anything whose label write failed once. Skipping straight past
            // these made the backlog permanently unlabellable — the label could only ever be
            // written in the same pass that first stored the message.
            //
            // Classification is a pure function and cheap, so deciding the label again costs
            // nothing. The verdict is NOT re-stored and the counts are not touched: those
            // measure new work, and inflating them would break rule 7's invariant.
            if let Some(ids) = &label_ids {
                let verdict = classify::classify(
                    message.from.as_deref(),
                    message.subject.as_deref(),
                    message.snippet.as_deref(),
                    &context,
                );
                if let Err(err) =
                    apply_label(pool, &client, &token, ids, &message, verdict.category, now).await
                {
                    eprintln!("inbox: could not label {}: {err:?}", message.id);
                }
            }
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

        // 8d: the second producer for `hunt_events`. Pressing mail — an OA, an interview, an
        // offer — becomes a desktop notification through the channel 8e built, with no second
        // pipeline and no changes to the poll or the extension.
        //
        // Rule 8 is why this does not wait for a match: an unmatched interview invite is the
        // single most costly thing this tool could drop, and the category was decided from the
        // email alone precisely so the alert does not depend on the matcher succeeding.
        // The mailbox write. Last, deliberately: everything above is recorded in our own
        // database first, so a failure here loses a label and nothing else.
        if let Some(ids) = &label_ids
            && let Err(err) =
                apply_label(pool, &client, &token, ids, &message, verdict.category, now).await
        {
            eprintln!("inbox: could not label {}: {err:?}", message.id);
        }

        // 8c, the reversible half: propose a status change, never make one silently.
        if let Err(err) =
            propose_status(pool, user_id, &message, &verdict, &applications, threshold, now).await
        {
            eprintln!("inbox: could not record a status proposal: {err:?}");
        }

        if verdict.category.is_pressing()
            && let Err(err) = raise_alert(pool, user_id, &message, &verdict, now).await
        {
                // An undelivered notification is a worse day, not lost data — the message and
                // its verdict are already stored. Same posture as the posting producer.
            eprintln!("inbox: could not raise an alert for {}: {err:?}", message.id);
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

/// Whether this agent may write labels into the mailbox.
///
/// **On by default.** The one irreversible-feeling thing here is not actually irreversible: a
/// wrong label is visible and removable in Gmail, and the granted scope withholds delete and
/// send entirely. `INBOX_APPLY_LABELS=false` turns it off without touching anything else.
fn labelling_enabled() -> bool {
    !std::env::var("INBOX_APPLY_LABELS").is_ok_and(|v| v == "false" || v == "0")
}

/// Put the category's label on a message, once.
///
/// Records the write on our side before returning. Gmail treats a repeated add as a no-op, but
/// "harmless if the remote API behaves" is weaker than not calling it twice — and the record
/// also answers "which of my emails has this touched", which is worth being able to ask about
/// the first thing in this project that changes someone else's account.
async fn apply_label(
    pool: &SqlitePool,
    client: &reqwest::Client,
    token: &str,
    label_ids: &std::collections::HashMap<String, String>,
    message: &gmail::Message,
    category: Category,
    now: DateTime<Utc>,
) -> Result<bool> {
    // Rule 7: a disregarded message is recorded and the inbox is left alone.
    let Some(name) = labels::label_for(category) else {
        return Ok(false);
    };
    let Some(label_id) = label_ids.get(name) else {
        return Ok(false);
    };

    let already: Option<Option<String>> = sqlx::query_scalar(
        "SELECT labels_applied FROM email_messages WHERE gmail_message_id = ?",
    )
    .bind(&message.id)
    .fetch_optional(pool)
    .await?;
    if let Some(Some(applied)) = already
        && applied == name
    {
        return Ok(false);
    }

    labels::apply(client, token, &message.id, label_id).await?;

    sqlx::query(
        "UPDATE email_messages SET labels_applied = ?1, labels_applied_at = ?2
          WHERE gmail_message_id = ?3",
    )
    .bind(name)
    .bind(now.to_rfc3339())
    .bind(&message.id)
    .execute(pool)
    .await?;

    Ok(true)
}

/// The confidence at which a forward status change may be applied without review.
///
/// **`None` by default, so nothing auto-applies.** The open question in
/// `apps/hunt-extension/CLAUDE.md` says to set this after 8b gives real numbers, and 8b's
/// checkpoint is not met — guessing it would be inventing the measurement it is meant to come
/// from. Set `INBOX_AUTO_APPLY_CONFIDENCE` to enable it once there is a number.
fn auto_apply_threshold() -> Option<f64> {
    std::env::var("INBOX_AUTO_APPLY_CONFIDENCE")
        .ok()
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .filter(|value| (0.0..=1.0).contains(value))
}

/// Record what this email implies about an application's status. **Rule 2.**
///
/// A misclassification must never silently rewrite the tracker, so every email-driven change
/// is a row that names the verdict that caused it. **The link from the change back to the
/// email is what makes it reversible** — without it, a false positive that flips
/// `applied -> rejected` destroys real state with no record of why.
///
/// Returns without writing when there is nothing to propose: no match, no implied status, or a
/// move rule 3 forbids.
async fn propose_status(
    pool: &SqlitePool,
    user_id: &str,
    message: &gmail::Message,
    verdict: &classify::EmailVerdict,
    applications: &[(String, String)],
    threshold: Option<f64>,
    now: DateTime<Utc>,
) -> Result<bool> {
    let Some(to_status) = advance::implied_status(verdict.category) else {
        return Ok(false);
    };
    let Some(application_id) =
        advance::match_application(verdict.company_guess.as_deref(), applications)
    else {
        // Rule 8: no match is not a failure. The email is classified, stored and — if
        // pressing — alerted regardless. It simply proposes nothing.
        return Ok(false);
    };

    let current: Option<String> = sqlx::query_scalar(
        "SELECT status FROM internship_applications WHERE id = ? AND user_id = ?",
    )
    .bind(application_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    .flatten();

    let Some(from_status) = current.as_deref().and_then(ApplicationStatus::parse) else {
        return Ok(false);
    };

    // Rule 3, applied before anything is written.
    if !advance::may_advance(from_status, to_status) {
        return Ok(false);
    }

    let verdict_id: Option<String> = sqlx::query_scalar(
        "SELECT v.id FROM email_verdicts v
           JOIN email_messages m ON m.id = v.message_id
          WHERE m.gmail_message_id = ? ORDER BY v.created_at DESC LIMIT 1",
    )
    .bind(&message.id)
    .fetch_optional(pool)
    .await?;
    let Some(verdict_id) = verdict_id else {
        return Ok(false);
    };

    let auto = advance::may_auto_apply(to_status, verdict.confidence, threshold);

    // **The proposal and the change it describes commit together.** `applied_automatically`
    // is a claim that the tracker already moved: the panel renders it as *"already applied —
    // rejecting undoes it"*, and reject then restores a status the application was never at.
    // Two statements outside a transaction make that claim survivable on its own.
    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO status_proposals
             (id, application_id, verdict_id, from_status, to_status,
              applied_automatically, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(application_id)
    .bind(&verdict_id)
    .bind(from_status.as_str())
    .bind(to_status.as_str())
    .bind(i64::from(auto))
    .bind(now.to_rfc3339())
    .execute(&mut *tx)
    .await?;

    if auto {
        // Only ever a forward, non-terminal move above the threshold — `may_auto_apply`
        // guarantees all three. `status_changed_at` moves with it, because Phase 7 made that
        // column mean "how long have I been at this stage".
        let moved = sqlx::query(
            "UPDATE internship_applications
                SET status = ?1, status_changed_at = ?2, updated_at = ?2
              WHERE id = ?3 AND user_id = ?4",
        )
        .bind(to_status.as_str())
        .bind(now.to_rfc3339())
        .bind(application_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

        // The status was read out of this same row moments ago, so zero here means it moved
        // or vanished underneath us. Rolling back is the only outcome that leaves the
        // proposal and the tracker agreeing.
        if moved.rows_affected() != 1 {
            anyhow::bail!(
                "auto-applying to application {application_id} matched {} rows, expected 1",
                moved.rows_affected()
            );
        }
    }

    tx.commit().await?;

    Ok(true)
}

/// Raise a desktop alert for pressing mail.
///
/// Writes to the same `hunt_events` table the posting producer uses. Two producers, one table,
/// one poll, one notification path — the shape 8e was built to accommodate, so this adds a
/// producer rather than a pipeline.
async fn raise_alert(
    pool: &SqlitePool,
    user_id: &str,
    message: &gmail::Message,
    verdict: &classify::EmailVerdict,
    now: DateTime<Utc>,
) -> Result<bool> {
    let what = match verdict.category {
        Category::Oa => "Assessment",
        Category::Interview => "Interview",
        Category::Offer => "Offer",
        // Only the pressing three reach here; anything else is a caller bug rather than a
        // category to invent a label for.
        other => return Err(anyhow::anyhow!("{other:?} is not pressing")),
    };

    let who = verdict
        .company_guess
        .clone()
        .or_else(|| sender_name(message.from.as_deref()))
        .unwrap_or_else(|| "your inbox".to_string());

    let event = NewHuntEvent {
        kind: EventKind::Email,
        // NOT NULL, always. Migration 0014's rule: a NULL user_id means "from the shared
        // posting corpus and private to nobody", and this is somebody's mail. Setting it is
        // what makes the read path's `user_id IS NULL OR user_id = :me` safe by construction.
        user_id: Some(user_id.to_string()),
        // The Gmail message id, so re-classifying the same email cannot alert twice — the
        // UNIQUE (kind, subject_id) that made the posting producer idempotent, reused.
        subject_id: message.id.clone(),
        title: format!("{what} — {who}"),
        body: message.subject.clone().unwrap_or_else(|| "(no subject)".to_string()),
        url: Some(format!("https://mail.google.com/mail/u/0/#all/{}", message.id)),
        payload: serde_json::json!({
            "category": verdict.category.as_str(),
            "confidence": verdict.confidence,
            "company_guess": verdict.company_guess,
            "evidence": verdict.evidence,
            "gmail_message_id": message.id,
            "from": message.from,
            "subject": message.subject,
        }),
    };

    events::emit(pool, &event, now).await
}

/// The display name from a `From` header, if it has one.
fn sender_name(from: Option<&str>) -> Option<String> {
    let from = from?.trim();
    let name = from.split('<').next()?.trim().trim_matches('"');
    (!name.is_empty() && name != from).then(|| name.to_string())
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

/// How often the inbox syncs when nothing overrides it.
///
/// Fifteen minutes, not the collector's six hours: a job board changes on the order of hours,
/// but an assessment invitation with a deadline does not want to sit unnoticed for one. Gmail's
/// quota is generous enough that this is not close to rude, and the sync is a capped list plus
/// metadata reads.
pub const DEFAULT_SYNC_INTERVAL_SECS: u64 = 900;

/// Start the background sync. Call once from `main`.
///
/// Spawned rather than awaited — a slow or unreachable Gmail must never delay the server
/// binding its port, exactly as with the collector.
pub fn spawn(pool: SqlitePool, config: Option<crate::auth::GoogleOAuthConfig>) {
    let Some(config) = config else {
        println!("inbox: Google OAuth not configured — no inbox sync");
        return;
    };

    let interval = match std::env::var("INBOX_SYNC_INTERVAL_SECS") {
        Err(_) => Some(Duration::from_secs(DEFAULT_SYNC_INTERVAL_SECS)),
        Ok(value) => match value.trim().parse::<u64>() {
            Ok(0) => None,
            Ok(secs) => Some(Duration::from_secs(secs)),
            Err(_) => {
                eprintln!(
                    "inbox: INBOX_SYNC_INTERVAL_SECS={value:?} is not a number — sync disabled; \
                     use 0 to disable deliberately"
                );
                None
            }
        },
    };

    let Some(interval) = interval else {
        println!("inbox: scheduled sync disabled — POST /hunt/inbox/sync only");
        return;
    };

    println!("inbox: syncing every {}s", interval.as_secs());

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;

            // Every connected account, not just one. There is one today; looping is what makes
            // that a fact about the data rather than an assumption in the code.
            let users: Vec<String> = sqlx::query_scalar("SELECT user_id FROM gmail_accounts")
                .fetch_all(&pool)
                .await
                .unwrap_or_default();

            for user_id in users {
                match run(
                    &pool,
                    &user_id,
                    &config.client_id,
                    &config.client_secret,
                    DEFAULT_MAX_MESSAGES,
                    Utc::now(),
                )
                .await
                {
                    // Quiet on a quiet inbox: a line every fifteen minutes saying "nothing"
                    // trains you to stop reading the log, which is where the failures are.
                    Ok(report) if report.classified == 0 => {}
                    Ok(report) => println!(
                        "inbox: {} new — {} pressing, {} confirmation, {} outreach, {} disregarded",
                        report.classified,
                        report.pressing,
                        report.confirmation,
                        report.outreach,
                        report.disregarded
                    ),
                    // A failed pass has already written its reason to `inbox_runs`; this is the
                    // database itself being unusable, which nothing downstream can record.
                    Err(err) => eprintln!("inbox: sync could not run at all: {err:?}"),
                }
            }
        }
    });
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


    /// 8d: pressing mail becomes an alert, and does so **without** a matched application.
    ///
    /// Rule 8's whole point. An unmatched interview invite is the costliest thing this tool
    /// could drop, so the alert must not depend on the matcher — which does not even exist yet.
    #[tokio::test]
    async fn pressing_mail_raises_an_alert_even_with_nothing_to_match_it_to() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        let now = Utc::now();
        let message = gmail::Message {
            id: "gmail-oa-1".into(),
            thread_id: None,
            from: Some("Roblox Assessment <noreply@email.roblox.com>".into()),
            subject: Some("[Action Required] Your Roblox Assessments Invitation".into()),
            received_at: Some(now.to_rfc3339()),
            snippet: Some("We're thrilled to invite you to the assessments".into()),
        };
        let verdict = classify::EmailVerdict {
            category: Category::Oa,
            confidence: 0.8,
            company_guess: Some("roblox".into()),
            evidence: "assessment invitation".into(),
        };

        assert!(raise_alert(&pool, "u1", &message, &verdict, now).await.expect("alert"));

        let row: (String, String, Option<String>) = sqlx::query_as(
            "SELECT kind, title, user_id FROM hunt_events")
            .fetch_one(&pool).await.expect("event");
        assert_eq!(row.0, "email");
        assert!(row.1.contains("Assessment"), "{}", row.1);
        assert_eq!(row.2.as_deref(), Some("u1"), "an email alert is private to its owner");
    }

    /// The same message classified twice must not alert twice — the UNIQUE (kind, subject_id)
    /// that made the posting producer idempotent, doing the same job here.
    #[tokio::test]
    async fn the_same_email_never_alerts_twice() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        let now = Utc::now();
        let message = gmail::Message {
            id: "gmail-oa-1".into(), thread_id: None,
            from: Some("a@b.com".into()), subject: Some("Interview".into()),
            received_at: None, snippet: None,
        };
        let verdict = classify::EmailVerdict {
            category: Category::Interview, confidence: 0.8,
            company_guess: None, evidence: "x".into(),
        };

        assert!(raise_alert(&pool, "u1", &message, &verdict, now).await.expect("first"));
        assert!(!raise_alert(&pool, "u1", &message, &verdict, now).await.expect("second"));

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM hunt_events")
            .fetch_one(&pool).await.expect("count");
        assert_eq!(count, 1);
    }

    /// A posting alert and an email alert can share a subject id without colliding — the
    /// reason the key is (kind, subject_id) rather than subject_id alone.
    #[tokio::test]
    async fn an_email_alert_does_not_collide_with_a_posting_alert() {
        let pool = test_pool().await;
        user(&pool, "u1").await;
        let now = Utc::now();

        crate::hunt::events::emit(&pool, &crate::hunt::events::NewHuntEvent {
            kind: crate::hunt::events::EventKind::Posting,
            user_id: None,
            subject_id: "shared-id".into(),
            title: "New at Roblox".into(), body: "b".into(), url: None,
            payload: serde_json::Value::Null,
        }, now).await.expect("posting");

        let message = gmail::Message {
            id: "shared-id".into(), thread_id: None, from: None,
            subject: Some("Offer".into()), received_at: None, snippet: None,
        };
        let verdict = classify::EmailVerdict {
            category: Category::Offer, confidence: 0.9, company_guess: None,
            evidence: "x".into(),
        };
        assert!(raise_alert(&pool, "u1", &message, &verdict, now).await.expect("email"));

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM hunt_events")
            .fetch_one(&pool).await.expect("count");
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn non_pressing_mail_raises_nothing() {
        // A confirmation is filed, not announced. The open question about whether outreach
        // should interrupt is answered "no" here, and it is one predicate to change.
        let pool = test_pool().await;
        user(&pool, "u1").await;
        let message = gmail::Message {
            id: "gmail-1".into(), thread_id: None, from: None,
            subject: Some("Thank you for applying".into()), received_at: None, snippet: None,
        };
        for category in [Category::Confirmation, Category::Outreach, Category::Disregarded,
                         Category::Rejection] {
            let verdict = classify::EmailVerdict {
                category, confidence: 0.8, company_guess: None, evidence: "x".into(),
            };
            assert!(raise_alert(&pool, "u1", &message, &verdict, Utc::now()).await.is_err(),
                    "{category:?} must not reach the alert path");
        }
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM hunt_events")
            .fetch_one(&pool).await.expect("count");
        assert_eq!(count, 0);
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
