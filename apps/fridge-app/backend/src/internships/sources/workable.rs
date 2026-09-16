//! Workable's public job-board widget API.
//!
//! `GET https://apply.workable.com/api/v1/widget/accounts/{account}?details=true` returns
//! `{name, description, jobs: […]}` — the whole board in one request, no pagination.
//! `docs/INTERNSHIP_SCRAPING.md` § A.1 listed this endpoint with its job shape **unverified**,
//! because the one account tried then had no jobs. Verified 2026-09-16 against all 43 harvested
//! accounts: every one answered 200, 39 had jobs, 1,648 jobs in total.
//!
//! `robots.txt` on `apply.workable.com` is `Disallow:` (empty) for `*` — everything permitted.
//!
//! # "No pagination" was checked, not assumed
//!
//! The response carries no `total` and no cursor, and the two largest boards returned 248 and
//! 245 jobs — close enough to 250 to look like a silent cap. A silent cap would be the worst
//! kind of bug here: a truncated board reads as a complete enumeration, and a complete
//! enumeration missing live postings is exactly what makes them expire (§ D). So the counts were
//! compared against the paginated v3 API's own `total` for the same accounts — 248 vs 247,
//! 245 vs 245, 61 vs 60. The widget returns the whole board, and occasionally one more than a
//! request made a second later. Each board is therefore a valid scope.
//!
//! # Three things this source gets wrong if you take the obvious route
//!
//! 1. **The job `url` names no account.** It is `apply.workable.com/j/{shortcode}`, but the
//!    URL that Simplify links to — and that `dedup::ats_identity` parses — is
//!    `apply.workable.com/{account}/j/{shortcode}`. Emitting the API's URL would make every
//!    Workable posting miss its own duplicate from Simplify. We know the account, because it is
//!    the board we asked for, so the canonical form is built here. The same move Lever makes
//!    for its missing company name.
//! 2. **`employment_type` is not an internship signal.** Across every harvested board it takes
//!    six values — `Full-time`, `Part-time`, `Contract`, `Temporary`, `Other`, and empty — and
//!    none of them is "Internship". Real software internships are labelled `Full-time`,
//!    `Temporary`, `Other` and blank. It is therefore **not** handed to QC as the term hint the
//!    way Ashby's `employmentType == "Intern"` is: it would add noise to the one field QC reads
//!    beside the title, and carry no information. The title decides.
//! 3. **There is no pay field at all**, on any board. `pay_raw` is `None`, which the ranking
//!    reads as unknown rather than zero.
//!
//! # A 404 is a board that is gone
//!
//! A nonexistent account answers 404 on this endpoint, the same definitive "no such board" the
//! other three ATS adapters treat as a completed scope with nothing in it and record in
//! `source_run_scopes.gone`.

use serde_json::Value;

use super::super::models::{RawPosting, ScopeRun};
use super::{
    BoxFuture, Source, SourceContext, SourceFetch, first_string,
    greenhouse::{completed_scope, finish},
    join_locations,
};

/// The ATS key in [`BoardDirectory`](super::BoardDirectory).
pub const ATS: &str = "workable";

pub struct WorkableSource;

impl Default for WorkableSource {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkableSource {
    pub fn new() -> Self {
        WorkableSource
    }
}

/// The widget endpoint, with the parameter that includes each job's details.
pub fn board_url(account: &str) -> String {
    format!("https://apply.workable.com/api/v1/widget/accounts/{account}?details=true")
}

/// The canonical posting URL: the form Simplify links to and `dedup::ats_identity` parses.
pub fn posting_url(account: &str, shortcode: &str) -> String {
    format!("https://apply.workable.com/{account}/j/{shortcode}")
}

impl Source for WorkableSource {
    fn name(&self) -> &str {
        ATS
    }

    fn description(&self) -> &str {
        "Workable widget API — whole board per request, no pay field, remote boolean"
    }

