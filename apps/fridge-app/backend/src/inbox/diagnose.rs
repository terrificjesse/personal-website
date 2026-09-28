//! What the classifier said, what it says now, and what it cannot see.
//!
//! # Why this is not `inbox reclassify`
//!
//! A dry-run reclassification prints the messages whose category *changed*. That is a diff, and
//! a diff answers "what would writing do" — not "is this right". It also drops every message
//! whose body would not fetch (`reclassify.rs`'s `continue`, which is correct there and wrong
//! here) and prints no totals, so a bucket that is quietly half wrong looks identical to one
//! that is fine.
//!
//! # The third column is the whole point
//!
//! Every message is classified twice: once with the body, as production does since 2026-09-11,
//! and once from subject and snippet alone, as `labelset gate` and `labelset score` do — both
//! call [`classify::classify`], which hard-codes `body = None`. Wherever those two columns
//! disagree is behaviour the regression gate **cannot exercise**, and therefore a rule that can
//! be broken without any test going red. That gap is why the same misclassifications kept
//! coming back after each round of tuning, and it is measured here as a number rather than
//! argued about.
//!
//! # This module writes nothing
//!
//! No verdict rows, no ledger, no Gmail labels. Its only side effect is reading message bodies,
//! which the Gmail scope already permits and which are dropped as soon as they are classified —
//! `gmail::Message::body` is never stored, here or anywhere.
//!
//! # Sealed messages are counted, never quoted
//!
//! `labelsets/sealed-*.csv` is the held-out set for Checkpoint 13b. An agent that reads a
//! message and then tunes rules has spent it, so ids listed there have their sender, subject
//! and evidence redacted in every mode of this report. They still appear in every total: a
//! held-out set that distorted the totals would be a second problem rather than a protection.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use sqlx::SqlitePool;

use super::classify::{self, Category};
use super::labelset::{ALL_CATEGORIES, category_index, print_matrix};
use super::reclassify::{fetch_body_politely, gmail_session};

/// A stored message paired with its newest verdict.
///
/// The newest-verdict join is the correlated subquery rather than a plain join on `message_id`:
/// `email_verdicts` is append-only, so 23 of the 118 messages carry more than one row and a
/// naive join reports more messages than exist.
#[derive(sqlx::FromRow)]
struct Stored {
    gmail_message_id: String,
    received_at: Option<String>,
    from_address: Option<String>,
    subject: Option<String>,
    snippet: Option<String>,
    category: String,
    evidence: Option<String>,
}

/// What reading the body produced, kept as three cases rather than an `Option`.
///
/// "No session" and "the fetch failed" and "this message has no body" have to stay
/// distinguishable, because only the middle one means the row below is less trustworthy than
/// the rest of the report.
enum Body {
    Read(Option<String>),
    Unreachable(String),
    NoSession,
}

impl Body {
    fn text(&self) -> Option<&str> {
        match self {
            Body::Read(body) => body.as_deref(),
            Body::Unreachable(_) | Body::NoSession => None,
        }
    }
}

/// One message, classified three ways.
struct Finding {
    received_at: String,
    from: String,
    subject: String,
    stored: String,
    stored_evidence: String,
    with_body: Category,
    with_body_evidence: String,
    metadata_only: Category,
    metadata_only_evidence: String,
    body: Body,
    sealed: bool,
}

impl Finding {
    /// Did the rules change their mind since this verdict was recorded?
    fn moved(&self) -> bool {
        self.with_body.as_str() != self.stored
    }

    /// Is this message's live behaviour invisible to the regression gate?
    fn invisible_to_the_gate(&self) -> bool {
        self.with_body != self.metadata_only
    }
}

