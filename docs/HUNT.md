# Internship hunt tooling — reference

Phase 8: a tool for running an internship hunt. Two tracks that share a backend and are
otherwise independent — an inbox agent that reads a burner Gmail (Track A), and a Firefox
extension that fills applications and raises desktop alerts (Track B).

**Only 8e is built.** This is the **what and where** for what exists, plus a map of the seams
it left for the rest. Design *rules* live in `apps/hunt-extension/CLAUDE.md` — read that before
changing anything here, because most of what looks like a detail below is a rule from that
file made concrete. Phase status is `docs/PLAN.md` § Phase 8.

**None of this is Learning Mode.** The user decided on 2026-08-29 that all of Phase 8 is
`[gen]`, including the email classifier and the email→application matcher, both of which are
NLP-shaped and would otherwise fall under the flagged subsystems. That exception does not
extend to the `[learn]` files themselves: `src/nlp.rs` and friends may be **called** and never
edited.

## Feature overview

| Feature | State |
|---|---|
| `hunt_events` table, both producers designed in | ✅ migration `0014` |
| `GET /hunt/events` — poll for undelivered alerts | ✅ `CurrentUser`-scoped |
| `POST /hunt/events/{id}/ack` — delivery receipt | ✅ idempotent |
| Posting producer — tier-1/2 company posts something new | ✅ `internships::alerts` |
| Firefox MV3 extension: alarm poll, notifications, popup, options | ✅ `apps/hunt-extension/` |
| Verified in Firefox itself | ⬜ **not yet** — see "What is not proven" below |
| Email producer — OA / interview / offer mail | ⬜ 8d, writes to the same table |
| Gmail OAuth, sync, classify, match, labels | ⬜ 8a–8c |
| CV autofill on ATS pages | ✅ Greenhouse, Lever, Ashby — verified live 2026-08-30 |
| Answer library | ⬜ 8g |

## The one structural idea

**Two producers, one table, one poll endpoint, one notification path.**

```
  internships::collector ─── new posting from a tier-1/2 company ───┐
                                                                    ├─► hunt_events
  inbox::classify (8d) ───── mail that means OA/interview/offer ────┘        │
                                                                             ▼
                                              GET /hunt/events ──► extension background poll
                                                                     └─► browser.notifications
```

8d adds a producer, not a pipeline. If a future alert kind needs its own table, its own poll,
or its own notification code, something has gone wrong with this shape.

## Data model

**`hunt_events`** (migration `0014_create_hunt_events.sql`):

| Column | Notes |
|---|---|
| `id` | UUID text, like every other table here |
| `kind` | `'posting'` \| `'email'`. The extension filters alert kinds on it, so "should cold outreach interrupt me" stays a client-side predicate rather than a schema change |
| `user_id` | **NULL = from the shared posting corpus, visible to every signed-in user. NOT NULL = private to that user.** The email producer must always set it; a leak would require it to write NULL, which is a visible bug at the write site rather than a forgotten predicate at the read site |
| `subject_id` | What the event is about, and the idempotency key: a posting id, or a Gmail message id in 8d. Polymorphic, so no `REFERENCES` clause is possible |
| `title`, `body` | **Rendered, not structured.** One notification path serves both producers; each producer decides how its own event reads |
| `url` | Where clicking goes. Nullable |
| `payload_json` | The facts behind those two lines — company, tier, term, source — for the popup |
| `created_at` | |
| `acked_at` | **A delivery receipt, not a user dismissal.** Set once a client has raised a notification. NULL means undelivered, which is what the background poll asks for |
| `UNIQUE (kind, subject_id)` | One alert per subject, made structural. Re-running collection cannot write a second event even if the producer's newness check later changes |

Two indexes: a partial one on `created_at WHERE acked_at IS NULL` for the poll, and a plain one
on `created_at` for the popup's recent-alerts list.

**Ack is global for a NULL-user event.** A second registered user acking a posting alert acks
it for everyone. Accepted deliberately for a single-user tool; per-user ack state would be a
`hunt_event_acks (event_id, user_id)` join table, and the time to add it is when a second
person actually uses this.

