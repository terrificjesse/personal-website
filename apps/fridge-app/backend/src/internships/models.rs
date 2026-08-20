//! Shared types for the internship tab (Phase 7).
//!
//! This module is the contract between the three halves of the pipeline — source adapters
//! produce [`RawPosting`], the QC pass turns each into a [`QcOutcome`], and ranking consumes
//! the [`Posting`] rows that survive. It is deliberately the only place any of them agree on
//! a shape, so that "what is a posting" cannot drift between them.
//!
//! # The rule this module exists to enforce
//!
//! **Absent is not zero.** Pay is missing from most sources; a posting with no salary must
//! not rank as though it pays nothing, and a company we know nothing about is not a company
//! we know to be bad. Migration `0012` enforces that with CHECK constraints; this module
//! enforces the same thing in the type system, which is stronger because it fails at compile
//! time rather than on insert:
//!
//! - Pay is `Option<PayRange>`, **not** four independent `Option` fields. You cannot read an
//!   amount without having handled the absent case, and you cannot construct half a pay
//!   figure — an amount with no currency is not a comparable quantity.
//! - [`Location::is_remote`] is `Option<bool>`. `None` is *unknown*, which is a third state
//!   and not a synonym for `Some(false)`.
//! - [`Posting::posted_at`] carries [`Posting::posted_at_is_estimated`] beside it, because a
//!   date we inferred and a date the source stated are different evidence.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ------------------------------------------------------------------------------------------
// Small enums
// ------------------------------------------------------------------------------------------

/// Which hiring season a posting is for. `None` on a [`Posting`] means the source didn't say —
/// deliberately never guessed from the posted date, since a listing published in October is
/// more often for next summer than for this fall.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Season {
    Summer,
    Fall,
    Winter,
    Spring,
}

/// The period a pay figure is quoted over. Without this an amount is meaningless: `45` is a
/// good hourly rate and a catastrophic annual one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PayPeriod {
    Hour,
    Month,
    Year,
}

/// Why a posting was expired. Always set together with `expired_at`; migration `0012` has a
/// CHECK making the half-set state unrepresentable in the database too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpiryReason {
    /// The source said so outright — Simplify's `active: false`, SmartRecruiters' `active`,
    /// or a Greenhouse API 404 on the job. The strongest closure evidence available, and
    /// preferred over every rule below when a source offers it.
    ///
    /// Note the trap recorded in `docs/INTERNSHIP_SCRAPING.md` D.2: a dead Greenhouse job's
    /// *public HTML URL* redirects to the board root with **HTTP 200**, so liveness checked
    /// by status code on the public URL concludes every dead posting is alive, forever, with
    /// no error to alert on. Only the API endpoint answers honestly.
    SourceMarkedClosed,
    /// The posting's stated deadline is in the past.
    DeadlinePassed,
    /// Every source that carried it has stopped listing it, across enough *expiry-eligible*
    /// runs to rule out a transient failure. See `posting_sightings.consecutive_misses`.
    VanishedFromSources,
    /// A human said so.
    Manual,
}

/// How a source run ended.
///
/// Four variants rather than success/failure because the difference between them is what
/// keeps the expiry sweep honest. Only [`SourceOutcome::Success`] can ever earn the right to
/// expire postings, and even then not unconditionally — see `source_runs.counts_for_expiry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceOutcome {
    /// The adapter completed its **full** enumeration. Absence from this run is evidence.
    Success,
    /// The adapter got partway and gave up (page 3 of 10, 4 boards of 30). The postings it
    /// did return are real; absence proves nothing. Folding this into `Success` is how a
    /// paginated fetch that dies early reports healthy while most of its postings appear to
    /// vanish at once.
    Partial,
    /// Nothing usable: blocked, rate-limited, reshaped, unparseable, timed out.
    Failed,
    /// Deliberately not fetched — `robots.txt` disallowed it, or it is switched off. A
    /// correct outcome, and it must not read as a failure in the run-health panel.
    Skipped,
}

/// Whether a row we didn't keep was correctly excluded or wrongly lost.
///
/// The split is the whole point of `posting_rejects`: a source returning thousands of
/// non-internship jobs we filter out is healthy, while fourteen rows that should have parsed
/// and didn't is a defect. Summed into one number, the defect is invisible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RejectKind {
    /// Correctly excluded: not an internship, not software, wrong term.
    Filtered,
    /// Should have been usable and wasn't. Every one of these is a potential bug.
    Rejected,
}

// ------------------------------------------------------------------------------------------
// Value types
// ------------------------------------------------------------------------------------------

