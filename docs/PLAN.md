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
- [gen] **8b — Classify + match. Rules layer built; checkpoint NOT met — see below.** Rules first, Claude API on ambiguity.
  Verdicts stored. *Checkpoint:* against a hand-labelled set of **real burner-inbox mail across
  a whole two-week window**, not 50 curated job emails — a curated set contains no newsletters,
  so it cannot measure the relevance gate, which is the highest-volume decision in the system.
  Measure junk leaking into `Hunt/Outreach` and **real mail getting disregarded** separately;
  the second is the one that costs an interview.
- [gen] **8c — Writes. — complete**, in two commits and in the order the build order demands:
  `c976955` shipped the reversible half (`status_proposals` — propose a status change, never
  make one silently), and `f911f46` shipped the irreversible-feeling half (Gmail labels,
  creating them when they do not exist). Both halves are in `src/inbox/`; `labels.rs` is the
  only module in the crate that modifies a mailbox, kept apart from the read-only `gmail.rs`
  by a test that fails the build if a write call appears there.

  *Checkpoint:* ⚠️ **met by test, not live.** `advance::rank` / `is_terminal` implement rule 3,
  and `a_late_autoresponder_cannot_drag_an_interview_back_to_applied`
  (`src/inbox/advance.rs:129`) pins it, alongside eleven siblings covering terminal arrivals,
  same-status no-ops, and the auto-apply refusals. The live version of this checkpoint needs a
  real late autoresponder to actually arrive, which is the same mail Phase 13 is waiting on.

  **Two defaults chosen here that Phase 10 revisits:** label writing is **on** by default
  (`INBOX_APPLY_LABELS=false` opts out — `sync.rs:354`), and auto-apply is **off** by default
  (`auto_apply_threshold()` returns `None` unless `INBOX_AUTO_APPLY_CONFIDENCE` is set —
  `sync.rs:414`). The second is this phase declining to guess a number 8b has not produced.
- [gen] **8d — The email producer. — complete 2026-08-31** (`95f0443`). Pressing mail raises a
  desktop alert through the channel 8e built: `sync.rs:222` emits a `kind = 'email'` event
  keyed on the Gmail message id, gated on `verdict.category.is_pressing()` (`sync.rs:245`) so
  only an OA, an interview or an offer interrupts you. It added a producer and changed nothing
  about the table, the poll, the ack or the extension — which is what the two-producer shape
  was for.
- [gen] **8e — The extension shell, end to end. — complete 2026-08-30**
- [gen] **8f — Autofill. — complete 2026-08-30**
- [gen] **8g — The answer library. — checkpoint met at the HTTP layer 2026-08-31; the browser
  half is still unverified.** Save answers, similarity retrieval, company-specific flagging.
  *Checkpoint:* a "why do you want to work here" answer stored against one company is **not**
  offered for another, and a genuinely reusable one ("a project you're proud of") is.

  Both halves of that are now asserted through the real handlers in
  `routes::hunt::answer_loop_tests`, driven with the **exact request shapes `popup.js` builds** —
  its `?q=`/`&company=` query string and its three-field save body — because the seam between
  the extension and the routes is in two languages, invisible to the compiler, and is what
  "never closed by hand" would actually have caught. A renamed query parameter degrades to *no
  suggestions*, which is indistinguishable from an empty library; the mutation that proves the
  tests bite also shows the withholding assertion still passing under it, which is why the loop
  needs asserting in **both** directions rather than just the safe one.

  **What is still not verified is everything inside the browser**: whether `questions()` finds
  the free-text boxes on a real ATS form, whether `describePage()` names the employer, and
  whether the popup's Save and Suggest buttons behave against a live page. That needs the
  extension loaded in Firefox on two real forms — it cannot be reached from here, and jsdom
  would be a new npm dependency in a folder that is deliberately plain JS with no build step.

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

### 8b — measured 2026-08-31, and the checkpoint is not met

Run honestly, and the honest result is that **the checkpoint cannot be met yet and this is not
it.** Recorded because a partial measurement stated as partial is worth more than an unmeasured
classifier, and far more than a number that looks like a pass.

**Why it is not the checkpoint.** It asks for a hand-labelled set of every message across ~2
weeks. The burner holds **14 messages over 2 days, with zero digests, newsletters or staffing
blasts** — so the relevance gate, which the checkpoint singles out as the highest-volume
decision in the system, has nothing to measure against at all.

**And most of the corpus was already spent.** 8 of the 14 were the mail the rules were written
by reading, with three defects fixed against them. Grading on those measures the tuning, not the
classifier, and would have returned ~100%.

**What was measurable:** 6 messages arrived after the rules were committed. On that held-out
set, hand-labelled by the user rather than by the author of the rules:

| | |
|---|---|
| Correct | **4 of 6** |
| Junk leaked into `Hunt/Outreach` | **1** — an event RSVP confirmation |
| Real application mail disregarded | **1** — an ATS account-setup email |

Both failure modes the checkpoint names, one instance each, on six messages. Both diagnosed and
fixed:

- The RSVP reached outreach because its sender was `…@connect.roblox.com` — the domain matched
  a known company and the address did not contain "noreply", which is a thin basis for deciding
  a human wrote to you. Event RSVPs and registrations now fall to the relevance gate.
- The account-setup mail said "Thank you for **expressing** interest in", and only the "your
  interest in" phrasing was listed. It also came from `msg.paycomonline.com` — Paycom, an ATS
  nothing recognised, the same shape as Phase 7's ATS-coverage gap one subsystem over.

All 14 now agree with the user's labels, and both real strings are pinned as tests.

**The held-out set is now spent.** Fixing against it made those six in-sample too; a set can
only be measured once. The next honest measurement needs mail that arrives from here, and the
relevance gate stays unmeasured until digests and staffing blasts actually turn up.

**The harness for the next attempt is built** — `src/inbox/labelset.rs`, reached as
`cargo run --release -- labelset export|score`. It exports *every* stored message to a CSV with
an empty label column (no verdict shown, so the labels are not anchored), re-runs the rules over
the filled-in sheet, and reports the two failure modes against separate denominators, with no
single accuracy figure to quote instead of them. It keeps a ledger beside the labels file
recording the fingerprint of the rules each grading ran under, so a set that was graded and then
tuned against reports itself as in-sample rather than silently passing as fresh.

It surfaced two defects before a single message had been labelled.

**Fixed — `guess_company` matched a company as a bare substring** (2026-08-31). The known-company
list is real and holds 586 names, including three-letter ones, so a bare `contains` matched the
*inside of ordinary words*. The worst case was not in the sender at all: **PPL** is a utility in
the corpus, and "a*ppl*y" / "a*ppl*ication" appear in **9 of the 14** messages in the burner
inbox. Longest-match-wins hid it in eight of them; in the ninth it was the guess, and
`company_guess` is what `advance::match_application` keys on — rule 2's failure exactly, an
email matched to a company it has nothing to do with. Three more, all verbatim from live mail:
`systemmessage@paycomonline.com` named **Sage** ("mes*sage*"), `donotreply@msg.paycomonline.com`
named **KLA** ("O*kla*homa City Thunder"), and `jobs@ziprecruiter.com` named **Zip** — the last
being the same bug pointed at the relevance gate, where a job-board digest "names a specific
employer" and stops looking like junk.

The fix requires a non-alphanumeric boundary on both sides of the match, compared as `char`s
rather than bytes so a two-byte letter is not mistaken for a delimiter. Measured over every real
message: **11 of 14 guesses unchanged, 3 changed** — two false positives removed, and one *true*
positive recovered (`Zip Hiring Team <no-reply@ashbyhq.com>` guessed `ppl` before and now
guesses `zip`). Every legitimate match in the corpus — Roblox, Tesla, Jump Trading, Epic Games,
Google — survives, via a whole domain label or a display-name word.

**Still open — `is_machine_sender` does not know `systemmessage@`**, so that sender reads as a
person. No effect on today's mail (`paycomonline.com` is an ATS domain and reaches outreach
either way), but it is the same shape of gap.

### Open questions

Three of these were answered by the code that shipped; the answers are recorded here rather
than left as questions, because an open question nobody closes is read as undecided forever.

- **Should `Hunt/Outreach` raise a notification? Answered: no**, and it is now enforced rather
  than merely intended — 8d's producer runs behind `verdict.category.is_pressing()`
  (`src/inbox/sync.rs:245`), so outreach is labelled and recorded but never interrupts. Cold
  outreach is high-volume and low-precision, and a noisy channel gets muted wholesale, taking
  the OA alerts with it.
- **Confidence threshold for auto-apply. Answered: deliberately unset.**
  `auto_apply_threshold()` returns `None` unless `INBOX_AUTO_APPLY_CONFIDENCE` is set
  (`src/inbox/sync.rs:414`), so nothing auto-applies today. Setting it is task 13e, after 8b's
  checkpoint produces a real number — guessing it would invent the measurement it is meant to
  come from.
- **Does the answer library want embeddings, or is `strsim` enough? Answered: `strsim`**, and
  it shipped — `strsim::normalized_damerau_levenshtein` over normalized question text, with a
  floor below which nothing is offered at all (`src/hunt/answers.rs:243`). Revisit only if
  real retrieval starts missing, not on principle.
- Does the extension need the internship *list*, or only alerts? **Still open.** 8e shipped
  alerts only and Phase 9 added an inbox-status line; neither made a list feel missing.
- How does an answer first get into the library? **Still open.** Cheapest: after a fill, offer
  to save what you typed into the free-text boxes — also the version most likely to catch
  answers while they are still good.

---

## Phase 9 — Make the hunt tooling usable daily

**Not new capability. The gap between built and used.**

Phase 8 works and is driven entirely by hand: the inbox syncs only when something POSTs to it,
status proposals are reachable only by `curl`, and nothing anywhere shows whether the agent is
alive. A tool that needs an operator does not get used during an actual internship hunt — and
being used is also what produces the corpus 8b's checkpoint needs, so this unblocks that too.

### Scope

- [gen] **An interval worker for the inbox.** Same shape as `BLOG_SYNC_INTERVAL_SECS` and the
  collector: cadence from an env var, `0` disables, and it **never fetches from a request
  handler** — the root `CLAUDE.md` cache rule applies to Gmail as much as to a job board.
