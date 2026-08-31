# Blog tab & admin permissions — reference

Everything added on 2026-08-19: an `is_admin` flag on accounts, and a blog tab that admins
write to and everyone else reads. Phase 6 completed it the same day — markdown rendering,
sort, search, and posts sourced from `.md` files in the repo. This is the **what and where**;
design *rules* live in `apps/fridge-app/CLAUDE.md`, phase status in `docs/PLAN.md`.

**None of this is Learning Mode.** Unlike the fridge app's six `[learn]` modules, every file
here is `[gen]` — Claude writes it fully. The one exception is `auth::require_admin`, which
lives in the `[learn]` file `src/auth.rs` and was written by the user.

## Feature overview

| Feature | State |
|---|---|
| Accounts can be marked admin | ✅ `users.is_admin`, granted by SQL only |
| Admin-only write routes | ✅ enforced by the `RequireAdmin` extractor |
| Write/edit/delete posts from the website | ✅ `/blog/admin` |
| Public reading of published posts | ✅ `/blog`, `/blog/[slug]` — works signed out |
| Drafts hidden from non-admins | ✅ absent from lists, 404 by slug |
| Markdown rendering | ✅ `react-markdown` + `remark-gfm`, client-side |
| Sort by date | ✅ `GET /blog/posts?sort=newest\|oldest` |
| Keyword search | ✅ `GET /blog/posts?q=…` over title + body, SQLite `LIKE` |
| Posts sourced from `.md` files in git | ✅ `content/blog/*.md`, synced into `blog_posts` |

Phase 6 is complete. The constraint that shaped it: **search and sort must behave identically
for file-sourced and browser-authored posts.** That is why files are synced into the same
table rather than read at request time — there is one SQL query, not a merge of two stores.

## Data model

**`users.is_admin`** (migration `0009`) — `INTEGER NOT NULL DEFAULT 0`. A plain boolean rather
than a roles table, because the requirement is "is this me." No API writes it; the default
means every new account is a non-admin without anyone remembering to set it.

**`blog_posts`** (migration `0010`):

| Column | Notes |
|---|---|
| `id` | UUID text, like every other table here |
| `author_id` | `NOT NULL` — no pre-auth blog data existed, so there is no "unclaimed" state |
| `title`, `body` | plain text; body is **markdown source everywhere** — database, API, editor, and what search matches. Only the browser renders it |
| `slug` | `UNIQUE`. Derived from the title at creation and **never rewritten**, so a published URL survives a title edit |
| `published` | `0` = draft (admin-only), `1` = public |
| `created_at`, `updated_at` | `updated_at` moves on every edit; `created_at` never does |
| `source` | migration `0011`. `'db'` = written in the browser, `'file'` = synced from `content/blog/*.md`. `DEFAULT 'db'` backfilled every pre-existing row correctly |
| `source_path` | migration `0016`. The filename a file post came from. NULL for db posts and for file posts predating the migration. **This is what the mirror sweep keys on** — see below |

## Backend

### `src/routes/blog.rs` *(new — all `[gen]`)*

| Function | What it does |
|---|---|
| `list_posts` | `GET /blog/posts?sort=&q=&limit=&offset=`. Takes `MaybeUser`, so it works signed out. Admins get drafts too; everyone else gets `published = 1` only. Builds one statement from a list of conditions — draft filter, optional search, `ORDER BY` — so sort and search compose and cover both post kinds. |
| `like_pattern` | Escapes `\`, `%`, `_` and wraps in `%…%`, paired with `ESCAPE '\'`. Without it, searching `100%` returns every post. Unit-tested. |
| `reject_if_file_sourced` | The **409** on a file-sourced post. Called by `update_post` and `delete_post`. |
| `sync_posts` | `POST /blog/sync`. Admin-only. Re-runs the file sync and returns `{created, updated, deleted, skipped}` so a push can publish without a backend restart. |
| `get_post` | `GET /blog/posts/by-slug/{slug}`. A draft returns **404** to a non-admin, not 403 — a 403 would confirm the post exists. |
| `unique_slug` | Appends `-2`, `-3`, … until the slug is free. Stops two same-titled posts from 500ing on the `UNIQUE` constraint. |
| `create_post` | `POST /blog/posts`. Admin-only. Validates title/body length, slugifies, inserts. |
| `update_post` | `PATCH /blog/posts/{id}`. Admin-only, partial — only fields present in the body change. Deliberately never touches `slug`. **409 on a file-sourced post.** |
| `delete_post` | `DELETE /blog/posts/{id}`. Admin-only. Reads the row first, so a file-sourced post is 409 and a missing one is still 404. |

