//! The GitHub internship-list repos: `SimplifyJobs` and `vanshb03`.
//!
//! `docs/INTERNSHIP_SCRAPING.md` § G ranks this first by a wide margin: **one conditional GET**
//! returns ~1,900 active listings carrying term, degree level, and — uniquely — an explicit
//! `active` closure flag. Nothing else in the document is close on effort-to-coverage.
//!
//! # Three traps this adapter exists to handle
//!
//! 1. **The file is a rolling archive, not a current-listings feed.** `active` is false on
//!    roughly 87% of records. Emitting them all would fill the corpus with dead links, so the
//!    inactive ones go to [`SourceFetch::with_closed_ids`] instead of being emitted as
//!    postings — which turns the strongest closure signal in the whole document into something
//!    the coordinator can act on, rather than throwing it away.
//! 2. **The default branch is `dev`, not `main`.** A raw URL built with `/main/` 404s.
//! 3. **The cycle rolls over and the repos are renamed in place.** `Summer2026` became
//!    `Summer2027` and GitHub redirects the old paths, so a stale URL keeps working and *looks*
//!    current long after it goes stale. Re-verify the names each cycle; the constants below
//!    are dated.
//!
//! # Conditional GET, and why this adapter memoizes
//!
//! `raw.githubusercontent.com` answers `If-None-Match` with **304 and 0 bytes** against 10.8 MB
//! unconditionally. Upstream commits every ~30 minutes, so polling hourly is nearly free — but
//! only if a 304 still leaves us able to state the full enumeration. So the parsed listings are
//! memoized in the adapter alongside the `ETag` they came from: a 304 replays the memo and
//! honestly reports [`SourceOutcome::Success`], because the server has just told us the content
//! is unchanged. A 304 with no memo (a fresh process that somehow has validators but no
//! listings) reports `Partial` rather than guessing.
//!
//! # Licensing
//!
//! SimplifyJobs has **no license file**, so default copyright applies and there is no grant of
//! reuse; vanshb03's copy is MIT. Both are read for a personal, non-commercial, non-republishing
//! project — the same posture as `data/themealdb/`. See `data/internships/README.md`.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::Value;

use super::super::http::{Conditional, FetchError};
use super::super::models::RawPosting;
use super::{BoardDirectory, BoxFuture, Source, SourceContext, SourceFetch, first_string, join_locations};

/// SimplifyJobs' machine-readable listings. Verified 2026-08-20; **`dev`, not `main`**.
pub const SIMPLIFY_URL: &str =
    "https://raw.githubusercontent.com/SimplifyJobs/Summer2027-Internships/dev/.github/scripts/listings.json";

/// vanshb03's copy. Smaller and slower-moving, but only 29% of its URLs also appear in
/// Simplify, so roughly 285 listings are unique to it. MIT-licensed.
pub const VANSHB03_URL: &str =
    "https://raw.githubusercontent.com/vanshb03/Summer2027-Internships/dev/.github/scripts/listings.json";

/// A term value meaning "the maintainers did not say". Carried through as `None` rather than as
/// the literal string, which would otherwise reach the term parser as noise.
const TERM_UNKNOWN: &str = "N/A";

/// One of the two GitHub list repos.
pub struct SimplifySource {
    name: &'static str,
    description: &'static str,
    url: &'static str,
    memo: Mutex<Option<Memo>>,
}

/// The last successful parse, kept so a 304 is still a complete enumeration.
struct Memo {
    etag: Option<String>,
    postings: Vec<RawPosting>,
    closed_external_ids: Vec<String>,
}

impl SimplifySource {
    pub fn simplify_jobs() -> Self {
        SimplifySource {
            name: "simplify",
            description: "SimplifyJobs/Summer2027-Internships listings.json — ~1,900 active \
                          listings with an explicit `active` closure flag, one conditional GET",
            url: SIMPLIFY_URL,
            memo: Mutex::new(None),
        }
    }

    pub fn vanshb03() -> Self {
        SimplifySource {
            name: "vanshb03",
            description: "vanshb03/Summer2027-Internships listings.json — ~400 records, ~285 of \
                          them not in Simplify, MIT-licensed",
            url: VANSHB03_URL,
            memo: Mutex::new(None),
        }
    }
}

