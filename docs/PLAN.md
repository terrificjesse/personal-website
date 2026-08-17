# Fridge App — Phased Implementation Plan

Legend for each task:
- **[gen]** — fine for Claude Code to generate fully.
- **[learn]** — flagged learning area (auth / NLP / recommendations). Claude should scaffold
  the surrounding structure and stop at the boundary; the user implements the core logic.
  See `CLAUDE.md` → Learning Mode.
- **[you]** — decisions or setup only the user should do (accounts, keys, preferences).

---

## Phase 0 — Repo & environment setup (do this before Phase 1)

- [gen] `frontend/`: `create-next-app` with TypeScript + Tailwind + App Router.
- [gen] `apps/fridge-app/backend/`: `cargo init`, add `axum`, `tokio`, `serde`,
  `serde_json`, `sqlx` (sqlite driver to start — fast local dev, swap to Postgres later
  if you want that experience too).
- [gen] Basic health-check endpoint (`GET /health`) and a root Next.js page that fetches
  it, just to prove the two sides talk to each other.
- [you] Decide: SQLite for the whole project, or Postgres from the start? SQLite is less
  setup and is plenty for a single-user app; Postgres is more realistic if you want that
  experience. Either is fine — flag your choice so it goes in this doc.
- [gen] `.gitignore`, basic `README.md`, initialize git repo, first commit.

---

## Phase 1 — Fridge core + NLP item matching

**Goal:** CRUD for fridge items, expiration projection, typo/synonym-tolerant item
identity ("tomato" == "tomatoes" == "tomatoe").

### Backend (Rust)
- [gen] Data model: `FridgeItem { id, canonical_name, quantity, unit, added_at,
  estimated_expiration }`.
- [gen] Migrations + `sqlx` queries for add/remove/list items.
- [gen] Endpoint skeletons: `POST /items`, `DELETE /items/:id`, `GET /items`,
  `PATCH /items/:id`.
- [learn] **Item name normalization** — when a new item is added, decide whether it matches
  an existing canonical item (or a known dictionary entry) despite typos/pluralization.
  Claude will stub a trait, e.g. `fn resolve_item_name(input: &str, known: &[String]) ->
  MatchResult`, and write tests against it (exact match, plural, common typo, unrelated
  word → no match). You implement the body. Worth researching before you start: edit
  distance (Levenshtein/Damerau-Levenshtein), simple stemming for plurals, and whether a
  small local synonym dictionary is more practical than a "smart" algorithm for a
  single-user fridge app.
- [learn] **Expiration projection** — reclassified as a learning area at the user's
  request (originally scoped as [gen]). Claude stubs `fn estimate_expiration(item_name:
  &str, added_at: DateTime<Utc>) -> DateTime<Utc>` with tests (produce vs. pantry vs. dairy
  category gets a materially different shelf life; unknown item gets a sane fallback
  rather than panicking). You implement the body — a category → shelf-life lookup table is
  a perfectly good starting point, no need to overengineer it.

### Frontend (Next.js)
- [gen] `/fridge` tab route, item list view, add-item form, remove button, expiration
  badges (color-coded by days remaining).
- [gen] API client hooks (`useFridgeItems`, etc.) wired to the Rust backend.

### Checkpoint
You should be able to add "tomatoe", have it resolve to "tomato", see it in the list with
a projected expiration date, and remove it.

---

## Phase 2 — Shopping list + purchase-based recommendations

**Goal:** A shopping list you can add to manually, plus suggested items based on purchase
history and near-expiry fridge items. Non-grocery items excluded from recommendation logic.

### Backend
- [gen] Data model: `ShoppingListItem { id, name, is_grocery: bool, added_manually: bool,
  status }` and `PurchaseHistory { item_name, purchased_at, quantity }` (populate purchase
  history when an item is marked "bought" off the list, or when a fridge item is added —
  decide which trigger makes more sense as you build it).
