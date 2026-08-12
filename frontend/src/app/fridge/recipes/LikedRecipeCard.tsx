import type { Recipe } from "@/lib/recipesApi";

/**
 * Simpler than `RecipeCard` on purpose — `GET /recipes/liked` returns bare `Recipe`s, not
 * `RecommendedRecipe`s, so there's no matched-ingredient count to show here.
 */
export function LikedRecipeCard({ recipe }: { recipe: Recipe }) {
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
        <p className="font-medium">{recipe.name}</p>
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
