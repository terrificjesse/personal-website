"use client";

import { useEffect, useState } from "react";
import { fetchLikedRecipes, type RankedRecipe } from "@/lib/reviewsApi";
import { LikedRecipeCard } from "./LikedRecipeCard";

/**
 * Recipes you've rated 4★ or higher, reordered by the backend's `rerank_recommendations`
 * ([learn], `src/rerank.rs`). Separate from the general recommendations below, per PLAN.md
 * Phase 4. Always empty until that's implemented and you've submitted a high rating — that's
 * expected, not a bug.
 */
export function LikedRecipesSection() {
  const [recipes, setRecipes] = useState<RankedRecipe[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetchLikedRecipes()
      .then(setRecipes)
      .catch(() => {
        // Best-effort — the main recommended list surfaces the "backend unreachable" error.
      })
      .finally(() => setLoading(false));
  }, []);

  if (loading) return null;

  if (recipes.length === 0) {
    return (
      <p className="mt-3 text-sm opacity-60">
        No liked recipes yet — rate one 4★ or higher after marking it cooked.
      </p>
    );
  }

  return (
    <ul className="mt-3 space-y-3">
      {recipes.map((ranked) => (
        <LikedRecipeCard key={ranked.recipe.id} ranked={ranked} />
      ))}
    </ul>
  );
}