- [gen] **A proposals review panel** on the internships tab. Accept or reject a status change,
  with the email that caused it visible beside it — rule 2's audit trail is worthless if the
  only way to read it is SQL.
- [gen] **Inbox status in the extension.** Whether an account is connected, when the last run
  was, and its outcome. Rule 5 says a broken sync must be visible, and "visible" has so far
  meant a JSON endpoint nobody opens. The 7-day token expiry makes this the difference between
  noticing in an hour and noticing in a fortnight.

### What this phase is not — ~~held~~ **overtaken, 2026-08-31**

This said: *not the Gmail label writes; those are 8c's remaining half and stay held until 8b has
met a real corpus.* They did not stay held. `f911f46` shipped them the same week, with
`labelling_enabled()` **on by default**, and 8b's checkpoint is still unmet.

Kept rather than deleted, because the disagreement is the useful part. The code's reasoning is
recorded at `src/inbox/sync.rs:351` and is not unreasonable — the granted `gmail.modify` scope
withholds permanent delete and send, `labels.rs` never removes a label, never archives and
never touches a disregarded message, so the worst case is a visible, removable, wrong label on
a message you can still find. That is a genuinely different risk from the one this paragraph
was written about.

But *the plan said held and the code says on*, and nobody reconciled the two until Phase 10a.
**Task 10k makes it an explicit decision** before 10h puts the agent on a host that runs
unattended against a real mailbox.

### Status — the code landed in `c001445`, the checkpoint was never written (recorded 2026-09-02)

All three scope items above shipped **in the same commit that added this section**, which is
why none of them is ticked:

- The interval worker — `inbox::sync::spawn`, `INBOX_SYNC_INTERVAL_SECS`, called from
  `main.rs:93`, spawned rather than awaited so a slow Gmail cannot delay startup.
- The proposals panel — `frontend/src/app/internships/InboxPanel.tsx` (200 lines) against
  `GET /hunt/proposals` and `POST /hunt/proposals/{id}/{accept,reject}`.
- The extension's inbox line — `popup.js:597`, reporting *no account* / *no sync yet* /
  *reconnected* / *failed with reason* as four distinct states, per rule 5.

**What is missing is the checkpoint.** None of it has run unattended against real mail for
long enough to prove it works, because nothing has been running unattended at all. Phase 10
closes that rather than leaving a phase quietly half-open.

**And "What this phase is not", directly above, is now false.** It says the Gmail label writes
stay held until 8b has met a real corpus. They shipped in `f911f46`, and `labelling_enabled()`
(`inbox/sync.rs:354`) is **on by default** — `INBOX_APPLY_LABELS=false` opts out. The code's
reasoning is recorded and is not unreasonable: a wrong label is visible and removable in Gmail,
and the granted scope withholds delete and send, so the one irreversible-feeling thing is not
irreversible. But the plan said *held*, the code says *on*, and nobody reconciled the two —
which is worth noticing now rather than after the agent has been labelling a real mailbox
unattended for a week. **10k made it an explicit decision, and on 2026-09-03 the owner decided
ON.** The plan's "held" language above is superseded: label writing is deliberate, not
inherited, and `deploy/env.production.example` ships `INBOX_APPLY_LABELS=true`.

One defect found while reconciling: `InboxPanel.tsx:172` renders the email **subject** under
the label `from:`. Cosmetic in isolation — but the panel exists so you can check a proposal
against the mail that caused it, and a reviewer who believes they are reading the sender is
checking a field nobody showed them. Fixed in 10g.

---

## Phases 10–13 — The hunt pipeline: four weeks, two agents

**Planned 2026-09-02.** Phases 7–9 built a scraper, an alert channel, an autofill extension and
a Gmail agent. They are driven by a human who remembers to drive them, and nothing yet tells
you whether any of it *works* — which source converts, which resume gets replies, whether the
classifier is right. **The goal of this month is a hunt that runs itself, tells you something
you did not know, and can prove it.**

Worked by **two agents against two separate weekly credit budgets** (Claude Code and Codex).
The lane rules, the migration-number reservation, and the swappability requirement are binding
and live in the root `CLAUDE.md` → *Working with two coding agents*. This plan is where the
assignments live.

### Legend additions for these four phases

- **Lane** — A (backend), B (client), C (docs). Per the rules file, A and B run in parallel
  worktrees and never share a file; C is written by whoever finishes first.
- **Primary** — who does it by default. Not a reservation.
- **Swap** — ✅ means the other agent can pick it up cold from the named spec and files, so a
  spent weekly budget never stalls the week. ⛔ means it genuinely cannot move, and the reason
  is always given; almost every ⛔ is `[you]`.
- **Est.** — elapsed hours *including the user's review time*. Agent runtime is not the
  constraint here; reading the diff is.

### Two scheduling facts that determine the whole order

1. **8b's checkpoint is calendar-gated, not effort-gated.** It needs a hand-labelled fortnight
   of real burner mail; the current corpus is 14 messages over two days, contains no
   newsletters or digests at all, and is entirely spent. No amount of agent throughput
   produces that faster. **So the deploy is in week 1, not week 4** — the clock on the corpus
   starts the day the sync runs without a laptop being open, and week 4 exists to spend what
   weeks 1–3 accumulated.
2. **`application_events` has to come before anything that reads history.** Every feature this
   month — response rates, time-to-response, resume attribution, nudges — is a query over the
   transitions the app is currently throwing away. Retrofitting the log in week 3 means
   rewriting week 2 on top of it, which is the lesson `0006_add_review_ownership.sql` already
   taught this repo once.

---

## Phase 10 — Week 1 (2026-09-02 → 09-08): the spine and the clock

**Goal:** an append-only `application_events` log that every writer emits into, and the whole
tool running unattended on a host that stays up.

| # | Task | Tag | Lane | Primary | Swap | Est. |
|---|---|---|---|---|---|---|
| 10a ✅ | Reconcile `PLAN.md` + `HUNT.md` with what actually shipped in `c001445`, `f911f46`, `f17d983` | `[gen]` | C | Claude Code | ✅ | 1–2h |
| 10b | `AGENTS.md` unification — **done 2026-09-02** | `[gen]` | C | — | — | — |
| 10c ✅ | Write the `application_events` schema + emit contract into `docs/HUNT.md` **before any code** | `[gen]` | C | Claude Code | ✅ | 1–2h |
| 10d ✅ | Migration `0021_create_application_events.sql` + backfill from existing tables | `[gen]` | A | Codex | ✅ | 3–4h |
| 10e ✅ | Every writer emits: proposal accept/reject, auto-apply, extension "track this application", expiry sweep, manual edits | `[gen]` | A | Claude Code | ✅ | 3–4h |
| 10f ✅ | Invariant test: `status == fold(events)` for every application, over a copy of the live DB | `[gen]` | A | Codex | ✅ | 1–2h |
| 10g ✅ | `InboxPanel` shows the sender under `from:` and the subject under `subject:` | `[gen]` | B | Codex | ✅ | 30m |
| 10h ◐ | Deploy: host, HTTPS, `COOKIE_SECURE=1`, service unit, restart-on-boot, `.env` off the repo | `[gen]`+`[you]` | A | You + either | ⛔ secrets, DNS and the host account are yours; the unit files and scripts are not | 8–12h |
| 10i ✅ | `fridge.db` backup on a schedule, and a **restore drill** that actually restores | `[gen]` | A | Codex | ✅ | 2h |
| 10j ✅ | Rate limit `POST /auth/login` (and `/reviews`) — Argon2 with no throttle is a cheap DoS | `[gen]` | A | Codex | ✅ | 2h |
| 10k ✅ | **Decide `INBOX_APPLY_LABELS` before the host goes unattended** — decided ON, 2026-09-03 | `[you]` | C | You | ⛔ it is a write-access call on a real mailbox | 30m |

**Board corrected 2026-09-03.** 10d and 10f were still marked not-started and had both
shipped — `migrations/0021_create_application_events.sql` exists, and `verify_invariant` plus
its ignored over-a-copy test landed in `e3f6c28`. Two ticks nobody applied is how a board stops
being read, and this one is the only shared picture of what is left.

**Progress — 2026-09-02.** ✅ done · ◐ part done · ⬜ not started. **`phase-10-hardening` is
merged into `phase-10-spec`**, which was deploy gate 1. The combined suite is 758 passing —
exactly both branches' additions with nothing lost — clippy 36, `tsc` and `eslint` clean.

The merge is also what caught the two tasks having invented **different deployment contracts**:
10h wrote `/srv/hunt/app` + user `hunt` + `/var/lib/hunt/fridge.db`, while 10i's backup scripts
assumed `/opt/personal-website` + user `personal-website` + `/var/lib/personal-website`. An
operator following both would have installed a backup timer pointed at a database nobody writes
to, and found out at the first restore. Unified on 10i's naming.

| Task | Where it stands |
|---|---|
| 10a | `57137d8`. Six discrepancies beyond the four already recorded, incl. `HUNT.md`'s migration numbers contradicting its own prose twice |
| 10b | `7dcc0fd`. `AGENTS.md` is a symlink; two more added where Codex had no rules file at all |
| 10c | `87a6b08`. The spec is in `docs/HUNT.md`; 10d builds from it |
| 10e | **Done.** Part 1 (`d0caf73`, `8cb4c10`) made all three status writers transactional — fixing a live defect on the way, `decide` discarding the application UPDATE's `Result` — and added `routes::auth::Credential`. Part 2 (`5c161f9`) emits from all five call sites, including `create_application`, which had no transaction at all |
| 10g | `fed4813` (Codex). Sender and subject are now separate fields end to end |
| 10j | `227a5fd` (Codex). Two-bucket sliding window, IP and account, login and reviews. **See the proxy interaction in `docs/DEPLOY.md` before deploying** |
| 10h | **In progress.** Everything that is not a secret is in `deploy/` and `docs/DEPLOY.md`; the `[you]` half is listed at the end of that runbook |
| 10i | `790669c` (Codex). SQLite online-backup API, `integrity_check` and migration-ledger verification, atomic publish, a daily timer, and a drill performed against the dev database |
| 10d, 10f | `da0ed0a`, `e3f6c28` (Codex). The table, `record()`, the backfill and the fold verification |
| 10k | Outstanding with the user, and 10h names it as a gate |

