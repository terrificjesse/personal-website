//! Lever's public postings API.
//!
//! `GET https://api.lever.co/v0/postings/{site}?mode=json` — a **bare array** of postings, with
//! `skip`/`limit` paging. Officially documented and intended for building career sites
//! (`docs/INTERNSHIP_SCRAPING.md` § F). Its `robots.txt` publishes `Crawl-delay: 1`, which the
//! shared fetch layer honours automatically.
//!
//! # Four things this source gets wrong if you take the obvious route
//!
//! 1. **There is no company-name field.** The response is a bare array; the company is only
//!    knowable from the `{site}` slug you asked for. Carry it through yourself — which is what
//!    [`parse_board`]'s first argument is for.
//! 2. **Do not filter server-side on `commitment`.** The official docs use `Intern` as the
//!    example value and it returns **zero** results on a board whose actual vocabulary is
//!    `Full-Time`, `Contract`, `Internship`, … The vocabulary is employer-defined free text, so
//!    a server-side filter silently returns an empty set rather than an error — a
//!    fully-successful-looking run that collected nothing. Fetch the whole board, filter
//!    locally, and let QC do the filtering.
//! 3. **`createdAt` is epoch *milliseconds*,** unlike every other source in the document.
//!    `normalize::parse_timestamp` distinguishes 13-digit from 10-digit input, so the value is
//!    passed through unconverted — but read as seconds it would date every posting to 1970 or
//!    to the year 58,000 depending on which way you got it wrong.
//! 4. **404 and 200-with-an-empty-array are different conditions.** A company not on Lever
//!    returns 404 (retire the slug); a live board with nothing posted returns 200 and `[]`
//!    (keep polling). Conflating them either retires live boards or polls dead ones forever.
//!
//! # Paging, and why it is explicit
//!
//! § A.1 records `skip`/`limit` paging without recording what an unparameterized request
//! returns. An implicit server-side cap would silently truncate a large board while looking
//! like a complete fetch — and a complete-looking truncated fetch is precisely what makes
//! postings appear to vanish. So this adapter pages explicitly and stops when a page comes back
//! short, which is a *verifiable* end-of-board rather than an assumed one. Running out of pages
//! before that makes the board incomplete, and the source reports `Partial`.

use serde_json::Value;

use super::super::models::RawPosting;
use super::{
    BoxFuture, Source, SourceContext, SourceFetch, first_string, greenhouse::finish, id_string,
    join_locations,
};

/// The ATS key in [`BoardDirectory`](super::BoardDirectory).
pub const ATS: &str = "lever";

/// Postings per page.
const PAGE_SIZE: usize = 100;

/// Pages per board before we stop and call the board incomplete. 20 × 100 is far past any real
/// board; the cap exists so a server that ignores `skip` cannot spin forever.
const MAX_PAGES: usize = 20;

pub struct LeverSource;

impl Default for LeverSource {
    fn default() -> Self {
        Self::new()
    }
}

impl LeverSource {
    pub fn new() -> Self {
        LeverSource
    }
}

/// One page of a board.
pub fn board_url(slug: &str, skip: usize) -> String {
    format!("https://api.lever.co/v0/postings/{slug}?mode=json&limit={PAGE_SIZE}&skip={skip}")
}

impl Source for LeverSource {
    fn name(&self) -> &str {
        ATS
    }

    fn description(&self) -> &str {
        "Lever postings API — whole board per request, structured salaryRange, no company field"
    }

