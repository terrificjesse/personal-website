# Internship hunt tooling — reference

Phase 8: a tool for running an internship hunt. Two tracks that share a backend and are
otherwise independent — an inbox agent that reads a burner Gmail (Track A), and a Firefox
extension that fills applications and raises desktop alerts (Track B).

**Status 2026-09-02: 8a–8g and Phase 9 are all built.** This file said *"only 8e is built"*
until today, which stopped being true on 2026-08-30 and was never corrected — the reconciliation
in task 10a is what found it. Two things are genuinely unfinished, and both are waiting on
something other than code: **8b's checkpoint** needs a hand-labelled fortnight of real mail
(Phase 13), and **the browser half of 8g** needs the extension loaded in Firefox against two
live ATS forms (task 12g).

This is the **what and where** for what exists, plus a map of the seams it left. Design *rules*
live in `apps/hunt-extension/CLAUDE.md` — read that before changing anything here, because most
of what looks like a detail below is a rule from that file made concrete. Phase status is
`docs/PLAN.md` § Phase 8 and § Phases 10–13.

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
| Verified in Firefox itself | ✅ 2026-08-30 — see "Verified in Firefox" below |
| Extension auth — `hunt_tokens` bearer, minted from the site | ✅ migration `0015` |
| Email producer — OA / interview / offer mail | ✅ 8d (`95f0443`), same table, gated on `is_pressing()` |
| Gmail OAuth, sync, classify, match | ✅ 8a–8b (`afe7c8c`), rules layer; **classifier checkpoint unmet** |
| Gmail label writes | ✅ 8c (`f911f46`) — **on by default**, `INBOX_APPLY_LABELS=false` opts out |
| Status proposals — every email-driven change is reviewable | ✅ 8c (`c976955`), `status_proposals` |
| Auto-apply above a confidence threshold | ◐ built, **off by default** — no threshold set until 13e |
| Unattended sync on an interval | ✅ Phase 9 (`c001445`), `INBOX_SYNC_INTERVAL_SECS`, `0` disables |
| Proposals review panel on the internships tab | ✅ Phase 9, `InboxPanel.tsx` |
| Inbox health visible in the extension popup | ✅ Phase 9, four distinct states |
| 8b's measurement harness — export, grade, ledger | ✅ `src/inbox/labelset.rs` (`c05e710`) |
| CV autofill on ATS pages | ✅ Greenhouse, Lever, Ashby — verified live 2026-08-30 |
| Answer library | ◐ 8g — built; retrieval + the popup's request contract verified 2026-08-31, browser half unverified |

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
| `subject_id` | What the event is about, and the idempotency key: a posting id, or a Gmail message id (8d). Polymorphic, so no `REFERENCES` clause is possible |
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

## `application_events` — the Phase 10 spine (spec for migration `0021`)

**Written 2026-09-02 as task 10c, before any code exists.** This is the contract task 10d builds
the migration from and task 10e wires every writer into. It is written to be buildable by an
agent that has read this section and the rules file and nothing else.

### Why the table exists

`internship_applications.status` is a mutable column, and the history of how it got there is
spread across `status_proposals`, `email_verdicts` and `hunt_events` — which is to say nowhere.
Every feature Phases 11–12 want (response rates, time-to-response, per-source conversion,
resume-variant attribution) is a query over transitions the app currently throws away. This
table keeps them.

### Columns

| Column | Type | Null | Notes |
|---|---|---|---|
| `id` | TEXT PK | no | UUID v4 text, as every table here |
| `application_id` | TEXT | no | `REFERENCES internship_applications (id)` — a real, enforced FK (see below) |
| `at` | TEXT | no | **When the transition happened**, RFC3339 UTC. From the causing email's `received_at`, the proposal's `created_at`, or `Utc::now()` for a live edit |
| `created_at` | TEXT | no | **When the row was written.** Differs from `at` for every backfilled row, and that difference is how you tell reconstructed history from observed history |
| `from_status` | TEXT | **yes** | NULL means *not known*, which is the honest value for the creation event and for backfilled rows whose prior state cannot be proved. Never invent it |
| `to_status` | TEXT | no | `CHECK (to_status IN ('applied','oa','interview','offer','rejected'))` — the same five as `internship_applications.status`, spelled the same way |
| `actor` | TEXT | no | `CHECK (actor IN ('email','extension','manual','sweep','unknown'))` |
| `cause_kind` | TEXT | yes | `'status_proposal'` \| `'email_verdict'` \| `'hunt_event'`, or NULL |
| `cause_id` | TEXT | yes | The id in that table. **Loose reference, no `REFERENCES` clause** — see below |
| `note` | TEXT | yes | Free text for a human-entered reason. Never parsed |

**Indexes, and only these two:**

