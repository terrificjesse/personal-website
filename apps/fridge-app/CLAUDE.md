# fridge-app — status & notes

Repo-wide rules (Learning Mode, phase discipline) live in the root `CLAUDE.md`. This file
tracks fridge-app-specific state so a new session doesn't have to rediscover it.

Trimmed 2026-08-19. Decisions are recorded here as *rules*; the reasoning behind them is in
git history, in `docs/PLAN.md`'s phase checkpoints, and in each module's own doc comment.
When a module doc and this file disagree, the module doc wins — it's closer to the code.

## Current status: Phases 1–6 complete (blog tab finished 2026-08-19)

All six `[learn]` pieces — `nlp.rs`, `expiration.rs`, `recommend.rs`, `recommend_recipes.rs`,
`rerank.rs`, `auth.rs` — were implemented by the user, with Claude reviewing rather than
writing them. Every phase was verified against real data, not just fixtures; the per-phase
evidence lives in `docs/PLAN.md`'s checkpoints, not here.

`cargo test`: **663 passed, 0 failed**, clippy clean. (Phase 6 alone was 141; the rest is
Phase 7 work from a parallel session plus the blog stress-test suite.)

**Still open, none blocking:** the deferred `[learn]` items in PLAN.md (small-sample rating
statistics; weighting personal vs. global feedback in `rerank`); rate limiting on
`POST /reviews`; and a moderation path for `hidden`.

### Scoring models, in one place

- `recommend_recipes` — hard-filters cuisine/meal-type, then sorts by (trivial-recipe flag,
  missing-ingredient count asc, total-ingredient-count desc).
- `rerank_recommendations` — scores each recipe as the **max** over its reviews of
  `(rating - NEUTRAL_RATING) × 0.5^(age_days / DECAY_HALFLIFE)`, sorts descending, then moves
  up to three eligible recipes into `FAVORITE_SLOTS`. Centering before decaying stops an old
  rave from reading as a bad review (raw ratings decay toward 0, *below* the 1–5 scale); max
  rather than sum encodes the user's preference for peak quality over cooking frequency.
  `DECAY_HALFLIFE = 120.0` is a free parameter — verified none of the ordering tests pin it.
  Favorite selection is random but **seeded by the day** (`num_days_from_ce()`), so the badges
  are stable for every request within a UTC day and rotate at midnight. Verified 2026-08-15:
  six consecutive calls returned identical favorites, where unseeded they changed every time.
  Note this is seeded random, not true rotation — no coverage guarantee, so a recipe can
  repeat or be skipped for a stretch.

**Viewer scoping in `rerank.rs`:** `score_recipe` and `is_favorite_eligible` both filter
internally with `Review::is_by(viewer)`, matching the `routes/recipes.rs` precedent (helpers
scope themselves rather than trusting callers). `score_recipe` still receives the **full**
per-recipe slice so the deferred personal-vs-global weighting has the crowd available — **do
not move that filter up** to the grouping site in `rerank_recommendations`. Three tests pin
this (`a_strangers_rave_does_not_lift_a_recipe_in_your_ranking` and neighbors); the older
fixtures cannot, since `review_at` hardcodes `user_id: Some(VIEWER)`.

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
- `src/blog_files.rs` — `[gen]` markdown-file ingestion for the blog. Hand-rolled frontmatter
  parser (no crate — **an unknown key and a repeated key are both errors**, because the whole
  point is catching a typo the author cannot otherwise see), `sync`/`sync_in` (mirrors
  `content/blog/*.md` into `blog_posts`), and
  `spawn_watcher` (polls a `(name, mtime, size)` fingerprint every `BLOG_SYNC_INTERVAL_SECS`).
  **Two invariants a 2026-08-30 stress test had to install — don't undo them:** the sweep
  deletes on *file absence* (`source_path`, migration `0016`), never on parse failure; and
  `watch_tick` advances its fingerprint only on `SyncOutcome::Completed`, never on `Deferred`.
  Both bugs were the same mistake — treating "I looked" as "I succeeded".
  Read its module doc before changing it — the filename-not-title slug rule and the
  frontmatter-not-mtime date rule both look like details and are not.
  **`is_post_file` is deliberately shared** by the reader and the watcher; let those two drift
  and you get either changes that never sync or a sync that loops on a file it ignores.
  Note mtime is used for change detection while being refused for `created_at` — not an
  inconsistency: for ordering it gives a wrong answer, for detection a redundant one, and
  `sync` makes a redundant trigger a no-op.
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
- **Never edit a migration that has already been applied.** sqlx stores a checksum of every
  migration in `_sqlx_migrations` and refuses to start if the file no longer matches:
  `migration 12 was previously applied but has been modified`. **A comment-only edit breaks it
  just as thoroughly as a schema change** — this happened to `0012` on 2026-08-20, editing only
  a comment block, and it took the backend (and so the whole site) down until the file was
  restored byte-for-byte with `git checkout`. Once a migration has run *anywhere*, it is
  immutable: corrections go in a new migration, or in a code comment. The failure surfaces as
  "failed to fetch" in the browser, because the frontend's error is downstream of a backend
  that never started.