### `src/blog_files.rs` *(new — all `[gen]`)*

Ingests `content/blog/*.md`. Runs at startup from `main.rs` and on demand from `POST /blog/sync`.

| Function | What it does |
|---|---|
| `content_dir` | `BLOG_CONTENT_DIR`, else `content/blog` resolved off `CARGO_MANIFEST_DIR`. **`include_str!` can't be used here** — the pattern `foodkeeper.rs`/`themealdb.rs` use needs a fixed file list at compile time, and the whole point is adding a file without touching code. A runtime read means the working directory matters, hence the manifest-relative default. |
| `parse_front_matter` | Hand-rolled, no crate: the schema is four flat scalars. Returns `(FrontMatter, body)`. **An unknown key is an error** — a misspelled `pubished: true` would otherwise leave a post a draft forever with no symptom. |
| `read_dir_posts` | Reads and validates every `.md`; skips `README.md`; enforces the same title/body length limits `create_post` does, since a file bypasses that handler. A bad file is skipped and logged, never fatal. |
| `sync` / `sync_in` | Upserts by slug among `source = 'file'` rows, then **deletes rows whose file is absent from disk**. Mirrors the directory rather than importing from it. Returns a `SyncOutcome`, not a bare report. `sync_in` takes an explicit directory — the seam tests use, since `content_dir()` reads an env var and mutating process env races every other test. |
| `SyncOutcome` | `Completed(report)` = the directory was reconciled. `Deferred(reason)` = nothing was attempted (no admin yet, or the directory could not be read). **The watcher advances its fingerprint only on `Completed`**; see below. |
| `watch_tick` / `WatchState` | One watcher iteration, extracted so tests can drive it. `WatchState` holds the last *successfully reconciled* fingerprint plus the current deferral reason, so a retry loop logs its reason once rather than every tick. |
| `is_post_file` | The one predicate for "is this a post" (`.md`, not `README`). **Shared by the reader and the watcher on purpose** — if their idea of which files matter drifted, you'd get changes that never trigger a sync, or a sync that loops on a file it then ignores. |
| `fingerprint` | `(filename, mtime, size)` per post file, sorted. `None` for a missing directory, distinct from `Some(vec![])` for an empty one — creating the directory is itself a change. |
| `parse_interval` / `sync_interval` | `BLOG_SYNC_INTERVAL_SECS`, default 5, `0` disables. Split so the parsing is testable — mutating process env from a test races every other test in the binary. |
| `spawn_watcher` | The background task. Takes its first fingerprint **after** the startup sync, so startup's work doesn't immediately re-trigger. |

Rules that are easy to break by "simplifying":

- **The sweep tests file presence, not parse success.** It originally matched rows against the
  slugs of *successfully parsed* files, so a frontmatter typo deleted the live post — a failure
  to *read* was being treated as evidence of *absence*, and repairing the typo reinserted the
  post under a fresh UUID. Rows now carry `source_path` and are deleted only when that file is
  genuinely gone. Slug alone could not carry this: a file with an explicit frontmatter `slug:`
  cannot be matched back to its row once it stops parsing. Pre-`0016` rows (NULL `source_path`)
  fall back to the old slug test so upgrading deletes nothing, and backfill on the next sync.
- **A file's slug comes from its filename, not its title** (frontmatter `slug` overrides).
  The filename is the only identity a file has that editing its contents doesn't change —
  which is what makes the never-rewrite-a-slug rule hold for files too.
- **`created_at` comes from frontmatter `date`, never file mtime.** mtime is reset by
  `git clone`/`git checkout`, so sorting by it would reshuffle the blog on a fresh checkout.
- **`author_id` is the first-registered admin** — `ORDER BY created_at ASC, id ASC`, with `id`
  only as a tiebreaker. It once ordered by `id` alone, which sorts random UUIDs, so the owning
  admin was arbitrary and contradicted this very sentence. The column is `NOT NULL REFERENCES users(id)`
  and a file carries no author. With no admin yet, the sync logs and skips rather than
  panicking during boot.