```sql
CREATE INDEX idx_application_events_app ON application_events (application_id, at);
CREATE INDEX idx_application_events_at  ON application_events (at);
```

The first serves the fold and the per-application timeline; the second serves Phase 11's
date-windowed analytics. **No index on `actor`** — five values over a small table, and every
query that filters on it also filters a window, so it would be read past.

**Idempotency is structural, as it is for `hunt_events`:**

```sql
UNIQUE (application_id, cause_kind, cause_id, to_status)
```

`to_status` is in the key for one non-obvious reason: **rejecting a previously auto-applied
proposal writes a second event with the same cause** — the undo. Accept-then-undo is
`(app, 'status_proposal', p1, 'oa')` and `(app, 'status_proposal', p1, 'applied')`, which are
distinct, while replaying the same accept twice is not. Without `to_status` the undo silently
fails to record and the fold diverges from the column. NULL `cause_id` values are distinct to
SQLite, which is the behaviour we want: two manual edits are two events.

### `actor`, and exactly which code path produces each

| Actor | Produced by | Where |
|---|---|---|
| `email` | The auto-apply path — the classifier's verdict changing status with no human in the loop | `src/inbox/sync.rs:503` |
| `manual` | A human accepting or rejecting a proposal, and a status edit from the internships tab | `src/routes/inbox.rs:383`, `:407`; `src/routes/internships.rs:263` |
| `extension` | "Track this application" from the popup, which creates the application | `src/routes/internships.rs:191` |
| `sweep` | **No producer today.** Reserved for Phase 11's dead-application detection, which is the first thing that will change a status with no human and no email | — |
| `unknown` | The 10d backfill only. Never written by live code | — |

**Accepting a proposal is `manual`, not `email`.** The cause is an email and `cause_id` records
it; the *actor* is the person who clicked. Collapsing the two would make "how often do I accept
what the classifier proposes" — the number 13e needs to set a confidence threshold — impossible
to ask.

**`extension` is decided by the credential, not by a field in the request body.** The extension
authenticates with a `hunt_tokens` bearer and the site with a session cookie. The alternative —
the client asserting `source: "extension"` — is a claim by the party being described, and it is
wrong exactly when someone is debugging why it is wrong.

**Shipped 2026-09-02 as `routes::auth::Credential`**, not as the field on `CurrentUser` this
paragraph originally predicted: `CurrentUser` is a tuple struct destructured at 42 call sites
across 7 route files, and all 42 would have changed so that one handler could read one value.
`Credential` is its own extractor, so only the handler that cares mentions it. Resolution is
cached in the request extensions and shared with `MaybeUser`/`CurrentUser`, so a handler taking
both validates once and cannot get two different answers from the two extractors.
`Credential::actor()` is the mapping, in one place: `HuntToken → extension`, `Session → manual`,
`Anonymous → unknown`.

### `cause_id` is a loose reference, deliberately

It points into one of three tables depending on `cause_kind`, so no single `REFERENCES` clause
can express it — the same polymorphism `hunt_events.subject_id` already has, for the same
reason, and the reader resolves it by `cause_kind`.

This matters more here than it looks, because **foreign keys in this database really are
enforced**: `sqlx` turns `PRAGMA foreign_keys` on per connection, which was proved the hard way
by an insert-ordering bug in `internships::collector` that failed with `FOREIGN KEY constraint
failed`. Two consequences for 10d and 10e:

- `application_id` **is** a real FK and the event must be inserted *after* the application row
  exists. Writing events first is the collector's bug, one subsystem over.
- The `sqlite3` CLI does **not** enable the pragma, so a row hand-deleted at the prompt leaves
  a dangling `cause_id`. Readers must treat an unresolvable `cause_id` like a NULL one —
  `LEFT JOIN`, never `INNER` — which is the rule `routes/internships.rs` already documents for
  `posting_id`.

### The invariant: `status` stays, and the fold must agree with it

`internship_applications.status` is **not** dropped. It stays as a cache, because every existing
read path uses it and a derived-on-read status would put a fold in front of the tracker's
hottest query. The log is the truth; the column is a denormalization of it, and 10f pins them
together:

> **fold(events) — for one application, order its events by `at` ASC, then `created_at` ASC,
> then `id` ASC; the fold is the `to_status` of the last one. An application with no events
> folds to NULL and is exempt** (nothing has been recorded for it yet, which is only legal
> before the backfill has run).

Last-event-wins is correct here and does not contradict rule 3. Rule 3 governs what an email may
*propose*: `advance` refuses a backwards transition, so a late autoresponder never produces an
event at all. The log therefore contains only transitions that actually happened, and the latest
one is the current state.

The tie-breaks exist to make the fold **deterministic**, not to make it right — a tie on `at`
should not occur, because the backfill draws its timestamps from distinct sources. 10f's test
asserts determinism; it does not assert that a tie means anything.

