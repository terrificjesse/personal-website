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

## Phase 6 — Blog tab — **complete 2026-08-19**

**Not a Learning Mode phase.** Every task below is `[gen]` — Claude implements fully. The
flagged subsystems are auth, NLP, recommendations, and expiration; content management is none
of those. Don't stop at a boundary here.

**Already done (2026-08-19), documented in `docs/BLOG.md`:** `users.is_admin` + the
`RequireAdmin` extractor; `blog_posts` table; full CRUD behind admin; public `/blog` and
`/blog/[slug]` that work signed out; drafts hidden from non-admins.

### Remaining scope — all done

- [gen] **Markdown rendering.** **Done** — frontend `react-markdown` + `remark-gfm`, chosen
  over backend `pulldown-cmark` so `body` stays markdown source in exactly one representation
  (database, API, editor, and what search matches are all the same string). `react-markdown`
  builds a React tree rather than setting `innerHTML`, so raw HTML in a post is escaped and no
  sanitizer is needed — **`rehype-raw` would switch that off and must not be added casually.**
  Rendering lives in one component, `app/blog/MarkdownBody.tsx`, shared by the post page and
  the editor's preview.

  The non-obvious half was CSS: Tailwind's preflight resets headings and list markers, so
  `react-markdown` alone renders markdown that still *looks* like plain text. A scoped
  `.markdown-body` block in `globals.css` does that work, hand-written rather than pulling in
  `@tailwindcss/typography` for a third dependency.
- [gen] **Sort by date.** **Done** — `GET /blog/posts?sort=newest|oldest`, as a two-variant
  enum so `?sort=oldset` is a **400** rather than a silent fall back to newest.
- [gen] **Keyword search.** **Done** — `GET /blog/posts?q=…`, SQLite `LIKE` over title and
  body, with `%`/`_`/`\` escaped and `ESCAPE '\'`. Without the escaping a search for `100%`
  returns every post. FTS5 stayed out: it buys ranking, stemming, and phrase queries that
  nobody asked for, and costs a shadow table the file sync would have to write to as well.
  **Revisit when you want ranked results, not when you have more posts.**
- [gen] **Markdown files from git.** **Done** — design 1 below, `content/blog/*.md` synced
  into `blog_posts` with a `source` column. Author-facing docs are `content/blog/README.md`.
- [gen] Search + sort compose and cover both kinds identically. **Achieved literally**: there
  is no branch on `source` anywhere in the read path — `list_posts` builds one statement from
  a draft filter, an optional `LIKE`, and an `ORDER BY`.

### The file-ingestion decision — chose 1

Three shapes, in rough order of preference:

1. **Sync-on-startup — chosen.** Backend reads `content/blog/*.md` at boot and upserts
   into `blog_posts` with a `source` column (`'db'` | `'file'`). **One query path**, so sort
   and search work uniformly and no endpoint has to merge two stores. File-sourced posts
   become read-only in the admin UI. Cost: a restart (or an explicit re-sync endpoint) to pick
   up changes.
2. **Read-through at request time.** Always fresh, but every list/search/sort has to merge two
   sources and reconcile ordering — the complexity lands in the query layer permanently.
3. **GitHub API at runtime.** No redeploy needed, but adds a network dependency, rate limits,
   and a token if the repo is ever private.

Whichever is chosen: YAML frontmatter (`title`, `date`, `published`, optional `slug`) is the
conventional carrier for metadata, and slug stability rules from `docs/BLOG.md` still apply —
a file's slug must not change when its title does.

**What settled it** was the constraint above, not the ordering here: option 1 is the only one
where sort and search are written *once*. Options 2 and 3 merge two stores in the query layer,
so every future query feature gets implemented twice. The frontmatter parser is hand-rolled
(~30 lines, no crate) since the schema is four flat scalars; **an unknown key is an error**,
because a misspelled `pubished: true` would otherwise leave a post a draft forever with no
symptom but a post that never appears.

Decisions taken while building, each of which had a wrong answer that looks reasonable:

- **A file's slug comes from its filename, not its title** — the filename is the only identity
  a file has that editing its contents doesn't change.
- **`created_at` from frontmatter `date`, never file mtime** — mtime is reset by `git clone`
  and `git checkout`, so the blog would reshuffle itself on a fresh checkout.
