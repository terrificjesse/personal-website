//! 8b's measurement harness: hand-label real mail, then grade the rules against it.
//!
//! This is **tooling for a checkpoint, not part of the pipeline.** Nothing here runs during a
//! sync, and nothing here writes to `email_messages`, `email_verdicts` or a mailbox. It reads
//! mail that a sync already stored, asks a human what each message actually was, and reports
//! how often the rules layer agreed.
//!
//! # What the checkpoint actually asks for
//!
//! A hand-labelled set of **every message** across ~2 weeks — not a curated set of job emails.
//! The distinction is the whole point: a curated set contains no digests, no staffing blasts
//! and no newsletters, so it cannot measure the relevance gate, which is the highest-volume
//! decision in the system. `export` therefore takes every row in the window, junk included.
//!
//! # Two numbers, never averaged into one
//!
//! The checkpoint names two failure modes and they do not cost the same:
//!
//! - **Junk leaked into `Hunt/Outreach`** — noise. Costs one glance at a folder.
//! - **Real mail disregarded** — this is the one that costs you an interview.
//!
//! A single "accuracy" figure hides the second behind the first, because disregard is the
//! high-volume path and a classifier that disregards everything scores well on volume. So they
//! are computed against separate denominators and printed separately. Same reasoning as rule
//! 7's `classified = pressing + confirmation + outreach + disregarded`: sum them into one
//! number and the defect is invisible.
//!
//! # Why the export does not show you what the classifier said
//!
//! Anchoring. If the label column arrives pre-filled with the machine's answer, a human
//! reviewing 200 messages agrees with it, and the measurement becomes a measurement of how
//! agreeable the reviewer is. The columns are the ones the classifier itself gets — sender,
//! subject, snippet — and nothing else.
//!
//! # Why scoring re-runs the classifier instead of reading `email_verdicts`
//!
//! A stored verdict is from whenever the sync that wrote it ran, under whatever rules were
//! compiled in that day. Grading against those measures a mix of every version of the rules
//! that has ever run. `score` calls [`classify::classify`] directly, so the number is always
//! about the rules in *this* binary — which is also what makes the fingerprint below mean
//! something.
//!
//! # A held-out set can only be measured once
//!
//! 8b's first attempt spent 8 of 14 messages before it started: the rules had been written by
//! reading them, so grading on them measured the tuning. Then the remaining 6 were graded, the
//! two failures were fixed against them, and those 6 became in-sample too.
//!
//! That is not carelessness, it is the natural life of a labelled set, and the only defence is
//! to make it visible. Every `score` run appends to a ledger next to the labels file recording
//! which messages it graded and the **fingerprint of the rules it graded them under**. A
//! message that was graded under a *different* fingerprint is reported as spent, because the
//! rules changed after someone saw how they did on it. Re-scoring under an unchanged
//! fingerprint is free: it is the same measurement, and it cannot have taught anyone anything.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use sqlx::SqlitePool;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::classify::{self, Category};

/// The rules layer's own source, compiled in so the fingerprint describes the binary that is
/// doing the grading rather than whatever happens to be on disk.
const RULES_SOURCE: &str = include_str!("classify.rs");

/// A short hash of the rules layer. Two `score` runs that print the same fingerprint graded
/// against identical rules and are the same measurement.
pub fn rules_fingerprint() -> String {
    let mut hash = sha2::Sha256::default();
    hash.update(RULES_SOURCE.as_bytes());
    hex::encode(hash.finalize())[..12].to_string()
}

/// Every category, for the confusion matrix's axes.
///
/// Kept in step with the enum by [`category_index`] below, which stops compiling if a variant
/// is added — a silently short list here would drop a whole row of the matrix.
const ALL_CATEGORIES: [Category; 7] = [
    Category::Confirmation,
    Category::Oa,
    Category::Interview,
    Category::Offer,
    Category::Rejection,
    Category::Outreach,
    Category::Disregarded,
];

/// Sort order for the matrix, and the compile-time guard on [`ALL_CATEGORIES`].
///
/// If you add a variant to [`Category`], this match breaks. Add it to the array too.
fn category_index(category: Category) -> usize {
    match category {
        Category::Confirmation => 0,
        Category::Oa => 1,
        Category::Interview => 2,
        Category::Offer => 3,
        Category::Rejection => 4,
        Category::Outreach => 5,
        Category::Disregarded => 6,
    }
}

