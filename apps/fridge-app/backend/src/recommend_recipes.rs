//! Recipe recommendations — flagged as a learning area, see CLAUDE.md.
//!
//! Goal, per PLAN.md's Phase 3: score/filter the vendored TheMealDB catalog
//! (`src/themealdb.rs`) by how many of a recipe's `fridge_ingredients` are already
//! available — in the fridge *or* on the shopping list — so a recipe you're closer to
//! being able to cook ranks higher. `extra_ingredients` (pantry staples/spices) are
//! deliberately excluded from the match calculation; PLAN.md doesn't expect a fridge app to
//! track whether you own salt.
//!
//! The user's typed filters (`RecipeFilters.cuisine`, `.meal_type`) are a **hard filter**,
//! not a scoring input — a recipe that fails the filter must never appear, no matter how
//! well it would otherwise score. This is what keeps results predictable: PLAN.md is
//! explicit that a recipe needing none of your current ingredients should still show up if
//! the filters allow it, so match quality is a ranking signal, not an inclusion gate.
//!
//! TODO(you): replace the body of `recommend_recipes`. The tests below describe the
//! required behavior — they will fail against the current placeholder (which always
//! returns an empty list) until you implement real scoring. Worth reading before you start:
//! set-overlap scoring (what fraction of a recipe's `fridge_ingredients` you already have —
//! similar in spirit to Jaccard similarity), and how to combine a hard filter with a soft
//! score without letting one leak into the other. See the "Working patterns" section of
//! `apps/fridge-app/CLAUDE.md` for pitfalls that bit `nlp.rs` and `recommend.rs` and will
//! likely bite this too (band overflows, unreachable branches, tests passing for the wrong
//! reason, trusting hand-built fixtures over real data).

use std::cmp::Reverse;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::models::{FridgeItem, Recipe, ShoppingListItem};

/// Hard filters from `GET /recipes/recommended?cuisine=&mealType=`. `#[serde(rename_all =
/// "camelCase")]` is what maps the `mealType` query param onto `meal_type` here — axum's
/// `Query` extractor deserializes straight into this struct.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeFilters {
    pub cuisine: Option<String>,
    pub meal_type: Option<String>,
}

/// A recipe plus enough context for the frontend to explain *why* it's ranked where it is,
/// without prescribing the ranking formula itself — that's `recommend_recipes`'s job.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecommendedRecipe {
    pub recipe: Recipe,
    /// How many of `recipe.fridge_ingredients` matched something in `fridge` or
    /// `shopping_list`. Informational for the frontend ("6/8 ingredients you have") — not a
    /// prescription for how `recommend_recipes` should compute its ranking.
    pub matched_ingredient_count: usize,
    pub total_ingredient_count: usize,
}

