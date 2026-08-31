# Blog tab — stress test

> **Status: first pass complete, 2026-08-30.** Seven hypotheses confirmed and fixed, two
> refuted, and the "areas to probe" table below still untouched.
> Findings and fixes are summarised in `docs/BLOG.md` § "Adversarial stress test"; this file
> keeps the *method* and the per-hypothesis reasoning, which is what makes a second pass cheap.
>
> | | |
> |---|---|
> | **Confirmed & fixed** | H1, H1b, H1c, H2, H3, H5, H7, H8, H9 |
> | **Refuted** | H4 (ordering is correct despite mixed encodings), H6 (`react-markdown` strips unsafe URL protocols) |
> | **Not yet attempted** | — |
>
> **Second pass, 2026-08-31** — the "areas to probe" table, worked through. Four findings, five
> refutations. See "Second pass" below.
>
> | | |
> |---|---|
> | **Confirmed & fixed** | J1 (concurrent creates returned 500), J17 (paging could repeat a post) |
> | **Confirmed, not fixed** | J2 (duplicate frontmatter keys), J12 (misleading skip reason) |
> | **Refuted** | J19 auth revocation, J16 pagination edges, J21 volume, J3/J4 bad input, J14 sync × API |

**Goal: find what's broken.** Not to re-confirm the Phase 6 checkpoint, which passed and
proved only that the paths I designed work when driven the way I designed them. Every
hypothesis below is a place that verification could not have caught, because it tests a
*combination* or a *failure mode* rather than a feature.

**Method used:** each confirmed bug got a failing test committed *before* any fix, so the bug
was proven rather than asserted. Fixes came only after review. Hypotheses that turned out
wrong are recorded as refuted rather than quietly dropped — knowing which fears were unfounded
is part of the result, and stops the next pass re-investigating them.

**What the two refutations cost, and why they were still worth writing:** H4 and H6 were the
two hypotheses where reading the code was not enough to settle the question. H4 needed an
actual lexicographic comparison in `sqlite3`; H6 needed reading `react-markdown`'s source.
Both were cheap, and both would otherwise have become "fixes" for problems that did not exist.

---

## Ranked hypotheses

Ordered by likelihood × severity. Each names the test that would prove it.

### H1 — A file that fails to parse **deletes its published post** 🔴 — CONFIRMED, FIXED

`read_dir_posts` skips an unparseable file (`continue`), so its slug never enters
`seen_slugs`. `sync`'s sweep then deletes every `source='file'` row whose slug isn't in that
set. So a typo in the frontmatter of a *live* post silently unpublishes it, and fixing the
typo republishes it **under a new UUID**.

This is precisely the "disappearance is not closure" trap I wrote into the Phase 7 spec, and
I built it into Phase 6 without noticing.

- **Test:** sync a valid file → assert row exists. Corrupt its frontmatter (unknown key) →
  sync → assert the row **still exists**. Currently expected to fail: the row is gone.
- **Second test:** repair the file → sync → assert the id is unchanged from the original.
- **Also covers:** the slug-collision skip path, which drops out of `seen_slugs` the same way.

### H2 — The watcher advances its fingerprint *before* confirming the sync worked 🔴 — CONFIRMED, FIXED

In `spawn_watcher`, `previous = current` runs before `sync(&pool).await`. So if sync fails —
a transient DB error, or **no admin account exists yet** — the change is recorded as seen and
is never retried. Granting `is_admin` after dropping files in means nothing syncs until the
files are touched again.

- **Test (unit-shaped):** drive the fingerprint/sync sequencing directly — sync returning
  `Err` must leave `previous` unadvanced.
- **Test (integration):** empty DB with no admin → drop a file → wait two intervals → grant
  admin → wait two intervals → assert the post appears. Expected to fail today.

### H3 — Length limits count **bytes**, but the contract says characters 🟠 — CONFIRMED, FIXED

`title.len() > MAX_BLOG_TITLE_LENGTH` on a `String` is byte length. `docs/BLOG.md` documents
200 / 100,000 **chars**. A 200-character title of non-ASCII text is 400–800 bytes and gets a
400. Applies to `create_post`, `update_post`, *and* the mirrored checks in `read_dir_posts`.

- **Test:** `create_post` with a 200-char CJK or emoji title → currently 400, should be 201.
  Same for a body just under the char limit but over the byte limit.

### H4 — Mixed timestamp encodings break `ORDER BY created_at` across post kinds — **REFUTED**

`created_at` is TEXT, so sorting is lexicographic — correct only if every row uses an
identical format. File posts get `NaiveDate::and_utc()`, API posts get `Utc::now()`, and the
Phase 6 checkpoint backdated a row with a hand-written `'2026-08-01T00:00:00Z'`. If sqlx
renders any of these differently (`Z` vs `+00:00`, presence/absence of fractional seconds),
ordering silently interleaves wrongly — and this directly attacks the "one query path"
guarantee, which is the whole design claim of the phase.

