//! Learned recipe re-ranking — flagged as a learning area, see CLAUDE.md.
//!
//! Orders the "Recipes you liked" section (`GET /recipes/liked`) using review history.
//!
//! ## Contract: this function orders and labels, it never drops
//!
//! `candidates` arrives **pre-filtered** by `routes/recipes.rs::liked` — every recipe in it
//! already carries at least one review the viewer rated `>= LIKED_RATING_THRESHOLD`. So
//! membership is already decided; the only question left is what order they go in. The
//! output must be a permutation of the input (`every_candidate_appears_exactly_once` pins
//! this down).
//!
//! ## Throwbacks
//!
//! Two jobs, deliberately kept apart rather than folded into one score: the base ranking
//! answers *"what am I into lately"* (recency-weighted), and throwbacks answer *"what did I
//! genuinely love and forget about"* (time-independent). An aggressive half-life is what
//! makes the base ranking feel current, and it's also what buries old favourites — so rather
//! than compromising the decay constant, eligible old favourites are **moved** into
//! `THROWBACK_SLOTS` and labelled `RankReason::Throwback`.
//!
//! Eligibility needs *both* gates. `THROWBACK_MIN_MEAN_RATING` is the quality half;
//! `THROWBACK_MIN_AGE_DAYS` is what makes it a throwback rather than just a good recipe.
//! Without the age gate your current favourites get labelled nostalgia.
//!
//! Moved, never copied — a recipe must not appear twice, once ranked and once as a throwback.
//! That's the same class of bug the liked/suppressed split had (see
//! `routes/recipes.rs::liked_recipe_ids`).
//!
//! ### On randomness
//!
//! Picking randomly among eligible throwbacks is intended, but `thread_rng` inside this
//! function would make the page reshuffle on every reload and leave the behavior untestable.
//! Seed it instead — deriving a seed from the current *date* gives throwbacks that rotate
//! daily, stay put within a session, and can be pinned to a fixed value in tests. Hashing
//! `(recipe_id, seed)` and taking the lowest hashes is a perfectly good selection without
//! pulling in the `rand` crate; ask before adding a dependency (root `CLAUDE.md`).
//!
//! Note that `THROWBACK_SLOTS` starts at index 3, so nothing is injected into lists shorter
//! than four — which is why none of the small-fixture ordering tests below are disturbed by
//! this feature. `throwbacks_land_only_in_their_slots_and_only_when_eligible` is the one that
//! exercises it, with ten candidates.
//!
//! Suppressing badly-rated recipes is deliberately **not** this function's job — it happens
//! on the general-recommendations path instead (`SUPPRESSED_RATING_THRESHOLD`, applied in
//! `routes/recipes.rs::recommended`), which is what PLAN.md's Phase 4 checkpoint actually
//! asks for. A filter composes cleanly with Phase 3's ingredient ranking; a competing
//! *ordering* would fight it.
//!
//! ## What the two inputs actually contain
//!
//! Only `candidates` is filtered. `reviews` is the whole visible history — every rating for
//! every recipe, low ones included (`routes/reviews.rs::fetch_visible_to`). That asymmetry is
//! the interesting part: a recipe stays a candidate on the strength of one old 5★ while its
//! recent 1★ still lands in `reviews`, so mixed history is a real case you have to decide
//! about, not a hypothetical.
//!
//! ## Personal vs. global feedback
//!
//! `reviews` carries **two populations in one slice**: the viewer's own reviews, and other
//! people's public ones. `Review::is_by(viewer)` tells them apart — that's the whole
//! mechanism, deliberately just an id check.
//!
//! They are not interchangeable, and averaging them together destroys both signals: your own
//! 5★ drowns under a hundred strangers' 3★, and a recipe nobody has rated becomes
//! indistinguishable from one rated mediocre by five hundred people. Your own history is
//! *personalization*; the crowd's is a *quality prior*.
//!
//! Pre-Phase-5 there are no accounts, so `viewer` is always `None`, `is_by` reports every
//! review as personal, and this half is inert. Small-sample statistics (why one 5★ must not
//! outrank two hundred averaging 4.6★, and the Bayesian/shrinkage fix) are deferred to
//! Phase 5 along with the accounts that make them matter — see `docs/PLAN.md`.
//!
//! ## What the tests rule out
//!
//! The four ordering tests below were checked against the obvious candidate models, and
//! between them they eliminate every naive one:
//!
//! | Model | Fails on |
//! |---|---|
//! | sum of ratings | `a_single_five_star_outranks_a_repeatedly_cooked_four_star` |
//! | mean of ratings | `a_recent_mediocre_cook_pulls_down_a_previously_loved_recipe` — `(5+3)/2` ties a lone `4` |
//! | max of ratings | same; also any tie the ratings alone can't break |
//! | recency-weighted **sum** | the frequency test again — three old 4★s still outweigh one 5★ |
//! | recency-weighted **mean** | `more_recent_review_breaks_a_rating_tie` — with a single review the weight cancels out of the average, so a 5★ from last week ties a 5★ from a year ago |
//!
//! That last one is the subtle one, and worth internalizing before you start: normalizing by
//! total weight is exactly what throws away the information the tie-break test is asking
//! about. At least two different models do satisfy all four; finding one is the exercise.
//!
//! TODO(you): replace the body of `rerank_recommendations`. The tests below describe the
//! required behavior — they will fail against the current placeholder (which always returns
//! an empty list) until you implement real scoring. See the "Working patterns" section of
//! `apps/fridge-app/CLAUDE.md` for the pitfalls that bit `nlp.rs`, `recommend.rs`, and
//! `recommend_recipes.rs` — this is the fourth scoring function here, so treat a green
//! `cargo test` as necessary, not sufficient.