/// A pay figure that is actually comparable.
///
/// Constructed as a whole or not at all — that is the point of the type. There is no way to
/// hold an amount without its currency and period, so no ranking code can accidentally
/// compare `45` (per hour) against `52000` (per year), and no absent salary can be read as
/// `0.0` without the compiler making you say so.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PayRange {
    pub min: f64,
    /// `None` means the source quoted a single figure, not a range — distinct from a range
    /// whose upper bound happens to equal its lower.
    pub max: Option<f64>,
    /// ISO 4217, uppercased ("USD"). Retained rather than normalized away, so a non-USD
    /// posting is visibly non-USD instead of silently mis-scaled.
    pub currency: String,
    pub period: PayPeriod,
}

impl PayRange {
    /// The midpoint of the range, or the point value when there is no upper bound.
    ///
    /// This is the single figure ranking should compare on, so that a wide range and a point
    /// value inside it are treated consistently.
    pub fn midpoint(&self) -> f64 {
        match self.max {
            Some(max) => (self.min + max) / 2.0,
            None => self.min,
        }
    }
}

/// Where the job is. Every field is optional because sources disagree wildly about how much
/// structure they give — some hand over a parsed city/region/country, most hand over a string.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Location {
    /// Exactly what the source said, kept so a parse that produced nothing is inspectable
    /// rather than merely empty.
    pub raw: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    /// `None` = unknown, `Some(false)` = onsite, `Some(true)` = remote.
    ///
    /// Three states, not two. Defaulting this to `false` would assert "onsite" for every
    /// source that omits the field, and a remote filter would then quietly exclude postings
    /// that may well be remote.
    pub is_remote: Option<bool>,
}

/// Eligibility expressed as a range of graduation years, which is the form that filters in
/// SQL. "Rising senior" and "graduating Dec 2026 – June 2027" both normalize into this.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClassYearRange {
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub raw: Option<String>,
}

impl ClassYearRange {
    /// Whether a given graduation year is eligible.
    ///
    /// An unbounded end is open, not closed: a posting that names no minimum admits every
    /// earlier year. **A range with no bounds at all admits everyone**, which is the correct
    /// reading of "the source didn't say" — an unstated restriction is not a restriction.
    pub fn admits(&self, grad_year: i64) -> bool {
        self.min.is_none_or(|min| grad_year >= min) && self.max.is_none_or(|max| grad_year <= max)
    }
}

// ------------------------------------------------------------------------------------------
// The pipeline's three stages
// ------------------------------------------------------------------------------------------

/// What a source adapter emits: one listing, unnormalized, straight off the wire.
///
/// Everything optional is a `String` because parsing is the QC pass's job, not the adapter's.
/// An adapter that tried to parse pay would have to be corrected once per source; the QC pass
/// is corrected once. The only fields an adapter must supply are the ones that identify the
/// listing at all — without a company, title, or URL there is nothing to keep.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawPosting {
    /// Stable source name, e.g. `"greenhouse"`. Matches `source_runs.source`.
    pub source: String,
    /// The source's own identifier for this listing. Paired with `source` this is the handle
    /// that makes the same listing recognizable across runs.
    pub external_id: String,
    pub url: String,

    pub company: String,
    pub title: String,

    pub location_raw: Option<String>,
    pub pay_raw: Option<String>,
    pub term_raw: Option<String>,
    pub class_year_raw: Option<String>,
    pub posted_at_raw: Option<String>,
    pub deadline_raw: Option<String>,
    pub description: Option<String>,

    /// A structured remote flag when the source genuinely has one. `None` means the source
    /// didn't say — QC may still infer remoteness from the location or description, but it
    /// must not read `None` here as "onsite".
    pub remote_hint: Option<bool>,

    /// The complete original record. Carried all the way through so a rejected row can be
    /// **diagnosed** rather than merely counted — a reject with no payload tells you
    /// something is wrong and nothing whatsoever about what.
    pub raw_json: String,
}

/// What the QC pass decides about one [`RawPosting`]. Every raw posting produces exactly one
/// of these, which is what makes `fetched = accepted + filtered + rejected` hold.
///
/// There is no fourth "silently dropped" variant, and that absence is the design: a scraper
/// that quietly discards half its input looks perfectly healthy right up until someone counts.
#[derive(Debug, Clone, PartialEq)]
pub enum QcOutcome {
    /// Survived normalization. Boxed because this variant is far larger than the other two,
    /// and an enum is as big as its widest variant.
    Accepted(Box<NormalizedPosting>),
    /// Correctly excluded. `reason` is a stable machine-readable code, not prose.
    Filtered {
        reason: String,
        detail: Option<String>,
    },
    /// Should have been usable and wasn't.
    Rejected {
        reason: String,
        /// Which field failed, when the failure is about one.
        field: Option<String>,
        detail: Option<String>,
    },
}

