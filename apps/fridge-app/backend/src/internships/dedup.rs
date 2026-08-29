//! Deciding when two postings from different sources are the same job.
//!
//! # The key, and why it is shaped this way
//!
//! Straight from `docs/INTERNSHIP_SCRAPING.md` § C, which measured this rather than guessing:
//!
//! 1. **Primary — the canonical ATS triple `(ats, board_slug, job_id)`**, recovered from the
//!    posting URL. It is an exact key needing no fuzzy matching, and it works because the
//!    GitHub aggregator lists link *directly at the ATS* — so both sides of the join carry the
//!    same identity. Covers 73% of Simplify listings (58% of active ones).
//! 2. **Fallback — `(company_key, title_key)`** for everything not on a pollable ATS.
//!
//! **Location is deliberately not in either key.** § C measured 1,409 Simplify records listing
//! more than one location (max 52). If one source explodes a posting into one row per location
//! and another keeps it as a single row, a location-bearing key double-counts the job. Its
//! advice is to key on the location *set* or drop it entirely; dropping it is simpler and
//! fails toward merging, which is the direction we want here.
//!
//! # URL normalization is load-bearing, not tidying
//!
//! Every item below was counted in a real corpus, and each one silently breaks the join:
//! Lever appends `/apply` (579 records), Ashby appends `/application`, 544 records carry
//! `?gh_jid=`, 574 `?mobile=`, 339 `?ats=`, and Greenhouse serves the same board from **two
//! hosts** (`job-boards.greenhouse.io`, 1,244 records; `boards.greenhouse.io`, 179).
//!
//! # What this module deliberately does not do
//!
//! **No fuzzy matching.** § C measured 18 groups of company-name variants collapsing within
//! Simplify alone — `KLA` / `KLA Corporation`, `WhatNot` / `Whatnot`, `Moog` / `Moog ` — and
//! notes that parent/subsidiary pairs ("Google" / "Alphabet") are not string problems at all.
//! Fuzzy company/title matching is the NLP learning area, reserved for the repo owner; § C
//! also concludes `src/nlp.rs` is the wrong shape to reuse here (it is a ranked typeahead, not
//! a pairwise identity test, and its substring band would fire on "Engineer" across most of
//! the corpus).
//!
//! [`FuzzyMatcher`] is the seam that work plugs into. Until something implements it, dedup is
//! exact-key only, which **under-merges** — the safer failure, per § C.

use crate::internships::models::{NormalizedPosting, Season};

/// A posting's identity at its applicant-tracking system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtsIdentity {
    pub ats: String,
    pub board_slug: String,
    pub job_id: String,
}

impl AtsIdentity {
    fn key(&self) -> String {
        format!("ats:{}:{}:{}", self.ats, self.board_slug, self.job_id)
    }
}

/// Strip a URL down to the part that identifies the job.
///
/// Lowercases the host (but never the path — Ashby and Lever slugs are case-sensitive),
/// drops the query and fragment entirely, removes trailing action segments, and canonicalizes
/// Greenhouse's two hosts onto one.
pub fn canonical_url(url: &str) -> String {
    let without_scheme = url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://");

    // Query and fragment carry tracking parameters that differ per source for the same job.
    let without_query = without_scheme
        .split(['?', '#'])
        .next()
        .unwrap_or(without_scheme);

    let (host, path) = match without_query.split_once('/') {
        Some((host, path)) => (host.to_lowercase(), path),
        None => (without_query.to_lowercase(), ""),
    };

    let host = host.trim_start_matches("www.");
    // Greenhouse serves the same boards from several hostnames, including a regional one.
    // `boards.` and `job-boards.` were in the research doc; **`job-boards.eu.greenhouse.io`
    // was not** — it turned up only when a real collection ran, carrying 5 postings that were
    // silently falling through to the fallback key.
    let host = if host.ends_with(".greenhouse.io")
        && (host.starts_with("boards.") || host.starts_with("job-boards."))
    {
        "job-boards.greenhouse.io"
    } else {
        host
    };

    let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    // Trailing action segments: Lever's `/apply`, Ashby's `/application`, Workday's locale.
    while let Some(last) = segments.last() {
        if matches!(*last, "apply" | "application") {
            segments.pop();
        } else {
            break;
        }
    }
    segments.retain(|s| !is_locale_segment(s));

    if segments.is_empty() {
        host.to_string()
    } else {
        format!("{host}/{}", segments.join("/"))
    }
}

/// `en-US`, `en_GB` and friends, which appear in Workday browse URLs and are not identity.
fn is_locale_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    (bytes.len() == 5 && (bytes[2] == b'-' || bytes[2] == b'_'))
        && segment[..2].chars().all(|c| c.is_ascii_lowercase())
        && segment[3..].chars().all(|c| c.is_ascii_alphabetic())
}

