//! Deciding when two postings from different sources are the same job.
//!
//! # The key, and why it is shaped this way
//!
//! Straight from `docs/INTERNSHIP_SCRAPING.md` § C, which measured this rather than guessing:
//!
//! 1. **Primary — the canonical ATS triple `(ats, board_slug, job_id)`**, recovered from the
//!    posting URL. It is an exact key needing no fuzzy matching, and it works because the
//!    GitHub aggregator lists link *directly at the ATS* — so both sides of the join carry the
//!    same identity. The source/host census projected 73% coverage (58% of active listings),
//!    but the Phase 7 run measured ~35% before the three parsers added below; Task 12b owns
//!    the independent post-change measurement.
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
//! The 2026-09-02 corpus added three more concrete URL families: Workable's
//! `/{account}/j/{id}[/apply]`, Rippling's `/[locale/]{company}/jobs/{uuid}`, and Workday's
//! two host/path layouts. Their parsers preserve every identity component the URL exposes;
//! where Workday's route suffix is ambiguous, they deliberately under-merge rather than
//! manufacture a stronger identity than the URL proves.
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
        return host.to_string();
    }

    let joined = segments.join("/");
    // Audit finding F7. The path is case-**sensitive** by default, because Ashby and Lever
    // slugs genuinely are — `jobs.ashbyhq.com/Etched/...` is not the same board as `etched`,
    // and folding it would point at nothing.
    //
    // Greenhouse is different, and this was measured rather than assumed: `boards-api
    // .greenhouse.io/v1/boards/anthropic/jobs` and `.../Anthropic/jobs` both answer 200 with
    // the same 571 jobs. So for that host alone, two URLs differing only in case are the same
    // board, and keying on the raw case splits one job into two postings.
    if HOSTS_WITH_CASE_INSENSITIVE_PATHS.contains(&host) {
        return format!("{host}/{}", joined.to_lowercase());
    }
    format!("{host}/{joined}")
}