use serde::Serialize;

use crate::models::{Recipe, Review};

/// Why a recipe is sitting where it is in the ranking. Lives here rather than `models.rs`
/// for the same reason `RecipeFilters` lives in `recommend_recipes.rs` — it describes this
/// function's output, not a stored entity.
// `dead_code` here and on the three constants below is scaffolding, not intent: nothing
// constructs a `RankReason` or reads the thresholds until `rerank_recommendations` has a
// body. Delete these attributes as you start using each one — if any is still unused when
// you're finished, that's a real signal, not noise.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RankReason {
    /// Ranked normally, by recency-weighted rating.
    Liked,
    /// Injected at one of `THROWBACK_SLOTS` — an old favourite the decay would otherwise
    /// have buried. The frontend badges these; without the label an old recipe near the top
    /// just looks like the ranking is broken.
    Throwback,
}

/// A ranked recipe plus why it's there, so the frontend can explain the placement without
/// re-deriving it. Same shape as `RecommendedRecipe` in `recommend_recipes.rs`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RankedRecipe {
    pub recipe: Recipe,
    pub reason: RankReason,
}

/// Positions throwbacks are injected at, as 0-based indices into the final list.
///
/// Fixed slots rather than a ratio: predictable placement means a user learns where to look,
/// and it keeps the interleaving trivially debuggable. Slots beyond the end of a short list
/// are simply skipped — with only two or three liked recipes there's nothing to throw back to.
#[allow(dead_code)]
pub const THROWBACK_SLOTS: [usize; 3] = [3, 5, 7];

/// Minimum mean rating for a recipe to be eligible as a throwback.
///
/// Quality gate only — pair it with `THROWBACK_MIN_AGE_DAYS`, or "throwback" degenerates into
/// "good recipe" and your current favourites get labelled as nostalgia.
#[allow(dead_code)]
pub const THROWBACK_MIN_MEAN_RATING: f64 = 4.0;

/// How long since the last cook before a recipe counts as a throwback. This is the gate that
/// makes the concept mean anything — tune it against how often you actually cook.
#[allow(dead_code)]
pub const THROWBACK_MIN_AGE_DAYS: f64 = 90.0;