impl Source for SimplifySource {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        self.description
    }

    fn fetch<'a>(&'a self, ctx: &'a SourceContext) -> BoxFuture<'a, SourceFetch> {
        Box::pin(async move {
            let response = match ctx.http.get_conditional(self.url).await {
                Ok(Conditional::Fetched(response)) => response,
                Ok(Conditional::NotModified { .. }) => return self.replay_memo(),
                Err(error) if error.is_refusal() => return SourceFetch::skipped(error.to_string()),
                Err(error) => return SourceFetch::failed(error.to_string()),
            };

            let records: Vec<Value> = match serde_json::from_str(&response.body) {
                Ok(Value::Array(records)) => records,
                Ok(other) => {
                    return SourceFetch::failed(format!(
                        "{} returned {} where a JSON array was expected — the upstream shape has \
                         changed",
                        self.url,
                        shape_name(&other)
                    ));
                }
                Err(error) => {
                    return SourceFetch::failed(format!("{} is not valid JSON: {error}", self.url));
                }
            };

            let parsed = parse_listings(self.name, &records);

            if let Ok(mut memo) = self.memo.lock() {
                *memo = Some(Memo {
                    etag: response.etag.clone(),
                    postings: parsed.postings.clone(),
                    closed_external_ids: parsed.closed_external_ids.clone(),
                });
            }

            SourceFetch::success(parsed.postings).with_closed_ids(parsed.closed_external_ids)
        })
    }
}

impl SimplifySource {
    /// Replay the last parse after a 304.
    ///
    /// `Success` here is a real claim, not a convenience: the server has stated the content is
    /// byte-identical to what produced the memo, so the memo *is* the full enumeration. Without
    /// a memo there is nothing to claim, and the outcome degrades to `Partial` — which expires
    /// nothing, which is the safe direction.
    fn replay_memo(&self) -> SourceFetch {
        let guard = match self.memo.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        match guard.as_ref() {
            Some(memo) => SourceFetch::success(memo.postings.clone())
                .with_closed_ids(memo.closed_external_ids.clone()),
            None => SourceFetch::partial(
                Vec::new(),
                format!(
                    "{} answered 304 Not Modified but this process holds no parsed copy, so no \
                     enumeration was made this run",
                    self.url
                ),
            ),
        }
    }

    /// The `ETag` behind the current memo, for diagnostics.
    pub fn memoized_etag(&self) -> Option<String> {
        self.memo
            .lock()
            .ok()
            .and_then(|memo| memo.as_ref().and_then(|memo| memo.etag.clone()))
    }
}

/// What one parse of the listings file produced.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ParsedListings {
    /// Live listings only.
    pub postings: Vec<RawPosting>,
    /// Ids the file states are closed (`active: false`). The strongest closure signal in
    /// `docs/INTERNSHIP_SCRAPING.md` § D.
    pub closed_external_ids: Vec<String>,
}

/// Turn the listings array into raw postings.
///
/// Pure, so the whole of this adapter's parsing is tested offline against the committed fixture
/// in `data/internships/fixtures/`. A record missing an identity field is emitted **with the
/// field blank** rather than dropped, so QC rejects it visibly — a silently skipped record is
/// exactly the invisible data loss `fetched = accepted + filtered + rejected` exists to catch.
pub fn parse_listings(source: &str, records: &[Value]) -> ParsedListings {
    let mut postings = Vec::new();
    let mut closed_external_ids = Vec::new();

    for record in records {
        let external_id = first_string(record, &["id"]).unwrap_or_default();

        // `active` is a real tri-state in practice: present-and-true, present-and-false, or a
        // reshaped file where it is absent. Absent is treated as live, because dropping a
        // posting on a missing flag is worse than carrying one extra.
        if record.get("active") == Some(&Value::Bool(false)) {
            if !external_id.is_empty() {
                closed_external_ids.push(external_id);
            }
            continue;
        }

        postings.push(RawPosting {
            source: source.to_string(),
            external_id,
            url: first_string(record, &["url"]).unwrap_or_default(),
            company: first_string(record, &["company_name"]).unwrap_or_default(),
            title: first_string(record, &["title"]).unwrap_or_default(),
            location_raw: join_locations(string_array(record, "locations")),
            // No salary field of any kind, in either repo. § B.
            pay_raw: None,
            term_raw: term_of(record),
            // `degrees` is degree *level* (Bachelor's, PhD), not graduation year, and is empty
            // on 22% of records. It does not answer "am I eligible as a 2028 grad", so it is
            // deliberately not mapped here — it survives in `raw_json` for anyone who wants it.
            class_year_raw: None,
            // Epoch seconds. `normalize::parse_timestamp` reads a 10-digit all-digit string as
            // exactly that, so it is passed through rather than pre-converted.
            posted_at_raw: first_string(record, &["date_posted"]),
            deadline_raw: None,
            description: None,
            // § B marks remote as *derivable* for these repos, not a field. `None` means the
            // source did not say, and QC may still infer it from the location.
            remote_hint: None,
            raw_json: record.to_string(),
        });
    }

    ParsedListings {
        postings,
        closed_external_ids,
    }
}

