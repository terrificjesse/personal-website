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
//! # Labels are added, never removed
//!
//! `labels` will not remove a label, including one it added, and a test enforces that by
//! grepping its own source. So this reconciles in the only direction it can: it adds the label
//! the corrected verdict projects. **The superseded label stays on the message**, and this
//! module reports exactly which ones, because a message carrying both `Hunt/Confirmed` and
//! `Hunt/Rejected` is a state a human has to resolve and must not have to discover.

use anyhow::{Result, bail};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::classify::{self, Category};
use super::labels;

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
pub async fn reclassify(pool: &SqlitePool, dry_run: bool) -> Result<Vec<Change>> {
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
        let verdict = classify::classify(
            from.as_deref(),
            subject.as_deref(),
            snippet.as_deref(),
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

        sqlx::query(
            "UPDATE email_messages SET labels_applied = ?2, labels_applied_at = ?3 WHERE id = ?1",
        )
        .bind(&change.message_id)
        .bind(name)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;

        if let Some(old) = change.stale_label.as_deref()
            && old != name
        {
            stale.push(format!("{old} still on: {}", change.subject));
        }
    }

    Ok(stale)
}

const USAGE: &str = "\
usage:
  inbox reclassify              show what the current rules would change (writes nothing)
  inbox reclassify --apply      write the new verdicts and add the corrected Gmail labels

Verdicts are APPENDED, never overwritten: the old row stays as the record of what the
classifier decided at the time. Labels are added, never removed — see inbox::labels.
";

pub async fn main(pool: &SqlitePool, args: &[String]) -> Result<()> {
    let apply = match &args[1..] {
        [] => false,
        [flag] if flag == "--apply" => true,
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
    if !stale.is_empty() {
        println!(
            "\n{} message(s) now carry a SUPERSEDED label too. `inbox::labels` never removes a\n\
             label, so these need one manual removal each in Gmail:",
            stale.len()
        );
        for line in &stale {
            println!("  {line}");
        }
    }
    Ok(())
}
