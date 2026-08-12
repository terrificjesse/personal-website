import type { Recipe } from "./recipesApi";

const API_BASE = process.env.NEXT_PUBLIC_FRIDGE_API_URL ?? "http://127.0.0.1:8080";

export type Review = {
  id: string;
  recipe_id: string;
  /** 1-5. */
  rating: number;
  cooked_at: string;
  notes: string | null;
};

export type ReviewWithRecipe = Review & {
  recipe_name: string;
  recipe_image_url: string | null;
};

export type SubmitReviewInput = {
  recipe_id: string;
  rating: number;
  notes?: string;
};

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
 * Recipes rated 4★ or higher at least once, ordered by the backend's (currently
 * unimplemented) `rerank_recommendations`. Always returns `[]` until that's implemented —
 * the endpoint and membership filtering work, but the ranking body is a stub. That's
 * expected, not a bug — same situation `fetchRecommendedRecipes` was in before
 * `recommend_recipes` was implemented.
 */
export async function fetchLikedRecipes(): Promise<Recipe[]> {
  const res = await fetch(`${API_BASE}/recipes/liked`, { cache: "no-store" });
  if (!res.ok) throw new Error(`Failed to fetch liked recipes: ${res.status}`);
  return res.json();
}
