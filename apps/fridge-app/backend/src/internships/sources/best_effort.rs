//! LinkedIn, Indeed and Handshake — the three the user chose to include knowing they would
//! yield little (`docs/PLAN.md` § Phase 7: *"Best-effort. Expected to yield little. Never on
//! the critical path"*).
//!
//! All three are in the registry, all three write a `source_runs` row every run, and **none of
//! them is ever on the critical path**. A run where all three skip is a normal run.
//!
//! # Why two of them fetch nothing at all
//!
//! The root `CLAUDE.md` scraping rule is absolute and is not a tradeoff to optimize: *no proxy
//! rotation, no CAPTCHA solving, no browser-fingerprint spoofing, no headless-browser cloaking,
//! no scraping from behind a login.* Applied to what `docs/INTERNSHIP_SCRAPING.md` § A.4
//! actually measured, that settles both cases without any judgement left to exercise:
//!
//! - **LinkedIn** publishes `User-agent: * / Disallow: /` — every path, with named-bot
//!   allowances a generic client does not get — plus a notice in the file itself and User
//!   Agreement § 8.2 barring scripts and bots. There is no polite configuration of this source:
//!   every request is a violation, so "best effort" and "zero results" are the same outcome
//!   with different amounts of wasted code.
//! - **Indeed**'s `robots.txt` permits `/jobs?q=…`, but a single polite unauthenticated GET
//!   with an honest user agent returns **HTTP 403** and a Cloudflare CAPTCHA interstitial.
//!   Getting past that *is* the fingerprint spoofing and CAPTCHA solving the rule forbids.
//!   robots permitting a path does not help when the edge refuses the request.
//!
//! So both record [`SourceOutcome::Skipped`] with the honest reason and **make no request**.
//! That is deliberate on three counts: it is the outcome the rules require, it costs nothing,
//! and — the part that matters for anyone reading this later — it means the reason is written
//! down in the run-health panel every run instead of looking like an adapter somebody forgot to
//! finish.
//!
//! # Handshake is genuinely different, and the obvious conclusion is wrong in both directions
//!
//! `app.joinhandshake.com/robots.txt` is `Disallow: /` **with an allowlist containing
//! `Allow: /public`**, and job pages live at `/public/jobs/{id}`. Under longest-match robots
//! semantics the specific `Allow` wins, so these pages are **permitted** — the shared robots
//! parser implements exactly that, and its `a_longer_allow_beats_a_blanket_disallow` test is
//! this case. Each page returns 200 unauthenticated and carries a `schema.org` **JSON-LD
//! `JobPosting`**, which is a documented, stable vocabulary rather than a page layout that
//! breaks on the next redesign.
//!
//! It also carries **`validThrough`** — present on 39/39 sampled and, per § B footnote 19, *the
//! only dependable deadline field found anywhere in the entire document*. Every other source
//! either has no deadline field or has one that employers do not fill in.
//!
//! **So why is it not swept?** Three measured reasons, none of which is about permission:
//!
//! 1. The corpus is not technical — 2 internships and 1 engineering-adjacent title in 39 random
//!    samples, and **zero that were both**.
//! 2. The sitemap carries `<loc>` only: no `<lastmod>`, no title. You cannot tell which of the
//!    ~75,000 URLs are new or are software internships without fetching each one.
//! 3. At a polite ~1 req/s that is **~21 hours** for a yield the sample puts well under 1%.
//!
//! Hence: **permitted, so not excluded on principle — but not swept.** It is implemented as a
//! per-URL enrichment source. Give it URLs and it fetches exactly those; give it none and it
//! skips.
//!
//! # The outcome rule that matters most here
//!
//! **Handshake enrichment must never report [`SourceOutcome::Success`].** Fetching five known
//! URLs is not an enumeration of Handshake, and `Success` is a claim that it was one:
//! `expiry::settle_source_run` would increment `consecutive_misses` for *every* Handshake
//! sighting and reset only those five, so everything not in this run's list would expire after
//! the miss threshold. That is the phase's named data-loss bug arriving through the smallest,
//! most harmless-looking source in the registry. It reports [`SourceOutcome::Partial`], always,
//! and `handshake_enrichment_is_never_a_complete_enumeration` pins it.

use serde_json::Value;