- **`author_id` is the first-registered admin**; with no admin yet, the sync logs and skips
  instead of panicking at boot.
- **On a slug collision with a browser-authored post, the file loses.** Taking the slug would
  repoint an already-published URL at content nobody linked to.
- **Sync mirrors rather than imports** — a deleted file deletes its post — and every write is
  scoped to `source = 'file'`.
- **File posts are read-only through the API (409), not merely hidden in the UI.** The next
  sync would overwrite an accepted edit, so accepting one is a lie.

Publishing is automatic: a background task polls a `(filename, mtime, size)` fingerprint of the
directory every `BLOG_SYNC_INTERVAL_SECS` (default 5) and syncs only when it changed. Startup
sync and admin-only `POST /blog/sync` both remain.

Polling rather than `notify` because **`sync` makes a spurious trigger cost a no-op** — it only
counts an update when content genuinely differs. False positives are free, false negatives are
a missed post, so an imprecise detector is the right tool, and it costs zero new crates against
`notify` plus a debouncer.

### Checkpoint — met 2026-08-19

Every clause verified against a throwaway copy of `fridge.db`, with `curl` for anything about
authorization and the browser for anything about rendering. Full matrix in `docs/BLOG.md`.

- Post written in the browser renders as markdown — headings, lists, code, GFM table. ✅
- A `.md` file appears alongside them; delete the file and its post goes with it. ✅
- `?sort=oldest` flips the order for **both kinds in one list**. ✅
- A term appearing only in a file-sourced post's body returns it. ✅
- Signed-out visitor sees published posts of both kinds and no drafts; a draft slug 404s. ✅

Beyond the checkpoint: `?q=%` returns only posts containing a literal `%` (the `LIKE`-escaping
bug it would otherwise have); `?sort=bogus` is a 400; `PATCH`/`DELETE` on a file post is 409
while a db post still edits normally; a `<script>` typed into the editor renders as text with
**zero** `<script>` elements in the DOM.

`cargo test`: **135 passed** (117 + 18 new), clippy clean, `tsc --noEmit` clean.

The one bug found, caught by the compiler rather than a test: `sync` accumulated its skip count
in a local that never reached the returned report, so `skipped` would always have read `0`. It
surfaced as an `unused_assignments` warning — a test asserting on the count would have had to
exist first, and the warning needed nothing.

**Added beyond the plan:**

- A `PORT` env var in `main.rs` (defaults to 8080), so a second backend instance can run
  against a throwaway database while the usual one keeps serving. That is how this checkpoint
  was verified without stopping anything.
- **Auto-sync** (`blog_files::spawn_watcher`), added at the user's request after the checkpoint
  — the plan had explicitly deferred a watcher. Verified live: dropped file ~3s, edit ~6s,
  delete ~6s, and no log output on quiet ticks.
- A **"Write a post"** button on `/blog`, shown only to admins. Nothing had ever linked *to*
  `/blog/admin` — it linked out to `/blog` and nothing linked back — so the editor was
  reachable only by typing its URL. That gap shipped with blog v1 and went unnoticed here
  because verification navigated straight to the admin URL rather than trying to find it.


---

## Phase 7 — Internship tab (scraper + ranking + applied tracker)

**Not a Learning Mode phase, and that includes the ranking.** The user decided this explicitly
on 2026-08-20. `rank_postings` is a scoring-and-ordering algorithm and so looks exactly like
the `[learn]` work in Phases 2–4 — it is nonetheless `[gen]`. Do not re-litigate it.

The one open Learning-Mode question is **dedup's fuzzy company/title matching**, which is the
NLP area's shape. Reuse `src/nlp.rs` if it fits; if it doesn't, **ask before writing a second
matcher** rather than assuming the `[gen]` exception covers it.

**Goal:** collect open SWE internship postings from several sources, normalize and dedup them,
rank them, let the user record which ones they applied to, and drop postings once they close.

### Where it lives

Backend in `apps/fridge-app/backend/` (auth and `users` are there — same reasoning as the
blog, see root `CLAUDE.md`). Frontend at `frontend/src/app/internships/`. Vendored source
snapshots under `apps/fridge-app/backend/data/internships/`, following `data/themealdb/`.

