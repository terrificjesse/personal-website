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
//! ## Favorites
//!
//! Two jobs, deliberately kept apart rather than folded into one score: the base ranking
//! answers *"what am I into lately"* (recency-weighted), and favorites answer *"what do I
//! reliably rate highly"* (time-independent). Highly-rated recipes are **moved** into
//! `FAVORITE_SLOTS` and labelled `RankReason::Favorite`, so the section cycles through your
//! best recipes instead of showing the same recency-ordered list every time.
//!
//! This started life as a "throwback" mechanism gated on *age* — surfacing old favourites the
//! decay had buried. That gate was dropped (2026-08-13): recent recipes are welcome here too,
//! the goal being rotation rather than nostalgia. The name changed with it, since an amber
//! "Throwback" badge on something cooked last week is simply false.
//!
//! ### Eligibility still needs two gates
//!
//! `FAVORITE_MIN_MEAN_RATING` is the quality half — an unweighted mean, deliberately
//! time-independent, computed in `is_favorite_eligible`.
//!
//! The second gate is **rank**, and it cannot live in `is_favorite_eligible` — that function
//! only sees one recipe's reviews and knows nothing about position. It belongs in
//! `rerank_recommendations`, after the base sort: a recipe already ranked above
//! `FAVORITE_SLOTS[0]` must not be selected, because "moving" it into a slot would push it
//! *down*. Without this, the mechanism can take your top result, demote it three places, and
//! badge it — the age gate used to make that impossible for free, since old favourites always
//! sat near the bottom.
//!
//! Moved, never copied — a recipe must not appear twice, once ranked and once as a favorite.
//! That's the same class of bug the liked/suppressed split had (see
//! `routes/recipes.rs::liked_recipe_ids`).
//!
//! ### On randomness
//!
//! Selection among eligible favorites is random but **seeded by the day**:
//! `StdRng::seed_from_u64(now.num_days_from_ce() as u64)`. So the choice is stable for every
//! request within a UTC day and changes at midnight — rotation, rather than a reshuffle on
//! every page load. It was briefly unseeded (`rand::rng()`), which made the badges jump
//! between reloads and meant a ranking you were debugging could never be reproduced.
//!
//! Seed from the `now` this function already computes, not a second `Utc::now()` — one clock
//! read per pass, the same reasoning that makes `score_recipe` take `now` as a parameter.
//!
//! Two things this does **not** give you. Seeded random is not true rotation: a recipe can be
//! picked several days running by chance, and another can go weeks unseen — there's no
//! coverage guarantee. And the function is still not deterministic *in tests*, because `now`
//! is read internally; promoting it to a parameter would make the whole thing pinnable and
//! let a test assert which recipes get badged instead of only structural invariants.
//!
//! Note that `FAVORITE_SLOTS` starts at index 3, so nothing is injected into lists shorter
//! than four — which is why none of the small-fixture ordering tests below are disturbed by
//! this feature. `favorites_land_only_in_their_slots_and_only_when_eligible` is the one that
//! exercises it, with eleven candidates.
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

use rand::{SeedableRng, rngs::StdRng, seq::IndexedRandom};
use std::collections::HashMap;

use chrono::{DateTime, Datelike, Duration, Utc};
use serde::Serialize;

use crate::models::{NEUTRAL_RATING, Recipe, Review};

/// Why a recipe is sitting where it is in the ranking. Lives here rather than `models.rs`
/// for the same reason `RecipeFilters` lives in `recommend_recipes.rs` — it describes this
/// function's output, not a stored entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RankReason {
    /// Ranked normally, by recency-weighted rating.
    Liked,
    /// Injected at one of `FAVORITE_SLOTS` — an old favourite the decay would otherwise
    /// have buried. The frontend badges these; without the label an old recipe near the top
    /// just looks like the ranking is broken.
    Favorite,
}

/// A ranked recipe plus why it's there, so the frontend can explain the placement without
/// re-deriving it. Same shape as `RecommendedRecipe` in `recommend_recipes.rs`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RankedRecipe {
    pub recipe: Recipe,
    pub reason: RankReason,
}

/// Positions favorites are injected at, as 0-based indices into the final list.
///
/// Fixed slots rather than a ratio: predictable placement means a user learns where to look,
/// and it keeps the interleaving trivially debuggable. Slots beyond the end of a short list
/// are simply skipped — with only two or three liked recipes there's nothing to rotate.
///
/// `FAVORITE_SLOTS[0]` doubles as the rank gate: a recipe already ranked above it is
/// ineligible, since moving it into a slot would demote it. Derive that bound from here
/// rather than repeating `3`.
pub const FAVORITE_SLOTS: [usize; 3] = [3, 5, 7];