**A mismatch is a writer that forgot to emit, and it should fail loudly.** That is the whole
value of keeping both.

### The backfill (10d), and the provenance rule

Three sources, in decreasing order of how much they can prove:

1. **`status_proposals` that were accepted or auto-applied** — fully provable. `at` =
   `created_at`, `from_status` / `to_status` from the row, `cause_kind = 'status_proposal'`,
   `cause_id` = the proposal id, `actor` = `email` when `applied_automatically = 1`, else
   `manual`. A rejected proposal that had been auto-applied also gets its undo event, at
   `reviewed_at`.
2. **`internship_applications.applied_at`** — the creation event for every application:
   `to_status = 'applied'`, `from_status = NULL`, `at = applied_at`, `actor = 'unknown'`. Two
   code paths create applications and neither left a record of which one ran.
3. **`internship_applications.status_changed_at`** — for an application whose current `status`
   is not explained by any event from (1) or (2), one event at `status_changed_at` with
   `to_status` = the current status, `from_status = NULL`, `actor = 'unknown'`.

**The provenance rule: a row whose origin cannot be proved is `unknown`, never `manual`.**
`manual` is a claim about a person having done something, and every chart in Phase 11 will
believe it. `unknown` is a fact about the record. There is no third option and no default.

Run order matters: (1) before (3), or (3) manufactures duplicates for transitions (1) already
explains. And the whole backfill runs in one transaction — a half-backfilled table fails the
10f invariant for reasons that have nothing to do with a writer.

### The insert helper — one function, called by every writer

```rust
// src/internships/application_events.rs

pub enum Actor { Email, Extension, Manual, Sweep, Unknown }

pub enum Cause<'a> {
    StatusProposal(&'a str),
    EmailVerdict(&'a str),
    HuntEvent(&'a str),
}

/// Everything a writer has to state. No `Default` impl: every field is a decision, and a
/// defaulted `actor` is the one mistake this table cannot survive.
pub struct NewApplicationEvent<'a> {
    pub application_id: &'a str,
    pub from_status: Option<ApplicationStatus>,
    pub to_status: ApplicationStatus,
    pub actor: Actor,
    pub cause: Option<Cause<'a>>,
    pub at: DateTime<Utc>,
    pub note: Option<&'a str>,
}

pub enum Recorded { Written, AlreadyRecorded }

/// Records a transition. `INSERT … ON CONFLICT DO NOTHING`; `AlreadyRecorded` is a normal
/// outcome, not an error — same contract as `hunt::events::emit`.
///
/// **Takes a transaction, not a pool, on purpose.** The status UPDATE and this INSERT must
/// land together or not at all: a committed status change with no event breaks the fold
/// invariant, and a committed event with no status change is a lie about the tracker.
///
/// Callers open that transaction with `db::begin_write` (`BEGIN IMMEDIATE`), never
/// `pool.begin()` — a deferred transaction that upgrades read→write fails instantly under a
/// competing writer instead of waiting. See `src/db.rs`.
pub async fn record(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    event: NewApplicationEvent<'_>,
) -> anyhow::Result<Recorded>;
```

### What 10e has to fix on the way in — the accept/reject path is not atomic today

Reading the writers to write this spec turned up a defect that matters more once events exist.
`routes::inbox::decide` (`src/routes/inbox.rs:376`) issues its `UPDATE internship_applications`
and its `UPDATE status_proposals` as **two separate statements outside any transaction**, and
the first is written `let _ = sqlx::query(…).execute(pool).await;` — the `Result` is discarded.
So a failed status update today leaves the proposal marked reviewed and accepted while the
tracker never moved, and nothing anywhere records that the two disagree.

With `application_events` in place the same failure also breaks the fold invariant, which is
how it would finally become visible — but 10e should not rely on that. **Wrapping the status
update, the proposal update and `record` in one transaction fixes all three at once**, and it is
the reason `record` takes a `&mut Transaction` rather than a pool. The same applies to
`internships.rs:263` and `sync.rs:503`.

### Three things 10e part 2 had to get right — and did

All three were predicted while writing part 1, and all three landed as predicted. Kept as a
record because each is a decision a future writer can undo by accident.

1. **`create_application` had no transaction** — a single INSERT. It has one now, with the
   event written *after* the application row, because `application_id` is a real enforced
   foreign key. The 409 on a duplicate application rolls back rather than leaving an orphan
   event behind.
2. **A rejected proposal that was never auto-applied emits nothing.** It changes no status, so
   there is no transition to record. Pinned by
   `rejecting_a_proposal_that_never_applied_records_nothing`.