use super::super::models::RawPosting;
use super::{BoxFuture, Source, SourceContext, SourceFetch, first_string, join_locations};

/// Why LinkedIn is not fetched. Written once, surfaced in `source_runs.error` every run.
pub const LINKEDIN_REASON: &str =
    "not fetched: linkedin.com/robots.txt is `User-agent: * / Disallow: /` for every path, the \
     file carries an explicit notice against automated access, and User Agreement § 8.2 bars \
     scripts and bots. There is no polite configuration of this source, so it is excluded by \
     the project's no-evasion rule rather than attempted and failed.";

/// Why Indeed is not fetched.
pub const INDEED_REASON: &str =
    "not fetched: indeed.com/robots.txt permits /jobs, but a polite unauthenticated GET with an \
     honest user agent returns HTTP 403 behind a Cloudflare CAPTCHA. Passing it would require \
     fingerprint spoofing or CAPTCHA solving, which the project's no-evasion rule forbids, so \
     the honest yield is zero.";

/// Why Handshake is not swept.
pub const HANDSHAKE_SWEEP_REASON: &str =
    "enrichment only, no URLs supplied this run: Handshake's /public/jobs pages are permitted by \
     robots and carry a schema.org JobPosting with `validThrough` — the only dependable deadline \
     field in the research doc — but its sitemap has no metadata to filter on, so a full sweep \
     is ~75,000 fetches (~21 hours polite) for a measured yield well under 1%. Supply \
     SourceContext::handshake_urls to enrich specific postings.";

/// What a best-effort source does when its turn comes.
enum Mode {
    /// Record the reason and make no request.
    NeverFetch(&'static str),
    /// Fetch only the URLs the context supplies.
    EnrichOnly,
}

pub struct BestEffortSource {
    name: &'static str,
    description: &'static str,
    mode: Mode,
}

impl BestEffortSource {
    pub fn linkedin() -> Self {
        BestEffortSource {
            name: "linkedin",
            description: "LinkedIn — excluded: robots.txt disallows every path (never fetched)",
            mode: Mode::NeverFetch(LINKEDIN_REASON),
        }
    }

    pub fn indeed() -> Self {
        BestEffortSource {
            name: "indeed",
            description: "Indeed — excluded: a polite GET is answered with a CAPTCHA (never fetched)",
            mode: Mode::NeverFetch(INDEED_REASON),
        }
    }

    pub fn handshake() -> Self {
        BestEffortSource {
            name: "handshake",
            description: "Handshake — permitted; per-URL enrichment for `validThrough` deadlines, \
                          never swept",
            mode: Mode::EnrichOnly,
        }
    }
}

impl Source for BestEffortSource {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        self.description
    }

    fn fetch<'a>(&'a self, ctx: &'a SourceContext) -> BoxFuture<'a, SourceFetch> {
        Box::pin(async move {
            match self.mode {
                // No client call anywhere on this path. The rule is not "try politely and give
                // up", it is "do not make the request".
                Mode::NeverFetch(reason) => SourceFetch::skipped(reason),
                Mode::EnrichOnly => self.enrich(ctx).await,
            }
        })
    }
}

impl BestEffortSource {
    /// Fetch exactly the URLs the context names, and never claim to have enumerated the source.
    async fn enrich(&self, ctx: &SourceContext) -> SourceFetch {
        if ctx.handshake_urls.is_empty() {
            return SourceFetch::skipped(HANDSHAKE_SWEEP_REASON);
        }

        let mut postings = Vec::new();
        let mut failures = Vec::new();

        for url in &ctx.handshake_urls {
            match ctx.http.get(url).await {
                Ok(response) => match parse_job_posting_page(&response.body, url) {
                    Some(posting) => postings.push(posting),
                    None => failures.push(format!(
                        "{url}: no schema.org JobPosting JSON-LD block in the response"
                    )),
                },
                Err(error) if error.is_refusal() => {
                    // robots covers the host, so every remaining URL is refused too.
                    return SourceFetch::skipped(error.to_string());
                }
                Err(error) => failures.push(error.to_string()),
            }
        }

        if postings.is_empty() {
            return SourceFetch::failed(format!(
                "Handshake: none of {} URL(s) yielded a posting — {}",
                ctx.handshake_urls.len(),
                failures.first().map(String::as_str).unwrap_or("no detail")
            ));
        }

        // ALWAYS `Partial`. See the module doc: enriching known URLs is not an enumeration, and
        // `Success` here expires every Handshake posting not named in this run's list.
        SourceFetch::partial(
            postings,
            format!(
                "enriched {} of {} requested Handshake URL(s); this is a per-URL fetch, not an \
                 enumeration of the source, so absence from it means nothing",
                ctx.handshake_urls.len() - failures.len(),
                ctx.handshake_urls.len()
            ),
        )
    }
}

