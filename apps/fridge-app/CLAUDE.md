# fridge-app — status & notes

Repo-wide rules (Learning Mode, phase discipline) live in the root `CLAUDE.md`. This file
tracks fridge-app-specific state so a new session doesn't have to rediscover it.

## Current status: Phases 1–3 complete

Phase 1 (fridge CRUD, NLP item matching, expiration estimates), Phase 2 (shopping list,
purchase history, purchase-based recommendations), and Phase 3 (recipe recommendations) are
all done. All four `[learn]` pieces — `nlp.rs`, `expiration.rs`, `recommend.rs`,
`recommend_recipes.rs` — were implemented by the user, with Claude reviewing rather than
writing them. `cargo test`: 53 passed, 0 failed, clippy clean. Verified working end-to-end
in-browser, backend and frontend, including against real fridge/shopping-list/catalog data
(not just the hand-built test fixtures).

`PATCH /items/:id` was decided against, not deferred — nothing in Phase 1 or 2 ended up
needing it. Revisit only if a real caller shows up.

`recommend_recipes`'s final formula: hard-filters on cuisine/meal-type, then sorts by a
3-key tuple — (a) whether `total_ingredient_count < 2` (trivial recipes sort last as a
group), (b) missing-ingredient count ascending within each group, (c) total-ingredient-count
descending as a tie-break within *that*. (b)+(c) together are what stop a 1-ingredient
recipe you happen to have from outranking a real dinner you're almost fully stocked for;
(a) is what stops a recipe whose every ingredient turned out to be a pantry staple (e.g.
"Griddled flatbreads" — flour ×2, yeast, sugar, olive oil, `total_ingredient_count == 0`)
from reading as a trivially "perfect" match. Two real bugs surfaced and got fixed while
building this — a `bool`'s `Ord` puts `false` before `true` in ascending order, and it's
easy to write the trivial-recipe key backwards (promoting instead of demoting) without
noticing, since none of the six hand-built tests mix trivial and non-trivial recipes in the
same ordering assertion. Both times the bug was invisible to `cargo test` and obvious
against the real 789-recipe catalog — caught by re-running the real-data check after the
change, not by the test suite. Data source: TheMealDB, vendored one-time snapshot (789
recipes, fetched 2026-08-10) — see `docs/PLAN.md` Phase 3 for why, and
`data/themealdb/README.md` for field-mapping details.

**Separate observation, not a bug (see `docs/TODO.md`):** exact-name ingredient matching
means singular/plural mismatches between fridge items and TheMealDB ingredient names
silently miss — e.g. a fridge item "tomatoes" matches the 21 recipes using `Tomatoes` but
not the 47 using singular `Tomato`.

## Backend (`apps/fridge-app/backend/`)

Rust, axum, sqlx (SQLite, file `fridge.db`, gitignored). Migrations in `migrations/`. Run:
`cargo run` (binds `0.0.0.0:8080`, see LAN note below). Test: `cargo test`.

- `src/models.rs` — all request/response/DB-row structs (`FridgeItem`, `ShoppingListItem`,
  `PurchaseHistory`, etc). `id` fields are `String` (UUID text), not `uuid::Uuid` — sidesteps
  sqlx's BLOB-based Uuid encoding mismatch with TEXT columns.
- `src/routes/items.rs` — fridge CRUD. `upsert_fridge_item` is the single place that both
  inserts/merges a fridge row *and* logs to `purchase_history` — called by `POST /items`
  directly and by `shopping_list::mark_purchased` for grocery items, so a purchase is never
  logged twice no matter which flow produced it. Merge-on-add requires matching name + unit
  + expiration within `MERGE_EXPIRATION_TOLERANCE_DAYS`; see `find_merge_target`'s doc
  comment for the tolerance/tie-break rules.
- `src/routes/shopping_list.rs` — shopping-list CRUD + `POST /:id/purchase` (the unified
  trigger above) + `GET /suggestions` (calls `recommend::suggest_shopping_items`).
  `POST /shopping-list` also merges on add (same name + unit, still `pending`); a purchased
  row never absorbs a new add.
