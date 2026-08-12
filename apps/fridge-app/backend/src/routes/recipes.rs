use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use sqlx::SqlitePool;

use crate::models::{Recipe, Review, LIKED_RATING_THRESHOLD, SUPPRESSED_RATING_THRESHOLD};
use crate::recommend_recipes::{self, RecipeFilters, RecommendedRecipe};
use crate::rerank::{self, RankedRecipe};
use crate::routes::{items, reviews, shopping_list};
use crate::themealdb::Catalog;

/// Recipes the viewer has rated at or below `SUPPRESSED_RATING_THRESHOLD` at least once.
///
/// One bad review is enough — this is "I tried it and didn't like it," which a later decent
/// night doesn't really undo. Scopes to the viewer's own reviews itself rather than trusting
/// the caller to pre-filter: a stranger disliking something must never hide it from *your*
/// recommendations, and that's too easy to get wrong at a call site handed the wider
/// `fetch_visible_to` set.
fn suppressed_recipe_ids<'a>(reviews: &'a [Review], viewer: Option<&str>) -> HashSet<&'a str> {
    reviews
        .iter()
        .filter(|review| review.is_by(viewer) && review.rating <= SUPPRESSED_RATING_THRESHOLD)
        .map(|review| review.recipe_id.as_str())
        .collect()
}

/// Recipes the viewer has rated at or above `LIKED_RATING_THRESHOLD` and has *not* since
/// soured on — the membership rule for the "Recipes you liked" section.
///
/// **Suppression takes precedence over liking.** Without that, the multi-review model lets a
/// recipe rated 5★ once and 1★ later satisfy both rules at once: it would show in "Recipes
/// you liked" while simultaneously being hidden from general recommendations. Making the two
/// sets disjoint here is what keeps that contradiction impossible rather than merely unlikely.
///
/// Note this is still a plain threshold rule, not aggregation — deciding what a *history* of
/// ratings adds up to is `rerank_recommendations`'s job, and deliberately stays there.
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

/// `GET /recipes/recommended?cuisine=&mealType=` — the full vendored catalog, filtered and
/// ranked against current fridge + shopping-list contents. See `recommend_recipes`'s module
/// doc for what the filters do and how ranking is meant to work.
///
/// Recipes the viewer has rated poorly are dropped afterwards, per PLAN.md's Phase 4
/// checkpoint. Deliberately a *filter over the output* rather than a second ranking pass:
/// suppression is independent of ingredient overlap, so it composes with Phase 3's ordering
/// instead of competing with it, and filtering the results avoids cloning all 789 catalog
/// recipes on every request just to remove a handful.
pub async fn recommended(
    State(pool): State<SqlitePool>,
    State(catalog): State<Arc<Catalog>>,
    Query(filters): Query<RecipeFilters>,
) -> Result<Json<Vec<RecommendedRecipe>>, StatusCode> {
    let fridge = items::fetch_all(&pool).await?;
    let shopping_list = shopping_list::fetch_all(&pool).await?;

    let viewer = reviews::current_viewer();
    let own_reviews = reviews::fetch_for_viewer(&pool, viewer.as_deref()).await?;
    let suppressed = suppressed_recipe_ids(&own_reviews, viewer.as_deref());

    let mut recommended = recommend_recipes::recommend_recipes(
        catalog.recipes(),
        &fridge,
        &shopping_list,
        &filters,
    );
    recommended.retain(|entry| !suppressed.contains(entry.recipe.id.as_str()));

    Ok(Json(recommended))
}

/// `GET /recipes/liked` — the "recipes you liked" section (Phase 4), separate from the
/// general recommendations above. Membership is a plain threshold (`LIKED_RATING_THRESHOLD`:
/// has the user rated this recipe highly at least once?); ordering among that set is
/// `rerank::rerank_recommendations`'s job. See its module doc for why the split is drawn
/// there.
///
/// Membership is deliberately scoped to the viewer's *own* reviews (`Review::is_by`) — this
/// section answers "what did **you** like," so a stranger's 5★ must never put a recipe here.
/// The `reviews` slice handed to `rerank_recommendations` is the wider set (your reviews
/// plus everyone's public ones), so the crowd can still inform *ordering* even though it
/// can't affect *membership*.
pub async fn liked(
    State(pool): State<SqlitePool>,
    State(catalog): State<Arc<Catalog>>,
) -> Result<Json<Vec<RankedRecipe>>, StatusCode> {
    let viewer = reviews::current_viewer();
    let visible_reviews = reviews::fetch_visible_to(&pool, viewer.as_deref()).await?;
    let liked = liked_recipe_ids(&visible_reviews, viewer.as_deref());

    let candidates: Vec<Recipe> = catalog
        .recipes()
        .iter()
        .filter(|recipe| liked.contains(recipe.id.as_str()))
        .cloned()
        .collect();

    Ok(Json(rerank::rerank_recommendations(
        &candidates,
        &visible_reviews,
        viewer.as_deref(),
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
        let reviews = vec![review("hated", 1), review("disliked", 2), review("loved", 5)];

        let suppressed = suppressed_recipe_ids(&reviews, Some(VIEWER));

        assert!(suppressed.contains("hated"));
        assert!(suppressed.contains("disliked"));
        assert!(!suppressed.contains("loved"));
    }

    #[test]
    fn a_middling_rating_does_not_suppress() {
        // 3★ is the boundary — "fine, not amazing" shouldn't hide a recipe you're well
        // stocked for. Guards against the threshold quietly drifting to `< 4`.
        let reviews = [review("middling", 3)];

        assert!(suppressed_recipe_ids(&reviews, Some(VIEWER)).is_empty());
    }

    #[test]
    fn one_bad_review_suppresses_despite_a_later_good_one() {
        // Documents the deliberate choice in `suppressed_recipe_ids`: any single bad review
        // is enough, rather than an average across the recipe's history.
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
        // "Recipes you liked" answers what *you* liked. The crowd can reorder that section
        // (via `rerank_recommendations`) but must never add to it.
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
        // The whole point of routing membership through `liked_recipe_ids`: under the
        // multi-review model a 5★ and a later 1★ both sit in the history, and treating
        // "liked" as "any review >= 4" would put this recipe in the liked section *and* hide
        // it from general recommendations at the same time.
        let reviews = vec![review("soured", 5), review("soured", 1)];

        assert!(!liked_recipe_ids(&reviews, Some(VIEWER)).contains("soured"));
        assert!(suppressed_recipe_ids(&reviews, Some(VIEWER)).contains("soured"));
    }

    #[test]
    fn liked_and_suppressed_are_always_disjoint() {
        // The invariant behind the test above, stated directly so it survives changes to
        // either threshold.
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
        // A merely-mediocre extra review shouldn't cost a recipe its place.
        assert!(liked.contains("mixed-good"));
    }

    #[test]
    fn pre_auth_reviews_count_as_the_local_users() {
        // viewer == None, user_id == NULL: the configuration the app runs in today.
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