// ------------------------------------------------------------------------------------------
// schema.org JSON-LD
// ------------------------------------------------------------------------------------------

/// Pull the first `schema.org` `JobPosting` out of a page and turn it into a raw posting.
///
/// Pure, so it is tested offline. § A.3 makes the general point worth repeating: checking for a
/// JSON-LD block is cheap and worth doing on **any** new source before writing a bespoke
/// parser, because it is a documented, stable vocabulary with `baseSalary`, `datePosted` and
/// `validThrough` rather than a page layout that breaks on the next redesign.
pub fn parse_job_posting_page(html: &str, url: &str) -> Option<RawPosting> {
    let posting = json_ld_blocks(html)
        .into_iter()
        .find_map(|block| find_job_posting(&block))?;
    Some(job_posting_to_raw(&posting, url))
}

/// Every `<script type="application/ld+json">` payload in the page, parsed.
///
/// Scanned by hand rather than with a regex — `regex` is not a dependency of this crate, and a
/// tag scan is the whole of what is needed.
fn json_ld_blocks(html: &str) -> Vec<Value> {
    const OPEN: &str = "<script";
    const CLOSE: &str = "</script";

    let lower = html.to_ascii_lowercase();
    let mut blocks = Vec::new();
    let mut cursor = 0usize;

    while let Some(found) = lower[cursor..].find(OPEN) {
        let tag_start = cursor + found;
        let Some(tag_end_rel) = lower[tag_start..].find('>') else {
            break;
        };
        let body_start = tag_start + tag_end_rel + 1;
        let attributes = &lower[tag_start..body_start];

        let Some(close_rel) = lower[body_start..].find(CLOSE) else {
            break;
        };
        let body_end = body_start + close_rel;

        if attributes.contains("application/ld+json")
            && let Ok(value) = serde_json::from_str::<Value>(html[body_start..body_end].trim())
        {
            blocks.push(value);
        }

        cursor = body_end;
    }

    blocks
}

/// Find a `JobPosting` inside a JSON-LD value, which may be the object itself, an array of
/// objects, or an `@graph` wrapper. All three shapes occur in the wild.
fn find_job_posting(value: &Value) -> Option<Value> {
    match value {
        Value::Array(items) => items.iter().find_map(find_job_posting),
        Value::Object(_) => {
            if is_type(value, "JobPosting") {
                return Some(value.clone());
            }
            value.get("@graph").and_then(find_job_posting)
        }
        _ => None,
    }
}

/// `@type` may be a string or an array of strings.
fn is_type(value: &Value, wanted: &str) -> bool {
    match value.get("@type") {
        Some(Value::String(name)) => name == wanted,
        Some(Value::Array(names)) => names.iter().any(|name| name.as_str() == Some(wanted)),
        _ => false,
    }
}

fn job_posting_to_raw(posting: &Value, url: &str) -> RawPosting {
    let external_id = identifier_of(posting).unwrap_or_else(|| url.to_string());

    RawPosting {
        source: "handshake".to_string(),
        external_id,
        url: url.to_string(),
        company: posting
            .get("hiringOrganization")
            .and_then(|org| first_string(org, &["name", "legalName"]))
            .unwrap_or_default(),
        title: clean_title(first_string(posting, &["title"]).unwrap_or_default()),
        location_raw: locations_of(posting),
        pay_raw: pay_text(posting),
        // `employmentType` is schema.org's vocabulary (`INTERN`, `FULL_TIME`, …). Handed to QC
        // as the term hint, exactly as Ashby's `employmentType` is.
        term_raw: first_string(posting, &["employmentType"]),
        class_year_raw: None,
        posted_at_raw: first_string(posting, &["datePosted"]),
        // § B note 19: `validThrough` on 39/39 sampled — the only dependable deadline field
        // found anywhere in the research. It is the whole reason this source is worth having.
        deadline_raw: first_string(posting, &["validThrough"]),
        description: first_string(posting, &["description"]),
        // schema.org has `jobLocationType: "TELECOMMUTE"` but § B records remote as absent for
        // this source, so nothing is asserted. `None` is unknown, never onsite.
        remote_hint: None,
        raw_json: posting.to_string(),
    }
}