**Load:** Claude Code ≈ 8h, Codex ≈ 10h, you ≈ 10h of deploy. Neither agent blocks the other:
10c is the only ordering constraint, and it is a doc.

### Checkpoint 10 — against the real thing, not fixtures

- The backend has run **48 unattended hours** on the deployed host with the laptop closed, and
  `inbox_runs` shows successful incremental passes with an advancing `historyId` watermark.
- `application_events` contains rows from `extension`, `manual` and `unknown`, and
  `status == fold(events)` holds for every application in a copy of the live DB.

  **This clause originally said "all four actors — `email`, `extension`, `manual`, `sweep`",
  and that was unmeetable by two decisions taken after it was written.** Corrected 2026-09-02
  rather than quietly failed:

  - **`sweep` has no producer at all.** 10c reserved the value and said so; Phase 11's
    dead-application detection is the first thing that will write one. Asking for it here was
    asking for a row nothing can create.
  - **`email` requires auto-apply, which is deliberately off.** `auto_apply_threshold()`
    returns `None` until `INBOX_AUTO_APPLY_CONFIDENCE` is set, and setting it is **13e**, after
    Phase 13 measures a real number. A classified email still writes a `status_proposals` row —
    rule 2 — but the status does not move, so there is no transition to record. An `email`
    actor row appearing during Phase 10 would mean the threshold had been guessed.

  `unknown` replaces them: it is what the backfill writes for history whose origin cannot be
  proved, and its presence is evidence the backfill ran. The full four-actor check belongs to
  Phase 13's checkpoint, once 13e sets the threshold.
- The restore drill produced a working database from a backup **before** it was needed.
- A cookie issued by the deployed backend carries `Secure`.
- The login limiter refuses the eleventh attempt and the log says so.

### Traps

1. **The backfill must not invent transitions it cannot prove.** Rows whose provenance is
   unknown get `actor = 'unknown'`, never `'manual'`. A backfilled row that claims to know who
   did it is worse than no row, and it will be believed by every chart in Phase 11.
2. **Keep `status` as a column and add the fold test.** A mismatch means a writer forgot to
   emit — that is exactly the failure you want loud rather than a reason to abandon the
   invariant.
3. **`COOKIE_SECURE=1` is not optional once you are on HTTPS**, and if the frontend and backend
   end up on genuinely different hosts you need `SameSite=None; Secure`. The extension is
   unaffected: it authenticates with a `hunt_tokens` bearer, which is 8e's cookie discovery
   still paying rent.
4. **The Gmail refresh token now lives on a host you do not sit in front of.** `.env` mode 600,
   no secrets in the service unit, and remember the 7-day expiry makes the extension's inbox
   line load-bearing rather than decorative.
5. **Still no fetching from a request handler.** The interval worker stays the only Gmail
   caller; the root cache rule applies to Gmail exactly as it does to a job board.
6. **Back up before Phase 12's uncapped run**, which is the first run that can expire anything.

---

## Phase 11 — Week 2 (2026-09-09 → 09-15): the feedback loop

**Goal:** the tool tells you something you did not know, and warns you before a deadline
lapses. Everything here reads `application_events`; nothing here adds a new writer.

| # | Task | Tag | Lane | Primary | Swap | Est. |
|---|---|---|---|---|---|---|
| 11a ✅ | Analytics contract into `docs/HUNT.md`: endpoint shapes, and the definitions of *response*, *dead*, *converted* | `[gen]` | C | Claude Code | ✅ | 1–2h |
| 11b ✅ | `GET /hunt/analytics?from=&to=` — funnel by source, by company tier, by month | `[gen]` | A | Codex | ✅ | 4–5h |
| 11c ✅ | Analytics panel on the internships tab — **no new npm dependency**, plain SVG/CSS bars | `[gen]` | B | Claude Code | ✅ | 3–4h |
| 11d ✅ | Time-to-first-response, per-source conversion, dead-application detection | `[gen]` | A | Codex | ✅ | 2–3h |
| 11e ✅ | Follow-up nudges: no response in N days → a `hunt_events` row through the channel 8e built | `[gen]` | A | Claude Code | ✅ after 11a | 4–5h |
| 11f ✅ | Deadline extraction from classified mail (OA due dates, interview times) → alert before it lapses | `[gen]` | A | Claude Code | ✅ after 11a | 4–6h |
| 11g ✅ | URL-sync the internship filters, so a filtered view is bookmarkable | `[gen]` | B | Codex | ✅ | 2h |

**Load:** Claude Code ≈ 15h, Codex ≈ 11h. This is the most lopsided week — if the Claude budget
is thin, **11e and 11f are the two to move**, and 11a is written precisely so they can be.

### Checkpoint 11

- The funnel numbers **reconcile with a hand count in SQL** over the same window. A dashboard
  that cannot be checked by hand is a dashboard nobody should believe.
- A backdated application with no response produces **exactly one nudge per threshold**, and
  the next sweep produces none. (Written as "exactly one" before 11e chose two thresholds,
  14 and 30 days; a 40-day silence therefore earns two, one from each. The property being
  checked — that silence is announced once and not repeated — is unchanged.)
- ~~A real OA email with a real due date raises an alert ahead of that date, and the alert body
  says which application it belongs to.~~ **Unmeetable as written, corrected 2026-09-02** — the
  second checkpoint clause in two phases to ask for something nothing can produce, and worth
  noticing as a pattern rather than a coincidence: both were written before the subsystem
  existed, and both were wrong about what it would be able to see.

  **No email in the corpus carries an extractable date, and none can.** `gmail.rs` fetches
  `format=metadata` on purpose — *"it is a burner account, but it is still someone's mail"* — so
  the body is never transferred, and extraction sees the subject plus a snippet averaging **199
  characters** (201 max). Measured over all **23** stored messages: **zero** extracted. The only
  date-shaped text in the whole corpus is an event range in a subject (`Roblox Week @ CMU -
  9/8-9/10`) and a forwarded `Date: Sun, Aug 30` header, both of which cue-anchoring refuses on
  purpose.

  The bar that replaces it, and it is a lower one — say so rather than letting it pass as the
  same test:

  - **The extraction → storage → alert path is verified end to end on a CONSTRUCTED message**
    carrying real deadline wording, against a copy of the real database. That proves the
    plumbing, not the recall.
  - **The real-corpus recall is reported as its own number** — 0 of 23 — rather than folded into
    a pass.

  The original clause becomes meetable when either dated mail arrives or bodies are fetched.
  **Fetching bodies reverses a recorded privacy decision and widens what untrusted text reaches
  the classifier; it is the user's call, not an agent's**, and until it is taken this feature is
  plumbing waiting for input.
- Filters survive a page reload and a copied URL.

### Checkpoint 11 — met 2026-09-02

Verified against a **copy of the live `fridge.db`**, driven over HTTP through the real server
and the real background workers rather than through fixtures. A session was minted directly in
the copy rather than registering an account.

- **The funnel reconciles with a hand count.** `GET /hunt/analytics` returned
  `applications 2, responded 1, reached_oa 1`; an independent SQL count over the same window
  returned 2, 1, 1. `by_tier` put Roblox in tier **2** and zip in **unknown** — not tier 3,
  which is the rule `None` has carried since 8e. ✅
- **One nudge per threshold, and no repeat.** A 40-day silent application produced exactly two
  nudges (`…:14`, `…:30`) on the first tick and **none** on the second, through the real
  worker at a 60-second cadence. ✅
- **The deadline path, on a constructed message.** Both leads fired (`dl-cp11:24`,
  `dl-cp11:72`) and the alert named the application: *"Roblox · due in 19h"*. **Real-corpus
  recall is 0 of 23**, reported as its own number per the corrected clause. ✅
- **Filters survive a copied URL and a reload.** `?remote=true&sort=posted&company=Roblox&pay_min=40`
  hydrated every control, and reloading kept all four. ✅

### What the real run caught that 818 tests could not

**1. The analytics endpoint reports confident zeroes on a database whose backfill has not run** —
and this is the one with a deploy consequence. Before `application-events backfill`:

```
applications: 2, responded: 0, reached_oa: 0
```

After it: `responded: 1, reached_oa: 1`. `applications` is counted from
`internship_applications` while every conversion number comes from `application_events`, so an
un-backfilled database renders a **populated-looking dashboard that is wrong** rather than an
obviously empty one, and nothing in the response says the log is empty. Recorded in
`docs/DEPLOY.md` as a required first-deploy step; the durable fix would be for the endpoint to
report how many applications have no events at all, which is a change in Codex's file.

**2. A nudge announced a guessed age as a measured one.** *"No reply from zip · 14 days"* for an
application silent 40 days: `applied_at` was parsed with a fallback to the threshold, so any row
not written in RFC3339 reported the threshold as its age. Every test wrote RFC3339 because the
app does; the row that broke it had been through SQLite's own `datetime()`. Fixed in `a20364a` —
skip and log, because a nudge whose age cannot be computed is a nudge that cannot be justified.

### Known gaps, recorded rather than fixed

- **An application created directly in a terminal status is counted as `no_response_dead`.**
  `metrics_for` derives everything from events *after* the creation event, so an application
  whose only event is a creation with `to_status = 'rejected'` has no response, is not counted
  in `rejected`, and becomes "dead" once older than the threshold. The contract says a closed
  application is never dead. Reachable through `POST /internships/applications` with
  `status: "rejected"`. The fix needs `a.status` in the analytics query plus two guards — it is
  in `src/routes/analytics.rs`, so it goes back to the agent that wrote it rather than being
  patched across a lane boundary.
- **The analytics window is filtered in Rust, not in SQL.** Every one of a user's applications
  is loaded and then filtered by `applied_at`. Correct, and fine at two applications; it is a
  full scan at two thousand.
- **`record_deadline` — the sync-time write — has no end-to-end coverage.** `extract` has 15
  tests and the sweep has 5, but the function joining them runs only inside a Gmail sync. It is
  the last unexercised seam in the deadline path.
- **A collection run started on its own** when the backend booted against the copy, because
  collection runs at startup when the data is stale. Harmless — the sources are polite by
  design — but worth knowing that booting a copy fetches from live job boards.

