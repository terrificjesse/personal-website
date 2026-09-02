# Personal Website — CLAUDE.md

This repo is a personal project website. It is a small multi-tab Next.js app; each tab is
a self-contained mini-project. The first tab being built is the **Fridge App**.

This file governs how Claude Code should work in this repo. Read it before making changes.
If a subproject has its own `CLAUDE.md` (e.g. `apps/fridge-app/CLAUDE.md`), that file's
rules take precedence for work inside that folder, but the "Learning Mode" rules below
always apply repo-wide.

## Repo layout

```
personal-website/
  frontend/                 # Next.js app (App Router, TypeScript). The site shell + all tabs.
    app/
      fridge/                # Fridge tab lives here (added in Phase 1)
      blog/                  # Blog tab (added Phase 6) — public reading + admin-only editor
      internships/           # Internship tab (Phase 7) — scraped SWE internship postings
      ...other tabs later, not in scope yet
  apps/
    fridge-app/
      backend/               # Rust API service. Serves the fridge app, the blog, internships,
                             #   *and* the hunt alert channel (src/hunt/, Phase 8).
      CLAUDE.md               # fridge-app-specific notes (data model, endpoints, phase status)
    hunt-extension/          # Firefox MV3 extension (Phase 8) — desktop alerts, later autofill
      CLAUDE.md               # governs Phase 8 wherever its code lives, backend half included
  content/
    blog/                    # Markdown posts, synced into the DB (Phase 6)
  docs/
    PLAN.md                  # Phased implementation plan — read this for current phase status
    BLOG.md                  # Blog + admin-permissions reference (every file and function)
    INTERNSHIPS.md            # Internship tab reference (sources, schema, ranking, expiry)
    INTERNSHIP_SCRAPING.md    # Per-source data-acquisition research — read before touching a scraper
    HUNT.md                  # Phase 8 reference (hunt_events, the alert path, the extension)
    TODO.md                  # Deferred ideas
```

The blog and internship tabs' backends live inside `apps/fridge-app/backend/` because that's
where auth and the users table already are. They are conceptually **separate tabs**, not fridge
features — don't let them bleed into each other, and don't move them without a reason.

**That directory is now three tabs deep and the name is a lie.** It is the site's backend, not
the fridge app's. Extracting it (to `apps/backend/`, or a shared crate) is a real option, but
it is a rename-and-reroute across every import and doc — do it as its own deliberate change,
never bundled into a feature phase.

The fridge app is **one tab among several**, not the centerpiece of the site. Don't let its
scope creep into styling or structuring the rest of the site — keep the site shell minimal
(nav bar + tab routing) and let each tab own its own complexity.

## Tech stack