- **`BLOG_CONTENT_DIR`** overrides where the blog looks for `.md` files; the default is
  `content/blog` at the repo root, resolved off `CARGO_MANIFEST_DIR` rather than the working
  directory (the backend runs three levels below the root). A missing directory is logged and
  skipped, not an error.
- **`BLOG_SYNC_INTERVAL_SECS`** (default 5) is how often the blog watcher re-checks that
  directory; `0`, or anything unparseable, disables it and falls back to startup +
  `POST /blog/sync`. A tick does no database work and logs nothing unless the directory
  actually changed.
- **`PORT`** (default 8080) lets a second backend run alongside the usual one — how Phase 6 was
  verified against a throwaway database without stopping the running instance.
- **`cargo fmt` reformats the whole crate, `[learn]` files included.** It stripped a leading
  blank line from `nlp.rs`, `expiration.rs`, `recommend.rs`, `recommend_recipes.rs`, and
  `rerank.rs` during Phase 6 and had to be reverted by hand. Format single files
  (`rustfmt src/foo.rs`) or check the diff afterwards.
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

## Admin flag & blog (added 2026-08-19)

**Full reference — every file and function — is `docs/BLOG.md`. Read that before working on
the blog.** Only the rules that constrain *this* backend are repeated here.

`users.is_admin` (migration `0009`) gates admin-only features. Not settable through any API;
grant it directly: `sqlite3 fridge.db "UPDATE users SET is_admin = 1 WHERE email = '…';"`
(match the email **lowercased** — `normalize_email` lowercases before insert). No re-login or
restart needed: `validate_session` re-joins `users` every request, so grants and revocations
land immediately.

- `auth::require_admin` is implemented (by the user, in the `[learn]` file `src/auth.rs`) —
  `Ok(())` when `is_admin`, else `Err(AuthError::Forbidden)` → **403**. Its doc comment is
  **stale**, still calling itself an unimplemented placeholder; leave it for the user to fix.
- **Validation lives in `models.rs`, once.** `exceeds_char_limit` (limits are **characters**;
  `str::len()` is bytes and silently punished non-ASCII) and `is_blank` (empty-or-whitespace,
  used to validate only — bodies are stored verbatim). Both exist because the same rule was
  written twice, in the API path and the file path, and the copies drifted. **Add a call, not
  a fifth copy.**
- **Phase 6 is done, and was stress-tested on 2026-08-30** — five real bugs, all fixed; see
  `docs/BLOG.md` § "Adversarial stress test" for the list and the two refuted hypotheses.
  Markdown rendering (frontend `react-markdown`), `?sort=`, `?q=`, and
  `content/blog/*.md` ingestion all landed 2026-08-19. Two rules worth not rediscovering:
  file-sourced posts answer `PATCH`/`DELETE` with **409** (the next sync would overwrite the
  edit), and **there is no branch on `source` in the read path** — `list_posts` is one query
  covering both kinds, which is the whole reason files are rows rather than a second store.
  Adding `rehype-raw` on the frontend would re-enable raw HTML in posts; don't, without
  deciding to accept that.
- **The editor had no inbound link until 2026-08-19.** `/blog/admin` linked out to `/blog` and
  nothing linked back, so it was reachable only by typing the URL. `/blog` now shows a "Write a
  post" button gated on `is_admin` — optimistic UI, same as every other `is_admin` read.
- **`Forbidden`/403 must stay distinct from `InvalidCredentials`/401.** `apiFetch` raises
  `UnauthorizedError` on 401 only, and `useApiError` redirects that to `/login` — routing a
  non-admin through 401 would bounce a signed-in user to a login page they're already past.
- `routes::auth::RequireAdmin` is the `[gen]` extractor: `CurrentUser` → `require_admin`.
  Every write route in `routes/blog.rs` takes it, so **a route's signature is the authority**
  on its own protection — the Phase 5 pattern, unchanged.
- **Two places answer "is this an admin":** `require_admin`, and two inline
  `user.is_admin` reads in `blog.rs` (`list_posts`, `get_post`) that widen results to include
  drafts. They agree only while `require_admin` is exactly `user.is_admin`. Make it richer and
  the read paths silently keep the old policy — same hazard as `fetch_for_viewer` vs
  `fetch_visible_to` above.
- `slug` is stored at creation and **never rewritten** on a title edit, so a published URL
  stays stable. `unique_slug` appends `-2`, `-3`, … on collision.
- **Never re-introduce a check-then-insert for slugs.** `create_post` attempts the insert and
  lets the `UNIQUE` constraint pick the suffix. The previous `SELECT EXISTS` + `INSERT` returned
  **500 six times out of ten** under a ten-way race — a double-clicked submit button reaches it.
  A prior SELECT can never be atomic with the INSERT after it.
