//! Re-run the classifier over mail that was already classified, and reconcile its labels.
//!
//! # Why this exists
//!
//! The classifier only ever sees a message once. When a rule is *fixed*, every message already
//! processed keeps the verdict the broken rule gave it — and, worse, keeps the Gmail label that
//! verdict projected. On 2026-09-10 three classifier bugs were fixed and four messages in the
//! live mailbox were left holding the wrong answer: two rejections filed as confirmations, two
//! confirmations filed as interviews.
//!
//! # A re-classification appends; it does not overwrite
//!
//! `email_verdicts` has no uniqueness on `message_id`, and every reader already takes the
//! newest row (`ORDER BY created_at DESC LIMIT 1`). So a new verdict is written beside the old
//! one rather than on top of it. That keeps the record of what the classifier decided *at the
//! time*, which is the difference between "the rules improved" and "the history was edited" —
//! and Checkpoint 13's ledger depends on being able to tell those apart.
//!
//! # Labels are reconciled, and only ours are removed
//!
//! The corrected label is added, and the one it replaces is taken off — but only through
//! [`labels::SupersededLabel`], which can be built from nothing except this agent's own record
//! of what it applied. A label you added by hand is not constructible and therefore not
//! removable, which is the half of the old absolute rule worth keeping.
//!
//! **Order matters: add first, then remove.** A failure between the two leaves the message
//! carrying both labels, which is visibly wrong. The other order would leave it carrying
//! neither, which looks like the agent never saw it.

use anyhow::{Result, bail};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::classify::{self, Category};
use super::{gmail, labels};

/// A stored message paired with its newest verdict.
#[derive(sqlx::FromRow)]
struct StoredVerdict {
    id: String,
    gmail_message_id: String,
    from_address: Option<String>,
    subject: Option<String>,
    snippet: Option<String>,
    category: String,
    labels_applied: Option<String>,
}

/// One message whose category changed.
#[derive(Debug, Clone)]
pub struct Change {
    pub message_id: String,
    pub gmail_message_id: String,
    pub subject: String,
    pub was: String,
    pub now: Category,
    pub evidence: String,
    /// The label the agent recorded applying under the old verdict, if any.
    pub stale_label: Option<String>,
}

/// Re-classify every stored message and record what moved.
///
/// Writes nothing to Gmail. `dry_run` additionally writes nothing to the database, so the
/// change list can be read before anything is committed to.
/// A live Gmail session, when one can be had.
///
/// Stored messages keep only the subject and the ~200-character snippet, so re-classifying from
/// the database alone cannot see anything the original pass could not. Since 2026-09-11 the
/// classifier reads message bodies, and a re-classification that skipped them would leave
/// exactly the verdicts the body was fetched to fix — Epic Games' rejection among them.
///
/// `None` when no account is connected or the token is dead. Re-classification still runs, from
/// subject and snippet, and says so rather than silently doing less.
async fn gmail_session(pool: &SqlitePool) -> Option<(reqwest::Client, String)> {
    let user_id: String = sqlx::query_scalar("SELECT user_id FROM gmail_accounts LIMIT 1")
        .fetch_optional(pool)
        .await
        .ok()??;
    let client_id = std::env::var("GOOGLE_CLIENT_ID").ok()?;
    let client_secret = std::env::var("GOOGLE_CLIENT_SECRET").ok()?;
    let token = super::oauth::access_token(pool, &user_id, &client_id, &client_secret)
        .await
        .ok()?;
    Some((reqwest::Client::new(), token))
}

/// Gmail's per-user quota is per MINUTE, and a re-classification is a burst.
///
/// A full pass over the stored corpus is one `format=full` fetch per message, back to back, and
/// Gmail answered 403 `rateLimitExceeded` for 52 of 83 on the first attempt. That is not a
/// source pushing back on a scraper — it is our own mailbox asking us to slow down, so the
/// scraping rule about giving up fast does not apply. Pace, and retry once.
const FETCH_PACING: std::time::Duration = std::time::Duration::from_millis(150);
const RATE_LIMIT_BACKOFF: std::time::Duration = std::time::Duration::from_secs(5);

async fn fetch_body_politely(
    client: &reqwest::Client,
    token: &str,
    gmail_id: &str,
) -> Result<gmail::Message> {
    tokio::time::sleep(FETCH_PACING).await;
    match gmail::fetch_message(client, token, gmail_id).await {
        Err(error) if format!("{error:?}").contains("rateLimitExceeded") => {
            // One retry, once. A loop here would be the retry storm the root rules forbid, and
            // the failure is already reported and re-runnable.
            tokio::time::sleep(RATE_LIMIT_BACKOFF).await;
            gmail::fetch_message(client, token, gmail_id).await
        }
        other => other,
    }
}