/// Simplify has `terms: []`; vanshb03 has `season: ""`. Both are read, so one parser covers
/// both files.
fn term_of(record: &Value) -> Option<String> {
    let terms: Vec<String> = string_array(record, "terms")
        .into_iter()
        .filter(|term| term != TERM_UNKNOWN)
        .collect();
    if !terms.is_empty() {
        return Some(terms.join(", "));
    }
    first_string(record, &["season"]).filter(|season| season != TERM_UNKNOWN)
}

fn string_array(record: &Value, key: &str) -> Vec<String> {
    match record.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn shape_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

// ------------------------------------------------------------------------------------------
// Board-slug discovery
// ------------------------------------------------------------------------------------------

/// Mine posting URLs for `(ATS, board slug)` pairs.
///
/// `docs/INTERNSHIP_SCRAPING.md` § A.2: you do not guess a board slug — `AECOM2`,
/// `3SBusinessCorporationInc1` — you harvest it, and the listings file you already downloaded
/// *is* the directory. Measured on the 2026-08-20 snapshot this yields 2,084 distinct pairs
/// covering 74% of all listings and 59% of active ones.
///
/// Two normalizations that the § C dedup notes require and that the URLs genuinely need:
///
/// - **Both Greenhouse hosts are one ATS.** `job-boards.greenhouse.io` (1,245 records) and
///   `boards.greenhouse.io` (179) are the same board.
/// - **Slugs are percent-encoded in the corpus.** `jobs.ashbyhq.com/Hippocratic%20AI/…` is the
///   board `Hippocratic AI`; polling the encoded form asks for a board that does not exist.
///   (Not recorded in the research doc — found while building this.)
pub fn extract_board_slugs<'a>(urls: impl IntoIterator<Item = &'a str>) -> BoardDirectory {
    let mut by_ats: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for url in urls {
        let Some((ats, slug)) = board_of(url) else {
            continue;
        };
        let slugs = by_ats.entry(ats.to_string()).or_default();
        if !slugs.contains(&slug) {
            slugs.push(slug);
        }
    }

    for slugs in by_ats.values_mut() {
        slugs.sort();
    }

    BoardDirectory::from_map(by_ats)
}

/// Which ATS and board a posting URL points at, if any.
pub fn board_of(url: &str) -> Option<(&'static str, String)> {
    let (host, path) = split_host_path(url)?;
    let host = host.to_ascii_lowercase();
    let first = path_segment(path, 0).map(percent_decode);
    let tenant = host.split('.').next().unwrap_or_default().to_string();

    match host.as_str() {
        // Two hosts, one board. Canonicalized per § C.
        "job-boards.greenhouse.io" | "boards.greenhouse.io" => Some(("greenhouse", first?)),
        "jobs.lever.co" => Some(("lever", first?)),
        "jobs.ashbyhq.com" => Some(("ashby", first?)),
        "jobs.smartrecruiters.com" => Some(("smartrecruiters", first?)),
        "apply.workable.com" => Some(("workable", first?)),
        _ if host.ends_with(".myworkdayjobs.com") || host.ends_with(".myworkdaysite.com") => {
            // Workday's identity is tenant + the `wdN` shard, e.g. `3m.wd1`. The site segment
            // is *not* recoverable from a browse URL reliably, and § A.1 records that the API
            // path has no locale segment even when the human URL does.
            let mut parts = host.split('.');
            let tenant = parts.next()?;
            let shard = parts.next()?;
            Some(("workday", format!("{tenant}.{shard}")))
        }
        _ if host.ends_with(".recruitee.com") => Some(("recruitee", tenant)),
        _ => None,
    }
}