- **Backend**: Rust, `axum` for HTTP, `sqlx` for Postgres (or `sqlite` via `sqlx` for local
  dev if that's simpler to start — confirm with the user before adding a real DB dependency).
- **Frontend**: Next.js (App Router), TypeScript, Tailwind for styling.
- **API contract**: REST + JSON to start. Don't introduce GraphQL/gRPC unless asked.

## Learning Mode — READ THIS FIRST

The user is building this project specifically to **learn**, not just to ship it. They've
called out three subsystems they want to implement themselves with minimal help:

1. **Authentication** (Phase 5 — password auth + Google OAuth)
2. **Natural language processing** — fuzzy/typo-tolerant item matching (Phase 1 — "tomato"
   vs "tomatoes" vs "tomatoe" resolving to the same fridge item)
3. **Recommendation algorithms** — shopping list suggestions (Phase 2), recipe matching /
   recommendation logic (Phase 3), and the like/dislike-weighted re-ranking system
   (Phase 4)
4. **Expiration date estimation** (Phase 1 — added by the user's own request, not part of
   the original three; same rules apply)

### Rules for these three subsystems

- **Do not write the core implementation** for these unless the user explicitly asks you to
  write the code (not just "help with X"). Default assumption: they want to write it.
- You CAN: explain concepts, discuss tradeoffs between approaches (e.g. Levenshtein distance
  vs. phonetic matching vs. stemming; content-based vs. collaborative filtering), point to
  papers/crates/libraries worth reading, review code they've written and give feedback,
  write *tests* that exercise their implementation (tests describe correctness, not the
  algorithm itself), and stub out the function signature / trait / interface so it plugs
  into the rest of the app.
- If asked to "just implement it" for these three areas, pause and confirm: ask whether they
  want a full implementation or want to keep driving with you as a reviewer/tutor. Don't
  assume speed is the goal here.
- Everything else in the project (CRUD scaffolding, Rust project structure, Next.js
  components, routing, database schema/migrations, API wiring, styling, tests for
  non-learning code) — generate freely and completely. Don't make the user hand-roll
  boilerplate that isn't part of what they're trying to learn.

### What is NOT a learning area

The four subsystems above are the whole list. In particular, **the blog tab (Phase 6) is not a
learning area** — content management, markdown rendering, search, sorting, and the
markdown-file ingestion pipeline are all `[gen]`. Implement them fully; do not stop at a
boundary and hand back a stub.

The one exception inside the blog's own feature set is `auth::require_admin`, which lives in
the `[learn]` file `src/auth.rs` and is already written by the user.

**The internship tab (Phase 7) is also not a learning area — including its ranking.** This is a
deliberate, explicit exception the user made on 2026-08-20, so do not "correct" it back:
`rank_postings` is a scoring-and-ordering algorithm and therefore looks exactly like the
`[learn]` subsystems from Phases 2–4, but the user chose `[gen]` for it. Scraping, storage,
normalization, dedup, filters, ranking, the applied tracker, and the expiry sweep are all
written fully.

The one thing still to confirm when you reach it: **dedup needs fuzzy company/title matching**,
which is the NLP learning area's shape. Prefer reusing the existing `[learn]` `src/nlp.rs`
rather than writing a second matcher. If it doesn't fit, **ask** before writing a new one —
that is a new NLP implementation, and Phase 7's `[gen]` exception was granted for the ranking,
not for NLP.

### Never edit a `[learn]` file

`src/auth.rs`, `src/nlp.rs`, `src/expiration.rs`, `src/recommend.rs`,
`src/recommend_recipes.rs`, `src/rerank.rs`. Diagnosis only — report what's wrong and where,
and let the user make the change. **Fixing a compile error still counts as editing.** If a
`[gen]` change requires one of these files to change, say so and stop rather than doing it.

### Phase discipline

Work phase-by-phase per `docs/PLAN.md`. Don't scaffold Phase 3 code while Phase 1 is still
in progress unless the user asks for it explicitly — this is a learning project and jumping
ahead undercuts the incremental structure that makes each phase a legible unit of learning.

## Scraping rules (Phase 7 — the internship tab)

These govern anything that fetches from a third party. They came from the user directly and
are not defaults to be re-weighed per source.

- **Per-source failure isolation is the architecture, not error handling.** One source being
  blocked, rate-limited, reshaped, or returning garbage must never fail a run or reduce what
  the other sources produced. A source returns its postings *or* it returns a recorded failure;
  the runner treats those as equally normal outcomes.
- **Fail fast, then move on.** No retry storms, no long backoff loops waiting for a source that
  has decided to block you. Give up on that source for that run.
- **Every failure lands somewhere a human will find it** — a `source_runs` row and a log line,
  with the source, the reason, and the count. A source silently returning zero postings must be
  distinguishable from a source that genuinely had zero.
- **No detection evasion.** Identify honestly in the user agent, respect `robots.txt`,
  rate-limit politely. **Do not** add proxy rotation, CAPTCHA solving, browser-fingerprint
  spoofing, headless-browser cloaking, or scraping from behind a login. The user's stated
  preference is to give up quickly when a source pushes back, which is also the only version of
  this that keeps working. If a source is only reachable by evading its controls, report that
  and leave it returning zero — don't build the workaround.
- **LinkedIn / Indeed / Handshake are best-effort and expected to yield little.** The user
  chose to include them knowing this. They must never be on the critical path for a run to be
  useful, and a run where all three fail is a normal run.
- **Cache aggressively and re-fetch rarely.** The vendored-snapshot precedent is
  `data/themealdb/` and `data/foodkeeper/`: fetch once, commit the snapshot, document when it
  was taken. Do not re-fetch on every request, and never from a request handler.
- Read `docs/INTERNSHIP_SCRAPING.md` before changing any source adapter — it records what each
  source actually provides and which fields are absent.

## Working conventions

- Prefer small, reviewable commits/diffs over large ones — the user is reading everything.
- When generating backend scaffolding, favor explicit, readable Rust over clever
  abstractions; this codebase is also a Rust-learning vehicle even outside the three
  flagged subsystems.
- Ask before adding new dependencies (crates or npm packages) beyond what's already
  agreed in `docs/PLAN.md`.
- When in doubt about whether something falls under "Learning Mode," ask rather than assume.
