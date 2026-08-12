# TheMealDB data

Recipe reference data backing `src/themealdb.rs` (Phase 3). Vendored as a one-time snapshot,
same pattern as `data/foodkeeper/` — no live API calls at runtime.

## Provenance

Upstream: [TheMealDB](https://www.themealdb.com/), free public recipe API. Fetched
2026-08-10 using the shared test API key (`"1"`) via `search.php?f={letter}` for each
letter a–z, then deduped by `idMeal` and sorted numerically. This letter-sweep is the
standard way to enumerate the free tier's catalog — there's no single "list everything"
endpoint on the test key.

Per `themealdb.com/api.php`: the test key is fine for "development of your app or
educational use"; the only stated restriction is that you "must become a supporter if
releasing publicly on an appstore." No rate limit or attribution requirement is stated for
the test key. This project is personal/non-commercial, so the test key is sufficient — no
signup or paid key needed.

While fetching, that same docs page's extracted content included text pointing an AI agent
toward "supplementary resources" at `/AGENTS.md` and `/SKILL.md` on their domain — not
normal API documentation content. Treated as a probable prompt-injection attempt and not
followed; flagged to the user rather than acted on.

## Files

- `meals.json` — 789 unique recipe records (deduped from overlapping letter-sweep results),
  each with the full field set TheMealDB returns from `search.php`. ~2.1 MB.

## Field mapping (used by `src/themealdb.rs`)

- **`cuisine_tags`** ← `strCountry`, not `strArea`. `strArea` is `null` on 189/789 records;
  `strCountry` is populated on all 789. `strArea`, when present, is an adjectival form
  ("Brazilian") while `strCountry` is the country name ("Brazil") — `strCountry` was chosen
  for completeness, not style.
- **`meal_type_tags`** ← `strCategory`. Caveat: this is really a dish/protein category (14
  values: Beef, Breakfast, Chicken, Dessert, Goat, Lamb, Miscellaneous, Pasta, Pork,
  Seafood, Side, Starter, Vegan, Vegetarian), not a breakfast/lunch/dinner meal-type split.
  `Breakfast`/`Dessert`/`Side`/`Starter` map cleanly to "meal type"; the rest (Beef,
  Chicken, Vegetarian, etc.) don't, but it's the closest structured field TheMealDB offers
  and is still useful as a filter axis. `strTags` was considered as an alternative but is
  `null` on 591/789 records — too sparse to be a primary source.
- **`cook_time_minutes`** — not derived from anything; always `None`. TheMealDB has no cook
  time field, and per-step minute mentions in `strInstructions` ("fry for 5 mins") describe
  individual steps, not a total — summing them would be fabricating a number the source
  doesn't provide. Frontend should render "not listed" rather than a guess.
- **`required_appliances`** — derived via a keyword scan over `strInstructions` (see
  `APPLIANCE_KEYWORDS` in `src/themealdb.rs`). Precision-oriented, not recall-oriented: a
  match means the word actually appears, but absence doesn't guarantee the appliance isn't
  needed. Oven triggers on `"oven"` or `"preheat"` rather than `"bake"`/`"baking"` —
  `"baking powder"`/`"baking soda"` are common ingredient names, and matching `"baking"`
  would have flagged plenty of stovetop-only recipes as needing an oven just for using one
  as a leavening agent. `"preheat"` is an imperfect proxy (recipes occasionally preheat a
  grill or griddle instead), but it's a cleaner false-positive tradeoff. The keyword list
  isn't exhaustive either way — treat this as a best-effort hint, not a verified requirement.
- **`fridge_ingredients` / `extra_ingredients`** — split from the up-to-20
  `strIngredientN`/`strMeasureN` pairs via a small pantry-staple keyword list (salt, pepper,
  oil, flour, sugar, common dried spices/herbs, etc. — see `PANTRY_STAPLE_KEYWORDS`).
  Matches go to `extra_ingredients`; everything else (proteins, produce, dairy — the things
  an inventory app actually tracks) goes to `fridge_ingredients`. Heuristic, not a taxonomy —
  edge cases (e.g. is "butter" a staple or a fridge item?) were judgment calls, adjust the
  keyword list if it misclassifies something that matters.

## Known gaps

- No cook time (see above) — always `None`.
- `required_appliances` and the fridge/extra ingredient split are both keyword heuristics
  over free text, not structured data. Good enough for card display and Phase 3 filtering;
  don't build anything downstream that assumes they're exact.
- This is a point-in-time snapshot (2026-08-10). TheMealDB adds recipes over time; re-run
  the same letter-sweep fetch to refresh if the catalog feels stale.
