
use std::cmp::Reverse;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::models::{FridgeItem, Recipe, ShoppingListItem};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeFilters {
    pub cuisine: Option<String>,
    pub meal_type: Option<String>,
}

/// Struct for recording the number of ingredients matched in the recipe
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecommendedRecipe {
    pub recipe: Recipe,
    pub matched_ingredient_count: usize,
    pub total_ingredient_count: usize,
}

/// recipes: prefiltered list of nondisliked recipes
/// fridge: list of fridge items
/// shopping_list: list of shopping list items
/// filters: user defined filters for recipe type
///
/// Hashes all of the ingredients in the fridge and shopping list. Iterates
/// through all of the recipes to identify how many ingredients are present in
/// the set and ensures that the recipes adhers to the filters. Then the
/// recommendations are sorted by the raw number of missing ingredients in
/// ascending order, with small ingredient recipes pushed to the back of the
/// ranking.
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
            instructions: "Mix and cook.".to_string(),
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
