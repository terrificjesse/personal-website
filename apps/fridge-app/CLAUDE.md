# fridge-app — status & notes

Repo-wide rules (Learning Mode, phase discipline) live in the root `CLAUDE.md`. This file
tracks fridge-app-specific state so a new session doesn't have to rediscover it.

## Current status: Phases 1–3 complete; Phase 4 scaffolded, `rerank_recommendations` open

Phase 1 (fridge CRUD, NLP item matching, expiration estimates), Phase 2 (shopping list,
purchase history, purchase-based recommendations), and Phase 3 (recipe recommendations) are
all done. All four `[learn]` pieces — `nlp.rs`, `expiration.rs`, `recommend.rs`,
`recommend_recipes.rs` — were implemented by the user, with Claude reviewing rather than
writing them. Verified end-to-end in-browser against real fridge/shopping-list/catalog data,
not just hand-built test fixtures.

Phase 4's `[gen]` surface — `Review` data model, `POST /reviews`, `GET /reviews`, `GET
/recipes/liked`, and the frontend (review form on each recipe card, review history page,
"Recipes you liked" section) — was scaffolded by Claude and verified end-to-end in-browser:
submitted a real review through the UI, confirmed it shows up on `GET /reviews`/the review
history page with the right recipe name/image joined in, and confirmed `GET /recipes/liked`
correctly identifies the reviewed recipe as liked (rating ≥ `LIKED_RATING_THRESHOLD`) but
currently returns `[]` because `rerank_recommendations` — the one `[learn]` piece — is still
the unimplemented stub. `cargo test`: 68 passed, 6 failed, clippy clean; the 6 failures are
`rerank.rs`'s tests, expected to fail until the user implements it (same situation
`recommend_recipes.rs`'s tests were in before Phase 3 was implemented). Suppression on the
general path was verified against the real catalog, not just fixtures: rating a recipe 1★
drops it from `GET /recipes/recommended` (789 → 788) while preserving Phase 3's ingredient
ordering, 3★ leaves it in place, and 2★ removes it — the threshold behaves exactly at its
boundary. The liked-set half of the disjointness fix is covered by unit tests rather than
end-to-end, since the `rerank` stub returns `[]` and masks any membership change at the
endpoint; re-check it in-browser once the function is real.

### How the three review-driven behaviors are split across the code

Settled 2026-08-12 after the original scaffolding put all three inside
`rerank_recommendations`, which turned out to contradict how the endpoints actually work.
Four of its six tests described inputs the live caller could never produce. The fix
("option A") narrowed the function and moved suppression out:

| Behavior | Lives in | Kind |
|---|---|---|
| Membership — is this "liked"? | `routes/recipes.rs::liked_recipe_ids`, `LIKED_RATING_THRESHOLD` | `[gen]` threshold |
| Suppression — drop the disliked | `routes/recipes.rs::suppressed_recipe_ids`, `SUPPRESSED_RATING_THRESHOLD` (≤2, no decay) | `[gen]` filter |
| Ordering within the liked set | `rerank_recommendations` | **`[learn]`** |
| Throwback selection + interleaving | `rerank_recommendations` | **`[learn]`** |

**Suppression takes precedence over liking, and the two sets are disjoint by construction.**
`liked_recipe_ids` subtracts `suppressed_recipe_ids`. This is not cosmetic: under the
multi-review model a recipe rated 5★ once and 1★ later satisfies both raw thresholds, so
without the precedence it would appear in "Recipes you liked" *while* being hidden from
general recommendations. `liked_and_suppressed_are_always_disjoint` pins the invariant.

Both helpers scope to the viewer's own reviews internally (`Review::is_by`) rather than
trusting callers to pre-filter — `liked` passes them the wider `fetch_visible_to` set, and
getting that wrong is how a stranger's rating would leak into your personal sections.

`rerank_recommendations` now **orders and never drops** — its output must be a permutation of
its input. Suppression sits on the *general* recommendations path instead, which is what
PLAN.md's Phase 4 checkpoint actually asks for ("rate one poorly, confirm it drops out of
general recommendations"). A filter composes with Phase 3's ingredient ranking; a competing
ordering would fight it. It's applied to `recommend_recipes`'s *output* rather than its input
to avoid cloning all 789 catalog recipes per request.

Note the input asymmetry that makes the `[learn]` part non-trivial: only `candidates` is
pre-filtered. The `reviews` slice is the whole visible history — so a candidate can carry a
mediocre 3★ alongside its qualifying 5★, and mixed history is a real case rather than a
hypothetical. (Anything ≤2★ can *not* reach it, since that suppresses the recipe outright —
which is why `rerank.rs`'s mixed-history test uses a 3★.)

### Throwbacks (user's design, 2026-08-12)

An aggressive half-life makes the base ranking feel current but buries old favourites, which
works against PLAN.md's stated goal of liked recipes *resurfacing*. Rather than compromising
the decay constant to serve both goals badly, the user's design keeps a short half-life and
adds a **separate mechanism**: eligible old favourites are moved into fixed slots and
labelled, so the ranking answers "what am I into lately" while the throwbacks answer "what
did I love and forget."

Scaffolded (`[gen]`, done): `RankedRecipe { recipe, reason }` + `RankReason::{Liked,
Throwback}` in `rerank.rs`, the `liked` handler's return type, the frontend badge, and three
constants — `THROWBACK_SLOTS = [3, 5, 7]` (0-based), `THROWBACK_MIN_MEAN_RATING = 4.0`,
`THROWBACK_MIN_AGE_DAYS = 90.0`. All carry `#[allow(dead_code)]` until the body uses them;
those attributes are meant to be deleted as each is consumed.

