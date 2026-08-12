"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { fetchRecommendedRecipes, type RecommendedRecipe } from "@/lib/recipesApi";
import { fetchItems } from "@/lib/fridgeApi";
import { fetchShoppingList } from "@/lib/shoppingListApi";
import { LikedRecipesSection } from "./LikedRecipesSection";
import { RecipeCard } from "./RecipeCard";
import { RecipeFilterBar } from "./RecipeFilterBar";

export default function RecipesPage() {
  const [allRecipes, setAllRecipes] = useState<RecommendedRecipe[]>([]);
  const [recipes, setRecipes] = useState<RecommendedRecipe[]>([]);
  const [availableIngredients, setAvailableIngredients] = useState<Set<string>>(new Set());
  const [cuisine, setCuisine] = useState("");
  const [mealType, setMealType] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Unfiltered fetch, once — used only to populate the filter dropdowns with cuisines and
  // meal types that actually appear in the catalog, rather than a hardcoded list.
  useEffect(() => {
    fetchRecommendedRecipes()
      .then(setAllRecipes)
      .catch(() => {
        // The main fetch below surfaces the error; this one is best-effort.
      });
  }, []);

  // Fridge + shopping-list contents, once — lets each card highlight which of its
  // ingredients you already have. Mirrors the backend's "fridge or shopping list counts as
  // available" rule (recommend_recipes.rs) so the highlighting agrees with the
  // matched/total count the card already shows.
  useEffect(() => {
    Promise.all([fetchItems(), fetchShoppingList()])
      .then(([fridgeItems, shoppingListItems]) => {
        setAvailableIngredients(
          new Set([
            ...fridgeItems.map((item) => item.canonical_name.toLowerCase()),
            ...shoppingListItems.map((item) => item.name.toLowerCase()),
          ]),
        );
      })
      .catch(() => {
        // Best-effort — cards just show no highlights if this fails.
      });
  }, []);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);

    fetchRecommendedRecipes({ cuisine: cuisine || undefined, mealType: mealType || undefined })
      .then((data) => {
        if (cancelled) return;
        setRecipes(data);
        setError(null);
      })
      .catch(() => {
        if (cancelled) return;
        setError("Couldn't reach the fridge API. Is the backend running on :8080?");
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [cuisine, mealType]);

  const cuisines = [...new Set(allRecipes.flatMap((r) => r.recipe.cuisine_tags))].sort();
  const mealTypes = [...new Set(allRecipes.flatMap((r) => r.recipe.meal_type_tags))].sort();

  return (
    <div className="mx-auto max-w-2xl px-4 py-10">
      <h1 className="text-xl font-semibold">Recipes</h1>
      <p className="mt-1 text-sm opacity-70">
        <Link href="/fridge" className="underline underline-offset-4">
          Fridge
        </Link>
        {" · "}
        <Link href="/fridge/shopping-list" className="underline underline-offset-4">
          Shopping list
        </Link>
        {" · "}
        <Link href="/fridge/recipes/reviews" className="underline underline-offset-4">
          Review history
        </Link>
        {" · Recommended from what you have."}
      </p>

      <section className="mt-8">
        <h2 className="text-sm font-semibold opacity-80">Recipes you liked</h2>
        <LikedRecipesSection />
      </section>

      <div className="mt-8">
        <RecipeFilterBar
          cuisines={cuisines}
          mealTypes={mealTypes}
          cuisine={cuisine}
          mealType={mealType}
          onCuisineChange={setCuisine}
          onMealTypeChange={setMealType}
        />
      </div>

      {error && <p className="mt-6 text-sm text-red-600">{error}</p>}

      {loading ? (
        <p className="mt-6 text-sm opacity-60">Loading…</p>
      ) : recipes.length === 0 && !error ? (
        <p className="mt-6 text-sm opacity-60">No recipes to show yet.</p>
      ) : (
        <ul className="mt-6 space-y-3">
          {recipes.map((recommended) => (
            <RecipeCard
              key={recommended.recipe.id}
              recommended={recommended}
              availableIngredients={availableIngredients}
            />
          ))}
        </ul>
      )}
    </div>
  );
}
