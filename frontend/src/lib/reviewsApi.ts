import type { Recipe } from "./recipesApi";

const API_BASE = process.env.NEXT_PUBLIC_FRIDGE_API_URL ?? "http://127.0.0.1:8080";

export type Review = {
  id: string;
  recipe_id: string;
  /** 1-5. */
  rating: number;
  cooked_at: string;
  notes: string | null;
  /** Null for anything written before Phase 5 added accounts. */
  user_id: string | null;
  /** Opt-in world-readability. Private reviews still count toward your own recommendations. */
  is_public: boolean;
  /** Moderation tombstone — hidden rows never reach any read endpoint, so this is always false here. */
  hidden: boolean;
};

export type ReviewWithRecipe = Review & {
  recipe_name: string;
  recipe_image_url: string | null;
};

export type SubmitReviewInput = {
  recipe_id: string;
  rating: number;
  notes?: string;
  /** Defaults to false (private) server-side if omitted. */
  is_public?: boolean;
};

/** Matches `MAX_NOTES_LENGTH` in the backend's `models.rs`; over this the API returns 400. */
export const MAX_NOTES_LENGTH = 2000;

export async function submitReview(input: SubmitReviewInput): Promise<Review> {
  const res = await fetch(`${API_BASE}/reviews`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
  });
  if (!res.ok) throw new Error(`Failed to submit review: ${res.status}`);
  return res.json();
}

/** Full review history, most recently cooked first. */
export async function fetchReviews(): Promise<ReviewWithRecipe[]> {
  const res = await fetch(`${API_BASE}/reviews`, { cache: "no-store" });
  if (!res.ok) throw new Error(`Failed to fetch reviews: ${res.status}`);
  return res.json();
}

/**
 * Why a recipe landed where it did in the liked ranking.
 *
 * - `liked` — ranked normally, by recency-weighted rating.
 * - `favorite` — an old favourite deliberately lifted into a fixed slot, which the recency
 *   decay would otherwise have buried. These are badged: without the label an old recipe
 *   near the top just reads as a broken ranking.
 */
export type RankReason = "liked" | "favorite";

export type RankedRecipe = {
  recipe: Recipe;
  reason: RankReason;
};

/**
 * Recipes *you* rated 4★ or higher at least once, ordered by the backend's (currently
 * unimplemented) `rerank_recommendations`. Always returns `[]` until that's implemented —
 * the endpoint and membership filtering work, but the ranking body is a stub. That's
 * expected, not a bug — same situation `fetchRecommendedRecipes` was in before
 * `recommend_recipes` was implemented.
 *
 * Membership is scoped to your own reviews, so once other users exist a stranger's high
 * rating can influence the *order* here but can never add a recipe to the list.
 */
export async function fetchLikedRecipes(): Promise<RankedRecipe[]> {
  const res = await fetch(`${API_BASE}/recipes/liked`, { cache: "no-store" });
  if (!res.ok) throw new Error(`Failed to fetch liked recipes: ${res.status}`);
  return res.json();
}

/**
 * Everyone's opt-in public reviews for one recipe — the read half of the global aggregator
 * (PLAN.md Phase 5). Returns only `is_public` rows, so it excludes your own private reviews
 * too; use `fetchReviews` for your own history. Until accounts exist this only ever contains
 * reviews you marked public yourself.
 */
export async function fetchRecipeReviews(recipeId: string): Promise<Review[]> {
  const res = await fetch(`${API_BASE}/recipes/${encodeURIComponent(recipeId)}/reviews`, {
    cache: "no-store",
  });
  if (!res.ok) throw new Error(`Failed to fetch recipe reviews: ${res.status}`);
  return res.json();
}
