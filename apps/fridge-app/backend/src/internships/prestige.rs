//! The prestige signal: how desirable a company is, on `0.0..=1.0`.
//!
//! # Why this is half curated and half derived
//!
//! It began fully derived, on the reasonable principle that a signal computed from collection
//! data beats a list somebody has to maintain. Measured against a real corpus of 455
//! companies, that did not survive contact:
//!
//! - The only derived signal implemented was "how many sources carry this company", and the
//!   corpus maxed out at two. So **60 of 455 companies scored, every one of them exactly
//!   1.0**, and the other 395 were `NULL`. A binary flag wearing a score's clothing.
//! - The obvious replacement, posting volume, ranks **Tesla (78 postings) and TikTok (114)
//!   above Google (1)**. It measures hiring scale, not desirability.
//! - Pay would be the honest proxy, and **exactly 1 company of 455 had a pay figure**.
//!
//! The signal simply is not in the data. So the top band is stated outright, in
//! `data/internships/company-tiers.json`, and everything else is ranked on the evidence we do
//! have. Being a judgement call is why it lives in a data file rather than in this code.
//!
//! # The bands, and why they do not overlap
//!
//! | band | range | meaning |
//! |---|---|---|
//! | curated tier 1 | 1.00 | stated |
//! | curated tier 2 | 0.88 | stated |
//! | curated tier 3 | 0.78 | stated |
//! | derived | 0.35–0.65 | inferred from evidence |
//! | no evidence | `None` | unknown — **never** zero |
//!
//! A curated company always outranks an inferred one, because a judgement about quality beats
//! an inference about volume. Same reasoning as the non-overlapping bands in `nlp.rs`.
//!
//! The derived band is deliberately **centred on 0.5**, which is what `rank` substitutes for
//! an unknown prestige. Were it 0.0–0.6, a company we had merely thin evidence about would
//! score *below* one we knew nothing about — having data would hurt you, which inverts the
//! rule the whole phase is built on.

use serde::Deserialize;
use std::collections::HashMap;

/// Scores for the curated bands. Not evenly spaced: the gap between tier 1 and everything else
/// is meant to be decisive, while 2 and 3 are finer distinctions.
pub const TIER_1_SCORE: f64 = 1.00;
pub const TIER_2_SCORE: f64 = 0.88;
pub const TIER_3_SCORE: f64 = 0.78;

/// The derived band, centred on the 0.5 that `rank` uses for an unknown prestige.
pub const DERIVED_MIN: f64 = 0.35;
pub const DERIVED_MAX: f64 = 0.65;

/// Below this much evidence a company scores `None` (unknown), not a low number. One posting
/// from one source with no salary tells us nothing about a company, and "we have not heard
/// much about them" must not read as "they are bad".
pub const MIN_POSTINGS_FOR_EVIDENCE: i64 = 2;

/// Volume is log-scaled and saturates here. Past this, more postings say "large employer",
/// which is not the question being asked — and unscaled, TikTok's 114 would swamp everything.
pub const VOLUME_SATURATION: f64 = 40.0;

/// Relative weights inside the derived band. Pay leads because it is the only one of the three
/// that speaks to the offer rather than to the company's size.
const WEIGHT_PAY: f64 = 0.5;
const WEIGHT_VOLUME: f64 = 0.3;
const WEIGHT_SOURCES: f64 = 0.2;

/// Hourly USD treated as the top of the pay scale when normalizing. Well above a typical
/// internship rate, so ordinary pay does not saturate.
const PAY_CEILING_HOURLY_USD: f64 = 90.0;

/// The curated file, as parsed.
#[derive(Debug, Clone, Default)]
pub struct CompanyTiers {
    /// normalized company key -> tier number.
    by_key: HashMap<String, u8>,
}

#[derive(Debug, Deserialize)]
struct TierFile {
    companies: Vec<TierEntry>,
}

#[derive(Debug, Deserialize)]
struct TierEntry {
    tier: u8,
    #[allow(dead_code)]
    name: String,
    keys: Vec<String>,
}