/// Hosts whose path may be case-folded, each verified against the live API rather than assumed.
///
/// **Adding a host here is a claim that must be tested**, because folding a case-sensitive
/// slug merges two different boards — and over-merging destroys a posting, where under-merging
/// only shows a duplicate. `docs/INTERNSHIP_SCRAPING.md` § C is explicit that under-merging is
/// the safer failure.
const HOSTS_WITH_CASE_INSENSITIVE_PATHS: &[&str] = &["job-boards.greenhouse.io"];

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
    let (host, path) = match canonical.split_once('/') {
        Some(parts) => parts,
        // No path at all, but a `?gh_jid=` may still identify the job.
        None => return greenhouse_job_id_param(url).map(job_id_identity),
    };
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    let identity = match host {
        // job-boards.greenhouse.io/{slug}/jobs/{id}
        "job-boards.greenhouse.io" => match segments.as_slice() {
            [slug, "jobs", id, ..] => (slug, id),
            _ => return greenhouse_job_id_param(url).map(job_id_identity),
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
        // apply.workable.com/{account}/j/{id}[/apply]
        //
        // `canonical_url` has already removed the optional trailing `/apply`. Keep the job
        // id's case: the real corpus uses uppercase hexadecimal ids, but Workable has not
        // documented that case folding is safe.
        "apply.workable.com" => match segments.as_slice() {
            [account, "j", id] => {
                return Some(AtsIdentity {
                    ats: "workable".to_string(),
                    board_slug: (*account).to_string(),
                    job_id: (*id).to_string(),
                });
            }
            _ => return None,
        },
        // ats.rippling.com/[locale/]{company}/jobs/{uuid}
        //
        // Locale removal is centralized in `canonical_url`, so both the localized and bare
        // forms arrive here as exactly three segments.
        "ats.rippling.com" => match segments.as_slice() {
            [company, "jobs", id] => {
                return Some(AtsIdentity {
                    ats: "rippling".to_string(),
                    board_slug: (*company).to_string(),
                    job_id: (*id).to_string(),
                });
            }
            _ => return None,
        },
        _ if host.ends_with(".myworkdayjobs.com") || host.ends_with(".myworkdaysite.com") => {
            return workday_identity(host, &segments);
        }
        // Not an ATS host. A Greenhouse job id in the query string still identifies the
        // job — see `greenhouse_job_id_param`.
        _ => return greenhouse_job_id_param(url).map(job_id_identity),
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

/// Recover a Workday identity from either public career-site layout.
///
/// The real corpus contains both:
///
/// - `{tenant}.wd{N}.myworkdayjobs.com/[locale/]{site}/job/.../{slug}_{id}`
/// - `wd{N}.myworkdaysite.com/[locale/]recruiting/{tenant}/{site}/job/.../{slug}_{id}`
///
/// The board slug retains the shard (`tenant.wdN`). A tenant can move shards, so this may
/// under-merge two URLs for one requisition; dropping the shard would assert an equivalence
/// the URL corpus does not prove, and over-merging loses a posting. For the bare-shard
/// `myworkdaysite.com` form, `sources::simplify::board_of` still derives the wrong tenant from
/// the host; that board-discovery gap is separate from the identity parsed here.
fn workday_identity(host: &str, segments: &[&str]) -> Option<AtsIdentity> {
    let labels: Vec<&str> = host.split('.').collect();

    let (tenant, shard, external_path) = match labels.as_slice() {
        // `{tenant}.wd{N}.myworkdayjobs.com/{site}/job/.../{slug}_{id}` and the less common
        // tenant-hosted `myworkdaysite.com` variant documented by board discovery.
        [tenant, shard, domain, "com"]
            if (*domain == "myworkdayjobs" || *domain == "myworkdaysite")
                && is_workday_shard(shard) =>
        {
            match segments {
                [_site, "job", external_path @ ..] if !external_path.is_empty() => {
                    (*tenant, *shard, *external_path.last()?)
                }
                _ => return None,
            }
        }
        // The real `myworkdaysite.com` corpus puts the tenant in the recruiting path, not
        // the host: `wd3.myworkdaysite.com/recruiting/magna/Magna/job/...`.
        [shard, "myworkdaysite", "com"] if is_workday_shard(shard) => match segments {
            ["recruiting", tenant, _site, "job", external_path @ ..]
                if !external_path.is_empty() =>
            {
                (*tenant, *shard, *external_path.last()?)
            }
            _ => return None,
        },
        _ => return None,
    };

    // Workday separates its display slug from its external-path identity with the first
    // underscore. The identity itself can contain underscores (`R_12318`,
    // `REQ_0000080335-1`), so splitting at the last underscore would silently discard part
    // of a real requisition id. Preserve a trailing `-1`/`-2`: same-title URLs in the corpus
    // suggest it is sometimes route disambiguation, but URL-only parsing cannot distinguish
    // that from a requisition id that genuinely ends the same way. The conservative result is
    // an under-merge, never two distinct jobs collapsed together.
    let (_, job_id) = external_path.split_once('_')?;
    if job_id.is_empty() {
        return None;
    }

    Some(AtsIdentity {
        ats: "workday".to_string(),
        board_slug: format!("{tenant}.{shard}"),
        job_id: job_id.to_string(),
    })
}

fn is_workday_shard(label: &str) -> bool {
    label
        .strip_prefix("wd")
        .is_some_and(|number| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
}

/// Identity for a Greenhouse job known only by its id, with no board slug available.
///
/// The pseudo-slug keeps this distinct from the path form `ats:greenhouse:{slug}:{id}`. That
/// is an **under-merge on purpose**: treating the bare id as globally unique would merge two
/// boards' jobs if Greenhouse ever reused an id across them, and § C is explicit that
/// under-merging is the safer failure.
fn job_id_identity(job_id: String) -> AtsIdentity {
    AtsIdentity {
        ats: "greenhouse".to_string(),
        board_slug: "gh_jid".to_string(),
        job_id,
    }
}

/// A Greenhouse job id carried in a `gh_jid` query parameter.
///
/// Companies that host their own careers page but run hiring through Greenhouse link like
/// `https://www.oldmissioncapital.com/careers/?gh_jid=7796180003`. `canonical_url` strips the
/// query as tracking noise, which leaves `…/careers/` — an **index page shared by every one of
/// that company's jobs**. Two consequences, both observed in live data:
///
/// - The same job listed by two sources with differently-worded titles split into two
///   postings, because the fallback key had nothing but company and title to work with.
/// - Keying on the bare canonical URL instead — the obvious repair — would have been far
///   worse: `zipline.com/open-roles?gh_jid=7974897003` and `?gh_jid=7929236003` are two
///   *different* jobs at one path, and merging them destroys a posting.
///
/// Reading the id out of the query fixes the first without risking the second. Same shape as
/// the `embed/job_app?token=` case: a parameter that is identity, not tracking.
/// The Greenhouse job id a URL carries, whichever shape it is in and **whatever the host**.
///
/// # Why this is separate from [`ats_identity`] rather than folded into it
///
/// A company's own careers page can host a Greenhouse job:
/// `www.jumptrading.com/hr/job?gh_jid=8007788` is the same job as
/// `job-boards.greenhouse.io/jumptrading/jobs/8007788`. Teaching `ats_identity` to see that
/// would be the obvious move and would be a mistake, because its output **is the dedup key**.
/// Changing what it returns for existing URLs re-keys every Greenhouse posting in the corpus:
/// the stored rows keep their old `dedup_key`, the next run computes new ones, and every one
/// of them inserts as a new posting. A better identity, bought by duplicating the corpus.
///
/// So this exists for *lookup* — answering "is the page I am on one of the postings I already
/// collected" — where a wrong answer costs a button that does not appear, not a table that
/// doubles. If the merge key should ever learn this, it needs a migration that recomputes
/// `dedup_key` for every row, which is its own deliberate change.
///
/// Greenhouse job ids are unique across the platform, so the board slug is not needed to make
/// this a safe comparison.
pub fn greenhouse_job_id(url: &str) -> Option<String> {
    if let Some(token) = greenhouse_embed_token(url) {
        return Some(token);
    }
    if let Some(id) = greenhouse_job_id_param(url) {
        return Some(id);
    }
    // The path form, via the parser that already knows every Greenhouse hostname.
    match ats_identity(url) {
        Some(identity) if identity.ats == "greenhouse" => Some(identity.job_id),
        _ => None,
    }
}

fn greenhouse_job_id_param(url: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    let id = query
        .split(['&', '#'])
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.eq_ignore_ascii_case("gh_jid"))
        .map(|(_, value)| value)?;
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| id.to_string())
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

    #[test]
    fn a_greenhouse_job_id_is_recovered_from_every_shape_it_appears_in() {
        // The three ways the same job can be linked, all of which must reduce to one id.
        assert_eq!(
            greenhouse_job_id("https://job-boards.greenhouse.io/jumptrading/jobs/8007788"),
            Some("8007788".to_string())
        );
        assert_eq!(
            greenhouse_job_id("https://www.jumptrading.com/hr/job?gh_jid=8007788"),
            Some("8007788".to_string()),
            "a company careers page hosting a Greenhouse job"
        );
        assert_eq!(
            greenhouse_job_id("https://boards.greenhouse.io/embed/job_app?token=7231006"),
            Some("7231006".to_string())
        );
    }

    #[test]
    fn a_company_page_and_its_ats_listing_resolve_to_the_same_job() {
        // The point of the whole function: these two URLs are one job, and nothing else in
        // this module can tell.
        let company = "https://www.jumptrading.com/hr/job?gh_jid=8007788";
        let ats = "https://job-boards.greenhouse.io/jumptrading/jobs/8007788";
        assert_eq!(greenhouse_job_id(company), greenhouse_job_id(ats));
        assert_ne!(
            ats_identity(company),
            ats_identity(ats),
            "and ats_identity still does NOT equate them — the dedup key is unchanged"
        );
    }

    #[test]
    fn nothing_greenhouse_shaped_yields_no_job_id() {
        for url in [
            "https://jobs.lever.co/acme/2f8a1e00-0000-0000-0000-000000000000",
            "https://careers.example.com/jobs/123",
            "https://www.example.com/apply?utm_source=x",
            "https://www.example.com/apply?gh_jid=",
        ] {
            assert_eq!(greenhouse_job_id(url), None, "{url} should yield nothing");
        }
    }
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
    fn greenhouse_board_slug_casing_does_not_split_a_posting() {
        // Audit finding F7. Verified against the live API before folding anything:
        // `/v1/boards/anthropic/jobs` and `/v1/boards/Anthropic/jobs` both return 200 with the
        // same 571 jobs, so these are one board and must be one posting.
        let lower = posting("acme", "Software Engineer Intern",
            "https://job-boards.greenhouse.io/acme/jobs/1");
        let upper = posting("acme", "Software Engineer Intern",
            "https://job-boards.greenhouse.io/ACME/jobs/1");
        assert_eq!(dedup_key(&lower), dedup_key(&upper));
        assert_eq!(
            ats_identity("https://job-boards.greenhouse.io/ACME/jobs/1").unwrap().board_slug,
            "acme"
        );
    }

    #[test]
    fn case_folding_does_not_leak_to_case_sensitive_hosts() {
        // The guard on the fix. Ashby and Lever slugs really are case-sensitive, so folding
        // them would merge two different boards — and over-merging destroys a posting, where
        // under-merging only shows a duplicate.
        for (lower_url, upper_url) in [
            ("https://jobs.ashbyhq.com/etched/uuid-1", "https://jobs.ashbyhq.com/Etched/uuid-1"),
            ("https://jobs.lever.co/acme/uuid-1", "https://jobs.lever.co/Acme/uuid-1"),
        ] {
            assert_ne!(
                canonical_url(lower_url),
                canonical_url(upper_url),
                "{upper_url} must not be folded onto {lower_url}"
            );
        }
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
    fn workable_uses_the_account_and_job_token_from_real_urls() {
        // The same Pony.ai job appeared in the real corpus both with and without Workable's
        // trailing action segment.
        let bare = ats_identity("https://apply.workable.com/pony-dot-ai/j/BA5FFDBC71/")
            .expect("bare Workable URL should be recognized");
        let apply = ats_identity("https://apply.workable.com/pony-dot-ai/j/BA5FFDBC71/apply")
            .expect("Workable apply URL should be recognized");
        assert_eq!(bare, apply);
        assert_eq!(bare.ats, "workable");
        assert_eq!(bare.board_slug, "pony-dot-ai");
        assert_eq!(bare.job_id, "BA5FFDBC71");

        let other = ats_identity("https://apply.workable.com/altom-transport/j/9FC654F05E/apply")
            .expect("second real Workable URL should be recognized");
        assert_ne!(bare, other);
    }

    #[test]
    fn rippling_removes_locale_but_keeps_company_and_uuid() {
        // Both forms for this exact SpreeAI job occur in the real corpus.
        let localized = ats_identity(
            "https://ats.rippling.com/en-GB/spreeai/jobs/c52472cb-2671-45d7-b666-17196dc3df25",
        )
        .expect("localized Rippling URL should be recognized");
        let bare = ats_identity(
            "https://ats.rippling.com/spreeai/jobs/c52472cb-2671-45d7-b666-17196dc3df25",
        )
        .expect("bare Rippling URL should be recognized");
        assert_eq!(localized, bare);
        assert_eq!(bare.ats, "rippling");
        assert_eq!(bare.board_slug, "spreeai");
        assert_eq!(bare.job_id, "c52472cb-2671-45d7-b666-17196dc3df25");

        let other = ats_identity(
            "https://ats.rippling.com/spreeai/jobs/d34aed29-7a11-4e37-b5bc-e9317f82f0b1",
        )
        .expect("second real Rippling URL should be recognized");
        assert_ne!(bare, other);
    }

    #[test]
    fn workday_recognizes_both_real_host_and_path_layouts() {
        let hosted = ats_identity("https://oxy.wd5.myworkdayjobs.com/Corporate/job/_JR100413")
            .expect("tenant-hosted Workday URL should be recognized");
        assert_eq!(hosted.ats, "workday");
        assert_eq!(hosted.board_slug, "oxy.wd5");
        assert_eq!(hosted.job_id, "JR100413");

        let site = ats_identity(
            "https://wd3.myworkdaysite.com/recruiting/magna/Magna/job/Grand-Rapids-Michigan-US/Product-Engineering-Intern_R00243272",
        )
        .expect("recruiting-path Workday URL should be recognized");
        assert_eq!(site.ats, "workday");
        assert_eq!(site.board_slug, "magna.wd3");
        assert_eq!(site.job_id, "R00243272");

        let localized_site = ats_identity(
            "https://wd5.myworkdaysite.com/en-US/recruiting/devonenergy/Careers/job/Oklahoma-City-OK/Technology-Summer-Intern-2027_R26264-1",
        )
        .expect("localized recruiting-path Workday URL should be recognized");
        assert_eq!(localized_site.board_slug, "devonenergy.wd5");
        assert_eq!(localized_site.job_id, "R26264-1");
    }

    #[test]
    fn workday_locale_does_not_split_a_real_duplicate() {
        let bare = ats_identity(
            "https://coreandmain.wd1.myworkdayjobs.com/coreandmain/job/Saint-Louis-MO-63146/Intern---Data-Engineering----Corp_45804",
        )
        .unwrap();
        let localized = ats_identity(
            "https://coreandmain.wd1.myworkdayjobs.com/en-US/coreandmain/job/Saint-Louis-MO-63146/Intern---Data-Engineering----Corp_45804",
        )
        .unwrap();
        assert_eq!(bare, localized);
    }

    #[test]
    fn workday_identity_keeps_internal_underscores_and_route_suffixes() {
        // These are real URL shapes. Workday's visible page identifies the former as
        // `R_12318`; splitting at the final underscore would reduce three distinct prefixes
        // (`R_`, `REQ_`, and a bare number) to the same numeric tail.
        let embedded = ats_identity(
            "https://aoins.wd5.myworkdayjobs.com/AutoOwners/job/Lansing-MI/Data-Engineering-Internship---Summer-2026_R_12318",
        )
        .expect("Workday id containing an underscore should be recognized");
        assert_eq!(embedded.job_id, "R_12318");

        let req = ats_identity(
            "https://psu.wd1.myworkdayjobs.com/PSU_Staff/job/Penn-State-University-Park/Research-Engineering-Interns_REQ_0000080335-1",
        )
        .expect("Workday REQ id should be recognized");
        assert_eq!(req.job_id, "REQ_0000080335-1");

        // `-1` is sometimes Workday route disambiguation, but the URL alone cannot prove it
        // is not part of the requisition id. Keep the two keys distinct: this can show a
        // duplicate, while stripping it could destroy a genuinely separate job.
        let plain = ats_identity(
            "https://medtronic.wd1.myworkdayjobs.com/redeploymentmedtroniccareers/job/Fridley-Minnesota-United-States-of-America/Software-Engineering-Intern---Summer-2027_R73630",
        )
        .unwrap();
        let suffixed = ats_identity(
            "https://medtronic.wd1.myworkdayjobs.com/redeploymentmedtroniccareers/job/Fridley-Minnesota-United-States-of-America/Software-Engineering-Intern---Summer-2027_R73630-1",
        )
        .unwrap();
        assert_ne!(plain, suffixed);
    }

    #[test]
    fn ats_like_but_non_job_paths_do_not_get_an_identity() {
        for url in [
            "https://apply.workable.com/pony-dot-ai/jobs/BA5FFDBC71",
            "https://ats.rippling.com/spreeai/job/c52472cb-2671-45d7-b666-17196dc3df25",
            "https://oxy.wd5.myworkdayjobs.com/Corporate",
            "https://oxy.wd5.myworkdayjobs.com/Corporate/job/Software-Engineer",
            "https://wd3.myworkdaysite.com/recruiting/magna/Magna",
            "https://notworkdayjobs.com/Corporate/job/Role_JR100413",
        ] {
            assert_eq!(ats_identity(url), None, "{url} must not create an ATS key");
        }
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

    #[test]
    fn a_company_careers_page_keys_on_its_greenhouse_job_id() {
        // Live example. The query is stripped as tracking noise, leaving `…/careers/` — an
        // index page shared by every one of that company's jobs — so two sources wording the
        // title differently produced two postings for one job.
        let terse = posting("old mission", "SWE Intern",
            "https://www.oldmissioncapital.com/careers/?gh_jid=7796180003");
        let verbose = posting("old mission", "Software Engineer Intern, Summer 2027",
            "https://www.oldmissioncapital.com/careers/?gh_jid=7796180003");
        assert_eq!(dedup_key(&terse), dedup_key(&verbose));
        assert!(dedup_key(&terse).starts_with("ats:greenhouse:gh_jid:"));
    }

    #[test]
    fn two_jobs_sharing_one_careers_page_stay_distinct() {
        // The guard, and the reason this is not fixed by keying on the canonical URL. Both of
        // these live at `zipline.com/open-roles`; merging them would destroy a posting, which
        // is the failure direction § C warns is unrecoverable.
        let a = posting("zipline", "Software Engineer Intern",
            "https://www.zipline.com/open-roles?gh_jid=7974897003");
        let b = posting("zipline", "Software Engineer Intern",
            "https://www.zipline.com/open-roles?gh_jid=7929236003");
        assert_ne!(dedup_key(&a), dedup_key(&b));
    }

    #[test]
    fn a_path_based_ats_identity_still_wins_over_the_query_parameter() {
        // Ordering: the board slug is more specific than a bare id, so a real ATS URL must not
        // be demoted to the `gh_jid` pseudo-slug just because the query also carries the id.
        let identity = ats_identity(
            "https://job-boards.greenhouse.io/acme/jobs/123?gh_jid=123",
        )
        .expect("the path form should be recognized");
        assert_eq!(identity.board_slug, "acme");
    }

    #[test]
    fn a_malformed_gh_jid_is_ignored_rather_than_keyed_on() {
        for url in [
            "https://acme.com/careers/?gh_jid=",
            "https://acme.com/careers/?gh_jid=../../etc",
            "https://acme.com/careers/?other=1",
            "https://acme.com/careers/",
        ] {
            assert!(
                ats_identity(url).is_none(),
                "{url} should not produce an ATS identity"
            );
        }
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

    /// Task 12b's reproducible, read-only blast-radius report.
    ///
    /// This is ignored because the fixture is deliberately a copy of the real database, not
    /// a committed test database. `docs/INTERNSHIP_SCRAPING.md` § C gives the exact copy and
    /// invocation. The author of the key does not run the measurement.
    #[tokio::test]
    #[ignore = "12b runs this against /tmp/fridge-12b-copy.db"]
    async fn report_new_ats_key_merge_and_split_candidates() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::collections::{BTreeMap, BTreeSet};
        use std::path::PathBuf;

        #[derive(sqlx::FromRow)]
        struct Sighting {
            posting_id: String,
            old_key: String,
            source: String,
            url: String,
        }

        let fixture = PathBuf::from(
            std::env::var("REKEY_FIXTURE_DB")
                .expect("set REKEY_FIXTURE_DB to the documented database copy"),
        );
        assert_ne!(
            fixture.file_name().and_then(|name| name.to_str()),
            Some("fridge.db"),
            "refusing the live database path; make the documented copy first"
        );

        let options = SqliteConnectOptions::new()
            .filename(&fixture)
            .read_only(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("open the fixture read-only");
        let sightings = sqlx::query_as::<_, Sighting>(
            r#"
            SELECT
                p.id AS posting_id,
                p.dedup_key AS old_key,
                s.source,
                s.url
            FROM posting_sightings AS s
            JOIN internship_postings AS p ON p.id = s.posting_id
            ORDER BY p.id, s.source, s.external_id
            "#,
        )
        .fetch_all(&pool)
        .await
        .expect("read posting sightings");

        let mut identities: BTreeMap<String, Vec<&Sighting>> = BTreeMap::new();
        let mut identities_by_posting: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
        for sighting in &sightings {
            match ats_identity(&sighting.url) {
                Some(identity)
                    if matches!(identity.ats.as_str(), "workday" | "workable" | "rippling") =>
                {
                    let key = identity.key();
                    identities.entry(key.clone()).or_default().push(sighting);
                    identities_by_posting
                        .entry(&sighting.posting_id)
                        .or_default()
                        .insert(key);
                }
                _ => {
                    // A sighting outside the three new parsers keeps the key its stored row
                    // had before 12a. Including that unchanged side is essential: a row with
                    // one Workday sighting and one company-careers-page sighting will split
                    // after re-keying even though only one of its URLs gained an ATS identity.
                    identities_by_posting
                        .entry(&sighting.posting_id)
                        .or_default()
                        .insert(sighting.old_key.clone());
                }
            }
        }

        let merge_groups: Vec<_> = identities
            .iter()
            .filter(|(_, group)| {
                group
                    .iter()
                    .map(|row| row.posting_id.as_str())
                    .collect::<BTreeSet<_>>()
                    .len()
                    > 1
            })
            .collect();
        let split_groups: Vec<_> = identities_by_posting
            .iter()
            .filter(|(_, keys)| keys.len() > 1)
            .collect();

        println!("new-ATS identity keys: {}", identities.len());
        println!(
            "merge candidates (one new key, multiple stored rows): {}",
            merge_groups.len()
        );
        for (key, group) in merge_groups {
            println!("MERGE {key}");
            for row in group {
                println!(
                    "  posting={} old_key={} source={} url={}",
                    row.posting_id, row.old_key, row.source, row.url
                );
            }
        }
        println!(
            "split candidates (one stored row, multiple new keys): {}",
            split_groups.len()
        );
        for (posting_id, keys) in split_groups {
            println!("SPLIT posting={posting_id} keys={keys:?}");
        }
    }
}
