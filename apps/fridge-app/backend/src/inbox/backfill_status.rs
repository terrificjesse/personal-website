//! Propose the status changes the mailbox already implies and the tracker never heard about.
//!
//! # Why this exists
//!
//! Until 2026-09-23 the only caller of [`sync::propose_status`] was the new-message branch of a
//! live sync. A verdict corrected afterwards by `inbox reclassify` appended a row to
//! `email_verdicts` and stopped there. Two separate defects then pointed the same way:
//!
//! - fifteen of the seventeen rejections in the corpus exist **only** because of a
//!   re-classification, so none of them ever produced a proposal; and
//! - the ones that did arrive by sync arrived body-blind, so nine Microsoft rejections whose
//!   refusal is only in the body were classified `confirmation`, implied `applied`, and were
//!   refused by `may_advance` as a same-status no-op.
//!
//! The Application outcomes panel reported that faithfully — 1 rejected and 2 OA against 17 and
//! 12 in the mailbox — because `routes/analytics.rs` folds `application_events` after creation
//! and there were two. Nothing downstream was broken. It was never fed.
//!
//! [`reclassify`](super::reclassify) now proposes as it corrects, so this command is for the
//! backlog that accumulated before it did. It stays afterwards as the way to answer "is the
//! tracker in step with the mail", which is a question worth being able to ask cheaply.
//!
//! # Dry by default, and it writes no status
//!
//! `--apply` writes `status_proposals` rows and nothing else. Whether any of them also *move* an
//! application is [`advance::may_auto_apply`]'s decision inside `propose_status`, and it refuses
//! every terminal status at any confidence — so a rejection is still a human's call. Rule 2 is
//! not relaxed here; it is reached for the first time.

use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;

use std::collections::HashSet;

use super::{advance, classify, sync};

/// A stored message, its newest verdict, and whether that verdict was ever acted on.
#[derive(sqlx::FromRow)]
struct Candidate {
    gmail_message_id: String,
    from_address: Option<String>,
    subject: Option<String>,
    snippet: Option<String>,
    category: String,
    confidence: Option<f64>,
    already_proposed: i64,
}

/// What happened to one candidate, kept apart rather than summed.
///
/// "Already proposed" and "no application to attach this to" are different facts, and reporting
/// them as one number is how the original gap stayed invisible for three weeks.
#[derive(Default)]
struct Tally {
    proposed: Vec<String>,
    unmatched: Vec<String>,
    already: usize,
    not_a_status: usize,
    refused: Vec<String>,
}