## Backend

### `src/hunt/events.rs` *(new — `[gen]`)*

Producer-agnostic. Knows how an event is stored, who may see it, and what "already delivered"
means — never what is worth alerting about.

| Item | What it does |
|---|---|
| `EventKind` | `Posting` \| `Email`, `sqlx::Type` + serde, lowercase. `as_str` matches the migration's CHECK |
| `NewHuntEvent` | What a producer builds: kind, optional `user_id`, `subject_id`, rendered title/body/url, `payload` |
| `HuntEvent` | What the API returns. `HuntEventRow` is the row; the `From` impl parses `payload_json` and falls back to `Value::Null` — an unreadable payload costs the popup some detail, never the alert |
| `VISIBLE_TO_VIEWER` | `(user_id IS NULL OR user_id = ?)`, written once and used by every read so the two halves cannot drift |
| `emit` | `INSERT … ON CONFLICT (kind, subject_id) DO NOTHING`. Returns whether a row was written; `false` is normal, not an error |
| `EventQuery` | `viewer`, optional `since`, `include_acked`, `limit` |
| `list` | Visible events, newest first |
| `unacked_total` | Every undelivered event, not just the ones under `limit`. Without it a truncated page is indistinguishable from a complete one |
| `ack` | Idempotent, first receipt wins. Returns `Acked` / `AlreadyAcked` / `NotFound` — "not yours" and "doesn't exist" are deliberately one outcome |

### `src/internships/alerts.rs` *(new — `[gen]`)*

The whole predicate: **curated tier 1 or 2, and nothing else.**

| Item | What it does |
|---|---|
| `ALERT_TIERS` | `[1, 2]`. Named so "which tiers alert" is one line to read |
| `posting_event` | `Option<NewHuntEvent>` for a newly collected posting. Judges the company, not the novelty — the caller decides what is new |
| `MAX_BODY_CHARS` | 140. A notification shows about two lines and truncates the rest silently; cutting here makes the cut visible and lands it after the role |
| `notification_body` | Role, then term, then location — **absent facts omitted, never filled in** |
| `summarize_locations` | `Palo Alto, CA +29 more`. See the verification section for why this exists |
| `truncate` | Cuts on a character boundary (`chars`, not bytes) and appends `…` |

### `src/routes/hunt.rs` *(new — `[gen]`)*

```
GET  /hunt/events?since=&include_acked=&limit=    CurrentUser
     -> { "events": [ … ], "unacked_total": n }
POST /hunt/events/{id}/ack                        CurrentUser -> 204
```

| Item | What it does |
|---|---|
| `list_events` | Default limit 50, clamped at 200 — an oversized `limit` is a client bug, not a reason to fail. A malformed `since` is a **400**, not silently ignored |
| `ack_event` | `204` for both `Acked` and `AlreadyAcked`; `404` for unknown or invisible. A retry the extension isn't sure landed must not look like a failure, or it gives up and re-notifies instead |

**The background poller does not send `since`.** A watermark held by the client is exactly the
state an MV3 background page loses, and an event that arrived while Firefox was closed would
sit behind a watermark that had already moved past it. `acked_at` is the record; `since` exists
for the popup.

### `src/internships/collector.rs` *(changed)*

| Change | Why |
|---|---|
| `Upserted { id, created }` replaces `Result<bool>` from `upsert_posting` | The producer needs a stable key, and the function had already looked the id up |
| `emit_posting_alert` | Called only where a posting is genuinely new. **A failed alert never fails the posting** — it is logged, not recorded as a reject, because a reject means the row did not land and this one did |
| `CompanyTiers::load()` hoisted into `collect_with` | It reads a file. Shared with `recompute_company_signals`, which used to load it separately |
| `CollectionReport.alerts_created` | Surfaces in `POST /internships/collect`, so the checkpoint is checkable from the response instead of by hand in `sqlite3` |

Also changed: `src/main.rs` (`mod hunt`), `src/routes/mod.rs` (two routes),
`src/routes/internships.rs` (`CollectionSummary.alerts_created`).