- [gen] Endpoints for add/remove/list shopping list items, mark-purchased.
- [learn] **Recommendation logic** for "what to add to the shopping list" — Claude stubs
  `fn suggest_shopping_items(history: &[PurchaseHistory], fridge: &[FridgeItem]) ->
  Vec<Suggestion>` with tests (e.g. "item purchased weekly and not in fridge → suggested",
  "item expiring in <2 days → suggested as replacement"). You implement it. Worth reading
  about before starting: simple frequency/recency heuristics are a legitimate starting
  point — you don't need ML here. This is a good phase to learn the difference between
  a rules-based recommender and a learned one, since Phase 4 will push you toward the
  latter.
- [gen] Non-grocery items: just a boolean flag that excludes them from the history table
  and suggestion input — no learning content here, straightforward filtering.

### Frontend
- [gen] Shopping list tab/section, add/remove UI, "suggested items" panel (separate from
  the manual list), grocery vs. non-grocery toggle on add.

### Checkpoint
Mark a few fridge items as purchased over a couple of simulated "weeks" (you can backdate
test data), confirm suggestions reflect frequency and expiring items, confirm a
non-grocery item (e.g. "paper towels") never shows up as a suggestion.

---

## Phase 3 — Recipe recommendations

**Goal:** Recipes recommended from fridge + shopping list contents, with cook time,
required appliances, extra ingredients (spices etc.), and typed filters (cuisine,
meal type).

- [gen] Data model: `Recipe { id, name, cuisine_tags, meal_type_tags, cook_time_minutes,
  required_appliances, fridge_ingredients, extra_ingredients }`.