/// A [`RawPosting`] that survived QC: parsed, validated, and ready to be deduped into a
/// [`Posting`]. Still carries its source identity, because dedup needs to know which
/// sighting to record.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedPosting {
    pub source: String,
    pub external_id: String,
    pub url: String,

    /// Display form, as the source spelled it.
    pub company_name: String,
    /// Normalized for matching and for joining `company_signals`.
    pub company_key: String,
    pub title: String,

    pub term_season: Option<Season>,
    pub term_year: Option<i64>,
    pub location: Location,

    /// `None` is the common case and means **unknown**, never zero.
    pub pay: Option<PayRange>,
    /// What the source said about pay, even when it didn't parse. Keeps "we couldn't parse
    /// it" distinct from "there wasn't one".
    pub pay_raw: Option<String>,

    pub class_years: ClassYearRange,

    /// `None` means the source stated no posting date. The runner backfills this from first
    /// sighting and sets `posted_at_is_estimated`; QC never invents one.
    pub posted_at: Option<DateTime<Utc>>,
    /// `None` means no stated deadline. **Not** "expired", and **not** "closes now" — most
    /// sources have no deadline field at all, and those postings expire by disappearance.
    pub deadline: Option<DateTime<Utc>>,

    pub raw_json: String,
}

/// A deduped posting as it lives in the database: one row per real-world job, however many
/// sources carry it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Posting {
    pub id: String,
    /// `normalized(company)|normalized(title)|term|location`. UNIQUE in the database, so
    /// "a posting present in two sources appears once" is a storage guarantee rather than
    /// application logic a future upsert could forget.
    pub dedup_key: String,

    pub company_key: String,
    pub company_name: String,
    pub title: String,
    pub canonical_url: String,

    pub term_season: Option<Season>,
    pub term_year: Option<i64>,
    pub location: Location,

    pub pay: Option<PayRange>,
    pub pay_raw: Option<String>,
    pub class_years: ClassYearRange,

    pub posted_at: Option<DateTime<Utc>>,
    /// True when `posted_at` was backfilled from first sighting rather than stated by the
    /// source. Without this distinction the entire cold-start corpus dates to the day
    /// collection began and reads to the ranking as "all posted today" — thousands of
    /// postings tied at maximum freshness, which is worse than having no recency signal.
    pub posted_at_is_estimated: bool,
    pub deadline: Option<DateTime<Utc>>,

    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,

    /// `None` = live. Expiry is a soft delete; nothing removes the row.
    pub expired_at: Option<DateTime<Utc>>,
    pub expiry_reason: Option<ExpiryReason>,
}

impl Posting {
    /// Whether this posting is still live. Cheap, but named so read paths say what they mean
    /// rather than repeating `expired_at.is_none()` and eventually getting one of them wrong.
    pub fn is_live(&self) -> bool {
        self.expired_at.is_none()
    }
}

/// Derived per-company signals backing the ranking's prestige input.
///
/// The inputs are stored alongside the output so a score is reproducible by hand — a derived
/// signal otherwise gives you no way to ask why a company scored what it did, which is the
/// one thing a hand-maintained tier list would have given for free.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompanySignals {
    pub company_key: String,
    pub company_name: String,

    pub distinct_sources: i64,
    pub live_postings: i64,
    pub total_postings_seen: i64,
    pub pay_observations: i64,
    pub median_pay_hourly_usd: Option<f64>,

    /// `None` = **not enough evidence**, which ranking must read as unknown rather than as
    /// worst. A derived signal makes this trap easier to fall into than a tier list does,
    /// because the absence looks exactly like a computed `0.0`.
    pub prestige: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midpoint_of_a_range_is_between_its_bounds() {
        let pay = PayRange {
            min: 40.0,
            max: Some(60.0),
            currency: "USD".to_string(),
            period: PayPeriod::Hour,
        };
        assert_eq!(pay.midpoint(), 50.0);
    }

    #[test]
    fn a_point_value_is_its_own_midpoint() {
        let pay = PayRange {
            min: 45.0,
            max: None,
            currency: "USD".to_string(),
            period: PayPeriod::Hour,
        };
        assert_eq!(pay.midpoint(), 45.0);
    }

    #[test]
    fn an_unstated_class_year_restriction_admits_everyone() {
        let any = ClassYearRange::default();
        assert!(any.admits(2024));
        assert!(any.admits(2030));
    }

    #[test]
    fn a_one_sided_class_year_range_is_open_at_the_other_end() {
        let no_earlier_than_2027 = ClassYearRange {
            min: Some(2027),
            max: None,
            raw: None,
        };
        assert!(!no_earlier_than_2027.admits(2026));
        assert!(no_earlier_than_2027.admits(2027));
        assert!(no_earlier_than_2027.admits(2031));
    }

    #[test]
    fn class_year_bounds_are_inclusive_on_both_ends() {
        // Phase 4 lost every recipe rated exactly 4 star to a `>` that should have been `>=`.
        // The boundary years are the ones a real posting actually names, so pin both.
        let range = ClassYearRange {
            min: Some(2026),
            max: Some(2028),
            raw: None,
        };
        assert!(range.admits(2026), "the minimum year must be eligible");
        assert!(range.admits(2028), "the maximum year must be eligible");
        assert!(!range.admits(2025));
        assert!(!range.admits(2029));
    }

    #[test]
    fn a_live_posting_is_one_with_no_tombstone() {
        let posting = Posting {
            id: "p1".to_string(),
            dedup_key: "k1".to_string(),
            company_key: "acme".to_string(),
            company_name: "Acme".to_string(),
            title: "SWE Intern".to_string(),
            canonical_url: "http://x".to_string(),
            term_season: None,
            term_year: None,
            location: Location::default(),
            pay: None,
            pay_raw: None,
            class_years: ClassYearRange::default(),
            posted_at: None,
            posted_at_is_estimated: false,
            deadline: None,
            first_seen_at: Utc::now(),
            last_seen_at: Utc::now(),
            expired_at: None,
            expiry_reason: None,
        };
        assert!(posting.is_live());

        let expired = Posting {
            expired_at: Some(Utc::now()),
            expiry_reason: Some(ExpiryReason::DeadlinePassed),
            ..posting
        };
        assert!(!expired.is_live());
    }
}