- **On a slug collision with a `source = 'db'` post, the file loses** and is logged. Taking the
  slug would repoint an already-published URL at content nobody linked to.
- **Every write is scoped to `source = 'file'`.** A browser-authored post is never created,
  updated, or deleted by a sync.

### `src/models.rs` *(changed)*

| Item | What it is |
|---|---|
| `BlogPost.source` | `"db"` or `"file"`; serialized to JSON, which is how the admin UI knows to hide Edit/Delete |
| `BLOG_SOURCE_DB` / `BLOG_SOURCE_FILE` | The two values, named once |
| `SortOrder` | `newest` / `oldest` enum. Being an enum is the point: `Query` rejects `?sort=oldset` as **400** rather than silently defaulting to newest |
| `ListPostsQuery` | `{ sort, q, limit, offset }`, all optional. `limit`/`offset` are `u32`, so a negative is a 400 before it reaches SQL — where `LIMIT -1` means *no limit*, turning a typo into "return everything" |
| `DEFAULT_BLOG_PAGE_SIZE` / `MAX_BLOG_PAGE_SIZE` | 20 / 100. A `const _: () = assert!(default <= max)` fails the *build*, not a test |
| `BlogPostPage` | `{ posts, total, limit, offset }` — the response envelope |
| `User.is_admin` | The flag, read fresh from the DB on every request |
| `BlogPost` | The row struct — serialized straight to JSON as the API response |
| `CreateBlogPostRequest` | `{ title, body, published }`; `published` defaults false |
| `UpdateBlogPostRequest` | Same fields, all `Option` — absent means "leave alone" |
| `slugify(title)` | Lowercases, collapses non-alphanumeric runs to one hyphen, trims hyphens. Pure and unit-tested (4 tests). |
| `MAX_BLOG_TITLE_LENGTH` / `MAX_BLOG_BODY_LENGTH` | 200 / 100,000 **characters** |
| `exceeds_char_limit` | The limit check. `str::len()` is bytes, and using it made the limits silently stricter for non-ASCII — a 200-character CJK title is 600 bytes and was rejected. Counts scalar values, not grapheme clusters. |
| `is_blank` | Empty-or-whitespace. Used to *validate* only; bodies are stored verbatim, since trimming would rewrite an author's markdown and leading whitespace is significant to an indented code block. |

### `src/routes/auth.rs` *(changed)*

| Item | What it is |
|---|---|
| `RequireAdmin` | The extractor. Runs `CurrentUser` (→ 401 if no session), then `auth::require_admin` (→ 403 if not admin). A route taking it is admin-only **by its signature** — no middleware list to drift. |
| `AuthenticatedUser.is_admin` | Added so `/auth/me` tells the frontend whether to show the editor |
| `AuthError::Forbidden` arm | Maps to **403**. Distinct from 401 on purpose — see below. |

### `src/auth.rs` *(changed — `[learn]` file, user-owned)*

| Item | What it is |
|---|---|
| `require_admin(&User)` | `Ok(())` if `is_admin`, else `Err(Forbidden)`. The single place the question is answered. **Written by the user.** |
| `AuthError::Forbidden` | New variant |
| `validate_session` | SELECT now includes `u.is_admin` — this is the per-request path that actually feeds authorization |

> ⚠️ **Known stale**: the doc comment above `require_admin` still describes it as an
> unimplemented placeholder that "denies everyone." It's implemented. Claude won't edit
> `[learn]` files, so this one is yours to fix.

### Why 401 and 403 are different errors

`apiFetch` throws `UnauthorizedError` on **401 only**, and `useApiError` turns that into a
redirect to `/login`. If a non-admin's rejection came back as 401, a signed-in user would be
bounced to a login page they're already past — a loop with no exit. As a **403** it renders as
an ordinary error message in place.

**401 = "authenticate." 403 = "you did, and it's still no."**

## Frontend

