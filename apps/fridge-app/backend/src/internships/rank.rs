//! Filtering and ranking for the internship tab (Phase 7).
//!
//! Not a Learning Mode area — the user decided that explicitly on 2026-08-20 for the ranking
//! specifically. See `docs/PLAN.md` § Phase 7.
//!
//! This module is **pure**: no SQL, no `sqlx`, no handlers. It takes the postings the query
//! layer already loaded and returns them filtered, scored and ordered.
//!
//! # Three passes, and they are separate on purpose
//!
//! 1. **Hard filters** ([`InternshipFilters::admits`]) — pure exclusion. A user-supplied
//!    filter *removes* postings; it never nudges a score. This rule was learned the hard way
//!    in Phases 3–4 and is enforced structurally here: [`score_posting`] does not receive
//!    [`InternshipFilters`] and therefore **cannot** read one. Filtering only deletes
//!    elements, so the relative order of the survivors is identical to their order in an
//!    unfiltered ranking — pinned by
//!    `a_filter_removes_postings_without_reordering_the_survivors`.
//! 2. **Scoring** ([`score_posting`]) — a weighted composite over five inputs, each scored on
//!    its own `0.0..=1.0` scale and then weighted. The per-input breakdown is published on
//!    [`RankedPosting::breakdown`]; [`RankedPosting::score`] is by construction the sum of the
//!    contributions in that breakdown, so a composite score can always be decomposed.
//! 3. **Ordering** ([`SortBy`]) — every posting is scored the same way regardless of the sort;
//!    only the order changes, so the breakdown stays available whichever axis is on screen.
//!
//! Every sort is descending by `total_cmp` (`f64` has no `Ord`) over a single key, tie-broken
//! by ascending posting id so two identical requests return an identical list.
//!
//! # Sorting by a single axis, and why unknowns behave differently there
//!
//! [`SortBy::Composite`] is the default and orders by the weighted score above. The other
//! variants order by one field. Each has exactly one natural direction — pay highest first,
//! posted newest first, deadline soonest first, prestige highest first — so there is no
//! ascending/descending flag to get wrong.
//!
//! **Under a single-axis sort, unknown values are not imputed: they sort last, in every
//! direction.** This is not a contradiction of the absent-data policy below, it is the same
//! reasoning applied to a different question. The composite answers "how good is this posting
//! overall", where refusing to guess would let one missing field decide everything, so an
//! unknown is imputed to neutral. A single-axis sort answers "show me these ordered by their
//! actual pay", where imputation is a lie with a number attached: a posting with no salary
//! imputed to [`PAY_NEUTRAL_HOURLY_USD`] would land mid-list among postings whose figures are
//! real, and "no deadline" would head a soonest-first list. So the axis sorts on the raw
//! field, and anything unknown goes to the bottom.
//!
//! Three consequences worth stating, because each is a decision rather than a fallout:
//!
//! - **An axis reads the raw field, not the composite's sub-score.** `pay_scale` saturates at
//!   [`PAY_SCALE_CEILING_HOURLY_USD`] and the recency score caps estimated dates at neutral —
//!   both correct for scoring, both destructive of ordering, since they flatten distinct
//!   values into ties. [`SortBy::Pay`] orders by hourly USD and [`SortBy::Posted`] by the
//!   timestamp.
//! - **An inherited company median is unknown to [`SortBy::Pay`].** It is evidence about the
//!   company, not this posting's figure, and a request to sort by pay is a request to see
//!   figures. It still counts for the composite, where it is basis-labelled.
//! - **An estimated `posted_at` is *known* to [`SortBy::Posted`].** First sighting is an
//!   observation, not an absence, and burying the whole cold-start corpus at the bottom of the
//!   sort it was asked for helps nobody. The composite still refuses to let that estimate buy
//!   freshness; the axis just orders by the date we have.
//!
//! A deadline that has already passed is a third case: known, but not actionable. It sorts
//! after every still-open deadline and before the unknowns, most-recently-passed first. This
//! only shows up in the window between a deadline passing and the expiry sweep running, which
//! is exactly when a soonest-first list would otherwise be topped by dead postings.
//!
//! # Absent data — the policy table
//!
//! This is the core of the module. Most sources carry pay for no posting at all, half of them
//! carry no deadline, and `prestige` is `None` until a company has been seen enough times.
//! **Absent is never zero and never best.** Every input is imputed to `UNKNOWN_SCORE`, the
//! exact midpoint of the common `0.0..=1.0` sub-score scale, unless there is real evidence to
//! do better. The reported [`ScoreBasis`] says which happened, per input, per posting.
//!
//! | Input | What absent looks like | Policy | Equivalent to a posting that… |
//! |---|---|---|---|
//! | Pay | `pay: None` (the common case) | company median if the company has ≥ [`PAY_IMPUTATION_MIN_OBSERVATIONS`] pay observations, shrunk toward neutral by [`PAY_IMPUTATION_CONFIDENCE`]; otherwise neutral | …pays [`PAY_NEUTRAL_HOURLY_USD`] |
//! | Pay | stated but not in [`PAY_REFERENCE_CURRENCY`] | neutral, basis [`ScoreBasis::NotComparable`] — no median imputation, because a second guess laid over a known figure is worse than admitting we can't convert it | …pays [`PAY_NEUTRAL_HOURLY_USD`] |
//! | Posted date | `posted_at: None` | neutral | …was posted [`POSTED_HALFLIFE_DAYS`] ago |
//! | Posted date | `posted_at_is_estimated: true` | decay from the estimate, **capped at neutral** | …is at least that old — see below |
//! | Deadline | `deadline: None` (most postings) | neutral | …closes midway down the urgency ramp |
//! | Location | `is_remote: None` | neutral, which is the exact midpoint of [`LOCATION_REMOTE_SCORE`] and [`LOCATION_ONSITE_SCORE`] | …is half remote, which is what "we don't know" means here |
//! | Prestige | `prestige: None`, or no `company_signals` row at all | neutral | …is a company with an average derived signal |
//!
//! Two of those deserve their reasoning spelled out:
//!
//! - **An estimated posted date is capped, not trusted.** `posted_at_is_estimated` means the
//!   date was backfilled from first sighting, so it is a *lower bound* on the posting's true
//!   age — which makes the freshness computed from it an *upper bound* on true freshness.
//!   Taking it at face value is what makes an entire cold-start corpus read as "posted today".
//!   So it decays normally but can never score above `UNKNOWN_SCORE`: an estimate may tell us
//!   a posting is old (we have been watching it for ninety days, so it *is* at least ninety
//!   days old), and may never tell us one is fresh.
//! - **Why imputation rather than renormalizing the weights over the present inputs.**
//!   Renormalizing sounds more principled and is the worse failure: a posting with one known
//!   input and five unknowns has its entire score set by that one input, so anything with a
//!   single strong signal and no other data floats to the top — the "unknown ranks best" trap,
//!   arrived at while fixing the "unknown ranks zero" one. Imputation bounds every unknown to
//!   the middle of its own scale, and the weights below are therefore **fixed and never
//!   renormalized**. A posting about which literally nothing is known scores exactly
//!   `UNKNOWN_SCORE`, dead centre — pinned by `a_posting_we_know_nothing_about_scores_neutral`.
//!
//! # Hard filters meeting unknown data
//!
//! A filter has three states, not two: absent, present-and-keeping-unknowns, and
//! present-and-dropping-them. The third state is why [`OnUnknown`] exists and why it has no
//! `Default` — a caller has to say which it means, because the silent default is exactly the
//! bug. "Pay ≥ $30/hr" against a corpus where pay is mostly absent either hides most of the
//! corpus or stops being a floor, and neither is right often enough to be assumed.
//!
//! | Filter | Meets unknown when | Decision |
//! |---|---|---|
//! | [`PayRangeFilter`] | pay absent, or quoted in another currency | caller's [`OnUnknown`]. No default. Compares [`PayRange::midpoint`] against both bounds, **inclusive** — a posting paying exactly the floor or exactly the ceiling passes |
//! | [`LocationFilter`] | `is_remote: None`, or no location text at all | caller's [`OnUnknown`] |
//! | [`TermFilter`] | `term_season`/`term_year` `None` — common on the ATS APIs | caller's [`OnUnknown`] |
//! | [`ClassYearFilter`] | `ClassYearRange` with no bounds — which is nearly always, see below | caller's [`OnUnknown`]. No default, and here the policy *is* the feature: `Keep` returns almost the whole corpus and `Drop` almost none of it |
//! | company | never — `company_key` is `NOT NULL` | n/a |
//!
//! **The class-year filter's unknown policy is the only thing it mostly does.**
//! `docs/INTERNSHIP_SCRAPING.md` § B measured graduation-year eligibility as absent from every
//! source: `N` on all seven ATSs and both GitHub lists, and `!` — present but uninformative —
//! on Simplify, whose `degrees[]` is degree *level*, not year, and is empty on 22% of rows. So
//! a class-year filter meets an unstated range on very nearly every posting, and whichever way
//! [`OnUnknown`] falls decides the result for the whole corpus rather than for an edge case.
//! That is exactly why it is a required argument and not a default.
//!
//! [`ClassYearFilter`] does **not** reimplement the bounded comparison. It intercepts the
//! wholly-unbounded range — the one case [`ClassYearRange::admits`] answers as an
//! unconditional keep — and delegates every other range, bounded on one side or both, to
//! `admits`. One definition of "does this stated range admit this year", one explicit override
//! of the unstated case, and a test (`the_unknown_class_year_case_is_exactly_what_admits_waves_through`)
//! pinning the coupling so the two cannot drift apart unnoticed.
//!
//! # What this module cannot filter
//!
//! **Source.** `docs/PLAN.md` lists a source filter, and [`Posting`] carries no source: a
//! deduped posting can come from several, and that lives in `posting_sightings`. Rather than
//! accept a field this function would silently ignore — a filter that no-ops is worse than one
//! that does not exist, because the UI ships a control that appears to work — the field is
//! absent from [`InternshipFilters`] and the source filter must be applied by the query layer.
//!
//! # Weights and thresholds — un-tuned placeholders
//!
//! **Nothing here is tuned against real data.** `docs/INTERNSHIP_SCRAPING.md`, which was to
//! carry the per-source field-availability matrix, has since been restored, and the weights
//! below were redistributed once on its evidence (see the class-year note under them). That
//! settles *which inputs exist*; it does not tune what they are worth relative to each other,
//! and nothing here has been checked against a real ranked list. Every number below is a named
//! constant precisely so it stays retunable in one edit.
//!
//! The design is deliberately robust to *any* availability distribution: because unknowns are
//! imputed to a fixed neutral and weights are never renormalized, an input that is absent
//! corpus-wide simply stops differentiating — it does not drag, lift, or reshuffle whole
//! sources. That property does not depend on the weights, which is what makes retuning them
//! safe.
//!
//! | Input | Weight |
//! |---|---|
//! | Pay | [`WEIGHT_PAY`] |
//! | Posted date | [`WEIGHT_RECENCY`] |
//! | Deadline proximity | [`WEIGHT_DEADLINE`] |
//! | Location | [`WEIGHT_LOCATION`] |
//! | Prestige | [`WEIGHT_PRESTIGE`] |
//!
//! They sum to 1.0 (pinned by `the_weights_sum_to_one`), so a total score is itself on
//! `0.0..=1.0` and comparable across requests.
//!
//! # Thresholds
//!
//! Every continuous score here has a named threshold and an explicit inclusive/exclusive
//! choice, because Phase 4 lost every recipe rated exactly 4★ to a `>` that should have been
//! `>=`. The boundary cases are tested landing exactly *on* the constant, never near it:
//! a deadline falling exactly at `now` is still open; pay exactly at the floor passes;
//! `PAY_IMPUTATION_MIN_OBSERVATIONS` observations is enough, not one short.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::internships::models::{
    ClassYearRange, CompanySignals, Location, PayPeriod, PayRange, Posting, Season,
};

// ------------------------------------------------------------------------------------------
// Tuning constants — all un-tuned placeholders. See the module doc.
// ------------------------------------------------------------------------------------------

/// The bottom of every input's sub-score scale.
pub const SCORE_MIN: f64 = 0.0;
/// The top of every input's sub-score scale.
pub const SCORE_MAX: f64 = 1.0;