    fn fetch<'a>(&'a self, ctx: &'a SourceContext) -> BoxFuture<'a, SourceFetch> {
        Box::pin(async move {
            let all_slugs = ctx.boards.slugs(ATS);
            if all_slugs.is_empty() {
                return SourceFetch::failed(
                    "no Workable accounts are known — harvest them from Simplify's `url` field \
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
                match ctx.http.get(&board_url(slug)).await {
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
                    // Report no scopes with it: `Skipped` means we did not fetch this source.
                    Err(error) if error.is_refusal() => {
                        return SourceFetch::skipped(error.to_string());
                    }
                    // No such account: a definitive zero, recorded as gone.
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
                    "internships: {} Workable board(s) 404'd and should be retired: {}",
                    retired.len(),
                    retired.join(", ")
                );
            }

            finish(
                "Workable",
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

/// One enumerated board: its postings and its scope verdict, from a **single** parse — see
/// [`completed_scope`] for why the two are never computed apart.
fn board_result(account: &str, body: &Value) -> (Vec<RawPosting>, ScopeRun) {
    let postings = parse_board(account, body);
    let scope = completed_scope(account, &postings);
    (postings, scope)
}

/// Turn one board response into raw postings. Pure, tested offline against the committed
/// fixture.
pub fn parse_board(account: &str, body: &Value) -> Vec<RawPosting> {
    let Some(Value::Array(jobs)) = body.get("jobs") else {
        return Vec::new();
    };
    // The account's display name, when the response has one. Better than the slug, which is
    // lowercase and hyphenated ("pony-dot-ai" for Pony.ai).
    let company = first_string(body, &["name"]).unwrap_or_else(|| account.to_string());
    jobs.iter()
        .map(|job| parse_job(account, &company, job))
        .collect()
}

fn parse_job(account: &str, company: &str, job: &Value) -> RawPosting {
    // `shortcode` is filled on every one of 1,648 jobs measured, and is the id in the URL.
    // `code` is an employer-assigned requisition number, empty on two thirds of them.
    let external_id = first_string(job, &["shortcode"]).unwrap_or_default();

    RawPosting {
        source: ATS.to_string(),
        url: posting_url(account, &external_id),
        external_id,
        company: company.to_string(),
        title: first_string(job, &["title"]).unwrap_or_default(),
        location_raw: locations(job),
        pay_raw: None,
        // See the module doc: none of the six values `employment_type` takes is "internship".
        term_raw: None,
        class_year_raw: None,
        // A bare date, "2026-09-08". Filled on every job measured.
        posted_at_raw: first_string(job, &["published_on", "created_at"]),
        deadline_raw: None,
        description: first_string(job, &["description"]),
        // A real boolean, filled on every job measured.
        remote_hint: job.get("telecommuting").and_then(Value::as_bool),
        raw_json: job.to_string(),
    }
}

/// Every location, from the structured array where present and the flat fields otherwise.
fn locations(job: &Value) -> Option<String> {
    let mut found = Vec::new();
    if let Some(Value::Array(entries)) = job.get("locations") {
        for entry in entries {
            let parts: Vec<String> = ["city", "region", "country"]
                .iter()
                .filter_map(|key| first_string(entry, &[key]))
                .collect();
            if !parts.is_empty() {
                found.push(parts.join(", "));
            }
        }
    }
    if found.is_empty() {
        let parts: Vec<String> = ["city", "state", "country"]
            .iter()
            .filter_map(|key| first_string(job, &[key]))
            .collect();
        if !parts.is_empty() {
            found.push(parts.join(", "));
        }
    }
    join_locations(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internships::dedup::ats_identity;
    use crate::internships::models::QcOutcome;
    use crate::internships::normalize::normalize;

    /// Real jobs from the `pony-dot-ai` board, fetched 2026-09-16. Descriptions replaced with a
    /// marked placeholder; see `data/internships/README.md`.
    const FIXTURE: &str =
        include_str!("../../../data/internships/fixtures/workable-board.sample.json");

    fn fixture() -> Value {
        serde_json::from_str(FIXTURE).expect("the committed fixture parses")
    }

    #[test]
    fn every_job_in_the_fixture_parses_into_an_identifiable_posting() {
        let postings = parse_board("pony-dot-ai", &fixture());
        assert_eq!(postings.len(), 4);
        for posting in &postings {
            assert!(!posting.external_id.is_empty(), "{posting:?}");
            assert!(!posting.title.is_empty(), "{posting:?}");
        }
    }

    #[test]
    fn the_company_is_the_accounts_display_name_not_its_slug() {
        // The account's own `name`, exactly as Workable returns it — "pony.ai", lowercase,
        // which is how that employer spells itself there. The slug "pony-dot-ai" is the thing
        // this must not fall back to while a name is present.
        let postings = parse_board("pony-dot-ai", &fixture());
        assert!(postings.iter().all(|p| p.company == "pony.ai"), "{postings:?}");
        // …and does fall back to it when the response carries no name.
        let nameless = serde_json::json!({ "jobs": [{ "shortcode": "X1", "title": "SWE Intern" }] });
        assert_eq!(parse_board("pony-dot-ai", &nameless)[0].company, "pony-dot-ai");
    }

    #[test]
    fn the_posting_url_carries_the_account_so_dedup_can_see_it() {
        // The API's own `url` is `apply.workable.com/j/{shortcode}`, with no account. That form
        // produces no ATS identity, so the posting would miss its own duplicate from Simplify.
        let postings = parse_board("pony-dot-ai", &fixture());
        let intern = postings
            .iter()
            .find(|p| p.title.starts_with("Software Engineer Intern"))
            .expect("the software intern is in the fixture");
        assert_eq!(intern.url, "https://apply.workable.com/pony-dot-ai/j/BA5FFDBC71");

        let identity = ats_identity(&intern.url).expect("dedup recognises the canonical form");
        assert_eq!(identity.ats, "workable");

        // And the API's own form is the one that would have been invisible.
        assert!(ats_identity("https://apply.workable.com/j/BA5FFDBC71").is_none());
    }

    #[test]
    fn the_same_posting_from_simplify_dedups_to_the_same_identity() {
        // Simplify links the apply page. Trailing `/apply` is not identity.
        let ours = ats_identity(&posting_url("pony-dot-ai", "BA5FFDBC71"));
        let theirs = ats_identity("https://apply.workable.com/pony-dot-ai/j/BA5FFDBC71/apply");
        assert!(ours.is_some());
        assert_eq!(ours, theirs);
    }

    #[test]
    fn employment_type_is_never_handed_to_qc_as_a_term() {
        // The fixture's software intern has `employment_type: ""` and its research intern has
        // `"Other"`; across every harvested board full-time roles and internships share the
        // same six values.
        for posting in parse_board("pony-dot-ai", &fixture()) {
            assert_eq!(posting.term_raw, None, "{}", posting.title);
        }
    }

    #[test]
    fn there_is_no_pay_to_invent() {
        for posting in parse_board("pony-dot-ai", &fixture()) {
            assert_eq!(posting.pay_raw, None);
        }
    }

    #[test]
    fn the_remote_flag_is_a_real_boolean() {
        for posting in parse_board("pony-dot-ai", &fixture()) {
            assert_eq!(posting.remote_hint, Some(false), "{}", posting.title);
        }
    }

    #[test]
    fn qc_keeps_the_software_intern_and_filters_the_others() {
        // The whole pipeline, from a real response. The adapter does not filter; QC does, and
        // it must still be able to tell these apart with no employment-type hint.
        let now = chrono::Utc::now();
        let kept: Vec<String> = parse_board("pony-dot-ai", &fixture())
            .into_iter()
            .filter_map(|raw| match normalize(&raw, now) {
                QcOutcome::Accepted(posting) => Some(posting.title),
                _ => None,
            })
            .collect();
        assert!(
            kept.iter().any(|t| t.starts_with("Software Engineer Intern")),
            "the software intern must survive QC: kept {kept:?}"
        );
        assert!(!kept.iter().any(|t| t == "HR Intern"), "not software: kept {kept:?}");
        assert!(
            !kept.iter().any(|t| t.starts_with("(Senior)")),
            "not an internship: kept {kept:?}"
        );
    }

    #[test]
    fn a_boards_scope_ids_are_exactly_the_postings_it_returned() {
        let (postings, scope) = board_result("pony-dot-ai", &fixture());
        assert!(scope.is_completed());
        assert_eq!(scope.fetched, postings.len() as i64);
        let ids: Vec<String> = postings.iter().map(|p| p.external_id.clone()).collect();
        assert_eq!(scope.external_ids, ids);
    }

    #[test]
    fn a_reshaped_response_yields_no_postings_rather_than_panicking() {
        assert!(parse_board("pony-dot-ai", &serde_json::json!({ "jobs": "nope" })).is_empty());
        assert!(parse_board("pony-dot-ai", &serde_json::json!({})).is_empty());
    }
}