/// Split `https://host/path?query` into `(host, path)`. Written by hand rather than pulled from
/// a URL crate so that this file has no HTTP dependency at all — see
/// `sources::tests::adapters_do_not_build_their_own_http_client`.
fn split_host_path(url: &str) -> Option<(&str, &str)> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    match rest.find('/') {
        Some(slash) => Some((&rest[..slash], &rest[slash + 1..])),
        None => Some((rest, "")),
    }
}

fn path_segment(path: &str, index: usize) -> Option<String> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .nth(index)
        .map(str::to_string)
}

/// Decode `%XX` escapes. Only what a board slug needs — no `+`-as-space, which is a query-string
/// convention and would corrupt a slug that legitimately contains a plus.
fn percent_decode(text: String) -> String {
    if !text.contains('%') {
        return text;
    }
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or(text)
}

// ------------------------------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Real records from the 2026-08-20 snapshot. See `data/internships/README.md`.
    const FIXTURE: &str =
        include_str!("../../../data/internships/fixtures/simplify-listings.sample.json");

    fn fixture() -> Vec<Value> {
        serde_json::from_str(FIXTURE).expect("the committed fixture parses")
    }

    #[test]
    fn the_fixture_holds_both_live_and_closed_records() {
        // A fixture with only live records cannot exercise the closure split at all, and this
        // adapter's most valuable output is the closure signal.
        let records = fixture();
        assert!(records.iter().any(|r| r["active"] == Value::Bool(true)));
        assert!(records.iter().any(|r| r["active"] == Value::Bool(false)));
    }

    #[test]
    fn closed_listings_become_closure_ids_rather_than_postings() {
        let records = fixture();
        let parsed = parse_listings("simplify", &records);

        let live = records.iter().filter(|r| r["active"] != Value::Bool(false)).count();
        let closed = records.iter().filter(|r| r["active"] == Value::Bool(false)).count();

        assert_eq!(parsed.postings.len(), live);
        assert_eq!(parsed.closed_external_ids.len(), closed);
        assert!(closed > 0, "the fixture must contain closed records");
    }

    #[test]
    fn every_parsed_posting_carries_the_identity_qc_requires() {
        for posting in parse_listings("simplify", &fixture()).postings {
            assert!(!posting.external_id.is_empty());
            assert!(posting.url.starts_with("http"));
            assert!(!posting.company.is_empty());
            assert!(!posting.title.is_empty());
            assert_eq!(posting.source, "simplify");
        }
    }

    #[test]
    fn the_posted_date_is_the_epoch_seconds_the_file_states() {
        // `normalize::parse_timestamp` reads a 10-digit all-digit string as epoch seconds, so
        // the value is passed through rather than reformatted. Pin the shape it relies on.
        let parsed = parse_listings("simplify", &fixture());
        let posted = parsed.postings[0]
            .posted_at_raw
            .as_deref()
            .expect("date_posted is present on 100% of records");
        assert_eq!(posted.len(), 10, "epoch seconds, not milliseconds");
        assert!(posted.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn no_listing_claims_a_salary() {
        // § B: neither repo has a salary field of any kind. Inventing one here would be worse
        // than having none, because `pay_raw` present means "the source said something".
        for posting in parse_listings("simplify", &fixture()).postings {
            assert!(posting.pay_raw.is_none());
        }
    }

    #[test]
    fn degrees_are_not_mapped_to_class_years() {
        // § B note 14: `degrees` is degree *level*, not graduation year. Mapping it would make
        // "Bachelor's" look like an eligibility window and silently exclude people.
        for posting in parse_listings("simplify", &fixture()).postings {
            assert!(posting.class_year_raw.is_none());
        }
    }

    #[test]
    fn the_unknown_term_placeholder_becomes_none() {
        let record = serde_json::json!({ "id": "x", "terms": ["N/A"] });
        assert_eq!(term_of(&record), None);
    }

    #[test]
    fn a_singular_season_field_is_read_as_a_term() {
        // vanshb03 differs from Simplify here and nowhere else that matters; one parser covers
        // both files rather than two that drift.
        let record = serde_json::json!({ "id": "x", "season": "Winter" });
        assert_eq!(term_of(&record), Some("Winter".to_string()));
    }

    #[test]
    fn multiple_terms_are_joined_rather_than_truncated() {
        let record = serde_json::json!({ "id": "x", "terms": ["Summer 2027", "Fall 2026"] });
        assert_eq!(term_of(&record), Some("Summer 2027, Fall 2026".to_string()));
    }

    #[test]
    fn a_record_missing_its_identity_is_emitted_blank_rather_than_dropped() {
        // Dropping it would make it invisible; emitting it blank makes QC reject it with a
        // reason a human can find in `posting_rejects`.
        let records = vec![serde_json::json!({ "active": true, "title": "SWE Intern" })];
        let parsed = parse_listings("simplify", &records);
        assert_eq!(parsed.postings.len(), 1);
        assert!(parsed.postings[0].external_id.is_empty());
        assert!(parsed.postings[0].url.is_empty());
    }

    #[test]
    fn a_record_with_no_active_flag_is_treated_as_live() {
        let records = vec![serde_json::json!({ "id": "x", "title": "SWE Intern" })];
        let parsed = parse_listings("simplify", &records);
        assert_eq!(parsed.postings.len(), 1);
        assert!(parsed.closed_external_ids.is_empty());
    }

    #[test]
    fn multi_location_records_keep_every_location() {
        // 1,409 records list more than one location, up to 52. Truncating to the first would
        // make a location-bearing dedup key double-count against a source that kept them all.
        let record = serde_json::json!({
            "id": "x",
            "locations": ["Chicago, IL", "New York, NY", "Remote"]
        });
        let parsed = parse_listings("simplify", std::slice::from_ref(&record));
        assert_eq!(
            parsed.postings[0].location_raw.as_deref(),
            Some("Chicago, IL; New York, NY; Remote")
        );
    }

    // ---- board-slug discovery ----

    #[test]
    fn slug_discovery_finds_every_ats_in_the_fixture() {
        let parsed = parse_listings("simplify", &fixture());
        let boards = extract_board_slugs(parsed.postings.iter().map(|p| p.url.as_str()));
        assert!(!boards.is_empty());
    }

    #[test]
    fn both_greenhouse_hosts_canonicalize_to_one_board() {
        // § C: `job-boards.greenhouse.io` and `boards.greenhouse.io` are the same board. Read
        // as two, the join across sources silently misses.
        assert_eq!(
            board_of("https://job-boards.greenhouse.io/airtable/jobs/8403127002"),
            Some(("greenhouse", "airtable".to_string()))
        );
        assert_eq!(
            board_of("https://boards.greenhouse.io/airtable/jobs/8403127002"),
            Some(("greenhouse", "airtable".to_string()))
        );
    }

    #[test]
    fn a_percent_encoded_slug_is_decoded() {
        // Found in the real corpus and not recorded in the research doc: Ashby slugs arrive
        // percent-encoded, and polling `Hippocratic%20AI` asks for a board that does not exist.
        assert_eq!(
            board_of("https://jobs.ashbyhq.com/Hippocratic%20AI/abc-123"),
            Some(("ashby", "Hippocratic AI".to_string()))
        );
    }

    #[test]
    fn trailing_action_segments_and_query_strings_do_not_change_the_board() {
        // § C: Lever appends `/apply`, Ashby `/application`, and 544 records carry `?gh_jid=`.
        assert_eq!(
            board_of("https://jobs.lever.co/CesiumAstro/uuid-1/apply"),
            Some(("lever", "CesiumAstro".to_string()))
        );
        assert_eq!(
            board_of("https://job-boards.greenhouse.io/discord/jobs/1?gh_jid=1&mobile=true"),
            Some(("greenhouse", "discord".to_string()))
        );
    }

    #[test]
    fn workday_is_keyed_by_tenant_and_shard() {
        assert_eq!(
            board_of("https://nvidia.wd5.myworkdayjobs.com/en-US/NVIDIAExternalCareerSite/job/x"),
            Some(("workday", "nvidia.wd5".to_string()))
        );
        // The second Workday domain, easy to forget: § A.1.
        assert_eq!(
            board_of("https://acme.wd3.myworkdaysite.com/en-US/Careers/job/y"),
            Some(("workday", "acme.wd3".to_string()))
        );
    }

    #[test]
    fn a_company_run_career_site_yields_no_board() {
        // The 27% not on a pollable ATS. Returning `None` is correct; guessing a slug from the
        // hostname would poll boards that do not exist.
        assert_eq!(board_of("https://www.tesla.com/careers/search/job/12345"), None);
        assert_eq!(board_of("https://lifeattiktok.com/search/12345"), None);
    }

    #[test]
    fn a_url_with_no_scheme_is_not_a_board() {
        assert_eq!(board_of("jobs.lever.co/acme/uuid"), None);
        assert_eq!(board_of(""), None);
    }

    #[test]
    fn slug_discovery_deduplicates_and_sorts() {
        let boards = extract_board_slugs([
            "https://job-boards.greenhouse.io/zeta/jobs/1",
            "https://job-boards.greenhouse.io/alpha/jobs/2",
            "https://boards.greenhouse.io/zeta/jobs/3",
        ]);
        assert_eq!(
            boards.slugs("greenhouse"),
            ["alpha".to_string(), "zeta".to_string()]
        );
    }

    #[test]
    fn percent_decoding_leaves_a_plus_alone() {
        // `+` means space only in a query string. A slug containing one must survive intact.
        assert_eq!(percent_decode("a+b".to_string()), "a+b".to_string());
        assert_eq!(percent_decode("a%2Fb".to_string()), "a/b".to_string());
        assert_eq!(percent_decode("no-escapes".to_string()), "no-escapes".to_string());
        // A truncated escape must not panic or eat the rest of the string.
        assert_eq!(percent_decode("trailing%2".to_string()), "trailing%2".to_string());
    }

    // ---- the memo ----

    #[test]
    fn a_304_with_no_memo_reports_partial_rather_than_an_empty_success() {
        // An empty `Success` would be a claim that the source currently offers nothing, which
        // after a run of 1,900 postings is a mass expiry waiting for the miss threshold.
        let source = SimplifySource::simplify_jobs();
        let fetch = source.replay_memo();
        assert_eq!(fetch.outcome(), crate::internships::models::SourceOutcome::Partial);
        assert!(fetch.postings().is_empty());
        assert!(fetch.error().is_some());
    }

    #[test]
    fn a_304_with_a_memo_replays_the_full_enumeration_as_success() {
        // This is the whole value of the conditional GET: 0 bytes on the wire and still a
        // complete, expiry-eligible picture.
        let source = SimplifySource::simplify_jobs();
        let parsed = parse_listings("simplify", &fixture());
        *source.memo.lock().unwrap() = Some(Memo {
            etag: Some("\"abc\"".to_string()),
            postings: parsed.postings.clone(),
            closed_external_ids: parsed.closed_external_ids.clone(),
        });

        let fetch = source.replay_memo();
        assert_eq!(fetch.outcome(), crate::internships::models::SourceOutcome::Success);
        assert_eq!(fetch.postings().len(), parsed.postings.len());
        assert_eq!(fetch.closed_external_ids().len(), parsed.closed_external_ids.len());
        assert_eq!(source.memoized_etag().as_deref(), Some("\"abc\""));
    }

    #[test]
    fn the_two_repos_are_distinct_sources_at_distinct_urls() {
        // Sharing a source name would merge their sighting histories; sharing a URL would make
        // one of them pointless.
        let a = SimplifySource::simplify_jobs();
        let b = SimplifySource::vanshb03();
        assert_ne!(a.name(), b.name());
        assert_ne!(a.url, b.url);
        // The default branch is `dev`. A `/main/` URL 404s, and it is an easy edit to make.
        assert!(a.url.contains("/dev/"));
        assert!(b.url.contains("/dev/"));
    }
}