- **Test:** insert posts of both kinds with known instants, read raw `created_at` strings,
  assert a single format; then assert `?sort=oldest` returns true chronological order.
- **Related:** `sync`'s no-op check compares `created_at <> ?`. A format mismatch there makes
  every sync report `updated: 1` forever and churn `updated_at`.

**Finding — refuted.** The encodings *do* differ: `2026-08-19T00:00:00+00:00` from a file,
`2026-08-30T15:16:57.656421+00:00` from the API. But lexicographic order is still
chronologically correct, because the fractional part appears only *after* the seconds field,
and `+` (0x2B) sorts before `.` (0x2E) — so `00:00:00+00:00` precedes `00:00:00.5+00:00`, which
is right. Confirmed by direct comparison in `sqlite3`, not by reasoning alone.

**The latent risk is real though**, and the test was kept as a regression guard
(`created_at_sorts_chronologically_across_both_post_kinds`): a `Z`-suffixed timestamp breaks
it, since `Z` (0x5A) sorts *after* `.`. Nothing in the app writes `Z` — but the Phase 6
checkpoint backdated a row by hand with exactly that, so it is one careless `sqlite3` away.

### H5 — Whitespace-only bodies: accepted by the API, rejected by the file path 🟠 — CONFIRMED, FIXED

`create_post` checks `req.body.is_empty()`; `read_dir_posts` checks `body.trim().is_empty()`.
A body of `"   "` is a valid post via `POST /blog/posts` and an invalid one via a file. Same
for a title that is whitespace — `create_post` trims first, so that one's fine; the body is
the gap.

- **Test:** `POST` a whitespace-only body → assert 400. Expected to fail (currently 201).

### H6 — `javascript:` URLs in markdown — **REFUTED**

`react-markdown` is XSS-safe for raw HTML — verified. Its URL handling is a *separate*
mechanism (`urlTransform`), which I never tested. `[click](javascript:alert(1))` is the probe.

- **Test:** render that markdown, assert the emitted `href` is not a `javascript:` URL.

**Finding — refuted.** `react-markdown` ships
`safeProtocol = /^(https?|ircs?|mailto|xmpp)$/i` and its `defaultUrlTransform` drops anything
else, so `javascript:` never reaches an `href`. Worth having checked: this is a *different*
mechanism from the raw-HTML escaping verified during Phase 6, and neither one implies the
other. Adding `rehype-raw` would disable the HTML half while leaving this half intact.

### H7 — "First-registered admin" is actually "lowest UUID" 🟡 — CONFIRMED, FIXED

`SELECT id FROM users WHERE is_admin = 1 ORDER BY id LIMIT 1`. `id` is a random UUID, so the
ordering is arbitrary, not chronological. The doc comment and `docs/BLOG.md` both claim
first-registered. With two admins, file-post authorship is arbitrary and can differ between
posts created at different times.

- **Test:** two admins whose UUIDs sort opposite to their `created_at` → assert the older
  account owns the file post. Expected to fail. (Doc-vs-code mismatch; the fix may be to
  correct the doc rather than the code — a review question, not mine to settle.)

### H8 — Layout: long unbroken tokens overflow the post body 🟡 — CONFIRMED, FIXED

`.markdown-body pre` has `overflow-x: auto`, but a long URL or token inside a `<p>` has no
`overflow-wrap`. The body would push the page sideways — the thing the artifact rules call
out as never acceptable.

- **Test:** browser check with a 300-character unbroken string; assert
  `document.body.scrollWidth <= clientWidth`.

**Finding — confirmed.** A 260-character token made the page **2255px wide in a 900px
viewport**, overflowing by 1355px and pushing every other element sideways.

**Fix.** `.markdown-body` gets `overflow-wrap: anywhere` and `min-width: 0`. Two choices:
`anywhere` rather than `break-word`, because only `anywhere` also shrinks the element's
intrinsic *min-content* width — which is what stops a flex or grid parent being sized by the
unbroken token in the first place; and `pre` explicitly keeps `overflow-wrap: normal`, because
prose should wrap but code should scroll — re-flowing a code block changes what it appears to
say. Verified at 900px and at 375px, with the fenced block still scrolling inside its own box.

### H9 — A failed search leaves stale results on screen 🟡 — CONFIRMED, FIXED

`fetchPosts` rejection sets `error` but leaves `posts` populated, so the list shows the
previous query's results under an error banner. Also `setLoading(false)` never returns true
after mount, so slow searches give no feedback.