/// Recover `(ats, board_slug, job_id)` from a posting URL, when it is on a known ATS.
///
/// Returns `None` for anything else — a company's own careers page, an aggregator's own
/// detail page — which is what sends the posting to the fallback key.
pub fn ats_identity(url: &str) -> Option<AtsIdentity> {
    // Greenhouse's embed form carries the job id in the query string:
    // `boards.greenhouse.io/embed/job_app?token=7231006`. Everywhere else the query is
    // tracking noise and is stripped, so this has to be read before that happens — which is
    // why it is here rather than inside `canonical_url`.
    //
    // The board slug is simply absent from this shape, so these key on a fixed `embed` slug.
    // That means the same job seen both ways does **not** merge: an under-merge, which § C
    // calls the safer failure and which is strictly better than the fallback key's behaviour
    // of collapsing every same-titled job at the company into one row.
    if let Some(token) = greenhouse_embed_token(url) {
        return Some(AtsIdentity {
            ats: "greenhouse".to_string(),
            board_slug: "embed".to_string(),
            job_id: token,
        });
    }

    let canonical = canonical_url(url);
    let (host, path) = canonical.split_once('/')?;
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    let identity = match host {
        // job-boards.greenhouse.io/{slug}/jobs/{id}
        "job-boards.greenhouse.io" => match segments.as_slice() {
            [slug, "jobs", id, ..] => (slug, id),
            _ => return None,
        },
        // jobs.lever.co/{site}/{uuid}
        "jobs.lever.co" => match segments.as_slice() {
            [site, id, ..] => (site, id),
            _ => return None,
        },
        // jobs.ashbyhq.com/{org}/{uuid}
        "jobs.ashbyhq.com" => match segments.as_slice() {
            [org, id, ..] => (org, id),
            _ => return None,
        },
        // jobs.smartrecruiters.com/{co}/{id}-{slug}
        "jobs.smartrecruiters.com" => match segments.as_slice() {
            [co, rest, ..] => {
                let id = rest.split('-').next().filter(|s| !s.is_empty())?;
                return Some(AtsIdentity {
                    ats: "smartrecruiters".to_string(),
                    board_slug: (*co).to_string(),
                    job_id: id.to_string(),
                });
            }
            _ => return None,
        },
        _ => return None,
    };

    let ats = match host {
        "job-boards.greenhouse.io" => "greenhouse",
        "jobs.lever.co" => "lever",
        "jobs.ashbyhq.com" => "ashby",
        _ => return None,
    };

    Some(AtsIdentity {
        ats: ats.to_string(),
        board_slug: identity.0.to_string(),
        job_id: identity.1.to_string(),
    })
}

/// The job id from a Greenhouse embed URL, if this is one.
fn greenhouse_embed_token(url: &str) -> Option<String> {
    let lower = url.to_lowercase();
    if !lower.contains("greenhouse.io/embed/job_app") {
        return None;
    }
    let query = url.split_once('?')?.1;
    let token = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.eq_ignore_ascii_case("token"))
        .map(|(_, value)| value)?;
    (!token.is_empty() && token.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| token.to_string())
}

/// Season words and their adjacent noise, stripped from a title before comparing.
const TITLE_NOISE: &[&str] = &[
    "summer", "fall", "autumn", "winter", "spring", "intern", "interns", "internship",
    "internships", "co-op", "coop", "program", "programme",
];

/// Reduce a title to the part that identifies the role.
///
/// § C: "SWE Intern" and "Software Engineer Intern, Summer 2026" share almost no surface form.
/// Strip the season, the requisition id, and any parenthetical before comparing. It warns this
/// key will **under-merge rather than over-merge**, which is the safer failure and is why no
/// cleverness is attempted here.
pub fn title_key(title: &str) -> String {
    let mut cleaned = String::with_capacity(title.len());
    let mut depth = 0usize;

    // Drop parentheticals and bracketed requisition ids wholesale.
    for ch in title.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => cleaned.push(ch),
            _ => {}
        }
    }

    cleaned
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| word.to_lowercase())
        // A bare four-digit year is a term marker, not part of the role.
        .filter(|word| !(word.len() == 4 && word.chars().all(|c| c.is_ascii_digit())))
        .filter(|word| !TITLE_NOISE.contains(&word.as_str()))
        .collect::<Vec<_>>()
        .join("-")
}

/// The key two postings must share to be treated as the same job.
///
/// ATS triple when the URL yields one, `company|title` otherwise. The term is included in the
/// fallback so the same role in consecutive years stays distinct — a Summer 2027 posting is
/// not a repost of Summer 2026.
pub fn dedup_key(posting: &NormalizedPosting) -> String {
    if let Some(identity) = ats_identity(&posting.url) {
        return identity.key();
    }

    let season = posting
        .term_season
        .map(season_str)
        .unwrap_or("any");
    let year = posting
        .term_year
        .map(|y| y.to_string())
        .unwrap_or_else(|| "any".to_string());

    format!(
        "co:{}|{}|{season}-{year}",
        posting.company_key,
        title_key(&posting.title)
    )
}