// ------------------------------------------------------------------------------------------
// Applied tracker
// ------------------------------------------------------------------------------------------

/// Where an application has got to. Mirrors the CHECK constraint on
/// `internship_applications.status`; the two must agree, so change them together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApplicationStatus {
    Applied,
    /// Online assessment.
    Oa,
    Interview,
    Offer,
    Rejected,
}

impl ApplicationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ApplicationStatus::Applied => "applied",
            ApplicationStatus::Oa => "oa",
            ApplicationStatus::Interview => "interview",
            ApplicationStatus::Offer => "offer",
            ApplicationStatus::Rejected => "rejected",
        }
    }

    /// Parses the wire form. Returns `None` for anything unrecognized rather than defaulting
    /// to `Applied` — a typo'd status that silently became "applied" would quietly reset the
    /// user's progress on that application, which is worse than a 400.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "applied" => Some(ApplicationStatus::Applied),
            "oa" => Some(ApplicationStatus::Oa),
            "interview" => Some(ApplicationStatus::Interview),
            "offer" => Some(ApplicationStatus::Offer),
            "rejected" => Some(ApplicationStatus::Rejected),
            _ => None,
        }
    }
}

/// One tracked application, as the API returns it.
///
/// **Every field down to `notes` comes from the application row's own snapshot.** The tracker
/// view renders from those alone — see the header comment on `internship_applications` in
/// migration `0012`. `posting_is_live` is the only field that depends on the posting still
/// existing, and it is deliberately an `Option` so "the posting is gone" has somewhere to be
/// said rather than being silently reported as live.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Application {
    pub id: String,
    /// May be set but no longer resolve: foreign keys are not enforced in this database, so a
    /// hard-deleted posting leaves this dangling rather than NULL.
    pub posting_id: Option<String>,

    pub company_name: String,
    pub title: String,
    pub url: String,
    pub location_raw: Option<String>,
    pub pay_min: Option<f64>,
    pub pay_max: Option<f64>,
    pub pay_currency: Option<String>,
    pub pay_period: Option<String>,
    pub term_season: Option<String>,
    pub term_year: Option<i64>,
    pub source: Option<String>,
    pub snapshot_at: DateTime<Utc>,

    pub status: String,
    pub applied_at: DateTime<Utc>,
    pub status_changed_at: DateTime<Utc>,
    pub notes: Option<String>,

    /// Three states, and the third is the point:
    /// - `Some(true)` — the posting still exists and is open.
    /// - `Some(false)` — it exists and has been expired.
    /// - `None` — there is no posting to ask: either never linked, or the row is gone and
    ///   this application's `posting_id` no longer resolves.
    ///
    /// A `None` here must never be rendered as "closed" — we don't know that, and the
    /// snapshot above is still perfectly good.
    pub posting_is_live: Option<bool>,
}

/// Longest note we'll store. Same reasoning as the blog's body cap: an unbounded text column
/// filled by a client is a denial-of-service vector, not a feature.
pub const MAX_APPLICATION_NOTES_LENGTH: usize = 10_000;

#[derive(Debug, Clone, Deserialize)]
pub struct CreateApplicationRequest {
    pub posting_id: String,
    /// Defaults to `applied` when omitted, which is what pressing "I applied" means.
    pub status: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateApplicationRequest {
    pub status: Option<String>,
    /// `Some("")` clears the notes; `None` leaves them alone. The distinction is why this is
    /// `Option<String>` and not `String`.
    pub notes: Option<String>,
}