Selection and interleaving are the user's (`[learn]`). Three things settled while designing it:

- **Both gates are required.** Mean rating alone makes "throwback" mean "good recipe," and
  current favourites get badged as nostalgia. The age gate is what makes the concept real.
- **Move, never copy.** A throwback must not also appear at its natural rank — same
  duplicate-membership bug class as liked/suppressed.
- **Randomness needs a seed.** The user wants random selection among eligible recipes;
  `thread_rng` would reshuffle the list on every page load and make the behavior untestable.
  Seeding from the current date gives daily rotation, session stability, and pinnable tests.
  Hashing `(recipe_id, seed)` from `std` avoids adding the `rand` crate — **ask before adding
  it.**

Why the existing small ordering tests are unaffected: `THROWBACK_SLOTS` starts at index 3, so
nothing is injected into lists shorter than four, and every other fixture has two or three
candidates. `throwbacks_land_only_in_their_slots_and_only_when_eligible` uses ten. It asserts
structural invariants (throwbacks only at slot indices; only eligible recipes labelled)
rather than naming a winner, since random selection makes any specific outcome flaky.

### The multi-review model is deliberate — do not "simplify" it

`reviews` is an append-only history: one row per cooking event, many rows per recipe, no
unique constraint. Every other table in this app merges on add, so this genuinely is the odd
one out, and it was re-examined on 2026-08-12 with a concrete proposal to switch to one
updatable review per (user, recipe) — upsert, `UNIQUE(recipe_id, COALESCE(user_id, ''))`,
which would also have made liked/suppressed disjoint for free and matched what a Phase 5
global aggregator wants (one-per-user resists vote-stuffing).

**The user declined, explicitly choosing learning value over model simplicity.** The history
model is what preserves the multi-review aggregation decision — how a series of ratings over
time collapses into one score — which is the actual `[learn]` content of Phase 4. Collapsing
it to one-review-per-recipe would reduce `rerank_recommendations` to "sort by rating, recency
as tie-break" and delete the exercise. Don't propose it again without new information.

### Global review aggregator — schema scaffolded ahead of Phase 5

Migration `0006_add_review_ownership.sql` adds `user_id` / `is_public` / `hidden` to
`reviews`, plus `GET /recipes/{id}/reviews` (public wall for one recipe) and visibility-scoped
reads. Built during Phase 4 at the user's request even though it only *activates* in Phase 5,
because retrofitting ownership onto rows that never had it is worse than carrying nullable
columns for a phase. Full Phase 5 task list is in `docs/PLAN.md` → "Global review aggregator".

Two things to understand before touching any of it:

1. **`reviews::current_viewer()` is the auth seam.** Returns `None` unconditionally — there
   are no accounts yet. Every read path already threads its result through (into the
   visibility queries and into `rerank_recommendations`), so Phase 5 replaces that one
   function body and the rest keeps working. Don't scatter `None` literals at call sites.
2. **`Review::is_by(viewer)` is the entire personal-vs-global mechanism** — deliberately just
   an id check. `viewer == None` reports *every* review as personal, which is correct
   pre-auth (single user, so every row is theirs) and makes the global half inert until
   accounts exist.