3. **A manual event's NULL `cause_id` does not deduplicate**, and that is correct: two manual
   edits are two events. The key's idempotency covers email- and extension-caused writes, which
   is exactly where a producer can legitimately run twice.

A fourth, found while doing it: **accepting a proposal that was already auto-applied writes no
second event.** The email actor's row and the human's accept share
`(application_id, cause_kind, cause_id, to_status)`, so `record` returns `AlreadyRecorded` — a
normal outcome. The undo differs only in `to_status`, which is the whole reason that column is
in the key.

### Verified against the real database, 2026-09-02

Against a **copy of the live `fridge.db`**, not fixtures — the first time the fold invariant has
covered rows the application itself wrote rather than rows the backfill reconstructed:

| | covered | exempt | mismatches |
|---|---|---|---|
| after `application-events backfill` | 2 | 0 | **0** |
| after driving the real handlers | 3 | 0 | **0** |

The events on the two touched applications came out `unknown` → `manual` → `extension`: the
backfilled creation whose origin could not be proved, a live manual status move, and a new
application created on a hunt-token credential. Three actors, in the order they happened.

### What is deliberately not in this table

- **No `user_id`.** Ownership lives on `internship_applications`, every read scopes through it,
  and Phase 11's analytics join that table anyway for company, source and tier. A second copy of
  ownership is a second thing that can disagree — the instinct behind `VISIBLE_TO_VIEWER` being
  written once.
- **No email text, subject or snippet.** That is `email_messages`, reachable through
  `cause_id`. Copying it makes two places to redact and grows the hot table with facts nobody
  queries.
- **No confidence score.** It belongs to the verdict and is reachable the same way. Storing it
  here invites ranking events by a number that means nothing outside its verdict.
- **No `days_in_previous_status`.** Two timestamps subtract; a stored copy is a second thing
  that can disagree with them, and it would have to be rewritten by every later event.
- **No soft-delete, and no UPDATE path.** The table is append-only. A wrong event is corrected
  by a compensating event, which is what an audit trail *is*; an editable audit trail answers
  no question its own history cannot be doubted on.

### Two things 10d must not do

1. **Do not edit migration `0021` after it has run anywhere.** sqlx compares checksums and
   refuses to start with `migration 21 was previously applied but has been modified`. A
   correction is a new migration or a comment in the code — this is recorded in
   `routes/internships.rs:31`, which is where migration `0012`'s note had to move to.
2. **Do not take `0022`.** Lane A owns `0021–0029`; `0021` is this table and the rest are
   unassigned. Announce a number before using it.

### Handoff — 10e part 1 landed 2026-09-02, and what 10d can now assume

The two halves of 10e that do not need the table are done (`d0caf73`, `8cb4c10`). What changed
that `record()` depends on:

**Every status write now runs inside a transaction, so there is something to hang `record` on.**
None of them was transactional before, and one was actively losing writes:

| Call site | The transaction | What it was |
|---|---|---|
| `routes::inbox::decide` (`src/routes/inbox.rs:356`) | Opens at the top, commits after the proposal is marked reviewed | Two statements, and the application UPDATE's `Result` was discarded with `let _ =` — a failed status change left the proposal *reviewed and accepted* while the tracker never moved |
| `routes::internships::update_application` (`src/routes/internships.rs:262`) | Wraps the status and notes updates together | Two independent statements; half an edit could land and still return 200 |
| `inbox::sync::propose_status` (`src/inbox/sync.rs:486`) | Wraps the `status_proposals` INSERT with the auto-apply UPDATE | A proposal could survive a failed UPDATE while claiming `applied_automatically = 1`, which the panel renders as *"already applied — rejecting undoes it"* |

Both status updates now assert `rows_affected() == 1`. Three tests in `routes::inbox::decide_tests`
pin it, including one that forces a real database failure on the undo path.

**The credential is available as an extractor**, described above: `routes::auth::Credential`,
with `Credential::actor()` returning `"extension"` / `"manual"` / `"unknown"`. When 10d lands the
`Actor` enum, that method becomes a `From<Credential> for Actor` and the `&'static str` goes
away — it is deliberately the events table's vocabulary and not a second one.

**What 10d still does not need from 10e.** The backfill has no live credential to read and
records `unknown` for every creation event regardless, so nothing here blocks it.

**What is left of 10e** is the part that needs the table: adding `record(&mut tx, …)` inside the
three transactions above, plus `routes::internships::create_application`, which currently logs
its provenance rather than storing it (`internships.rs:225`). That is one commit once 10d
merges, and 10f's fold invariant then covers live writes as well as backfilled ones — so run it
again after that, not only after the backfill.

### Handoff — ready for 10d

The migration, the backfill and the helper above are fully specified; nothing in 10d needs a
decision this section does not make. Two things are **not** decided here, both on purpose:

- **Which writers exist is 10e's problem, not 10d's.** The actor table above lists the five call
  sites found on 2026-09-02 (`sync.rs:503`, `inbox.rs:383`/`:407`, `internships.rs:191`/`:263`).
  If 10e finds a sixth, it emits from it and adds a row here.
- **The `CurrentUser` credential field** that distinguishes `extension` from `manual` is 10e's
  change, in `routes/auth.rs`. 10d does not need it: the backfill has no live credential to read
  and records `unknown` for every creation event regardless.

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

## The inbox agent — `src/inbox/` (8a–8d, plus Phase 9)

Track A. Reads a burner Gmail, decides what each message is, matches it to an application, and
proposes what that means for the application's status. **The four email categories were already
in the database before this existed** — Phase 7's `internship_applications.status` is
`applied → oa → interview → offer → rejected` — so this never sorts mail into folders. It
matches mail to a row and proposes a transition; Gmail labels are written afterwards as a
*projection* of application status. Built the other way round you get two taxonomies that
drift.

### Modules

| Module | What it is |
|---|---|
| `oauth` | Connecting the account and keeping the access token fresh |
| `gmail` | The Gmail surface actually used. **Read-only by construction** — a test fails the build if a write call appears here |
| `labels` | The **only** module that modifies a mailbox. Adds labels; never removes one (including its own), never archives, never touches a disregarded message |
| `sync` | The pass: fetch, record, classify, count. Owns `inbox_runs`, and hosts both write decisions (`labelling_enabled`, `auto_apply_threshold`) |
| `classify` | The rules layer. A pure function: email in, a constrained enum out, no tools, no SQL — rule 1 |
| `advance` | Matching, and what an email may do to a status. Rules 2 and 3, pure, no database |
| `labelset` | 8b's measurement harness. Tooling, not pipeline — documented above |

### Tables (migration `0019`, plus `0020`)

`gmail_accounts`, `inbox_runs`, `email_messages`, `email_verdicts`, `status_proposals`; `0020`
adds `email_messages.labels_applied` and `labels_applied_at`, so *"which of my emails has this
touched"* is answerable — worth being able to ask about the first thing in this project that
changes someone else's account.

`email_verdicts` is written for **every** message including disregarded ones (rule 7), and kept
even when superseded, so a bad call is diagnosable rather than merely wrong — the same instinct
as `posting_rejects` one subsystem over. `sync` pins the invariant
`classified = pressing + confirmation + outreach + disregarded` (`sync.rs:62`): without it,
*"correctly ignored 400 newsletters"* and *"ate an OA"* produce identical output.

### Endpoints

```
GET  /auth/gmail/start                  CurrentUser -> consent redirect
GET  /auth/gmail/callback               state cookie checked BEFORE the jar is cleared
GET  /hunt/inbox/status                 connected account, last run, outcome
POST /hunt/inbox/sync                   run a pass now (the interval worker is the normal path)
POST /hunt/inbox/disconnect
GET  /hunt/proposals                    pending only, newest first, joined to the causing email
POST /hunt/proposals/{id}/accept        apply the status change, mark reviewed
POST /hunt/proposals/{id}/reject        mark reviewed; undoes an auto-applied change
```

### The three environment variables that decide how much it may do

| Variable | Default | Effect |
|---|---|---|
| `INBOX_SYNC_INTERVAL_SECS` | **900s when unset** | The unattended pass. Unset is *not* off — `DEFAULT_SYNC_INTERVAL_SECS` is 900. `0` disables deliberately; a non-number disables it and logs, which looks identical to a Gmail outage from outside. Spawned in `main.rs:93`, never called from a request handler |
| `INBOX_APPLY_LABELS` | **on** | `false` or `0` stops all mailbox writes without touching anything else (`sync.rs:354`) |
| `INBOX_AUTO_APPLY_CONFIDENCE` | **unset — nothing auto-applies** | A float in `0.0..=1.0`. Only forwards, and **never `offer` or `rejected` at any confidence** |

The last two are the whole write-access posture, and the first default is the one Phase 10's
task 10k asks you to confirm deliberately before the agent runs unattended on a deployed host.

### Phase 9 — the parts that made it usable without an operator

- **The interval worker** (`inbox::sync::spawn`), so a sync is not something you remember to
  POST. Spawned rather than awaited: a slow Gmail cannot delay startup.
- **The proposals panel** (`frontend/src/app/internships/InboxPanel.tsx`), showing the causing
  email beside each proposed transition. Rule 2's audit trail is worthless if the only way to
  read it is SQL.
- **The inbox line in the extension popup** (`popup.js:597`), reporting *no account* / *no sync
  yet* / *reconnected* / *failed, with the reason* as four distinct states. Rule 5 says a broken
  sync must be visible, and the Gmail token's 7-day expiry is the difference between noticing in
  an hour and noticing in a fortnight.