- [you] **Decided:** [TheMealDB](https://www.themealdb.com/), vendored as a one-time
  snapshot rather than called live — same pattern as `data/foodkeeper/`. Fetched
  2026-08-10 via the shared test API key (`"1"`, no signup needed — free for
  development/personal/educational use per their terms), enumerated with the standard
  letter-sweep (`search.php?f=a..z`, deduped by `idMeal`). 789 unique recipes, stored at
  `apps/fridge-app/backend/data/themealdb/meals.json`. Considered Spoonacular (better
  field fidelity — native cook time and equipment — but needs your own registered key and
  a 150 req/day free-tier cap) and Edamam (largest index, but the public free tier reads
  as trial-limited plus mandatory attribution); TheMealDB won on zero setup friction for a
  personal project. Full tradeoffs and field-mapping gotchas (no cook-time field, sparse
  `strArea`, `strCategory` isn't a true meal-type split) are in
  `apps/fridge-app/backend/data/themealdb/README.md`. TheMealDB's `idMeal` is what Phase
  4's `Review.recipe_id` will reference.
- [learn] **Matching/recommendation logic** — reclassified as a learning area at the user's
  request (originally scoped as [gen]). Score/filter recipes by how many required
  ingredients are already in fridge+shopping list; the user's typed filters (cuisine/meal
  type) apply as a hard filter, not a scoring input, so results stay predictable. Claude
  stubs a function signature — something like `fn recommend_recipes(recipes: &[Recipe],
  fridge: &[FridgeItem], shopping_list: &[ShoppingListItem], filters: &RecipeFilters) ->
  Vec<RecommendedRecipe>` — with tests describing the desired behavior (a recipe using more
  of what you already have ranks higher; a cuisine/meal-type filter excludes non-matching
  recipes regardless of how well they'd otherwise score; a recipe needing none of your
  current ingredients still appears if the filters allow it). You implement the body.
  Worth reading before starting: set-overlap scoring (what fraction of a recipe's
  ingredients you already have — similar in spirit to Jaccard similarity), and how to
  combine a hard filter with a soft score cleanly. Good phase to get comfortable with a
  rules-based recommender's shape before Phase 4 pushes toward a learned one.
- [gen] Endpoints: `GET /recipes/recommended?cuisine=&mealType=`.
- [gen] Frontend: recipe cards showing cook time, appliances, extra ingredients needed,
  filter chips/dropdown for cuisine and meal type.

### Checkpoint
Filtering by a cuisine returns only recipes tagged with it; recipes using more of your
current fridge contents show up first.

---

## Phase 4 — Review system + learned recommendation

**Goal:** Rate past recipes; liked recipes resurface (in a separate "recommended again"
section); disliked ones stop appearing; review history is browsable.

- [gen] Data model: `Review { recipe_id, rating, cooked_at, notes }`.
- [gen] Endpoints: submit review, list review history.
- [gen] Frontend: review form after marking a recipe "cooked," a review history page, and
  a second recommendation section ("Recipes you liked") separate from Phase 3's general
  recommendations.
- [learn] **The actual re-ranking algorithm** — **done.** Implemented by the user as
  `fn rerank_recommendations(candidates: &[Recipe], reviews: &[Review], viewer: Option<&str>)
  -> Vec<RankedRecipe>` in `apps/fridge-app/backend/src/rerank.rs`. Final model: score each
  recipe as the **max** over its reviews of `(rating - NEUTRAL_RATING) × 0.5^(age_days /
  DECAY_HALFLIFE)`, sort descending. Centering before decaying stops an old rave from reading
  as a bad review (raw ratings decay toward 0, which is *below* the 1–5 scale); max rather
  than sum encodes the user's stated preference for peak quality over cooking frequency.

  Two behaviors moved *out* of this function mid-phase, both `[gen]`: membership
  (`liked_recipe_ids`) and suppression (`suppressed_recipe_ids`) now live on the route
  handlers. Suppression is a filter, which composes with Phase 3's ingredient ranking,
  whereas a second *ordering* would fight it. See `apps/fridge-app/CLAUDE.md` for the full
  split and reasoning.

- [learn] **Favorites** — added mid-phase at the user's request, not in the original plan.
  The base ranking is deterministic, so the liked section showed the same order every visit.
  Favorites move up to three highly-rated recipes (unweighted mean ≥ 4.0) into fixed slots
  `[3, 5, 7]`, badged in the UI so an out-of-order entry reads as intentional rather than
  broken. Began as an age-gated "throwback" mechanism; when the age gate was dropped in
  favor of pure rotation, a **rank gate** replaced it — never promote something already
  ranked above the first slot, or the "move" is a demotion.

### Checkpoint — met 2026-08-13

Verified against real seeded data (16 reviews across 10 recipes, backdated via
`POST /reviews`), not just hand-built fixtures:

- Rate a recipe highly → it appears in the "Recipes you liked" section. ✅
- Rate one poorly (≤2★) → it drops out of general recommendations: 789 → 787. ✅
- A recipe rated 5★ then 1★ is absent from *both* lists — liked and suppressed are disjoint
  by construction. ✅
- The base ranking reproduced the model's predicted order exactly across all 8 liked recipes.
- In-browser: 8 cards, "Favorite" badge on exactly the expected slots, no console errors.

`cargo test`: 75 passed, 0 failed, clippy clean.

---

## Phase 5 — Authentication — **complete 2026-08-15**

**Goal:** Password-based accounts, optional Google OAuth. Single user in practice, but
built for real.

- [gen] **Done.** Scaffolding built and verified 2026-08-13: migrations `0007` (users,
  sessions, oauth_identities) and `0008` (per-account `user_id` on fridge/shopping/purchase
  rows); `POST /auth/register`, `/auth/login`, `/auth/logout`, `GET /auth/me`,
  `GET /auth/google/start`, `GET /auth/google/callback`; `CurrentUser`/`MaybeUser` extractors;
  every data query scoped by account; CORS switched off the wildcard so credentialed requests
  work; and on the Next side `proxy.ts` route protection, login/register pages, `SessionNav`,
  and a shared `apiFetch` that can't forget `credentials: "include"`.

  Two decisions taken during scaffolding that are worth knowing about:
  - **Sessions are server-side opaque tokens, not JWTs**, so logout actually revokes rather
    than asking the browser to forget. The `sessions` row stores a *hash* of the token.
  - **Route protection is per-handler, via the `CurrentUser` extractor**, not a middleware
    list — a route's signature is the authority on whether it needs a session.
- [learn] **The actual auth implementation** — **done**, written by the user with Claude
  reviewing and debugging. Ten functions in `src/auth.rs`:
  - **Passwords** — `hash_password` / `verify_password` via **Argon2id** (crate defaults:
    19 MiB, t=2, p=1, matching OWASP's minimum). PHC string in one `TEXT` column, fresh salt
    per call. `verify_password` discriminates `Error::Password` (→ `Ok(false)`, a wrong
    password) from every other variant (→ `Err`, a corrupt stored hash) — one test pins each
    direction.
  - **Sessions** — opaque 32-byte CSPRNG token, hex-encoded, **SHA-256'd before storage** so a
    database leak yields digests rather than live sessions. Expiry checked in SQL; absent and
    expired both return `Ok(None)`; logout deletes the row by `token_hash`, so one device's
    sign-out doesn't end the others.
  - **Google OAuth** — authorize URL built with `Url::parse_with_params` (percent-encoding is
    the whole job), token exchange POSTed form-encoded per RFC 6749 §4.1.3, status checked
    before deserializing so Google's `invalid_grant` body survives into the error. Identity
    read from the **UserInfo** endpoint rather than the ID token — sound because that response
    arrives directly from Google over TLS with no untrusted intermediary, which is the
    reasoning to be able to state rather than the shortcut to take silently.
- [you] **Done.** Google Cloud OAuth client registered; `GOOGLE_CLIENT_ID`,
  `GOOGLE_CLIENT_SECRET`, `GOOGLE_REDIRECT_URI` in `.env` (gitignored).

  Two setup traps worth recording: copying `.env.example` verbatim leaves every line
  commented, so `dotenvy` sets nothing and the backend reports `Google OAuth not configured`;
  and `GOOGLE_REDIRECT_URI` must use the **same host you browse on** — `localhost` and
  `127.0.0.1` are different hosts to the cookie jar, so a mismatch means the OAuth state
  cookie isn't sent to the callback and the flow fails the CSRF check.

### Global review aggregator (added Phase 4, activates here)

Decided during Phase 4: reviews should eventually be shareable across users — anyone can
write one, opt in to publishing it, and see everyone else's public reviews. The **schema and
endpoints for this were scaffolded in Phase 4** (migration `0006_add_review_ownership.sql`)
because retrofitting ownership onto rows that never had it is far more painful than carrying
nullable columns for a phase. What's already in place: `reviews.user_id` (NULL pre-auth),
`reviews.is_public` (defaults 0 — opt-in, never opt-out), `reviews.hidden` (moderation
tombstone), `GET /recipes/{id}/reviews` (public wall for one recipe), visibility-scoped
reads (`fetch_for_viewer` / `fetch_visible_to`), a notes length cap, and a
`reviews::current_viewer()` seam that returns `None` until sessions exist.

- [gen] **Done.** `reviews::current_viewer()` is replaced by the `CurrentUser` extractor in
  `routes/auth.rs`; it landed in one place as predicted, with handlers passing `user.viewer()`
  into the `Option<&str>` parameters that were already threaded through. Backfill is
  `routes::auth::claim_unowned_rows`, run inside the first registration's transaction across
  all four owned tables. NULL means *unclaimed*, not public — those rows are invisible to
  every scoped read until then.
- [gen] Rate limiting on `POST /reviews`, and a moderation path for setting `hidden`. **Not
  done** — deliberately left, since neither is needed while the app is single-user behind a
  login, and both are easier to size once real usage exists.
- [learn] **Small-sample rating statistics.** Once reviews come from many users, a naive
  mean is badly behaved: one 5★ review must not outrank two hundred averaging 4.6★.
  Deliberately deferred from Phase 4 — with a single user there is no crowd to average, so
  the problem doesn't exist yet. Worth researching when you get here: Bayesian averaging /
  shrinkage toward the global mean (the IMDb Top 250 formula is the canonical reference
  implementation), Wilson score lower bounds if you ever collapse ratings to binary
  like/dislike, and Laplace/add-k smoothing as the crude first cut. Same rules as every
  other `[learn]` item — Claude discusses and reviews, you implement.
- [learn] **Weighting personal vs. global feedback** in `rerank_recommendations`. The
  plumbing exists (`Review::is_by(viewer)` distinguishes the two populations, and
  `fetch_visible_to` hands the function both in one slice); the weighting itself is yours.
  Note these are different signals, not one pooled average — your own history is
  personalization, the crowd's is a quality prior. See `src/rerank.rs`'s module doc.
- [learn] Optional, once a real multi-user corpus exists: collaborative filtering
  ("people who liked what you liked also liked X"). This is the condition Phase 4's notes
  set for reaching past simple weighted scoring — don't start here.

~~**Do not expose the review endpoints publicly before auth lands.**~~ Resolved 2026-08-13:
every route except `/health` and `/auth/*` now requires a session, the review endpoints
included. `GET /recipes/{id}/reviews` requires one too even though everything it returns is
public — nothing there is secret, but an unauthenticated endpoint on a backend bound to
`0.0.0.0` is exactly what this warning was about, and the app has no anonymous-browsing story
to justify the exception. `COOKIE_SECURE` still defaults off for plain-HTTP LAN use, so this
is a trusted network, not the open internet.

### Checkpoint — met 2026-08-15

Register an account with a password, log out, log back in; connect Google as an alternate
login method; confirm fridge/shopping/recipe data is scoped to your account (even though
you're the only user, verify the scoping actually works — e.g. a fresh second test account
sees an empty fridge). With two test accounts, confirm a public review written by one is
visible to the other while a private one is not, and that a recipe only the *other* account
rated highly never appears in your "Recipes you liked" section.

All of it verified, over HTTP and in-browser, against **copies of the real `fridge.db`** — not
hand-built fixtures. Copies rather than the original because the first registration
permanently claims the 22 pre-auth rows, and that slot belongs to the user's own account.

- Register → `HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000`, 64-hex token. Log out → session
  row deleted, old cookie 401s. Log back in → 200. ✅
- First registration logs `claimed 22 pre-auth rows`; the claim runs exactly once. ✅
- The plaintext session token appears in **no** column of `sessions` — only its SHA-256 digest. ✅
- **A fresh second account sees an empty fridge**: 0 items, 0 reviews, 0 liked. Suppression is
  per-account too — acct1 sees 787 recipes, acct2 all 789. ✅
- A public review by acct2 is visible to acct1; the private one is not. ✅
- A recipe only acct2 rated highly never enters acct1's liked list. ✅
- **Google**: consent screen → callback → identity linked on the `sub` claim (21 digits, not an
  email), no duplicate account created. ✅
- In-browser: `/fridge` redirects to `/login?next=…`, registration lands signed in with the
  claimed fridge rendered, liked section shows 8 with the Favorite badge on a real slot, sign
  out returns to `/login`. No console errors. ✅

Liked count (8) and general-recommendation count (787) match the Phase 4 checkpoint exactly —
the strongest available evidence that the backfill and viewer threading changed nothing they
shouldn't have.

`cargo test`: **113 passed, 0 failed**, clippy clean.

**Two bugs this checkpoint caught that the test suite could not**, both in scaffolding, both in
paths that had never executed:

1. `google_callback` added the state-removal cookie *before* reading the incoming state.
   `CookieJar::add` writes into the same map `get` reads, so every callback compared against an
   empty string and returned `OAuthStateMismatch` regardless of configuration.
2. `claim_unowned_rows` was wired only into password registration. Creating the first account
   via **Google** skipped it, stranding all 22 pre-auth rows permanently with no UI able to
   reach them. Now `claim_if_first_account`, called from both creation paths.

Neither was reachable before Google OAuth was configured — which is exactly why the checkpoint
is written as a real-data exercise rather than a test-suite target.

---

## After Phase 5

Not planned in detail yet: additional site tabs beyond the fridge app, deployment
(Vercel for `frontend/`, a small VPS or fly.io for the Rust backend + DB are reasonable
starting points), and whether Postgres replaces SQLite if you started there.

**Before deploying anywhere reachable from the internet**, three things that are deliberately
dev-shaped right now:

- `COOKIE_SECURE` defaults to **off** so cookies work over plain HTTP on a LAN. It must be on
  behind HTTPS, or session cookies travel in the clear.
- If the frontend and backend end up on genuinely different hosts (not just different ports),
  `SameSite=Lax` stops working and you need `SameSite=None; Secure` — which requires HTTPS on
  both ends. Same-host deployment avoids the problem entirely.
- Rate limiting on `POST /auth/login` and `POST /reviews`, neither of which exists. A login
  endpoint with no throttle plus Argon2's cost is also a cheap denial-of-service target.

The four deferred `[learn]` items above (small-sample statistics, personal-vs-global weighting,
collaborative filtering) all wait on a real multi-user corpus. Building them against a single
account means inventing the data they're meant to respond to.
