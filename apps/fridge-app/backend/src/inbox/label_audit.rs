//! `inbox audit-labels`: what Gmail actually shows on each message, against its stored verdict.
//!
//! **Read-only unless `--apply`**, and then it only ever **removes** a label, and only one it can
//! show this agent put there: an earlier verdict projected it (`SupersededLabel::
//! left_by_an_earlier_verdict`), or `labels_applied` records it. Anything else it reports as
//! "no record" and leaves for you. It never adds a label; `reclassify --apply` does that.
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

use anyhow::{Result, bail};
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
    id: String,
    gmail_message_id: String,
    received_at: Option<String>,
    from_address: Option<String>,
    subject: Option<String>,
    category: String,
    labels_applied: Option<String>,
}

/// Why we may take a stray label off, if we may.
fn provenance(
    stray: &str,
    verdicts: &[Category],
    now: Category,
    recorded: Option<&str>,
) -> Option<(labels::SupersededLabel, &'static str)> {
    if let Some(label) = labels::SupersededLabel::left_by_an_earlier_verdict(stray, verdicts, now) {
        return Some((label, "an earlier verdict"));
    }
    if recorded == Some(stray)
        && let Some(label) = labels::SupersededLabel::withdrawn(recorded)
    {
        return Some((label, "the record"));
    }
    None
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

pub async fn main(pool: &SqlitePool, args: &[String]) -> Result<()> {
    let apply = match args {
        [] => false,
        [flag] if flag == "--apply" => true,
        _ => bail!("usage: inbox audit-labels [--apply]"),
    };
    let Some((client, token)) = reclassify::gmail_session(pool).await else {
        bail!("no live Gmail session: this reads the labels in Gmail. Reconnect and re-run.");
    };

    let ids_by_name: HashMap<String, String> = labels::existing(&client, &token)
        .await?
        .into_iter()
        .filter(|(name, _)| labels::is_ours(name))
        .collect();
    let names_by_id: HashMap<String, String> =
        ids_by_name.iter().map(|(name, id)| (id.clone(), name.clone())).collect();

    let rows: Vec<Row> = sqlx::query_as(
        "SELECT m.id, m.gmail_message_id, m.received_at, m.from_address, m.subject, v.category,
                m.labels_applied
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
    let (mut removable, mut removed, mut no_record) = (0usize, 0usize, 0usize);

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
        } else {
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

        let verdicts: Vec<Category> =
            sqlx::query_scalar::<_, String>("SELECT category FROM email_verdicts WHERE message_id = ?")
                .bind(&row.id)
                .fetch_all(pool)
                .await?
                .iter()
                .filter_map(|category| Category::parse(category))
                .collect();
        for stray in &mismatch.extra {
            let Some((label, because)) =
                provenance(stray, &verdicts, category, row.labels_applied.as_deref())
            else {
                no_record += 1;
                if !hidden {
                    println!("    {stray}: no record that we added it, so it is left for you");
                }
                continue;
            };
            removable += 1;
            if !apply {
                if !hidden {
                    println!("    {stray}: ours, by {because}; --apply would remove it");
                }
                continue;
            }
            let Some(label_id) = ids_by_name.get(label.name()) else {
                continue;
            };
            tokio::time::sleep(reclassify::FETCH_PACING).await;
            labels::remove_superseded(&client, &token, &row.gmail_message_id, label_id, &label)
                .await?;
            removed += 1;
            if !hidden {
                println!("    {stray}: ours, by {because}; removed");
            }
        }
        // The record must name what is on the message now, or the next correction compares
        // against a label that is gone. Only when the verdict's own label is in place.
        if apply && mismatch.missing.is_none() {
            sqlx::query("UPDATE email_messages SET labels_applied = ?2 WHERE id = ?1")
                .bind(&row.id)
                .bind(labels::label_for(category))
                .execute(pool)
                .await?;
        }
    }

    println!();
    println!("{} messages: {agree} agree, {wrong} disagree, {unread} could not be read.", rows.len());
    println!("{held_back} of the disagreements are sealed or held out, so counted and not named.");
    println!(
        "Stray labels: {removable} provably ours ({removed} removed), {no_record} with no record."
    );
    if !apply && removable > 0 {
        println!("Re-run with --apply to remove the ones that are provably ours.");
    }
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

    /// The promise in the module doc, checked against the source: this can take a label off,
    /// and nothing here can put one on.
    #[test]
    fn this_module_never_adds_a_label() {
        let source = include_str!("label_audit.rs");
        let code = source.split("mod tests").next().expect("the module above its tests");
        for forbidden in [".post(", "ensure_all", "relabel", "labels::apply", "labels::create"] {
            let found = code
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .any(|line| line.contains(forbidden));
            assert!(!found, "label_audit.rs contains {forbidden:?}, and it promises never to add a label");
        }
    }

    #[test]
    fn a_stray_an_earlier_verdict_projected_is_ours() {
        let found = provenance(
            "Hunt/Interview",
            &[Category::Interview, Category::Confirmation],
            Category::Confirmation,
            Some("Hunt/Confirmed"),
        );
        assert_eq!(found.map(|(label, because)| (label.name().to_string(), because)),
            Some(("Hunt/Interview".to_string(), "an earlier verdict")));
    }

    #[test]
    fn a_stray_the_record_names_is_ours() {
        let found = provenance("Hunt/Outreach", &[Category::Disregarded], Category::Disregarded, Some("Hunt/Outreach"));
        assert_eq!(found.map(|(_, because)| because), Some("the record"));
    }

    /// The case the owner has to settle: on the message, ours by name, and nothing says we put it there.
    #[test]
    fn a_stray_with_no_record_is_left_alone() {
        assert!(provenance("Hunt/Outreach", &[Category::Disregarded], Category::Disregarded, None).is_none());
    }
}