**Full reference — every file and function — is `docs/INTERNSHIPS.md`** (written 2026-08-29,
after the fact). This section stays the phase record: scope, decisions, and the checkpoint.

### Sources — all four classes, isolated from each other

Chosen by the user with the tradeoffs stated. `docs/INTERNSHIP_SCRAPING.md` holds the
per-source research: endpoints, real field names, and a **field-availability matrix** that says
which of the ranking's inputs each source actually provides.

| Class | Expectation |
|---|---|
| ATS public JSON APIs (Greenhouse, Lever, Ashby) | Primary. Structured, stable, no HTML parsing |
| GitHub internship-list repos (Summer 2026/2027) | Breadth and cold-start corpus |
| RSS / JSON feeds where offered | Supplement |
| LinkedIn / Indeed / Handshake | **Best-effort. Expected to yield little.** Never on the critical path |

The **scraping rules in the root `CLAUDE.md` are binding**: per-source isolation, fail fast,
every failure recorded in `source_runs` and the log, and **no detection evasion** — identify
honestly, respect `robots.txt`, rate-limit, and give up on a source that pushes back rather
than working around it.

### Scope

- [gen] **Source adapters** behind one trait, each returning either postings or a recorded
  failure. Adding a source must not touch the runner.
- [gen] **Run record** (`source_runs`): source, started/finished, count, outcome, error. A
  source that silently returned zero must be distinguishable from one that had zero.
- [gen] **Normalization + QC**: parse pay into a numeric range with currency and period; parse
  term/season; normalize location and a remote flag; parse class-year eligibility. Reject or
  flag rows that don't survive it, and **make the rejects visible** — silent drops are how a
  scraper looks healthy while losing half its data.
- [gen] **Dedup** across sources. Merge key plus fuzzy fallback — see the NLP note above.
- [gen] **Ranking** — `rank_postings`. Composite over pay, posted date, deadline proximity,
  location match, class-year fit, and a **derived** prestige signal (the user chose derived
  signals over a hand-maintained tier list). Two rules carried from Phases 3–4, which were
  learned the hard way: **user-supplied filters are hard filters, not scoring inputs**, so
  results stay predictable; and **every continuous score needs an explicit threshold** —
  `>` vs `>=` silently excluded every 4★ recipe in Phase 4.
  **Missing data is the central difficulty here, not the weighting.** Pay is absent from most
  sources; a posting with no salary must not be ranked as though it pays zero. Decide and
  document what absent means for each input.
- [gen] **Filters**: term, location/remote, class year, pay floor, source, company.
- [gen] **Applied tracker**: per-user, with status (applied → OA → interview → offer/rejected),
  applied date, and notes.
- [gen] **Expiry sweep**: drop postings past their deadline, and postings that have vanished
  from their source for N consecutive runs. Cadence via env var, same shape as
  `BLOG_SYNC_INTERVAL_SECS`.
- [gen] Frontend: ranked list with filter controls, an applied-tracker view, and a run-health
  panel so a quietly broken source is visible rather than merely absent.

### Two design traps to settle before writing the schema

1. **An applied posting must survive expiry.** If the sweep deletes a posting the user applied
   to, their application history is orphaned or lost. Snapshot company/title/URL/pay onto the
   applied row, or soft-delete and keep it joinable. Decide before the migration, not after.
2. **Disappearance is not closure.** A source erroring, being rate-limited, or reshaping its
   response also makes postings "vanish." **The expiry sweep must only count a disappearance
   from a run that actually succeeded**, or one blocked LinkedIn fetch silently expires
   everything it ever supplied. This is the single most likely data-loss bug in the phase.

### Checkpoint — met 2026-08-20, with one clause verified by test rather than live

Verified against a **real collection run**, not fixtures: 2,746 rows fetched from Simplify,
Greenhouse, Lever, Ashby and WeWorkRemotely in 22 seconds, capped at
`INTERNSHIP_MAX_BOARDS_PER_RUN=6` so the run was a few dozen polite requests rather than the
~2,084-board, half-hour sweep an uncapped run performs.

- **The others still land when a source doesn't.** LinkedIn and Indeed recorded `skipped` with
  their honest `robots.txt` reasons and **made zero requests**; three ATS sources recorded
  `partial` on hitting the board cap; Simplify still returned 924 accepted postings. Every
  outcome carried a reason into `source_runs` and the log. ✅