fn parse_label(raw: &str) -> Option<Category> {
    ALL_CATEGORIES
        .into_iter()
        .find(|c| c.as_str() == raw.trim().to_lowercase())
}

/// One row of the labelling sheet.
///
/// These are exactly the fields [`classify::classify`] is given, plus an empty `label` for the
/// human. Deliberately no verdict column — see the module docs on anchoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Row {
    gmail_message_id: String,
    received_at: String,
    from: String,
    subject: String,
    snippet: String,
    /// Blank on export. One of [`ALL_CATEGORIES`] once a human has been through it.
    label: String,
}

/// A message as the sync stored it, before a human has said what it is.
#[derive(Debug, Clone, sqlx::FromRow)]
struct StoredMessage {
    gmail_message_id: String,
    received_at: Option<String>,
    from_address: Option<String>,
    subject: Option<String>,
    snippet: Option<String>,
}

/// A record of one `score` run, so a spent set cannot quietly be re-used as a fresh one.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LedgerEntry {
    graded_at: String,
    fingerprint: String,
    message_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Ledger {
    entries: Vec<LedgerEntry>,
}

impl Ledger {
    fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err.into()),
        }
    }

    fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Whether this message was graded under rules that have since changed.
    ///
    /// Grading is not what spends a message — *changing the rules after seeing the result* is.
    /// So a differing fingerprint is the signal, not the mere presence of an earlier entry.
    fn is_spent(&self, message_id: &str, current: &str) -> bool {
        self.entries
            .iter()
            .any(|e| e.fingerprint != current && e.message_ids.iter().any(|id| id == message_id))
    }
}

fn ledger_path(labels: &Path) -> PathBuf {
    let mut name = labels.as_os_str().to_os_string();
    name.push(".graded.json");
    PathBuf::from(name)
}

const USAGE: &str = "\
labelset — 8b's measurement harness

  labelset export --out <file.csv> [--since <ISO8601>] [--until <ISO8601>] [--force]
      Write every stored message in the window to a CSV with an empty `label` column.

  labelset score --labels <file.csv>
      Re-run the rules over the labelled rows and report how they did.

  labelset gate [--update]
      Grade the committed SYNTHETIC fixture and fail if any failure mode regressed.
      Needs no database. --update prints a fresh baseline instead of checking one.
      This grades the harness, not the classifier — see the module docs.

Labels: confirmation, oa, interview, offer, rejection, outreach, disregarded
";

/// Dispatch for the `labelset` subcommand.
pub async fn main(pool: &SqlitePool, args: &[String]) -> Result<()> {
    let Some(command) = args.first().map(String::as_str) else {
        print!("{USAGE}");
        return Ok(());
    };

    match command {
        "export" => {
            let out = flag(args, "--out")
                .ok_or_else(|| anyhow::anyhow!("export needs --out <file.csv>"))?;
            export(
                pool,
                Path::new(&out),
                flag(args, "--since"),
                flag(args, "--until"),
                args.iter().any(|a| a == "--force"),
            )
            .await
        }
        "gate" => gate(args.iter().any(|a| a == "--update")),
        "score" => {
            let labels = flag(args, "--labels")
                .ok_or_else(|| anyhow::anyhow!("score needs --labels <file.csv>"))?;
            score(pool, Path::new(&labels)).await
        }
        other => {
            print!("{USAGE}");
            bail!("unknown labelset command: {other}")
        }
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    let position = args.iter().position(|a| a == name)?;
    args.get(position + 1).cloned()
}

/// Write every stored message in the window to a labelling sheet.
///
/// "Every" is load-bearing. Filtering to the job-looking mail here would hand back a set that
/// cannot measure the relevance gate, which is the decision this whole exercise exists to
/// measure.
async fn export(
    pool: &SqlitePool,
    out: &Path,
    since: Option<String>,
    until: Option<String>,
    force: bool,
) -> Result<()> {
    // Hand-labelling is hours of human work and the file is the only copy of it. Refuse to
    // land on top of one unless told to.
    if out.exists() && !force {
        bail!(
            "{} already exists — labelling it again would overwrite the labels already in it. \
             Pass --force if that is what you want.",
            out.display()
        );
    }

    // One burner inbox is the expected case, but this writes real personal mail to a file, so
    // being vague about whose is not acceptable.
    let users: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT user_id FROM email_messages ORDER BY user_id")
            .fetch_all(pool)
            .await?;
    if users.len() > 1 {
        bail!("email_messages holds mail for {} users; this tool exports one inbox", users.len());
    }

    let rows: Vec<StoredMessage> = sqlx::query_as(
        "SELECT gmail_message_id, received_at, from_address, subject, snippet
           FROM email_messages
          WHERE (?1 IS NULL OR received_at >= ?1)
            AND (?2 IS NULL OR received_at <= ?2)
          ORDER BY received_at, gmail_message_id",
    )
    .bind(&since)
    .bind(&until)
    .fetch_all(pool)
    .await?;

    let mut writer = csv::Writer::from_path(out)?;
    for message in &rows {
        writer.serialize(Row {
            gmail_message_id: message.gmail_message_id.clone(),
            received_at: message.received_at.clone().unwrap_or_default(),
            from: message.from_address.clone().unwrap_or_default(),
            subject: message.subject.clone().unwrap_or_default(),
            snippet: message.snippet.clone().unwrap_or_default(),
            label: String::new(),
        })?;
    }
    writer.flush()?;

    println!("wrote {} messages to {}", rows.len(), out.display());
    if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
        println!(
            "window: {} .. {}",
            first.received_at.clone().unwrap_or_default(),
            last.received_at.clone().unwrap_or_default()
        );
    }
    println!();
    println!("Fill the `label` column with one of:");
    println!("  confirmation  oa  interview  offer  rejection  outreach  disregarded");
    println!();
    println!("Label what the message ACTUALLY is, not what you think the rules will say.");
    println!("Leave a row blank to exclude it; blank rows are reported, never counted.");
    println!();
    if !out.components().any(|c| c.as_os_str() == "labelsets") {
        println!(
            "NOTE: this file holds real subject lines and snippets. `labelsets/` is gitignored \n\
             for exactly that; {} is not in it.",
            out.display()
        );
    }
    Ok(())
}