async fn backfill(pool: &SqlitePool, apply: bool) -> Result<Tally> {
    let user_id: String = sqlx::query_scalar("SELECT user_id FROM gmail_accounts LIMIT 1")
        .fetch_optional(pool)
        .await?
        .flatten()
        .unwrap_or_default();

    let companies = sync::known_companies(pool).await;
    let context = classify::Context { known_companies: &companies };

    let applications: Vec<advance::TrackedApplication> = sqlx::query_as(
        "SELECT id, company_name AS company, title FROM internship_applications WHERE user_id = ?",
    )
    .bind(&user_id)
    .fetch_all(pool)
    .await?;

    // The newest verdict per message, via the correlated subquery every bulk reader here uses:
    // `email_verdicts` is append-only, so a plain join reports more rows than there are
    // messages. `already_proposed` is computed in SQL so a second run is cheap and idempotent.
    let rows: Vec<Candidate> = sqlx::query_as(
        "SELECT m.gmail_message_id, m.from_address, m.subject, m.snippet,
                v.category, v.confidence,
                EXISTS (SELECT 1 FROM status_proposals sp WHERE sp.verdict_id = v.id)
                    AS already_proposed
           FROM email_messages m
           JOIN email_verdicts v ON v.id = (
               SELECT id FROM email_verdicts
                WHERE message_id = m.id ORDER BY created_at DESC LIMIT 1)
          WHERE m.user_id = ?
          ORDER BY m.received_at",
    )
    .bind(&user_id)
    .fetch_all(pool)
    .await?;

    let now = Utc::now();
    let threshold = sync::auto_apply_threshold();
    let mut tally = Tally::default();
    // Moves already spoken for in THIS run. `propose_status` suppresses a duplicate by reading
    // the rows it has written, which a dry run has not; without this the dry count exceeds what
    // `--apply` would actually do, and a dry run that overstates itself is the one thing this
    // command must not be.
    let mut claimed: HashSet<(String, &'static str)> = HashSet::new();

    for row in rows {
        let Some(category) = classify::Category::parse(&row.category) else {
            continue;
        };
        if advance::implied_status(category).is_none() {
            tally.not_a_status += 1;
            continue;
        }
        if row.already_proposed != 0 {
            tally.already += 1;
            continue;
        }

        let subject = row.subject.clone().unwrap_or_default();

        // **The category comes from the STORED verdict, never from a re-classification here.**
        //
        // `email_verdicts` has no `company_guess` column — the guess is a function of rules
        // that have moved twice this week, so freezing it would be worse — and the obvious way
        // to recover it is to classify the message again. The first version of this loop did
        // exactly that, with `classify`, which reads no body. Every Microsoft rejection came
        // back `confirmation`, because their subject and snippet say "Thank you for your
        // application!" and the refusal is four hundred characters into the body. The dry run
        // then reported them as `applied -> applied`, refused by rule 3, and the backfill
        // written to fix a body-blind bug had reintroduced it one file over.
        //
        // Re-deciding a category is `inbox reclassify`'s job and it fetches bodies to do it.
        // This command's job is to propagate what was already decided, so the re-classification
        // is used for the company guess alone.
        let guessed = classify::classify(
            row.from_address.as_deref(),
            row.subject.as_deref(),
            row.snippet.as_deref(),
            &context,
        );
        let verdict = classify::EmailVerdict {
            category,
            confidence: row.confidence.unwrap_or(0.0),
            company_guess: guessed.company_guess,
            evidence: String::new(),
        };
        let matched = advance::match_application(
            verdict.company_guess.as_deref(),
            super::untracked::role_from(row.subject.as_deref(), row.snippet.as_deref()).as_deref(),
            &applications,
        );
        // The role is shown because "matched no application" has two very different causes —
        // the company is untracked, or the role did not line up — and only the second is a
        // matcher problem. Printing the key the matcher actually compared is what turns a
        // guess about which one it is into a reading.
        let role = super::untracked::role_from(row.subject.as_deref(), row.snippet.as_deref());
        // How many applications the company has, because "company not tracked" and "several
        // applications and no readable role" are different problems with the same symptom, and
        // only the second is something a better extractor would fix.
        let company_key = verdict
            .company_guess
            .as_deref()
            .map(crate::internships::normalize::company_key)
            .unwrap_or_default();
        let at_this_company = applications
            .iter()
            .filter(|a| crate::internships::normalize::company_key(&a.company) == company_key)
            .count();
        let unmatched_line = format!(
            "  {:<12} {:<22} ({} tracked) role={}",
            category.as_str(),
            verdict.company_guess.as_deref().unwrap_or("(no company)"),
            at_this_company,
            role.as_deref().unwrap_or("(none extracted)")
        );
        let title = matched
            .and_then(|id| applications.iter().find(|a| a.id == id))
            .and_then(|a| a.title.clone())
            .unwrap_or_else(|| "(no title)".to_string());
        let line = format!("{:<26} {}", truncate(&title, 26), truncate(&subject, 44));

        // `propose_status` owns every gate — rule 3's `may_advance`, rule 2's
        // `may_auto_apply` — so a caller cannot skip one by calling it differently, and a dry
        // run asks the same gates rather than a second copy of them.
        let outcome = sync::propose_status(
            pool,
            &user_id,
            sync::MessageFacts {
                gmail_message_id: &row.gmail_message_id,
                subject: row.subject.as_deref(),
                snippet: row.snippet.as_deref(),
            },
            &verdict,
            &applications,
            threshold,
            now,
            apply,
        )
        .await?;

        // Each outcome is reported as itself. "Already at that status" is correct behaviour,
        // not a failure, and not a success either — collapsing the three is what made the
        // first dry run claim 75.
        match outcome {
            sync::Proposed::Yes { application, from, to } => {
                if claimed.insert((application, to.as_str())) {
                    tally
                        .proposed
                        .push(format!("  {:<10} -> {:<8} {line}", from.as_str(), to.as_str()));
                } else {
                    tally.already += 1;
                }
            }
            sync::Proposed::AlreadyThere { from, to } => tally
                .refused
                .push(format!("  {:<10} -> {:<8} {line}", from.as_str(), to.as_str())),
            sync::Proposed::NoMatch => tally.unmatched.push(unmatched_line.clone()),
            sync::Proposed::AlreadyPending { .. } => tally.already += 1,
            sync::Proposed::NoStatusImplied => tally.not_a_status += 1,
            sync::Proposed::Incomplete => tally.refused.push(format!("  (incomplete) {line}")),
        }
    }

    Ok(tally)
}

fn truncate(text: &str, limit: usize) -> String {
    let cleaned = text.replace('\n', " ");
    if cleaned.chars().count() <= limit {
        return cleaned;
    }
    cleaned.chars().take(limit.saturating_sub(1)).collect::<String>() + "…"
}

const USAGE: &str = "\
usage:
  inbox backfill-status [--apply]

Proposes the status changes the mailbox implies and the tracker has never been told about.
Dry by default. `--apply` writes `status_proposals` rows; it applies no status on its own —
`may_auto_apply` refuses every terminal status at any confidence, so a rejection still waits
for you to accept it.
";

pub async fn main(pool: &SqlitePool, args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let apply = args.iter().any(|a| a == "--apply");
    let tally = backfill(pool, apply).await?;

    let verb = if apply { "proposed" } else { "would propose" };
    println!("backfill-status: {} {}\n", tally.proposed.len(), verb);
    for line in &tally.proposed {
        println!("{line}");
    }

    if !tally.refused.is_empty() {
        println!(
            "\n{} refused by the status rules (already at or past that status):",
            tally.refused.len()
        );
        for line in &tally.refused {
            println!("{line}");
        }
    }

    if !tally.unmatched.is_empty() {
        println!(
            "\n{} imply a status but match no tracked application:",
            tally.unmatched.len()
        );
        for line in &tally.unmatched {
            println!("{line}");
        }
    }

    println!(
        "\n{} already proposed or already pending, {} imply no status change.",
        tally.already, tally.not_a_status
    );
    if !apply && !tally.proposed.is_empty() {
        println!("\n  re-run with --apply to write them.");
    }
    Ok(())
}
