//! Greenhouse's public job-board API.
//!
//! `GET https://boards-api.greenhouse.io/v1/boards/{token}/jobs?pay_transparency=true` — one
//! request returns the **whole** board, `{jobs: […], meta: {total}}`, with no pagination. That
//! makes "the fetch succeeded" unambiguous for this source, which is exactly what
//! `docs/INTERNSHIP_SCRAPING.md` § D.3 says a disappearance-based closure rule needs.
//!
//! # `pay_transparency=true` is the whole reason this source has pay
//!
//! Salary lives in `pay_input_ranges` and is **absent unless that parameter is passed**. The
//! similarly-named `pay_input_ranges=true` does nothing. § A.1 also records that the parameter
//! works on the *list* endpoint even though the official docs only document it under "Retrieve
//! a job" — worth one request per board instead of N+1, and **undocumented, therefore liable
//! to change**. If `pay_input_ranges` stops appearing on list responses, that is the signal to
//! fall back to per-job fetches, not evidence that boards stopped publishing pay.
//!
//! # Pay is deliberately not resolved here
//!
//! `min_cents`/`max_cents` carry **no interval field**. Across the boards measured in § A.1, 32
//! ranges were necessarily hourly and 1,533 annual, in the same field with nothing
//! distinguishing them, and a magnitude threshold is least reliable exactly where internships
//! live — a monthly stipend and a low annual salary land in the same band. So this adapter
//! emits the amount, the currency and the employer's own range title as text and lets
//! `normalize::parse_pay` apply its one named threshold, which refuses rather than guesses
//! below `MIN_UNAMBIGUOUS_ANNUAL_USD`. One place decides, and it says no when it does not know.
//!
//! # Outcomes
//!
//! One `source_runs` row covers hundreds of boards, so the aggregate rule matters more than any
//! single board:
//!
//! - **`Success` only if every board was enumerated.** 484 good boards and one 500 is
//!   `Partial` — the postings on the unreached board are not gone, they are unobserved.
//! - **A 404 on a board list counts as enumerated.** § A.1: on the list endpoint a 404 is
//!   unambiguous ("no such board"), so the board offers zero postings and the slug should be
//!   retired. Treating it as a failure instead would leave this source permanently `Partial`
//!   and therefore permanently unable to expire anything.
//! - **A board budget that truncates the list is `Partial`,** however well the fetches it made
//!   went.
//!
//! # ...and why that stopped being enough
//!
//! All three rules are still exactly right, and all three together cost this source half its
//! expiry evidence. On the 2026-09-02 uncapped run 484 boards read cleanly, `designmehair`
//! returned a network error, and the source-level verdict was `Partial` — so not one of the
//! 484 complete enumerations counted for expiry.
//!
//! **Corrected 2026-09-03**: an earlier version of this doc called a clean sweep "improbable"
//! and said the source could never expire anything. Measured, Greenhouse succeeds on **8 of 16
//! runs**. Half its runs are wasted for expiry, not all of them — which is what scoping
//! recovers.
//!
//! This adapter therefore also reports a [`ScopeRun`] per board. The aggregate rules above are
//! untouched; the boards that *were* fully enumerated now say so individually, and
//! `expiry::settle_source_run` advances disappearance counters for those and no others.
//!
//! # The § D.2 trap, and why it does not apply here
//!
//! A dead Greenhouse job's *public HTML URL* redirects to the board root with **HTTP 200**, so
//! liveness checked by status code on the public URL concludes every dead posting is alive,
//! forever, with no error to alert on. This adapter never asks the public URL anything — it
//! reads the API, where a dead job answers `{"status":404,"error":"Job not found"}`. The public
//! URL is recorded as the posting's link and never used as a liveness probe.

use serde_json::Value;

use super::super::models::{RawPosting, ScopeRun};
use super::{BoxFuture, Source, SourceContext, SourceFetch, first_string, id_string};

/// The ATS key in [`BoardDirectory`](super::BoardDirectory).
pub const ATS: &str = "greenhouse";

pub struct GreenhouseSource;

impl Default for GreenhouseSource {
    fn default() -> Self {
        Self::new()
    }
}

impl GreenhouseSource {
    pub fn new() -> Self {
        GreenhouseSource
    }
}