### Traps

1. **Nudge dedup fights `UNIQUE (kind, subject_id)`.** That constraint is what makes alert
   dedup structural rather than a caller remembering — but a second nudge for the same
   application at a *different* threshold is a legitimately different event. Decide the key in
   11a and write it down: `subject_id = "{application_id}:{threshold_days}"` is the cheap
   answer. Get it wrong in the obvious direction and you get one nudge ever; wrong in the other
   and you get one every sweep, which is how a channel gets muted and takes the OA alerts with it.
2. **"No response" is not "rejected."** A silent application is its own state; collapsing the
   two makes the funnel lie about your rejection rate in the flattering direction.
3. **Deadlines are parsed from untrusted mail — rule 1 still holds.** Extraction is a pure
   function over text, it fetches no URL, and it writes to no calendar. It raises a
   notification; that is the entire blast radius.
4. **Store UTC, render local.** An OA deadline off by a day is the single bug that costs the
   thing this whole tool exists to protect.
5. **A posting expiring is not an application ending.** Analytics must drop expired *postings*
   from supply metrics without dropping *applications* made to them.
6. **No new npm package for charts without asking** — root rule, and it applies to a chart
   library exactly as it applied to `@tailwindcss/typography`, which was declined for less.

---

## Phase 12 — Week 3 (2026-09-16 → 09-22): coverage and attribution

**Goal:** the two known measurement gaps in Phase 7 closed, and the first attribution signal
that is actually about *you* rather than about the market.

| # | Task | Tag | Lane | Primary | Swap | Est. |
|---|---|---|---|---|---|---|
| 12a ✅ | `dedup::ats_identity` for Workday, `apply.workable.com`, `ats.rippling.com`; record all three in `INTERNSHIP_SCRAPING.md` | `[gen]` | A | Codex | ✅ given the URL corpus + test list | 5–7h |
| 12b ✅ | Re-key safety measurement: does the new key **merge rows that were distinct**? Measured over a copy of the live DB | `[gen]` | A | Claude Code | ✅ | 2–3h |
| 12c ✅ | The first **uncapped** collection run; measure expiry and pay coverage honestly | `[gen]`+`[you]` | A | You start it, either agent reads it | ⛔ it is a long real run against live sources | 1h + the run |
| 12d ✅ | **Decision:** fuzzy company/title dedup — reuse `[learn]` `nlp.rs`, or write a second matcher | `[you]` | — | You | ⛔ it is a Learning Mode boundary call | 1h |
| 12e ✅ | Fuzzy dedup implementation — conditional on 12d | `[gen]` **or** `[learn]` | A | depends on 12d | ⛔ until 12d | 3–5h |
| 12f ✅ | Resume-variant attribution: variants table, the extension records which one was used at fill time, outcome by variant | `[gen]` | A+B | Claude Code | ✅ after the contract | 5–7h |
| 12g ⬜ | Verify 8g in Firefox on **two live ATS forms** — `questions()`, `describePage()`, Save and Suggest | `[gen]`+`[you]` | B | You + Claude Code | ⛔ a human loads the extension and opens the forms | 3–4h |
| 12h ✅ | `is_machine_sender` does not know `systemmessage@` | `[gen]` | A | Codex | ✅ | 1h |
| 12i ✅ | **Scoped expiry** — per-board verdicts, so one dead board out of 485 stops disqualifying Greenhouse. Found by 12c | `[gen]` | A (+ the panel half) | Claude Code | ✅ | 4–5h |
| 12j ✅ | 0026 watched during a real collection; migration `0027` deletes the immortal rows 12c found | `[gen]` | A | Claude Code | ✅ | 3–4h |
| 12k ✅ | Migration `0028` backfills the scope tags 0026 can only earn forward, generated from `dedup::ats_identity` | `[gen]` | A | Claude Code | ✅ | 3h |
| 12l ✅ | Collapse the five-branch stack onto one integration branch; review Codex's variants work | `[gen]` | C | Claude Code | ✅ | 2h |

**Load:** Claude Code ≈ 17h, Codex ≈ 7h, you ≈ 6h. **12d blocks 12e and nothing else** — make
the call at the start of the week so it never becomes the reason a week ended short.

### The decision in 12d, stated plainly so it can be made in one sitting

`dedup::FuzzyMatcher` is an unimplemented seam, and `KLA` / `KLA Corporation` are two rows
because of it. Fuzzy company matching is NLP-shaped, and the root rules say: prefer reusing the
existing `[learn]` `src/nlp.rs`, and **ask** before writing a second matcher — Phase 7's `[gen]`
exception was granted for the ranking, not for NLP. Three outcomes, all legitimate:

- **Reuse `nlp.rs` as a caller.** `[gen]`, either agent, and the file itself is never edited —
  including to fix a compile error it throws.
- **Write a new matcher yourself.** `[learn]`, yours, agents review and write tests only.
- **Leave it.** Under-merging is the safer failure, `INTERNSHIP_SCRAPING.md` § C says so, and
  12a may move coverage enough that this stops mattering this month.

### Traps

1. **Over-merging is the dangerous direction.** Both dedup bugs the Phase 7 real run caught were
   over-merges — two distinct jobs collapsing into one row. A new ATS key changes identities for
   rows that already exist, so **12b runs before 12a ships**, over a copy, and reports merges as
   well as splits.
2. **Workday is in § C's own table and was never implemented; Workable and Rippling are not in
   § C at all.** Whatever you learn about their URL shapes goes into `INTERNSHIP_SCRAPING.md` in
   the same commit — that file is the reason this is a 5-hour task and not a re-investigation.
3. **A capped run can never expire anything**, so 12c is the first run where the disappearance
   counters do real work. Back up first (10i), and expect the first uncapped run to be slow and
   to surface source failures the capped runs never reached.
4. **Resume variants cross the backend↔client seam**, so the contract goes into `docs/HUNT.md`
   before either half is written, and one lane writes both halves. This is rule 4 in the rules
   file, and `f17d983` is what ignoring it costs.
5. **12g is the only verification neither agent can do alone**, and it is the one that has been
   deferred twice. Two real forms, nothing submitted on either.

---

### 12b — the re-key measured, 2026-09-02, and it found something bigger than a merge risk

Measured over a **consistent snapshot of the live database** (`sqlite3 .backup`, not `cp` — a
collection run was writing at the time). 12a is purely additive — 351 lines added, none removed
— so a posting's key can only change where the new parser recognises a host the old one did not.

| | |
|---|---|
| Postings | **1,828** |
| Keys that change | **481** (26%) |
| ATS coverage | **707 → 1,188**, i.e. **38.7% → 65.0%** (§ C predicted 73%; previously measured 35%) |
| Groups that would newly merge | **60** |
| Groups that would split apart | **0** |
| **True over-merges — distinct jobs colliding** | **0** |

**The over-merge count is zero, and that is the headline.** All 60 merging groups are duplicates
the *old* key wrongly kept apart:

- **46** have byte-identical URLs in every row — the same posting stored twice, once before the
  `gh_jid` query-param handling existed and once after.
- **11** share one ATS job id under different titles (Epic Games' "Machine Learning Intern -
  Special Projects" and "Machine Learning Research Intern" are one Greenhouse job with two
  surface names).
- **5** look like different companies and are not: `Tower Research` / `Tower Research Capital`,
  `SpreeAI` / `SPREEAI`, `pony.ai` / `Pony.ai`, `Monolithic Power Systems` /
  `Monolithic Power Systems, Inc.`, `Nightwing` / `Nightwing Intelligence Solutions` — same ATS,
  same board, same job id. **These are exactly the cases the deferred fuzzy matcher (12d/12e)
  exists for, and the ATS key solves them for free.** Worth knowing before that decision is made.

### What the measurement actually surfaced

**The next collection run will duplicate 481 postings rather than update them.**
`collector::upsert_posting` computes `dedup_key(posting)` and relies on `ON CONFLICT` against
the unique index to keep the existing row's id. A posting whose key changed does not conflict
with its own row, so it is **inserted as a new row**; the old row then stops being seen, its
disappearance counters climb, and it expires.

Nobody planned this, and it is not a defect in 12a — it is what shipping a key change against a
populated table means. It matters most for the **one application that reached OA**: of the two
applications pointing at a posting, the Roblox one is inside a merging group and its key
changes, so after the next uncapped run it points at a row that is on its way to expiring.

### Recommendation — re-key, but only with repointing, and before the next uncapped run

1. **Keep 12a as shipped.** Coverage 38.7% → 65.0%, zero splits, zero true over-merges. It is a
   clear improvement and needs no revision.
2. ~~Add a migration that re-keys existing rows and merges the affected groups~~ — **done,
   `0025_rekey_ats_postings.sql`.** Applied to a copy: 1,828 → 1,764 postings, ATS-keyed
   707 → 1,120, zero orphaned applications or sightings, and the Roblox application repointed
   to its survivor. It is a generated list of literal ids rather than logic, because
   `ats_identity` is Rust and a boot-time routine would decide at runtime what to merge. Three
   defects surfaced by applying it to a copy rather than reading it: a UNIQUE collision from
   claim-before-release (fixed with a two-phase swap through `rekey-0025:<id>`), a second from
   rewriting both rows of a *refused* group to their shared key, and `sqlx::migrate!` embedding
   at compile time so a changed `.sql` needs `touch src/main.rs` before it is really being
   tested. The original recommendation to
   **repoints `internship_applications.posting_id` at the surviving row before deleting the
   loser**. Test it against the Roblox row specifically, by name: it is the one row in this
   database whose loss would be felt.
3. **Do not simply do nothing.** Doing nothing produces the same 60 merges anyway, via new rows
   and expiry, but *without* repointing anything — strictly worse than doing it deliberately.
   "Leave it and let expiry sort it out" is the option that looks cheapest and is not.

**What would change this recommendation:** a non-zero true over-merge count, or evidence that
any of the 5 same-company-different-name groups is genuinely two employers sharing an ATS job
id. Neither appeared in 1,828 rows.

### 12c — the first uncapped run: what it needs and what to watch

**Not run yet.** It is `[you]`: it fetches from every live source and takes roughly ten minutes
(sources are polled one host at a time to stay polite). Everything below is the preparation, so
the run itself is a decision rather than a project.

