export function RecipeFilterBar({
  cuisines,
  mealTypes,
  cuisine,
  mealType,
  onCuisineChange,
  onMealTypeChange,
}: {
  cuisines: string[];
  mealTypes: string[];
  cuisine: string;
  mealType: string;
  onCuisineChange: (value: string) => void;
  onMealTypeChange: (value: string) => void;
}) {
  return (
    <div className="flex flex-wrap items-end gap-3">
      <div className="flex flex-col">
        <label htmlFor="recipe-cuisine-filter" className="text-xs opacity-70">
          Cuisine
        </label>
        <select
          id="recipe-cuisine-filter"
          value={cuisine}
          onChange={(e) => onCuisineChange(e.target.value)}
          className="rounded border border-black/15 bg-transparent px-2 py-1.5 text-sm dark:border-white/20"
        >
          <option value="">All cuisines</option>
          {cuisines.map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
      </div>
      <div className="flex flex-col">
        <label htmlFor="recipe-meal-type-filter" className="text-xs opacity-70">
          Meal type
        </label>
        <select
          id="recipe-meal-type-filter"
          value={mealType}
          onChange={(e) => onMealTypeChange(e.target.value)}
          className="rounded border border-black/15 bg-transparent px-2 py-1.5 text-sm dark:border-white/20"
        >
          <option value="">All meal types</option>
          {mealTypes.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}