/// What an unknown input scores: the exact midpoint of the common sub-score scale.
///
/// Derived rather than written as `0.5` so that widening the scale cannot leave the neutral
/// value quietly off-centre. Every "absent" cell in the module doc's policy table resolves
/// here.
pub const UNKNOWN_SCORE: f64 = (SCORE_MIN + SCORE_MAX) / 2.0;

// The five weights below are **still un-tuned placeholders**. They have been redistributed
// once, on evidence, but never calibrated: no ranked list has been checked against real
// postings, and nothing here is derived from how often each field turns out to be populated.
//
// **Class-year fit was a sixth input at 0.10 and was removed on 2026-08-20**, its weight
// redistributed to pay, recency and prestige in proportion to their existing shares. The
// reason is evidential, not conceptual: `docs/INTERNSHIP_SCRAPING.md` § B measured graduation
// year as `N` at every ATS and both GitHub lists, and `!` at Simplify, whose `degrees[]` is
// degree level rather than year and is empty on 22% of rows. § B's instruction is explicit —
// "do not design ranking inputs that require them". Class-year fit remains a perfectly
// sensible thing to score; there is simply no data to score it from, so **do not re-add it
// without a source that actually carries graduation year.** It survives as a hard filter,
// where the user supplies the year rather than the corpus.

/// Relative weight of pay.
pub const WEIGHT_PAY: f64 = 0.29;
/// Relative weight of how recently the posting went up.
pub const WEIGHT_RECENCY: f64 = 0.23;
/// Relative weight of deadline proximity.
pub const WEIGHT_DEADLINE: f64 = 0.15;
/// Relative weight of the location input.
pub const WEIGHT_LOCATION: f64 = 0.10;
/// Relative weight of the derived company prestige signal.
pub const WEIGHT_PRESTIGE: f64 = 0.23;

/// The one unit pay is compared in, everywhere in this module. `company_signals` stores its
/// median in the same unit, which is what makes the imputation below a like-for-like swap.
pub const PAY_REFERENCE_CURRENCY: &str = "USD";

/// Hours in a working year (52 × 40). Used to bring annual figures into hourly.
pub const WORK_HOURS_PER_YEAR: f64 = 2_080.0;
/// Hours in a working month, derived so the two conversions cannot drift apart.
pub const WORK_HOURS_PER_MONTH: f64 = WORK_HOURS_PER_YEAR / 12.0;

/// Hourly pay at or below which the pay input scores [`SCORE_MIN`].
pub const PAY_SCALE_FLOOR_HOURLY_USD: f64 = 15.0;
/// The wage an unknown-pay posting is imputed at.
///
/// This is the number to change to move where "we don't know" sits — the scale's ceiling is
/// derived from it so that unknown pay lands on [`UNKNOWN_SCORE`] exactly, rather than
/// wherever an independently chosen ceiling happened to put the midpoint.
pub const PAY_NEUTRAL_HOURLY_USD: f64 = 40.0;
/// Hourly pay at or above which the pay input scores [`SCORE_MAX`]. Derived, so that
/// [`PAY_NEUTRAL_HOURLY_USD`] is the midpoint of the scale by construction.
pub const PAY_SCALE_CEILING_HOURLY_USD: f64 =
    PAY_SCALE_FLOOR_HOURLY_USD + 2.0 * (PAY_NEUTRAL_HOURLY_USD - PAY_SCALE_FLOOR_HOURLY_USD);

/// How many pay observations a company needs before its median may stand in for a posting's
/// missing pay. Inclusive: exactly this many is enough.
pub const PAY_IMPUTATION_MIN_OBSERVATIONS: i64 = 3;
/// How far an imputed company median is trusted, as a fraction of its distance from neutral.
///
/// A median inherited from sibling postings is weaker evidence than a figure this posting
/// actually states, so it is shrunk toward neutral and can never reach the extremes a stated
/// figure reaches. `1.0` would trust it completely; `0.0` would ignore it.
pub const PAY_IMPUTATION_CONFIDENCE: f64 = 0.7;

/// Days after which a stated posting date has decayed to half its freshness. An unknown
/// posted date scores as a posting exactly this old.
pub const POSTED_HALFLIFE_DAYS: f64 = 30.0;

/// Days to a deadline at or below which a posting is maximally urgent.
pub const DEADLINE_IMMINENT_DAYS: f64 = 7.0;
/// Days to a deadline at or beyond which there is no urgency left to score.
pub const DEADLINE_HORIZON_DAYS: f64 = 90.0;
/// What a deadline at or beyond the horizon scores.
pub const DEADLINE_DISTANT_SCORE: f64 = SCORE_MIN;
/// What an already-passed deadline scores.
///
/// The expiry sweep should have tombstoned the posting before ranking ever sees it; this is
/// the pre-sweep window, where a live posting's stated deadline has just gone by. It scores
/// the same as a deadline past the horizon — both mean there is no urgency to be had here —
/// rather than going negative and dragging the composite below its own scale.
pub const DEADLINE_PASSED_SCORE: f64 = SCORE_MIN;

/// What a posting known to be remote scores on the location input.
pub const LOCATION_REMOTE_SCORE: f64 = SCORE_MAX;
/// What a posting known to be onsite scores on the location input.
///
/// With no location *preference* in scope (see [`InternshipFilters`] — location is a hard
/// filter here, never a weight), the only intrinsic ordering available is accessibility: a
/// remote posting is open to the reader wherever they are and an onsite one may not be. If
/// the tab later grows a stated location preference, this is the input that preference
/// replaces.
pub const LOCATION_ONSITE_SCORE: f64 = SCORE_MIN;

// ------------------------------------------------------------------------------------------
// Hard filters
// ------------------------------------------------------------------------------------------

/// What a hard filter does when it meets a posting whose value for that field is unknown.
///
/// Deliberately has **no `Default`**. Every filter that can meet unknown data makes the caller
/// name a policy, because the wrong silent default is the whole bug: dropping unknowns hides
/// most of a corpus where the field is mostly absent, and keeping them means the floor is not
/// a floor. Which is right depends on what the user meant, so the user's layer chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnUnknown {
    /// Keep the posting. The filter constrains only postings that have the field.
    Keep,
    /// Drop the posting. The filter is a strict requirement, and unknown does not meet it.
    Drop,
}

impl OnUnknown {
    /// Whether this policy keeps a posting whose value is unknown.
    pub fn keeps(self) -> bool {
        matches!(self, OnUnknown::Keep)
    }
}

/// Hard filter on the hiring term. A posting whose term the source never stated is the common
/// case on the ATS APIs, which is why `on_unknown` is not optional.
#[derive(Debug, Clone, PartialEq)]
pub struct TermFilter {
    pub season: Option<Season>,
    pub year: Option<i64>,
    pub on_unknown: OnUnknown,
}

/// Hard filter on where the job is. Covers the plan's "location/remote" as one filter, since
/// they are one dimension and share an unknown policy.
#[derive(Debug, Clone, PartialEq)]
pub struct LocationFilter {
    /// `Some(true)` requires remote, `Some(false)` requires onsite, `None` does not constrain
    /// remoteness. This is a **filter**: `Some(true)` excludes onsite postings outright rather
    /// than down-weighting them.
    pub remote: Option<bool>,
    /// Case-insensitive substring, matched against the raw location string and the parsed
    /// city / region / country.
    pub contains: Option<String>,
    pub on_unknown: OnUnknown,
}

/// Hard filter on pay, expressed in the module's reference unit (hourly USD).
///
/// Both bounds are optional and both are **inclusive**: a posting paying exactly the floor or
/// exactly the ceiling passes. Both `None` is no constraint at all, which keeps
/// `Some(PayRangeFilter)` from being a way to accidentally filter on nothing while still
/// tripping the unknown policy.
///
/// The comparison is against [`PayRange::midpoint`] — the figure that type documents as the
/// one ranking should compare on, so a wide range and a point value inside it are treated
/// alike.
#[derive(Debug, Clone, PartialEq)]
pub struct PayRangeFilter {
    pub min_hourly_usd: Option<f64>,
    pub max_hourly_usd: Option<f64>,
    pub on_unknown: OnUnknown,
}

impl PayRangeFilter {
    /// A pay window that keeps postings whose pay is unknown: "in this range, if it says".
    pub fn keeping_unknown(min_hourly_usd: Option<f64>, max_hourly_usd: Option<f64>) -> Self {
        PayRangeFilter {
            min_hourly_usd,
            max_hourly_usd,
            on_unknown: OnUnknown::Keep,
        }
    }

    /// A pay window that drops postings whose pay is unknown: "only postings that say, and
    /// say something in this range".
    pub fn dropping_unknown(min_hourly_usd: Option<f64>, max_hourly_usd: Option<f64>) -> Self {
        PayRangeFilter {
            min_hourly_usd,
            max_hourly_usd,
            on_unknown: OnUnknown::Drop,
        }
    }

    /// Whether this filter constrains anything at all. Both bounds absent is not a filter.
    fn is_unconstrained(&self) -> bool {
        self.min_hourly_usd.is_none() && self.max_hourly_usd.is_none()
    }

    /// Whether a known hourly figure falls inside the window. Both bounds inclusive.
    fn admits_hourly(&self, hourly_usd: f64) -> bool {
        self.min_hourly_usd.is_none_or(|min| hourly_usd >= min)
            && self.max_hourly_usd.is_none_or(|max| hourly_usd <= max)
    }
}

/// Hard filter on graduation-year eligibility.
///
/// `on_unknown` decides **one case and only one**: a posting whose [`ClassYearRange`] states no
/// bounds at all. Every bounded range, on one side or both, goes to [`ClassYearRange::admits`],
/// which stays the single definition of "does this stated range admit this year". Nothing here
/// re-derives that comparison.
///
/// The policy is required rather than defaulted because it decides the whole corpus, not an
/// edge case — see the module doc's note on `docs/INTERNSHIP_SCRAPING.md` § B.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassYearFilter {
    pub grad_year: i64,
    pub on_unknown: OnUnknown,
}

/// The user's hard filters. **Every field here excludes; none of them scores.**
///
/// [`score_posting`] does not take this struct, so a filter physically cannot become a
/// scoring input — which is the Phase 3–4 rule made structural rather than remembered.
///
/// No `source` field: see the module doc, § What this module cannot filter.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InternshipFilters {
    pub term: Option<TermFilter>,
    pub location: Option<LocationFilter>,
    pub class_year: Option<ClassYearFilter>,
    pub pay: Option<PayRangeFilter>,
    /// Matched against [`Posting::company_key`] (the normalized form), case-insensitively.
    pub company_key: Option<String>,
}

impl InternshipFilters {
    /// Whether a posting survives every hard filter.
    ///
    /// Exclusion only. Nothing in here or below it returns a score, a weight, or a nudge.
    pub fn admits(&self, posting: &Posting) -> bool {
        self.term_admits(posting)
            && self.location_admits(posting)
            && self.class_year_admits(posting)
            && self.pay_admits(posting)
            && self.company_admits(posting)
    }

    fn term_admits(&self, posting: &Posting) -> bool {
        let Some(filter) = &self.term else {
            return true;
        };
        let season_ok = match (filter.season, posting.term_season) {
            (None, _) => true,
            (Some(_), None) => filter.on_unknown.keeps(),
            (Some(wanted), Some(actual)) => wanted == actual,
        };
        let year_ok = match (filter.year, posting.term_year) {
            (None, _) => true,
            (Some(_), None) => filter.on_unknown.keeps(),
            (Some(wanted), Some(actual)) => wanted == actual,
        };
        season_ok && year_ok
    }

    fn location_admits(&self, posting: &Posting) -> bool {
        let Some(filter) = &self.location else {
            return true;
        };
        let remote_ok = match (filter.remote, posting.location.is_remote) {
            (None, _) => true,
            (Some(_), None) => filter.on_unknown.keeps(),
            (Some(wanted), Some(actual)) => wanted == actual,
        };
        let text_ok = match &filter.contains {
            None => true,
            Some(needle) => match location_haystack(&posting.location) {
                None => filter.on_unknown.keeps(),
                Some(haystack) => haystack.contains(&needle.to_lowercase()),
            },
        };
        remote_ok && text_ok
    }