**Before it:** take a backup (`ops/backup-fridge-db.sh`). This is the first run permitted to
expire anything, and expiry is the one thing here that removes rows nobody asked it to.

**The environment.** Unset `INTERNSHIP_MAX_BOARDS_PER_RUN` entirely and leave
`INTERNSHIP_DISABLED_SOURCES` empty. **A cap is not a smaller version of this run — it is a
different run**: a capped source reports `partial` by construction, and `partial` is never
permitted to advance disappearance counters. Every measurement below is unavailable under a cap,
which is why the last pay-coverage number (2 of 808) means nothing.

**What "it worked" looks like:**

- Every source reaches `success`, `partial` or `failed` on its own merits, and **none is
  `skipped`** for reasons of the cap. `source_runs.counts_for_expiry = 1` for the successful
  ones — that flag flipping true for the first time is the actual event here.
- `consecutive_misses` advances for postings the successful sources did not return, and
  `swept_vanished` is non-zero once a source crosses `INTERNSHIP_MISS_THRESHOLD`. Expiry firing
  for the first time is the point of the run.
- **Pay coverage measured honestly.** § B expects "well under half"; the standing figure of 2 of
  808 is an artefact of Simplify (which carries no salary at all) having supplied almost
  everything under the cap. This run is the first that can answer the question.
- **A prediction worth checking, from the capped run:** Tower Research gains one row —
  `ats:greenhouse:gh_jid:8044334` — beside its two stale `co:` rows, exactly as Nightwing did.
  If it does not, something about that URL does not parse and § C wants to know.

**What makes it a failure rather than a slow success:**

- **A source failing is a normal run.** The scraping rules are explicit: LinkedIn, Indeed and
  Handshake are best-effort, and a run where all three fail is normal. Do not treat a failure
  count as the result.
- It is a failure if a **failed or partial** source's postings get expired — that is the named
  data-loss bug of Phase 7, and `a_failed_source_does_not_expire_the_postings_it_previously_supplied`
  is the test that says it must not happen.
- It is a failure if `postings_created` is large relative to `accepted` — that would mean keys
  are still diverging after 0025, and the corpus is re-accumulating duplicates.
- It is a failure if expiry removes a posting an application points at. There are two such
  applications; check them by id before and after.

### 12c — the first uncapped run, 2026-09-02

Backup taken first (`fridge-20260902T204138Z.db`). ~15 minutes, every source attempted.

```
fetched 49,913 · accepted 2,058 · filtered 47,854
postings_created 81 · postings_updated 1,977 · alerts_created 1
swept_deadline 0 · swept_vanished 0 · marked_closed 15
```

| Source | Outcome | Counts for expiry |
|---|---|---|
| vanshb03, simplify, lever, ashby | success | **yes** — the first time this flag has ever been true for four sources at once |
| greenhouse | partial (1 of 485 boards failed: a network error on `designmehair`) | no |
| weworkremotely | partial (the feed publishes only recent postings and is never a complete enumeration) | no |
| linkedin, indeed, handshake | skipped | no |

**The skips are the scraping rules working, not a cap.** LinkedIn's robots.txt disallows every
path and carries an explicit notice against automated access; Indeed permits `/jobs` in robots
but answers a polite, honestly-identified GET with a 403 behind a CAPTCHA. Both are recorded as
skipped with the reason, which is what "give up quickly when a source pushes back" looks like
when it is written down instead of retried.

**Pay coverage: 105 of 1,845 (5.7%).** § B expected "well under half" and the standing figure of
2 of 808 was an artefact of the cap. 5.7% is the honest number, and it is stable — the uncapped
run added no pay data, because the sources that carry salary are the ATS ones and they are a
small share of the corpus.

### Three findings, one of which corrects an earlier claim in this document

**1. Greenhouse can almost never expire anything.** One failed board out of 485 makes the whole
source `partial`, and `partial` may never advance disappearance counters. That is exactly the
rule Phase 7 wanted — a blocked fetch must not expire what it used to supply — but at 485 boards
the chance of a clean sweep is small, so in practice the largest ATS source is permanently
disqualified from expiry. Worth deciding deliberately rather than discovering later: per-board
outcomes, rather than one verdict for the source, would let the 484 that succeeded count.
**Decided and built — see 12i below.**

**2. Expiry still did not fire (`swept_vanished 0`), and that is correct.** The threshold is 3
consecutive expiry-eligible runs, and this was the first. Counters advanced — sightings with
misses went 275 → 280, and the run's own eligible sources will advance them again next time.
Two more uncapped runs are needed before anything vanishes.

**3. ~~The stale rows go quiet, and expiry removes them.~~ It does not, and they are immortal.**
The previous section claimed the refused groups self-heal. They do not.

When a re-key makes a sighting compute a different key, the sighting is **moved onto the row
that holds that key** — `UNIQUE (source, external_id)` means it is updated, not duplicated. The
row it left keeps no sightings at all. And `expiry.rs:260` will never sweep a posting with zero
sightings, deliberately and with the reasoning attached: `NOT EXISTS (a sighting below
threshold)` is vacuously true for a posting with none, so without that guard a brand-new posting
whose sightings failed to record would expire on its first sweep.

The consequence nobody had traced: **a posting whose sightings all migrate away can never
expire.** There are **4** such rows live today, two of them the stale Tower Research rows this
run vacated. It is a slow leak rather than a problem — 4 rows in 1,643 — but the "it self-heals"
argument for refusing to merge is void, and should not be reused.

**Recommended:** a follow-up migration deleting orphaned postings that have no sightings, no
applications, and were created before this run. The alternative — teaching the sweep to expire
sighting-less rows past an age — reopens exactly the hazard the guard was written for, and is a
change to `expiry.rs` that wants its own decision.

**Tower Research, the falsifiable prediction: wrong in its detail, right in its mechanism.** It
did not gain a fourth row, because the `ats:greenhouse:gh_jid:8044334` row already existed —
both sightings simply moved onto it, and the two `co:` rows were left with none. The prediction
of "one new row per refused group" holds only where no ATS-keyed row exists yet.

**The two applications are intact**, both still pointing at present postings.

### 12i — scoped expiry, 2026-09-02

12c's first finding, acted on. Migration `0026`, and the mechanism is source-agnostic.

A **scope** is a sub-unit of a source that can be enumerated completely on its own. Greenhouse's
scope is the board slug; every other source is one endpoint and reports none, taking a settle
path that is the pre-0026 code verbatim. The rule Phase 7 wrote is unchanged — absence is
evidence only from a complete enumeration — and only its grain moved.

**Confirmed against a copy of the live database.** 2,341 sightings, all `scope IS NULL`, which
is the correct reading of every row that predates 0026 and is why there is nothing to backfill.
Tagging 150 greenhouse sightings to a completed board, 50 to a failed one, and leaving 54
untagged, then issuing the exact increment the settle path issues for a `partial` run:

| | before | after |
|---|---|---|
| tagged to the completed board (150 rows) | 48 misses | **198** — every one advanced |
| tagged to the failed board (50 rows) | 1 | **1** — untouched |
| untagged, i.e. every pre-0026 row (54) | 12 | **12** — untouched on a partial run |
| every other source | 3,254 | **3,254** |

#### Is per-scope eligibility sound? Mostly it under-expires, and once it does not

Asked before the settle query was written, and the honest answer is not "yes".

**Under-expiry, which is the safe direction and most of the behaviour.** A sighting whose scope
failed is untouched. Sightings recorded before 0026 carry `scope IS NULL` and do not advance on
a scoped *partial* run — they are tagged the next time they are seen, so it self-clears in a run
or two. And a slug dropped from the board directory now produces no completed scope, so its
postings stop advancing entirely: a **strict improvement**, because `BoardDirectory`'s own doc
warns that pruning a slug used to make every posting on it expire at once, indistinguishable
from the board genuinely closing.

**The one over-expiry path, stated rather than explained away.** A sighting is tagged with the
board it was last seen in. If a job's only sighting is tagged board A, the job moves to board B,
and B then fails on three consecutive runs while A completes — A really is a complete
enumeration and really does not contain the job, so its counter climbs and the posting expires
while live on B. Today the source-level rule would have saved it, because B's failure would make
the whole source partial.

That is a real regression in one narrow case. It is bounded three ways: the sweep expires a
posting only when **every** sighting is at threshold, so a Simplify or Lever sighting protects it
outright; expiry is a soft delete, so an application referencing the posting still resolves; and
reappearance anywhere resets the counter to 0. It is also not a new *kind* of error — the
source-level rule already concludes "gone" from absence in an enumeration that may not cover
where the job now lives, since a board whose slug was never harvested has always been invisible
in exactly this way. Scoping makes that assumption reachable in one more situation rather than
introducing it.

The trade: 484 boards' worth of expiry that could not happen at all, against one narrow path to a
soft delete that reverses itself the moment the job is seen again. Taken knowingly, and written
into `expiry.rs`'s module doc so the next reader inherits the reasoning and not just the code.

#### Two things worth knowing

- **`counts_for_expiry` widened.** It meant "this source was fully enumerated"; it now means
  "this run was trusted for at least one scope". Identical for every unscoped source. The
  granularity that would otherwise be lost lives in the new `source_run_scopes` table, and the
  run-health panel reads it: a run that advanced 484 of 485 boards renders as
  `expired 484/485 boards`, which is neither "expired" nor the old `didn't expire` badge.
- **A test the code claimed to have did not exist.** `expiry.rs`'s comment on the sweep's
  `EXISTS` guard has cited `a_posting_with_no_sightings_is_never_swept` as pinning it since
  Phase 7. Nothing by that name was ever written — the guard was load-bearing, correct, and
  entirely unpinned. It is written now, with a control so it cannot pass by the sweep doing
  nothing. Worth a moment's suspicion about other "pinned by" comments in this codebase.

**Lever and Ashby are the obvious next scoped sources** and were deliberately left out of this
change: both are multi-board with the same problem, and the adapter half is all that is missing.

### 12j — 0026 watched during a real run, 2026-09-02, and it narrows a claim I made in 12i