- **`GET /blog/posts` is paginated by keyset** (`limit`/`cursor`, default 20, max 100, envelope
  `{posts,total,limit,next_cursor}`). It shipped with `offset` and that repeated rows under
  concurrent publishing — **do not go back to offset.** Three more rules not to undo:
  over-limit is a **400, not a clamp**;
  `total` is counted **without the cursor** but **with** the draft filter, so it neither counts
  down as you page nor leaks the draft count to a non-admin; and
  **`ORDER BY created_at …, id ASC`** — file posts all share a midnight timestamp, so without
  the `id` tiebreaker paging repeats one post and drops another. It is also a **coupled**
  change: an old backend binary serving a bare array breaks the frontend outright.
- **Verify authorization with `curl`, not the browser.** `proxy.ts` and the admin page's
  `is_admin` check are both optimistic UI; only `RequireAdmin` enforces, and only curl skips
  the other two.
- The blog is a **separate tab** that happens to live in this backend because auth and `users`
  are here. Not a fridge feature — don't let the two bleed together.
- **Phase 6 (markdown rendering, sort, search, `.md` files from git) is `[gen]`, not Learning
  Mode.** See `docs/PLAN.md`.

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

**Rust cannot check the inside of a string.** Every Phase 5 bug that survived compilation
lived in a string literal — a contract with something *outside* the program. `sqlx::query`
does not parse its argument, `Client::post` does not validate its URL, serde cannot know what
the server actually sends.

| Contract | Symptom | How to check it in five seconds |
|---|---|---|
| SQL passed to `sqlx::query` | `row value misused`, `syntax error near "."` | paste into `sqlite3 fridge.db` |
| A URL | `404`, or a serde "missing field" error | `curl -i` it — **404 means wrong, 401 means right-but-unauthenticated** |
| JSON field names | `missing field 'subject'` | Google's discovery doc / `claims_supported` |

When an error message doesn't fit the code you're staring at, suspect the string.

**Untested code is where the bugs live.** Phase 5's four worst bugs were all in paths that had
never executed (OAuth callback, Google account creation, `validate_session`,
`exchange_google_code`) — `cargo test` reported 110 passing while Google sign-in was
completely broken. When a function can't be unit-tested (network, live pool), that is a reason
to verify it *harder* by hand, not to assume it works.

**Beware the vacuous pass.** A green test is not evidence unless it *could* have failed. Three
real examples: three password tests passed against a `verify_password` that denied everyone;
`a_strangers_high_ratings_do_not_make_a_recipe_your_favorite` passed because its fixture
tripped the rank gate before reaching the quality gate under test;
`session_tokens_are_unique_across_calls` passed on a 32-bit token.

**A fixture sized for the happy path can't reach the boundary the code trips on.** The Phase 4
favorite-capacity bug dropped a recipe when the chosen count exceeded the placeable slots — the
test used 11 candidates where all three slots fit; real data had 8. When behavior depends on
collection size, test at a size where the resource runs out.

**Don't trust a green suite without checking real data.** Four scoring functions shipped
passing tests while getting real data wrong (`nlp.rs` once, `recommend_recipes.rs` twice,
`rerank.rs` once) — every one invisible to `cargo test`, obvious on first real input.

Also: enforce numeric invariants by construction; name and derive constants rather than
repeating magic literals; give any continuous score an explicit threshold (`>` vs `>=` on
`FAVORITE_MIN_MEAN_RATING` silently excluded every recipe rated exactly 4★); check a branch is
reachable before writing it.

Rust traps hit here: `^` is XOR, not exponentiation (`powf`; on integers `^` compiles and
silently computes nonsense); `b.total_cmp(b)` compares a value with itself, making a sort a
no-op announced only by an `unused variable` warning; `f64` has no `Ord`, so use `total_cmp`
rather than `partial_cmp().unwrap()`; `&Vec<T>` has no `Default` but `&[T]` does.

## Git / GitHub

Repo root, branch `main`, remote `origin` →
`git@github.com:terrificjesse/personal-website.git` over SSH (`~/.ssh/id_ed25519`).

## Next up

Nothing blocking. In rough priority:

1. **Fix `require_admin`'s stale doc comment** in `src/auth.rs` — it still calls itself an
   unimplemented placeholder that denies everyone. `[learn]` file, so it's yours.
2. **A third blog stress pass**, if wanted. Both passes are closed with every finding fixed;
   what is left unexplored is frontmatter fuzzing beyond what was covered (symlinks,
   megabyte single-line files) and the frontend beyond the first pass.
   `docs/BLOG_STRESS_TEST_PLAN.md` has the method.
3. **Two pre-existing frontend lint errors** (`react-hooks/set-state-in-effect` in
   `GroceryListPopup.tsx:18` and `recipes/page.tsx:54`). Both predate Phase 5 — re-verified
   2026-08-19.

`docs/TODO.md` holds four deferred ideas.
