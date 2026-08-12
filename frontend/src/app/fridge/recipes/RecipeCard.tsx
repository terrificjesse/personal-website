"use client";

import { useState } from "react";
import type { RecommendedRecipe } from "@/lib/recipesApi";
import { ReviewForm } from "./ReviewForm";

export function RecipeCard({
  recommended,
  availableIngredients,
}: {
  recommended: RecommendedRecipe;
  /** Lowercased fridge/shopping-list item names, used to highlight ingredients you have. */
  availableIngredients: Set<string>;
}) {
  const { recipe, matched_ingredient_count, total_ingredient_count } = recommended;
  const [showInstructions, setShowInstructions] = useState(false);
  const [showReviewForm, setShowReviewForm] = useState(false);
  const [submittedRating, setSubmittedRating] = useState<number | null>(null);

  return (
    <li className="flex gap-4 rounded border border-black/10 p-4 dark:border-white/10">
      {recipe.image_url && (
        // eslint-disable-next-line @next/next/no-img-element -- external, unoptimized thumbnail from the vendored catalog
        <img
          src={recipe.image_url}
          alt=""
          className="h-20 w-20 shrink-0 rounded object-cover"
        />
      )}
      <div className="min-w-0 flex-1">
        <p className="font-medium">{recipe.name}</p>
        <p className="mt-0.5 text-xs opacity-60">
          {total_ingredient_count > 0
            ? `${matched_ingredient_count}/${total_ingredient_count} ingredients you have`
            : "No fridge ingredients listed"}
          {" · "}
          {recipe.cook_time_minutes !== null ? `${recipe.cook_time_minutes} min` : "Time not listed"}
        </p>

        {recipe.fridge_ingredients.length > 0 && (
          <ul className="mt-2 space-y-0.5 text-xs">
            {recipe.fridge_ingredients.map((ingredient, index) => {
              const have = availableIngredients.has(ingredient.name.toLowerCase());
              return (
                <li
                  key={`${ingredient.name}-${index}`}
                  className={have ? "font-medium text-green-700 dark:text-green-400" : "opacity-60"}
                >
                  {have ? "✓ " : "· "}
                  {ingredient.measure} {ingredient.name}
                </li>
              );
            })}
          </ul>
        )}

        {(recipe.cuisine_tags.length > 0 || recipe.meal_type_tags.length > 0) && (
          <div className="mt-2 flex flex-wrap gap-1">
            {[...recipe.cuisine_tags, ...recipe.meal_type_tags].map((tag) => (
              <span
                key={tag}
                className="rounded-full bg-black/5 px-2 py-0.5 text-[11px] dark:bg-white/10"
              >
                {tag}
              </span>
            ))}
          </div>
        )}

        {recipe.required_appliances.length > 0 && (
          <p className="mt-2 text-xs opacity-60">Needs: {recipe.required_appliances.join(", ")}</p>
        )}

        {recipe.extra_ingredients.length > 0 && (
          <p className="mt-1 text-xs opacity-60">
            Extras: {recipe.extra_ingredients.map((i) => i.name).join(", ")}
          </p>
        )}

        {recipe.instructions && (
          <div className="mt-2">
            <button
              onClick={() => setShowInstructions((prev) => !prev)}
              className="text-xs text-blue-600 hover:underline dark:text-blue-400"
            >
              {showInstructions ? "Hide instructions" : "Show instructions"}
            </button>
            {showInstructions && (
              <p className="mt-1 whitespace-pre-line text-xs opacity-80">
                {recipe.instructions}
              </p>
            )}
          </div>
        )}

        <div className="mt-2">
          {submittedRating !== null ? (
            <span className="text-xs opacity-60">
              Reviewed {"★".repeat(submittedRating) + "☆".repeat(5 - submittedRating)}
            </span>
          ) : (
            <button
              onClick={() => setShowReviewForm((prev) => !prev)}
              className="text-xs text-blue-600 hover:underline dark:text-blue-400"
            >
              {showReviewForm ? "Cancel" : "Mark cooked"}
            </button>
          )}
          {showReviewForm && submittedRating === null && (
            <ReviewForm
              recipeId={recipe.id}
              onSubmitted={(rating) => {
                setSubmittedRating(rating);
                setShowReviewForm(false);
              }}
            />
          )}
        </div>
      </div>
    </li>
  );
}
