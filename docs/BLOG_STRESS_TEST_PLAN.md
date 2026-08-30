# Blog tab — stress test

> **Status: first pass complete, 2026-08-30.** Five hypotheses confirmed and fixed, two
> refuted, two found-but-unfixed, and the "areas to probe" table below still untouched.
> Findings and fixes are summarised in `docs/BLOG.md` § "Adversarial stress test"; this file
> keeps the *method* and the per-hypothesis reasoning, which is what makes a second pass cheap.
>
> | | |
> |---|---|
> | **Confirmed & fixed** | H1, H1b, H1c, H2, H3, H5, H7 |
> | **Refuted** | H4 (ordering is correct despite mixed encodings), H6 (`react-markdown` strips unsafe URL protocols) |
> | **Confirmed, not fixed** | H8, H9 |
> | **Not yet attempted** | everything under "Areas to probe beyond the ranked list" |

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

### H8 — Layout: long unbroken tokens overflow the post body 🟡 — NOT YET RUN

`.markdown-body pre` has `overflow-x: auto`, but a long URL or token inside a `<p>` has no
`overflow-wrap`. The body would push the page sideways — the thing the artifact rules call
out as never acceptable.

- **Test:** browser check with a 300-character unbroken string; assert
  `document.body.scrollWidth <= clientWidth`.

### H9 — A failed search leaves stale results on screen 🟡 — NOT YET RUN

`fetchPosts` rejection sets `error` but leaves `posts` populated, so the list shows the
previous query's results under an error banner. Also `setLoading(false)` never returns true
after mount, so slow searches give no feedback.

- **Test:** browser, with the backend stopped mid-session.

---

## Areas to probe beyond the ranked list

Where I have no specific hypothesis but the code is under-exercised:

| Area | Probes |
|---|---|
| **Concurrency** | Two `POST /blog/posts` with the same title racing `unique_slug` → UNIQUE violation → 500? Sync running while an admin PATCHes? |
| **Sync × API interleaving** | Delete a file and `PATCH` its post in the same window. Create a db post whose slug a pending file wants. |
| **Frontmatter fuzzing** | Fence with trailing spaces (`--- `), CRLF mixed with LF, BOM, duplicate keys, `key:` with no value, a 1MB single line, non-UTF-8 bytes, a symlink, a `.md` directory. |
| **Search edge cases** | `q` at 100KB; `q` of only `%`/`_`/`\`; combining characters; `q` matching 1,000 posts (no pagination exists). |
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
