const API_BASE = process.env.NEXT_PUBLIC_FRIDGE_API_URL ?? "http://127.0.0.1:8080";

export type RecipeIngredient = {
  name: string;
  measure: string;
};

export type Recipe = {
  id: string;
  name: string;
  cuisine_tags: string[];
  meal_type_tags: string[];
  /** Always null for now — TheMealDB has no cook-time field, see backend data README. */
  cook_time_minutes: number | null;
  /** Best-effort, keyword-derived from the recipe instructions. Not a verified list. */
  required_appliances: string[];
  fridge_ingredients: RecipeIngredient[];
  extra_ingredients: RecipeIngredient[];
  image_url: string | null;
  /** Free-text, straight from TheMealDB. Quality varies — some are one line. */
  instructions: string;
};

export type RecommendedRecipe = {
  recipe: Recipe;
  matched_ingredient_count: number;
  total_ingredient_count: number;
};

export type RecipeQuery = {
  cuisine?: string;
  mealType?: string;
};

/**
 * Ranked, filtered recipes from the vendored TheMealDB catalog. Always returns `[]` until
 * the backend's `recommend_recipes` ([learn]) is implemented — the endpoint and filtering
 * plumbing work, but the ranking/filtering body is a stub. That's expected, not a bug.
 */
export async function fetchRecommendedRecipes(query: RecipeQuery = {}): Promise<RecommendedRecipe[]> {
  const params = new URLSearchParams();
  if (query.cuisine) params.set("cuisine", query.cuisine);
  if (query.mealType) params.set("mealType", query.mealType);

  const search = params.toString();
  const res = await fetch(`${API_BASE}/recipes/recommended${search ? `?${search}` : ""}`, {
    cache: "no-store",
  });
  if (!res.ok) throw new Error(`Failed to fetch recommended recipes: ${res.status}`);
  return res.json();
}