/// Recommends recipes from the full catalog, scored by ingredient overlap with `fridge` and
/// `shopping_list`, hard-filtered by `filters`. See the module doc for what "hard filter"
/// vs "soft score" means here, and the module boundary — this function's body is what you
/// implement.
pub fn recommend_recipes(
    recipes: &[Recipe],
    fridge: &[FridgeItem],
    shopping_list: &[ShoppingListItem],
    filters: &RecipeFilters,
) -> Vec<RecommendedRecipe> {
    let set = fridge
        .iter()
        .map(|item| item.canonical_name.clone())
        .chain(shopping_list.iter().map(|item| item.name.clone()))
        .collect::<HashSet<String>>();
    let mut result = Vec::new();
    for recipe in recipes {
        let mut count = 0;
        for item in recipe.fridge_ingredients.clone() {
            if set.contains(&item.name.to_lowercase()) {
                count += 1;
            }
        }
        if filters
            .cuisine
            .as_ref()
            .is_none_or(|c| recipe.cuisine_tags.contains(c))
            && filters
                .meal_type
                .as_ref()
                .is_none_or(|t| recipe.meal_type_tags.contains(t))
        {
            result.push(RecommendedRecipe {
                recipe: (recipe.clone()),
                matched_ingredient_count: (count),
                total_ingredient_count: (recipe.fridge_ingredients.len()),
            })
        }
    }
    result.sort_by_key(|r| {
        (
            r.total_ingredient_count < 2,
            r.total_ingredient_count - r.matched_ingredient_count,
            Reverse(r.total_ingredient_count),
        )
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{RecipeIngredient, ShoppingListStatus};
    use chrono::Utc;

    fn ingredient(name: &str) -> RecipeIngredient {
        RecipeIngredient {
            name: name.to_string(),
            measure: "1".to_string(),
        }
    }

    fn recipe(id: &str, cuisine: &str, meal_type: &str, fridge_ingredients: &[&str]) -> Recipe {
        Recipe {
            id: id.to_string(),
            name: id.to_string(),
            cuisine_tags: vec![cuisine.to_string()],
            meal_type_tags: vec![meal_type.to_string()],
            cook_time_minutes: None,
            required_appliances: vec![],
            fridge_ingredients: fridge_ingredients.iter().map(|n| ingredient(n)).collect(),
            extra_ingredients: vec![ingredient("Salt")],
            image_url: None,
        }
    }

    fn fridge_item(name: &str) -> FridgeItem {
        FridgeItem {
            id: "test".to_string(),
            canonical_name: name.to_lowercase(),
            quantity: 1.0,
            unit: "count".to_string(),
            added_at: Utc::now(),
            estimated_expiration: None,
            foodkeeper_product_id: None,
        }
    }

    fn shopping_list_item(name: &str) -> ShoppingListItem {
        ShoppingListItem {
            id: "test".to_string(),
            name: name.to_lowercase(),
            quantity: 1.0,
            unit: "count".to_string(),
            is_grocery: true,
            added_manually: true,
            status: ShoppingListStatus::Pending,
            foodkeeper_product_id: None,
            added_at: Utc::now(),
        }
    }

    fn position_of(results: &[RecommendedRecipe], id: &str) -> Option<usize> {
        results.iter().position(|r| r.recipe.id == id)
    }

    #[test]
    fn recipe_using_more_of_what_you_have_ranks_higher() {
        // "well_stocked" needs three ingredients you have all three of; "sparse" needs
        // three ingredients you have only one of. The better match should rank first.
        let recipes = vec![
            recipe(
                "sparse",
                "Italian",
                "Dinner",
                &["Chicken", "Basil", "Parmesan"],
            ),
            recipe(
                "well_stocked",
                "Italian",
                "Dinner",
                &["Chicken", "Tomato", "Garlic"],
            ),
        ];
        let fridge = vec![
            fridge_item("Chicken"),
            fridge_item("Tomato"),
            fridge_item("Garlic"),
        ];

        let results = recommend_recipes(&recipes, &fridge, &[], &RecipeFilters::default());

        let well_stocked_pos = position_of(&results, "well_stocked").expect("should appear");
        let sparse_pos = position_of(&results, "sparse").expect("should appear");
        assert!(
            well_stocked_pos < sparse_pos,
            "recipe matching more available ingredients should rank first"
        );
    }

    #[test]
    fn ingredients_on_the_shopping_list_count_as_available_too() {
        // PLAN.md: recommendations draw on "fridge + shopping list contents," not fridge
        // alone — an ingredient you're already planning to buy shouldn't be treated as
        // missing.
        let recipes = vec![recipe("target", "Mexican", "Dinner", &["Avocado", "Lime"])];
        let fridge = vec![fridge_item("Avocado")];
        let shopping_list = vec![shopping_list_item("Lime")];

        let results =
            recommend_recipes(&recipes, &fridge, &shopping_list, &RecipeFilters::default());

        let result = results
            .iter()
            .find(|r| r.recipe.id == "target")
            .expect("should appear");
        assert_eq!(result.matched_ingredient_count, 2);
    }

    #[test]
    fn cuisine_filter_excludes_non_matching_recipes_regardless_of_score() {
        // "perfect_match" has every ingredient you own but is the wrong cuisine — it must
        // not appear. "partial_match" is the right cuisine despite matching nothing.
        let recipes = vec![
            recipe("perfect_match", "Japanese", "Dinner", &["Chicken"]),
            recipe("partial_match", "Italian", "Dinner", &["Anchovy"]),
        ];
        let fridge = vec![fridge_item("Chicken")];
        let filters = RecipeFilters {
            cuisine: Some("Italian".to_string()),
            meal_type: None,
        };

        let results = recommend_recipes(&recipes, &fridge, &[], &filters);

        assert!(position_of(&results, "perfect_match").is_none());
        assert!(position_of(&results, "partial_match").is_some());
    }

    #[test]
    fn meal_type_filter_excludes_non_matching_recipes() {
        let recipes = vec![
            recipe("dessert_recipe", "French", "Dessert", &["Sugar"]),
            recipe("dinner_recipe", "French", "Dinner", &["Sugar"]),
        ];
        let filters = RecipeFilters {
            cuisine: None,
            meal_type: Some("Dinner".to_string()),
        };

        let results = recommend_recipes(&recipes, &[], &[], &filters);

        assert!(position_of(&results, "dessert_recipe").is_none());
        assert!(position_of(&results, "dinner_recipe").is_some());
    }

    #[test]
    fn recipe_needing_none_of_your_ingredients_still_appears_if_filters_allow_it() {
        // Match quality is a ranking signal, not an inclusion gate — PLAN.md is explicit
        // about this.
        let recipes = vec![recipe(
            "no_overlap",
            "Thai",
            "Dinner",
            &["Lemongrass", "Fish Sauce"],
        )];

        let results = recommend_recipes(&recipes, &[], &[], &RecipeFilters::default());

        assert!(position_of(&results, "no_overlap").is_some());
    }

    #[test]
    fn extra_ingredients_do_not_affect_matching() {
        // Every fixture recipe includes "Salt" as an extra_ingredient the fridge never has
        // — if that leaked into scoring, no recipe could ever be a "perfect" match.
        let recipes = vec![recipe("target", "Greek", "Dinner", &["Feta"])];
        let fridge = vec![fridge_item("Feta")];

        let results = recommend_recipes(&recipes, &fridge, &[], &RecipeFilters::default());

        let result = results
            .iter()
            .find(|r| r.recipe.id == "target")
            .expect("should appear");
        assert_eq!(
            result.matched_ingredient_count,
            result.total_ingredient_count
        );
    }

    #[test]
    fn no_recipes_means_no_results() {
        assert_eq!(
            recommend_recipes(&[], &[], &[], &RecipeFilters::default()),
            Vec::new()
        );
    }
}
