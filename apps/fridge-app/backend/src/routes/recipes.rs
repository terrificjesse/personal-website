use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use sqlx::SqlitePool;

use crate::models::{LIKED_RATING_THRESHOLD, Recipe, Review, SUPPRESSED_RATING_THRESHOLD};
use crate::recommend_recipes::{self, RecipeFilters, RecommendedRecipe};
use crate::rerank::{self, RankedRecipe};
use crate::routes::auth::CurrentUser;
use crate::routes::{items, reviews, shopping_list};
use crate::themealdb::Catalog;

/// Collects all recipes with a self review below the suppression threshold
fn suppressed_recipe_ids<'a>(reviews: &'a [Review], viewer: Option<&str>) -> HashSet<&'a str> {
    reviews
        .iter()
        .filter(|review| review.is_by(viewer) && review.rating <= SUPPRESSED_RATING_THRESHOLD)
        .map(|review| review.recipe_id.as_str())
        .collect()
}

/// Includes recipes with all self ratings above the supression threshold and a
/// self rating above the liked threshold.
fn liked_recipe_ids<'a>(reviews: &'a [Review], viewer: Option<&str>) -> HashSet<&'a str> {
    let suppressed = suppressed_recipe_ids(reviews, viewer);

    reviews
        .iter()
        .filter(|review| {
            review.is_by(viewer)
                && review.rating >= LIKED_RATING_THRESHOLD
                && !suppressed.contains(review.recipe_id.as_str())
        })
        .map(|review| review.recipe_id.as_str())
        .collect()
}

/// Fetches the fridge data and shopping list data and recommends some recipes
/// based on the ingredients and recipes that the user hasn't strongly disliked
pub async fn recommended(
    State(pool): State<SqlitePool>,
    State(catalog): State<Arc<Catalog>>,
    user: CurrentUser,
    Query(filters): Query<RecipeFilters>,
) -> Result<Json<Vec<RecommendedRecipe>>, StatusCode> {
    let fridge = items::fetch_all(&pool, &user.0.id).await?;
    let shopping_list = shopping_list::fetch_all(&pool, &user.0.id).await?;

    let own_reviews = reviews::fetch_for_viewer(&pool, user.viewer()).await?;
    let suppressed = suppressed_recipe_ids(&own_reviews, user.viewer());

    let mut recommended =
        recommend_recipes::recommend_recipes(catalog.recipes(), &fridge, &shopping_list, &filters);
    recommended.retain(|entry| !suppressed.contains(entry.recipe.id.as_str()));

    Ok(Json(recommended))
}

/// Constructs a Vector with all of the liked recipes by the viewer and passes
/// it off to rerank_recommendations where it is assigned an ordering and
/// filtered
pub async fn liked(
    State(pool): State<SqlitePool>,
    State(catalog): State<Arc<Catalog>>,
    user: CurrentUser,
) -> Result<Json<Vec<RankedRecipe>>, StatusCode> {
    let visible_reviews = reviews::fetch_visible_to(&pool, user.viewer()).await?;
    let liked = liked_recipe_ids(&visible_reviews, user.viewer());

    let candidates: Vec<Recipe> = catalog
        .recipes()
        .iter()
        .filter(|recipe| liked.contains(recipe.id.as_str()))
        .cloned()
        .collect();

    Ok(Json(rerank::rerank_recommendations(
        &candidates,
        &visible_reviews,
        user.viewer(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    const VIEWER: &str = "viewer";

    fn review(recipe_id: &str, rating: i64) -> Review {
        Review {
            id: "test".to_string(),
            recipe_id: recipe_id.to_string(),
            rating,
            cooked_at: Utc::now(),
            notes: None,
            user_id: Some(VIEWER.to_string()),
            is_public: false,
            hidden: false,
        }
    }

    fn review_by_stranger(recipe_id: &str, rating: i64) -> Review {
        Review {
            user_id: Some("someone-else".to_string()),
            is_public: true,
            ..review(recipe_id, rating)
        }
    }

    #[test]
    fn poorly_rated_recipes_are_suppressed() {
        let reviews = vec![
            review("hated", 1),
            review("disliked", 2),
            review("loved", 5),
        ];

        let suppressed = suppressed_recipe_ids(&reviews, Some(VIEWER));

        assert!(suppressed.contains("hated"));
        assert!(!suppressed.contains("disliked"));
        assert!(!suppressed.contains("loved"));
    }

    #[test]
    fn a_middling_rating_does_not_suppress() {
        let reviews = [review("middling", 3)];

        assert!(suppressed_recipe_ids(&reviews, Some(VIEWER)).is_empty());
    }

    #[test]
    fn one_bad_review_suppresses_despite_a_later_good_one() {
        let reviews = vec![review("mixed", 1), review("mixed", 5)];

        assert!(suppressed_recipe_ids(&reviews, Some(VIEWER)).contains("mixed"));
    }

    #[test]
    fn a_strangers_bad_review_does_not_suppress_your_recommendations() {
        let reviews = vec![review_by_stranger("fine-by-me", 1)];

        assert!(suppressed_recipe_ids(&reviews, Some(VIEWER)).is_empty());
    }

    #[test]
    fn a_strangers_high_rating_does_not_make_a_recipe_liked() {
        let reviews = vec![review_by_stranger("theirs", 5)];

        assert!(liked_recipe_ids(&reviews, Some(VIEWER)).is_empty());
    }

    #[test]
    fn highly_rated_recipes_are_liked() {
        let reviews = vec![review("loved", 5), review("good", 4), review("meh", 3)];

        let liked = liked_recipe_ids(&reviews, Some(VIEWER));

        assert!(liked.contains("loved"));
        assert!(liked.contains("good"));
        assert!(!liked.contains("meh"));
    }

    #[test]
    fn a_recipe_you_soured_on_is_not_liked() {
        let reviews = vec![review("soured", 5), review("soured", 1)];

        assert!(!liked_recipe_ids(&reviews, Some(VIEWER)).contains("soured"));
        assert!(suppressed_recipe_ids(&reviews, Some(VIEWER)).contains("soured"));
    }

    #[test]
    fn liked_and_suppressed_are_always_disjoint() {
        let reviews = vec![
            review("loved", 5),
            review("soured", 5),
            review("soured", 1),
            review("hated", 1),
            review("meh", 3),
            review("mixed-good", 4),
            review("mixed-good", 3),
        ];

        let liked = liked_recipe_ids(&reviews, Some(VIEWER));
        let suppressed = suppressed_recipe_ids(&reviews, Some(VIEWER));

        assert!(
            liked.is_disjoint(&suppressed),
            "a recipe must never be both liked and suppressed: {:?}",
            liked.intersection(&suppressed).collect::<Vec<_>>()
        );
        assert!(liked.contains("mixed-good"));
    }

    #[test]
    fn pre_auth_reviews_count_as_the_local_users() {
        let reviews = vec![
            Review {
                user_id: None,
                ..review("loved", 5)
            },
            Review {
                user_id: None,
                ..review("hated", 1)
            },
        ];

        assert!(liked_recipe_ids(&reviews, None).contains("loved"));
        assert!(suppressed_recipe_ids(&reviews, None).contains("hated"));
    }
}