- **A posting present in two sources appears once.** `AfterQuery — Software Engineering Intern`
  arrived from both `simplify` and `ashby` and is one posting with two sightings. Beyond the
  checkpoint, the key also merged **65 postings that one source had exploded per-location** —
  RTX 14 listings, American Express 8, TikTok 7 — which is exactly the multi-location
  double-counting § C warned about, and the reason location is not in the key. ✅
- **Every fetched row is accounted for.** `fetched = accepted + filtered + rejected`:
  2,746 = 926 + 1,820 + 0. The 1,820 filtered are non-SWE-internship rows, which is healthy
  bulk; **0 rejected** means nothing that should have parsed didn't. ✅
- **A posting with unknown pay is neither first nor last.** Only 2 of 808 postings state pay.
  Under `sort=composite` they land at ranks **247 and 634 of 808** — mid-pack — while both
  first place (0.615) and last place (0.386) are unknown-pay postings. Under `sort=pay` the two
  stated figures lead and every unknown follows. ✅
- **Pay parses to the right magnitude.** `USD 10000.00 per month` → `10000.00 USD/month`, not
  an hourly rate; `USD 30.00 - 35.00 per hour` → `30–35 USD/hour`. Ashby's explicit interval
  survived stringification and beat the magnitude heuristic, which is the whole point of the
  `pay_raw` contract. ✅
- **An applied posting survives expiry with its details intact.** Applied to a real posting,
  then expired it, then deleted the row outright. The application kept company, title, URL,
  pay, term and notes through all three states; `posting_is_live` went `true` → `false` →
  `null`, and **`null` renders no badge at all** rather than claiming "Closed", which would be
  a lie. ✅
- **A failed run does not expire that source's postings.** ⚠️ **Verified by test, not live.**
  No source hard-failed during the real run — they succeeded, hit the board cap (`partial`), or
  skipped. The behaviour is pinned by
  `collector::integration_tests::a_failed_source_does_not_expire_the_postings_it_previously_supplied`
  (three consecutive failed runs leave `consecutive_misses` at 0 and the posting live), by its
  non-vacuous counterpart `a_genuine_disappearance_from_successful_runs_does_expire`, and by
  the run-health panel rendering `counts_for_expiry = false` against seeded failure data. A
  live hard failure is worth catching opportunistically the first time a source really breaks.

`cargo test`: **510 passed**, clippy clean, `tsc`/`eslint` clean.

### What the real run caught that the test suite could not

Both are dedup bugs, both invisible to 510 green tests, both found within minutes of real data —
the pattern `apps/fridge-app/CLAUDE.md` records for four earlier scoring functions.

1. **`job-boards.eu.greenhouse.io` is a third Greenhouse hostname**, not recorded in
   `INTERNSHIP_SCRAPING.md`, which lists only `boards.` and `job-boards.`. Postings on the EU
   host fell through to the fallback key.
2. **`boards.greenhouse.io/embed/job_app?token=N`** puts the job id in the query string —
   which every other URL shape treats as strippable tracking noise, so it was discarded.

Both were **over-merging**, the dangerous direction: without an ATS key these collapsed into
`company|title`, so two distinct jobs at one company sharing a title became one row. After the
fix, ATS-triple coverage went **266/804 → 285/808**, and the four extra postings are jobs that
had been wrongly merged into each other.

### Known gaps, recorded rather than fixed

- **ATS-triple coverage is ~35%, where § C predicted 73%.** The shortfall is entirely ATS
  platforms `dedup::ats_identity` does not parse: **Workday** (`*.myworkdayjobs.com`,
  `*.myworkdaysite.com` — listed in § C's own table and never implemented), plus
  `apply.workable.com` and `ats.rippling.com`, which § C does not mention at all. Everything
  else in the fallback is a company's own careers page (TikTok 82, Tesla 51, ByteDance 38,
  Apple 7), which correctly has no ATS identity.
- **Pay coverage in this run was 2 of 808 (0.2%)**, far below § B's "well under half". That is
  an artefact of the board cap, not a defect: Simplify supplied 924 of 926 accepted postings
  and carries no salary at all, while the ATS sources that do were capped at 6 boards each. An
  uncapped run is the only way to measure this honestly.
