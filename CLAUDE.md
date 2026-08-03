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
      ...other tabs later, not in scope yet
  apps/
    fridge-app/
      backend/               # Rust API service for the fridge app (axum)
      CLAUDE.md               # fridge-app-specific notes (data model, endpoints, phase status)
  docs/
    PLAN.md                  # Phased implementation plan — read this for current phase status
```

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
3. **Recommendation algorithms** — shopping list suggestions (Phase 2) and recipe
   recommendation + the like/dislike-weighted re-ranking system (Phase 4)
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

### Phase discipline

Work phase-by-phase per `docs/PLAN.md`. Don't scaffold Phase 3 code while Phase 1 is still
in progress unless the user asks for it explicitly — this is a learning project and jumping
ahead undercuts the incremental structure that makes each phase a legible unit of learning.

## Working conventions

- Prefer small, reviewable commits/diffs over large ones — the user is reading everything.
- When generating backend scaffolding, favor explicit, readable Rust over clever
  abstractions; this codebase is also a Rust-learning vehicle even outside the three
  flagged subsystems.
- Ask before adding new dependencies (crates or npm packages) beyond what's already
  agreed in `docs/PLAN.md`.
- When in doubt about whether something falls under "Learning Mode," ask rather than assume.