/// Minimum mean rating (inclusive) for a recipe to be eligible as a favorite.
///
/// Unweighted mean, on purpose — see the module doc. Inclusive matters: ratings are integers,
/// so a recipe you've only ever given 4★ has a mean of exactly 4.0, and a `>` comparison would
/// silently exclude a large share of what this is meant to surface.
///
/// Quality gate only. The rank gate that stops this from demoting your top results lives in
/// `rerank_recommendations`, since eligibility by position can't be judged from one recipe's
/// reviews alone.
pub const FAVORITE_MIN_MEAN_RATING: f64 = 4.0;

/// Days until a review counts half as much in the **base** ranking.
///
/// Only affects `score_recipe` — never favorite eligibility, which is deliberately
/// time-independent.
///
/// Checked against all four ordering tests at 60, 120 and 180: none of them pin this down, so
/// it's a free parameter to tune by feel. Shorter makes the list feel current at the cost of
/// forgetting quickly (at 60 a 5★ is worth 0.07 after a year); longer keeps favourites alive
/// (at 120 that same review is still 0.62 after six months).
pub const DECAY_HALFLIFE: f64 = 120.0;

/// Reorders `candidates` — all of which the viewer has already rated highly — using
/// `reviews`, the full visible review history, and labels each result with why it landed
/// where it did.
///
/// `viewer` is the account whose page this is (`None` pre-Phase-5); pass it to
/// `Review::is_by` to separate your own feedback from the crowd's.
///
/// Returns a permutation of `candidates`: reorder and label freely, but never add or drop —
/// favorites are *moved* to their slots, not duplicated, so a recipe never appears twice.
/// This function's body is what you implement.
pub fn rerank_recommendations(
    candidates: &[Recipe],
    reviews: &[Review],
    viewer: Option<&str>,
) -> Vec<RankedRecipe> {
    let mut map: HashMap<&str, Vec<&Review>> = HashMap::new();
    for review in reviews {
        map.entry(review.recipe_id.as_str())
            .or_default()
            .push(review);
    }

    let now = Utc::now();

    // (score, recipe, favorite-eligible). Eligibility rides along through the sort because
    // the rank gate below can only be applied once positions are known.
    //
    // A candidate with no reviews scores 0.0 rather than being skipped — membership makes
    // that impossible today, but dropping it would violate the permutation contract.
    let mut scored: Vec<(f64, Recipe, bool)> = candidates
        .iter()
        .map(|candidate| {
            let own = map
                .get(candidate.id.as_str())
                .map(Vec::as_slice)
                .unwrap_or_default();
            (
                score_recipe(own, viewer, now),
                candidate.clone(),
                is_favorite_eligible(own, viewer),
            )
        })
        .collect();

    scored.sort_by(|(a, _, _), (b, _, _)| b.total_cmp(a));

    // The rank gate. Only recipes already sitting at or below the first slot may be promoted
    // — pulling something from above it into a slot would push it *down* the list.
    let promotable: Vec<usize> = scored
        .iter()
        .enumerate()
        .filter(|(index, (_, _, eligible))| *eligible && *index >= FAVORITE_SLOTS[0])
        .map(|(index, _)| index)
        .collect();

    // Only select as many favorites as can actually be *placed*. A chosen favorite is removed
    // from the ranking before being re-inserted, so one that can't reach its slot isn't merely
    // left alone — it's lost, and the permutation contract breaks. With `n` candidates and `k`
    // removed, slot `j` is reachable only if `FAVORITE_SLOTS[j] < n - k + j`; rearranged below
    // to keep the usize arithmetic from underflowing.
    //
    // Concretely: 8 candidates can hold two favorites, not three — the third would need a
    // list of at least 9 to reach index 7.
    let capacity = (0..=FAVORITE_SLOTS.len().min(promotable.len()))
        .rev()
        .find(|&k| {
            FAVORITE_SLOTS
                .iter()
                .take(k)
                .enumerate()
                .all(|(j, &slot)| slot + k < scored.len() + j)
        })
        .unwrap_or(0);

    let seed = now.num_days_from_ce() as u64;
    // Sample indices, not recipes: the chosen ones have to be removed from the ranking
    // before they can be re-inserted at their slots, or they'd appear twice.
    let mut chosen: Vec<usize> = promotable
        .sample(&mut StdRng::seed_from_u64(seed), capacity)
        .copied()
        .collect();

    // Remove highest index first so the lower ones stay valid; that yields worst-ranked
    // first, so reverse to put the best-ranked favorite in the earliest slot.
    chosen.sort_unstable_by(|a, b| b.cmp(a));
    let mut favorites: Vec<Recipe> = chosen.iter().map(|&index| scored.remove(index).1).collect();
    favorites.reverse();

    let ranked: Vec<Recipe> = scored.into_iter().map(|(_, recipe, _)| recipe).collect();
    interleave_favorites(ranked, favorites)
}