pub async fn reclassify(pool: &SqlitePool, dry_run: bool) -> Result<Vec<Change>> {
    let session = gmail_session(pool).await;
    if session.is_none() {
        eprintln!(
            "reclassify: no live Gmail session — classifying from subject and snippet only, \
             which is what produced the verdicts being corrected. Reconnect for a full pass."
        );
    }

    let companies: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT lower(company_name) FROM internship_postings WHERE company_name IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    let context = classify::Context {
        known_companies: &companies,
    };

    let rows: Vec<StoredVerdict> = sqlx::query_as(
        "SELECT m.id, m.gmail_message_id, m.from_address, m.subject, m.snippet,
                    v.category, m.labels_applied
               FROM email_messages m
               JOIN email_verdicts v ON v.id = (
                   SELECT id FROM email_verdicts
                    WHERE message_id = m.id ORDER BY created_at DESC LIMIT 1)
              ORDER BY m.received_at",
    )
    .fetch_all(pool)
    .await?;

    let now = Utc::now();
    let mut changes = Vec::new();
    let mut unreachable_bodies: Vec<String> = Vec::new();
    for StoredVerdict {
        id,
        gmail_message_id: gmail_id,
        from_address: from,
        subject,
        snippet,
        category: was,
        labels_applied: applied,
    } in &rows
    {
        // The body, if we can reach it.
        //
        // **A failed fetch skips the message rather than falling back to the snippet.** Two
        // dry runs minutes apart disagreed — thirteen changes, then four — because Gmail
        // rate-limited the second and every failure quietly produced a snippet-only verdict.
        // That is not a smaller correction, it is a different and worse one: it would "correct"
        // a rejection back to a confirmation using less evidence than the pass was run to use.
        // Silently doing less is the failure this whole subsystem is written against.
        let body = match &session {
            Some((client, token)) => match fetch_body_politely(client, token, gmail_id).await {
                Ok(message) => message.body,
                Err(error) => {
                    unreachable_bodies.push(format!(
                        "{}: {error}",
                        subject.as_deref().unwrap_or("(no subject)")
                    ));
                    continue;
                }
            },
            None => None,
        };

        let verdict = classify::classify_with_body(
            from.as_deref(),
            subject.as_deref(),
            snippet.as_deref(),
            body.as_deref(),
            &context,
        );
        let current = format!("{:?}", verdict.category).to_lowercase();
        if &current == was {
            continue;
        }

        if !dry_run {
            sqlx::query(
                "INSERT INTO email_verdicts
                     (id, message_id, category, confidence, classifier, evidence, created_at)
                 VALUES (?1, ?2, ?3, ?4, 'rules', ?5, ?6)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(id)
            .bind(&current)
            .bind(verdict.confidence)
            // `classifier` is CHECK-constrained to 'rules' | 'llm', and this genuinely was the
            // rules layer — widening the constraint to say "rules, again" would be a migration
            // for a label. The fact that it is a re-run is carried here instead, where it is
            // read by a human rather than matched on.
            .bind(format!("reclassified {}: {}", now.date_naive(), verdict.evidence))
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;
        }

        changes.push(Change {
            message_id: id.clone(),
            gmail_message_id: gmail_id.clone(),
            subject: subject.clone().unwrap_or_default(),
            was: was.clone(),
            now: verdict.category,
            evidence: verdict.evidence.clone(),
            stale_label: applied.clone(),
        });
    }

    if !unreachable_bodies.is_empty() {
        eprintln!(
            "reclassify: {} message(s) skipped — their body could not be fetched, and a \
             snippet-only verdict would be worse evidence than the one already stored:",
            unreachable_bodies.len()
        );
        for line in unreachable_bodies.iter().take(5) {
            eprintln!("  {line}");
        }
        eprintln!("  re-run to pick them up; Gmail rate-limits a long pass.");
    }

    Ok(changes)
}

/// Add the label each corrected verdict projects.
///
/// Returns the messages now carrying a superseded label as well, which is every change whose
/// old verdict had produced one. Nothing is removed — see the module doc.
pub async fn relabel(pool: &SqlitePool, changes: &[Change]) -> Result<Vec<String>> {
    if changes.is_empty() {
        return Ok(Vec::new());
    }

    let user_id: String = sqlx::query_scalar("SELECT user_id FROM gmail_accounts LIMIT 1")
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no Gmail account is connected"))?;

    let client_id = std::env::var("GOOGLE_CLIENT_ID")?;
    let client_secret = std::env::var("GOOGLE_CLIENT_SECRET")?;
    let token = super::oauth::access_token(pool, &user_id, &client_id, &client_secret).await?;
    let client = reqwest::Client::new();
    let ids = labels::ensure_all(&client, &token).await?;

    let mut stale = Vec::new();
    for change in changes {
        let Some(name) = labels::label_for(change.now) else {
            // Disregarded projects no label. Rule 7: the inbox stays untouched, and that holds
            // just as much when the correction is *toward* disregarded.
            continue;
        };
        let Some(label_id) = ids.get(name) else {
            bail!("Gmail does not know the label {name}");
        };
        labels::apply(&client, &token, &change.gmail_message_id, label_id).await?;

        // Then take off the one it replaced, if we are the ones who put it there.
        let superseded = labels::SupersededLabel::from_recorded(change.stale_label.as_deref(), name);
        if let Some(superseded) = &superseded {
            match ids.get(superseded.name()) {
                Some(old_id) => {
                    labels::remove_superseded(
                        &client,
                        &token,
                        &change.gmail_message_id,
                        old_id,
                        superseded,
                    )
                    .await?;
                    stale.push(format!("{} removed from: {}", superseded.name(), change.subject));
                }
                // The label is recorded but Gmail has no such label any more — somebody deleted
                // it. Nothing to remove, and inventing it to delete it would be absurd.
                None => stale.push(format!(
                    "{} recorded but not in Gmail, left alone: {}",
                    superseded.name(),
                    change.subject
                )),
            }
        }

        sqlx::query(
            "UPDATE email_messages SET labels_applied = ?2, labels_applied_at = ?3 WHERE id = ?1",
        )
        .bind(&change.message_id)
        .bind(name)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
    }

    Ok(stale)
}

/// Messages whose recorded label disagrees with their newest verdict.
///
/// [`reclassify`] only reports messages whose *category* moved, which is the right unit while
/// the verdict and the label are written together. They can still come apart: a relabel that
/// half-succeeded, or — as on 2026-09-10 — a correction applied before this module could remove
/// anything, which updated `labels_applied` to the new name and left the old one on the message
/// with nothing recording it. This finds that state from the data rather than from memory.
pub async fn label_drift(pool: &SqlitePool) -> Result<Vec<Change>> {
    let rows: Vec<StoredVerdict> = sqlx::query_as(
        "SELECT m.id, m.gmail_message_id, m.from_address, m.subject, m.snippet,
                v.category, m.labels_applied
           FROM email_messages m
           JOIN email_verdicts v ON v.id = (
               SELECT id FROM email_verdicts
                WHERE message_id = m.id ORDER BY created_at DESC LIMIT 1)
          WHERE m.labels_applied IS NOT NULL
          ORDER BY m.received_at",
    )
    .fetch_all(pool)
    .await?;

    let mut drifted = Vec::new();
    for row in &rows {
        let Some(category) = Category::parse(&row.category) else {
            continue;
        };
        let Some(should_be) = labels::label_for(category) else {
            continue;
        };
        if row.labels_applied.as_deref() == Some(should_be) {
            continue;
        }
        drifted.push(Change {
            message_id: row.id.clone(),
            gmail_message_id: row.gmail_message_id.clone(),
            subject: row.subject.clone().unwrap_or_default(),
            was: row.labels_applied.clone().unwrap_or_default(),
            now: category,
            evidence: "label disagrees with the stored verdict".to_string(),
            stale_label: row.labels_applied.clone(),
        });
    }
    Ok(drifted)
}

const USAGE: &str = "\
usage:
  inbox reclassify              show what the current rules would change (writes nothing)
  inbox reclassify --apply      write the new verdicts and add the corrected Gmail labels
  inbox reclassify --labels     reconcile Gmail labels against the stored verdicts

Verdicts are APPENDED, never overwritten: the old row stays as the record of what the
classifier decided at the time. Labels are added, never removed — see inbox::labels.
";

pub async fn main(pool: &SqlitePool, args: &[String]) -> Result<()> {
    let apply = match &args[1..] {
        [] => false,
        [flag] if flag == "--apply" => true,
        [flag] if flag == "--labels" => {
            let drifted = label_drift(pool).await?;
            if drifted.is_empty() {
                println!("reclassify: every labelled message agrees with its verdict.");
                return Ok(());
            }
            println!("reclassify: {} message(s) carry the wrong label\n", drifted.len());
            for change in &drifted {
                println!(
                    "  {:<16} should be {:<16} {}",
                    change.was,
                    labels::label_for(change.now).unwrap_or("(none)"),
                    change.subject.chars().take(48).collect::<String>()
                );
            }
            let stale = relabel(pool, &drifted).await?;
            println!("\nGmail: reconciled.");
            for line in &stale {
                println!("  {line}");
            }
            return Ok(());
        }
        _ => {
            print!("{USAGE}");
            return Ok(());
        }
    };

    let changes = reclassify(pool, !apply).await?;
    if changes.is_empty() {
        println!("reclassify: nothing changes under the current rules.");
        return Ok(());
    }

    println!(
        "reclassify: {} of the stored corpus change category{}\n",
        changes.len(),
        if apply { "" } else { " (dry run — nothing written)" }
    );
    for change in &changes {
        println!(
            "  {:>12} -> {:<12} {}",
            change.was,
            format!("{:?}", change.now).to_lowercase(),
            change.subject.chars().take(56).collect::<String>()
        );
        println!("               {}", change.evidence);
    }

    if !apply {
        println!("\n  re-run with --apply to write them.");
        return Ok(());
    }

    let stale = relabel(pool, &changes).await?;
    println!("\nGmail: corrected labels added.");
    for line in &stale {
        println!("  {line}");
    }
    Ok(())
}