- **A capped run can never expire anything.** `INTERNSHIP_MAX_BOARDS_PER_RUN` makes a source
  report `partial` by construction, and `partial` is not permitted to advance disappearance
  counters. Convenient for development, wrong for steady state — an uncapped run is required
  for expiry to function at all.
- **Fuzzy company/title matching is still an unimplemented seam** (`dedup::FuzzyMatcher`), so
  `KLA` / `KLA Corporation` remain two postings. Under-merging, which § C calls the safer
  failure.
- The frontend's filters are **not URL-synced**, so a filtered view cannot be bookmarked or
  shared. The backend accepts every parameter; only the page ignores them.

---

## Phase 8 — Internship-hunt tooling (inbox agent + Firefox extension)

**Not a Learning Mode phase, and that includes the email classifier and the email→application
matcher.** The user decided this explicitly on 2026-08-29: *"This is meant to be a tool for me
that I want quickly, so I don't want to be writing any of it."* Classification is NLP-shaped
and matching is fuzzy-matching-shaped, and both are `[gen]` here. This is the second such
exception after Phase 7's ranking — do not re-litigate either, and **do not stop at a stub
boundary and hand back a signature.**

The exception does not reach the `[learn]` files themselves. `src/nlp.rs` and its neighbours
may be **called** and never edited — including to fix a compile error they throw. If Phase 8
needs one of them to change, say so and stop.

**Full rules are `apps/hunt-extension/CLAUDE.md`** — read it before touching any of this; it
governs Phase 8 wherever the code lives. The reference for what exists is `docs/HUNT.md`.

### Where it lives

The extension is `apps/hunt-extension/`. The backend half is `apps/fridge-app/backend/` —
`src/hunt/` for the alert channel, `src/inbox/` for the Gmail agent when it lands — for the
same reason the blog and internship tabs are there: auth and `users` are there. That makes it
the *fourth* tab in a folder named after the first one. The root `CLAUDE.md` already calls that
name a lie; **extracting it is its own deliberate change and must not be bundled into Phase 8.**

### Two tracks, and B does not depend on A

| Track | What | Needs |
|---|---|---|
| **A — inbox agent** | Read a burner Gmail, match mail to applications, propose status transitions, project them onto Gmail labels | OAuth, a Google Cloud project, an Anthropic key |
| **B — filling applications** | Autofill CV details into ATS forms, plus an answer library for questions already answered well | Nothing |

**8e + 8f is the shortest path to something useful every day.** Track B needs no OAuth and no
API key at all.

### The one structural idea

**The four email categories already exist in the database.** Phase 7 shipped
`internship_applications.status` — `applied → oa → interview → offer → rejected` — which is
exactly "confirmation folder / OA folder / interview folder". So the classifier's job is **not**
"sort this email into a folder"; it is *match this email to an application row, and propose a
status transition.* Gmail labels are written afterwards as a **projection** of application
status. Build it the other way round and you get two taxonomies that drift, and a tracker still
reading `applied` for a job you already interviewed at.

### Build order — classification earns write access, it does not start with it

- [gen] **8a — Read-only pipeline. — complete 2026-08-31.** Verified against the real burner
  inbox: 10 messages synced and recorded, `outcome=success`, and a second pass reported
  `already_seen=10, classified=0` — rule 4's no-op, live. Rule 7's invariant balanced
  (`10 = 0+0+0+10`), the `historyId` watermark was stored for the next incremental pass, and
  **nothing was written outside our own tables** — no labels, no status changes, no alerts.
  Every message classified `disregarded` because the stub says so rather than guessing.

  Two failures worth keeping: the callback read its state cookie *after* clearing the jar, so
  every consent 400'd and the flow could not have worked once — invisible to 677 tests because
  the path needs Google at the other end, which is the Phase 5 lesson in the same function it
  was learned in. And a stale `cargo run` debug binary held port 8080, so a freshly built
  release binary failed to bind and the *old* build kept answering, which read as a missing
  route.
- [gen] **8b — Classify + match, still no writes.** Rules first, Claude API on ambiguity.
  Verdicts stored. *Checkpoint:* against a hand-labelled set of **real burner-inbox mail across
  a whole two-week window**, not 50 curated job emails — a curated set contains no newsletters,
  so it cannot measure the relevance gate, which is the highest-volume decision in the system.
  Measure junk leaking into `Hunt/Outreach` and **real mail getting disregarded** separately;
  the second is the one that costs an interview.