/// Ids the agent has undertaken not to read. See the module docs.
fn sealed_ids() -> BTreeSet<String> {
    let mut sealed = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir("labelsets") else {
        return sealed;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("sealed-") || !name.ends_with(".csv") {
            continue;
        }
        let Ok(mut reader) = csv::Reader::from_path(entry.path()) else {
            continue;
        };
        for record in reader.records().flatten() {
            if let Some(id) = record.get(0) {
                sealed.insert(id.to_string());
            }
        }
    }
    sealed
}

async fn diagnose(pool: &SqlitePool, options: &Options) -> Result<Vec<Finding>> {
    let sealed = sealed_ids();
    let session = gmail_session(pool).await;
    if session.is_none() {
        eprintln!(
            "diagnose: no live Gmail session. The body column will be empty for every message, \
             which makes this report a comparison of the rules against themselves. Reconnect \
             before drawing conclusions from it."
        );
    }

    let companies = super::sync::known_companies(pool).await;
    let context = classify::Context {
        known_companies: &companies,
    };

    let rows: Vec<Stored> = sqlx::query_as(
        "SELECT m.gmail_message_id, m.received_at, m.from_address, m.subject, m.snippet,
                v.category, v.evidence
           FROM email_messages m
           JOIN email_verdicts v ON v.id = (
               SELECT id FROM email_verdicts
                WHERE message_id = m.id ORDER BY created_at DESC LIMIT 1)
          ORDER BY m.received_at",
    )
    .fetch_all(pool)
    .await?;

    let mut findings = Vec::new();
    for row in rows {
        let received = row.received_at.clone().unwrap_or_default();
        if options
            .since
            .as_ref()
            .is_some_and(|since| &received < since)
        {
            continue;
        }
        if options
            .until
            .as_ref()
            .is_some_and(|until| &received >= until)
        {
            continue;
        }

        let body = match &session {
            None => Body::NoSession,
            Some((client, token)) => {
                match fetch_body_politely(client, token, &row.gmail_message_id).await {
                    Ok(message) => Body::Read(message.body),
                    // Reported, not skipped. A diagnostic that drops the messages it could not
                    // read is the quiet-inbox failure rule 7 exists to prevent, one layer up.
                    Err(error) => Body::Unreachable(format!("{error}")),
                }
            }
        };

        let with_body = classify::classify_with_body(
            row.from_address.as_deref(),
            row.subject.as_deref(),
            row.snippet.as_deref(),
            body.text(),
            &context,
        );
        let metadata_only = classify::classify(
            row.from_address.as_deref(),
            row.subject.as_deref(),
            row.snippet.as_deref(),
            &context,
        );

        findings.push(Finding {
            received_at: received,
            from: row.from_address.unwrap_or_default(),
            subject: row.subject.unwrap_or_default(),
            stored: row.category,
            stored_evidence: row.evidence.unwrap_or_default(),
            with_body: with_body.category,
            with_body_evidence: with_body.evidence,
            metadata_only: metadata_only.category,
            metadata_only_evidence: metadata_only.evidence,
            body,
            sealed: sealed.contains(&row.gmail_message_id),
        });
    }

    Ok(findings)
}

#[derive(Default)]
struct Options {
    since: Option<String>,
    until: Option<String>,
    category: Option<String>,
    full: bool,
    moved_only: bool,
    /// Print this many characters of the body. Off by default.
    ///
    /// The body is the one input nothing stores, so when a verdict turns on it there is
    /// otherwise no artifact to check the reasoning against — which is how "the negative gate
    /// misfired" stays a guess. Sealed messages are exempt like everything else.
    show_body: usize,
}

fn tally(findings: &[Finding], pick: impl Fn(&Finding) -> &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for finding in findings {
        *counts.entry(pick(finding).to_string()).or_insert(0) += 1;
    }
    counts
}

fn print_tally(caption: &str, counts: &BTreeMap<String, usize>) {
    print!("{caption:<22}");
    for category in ALL_CATEGORIES {
        print!("{:>8}", counts.get(category.as_str()).copied().unwrap_or(0));
    }
    println!();
}