### `src/inbox/labelset.rs` *(new — `[gen]`)*

8b's measurement harness. **Tooling, not pipeline** — it never runs during a sync, writes to no
table and no mailbox, and reads only. Reached as a subcommand of the server binary, dispatched
in `main.rs` before any background work starts:

```
cargo run --release -- labelset export --out labelsets/aug.csv [--since ISO] [--until ISO]
cargo run --release -- labelset score  --labels labelsets/aug.csv
```

| Item | What it does |
|---|---|
| `export` | Every stored message in the window to a CSV with an empty `label` column. "Every" is load-bearing: filtering to job-looking mail yields a set that cannot measure the relevance gate |
| `Row` | Exactly the fields `classify` gets — sender, subject, snippet — **and no verdict column**. Pre-filling the machine's answer measures how agreeable the reviewer is |
| `score` | Re-runs `classify::classify` over the labelled rows. Deliberately not `email_verdicts`, which holds whatever rules ran the day the sync did |
| `Summary` | The two failure modes with **separate denominators**, and no `accuracy` field to quote instead. Also `pressing_missed` — rule 8's failure is broader than the disregard branch |
| `rules_fingerprint` | SHA-256 of `classify.rs`, compiled in via `include_str!`, so it describes the binary doing the grading |
| `Ledger` | `<labels>.graded.json`. A message is **spent** when it was graded under a *different* fingerprint — grading does not spend a set, changing the rules after seeing the result does. Re-scoring under unchanged rules is free |

`labelsets/` is gitignored: the sheets hold real subject lines and snippets.

## The extension — `apps/hunt-extension/`

Firefox MV3, plain JS. No bundler, no framework, no TypeScript, no dependencies.

| File | Notes |
|---|---|
| `manifest.json` | `background.scripts`, **not** `service_worker` — Firefox MV3. `browser_specific_settings.gecko.id` is `hunt@personal-website` and is **required**, or stored settings do not survive a sideload. Permissions are exactly `alarms`, `notifications`, `storage`, plus host permission for `localhost:8080` and `127.0.0.1:8080` |
| `background.js` | The event page. See below |
| `popup/` | Recent alerts, last check, "Check now" |
| `options/` | Backend URL, site URL, poll interval, notification budget, alert kinds, **Test connection** |
| `icons/icon.svg` | Firefox accepts SVG for extension icons. Notifications carry no `iconUrl` and fall back to this |
| `README.md` | How to sideload |

### `background.js`

| Function | What it does |
|---|---|
| `poll` | Fetches, filters by enabled kinds, raises, records status. **Never throws** — the callers are an alarm and a button |
| `raise` | Notifies up to `maxNotificationsPerPoll` (default 3) individually, one summary for the rest, then acks everything shown |
| `notify` | The **notification id is the event id**, so a re-notify after a failed ack replaces rather than stacks |
| `ack` | Failure leaves the event unacked on purpose: it is offered again next poll |
| `setStatus` | Records one of `ok` / `unreachable` / `unauthenticated` / `error` |
| `cacheForPopup` | Last 50 events, so the popup is never blank and a notification click can still resolve its URL after the page has been killed |
| `bumpUnseen` / `paintBadge` | Badge counts alerts raised since the popup was last opened; `!` in red for any non-`ok` status |
| `ensureAlarm` | Creates the alarm **only if absent or at the wrong period**. `alarms.create` on an existing name resets the countdown, so calling it on every wake would push the next poll further out each time and polling would quietly stop |

Every listener is registered synchronously at the top level, or the event meant to wake the
page arrives before anything is listening.

**Notify, then ack — in that order.** At-least-once, deliberately. A failed ack costs a
duplicate notification; acking first would cost a silently dropped alert.

## Auth: a bearer token, because the cookie cannot get there

**The cookie was tried first and it does not work.** `fridge_session` is `SameSite=Lax`, a
request from a `moz-extension://` page is cross-site, and Firefox therefore never attaches it.
The backend saw an anonymous request and answered 401 while the user was demonstrably signed
in on the site. That is the fallback condition `apps/hunt-extension/CLAUDE.md` anticipated.