- [gen] **8c — Writes.** Gmail labels + `status_proposals`. *Checkpoint:* a late-arriving
  autoresponder does not drag an interview back to `applied`.
- [gen] **8d — The email producer.** Classified mail writes `hunt_events` rows. Depends on the
  table, which 8e built.
- [gen] **8e — The extension shell, end to end. — complete 2026-08-30**
- [gen] **8f — Autofill. — complete 2026-08-30**
- [gen] **8g — The answer library.** Save answers, similarity retrieval, company-specific
  flagging. *Checkpoint:* a "why do you want to work here" answer stored against one company is
  **not** offered for another, and a genuinely reusable one ("a project you're proud of") is.

### The traps, in one place

Each is a real failure mode, not a style preference. `apps/hunt-extension/CLAUDE.md` carries all
twelve with their reasoning; these are the ones that shape the schema.

1. **Email is untrusted content, and the classifier sits upstream of Gmail write access.**
   `classify` is a pure function that gets no tools and returns a constrained enum — never an
   action, a label name, or SQL. Every write happens in Rust, outside the model call. Never
   fetch a URL found in an email.
2. **A misclassification must never silently rewrite the tracker.** Every email-driven change
   writes a `status_proposals` row linking back to the message. Auto-apply only above the
   confidence threshold and only forwards; **never auto-apply `offer` or `rejected`.**
3. **Status advances; it does not follow the newest email.** Email order is not event order.
4. **"Disregarded" means unlabelled, not unrecorded.** This is `posting_rejects` one subsystem
   over: a dropped email that leaves no trace makes "correctly ignored 400 newsletters" and
   "ate an OA" produce identical output. Pin
   `classified = pressing + confirmation + outreach + disregarded` with a test.
5. **Category is decided before the match, and a pressing email is never disregarded.** The
   matcher is fuzzy and will miss; if unmatched routed to disregard, one miss silently eats an
   interview invite. An OA email matching no application is still labelled and still alerted.
6. **Autofill never fires on its own and never submits.** Explicit user action only; hard
   blocklist on password/payment/SSN fields checked *before* the fuzzy mapper; EEO questions
   opt-in and default off. **Do not ship `<all_urls>`.**

### 8e — complete 2026-08-30

**The vertical slice that ends in a desktop notification**, and it needed no Gmail and no API
key: `hunt_events`, the poll and ack endpoints, the posting producer, and the extension shell.
It proves the whole alert path before the inbox agent exists. Full reference: `docs/HUNT.md`.

Two producers were designed in from the start (`kind` is `posting | email`) so 8d adds a
producer rather than a pipeline.

Verified against a **real Simplify run over a copy of the live database**, not fixtures:

- **A new tier-1/2 posting writes exactly one row.** 2,247 fetched, 206 postings created,
  **22 alerts, every one a tier-1 or tier-2 company.** Tier-3 controls produced none. ✅
- **Re-running collection writes no second event.** A second run updated 1,097 postings and
  reported `alerts_created: 0`. Dedup is structural — `UNIQUE (kind, subject_id)` — not a
  caller remembering to check. ✅
- **The alert predicate is the existing `prestige::CompanyTiers::tier()`, tiers 1 and 2.** No
  new ranking code. **`None` is not tier 3**: unlisted means *unknown*, the curated file names
  44 of ~455 companies, and alerting on unknown would alert on nearly everything. ✅
- **Endpoints:** `204` first ack, `204` repeat, `404` unknown, `401` signed out, `400` on a
  malformed `since`. ✅
- **The extension's logic against the live backend**, driven with a stubbed WebExtension API:
  10 waiting events → 3 notifications plus one "+7 more", all 10 acked, immediate second poll
  raised nothing. All three failure modes distinct and badged. ✅

**Notification dedup is the server's job** (`hunt_events.acked_at`), because an MV3 background
page is killed and restarted at the browser's convenience and anything it remembers is lost.
`browser.storage.local` is a cache, never the record. `acked_at` is a **delivery receipt** — the
extension acks what it showed — so the popup lists *recent* events rather than unacked ones.