fn report(findings: &[Finding], options: &Options) {
    let shown: Vec<&Finding> = findings
        .iter()
        .filter(|f| {
            options
                .category
                .as_ref()
                .is_none_or(|c| f.stored == *c || f.with_body.as_str() == *c)
        })
        .filter(|f| !options.moved_only || f.moved())
        .collect();

    for finding in &shown {
        // Redaction, not omission. The row is here so the reader knows a sealed message sat at
        // this point in the sequence; what it says is 13b's to see first.
        let (from, subject) = if finding.sealed {
            ("(sealed)".to_string(), "(sealed)".to_string())
        } else {
            (
                finding.from.chars().take(48).collect(),
                finding.subject.chars().take(72).collect(),
            )
        };
        println!(
            "{}  {from}",
            &finding.received_at.chars().take(10).collect::<String>()
        );
        println!("    {subject}");
        let flag = if finding.moved() { " <- moved" } else { "" };
        println!("    stored         {:<14}{flag}", finding.stored);
        println!("    with body      {:<14}", finding.with_body.as_str());
        let gap = if finding.invisible_to_the_gate() {
            "  <- the gate cannot see this"
        } else {
            ""
        };
        println!(
            "    metadata only  {:<14}{gap}",
            finding.metadata_only.as_str()
        );
        if let Body::Unreachable(error) = &finding.body {
            println!("    body           UNREACHABLE: {error}");
        }
        if options.show_body > 0 && !finding.sealed {
            match &finding.body {
                Body::Read(Some(text)) => println!(
                    "      body:     {}",
                    text.chars().take(options.show_body).collect::<String>()
                ),
                Body::Read(None) => println!("      body:     (none)"),
                Body::Unreachable(error) => println!("      body:     UNREACHABLE: {error}"),
                Body::NoSession => {}
            }
        }
        if options.full && !finding.sealed {
            println!("      stored:   {}", finding.stored_evidence);
            println!("      now:      {}", finding.with_body_evidence);
            if finding.invisible_to_the_gate() {
                println!("      no body:  {}", finding.metadata_only_evidence);
            }
        }
        println!();
    }

    println!("{} of {} messages shown", shown.len(), findings.len());
    println!();

    print!("{:<22}", "");
    for category in ALL_CATEGORIES {
        print!(
            "{:>8}",
            &category.as_str()[..category.as_str().len().min(7)]
        );
    }
    println!();
    print_tally("stored", &tally(findings, |f| &f.stored));
    print_tally("with body", &tally(findings, |f| f.with_body.as_str()));
    print_tally(
        "metadata only",
        &tally(findings, |f| f.metadata_only.as_str()),
    );
    println!();

    let mut stored_vs_now: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut body_vs_metadata: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for finding in findings {
        if let Some(stored) = Category::parse(&finding.stored) {
            *stored_vs_now
                .entry((category_index(stored), category_index(finding.with_body)))
                .or_insert(0) += 1;
        }
        *body_vs_metadata
            .entry((
                category_index(finding.with_body),
                category_index(finding.metadata_only),
            ))
            .or_insert(0) += 1;
    }
    print_matrix(
        "Stored vs now — rows are the stored verdict, columns what the rules say today:",
        &stored_vs_now,
    );
    print_matrix(
        "With body vs without — rows are production, columns are what the gate grades:",
        &body_vs_metadata,
    );

    let moved = findings.iter().filter(|f| f.moved()).count();
    let invisible = findings
        .iter()
        .filter(|f| f.invisible_to_the_gate())
        .count();
    let unreachable = findings
        .iter()
        .filter(|f| matches!(f.body, Body::Unreachable(_)))
        .count();
    let sealed = findings.iter().filter(|f| f.sealed).count();

    println!("{moved} messages would get a different verdict today than the one stored.");
    println!(
        "{invisible} messages classify differently with and without their body — that is the \
         share of this corpus the regression gate cannot exercise."
    );
    println!("{unreachable} bodies could not be read; those rows compare metadata only.");
    println!("{sealed} messages are sealed for 13b and were counted but not quoted.");
}