    /// Class-year eligibility.
    ///
    /// The bounded comparison is [`ClassYearRange::admits`]'s and stays that way — the hazard
    /// the fridge app hit with `require_admin` versus its inline `is_admin` reads was two
    /// places *answering the same question*, agreeing only until one of them changed. This is
    /// not that: the unstated case is intercepted and answered by the caller's policy, and
    /// every stated range is handed straight to `admits` without being re-read. The one thing
    /// that could still drift — `admits` ceasing to wave the unstated case through — is pinned
    /// by `the_unknown_class_year_case_is_exactly_what_admits_waves_through`.
    fn class_year_admits(&self, posting: &Posting) -> bool {
        let Some(filter) = &self.class_year else {
            return true;
        };
        if class_year_is_unstated(&posting.class_years) {
            return filter.on_unknown.keeps();
        }
        posting.class_years.admits(filter.grad_year)
    }

    fn pay_admits(&self, posting: &Posting) -> bool {
        let Some(filter) = &self.pay else {
            return true;
        };
        if filter.is_unconstrained() {
            return true;
        }
        match pay_knowledge(posting) {
            PayKnowledge::Known(hourly_usd) => filter.admits_hourly(hourly_usd),
            PayKnowledge::NotComparable | PayKnowledge::Absent => filter.on_unknown.keeps(),
        }
    }

    fn company_admits(&self, posting: &Posting) -> bool {
        match &self.company_key {
            None => true,
            Some(key) => posting.company_key.eq_ignore_ascii_case(key),
        }
    }
}

// ------------------------------------------------------------------------------------------
// Scored output
// ------------------------------------------------------------------------------------------

/// The six inputs the composite is built from. Named rather than stringly-typed so a caller
/// (or a test) can ask about one input without matching on prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreInput {
    Pay,
    Recency,
    Deadline,
    Location,
    Prestige,
}

/// Where an input's value came from. This is the field that makes the absent-data policy
/// visible per posting instead of buried in a doc comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreBasis {
    /// Computed from what the source actually stated.
    Stated,
    /// The input was absent; the value is [`UNKNOWN_SCORE`].
    ImputedNeutral,
    /// Pay was absent and the company had enough observed pay to stand in for it. Shrunk
    /// toward neutral by [`PAY_IMPUTATION_CONFIDENCE`], because inherited evidence is weaker
    /// than stated evidence.
    ImputedFromCompanyMedian,
    /// Present but weaker than stated: an estimated `posted_at`, whose freshness is capped at
    /// [`UNKNOWN_SCORE`] because first-sighting age is only a lower bound on true age.
    EstimatedCapped,
    /// Present but not comparable: pay quoted in a currency this module cannot convert.
    /// Scored as unknown, and flagged so that it is visible rather than silent.
    NotComparable,
}

/// One input's contribution to a composite score, with everything needed to reproduce it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InputScore {
    /// The input's own score, on `SCORE_MIN..=SCORE_MAX`.
    pub value: f64,
    /// The weight this input carries. Fixed; never renormalized over the present inputs.
    pub weight: f64,
    /// `value * weight` — what this input actually contributed to the total.
    pub contribution: f64,
    /// Why `value` is what it is.
    pub basis: ScoreBasis,
}

impl InputScore {
    fn new(value: f64, weight: f64, basis: ScoreBasis) -> Self {
        InputScore {
            value,
            weight,
            contribution: value * weight,
            basis,
        }
    }
}

/// The per-input decomposition of a [`RankedPosting`]'s score.
///
/// A composite score nobody can decompose is a composite score nobody can debug, so this is
/// part of the return value rather than a debug aid behind a flag.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScoreBreakdown {
    pub pay: InputScore,
    pub recency: InputScore,
    pub deadline: InputScore,
    pub location: InputScore,
    pub prestige: InputScore,
}

impl ScoreBreakdown {
    /// Every input, paired with its name. The single place that enumerates them, so
    /// [`ScoreBreakdown::total`] cannot silently omit one that was added to the struct.
    pub fn inputs(&self) -> [(ScoreInput, &InputScore); 5] {
        [
            (ScoreInput::Pay, &self.pay),
            (ScoreInput::Recency, &self.recency),
            (ScoreInput::Deadline, &self.deadline),
            (ScoreInput::Location, &self.location),
            (ScoreInput::Prestige, &self.prestige),
        ]
    }

    /// One input by name.
    pub fn get(&self, input: ScoreInput) -> &InputScore {
        match input {
            ScoreInput::Pay => &self.pay,
            ScoreInput::Recency => &self.recency,
            ScoreInput::Deadline => &self.deadline,
            ScoreInput::Location => &self.location,
            ScoreInput::Prestige => &self.prestige,
        }
    }

    /// The composite. `RankedPosting::score` is this, so the published breakdown and the
    /// number the list is sorted by cannot disagree.
    pub fn total(&self) -> f64 {
        self.inputs()
            .iter()
            .map(|(_, input)| input.contribution)
            .sum()
    }
}

/// A posting with its score and the reason for it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RankedPosting {
    pub posting: Posting,
    /// On `SCORE_MIN..=SCORE_MAX`, because the weights sum to 1.0.
    pub score: f64,
    pub breakdown: ScoreBreakdown,
}

// ------------------------------------------------------------------------------------------
// The entry point
// ------------------------------------------------------------------------------------------

/// How the ranked list is ordered.
///
/// Each variant has exactly one natural direction, so there is no ascending/descending flag —
/// that would double the surface of the API to express "oldest postings first", which nobody
/// wants. Under any variant but [`SortBy::Composite`], unknown values sort **last**, and are
/// not imputed; see the module doc for why that differs from the composite's policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    /// The weighted composite score, highest first. What the endpoint did before this
    /// parameter existed, which is what makes it the right default — unlike [`OnUnknown`],
    /// where there is no correct fallback and so no `Default` derive.
    #[default]
    Composite,
    /// Stated pay, highest first, in hourly USD.
    Pay,
    /// Posted date, newest first.
    Posted,
    /// Stated deadline, soonest first.
    Deadline,
    /// Derived company prestige, highest first.
    Prestige,
}

impl SortBy {
    /// Parse the value of a `?sort=` query parameter, case-insensitively.
    ///
    /// `None` on anything unrecognized, so the route can answer **400** rather than silently
    /// falling back — the precedent is the blog's `SortOrder`, where `?sort=oldset` is an
    /// error instead of quietly meaning "newest".
    ///
    /// This is deliberately the *only* mapping from strings to variants. `SortOrder` gets its
    /// from a `Deserialize` derive; adding both here would be two places answering "what does
    /// this string mean", which is the hazard this file keeps designing against.
    /// The canonical wire name, so a response can echo back which sort it actually applied
    /// rather than the user's raw input (which may differ in case or whitespace, or be absent
    /// entirely when the default applied).
    ///
    /// This is the inverse of [`SortBy::parse`], **not** a second parser — the hazard this
    /// module keeps designing against is two places mapping strings *to* variants, which is
    /// why there is still no `Deserialize` derive here. `sort_names_round_trip` pins the two
    /// against each other over every variant, so adding a variant to one and forgetting the
    /// other fails a test rather than silently mislabelling a response.
    pub fn as_str(self) -> &'static str {
        match self {
            SortBy::Composite => "composite",
            SortBy::Pay => "pay",
            SortBy::Posted => "posted",
            SortBy::Deadline => "deadline",
            SortBy::Prestige => "prestige",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "composite" => Some(SortBy::Composite),
            "pay" => Some(SortBy::Pay),
            "posted" => Some(SortBy::Posted),
            "deadline" => Some(SortBy::Deadline),
            "prestige" => Some(SortBy::Prestige),
            _ => None,
        }
    }
}

/// Sort tier for a known, orderable value. Lower tiers come first.
const TIER_KNOWN: u8 = 0;
/// Sort tier for a deadline that has already gone by: known, but not actionable.
const TIER_DEADLINE_PASSED: u8 = 1;
/// Sort tier for a value the source never gave. Always last, under every sort.
const TIER_UNKNOWN: u8 = 2;

/// What one posting sorts on: a tier first, then a value within it, higher first.
///
/// Every sort — composite and single-axis alike — goes through this one key and one
/// comparator, so a new variant cannot arrive with its own tie-break, or without one.
#[derive(Debug, Clone, Copy, PartialEq)]
struct AxisKey {
    tier: u8,
    value: f64,
}

impl AxisKey {
    fn known(value: f64) -> Self {
        AxisKey {
            tier: TIER_KNOWN,
            value,
        }
    }

    /// Every member of this tier shares one value, so the id tie-break decides among them.
    fn unknown() -> Self {
        AxisKey {
            tier: TIER_UNKNOWN,
            value: 0.0,
        }
    }
}

/// The key one posting sorts on under `sort`.
///
/// The single-axis variants read the **raw field**, not the composite's sub-score: `pay_scale`
/// saturates and the recency decay caps estimated dates, both of which flatten distinct values
/// into ties that are fine for scoring and wrong for ordering.
fn axis_key(sort: SortBy, ranked: &RankedPosting, now: DateTime<Utc>) -> AxisKey {
    match sort {
        // The composite is the one sort that imputes, because that is what the composite is
        // for: every posting has a score, so every posting is in the known tier.
        SortBy::Composite => AxisKey::known(ranked.score),
        SortBy::Pay => match pay_knowledge(&ranked.posting) {
            // An inherited company median is deliberately not enough here. It is evidence
            // about the company, not this posting's figure.
            PayKnowledge::Known(hourly_usd) => AxisKey::known(hourly_usd),
            PayKnowledge::NotComparable | PayKnowledge::Absent => AxisKey::unknown(),
        },
        // An estimated date is an observation, not an absence, so it sorts as known. The
        // composite still refuses to let it buy freshness; this only orders by the date.
        SortBy::Posted => match ranked.posting.posted_at {
            Some(posted_at) => AxisKey::known(posted_at.timestamp() as f64),
            None => AxisKey::unknown(),
        },
        SortBy::Deadline => match ranked.posting.deadline {
            None => AxisKey::unknown(),
            Some(deadline) => {
                let days_left = days_between(deadline, now);
                if deadline_has_passed(days_left) {
                    // Behind every live deadline, ahead of the unknowns, most recent first.
                    AxisKey {
                        tier: TIER_DEADLINE_PASSED,
                        value: days_left,
                    }
                } else {
                    // Soonest first, and the key is "higher first" everywhere, so negate.
                    AxisKey::known(-days_left)
                }
            }
        },
        // `basis` is what says whether the published value is real or imputed, so the sort
        // reads it rather than re-deriving the same question from `signals`.
        SortBy::Prestige => match ranked.breakdown.prestige.basis {
            ScoreBasis::Stated => AxisKey::known(ranked.breakdown.prestige.value),
            _ => AxisKey::unknown(),
        },
    }
}

/// Filter, score and order internship postings.
///
/// `signals` is keyed by [`Posting::company_key`]; a posting whose company has no entry is
/// treated exactly like one whose entry has `prestige: None` — unknown, not bad.
///
/// `sort` changes only the order. Every posting is scored the same way under all of them, so
/// [`RankedPosting::breakdown`] is available whichever axis is on screen.
///
/// `now` is a parameter rather than a `Utc::now()` call inside, so that recency and deadline
/// proximity are deterministic under test.
///
/// The three passes below are separate on purpose. See the module doc.
pub fn rank_postings(
    postings: &[Posting],
    signals: &HashMap<String, CompanySignals>,
    filters: &InternshipFilters,
    sort: SortBy,
    now: DateTime<Utc>,
) -> Vec<RankedPosting> {
    // Pass 1 — hard filters. Pure exclusion; no score exists yet.
    //
    // `is_live` is the one filter the user does not supply. Expiry is a soft delete, so an
    // expired row is still a perfectly well-formed `Posting`; the query layer's partial index
    // normally excludes them, and this makes a forgotten `WHERE expired_at IS NULL` fail
    // safely instead of surfacing a closed posting at the top of the list.
    let survivors: Vec<&Posting> = postings
        .iter()
        .filter(|posting| posting.is_live() && filters.admits(posting))
        .collect();

    // Pass 2 — scoring. Note what is *not* passed in: `filters`. A hard filter cannot become
    // a scoring input here because the scorer cannot see one.
    let ranked: Vec<RankedPosting> = survivors
        .into_iter()
        .map(|posting| score_posting(posting, signals.get(&posting.company_key), now))
        .collect();

    // Pass 3 — deterministic order. `total_cmp` because `f64` has no `Ord`, and the id
    // tie-break because two identical requests must return an identical list.
    //
    // Keyed up front rather than recomputed inside the comparator, so the key a posting sorts
    // on cannot depend on what it is being compared against.
    let mut keyed: Vec<(AxisKey, RankedPosting)> = ranked
        .into_iter()
        .map(|ranked| (axis_key(sort, &ranked, now), ranked))
        .collect();
    keyed.sort_by(|(left_key, left), (right_key, right)| {
        left_key
            .tier
            .cmp(&right_key.tier)
            .then_with(|| right_key.value.total_cmp(&left_key.value))
            .then_with(|| left.posting.id.cmp(&right.posting.id))
    });
    keyed.into_iter().map(|(_, ranked)| ranked).collect()
}