0026 was in the position 0025 had been in: applied to the live database, unit tests green, and
never watched during an actual collection. Zero sightings carried a scope tag and
`source_run_scopes` had zero rows, so none of the scoped path had run against real data.

**Setup.** Greenhouse only, every other source disabled, `INTERNSHIP_MAX_BOARDS_PER_RUN=100`,
against a `sqlite3 .backup` copy. A capped run is exactly the case 12i was built for: the budget
truncates the board list so the run reports `partial`, and the boards it *did* read report
themselves as completed scopes. The 100 alphabetically-first slugs cover 17 of the 82 boards
that existing sightings live on, holding 42 of the 254 greenhouse sightings.

```
fetched 8,309 · accepted 36 · filtered 8,273 · created 0 · updated 36
greenhouse: partial — disappearance counters advanced for 100 of 100 scope(s)
```

**The predictions, written before the run.**

| | prediction | result |
|---|---|---|
| P1 | 100 scope rows, completed + failed = 100, and 0 failures | ✅ 100 completed, 0 failed. 6 boards 404'd and were recorded `completed` with no postings, which is the intended reading of "no such board" |
| P2 | `outcome = partial` **and** `counts_for_expiry = 1` — impossible before 0026 | ✅ |
| P3 | at most 42 sightings tagged, every tag one of the polled 100 | ✅ 37 tagged, 0 tags outside the polled set, 0 tags disagreeing with the slug in the sighting's own URL |
| P4 | **zero** miss counters move, greenhouse and everything else | ✅ 61 → 61 for greenhouse, 3,254 → 3,254 for the rest |
| P5 | a handful of postings created | ❌ **0 created.** Careless: 12c had polled all 485 boards seven hours earlier, so nothing on these boards was new. The prediction ignored a run I had written up myself |
| P6 | the panel reports 100/100 for greenhouse, 0/0 for the disabled sources | ✅ |

### Finding 1 — scoped expiry is forward-looking only, and 12i's doc overclaimed

P3's shortfall is the finding. 42 legacy sightings sat on boards that were **completely
enumerated**, and only 37 were tagged. The missing 5 — four on `astranis`, one on
`axontalentcommunity` — are sightings whose jobs are already gone from those boards.

A sighting is tagged only when it is **seen**. A sighting whose job is already dead can never be
seen, so it is never tagged; and an untagged sighting does not advance on a partial run, which
is nearly every Greenhouse run. Those 5 are therefore stuck, and `expiry.rs`'s claim that the
untagged population "self-clears over a run or two" — written in 12i, by me, without measuring —
is wrong. It self-clears for everything still listed, and does nothing for what had already
gone.

**This is not a regression.** Before 0026 a partial run advanced nothing at all, so those 5 rows
were equally immortal. But it bounds what 12i actually bought: scoped expiry expires what
disappears *after* it starts watching, and it does not reach backwards. The corrected claim is
now in the module doc with the measurement attached.

**Proposed, not done: migration `0028`, backfilling `posting_sightings.scope` from the URL.** A
greenhouse sighting's URL already records its board — `job-boards.greenhouse.io/{slug}/jobs/{id}`
— and parsing it is exactly as reliable as the tag written at fetch time, because it *is* the
slug the fetch used. Backfilling would give the 254 legacy sightings their scopes without
waiting to see each one, and the 5 dead ones would then advance the next time their board is
polled cleanly. It is a separate decision from 12j's two commits and wants its own measurement
of how many of the 254 URLs parse.

### Finding 2 — a capped run had stopped warning, and that was my regression

Before 0026 the run-health panel showed `didn't expire` for a capped Greenhouse run, which was
true and useful. After 0026 a capped run has `counts_for_expiry = 1` and `scopes 100/100` —
every board it *attempted* completed — so the badge went silent and the run read as clean, while
385 boards had not been looked at. The un-attempted boards deliberately produce no scope row
(no verdict is the honest record of not looking), which is right, and it makes
`scopes_attempted` a count of boards reached rather than boards that exist.

Fixed in the same commit: a `partial` run that advanced any scope now renders
`expired within N boards` rather than nothing. The 484-of-485 case still renders
`expired 484/485 boards`.

### Also worth recording

The run's own log printed *"capped sources report Partial and will never expire postings"*
immediately before *"disappearance counters advanced for 100 of 100 scope(s)"*. The binary
predated `2880651`, the commit that corrected exactly that sentence — so the stale build
demonstrated the defect that commit was written for, on real output.

### 12n — the first production expiry, reconstructed after the fact

A startup collection ran at `2026-09-03T04:32:33`. Nothing was watching it. It is where the
whole 12i → 12j → 12k arc finally acted on the real database, and the evidence for that is
weaker than for anything else in this phase — which is the first thing to say.

**This is forensic reconstruction from a backup, not an experiment.** 12j and 12k each wrote a
falsifiable prediction *before* running, and 12k's held in all six clauses. Nothing was predicted
here, because by the time anyone looked it had already happened. A prediction written now would
be a postdiction, and the write-ups in this file are worth something precisely because they were
not. What that costs: the run cannot be re-run, the "before" is whatever
`fridge-20260903T002827Z.db` happens to contain, and anything the backup does not capture is
simply unavailable.

**Greenhouse was `success` for the first time ever** — 485 of 485 scopes completed. On 12c one
board of 485 failed, the whole source was disqualified, and that is the finding this entire arc
started from. Five sources counted for expiry, the most in any run so far.

#### The arithmetic, verified against the backup rather than taken on trust