/// Reorders `candidates` — all of which the viewer has already rated highly — using
/// `reviews`, the full visible review history, and labels each result with why it landed
/// where it did.
///
/// `viewer` is the account whose page this is (`None` pre-Phase-5); pass it to
/// `Review::is_by` to separate your own feedback from the crowd's.
///
/// Returns a permutation of `candidates`: reorder and label freely, but never add or drop —
/// throwbacks are *moved* to their slots, not duplicated, so a recipe never appears twice.
/// This function's body is what you implement.
pub fn rerank_recommendations(
    _candidates: &[Recipe],
    _reviews: &[Review],
    _viewer: Option<&str>,
) -> Vec<RankedRecipe> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    const VIEWER: &str = "viewer";

    fn recipe(id: &str) -> Recipe {
        Recipe {
            id: id.to_string(),
            name: id.to_string(),
            cuisine_tags: vec![],
            meal_type_tags: vec![],
            cook_time_minutes: None,
            required_appliances: vec![],
            fridge_ingredients: vec![],
            extra_ingredients: vec![],
            image_url: None,
            instructions: "Mix and cook.".to_string(),
        }
    }

    /// A review by the viewer themselves, cooked `days_ago` days back. Dates are always
    /// explicit here — recency is part of the spec, so a fixture that hides it would let a
    /// time-blind implementation pass by accident.
    fn review_at(recipe_id: &str, rating: i64, days_ago: i64) -> Review {
        Review {
            id: format!("{recipe_id}-{days_ago}"),
            recipe_id: recipe_id.to_string(),
            rating,
            cooked_at: Utc::now() - Duration::days(days_ago),
            notes: None,
            user_id: Some(VIEWER.to_string()),
            is_public: false,
            hidden: false,
        }
    }

    fn position_of(results: &[RankedRecipe], id: &str) -> Option<usize> {
        results.iter().position(|r| r.recipe.id == id)
    }

    /// Asserts `first` outranks `second`, with a message naming both — a bare `assert!` on
    /// two positions is unreadable when it fails.
    fn assert_ranks_above(results: &[RankedRecipe], first: &str, second: &str) {
        let first_pos = position_of(results, first).unwrap_or_else(|| panic!("{first} missing"));
        let second_pos = position_of(results, second).unwrap_or_else(|| panic!("{second} missing"));
        assert!(
            first_pos < second_pos,
            "expected {first} (position {first_pos}) to rank above {second} (position {second_pos})"
        );
    }

    #[test]
    fn higher_rated_recipe_ranks_higher() {
        // The baseline. Obvious, but pinned so a later change can't quietly break it.
        let candidates = vec![recipe("good"), recipe("great")];
        let reviews = vec![review_at("good", 4, 10), review_at("great", 5, 10)];

        let results = rerank_recommendations(&candidates, &reviews, Some(VIEWER));

        assert_ranks_above(&results, "great", "good");
    }

    #[test]
    fn more_recent_review_breaks_a_rating_tie() {
        // Both are 5★, so rating alone can't separate them and a stable sort would just
        // preserve input order — which is why "stale" is listed first. This is the test that
        // makes recency mandatory rather than optional.
        let candidates = vec![recipe("stale"), recipe("fresh")];
        let reviews = vec![review_at("stale", 5, 400), review_at("fresh", 5, 7)];

        let results = rerank_recommendations(&candidates, &reviews, Some(VIEWER));

        assert_ranks_above(&results, "fresh", "stale");
    }

    #[test]
    fn a_single_five_star_outranks_a_repeatedly_cooked_four_star() {
        // Your call (2026-08-12): a potentially excellent recipe should beat a reliably
        // merely-good one, so peak rating wins over cooking frequency. This rules out
        // count-weighted aggregation — a plain sum would rank "reliable" (4+4+4=12) above
        // "rave" (5). Mean and max both satisfy it.
        //
        // Invert this if you change your mind; nothing else depends on the direction.
        let candidates = vec![recipe("reliable"), recipe("rave")];
        let reviews = vec![
            review_at("reliable", 4, 30),
            review_at("reliable", 4, 60),
            review_at("reliable", 4, 90),
            review_at("rave", 5, 30),
        ];

        let results = rerank_recommendations(&candidates, &reviews, Some(VIEWER));

        assert_ranks_above(&results, "rave", "reliable");
    }

    #[test]
    fn a_recent_mediocre_cook_pulls_down_a_previously_loved_recipe() {
        // The mixed-history case. It has to be a 3★ rather than a 1★ to stay reachable: a
        // recipe with any review <= SUPPRESSED_RATING_THRESHOLD is excluded from the liked
        // set entirely (`routes/recipes.rs::liked_recipe_ids`), so it would never arrive here
        // as a candidate. 3★ is the strongest negative signal that can actually reach this
        // function.
        //
        // "faded" is listed first so a no-op ordering fails, and both recipes' best rating is
        // 5★ vs 4★ — so passing this requires that the recent 3★ actually counts for
        // something, rather than only the single best review mattering.
        let candidates = vec![recipe("faded"), recipe("steady")];
        let reviews = vec![
            review_at("faded", 5, 365),
            review_at("faded", 3, 7),
            review_at("steady", 4, 7),
        ];

        let results = rerank_recommendations(&candidates, &reviews, Some(VIEWER));

        assert_ranks_above(&results, "steady", "faded");
    }

    #[test]
    fn every_candidate_appears_exactly_once() {
        // The invariant that makes this an ordering function rather than a filter:
        // suppression belongs to the general-recommendations path, not here.
        let candidates = vec![recipe("a"), recipe("b"), recipe("c")];
        let reviews = vec![
            review_at("a", 5, 10),
            review_at("b", 4, 20),
            review_at("c", 5, 30),
        ];

        let results = rerank_recommendations(&candidates, &reviews, Some(VIEWER));

        assert_eq!(results.len(), candidates.len(), "must not add or drop candidates");
        for candidate in &candidates {
            assert!(
                position_of(&results, &candidate.id).is_some(),
                "{} went missing",
                candidate.id
            );
        }
    }

    #[test]
    fn pre_auth_reviews_with_no_user_id_still_rank() {
        // The only test exercising the configuration the app actually runs in today: no
        // accounts, `viewer == None`, `user_id` NULL everywhere. `Review::is_by(None)` treats
        // them all as personal (see `models.rs`), so ranking must work exactly as above.
        let candidates = vec![recipe("good"), recipe("great")];
        let reviews = vec![
            Review {
                user_id: None,
                ..review_at("good", 4, 10)
            },
            Review {
                user_id: None,
                ..review_at("great", 5, 10)
            },
        ];

        let results = rerank_recommendations(&candidates, &reviews, None);

        assert_ranks_above(&results, "great", "good");
    }

    #[test]
    fn throwbacks_land_only_in_their_slots_and_only_when_eligible() {
        // Ten candidates, so `THROWBACK_SLOTS` (3, 5, 7) actually exist — every other test
        // here uses two or three, which is why none of them are affected by throwbacks.
        //
        // Deliberately structural rather than naming which recipe should be thrown back:
        // you've chosen to pick randomly among eligible recipes, so asserting a specific
        // winner would either over-specify the selection or turn flaky. These two invariants
        // hold whatever selection strategy you land on.
        let candidates: Vec<Recipe> = (0..10).map(|i| recipe(&format!("r{i}"))).collect();

        // Half recent and merely good, half old and loved — so there's something to pick
        // from and something that must never be picked.
        let mut reviews = Vec::new();
        for i in 0..5 {
            reviews.push(review_at(&format!("r{i}"), 4, 5));
        }
        for i in 5..10 {
            reviews.push(review_at(&format!("r{i}"), 5, 400));
            reviews.push(review_at(&format!("r{i}"), 5, 430));
        }

        let results = rerank_recommendations(&candidates, &reviews, Some(VIEWER));

        // Without this the test passes vacuously against an empty result — there'd be no
        // throwbacks to find, and the invariants below would check nothing.
        assert_eq!(results.len(), candidates.len());

        for (index, ranked) in results.iter().enumerate() {
            if ranked.reason == RankReason::Throwback {
                assert!(
                    THROWBACK_SLOTS.contains(&index),
                    "{} was labelled a throwback at index {index}, which is not a throwback slot",
                    ranked.recipe.id
                );
                // Only the r5..r9 group clears both gates (mean 5.0, last cooked 400 days
                // ago); r0..r4 were cooked this week.
                assert!(
                    ranked.recipe.id.trim_start_matches('r').parse::<u32>().unwrap() >= 5,
                    "{} is recent and shouldn't be eligible as a throwback",
                    ranked.recipe.id
                );
            }
        }
    }

    #[test]
    fn no_candidates_means_no_results() {
        assert_eq!(rerank_recommendations(&[], &[], None), Vec::new());
    }
}