    fn fetch<'a>(&'a self, ctx: &'a SourceContext) -> BoxFuture<'a, SourceFetch> {
        Box::pin(async move {
            let all_slugs = ctx.boards.slugs(ATS);
            if all_slugs.is_empty() {
                return SourceFetch::failed(
                    "no Lever board slugs are known — harvest them from Simplify's `url` field \
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

            'boards: for slug in slugs {
                let mut skip = 0usize;
                let mut pages = 0usize;
                let mut board_postings = Vec::new();

                loop {
                    let url = board_url(slug, skip);
                    let page = match ctx.http.get(&url).await {
                        Ok(response) => match response.json() {
                            Ok(Value::Array(page)) => page,
                            Ok(_) => {
                                failures.push(format!(
                                    "{slug}: response root is not the documented bare array"
                                ));
                                continue 'boards;
                            }
                            Err(error) => {
                                failures.push(format!("{slug}: {error}"));
                                continue 'boards;
                            }
                        },
                        // One robots refusal covers the host, so every remaining board is
                        // refused too. Stop rather than making the requests anyway.
                        Err(error) if error.is_refusal() => {
                            return SourceFetch::skipped(error.to_string());
                        }
                        // "Not on Lever." A definitive zero, not a failed read.
                        Err(error) if error.is_not_found() && skip == 0 => {
                            retired.push(slug.clone());
                            enumerated += 1;
                            continue 'boards;
                        }
                        Err(error) => {
                            failures.push(format!("{slug}: {error}"));
                            continue 'boards;
                        }
                    };

                    let short_page = page.len() < PAGE_SIZE;
                    board_postings.extend(parse_board(slug, &page));
                    pages += 1;

                    // A short page is a *verifiable* end of board. This is the only exit that
                    // counts the board as enumerated.
                    if short_page {
                        postings.append(&mut board_postings);
                        enumerated += 1;
                        continue 'boards;
                    }

                    if pages >= MAX_PAGES {
                        // Keep the rows; do not claim the board was enumerated.
                        postings.append(&mut board_postings);
                        failures.push(format!(
                            "{slug}: still returning full pages after {MAX_PAGES} pages, so the \
                             board was not fully read"
                        ));
                        continue 'boards;
                    }

                    skip += PAGE_SIZE;
                }
            }

            if !retired.is_empty() {
                println!(
                    "internships: {} Lever board(s) 404'd and should be retired: {}",
                    retired.len(),
                    retired.join(", ")
                );
            }

            finish(
                "Lever",
                postings,
                enumerated,
                slugs.len(),
                all_slugs.len(),
                truncated,
                &failures,
            )
        })
    }
}

/// Turn one page into raw postings. Pure, tested offline against the committed fixture.
///
/// `slug` is the company: see the module doc — the payload does not carry one.
pub fn parse_board(slug: &str, page: &[Value]) -> Vec<RawPosting> {
    page.iter().map(|job| parse_posting(slug, job)).collect()
}

fn parse_posting(slug: &str, job: &Value) -> RawPosting {
    let external_id = id_string(job, &["id"]).unwrap_or_default();
    let categories = job.get("categories");

    RawPosting {
        source: ATS.to_string(),
        url: first_string(job, &["hostedUrl", "applyUrl"])
            // § C records this URL shape, so the fallback is a reconstruction, not a guess.
            .unwrap_or_else(|| format!("https://jobs.lever.co/{slug}/{external_id}")),
        external_id,
        // No company field in the payload. § B note 3.
        company: slug.to_string(),
        // `text` is the title. Marked present in § B without being named there; this is what
        // the live response actually carries.
        title: first_string(job, &["text"]).unwrap_or_default(),
        location_raw: location_of(categories),
        pay_raw: pay_text(job),
        // `categories.commitment` — employer-defined free text (`Full-Time`, `Internship`,
        // `Apprenticeship`, …). Passed to QC as the term hint precisely because it is *not*
        // safe to filter on server-side.
        term_raw: categories.and_then(|c| first_string(c, &["commitment"])),
        class_year_raw: None,
        // Epoch **milliseconds**. `normalize::parse_timestamp` reads a 13-digit all-digit
        // string as such; converting here would be one more place to get the factor wrong.
        posted_at_raw: id_string(job, &["createdAt"]),
        deadline_raw: None,
        description: first_string(job, &["descriptionPlain"]),
        remote_hint: remote_of(job),
        raw_json: job.to_string(),
    }
}

/// Every location the posting names, not just the first.
fn location_of(categories: Option<&Value>) -> Option<String> {
    let categories = categories?;
    if let Some(Value::Array(all)) = categories.get("allLocations") {
        let joined = join_locations(
            all.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>(),
        );
        if joined.is_some() {
            return joined;
        }
    }
    first_string(categories, &["location"])
}

/// `workplaceType` is `"remote"`, `"hybrid"` or `"onsite"` on the live response.
///
/// Hybrid reads as **not remote**, matching `normalize::detect_modality`, which maps the word
/// "hybrid" in a location string the same way. Two components disagreeing about what hybrid
/// means would make a remote filter depend on which source a posting came from.
fn remote_of(job: &Value) -> Option<bool> {
    match first_string(job, &["workplaceType"])?.to_ascii_lowercase().as_str() {
        "remote" => Some(true),
        "onsite" | "on-site" | "hybrid" => Some(false),
        _ => None,
    }
}

/// Render `salaryRange` — or, failing that, the employer's free-text salary line — for
/// `normalize::parse_pay`.
///
/// Unlike Greenhouse, Lever states the interval, so it is translated into the period words the
/// pay parser reads rather than left to a magnitude heuristic. An interval we do not recognize
/// contributes no period word at all, which makes the parser fall back to its own named
/// threshold and refuse when the amount is ambiguous.
fn pay_text(job: &Value) -> Option<String> {
    if let Some(range) = job.get("salaryRange") {
        let min = range.get("min").and_then(Value::as_f64);
        if let Some(min) = min {
            let max = range.get("max").and_then(Value::as_f64);
            let currency = first_string(range, &["currency"]).unwrap_or_else(|| "USD".to_string());

            let mut text = match max {
                Some(max) if max > min => format!("{currency} {min:.2} - {max:.2}"),
                _ => format!("{currency} {min:.2}"),
            };
            if let Some(period) = interval_words(range.get("interval").and_then(Value::as_str)) {
                text.push(' ');
                text.push_str(period);
            }
            return Some(text);
        }
    }

    // No structured range. The free-text version still says something, and preserving it keeps
    // "we could not parse it" distinct from "there was not one".
    first_string(job, &["salaryDescriptionPlain", "salaryDescription"])
}

/// Lever's interval vocabulary, as period words `normalize::detect_period` reads.
///
/// `per-week-salary` and `per-day-salary` map to words the parser explicitly refuses, which is
/// the point: `PayPeriod` cannot express them, and a weekly figure silently promoted to monthly
/// is off by four and looks entirely plausible.
fn interval_words(interval: Option<&str>) -> Option<&'static str> {
    let interval = interval?.to_ascii_lowercase();
    if interval.contains("hour") {
        Some("per hour")
    } else if interval.contains("month") {
        Some("per month")
    } else if interval.contains("year") || interval.contains("annual") {
        Some("per year")
    } else if interval.contains("week") {
        Some("per week")
    } else if interval.contains("day") {
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
    use crate::internships::models::PayPeriod;
    use crate::internships::normalize::{parse_pay, parse_timestamp};

    /// Real postings from the `belvederetrading` board, fetched 2026-08-20. Descriptions
    /// truncated; see `data/internships/README.md`.
    const FIXTURE: &str =
        include_str!("../../../data/internships/fixtures/lever-board.sample.json");

    fn fixture() -> Vec<Value> {
        serde_json::from_str(FIXTURE).expect("the committed fixture parses")
    }

    #[test]
    fn the_board_url_asks_for_json_and_pages_explicitly() {
        let url = board_url("belvederetrading", 200);
        assert!(url.contains("mode=json"));
        assert!(url.contains("limit=100"));
        assert!(url.contains("skip=200"));
        // Filtering server-side on `commitment` returns an empty set with no error, which is a
        // successful-looking run that collected nothing.
        assert!(!url.contains("commitment"));
    }

    #[test]
    fn the_company_comes_from_the_slug_because_the_payload_has_none() {
        let postings = parse_board("belvederetrading", &fixture());
        assert!(!postings.is_empty());
        for posting in &postings {
            assert_eq!(posting.company, "belvederetrading");
            assert!(!posting.title.is_empty());
            assert!(!posting.external_id.is_empty());
            assert!(posting.url.starts_with("https://"));
        }
        // And the payload really does lack one, so this is not a redundant assertion.
        assert!(fixture()[0].get("company").is_none());
        assert!(fixture()[0].get("company_name").is_none());
    }

    #[test]
    fn created_at_is_passed_through_as_thirteen_digit_milliseconds() {
        // The one source in the document using milliseconds. Read as seconds this dates every
        // posting to the year 58,000 — a number no test that only checks `is_some` would catch.
        let postings = parse_board("belvederetrading", &fixture());
        let raw = postings[0]
            .posted_at_raw
            .as_deref()
            .expect("createdAt is present");
        assert_eq!(raw.len(), 13, "epoch milliseconds, not seconds");
        let parsed = parse_timestamp(raw).expect("normalize reads a 13-digit epoch");
        assert!(
            parsed.format("%Y").to_string().starts_with("20"),
            "a plausible year, got {parsed}"
        );
    }

    #[test]
    fn a_structured_salary_range_keeps_its_stated_interval() {
        // Lever states the interval, so no magnitude guessing is needed or wanted.
        let job = serde_json::json!({
            "id": "x",
            "text": "SWE Intern",
            "salaryRange": { "min": 150000, "max": 200000, "currency": "USD", "interval": "per-year-salary" }
        });
        let text = pay_text(&job).expect("a range renders");
        let parsed = parse_pay(&text).expect("and parses");
        assert_eq!(parsed.min, 150000.0);
        assert_eq!(parsed.max, Some(200000.0));
        assert_eq!(parsed.currency, "USD");
        assert_eq!(parsed.period, PayPeriod::Year);
    }

    #[test]
    fn an_hourly_interval_is_not_promoted_to_annual_by_magnitude() {
        // $45 with no period would be refused; with the stated interval it is a real rate.
        let job = serde_json::json!({
            "id": "x",
            "salaryRange": { "min": 45, "max": 60, "currency": "USD", "interval": "per-hour-wage" }
        });
        let parsed = parse_pay(&pay_text(&job).unwrap()).expect("parses");
        assert_eq!(parsed.period, PayPeriod::Hour);
        assert_eq!(parsed.min, 45.0);
    }

    #[test]
    fn a_period_the_model_cannot_express_is_refused_rather_than_rounded() {
        // A weekly figure read as monthly is off by four and looks entirely plausible.
        let job = serde_json::json!({
            "id": "x",
            "salaryRange": { "min": 60000, "max": 80000, "currency": "USD", "interval": "per-week-salary" }
        });
        let text = pay_text(&job).unwrap();
        assert!(text.contains("per week"));
        assert_eq!(parse_pay(&text), None);
    }

    #[test]
    fn a_free_text_salary_line_survives_when_there_is_no_structured_range() {
        // Keeps "we could not parse it" distinct from "there was not one".
        let job = serde_json::json!({
            "id": "x",
            "salaryDescriptionPlain": "Competitive, commensurate with experience"
        });
        assert_eq!(
            pay_text(&job).as_deref(),
            Some("Competitive, commensurate with experience")
        );
    }

    #[test]
    fn a_posting_with_no_pay_information_claims_none() {
        let job = serde_json::json!({ "id": "x", "text": "SWE Intern" });
        assert_eq!(pay_text(&job), None);
    }

    #[test]
    fn every_location_is_kept_not_just_the_primary_one() {
        let job = serde_json::json!({
            "categories": { "location": "Miami, Florida",
                            "allLocations": ["Miami, Florida", "New York, New York"] }
        });
        assert_eq!(
            location_of(job.get("categories")).as_deref(),
            Some("Miami, Florida; New York, New York")
        );
    }

    #[test]
    fn the_commitment_field_reaches_qc_as_the_term_hint() {
        // Some sources put "Internship" in an employment-type field rather than the title, and
        // `normalize::normalize` reads title and term together for exactly that reason.
        let postings = parse_board("belvederetrading", &fixture());
        assert!(
            postings.iter().any(|p| p.term_raw.as_deref() == Some("Intern")),
            "the fixture must contain an Intern commitment"
        );
    }

    #[test]
    fn workplace_type_becomes_a_structured_remote_flag() {
        let remote = serde_json::json!({ "workplaceType": "remote" });
        let onsite = serde_json::json!({ "workplaceType": "onsite" });
        // Matches `normalize::detect_modality`, which reads the word "hybrid" as not-remote.
        // Two components disagreeing would make a remote filter depend on the source.
        let hybrid = serde_json::json!({ "workplaceType": "hybrid" });
        let silent = serde_json::json!({ "id": "x" });

        assert_eq!(remote_of(&remote), Some(true));
        assert_eq!(remote_of(&onsite), Some(false));
        assert_eq!(remote_of(&hybrid), Some(false));
        assert_eq!(remote_of(&silent), None, "unknown is a third state, not onsite");
    }

    #[test]
    fn a_missing_hosted_url_is_reconstructed_from_the_shape_the_doc_records() {
        let job = serde_json::json!({ "id": "abc-123", "text": "SWE Intern" });
        let posting = parse_posting("CesiumAstro", &job);
        assert_eq!(posting.url, "https://jobs.lever.co/CesiumAstro/abc-123");
    }

    #[test]
    fn an_empty_board_parses_to_no_postings_without_error() {
        // 200 with `[]` means a live board with nothing posted — keep polling. Distinct from a
        // 404, which means the company is not on Lever at all.
        assert!(parse_board("acceldata", &[]).is_empty());
    }

    #[test]
    fn an_unrecognized_interval_contributes_no_period_word() {
        assert_eq!(interval_words(Some("one-time-payment")), None);
        assert_eq!(interval_words(None), None);
        assert_eq!(interval_words(Some("per-year-salary")), Some("per year"));
        assert_eq!(interval_words(Some("PER-MONTH-SALARY")), Some("per month"));
    }
}