/// What one set of graded messages looked like.
///
/// The two rates the checkpoint names are **separate fields with separate denominators**, and
/// there is deliberately no `accuracy` field to quote instead of them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Summary {
    graded: usize,
    agreed: usize,
    /// Truly junk, but routed to `Hunt/Outreach`. Denominator: [`Self::truly_junk`].
    junk_leaked_to_outreach: usize,
    truly_junk: usize,
    /// Truly real mail, but disregarded. Denominator: [`Self::truly_real`]. The expensive one.
    real_mail_disregarded: usize,
    truly_real: usize,
    /// Rule 8's failure: an OA/interview/offer that did not come out pressing. A subset of the
    /// above when it was disregarded, but broader — landing an interview invite in
    /// `confirmation` also loses the alert.
    pressing_missed: usize,
    truly_pressing: usize,
}

fn summarize(graded: &[(Category, Category)]) -> Summary {
    let mut s = Summary { graded: graded.len(), ..Default::default() };
    for &(truth, predicted) in graded {
        if truth == predicted {
            s.agreed += 1;
        }
        if truth == Category::Disregarded {
            s.truly_junk += 1;
            if predicted == Category::Outreach {
                s.junk_leaked_to_outreach += 1;
            }
        } else {
            s.truly_real += 1;
            if predicted == Category::Disregarded {
                s.real_mail_disregarded += 1;
            }
        }
        if truth.is_pressing() {
            s.truly_pressing += 1;
            if !predicted.is_pressing() {
                s.pressing_missed += 1;
            }
        }
    }
    s
}

fn rate(numerator: usize, denominator: usize) -> String {
    if denominator == 0 {
        return "n/a (nothing in this class)".to_string();
    }
    format!(
        "{numerator} of {denominator} ({:.0}%)",
        100.0 * numerator as f64 / denominator as f64
    )
}

fn report(title: &str, s: &Summary) {
    println!("{title}");
    println!("  graded                    {}", s.graded);
    println!("  agreed                    {}", rate(s.agreed, s.graded));
    println!("  junk leaked to Outreach   {}", rate(s.junk_leaked_to_outreach, s.truly_junk));
    println!("  REAL MAIL DISREGARDED     {}", rate(s.real_mail_disregarded, s.truly_real));
    println!("  pressing mail missed      {}", rate(s.pressing_missed, s.truly_pressing));
}