/// Base score for one recipe, from its own review history. Higher sorts first.
///
/// `reviews` is just that recipe's reviews — group before calling. Scope to the viewer's own
/// with `Review::is_by` (a no-op today, correct once accounts exist).
///
/// This is where `DECAY_HALFLIFE` and `NEUTRAL_RATING` belong. See the module doc's table for
/// which combinations the tests eliminate — the non-obvious one is that normalizing by total
/// weight cancels a lone review's recency.
///
/// `now` is passed in rather than read from the clock so that every recipe in a single
/// ranking pass is mseasured against the same instant, and so tests can pin it.
fn score_recipe(reviews: &[&Review], viewer: Option<&str>, now: DateTime<Utc>) -> f64 {
    reviews
        .iter()
        .filter(|r| r.is_by(viewer))
        .fold(0.0, |acc, review| {
            let age_days = ((now - review.cooked_at).as_seconds_f64()
                / Duration::days(1).as_seconds_f64())
            .max(0.0);
            let weight = 0.5_f64.powf(age_days / DECAY_HALFLIFE);
            let contribution = (review.rating as f64 - NEUTRAL_RATING) * weight;
            f64::max(acc, contribution)
        })
}

/// The **quality** gate: is this recipe's unweighted mean rating at or above
/// `FAVORITE_MIN_MEAN_RATING`?
///
/// This is only half of eligibility. The **rank** gate — don't select something already
/// ranked above `FAVORITE_SLOTS[0]`, or the "move" demotes it — lives in
/// `rerank_recommendations`, because position isn't knowable from one recipe's reviews.
///
/// `reviews` is just that recipe's reviews, scoped to the viewer with `Review::is_by` — once
/// accounts exist, a stranger's 5★ must not make something *your* favorite.
///
/// **Do not apply decay to the mean here.** The base score is recency-weighted precisely so
/// the list feels current; this gate is what lets a recipe surface regardless of when you last
/// cooked it. Weight this mean by recency too and it collapses back into the base ranking,
/// selecting the same recipes that were already on top.
///
/// Takes no `now` for that reason — nothing here is time-dependent.
fn is_favorite_eligible(reviews: &[&Review], viewer: Option<&str>) -> bool {
    let filtered: Vec<&&Review> = reviews.iter().filter(|r| r.is_by(viewer)).collect();
    if filtered.is_empty() {
        return false;
    }
    let sum = filtered.iter().fold(0.0, |acc, r| acc + r.rating as f64);
    sum / filtered.len() as f64 >= FAVORITE_MIN_MEAN_RATING
}

