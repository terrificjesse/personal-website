//! Ashby's public job-board API — the best source in the corpus for pay, and the only one with
//! an unambiguous interval.
//!
//! `GET https://api.ashbyhq.com/posting-api/job-board/{org}?includeCompensation=true` returns
//! `{apiVersion, jobs: […]}` — the whole board, no pagination, no filtering, no single-job
//! endpoint. Officially documented and unauthenticated (`docs/INTERNSHIP_SCRAPING.md` § F).
//!
//! # Why this source is worth polling first among the ATSs
//!
//! - **`compensation.summaryComponents[]` states its interval outright** — `"1 YEAR"`,
//!   `"1 MONTH"`, `"NONE"`. § B calls this the only unambiguous salary interval anywhere in the
//!   document. Every other source needs either a magnitude heuristic or free-text parsing.
//! - **`employmentType` has a first-class `Intern` value** — the cleanest internship signal
//!   found anywhere, and far better than title matching. It is emitted as the term hint so QC
//!   can use it; the adapter does not filter on it itself (see below).
//! - It is the only source with both a clean `isRemote` boolean and structured
//!   `secondaryLocations[]`.
//!
//! # The caveat that undercuts all of that
//!
//! **A populated `summaryComponents` does not mean a known salary.** A component can be
//! `{"compensationType":"EquityPercentage","interval":"NONE","currencyCode":null,
//! "minValue":null,"maxValue":null}` — a real record with no number in it. So a component is
//! only read when `compensationType == "Salary"` **and** `minValue` is non-null. Anything
//! looser turns "this company grants equity" into a salary of nothing.
//!
//! Intern pay specifically is spotty even here: on the board this adapter's fixture came from,
//! every `Intern` posting carried an empty `summaryComponents` while the board overall had
//! plenty. Expect absence, and never read it as zero.
//!
//! # Scopes, added in 12r
//!
//! One `source_runs` row covers 297 orgs. Before scopes, one unreachable org made the whole
//! source `Partial`, and none of the other 296 complete enumerations counted for expiry —
//! 4 runs of 19 lost that way. This adapter now reports a [`ScopeRun`] per org, built from the
//! same parse that produced the board's postings so the ids cannot drift from the rows.
//!
//! A 404 org is `Completed` with no ids, not `Failed`: "no such org" says it offers nothing,
//! which is evidence, whereas a failed read is evidence of nothing. That distinction is the
//! whole of scoped expiry in one line.
//!
//! # `robots.txt`
//!
//! `api.ashbyhq.com/robots.txt` answers **401**, not 200 or 404. Under RFC 9309 § 2.3.1.3 a 4xx
//! is "unavailable" and permits access, which is how the shared fetch layer reads it — and it
//! agrees with § F, which records this API as publicly documented and permitted.

use serde_json::Value;

use super::super::models::{RawPosting, ScopeRun};
use super::{
    BoxFuture, Source, SourceContext, SourceFetch, first_string,
    greenhouse::{completed_scope, finish},
    id_string, join_locations,
};

/// The ATS key in [`BoardDirectory`](super::BoardDirectory).
pub const ATS: &str = "ashby";

/// The only `compensationType` that names actual salary. `EquityPercentage` and friends carry
/// null amounts and would otherwise be read as a salary of nothing.
const SALARY_COMPONENT: &str = "Salary";

/// Ashby's interval value meaning "this component has no period". Not a period, and emitting
/// one for it would invent information the source explicitly declined to give.
const INTERVAL_NONE: &str = "NONE";

pub struct AshbySource;

impl Default for AshbySource {
    fn default() -> Self {
        Self::new()
    }
}

impl AshbySource {
    pub fn new() -> Self {
        AshbySource
    }
}

/// The board endpoint, with the parameter that adds compensation.
pub fn board_url(slug: &str) -> String {
    format!("https://api.ashbyhq.com/posting-api/job-board/{slug}?includeCompensation=true")
}

impl Source for AshbySource {
    fn name(&self) -> &str {
        ATS
    }

    fn description(&self) -> &str {
        "Ashby posting API — whole board per request, explicit pay interval, employmentType=Intern"
    }