## The analytics contract (spec for Phase 11)

**Written 2026-09-02 as task 11a, before any of it is built**, so 11b (the endpoint), 11c (the
panel) and 11d (the second slice) can be picked up by either agent from this section alone.

**The analytics layer reads and never writes.** `GET /hunt/analytics` runs entirely over
`application_events` joined to `internship_applications`; it creates no table, no column and no
row. The one Phase 11 task that *does* write is 11e's nudge producer, which appends to
`hunt_events` and is specified at the end of this section — keeping those two facts apart is
what stops "analytics" quietly growing a writer.

### Why this needs the event log at all

The single question that justifies the whole of Phase 10: **an application that went
`applied → oa → rejected` did reach OA.** `internship_applications.status` says `rejected` and
cannot say anything else. Every conversion number below is computed from the *maximum stage
ever reached*, which only the log knows.

### The endpoint

```
GET /hunt/analytics?from=<RFC3339>&to=<RFC3339>&dead_after_days=<n>   CurrentUser
```

```jsonc
{
  "window": { "from": "…", "to": "…", "dead_after_days": 45 },
  "totals": {
    "applications": 128,
    "responded": 41,          // reached any stage past `applied`
    "no_response_live": 52,   // silent, younger than dead_after_days
    "no_response_dead": 35,   // silent, older than it
    "reached_oa": 24,
    "reached_interview": 12,
    "offers": 2,
    "rejected": 29            // an explicit rejection, which IS a response
  },
  "time_to_first_response_days": { "median": 6.0, "p90": 21.0, "n": 41 },
  "by_source":  [ { "key": "simplify",  "…totals…": {} } ],
  "by_tier":    [ { "key": "1",  "…totals…": {} }, { "key": "unknown", "…": {} } ],
  "by_month":   [ { "key": "2026-08", "…totals…": {} } ]
}
```

Every breakdown carries the **same totals shape** as the top level, so the panel renders one
component three times and a future breakdown costs nothing. `limit`-style paging is not needed:
the cardinality is bounded by sources (~8), tiers (4) and months.

### The definitions, which are the actual work

Each of these has a plausible wrong answer that makes the funnel flatter — that is, more
flattering — than reality.

**A RESPONSE is the first event after creation whose `to_status` is not `applied`, whatever the
actor.** Not "an email-driven transition": the employer replied whether the evidence arrived in
the burner inbox or you typed it in after a phone call, and scoring only what the classifier
caught would measure the classifier, not the hunt. The actor still matters — for *"how often do
I accept what the classifier proposes"*, which is 13e's question and a different query over the
same rows.

**A rejection is a response.** Counting only positive outcomes as responses would report a
response rate made of good news. `rejected` appears in both `responded` and `rejected`.

**NO RESPONSE is not REJECTED, and the two must never be summed.** A silent application is its
own bucket. Collapsing them is the standard way a job-hunt tracker lies in the flattering
direction: it converts *"nobody replied"* — which is information about your applications — into
*"they said no"*, which is information about employers.

**DEAD is derived, never stored: no response, and created more than `dead_after_days` ago.**
Default 45, and it is a query parameter rather than a constant because the honest value depends
on the season and nobody knows it yet. Dead is not a status, nothing writes it, and an
application that answers after 60 days simply stops being dead. **`dead` and `closed` are
different**: an application with a terminal status (`offer`/`rejected`) is closed and is never
counted as dead.

**CONVERTED is computed from the maximum stage ever reached, not the current status.**
`reached_oa` counts every application that was ever at `oa` or beyond, including those later
rejected. Two rates are reported and never multiplied together into one "success rate":
interview rate and offer rate. A single blended number hides which stage you are actually
losing at, which is the only thing this panel is for.

**An expired posting is not an ended application.** `internship_postings.expired_at` describes
the market; `internship_applications` is the record of what you did. The join to the posting is
`LEFT` and enrichment-only — an `INNER JOIN` here silently drops applications whose posting was
pruned, which is trap 1 in `routes/internships.rs` arriving through a new door.

### Cohort semantics — `from`/`to` filter the APPLICATION, not the event

The window is compared against `internship_applications.applied_at`. An application made in June
that reaches interview in September belongs to **June's** cohort in every metric, including
September's report.

Filtering on event time instead is the tempting mistake, and it produces a September row with
interviews in the numerator and no denominator — a conversion rate above 100%, or worse, one
just under it that looks plausible.

### Time to first response

Median and p90 of `(first response event.at − creation event.at)` in days, over applications
that responded. `n` is reported beside them so a median over four applications is visibly a
median over four applications.

**Median, not mean**: one company replying after seven months should not move the number that
tells you whether a week of silence is normal.

### Timezones — where the boundary is