`hunt_tokens` (migration `0015`) holds SHA-256 hashes of long-lived bearer tokens. Minted from
the site's **Extension access** panel on the internships tab, pasted into the extension's
Settings, sent as `Authorization: Bearer …`.

**This is a second credential, not a second auth system**, and the difference is what keeps the
8e constraint honest:

- Hashing and generation are `auth::session_token_hash` / `auth::generate_session_token` —
  **called, never modified**; `src/auth.rs` is a `[learn]` file. One definition of "hash a
  bearer credential" rather than two that drift.
- The token is accepted inside the existing `MaybeUser` extractor, so every route keeps its
  `CurrentUser` signature and **no route knows tokens exist**. A token is exactly as powerful
  as a session and no more.
- Minting requires being signed in already, so this widens how you prove who you are, never
  who you can be.

**No expiry column, deliberately.** A session expires because it rides in a browser you walk
away from; this is a device credential, and a clock nobody watches would silently stop the
notifier weeks later — a failure indistinguishable from a quiet job market. The control is
revocation, which is explicit and visible.

## The auth path that did not work, kept for the record

`host_permissions` for the backend origin plus `fetch(..., { credentials: "include" })` puts the
ordinary `fridge_session` cookie on the request. Signed in on the site means signed in in the
extension. **There is no extension token and no second auth path.**

### Firefox does not grant `host_permissions` from the manifest

**Found the hard way on 2026-08-29, and it will bite Phase 8f the same way.** In Chrome,
`host_permissions` are granted at install and `fetch` to that origin just works. **Firefox MV3
treats them as optional** — the user grants them per origin — and until they do, the fetch
fails with a bare `TypeError` that is *identical* to the one a dead backend produces. The
first version of this extension assumed Chrome's behaviour and consequently reported "can't
reach localhost" while the backend was serving 200s to `curl`.

So the extension asks rather than assumes: **Test connection** calls `permissions.request`,
which Firefox honours because it runs inside a click handler. The background page cannot ask —
`permissions.request` requires a user gesture and an alarm is not one — so `poll()` checks
`permissions.contains` first and reports the distinct `unpermitted` state instead of blaming
the server.

### The backend must name the extension's origin, and now does so by scheme

`credentials: "include"` makes every extension request a **credentialed cross-origin** request,
and the browser discards the response unless it carries an `Access-Control-Allow-Origin` naming
the caller. `ALLOWED_ORIGINS` listed the site's origins and nothing else, so the same endpoint
returned the header to `http://localhost:3000` and no header at all to `moz-extension://…`.
The response reached JS as a bare `TypeError` — a third distinct cause producing the identical
"can't reach" symptom.

`routes::is_allowed_origin` now admits **any** `moz-extension://` origin alongside the
configured list. Pinning one UUID was the tighter option and was rejected deliberately: the
UUID is per Firefox profile, so it would mean a per-machine `.env` edit that silently breaks on
a new profile. **The accepted cost is that any Firefox extension the user installs could call
this API with their session cookie** — narrowed by Firefox making the user grant a host
permission per extension, but real, and a local-development posture rather than a deployable
one. Revisit it with the other three items in `docs/PLAN.md` § After Phase 5.

### The failure modes, kept distinct

They all present as "no notifications" and have completely different fixes, so each is a
separate stored state rather than one "error":

| State | Meaning | Fix |
|---|---|---|
| `unpermitted` | the origin is requested but not granted | Test connection, and accept the prompt |
| *discarded response* | no CORS header for this origin | the backend must allow it — fixed by the scheme rule above |
| `unreachable` | the request left and got nothing back | start the backend |
| `unauthenticated` | reachable, 401 | sign in on the site — **or this is the SameSite problem** |
| `error` | reachable, unexpected status | read the status code |

The remaining open question is the cookie: `fridge_session` is `SameSite=Lax` and a request
from a `moz-extension://` page is cross-site. Host permissions exempt the request from CORS;
whether Firefox also attaches a Lax cookie is what `unauthenticated` vs `ok` answers.