    fn fetch<'a>(&'a self, ctx: &'a SourceContext) -> BoxFuture<'a, SourceFetch> {
        Box::pin(async move {
            let all_slugs = ctx.boards.slugs(ATS);
            if all_slugs.is_empty() {
                return SourceFetch::failed(
                    "no Ashby board slugs are known — harvest them from Simplify's `url` field \
                     (see simplify::extract_board_slugs) or restore \
                     data/internships/board-slugs.json",
                );
            }

            let budget = ctx.max_boards_per_run.min(all_slugs.len());
            let slugs = &all_slugs[..budget];
            let truncated = budget < all_slugs.len();

            let mut postings = Vec::new();
            let mut enumerated = 0usize;
            let mut retired = Vec::new();
            let mut failures = Vec::new();
            // One verdict per board. Boards the budget never reached get no entry: absence of
            // a row is the honest record of "no verdict", and inventing a `Failed` one would
            // claim we looked.
            let mut scopes: Vec<ScopeRun> = Vec::new();

            for slug in slugs {
                let url = board_url(slug);
                match ctx.http.get(&url).await {
                    Ok(response) => match response.json() {
                        Ok(body) => {
                            let (board, scope) = board_result(slug, &body);
                            scopes.push(scope);
                            postings.extend(board);
                            enumerated += 1;
                        }
                        Err(error) => {
                            failures.push(format!("{slug}: {error}"));
                            scopes.push(ScopeRun::failed(slug.as_str(), error.to_string()));
                        }
                    },
                    // One refusal covers the host, so every remaining board is refused too.
                    // Report no scopes with it: `Skipped` means we did not fetch this source,
                    // and per-board verdicts would contradict that.
                    Err(error) if error.is_refusal() => {
                        return SourceFetch::skipped(error.to_string());
                    }
                    // No such org. A definitive zero, not a failed read — so as a scope it is
                    // `Completed` with no ids, and anything still tagged to it advances toward
                    // expiry. That is new for this source: before scopes, a dead org's
                    // postings could only expire on a run where all 297 boards succeeded.
                    Err(error) if error.is_not_found() => {
                        retired.push(slug.clone());
                        scopes.push(ScopeRun::gone(slug.as_str()));
                        enumerated += 1;
                    }
                    Err(error) => {
                        failures.push(format!("{slug}: {error}"));
                        scopes.push(ScopeRun::failed(slug.as_str(), error.to_string()));
                    }
                }
            }

            if !retired.is_empty() {
                println!(
                    "internships: {} Ashby board(s) 404'd and should be retired: {}",
                    retired.len(),
                    retired.join(", ")
                );
            }

            finish(
                "Ashby",
                postings,
                enumerated,
                slugs.len(),
                all_slugs.len(),
                truncated,
                &failures,
            )
            .with_scopes(scopes)
        })
    }
}

/// One enumerated board: its postings and its scope verdict, from a **single** parse.
///
/// The same shape as Greenhouse's, and for the same reason — see [`completed_scope`]. Re-deriving
/// the ids by filtering the run's accumulated postings on the slug would work today and break
/// the first time two orgs share a posting.
fn board_result(slug: &str, body: &Value) -> (Vec<RawPosting>, ScopeRun) {
    let postings = parse_board(slug, body);
    let scope = completed_scope(slug, &postings);
    (postings, scope)
}

/// Turn one board response into raw postings. Pure, tested offline against the committed
/// fixture.
///
/// `slug` is the company: the payload carries no company name (§ B note 3).
pub fn parse_board(slug: &str, body: &Value) -> Vec<RawPosting> {
    let Some(Value::Array(jobs)) = body.get("jobs") else {
        return Vec::new();
    };
    jobs.iter().map(|job| parse_job(slug, job)).collect()
}