**What the real run caught that 22 green tests could not.** Simplify packs every city into one
location string, so a single Google posting produced a **429-character** notification body with
the role pushed off the end — the normal shape of a big-company listing, since `dedup`
deliberately keeps location out of the merge key. Locations now collapse to `first +N more`,
body capped at 140 characters, pinned by a test using the real 30-city string. Same pattern
`apps/fridge-app/CLAUDE.md` records for four earlier scoring functions.

**Verified in Firefox 2026-08-30, and the cookie plan did not survive it.** `SameSite=Lax`
means Firefox never attaches `fridge_session` to a request from a `moz-extension://` page, so
the backend answered a truthful 401 to a signed-in user. The recorded fallback — a dedicated
bearer token — is now what the extension uses: `hunt_tokens` (migration `0015`), minted from an
**Extension access** panel on the internships tab. It is a second *credential*, not a second
auth system: it reuses `auth`'s hashing by calling it, and is accepted inside the existing
`MaybeUser` extractor, so every route keeps its `CurrentUser` signature.

Three other faults wore that same "can't reach the backend" symptom on the way: the dev servers
genuinely dying, Firefox MV3 declining to grant `host_permissions` the manifest merely requests
(Chrome grants them at install), and CORS never naming the extension's origin so responses were
discarded before the extension could read them. **The durable fix is that all four now report
themselves distinctly** — `unpermitted`, `unreachable`, `no-token`, `token-rejected` — because
one message covering four unrelated causes is what turned a ten-minute check into a day. 8f
authenticates identically and inherits the diagnosis rather than the search.

### 8f — complete 2026-08-30

Content script, label-based mapper, `cv_profile`, the `activeTab` path and the
"track this application" offer. Full reference: `docs/HUNT.md`.

**All three checkpoint ATSs fill from a real posting**, including the hardest variant: the
Greenhouse one was reached through a *company careers page* (`jumptrading.com/hr/job?gh_jid=…`)
that embeds `job-boards.greenhouse.io` in an iframe, exercising `activeTab`, cross-frame
injection and frame selection at once. Lever and Ashby are direct ATS pages on the declarative
path.

**Reading the live forms before filling them found three defects a synthetic form could not**,
and one was in the safety layer:

- **The demographic blocklist silently failed on Lever.** It renders a `<select>`'s option text
  into the label with no separator, so "Gender" arrives as `GenderSelect ...MaleFemale…` —
  normalized `genderselect`, and `\bgender\b` never fires. Race the same. Veteran status was
  caught *only* because its pattern happens to lack a word boundary, which is what made the
  failure look like success. Refusal checks now also see the label with run-together words
  split; matching deliberately does not, since the same split breaks `LinkedIn` and `GitHub`.
- **"Other website" was filled with the portfolio URL**, and sits beside "Portfolio URL" on
  Lever — one URL in two different questions.
- **Ashby labels its name field simply "Name"**, which was excluded on purpose because it
  matches inside "Company Name". Now an exact-only match, with tests holding that line.

CAPTCHA fields are also refused outright: Ashby renders a real `g-recaptcha-response` textarea
into the form. Nothing mapped to it, but "nothing happens to match" is not a policy and rule 11
is.

Every label from all three live forms is now a test — 79 checks against markup that exists.

**The other three clauses hold too**, each confirmed on a live form rather than inferred:
values survive clicking around a React-controlled form, so the native-setter path really does
register with the framework rather than being wiped on the next render; nothing fires on page
load, which is rule 10's core promise; and no EEO field is touched — refused by the classifier,
pinned by tests, and visibly untouched on Lever's gender, race and veteran selects.

Nothing was submitted on any of the three.

### Open questions

- Should `Hunt/Outreach` raise a notification? **Currently no.** Cold outreach is high-volume
  and low-precision, and a noisy channel gets muted wholesale, taking the OA alerts with it.
  8e made this a one-line predicate plus an existing checkbox in the options page.
- Confidence threshold for auto-apply — set it after 8b gives real numbers, not by guessing.
- Does the extension need the internship *list*, or only alerts? 8e shipped alerts only.
- How does an answer first get into the library? Cheapest: after a fill, offer to save what you
  typed into the free-text boxes — also the version most likely to catch answers while good.
- Does the answer library want embeddings, or is `strsim` over normalized question text enough?
  Start with `strsim`; it is already a dependency and the corpus is tiny.

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