impl CompanyTiers {
    /// Load the vendored tier file.
    ///
    /// A missing or malformed file is logged and treated as empty rather than fatal: prestige
    /// degrading to derived-only is a worse ranking, not a broken app, and the fridge and blog
    /// tabs have nothing to do with any of this.
    pub fn load() -> Self {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data/internships/company-tiers.json");

        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                eprintln!(
                    "internships: no company tier file at {} ({err}) — prestige will be \
                     derived-only",
                    path.display()
                );
                return Self::default();
            }
        };

        match serde_json::from_str::<TierFile>(&text) {
            Ok(file) => Self::from_entries(file.companies),
            Err(err) => {
                eprintln!(
                    "internships: company tier file is malformed ({err}) — prestige will be \
                     derived-only"
                );
                Self::default()
            }
        }
    }

    fn from_entries(entries: Vec<TierEntry>) -> Self {
        let mut by_key = HashMap::new();
        for entry in entries {
            for key in entry.keys {
                let key = key.trim().to_lowercase();
                if key.is_empty() {
                    continue;
                }
                // A key listed twice keeps the better (lower) tier, so a careless duplicate
                // cannot silently demote a company.
                by_key
                    .entry(key)
                    .and_modify(|tier: &mut u8| *tier = (*tier).min(entry.tier))
                    .or_insert(entry.tier);
            }
        }
        CompanyTiers { by_key }
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    /// The curated tier for a company key, if it is listed.
    pub fn tier(&self, company_key: &str) -> Option<u8> {
        self.by_key.get(&company_key.trim().to_lowercase()).copied()
    }

    /// The curated score for a company key, if it is listed.
    ///
    /// Tiers beyond the three named bands fall back to the lowest curated score rather than
    /// dropping out of the curated band — being listed at all is itself the judgement.
    pub fn score(&self, company_key: &str) -> Option<f64> {
        self.tier(company_key).map(|tier| match tier {
            1 => TIER_1_SCORE,
            2 => TIER_2_SCORE,
            _ => TIER_3_SCORE,
        })
    }
}

/// What collection observed about a company, as inputs to the derived score.
#[derive(Debug, Clone, Copy, Default)]
pub struct DerivedInputs {
    pub live_postings: i64,
    pub distinct_sources: i64,
    pub max_distinct_sources: i64,
    pub median_pay_hourly_usd: Option<f64>,
}

/// Prestige for a company: curated when listed, derived when there is enough evidence,
/// `None` otherwise.
///
/// `None` means **unknown**, which `rank` reads as neutral. It must never be conflated with a
/// score of zero.
pub fn score(tiers: &CompanyTiers, company_key: &str, inputs: DerivedInputs) -> Option<f64> {
    if let Some(curated) = tiers.score(company_key) {
        return Some(curated);
    }
    derived_score(inputs)
}