/// Merges the chosen favorites into the base ordering, labelling each entry.
///
/// `ranked` must **already exclude** everything in `favorites` — that's what enforces
/// "moved, never copied," and it's why the partition happens in the caller rather than here.
/// The result is a permutation of their concatenation: same length, nothing added or dropped
/// (`every_candidate_appears_exactly_once`).
///
/// Slots past the end of a short list are skipped rather than clamped — appending a favorite
/// to a 3-item list would put it last, which is the opposite of surfacing it.
fn interleave_favorites(ranked: Vec<Recipe>, favorites: Vec<Recipe>) -> Vec<RankedRecipe> {
    let mut result: Vec<RankedRecipe> = ranked
        .into_iter()
        .map(|recipe| RankedRecipe {
            recipe,
            reason: RankReason::Liked,
        })
        .collect();

    // `favorites` arrives already chosen and already removed from `ranked` — selection has to
    // happen upstream, since only the sampled ones may be taken out of the ranking.
    for (&slot, favorite) in FAVORITE_SLOTS.iter().zip(favorites) {
        // Strictly less than: inserting at exactly `len` appends, which buries the favorite
        // at the bottom instead of surfacing it. Slots ascend, so once one doesn't fit,
        // neither will the rest.
        if slot >= result.len() {
            break;
        }
        result.insert(
            slot,
            RankedRecipe {
                recipe: favorite,
                reason: RankReason::Favorite,
            },
        );
    }

    result
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

    /// A public review by somebody else. `fetch_visible_to` puts these in the same slice as
    /// the viewer's own (that's the point of the slice), so anything here has to be told
    /// apart with `Review::is_by` rather than by where it came from.
    fn review_by_stranger(recipe_id: &str, rating: i64, days_ago: i64) -> Review {
        Review {
            user_id: Some("someone-else".to_string()),
            is_public: true,
            ..review_at(recipe_id, rating, days_ago)
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

        assert_eq!(
            results.len(),
            candidates.len(),
            "must not add or drop candidates"
        );
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
    fn favorites_land_only_in_their_slots_and_only_when_eligible() {
        // Eleven candidates, so `FAVORITE_SLOTS` (3, 5, 7) actually exist — every other test
        // here uses two or three, which is why none of them are affected by favorites.
        //
        // Deliberately structural rather than naming which recipe should be promoted: you pick
        // randomly among eligible recipes, so asserting a specific winner would either
        // over-specify the selection or turn flaky. These invariants hold whatever selection
        // strategy you land on.
        //
        // Fixture, by base score (max centered decayed contribution) descending:
        //   top0..top4   4★ cooked 1..5 days ago      mean 4.0  -> highest scores, indices 0-4
        //   low_mean     5★ 300d + two 3★ 5d          mean 3.67 -> middling score
        //   old0..old4   two 5★, 400d and 430d        mean 5.0  -> lowest scores
        // Distinct ages within each group so the ordering is deterministic rather than
        // relying on a stable sort over equal scores.
        let mut candidates: Vec<Recipe> = (0..5).map(|i| recipe(&format!("top{i}"))).collect();
        candidates.push(recipe("low_mean"));
        candidates.extend((0..5).map(|i| recipe(&format!("old{i}"))));

        let mut reviews = Vec::new();
        for i in 0..5 {
            reviews.push(review_at(&format!("top{i}"), 4, i as i64 + 1));
        }
        reviews.push(review_at("low_mean", 5, 300));
        reviews.push(review_at("low_mean", 3, 5));
        reviews.push(review_at("low_mean", 3, 5));
        for i in 0..5 {
            reviews.push(review_at(&format!("old{i}"), 5, 400));
            reviews.push(review_at(&format!("old{i}"), 5, 430));
        }

        let results = rerank_recommendations(&candidates, &reviews, Some(VIEWER));

        // Without this the test passes vacuously against an empty result — there'd be no
        // favorites to find, and the invariants below would check nothing.
        assert_eq!(results.len(), candidates.len());

        for (index, ranked) in results.iter().enumerate() {
            if ranked.reason != RankReason::Favorite {
                continue;
            }

            assert!(
                FAVORITE_SLOTS.contains(&index),
                "{} was labelled a favorite at index {index}, which is not a favorite slot",
                ranked.recipe.id
            );

            // Quality gate: mean 3.67 is below FAVORITE_MIN_MEAN_RATING. Note this recipe
            // still *has* a 5★ review — it's a legitimate candidate, it just isn't a
            // favorite, which is exactly the case a max-based base score can't distinguish.
            assert_ne!(
                ranked.recipe.id, "low_mean",
                "low_mean averages 3.67 and shouldn't clear the quality gate"
            );

            // Rank gate: top0..top2 would sit above the first slot on base score alone, so
            // "promoting" them into a slot would actually push them *down* the list. The old
            // age gate used to make this impossible for free; with it gone, position has to
            // be checked explicitly (see the module doc).
            assert!(
                !matches!(ranked.recipe.id.as_str(), "top0" | "top1" | "top2"),
                "{} already ranks above the first favorite slot — moving it there is a demotion",
                ranked.recipe.id
            );
        }
    }

    #[test]
    fn no_candidates_means_no_results() {
        assert_eq!(rerank_recommendations(&[], &[], None), Vec::new());
    }

    // ------------------------------------------------------------------------------------
    // Viewer scoping. Added in Phase 5 — these fail against the Phase 4 implementation,
    // which takes `_viewer` in both `score_recipe` and `is_favorite_eligible` and ignores it
    // in each. Every test above this point builds reviews with `review_at`, which hardcodes
    // `user_id: Some(VIEWER)`, so none of them can tell a scoped implementation from an
    // unscoped one — the fixture, not the assertion, is what let this through.
    // ------------------------------------------------------------------------------------

    #[test]
    fn a_strangers_rave_does_not_lift_a_recipe_in_your_ranking() {
        // "mine" has one recent 4★ from the viewer. "theirs" has a weak 4★ of the viewer's
        // own plus a stranger's glowing, very recent 5★ — enough to overtake "mine" if the
        // crowd's reviews are pooled into the base score.
        //
        // Note this is about the *current* model, not a permanent ban on crowd signal:
        // PLAN.md's Phase 5 explicitly wants a global term in here eventually. What it must
        // not be is an accident. When you add deliberate weighting, revisit this test and
        // decide what it should say then — but a stranger silently outranking your own
        // history is not that feature.
        let candidates = vec![recipe("theirs"), recipe("mine")];
        let reviews = vec![
            review_at("theirs", 4, 300),
            review_by_stranger("theirs", 5, 1),
            review_at("mine", 4, 30),
        ];

        let results = rerank_recommendations(&candidates, &reviews, Some(VIEWER));

        assert_ranks_above(&results, "mine", "theirs");
    }

    #[test]
    fn a_strangers_high_ratings_do_not_make_a_recipe_your_favorite() {
        // The quality gate reads an unweighted mean, so an unscoped `is_favorite_eligible`
        // averages strangers in. Here the viewer's own opinion of "borrowed" is a flat 3★
        // (mean 3.0, well under FAVORITE_MIN_MEAN_RATING); pooling three strangers' 5★s
        // pulls the mean to 4.5 and it clears the gate on other people's say-so.
        //
        // The fixture has to clear two hurdles before it tests anything, and getting either
        // wrong makes this pass vacuously — which the first draft of it did:
        //
        // 1. **"borrowed" must rank at or below FAVORITE_SLOTS[0]**, or the rank gate rejects
        //    it and the quality gate is never consulted. Its 5★s are therefore ancient: decay
        //    drives their base-score contribution to roughly zero even when pooled, while the
        //    eligibility mean is deliberately *not* decayed and stays at 4.5. Age is the only
        //    lever that separates the two gates like this.
        // 2. **No other candidate may be eligible**, or the random selection among eligible
        //    recipes might simply not pick "borrowed" and the assertion would hold by luck.
        //    So `own0..own9` each carry a 4★ and a 3★: mean 3.5, under the gate, while their
        //    max-based base score still sits well above "borrowed".
        let mut candidates: Vec<Recipe> = (0..10).map(|i| recipe(&format!("own{i}"))).collect();
        candidates.push(recipe("borrowed"));

        let mut reviews = Vec::new();
        for i in 0..10 {
            let days_ago = i as i64 + 1;
            reviews.push(review_at(&format!("own{i}"), 4, days_ago));
            reviews.push(review_at(&format!("own{i}"), 3, days_ago));
        }
        reviews.push(review_at("borrowed", 3, 200));
        for days_ago in [3000, 3001, 3002] {
            reviews.push(review_by_stranger("borrowed", 5, days_ago));
        }

        let results = rerank_recommendations(&candidates, &reviews, Some(VIEWER));

        let borrowed = results
            .iter()
            .find(|ranked| ranked.recipe.id == "borrowed")
            .expect("borrowed must still appear — this function never drops");

        assert_ne!(
            borrowed.reason,
            RankReason::Favorite,
            "you rate this 3★; other people's 5★s must not make it one of *your* favorites"
        );
    }

    #[test]
    fn a_recipe_only_strangers_have_reviewed_still_ranks() {
        // The empty-after-filter case, which scoping makes reachable for the first time.
        // Membership (`liked_recipe_ids`) means a real candidate always has at least one
        // review of the viewer's own, but this function must not depend on a guarantee made
        // in another file — the permutation contract is unconditional.
        //
        // Worth writing because the obvious implementation of the quality gate divides by
        // the review count, and an empty slice makes that `0.0 / 0.0` — NaN, which compares
        // false against everything and is only accidentally the right answer.
        // The stranger's rating is deliberately *higher* than the viewer's own. Equal ratings
        // would leave the two recipes on near-identical scores under an unscoped
        // implementation, and which one sorted first would come down to microseconds of
        // difference in fixture construction time — a coin flip, not a result.
        let candidates = vec![recipe("mine"), recipe("only_theirs")];
        let reviews = vec![
            review_at("mine", 4, 10),
            review_by_stranger("only_theirs", 5, 10),
        ];

        let results = rerank_recommendations(&candidates, &reviews, Some(VIEWER));

        assert_eq!(
            results.len(),
            2,
            "must not drop a candidate it has no opinion on"
        );
        assert!(position_of(&results, "only_theirs").is_some());
        assert_ranks_above(&results, "mine", "only_theirs");
    }
}