/// Grade the rules against a filled-in labelling sheet.
/// The committed regression fixture, its company list, and the baseline it is graded against.
///
/// All three are `include_str!`d rather than read from disk so `gate` cannot silently grade a
/// different set than the one in the repository — which is the whole failure a regression gate
/// is supposed to prevent, arriving through the gate itself.
const FIXTURE_SET: &str = include_str!("../../data/inbox/regression-set.csv");
const FIXTURE_COMPANIES: &str = include_str!("../../data/inbox/regression-companies.txt");
const FIXTURE_BASELINE: &str = include_str!("../../data/inbox/regression-baseline.json");

/// The counts a regression is measured against.
///
/// **Counts, not rates.** The fixture is fixed, so every denominator is fixed, and comparing
/// integers avoids asking whether 8.33% and 8.34% are the same number. Rates are still printed
/// for a human; the gate never compares them.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    graded: usize,
    agreed: usize,
    junk_leaked_to_outreach: usize,
    real_mail_disregarded: usize,
    pressing_missed: usize,
}

impl Baseline {
    fn of(summary: &Summary) -> Self {
        Baseline {
            graded: summary.graded,
            agreed: summary.agreed,
            junk_leaked_to_outreach: summary.junk_leaked_to_outreach,
            real_mail_disregarded: summary.real_mail_disregarded,
            pressing_missed: summary.pressing_missed,
        }
    }
}

/// Grade the committed fixture and refuse to pass if any failure mode got worse.
///
/// # What counts as a regression, given there is deliberately no single number
///
/// The checkpoint names two failure modes with separate denominators and refuses to average
/// them, so this refuses to average them too. Each is compared on its own, and **any one of
/// them worsening fails the gate** — there is no budget where a gain in one pays for a loss in
/// the other, because they do not cost the same thing. Losing an interview is not repaid by a
/// tidier Outreach folder.
///
/// `agreed` going down also fails, which catches a rewrite that moves messages between two
/// categories that are both "not disregarded" and would otherwise show as no change at all.
///
/// # This fixture is SYNTHETIC and it grades the harness, not the classifier
///
/// Twelve invented messages written to exercise each category once. A green gate here means
/// the rules still do what they did to these twelve strings — nothing more. It is **not** a
/// quality signal about real mail, and reading it as one would be exactly the failure this
/// project keeps finding: a number that looks like evidence and is not. The real measurement is
/// 13b's hand-labelled fortnight, and when that exists this gate should be pointed at it and
/// this fixture kept only as a fast smoke test.
/// Every way `current` is worse than `baseline`, as sentences.
///
/// Pure, so the failing direction can be tested without editing the committed baseline — a gate
/// nobody has watched go red is a gate nobody knows the failing behaviour of, which is the
/// defect this whole file exists to measure.
fn regressions(current: &Baseline, baseline: &Baseline) -> Vec<String> {
    let mut worse = Vec::new();
    if current.graded != baseline.graded {
        worse.push(format!(
            "the fixture changed size: {} rows, baseline {}. Re-baseline deliberately with \
             `labelset gate --update`, never as a reflex",
            current.graded, baseline.graded
        ));
    }
    if current.agreed < baseline.agreed {
        worse.push(format!(
            "agreement fell: {} of {}, baseline {}",
            current.agreed, current.graded, baseline.agreed
        ));
    }
    // Each on its own, and any one of them failing the gate. There is no budget where a gain in
    // one pays for a loss in another: a tidier Outreach folder does not repay a lost interview.
    for (name, now, was) in [
        ("junk leaked to Outreach", current.junk_leaked_to_outreach, baseline.junk_leaked_to_outreach),
        ("REAL MAIL DISREGARDED", current.real_mail_disregarded, baseline.real_mail_disregarded),
        ("pressing mail missed", current.pressing_missed, baseline.pressing_missed),
    ] {
        if now > was {
            worse.push(format!("{name} rose: {now}, baseline {was}"));
        }
    }
    worse
}