/// The evidence-based score for a company nobody has ranked by hand.
pub fn derived_score(inputs: DerivedInputs) -> Option<f64> {
    let has_pay = inputs.median_pay_hourly_usd.is_some();
    let enough_evidence = inputs.live_postings >= MIN_POSTINGS_FOR_EVIDENCE
        || inputs.distinct_sources >= 2
        || has_pay;
    if !enough_evidence {
        return None;
    }

    // Log-scaled so 114 postings is not 114x the signal of one. `ln(1 + n)` keeps a
    // single-posting company at 0 rather than undefined.
    let volume = (1.0 + inputs.live_postings.max(0) as f64).ln()
        / (1.0 + VOLUME_SATURATION).ln();
    let volume = volume.clamp(0.0, 1.0);

    let sources = if inputs.max_distinct_sources > 1 {
        (inputs.distinct_sources.max(0) as f64) / (inputs.max_distinct_sources as f64)
    } else {
        // Every company is carried by at least one source, so when nothing is carried by more
        // than one this input distinguishes nobody. Neutral rather than a free full mark.
        0.5
    };
    let sources = sources.clamp(0.0, 1.0);

    // Absent pay falls back to the midpoint of the other two rather than to zero — the same
    // rule as everywhere else: not knowing what a company pays is not evidence it pays badly.
    let pay = match inputs.median_pay_hourly_usd {
        Some(hourly) => (hourly / PAY_CEILING_HOURLY_USD).clamp(0.0, 1.0),
        None => (volume + sources) / 2.0,
    };

    let blended = WEIGHT_PAY * pay + WEIGHT_VOLUME * volume + WEIGHT_SOURCES * sources;
    Some(DERIVED_MIN + blended.clamp(0.0, 1.0) * (DERIVED_MAX - DERIVED_MIN))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiers() -> CompanyTiers {
        CompanyTiers::from_entries(vec![
            TierEntry {
                tier: 1,
                name: "Google".into(),
                keys: vec!["google".into(), "alphabet".into()],
            },
            TierEntry {
                tier: 2,
                name: "Palantir".into(),
                keys: vec!["palantir".into(), "palantir technologies".into()],
            },
        ])
    }

    #[test]
    fn the_vendored_file_parses_and_is_not_empty() {
        // It is data, so a typo breaks it silently at runtime rather than at compile time.
        let loaded = CompanyTiers::load();
        assert!(!loaded.is_empty(), "the shipped tier file should parse");
        assert_eq!(loaded.tier("google"), Some(1));
    }

    #[test]
    fn aliases_resolve_to_the_same_tier() {
        // This file doubles as the alias table dedup deliberately lacks. `palantir` and
        // `palantir technologies` arrived from live data as two separate companies.
        let tiers = tiers();
        assert_eq!(tiers.score("palantir"), tiers.score("palantir technologies"));
        assert_eq!(tiers.score("google"), tiers.score("alphabet"));
    }

    #[test]
    fn company_keys_match_case_insensitively() {
        assert_eq!(tiers().tier("  GOOGLE "), Some(1));
    }

    #[test]
    fn a_curated_company_always_outranks_a_derived_one() {
        // The whole point of the change. Google had one posting from one source; without the
        // curated band no arrangement of the evidence puts it above a high-volume employer.
        let google = score(&tiers(), "google", DerivedInputs::default());
        let busy_unknown = score(
            &tiers(),
            "some-large-employer",
            DerivedInputs {
                live_postings: 114,
                distinct_sources: 2,
                max_distinct_sources: 2,
                median_pay_hourly_usd: Some(PAY_CEILING_HOURLY_USD),
            },
        );
        assert!(
            google.unwrap() > busy_unknown.unwrap(),
            "google {google:?} should beat the busiest possible derived company {busy_unknown:?}"
        );
    }

    /// The bands must not overlap, or a hand-ranked company could be beaten by an inferred
    /// one. A `const` block rather than a `#[test]`: both sides are compile-time constants, so
    /// a violation should stop the build rather than wait for the suite to run.
    const _: () = assert!(TIER_3_SCORE > DERIVED_MAX);

    #[test]
    fn a_company_with_no_evidence_is_unknown_not_zero() {
        // The rule the whole phase turns on. A single posting from a single source with no
        // salary tells us nothing, and nothing must not read as bad.
        let thin = derived_score(DerivedInputs {
            live_postings: 1,
            distinct_sources: 1,
            max_distinct_sources: 2,
            median_pay_hourly_usd: None,
        });
        assert_eq!(thin, None);
    }

    /// If the derived band sat entirely below 0.5, a company we knew a little about would
    /// rank *worse* than one we knew nothing about — having data would be a liability, which
    /// inverts the rule this phase is built on. Compile-time, as above.
    const _: () = assert!(DERIVED_MIN < 0.5 && DERIVED_MAX > 0.5);

    #[test]
    fn volume_saturates_rather_than_dominating() {
        let ten = derived_score(DerivedInputs {
            live_postings: 10,
            distinct_sources: 1,
            max_distinct_sources: 2,
            median_pay_hourly_usd: None,
        })
        .unwrap();
        let hundred = derived_score(DerivedInputs {
            live_postings: 114,
            distinct_sources: 1,
            max_distinct_sources: 2,
            median_pay_hourly_usd: None,
        })
        .unwrap();
        assert!(hundred > ten, "more postings should still be worth more");
        assert!(
            hundred - ten < 0.1,
            "but 11x the postings must not be 11x the score (got {ten} vs {hundred})"
        );
    }

    #[test]
    fn every_derived_score_lands_inside_its_band() {
        for postings in [2, 5, 40, 500] {
            for pay in [None, Some(5.0), Some(200.0)] {
                let value = derived_score(DerivedInputs {
                    live_postings: postings,
                    distinct_sources: 2,
                    max_distinct_sources: 2,
                    median_pay_hourly_usd: pay,
                })
                .unwrap();
                assert!(
                    (DERIVED_MIN..=DERIVED_MAX).contains(&value),
                    "{value} outside the derived band for {postings} postings, pay {pay:?}"
                );
            }
        }
    }

    #[test]
    fn a_duplicate_key_keeps_the_better_tier() {
        let tiers = CompanyTiers::from_entries(vec![
            TierEntry { tier: 3, name: "X".into(), keys: vec!["x".into()] },
            TierEntry { tier: 1, name: "X again".into(), keys: vec!["x".into()] },
        ]);
        assert_eq!(tiers.tier("x"), Some(1));
    }
}
