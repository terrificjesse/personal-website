//! Which new postings are worth interrupting you about.
//!
//! The whole predicate is: **the company is curated tier 1 or tier 2**. That is the existing
//! [`prestige::CompanyTiers::tier`] and nothing else — no new score, no new weighting, no
//! second ranking to keep in sync with `rank`.
//!
//! # `None` is not tier 3
//!
//! An unlisted company scores `None`, which means *unknown*, not *low* — the rule this whole
//! phase is built on, stated in `prestige`'s own module doc and in migration `0012`'s comment
//! on `company_signals.prestige`. Most companies are unlisted: the curated file names 44 of
//! the ~455 in the corpus. So alerting on `None` alerts on essentially every posting
//! collected, and a channel that fires on everything gets muted, taking the tier-1 alerts
//! with it.
//!
//! That is also why this consults the tier file directly rather than
//! `company_signals.prestige`: the derived band exists to *rank* companies we know little
//! about, which is a different question from whether to wake you up. And the signals table is
//! recomputed after every source has finished, so at the moment a posting is inserted it does
//! not yet hold a score for a company first seen in this very run.

use crate::hunt::events::{EventKind, NewHuntEvent};

use super::models::NormalizedPosting;
use super::prestige::CompanyTiers;

/// The curated tiers that raise an alert. Named rather than written inline at the comparison,
/// so "which tiers alert" is answerable by reading one line.
pub const ALERT_TIERS: [u8; 2] = [1, 2];

/// An alert for a newly collected posting, or `None` if it isn't worth one.
///
/// Called only where a posting is genuinely new — see `collector::collect_with`. This
/// function judges the company, not the novelty.
pub fn posting_event(
    tiers: &CompanyTiers,
    posting: &NormalizedPosting,
    posting_id: &str,
) -> Option<NewHuntEvent> {
    let tier = tiers.tier(&posting.company_key)?;
    if !ALERT_TIERS.contains(&tier) {
        return None;
    }

    Some(NewHuntEvent {
        kind: EventKind::Posting,
        // Postings come from the shared corpus and belong to no one user. See migration
        // `0014`'s comment on `hunt_events.user_id`.
        user_id: None,
        subject_id: posting_id.to_string(),
        title: format!("New at {}", posting.company_name),
        body: notification_body(posting),
        url: Some(posting.url.clone()),
        payload: serde_json::json!({
            "posting_id": posting_id,
            "company_key": posting.company_key,
            "company_name": posting.company_name,
            "title": posting.title,
            "tier": tier,
            "url": posting.url,
            "source": posting.source,
            "term_season": posting.term_season,
            "term_year": posting.term_year,
            "location_raw": posting.location.raw,
            "is_remote": posting.location.is_remote,
        }),
    })
}

/// Longest body we will put in a notification.
///
/// A desktop notification shows roughly two lines and truncates the rest without saying so.
/// Truncating here instead means the cut is visible (an ellipsis) and lands after the parts
/// that matter, since the role comes first.
const MAX_BODY_CHARS: usize = 140;

/// The notification's second line: the role, then whatever else we actually know.
///
/// Absent facts are **omitted, never filled in** — no "Location unknown", no guessed term.
/// Same rule as everywhere else in this phase: a posting whose term the source didn't state
/// must not be presented as though it had.
fn notification_body(posting: &NormalizedPosting) -> String {
    let mut parts = vec![posting.title.clone()];

    if let (Some(season), Some(year)) = (posting.term_season, posting.term_year) {
        parts.push(format!("{season:?} {year}"));
    } else if let Some(year) = posting.term_year {
        parts.push(year.to_string());
    }

    match (&posting.location.raw, posting.location.is_remote) {
        (Some(raw), _) => parts.push(summarize_locations(raw)),
        (None, Some(true)) => parts.push("Remote".to_string()),
        (None, _) => {}
    }

    truncate(&parts.join(" · "), MAX_BODY_CHARS)
}

/// One location, plus a count of the rest.
///
/// Found on the first real run rather than in a test: Simplify packs every city a job is open
/// in into one `location` string, and a single Google posting produced **thirty** of them —
/// a 429-character notification body whose role and term were pushed off the end. The
/// per-location rows are one posting by design (`dedup` deliberately keeps location out of
/// the merge key), so this is the normal shape of a big-company listing, not an outlier.
fn summarize_locations(raw: &str) -> String {
    let mut locations = raw.split(';').map(str::trim).filter(|part| !part.is_empty());

    let Some(first) = locations.next() else {
        return raw.trim().to_string();
    };

    match locations.count() {
        0 => first.to_string(),
        rest => format!("{first} +{rest} more"),
    }
}