Everything stored is UTC (RFC3339, as every timestamp in this schema). `from`/`to` are parsed as
UTC instants; a malformed value is a **400**, never a silent default — the same rule the
`?sort=` enum follows one tab over.

**`by_month` buckets in UTC**, and the frontend renders labels in local time. This is wrong by
up to a day for applications made late in the evening, which is accepted: the alternative is a
`tz` parameter threaded through every bucket, and month boundaries are not a decision anyone
makes an hour before midnight. If that ever matters, it is a parameter, not a rewrite.

### 11e's nudge producer — and the migration nobody has budgeted for

The trap `docs/PLAN.md` § Phase 11 names is real, and there is a second one under it.

**The key.** `hunt_events` has `UNIQUE (kind, subject_id)`, which is what makes alert dedup
structural rather than a caller remembering to check. A nudge at 14 days and a nudge at 30 days
for the same application are legitimately different events, so:

```
kind       = 'nudge'
subject_id = "{application_id}:{threshold_days}"
```

Keyed on the application alone you get one nudge ever; keyed on something that varies per sweep
you get one nudge per sweep, which is how a channel gets muted — taking the OA alerts with it.

**The cost nobody has counted:** `kind` is `CHECK (kind IN ('posting', 'email'))`, and **SQLite
cannot alter a CHECK constraint**. Adding `'nudge'` means a full table rebuild — create, copy,
drop, rename — on the alert channel itself. That migration must preserve `acked_at` exactly:
losing it re-raises every historical alert at once, which is the single failure the entire ack
design exists to prevent. Take a backup first (`ops/backup-fridge-db.sh`), and pin the
before/after row counts and the `acked_at` non-null count in the migration's test.

Do **not** dodge this by reusing an existing kind. The extension filters alert kinds on `kind`
and the options page has a checkbox per kind; a nudge filed under `'email'` gets muted along
with cold outreach, which is exactly backwards.

**A kind the extension has never heard of defaults to ENABLED.** Stored settings from an older
install have no checkbox for `nudge`, and the safe default for a *notifier* is to notify: a
producer that ships and silently raises nothing is the failure mode this project keeps
re-learning, and one unwanted notification is cheaper than a missed OA.

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
| `poll` | Fetches, raises, records status. **Never throws** — the callers are an alarm and a button. Hands `raise` *every* event, including kinds the user switched off: those are still delivered and must still be acked |
| `raise` | Notifies only events **not already notified** and of an enabled kind — up to `maxNotificationsPerPoll` (default 3) individually, one summary for the rest — then caches and acks *everything received*. Returns how many notifications it raised |
| `loadNotified` / `rememberNotified` | `notifiedEventIds` in `storage.local`, newest-first and capped at 300. Recorded **before** the ack: a failed ack still re-offers the event, but it has been shown once and must not be shown again |
| `notify` | The **notification id is the event id**, so a re-notify after a failed ack replaces rather than stacks |
| `ack` | Failure leaves the event unacked on purpose: it is offered again next poll |
| `setStatus` | Records one of `ok` / `unreachable` / `unauthenticated` / `error` |
| `cacheForPopup` | Last 50 events, so the popup is never blank and a notification click can still resolve its URL after the page has been killed |
| `bumpUnseen` / `paintBadge` | Badge counts alerts raised since the popup was last opened; `!` in red for any non-`ok` status |
| `ensureAlarm` | Creates the alarm **only if absent or at the wrong period**. `alarms.create` on an existing name resets the countdown, so calling it on every wake would push the next poll further out each time and polling would quietly stop |

Every listener is registered synchronously at the top level, or the event meant to wake the
page arrives before anything is listening.

**Notify, then ack — in that order.** At-least-once for *delivery*, deliberately: acking first
would turn a failure into a silently dropped alert.

**"Unacked" is not "new", and only new is worth a notification.** A poll runs on the alarm, on
browser startup, on Settings being saved and on the popup's *Check now* button, and each one
used to re-raise the whole unacked backlog — pressing *Check now* notified you about the alerts
you had the popup open to read, and any spell where acks could not land (backend down, token
expired, Firefox closed for a day) arrived as a pile of notifications for hours-old events.
`notifiedEventIds` makes a notification once-per-event; losing that cache costs one repeat,
which is the same bargain delivery already makes.

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

**Everything in this section is the superseded design, written in the present tense it was
written in.** It is kept because the four failure modes below are still the four failure modes,
and three of the four fixes are still live. Only the cookie half was replaced — by
`hunt_tokens`, documented in the section above. Read this as history; read that as fact.

The plan was: `host_permissions` for the backend origin plus `fetch(..., { credentials:
"include" })` puts the ordinary `fridge_session` cookie on the request. Signed in on the site
means signed in in the extension. ~~There is no extension token and no second auth path.~~
There is; that sentence is exactly what stopped being true.

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