- **Test:** browser, with the backend stopped mid-session.

**Finding — confirmed**, by intercepting `window.fetch` to reject `/blog/posts`: the error
banner appeared *and* the previous query's post stayed in the list, presenting stale content
as though it answered the current search.

**Fix.** The catch clears `posts`, so a failed search shows the error alone. `loading` is now
**derived** — a `requestKey` of sort + trimmed query, compared against the last settled key —
rather than a `setLoading(true)` at the top of the effect. That pattern is the
`react-hooks/set-state-in-effect` error this codebase already carries two of, and it would not
have shown a spinner on re-search anyway, since the original only ran once on mount.

**A note on measuring this one.** Two of my probes reported "no spinner" and were both wrong:
the first sampled before the 250ms debounce had fired, the second after the slow-response patch
had been replaced by an instantly-rejecting one. Only a *timed series* across the request
showed the truth — `Overflow Probe` → `Loading…` → `No posts match "zz"`. A single
badly-timed sample of an async UI is evidence of nothing.

---

## Areas to probe beyond the ranked list

Where I have no specific hypothesis but the code is under-exercised:

| Area | Probes |
|---|---|
| **Concurrency** | Two `POST /blog/posts` with the same title racing `unique_slug` → UNIQUE violation → 500? Sync running while an admin PATCHes? |
| **Sync × API interleaving** | Delete a file and `PATCH` its post in the same window. Create a db post whose slug a pending file wants. |
| **Frontmatter fuzzing** | Fence with trailing spaces (`--- `), CRLF mixed with LF, BOM, duplicate keys, `key:` with no value, a 1MB single line, non-UTF-8 bytes, a symlink, a `.md` directory. |
| **Search edge cases** | `q` at 100KB; `q` of only `%`/`_`/`\`; combining characters; `q` matching 1,000 posts — paging now exists, so check `total` against a page. |
| **Volume** | 1,000 file posts: sync duration, watcher tick cost, unpaginated list response size. |
| **Auth boundaries** | Revoke `is_admin` mid-session → does the open editor degrade correctly? Draft visibility for a *second* admin. |
| **Expiry of assumptions** | `BLOG_CONTENT_DIR` pointing at a file, a missing dir, a dir that disappears while the watcher runs, a dir with no read permission. |

---

## Method

1. **Unit/integration tests in the Rust suite** for H1–H5, H7 — these are logic bugs and belong
   where they'll keep being checked. Sync tests need a pool; if no in-memory-SQLite test
   harness exists yet, building one is part of the work (and is reusable for Phase 7).
2. **`curl` against a throwaway `fridge.db` copy** for anything about authorization or HTTP
   status, per the repo's own rule. Never the real database.
3. **Browser** only for H6, H8, H9, which are rendering and UI-state bugs.
4. Run the backend on `PORT=8081` so nothing running is disturbed.

**Format-safety:** `cargo fmt` and `rustfmt src/main.rs` both reformat the whole module tree
including `[learn]` files. Format leaf files only, and diff-check the six `[learn]` files
before reporting.

## Outcome

Seven failing tests were written and shown red, then the five underlying bugs were fixed and
each re-verified end-to-end over HTTP against a copy of the real `fridge.db` — not only in
unit tests. `cargo test`: **635 passed, 0 failed**.

Both review questions were resolved:

- **H1's correct behavior is "keep the post untouched"**, with the parse failure surfacing as a
  skip in the sync report and the log. "Keep but mark stale" would have needed a new state in
  the schema for a condition the author fixes in seconds.
- **H7 was a code bug, not a documentation bug.** The docs described the more useful behavior;
  the query was simply wrong.

### The lesson worth carrying forward

Four of the five bugs fall into two pairs, and both pairs are general:

1. **H1 and H2 both treated "I looked" as "I succeeded."** A failure to *read* was taken as
   evidence of *absence*. This is the "disappearance is not closure" trap already written into
   `docs/PLAN.md` § Phase 7 — authored *after* this code shipped with two instances of it.
   Phase 7's collection runner has exactly the same shape and should be audited for it.
2. **H3 and H5 were both one rule written twice that drifted.** Each was fixed by extracting a
   named predicate (`exceeds_char_limit`, `is_blank`) rather than patching the wrong copy.

The checkpoint that "passed" was written by whoever designed the feature, and so tested the
paths as designed. That is the structural reason this pass found anything at all.

---

# Second pass — 2026-08-31

The first pass drained the ranked hypotheses. This one worked through the "areas to probe"
table, which had no specific suspicions attached — so the expected yield was lower, and was.
Everything below ran against a throwaway copy of `fridge.db` on an isolated port.

## Confirmed

### J1 — concurrent creates returned 500 🔴 — FIXED

`unique_slug` ran `SELECT EXISTS(...)` to find a free slug, then `create_post` inserted, with
nothing holding between the two. Both requests could see the same slug free; the loser hit the
`UNIQUE` constraint, which mapped to 500.

**Not theoretical.** Ten concurrent creates of the same title produced **six 500s and only four
posts**. One double-clicked submit button reaches it.

**Fix:** the constraint is the only thing that can decide atomically, so it decides.
`insert_post_with_unique_slug` attempts the insert and treats a unique violation as "someone
took that one", stepping to the next suffix. Bounded at 50 attempts — every retry is driven by
a database error, and `is_unique_violation` also covers the `id` primary key, which is not
re-rolled, so an unbounded loop could spin forever on a non-slug cause. Re-running the
identical race: **10× 201, ten distinct slugs, zero 500s.**

### J17 — paging can show the same post twice 🟠 — FIXED

Offset pagination over a list sorted newest-first. Publish a post while someone is browsing and
every row shifts down one, so page 2 re-serves the last row of page 1. Demonstrated:
`race-me` appeared in both pages, and the accumulated Load more list renders it twice. The
mirror-image failure — a post *skipped* — happens when a row is removed mid-browse.

Inherent to offset pagination — an offset is a position in a list that moves.

**Fix: keyset paging.** `offset` is replaced by an opaque `cursor` carrying the
`(created_at, id)` of the last row on the page — the sort key itself, because a cursor that
does not match the `ORDER BY` cannot describe a position in it. The response carries
`next_cursor`, `null` on the last page, so "is there more" is the server's answer rather than
arithmetic on `total` that concurrent publishing invalidates.

Two things the fix had to get right:

- **`total` is counted without the cursor**, so "showing 20 of 143" keeps saying 143 as you
  page instead of counting down what is left. Verified: publishing mid-walk moved it 30 → 31.
- **The cursor predicate mirrors the mixed `ORDER BY`.** Sorting is `created_at <dir>, id ASC`
  — direction varies on one column but not the other — so it cannot be a single row-value
  comparison and is written as `(created_at <op> ? OR (created_at = ? AND id > ?))`. Getting
  this wrong does not error; it silently skips or repeats, which is the bug being fixed.

Verified: publishing between page 1 and page 2 now yields **no overlap**, and a full walk in
4-row pages while publishing every third page covered all 30 originals with **zero duplicates**
in both sort directions. Malformed cursors are 400, not an empty first page.

### J2 — duplicate frontmatter keys silently take the last 🟡

`title: First` … `title: Second` stores "Second" with no warning. This contradicts the parser's
own governing rule: an **unknown** key is a hard error precisely to catch typos, but a
**duplicated** key — equally likely a mistake — passes silently.

### J12 — misleading skip reason 🟡

A frontmatter `slug: "!!!"` that slugifies to nothing is skipped with *"filename produces an
empty slug"*. The filename was fine; the frontmatter was not. The reader renames the wrong
thing.

## Refuted

- **J19 — auth revocation.** Immediate and complete with no re-login: drafts 7→6, draft slug
  404s, `create` 403s, regrant restores. Exactly as `docs/BLOG.md` claims.
- **J16 — pagination edges.** Offset past the end returns empty with a correct total; `u32`
  max is 200; overflow is 400.
- **J21 — volume.** 1000 file posts: initial sync **380ms**, no-op re-sync **127ms**, every
  read (page 1, search, deep offset) **9ms**. A quiet watcher tick over 1006 files is
  **2.1ms**. This is direct evidence for two earlier decisions that were argued rather than
  measured: **LIKE over FTS5**, and **polling over `notify`**.
- **J3 / J4 — hostile input.** Invalid UTF-8, a directory named `*.md`, and a 100KB query are
  all handled with accurate reasons (the query gets a 414 from the server, not a crash).
- **J14 — sync × API collision.** A file and an API create racing for the same slug resolved
  cleanly: the API create won, sync skipped with a correct log line.

## A documented-behaviour gap, not a bug

**Removing the entire content directory keeps every post; removing one file deletes its post.**
With 1007 posts synced, moving the directory away left all 1007 in place, and an explicit sync
reported all zeros. The conservative choice is right — a missing directory is ambiguous between
"content deleted" and "disk unmounted", and this is the same principle that fixed H1 — but it
contradicts the flat claim that sync "mirrors" the directory. Stated here so it is a decision
rather than a surprise.

## Still unexplored

Frontmatter fuzzing was only partly covered (BOM, CRLF, duplicate keys, non-UTF-8, directories
and empty slugs were tested; symlinks and megabyte-single-line files were not), and nothing
here touched the *frontend* beyond what the first pass covered.
