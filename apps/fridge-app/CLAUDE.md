# fridge-app — status & notes

Repo-wide rules (Learning Mode, phase discipline) live in the root `CLAUDE.md`. This file
tracks fridge-app-specific state so a new session doesn't have to rediscover it.

## Current status: Phases 1–4 complete

Phases 1–4 are all done. All five `[learn]` pieces — `nlp.rs`, `expiration.rs`,
`recommend.rs`, `recommend_recipes.rs`, `rerank.rs` — were implemented by the user, with
Claude reviewing rather than writing them. `cargo test`: **75 passed, 0 failed**, clippy clean.

Phase 4's `[gen]` surface (`Review` model, `POST`/`GET /reviews`, `GET /recipes/liked`, `GET
/recipes/{id}/reviews`, review form, review-history page, "Recipes you liked" section with
the Favorite badge) was scaffolded by Claude; `rerank_recommendations` and its two helpers
are the user's.

**`rerank_recommendations`'s final model:** score each recipe as the **max** over its reviews
of `(rating - NEUTRAL_RATING) × 0.5^(age_days / DECAY_HALFLIFE)`, sort descending, then move
up to three eligible recipes into `FAVORITE_SLOTS`. Centering before decaying is what stops
an old rave from reading as a bad review (on the raw scale a decayed rating shrinks toward 0,
*below* the scale); max rather than sum is what makes one 5★ beat three 4★s, per the user's
stated preference. `DECAY_HALFLIFE = 120.0`, chosen by the user; verified none of the four
ordering tests pin it (60/120/180 all pass), so it's free to tune.

**Verified against real data, not just fixtures** (seeded via `POST /reviews` with backdated
`cooked_at` — 16 reviews across 10 recipes, still in `fridge.db`):

- The base ranking reproduced the model's predicted order **exactly** across all 8 liked
  recipes.
- Suppression: 789 → 787 on `GET /recipes/recommended`; a 5★-then-1★ recipe is correctly
  absent from *both* the general list and the liked list (the disjointness rule, which had
  only been unit-tested while the stub masked it).
- Favorites rotate between requests, always land at slots 3 and 5, and `n == unique == 8`
  every time.
- In-browser: 8 cards, amber "Favorite" badge on exactly indices 3 and 5, no console errors.

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
| Favorite selection + interleaving | `rerank_recommendations` | **`[learn]`** |

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

### Favorites — was "throwbacks" until 2026-08-13

Recency decay makes the base ranking feel current but also flattens it into the same order
every visit. The user's design adds a **separate mechanism**: highly-rated recipes are moved
into fixed slots and badged, so the section cycles through your best recipes rather than
replaying one recency-sorted list.

**Renamed from "throwback."** The original design gated eligibility on *age* — rescuing old
favourites the decay had buried. The user dropped that gate on 2026-08-13: recent recipes are
welcome too, the goal being rotation, not nostalgia. The name went with it, because an amber
"Throwback" badge on something cooked last week is simply false. `RankReason::Favorite`,
`FAVORITE_*`, `is_favorite_eligible`, `interleave_favorites`, badge text "Favorite."

`RankedRecipe { recipe, reason }` + `RankReason::{Liked, Favorite}` live in `rerank.rs`, with
`FAVORITE_SLOTS = [3, 5, 7]` (0-based) and `FAVORITE_MIN_MEAN_RATING = 4.0`.
(`FAVORITE_MIN_AGE_DAYS` existed briefly and was deleted with the age gate.) The user added
`rand = "0.10.2"` for selection and uses `IndexedRandom::sample`, which draws *without*
replacement — the right primitive, since `choose` in a loop would put the same recipe in
several slots.

How it works, and the constraints that shaped it:

- **Two gates, but only one of them lives in `is_favorite_eligible`.** That function does the
  quality half (unweighted mean ≥ `FAVORITE_MIN_MEAN_RATING`, *inclusive* — ratings are
  integers, so a strict `>` silently excludes every recipe rated exactly 4★). The other half
  is a **rank gate** and cannot live there, since one recipe's reviews say nothing about its
  position: a recipe already ranked above `FAVORITE_SLOTS[0]` must not be selected, because
  "moving" it into a slot pushes it *down*. The old age gate made that impossible for free —
  old favourites always sat near the bottom — so dropping it created the need for an explicit
  positional check in `rerank_recommendations`.
- **Don't decay the eligibility mean.** It's deliberately time-independent; weight it by
  recency and it collapses into the base ranking, selecting whatever was already on top.
- **Move, never copy.** A favorite must not also appear at its natural rank — same
  duplicate-membership bug class as liked/suppressed.
- **Selection must be capped at what can be *placed*, not at `FAVORITE_SLOTS.len()`.** This
  is the subtle one, and it cost a real bug (see "Working patterns"). A chosen favorite is
  *removed* from the ranking before being re-inserted, so a slot that doesn't fit doesn't
  leave the recipe alone — it loses it. With `n` candidates and `k` chosen, slot `j` is
  reachable only if `FAVORITE_SLOTS[j] < n - k + j`; `rerank_recommendations` searches
  downward from `min(3, promotable)` for the largest workable `k`. Concretely, **8 candidates
  hold two favorites, not three** — the third would need a list of 9 to reach index 7.
- **Randomness is currently unseeded** (`rand::rng()`). It works, and the rotation is visible
  between requests, but the list reshuffles on every page load and a surprising ranking can't
  be reproduced while debugging. `StdRng::seed_from_u64` with a date-derived seed would give
  daily rotation, session stability, and pinnable tests. Left as the user's call.

Why the small ordering tests are unaffected: `FAVORITE_SLOTS` starts at index 3, so nothing is
injected into lists shorter than four, and every other fixture has two or three candidates.
`favorites_land_only_in_their_slots_and_only_when_eligible` uses eleven, with distinct ages so
the base order is deterministic rather than relying on stable-sort tie behaviour. It asserts
structural invariants — favorites only at slot indices, a below-threshold recipe never
labelled, and the top-three never promoted (the rank gate) — rather than naming a winner,
since random selection makes any specific outcome flaky.

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
- `src/rerank.rs` — **[learn] implemented.** `rerank_recommendations(candidates: &[Recipe],
  reviews: &[Review], viewer: Option<&str>) -> Vec<RankedRecipe>`, plus helpers `score_recipe`,
  `is_favorite_eligible`, `interleave_favorites`. Its only caller is
  `routes/recipes.rs::liked`. **Orders and labels, never drops** — the output is a permutation
  of the input. See "Current status" for the scoring model and the Favorites section for the
  slot mechanics.

  Seven tests, all describing inputs the live caller can actually produce. The four ordering
  tests were **checked against the candidate models rather than assumed** — they eliminate
  sum, mean, max, recency-weighted sum, *and* recency-weighted mean, while at least two
  distinct models satisfy all four, so the spec constrains without forcing one answer. The
  non-obvious exclusion is recency-weighted **mean**: normalizing by total weight cancels a
  lone review's weight, so it ties a 5★ from last week against one from a year ago and fails
  `more_recent_review_breaks_a_rating_tie`. (An earlier version of this file wrongly
  recommended exactly that model — the table in `rerank.rs`'s module doc is the corrected
  reference.) `a_single_five_star_outranks_a_repeatedly_cooked_four_star` encodes the user's
  explicit preference (2026-08-12) for peak quality over cooking frequency; it's marked
  in-comment as an *option*, safe to invert.

  `favorites_land_only_in_their_slots_and_only_when_eligible` uses 11 candidates and does
  **not** exercise the slot-capacity boundary — see "Working patterns" for the bug that hid
  behind exactly that gap.

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
  `RecipeCard` depends on. `reason === "favorite"` renders an amber badge beside the title,
  verified in both light and dark mode. Confirmed in-browser against the real seeded fixture:
  8 cards, badge on exactly indices 3 and 5, no console errors.
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

## Working patterns from Phase 1–4 scoring bugs

Enforce numeric invariants by construction, name and derive constants instead of repeating
magic literals, give any continuous score an explicit threshold (`>` vs `>=` on
`FAVORITE_MIN_MEAN_RATING` silently excluded every recipe rated exactly 4★ until it was
caught), and check a branch is reachable before writing it.

Above all: **don't trust a green test suite without checking against real data.** Four
scoring functions have now shipped passing tests while getting real data wrong — `nlp.rs`
once, `recommend_recipes.rs` twice, and `rerank.rs` once. Every one was invisible to
`cargo test` and obvious the moment real catalog/fridge/review data ran through it.

Phase 4's instance is worth remembering because of *why* the test missed it. The
favorite-capacity bug dropped a recipe whenever the chosen count exceeded what the slots could
hold; `favorites_land_only_in_their_slots_and_only_when_eligible` uses 11 candidates, where
all three slots fit comfortably, so it stayed green. The real fixture had 8 — and 8 candidates
can only hold two favorites. **A fixture sized for the happy path can't exercise the boundary
the code actually trips on.** When a function's behavior depends on collection size, test at a
size where the resource runs out, not just a comfortable one.

Two Rust-specific traps from the same session: `^` is XOR, not exponentiation (`powf` is what
you want, and on integers `^` compiles and silently computes nonsense), and `b.total_cmp(b)`
compares a value with itself — the sort became a no-op, announced only by an
`unused variable: a` warning.

## Git / GitHub

Repo root, branch `main`, remote `origin` →
`git@github.com:terrificjesse/personal-website.git` over SSH (`~/.ssh/id_ed25519`).

## Next up

**Phase 5 — authentication.** See `docs/PLAN.md`, including the "Global review aggregator"
section, whose schema (`user_id` / `is_public` / `hidden`, `GET /recipes/{id}/reviews`,
visibility-scoped reads, `reviews::current_viewer()` as the auth seam) is already in place
from Phase 4 and just needs wiring to real sessions.

Two loose ends carried over from Phase 4, neither blocking:

- **`score_recipe` ignores `viewer`** (its parameter is `_viewer`) while `is_favorite_eligible`
  honours it. Inert today — `is_by(None)` is true for every review — but once accounts exist
  the base score would silently include strangers' reviews while eligibility wouldn't. Worth
  making consistent as the first thing Phase 5 touches.
- **Unseeded `rand::rng()`** in `rerank_recommendations` (see the Favorites section).

`docs/TODO.md` holds three deferred ideas; none of them block Phase 5.

Once it's real, worth a real-data pass in-browser (seed several reviews across a few recipes —
`POST /reviews` accepts an explicit `cooked_at`, so backdate them the way Phase 2's purchase
history was seeded — then confirm "Recipes you liked" populates and orders sensibly). Same
"don't trust a green test suite" discipline as the last three scoring functions; see "Working
patterns" below. `docs/TODO.md` holds three deferred ideas; nothing there blocks Phase 4.