fn gate(update: bool) -> Result<()> {
    let companies: Vec<String> = FIXTURE_COMPANIES
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    let context = classify::Context { known_companies: &companies };

    let mut reader = csv::Reader::from_reader(FIXTURE_SET.as_bytes());
    let mut graded: Vec<(Category, Category)> = Vec::new();
    for row in reader.deserialize() {
        let row: Row = row?;
        let truth = parse_label(&row.label).ok_or_else(|| {
            anyhow::anyhow!("fixture row {} has an unknown label {:?}", row.gmail_message_id, row.label)
        })?;
        let verdict = classify::classify(
            Some(&row.from),
            Some(&row.subject),
            Some(&row.snippet),
            &context,
        );
        if truth != verdict.category {
            // Named, not counted. A gate that says "one regression" without saying which row
            // sends the reader to diff two integers.
            println!(
                "  disagrees: {} — labelled {}, classified {} :: {}",
                row.gmail_message_id,
                truth.as_str(),
                verdict.category.as_str(),
                row.subject
            );
        }
        graded.push((truth, verdict.category));
    }

    let summary = summarize(&graded);
    let current = Baseline::of(&summary);

    if update {
        let json = serde_json::to_string_pretty(&current)?;
        println!("{json}");
        println!("\n-- paste into data/inbox/regression-baseline.json --");
        return Ok(());
    }

    let baseline: Baseline = serde_json::from_str(FIXTURE_BASELINE)?;
    report("labelset gate — SYNTHETIC fixture", &summary);

    let worse = regressions(&current, &baseline);

    if worse.is_empty() {
        println!("\ngate: no failure mode regressed against the committed baseline");
        return Ok(());
    }
    for line in &worse {
        eprintln!("gate: {line}");
    }
    bail!("{} regression(s) against data/inbox/regression-baseline.json", worse.len())
}

