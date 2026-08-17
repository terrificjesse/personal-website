# fridge-app — status & notes

Repo-wide rules (Learning Mode, phase discipline) live in the root `CLAUDE.md`. This file
tracks fridge-app-specific state so a new session doesn't have to rediscover it.

Trimmed 2026-08-13 after Phase 4. Decisions are recorded here as *rules*; the reasoning behind
them is in git history and in each module's own doc comment. When a module doc and this file
disagree, the module doc wins — it's closer to the code.

## Current status: Phases 1–5 complete (2026-08-15)

All six `[learn]` pieces — `nlp.rs`, `expiration.rs`, `recommend.rs`, `recommend_recipes.rs`,
`rerank.rs`, `auth.rs` — were implemented by the user, with Claude reviewing rather than
writing them. Every phase was verified against real data, not just fixtures.

`cargo test`: **113 passed, 0 failed**, clippy clean.

Phase 5 delivered password auth (Argon2id), server-side revocable sessions, per-account
scoping of every data query, Google OAuth sign-in and linking, and the frontend session
plumbing. See "Phase 5 verification" below for what was actually exercised against real data.

**Still open, none blocking:** the deferred `[learn]` items in PLAN.md (small-sample rating
statistics; weighting personal vs. global feedback in `rerank`), rate limiting on
`POST /reviews`, and a moderation path for `hidden`.

### Scoring models, in one place

- `recommend_recipes` — hard-filters cuisine/meal-type, then sorts by (trivial-recipe flag,
  missing-ingredient count asc, total-ingredient-count desc).
- `rerank_recommendations` — scores each recipe as the **max** over its reviews of
  `(rating - NEUTRAL_RATING) × 0.5^(age_days / DECAY_HALFLIFE)`, sorts descending, then moves
  up to three eligible recipes into `FAVORITE_SLOTS`. Centering before decaying stops an old
  rave from reading as a bad review (raw ratings decay toward 0, *below* the 1–5 scale); max
  rather than sum encodes the user's preference for peak quality over cooking frequency.
  `DECAY_HALFLIFE = 120.0` is a free parameter — verified none of the ordering tests pin it.

### The three review-driven behaviors, and where each lives

| Behavior | Lives in | Kind |
|---|---|---|
| Membership — is this "liked"? | `routes/recipes.rs::liked_recipe_ids`, `LIKED_RATING_THRESHOLD` (≥4) | `[gen]` threshold |
| Suppression — drop the disliked | `routes/recipes.rs::suppressed_recipe_ids`, `SUPPRESSED_RATING_THRESHOLD` (≤2) | `[gen]` filter |
| Ordering + favorites | `rerank.rs` | `[learn]` |

Rules that hold this together — breaking any of them reintroduces a bug that was already fixed:

- **Suppression takes precedence over liking; the two sets are disjoint by construction.**
  `liked_recipe_ids` subtracts `suppressed_recipe_ids`. Under the multi-review model a recipe
  rated 5★ once and 1★ later satisfies both raw thresholds, and would otherwise show in
  "Recipes you liked" *while* being hidden from general recommendations.
- **Both helpers scope to the viewer's own reviews internally** (`Review::is_by`) rather than
  trusting callers — `liked` hands them the wider `fetch_visible_to` set.
- **`rerank_recommendations` orders and labels, never drops.** Output is a permutation of
  input. Suppression belongs on the general-recommendations path, where a filter composes with
  Phase 3's ingredient ranking instead of fighting it.
- Only `candidates` is pre-filtered; `reviews` is the whole visible history. A candidate can
  carry a mediocre 3★ alongside its qualifying 5★. (≤2★ can't reach it — that suppresses the
  recipe outright.)

### Favorites

Highly-rated recipes (unweighted mean ≥ `FAVORITE_MIN_MEAN_RATING`) are **moved** into
`FAVORITE_SLOTS = [3, 5, 7]` and badged, so the section rotates instead of replaying one
recency-sorted list. Renamed from "throwback" on 2026-08-13 when the age gate was dropped —
rotation, not nostalgia.

