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
- `data/foodkeeper/` — USDA FSIS FoodKeeper shelf-life reference data (661 products, 25
  categories), vendored as CSV to back `expiration.rs`. Nothing reads it yet. **Read
  `data/foodkeeper/README.md` before writing any parsing code** — it documents provenance
  (mirror verified against the official feed by hash), the `DOP_` = "date of purchase"
  column semantics that are easy to get backwards, and seven data-shape traps found by
  profiling (`_Metric` is a tagged union carrying `Not Recommended`/`Indefinitely`, prose
  in integer fields, 184 rows with no refrigerate data, `Name` is not unique).
- Run: `cargo run` (serves on `0.0.0.0:8080` — see LAN access note below). Test: `cargo test`.

## Frontend (`frontend/src/app/fridge/`)

- `page.tsx` — client component, fetches from the Rust API via `src/lib/fridgeApi.ts`
  (`NEXT_PUBLIC_FRIDGE_API_URL`, defaults to `http://127.0.0.1:8080`).
- `AddItemForm.tsx`, `ExpirationBadge.tsx` — presentational pieces.
- Verified working in-browser: add item, see it listed with expiration badge, remove it.

## LAN access (testing from other devices)

- Backend binds `0.0.0.0:8080` (changed from `127.0.0.1` in `src/main.rs`) so other devices
  on the same Wi-Fi can reach it. First run after this change, macOS prompts to allow
  incoming connections for `fridge_backend` — must be approved manually, not scriptable.
- `frontend/.env.local` sets `NEXT_PUBLIC_FRIDGE_API_URL` to the host machine's LAN IP
  (was `192.168.12.146` as of this writing) instead of `127.0.0.1`, since a browser on
  another device resolves `127.0.0.1` to itself, not this machine.
- That IP is DHCP-assigned and can change (reboot, Wi-Fi reconnect, etc). If LAN access
  stops working, re-check it with `ipconfig getifaddr en0` and update `.env.local`.
- No auth exists yet (Phase 5), so anything on the LAN can hit the API while it's bound to
  `0.0.0.0` — fine on a trusted home network, not elsewhere.
- `.env.local` is gitignored, so this LAN IP is local-machine-only config, not committed.

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
- If the fridge tab seems to "hang" on loading forever (not error, just an endless
  spinner), first suspect is loading the page via the wrong URL — e.g. the Next.js dev
  server's printed "Network" URL from a browser on this same machine still works, but
  loading it as if it were a different machine's localhost won't. Confirm which URL is
  loaded before assuming a code bug.

## Git / GitHub

- Repo is initialized at the `personal-website` root, on branch `main`, with remote
  `origin` set to `git@github.com:terrificjesse/personal-website.git` over SSH (auth
  already configured — see `~/.ssh/id_ed25519`).

## Next up

- You implement `resolve_item_name` and `estimate_expiration` (see TODOs in those files).
- Once both are green under `cargo test`, Phase 1 is done — move to Phase 2 in
  `docs/PLAN.md`.
