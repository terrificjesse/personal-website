# fridge-app — status & notes

Repo-wide rules (Learning Mode, phase discipline) live in the root `CLAUDE.md`. This file
tracks fridge-app-specific state so a new session doesn't have to rediscover it.

## Current status: Phase 1 essentially complete

Working end to end (add/list/remove items, typeahead, expiration estimates, UI, DB), with
`cargo test` fully green — 30 passing, 0 failing. Both `[learn]` pieces (`nlp.rs`,
`expiration.rs`) are implemented.

The one Phase 1 item from `docs/PLAN.md` not built is **`PATCH /items/:id`** — the router
has GET/POST/DELETE plus `GET /items/suggest`, but no update endpoint. Nothing in the UI
needs it yet (quantity changes happen through the add-and-merge path), so it was never
wired up. Decide whether you want it before calling Phase 1 closed.

## Backend (`apps/fridge-app/backend/`)

- Rust, axum, sqlx (SQLite, file `fridge.db`, gitignored). Migrations in `migrations/`.
- `src/models.rs` — `FridgeItem` struct. `id` is a `String` (UUID text), not `uuid::Uuid`,
  to sidestep sqlx's BLOB-based Uuid encoding mismatch with our TEXT column.
- `src/routes/items.rs` — `GET /items`, `POST /items`, `DELETE /items/:id`. Calls
  `expiration::estimate_expiration` on add. It no longer resolves names: the user confirms
  the name in the dropdown before the request is sent, so the server takes it at face value
  and only normalizes whitespace/casing.
- `src/routes/suggest.rs` — `GET /items/suggest?q=&limit=`, backing the add-item typeahead.
  Empty `q` returns recent fridge items (doesn't touch the ranker); non-empty `q` builds a
  candidate list of fridge names + FoodKeeper catalog and calls `nlp::suggest_item_names`.
  Catalog entries whose name matches a fridge item are **merged into** that item rather than
  appended — otherwise anything in both lists (eggs, ham, milk) shows as a visible duplicate
  in the dropdown. The merged candidate keeps `SuggestionSource::Fridge` but inherits the
  catalog's aliases and product id, so a freehand-added item picks up its synonyms and its
  link to shelf-life data.
- `src/foodkeeper.rs` — parses the vendored CSV into a `Catalog` of 466 distinct names with
  their `Keywords` as aliases, loaded once at startup into `AppState`. Collapses duplicate
  names (README gotcha 6) and keeps every product id per name.
- `src/nlp.rs` — **[learn] implemented.** `suggest_item_names -> Vec<Suggestion>` (ranked,
  best-first), replacing the original auto-merging `resolve_item_name -> MatchResult` once
  the design changed so a human confirms every match. Scoring is a banded tier stack —
  exact / prefix / substring / fuzzy, each with a name variant and an alias variant one
  `ALIAS_CONST` lower. **The module doc is the reference**; it carries the band table and
  the invariants that make first-hit-return sound. Uses `strsim` for the fuzzy tier
  (similarity threshold 0.7, chosen from measured data: real typos land at 0.75–0.875,
  best unrelated noise at 0.571).
- `src/expiration.rs` — **[learn] implemented.** Parses the FoodKeeper CSV directly and
  walks a storage-preference chain (`DOP_Refrigerate` first, then opened/after-date/pantry/
  freezer) to pick a shelf life. Note: `produce_gets_a_short_shelf_life` was **deliberately
  removed** — it asserted lettuce expires in 3–10 days, but FoodKeeper has two `Lettuce`
  rows (iceberg/romaine 1–2 weeks, leaf/spinach 3–7 days) and the answer depends on which
  row wins. That's README gotcha 6; revisit if `Name_subtitle` disambiguation gets built.
- `data/foodkeeper/` — USDA FSIS FoodKeeper shelf-life reference data (661 products, 25
  categories), vendored as CSV. Read twice, independently: `foodkeeper.rs` for names and
  synonyms, `expiration.rs` for shelf lives. **Read
  `data/foodkeeper/README.md` before writing any parsing code** — it documents provenance
  (mirror verified against the official feed by hash), the `DOP_` = "date of purchase"
  column semantics that are easy to get backwards, and seven data-shape traps found by
  profiling (`_Metric` is a tagged union carrying `Not Recommended`/`Indefinitely`, prose
  in integer fields, 184 rows with no refrigerate data, `Name` is not unique).
- Run: `cargo run` (serves on `0.0.0.0:8080` — see LAN access note below). Test: `cargo test`.

## Frontend (`frontend/src/app/fridge/`)

- Next.js 16.2.12. `frontend/AGENTS.md` requires reading `node_modules/next/dist/docs/`
  before writing frontend code — this version has breaking changes vs. older Next.
- `page.tsx` — client component, fetches from the Rust API via `src/lib/fridgeApi.ts`
  (`NEXT_PUBLIC_FRIDGE_API_URL`, defaults to `http://127.0.0.1:8080`).
- `AddItemForm.tsx`, `ExpirationBadge.tsx` — presentational pieces.
- `ItemNameCombobox.tsx` — the add-item typeahead. **Deliberately never preselects a
  suggestion**: `activeIndex` starts at -1 and resets on every keystroke, so Enter commits
  the literal typed text unless the user arrows onto a suggestion first. This is the whole
  safety property of the design — don't "helpfully" auto-highlight the top result.
- Verified working in-browser: recent-items empty state, typed suggestions, arrow
  navigation with wrap back to raw text, Enter-selects vs. Enter-submits, add, remove.

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

## Open decisions (not yet made)

- **`foodkeeper_product_id` is a representative row for collapsed names.** `Ham` collapses
  20 CSV rows with different shelf lives; the stored id is just the first. `expiration.rs`
  will need `Name_subtitle` to disambiguate properly (README gotcha 6).
- **`expiration.rs` re-parses the whole CSV on every call** and has its own `PRODUCTS_CSV`
  `include_str!` + `FoodKeeperRow` separate from `foodkeeper.rs`. Worth reconciling once
  `estimate_expiration` settles — the catalog is already loaded once into `AppState`.

## Quantity merging on add

`POST /items` folds a new item into an existing row instead of creating a duplicate, but
only when **name + unit + expiration** all line up (`routes/items.rs`):

- Name and unit must match exactly (2 count + 1 litre isn't 3 of anything).
- Expirations must be within `MERGE_EXPIRATION_TOLERANCE_DAYS` (3) of each other, so
  two-week-old milk never absorbs today's. Rows with a NULL expiration never merge.
- On merge the **earlier** expiration wins — warning early about good food is cheaper than
  staying quiet about spoiled food.
- `foodkeeper_product_id` is backfilled via `COALESCE` if the existing row was freehand.
- Returns **200** on merge, **201** on insert.

The decision itself is `find_merge_target`, kept pure and separate from the SQL, with 7
unit tests. Tune the tolerance constant there if merging feels too eager or too shy.

## Next up

- Decide whether `PATCH /items/:id` is wanted before closing Phase 1 (see status above).
- Then Phase 2 in `docs/PLAN.md` — shopping list + purchase-based recommendations. The
  `[learn]` piece there is `suggest_shopping_items`.
- `docs/TODO.md` holds deferred ideas that came up during Phase 1 and were consciously
  postponed; out-of-order token matching for the NLP tier is the first entry.