- **Two gates, in two places.** Quality (the mean) is in `is_favorite_eligible`. **Rank** is in
  `rerank_recommendations` and can't move — one recipe's reviews say nothing about position.
  Never promote something already ranked above `FAVORITE_SLOTS[0]`, or the "move" demotes it.
- **Don't decay the eligibility mean.** Weight it by recency and it collapses into the base
  ranking, selecting whatever was already on top.
- **Cap selection at what can be *placed*, not at `FAVORITE_SLOTS.len()`.** A chosen favorite
  is removed from the ranking before re-insertion, so an unplaceable one is *lost*. Slot `j` is
  reachable only if `FAVORITE_SLOTS[j] < n - k + j`. **8 candidates hold two favorites, not
  three.**
- Selection uses `rand`'s `IndexedRandom::sample` (without replacement). Currently **unseeded**
  — see "Next up".

### The multi-review model is deliberate — do not "simplify" it

`reviews` is an append-only history: one row per cooking event, many rows per recipe, no unique
constraint. Every other table in this app merges on add, so it's genuinely the odd one out.

A switch to one updatable review per (user, recipe) was proposed and **the user declined,
explicitly choosing learning value over model simplicity** — the history is what preserves the
multi-review aggregation decision, which was the `[learn]` content of Phase 4. Don't propose it
again without new information.

### Review ownership — schema ready for Phase 5

Migration `0006_add_review_ownership.sql` added `user_id` (NULL pre-auth), `is_public`
(defaults 0 — opt-in, never opt-out), and `hidden` (moderation tombstone), plus
`GET /recipes/{id}/reviews`. Built during Phase 4 because retrofitting ownership onto rows that
never had it is worse than carrying nullable columns for a phase.

- **The `current_viewer()` seam is gone** — Phase 5 replaced it with the `CurrentUser`
  extractor in `routes/auth.rs`. Handlers take `CurrentUser` and pass `user.viewer()` into the
  same `Option<&str>` parameters that were already threaded everywhere; nothing below the
  handler layer changed.
- **The viewer parameter stays `Option<&str>`, not `&str`,** even though every handler now
  passes `Some`. `None` is still the documented pre-auth meaning throughout the review
  plumbing, still unit-tested in `models.rs`, and still what the `rerank.rs` fixtures use.
- **`Review::is_by(viewer)` is the whole personal-vs-global mechanism** — just an id check.
  `viewer == None` reports every review as personal, correct pre-auth.