fn season_str(season: Season) -> &'static str {
    match season {
        Season::Summer => "summer",
        Season::Fall => "fall",
        Season::Winter => "winter",
        Season::Spring => "spring",
    }
}

/// The seam for fuzzy company/title matching.
///
/// **Deliberately unimplemented.** Fuzzy matching is the NLP learning area and belongs to the
/// repo owner; see this module's header and `docs/INTERNSHIP_SCRAPING.md` § C, which concludes
/// `src/nlp.rs` cannot be reused as-is (wrong output shape, and its substring band would fire
/// on "Engineer" across most of the corpus) and that generalizing it would mean *editing* a
/// `[learn]` file.
///
/// Until something implements this, [`dedup_key`] is exact-only and under-merges: the same job
/// listed as `KLA` on one source and `KLA Corporation` on another becomes two rows. That is
/// visible and recoverable. Over-merging — collapsing two genuinely different jobs — is not.
pub trait FuzzyMatcher: Send + Sync {
    /// Whether these two postings are the same job, given the exact keys already disagreed.
    fn same_posting(&self, left: &NormalizedPosting, right: &NormalizedPosting) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internships::models::{ClassYearRange, Location};

    fn posting(company_key: &str, title: &str, url: &str) -> NormalizedPosting {
        NormalizedPosting {
            source: "test".to_string(),
            external_id: "x".to_string(),
            url: url.to_string(),
            company_name: company_key.to_string(),
            company_key: company_key.to_string(),
            title: title.to_string(),
            term_season: Some(Season::Summer),
            term_year: Some(2027),
            location: Location::default(),
            pay: None,
            pay_raw: None,
            class_years: ClassYearRange::default(),
            posted_at: None,
            deadline: None,
            raw_json: "{}".to_string(),
        }
    }

    // ---- URL normalization: each case was counted in a real corpus ----

    #[test]
    fn greenhouses_two_hosts_are_the_same_board() {
        // 1,244 records use one host and 179 the other. Without this they never join.
        assert_eq!(
            canonical_url("https://boards.greenhouse.io/acme/jobs/123"),
            canonical_url("https://job-boards.greenhouse.io/acme/jobs/123")
        );
    }

    #[test]
    fn tracking_query_strings_are_stripped() {
        // 544 records carry ?gh_jid=, 574 ?mobile=, 339 ?ats=.
        let bare = canonical_url("https://job-boards.greenhouse.io/acme/jobs/123");
        for noisy in [
            "https://job-boards.greenhouse.io/acme/jobs/123?gh_jid=456",
            "https://job-boards.greenhouse.io/acme/jobs/123?mobile=true",
            "https://job-boards.greenhouse.io/acme/jobs/123?ats=simplify&nl=1",
            "https://job-boards.greenhouse.io/acme/jobs/123#apply-now",
        ] {
            assert_eq!(canonical_url(noisy), bare, "{noisy} did not normalize");
        }
    }

    #[test]
    fn trailing_action_segments_are_removed() {
        // Lever appends /apply on 579 records; Ashby appends /application.
        assert_eq!(
            canonical_url("https://jobs.lever.co/acme/uuid-1/apply"),
            canonical_url("https://jobs.lever.co/acme/uuid-1")
        );
        assert_eq!(
            canonical_url("https://jobs.ashbyhq.com/acme/uuid-1/application"),
            canonical_url("https://jobs.ashbyhq.com/acme/uuid-1")
        );
    }

    #[test]
    fn path_case_is_preserved_because_slugs_are_case_sensitive() {
        // Ashby org slugs like `Hippocratic AI` are case-sensitive; lowercasing the path
        // would point at a board that does not exist.
        assert!(canonical_url("https://jobs.ashbyhq.com/Etched/uuid-1").contains("Etched"));
    }

    // ---- the ATS triple ----

    #[test]
    fn the_same_job_reached_by_two_different_urls_gets_one_key() {
        // This is the entire point: the aggregator links at the ATS, so both sides recover
        // the same identity even though the URLs differ.
        let from_simplify = posting(
            "acme",
            "Software Engineer Intern",
            "https://job-boards.greenhouse.io/acme/jobs/123?utm_source=simplify",
        );
        let from_greenhouse = posting(
            "acme",
            "Software Engineer Intern, Summer 2027",
            "https://boards.greenhouse.io/acme/jobs/123",
        );
        assert_eq!(dedup_key(&from_simplify), dedup_key(&from_greenhouse));
    }

