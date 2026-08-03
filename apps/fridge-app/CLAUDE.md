# fridge-app — status & notes

Repo-wide rules (Learning Mode, phase discipline) live in the root `CLAUDE.md`. This file
tracks fridge-app-specific state so a new session doesn't have to rediscover it.

## Current status: Phase 1 in progress

Scaffolding is done and working end to end (add/list/remove items, UI, DB). Two pieces are
intentionally left as failing-test stubs for the user to implement — see below.

## Backend (`apps/fridge-app/backend/`)

- Rust, axum, sqlx (SQLite, file `fridge.db`, gitignored). Migrations in `migrations/`.
- `src/models.rs` — `FridgeItem` struct. `id` is a `String` (UUID text), not `uuid::Uuid`,
  to sidestep sqlx's BLOB-based Uuid encoding mismatch with our TEXT column.
- `src/routes/items.rs` — `GET /items`, `POST /items`, `DELETE /items/:id`. Wired to call
  `nlp::resolve_item_name` and `expiration::estimate_expiration` on add.
- `src/nlp.rs` — **[learn] not yet implemented.** `resolve_item_name` only does exact
  case-insensitive matching today. `cargo test` has 2 failing tests here
  (`plural_resolves_to_singular`, `common_typo_resolves`) marking the gap.
- `src/expiration.rs` — **[learn] not yet implemented.** `estimate_expiration` returns a
  flat 7 days regardless of item. 1 failing test (`pantry_items_get_a_long_shelf_life`)
  marks the gap; the other tests pass by coincidence (7 days happens to fall in their
  accepted ranges) — don't take those passes as "done."
- Run: `cargo run` (serves on `127.0.0.1:8080`). Test: `cargo test`.

## Frontend (`frontend/src/app/fridge/`)

- `page.tsx` — client component, fetches from the Rust API via `src/lib/fridgeApi.ts`
  (`NEXT_PUBLIC_FRIDGE_API_URL`, defaults to `http://127.0.0.1:8080`).
- `AddItemForm.tsx`, `ExpirationBadge.tsx` — presentational pieces.
- Verified working in-browser: add item, see it listed with expiration badge, remove it.

## Known environment quirks

- Turbopack tried to infer the workspace root as `~/Documents` (a stray `bun.lock` in the
  home directory confused it) and crashed on `next build` due to a sandboxed shell not
  being able to list that directory. Fixed by pinning `turbopack.root` in
  `frontend/next.config.ts`. If you see a similar `TurbopackInternalError: reading dir`
  again, that config is the first place to check.
- Local dev preview for this repo is configured at
  `/Users/jesseli/projects/meal/.claude/launch.json` (name `personal-website-frontend`,
  `autoPort: true` since port 3000 is often occupied by an unrelated project on this
  machine).

## Next up

- You implement `resolve_item_name` and `estimate_expiration` (see TODOs in those files).
- Once both are green under `cargo test`, Phase 1 is done — move to Phase 2 in
  `docs/PLAN.md`.