| File | What it is |
|---|---|
| `src/lib/blogApi.ts` | All blog HTTP calls. `fetchPosts` / `fetchPostBySlug` use plain `fetch` (they work signed out, so a missing cookie isn't a 401 to raise); `createPost` / `updatePost` / `deletePost` / `syncBlogFiles` use `apiFetch`. `fetchPosts` takes `{sort, q}` and omits empty values from the query string. |
| `src/app/blog/MarkdownBody.tsx` *(new)* | The **only** place a post body is rendered, shared by the post page and the editor preview so they can't drift. |
| `src/app/blog/page.tsx` | Public post list, with a debounced search box and a newest/oldest toggle. Both feed one `fetchPosts` call. Drafts labelled when an admin is looking. |
| `src/app/blog/[slug]/page.tsx` | Single post. Reads `params` via React's `use()` — `params` is a Promise in this Next version. |
| `src/app/blog/admin/page.tsx` | The editor. Gates on `is_admin` from `/auth/me`, lists all posts with Edit/Delete, and has a **Re-sync files** button. File-sourced posts show "From file" and have no Edit/Delete. |
| `src/app/blog/admin/PostForm.tsx` | Shared create/edit form, with a Write/Preview toggle. "Published" defaults **unchecked**, matching `ReviewForm`'s opt-in precedent. |
| `src/app/globals.css` *(changed)* | The `.markdown-body` block. **Load-bearing, not polish** — Tailwind's preflight resets headings and list markers, so without it a rendered post looks almost exactly like the plain text it replaced. |
| `src/proxy.ts` *(changed)* | Matcher now covers `/blog/admin/:path*`. `/blog` and `/blog/[slug]` stay public. |
| `src/lib/authApi.ts` *(changed)* | `AuthenticatedUser` gained `is_admin` |
| `src/app/layout.tsx`, `src/app/page.tsx` *(changed)* | "Blog" nav link and landing-page link |

## Markdown rendering — why the frontend, and why no sanitizer

`react-markdown` builds a React element tree instead of setting `innerHTML`, so raw HTML in a
post is escaped and displayed as text rather than executed. That is the whole reason there's no
sanitizer in this stack — and why **`rehype-raw` must not be added** without deciding to accept
HTML injection, since switching that protection off is precisely what it's for.

Rendering on the frontend also keeps `body` as markdown source in exactly one representation.
The backend alternative (`pulldown-cmark`) would have meant either storing rendered HTML —
which makes search match `<strong>` — or carrying a second `body_html` field alongside the
source. One representation is what keeps search identical across both post kinds.

## Posts from markdown files

`content/blog/*.md`, documented for authors in `content/blog/README.md`. Frontmatter is
`title` (required), `date` (required), `published` (defaults false), `slug` (optional).

Publishing is **automatic**: a background task re-checks the directory every
`BLOG_SYNC_INTERVAL_SECS` (default 5) and syncs when it changed. Startup sync and admin-only
`POST /blog/sync` both remain — the latter forces a check immediately.

**A tick advances the fingerprint only when the sync actually reconciled.** It used to advance
before calling sync at all, so a tick whose sync did nothing still consumed the change. The
realistic trigger was a fresh database: with no admin, `sync` correctly does nothing by design,
the watcher recorded the change as handled, and files dropped in beforehand never appeared no
matter how long it ran. `Deferred` now leaves the fingerprint untouched so the next tick
retries. A missing directory is `Completed`, not `Deferred` — it is a stable state with nothing
to reconcile, and deferring would spin forever; a directory that exists but cannot be *read* is
`Deferred`, because an I/O error is not evidence its posts are gone.

**Why polling rather than `notify`.** Two reasons. First, cost: a tick is one `read_dir` plus a
`stat` per file, with no file reads, no database round-trip, and nothing logged unless the
fingerprint actually changed — so the idle cost is real but negligible, and `notify` plus a
debouncer would have been two crates. Second, and more decisive: `sync` only counts a post as
`updated` when its content genuinely differs, **so a spurious trigger costs a no-op**. That
asymmetry — false positives are free, false negatives are a missed post — is what makes an
imprecise detector the right tool. A `git checkout` that resets mtimes causes one wasted sync
rather than a wrong answer.

Note this is the *opposite* call from `created_at`, which deliberately refuses to use mtime.
Same fact about mtime, different consequence: for ordering it would produce a wrong result, for
change detection it produces a redundant one.

Three designs were considered (`docs/PLAN.md` lists all three). Sync-into-one-table won because
it is the only one where sort and search are written *once*: read-through and the GitHub API
both merge two stores in the query layer, so every future query feature gets implemented twice.
The cost is that changes need a sync rather than being live.

## Pagination

`GET /blog/posts?limit=&offset=` returning `{ posts, total, limit, offset }`. Default 20, max
100. Four decisions, each with a plausible-looking wrong answer:

- **An envelope, not a bare array plus `X-Total-Count`.** A custom header needs
  `Access-Control-Expose-Headers` to be readable cross-origin, and a header the browser
  silently declines to expose is a worse failure mode than a slightly larger body. The count
  is also not derivable from the page — a full page says nothing about whether more exist.
- **Over-limit is a 400, not a clamp.** `?limit=1000` quietly returning 100 hands the caller a
  partial answer it believes is complete. Same reasoning as `?sort=oldset` being a 400 rather
  than a silent fallback to newest.
- **`total` reuses the page's `WHERE`, draft filter included.** Counting without it would tell
  a signed-out visitor exactly how many unpublished posts exist — the number leaks precisely
  what hiding the rows protects. Verified: 25 published + 2 drafts reads as `total=25` to anon
  and `total=27` to an admin.
- **`ORDER BY created_at …, id ASC` — the tiebreaker is load-bearing.** File posts take
  `created_at` from a frontmatter *day*, so they are all midnight and ties are the norm, not
  the exception. Without a total order SQLite may answer page 2 in a different order than page
  1, **showing one post twice and silently dropping another**. Verified by paging 25 tied posts
  at `limit=7` across no-search, `?q=`, and `sort=oldest`: every case covered all 25 exactly
  once.

Frontend: `/blog` appends with a Load more button and a "Showing 20 of 23" line, resetting to
offset 0 whenever the search or sort changes. The loading state only takes over the page when
there is nothing to show yet — while *appending*, the list stays and the button carries it.
`/blog/admin` asks for `BLOG_ADMIN_PAGE_LIMIT` (100) in one go rather than paging, and renders
a visible warning if `total` exceeds it instead of silently showing a prefix.

**This is a coupled change.** The frontend requires the envelope, so a backend still running a
pre-pagination binary breaks `/blog` outright — restart the backend and the dev server
together.

## Why `LIKE` and not FTS5

FTS5 buys bm25 relevance ranking, stemming, and phrase/prefix queries — none of which were
wanted — at the cost of a shadow table that must be kept in sync, which would make the file
sync a second writer into it. A full scan of a personal blog's row count is microseconds.
**Revisit when you want ranked results, not when you have more posts.**

`LIKE` is case-insensitive over ASCII only; a non-ASCII term matches case-sensitively. FTS5
without an ICU tokenizer has the same limitation.

## The three tiers — only one of them enforces

```
proxy.ts           sees: does a cookie exist?    →  UX only. Cannot see is_admin at all.
/blog/admin page   sees: is_admin via /auth/me   →  Hides the editor. Optimistic.
RequireAdmin       sees: the database            →  The only real enforcement.
```

The top two keep people out of a UI that wouldn't work for them. Neither stops a non-admin
from writing a post — a plain `curl` skips both. **Verify authorization with curl, not the
browser.**

## Two places answer "is this an admin"

`require_admin` (the write path) and two inline reads in `blog.rs` — `list_posts:24` and
`get_post:49` — which do `user.as_ref().is_some_and(|u| u.is_admin)` directly.

They agree only while `require_admin` is exactly `user.is_admin`. Make it richer (an
allowlist, a role table, per-resource rules) and **the read paths silently keep the old
policy**. This is the same hazard `apps/fridge-app/CLAUDE.md` already flags for
`fetch_for_viewer` vs `fetch_visible_to`.

Fix if it ever matters: extract `fn is_admin(&User) -> bool`, make `require_admin` the
`Result`-returning wrapper over it, and have the read paths call the predicate.

## Granting admin

The account must exist first — register through the UI, then:

```bash
sqlite3 apps/fridge-app/backend/fridge.db "SELECT email, is_admin FROM users;"
```

```bash
sqlite3 apps/fridge-app/backend/fridge.db "UPDATE users SET is_admin = 1 WHERE email = 'you@example.com';"
```

- **Match the email lowercased** — `normalize_email` trims and lowercases before insert. An
  `UPDATE` matching zero rows still exits 0, so re-run the `SELECT` to confirm.
- **No re-login, no backend restart.** `validate_session` re-joins `users` every request, so
  grants *and* revocations take effect on the next request.
- Set `is_admin = 0` to revoke.

## File-sourced posts are read-only, in the backend

`PATCH` and `DELETE` on a `source = 'file'` post return **409**, not 403 and not 404: the
request isn't unauthorized (the same admin may edit any db post) and the post isn't missing —
it conflicts with the state of that resource. The next sync would rewrite the row from disk, so
an accepted edit would silently vanish.

The admin UI hides Edit and Delete for these posts, but that is **optimistic UI in the same
sense as the `is_admin` check** — a `curl` skips it and meets the 409. Edit the file.

## Verification performed (2026-08-19)

Against a throwaway copy, with `curl` rather than the browser:

| Case | Result |
|---|---|
| Two accounts, before any grant | `403`, `403` |
| Admin creates post | `201` |
| Non-admin creates post | `403` |
| Not signed in | `401` |
| Non-admin PATCH / DELETE another's post | `403`, `403` |
| Draft in list / by slug | admin 2 posts & `200`; non-admin and anon 1 post & `404` |

`cargo test`: 117 passed, clippy clean, `tsc --noEmit` clean.

## Phase 6 verification (2026-08-19)

Same method — throwaway copy of `fridge.db`, `curl` for anything about authorization. One
file-sourced post and two database posts (one published, one draft), with `created_at`
backdated so an ordering change is unambiguous.

**Sort and search, over both kinds at once:**

| Case | Result |
|---|---|
| `?sort=oldest` | order flips for file **and** db post together |
| `?q=` term only in the file post's body | just that post |
| `?q=` term only in the db post's body | just that post |
| `?q=…&sort=oldest` | composes; one request, one query |
| `?sort=bogus` / `?sort=` / `?sort=DESC` | `400`, `400`, `400` |

**`LIKE` escaping** — only the db post contains a literal `%`:

| Query | Returned |
|---|---|
| `?q=100%25` | the db post only |
| `?q=%25` (bare percent) | the db post only — **unescaped this matches every post** |
| `?q=_` (bare underscore) | only the post containing a literal `_` |

**File sync:** startup log `blog sync: 1 created`; slug taken from the filename; `created_at`
from frontmatter. Edit the file → `1 updated`, `created_at` unmoved. Re-sync unchanged → all
zeros, so `updated_at` doesn't churn. Delete the file → `1 deleted`, db posts untouched.
Unpublished file post → invisible to anon, `404` by slug, visible to admin. A file whose slug
collides with a db post → `skipped: 1`, db post unharmed. A file with `pubished:` typo →
`skipped: 1`, named in the log, other files still synced.

**Write protection on file posts:** admin `PATCH` `409`, admin `DELETE` `409`, non-admin
`403`, anon `401`, admin `PATCH` on a **db** post still `200`, unknown id still `404`. Title
unchanged after every attempt. `POST /blog/sync`: anon `401`, non-admin `403`, admin `200`.

**In-browser** (against the same throwaway backend): headings, lists, inline and fenced code,
blockquote, and a GFM table all render; computed styles confirm the CSS is doing the work
(`list-style: disc` restored, `h2` 20px vs 14px body, `pre`/`table` scrolling on their own);
dark mode inverts correctly through `color-mix`. Search debounced 14 keystrokes into **one**
request. Admin page shows "From file" with no Edit/Delete on the file post, Re-sync reports
its counts. **A `<script>` tag typed into the editor renders as literal text — zero `<script>`
elements in the DOM**, which is the `react-markdown`-escapes-by-default property being real
rather than assumed. No console errors.

`cargo test` at the time: **141 passed** (117 + 24 new), clippy clean, `tsc --noEmit` clean,
`npm run lint` still exactly the 2 pre-existing errors.

**Auto-sync verified live** against the real `fridge.db`, with no restart and no button press:
a dropped file appeared in ~3s, an in-place edit landed in ~6s, and deleting the file removed
its post in ~6s. The log carried exactly three lines for those three changes and nothing for
the ~8 quiet ticks in between — which is the guarantee that matters, since a watcher logging
every poll would bury every other line the backend prints.

One bug the compiler caught that a test would not have: `sync` accumulated its skip count in a
local that never reached the returned `SyncReport`, so `skipped` would always have been `0` —
an `unused_assignments` warning, not a failing assertion.

## Adversarial stress test (2026-08-30)

The Phase 6 checkpoint above passed and proved less than it looked like it did: it was written
by the same person who designed the feature, so it exercised the paths as designed. A
deliberately adversarial pass — "assume there are real bugs you haven't seen" — found **five**,
none of which the checkpoint could have caught, because each is a *combination* or a *failure
mode* rather than a feature. Method and hypotheses: `docs/BLOG_STRESS_TEST_PLAN.md`.

| # | Bug | Fixed by |
|---|---|---|
| H1 | A frontmatter typo **deleted the published post**, and repairing it reinserted the post under a new UUID | `source_path` (migration `0016`); the sweep keys on file presence |
| H2 | The watcher advanced its fingerprint before the sync ran, so a tick that did nothing still consumed the change — files added before the first admin never appeared | `SyncOutcome::Deferred`; the fingerprint advances only on `Completed` |
| H3 | Length limits counted **bytes** while the docs promised characters, so a 200-char CJK title 400'd | `models::exceeds_char_limit` |
| H5 | A whitespace-only body was **valid via the API and invalid via a file** | `models::is_blank`, shared by all four call sites |
| H7 | "First-registered admin" was really "lowest UUID" | `ORDER BY created_at ASC, id ASC` |
| H8 | A long unbroken token made the page **2255px wide in a 900px viewport**, pushing every element sideways | `overflow-wrap: anywhere` + `min-width: 0` on `.markdown-body` |
| H9 | A failed search kept the **previous query's results** on screen under the error banner, with no loading state | the catch clears `posts`; `loading` is derived from a request key |

Two hypotheses were **refuted**, and are worth recording so nobody re-investigates them:

- **Timestamp ordering.** File posts (`…T00:00:00+00:00`) and API posts
  (`…T15:16:57.656421+00:00`) genuinely use different encodings, but lexicographic `ORDER BY`
  is still chronologically correct, because the fraction appears only after the seconds field
  and `+` (0x2B) sorts before `.` (0x2E). A **`Z`-suffixed** timestamp *would* break it, since
  `Z` sorts after `.` — nothing in the app writes one, but the Phase 6 checkpoint backdated a
  row by hand with exactly that. `created_at_sorts_chronologically_across_both_post_kinds`
  guards the invariant.
- **`javascript:` URLs in markdown.** `react-markdown` ships
  `safeProtocol = /^(https?|ircs?|mailto|xmpp)$/i` and strips anything else. Note this is a
  *separate* mechanism from the raw-HTML escaping already verified — both are needed, and
  neither implies the other.

**The pattern behind H1 and H2 is the same, and it is worth naming: treating "I looked" as "I
succeeded."** H1 read a failure to parse as evidence the file was gone; H2 read a completed
tick as evidence the work was done. Both are the "disappearance is not closure" trap written
into `docs/PLAN.md` § Phase 7 — which was authored *after* this code shipped with the bug in it.

**H3 and H5 also share a cause:** one rule written twice, in two places, which then drifted.
Both were fixed by extracting a named predicate rather than patching the copy that was wrong.

`cargo test`: **663 passed, 0 failed**, clippy clean, `tsc` clean, lint back to exactly the 2
pre-existing errors. The five backend fixes were re-verified end-to-end against a copy of the
real `fridge.db` over HTTP; H8 and H9 are frontend and were verified in the browser, since
there is no JS test harness in this project.

**Two rules the CSS fix encodes**, both easy to undo by accident:

- `overflow-wrap: **anywhere**`, not `break-word` — only `anywhere` also shrinks the element's
  intrinsic min-content width, which is what stops a flex or grid parent being sized by an
  unbroken token.
- `pre` keeps `overflow-wrap: normal` **on purpose**. Prose wraps; code scrolls. Re-flowing a
  code block changes what it appears to say.

### Still open from that pass

- Untested areas: concurrency (racing `unique_slug` → UNIQUE violation → 500?), sync × API
  interleaving, frontmatter fuzzing (BOM, CRLF, duplicate keys, non-UTF-8, symlinks), and
  volume (1,000 posts).