- `src/routes/suggest.rs` — item-name typeahead, calls `nlp::suggest_item_names`.
- `src/nlp.rs` — **[learn] implemented.** Banded-tier fuzzy/prefix/substring matcher; its
  module doc is the reference for the scoring bands.
- `src/expiration.rs` — **[learn] implemented.** FoodKeeper-CSV-backed shelf-life lookup.
- `src/recommend.rs` — **[learn] implemented.** `suggest_shopping_items`: an expiring-soon
  filter on `fridge`, plus a frequency signal (group `history` by item, median gap via
  `calculate_mad`, suggest if absent from `fridge` and overdue). `calculate_mad` currently
  computes the median gap, not full MAD, and hasn't been checked against real purchase data
  yet — only the hand-built test fixtures. Worth a real-data pass before trusting it fully.
- `src/purchase_history.rs`, `src/foodkeeper.rs` — straightforward; see their own doc
  comments.
- `data/foodkeeper/README.md` — **read before touching either FoodKeeper-parsing module.**
  Documents provenance and real data-shape traps (tagged-union `_Metric` fields, prose in
  integer columns, non-unique `Name`, etc).
- `src/routes/recipes.rs` — `GET /recipes/recommended?cuisine=&mealType=`, calls
  `recommend_recipes::recommend_recipes` against the static `Recipe` catalog plus current
  fridge/shopping-list contents (fetched via `items::fetch_all` /
  `shopping_list::fetch_all`).
- `src/themealdb.rs` — parses the vendored `data/themealdb/meals.json` into `Vec<Recipe>`
  at startup, same embed-at-compile-time pattern as `foodkeeper.rs`. Also where
  `required_appliances` (keyword scan over instructions) and the `fridge_ingredients` /
  `extra_ingredients` split (pantry-staple keyword list) are derived — both are documented
  heuristics, not structured facts; see its module doc and `data/themealdb/README.md`
  before adjusting either keyword list. `Recipe.instructions` is `strInstructions`
  trimmed and passed through unprocessed — the one field here that's a straight pass-through
  rather than a heuristic, since every one of the 789 records has a real (if occasionally
  one-line) value.
- `src/recommend_recipes.rs` — **[learn] implemented.** `recommend_recipes` hard-filters on
  cuisine/meal-type first, then sorts by (trivial-recipe flag, missing-ingredient count
  ascending, total-ingredient-count descending) — see "Current status" above for the full
  formula and the two real bugs the real-data check caught along the way.
  `RecipeFilters`/`RecommendedRecipe` live here rather than `models.rs`, same reasoning as
  `Suggestion`/`SuggestionReason` living in `recommend.rs`.
- `data/themealdb/README.md` — **read before touching `src/themealdb.rs`.** Field-mapping
  decisions (why `strCountry` not `strArea`, why `cook_time_minutes` is always `None`) and
  the appliance/pantry-staple keyword heuristics, including known false-positive risks.

## Frontend (`frontend/src/app/fridge/`)

Next.js 16.2.12. `frontend/AGENTS.md` requires reading `node_modules/next/dist/docs/` before
writing frontend code — this version has breaking changes vs. older Next.

- `page.tsx` / `lib/fridgeApi.ts` — fridge tab. `ItemNameCombobox.tsx` deliberately never
  preselects a suggestion (`activeIndex` starts at -1, resets every keystroke) — Enter
  commits the literal typed text unless the user arrows onto a suggestion first. Don't
  "helpfully" change that.