    #[test]
    fn different_jobs_on_one_board_stay_distinct() {
        let a = posting("acme", "SWE Intern", "https://jobs.lever.co/acme/uuid-1");
        let b = posting("acme", "SWE Intern", "https://jobs.lever.co/acme/uuid-2");
        assert_ne!(dedup_key(&a), dedup_key(&b));
    }

    #[test]
    fn the_same_id_on_different_boards_stays_distinct() {
        let a = posting("acme", "SWE Intern", "https://jobs.lever.co/acme/uuid-1");
        let b = posting("other", "SWE Intern", "https://jobs.lever.co/other/uuid-1");
        assert_ne!(dedup_key(&a), dedup_key(&b));
    }

    #[test]
    fn a_non_ats_url_falls_back_rather_than_producing_a_bogus_triple() {
        assert_eq!(ats_identity("https://acme.com/careers/swe-intern"), None);
        let fallback = dedup_key(&posting(
            "acme",
            "SWE Intern",
            "https://acme.com/careers/swe-intern",
        ));
        assert!(fallback.starts_with("co:"), "got {fallback}");
    }

    #[test]
    fn greenhouses_regional_host_is_the_same_board() {
        // Found by a real collection, not by reading the docs: `job-boards.eu.greenhouse.io`
        // is a third Greenhouse hostname, and five postings were falling through to the
        // fallback key because of it.
        let identity = ats_identity("https://job-boards.eu.greenhouse.io/axiomaticai/jobs/4848121101")
            .expect("eu host should be recognized");
        assert_eq!(identity.ats, "greenhouse");
        assert_eq!(identity.board_slug, "axiomaticai");
        assert_eq!(identity.job_id, "4848121101");
    }

    #[test]
    fn the_greenhouse_embed_form_keys_on_its_token() {
        // Real URL from the same run. The id is in the query string, which every other shape
        // treats as strippable tracking noise.
        let identity = ats_identity("https://boards.greenhouse.io/embed/job_app?token=7231006")
            .expect("embed form should be recognized");
        assert_eq!(identity.ats, "greenhouse");
        assert_eq!(identity.job_id, "7231006");
    }

    #[test]
    fn two_embedded_jobs_at_one_company_stay_distinct() {
        // The point of the fix: under the fallback key these two shared a company and title
        // and collapsed into one posting. Over-merging loses a real job; under-merging shows
        // a duplicate. This asserts we now do the second.
        let a = posting("acme", "SWE Intern", "https://boards.greenhouse.io/embed/job_app?token=7231006");
        let b = posting("acme", "SWE Intern", "https://boards.greenhouse.io/embed/job_app?token=7905463");
        assert_ne!(dedup_key(&a), dedup_key(&b));
    }

    // ---- the fallback key ----

    #[test]
    fn season_and_year_noise_is_stripped_from_titles() {
        // § C's example: these share almost no surface form until the noise comes off.
        assert_eq!(
            title_key("Software Engineer Intern, Summer 2026"),
            title_key("Software Engineer Internship")
        );
    }

    #[test]
    fn parentheticals_and_requisition_ids_are_dropped() {
        assert_eq!(
            title_key("Software Engineer Intern (Summer 2027) [REQ-4821]"),
            title_key("Software Engineer Intern")
        );
    }

    #[test]
    fn the_fallback_key_still_separates_consecutive_years() {
        // A Summer 2027 posting is not a repost of Summer 2026, even at the same company.
        let mut next_year = posting("acme", "SWE Intern", "https://acme.com/careers/1");
        next_year.term_year = Some(2026);
        let this_year = posting("acme", "SWE Intern", "https://acme.com/careers/1");
        assert_ne!(dedup_key(&next_year), dedup_key(&this_year));
    }

    #[test]
    fn the_fallback_key_under_merges_rather_than_over_merging() {
        // `KLA` vs `KLA Corporation` normalize differently, so they stay two rows. This is
        // the documented, accepted failure of exact-only dedup — pinned so that if a fuzzy
        // matcher ever lands, this test is the one that has to be consciously changed.
        let short = posting("kla", "SWE Intern", "https://kla.com/careers/1");
        let long = posting("kla-corporation", "SWE Intern", "https://kla.com/careers/1");
        assert_ne!(
            dedup_key(&short),
            dedup_key(&long),
            "exact dedup is expected to under-merge company-name variants"
        );
    }

    #[test]
    fn titles_that_merely_contain_intern_are_not_specially_handled_here() {
        // The § C substring trap ("Manager, International") is QC's job, not dedup's. All
        // this asserts is that dedup does not silently collapse two unrelated roles.
        let a = posting("acme", "Director, US International Tax", "https://acme.com/1");
        let b = posting("acme", "Internal Communications Manager", "https://acme.com/2");
        assert_ne!(dedup_key(&a), dedup_key(&b));
    }
}