/// Score one posting. Takes no filters — see [`rank_postings`].
pub fn score_posting(
    posting: &Posting,
    signals: Option<&CompanySignals>,
    now: DateTime<Utc>,
) -> RankedPosting {
    let breakdown = ScoreBreakdown {
        pay: pay_input(posting, signals),
        recency: recency_input(posting, now),
        deadline: deadline_input(posting, now),
        location: location_input(&posting.location),
        prestige: prestige_input(signals),
    };
    RankedPosting {
        posting: posting.clone(),
        score: breakdown.total(),
        breakdown,
    }
}

// ------------------------------------------------------------------------------------------
// Per-input scoring
// ------------------------------------------------------------------------------------------

/// What is known about a posting's pay, in the one unit this module compares in.
///
/// Three states, and the third is the point: a figure we hold but cannot convert is not the
/// same as no figure at all, and reporting them identically is how a EUR posting silently
/// gets ranked as though its number were dollars.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PayKnowledge {
    Known(f64),
    /// Stated, but not in [`PAY_REFERENCE_CURRENCY`], and this module has no FX table. The
    /// `PayRange` type exists so `45` per hour is never compared against `52000` per year;
    /// comparing 45 EUR against 45 USD is the same mistake one field over.
    NotComparable,
    Absent,
}

/// The single definition of "what does this posting pay, hourly, in USD".
///
/// Used by both the pay floor filter and the pay score, so the two cannot drift into
/// disagreeing about which postings clear a threshold.
fn pay_knowledge(posting: &Posting) -> PayKnowledge {
    let Some(pay) = &posting.pay else {
        return PayKnowledge::Absent;
    };
    if !pay.currency.eq_ignore_ascii_case(PAY_REFERENCE_CURRENCY) {
        return PayKnowledge::NotComparable;
    }
    let hourly = hourly_from(pay);
    if hourly.is_finite() {
        PayKnowledge::Known(hourly)
    } else {
        PayKnowledge::NotComparable
    }
}

/// Bring a pay range into hourly. `PayRange` guarantees the period is present, which is the
/// whole reason this conversion is safe to write.
fn hourly_from(pay: &PayRange) -> f64 {
    let midpoint = pay.midpoint();
    match pay.period {
        PayPeriod::Hour => midpoint,
        PayPeriod::Month => midpoint / WORK_HOURS_PER_MONTH,
        PayPeriod::Year => midpoint / WORK_HOURS_PER_YEAR,
    }
}

/// Map hourly USD onto the sub-score scale, linearly between the floor and the derived
/// ceiling. Saturates at both ends rather than rewarding an outlier without limit.
fn pay_scale(hourly_usd: f64) -> f64 {
    let span = PAY_SCALE_CEILING_HOURLY_USD - PAY_SCALE_FLOOR_HOURLY_USD;
    ((hourly_usd - PAY_SCALE_FLOOR_HOURLY_USD) / span).clamp(SCORE_MIN, SCORE_MAX)
}

/// Pull the company's observed median pay, if there is enough of it to stand in for a
/// posting's missing figure.
fn usable_company_median(signals: Option<&CompanySignals>) -> Option<f64> {
    let signals = signals?;
    if signals.pay_observations < PAY_IMPUTATION_MIN_OBSERVATIONS {
        return None;
    }
    let median = signals.median_pay_hourly_usd?;
    if median.is_finite() && median >= 0.0 {
        Some(median)
    } else {
        None
    }
}

/// Shrink an inferred value toward neutral. An imputed figure should move the ranking, but
/// never as far as a stated one.
fn shrink_toward_neutral(value: f64) -> f64 {
    UNKNOWN_SCORE + (value - UNKNOWN_SCORE) * PAY_IMPUTATION_CONFIDENCE
}

fn pay_input(posting: &Posting, signals: Option<&CompanySignals>) -> InputScore {
    match pay_knowledge(posting) {
        PayKnowledge::Known(hourly_usd) => {
            InputScore::new(pay_scale(hourly_usd), WEIGHT_PAY, ScoreBasis::Stated)
        }
        // Deliberately no median imputation here: we hold a figure and cannot convert it, and
        // laying a second guess over a known unknown makes the result less inspectable, not
        // more.
        PayKnowledge::NotComparable => {
            InputScore::new(UNKNOWN_SCORE, WEIGHT_PAY, ScoreBasis::NotComparable)
        }
        PayKnowledge::Absent => match usable_company_median(signals) {
            Some(median) => InputScore::new(
                shrink_toward_neutral(pay_scale(median)),
                WEIGHT_PAY,
                ScoreBasis::ImputedFromCompanyMedian,
            ),
            None => InputScore::new(UNKNOWN_SCORE, WEIGHT_PAY, ScoreBasis::ImputedNeutral),
        },
    }
}

/// Days from `earlier` to `later`, fractional. Negative when `later` precedes `earlier`.
fn days_between(later: DateTime<Utc>, earlier: DateTime<Utc>) -> f64 {
    (later - earlier).as_seconds_f64() / Duration::days(1).as_seconds_f64()
}

fn recency_input(posting: &Posting, now: DateTime<Utc>) -> InputScore {
    let Some(posted_at) = posting.posted_at else {
        return InputScore::new(UNKNOWN_SCORE, WEIGHT_RECENCY, ScoreBasis::ImputedNeutral);
    };
    // A posting dated in the future is not fresher than one dated now.
    let age_days = days_between(now, posted_at).max(0.0);
    // `powf`, not `^` — `^` is XOR, which on integers compiles and computes nonsense.
    let decay = 0.5_f64.powf(age_days / POSTED_HALFLIFE_DAYS);
    if posting.posted_at_is_estimated {
        // The estimate is a lower bound on age, so `decay` is an upper bound on freshness.
        // It may show a posting to be old; it may never show one to be fresh.
        InputScore::new(
            decay.min(UNKNOWN_SCORE),
            WEIGHT_RECENCY,
            ScoreBasis::EstimatedCapped,
        )
    } else {
        InputScore::new(decay, WEIGHT_RECENCY, ScoreBasis::Stated)
    }
}

fn deadline_input(posting: &Posting, now: DateTime<Utc>) -> InputScore {
    let Some(deadline) = posting.deadline else {
        // Most postings have no deadline field at all. Absence is not "closes now" and not
        // "never closes" — it is unknown. The documented consequence: on this input alone a
        // posting with no stated deadline outranks one closing in six months.
        return InputScore::new(UNKNOWN_SCORE, WEIGHT_DEADLINE, ScoreBasis::ImputedNeutral);
    };
    let days_left = days_between(deadline, now);
    let value = if deadline_has_passed(days_left) {
        DEADLINE_PASSED_SCORE
    } else if days_left <= DEADLINE_IMMINENT_DAYS {
        SCORE_MAX
    } else if days_left >= DEADLINE_HORIZON_DAYS {
        DEADLINE_DISTANT_SCORE
    } else {
        let ramp =
            (days_left - DEADLINE_IMMINENT_DAYS) / (DEADLINE_HORIZON_DAYS - DEADLINE_IMMINENT_DAYS);
        SCORE_MAX - ramp * (SCORE_MAX - DEADLINE_DISTANT_SCORE)
    };
    InputScore::new(value, WEIGHT_DEADLINE, ScoreBasis::Stated)
}

/// Whether a stated deadline has gone by, given its distance from `now` in days.
///
/// Threshold: a deadline falling exactly at `now` is still **open**. Shared by the deadline
/// score and [`SortBy::Deadline`] so the two cannot disagree about which side of the boundary
/// a posting is on.
fn deadline_has_passed(days_left: f64) -> bool {
    days_left < 0.0
}

fn location_input(location: &Location) -> InputScore {
    match location.is_remote {
        Some(true) => InputScore::new(LOCATION_REMOTE_SCORE, WEIGHT_LOCATION, ScoreBasis::Stated),
        Some(false) => InputScore::new(LOCATION_ONSITE_SCORE, WEIGHT_LOCATION, ScoreBasis::Stated),
        // Exactly between the two known states, which is what "the source didn't say" means.
        None => InputScore::new(UNKNOWN_SCORE, WEIGHT_LOCATION, ScoreBasis::ImputedNeutral),
    }
}

fn prestige_input(signals: Option<&CompanySignals>) -> InputScore {
    // A company with no `company_signals` row at all is the same as one whose row says
    // `None`: not enough evidence. Not a company we know to be bad.
    match signals.and_then(|s| s.prestige) {
        Some(prestige) if prestige.is_finite() => InputScore::new(
            prestige.clamp(SCORE_MIN, SCORE_MAX),
            WEIGHT_PRESTIGE,
            ScoreBasis::Stated,
        ),
        _ => InputScore::new(UNKNOWN_SCORE, WEIGHT_PRESTIGE, ScoreBasis::ImputedNeutral),
    }
}

/// Whether a class-year range states no restriction at all.
///
/// This is exactly the case [`ClassYearRange::admits`] answers as an unconditional keep, and
/// the only case [`ClassYearFilter::on_unknown`] is allowed to override. Note a range whose
/// `raw` text is present but parsed to nothing is unstated too — text we failed to parse is
/// not a restriction we know about.
fn class_year_is_unstated(range: &ClassYearRange) -> bool {
    range.min.is_none() && range.max.is_none()
}