## Three rules worth not rediscovering

1. **Dedup is the server's job.** An MV3 background page is killed and restarted at the
   browser's convenience, so anything it remembers is gone and every alert re-fires.
   `browser.storage.local` is a cache, never the record.
2. **`None` prestige is not tier 3.** An unlisted company scores `None`, meaning *unknown*.
   The curated file names 44 companies of ~455 in the corpus, so alerting on `None` alerts on
   essentially everything, and a channel that fires on everything gets muted — taking the
   tier-1 alerts with it.
3. **The alert predicate reads the tier file, not `company_signals.prestige`.** The derived
   band exists to *rank* companies we know little about, which is a different question from
   whether to wake someone up. And that table is recomputed after every source finishes, so at
   the moment a posting is inserted it holds no score for a company first seen in that run.

## Verification performed (2026-08-29)

22 tests added: 9 in `hunt::events`, 9 in `internships::alerts`, 4 in
`collector::integration_tests`. Clippy produced no new warnings.

Verified against a **real Simplify run over a copy of the live database**, not fixtures:

- **A new tier-1/2 posting writes exactly one row.** 2,247 rows fetched, 206 postings created,
  **22 alerts — every one a tier-1 or tier-2 company.** Tier-3 controls (Intel) were deleted and
  re-collected and produced nothing. ✅
- **Re-running collection does not write a second event.** A second run updated 1,097 postings
  and reported `alerts_created: 0`. ✅
- **The endpoints behave.** `204` first ack, `204` repeat, `404` unknown id, `401` signed out,
  `400` on a malformed `since`; `unacked_total` fell 22 → 21 and the acked event left the poll
  while staying in the popup's view. ✅
- **The extension's logic, driven against the live backend** with a stubbed WebExtension API:
  10 waiting events produced 3 notifications plus one "+7 more", all 10 acked, and an immediate
  second poll raised nothing. All three failure modes came out distinct and badged. ✅

### What the real run caught that the tests could not

The same pattern `apps/fridge-app/CLAUDE.md` records for four earlier scoring functions.

**Simplify packs every city a posting is open in into one location string.** A single Google
posting produced a **429-character** notification body with the role pushed off the end — and
this is the normal shape of a big-company listing, not an outlier, because `dedup` deliberately
keeps location out of the merge key so per-location rows merge into one posting. Locations now
collapse to `first +N more` and the body is capped at 140 characters. Pinned by a test using
the real 30-city string.

### Verified in Firefox, 2026-08-30

The checkpoint, end to end, against the real database and a real Firefox:

- **A new tier-1/2 posting raises exactly one notification.** ✅
- **A tier-3 or untiered one raises none.** ✅
- **Re-running collection raises no second notification.** ✅
- **Acking, then quitting and reopening Firefox, does not re-raise it.** ✅ 39 of 39 events
  acked; nothing fired on restart. This is the property the whole design turns on — the
  background page loses everything it knew, so only `acked_at` on the server prevents a
  re-notify.

### Four causes, one symptom — the thing that actually cost the day

Every one of these presented as *"can't reach the backend"*, and two were misdiagnosed
confidently before anyone measured:

| Cause | Why it looked like a dead server |
|---|---|
| The dev servers genuinely dying | It was a dead server, twice, mid-test |
| Firefox MV3 not granting `host_permissions` | Chrome grants them at install; Firefox makes the user grant them, and a blocked fetch throws a bare `TypeError` |
| CORS not naming the extension's origin | The backend answered 200 and the browser discarded the response before JS could see it — again a bare `TypeError` |
| `SameSite=Lax` on the session cookie | The request arrived with no cookie, so the 401 was truthful and useless |

**The lasting fix is not any one of those repairs — it is that they now report themselves
distinctly.** `unpermitted`, `unreachable`, `no-token`, `token-rejected`, each a separate
stored state with its own message naming its own fix. A single "can't reach" covering four
unrelated causes is what turned a ten-minute check into a day, and 8f authenticates the same
way, so it inherits the diagnosis rather than the search.