async fn score(pool: &SqlitePool, labels: &Path) -> Result<()> {
    let mut reader = csv::Reader::from_path(labels)?;
    let mut rows: Vec<Row> = Vec::new();
    for row in reader.deserialize() {
        rows.push(row?);
    }

    // A typo in the label column would otherwise be silently dropped from the denominator,
    // which flatters every rate in the report.
    let mut unlabelled = 0usize;
    let mut labelled: Vec<(Row, Category)> = Vec::new();
    for row in rows {
        if row.label.trim().is_empty() {
            unlabelled += 1;
            continue;
        }
        match parse_label(&row.label) {
            Some(category) => labelled.push((row, category)),
            None => bail!(
                "unknown label {:?} on message {} ({:?}) — expected one of: \
                 confirmation, oa, interview, offer, rejection, outreach, disregarded",
                row.label,
                row.gmail_message_id,
                row.subject
            ),
        }
    }

    if labelled.is_empty() {
        bail!("no labelled rows in {} — fill the `label` column first", labels.display());
    }

    // The same world the sync gives the classifier, loaded the same way. A different company
    // list is a different classifier, and would make this measure something else.
    let known_companies: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT lower(company_name) FROM internship_postings
          UNION SELECT DISTINCT lower(company_name) FROM internship_applications",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let context = classify::Context { known_companies: &known_companies };

    let fingerprint = rules_fingerprint();
    let ledger_file = ledger_path(labels);
    let mut ledger = Ledger::load(&ledger_file)?;

    let mut held_out: Vec<(Category, Category)> = Vec::new();
    let mut spent: Vec<(Category, Category)> = Vec::new();
    let mut disagreements: Vec<(&Row, Category, Category)> = Vec::new();
    let mut matrix: BTreeMap<(usize, usize), usize> = BTreeMap::new();

    for (row, truth) in &labelled {
        let verdict = classify::classify(
            Some(&row.from),
            Some(&row.subject),
            Some(&row.snippet),
            &context,
        );
        let predicted = verdict.category;

        if ledger.is_spent(&row.gmail_message_id, &fingerprint) {
            spent.push((*truth, predicted));
        } else {
            held_out.push((*truth, predicted));
            *matrix
                .entry((category_index(*truth), category_index(predicted)))
                .or_insert(0) += 1;
            if *truth != predicted {
                disagreements.push((row, *truth, predicted));
            }
        }
    }

    println!("labels        {}", labels.display());
    println!("rules         {fingerprint}");
    println!("labelled      {}", labelled.len());
    if unlabelled > 0 {
        println!("unlabelled    {unlabelled} (excluded, not counted against anything)");
    }
    println!();

    if held_out.is_empty() {
        println!("NO HELD-OUT MESSAGES. Every labelled message here was already graded under");
        println!("different rules, so all of it is in-sample and none of it can measure");
        println!("anything. The next honest number needs mail that arrived since.");
        println!();
    } else {
        report("HELD-OUT — this is the measurement", &summarize(&held_out));
        println!();
    }

    if !spent.is_empty() {
        report(
            "SPENT (in-sample — graded, then the rules changed; NOT a measurement)",
            &summarize(&spent),
        );
        println!();
    }

    if !disagreements.is_empty() {
        println!("Disagreements on the held-out set:");
        for (row, truth, predicted) in &disagreements {
            let flag = if truth.is_pressing() && !predicted.is_pressing() {
                "  <-- PRESSING MAIL MISSED"
            } else {
                ""
            };
            println!(
                "  said {:<12} truth {:<12} {}{flag}",
                predicted.as_str(),
                truth.as_str(),
                row.subject
            );
            println!("       from {}", row.from);
        }
        println!();
    }

    if !held_out.is_empty() {
        println!("Confusion matrix (held-out) — rows are truth, columns are what the rules said:");
        print!("{:<14}", "");
        for c in ALL_CATEGORIES {
            print!("{:>8}", &c.as_str()[..c.as_str().len().min(7)]);
        }
        println!();
        for truth in ALL_CATEGORIES {
            print!("{:<14}", truth.as_str());
            for predicted in ALL_CATEGORIES {
                let n = matrix
                    .get(&(category_index(truth), category_index(predicted)))
                    .copied()
                    .unwrap_or(0);
                print!("{n:>8}");
            }
            println!();
        }
        println!();
    }

    // Recorded last, so a run that failed above does not burn the set.
    ledger.entries.push(LedgerEntry {
        graded_at: chrono::Utc::now().to_rfc3339(),
        fingerprint: fingerprint.clone(),
        message_ids: labelled.iter().map(|(r, _)| r.gmail_message_id.clone()).collect(),
    });
    ledger.save(&ledger_file)?;
    println!("recorded this grading in {}", ledger_file.display());
    println!("Change the rules after reading this and these messages become in-sample.");

    Ok(())
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    fn base() -> Baseline {
        Baseline { graded: 12, agreed: 11, junk_leaked_to_outreach: 0, real_mail_disregarded: 1, pressing_missed: 0 }
    }

    #[test]
    fn the_committed_fixture_passes_its_committed_baseline() {
        gate(false).expect("the fixture and its baseline must agree in the repository");
    }

    #[test]
    fn every_failure_mode_fails_the_gate_on_its_own() {
        // No budget between them: each one worsening is a failure by itself.
        for mutate in [
            |b: &mut Baseline| b.junk_leaked_to_outreach += 1,
            |b: &mut Baseline| b.real_mail_disregarded += 1,
            |b: &mut Baseline| b.pressing_missed += 1,
        ] {
            let mut current = base();
            mutate(&mut current);
            assert_eq!(regressions(&current, &base()).len(), 1, "{current:?}");
        }
    }

    #[test]
    fn a_gain_in_one_mode_does_not_pay_for_a_loss_in_another() {
        // The averaging this file refuses to do, expressed as a test: fewer leaks AND more
        // disregarded real mail is still a failure.
        let current = Baseline { junk_leaked_to_outreach: 0, real_mail_disregarded: 2, ..base() };
        let baseline = Baseline { junk_leaked_to_outreach: 3, ..base() };
        assert!(!regressions(&current, &baseline).is_empty());
    }

    #[test]
    fn a_smaller_fixture_fails_rather_than_flattering_the_rates() {
        // Deleting the rows a change breaks is the easiest way to make a gate green.
        let current = Baseline { graded: 11, agreed: 11, ..base() };
        let worse = regressions(&current, &base());
        assert!(worse.iter().any(|w| w.contains("changed size")), "{worse:?}");
    }

    #[test]
    fn improving_passes() {
        let current = Baseline { agreed: 12, real_mail_disregarded: 0, ..base() };
        assert!(regressions(&current, &base()).is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graded(pairs: &[(Category, Category)]) -> Summary {
        summarize(pairs)
    }

    #[test]
    fn the_two_rates_have_separate_denominators() {
        // Two junk messages, one of which leaked; four real ones, one of which was dropped.
        let s = graded(&[
            (Category::Disregarded, Category::Outreach),
            (Category::Disregarded, Category::Disregarded),
            (Category::Oa, Category::Disregarded),
            (Category::Interview, Category::Interview),
            (Category::Confirmation, Category::Confirmation),
            (Category::Rejection, Category::Rejection),
        ]);

        assert_eq!((s.junk_leaked_to_outreach, s.truly_junk), (1, 2));
        assert_eq!((s.real_mail_disregarded, s.truly_real), (1, 4));
        // 50% and 25% — different numbers off different bases. Sharing a denominator would
        // make both of them 1-of-6 and hide which failure actually happened.
        assert_ne!(
            s.junk_leaked_to_outreach as f64 / s.truly_junk as f64,
            s.real_mail_disregarded as f64 / s.truly_real as f64
        );
    }

    /// The reason there is no single `accuracy` number to quote.
    ///
    /// Disregard is the highest-volume path, so a classifier that disregards *everything* looks
    /// good on raw agreement while dropping every real email. The rate that matters has to stay
    /// separate or this is invisible.
    #[test]
    fn disregarding_everything_looks_fine_on_agreement_and_terrible_on_the_rate_that_matters() {
        let mut pairs = vec![(Category::Oa, Category::Disregarded)];
        pairs.extend(std::iter::repeat_n((Category::Disregarded, Category::Disregarded), 99));

        let s = graded(&pairs);

        assert_eq!(s.agreed, 99); // 99% "accurate"
        assert_eq!((s.real_mail_disregarded, s.truly_real), (1, 1)); // and 100% of real mail lost
        assert_eq!((s.pressing_missed, s.truly_pressing), (1, 1));
    }

    /// Rule 8's failure is broader than the disregard branch.
    #[test]
    fn an_interview_filed_as_confirmation_counts_as_pressing_missed() {
        let s = graded(&[(Category::Interview, Category::Confirmation)]);

        assert_eq!(s.real_mail_disregarded, 0, "it was not disregarded");
        assert_eq!(s.pressing_missed, 1, "but the alert was still lost");
    }

    #[test]
    fn a_class_with_no_members_reports_n_a_rather_than_zero_percent() {
        // Zero of zero is not 0% — printing 0% would read as a pass for something unmeasured.
        assert!(rate(0, 0).starts_with("n/a"));
        assert_eq!(rate(1, 4), "1 of 4 (25%)");
    }

    #[test]
    fn every_category_round_trips_through_its_label() {
        for category in ALL_CATEGORIES {
            assert_eq!(parse_label(category.as_str()), Some(category));
        }
        assert_eq!(parse_label("  Disregarded  "), Some(Category::Disregarded));
        assert_eq!(parse_label("disregared"), None, "a typo must not resolve");
        assert_eq!(parse_label(""), None);
    }

    #[test]
    fn all_categories_holds_every_variant() {
        let mut seen: Vec<usize> = ALL_CATEGORIES.into_iter().map(category_index).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), ALL_CATEGORIES.len(), "a variant is listed twice or missing");
    }

    /// Grading does not spend a set. *Changing the rules after grading* spends it.
    #[test]
    fn a_set_is_spent_only_once_the_rules_it_was_graded_under_have_changed() {
        let ledger = Ledger {
            entries: vec![LedgerEntry {
                graded_at: "2026-08-31T00:00:00Z".to_string(),
                fingerprint: "aaaaaaaaaaaa".to_string(),
                message_ids: vec!["m1".to_string()],
            }],
        };

        // Re-running the same measurement teaches nobody anything, so it stays held-out.
        assert!(!ledger.is_spent("m1", "aaaaaaaaaaaa"));
        // The rules moved after someone saw how they did on m1. It is in-sample now.
        assert!(ledger.is_spent("m1", "bbbbbbbbbbbb"));
        // A message that has never been graded is held-out whatever the rules say.
        assert!(!ledger.is_spent("m2", "bbbbbbbbbbbb"));
    }

    #[test]
    fn the_ledger_sits_beside_the_labels_it_describes() {
        assert_eq!(
            ledger_path(Path::new("/tmp/aug.csv")),
            PathBuf::from("/tmp/aug.csv.graded.json")
        );
    }

    /// The fingerprint has to move when the rules do, or a spent set reads as fresh forever.
    #[test]
    fn the_fingerprint_is_derived_from_the_rules_source() {
        let fingerprint = rules_fingerprint();
        assert_eq!(fingerprint.len(), 12);
        assert!(fingerprint.chars().all(|c| c.is_ascii_hexdigit()));

        let mut hash = sha2::Sha256::default();
        hash.update(RULES_SOURCE.as_bytes());
        assert_eq!(fingerprint, hex::encode(hash.finalize())[..12]);
        assert!(RULES_SOURCE.contains("const BULK"), "fingerprinting the wrong file");
    }
}