**That open question is answered: Firefox does not attach it.** `fridge_session` is
`SameSite=Lax`, a request from a `moz-extension://` page is cross-site, and the backend
therefore answered a truthful 401 to a signed-in user. `hunt_tokens` is the recorded fallback,
and the failure-state list above gained `no-token` and `token-rejected` — which is why two
tables in this file name overlapping-but-different states: this one is the cookie era, the
later one is current.

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

## The answer library — `src/hunt/answers.rs` (8g)

Questions you have already answered well, offered back when a form asks something close enough.

**Tables** (migration `0018`): `application_answers` and `answer_revisions` — an edit keeps the
prior text rather than overwriting it, because the reason to store an answer is that it was
good once.

```
GET    /hunt/answers                    list
POST   /hunt/answers                    save (question, answer, company)
PATCH  /hunt/answers/{id}               edit — the old text becomes a revision
DELETE /hunt/answers/{id}
GET    /hunt/answers/{id}/revisions
POST   /hunt/answers/{id}/used          usage, so "still good" is observable
GET    /hunt/answers?q=…&company=…      retrieval — the shape `popup.js` actually builds
```

| Piece | What it does |
|---|---|
| `normalize_question` | Retrieval compares normalized question text, not raw markup |
| `MIN_SIMILARITY` = `0.45` | Below this nothing is offered **at all**. A weak match is worse than none: it invites a glance rather than a read, and the failure mode here is pasting something that looked close enough |
| `suggest` | `strsim::normalized_damerau_levenshtein`, chosen over embeddings because the corpus is tiny and the dependency already exists |
| `detect_company_specific` | "Why do you want to work *here*" is not reusable; "a project you're proud of" is. Marked at save time from question markers, the answer text, and the company |
| `MAX_QUESTION_LENGTH` / `MAX_ANSWER_LENGTH` | 2,000 / 20,000. Unbounded client text is a denial-of-service vector, not a feature — the blog's body cap, one subsystem over |

**The seam is tested with the exact requests the popup builds** (`f17d983`). The extension and
the routes meet in two languages with no compiler between them, and a renamed query parameter
degrades to *no suggestions*, which is indistinguishable from an empty library. So
`routes::hunt::answer_loop_tests` drives the real handlers with `popup.js`'s own `?q=` /
`&company=` query string and its three-field save body, and asserts **both** directions: the
company-specific answer is withheld from another company, and the reusable one is offered. A
mutation that breaks the offering shows the withholding assertion still passing under it, which
is why one direction alone would not have bitten.

**What is still unverified is everything inside the browser**: whether `questions()` finds the
free-text boxes on a real ATS form, whether `describePage()` names the employer, and whether
Save and Suggest behave against a live page. That is task 12g, and it needs Firefox — jsdom
would be a new dependency in a folder that is deliberately plain JS with no build step.

## Seams — what was left for the rest of Phase 8, and what is left now

- ~~**8d writes to `hunt_events`, it does not alter it.**~~ **Done, and the prediction held.**
  `kind = 'email'`, `user_id` set, `subject_id` = the Gmail message id, emitted from
  `sync.rs:222`. Nothing about the poll, the ack or the extension's notification path changed —
  which is the two-producer shape paying for itself.
- ~~**The `email` alert kind has no producer.**~~ It has one, behind
  `verdict.category.is_pressing()` (`sync.rs:245`): an OA, an interview or an offer interrupts
  you; a confirmation or cold outreach is labelled and recorded and does not.
- **The migration numbers here were wrong** and contradicted this file's own prose in two
  other places. Corrected 2026-09-02: `0014` `hunt_events`, `0015` `hunt_tokens`, `0017`
  `cv_profile`, `0018` the answer library (`application_answers`, `answer_revisions`), `0019`
  the inbox tables (`gmail_accounts`, `inbox_runs`, `email_messages`, `email_verdicts`,
  `status_proposals`), `0020` `email_messages.labels_applied{,_at}`. (`0016` is
  `blog_post_source_path` — a different tab entirely.) `hunt_events` took `0014` because 8e
  needed it first, which is the only part of the original bullet that was true.
- **The extension shows no internship list**, only alerts and a link out. Whether it should is
  an open question in `apps/hunt-extension/CLAUDE.md`.
- **8b's checkpoint is waiting on mail, not on code** — still true, and now scheduled.
  `src/inbox/labelset.rs` is the harness; the burner held 14 messages over 2 days with zero
  digests when it was written, and the six held-out messages that could be graded were spent by
  being fixed against. Phase 10's deploy is what starts the corpus accumulating unattended, and
  Phase 13 is where it gets labelled and graded. See `docs/PLAN.md` § Phases 10–13.