/// Everything a location filter's text can match against, lowercased and joined.
///
/// `None` when the source gave no location text at all, which is the unknown case the
/// filter's [`OnUnknown`] policy decides.
fn location_haystack(location: &Location) -> Option<String> {
    let parts: Vec<String> = [
        location.raw.as_deref(),
        location.city.as_deref(),
        location.region.as_deref(),
        location.country.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(|part| part.trim().to_lowercase())
    .filter(|part| !part.is_empty())
    .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // --- fixtures -------------------------------------------------------------------------

    fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 12, 0, 0).unwrap()
    }

    fn now() -> DateTime<Utc> {
        at(2026, 8, 20)
    }

    /// A posting about which nothing is known but its identity. Every test starts here and
    /// changes exactly the field it is about, so a difference in the ranking has one cause.
    fn blank(id: &str) -> Posting {
        Posting {
            id: id.to_string(),
            dedup_key: format!("key-{id}"),
            company_key: "acme".to_string(),
            company_name: "Acme".to_string(),
            title: "SWE Intern".to_string(),
            canonical_url: format!("https://example.test/{id}"),
            term_season: None,
            term_year: None,
            location: Location::default(),
            pay: None,
            pay_raw: None,
            class_years: ClassYearRange::default(),
            posted_at: None,
            posted_at_is_estimated: false,
            deadline: None,
            first_seen_at: now(),
            last_seen_at: now(),
            expired_at: None,
            expiry_reason: None,
        }
    }

    fn usd(min: f64, max: Option<f64>, period: PayPeriod) -> Option<PayRange> {
        Some(PayRange {
            min,
            max,
            currency: "USD".to_string(),
            period,
        })
    }

    fn hourly(rate: f64) -> Option<PayRange> {
        usd(rate, None, PayPeriod::Hour)
    }

    fn days_from_now(days: f64) -> DateTime<Utc> {
        now() + Duration::seconds((days * Duration::days(1).as_seconds_f64()) as i64)
    }

    fn signals(company_key: &str, prestige: Option<f64>) -> CompanySignals {
        CompanySignals {
            company_key: company_key.to_string(),
            company_name: company_key.to_string(),
            distinct_sources: 1,
            live_postings: 1,
            total_postings_seen: 1,
            pay_observations: 0,
            median_pay_hourly_usd: None,
            prestige,
        }
    }

    fn signal_map(rows: Vec<CompanySignals>) -> HashMap<String, CompanySignals> {
        rows.into_iter()
            .map(|row| (row.company_key.clone(), row))
            .collect()
    }

    fn no_signals() -> HashMap<String, CompanySignals> {
        HashMap::new()
    }

    fn ids(ranked: &[RankedPosting]) -> Vec<String> {
        ranked.iter().map(|r| r.posting.id.clone()).collect()
    }

    fn filters_admit(posting: &Posting, grad_year: i64, on_unknown: OnUnknown) -> bool {
        class_year_filter(grad_year, on_unknown).admits(posting)
    }

    fn rank(postings: &[Posting]) -> Vec<RankedPosting> {
        rank_postings(
            postings,
            &no_signals(),
            &InternshipFilters::default(),
            SortBy::Composite,
            now(),
        )
    }

    fn score_of(posting: &Posting) -> f64 {
        score_posting(posting, None, now()).score
    }

    fn breakdown_of(posting: &Posting) -> ScoreBreakdown {
        score_posting(posting, None, now()).breakdown
    }

    const EPSILON: f64 = 1e-9;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < EPSILON
    }

    /// Guards against the vacuous pass this repo keeps getting bitten by: assert that two
    /// fixtures differ on **exactly** the input the test is about, so an ordering assertion
    /// cannot be quietly satisfied by some other field that also happened to differ.
    fn assert_differs_only_on(a: &Posting, b: &Posting, expected: ScoreInput) {
        let (left, right) = (breakdown_of(a), breakdown_of(b));
        let differing: Vec<ScoreInput> = left
            .inputs()
            .iter()
            .zip(right.inputs().iter())
            .filter(|((_, l), (_, r))| !close(l.value, r.value))
            .map(|((name, _), _)| *name)
            .collect();
        assert_eq!(
            differing,
            vec![expected],
            "fixtures must differ on exactly one input, or the ordering assertion proves \
             nothing about {expected:?}"
        );
    }

    // --- structural invariants ------------------------------------------------------------

    #[test]
    fn the_weights_sum_to_one() {
        let total =
            WEIGHT_PAY + WEIGHT_RECENCY + WEIGHT_DEADLINE + WEIGHT_LOCATION + WEIGHT_PRESTIGE;
        assert!(
            close(total, 1.0),
            "weights must sum to 1.0 so a composite score stays on its own scale, got {total}"
        );
    }

    #[test]
    fn the_score_is_the_sum_of_the_published_breakdown() {
        let mut posting = blank("p1");
        posting.pay = hourly(55.0);
        posting.posted_at = Some(days_from_now(-10.0));
        posting.deadline = Some(days_from_now(20.0));
        posting.location.is_remote = Some(true);
        let ranked = score_posting(&posting, Some(&signals("acme", Some(0.9))), now());
        let summed: f64 = ranked
            .breakdown
            .inputs()
            .iter()
            .map(|(_, input)| input.contribution)
            .sum();
        assert!(
            close(ranked.score, summed),
            "score {} must be decomposable into its breakdown {summed}",
            ranked.score
        );
        for (name, input) in ranked.breakdown.inputs() {
            assert!(
                close(input.contribution, input.value * input.weight),
                "{name:?} contribution must be value * weight"
            );
            assert_eq!(
                ranked.breakdown.get(name),
                input,
                "{name:?} must be reachable by name as well as by field"
            );
        }
    }

    #[test]
    fn a_posting_we_know_nothing_about_scores_neutral() {
        // The whole absent-data policy in one number: every input imputed to its neutral, and
        // the weights summing to 1.0, put a wholly-unknown posting dead centre.
        assert!(close(score_of(&blank("p1")), UNKNOWN_SCORE));
        for (name, input) in breakdown_of(&blank("p1")).inputs() {
            assert_eq!(
                input.basis,
                ScoreBasis::ImputedNeutral,
                "{name:?} should report why it was imputed"
            );
        }
    }

    // --- the imputations, stated as equivalences ------------------------------------------

    #[test]
    fn an_unknown_pay_posting_scores_exactly_like_one_paying_the_neutral_wage() {
        let unknown = blank("p1");
        let mut stated = blank("p2");
        stated.pay = hourly(PAY_NEUTRAL_HOURLY_USD);
        assert!(close(
            breakdown_of(&unknown).pay.value,
            breakdown_of(&stated).pay.value
        ));
        assert_eq!(breakdown_of(&unknown).pay.value, UNKNOWN_SCORE);
    }

    #[test]
    fn an_unknown_posted_date_scores_exactly_like_one_a_halflife_old() {
        let unknown = blank("p1");
        let mut stated = blank("p2");
        stated.posted_at = Some(days_from_now(-POSTED_HALFLIFE_DAYS));
        assert!(close(
            breakdown_of(&unknown).recency.value,
            breakdown_of(&stated).recency.value
        ));
    }

    #[test]
    fn an_unknown_deadline_scores_exactly_like_one_midway_down_the_ramp() {
        let midpoint_days = (DEADLINE_IMMINENT_DAYS + DEADLINE_HORIZON_DAYS) / 2.0;
        let unknown = blank("p1");
        let mut stated = blank("p2");
        stated.deadline = Some(days_from_now(midpoint_days));
        assert!(close(
            breakdown_of(&unknown).deadline.value,
            breakdown_of(&stated).deadline.value
        ));
    }

    #[test]
    fn an_unknown_location_scores_exactly_between_remote_and_onsite() {
        let mut remote = blank("p1");
        remote.location.is_remote = Some(true);
        let mut onsite = blank("p2");
        onsite.location.is_remote = Some(false);
        let unknown = blank("p3");
        let midpoint =
            (breakdown_of(&remote).location.value + breakdown_of(&onsite).location.value) / 2.0;
        assert!(close(breakdown_of(&unknown).location.value, midpoint));
    }

    // --- the headline requirement ---------------------------------------------------------

    #[test]
    fn an_unknown_pay_posting_ranks_neither_first_nor_last() {
        let mut rich = blank("z-rich");
        rich.pay = hourly(PAY_SCALE_CEILING_HOURLY_USD);
        let mut poor = blank("a-poor");
        poor.pay = hourly(PAY_SCALE_FLOOR_HOURLY_USD);
        let unknown = blank("m-unknown");

        assert_differs_only_on(&rich, &unknown, ScoreInput::Pay);
        assert_differs_only_on(&poor, &unknown, ScoreInput::Pay);

        let ranked = rank(&[poor, unknown, rich]);
        assert_eq!(
            ids(&ranked),
            vec!["z-rich", "m-unknown", "a-poor"],
            "unknown pay must be neither the best nor the worst reading of absence"
        );
    }

    #[test]
    fn an_unknown_pay_posting_is_not_treated_as_paying_zero() {
        let unknown = blank("p1");
        let mut free_labour = blank("p2");
        free_labour.pay = hourly(0.0);
        assert!(
            breakdown_of(&unknown).pay.value > breakdown_of(&free_labour).pay.value,
            "absent pay must not read as free labour"
        );
    }

    #[test]
    fn an_unknown_pay_posting_is_not_treated_as_paying_best() {
        let unknown = blank("p1");
        let mut top = blank("p2");
        top.pay = hourly(PAY_SCALE_CEILING_HOURLY_USD * 2.0);
        assert!(
            breakdown_of(&unknown).pay.value < breakdown_of(&top).pay.value,
            "absent pay must not read as top of market either — the failure introduced while \
             fixing the previous test"
        );
    }

    // --- pay: units, currency, imputation -------------------------------------------------

    #[test]
    fn pay_periods_are_normalized_to_one_comparable_unit() {
        let mut per_hour = blank("p1");
        per_hour.pay = hourly(50.0);
        let mut per_year = blank("p2");
        per_year.pay = usd(50.0 * WORK_HOURS_PER_YEAR, None, PayPeriod::Year);
        let mut per_month = blank("p3");
        per_month.pay = usd(50.0 * WORK_HOURS_PER_MONTH, None, PayPeriod::Month);
        assert!(close(
            breakdown_of(&per_hour).pay.value,
            breakdown_of(&per_year).pay.value
        ));
        assert!(close(
            breakdown_of(&per_hour).pay.value,
            breakdown_of(&per_month).pay.value
        ));
    }

    #[test]
    fn a_range_is_compared_on_its_midpoint() {
        let mut range = blank("p1");
        range.pay = usd(30.0, Some(50.0), PayPeriod::Hour);
        let mut point = blank("p2");
        point.pay = hourly(40.0);
        assert!(close(
            breakdown_of(&range).pay.value,
            breakdown_of(&point).pay.value
        ));
    }

    #[test]
    fn pay_in_another_currency_is_unknown_rather_than_mis_scaled() {
        let mut euros = blank("p1");
        euros.pay = Some(PayRange {
            min: 45.0,
            max: None,
            currency: "EUR".to_string(),
            period: PayPeriod::Hour,
        });
        let scored = breakdown_of(&euros).pay;
        assert_eq!(scored.basis, ScoreBasis::NotComparable);
        assert_eq!(
            scored.value, UNKNOWN_SCORE,
            "an unconvertible figure must not be read as though it were dollars"
        );
    }

    #[test]
    fn an_unknown_pay_posting_inherits_its_companys_median_when_there_is_enough_of_it() {
        let posting = blank("p1");
        let mut row = signals("acme", None);
        row.pay_observations = PAY_IMPUTATION_MIN_OBSERVATIONS;
        row.median_pay_hourly_usd = Some(PAY_SCALE_CEILING_HOURLY_USD);

        let imputed = score_posting(&posting, Some(&row), now()).breakdown.pay;
        assert_eq!(imputed.basis, ScoreBasis::ImputedFromCompanyMedian);
        assert!(
            imputed.value > UNKNOWN_SCORE,
            "evidence from the company's other postings should move the score"
        );
    }

    #[test]
    fn the_company_median_needs_exactly_the_named_number_of_observations() {
        // Land on the boundary, not comfortably either side of it.
        let posting = blank("p1");

        let mut enough = signals("acme", None);
        enough.pay_observations = PAY_IMPUTATION_MIN_OBSERVATIONS;
        enough.median_pay_hourly_usd = Some(PAY_SCALE_CEILING_HOURLY_USD);
        assert_eq!(
            score_posting(&posting, Some(&enough), now())
                .breakdown
                .pay
                .basis,
            ScoreBasis::ImputedFromCompanyMedian,
            "exactly the minimum must be enough — the Phase 4 `>` vs `>=` bug"
        );

        let mut one_short = enough.clone();
        one_short.pay_observations = PAY_IMPUTATION_MIN_OBSERVATIONS - 1;
        assert_eq!(
            score_posting(&posting, Some(&one_short), now())
                .breakdown
                .pay
                .basis,
            ScoreBasis::ImputedNeutral,
            "one short must fall back to neutral"
        );
    }

    #[test]
    fn an_inherited_median_never_outranks_a_stated_figure() {
        let unknown_at_rich_company = blank("p1");
        let mut row = signals("acme", None);
        row.pay_observations = PAY_IMPUTATION_MIN_OBSERVATIONS;
        row.median_pay_hourly_usd = Some(PAY_SCALE_CEILING_HOURLY_USD);

        let mut states_the_same = blank("p2");
        states_the_same.pay = hourly(PAY_SCALE_CEILING_HOURLY_USD);

        let imputed = score_posting(&unknown_at_rich_company, Some(&row), now())
            .breakdown
            .pay
            .value;
        let stated = score_posting(&states_the_same, Some(&row), now())
            .breakdown
            .pay
            .value;
        assert!(
            imputed < stated,
            "inherited evidence is weaker than stated evidence and must be shrunk toward neutral"
        );
        assert!(
            imputed > UNKNOWN_SCORE,
            "but it must still count for something"
        );
    }

    // --- recency --------------------------------------------------------------------------

    #[test]
    fn a_fresher_posting_outranks_an_older_one_on_recency_alone() {
        let mut fresh = blank("z-fresh");
        fresh.posted_at = Some(days_from_now(-1.0));
        let mut stale = blank("a-stale");
        stale.posted_at = Some(days_from_now(-200.0));
        assert_differs_only_on(&fresh, &stale, ScoreInput::Recency);
        assert_eq!(ids(&rank(&[stale, fresh])), vec!["z-fresh", "a-stale"]);
    }

    #[test]
    fn an_estimated_posted_date_cannot_buy_freshness() {
        // The cold-start trap: every backfilled posting dated "today" would otherwise tie at
        // maximum freshness, which is worse than having no recency signal at all.
        let mut estimated = blank("a-estimated");
        estimated.posted_at = Some(now());
        estimated.posted_at_is_estimated = true;
        let mut stated = blank("z-stated");
        stated.posted_at = Some(now());

        let estimated_score = breakdown_of(&estimated).recency;
        assert_eq!(estimated_score.basis, ScoreBasis::EstimatedCapped);
        assert_eq!(estimated_score.value, UNKNOWN_SCORE);
        assert!(breakdown_of(&stated).recency.value > estimated_score.value);
        assert_eq!(
            ids(&rank(&[estimated, stated])),
            vec!["z-stated", "a-estimated"]
        );
    }

    #[test]
    fn an_estimated_posted_date_can_still_show_a_posting_to_be_old() {
        // The cap is one-sided: an estimate is a lower bound on age, so a long-observed
        // posting is genuinely old and must be allowed to decay below neutral.
        let mut ancient = blank("p1");
        ancient.posted_at = Some(days_from_now(-365.0));
        ancient.posted_at_is_estimated = true;
        assert!(breakdown_of(&ancient).recency.value < UNKNOWN_SCORE);
    }

    #[test]
    fn a_posting_dated_in_the_future_is_not_fresher_than_one_dated_now() {
        let mut future = blank("p1");
        future.posted_at = Some(days_from_now(30.0));
        let mut present = blank("p2");
        present.posted_at = Some(now());
        assert!(close(
            breakdown_of(&future).recency.value,
            breakdown_of(&present).recency.value
        ));
    }

    // --- deadline -------------------------------------------------------------------------

    #[test]
    fn a_closer_deadline_outranks_a_distant_one() {
        let mut soon = blank("z-soon");
        soon.deadline = Some(days_from_now(3.0));
        let mut later = blank("a-later");
        later.deadline = Some(days_from_now(60.0));
        assert_differs_only_on(&soon, &later, ScoreInput::Deadline);
        assert_eq!(ids(&rank(&[later, soon])), vec!["z-soon", "a-later"]);
    }

    #[test]
    fn a_deadline_falling_exactly_now_is_still_open() {
        let mut exactly_now = blank("p1");
        exactly_now.deadline = Some(now());
        let scored = breakdown_of(&exactly_now).deadline;
        assert_eq!(
            scored.value, SCORE_MAX,
            "the boundary itself must be open — `>=`, not `>`"
        );

        let mut just_passed = blank("p2");
        just_passed.deadline = Some(days_from_now(-0.001));
        assert_eq!(
            breakdown_of(&just_passed).deadline.value,
            DEADLINE_PASSED_SCORE
        );
    }

    #[test]
    fn the_urgency_ramp_boundaries_land_on_their_constants() {
        let mut imminent = blank("p1");
        imminent.deadline = Some(days_from_now(DEADLINE_IMMINENT_DAYS));
        assert!(
            close(breakdown_of(&imminent).deadline.value, SCORE_MAX),
            "exactly the imminent threshold is still maximally urgent"
        );

        let mut horizon = blank("p2");
        horizon.deadline = Some(days_from_now(DEADLINE_HORIZON_DAYS));
        assert!(
            close(
                breakdown_of(&horizon).deadline.value,
                DEADLINE_DISTANT_SCORE
            ),
            "exactly the horizon is already distant"
        );

        let mut inside = blank("p3");
        inside.deadline = Some(days_from_now(DEADLINE_IMMINENT_DAYS + 0.001));
        assert!(breakdown_of(&inside).deadline.value < SCORE_MAX);
    }

    // --- location, class year, prestige ---------------------------------------------------

    #[test]
    fn a_remote_posting_outranks_an_onsite_one_on_location_alone() {
        let mut remote = blank("z-remote");
        remote.location.is_remote = Some(true);
        let mut onsite = blank("a-onsite");
        onsite.location.is_remote = Some(false);
        assert_differs_only_on(&remote, &onsite, ScoreInput::Location);
        assert_eq!(ids(&rank(&[onsite, remote])), vec!["z-remote", "a-onsite"]);
    }

    #[test]
    fn a_prestigious_company_outranks_an_unremarkable_one() {
        let mut high = blank("z-high");
        high.company_key = "high".to_string();
        let mut low = blank("a-low");
        low.company_key = "low".to_string();
        let map = signal_map(vec![signals("high", Some(1.0)), signals("low", Some(0.0))]);
        let ranked = rank_postings(
            &[low, high],
            &map,
            &InternshipFilters::default(),
            SortBy::Composite,
            now(),
        );
        assert_eq!(ids(&ranked), vec!["z-high", "a-low"]);
        assert!(
            close(ranked[0].score - ranked[1].score, WEIGHT_PRESTIGE),
            "the whole gap between them must be the prestige input's weight"
        );
    }

    #[test]
    fn an_unknown_prestige_ranks_between_the_known_ones() {
        let mut high = blank("z-high");
        high.company_key = "high".to_string();
        let mut unknown = blank("m-unknown");
        unknown.company_key = "unknown".to_string();
        let mut low = blank("a-low");
        low.company_key = "low".to_string();
        let map = signal_map(vec![
            signals("high", Some(1.0)),
            signals("unknown", None),
            signals("low", Some(0.0)),
        ]);
        let ranked = rank_postings(
            &[low, unknown, high],
            &map,
            &InternshipFilters::default(),
            SortBy::Composite,
            now(),
        );
        assert_eq!(ids(&ranked), vec!["z-high", "m-unknown", "a-low"]);
    }

    #[test]
    fn a_company_with_no_signals_row_is_unknown_not_worst() {
        let missing = blank("z-missing");
        let mut known_bad = blank("a-known-bad");
        known_bad.company_key = "bad".to_string();
        let map = signal_map(vec![signals("bad", Some(0.0))]);
        let ranked = rank_postings(
            &[missing, known_bad],
            &map,
            &InternshipFilters::default(),
            SortBy::Composite,
            now(),
        );
        assert_eq!(ranked[0].breakdown.prestige.value, UNKNOWN_SCORE);
        assert_eq!(ranked[0].posting.id, "z-missing");
    }

    #[test]
    fn an_out_of_range_prestige_is_clamped_rather_than_allowed_to_dominate() {
        let posting = blank("p1");
        let scored = score_posting(&posting, Some(&signals("acme", Some(87.0))), now());
        assert_eq!(scored.breakdown.prestige.value, SCORE_MAX);
        assert!(scored.score <= SCORE_MAX);

        let nonsense = score_posting(&posting, Some(&signals("acme", Some(f64::NAN))), now());
        assert_eq!(nonsense.breakdown.prestige.value, UNKNOWN_SCORE);
        assert_eq!(
            nonsense.breakdown.prestige.basis,
            ScoreBasis::ImputedNeutral
        );
    }

    // --- hard filters ---------------------------------------------------------------------

    #[test]
    fn a_remote_filter_excludes_rather_than_down_weights() {
        let mut remote = blank("p1");
        remote.location.is_remote = Some(true);
        let mut onsite = blank("p2");
        onsite.location.is_remote = Some(false);
        let mut unknown = blank("p3");
        unknown.location.raw = Some("Somewhere".to_string());

        let filters = InternshipFilters {
            location: Some(LocationFilter {
                remote: Some(true),
                contains: None,
                on_unknown: OnUnknown::Drop,
            }),
            ..InternshipFilters::default()
        };
        let ranked = rank_postings(
            &[remote, onsite, unknown],
            &no_signals(),
            &filters,
            SortBy::Composite,
            now(),
        );
        assert_eq!(
            ids(&ranked),
            vec!["p1"],
            "a filtered-out posting must be gone, not merely demoted"
        );
    }

    #[test]
    fn the_remote_filters_unknown_policy_is_the_callers_choice() {
        let mut unknown = blank("p1");
        unknown.location.raw = Some("Somewhere".to_string());
        let keep = InternshipFilters {
            location: Some(LocationFilter {
                remote: Some(true),
                contains: None,
                on_unknown: OnUnknown::Keep,
            }),
            ..InternshipFilters::default()
        };
        let drop = InternshipFilters {
            location: Some(LocationFilter {
                remote: Some(true),
                contains: None,
                on_unknown: OnUnknown::Drop,
            }),
            ..InternshipFilters::default()
        };
        let postings = vec![unknown];
        assert_eq!(
            rank_postings(&postings, &no_signals(), &keep, SortBy::Composite, now()).len(),
            1
        );
        assert_eq!(
            rank_postings(&postings, &no_signals(), &drop, SortBy::Composite, now()).len(),
            0
        );
    }

    #[test]
    fn a_filter_removes_postings_without_reordering_the_survivors() {
        // The Phase 3–4 rule, stated as a property: filtering deletes, it never re-weights.
        let mut remote_rich = blank("a");
        remote_rich.location.is_remote = Some(true);
        remote_rich.pay = hourly(60.0);
        let mut onsite_rich = blank("b");
        onsite_rich.location.is_remote = Some(false);
        onsite_rich.pay = hourly(65.0);
        let mut remote_poor = blank("c");
        remote_poor.location.is_remote = Some(true);
        remote_poor.pay = hourly(20.0);

        let all = vec![remote_rich, onsite_rich, remote_poor];
        let unfiltered = rank(&all);
        let filters = InternshipFilters {
            location: Some(LocationFilter {
                remote: Some(true),
                contains: None,
                on_unknown: OnUnknown::Drop,
            }),
            ..InternshipFilters::default()
        };
        let filtered = rank_postings(&all, &no_signals(), &filters, SortBy::Composite, now());

        let survivors: Vec<String> = ids(&filtered);
        let expected: Vec<String> = ids(&unfiltered)
            .into_iter()
            .filter(|id| survivors.contains(id))
            .collect();
        assert_eq!(survivors, expected);
        // And the surviving scores are untouched by the filter having been applied.
        for survivor in &filtered {
            let before = unfiltered
                .iter()
                .find(|r| r.posting.id == survivor.posting.id)
                .unwrap();
            assert_eq!(before.score, survivor.score);
        }
    }

    #[test]
    fn a_pay_floor_admits_a_posting_paying_exactly_the_floor() {
        let mut exactly = blank("p1");
        exactly.pay = hourly(30.0);
        let mut just_under = blank("p2");
        just_under.pay = hourly(29.99);
        let filters = InternshipFilters {
            pay: Some(PayRangeFilter::dropping_unknown(Some(30.0), None)),
            ..InternshipFilters::default()
        };
        let ranked = rank_postings(
            &[exactly, just_under],
            &no_signals(),
            &filters,
            SortBy::Composite,
            now(),
        );
        assert_eq!(
            ids(&ranked),
            vec!["p1"],
            "exactly the floor must pass — the Phase 4 4-star bug"
        );
    }

    #[test]
    fn a_pay_ceiling_admits_a_posting_paying_exactly_the_ceiling() {
        let mut exactly = blank("p1");
        exactly.pay = hourly(60.0);
        let mut just_over = blank("p2");
        just_over.pay = hourly(60.01);
        let filters = InternshipFilters {
            pay: Some(PayRangeFilter::dropping_unknown(None, Some(60.0))),
            ..InternshipFilters::default()
        };
        let ranked = rank_postings(
            &[exactly, just_over],
            &no_signals(),
            &filters,
            SortBy::Composite,
            now(),
        );
        assert_eq!(
            ids(&ranked),
            vec!["p1"],
            "both bounds are inclusive, not just the floor"
        );
    }

    #[test]
    fn a_pay_window_excludes_on_both_sides_and_admits_the_middle() {
        let mut under = blank("p1");
        under.pay = hourly(20.0);
        let mut inside = blank("p2");
        inside.pay = hourly(45.0);
        let mut over = blank("p3");
        over.pay = hourly(90.0);
        let filters = InternshipFilters {
            pay: Some(PayRangeFilter::dropping_unknown(Some(30.0), Some(60.0))),
            ..InternshipFilters::default()
        };
        let ranked = rank_postings(
            &[under, inside, over],
            &no_signals(),
            &filters,
            SortBy::Composite,
            now(),
        );
        assert_eq!(ids(&ranked), vec!["p2"]);
    }

    #[test]
    fn a_pay_filter_with_neither_bound_constrains_nothing() {
        // Both bounds absent is not a filter, so it must not reach the unknown policy and
        // quietly drop every posting whose pay is missing.
        let unknown = blank("p1");
        let mut stated = blank("p2");
        stated.pay = hourly(50.0);
        let filters = InternshipFilters {
            pay: Some(PayRangeFilter::dropping_unknown(None, None)),
            ..InternshipFilters::default()
        };
        let ranked = rank_postings(
            &[unknown, stated],
            &no_signals(),
            &filters,
            SortBy::Composite,
            now(),
        );
        // Membership is the claim here, not order — the stated figure outscores the imputed
        // one on the composite, which is the scoring pass doing its job.
        let mut survivors = ids(&ranked);
        survivors.sort();
        assert_eq!(survivors, vec!["p1", "p2"]);
    }

    #[test]
    fn the_pay_filters_unknown_policy_is_an_explicit_tri_state() {
        let unknown = blank("p1");
        let mut pays_enough = blank("p2");
        pays_enough.pay = hourly(50.0);
        let postings = vec![unknown, pays_enough];

        let no_floor = rank_postings(
            &postings,
            &no_signals(),
            &InternshipFilters::default(),
            SortBy::Composite,
            now(),
        );
        assert_eq!(no_floor.len(), 2, "state one: no pay filter at all");

        let keeping = InternshipFilters {
            pay: Some(PayRangeFilter::keeping_unknown(Some(30.0), None)),
            ..InternshipFilters::default()
        };
        assert_eq!(
            rank_postings(&postings, &no_signals(), &keeping, SortBy::Composite, now()).len(),
            2,
            "state two: a window that constrains only postings that state a figure"
        );

        let dropping = InternshipFilters {
            pay: Some(PayRangeFilter::dropping_unknown(Some(30.0), None)),
            ..InternshipFilters::default()
        };
        assert_eq!(
            ids(&rank_postings(
                &postings,
                &no_signals(),
                &dropping,
                SortBy::Composite,
                now()
            )),
            vec!["p2"],
            "state three: a window that requires a stated figure"
        );
    }

    #[test]
    fn a_pay_filter_treats_an_unconvertible_currency_as_unknown() {
        let mut euros = blank("p1");
        euros.pay = Some(PayRange {
            min: 500.0,
            max: None,
            currency: "EUR".to_string(),
            period: PayPeriod::Hour,
        });
        let postings = vec![euros];
        let keeping = InternshipFilters {
            pay: Some(PayRangeFilter::keeping_unknown(Some(30.0), None)),
            ..InternshipFilters::default()
        };
        let dropping = InternshipFilters {
            pay: Some(PayRangeFilter::dropping_unknown(Some(30.0), None)),
            ..InternshipFilters::default()
        };
        assert_eq!(
            rank_postings(&postings, &no_signals(), &keeping, SortBy::Composite, now()).len(),
            1,
            "a figure we cannot convert must not sail past a bound on its face value"
        );
        assert_eq!(
            rank_postings(
                &postings,
                &no_signals(),
                &dropping,
                SortBy::Composite,
                now()
            )
            .len(),
            0
        );
    }

    fn class_year_filter(grad_year: i64, on_unknown: OnUnknown) -> InternshipFilters {
        InternshipFilters {
            class_year: Some(ClassYearFilter {
                grad_year,
                on_unknown,
            }),
            ..InternshipFilters::default()
        }
    }

    #[test]
    fn the_class_year_unknown_policy_decides_the_unstated_case_both_ways() {
        // Per docs/INTERNSHIP_SCRAPING.md § B this is nearly the whole corpus, so this policy
        // is the filter's main effect rather than an edge case.
        let unstated = blank("p1");
        assert!(filters_admit(&unstated, 2027, OnUnknown::Keep));
        assert!(!filters_admit(&unstated, 2027, OnUnknown::Drop));
    }

    #[test]
    fn a_stated_range_ignores_the_unknown_policy_entirely() {
        let mut restricted = blank("p1");
        restricted.class_years = ClassYearRange {
            min: Some(2028),
            max: None,
            raw: None,
        };
        for policy in [OnUnknown::Keep, OnUnknown::Drop] {
            assert!(
                !filters_admit(&restricted, 2027, policy),
                "a stated range that excludes 2027 excludes it under {policy:?} too"
            );
            assert!(filters_admit(&restricted, 2028, policy));
        }
    }

    #[test]
    fn a_class_year_range_that_parsed_to_nothing_is_unstated() {
        // Text we failed to parse is not a restriction we know about, so it takes the unknown
        // policy rather than being treated as a bound.
        let mut unparsed = blank("p1");
        unparsed.class_years = ClassYearRange {
            min: None,
            max: None,
            raw: Some("rising senior".to_string()),
        };
        assert!(class_year_is_unstated(&unparsed.class_years));
        assert!(filters_admit(&unparsed, 2027, OnUnknown::Keep));
        assert!(!filters_admit(&unparsed, 2027, OnUnknown::Drop));
    }

    #[test]
    fn the_unknown_class_year_case_is_exactly_what_admits_waves_through() {
        // The coupling this file depends on: `class_year_is_unstated` intercepts precisely the
        // case `ClassYearRange::admits` answers as an unconditional keep, and nothing else. If
        // `admits` ever stops waving that case through, the interception becomes an override
        // of something rather than a delegation to it — and this fails.
        let unstated = ClassYearRange::default();
        for year in [1970, 2026, 2027, 2100] {
            assert!(
                unstated.admits(year),
                "an unbounded range must admit {year}, or the interception is hiding a real \
                 decision instead of standing in for a vacuous one"
            );
        }
        // And every partially-bounded range is NOT intercepted: it is `admits`'s to answer.
        let one_sided = ClassYearRange {
            min: Some(2028),
            max: None,
            raw: None,
        };
        assert!(!class_year_is_unstated(&one_sided));
        let other_side = ClassYearRange {
            min: None,
            max: Some(2028),
            raw: None,
        };
        assert!(!class_year_is_unstated(&other_side));
    }

    #[test]
    fn a_class_year_filter_lands_inclusively_on_both_bounds() {
        let mut posting = blank("p1");
        posting.class_years = ClassYearRange {
            min: Some(2026),
            max: Some(2028),
            raw: None,
        };
        for (year, admitted) in [(2025, false), (2026, true), (2028, true), (2029, false)] {
            assert_eq!(
                filters_admit(&posting, year, OnUnknown::Drop),
                admitted,
                "graduation year {year} against a 2026-2028 posting"
            );
        }
    }

    #[test]
    fn a_term_filter_matches_season_and_year_and_decides_unknowns_explicitly() {
        let mut summer_2027 = blank("p1");
        summer_2027.term_season = Some(Season::Summer);
        summer_2027.term_year = Some(2027);
        let mut fall_2027 = blank("p2");
        fall_2027.term_season = Some(Season::Fall);
        fall_2027.term_year = Some(2027);
        let unstated = blank("p3");
        let postings = vec![summer_2027, fall_2027, unstated];

        let dropping = InternshipFilters {
            term: Some(TermFilter {
                season: Some(Season::Summer),
                year: Some(2027),
                on_unknown: OnUnknown::Drop,
            }),
            ..InternshipFilters::default()
        };
        assert_eq!(
            ids(&rank_postings(
                &postings,
                &no_signals(),
                &dropping,
                SortBy::Composite,
                now()
            )),
            vec!["p1"]
        );

        let keeping = InternshipFilters {
            term: Some(TermFilter {
                season: Some(Season::Summer),
                year: Some(2027),
                on_unknown: OnUnknown::Keep,
            }),
            ..InternshipFilters::default()
        };
        assert_eq!(
            ids(&rank_postings(
                &postings,
                &no_signals(),
                &keeping,
                SortBy::Composite,
                now()
            )),
            vec!["p1", "p3"],
            "a posting whose term the source never stated is kept when the caller says so"
        );
    }

    #[test]
    fn a_location_text_filter_matches_any_of_the_parsed_parts_case_insensitively() {
        let mut city = blank("p1");
        city.location.city = Some("Seattle".to_string());
        let mut raw_only = blank("p2");
        raw_only.location.raw = Some("SEATTLE, WA (hybrid)".to_string());
        let mut elsewhere = blank("p3");
        elsewhere.location.city = Some("Austin".to_string());
        let no_text = blank("p4");

        let filters = InternshipFilters {
            location: Some(LocationFilter {
                remote: None,
                contains: Some("seattle".to_string()),
                on_unknown: OnUnknown::Drop,
            }),
            ..InternshipFilters::default()
        };
        let ranked = rank_postings(
            &[city, raw_only, elsewhere, no_text],
            &no_signals(),
            &filters,
            SortBy::Composite,
            now(),
        );
        assert_eq!(ids(&ranked), vec!["p1", "p2"]);
    }

    #[test]
    fn a_company_filter_matches_the_normalized_key() {
        let mut acme = blank("p1");
        acme.company_key = "acme".to_string();
        let mut other = blank("p2");
        other.company_key = "globex".to_string();
        let filters = InternshipFilters {
            company_key: Some("ACME".to_string()),
            ..InternshipFilters::default()
        };
        let ranked = rank_postings(
            &[acme, other],
            &no_signals(),
            &filters,
            SortBy::Composite,
            now(),
        );
        assert_eq!(ids(&ranked), vec!["p1"]);
    }

    #[test]
    fn an_expired_posting_is_never_ranked() {
        let live = blank("p1");
        let mut expired = blank("p2");
        expired.expired_at = Some(days_from_now(-1.0));
        expired.expiry_reason = Some(crate::internships::models::ExpiryReason::DeadlinePassed);
        assert_eq!(ids(&rank(&[live, expired])), vec!["p1"]);
    }

    // --- ordering -------------------------------------------------------------------------

    #[test]
    fn equal_scores_are_broken_by_id_so_the_list_never_reshuffles() {
        let first = blank("aaa");
        let second = blank("bbb");
        let third = blank("ccc");
        // Same score by construction: nothing is known about any of them.
        let ranked = rank(&[third.clone(), first.clone(), second.clone()]);
        assert_eq!(ids(&ranked), vec!["aaa", "bbb", "ccc"]);

        let again = rank(&[second, third, first]);
        assert_eq!(
            ids(&ranked),
            ids(&again),
            "two identical requests must return an identical list"
        );
    }

    #[test]
    fn the_ranking_is_sorted_descending_and_not_left_untouched() {
        // `b.total_cmp(&b)` compares a value with itself and makes the sort a silent no-op;
        // this pins that the output is actually ordered by score.
        let mut best = blank("z-best");
        best.pay = hourly(PAY_SCALE_CEILING_HOURLY_USD);
        best.location.is_remote = Some(true);
        best.posted_at = Some(now());
        let mut worst = blank("a-worst");
        worst.pay = hourly(0.0);
        worst.location.is_remote = Some(false);
        worst.posted_at = Some(days_from_now(-500.0));

        let ranked = rank(&[worst, best]);
        assert_eq!(
            ids(&ranked),
            vec!["z-best", "a-worst"],
            "the better posting must sort first even though its id sorts last"
        );
        for pair in ranked.windows(2) {
            assert!(pair[0].score >= pair[1].score);
        }
    }

    #[test]
    fn an_empty_corpus_ranks_to_an_empty_list() {
        assert!(rank(&[]).is_empty());
    }

    // --- single-axis sorts ----------------------------------------------------------------

    fn sorted(postings: &[Posting], sort: SortBy) -> Vec<String> {
        ids(&rank_postings(
            postings,
            &no_signals(),
            &InternshipFilters::default(),
            sort,
            now(),
        ))
    }

    #[test]
    fn sort_by_parses_its_own_vocabulary_and_rejects_everything_else() {
        assert_eq!(SortBy::parse("composite"), Some(SortBy::Composite));
        assert_eq!(SortBy::parse("pay"), Some(SortBy::Pay));
        assert_eq!(SortBy::parse("posted"), Some(SortBy::Posted));
        assert_eq!(SortBy::parse("deadline"), Some(SortBy::Deadline));
        assert_eq!(SortBy::parse("prestige"), Some(SortBy::Prestige));
        assert_eq!(SortBy::parse("PAY"), Some(SortBy::Pay), "case-insensitive");
        // The blog's `?sort=oldset` precedent: unrecognized is a 400, never a silent default.
        assert_eq!(SortBy::parse("payy"), None);
        assert_eq!(SortBy::parse(""), None);
        assert_eq!(SortBy::parse("score"), None);
    }

    #[test]
    fn the_default_sort_is_the_composite() {
        assert_eq!(SortBy::default(), SortBy::Composite);
    }

    #[test]
    fn sorting_by_pay_orders_by_the_raw_figure_highest_first() {
        let mut low = blank("a-low");
        low.pay = hourly(20.0);
        let mut high = blank("b-high");
        high.pay = hourly(70.0);
        let mut middle = blank("c-middle");
        middle.pay = hourly(45.0);
        assert_eq!(
            sorted(&[low, middle, high], SortBy::Pay),
            vec!["b-high", "c-middle", "a-low"]
        );
    }

    #[test]
    fn sorting_by_pay_separates_figures_the_composite_saturates_together() {
        // `pay_scale` clamps both of these to SCORE_MAX, so the composite ties them and the id
        // tie-break decides. The axis must read the raw figure instead.
        let mut very_high = blank("a-very-high");
        very_high.pay = hourly(PAY_SCALE_CEILING_HOURLY_USD * 3.0);
        let mut at_ceiling = blank("b-at-ceiling");
        at_ceiling.pay = hourly(PAY_SCALE_CEILING_HOURLY_USD);
        let postings = vec![at_ceiling, very_high];
        assert_eq!(
            breakdown_of(&postings[0]).pay.value,
            breakdown_of(&postings[1]).pay.value,
            "the fixtures must be tied on the composite, or this proves nothing"
        );
        assert_eq!(
            sorted(&postings, SortBy::Pay),
            vec!["a-very-high", "b-at-ceiling"]
        );
    }

    #[test]
    fn an_unknown_pay_sorts_last_under_a_pay_sort_rather_than_being_imputed() {
        // The imputed value would be PAY_NEUTRAL_HOURLY_USD, which lands mid-list among these.
        let mut low = blank("a-low");
        low.pay = hourly(PAY_NEUTRAL_HOURLY_USD - 10.0);
        let mut high = blank("b-high");
        high.pay = hourly(PAY_NEUTRAL_HOURLY_USD + 10.0);
        let unknown = blank("c-unknown");
        assert_eq!(
            sorted(&[low, unknown, high], SortBy::Pay),
            vec!["b-high", "a-low", "c-unknown"],
            "unknown must sort last, not into the middle where the imputation would put it"
        );
    }

    #[test]
    fn an_inherited_company_median_is_unknown_to_a_pay_sort() {
        // It counts for the composite and is basis-labelled there. It is still not this
        // posting's figure, and a pay sort is a request for figures.
        let mut stated = blank("a-stated");
        stated.pay = hourly(20.0);
        stated.company_key = "plain".to_string();
        let mut inherits = blank("b-inherits");
        inherits.company_key = "rich".to_string();

        let mut rich = signals("rich", None);
        rich.pay_observations = PAY_IMPUTATION_MIN_OBSERVATIONS;
        rich.median_pay_hourly_usd = Some(PAY_SCALE_CEILING_HOURLY_USD);
        let map = signal_map(vec![rich, signals("plain", None)]);

        let ranked = rank_postings(
            &[stated, inherits],
            &map,
            &InternshipFilters::default(),
            SortBy::Pay,
            now(),
        );
        assert_eq!(
            ranked[1].breakdown.pay.basis,
            ScoreBasis::ImputedFromCompanyMedian,
            "the fixture must actually be inheriting, or this proves nothing"
        );
        assert_eq!(ids(&ranked), vec!["a-stated", "b-inherits"]);
    }

    #[test]
    fn sorting_by_posted_puts_the_newest_first_and_the_undated_last() {
        let mut old = blank("a-old");
        old.posted_at = Some(days_from_now(-90.0));
        let mut new = blank("b-new");
        new.posted_at = Some(days_from_now(-1.0));
        let undated = blank("c-undated");
        assert_eq!(
            sorted(&[old, undated, new], SortBy::Posted),
            vec!["b-new", "a-old", "c-undated"]
        );
    }

    #[test]
    fn an_estimated_posted_date_still_orders_under_a_posted_sort() {
        // The composite caps every estimate at neutral, which ties them all. The axis orders
        // by the date itself, so a cold-start corpus is still usefully sorted rather than
        // being dumped at the bottom of the sort it was asked for.
        let mut recent = blank("a-recent-estimate");
        recent.posted_at = Some(days_from_now(-2.0));
        recent.posted_at_is_estimated = true;
        let mut older = blank("b-older-estimate");
        older.posted_at = Some(days_from_now(-100.0));
        older.posted_at_is_estimated = true;
        let undated = blank("c-undated");

        assert_eq!(
            breakdown_of(&recent).recency.basis,
            ScoreBasis::EstimatedCapped,
            "the fixture must actually be estimated, or this proves nothing"
        );
        assert_eq!(
            sorted(&[older, undated, recent], SortBy::Posted),
            vec!["a-recent-estimate", "b-older-estimate", "c-undated"]
        );
    }

    #[test]
    fn sorting_by_deadline_puts_the_soonest_first_and_no_deadline_last() {
        let mut far = blank("a-far");
        far.deadline = Some(days_from_now(60.0));
        let mut soon = blank("b-soon");
        soon.deadline = Some(days_from_now(2.0));
        let none = blank("c-none");
        assert_eq!(
            sorted(&[far, none, soon], SortBy::Deadline),
            vec!["b-soon", "a-far", "c-none"],
            "no deadline must not head a soonest-first list"
        );
    }

    #[test]
    fn a_passed_deadline_sorts_behind_every_open_one_and_ahead_of_the_undated() {
        // The pre-sweep window: the posting is still live, its deadline has just gone by.
        let mut open_late = blank("a-open-late");
        open_late.deadline = Some(days_from_now(45.0));
        let mut just_passed = blank("b-just-passed");
        just_passed.deadline = Some(days_from_now(-1.0));
        let mut long_passed = blank("c-long-passed");
        long_passed.deadline = Some(days_from_now(-30.0));
        let none = blank("d-none");
        assert_eq!(
            sorted(
                &[none, long_passed, just_passed, open_late],
                SortBy::Deadline
            ),
            vec!["a-open-late", "b-just-passed", "c-long-passed", "d-none"]
        );
    }

    #[test]
    fn a_deadline_exactly_at_now_sorts_as_open_under_a_deadline_sort() {
        // Same boundary as the score's, because both go through `deadline_has_passed`.
        let mut exactly_now = blank("a-exactly-now");
        exactly_now.deadline = Some(now());
        let mut passed = blank("b-passed");
        passed.deadline = Some(days_from_now(-0.001));
        let mut open_later = blank("c-open-later");
        open_later.deadline = Some(days_from_now(5.0));
        assert_eq!(
            sorted(&[passed, open_later, exactly_now], SortBy::Deadline),
            vec!["a-exactly-now", "c-open-later", "b-passed"]
        );
    }

    #[test]
    fn sorting_by_prestige_puts_the_unknown_companies_last() {
        let mut high = blank("a-high");
        high.company_key = "high".to_string();
        let mut low = blank("b-low");
        low.company_key = "low".to_string();
        let mut unknown = blank("c-unknown");
        unknown.company_key = "unknown".to_string();
        let mut missing = blank("d-missing");
        missing.company_key = "missing".to_string();
        let map = signal_map(vec![
            signals("high", Some(0.9)),
            signals("low", Some(0.1)),
            signals("unknown", None),
        ]);
        let ranked = rank_postings(
            &[missing, unknown, low, high],
            &map,
            &InternshipFilters::default(),
            SortBy::Prestige,
            now(),
        );
        assert_eq!(
            ids(&ranked),
            vec!["a-high", "b-low", "c-unknown", "d-missing"],
            "a company with no row and one with an inconclusive row are both unknown, and \
             unknown sorts last even though the composite imputes it to the middle"
        );
    }

    #[test]
    fn a_single_axis_sort_still_publishes_the_full_composite_breakdown() {
        let mut posting = blank("p1");
        posting.pay = hourly(50.0);
        let ranked = rank_postings(
            &[posting],
            &no_signals(),
            &InternshipFilters::default(),
            SortBy::Pay,
            now(),
        );
        assert_eq!(ranked[0].breakdown.pay.basis, ScoreBasis::Stated);
        assert!(close(ranked[0].score, ranked[0].breakdown.total()));
    }

    #[test]
    fn every_sort_keeps_the_id_tie_break() {
        // Nothing is known about any of these, so every axis ties them all and only the
        // tie-break can order them.
        let postings = vec![blank("bbb"), blank("aaa"), blank("ccc")];
        for sort in [
            SortBy::Composite,
            SortBy::Pay,
            SortBy::Posted,
            SortBy::Deadline,
            SortBy::Prestige,
        ] {
            assert_eq!(
                sorted(&postings, sort),
                vec!["aaa", "bbb", "ccc"],
                "{sort:?} must be deterministic"
            );
        }
    }

    #[test]
    fn a_sort_reorders_without_changing_what_survived_the_filters() {
        let mut remote_cheap = blank("a");
        remote_cheap.location.is_remote = Some(true);
        remote_cheap.pay = hourly(20.0);
        let mut remote_rich = blank("b");
        remote_rich.location.is_remote = Some(true);
        remote_rich.pay = hourly(70.0);
        let mut onsite_rich = blank("c");
        onsite_rich.location.is_remote = Some(false);
        onsite_rich.pay = hourly(80.0);

        let all = vec![remote_cheap, remote_rich, onsite_rich];
        let filters = InternshipFilters {
            location: Some(LocationFilter {
                remote: Some(true),
                contains: None,
                on_unknown: OnUnknown::Drop,
            }),
            ..InternshipFilters::default()
        };
        let by_pay = rank_postings(&all, &no_signals(), &filters, SortBy::Pay, now());
        let by_composite = rank_postings(&all, &no_signals(), &filters, SortBy::Composite, now());

        let mut pay_survivors = ids(&by_pay);
        let mut composite_survivors = ids(&by_composite);
        pay_survivors.sort();
        composite_survivors.sort();
        assert_eq!(
            pay_survivors, composite_survivors,
            "the sort must not change membership — that is the filters' job alone"
        );
        assert_eq!(ids(&by_pay), vec!["b", "a"]);
    }
}

#[cfg(test)]
mod sort_name_tests {
    use super::*;

    #[test]
    fn sort_names_round_trip() {
        // Every variant, listed explicitly rather than iterated: a new variant that nobody
        // adds here is the exact drift this test exists to catch, and an iterator over a
        // list that also needs updating would catch nothing.
        for sort in [
            SortBy::Composite,
            SortBy::Pay,
            SortBy::Posted,
            SortBy::Deadline,
            SortBy::Prestige,
        ] {
            assert_eq!(
                SortBy::parse(sort.as_str()),
                Some(sort),
                "{} did not survive a round trip",
                sort.as_str()
            );
        }
    }

    #[test]
    fn every_sort_name_is_distinct() {
        // Two variants sharing a name would round-trip one of them into the other, which the
        // test above would not catch.
        let names = [
            SortBy::Composite.as_str(),
            SortBy::Pay.as_str(),
            SortBy::Posted.as_str(),
            SortBy::Deadline.as_str(),
            SortBy::Prestige.as_str(),
        ];
        let mut unique = names.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "two sorts share a wire name");
    }
}