| | |
|---|---|
| postings | 1,841 → 1,868 |
| created | 28 |
| deleted | 1 (migration 0029's merge, not the run) |
| **newly expired** | **36** — 35 `source_marked_closed`, **1 `vanished_from_sources`** |
| un-expired | 5 — 4 `source_marked_closed`, 1 `vanished_from_sources` |

`202 + 36 − 5 − 0 = 233`, which is the expired count after. It closes exactly.

#### The one posting that vanished is the one 12k named

`e7346e4f` — Astranis, "Avionics Engineer Intern (Fall 2026)", expired `04:41:41`. Its only
sighting is `greenhouse / 4597413006 / scope=astranis / misses=3`, last seen 2026-09-01. The
external id is the same one written into 12k's prediction file two days ago.

The chain, end to end and every link checked:

1. That sighting **could never have been tagged by observation** — the job was already gone from
   the board, so no run could see it. Migration `0028` backfilled `scope = 'astranis'` from the
   sighting's own URL. This is the case 12j's Finding 1 said scoped expiry could not reach and
   0028 existed to reach.
2. The `astranis` board **completed** in this run, fetching 81 postings. `4597413006` was not
   among them.
3. Its counter went 2 → 3, the threshold.
4. Its posting had no other sighting, so the sweep took it.

#### What this run did *not* test, said plainly

**No board failed.** 12i's named over-expiry risk is a job migrating from a board that completes
to one that fails, and a run with 485 of 485 completing cannot exercise that path at all.
Reporting "no over-expiry observed" would imply the path was tested. It was not.

What the run *does* demonstrate is the sweep's all-sightings rule at scale, which had only ever
been seen in a single case: **267 sightings are at or past the threshold, and of the live
postings holding one, all 15 are protected by a second sighting below it.** Not one expired on
partial evidence.

#### `expired_at` is not a running total, and that is why the naive diff did not balance

`upsert_posting` clears `expired_at` and `expiry_reason` when a posting is seen again — 5 rows
in this run, one of which had previously `vanished_from_sources` and came back. That is correct:
a re-listed posting is live. But it means **`COUNT(expired_at IS NOT NULL)` is a snapshot of what
is closed now, never a count of what has ever closed**, and comparing two backups without
accounting for resurrection produces a number that is simply wrong. Recorded in
`docs/INTERNSHIPS.md` beside the column itself.

### 12m — the QC findings, fixed

A QC pass on 2026-09-03 turned up three things. All three are closed.

**A proposal whose evidence was missing vanished from the review queue.**
`fetch_proposals` inner-joined `status_proposals` → `email_verdicts` → `email_messages`, so a
proposal whose verdict no longer resolved was not listed at all — a status change waiting for
review, silently absent from the queue that exists to review it. The live database has exactly
one such row, and it drove the only status change in the database. Joins to the evidence are now
LEFT; the join to the application stays INNER, because `a.user_id` is what scopes the query to
the caller and widening it would leak other people's rows.

Widening the join alone would have been dishonest: all four evidence fields were *already*
nullable on both sides of the seam, so a degraded row would have rendered as three missing lines
and looked exactly like a terse email. The response carries `evidence_available` and the panel
says outright that there is nothing to check against. Both buttons stay enabled — refusing to
let such a proposal be accepted would decide for the reader that it is wrong, which the panel
does not know.

**Migration `0030` deletes the alert 0025 orphaned.** 0025 guarded its DELETE on
`internship_applications` only, and `hunt_events.subject_id` is a soft reference rather than a
declared foreign key, so neither it nor `PRAGMA foreign_keys` noticed. Deleted rather than
repointed: the merge survivor already carries its own alert and `UNIQUE (kind, subject_id)`
would refuse the collision. No date cutoff, unlike 0027 — `emit_posting_alert` runs after
`upsert_posting` returns a stored id, so "an alert whose posting does not exist" was never a
legitimate intermediate state.

**Migration numbers are now reserved per agent.** The old scheme gave a block to Lane A, which
stopped working the moment both agents did backend work — two agents inside one block collide
exactly as if there were no rule. Claude Code holds `0030–0059`, Codex `0060–0089`, and
exhaustion now has a protocol: at two numbers remaining, reserve the next block *before*
spending them. The first block ran out with no protocol at all, which is how it blocked a task
rather than the other way round.

### 12d + 12e — built, and the payoff is not the one the task was for

**The rule changed first.** On 2026-09-03 the owner lifted Learning Mode for this tab: NLP is a
learning area because of the *fridge app*, and here the goal is results. Four places said
otherwise and all four moved in the same commit — `CLAUDE.md`, `INTERNSHIP_SCRAPING.md` § C,
`dedup.rs`'s header, and the `FuzzyMatcher` doc. `src/nlp.rs` stays off-limits without asking,
now for blast radius rather than pedagogy: it is live fridge behaviour, and § C measured that
its bands break on job titles.

**The premise was already stale.** 12d was raised because `KLA` and `KLA Corporation` were two
rows. They are not, and have not been for some time: `normalize::company_key` strips legal
suffixes, so every example § C names — KLA, Moog, WhatNot — is one key today. The variants that
remain differ by *descriptive* tokens.

**No string rule can decide the remaining cases**, which is the finding that shaped the design.
The corpus holds `citadel` / `citadel securities` and `jump trading` / `jump trading group`. The
first is two employers, the second is one company, and both differ by one trailing descriptive
token. So the module splits the problem: a strict token-prefix **candidate generator** proposes,
and `data/internships/company-aliases.json` records what a human **decided** — 21 aliases and 3
refusals, each refusal carrying its reason, because the generator will propose them again.

#### The measurement, and the reframing it forced

| | |
|---|---|
| company keys in the corpus | 663 |
| candidates the generator proposed | 25 |
| merged after review | 22 |
| refused | 3 — Citadel Securities, the Rivian/VW joint venture, and `internship list` (neither is a company) |
| **duplicate postings this merges** | **1** |

One posting. That would not justify a re-key over live data, and the honest answer would have
been to stop. What justifies it is a second effect nobody was looking for:

**`company_signals` groups by `company_key`, and 19 companies had their signal split across two
or more keys — covering 130 postings.** Twelve of those fragments carry **no prestige at all**,
so `rank.rs` imputes them to the neutral midpoint while the sibling key scores real:
`jump trading group` beside `jump trading` at 1.0, `drw university jobs` beside `drw` at 0.88.
Those postings were being ranked as an average company when the company is top-tier.

So `0029` is a **ranking fix that merges a duplicate on the way past**, and 12d's original
framing — deduplication — was the least valuable thing about it.

#### Migration 0029, and the 0025 bug it does not repeat

47 postings get a new `company_key`; 8 of those are fallback-keyed so their `dedup_key` moves
too; 39 are ATS-keyed and cannot collide. One group merges, with no application and no alert on
either row. Generated from the same committed table the runtime reads, so the file and the
running code cannot disagree about what a company is called.

The DELETE guards **all three** references — sightings, applications, and
`hunt_events.subject_id`. That last one is the soft reference the 2026-09-03 QC pass caught 0025
missing, and a repoint that would violate `UNIQUE (kind, subject_id)` drops the duplicate alert
rather than failing the migration.

Verified against a copy: 1,841 → 1,840, no aliased key left, no orphaned sighting, idempotent on
a second application, and applying through `sqlx` on boot. `company_signals` converges on the
next collection run's recompute; until then a posting already matches its canonical signal row,
so the ranking fix takes effect immediately and the stale rows are inert.

**Lane A's migration range is exhausted** — `0029` was the last of `0021–0029`. `CLAUDE.md`
rule 3 now says so; reserve the next range before either agent writes another migration.

### 12l — the stack collapsed, and what is actually left

Five branches, none merged, `main` seventy commits behind. That is rule 8 going unpaid: two
agents producing at twice the rate the user can read, on lanes that had diverged. Everything is
now on **`phase-12-integration`**. Nothing is merged to `main` — that is a call for the user to
make deliberately, not a side effect of a cleanup.

**The merge was uneventful, which is the interesting part.** One file was touched by both lanes,
`frontend/src/lib/internshipsApi.ts`, and it auto-merged. No migration numbers collided, because
Lane A's 0021–0029 reservation meant Codex's 0024 and this lane's 0026–0028 were never competing
for the same next-free number — the failure rule 3 exists to prevent surfaces at
`sqlx migrate run`, not at merge, and it did not surface.

Verified on the combined tree rather than on either half: **868 tests, clippy 38, `tsc` clean**,
every migration through 0028 applying on a boot against a copy, and both panels rendering in one
build — `expired within 100 boards` on the run-health page against a real scoped partial run,
and the variants panel beside `By résumé variant` on the internships page.

### Reviewing Codex's three commits (rule 7), and it is clean

`8f7cc0d` analytics by variant, `578732d` the management panel, `a5813c8` the docs and loop.

- **The loop closes end to end.** A variant created through the panel comes back from
  `GET /hunt/resume-variants` with `archived_at: null` — which is exactly what the popup's
  picker reads and what `is_attachable` requires. The empty state even names the loop: *"Create
  one here and it will appear in the extension when you track an application."*
- **The three refusals are distinct, and better than specified.** 409 is ambiguous in the
  abstract but unambiguous per route — a 409 on create can only be a duplicate, a 409 on delete
  can only be in-use — and the API layer maps each one to its own sentence. The delete message
  goes further than asked and offers the alternative: retire it instead, retired variants stay
  available for comparison. Confirmed live: creating a duplicate label shows *"A résumé variant
  with that name already exists."*, not a generic failure.
- **No predicate drift.** `by_variant` uses `application_events::HAS_RESPONDED`, the same
  constant every other response question uses, rather than a second definition of "responded".
- **n = 1 is not misleading**, because the breakdowns render counts rather than rates. A single
  application shows as one, not as 100%.
- **`docs/HUNT.md` matches the code**, including the wire decision `8f7cc0d`'s comment had
  deferred: NULL variant serializes as the empty string, which label validation cannot store, so
  a real variant named `no variant` cannot collide with the bucket.

One thing worth naming rather than filing as a defect: `group_rows` calls `bail!` when an
application has a `resume_variant_id` whose join found no label, which would take the whole
analytics endpoint down. It is unreachable through the app — `is_attachable` rejects a variant
that is not the caller's own, `delete` refuses while `application_count > 0`, and there is a real
foreign key besides — so it is an invariant assertion rather than error handling, and a correct
one. Recorded because "unreachable" is a claim that ages.

### What is actually left, as of 2026-09-03

**Phase 12 is complete except for three items, and every one of them is yours.** 12d is the
fuzzy-dedup decision — a Learning Mode boundary call that no agent may make — and 12e is gated
entirely on it. 12g is the Firefox verification of 8g on two live ATS forms, which needs a human
loading the extension and opening the forms. 12f is now genuinely finished rather than
nominally: it was shipped API-only, and Codex's panel is what made it reachable.

**Phase 13 is blocked at its first two tasks, both `[you]`**: 13a exports the labelset, 13b
hand-labels it, and 13c through 13g all read those labels. Nothing an agent can start.

The other standing blockers are unchanged: deploy waits on the host, DNS, the Google OAuth
client and a restore drill. And expiry needs two more
uncapped runs before `swept_vanished` means anything at scale — 12c was the first of three, and
0026–0028 change what the second one can conclude.

### 12k — migration `0028`, and a prediction that held in every clause

12j found that scoped expiry is forward-looking: a sighting is tagged when a run *sees* it, so a
sighting whose job is already gone is never tagged, and an untagged sighting does not advance on
a partial run. `posting_sightings.url` already records the board, and `upsert_posting` rewrites
it every time the sighting is seen — so its slug is the same fact the tag carries, from the same
run. 0028 backfills from it.

**Generated, not hand-written, and not parsed in SQL.** `dedup::ats_identity` already knows
Greenhouse's three host forms and its one case-foldable path. A second parser in SQLite string
functions would agree with it until it didn't, and the failure mode of that divergence is a
sighting tagged to a board that does not exist. The generator is
`src/internships/scope_backfill.rs`, committed, with its invocation in
`INTERNSHIP_SCRAPING.md` § D.4.

**The measurement, which is what decided it.**

| | |
|---|---|
| greenhouse sightings | 254 |
| taggable | **253**, across 82 boards |
| skipped | 1 — `ats_identity` returns the pseudo-slug `gh_jid` for a job known only by a query-parameter id on a company's own careers page. Not a board, never polled |
| would be tagged with a slug the directory does not carry | **0** — so no row is made worse off than leaving it untagged |
| sightings on unscoped sources | 2,087 (1,310 ATS-shaped), none touched |

The one number that could have said "don't do this" was the fourth, and it came back zero.
Greenhouse's 485 directory slugs are, as it happens, all lowercase, so `ats_identity`'s
case-folding of that one host — which would have been a live hazard for SmartRecruiters, where
121 of 122 slugs are mixed-case — costs nothing here.

### The prediction, written before the migration was applied, and all six clauses held

| | prediction | result |
|---|---|---|
| 1 | the five known-dead sightings advance by exactly 1: 2→3, 3→4, 3→4, 7→8, 7→8 | ✅ exactly |
| 2 | the 37 live sightings on those boards reset to 0 | ✅ (42 rows on polled boards hold 27 misses, all of it the five) |
| 3 | boards 101–485 untouched: 39 misses before and after | ✅ |
| 4 | the `gh_jid` row stays NULL | ✅ 253 tagged, 1 untagged |
| 5 | `swept_vanished = 1`, and it is Astranis "Avionics Engineer Intern" — its only sighting, greenhouse at 2, reaching the threshold of 3 | ✅ expired `vanished_from_sources` |
| 6 | Astranis "Software Engineer Intern, Fall" does **not** expire, because vanshb03 still lists it at 0 misses | ✅ |

Clause 6 is the sweep's all-sightings rule working on live data: a posting gone from Greenhouse
and still carried by another source stays live, which is the behaviour `expiry.rs` claims and had
never been observed doing anything.

**The honest scale of what 12k buys.** Of the five sightings the backfill reaches on these
boards, three were already expired and one is protected by another source. Exactly one posting
expired. The backfill's value is not this run — it is that 253 sightings now advance whenever
their own board is read cleanly, instead of waiting for the fully-successful Greenhouse run that
12i established will essentially never happen.

**It narrows nothing.** On a fully successful run an untagged sighting advances because the
source was completely enumerated, and a tagged one advances because its board is in the completed
set — and on such a run every board is. `a_tagged_and_an_untagged_sighting_advance_together_on_a_full_run`
pins that rather than leaving it as an argument.

### 12j, commit 2 — migration `0027`, and every reference checked rather than assumed

The four immortal rows from 12c finding 3, deleted. All four are `co:` fallback-key postings
vacated by 0025, created 2026-08-21, duplicates of rows that are still live and still visible in
the ranked list:

```
Nightwing Intelligence Solutions — Software / Hardware Engineering Intern
Nightwing                        — Software / Hardware Engineering Intern
Tower Research Capital           — Quantitative Developer Intern
Tower Research                   — Quantitative Developer Intern
```

**What points at a posting, enumerated rather than recalled.** Two declared foreign keys —
`posting_sightings.posting_id` and `internship_applications.posting_id` — and **one soft
reference that is not a foreign key at all**: `hunt_events.subject_id`, for the 64 rows with
`kind = 'posting'`. `PRAGMA foreign_keys` would not have caught that one, and a posting deleted
out from under an alert leaves a notification whose link 404s. All three are excluded. A scan of
every column in the schema whose name mentions a posting, a subject or a job turned up nothing
else; `company_signals.live_postings` and `total_postings_seen` are aggregates recomputed by a
full SELECT at the end of every run, so they self-correct rather than drift.

**Why the date cutoff is in the predicate.** Without it this becomes a standing rule that
deletes any posting with no sightings — which is exactly the row `expiry::sweep`'s zero-sighting
guard exists to protect, a posting created moments ago whose sightings failed to write. The
cutoff is the start of the first uncapped run, and it makes this a cleanup of a known historical
mess rather than a policy. `a_newborn_posting_with_no_sightings_yet_is_never_touched` pins it.

**Verified**: 1,845 → 1,841 against a copy, both by `sqlite3` and end-to-end through `sqlx` on
boot; a second application changes nothing. What was explicitly **not** done is teaching the
sweep to expire sighting-less rows past some age — that reopens the hazard the guard was written
for and is a change to the one function that decides what "closed" means.

### 0025 verified against a real collection run, 2026-09-02

The migration was argued for on a claim about `upsert_posting`, verified against copies and
against the schema, and applied to the real database — but never watched during a collection,
which is the only thing that could show the claim was wrong. A capped run (Simplify only, board
cap 2) against a snapshot of the post-0025 database:

```
fetched 2,763 · accepted 1,328 · postings_created 79 · postings_updated 1,249
```

**335 of the 413 re-keyed postings were re-seen and updated rather than duplicated**, and all
413 are still present. That is the premise confirmed on live data: without 0025 those 335 would
have computed a key no row held and been inserted as new rows, with the originals left to
expire.

Postings sharing a canonical URL went **24 → 25**. Exactly one duplicate appeared, and it is the
interesting one.

### The refusal has a cost, and it is self-healing

The new duplicate is `ats:workday:nwis.wd12:JR101733` — the ATS-keyed row for a job the two
stale `co:`-keyed **Nightwing** rows already represent. That is one of the two groups 0025
deliberately refused to merge.

**Refusing to merge does not leave things as they were.** It leaves rows carrying keys that no
longer match what the collector computes, so the next run inserts the ATS-keyed row alongside
them. The choice was never "merge or leave alone"; it was "merge, or gain one row per refused
group at the next collection".

That is still the right call, but **the end state is not what this paragraph originally claimed
— see 12c below.** The new `ats:` row is the one future runs update and the `co:` rows do go
stale, but they are never expired: their sightings migrate away, and a posting with no sightings
is deliberately never swept. They are permanent.
The cost is a transient duplicate and the loss of the old rows' sighting history — acceptable
here because **no application points at either of them**. Had one done so, this would need the
repointing 0025 does for the merged groups, and the refusal would have been the wrong call.

Tower Research stayed at 3 rows this run: its posting was not re-fetched under the board cap.
Expect the same one-row addition there on a run that reaches it.

### The two counts, reconciled 2026-09-02

Exactly, not plausibly. Their tool filters to the three parsers 12a added —
`matches!(identity.ats.as_str(), "workday" | "workable" | "rippling")` — and treats every other
sighting as keeping its stored key. The independent probe recomputes every key from each
posting's canonical URL.

| | Groups |
|---|---|
| Probe (all postings, canonical URL, any ATS) | **60** — greenhouse 46, workday 11, rippling 2, workable 1 |
| Their tool (sightings, three new parsers only) | **18** |
| In the probe only | **46** — every one Greenhouse, which their filter excludes by design |
| In their tool only | **4** — Workday rows whose *sighting* URL yields an identity where the posting's canonical URL does not |

60 − 46 = 14 shared, + 4 = **18**. The arithmetic closes; both tools are correct about different
questions. Theirs measures *what 12a caused*. The probe measures *what the next collection run
will do*, which is the question a migration has to answer — and the 46 Greenhouse groups are
rows whose stored keys predate an earlier parser fix and were never back-applied.

The third hypothesis on record — the 53 postings with no sighting — is **killed**: every
probe-only group is explained by the ATS filter.

**The definitive set is 60 groups**, of which the conservative rule merges **58** and refuses 2.

### Two measurements, and they do not agree on the count

**Correction to an earlier claim in this section: the 12a handoff DID happen, and the first
version of this write-up said it had not.** `861dce0` ships
`dedup::tests::report_new_ats_key_merge_and_split_candidates` — an `#[ignore = "…"]` test —
and § C documents the exact `sqlite3 .backup` command and invocation. It was missed by checking
the commit's *file list* rather than reading § C, which is the file this task was told to read
first. The error is worth keeping visible: it produced a public claim that another agent had
skipped its work, from evidence that never supported it.

Running Codex's tool against the same snapshot reports **18 merge candidates**, where the
independent measurement above found **60 groups**. They measure different populations:

| | This section's probe | `report_new_ats_key_merge_and_split_candidates` |
|---|---|---|
| Source of the URL | `internship_postings.canonical_url`, one per posting | `posting_sightings.url`, one per source per posting |
| Population | all 1,828 postings | the 1,775 with at least one sighting |
| Counts a group where one row already carries the new key | yes | apparently not |

**The disagreement does not change the recommendation** — both report zero splits, every group
either measurement prints is a duplicate rather than two different jobs, and the
upsert-creates-a-new-row finding below is independent of either count. But **a migration would
need the exact set**, not the direction, so reconciling the two is a prerequisite for step 2
and not for the decision. The sighting-based population is the more faithful one to how
collection actually keys a posting, and is where a reconciliation should start.

---

## Phase 13 — Week 4 (2026-09-23 → 09-29): measurement week

**Goal:** 8b's checkpoint, met honestly — or reported honestly as still unmeetable, with the
reason. This week is the point of the other three.

| # | Task | Tag | Lane | Primary | Swap | Est. |
|---|---|---|---|---|---|---|
| 13a | `cargo run --release -- labelset export` over everything the month accumulated | `[you]` | A | You | ⛔ | 30m |
| 13b | Hand-label the sheet | `[you]` | — | You | ⛔ **by construction** — labels from the author of the rules measure the tuning, not the classifier | 3–4h |
| 13c | `labelset score`; report both failure modes against **separate denominators** | `[gen]` | A | Codex | ✅ | 1h |
| 13d | Diagnose and fix; pin every real string as a test | `[gen]` | A | Claude Code | ✅ | 4–6h |
| 13e | Set `INBOX_AUTO_APPLY_CONFIDENCE` from the measured numbers | `[you]` | — | You | ⛔ it is a risk threshold, not a parameter | 1h |
| 13f | Regression gate: `labelset score` in CI over the sealed sets, failing on a regression | `[gen]` | A | Codex | ✅ | 3–4h |
| 13g | Write the phase up: what the real corpus caught that the tests could not | `[gen]` | C | Claude Code | ✅ | 2h |

**Load:** Claude Code ≈ 8h, Codex ≈ 5h, you ≈ 5h. The lightest week for the agents and the
heaviest for you, which is correct — the scarce input this week is your judgment, not code.

### Checkpoint 13 — this is 8b's original checkpoint, finally reachable

Against a hand-labelled set of **real burner-inbox mail across a whole two-week window**, not
50 curated job emails:

- Junk leaking into `Hunt/Outreach` and **real mail getting disregarded**, measured separately.
- The second number is the one that costs an interview; it gets its own denominator and its own
  sentence.
- The relevance gate is only measured if digests, newsletters and staffing blasts actually
  turned up. **If they did not, say so and leave the gate unmeasured** — that is what happened
  on 2026-08-31 and recording it was worth more than a number.

### Traps

1. **A set can only be measured once.** Fixing against the graded set makes it in-sample; the
   ledger beside the labels file records the rules fingerprint each grading ran under, and it
   exists precisely so a spent set cannot quietly pass as fresh.
2. **No single accuracy figure.** The harness deliberately refuses to print one, because it
   would be quoted instead of the two numbers that matter.
3. **Auto-apply stays forward-only and never applies `offer` or `rejected`**, whatever the
   measured confidence turns out to be. 13e sets a threshold, not a policy.

---

## If the month slips — the cut list, decided in advance

Cut in this order, and cut whole tasks rather than the verification half of a task:

1. **12f** resume-variant attribution — the most self-contained, and it means more with a
   bigger corpus anyway.
2. **11f** deadline extraction — real value, but nudges (11e) already cover the "you forgot
   about this" failure mode.
3. **12e** fuzzy dedup — under-merging is the safe failure and `INTERNSHIP_SCRAPING.md` says so.
4. **11d** the second analytics slice.

**Never cut:** 10c–10f (the spine — everything later is a query over it), 10h (the deploy, which
starts the corpus clock and is therefore load-bearing for week 4), and 13a–13e (the measurement,
which is the entire reason this is a month and not a weekend).

## Picking up a task cold — the protocol that makes ✅ real

Swappability is a claim this plan makes, and it is only true if a task can be handed to the
other agent with three things and nothing else:

1. **The row above** — the task, its lane, its tag.
2. **The named spec section** in `docs/HUNT.md` or `docs/INTERNSHIPS.md`, written *before* the
   work started (10a, 10c, 11a and 12f's contract exist for exactly this reason).
3. **The rules file for that lane** — root `CLAUDE.md`, plus `apps/hunt-extension/CLAUDE.md` for
   anything touching the extension or the inbox agent.

If a task cannot be picked up from those three, the spec is incomplete and **that** is the bug —
not the agent. Fix the spec; the next handoff is free.

---

## After Phase 5

**Superseded in part by Phases 10–13 (planned 2026-09-02):** deployment is now 10h, and the
three hardening items below are 10h and 10j. What remains genuinely unplanned is additional
site tabs and the SQLite→Postgres question.

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