/// Cut on a character boundary, and say that it was cut.
fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    // `chars`, not bytes: a city name can end mid-UTF-8 and slicing there panics.
    let kept: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internships::models::{ClassYearRange, Location, Season};

    fn tiers() -> CompanyTiers {
        // The shipped file, so the tests exercise the real judgement rather than a fixture's.
        CompanyTiers::load()
    }

    fn posting(company_key: &str, company_name: &str) -> NormalizedPosting {
        NormalizedPosting {
            source: "greenhouse".to_string(),
            external_id: "1".to_string(),
            url: "https://boards.greenhouse.io/x/jobs/1".to_string(),
            company_name: company_name.to_string(),
            company_key: company_key.to_string(),
            title: "Software Engineering Intern".to_string(),
            term_season: Some(Season::Summer),
            term_year: Some(2027),
            location: Location {
                raw: Some("Mountain View, CA".to_string()),
                city: Some("Mountain View".to_string()),
                region: Some("CA".to_string()),
                country: Some("US".to_string()),
                is_remote: Some(false),
            },
            pay: None,
            pay_raw: None,
            class_years: ClassYearRange::default(),
            posted_at: None,
            deadline: None,
            raw_json: "{}".to_string(),
        }
    }

    #[test]
    fn a_tier_one_company_raises_an_alert() {
        let event = posting_event(&tiers(), &posting("google", "Google"), "p1").expect("alert");
        assert_eq!(event.kind, EventKind::Posting);
        assert_eq!(event.subject_id, "p1");
        assert_eq!(event.user_id, None);
        assert!(event.title.contains("Google"));
        assert_eq!(event.payload["tier"], 1);
    }

    #[test]
    fn a_tier_two_company_raises_an_alert() {
        let event = posting_event(&tiers(), &posting("stripe", "Stripe"), "p1").expect("alert");
        assert_eq!(event.payload["tier"], 2);
    }

    #[test]
    fn a_tier_three_company_does_not() {
        // Tier 3 is a real, deliberate band in the curated file — it is listed, and still not
        // worth an interruption. Guards against the predicate degrading to "is it listed".
        let tiers = tiers();
        let tier_three = tiers
            .tier("intel")
            .expect("intel should be in the curated file");
        assert_eq!(tier_three, 3, "fixture assumes intel is tier 3");
        assert_eq!(posting_event(&tiers, &posting("intel", "Intel"), "p1"), None);
    }

    #[test]
    fn an_unlisted_company_does_not_alert_because_unknown_is_not_low() {
        // The trap this module exists to avoid. `None` means we have no judgement about this
        // company, and most companies are `None` — alerting on them alerts on everything.
        let tiers = tiers();
        assert_eq!(tiers.tier("some-startup-nobody-listed"), None);
        assert_eq!(
            posting_event(&tiers, &posting("some-startup-nobody-listed", "Some Startup"), "p1"),
            None
        );
    }

    #[test]
    fn the_body_states_only_what_the_source_said() {
        let mut unknown = posting("google", "Google");
        unknown.term_season = None;
        unknown.term_year = None;
        unknown.location = Location::default();

        let body = notification_body(&unknown);
        assert_eq!(body, "Software Engineering Intern");
        assert!(!body.to_lowercase().contains("unknown"));
    }

    #[test]
    fn the_body_carries_the_term_and_location_when_they_are_known() {
        let body = notification_body(&posting("google", "Google"));
        assert_eq!(
            body,
            "Software Engineering Intern · Summer 2027 · Mountain View, CA"
        );
    }

    #[test]
    fn a_posting_open_in_thirty_cities_still_reads_as_one_line() {
        // The real string a live Simplify run produced for one Google posting. It made a
        // 429-character notification body, which no desktop notification will show.
        let mut wide = posting("google", "Google");
        wide.location.raw = Some(
            "Palo Alto, CA; Cambridge, MA; Madison, WI; Seattle, WA; Washington, DC; SF; \
             Austin, TX; LA; San Jose, CA; Irvine, CA; South SF; Redwood City, CA; \
             Raleigh, NC; San Bruno, CA; Redmond, WA; Durham, NC; Santa Cruz, CA; \
             Chicago, IL; Goleta, CA; Pittsburgh, PA; Kirkland, WA; Reston, VA; NYC; \
             Bellevue, WA; Sunnyvale, CA; Mountain View, CA; Portland, OR; Boulder, CO; \
             Atlanta, GA; San Diego, CA"
                .to_string(),
        );

        let body = notification_body(&wide);
        assert_eq!(
            body,
            "Software Engineering Intern · Summer 2027 · Palo Alto, CA +29 more"
        );
        assert!(body.chars().count() <= MAX_BODY_CHARS);
        // The role has to survive: it is the part that tells you whether to care.
        assert!(body.starts_with("Software Engineering Intern"));
    }

    #[test]
    fn a_single_location_is_left_alone() {
        assert_eq!(summarize_locations("Mountain View, CA"), "Mountain View, CA");
        assert_eq!(summarize_locations("Remote (US);"), "Remote (US)");
    }

    #[test]
    fn an_absurdly_long_title_is_cut_visibly_and_on_a_character_boundary() {
        let mut long = posting("google", "Google");
        long.title = "Software Engineering Intern — Distributed Systems, Infrastructure \
                      Platform, Storage and Réplication (Île-de-France)"
            .to_string();

        let body = notification_body(&long);
        assert!(body.chars().count() <= MAX_BODY_CHARS);
        assert!(body.ends_with('…'), "a cut must be visible: {body:?}");
    }
}