- `fetch_visible_to` hands `rerank_recommendations` **both populations in one slice** (your
  reviews + everyone's public). Membership, by contrast, is scoped to your own.
- **A NULL `user_id` means *unclaimed*, not public.** Every scoped read filters those rows
  out, so the pre-auth data is invisible until the first account registers and
  `routes::auth::claim_unowned_rows` assigns it. That runs inside the registration
  transaction, across all four owned tables at once.

## Backend (`apps/fridge-app/backend/`)

Rust, axum, sqlx (SQLite, file `fridge.db`, gitignored). Migrations in `migrations/`. Run:
`cargo run` (binds `0.0.0.0:8080`). Test: `cargo test`.

- `src/auth.rs` — **[learn]** password hashing/verification, session issue/validate, Google
  OAuth. Read its module doc before touching it: placeholder bodies deliberately differ —
  anything that *grants* access returns the denying value (`Ok(false)`, `Ok(None)`), anything
  that *mints* a credential is `todo!()`. Preserve that distinction.
- `src/routes/auth.rs` — `[gen]` the request plumbing: `CurrentUser`/`MaybeUser` extractors,
  cookie attributes, `/auth/*` handlers, `claim_unowned_rows`. **Every route except `/health`
  and `/auth/*` takes `CurrentUser`,** so a route's own signature says whether it's protected
  — there's no separate middleware list to keep in sync.
- `src/models.rs` — all request/response/DB-row structs. `id` fields are `String` (UUID text),
  not `uuid::Uuid` — sidesteps sqlx's BLOB-based Uuid encoding mismatch with TEXT columns.
- `src/routes/items.rs` — fridge CRUD. `upsert_fridge_item` is the single place that both
  inserts/merges a fridge row *and* logs to `purchase_history`, so a purchase is never logged
  twice regardless of which flow produced it. Merge-on-add rules are in `find_merge_target`'s
  doc comment.
- `src/routes/shopping_list.rs` — shopping-list CRUD + `POST /:id/purchase` + `GET /suggestions`.
  Merges on add (same name + unit, still `pending`); a purchased row never absorbs a new add.
- `src/routes/suggest.rs` — item-name typeahead, calls `nlp::suggest_item_names`.
- `src/routes/recipes.rs` — `GET /recipes/recommended?cuisine=&mealType=` (Phase 3 ranking,
  minus suppressed recipes) and `GET /recipes/liked`. Read the behavior-split table above
  before moving logic between here and `rerank.rs`.
- `src/routes/reviews.rs` — `POST /reviews` (insert-only), `GET /reviews` (own history, joined
  against the in-memory catalog), `GET /recipes/{id}/reviews` (public wall). Three `pub(crate)`
  read helpers with deliberately different scopes: `fetch_for_viewer` (own) vs.
  `fetch_visible_to` (own + public). **Picking the wrong one leaks a stranger's review into a
  personal view.**
- `src/nlp.rs` — **[learn]** banded-tier fuzzy/prefix/substring matcher; module doc has the bands.
- `src/expiration.rs` — **[learn]** FoodKeeper-CSV-backed shelf-life lookup.
- `src/recommend.rs` — **[learn]** `suggest_shopping_items`. See technical debt re `calculate_mad`.
- `src/recommend_recipes.rs` — **[learn]** see scoring models above. `RecipeFilters`/
  `RecommendedRecipe` live here, not `models.rs` (same reasoning as `Suggestion` in `recommend.rs`).
- `src/rerank.rs` — **[learn]** `rerank_recommendations` + `score_recipe`,
  `is_favorite_eligible`, `interleave_favorites`. Its module doc carries the table of which
  scoring models the tests eliminate and why — **read it before changing any test there.**
- `src/themealdb.rs`, `src/foodkeeper.rs`, `src/purchase_history.rs` — see their module docs.
  `required_appliances` and the `fridge_ingredients`/`extra_ingredients` split are documented
  keyword heuristics, not structured facts.
- `data/foodkeeper/README.md` — **read before touching either FoodKeeper-parsing module.**
- `data/themealdb/README.md` — **read before touching `src/themealdb.rs`.**

## Frontend (`frontend/src/app/fridge/`)

Next.js 16.2.12. `frontend/AGENTS.md` requires reading `node_modules/next/dist/docs/` before
writing frontend code — this version has breaking changes vs. older Next. Two that have
already bitten:

- **`middleware.ts` is renamed to `proxy.ts`** in this version. A `middleware.ts` at the
  project root is silently ignored. Route protection lives in `frontend/src/proxy.ts`.
- `useSearchParams` needs a `Suspense` boundary above it or the route opts into client-side
  rendering — see `(auth)/login/page.tsx`.

Auth-related files: `src/proxy.ts` (route protection), `src/app/(auth)/` (login + register,
sharing one `CredentialsForm`), `src/app/SessionNav.tsx`, `src/lib/authApi.ts`,
`src/lib/apiClient.ts`, `src/lib/useApiError.ts`.

- **All backend calls go through `apiClient.ts`'s `apiFetch`.** It applies
  `credentials: "include"` *after* the caller's `init` spread so it can't be overridden — one
  forgotten flag means a request without the cookie, which comes back 401 and looks exactly
  like being logged out.
- **`proxy.ts` is an optimistic check only.** It can see whether the session cookie exists, not
  whether it's valid — only the backend can hash it and hit the `sessions` table. Real
  enforcement is the `CurrentUser` extractor on every route. An expired-but-present cookie
  sails past the proxy and surfaces as a 401, which `useApiError` turns into a redirect.
- `SessionNav` asks `/auth/me` rather than reading the cookie server-side, for the same
  reason: cookie presence isn't session validity.

Everything is a sub-route of the Fridge *tab* (`shopping-list/`, `recipes/`,
`recipes/reviews/`), not a standalone nav tab — `apps/fridge-app` is one tab in the site's
philosophy. Each sub-route has its own `lib/*Api.ts` with deliberately separate type names to
avoid collisions.

Deliberate choices worth not "fixing":

- `ItemNameCombobox.tsx` never preselects a suggestion (`activeIndex` starts at -1, resets
  every keystroke) — Enter commits the literal typed text unless the user arrows onto one.
- `ReviewForm.tsx`'s "Share publicly" checkbox **defaults to unchecked**, matching the backend.
  Publishing is always deliberate.
- `RecipeCard.tsx` collapses `instructions` behind a toggle — 789 recipes averaging ~840
  characters is too much inline.
- `LikedRecipeCard` is deliberately separate from `RecipeCard`: the liked endpoint carries no
  `matched_ingredient_count`, which is a Phase-3 concept `RecipeCard` depends on.
- `recipes/page.tsx` fetches the catalog unfiltered once to populate filter dropdowns from real
  data rather than a hardcoded list.

## Environment gotchas

- A `next dev` server and/or `cargo run` backend are often already running from a previous
  session. Check `lsof -ti tcp:3000` / `tcp:8080` before starting another. **Restart the
  backend after backend edits** — a stale binary silently serves old behavior.
- Turbopack tried inferring the workspace root as `~/Documents` (a stray `bun.lock` there).
  Fixed via `turbopack.root` in `frontend/next.config.ts` — check there if
  `TurbopackInternalError: reading dir` reappears.
- Fridge tab hanging on load (endless spinner, no error) usually means it was opened via the
  LAN "Network" URL from the same machine that printed it.
- **LAN access changed in Phase 5. Do not re-pin `NEXT_PUBLIC_FRIDGE_API_URL` to a LAN IP.**
  Cookies are scoped by host and ignore port, so a page on `localhost:3000` calling an API on
  `192.168.x.x:8080` is a *cross-site* request: the `SameSite=Lax` session cookie is never
  sent, and `proxy.ts` (which runs on the `localhost:3000` request) could never see a cookie
  belonging to the LAN host anyway — every signed-in visit would bounce to `/login`.
  `apiBase()` in `frontend/src/lib/apiClient.ts` now derives the backend host from
  `window.location`, so `localhost:3000 -> localhost:8080` and `192.168.x.x:3000 ->
  192.168.x.x:8080` both work with no config. **To use the app from a phone, open the
  *frontend* at the LAN URL** rather than pointing the API at it. The env var still overrides,
  for a real deployment where the two are genuinely different hosts — but that needs
  `SameSite=None; Secure` and therefore HTTPS on both ends.
- **One host, everywhere. `localhost` and `127.0.0.1` are different hosts** to both the cookie
  jar and Google. This has bitten twice: once via `NEXT_PUBLIC_FRIDGE_API_URL` (above), once
  via `GOOGLE_REDIRECT_URI`. In the OAuth case the state cookie is set on whichever host
  `/auth/google/start` was reached on, and Google then redirects to `GOOGLE_REDIRECT_URI` — if
  those hosts differ the cookie isn't sent and the callback fails the state check. Keep
  `GOOGLE_REDIRECT_URI`, `FRONTEND_ORIGIN`, and the host you actually browse on identical.
  Register both variants in Google Cloud Console if you like; only `.env` has to be consistent.
- Backend binds `0.0.0.0` on purpose. Every data route now requires a session, but
  `COOKIE_SECURE` defaults to off (plain HTTP on a LAN), so still trusted networks only.
- `apps/fridge-app/backend/.env.example` documents every env var; `.env` is gitignored.
  **Copying it verbatim leaves every line commented out** — the file is written as
  documentation. `dotenvy` then sets nothing and the backend logs `Google OAuth not
  configured`. Uncomment what you fill in.
- **Verify auth changes against a copy of `fridge.db`, not the real one.** The first
  registration permanently claims the pre-auth rows; a throwaway test account would take them.
  `cp fridge.db /tmp/x.db` then `DATABASE_URL="sqlite:///tmp/x.db?mode=rwc" cargo run`.
- Repo root `.claude/launch.json` (`cwd: frontend`) is this project's dev-preview config; an
  unrelated `/Users/jesseli/projects/meal/.claude/launch.json` also exists — not this project.
- `fridge.db` currently holds 16 seeded reviews across 10 recipes (backdated `cooked_at`), 4
  fridge items and 1 purchase — useful as real-data fixtures. Gitignored. **All of it is
  currently unclaimed (`user_id IS NULL`)** and stays invisible until the first account
  registers. `fridge.db.pre-phase5-backup` is a copy taken before migrations 0007/0008 ran.

## Open technical debt

- `foodkeeper_product_id` on a collapsed name (e.g. `Ham`, 20 CSV rows) is just the first row's
  id. Needs `Name_subtitle` handling (FoodKeeper README gotcha 6).
- `expiration.rs` re-parses the FoodKeeper CSV independently of `foodkeeper.rs` instead of
  reusing the `Catalog` already in `AppState`.
- `calculate_mad` in `recommend.rs` computes the median gap, not full MAD, and has never been
  checked against real purchase data.
- `data/themealdb/meals.json` is a 2026-08-10 snapshot; re-run the letter-sweep fetch if stale.
- `favorites_land_only_in_their_slots_and_only_when_eligible` uses 11 candidates, where all
  three slots fit — it can't exercise the slot-capacity boundary. A fixture at ≤8 would.

## Working patterns from Phase 1–5

### Rust cannot check the inside of a string (Phase 5's dominant lesson)

Every Phase 5 bug that survived compilation lived in a string literal — a contract with
something *outside* the program that the type system has no visibility into:

| Contract | Symptom | How to check it in five seconds |
|---|---|---|
| SQL passed to `sqlx::query` | `row value misused`, `syntax error near "."` | paste into `sqlite3 fridge.db` |
| A URL | `404`, or a serde "missing field" error | `curl -i` it — **404 means wrong, 401 means right-but-unauthenticated** |
| JSON field names | `missing field 'subject'` | Google's discovery doc / `claims_supported` |

`sqlx::query` does not parse its argument; `Client::post` does not validate its URL; serde
cannot know what the server actually sends. Three separate SQL strings and two wrong URLs
compiled cleanly this phase.

**When an error message doesn't fit the code you're staring at, suspect the string.** "Missing
field `subject`" was a wrong URL. "Row value misused" was a stray pair of parentheses in SQL.

### Untested code is where the bugs live

Phase 5's four worst bugs were all in paths that had **never executed** — the OAuth callback,
the Google account-creation branch, `validate_session`, `exchange_google_code`. `cargo test`
reported 110 passing while Google sign-in was completely broken, because none of those had a
test and none could be reached without full OAuth configuration.

Corollary: when a function can't be unit-tested (network, live pool), that is a reason to
verify it *harder* by hand, not a reason to assume it works. The DB-backed session tests in
`auth.rs` (`test_pool()`, in-memory SQLite from the real migrations) caught the `validate_session`
SQL bug on their first run.

### Beware the vacuous pass

A test going green is not evidence unless it *could* have failed. This phase produced three:

- Three password tests passed against a `verify_password` that denied everyone.
- `a_strangers_high_ratings_do_not_make_a_recipe_your_favorite` passed because its fixture
  tripped the rank gate before reaching the quality gate it was written to test.
- `session_tokens_are_unique_across_calls` passed on a 32-bit token.

When a test goes green, ask what would make it red.

### From Phase 1–4 scoring bugs

Enforce numeric invariants by construction. Name and derive constants instead of repeating
magic literals. Give any continuous score an explicit threshold — `>` vs `>=` on
`FAVORITE_MIN_MEAN_RATING` silently excluded every recipe rated exactly 4★. Check a branch is
reachable before writing it.

Above all: **don't trust a green test suite without checking against real data.** Four scoring
functions have shipped passing tests while getting real data wrong — `nlp.rs` once,
`recommend_recipes.rs` twice, `rerank.rs` once. Every one was invisible to `cargo test` and
obvious the moment real data ran through it.

Phase 4's is worth remembering for *why* the test missed it: the favorite-capacity bug dropped
a recipe when the chosen count exceeded what the slots could hold, but the test used 11
candidates where all three slots fit. The real fixture had 8. **A fixture sized for the happy
path can't exercise the boundary the code trips on** — when behavior depends on collection
size, test at a size where the resource runs out.

Rust traps hit in this project: `^` is XOR, not exponentiation (`powf`; on integers `^`
compiles and silently computes nonsense); `b.total_cmp(b)` compares a value with itself, making
a sort a no-op announced only by an `unused variable` warning; `f64` has no `Ord`, so use
`total_cmp` rather than `partial_cmp().unwrap()`; `&Vec<T>` has no `Default` but `&[T]` does.

## Git / GitHub

Repo root, branch `main`, remote `origin` →
`git@github.com:terrificjesse/personal-website.git` over SSH (`~/.ssh/id_ed25519`).

## `rerank.rs` viewer scoping — fixed 2026-08-15

A Phase 4 note here claimed `score_recipe` ignored `viewer` while `is_favorite_eligible`
honoured it. **That was wrong in a way that mattered: neither honoured it.** Both took
`_viewer`, neither called `Review::is_by`, and the caller didn't pre-filter — so both pooled
strangers' reviews into the viewer's own.

Both now filter internally with `is_by(viewer)`, matching the `routes/recipes.rs` precedent
(helpers scope themselves rather than trusting callers). `score_recipe` still receives the
**full** per-recipe slice so the deferred personal-vs-global weighting has the crowd available
— do not move that filter up to the grouping site in `rerank_recommendations`.

Why it hid through Phase 4: every pre-existing test builds reviews via `review_at`, which
hardcodes `user_id: Some(VIEWER)`. No fixture in the file could tell a scoped implementation
from an unscoped one. Three tests were added to close that:

- `a_strangers_rave_does_not_lift_a_recipe_in_your_ranking`
- `a_recipe_only_strangers_have_reviewed_still_ranks`
- `a_strangers_high_ratings_do_not_make_a_recipe_your_favorite` — **this one initially passed,
  vacuously.** Its first fixture put the stranger's 5★s *recently*, which made the recipe rank
  first, so the **rank gate** rejected it before the quality gate was ever consulted. Ageing
  them to 3000 days drops the decayed base score without touching the undecayed eligibility
  mean. Same lesson as the Phase 4 favorite-capacity bug: a fixture sized for the happy path
  can't reach the boundary the code trips on.

**Still worth doing:** `is_favorite_eligible`'s `sum / len` has no explicit empty guard. It
returns the right answer via `0.0/0.0 = NaN` comparing false, which is accidental — and
fragile, since `NaN < x` is *also* false, so the negated form would be silently wrong.

Also still stale: six `#[allow(dead_code)]` in `rerank.rs`, one on `NEUTRAL_RATING` in
`models.rs`, and the `TODO(you)` in `rerank.rs`'s module doc still describing the body as an
empty-list placeholder. They produce no warnings *because* the attributes suppress them —
delete and let the compiler say which were real.

## Phase 5 verification (2026-08-15)

Run against **copies** of `fridge.db`, never the real one — the first registration permanently
claims the 22 pre-auth rows, and that slot belongs to the user's own account.

Password/session half, over HTTP and in-browser:

- Register → `HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000`, 64-hex token; `claimed 22
  pre-auth rows`; logout deletes the row and the old cookie 401s.
- Plaintext token appears in **no** column of `sessions` — only the SHA-256 digest.
- **A fresh second account sees an empty fridge**: 0 items, 0 reviews, 0 liked. Suppression is
  per-account (acct1 787 recipes, acct2 789).
- Public review visible to the other account; private one not; another account's 5★ never
  enters your liked list.
- In-browser: `/fridge` redirects to `/login?next=…`, registration lands signed in, liked
  section renders 8 with the Favorite badge on a real slot, logout returns to `/login`.

`rerank` scoping, with a discriminating case per function:

- **`score_recipe`** — acct2 wrote 32 fresh public 5★ across all 8 of acct1's liked recipes
  (acct1's visible slice: 48 reviews). acct1's **top-3 did not move** across three samples.
  Positions 0–2 are the right invariant to check: `FAVORITE_SLOTS[0]` is 3, so nothing can be
  promoted above index 3 and those slots are pure base-score order.
- **`is_favorite_eligible`** — recipe `53380` (Apple cake) has own-mean **3.67** but pooled-mean
  **4.43**, and sits at index 4, so it passes the rank gate and is genuinely promotable. Across
  20 samples it was **never badged**. Confirming it was promotable matters — otherwise the rank
  gate would reject it first and the check would prove nothing.

Liked count (8) and general-recommendation count (787) both match the Phase 4 checkpoint
exactly, which is the best evidence the backfill and viewer threading changed nothing they
shouldn't have.

## Two scaffolding bugs Claude introduced in Phase 5

Both were in `[gen]` code, both invisible to the test suite, both in paths that had never
executed. Recorded because the *shape* recurs.

1. **The OAuth state check could never pass.** `google_callback` added the removal cookie
   before reading the incoming state — and `CookieJar::add` writes into the same map `get`
   reads from, so `get` returned the empty value just inserted. `expected` was always
   `Some("")`, so **every** callback returned `OAuthStateMismatch` regardless of config. Fix:
   capture the incoming value first, then clear.
2. **Google sign-in could strand the pre-auth rows forever.** `claim_unowned_rows` was wired
   only into password registration. Creating the first account *via Google* skipped it, leaving
   all 22 rows unclaimed and unreachable with no UI able to recover them. Fix:
   `claim_if_first_account`, called from **both** account-creation paths inside their
   transactions.

## Next up

Nothing blocking. In rough priority:

1. **Register the real account on `fridge.db`.** It is still 0 users / 16 unclaimed reviews.
   That run claims the 22 rows for good — check the liked list reads 8 immediately after.
2. **`is_favorite_eligible` empty guard** and the stale-attribute sweep above.
3. **Unseeded `rand::rng()`** in `rerank_recommendations` — now confirmed live: four calls to
   `/recipes/liked` with zero data change produced four different favorite pairs. Reloading the
   recipes page makes badges jump, which reads as broken rather than as rotation.
   `StdRng::seed_from_u64` with a date-derived seed gives daily rotation, stability within a
   session, and pinnable tests.
4. **Two pre-existing frontend lint errors** (`react-hooks/set-state-in-effect` in
   `GroceryListPopup.tsx:18` and `recipes/page.tsx:54`). Both predate Phase 5 — verified
   against the committed versions.

`docs/TODO.md` holds four deferred ideas.