fn parse_job(slug: &str, job: &Value) -> RawPosting {
    let external_id = id_string(job, &["id"]).unwrap_or_default();

    let mut locations = Vec::new();
    if let Some(primary) = first_string(job, &["location"]) {
        locations.push(primary);
    }
    if let Some(Value::Array(secondary)) = job.get("secondaryLocations") {
        for entry in secondary {
            // Observed as bare strings; read as an object with a `location` key too, so a
            // reshape costs the extra locations rather than the posting.
            if let Some(text) = entry
                .as_str()
                .map(str::to_string)
                .or_else(|| first_string(entry, &["location", "locationName", "name"]))
            {
                locations.push(text);
            }
        }
    }

    RawPosting {
        source: ATS.to_string(),
        url: first_string(job, &["jobUrl", "applyUrl"])
            // § C records this URL shape.
            .unwrap_or_else(|| format!("https://jobs.ashbyhq.com/{slug}/{external_id}")),
        external_id,
        // No company field. § B note 3 — carry the requested slug.
        company: slug.to_string(),
        title: first_string(job, &["title"]).unwrap_or_default(),
        location_raw: join_locations(locations),
        pay_raw: pay_text(job),
        // `employmentType == "Intern"` is the cleanest internship signal in the document.
        // Handed to QC as the term hint rather than used as a filter here: `normalize` reads
        // title and term together precisely because some sources put "Internship" in an
        // employment-type field, and filtering here would shrink the denominator that makes
        // the `filtered` count a health signal.
        term_raw: first_string(job, &["employmentType"]),
        class_year_raw: None,
        // ISO 8601 with an offset (`2025-06-11T05:03:53.978+00:00`), which
        // `normalize::parse_timestamp` reads via RFC 3339.
        posted_at_raw: first_string(job, &["publishedAt"]),
        deadline_raw: None,
        description: first_string(job, &["descriptionPlain"]),
        // The only source in the document with a clean boolean here.
        remote_hint: job.get("isRemote").and_then(Value::as_bool),
        raw_json: job.to_string(),
    }
}

/// Render `compensation.summaryComponents` as a human-readable compensation string.
///
/// Three rules, all of them load-bearing:
///
/// 1. **Only `compensationType == "Salary"` with a non-null `minValue` counts.** A component
///    can be an equity percentage with every amount null; reading it as pay turns "this company
///    grants equity" into a salary of nothing.
/// 2. **The stated interval is reproduced faithfully**, because it is the only unambiguous one
///    in the corpus and losing it in stringification would throw away this source's whole
///    advantage. QC treats an explicit period as authoritative, so `"1 MONTH"` on an 11,700
///    figure resolves to a monthly stipend rather than being magnitude-guessed into an
///    implausible annual salary.
/// 3. **`"NONE"` is a value in the interval vocabulary and is not a period.** No period word is
///    emitted for it, which leaves QC's own named threshold to decide — the correct fallback,
///    and the same treatment Greenhouse gets.
///
/// Values are already in currency units (`minValue: 150000` is $150,000), verified against the
/// live response — no cents conversion applies here.
fn pay_text(job: &Value) -> Option<String> {
    let Some(Value::Array(components)) = job.pointer("/compensation/summaryComponents") else {
        return None;
    };

    let salary = components.iter().find(|component| {
        first_string(component, &["compensationType"]).as_deref() == Some(SALARY_COMPONENT)
            && component.get("minValue").and_then(Value::as_f64).is_some()
    })?;

    let min = salary.get("minValue").and_then(Value::as_f64)?;
    let max = salary.get("maxValue").and_then(Value::as_f64);
    let currency = first_string(salary, &["currencyCode"]).unwrap_or_else(|| "USD".to_string());

    let mut text = match max {
        Some(max) if max > min => format!("{currency} {min:.2} - {max:.2}"),
        _ => format!("{currency} {min:.2}"),
    };

    if let Some(period) = interval_words(first_string(salary, &["interval"]).as_deref()) {
        text.push(' ');
        text.push_str(period);
    }

    Some(text)
}

/// Ashby's interval vocabulary, as period words `normalize::detect_period` reads.
///
/// `"NONE"` yields `None` — it says the component has no period, and inventing one would
/// suppress the magnitude fallback with a fabricated fact.
fn interval_words(interval: Option<&str>) -> Option<&'static str> {
    let interval = interval?;
    if interval.eq_ignore_ascii_case(INTERVAL_NONE) {
        return None;
    }
    let lower = interval.to_ascii_lowercase();
    if lower.contains("hour") {
        Some("per hour")
    } else if lower.contains("month") {
        Some("per month")
    } else if lower.contains("year") || lower.contains("annual") {
        Some("per year")
    } else if lower.contains("week") {
        Some("per week")
    } else if lower.contains("day") {
        Some("per day")
    } else {
        None
    }
}