`fetch_visible_to` hands `rerank_recommendations` **both populations in one slice** (your
reviews + everyone's public ones) rather than pre-splitting them, because how to weight your
own feedback against the crowd's is the `[learn]` decision. Membership for the liked section,
by contrast, is scoped to the viewer's own reviews — a stranger's 5★ can reorder your liked
section but must never add a recipe to it.

**Small-sample rating statistics (Bayesian averaging / shrinkage) were explicitly deferred to
Phase 5** — with one user there is no crowd to average, so the problem doesn't exist yet. See
`docs/PLAN.md` Phase 5 for the reading list. Don't solve it early.

**Do not expose the review endpoints publicly before Phase 5.** The backend binds `0.0.0.0`
with no auth; until then a global review wall is an unauthenticated, unattributable write
endpoint. Trusted LAN only.

`PATCH /items/:id` was decided against, not deferred — nothing in Phase 1 or 2 ended up
needing it. Revisit only if a real caller shows up.

`recommend_recipes`'s final formula: hard-filters on cuisine/meal-type, then sorts by
(trivial-recipe flag, missing-ingredient count ascending, total-ingredient-count descending)
— trivial recipes (`total_ingredient_count < 2`, e.g. "Griddled flatbreads," whose every
ingredient turned out to be a pantry staple) sort last as a group; within a group, fewer
missing ingredients ranks higher, with size as a tie-break so a 1-ingredient match can't
outrank a near-complete real dinner. Data source: TheMealDB, vendored one-time snapshot (789
recipes, fetched 2026-08-10) — see `docs/PLAN.md` Phase 3 for why, and
`data/themealdb/README.md` for field-mapping details.

**Known limitations, not bugs (see `docs/TODO.md`):** ingredient matching is exact-name,
presence-only — no quantity/measure comparison ("2 cups flour" matches on having any flour
at all) and no singular/plural normalization (a fridge item "tomatoes" won't match a recipe
listing singular "Tomato").

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
  `shopping_list::fetch_all`), then drops recipes the viewer rated poorly via
  `suppressed_recipe_ids`. Also `GET /recipes/liked`. See "How the three review-driven
  behaviors are split" above before moving logic between here and `rerank.rs`.
- `src/themealdb.rs` — parses the vendored `data/themealdb/meals.json` into `Vec<Recipe>`
  at startup, same embed-at-compile-time pattern as `foodkeeper.rs`. Also where
  `required_appliances` (keyword scan over instructions) and the `fridge_ingredients` /
  `extra_ingredients` split (pantry-staple keyword list) are derived — both are documented
  heuristics, not structured facts; see its module doc and `data/themealdb/README.md`
  before adjusting either keyword list. `Recipe.instructions` is `strInstructions`
  trimmed and passed through unprocessed — the one field here that's a straight pass-through
  rather than a heuristic, since every one of the 789 records has a real (if occasionally
  one-line) value.
- `src/recommend_recipes.rs` — **[learn] implemented.** See "Current status" above for the
  ranking formula. `RecipeFilters`/`RecommendedRecipe` live here rather than `models.rs`,
  same reasoning as `Suggestion`/`SuggestionReason` living in `recommend.rs`.
- `data/themealdb/README.md` — **read before touching `src/themealdb.rs`.** Field-mapping
  decisions (why `strCountry` not `strArea`, why `cook_time_minutes` is always `None`) and
  the appliance/pantry-staple keyword heuristics, including known false-positive risks.
