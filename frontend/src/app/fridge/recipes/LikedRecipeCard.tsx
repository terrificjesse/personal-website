import type { RankedRecipe } from "@/lib/reviewsApi";

/**
 * Simpler than `RecipeCard` on purpose — `GET /recipes/liked` returns `RankedRecipe`s, which
 * carry no matched-ingredient count (that's a Phase 3 concept `RecipeCard` depends on).
 *
 * A `favorite` gets a badge. That label is the whole reason favorites can be mixed into
 * the ranking at all: an old recipe sitting at position 3 with no explanation reads as a
 * bug, and the same row with "Favorite" on it reads as a feature.
 */
export function LikedRecipeCard({ ranked }: { ranked: RankedRecipe }) {
  const { recipe, reason } = ranked;

  return (
    <li className="flex gap-4 rounded border border-black/10 p-4 dark:border-white/10">
      {recipe.image_url && (
        // eslint-disable-next-line @next/next/no-img-element -- external, unoptimized thumbnail from the vendored catalog
        <img
          src={recipe.image_url}
          alt=""
          className="h-16 w-16 shrink-0 rounded object-cover"
        />
      )}
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <p className="font-medium">{recipe.name}</p>
          {reason === "favorite" && (
            <span className="rounded-full bg-amber-100 px-2 py-0.5 text-[11px] font-medium text-amber-900 dark:bg-amber-400/15 dark:text-amber-300">
              Favorite
            </span>
          )}
        </div>
        {(recipe.cuisine_tags.length > 0 || recipe.meal_type_tags.length > 0) && (
          <div className="mt-1 flex flex-wrap gap-1">
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
      </div>
    </li>
  );
}