/// The board list endpoint, with the parameter that makes pay appear.
pub fn board_url(slug: &str) -> String {
    format!("https://boards-api.greenhouse.io/v1/boards/{slug}/jobs?pay_transparency=true")
}

impl Source for GreenhouseSource {
    fn name(&self) -> &str {
        ATS
    }

    fn description(&self) -> &str {
        "Greenhouse job-board API — whole board per request, pay via pay_transparency=true"
    }

    fn fetch<'a>(&'a self, ctx: &'a SourceContext) -> BoxFuture<'a, SourceFetch> {
        Box::pin(async move {
            let all_slugs = ctx.boards.slugs(ATS);
            if all_slugs.is_empty() {
                return SourceFetch::failed(
                    "no Greenhouse board slugs are known — harvest them from Simplify's `url` \
                     field (see simplify::extract_board_slugs) or restore \
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
            // One verdict per board. Boards the budget never reached get no entry at all:
            // absence of a row is the honest record of "no verdict", and inventing a `Failed`
            // one would claim we looked.
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
                    // robots.txt covers the host, not the board. One refusal means every board
                    // is refused, so stop rather than making 484 more requests we already know
                    // we must not make.
                    //
                    // The boards read before the refusal are dropped rather than reported as
                    // completed scopes. They genuinely were enumerated, so this under-expires
                    // — which is the safe direction, and it keeps `Skipped` meaning what the
                    // health panel says it means: we did not fetch this source.
                    Err(error) if error.is_refusal() => {
                        return SourceFetch::skipped(error.to_string());
                    }
                    // A definitive "no such board". The board offers zero postings and the
                    // slug should be retired; it is not an incomplete enumeration. As a scope
                    // it is `Completed` with no ids, so anything still tagged to it advances
                    // toward expiry — which is correct, and is new: before scopes, a retired
                    // board's postings could only expire on a run where all 485 succeeded.
                    Err(error) if error.is_not_found() => {
                        retired.push(slug.clone());
                        scopes.push(ScopeRun::completed(slug.as_str(), Vec::new()));
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
                    "internships: {} Greenhouse board(s) 404'd and should be retired: {}",
                    retired.len(),
                    retired.join(", ")
                );
            }

            finish(
                "Greenhouse",
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

/// Decide one multi-board source's outcome from what its boards did.
///
/// Shared by the three ATS adapters, because the rule is the same for all of them and three
/// copies of it is three chances to fold `Partial` into `Success`.
pub(crate) fn finish(
    label: &str,
    postings: Vec<RawPosting>,
    enumerated: usize,
    attempted: usize,
    total_boards: usize,
    truncated: bool,
    failures: &[String],
) -> SourceFetch {
    if enumerated == 0 {
        return SourceFetch::failed(format!(
            "{label}: none of {attempted} board(s) could be read — {}",
            summarize(failures)
        ));
    }

    if truncated {
        return SourceFetch::partial(
            postings,
            format!(
                "{label}: stopped at {attempted} of {total_boards} boards by the per-run budget, \
                 so this is not a complete enumeration"
            ),
        );
    }

    if !failures.is_empty() {
        return SourceFetch::partial(
            postings,
            format!(
                "{label}: {} of {attempted} board(s) failed — {}",
                failures.len(),
                summarize(failures)
            ),
        );
    }

    SourceFetch::success(postings)
}

/// First few failures plus a count. The whole list on 400 failed boards is unreadable, and an
/// unreadable error is one nobody reads.
fn summarize(failures: &[String]) -> String {
    const SHOWN: usize = 3;
    if failures.len() <= SHOWN {
        return failures.join("; ");
    }
    format!(
        "{}; and {} more",
        failures[..SHOWN].join("; "),
        failures.len() - SHOWN
    )
}

/// The verdict for a board that was completely enumerated, built from the postings that
/// enumeration produced.
///
/// Shared with Lever and Ashby for the same reason [`finish`] is: the invariant is that a
/// completed scope's ids are **exactly** the ids of the postings that board returned on this
/// run, and three copies of it are three chances for one to drift. Drift here is not cosmetic.
/// `expiry::settle_source_run` resets `consecutive_misses` for the ids named here and advances
/// it for every other sighting tagged to the scope, so an id missing from this list is a
/// posting marching toward expiry while sitting in plain view in the run's own postings.
///
/// It takes the board's own postings rather than the run's accumulated vector, so there is no
/// filtering step to get wrong.
pub(crate) fn completed_scope(slug: &str, board: &[RawPosting]) -> ScopeRun {
    ScopeRun::completed(
        slug,
        board.iter().map(|job| job.external_id.clone()).collect(),
    )
}

/// One enumerated board: its postings and its scope verdict, from a **single** parse.
///
/// Returning both together is the point. The scope says "absence from this board is evidence",
/// and the ids are the only thing that stops that conclusion from being drawn about postings
/// that were right there — so computing them in two passes (parse, then re-derive the ids by
/// filtering `postings` on the slug) is a divergence waiting to happen, and the failure mode of
/// that divergence is the whole board expiring. One call, one tuple, no way to have one without
/// the other.
fn board_result(slug: &str, body: &Value) -> (Vec<RawPosting>, ScopeRun) {
    let postings = parse_board(slug, body);
    let scope = completed_scope(slug, &postings);
    (postings, scope)
}

/// Turn one board response into raw postings. Pure, so it is tested offline against the
/// committed fixture.
pub fn parse_board(slug: &str, body: &Value) -> Vec<RawPosting> {
    let Some(Value::Array(jobs)) = body.get("jobs") else {
        return Vec::new();
    };

    jobs.iter().map(|job| parse_job(slug, job)).collect()
}

fn parse_job(slug: &str, job: &Value) -> RawPosting {
    let external_id = id_string(job, &["id"]).unwrap_or_default();

    RawPosting {
        source: ATS.to_string(),
        url: first_string(job, &["absolute_url"]).unwrap_or_else(|| {
            // § C records this URL shape, so it is a reconstruction rather than a guess. Note
            // it is a *link*, never a liveness probe — see the module doc.
            format!("https://job-boards.greenhouse.io/{slug}/jobs/{external_id}")
        }),
        external_id,
        // § B marks company as present for Greenhouse, and it is: `company_name` on 485/485 of
        // the board sampled. The slug is the fallback so a reshaped response still names
        // somebody rather than rejecting the row.
        company: first_string(job, &["company_name"]).unwrap_or_else(|| slug.to_string()),
        title: first_string(job, &["title"]).unwrap_or_default(),
        location_raw: job
            .get("location")
            .and_then(|location| first_string(location, &["name"])),
        pay_raw: pay_text(job),
        // No term field. § B marks season as derivable from the title, which is QC's job.
        term_raw: None,
        class_year_raw: None,
        // `first_published` is the posting date; `updated_at` is a later edit. Both were
        // populated on 485/485, both RFC 3339 with an offset, which
        // `normalize::parse_timestamp` reads directly.
        posted_at_raw: first_string(job, &["first_published", "updated_at"]),
        // § D.1: a first-class field that is almost never set — 14/127 on one board, 0 on four
        // others including the one this fixture came from. Read where present.
        deadline_raw: first_string(job, &["application_deadline"]),
        // The list endpoint carries no description unless `content=true`, which is ~9 MB for
        // one large board. Not worth it for a field QC only mines for class-year hints.
        description: None,
        // No structured remote flag. `None` is *unknown*; QC may still infer from the location.
        remote_hint: None,
        raw_json: job.to_string(),
    }
}

/// Render `pay_input_ranges` as a human-readable compensation string for
/// `normalize::parse_pay`.
///
/// Two rules, and the first one is the reason this function exists at all rather than the field
/// being passed through:
///
/// 1. **Cents become currency units.** `min_cents: 4500` is `"USD 45.00"`, never `"4500"`.
///    Passed through unconverted, `$45.00/hour` reaches QC as `4500`, which lands squarely in
///    the plausible monthly-stipend band, parses cleanly, and produces a completely fabricated
///    figure with no error anywhere — on the ranking's highest-weighted input.
/// 2. **No period is emitted, because Greenhouse states none.** `min_cents`/`max_cents` carry
///    no interval field, so QC's named magnitude threshold is the honest fallback and must be
///    allowed to run. A guessed `"per year"` on an hourly figure is worse than the heuristic
///    because it *suppresses* it — QC treats an explicit period as authoritative in both
///    directions.
///
/// The employer's own range `title` is deliberately **not** appended for the same reason. It is
/// prose ("the on-target earnings range for this role is:") that occasionally contains a period
/// word, and letting it override the heuristic means an employer's sentence structure decides
/// whether a figure reads as hourly or annual. It is preserved in `raw_json` instead, where §
/// A.1's advice to "store the raw cents plus the range title and decide late" is satisfied
/// without putting prose on the parsing path.
fn pay_text(job: &Value) -> Option<String> {
    let Some(Value::Array(ranges)) = job.get("pay_input_ranges") else {
        return None;
    };
    let first = ranges.first()?;

    let min_cents = first.get("min_cents").and_then(Value::as_f64)?;
    let max_cents = first.get("max_cents").and_then(Value::as_f64);
    let currency = first_string(first, &["currency_type"]).unwrap_or_else(|| "USD".to_string());

    let mut text = match max_cents {
        Some(max) if max > min_cents => format!(
            "{currency} {:.2} - {:.2}",
            min_cents / 100.0,
            max / 100.0
        ),
        _ => format!("{currency} {:.2}", min_cents / 100.0),
    };

    // More than one range means the employer published a band per geography. Recorded rather
    // than merged, because a synthetic span across tiers is a number nobody published. The
    // note carries no period word and no leading digit the range reader could mistake for an
    // upper bound — pinned by `the_multi_range_note_cannot_be_misread_as_pay`.
    if ranges.len() > 1 {
        text.push_str(&format!(" [+{} more published ranges]", ranges.len() - 1));
    }

    Some(text)
}

// ------------------------------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internships::models::PayPeriod;
    use crate::internships::normalize::parse_pay;

    /// Real jobs from the `anthropic` and `airtable` boards, fetched 2026-08-20. See
    /// `data/internships/README.md`.
    const FIXTURE: &str =
        include_str!("../../../data/internships/fixtures/greenhouse-board.sample.json");

    fn fixture() -> Value {
        serde_json::from_str(FIXTURE).expect("the committed fixture parses")
    }

    #[test]
    fn the_board_url_carries_the_parameter_that_makes_pay_appear() {
        // Without it `pay_input_ranges` is simply absent, and the board looks like one that
        // publishes no pay. The similarly-named `pay_input_ranges=true` does nothing.
        let url = board_url("airtable");
        assert!(url.contains("pay_transparency=true"));
        assert!(!url.contains("pay_input_ranges=true"));
        assert!(url.starts_with("https://boards-api.greenhouse.io/v1/boards/airtable/jobs"));
    }

    #[test]
    fn every_job_in_the_fixture_parses_into_an_identifiable_posting() {
        let postings = parse_board("anthropic", &fixture());
        assert!(!postings.is_empty());
        for posting in &postings {
            assert_eq!(posting.source, "greenhouse");
            assert!(!posting.external_id.is_empty());
            assert!(posting.url.starts_with("https://"));
            assert!(!posting.company.is_empty());
            assert!(!posting.title.is_empty());
        }
    }

    #[test]
    fn the_adapter_does_not_apply_the_internship_filter_itself() {
        // Every `intern` substring hit on the real Anthropic board is a false positive —
        // "Director, US International Tax", "Internal Communications Manager, Tech". § C says
        // to use a word-boundary match, and `normalize::is_internship_role` is where that
        // lives. Six adapters each re-implementing it is six chances to get it wrong, and it
        // would also make the `filtered` health count meaningless by shrinking its denominator.
        let postings = parse_board("anthropic", &fixture());
        let titles: Vec<&str> = postings.iter().map(|p| p.title.as_str()).collect();
        assert!(
            titles.iter().any(|t| t.contains("International") || t.contains("Internal")),
            "the fixture must contain the substring trap so this stays a real assertion"
        );
    }

    #[test]
    fn a_pay_range_becomes_text_that_normalize_can_read() {
        let postings = parse_board("anthropic", &fixture());
        let with_pay = postings
            .iter()
            .find(|p| p.pay_raw.is_some())
            .expect("the fixture must include a job with a published range");
        let text = with_pay.pay_raw.as_deref().unwrap();

        let parsed = parse_pay(text).expect("a six-figure band is unambiguously annual");
        assert_eq!(parsed.currency, "USD");
        assert_eq!(parsed.period, PayPeriod::Year);
        assert!(parsed.min > 0.0);
    }

    #[test]
    fn an_hourly_internship_rate_survives_the_whole_pipeline() {
        // Audit finding F1, end to end across the adapter/QC seam. Each half was correct on
        // its own: the adapter emitted a faithful periodless string because Greenhouse has no
        // interval field, and QC declined to guess a period below $50,000. Together they threw
        // away every internship rate Greenhouse publishes, and no test on either side failed —
        // the contract was only wrong *between* them.
        let hourly = serde_json::json!({
            "id": 1,
            "pay_input_ranges": [{ "min_cents": 4500, "max_cents": 5500, "currency_type": "USD" }]
        });
        let text = pay_text(&hourly).expect("the adapter should emit pay text");
        assert_eq!(text, "USD 45.00 - 55.00");

        let parsed = crate::internships::normalize::parse_pay(&text)
            .expect("QC must be able to read what this adapter emits");
        assert_eq!(parsed.min, 45.0);
        assert_eq!(parsed.max, Some(55.0));
        assert_eq!(parsed.period, crate::internships::models::PayPeriod::Hour);
    }

    #[test]
    fn cents_are_converted_to_currency_units_not_passed_through() {
        // THE test for this adapter. `min_cents` is cents; `pay_raw` is a human-readable
        // compensation string. Passed through unconverted, `$45.00/hour` reaches QC as the
        // string "4500", which sits squarely inside the plausible monthly-stipend band, parses
        // cleanly, and yields a completely fabricated figure with no error anywhere — on the
        // ranking's highest-weighted input. Anyone "simplifying" the `/ 100.0` away fails here.
        let hourly = serde_json::json!({
            "id": 1,
            "pay_input_ranges": [{ "min_cents": 4500, "max_cents": 5500, "currency_type": "USD" }]
        });
        let text = pay_text(&hourly).unwrap();
        assert_eq!(text, "USD 45.00 - 55.00");
        assert!(!text.contains("4500"), "the raw cents value must not appear");

        let annual = serde_json::json!({
            "id": 1,
            "pay_input_ranges": [{ "min_cents": 13000000, "max_cents": 13000000, "currency_type": "USD" }]
        });
        assert_eq!(pay_text(&annual).unwrap(), "USD 130000.00");
    }

    #[test]
    fn the_adapter_never_decides_the_period_itself() {
        // § A.1: `min_cents` carries no interval, hourly and annual share the field, and a
        // magnitude threshold is least reliable exactly where internships live. So an
        // ambiguous amount must reach QC ambiguous, and QC must refuse it rather than guess.
        let job = serde_json::json!({
            "id": 1,
            "pay_input_ranges": [{ "min_cents": 800000, "max_cents": 800000, "currency_type": "USD" }]
        });
        let text = pay_text(&job).unwrap();
        assert!(text.contains("8000.00"));
        assert_eq!(
            parse_pay(&text),
            None,
            "$8,000 could be a month or a summer; refusing is the correct answer"
        );

        // And no period word is emitted anywhere, in any branch. QC treats an explicit period
        // as authoritative, so a guessed one would silently suppress the heuristic that is the
        // only honest thing available for this source.
        for period in ["hour", "hourly", "month", "monthly", "year", "annual", "week"] {
            assert!(
                !text.to_lowercase().contains(period),
                "Greenhouse states no interval; emitting `{period}` would invent one"
            );
        }
    }

    #[test]
    fn the_employers_range_title_stays_off_the_parsing_path() {
        // It is prose, and prose occasionally contains a period word. Letting it through means
        // an employer's sentence structure decides whether a figure reads as hourly or annual.
        // The full title survives in `raw_json`.
        let job = serde_json::json!({
            "id": 1,
            "pay_input_ranges": [{
                "min_cents": 4500,
                "max_cents": 5500,
                "currency_type": "USD",
                "title": "The annual on-target earnings range for this role is:"
            }]
        });
        let text = pay_text(&job).unwrap();
        assert_eq!(text, "USD 45.00 - 55.00");
        let posting = parse_job("acme", &job);
        assert!(
            posting.raw_json.contains("on-target earnings range"),
            "the title must still be diagnosable from the preserved record"
        );
    }

    #[test]
    fn the_multi_range_note_cannot_be_misread_as_pay() {
        // The note carries a digit. Pin that it changes neither the amount, the range, nor the
        // period QC reads out.
        let job = serde_json::json!({
            "id": 1,
            "pay_input_ranges": [
                { "min_cents": 10000000, "max_cents": 12000000, "currency_type": "USD" },
                { "min_cents": 9000000, "max_cents": 11000000, "currency_type": "USD" }
            ]
        });
        let text = pay_text(&job).unwrap();
        let with_note = parse_pay(&text).expect("parses");
        let without_note = parse_pay("USD 100000.00 - 120000.00").expect("parses");
        assert_eq!(with_note, without_note);
    }

    #[test]
    fn a_second_published_range_is_recorded_rather_than_merged() {
        // Merging min-of-mins with max-of-maxes across geographies invents a band no employer
        // published.
        let job = serde_json::json!({
            "id": 1,
            "pay_input_ranges": [
                { "min_cents": 10000000, "max_cents": 12000000, "currency_type": "USD" },
                { "min_cents": 9000000, "max_cents": 11000000, "currency_type": "USD" }
            ]
        });
        let text = pay_text(&job).unwrap();
        assert!(text.contains("100000.00 - 120000.00"));
        assert!(text.contains("+1 more published ranges"));
    }

    #[test]
    fn a_job_with_no_pay_yields_none_rather_than_a_zero_or_a_placeholder() {
        // Absence is a real, common, correct answer. `Some("0")` or `Some("")` would both make
        // unknown pay read as known pay, which is the one thing `models.rs` exists to prevent.
        let job = serde_json::json!({ "id": 1, "title": "SWE Intern", "pay_input_ranges": [] });
        assert_eq!(parse_job("acme", &job).pay_raw, None);
    }

    #[test]
    fn a_non_usd_range_keeps_its_own_currency() {
        // The sampled board published GBP and EUR ranges. Reading them as dollars would
        // mis-scale a posting into a rank it did not earn.
        let job = serde_json::json!({
            "id": 1,
            "pay_input_ranges": [{ "min_cents": 6000000, "max_cents": 7000000, "currency_type": "GBP" }]
        });
        let parsed = parse_pay(&pay_text(&job).unwrap()).expect("parses");
        assert_eq!(parsed.currency, "GBP");
    }

    #[test]
    fn a_job_with_no_published_range_has_no_pay_text() {
        // Absent must stay absent: `pay_raw: Some(_)` means the source said something.
        let job = serde_json::json!({ "id": 1, "title": "SWE Intern", "pay_input_ranges": [] });
        assert_eq!(pay_text(&job), None);
        let job = serde_json::json!({ "id": 1, "title": "SWE Intern" });
        assert_eq!(pay_text(&job), None);
    }

    #[test]
    fn the_posted_date_prefers_first_publication_over_the_last_edit() {
        let job = serde_json::json!({
            "id": 1,
            "first_published": "2026-04-07T16:10:24-04:00",
            "updated_at": "2026-08-03T18:25:22-04:00"
        });
        let posting = parse_job("anthropic", &job);
        assert_eq!(
            posting.posted_at_raw.as_deref(),
            Some("2026-04-07T16:10:24-04:00")
        );
        assert!(
            crate::internships::normalize::parse_timestamp(
                posting.posted_at_raw.as_deref().unwrap()
            )
            .is_some(),
            "the emitted format must be one normalize actually reads"
        );
    }

    #[test]
    fn a_missing_absolute_url_is_reconstructed_from_the_shape_the_doc_records() {
        let job = serde_json::json!({ "id": 8350486002u64, "title": "SWE Intern" });
        let posting = parse_job("mcghealth", &job);
        assert_eq!(
            posting.url,
            "https://job-boards.greenhouse.io/mcghealth/jobs/8350486002"
        );
    }

    #[test]
    fn a_reshaped_response_yields_no_postings_rather_than_panicking() {
        assert!(parse_board("x", &serde_json::json!({ "results": [] })).is_empty());
        assert!(parse_board("x", &serde_json::json!([])).is_empty());
        assert!(parse_board("x", &Value::Null).is_empty());
    }

    // ---- the outcome rule ----

    fn one_posting() -> Vec<RawPosting> {
        parse_board("anthropic", &fixture())
    }

    #[test]
    fn every_board_enumerated_is_a_success() {
        let fetch = finish("Greenhouse", one_posting(), 10, 10, 10, false, &[]);
        assert_eq!(
            fetch.outcome(),
            crate::internships::models::SourceOutcome::Success
        );
    }

    #[test]
    fn one_failed_board_out_of_many_is_partial_not_success() {
        // The postings on the unreached board are not gone, they are unobserved. Reporting
        // `Success` here starts the miss counter on every one of them.
        let fetch = finish("Greenhouse", one_posting(), 9, 10, 10, false, &["acme: HTTP 500".into()]);
        assert_eq!(
            fetch.outcome(),
            crate::internships::models::SourceOutcome::Partial
        );
        assert!(fetch.error().unwrap().contains("1 of 10"));
        assert!(!fetch.postings().is_empty(), "the good boards' postings are still kept");
    }

    #[test]
    fn a_budget_that_truncates_the_board_list_is_partial() {
        // Every fetch it made succeeded. It still did not enumerate the source, and stopping
        // early on purpose is still stopping early.
        let fetch = finish("Greenhouse", one_posting(), 50, 50, 485, true, &[]);
        assert_eq!(
            fetch.outcome(),
            crate::internships::models::SourceOutcome::Partial
        );
        assert!(fetch.error().unwrap().contains("50 of 485"));
    }

    #[test]
    fn no_board_readable_at_all_is_a_failure() {
        let fetch = finish("Greenhouse", Vec::new(), 0, 10, 10, false, &["a: HTTP 503".into()]);
        assert_eq!(
            fetch.outcome(),
            crate::internships::models::SourceOutcome::Failed
        );
        assert!(fetch.error().is_some());
    }

    // ---- scopes ----

    #[test]
    fn a_boards_scope_ids_are_exactly_the_postings_it_returned() {
        // The invariant the whole scoped-expiry mechanism rests on. A scope reported
        // `Completed` whose ids do not cover its postings gets the board incremented with
        // nothing reset — every posting on it expiring after three runs.
        let (postings, scope) = board_result("anthropic", &fixture());
        assert!(scope.is_completed());
        assert_eq!(scope.fetched, postings.len() as i64);
        let ids: Vec<&str> = postings.iter().map(|job| job.external_id.as_str()).collect();
        assert_eq!(scope.external_ids, ids);
        assert!(!ids.is_empty(), "the fixture must actually carry a job");
    }

    #[test]
    fn scopes_ride_along_without_changing_the_source_level_verdict() {
        // Scopes are additive. A run that was `Partial` before reporting them is still
        // `Partial`; what changes is only that expiry can now ask about a single board.
        let scopes = vec![
            ScopeRun::completed("good", vec!["1".into()]),
            ScopeRun::failed("bad", "HTTP 500"),
        ];
        let bare = finish("Greenhouse", one_posting(), 9, 10, 10, false, &["bad: HTTP 500".into()]);
        let scoped = finish("Greenhouse", one_posting(), 9, 10, 10, false, &["bad: HTTP 500".into()])
            .with_scopes(scopes);

        assert_eq!(scoped.outcome(), bare.outcome());
        assert_eq!(scoped.error(), bare.error());
        assert_eq!(scoped.scopes().len(), 2);
        assert_eq!(scoped.scopes().iter().filter(|s| s.is_completed()).count(), 1);
    }

    #[test]
    fn a_source_that_reports_no_scopes_carries_none() {
        // Every other adapter. The unscoped settle path keys off exactly this being empty.
        assert!(finish("Greenhouse", one_posting(), 10, 10, 10, false, &[])
            .scopes()
            .is_empty());
    }

    #[test]
    fn a_long_failure_list_is_summarized_rather_than_dumped() {
        let failures: Vec<String> = (0..40).map(|i| format!("board{i}: HTTP 500")).collect();
        let summary = summarize(&failures);
        assert!(summary.contains("and 37 more"));
        assert!(summary.len() < 200, "an unreadable error is one nobody reads");
    }
}