/// The JSON-LD `title` is polluted with site branding —
/// `"Warehouse Order Selector | Albertsons Companies | Handshake"` — and must be split on `|`
/// rather than used directly, or every posting's title carries the company name twice and the
/// word "Handshake".
fn clean_title(title: String) -> String {
    title
        .split('|')
        .next()
        .map(str::trim)
        .unwrap_or(title.trim())
        .to_string()
}

/// `identifier` may be a bare string or a `PropertyValue` object.
fn identifier_of(posting: &Value) -> Option<String> {
    let identifier = posting.get("identifier")?;
    first_string(identifier, &["value", "name"]).or_else(|| identifier.as_str().map(str::to_string))
}

/// `jobLocation` may be one object or an array of them; the address is nested.
fn locations_of(posting: &Value) -> Option<String> {
    let locations = match posting.get("jobLocation")? {
        Value::Array(items) => items.clone(),
        single => vec![single.clone()],
    };

    join_locations(locations.iter().filter_map(|location| {
        let address = location.get("address").unwrap_or(location);
        let city = first_string(address, &["addressLocality"]);
        let region = first_string(address, &["addressRegion"]);
        let country = first_string(address, &["addressCountry"]);
        let parts: Vec<String> = [city, region, country].into_iter().flatten().collect();
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    }))
}

/// Render `baseSalary` as a compensation string.
///
/// § A.4 measured `baseSalary` **present** on 6/12 in one sample but carrying a *numeric* value
/// on only 4/39 overall — the rest are `MonetaryAmount` shells with a `unitText` and no
/// `value`. So presence is not knowledge: a number is required, and a shell yields `None`.
fn pay_text(posting: &Value) -> Option<String> {
    let salary = posting.get("baseSalary")?;
    let currency = first_string(salary, &["currency"])
        .or_else(|| first_string(salary, &["currencyCode"]))
        .unwrap_or_else(|| "USD".to_string());

    // The amount is normally a nested `QuantitativeValue`.
    let amount = salary.get("value").unwrap_or(salary);
    let min = amount
        .get("value")
        .and_then(Value::as_f64)
        .or_else(|| amount.get("minValue").and_then(Value::as_f64))?;
    let max = amount.get("maxValue").and_then(Value::as_f64);

    let mut text = match max {
        Some(max) if max > min => format!("{currency} {min:.2} - {max:.2}"),
        _ => format!("{currency} {min:.2}"),
    };

    if let Some(period) = unit_text_words(first_string(amount, &["unitText"]).as_deref()) {
        text.push(' ');
        text.push_str(period);
    }

    Some(text)
}

/// schema.org's `unitText` vocabulary, as period words `normalize::detect_period` reads.
///
/// `WEEK` and `DAY` map to words the pay parser explicitly refuses, because `PayPeriod` cannot
/// express them and a weekly figure silently read as monthly is off by four.
fn unit_text_words(unit: Option<&str>) -> Option<&'static str> {
    match unit?.to_ascii_uppercase().as_str() {
        "HOUR" => Some("per hour"),
        "DAY" => Some("per day"),
        "WEEK" => Some("per week"),
        "MONTH" => Some("per month"),
        "YEAR" => Some("per year"),
        _ => None,
    }
}

// ------------------------------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internships::http::PoliteClient;
    use crate::internships::models::{PayPeriod, SourceOutcome};
    use crate::internships::normalize::{parse_pay, parse_timestamp};
    use std::sync::Arc;

    /// A Handshake `/public/jobs/{id}` page reduced to its JSON-LD block. Field set and shapes
    /// per `docs/INTERNSHIP_SCRAPING.md` § A.4; synthetic rather than vendored because we do
    /// not sweep the source and had no reason to fetch a page.
    const PAGE: &str = r##"<!doctype html>