- `shopping-list/` / `lib/shoppingListApi.ts` — shopping-list sub-route of the Fridge tab
  (not a standalone nav tab — `apps/fridge-app` is one tab in the site's philosophy).
  Deliberately separate type names from `fridgeApi.ts` to avoid collisions.
- `GroceryListPopup.tsx` — sticky-note-styled popup on the fridge page for a quick-glance
  view of the pending shopping list.
- `recipes/` / `lib/recipesApi.ts` — recipes sub-route, same "sub-route of the Fridge tab"
  philosophy as `shopping-list/`. `page.tsx` fetches the full catalog unfiltered once (to
  populate the cuisine/meal-type `<select>` options in `RecipeFilterBar.tsx` from whatever
  actually exists in the data, not a hardcoded list) plus separately on every filter
  change. Verified in-browser with real ranked results and real filter values (not mock
  data) once `recommend_recipes` was implemented — filtering to a cuisine correctly narrows
  the set *and* preserves the missing-ingredient ranking within it.
- `RecipeCard.tsx` — `"use client"`, holds its own `showInstructions` toggle state per card.
  `recipe.instructions` renders collapsed behind a "Show instructions" button by default —
  789 recipes averaging ~840 characters of instructions each is too much to show inline on
  every card at once. `whitespace-pre-line` preserves the source text's line breaks
  (paragraphs, numbered steps) without needing to parse or split it.

## Environment gotchas

- Turbopack tried inferring the workspace root as `~/Documents` (a stray `bun.lock` in the
  home dir confused it). Fixed via `turbopack.root` in `frontend/next.config.ts` — check
  there first if `TurbopackInternalError: reading dir` reappears.
- A `next dev` server and/or `cargo run` backend are often already running from a previous
  session. Check `lsof -ti tcp:3000` / `tcp:8080` before starting another.
- Fridge tab hanging forever on load (not erroring, just an endless spinner) usually means
  it was loaded via the wrong URL — e.g. a LAN "Network" URL opened from the same machine
  that printed it.
- LAN access: backend binds `0.0.0.0` on purpose; `frontend/.env.local` (gitignored) points
  `NEXT_PUBLIC_FRIDGE_API_URL` at the host's LAN IP for other-device testing. That IP is
  DHCP-assigned — re-check with `ipconfig getifaddr en0` if it stops working. No auth yet
  (Phase 5), so this is fine on a trusted network only.
- Repo root `.claude/launch.json` (`cwd: frontend`) is this project's dev-preview config; an
  unrelated `/Users/jesseli/projects/meal/.claude/launch.json` also exists — not this
  project, don't be misled by it.

## Open technical debt

- `foodkeeper_product_id` on a collapsed name (e.g. `Ham`, which collapses 20 CSV rows) is
  just the first row's id, not disambiguated. Needs `Name_subtitle` handling to fix properly
  (FoodKeeper README gotcha 6).
- `expiration.rs` re-parses the FoodKeeper CSV independently of `foodkeeper.rs` (its own
  `PRODUCTS_CSV` + `FoodKeeperRow`), instead of reusing the `Catalog` already loaded once
  into `AppState`. Worth reconciling.
- `calculate_mad` in `recommend.rs` — see backend section above.
- `themealdb.rs`'s `required_appliances` and `fridge_ingredients`/`extra_ingredients` split
  are keyword heuristics over free text, not structured data — see its module doc and
  `data/themealdb/README.md`. Not a bug, but don't build anything downstream that assumes
  they're exact.
- `data/themealdb/meals.json` is a 2026-08-10 snapshot; TheMealDB adds recipes over time.
  Re-run the letter-sweep fetch (see the README) if the catalog starts feeling stale.

## Working patterns from Phase 1/2 scoring bugs

Relevant again for Phase 3's `recommend_recipes` and Phase 4's `rerank_recommendations` —
both are `[learn]` scoring functions too. In short: enforce numeric invariants by
construction, name and derive constants instead of repeating magic literals, give any
continuous score an explicit threshold, don't trust passing tests without checking against
real data, and check a branch is reachable before writing it. Full detail is in `nlp.rs` /
`recommend.rs` git history if wanted later.

## Git / GitHub

Repo root, branch `main`, remote `origin` →
`git@github.com:terrificjesse/personal-website.git` over SSH (`~/.ssh/id_ed25519`).

## Next up

Phases 1–3 are all complete, no open items in `recommend_recipes`. Phase 4 (review system +
learned re-ranking) is next in `docs/PLAN.md`, but per this repo's phase-discipline rule,
don't scaffold it until asked. `docs/TODO.md` holds three deferred ideas (out-of-order token
matching in `nlp.rs`, quantity/measure-aware ingredient matching, singular/plural
ingredient-name matching); nothing there blocks Phase 4.
