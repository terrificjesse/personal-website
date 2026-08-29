//! Failing tests from the 2026-08-22 adversarial audit.
//!
//! **These document confirmed defects and are expected to FAIL.** Each is evidence for one
//! finding in the audit report, not a fix. Nothing here changes behaviour; delete the module
//! (and its `mod audit;` line) once the findings are triaged.

#![cfg(test)]

use super::dedup::dedup_key;
use super::models::{ClassYearRange, Location, NormalizedPosting, Season};
use super::normalize::{company_key, parse_pay};

fn posting(company_key_value: &str, title: &str, url: &str) -> NormalizedPosting {
    NormalizedPosting {
        source: "audit".into(),
        external_id: "x".into(),
        url: url.into(),
        company_name: company_key_value.into(),
        company_key: company_key_value.into(),
        title: title.into(),
        term_season: Some(Season::Summer),
        term_year: Some(2027),
        location: Location::default(),
        pay: None,
        pay_raw: None,
        class_years: ClassYearRange::default(),
        posted_at: None,
        deadline: None,
        raw_json: "{}".into(),
    }
}

// ---------------------------------------------------------------------------------------
// F1 (Critical) — Greenhouse pay is structurally unparseable
// ---------------------------------------------------------------------------------------

/// The Greenhouse adapter emits `"{currency} {amount:.2}"` with **no period**, because
/// Greenhouse has no interval field (`sources/greenhouse.rs::pay_text`, line ~297). The QC
/// parser only infers a period above `MIN_UNAMBIGUOUS_ANNUAL_USD` (50,000), and returns `None`
/// below it — so every hourly and monthly Greenhouse figure is discarded.
///
/// $45.00/hr reaches QC as `"USD 45.00"` and becomes no pay at all.
#[test]
fn f1_greenhouse_hourly_pay_survives_the_qc_pass() {
    // min_cents = 4500 -> the adapter's exact output for a $45.00/hr internship.
    let emitted = format!("USD {:.2}", 4500_f64 / 100.0);
    assert_eq!(emitted, "USD 45.00");
    assert!(
        parse_pay(&emitted).is_some(),
        "every hourly Greenhouse rate is silently dropped: parse_pay({emitted:?}) == None"
    );
}

/// The same defect for a monthly stipend: min_cents = 800000 -> `"USD 8000.00"`.
#[test]
fn f1b_greenhouse_monthly_stipend_survives_the_qc_pass() {
    let emitted = format!("USD {:.2}", 800_000_f64 / 100.0);
    assert!(
        parse_pay(&emitted).is_some(),
        "monthly stipends below the annual threshold are dropped: {emitted:?} -> None"
    );
}

/// Why no existing test catches F1: the vendored fixture contains only full-time annual
/// salaries (135,000–295,000), every one above the threshold. A fixture that cannot reach the
/// boundary cannot fail at it — the lesson `apps/fridge-app/CLAUDE.md` already records.
#[test]
fn f1c_the_greenhouse_fixture_contains_an_internship_rate() {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/data/internships/fixtures/greenhouse-board.sample.json"
    ))
    .expect("fixture");
    let below_threshold = raw
        .split("\"min_cents\":")
        .skip(1)
        .filter_map(|rest| {
            rest.trim_start()
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|digits| digits.parse::<u64>().ok())
        })
        .any(|cents| (cents as f64) / 100.0 < 50_000.0);
    assert!(
        below_threshold,
        "the fixture has no sub-50k pay figure, so it cannot exercise the parser's threshold"
    );
}

// ---------------------------------------------------------------------------------------
// F3 (High) — HTML entities are not decoded before normalization
// ---------------------------------------------------------------------------------------

/// Source text arrives HTML-escaped from several feeds. Nothing decodes it, so `&amp;` becomes
/// the literal word "amp" and `&#39;` becomes "39" inside the company key — which is the
/// identity dedup, `company_signals` and the prestige alias table all join on.
#[test]
fn f3_html_entities_do_not_corrupt_the_company_key() {
    assert_eq!(
        company_key("Ben &amp; Jerry&#39;s"),
        company_key("Ben & Jerry's"),
        "an entity-encoded company gets a different identity than the same company in plain text"
    );
}

// ---------------------------------------------------------------------------------------
// F7 (Medium) — URL path case-sensitivity splits one job into two
// ---------------------------------------------------------------------------------------

/// `dedup::canonical_url` lowercases the host but deliberately preserves path case, because
/// Ashby and Lever slugs are case-sensitive. Greenhouse slugs are not, so the same job
/// advertised with different path casing by two sources becomes two postings.
#[test]
fn f7_greenhouse_board_slug_casing_does_not_split_a_posting() {
    let lower = posting("acme", "Software Engineer Intern",
        "https://job-boards.greenhouse.io/acme/jobs/1");
    let upper = posting("acme", "Software Engineer Intern",
        "https://job-boards.greenhouse.io/ACME/jobs/1");
    assert_eq!(
        dedup_key(&lower),
        dedup_key(&upper),
        "case-variant board slugs produce two postings for one job"
    );
}

// ---------------------------------------------------------------------------------------
// F9 / F10 (Low) — pay sanity bounds
// ---------------------------------------------------------------------------------------

/// A minus sign is dropped rather than rejected, so negative pay becomes positive pay.
#[test]
fn f9_negative_pay_is_rejected_rather_than_made_positive() {
    assert_eq!(
        parse_pay("$-20/hr"),
        None,
        "negative pay parsed as {:?} instead of being rejected",
        parse_pay("$-20/hr")
    );
}

/// There is no upper bound, so a mis-scaled or hostile figure is accepted verbatim. Pay is the
/// highest-weighted ranking input (0.29), so one such row tops every pay-sorted list.
#[test]
fn f10_absurd_hourly_pay_is_rejected() {
    assert_eq!(
        parse_pay("$1000000/hr"),
        None,
        "an implausible hourly rate was accepted: {:?}",
        parse_pay("$1000000/hr")
    );
}