- `src/routes/reviews.rs` — `POST /reviews` (insert-only, no merge — re-cooking and
  re-rating the same recipe is a new row, a history not a current-state table), `GET /reviews`
  (the viewer's own history, joined against the in-memory recipe catalog for `recipe_name`/
  `recipe_image_url` so the frontend doesn't need a second fetch), and `GET
  /recipes/{id}/reviews` (public wall for one recipe). Three `pub(crate)` read helpers with
  deliberately different scopes — `fetch_for_viewer` (own reviews, backs the history page) vs.
  `fetch_visible_to` (own + everyone's public, feeds `rerank_recommendations`); picking the
  wrong one is how a stranger's review would leak into a personal view. `current_viewer()` is
  the Phase 5 auth seam — see "Current status".
- `src/rerank.rs` — **[learn] stub, not yet implemented.** `rerank_recommendations(candidates:
  &[Recipe], reviews: &[Review], viewer: Option<&str>) -> Vec<Recipe>`, placeholder always
  returns `[]`. Its only caller is `routes/recipes.rs::liked`. Orders, never drops. The
  `viewer` param exists so the body can tell personal from global feedback via
  `Review::is_by`; see the module doc for why those must not be pooled into one average.
  Seven tests, all describing inputs the live caller can actually produce. The four ordering
  tests were **checked against the candidate models rather than assumed** — they eliminate
  sum, mean, max, recency-weighted sum, *and* recency-weighted mean, while at least two
  distinct models satisfy all four, so the spec is constraining without forcing one answer.
  The non-obvious exclusion is recency-weighted **mean**: normalizing by total weight cancels
  a lone review's weight, so it ties a 5★ from last week against one from a year ago and
  fails `more_recent_review_breaks_a_rating_tie`. (An earlier version of this file wrongly
  recommended exactly that model — the table in `rerank.rs`'s module doc is the corrected
  reference.) `a_single_five_star_outranks_a_repeatedly_cooked_four_star` encodes the user's
  explicit preference (2026-08-12) for peak quality over cooking frequency; it's marked
  in-comment as an *option*, safe to invert.

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
  (paragraphs, numbered steps) without needing to parse or split it. Also now holds a "Mark
  cooked" toggle that reveals `ReviewForm.tsx`; once submitted the card shows a static
  "Reviewed ★★★★★" line instead (self-contained per-card state, no lifted state — submitting
  a review doesn't need to affect the rest of the page).
- `ReviewForm.tsx` — rating `<select>` (1–5, defaults to 5), optional notes input (capped at
  `MAX_NOTES_LENGTH`, mirroring the backend's 400), and a "Share publicly" checkbox that
  **defaults to unchecked**, matching the backend default. Publishing is always deliberate;
  don't flip that default. No `cooked_at` field in the UI — the backend defaults it to
  submission time; PLAN.md's model has the field for completeness and for backdating test
  fixtures via `curl`, not because the v1 form needs to set it.
- `LikedRecipesSection.tsx` / `LikedRecipeCard.tsx` — the "Recipes you liked" section on the
  recipes page, fetching `GET /recipes/liked` (now `RankedRecipe[]`). Deliberately a separate,
  simpler card rather than reusing `RecipeCard` — the liked endpoint carries no
  `matched_ingredient_count`/`total_ingredient_count`, since that's a Phase-3-specific concept
  `RecipeCard` depends on. `reason === "throwback"` renders an amber badge beside the title;
  verified in both light and dark mode against mock data, since the live endpoint returns `[]`
  until `rerank_recommendations` exists. Always shows the empty state until then, even after
  submitting a qualifying review — confirmed in-browser that this is the stub, not a wiring
  bug (see "Current status").
- `recipes/reviews/page.tsx` — review history page (`/fridge/recipes/reviews`), lists every
  `GET /reviews` row most-recently-cooked-first. Plain read-only list, no edit/delete —
  PLAN.md's Phase 4 checkpoint only asks that history be "browsable."

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

## Working patterns from Phase 1–3 scoring bugs

Relevant again for Phase 4's `rerank_recommendations` — same category of `[learn]` scoring
function. Enforce numeric invariants by construction, name and derive constants instead of
repeating magic literals, give any continuous score an explicit threshold, and check a
branch is reachable before writing it. Above all: **don't trust a green test suite without
checking against real data.** Three scoring functions in this project have now shipped
passing tests while ranking real data wrong (`nlp.rs` once, `recommend_recipes.rs` twice) —
every one of those bugs was invisible to `cargo test` and obvious the moment the real
catalog/fridge contents ran through the function. Full detail in git history if wanted.

## Git / GitHub

Repo root, branch `main`, remote `origin` →
`git@github.com:terrificjesse/personal-website.git` over SSH (`~/.ssh/id_ed25519`).

## Next up

Phase 4's `[gen]` scaffolding is done, including the Phase-5-ready review-ownership schema and
the suppression filter (see "Current status"). **The only thing left in Phase 4 is
implementing `rerank_recommendations`** in `src/rerank.rs`, against the seven tests already
there.

Scope it honestly: with one user and a pre-filtered candidate set, this is ~20–30 lines
(group `reviews` by `recipe_id`, fold each group into a score, sort). The user reached this
conclusion themselves and agreed not to stretch it — the genuinely interesting version of the
problem arrives in Phase 5 with auth, a real second population, and small-sample statistics.
Don't inflate it. The personal-vs-global branch can stay trivial until Phase 5 gives it
something to weigh.

Once it's real, worth a real-data pass in-browser (seed several reviews across a few recipes —
`POST /reviews` accepts an explicit `cooked_at`, so backdate them the way Phase 2's purchase
history was seeded — then confirm "Recipes you liked" populates and orders sensibly). Same
"don't trust a green test suite" discipline as the last three scoring functions; see "Working
patterns" below. `docs/TODO.md` holds three deferred ideas; nothing there blocks Phase 4.