## Autofill (Phase 8f)

`cv_profile` (migration `0017`) holds the details; the site's **CV details** panel edits them;
the extension fills them into ATS forms **on a button press and nothing else**.

| Piece | Where |
|---|---|
| What may be filled, and what must never be | `content/fields.js` — pure, no DOM |
| Reading labels, setting values, reporting | `content/fill.js` |
| The button, injection, the tracking offer | `popup/popup.js` |
| The profile itself | `hunt::profile`, `GET`/`PUT /hunt/profile` |
| Matching a page to a collected posting | `GET /hunt/posting-for` |

**Labels, not selectors.** ATS markup is regenerated on every redesign; the visible label
survives. Five sources in order: `label[for]`, a wrapping `<label>`, `aria-labelledby`,
`aria-label`, `placeholder`, then `name`/`id` as a last resort.

**The blocklist runs before any matching** — rule 10's "checked before the fuzzy mapper, not
after". Reordering it is a silent safety regression that passes every happy-path test.

**Three refusals beyond the blocklist**, each its own failure if missed: a field that already
holds a value is never overwritten; radios and checkboxes are skipped entirely, which is where
EEO questions live; nothing is ever clicked.

### What reading three live forms caught

None of it was visible against a synthetic form, and one was in the safety layer.

| Form | Defect |
|---|---|
| Lever | **The demographic blocklist silently failed.** A `<select>`'s options are rendered into the label, so "Gender" arrives as `GenderSelect ...MaleFemale…` — normalized `genderselect`, and `\bgender\b` never fires. "Veteran status" matched *only* because its pattern happens to lack a word boundary, which is what made the failure look like success |
| Lever | "Other website" sits beside "Portfolio URL" and both were filled with the portfolio |
| Ashby | The name field is labelled simply "Name", excluded on purpose because it matches inside "Company Name" — now an exact-only match |
| Ashby | A real `g-recaptcha-response` textarea is in the form. Nothing mapped to it, but rule 11 says do not touch CAPTCHAs, and "nothing happens to match" is not a policy |
| Greenhouse | "What is your expected graduation date?" matched nothing. `graduation_date` is now derived as `MM/YYYY`, and fills **only** when both month and year are known — a bare year in a box labelled *date* looks like a complete answer and is not |

Every label from all three is now a test: 79 checks in `content/fields.test.mjs`, runnable with
`node content/fields.test.mjs`, against markup that exists rather than markup someone imagined.

### Two Firefox defaults that both point the wrong way for embeds

A company careers page often embeds the ATS rather than linking to it. `content_scripts`
defaults `all_frames` to **false**, so a matching document only gets the script as a tab's top
frame — an embedded Greenhouse form, whose URL matches perfectly, was skipped. And
`activeTab` grants the top-level origin only, so `allFrames` on `executeScript` cannot reach a
cross-origin iframe on its own. The declarative match with `all_frames: true` is what actually
covers the embed.

`scripting.executeScript` file paths also resolve relative to the **calling page**, not the
extension root — from `popup/popup.html`, `content/fields.js` became `popup/content/fields.js`.
Chrome resolves from the root, which is why every example omits the leading slash.

## Seams left for the rest of Phase 8

- **8d writes to `hunt_events`, it does not alter it.** `kind = 'email'`, `user_id` set,
  `subject_id` = the Gmail message id. Nothing about the poll, the ack, or the extension's
  notification path should need to change.
- **The `email` alert kind already has a checkbox** in the options page and a filter in
  `poll()`. It has no producer.
- **The inbox tables are migration `0015`**, and Track B's `cv_profile` is `0016`.
  `hunt_events` took `0014` because 8e needed it first.
- **The extension shows no internship list**, only alerts and a link out. Whether it should is
  an open question in `apps/hunt-extension/CLAUDE.md`.
- **8b's checkpoint is waiting on mail, not on code.** `src/inbox/labelset.rs` is the harness
  for it; the burner held 14 messages over 2 days with zero digests when it was written, so the
  relevance gate still has nothing to measure against.
