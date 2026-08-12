//! Learned recipe re-ranking — flagged as a learning area, see CLAUDE.md.
//!
//! Goal, per PLAN.md's Phase 4: use review history (`Review`, `src/models.rs`) to reorder a
//! set of candidate recipes so past feedback shapes what resurfaces. `GET /recipes/liked`
//! (`src/routes/recipes.rs::liked`) is the only current caller — it narrows the full catalog
//! down to recipes with at least one review at or above `LIKED_RATING_THRESHOLD`, then hands
//! that candidate set here for ordering. That membership filter is a plain `[gen]` threshold
//! check, not the learning content; this function's ranking (and any further suppression) is.
//!
//! Desired behavior, per PLAN.md's checkpoint: a recipe with a highly-rated review should
//! rank higher than one without, on repeat suggestion; a recipe with a poorly-rated review
//! should be suppressed rather than resurfaced; a recipe with no reviews at all should be
//! unaffected by the presence of other recipes' reviews. Worth reading before you start:
//! explicit-feedback recommenders, and a simple weighted score (rating × recency decay) as a
//! first pass — see PLAN.md Phase 4 for the fuller list of directions to research.
//!
//! TODO(you): replace the body of `rerank_recommendations`. The tests below describe the
//! required behavior — they will fail against the current placeholder (which always returns
//! an empty list) until you implement real scoring. See the "Working patterns" section of
//! `apps/fridge-app/CLAUDE.md` for pitfalls that bit `nlp.rs`, `recommend.rs`, and
//! `recommend_recipes.rs` and will likely bite this too — this is the third scoring function
//! in this project, so treat a green `cargo test` as necessary, not sufficient.

use crate::models::{Recipe, Review};

/// Reorders (and may drop) `candidates` based on `reviews`. See the module doc for the
/// desired behavior — this function's body is what you implement.
pub fn rerank_recommendations(_candidates: &[Recipe], _reviews: &[Review]) -> Vec<Recipe> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

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

    fn review(recipe_id: &str, rating: i64) -> Review {
        Review {
            id: "test".to_string(),
            recipe_id: recipe_id.to_string(),
            rating,
            cooked_at: Utc::now() - Duration::days(1),
            notes: None,
        }
    }

    fn position_of(results: &[Recipe], id: &str) -> Option<usize> {
        results.iter().position(|r| r.id == id)
    }

    #[test]
    fn liked_recipe_ranks_higher_on_repeat_suggestion() {
        // "unreviewed" comes first in the input; a highly-rated "liked" should still end up
        // ahead of it in the output.
        let candidates = vec![recipe("unreviewed"), recipe("liked")];
        let reviews = vec![review("liked", 5)];

        let results = rerank_recommendations(&candidates, &reviews);

        let liked_pos = position_of(&results, "liked").expect("should appear");
        let unreviewed_pos = position_of(&results, "unreviewed").expect("should appear");
        assert!(
            liked_pos < unreviewed_pos,
            "a highly-rated recipe should rank ahead of an unreviewed one"
        );
    }

    #[test]
    fn disliked_recipe_is_suppressed() {
        let candidates = vec![recipe("liked"), recipe("disliked")];
        let reviews = vec![review("liked", 5), review("disliked", 1)];

        let results = rerank_recommendations(&candidates, &reviews);

        assert!(
            position_of(&results, "disliked").is_none(),
            "a poorly-rated recipe should not resurface"
        );
        assert!(position_of(&results, "liked").is_some());
    }

    #[test]
    fn unreviewed_recipe_is_unaffected_by_other_recipes_reviews() {
        // A recipe with no review of its own should still appear even when other candidates
        // in the same batch have strong (positive or negative) reviews.
        let candidates = vec![recipe("liked"), recipe("unreviewed"), recipe("disliked")];
        let reviews = vec![review("liked", 5), review("disliked", 1)];

        let results = rerank_recommendations(&candidates, &reviews);

        assert!(position_of(&results, "unreviewed").is_some());
    }

    #[test]
    fn no_candidates_means_no_results() {
        assert_eq!(rerank_recommendations(&[], &[]), Vec::new());
    }
}
