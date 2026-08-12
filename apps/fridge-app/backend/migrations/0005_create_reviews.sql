-- One row per cook-and-review event. `recipe_id` references TheMealDB's `idMeal` (see
-- `Recipe.id` in src/themealdb.rs), not a foreign key — recipes are static reference data
-- loaded from data/themealdb/meals.json at startup, not a DB table. Re-cooking and
-- re-rating the same recipe is expected, so this is a history, not one row per recipe.
CREATE TABLE reviews (
    id TEXT PRIMARY KEY NOT NULL,
    recipe_id TEXT NOT NULL,
    rating INTEGER NOT NULL,
    cooked_at TEXT NOT NULL,
    notes TEXT
);
