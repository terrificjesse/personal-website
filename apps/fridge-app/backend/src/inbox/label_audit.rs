//! `inbox audit-labels`: what Gmail actually shows on each message, against its stored verdict.
//!
//! **Read-only.** It lists labels and reads each message's label ids, and that is all.
//!
//! # Why `reclassify --labels` was not enough
//!
//! That command compares a verdict with `email_messages.labels_applied`, which is this agent's
//! own record of the last label it put on. It cannot see a label left behind when a removal
//! failed or when a label was added before removals existed, and it has no dry run, so it
//! cannot be used to look. The mailbox is the thing the owner reads, so this reads the mailbox.
//!
//! # What it compares
//!
//! Only labels this agent projects (`labels::is_ours`). A label you made yourself is not ours and
//! is never reported.
//!
//! # What it will not print
//!
//! Sealed and held-out messages are counted and never named. Reading them would spend them, and a
//! count is enough to say whether their labels are consistent.

use anyhow::{bail, Result};
use std::collections::{BTreeSet, HashMap};

use sqlx::SqlitePool;

use super::classify::Category;
use super::labelling::HELD_OUT_SINCE;
use super::{diagnose, gmail, labels, reclassify};

/// How the labels on a message differ from what its verdict projects.
#[derive(Debug, PartialEq, Eq)]
struct Mismatch {
    /// The label the verdict calls for, when it is not on the message.
    missing: Option<String>,
    /// Labels of ours on the message that the verdict does not call for.
    extra: Vec<String>,
}

/// `None` when the message carries exactly what its verdict projects.
fn compare(expected: Option<&str>, on_message: &[String]) -> Option<Mismatch> {
    let missing = expected
        .filter(|label| !on_message.iter().any(|on| on == label))
        .map(str::to_string);
    let extra: Vec<String> = on_message
        .iter()
        .filter(|on| Some(on.as_str()) != expected)
        .cloned()
        .collect();
    if missing.is_none() && extra.is_empty() {
        None
    } else {
        Some(Mismatch { missing, extra })
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    gmail_message_id: String,
    received_at: Option<String>,
    from_address: Option<String>,
    subject: Option<String>,
    category: String,
}

async fn labels_politely(
    client: &reqwest::Client,
    token: &str,
    gmail_id: &str,
) -> Result<Vec<String>> {
    tokio::time::sleep(reclassify::FETCH_PACING).await;
    match gmail::message_label_ids(client, token, gmail_id).await {
        Err(error) if format!("{error:?}").contains("rateLimitExceeded") => {
            tokio::time::sleep(reclassify::RATE_LIMIT_BACKOFF).await;
            gmail::message_label_ids(client, token, gmail_id).await
        }
        other => other,
    }
}

pub async fn main(pool: &SqlitePool) -> Result<()> {
    let Some((client, token)) = reclassify::gmail_session(pool).await else {
        bail!("no live Gmail session: this reads the labels in Gmail. Reconnect and re-run.");
    };

    let names_by_id: HashMap<String, String> = labels::existing(&client, &token)
        .await?
        .into_iter()
        .filter(|(name, _)| labels::is_ours(name))
        .map(|(name, id)| (id, name))
        .collect();

    let rows: Vec<Row> = sqlx::query_as(
        "SELECT m.gmail_message_id, m.received_at, m.from_address, m.subject, v.category
           FROM email_messages m
           JOIN email_verdicts v ON v.id = (
               SELECT id FROM email_verdicts
                WHERE message_id = m.id ORDER BY created_at DESC, rowid DESC LIMIT 1)
          ORDER BY m.received_at",
    )
    .fetch_all(pool)
    .await?;

    let sealed: BTreeSet<String> = diagnose::sealed_ids();
    let (mut agree, mut wrong, mut unread, mut held_back) = (0usize, 0usize, 0usize, 0usize);

    for row in &rows {
        let Some(category) = Category::parse(&row.category) else {
            bail!("unknown stored category {:?}", row.category);
        };
        let ids = match labels_politely(&client, &token, &row.gmail_message_id).await {
            Ok(ids) => ids,
            Err(error) => {
                unread += 1;
                eprintln!("audit-labels: could not read a message: {error}");
                continue;
            }
        };
        let mut on_message: Vec<String> =
            ids.iter().filter_map(|id| names_by_id.get(id).cloned()).collect();
        on_message.sort();

        let Some(mismatch) = compare(labels::label_for(category), &on_message) else {
            agree += 1;
            continue;
        };
        wrong += 1;

        let received = row.received_at.as_deref().unwrap_or("");
        let hidden = sealed.contains(&row.gmail_message_id) || received >= HELD_OUT_SINCE;
        if hidden {
            held_back += 1;
            continue;
        }
        println!(
            "{}  verdict {:<13} missing {:<16} extra {}",
            received.chars().take(10).collect::<String>(),
            category.as_str(),
            mismatch.missing.as_deref().unwrap_or("-"),
            if mismatch.extra.is_empty() { "-".to_string() } else { mismatch.extra.join(", ") },
        );
        println!(
            "    {} | {}",
            row.from_address.as_deref().unwrap_or("").chars().take(44).collect::<String>(),
            row.subject.as_deref().unwrap_or("").chars().take(72).collect::<String>(),
        );
    }

    println!();
    println!("{} messages: {agree} agree, {wrong} disagree, {unread} could not be read.", rows.len());
    println!("{held_back} of the disagreements are sealed or held out, so counted and not named.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn the_right_label_alone_is_not_reported() {
        assert_eq!(compare(Some("Hunt/OA"), &on(&["Hunt/OA"])), None);
    }

    #[test]
    fn a_disregarded_message_with_nothing_on_it_is_not_reported() {
        assert_eq!(compare(None, &[]), None);
    }

    #[test]
    fn a_label_on_a_disregarded_message_is_extra() {
        assert_eq!(
            compare(None, &on(&["Hunt/Outreach"])),
            Some(Mismatch { missing: None, extra: on(&["Hunt/Outreach"]) })
        );
    }

    /// The mailbox state `labels_applied` cannot show: the record says Hunt/Rejected, and it is
    /// there, but so is the label it replaced.
    #[test]
    fn a_stale_label_beside_the_right_one_is_extra() {
        assert_eq!(
            compare(Some("Hunt/Rejected"), &on(&["Hunt/Confirmed", "Hunt/Rejected"])),
            Some(Mismatch { missing: None, extra: on(&["Hunt/Confirmed"]) })
        );
    }

    #[test]
    fn a_missing_label_is_reported_with_what_is_there_instead() {
        assert_eq!(
            compare(Some("Hunt/OA"), &on(&["Hunt/Confirmed"])),
            Some(Mismatch {
                missing: Some("Hunt/OA".to_string()),
                extra: on(&["Hunt/Confirmed"]),
            })
        );
    }

    /// The promise in the module doc, checked against the source: nothing here writes.
    #[test]
    fn this_module_makes_no_write_calls_to_gmail() {
        let source = include_str!("label_audit.rs");
        let code = source.split("mod tests").next().expect("the module above its tests");
        for forbidden in [".post(", "ensure_all", "remove_superseded", "relabel", "labels::apply"] {
            let found = code
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .any(|line| line.contains(forbidden));
            assert!(!found, "label_audit.rs contains {forbidden:?}, and it promises to be read-only");
        }
    }
}
