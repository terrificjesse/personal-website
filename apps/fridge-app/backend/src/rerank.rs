
use rand::{SeedableRng, rngs::StdRng, seq::IndexedRandom};
use std::collections::HashMap;

use chrono::{DateTime, Datelike, Duration, Utc};
use serde::Serialize;

use crate::models::{NEUTRAL_RATING, Recipe, Review};

/// Tracks reasons for a ranking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RankReason {
    /// All of the recipes ranked by a heuristic
    Liked,
    /// Certain recipes are boosted to the top to keep the list fresh
    Favorite,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RankedRecipe {
    pub recipe: Recipe,
    pub reason: RankReason,
}

/// Fixed slots for the favorite boosted recipes
pub const FAVORITE_SLOTS: [usize; 3] = [3, 5, 7];

/// Minimum rating for a recipe to be boosted
pub const FAVORITE_MIN_MEAN_RATING: f64 = 4.0;

/// A constant decay on liked recipes ratings to boost more recent high rated
/// recipes
pub const DECAY_HALFLIFE: f64 = 120.0;

/// Compiles an ordered Vector containing the recommendations to be pushed to a
/// viewer
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

    // Maps the candidate recipes to their rating score and favorite eligibity
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

    // Tracks if a recipe has the favorite filter and if moving it would be a promotion
    let promotable: Vec<usize> = scored
        .iter()
        .enumerate()
        .filter(|(index, (_, _, eligible))| *eligible && *index >= FAVORITE_SLOTS[0])
        .map(|(index, _)| index)
        .collect();

    // Tracks the number of promotions that can be feasibly made without exceeding the number of candidates
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
    // Samples a randomly generated seed for the possible promotable recipes for selection
    let mut chosen: Vec<usize> = promotable
        .sample(&mut StdRng::seed_from_u64(seed), capacity)
        .copied()
        .collect();

    // Sort the chosen in ascending order by index and remove them from the ranked set
    chosen.sort_unstable_by(|a, b| b.cmp(a));
    let mut favorites: Vec<Recipe> = chosen.iter().map(|&index| scored.remove(index).1).collect();
    favorites.reverse();

    let ranked: Vec<Recipe> = scored.into_iter().map(|(_, recipe, _)| recipe).collect();
    interleave_favorites(ranked, favorites)
}

/// Generates a score for the given recipe. Checks all the review ratings for a
/// maximum rating score. Older reviews are given a penalty depending on the
/// review age.
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

/// Checks if the average of the review ratings exceeds the minimum favorite
/// rating
fn is_favorite_eligible(reviews: &[&Review], viewer: Option<&str>) -> bool {
    let filtered: Vec<&&Review> = reviews.iter().filter(|r| r.is_by(viewer)).collect();
    if filtered.is_empty() {
        return false;
    }
    let sum = filtered.iter().fold(0.0, |acc, r| acc + r.rating as f64);
    sum / filtered.len() as f64 >= FAVORITE_MIN_MEAN_RATING
}

/// Interleaves the favorite recipes in the currently ranked recipes
fn interleave_favorites(ranked: Vec<Recipe>, favorites: Vec<Recipe>) -> Vec<RankedRecipe> {
    let mut result: Vec<RankedRecipe> = ranked
        .into_iter()
        .map(|recipe| RankedRecipe {
            recipe,
            reason: RankReason::Liked,
        })
        .collect();

    // As long as the slot is available in ranked, insert the favorite at the slot index
    for (&slot, favorite) in FAVORITE_SLOTS.iter().zip(favorites) {
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
        let candidates = vec![recipe("good"), recipe("great")];
        let reviews = vec![review_at("good", 4, 10), review_at("great", 5, 10)];

        let results = rerank_recommendations(&candidates, &reviews, Some(VIEWER));

        assert_ranks_above(&results, "great", "good");
    }

    #[test]
    fn more_recent_review_breaks_a_rating_tie() {
        let candidates = vec![recipe("stale"), recipe("fresh")];
        let reviews = vec![review_at("stale", 5, 400), review_at("fresh", 5, 7)];

        let results = rerank_recommendations(&candidates, &reviews, Some(VIEWER));

        assert_ranks_above(&results, "fresh", "stale");
    }

    #[test]
    fn a_single_five_star_outranks_a_repeatedly_cooked_four_star() {
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

            assert_ne!(
                ranked.recipe.id, "low_mean",
                "low_mean averages 3.67 and shouldn't clear the quality gate"
            );

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


    #[test]
    fn a_strangers_rave_does_not_lift_a_recipe_in_your_ranking() {
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