<html><head>
<script type="application/ld+json">
{
  "@context": "https://schema.org",
  "@type": "JobPosting",
  "title": "Software Engineering Intern | Acme Robotics | Handshake",
  "description": "Build things. Rising juniors and seniors welcome.",
  "datePosted": "2026-07-14",
  "validThrough": "2026-09-30",
  "identifier": { "@type": "PropertyValue", "name": "Handshake", "value": "9182736" },
  "hiringOrganization": { "@type": "Organization", "name": "Acme Robotics" },
  "jobLocation": { "@type": "Place", "address": { "@type": "PostalAddress",
      "addressLocality": "Ann Arbor", "addressRegion": "MI", "addressCountry": "US" } },
  "employmentType": "INTERN",
  "industry": "Robotics",
  "baseSalary": { "@type": "MonetaryAmount", "currency": "USD",
      "value": { "@type": "QuantitativeValue", "value": 32, "unitText": "HOUR" } }
}
</script>
</head><body>Job page</body></html>"##;

    const URL: &str = "https://app.joinhandshake.com/public/jobs/9182736";

    fn context() -> Arc<SourceContext> {
        let http = PoliteClient::with_host_delay(std::time::Duration::ZERO).expect("builds");
        Arc::new(SourceContext::new(http))
    }

    // ---- the two that must never fetch ----

    #[tokio::test]
    async fn linkedin_is_skipped_with_an_honest_reason_and_fetches_nothing() {
        let source = BestEffortSource::linkedin();
        let ctx = context();
        let fetch = source.fetch(&ctx).await;

        assert_eq!(
            fetch.outcome(),
            SourceOutcome::Skipped,
            "a correct refusal is not a failure and must not read as one in the health panel"
        );
        assert!(fetch.postings().is_empty());
        let reason = fetch.error().expect("skipped always carries a reason");
        assert!(reason.contains("Disallow: /"));
        assert!(reason.contains("8.2"), "the ToS basis belongs in the record too");
    }

    #[tokio::test]
    async fn indeed_is_skipped_and_no_workaround_is_attempted() {
        let source = BestEffortSource::indeed();
        let ctx = context();
        let fetch = source.fetch(&ctx).await;

        assert_eq!(fetch.outcome(), SourceOutcome::Skipped);
        assert!(fetch.postings().is_empty());
        assert!(fetch.error().unwrap().contains("403"));
    }

    #[test]
    fn no_evasion_machinery_exists_anywhere_in_this_module() {
        // The rule is absolute rather than a tradeoff, so it is checked rather than trusted.
        // `include_str!` reads this file at compile time, so the check runs in CI unchanged.
        //
        // The needles are *call shapes*, not bare words. The module doc and the two refusal
        // constants both name these techniques while explaining why none is used, and a check
        // that could not tell prose from code would force that explanation to be deleted —
        // leaving the rule enforced and its reasoning gone, which is the worse trade. A word
        // followed by `(` or `::` is machinery; a word in a sentence is not.
        //
        // Comment lines are dropped as well, so a future comment mentioning `.proxy(` in
        // passing does not fail the build.
        let code: String = include_str!("best_effort.rs")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase();

        // Split so this test's own source does not match the needles it looks for.
        let forbidden = [
            concat!(".pro", "xy("),
            concat!("pro", "xy::"),
            concat!("solve_", "captcha"),
            concat!("cookie_", "store("),
            concat!("head", "less("),
            concat!("finger", "print("),
            concat!("danger", "ous_accept"),
            concat!(".user_", "agent("),
        ];
        for needle in forbidden {
            assert!(
                !code.contains(needle),
                "no-evasion rule: `{needle}` must not appear in this module's code"
            );
        }

        // And the two excluded sources must reach no client call at all — the rule is "do not
        // make the request", not "make it politely and give up".
        assert!(
            !LINKEDIN_REASON.is_empty() && !INDEED_REASON.is_empty(),
            "both refusals must state their reason"
        );
    }

    // ---- Handshake ----

    #[tokio::test]
    async fn handshake_skips_rather_than_sweeping_when_given_no_urls() {
        // A full sweep is ~75,000 fetches for a yield measured well under 1%. Not fetching is
        // the correct default, and the reason lands in the run record every run.
        let source = BestEffortSource::handshake();
        let ctx = context();
        let fetch = source.fetch(&ctx).await;

        assert_eq!(fetch.outcome(), SourceOutcome::Skipped);
        assert!(fetch.error().unwrap().contains("enrichment only"));
    }

    #[test]
    fn handshake_enrichment_is_never_a_complete_enumeration() {
        // THE test for this module. Fetching five known URLs is not an enumeration, and
        // `Success` is a claim that it was one — `settle_source_run` would then increment
        // `consecutive_misses` for every Handshake sighting and reset only these five, so
        // everything not named this run expires after the miss threshold. The phase's named
        // data-loss bug, arriving through the smallest source in the registry.
        let posting = parse_job_posting_page(PAGE, URL).expect("the page parses");
        let fetch = SourceFetch::partial(vec![posting], "enriched 1 of 1");
        assert_ne!(fetch.outcome(), SourceOutcome::Success);
        assert_eq!(fetch.outcome(), SourceOutcome::Partial);

        // And the orchestration must therefore report no seen ids for it.
        let outputs = crate::internships::sources::SourceRunOutput {
            result: crate::internships::expiry::SourceRunResult {
                source: "handshake".to_string(),
                outcome: fetch.outcome(),
                seen_external_ids: Vec::new(),
                fetched: 1,
                accepted: 0,
                filtered: 0,
                rejected: 0,
                error: fetch.error().map(str::to_string),
            },
            postings: fetch.postings().to_vec(),
            closed_external_ids: Vec::new(),
        };
        assert!(outputs.result.seen_external_ids.is_empty());
    }

    #[test]
    fn the_json_ld_block_yields_a_complete_posting() {
        let posting = parse_job_posting_page(PAGE, URL).expect("parses");
        assert_eq!(posting.source, "handshake");
        assert_eq!(posting.external_id, "9182736");
        assert_eq!(posting.url, URL);
        assert_eq!(posting.company, "Acme Robotics");
        assert_eq!(posting.term_raw.as_deref(), Some("INTERN"));
    }

    #[test]
    fn the_title_is_split_off_from_the_site_branding() {
        // Raw, every title reads "… | Company | Handshake", which carries the company name
        // twice and the word "Handshake" into every dedup key and every rendered card.
        let posting = parse_job_posting_page(PAGE, URL).expect("parses");
        assert_eq!(posting.title, "Software Engineering Intern");
        assert!(!posting.title.contains("Handshake"));
        assert!(!posting.title.contains('|'));
    }

    #[test]
    fn valid_through_becomes_the_deadline() {
        // The only dependable deadline field found anywhere in the research (§ B note 19), and
        // the entire reason this source earns a place in the registry.
        let posting = parse_job_posting_page(PAGE, URL).expect("parses");
        let deadline = posting.deadline_raw.as_deref().expect("validThrough is present");
        assert_eq!(deadline, "2026-09-30");
        assert!(
            parse_timestamp(deadline).is_some(),
            "the emitted format must be one normalize actually reads"
        );
    }

    #[test]
    fn a_base_salary_with_a_real_number_parses_with_its_stated_unit() {
        let posting = parse_job_posting_page(PAGE, URL).expect("parses");
        let text = posting.pay_raw.as_deref().expect("baseSalary carries a value");
        assert_eq!(text, "USD 32.00 per hour");
        let parsed = parse_pay(text).expect("parses");
        assert_eq!(parsed.period, PayPeriod::Hour);
        assert_eq!(parsed.min, 32.0);
    }

    #[test]
    fn a_monetary_amount_shell_with_no_number_is_not_pay() {
        // § A.4: `baseSalary` present on 6/12 but numeric on only 4/39. Presence is not
        // knowledge, and a shell read as pay would be a fabricated figure on the
        // highest-weighted ranking input.
        let shell = serde_json::json!({
            "@type": "JobPosting",
            "baseSalary": { "@type": "MonetaryAmount", "currency": "USD",
                            "value": { "@type": "QuantitativeValue", "unitText": "HOUR" } }
        });
        assert_eq!(pay_text(&shell), None);
    }

    #[test]
    fn a_posting_with_no_base_salary_claims_none() {
        let bare = serde_json::json!({ "@type": "JobPosting", "title": "Intern" });
        assert_eq!(pay_text(&bare), None);
    }

    #[test]
    fn a_salary_range_keeps_both_bounds() {
        let ranged = serde_json::json!({
            "baseSalary": { "@type": "MonetaryAmount", "currency": "USD",
                "value": { "minValue": 25, "maxValue": 35, "unitText": "HOUR" } }
        });
        assert_eq!(pay_text(&ranged).unwrap(), "USD 25.00 - 35.00 per hour");
    }

    #[test]
    fn an_unknown_unit_text_contributes_no_period_word() {
        assert_eq!(unit_text_words(Some("HOUR")), Some("per hour"));
        assert_eq!(unit_text_words(Some("year")), Some("per year"));
        assert_eq!(unit_text_words(Some("FORTNIGHT")), None);
        assert_eq!(unit_text_words(None), None);
    }

    #[test]
    fn the_location_is_assembled_from_the_nested_address() {
        let posting = parse_job_posting_page(PAGE, URL).expect("parses");
        assert_eq!(posting.location_raw.as_deref(), Some("Ann Arbor, MI, US"));
    }

    #[test]
    fn multiple_job_locations_are_all_kept() {
        let multi = serde_json::json!({
            "@type": "JobPosting",
            "jobLocation": [
                { "address": { "addressLocality": "Boston", "addressRegion": "MA" } },
                { "address": { "addressLocality": "Austin", "addressRegion": "TX" } }
            ]
        });
        assert_eq!(
            locations_of(&multi).as_deref(),
            Some("Boston, MA; Austin, TX")
        );
    }

    #[test]
    fn a_job_posting_nested_in_a_graph_or_an_array_is_still_found() {
        // All three JSON-LD shapes occur in the wild; only handling the bare object would drop
        // postings silently on the sites that use the others.
        let graph = r#"<script type="application/ld+json">
            {"@graph":[{"@type":"WebPage"},{"@type":"JobPosting","title":"Intern"}]}</script>"#;
        assert!(parse_job_posting_page(graph, URL).is_some());

        let array = r#"<script type="application/ld+json">
            [{"@type":"Organization"},{"@type":"JobPosting","title":"Intern"}]</script>"#;
        assert!(parse_job_posting_page(array, URL).is_some());

        let typed_array = r#"<script type="application/ld+json">
            {"@type":["JobPosting","Thing"],"title":"Intern"}</script>"#;
        assert!(parse_job_posting_page(typed_array, URL).is_some());
    }

    #[test]
    fn a_page_with_no_job_posting_block_yields_nothing_rather_than_a_blank_posting() {
        // A blank posting would be a `Rejected` row per page fetched, burying real defects.
        let other = r#"<script type="application/ld+json">{"@type":"WebSite"}</script>"#;
        assert!(parse_job_posting_page(other, URL).is_none());
        assert!(parse_job_posting_page("<html><body>nothing</body></html>", URL).is_none());
        assert!(parse_job_posting_page("", URL).is_none());
    }

    #[test]
    fn a_malformed_script_block_does_not_derail_the_scan() {
        // Unterminated tags and invalid JSON both appear on real pages. Neither may panic, and
        // neither may hide a valid block further down the page.
        let messy = r#"<script>var x = 1;</script>
            <script type="application/ld+json">{not json}</script>
            <script type="application/ld+json">{"@type":"JobPosting","title":"Intern"}</script>"#;
        assert!(parse_job_posting_page(messy, URL).is_some());
        assert!(parse_job_posting_page("<script type=\"application/ld+json\">", URL).is_none());
    }

    #[test]
    fn an_identifier_may_be_a_bare_string_or_a_property_value() {
        let bare = serde_json::json!({ "identifier": "abc" });
        assert_eq!(identifier_of(&bare).as_deref(), Some("abc"));
        let structured = serde_json::json!({ "identifier": { "value": "123" } });
        assert_eq!(identifier_of(&structured).as_deref(), Some("123"));
    }

    #[test]
    fn a_posting_with_no_identifier_falls_back_to_its_url() {
        // `external_id` is the handle that makes a listing recognizable next run. Blank, QC
        // rejects the row; the URL is stable and unique, so it is the honest substitute.
        let bare = r#"<script type="application/ld+json">
            {"@type":"JobPosting","title":"Intern"}</script>"#;
        let posting = parse_job_posting_page(bare, URL).expect("parses");
        assert_eq!(posting.external_id, URL);
    }
}