// ------------------------------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internships::models::{PayPeriod, SourceOutcome};
    use crate::internships::normalize::{parse_pay, parse_timestamp};

    /// Real jobs from the `Etched` board, fetched 2026-08-20. Descriptions truncated; see
    /// `data/internships/README.md`.
    const FIXTURE: &str =
        include_str!("../../../data/internships/fixtures/ashby-board.sample.json");

    fn fixture() -> Value {
        serde_json::from_str(FIXTURE).expect("the committed fixture parses")
    }

    #[test]
    fn the_board_url_asks_for_compensation() {
        // Without it the compensation object is absent and this source loses the one thing
        // that makes it worth polling first.
        assert!(board_url("Etched").contains("includeCompensation=true"));
        assert!(board_url("Etched").starts_with("https://api.ashbyhq.com/posting-api/job-board/"));
    }

    #[test]
    fn a_percent_decoded_slug_reaches_the_url_intact() {
        // Simplify's corpus stores Ashby slugs percent-encoded; `simplify::board_of` decodes
        // them, so a space must survive into the request path rather than being re-encoded
        // into something else here.
        assert!(board_url("Hippocratic AI").contains("/Hippocratic AI?"));
    }

    #[test]
    fn every_job_in_the_fixture_parses_into_an_identifiable_posting() {
        let postings = parse_board("Etched", &fixture());
        assert!(!postings.is_empty());
        for posting in &postings {
            assert_eq!(posting.source, "ashby");
            assert_eq!(posting.company, "Etched", "the payload carries no company name");
            assert!(!posting.external_id.is_empty());
            assert!(posting.url.starts_with("https://"));
            assert!(!posting.title.is_empty());
        }
    }

    #[test]
    fn the_employment_type_reaches_qc_as_the_term_hint() {
        // The cleanest internship signal in the document. Losing it here would push QC back
        // onto title matching, which § C measured at 86% false positives.
        let postings = parse_board("Etched", &fixture());
        assert!(postings.iter().any(|p| p.term_raw.as_deref() == Some("Intern")));
        assert!(postings.iter().any(|p| p.term_raw.as_deref() == Some("FullTime")));
    }

    #[test]
    fn a_stated_interval_survives_stringification_exactly() {
        // The whole advantage of this source. § A.1's real example: Ramp's one intern posting
        // carried `{"interval":"1 MONTH","minValue":11700}`. Read as annual, $11,700 is an
        // implausible salary that would rank near the bottom; read as monthly it is a strong
        // internship stipend. The magnitude heuristic would get this wrong in the bad
        // direction, and the explicit interval is what stops it running at all.
        let job = serde_json::json!({
            "compensation": { "summaryComponents": [
                { "compensationType": "Salary", "interval": "1 MONTH",
                  "currencyCode": "USD", "minValue": 11700, "maxValue": null }
            ]}
        });
        let text = pay_text(&job).expect("a salary component renders");
        assert_eq!(text, "USD 11700.00 per month");

        let parsed = parse_pay(&text).expect("and parses");
        assert_eq!(parsed.period, PayPeriod::Month);
        assert_eq!(parsed.min, 11700.0);
        assert_eq!(parsed.max, None, "a single figure is not a range");
    }

    #[test]
    fn an_annual_range_keeps_both_bounds_and_its_period() {
        let job = serde_json::json!({
            "compensation": { "summaryComponents": [
                { "compensationType": "Salary", "interval": "1 YEAR",
                  "currencyCode": "USD", "minValue": 150000, "maxValue": 275000 }
            ]}
        });
        assert_eq!(pay_text(&job).unwrap(), "USD 150000.00 - 275000.00 per year");
        let parsed = parse_pay(&pay_text(&job).unwrap()).expect("parses");
        assert_eq!(parsed.period, PayPeriod::Year);
        assert_eq!(parsed.max, Some(275000.0));
    }

    #[test]
    fn an_equity_component_is_not_read_as_a_salary_of_nothing() {
        // The caveat that undercuts everything else about this source. A real record:
        // `{"compensationType":"EquityPercentage","interval":"NONE","currencyCode":null,
        //   "minValue":null,"maxValue":null}`.
        let job = serde_json::json!({
            "compensation": { "summaryComponents": [
                { "compensationType": "EquityPercentage", "interval": "NONE",
                  "currencyCode": null, "minValue": null, "maxValue": null }
            ]}
        });
        assert_eq!(pay_text(&job), None);
    }

    #[test]
    fn a_salary_component_with_no_amount_is_not_pay() {
        let job = serde_json::json!({
            "compensation": { "summaryComponents": [
                { "compensationType": "Salary", "interval": "1 YEAR",
                  "currencyCode": "USD", "minValue": null, "maxValue": null }
            ]}
        });
        assert_eq!(pay_text(&job), None);
    }

    #[test]
    fn the_salary_component_is_found_even_behind_other_components() {
        let job = serde_json::json!({
            "compensation": { "summaryComponents": [
                { "compensationType": "EquityPercentage", "interval": "NONE",
                  "currencyCode": null, "minValue": null, "maxValue": null },
                { "compensationType": "Salary", "interval": "1 YEAR",
                  "currencyCode": "USD", "minValue": 90000, "maxValue": 110000 }
            ]}
        });
        assert_eq!(pay_text(&job).unwrap(), "USD 90000.00 - 110000.00 per year");
    }

    #[test]
    fn an_interval_of_none_emits_no_period_word() {
        // "NONE" is a value in the vocabulary, not an absence of one — and it is not a period.
        // Emitting a period here would suppress QC's magnitude fallback with an invented fact.
        assert_eq!(interval_words(Some("NONE")), None);
        assert_eq!(interval_words(Some("none")), None);
        assert_eq!(interval_words(Some("1 YEAR")), Some("per year"));
        assert_eq!(interval_words(Some("1 MONTH")), Some("per month"));
        assert_eq!(interval_words(Some("1 HOUR")), Some("per hour"));
        assert_eq!(interval_words(None), None);

        let job = serde_json::json!({
            "compensation": { "summaryComponents": [
                { "compensationType": "Salary", "interval": "NONE",
                  "currencyCode": "USD", "minValue": 120000, "maxValue": null }
            ]}
        });
        // No period word, so QC falls back to its own named threshold — which for $120,000 is
        // unambiguously annual, the same answer Greenhouse would get.
        assert_eq!(pay_text(&job).unwrap(), "USD 120000.00");
        assert_eq!(parse_pay("USD 120000.00").unwrap().period, PayPeriod::Year);
    }

    #[test]
    fn a_non_usd_component_keeps_its_own_currency() {
        let job = serde_json::json!({
            "compensation": { "summaryComponents": [
                { "compensationType": "Salary", "interval": "1 YEAR",
                  "currencyCode": "GBP", "minValue": 60000, "maxValue": 70000 }
            ]}
        });
        assert_eq!(parse_pay(&pay_text(&job).unwrap()).unwrap().currency, "GBP");
    }

    #[test]
    fn interns_with_no_compensation_report_no_pay_rather_than_zero() {
        // On the real board this fixture came from, every `Intern` posting carried an empty
        // `summaryComponents` while the board overall had plenty. Absence is the common case
        // and must never read as zero.
        let postings = parse_board("Etched", &fixture());
        let interns: Vec<&RawPosting> = postings
            .iter()
            .filter(|p| p.term_raw.as_deref() == Some("Intern"))
            .collect();
        assert!(!interns.is_empty());
        for intern in interns {
            assert!(
                intern.pay_raw.is_none(),
                "an empty summaryComponents must yield None, got {:?}",
                intern.pay_raw
            );
        }
    }

    #[test]
    fn the_remote_flag_is_read_as_three_states() {
        let remote = serde_json::json!({ "id": "x", "isRemote": true });
        let onsite = serde_json::json!({ "id": "x", "isRemote": false });
        let silent = serde_json::json!({ "id": "x" });
        assert_eq!(parse_job("o", &remote).remote_hint, Some(true));
        assert_eq!(parse_job("o", &onsite).remote_hint, Some(false));
        assert_eq!(
            parse_job("o", &silent).remote_hint,
            None,
            "unknown is a third state; defaulting to false asserts onsite for every silent source"
        );
    }

    #[test]
    fn secondary_locations_are_kept_alongside_the_primary_one() {
        let job = serde_json::json!({
            "id": "x",
            "location": "San Jose",
            "secondaryLocations": ["New York", "Austin"]
        });
        assert_eq!(
            parse_job("o", &job).location_raw.as_deref(),
            Some("San Jose; New York; Austin")
        );
    }

    #[test]
    fn the_published_date_is_a_format_normalize_actually_reads() {
        let postings = parse_board("Etched", &fixture());
        let raw = postings[0]
            .posted_at_raw
            .as_deref()
            .expect("publishedAt is present");
        assert!(
            parse_timestamp(raw).is_some(),
            "normalize could not read {raw}"
        );
    }

    #[test]
    fn a_missing_job_url_is_reconstructed_from_the_shape_the_doc_records() {
        let job = serde_json::json!({ "id": "uuid-1", "title": "SWE Intern" });
        assert_eq!(
            parse_job("Etched", &job).url,
            "https://jobs.ashbyhq.com/Etched/uuid-1"
        );
    }

    #[test]
    fn a_reshaped_response_yields_no_postings_rather_than_panicking() {
        assert!(parse_board("o", &serde_json::json!({ "postings": [] })).is_empty());
        assert!(parse_board("o", &Value::Null).is_empty());
    }

    #[test]
    fn an_empty_board_is_still_a_complete_enumeration() {
        // A board with nothing posted is a fact, not a failure — and the run must be able to
        // say so, or a board that genuinely emptied could never expire its postings.
        let fetch = finish("Ashby", Vec::new(), 1, 1, 1, false, &[]);
        assert_eq!(fetch.outcome(), SourceOutcome::Success);
    }

    // ---- scopes ----

    #[test]
    fn a_boards_scope_ids_are_exactly_the_postings_it_returned() {
        // The invariant scoped expiry rests on: a `Completed` scope claims "absence from this
        // list is evidence", so an id missing from it is a live posting marching toward expiry
        // while sitting in plain view in the same run's postings.
        let (postings, scope) = board_result("Etched", &fixture());

        assert!(scope.is_completed());
        assert!(!postings.is_empty());
        assert_eq!(scope.fetched, postings.len() as i64);
        let ids: Vec<String> = postings.iter().map(|job| job.external_id.clone()).collect();
        assert_eq!(scope.external_ids, ids);
    }

    #[test]
    fn a_board_that_does_not_exist_is_a_completed_scope_with_nothing_in_it() {
        // A 404 org offers zero postings, so absence from it is evidence and anything still
        // tagged to it must advance. Recording it as `Failed` instead would be the bug 12i
        // fixed for Greenhouse: one dead org out of 297 freezing expiry for all of them.
        let scope = ScopeRun::completed("no-such-org", Vec::new());
        assert!(scope.is_completed());
        assert_eq!(scope.fetched, 0);
        assert!(scope.external_ids.is_empty());
    }

    #[test]
    fn a_reshaped_board_yields_a_completed_scope_with_no_ids_which_is_the_hazard() {
        // Worth stating rather than discovering. `parse_board` answers "no postings" both for
        // a genuinely empty board and for one whose shape changed, and this adapter cannot
        // tell them apart from a 200 response — so a mass reshape would read as a mass
        // closure. What stops it is not here: `expiry::scoped_eligibility` refuses a run whose
        // source-level fetch count collapsed to zero, whatever its scopes claim.
        let (postings, scope) = board_result("Etched", &serde_json::json!({ "jobs": "not a list" }));
        assert!(postings.is_empty());
        assert!(scope.is_completed());
        assert!(scope.external_ids.is_empty());
    }
}