const USAGE: &str = "\
usage:
  inbox diagnose [--since <ISO8601>] [--until <ISO8601>]
                 [--category <name>] [--moved] [--full]

Prints, for every stored message, the verdict on record, the verdict the rules give today
with the body fetched, and the verdict they give from metadata alone — the last being what
`labelset gate` grades. Writes nothing.

  --category  keep messages whose stored OR current verdict is this one
  --moved     only messages whose verdict would change today
  --full      print the evidence string behind each verdict
  --show-body N   print the first N characters of the body (never for a sealed message)
";

pub async fn main(pool: &SqlitePool, args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let options = Options {
        since: flag(args, "--since"),
        until: flag(args, "--until"),
        category: flag(args, "--category"),
        full: args.iter().any(|a| a == "--full"),
        moved_only: args.iter().any(|a| a == "--moved"),
        show_body: flag(args, "--show-body")
            .map(|n| n.parse())
            .transpose()?
            .unwrap_or(0),
    };
    if let Some(name) = &options.category
        && Category::parse(name).is_none()
    {
        anyhow::bail!("unknown category {name:?} — one of: {}", categories_list());
    }

    let findings = diagnose(pool, &options).await?;
    report(&findings, &options);
    Ok(())
}

fn categories_list() -> String {
    ALL_CATEGORIES
        .iter()
        .map(|c| c.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn flag(args: &[String], name: &str) -> Option<String> {
    let position = args.iter().position(|a| a == name)?;
    args.get(position + 1).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(stored: &str, with_body: Category, metadata_only: Category) -> Finding {
        Finding {
            received_at: "2026-09-01T00:00:00+00:00".to_string(),
            from: "someone@example.com".to_string(),
            subject: "a subject".to_string(),
            stored: stored.to_string(),
            stored_evidence: String::new(),
            with_body,
            with_body_evidence: String::new(),
            metadata_only,
            metadata_only_evidence: String::new(),
            body: Body::Read(None),
            sealed: false,
        }
    }

    #[test]
    fn a_verdict_that_still_agrees_with_the_record_has_not_moved() {
        let unchanged = finding(
            "confirmation",
            Category::Confirmation,
            Category::Confirmation,
        );
        assert!(!unchanged.moved());
        let changed = finding("confirmation", Category::Oa, Category::Oa);
        assert!(changed.moved());
    }

    /// The defect this whole report exists to measure: production reads the body, the gate does
    /// not, so a message that needs the body to classify correctly is one no test can protect.
    #[test]
    fn a_message_that_needs_its_body_is_flagged_as_invisible_to_the_gate() {
        let needs_body = finding("disregarded", Category::Rejection, Category::Disregarded);
        assert!(needs_body.invisible_to_the_gate());

        let same_either_way = finding("rejection", Category::Rejection, Category::Rejection);
        assert!(!same_either_way.invisible_to_the_gate());
    }

    #[test]
    fn a_body_that_could_not_be_read_is_not_a_body_that_was_empty() {
        assert_eq!(Body::Read(Some("text".into())).text(), Some("text"));
        assert_eq!(Body::Read(None).text(), None);
        assert_eq!(Body::Unreachable("429".into()).text(), None);
        assert_eq!(Body::NoSession.text(), None);
    }

    /// `--category` is matched against a parsed name, so a typo fails loudly at the flag rather
    /// than quietly returning an empty report that reads like a clean corpus.
    #[test]
    fn every_category_name_the_flag_accepts_round_trips() {
        for category in ALL_CATEGORIES {
            assert_eq!(Category::parse(category.as_str()), Some(category));
        }
        assert_eq!(Category::parse("rejections"), None);
    }
}
