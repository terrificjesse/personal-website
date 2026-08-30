# Internship tab — reference

Phase 7, complete 2026-08-20: collect open SWE internship postings from several sources,
normalize and dedup them, rank them, track applications, and drop postings once they close.

This is the **what and where**. Design *rules* live in the root `CLAUDE.md` § Scraping rules
and in `apps/fridge-app/CLAUDE.md`; the per-source field research is
`docs/INTERNSHIP_SCRAPING.md` — **read that before touching any adapter**; phase status and the
verification record are `docs/PLAN.md` § Phase 7. Every module here carries a doc comment that
is longer and more specific than this file; when they disagree, the module wins.

**Not a Learning Mode area — and that includes the ranking.** The user decided this explicitly
on 2026-08-20. `rank_postings` is a scoring-and-ordering algorithm and so looks exactly like
the `[learn]` work in Phases 2–4; it is nonetheless `[gen]`. The one thing still reserved is
**dedup's fuzzy company/title matching**, which is NLP-shaped — `dedup::FuzzyMatcher` is a
declared trait with no implementation, and writing one is not covered by the exception.

## Feature overview

| Feature | State |
|---|---|
| Source adapters behind one trait | ✅ 9 registered, adding one touches no other file |
| Per-source failure isolation | ✅ a source cannot fail the run or reduce what others produced |
| Run record — every outcome visible | ✅ `collection_runs` + `source_runs` |
| QC with `fetched = accepted + filtered + rejected` | ✅ total function, rejects stored with `raw_json` |
| Dedup across sources | ✅ exact keys; fuzzy is an unimplemented seam |
| Composite ranking + single-axis sorts | ✅ `rank_postings` |
| Hard filters — term, location, class year, pay, company, source | ✅ filters exclude, never score |
| Applied tracker | ✅ per-user, snapshotted so it survives its posting |
| Expiry sweep — deadline + disappearance | ✅ only a complete enumeration may expire anything |
| Run-health panel | ✅ `/internships/runs` |
| Alerts on new tier-1/2 postings | ✅ Phase 8e — see `docs/HUNT.md` |

## The two traps that shaped the schema

Both were named before the migration was written, and each has a table that exists for no other
reason.

**1. An applied posting must survive expiry.** `internship_applications` snapshots
company/title/URL/pay/term at apply time, and `posting_id` is *enrichment*, never a dependency.
The rule, stated so it can be tested: **the tracker renders correctly from
`internship_applications` alone, with zero joins.** Both mechanisms are needed — soft-delete
alone fails because a hard delete leaves `posting_id` dangling, and snapshot alone drifts while
the posting is live.

**2. Disappearance is not closure.** A source that is blocked, rate-limited or reshaped makes
its postings look closed. Only a run that *actually succeeded* may count a posting as vanished.
This is the single most likely data-loss bug in the phase, and the defence is structural — see
Expiry below.

## Data model — migration `0012_create_internships.sql`

Seven tables. Timestamps are RFC3339 TEXT, ids are TEXT UUIDs, matching every other table here.

| Table | Purpose |
|---|---|
| `collection_runs` | One row per pass across all sources. A row with no `finished_at` means the process died mid-run |
| `source_runs` | One row per (run, source). A source **cannot participate without writing one**, so a source that silently returned nothing is distinguishable from one that genuinely had nothing |
| `internship_postings` | One row per real-world job, however many sources carry it. `dedup_key` is `UNIQUE` |
| `posting_sightings` | One row per (posting, source). **Trap 2 lives here** |
| `posting_rejects` | Rows QC did not accept, with `raw_json` so a reject can be diagnosed rather than merely counted |
| `internship_applications` | The applied tracker. **Trap 1 lives here** |
| `company_signals` | Derived per-company inputs and the prestige output, stored rather than computed inline so a score is inspectable |

Migration `0013_mark_interrupted_runs.sql` adds `collection_runs.interrupted` and backfills
every unfinished row. One interrupted run otherwise poisons everything downstream: it reports
as permanently in flight, the UI hides "Collect now" behind its progress banner, and the
startup check reads the phantom as a recent run and declines to collect.

### `source_runs.outcome` — four values, not two

"Did it work" is genuinely four-valued, and collapsing it is how a paginated fetch that dies
early reports healthy while 80% of its postings appear to vanish at once.

| outcome | the claim it makes | may absence expire a posting? |
|---|---|---|
| `success` | I enumerated **everything** this source currently offers | yes |
| `partial` | I got some of it and stopped | no |
| `failed` | nothing usable — blocked, rate-limited, reshaped, timed out | no |
| `skipped` | deliberately not fetched: `robots.txt`, or disabled | no |

`counts_for_expiry` on the row is what enforces that, and `fetched`/`accepted`/`filtered`/
`rejected` are separate counts: 3,000 filtered non-internship rows is healthy bulk, while 14
rows that should have parsed and didn't is a defect. Summed into one number the defect is
invisible.

### `internship_postings` — where "absent is not zero" is enforced in SQL

CHECK constraints, not conventions: `pay_min` requires both `pay_currency` and `pay_period`
(an amount with no currency is not a comparable quantity), `pay_max >= pay_min`, `pay_min >= 0`,
`is_remote IN (0,1)` and nullable (**`NULL` is unknown, a third state, not onsite**), and
`(expired_at IS NULL) = (expiry_reason IS NULL)` so the half-expired state is unrepresentable.
`posted_at_is_estimated` sits beside `posted_at` because a date we inferred and a date the
source stated are different evidence.

## Sources

`registry()` in `src/internships/sources/mod.rs`. Adding a source is one file plus one line
there; nothing else in the pipeline changes.

| Adapter | Class | Notes |
|---|---|---|
| `simplify` (`SimplifySource::simplify_jobs`) | GitHub list | One conditional GET, ~1,900 active listings, term + degree + an explicit `active` closure flag. Supplies the bulk of the corpus |
| `vanshb03` (`SimplifySource::vanshb03`) | GitHub list | ~29% URL overlap with Simplify, so ~285 unique |
| `ashby` | ATS JSON | Best pay data anywhere — explicit interval — and `employmentType == "Intern"` |
| `greenhouse` | ATS JSON | Largest slug count after Workday; pay via `pay_transparency=true` |
| `lever` | ATS JSON | Structured `salaryRange`, whole board per request |
| `weworkremotely` | RSS | Cheap, but **truncated to 25 items**, so it can never be a complete enumeration and is never `Success` |
| `linkedin`, `indeed`, `handshake` | best-effort | **Not built**, and deliberately so. They record `Skipped` with an honest `robots.txt` reason and make **zero requests**. Their absence is a recorded fact rather than something that looks like a bug |

Three mechanisms enforce isolation, in increasing order of paranoia: `Source::fetch` **cannot
return an error** (its return type is always a recorded outcome, so there is no `?` to
propagate); every adapter runs in its own `tokio` task, so a **panic** comes back as `Failed`
rather than taking down the run; and nothing in `sources` touches the database — it returns
values and the coordinator persists them.

## Backend files and functions

### `src/internships/models.rs` — the contract

The only place the three halves of the pipeline agree on a shape. Enforces "absent is not zero"
in the **type system**, which is stronger than the CHECK constraints because it fails at compile
time: pay is `Option<PayRange>` rather than four loose `Option`s (you cannot construct half a
pay figure), `Location::is_remote` is `Option<bool>`, and `RawPosting` → `QcOutcome` → `Posting`
is the only path in.

### `src/internships/http.rs` — the polite-fetch layer

**Every adapter goes through it and no adapter builds its own client.** Politeness is a property
of the process: a per-host rate limit only limits anything if every request queues behind the
same limiter. The `reqwest::Client` is private to the module, and
`sources::adapters_do_not_build_their_own_http_client` **reads the adapter sources at compile
time and fails the build** if any of them so much as names `reqwest`.

- Identifies honestly in `USER_AGENT`. No fingerprint spoofing, no proxy rotation, no CAPTCHA
  solving, no cookie jar — nothing here is *capable* of pretending to be something else.
- `robots.txt` fetched once per host, cached for the process, consulted before every request.
  A disallowed path yields `RobotsDisallowed`, which the caller records as `Skipped` — a source
  we correctly declined to fetch is not a broken source.
- **Fail fast: one attempt, no retry, no backoff loop anywhere in the file.**
- Conditional GETs: `ETag`/`Last-Modified` replayed automatically, because
  `raw.githubusercontent.com` answers `If-None-Match` with 304 and 0 bytes against 10.8 MB.
- `final_url` is always recorded, because a **dead** Greenhouse job's public URL redirects to
  the board root with **HTTP 200** — liveness by status code concludes every dead posting is
  alive, forever, with no error to alert on.

### `src/internships/normalize.rs` — QC

`normalize` is **total**: every input returns exactly one `QcOutcome`, so
`fetched = accepted + filtered + rejected` holds by construction and there is no path that
returns nothing.

`Filtered` vs `Rejected` is load-bearing. Filtered means correctly excluded (not an internship,
not software, term long past) and is expected in bulk. Rejected means *should have been usable
and wasn't* — every one is a potential bug. Reason codes are stable and machine-readable
(`REASON_NOT_AN_INTERNSHIP`, `REASON_MISSING_COMPANY`, `REASON_INVALID_URL`, …); changing one is
a breaking change to `posting_rejects.reason`.

**Unparseable pay is not a rejection.** Only fields that identify the listing at all can reject
a row; a salary we could not parse is kept in `pay_raw`, which keeps "we couldn't parse it"
distinct from "there wasn't one".

### `src/internships/dedup.rs` — the merge key

| Function | What it does |
|---|---|
| `canonical_url` | Strips the noise that silently breaks the join: Lever's `/apply` (579 records), Ashby's `/application`, `?gh_jid=` (544), `?mobile=` (574), `?ats=` (339) |
| `ats_identity` | Recovers `(ats, board_slug, job_id)` from the URL. **Greenhouse, Lever and Ashby only** |
| `title_key`, `company_key` | Normalized forms for the fallback |
| `dedup_key` | Primary: the ATS triple. Fallback: `(company_key, title_key)` |

**Location is deliberately not in either key.** 1,409 Simplify records list more than one
location (max 52); if one source explodes a posting per location and another does not, a
location-bearing key double-counts the job. Dropping it fails toward merging, the direction we
want.

`FuzzyMatcher` is a trait with no implementation, so dedup is exact-key only, which
**under-merges** — the safer failure. `KLA` and `KLA Corporation` remain two postings.

### `src/internships/prestige.rs` — the company signal

Half curated, half derived, and the split was forced by measurement. The only derived signal
available scored **60 of 455 companies, every one of them exactly 1.0**; posting volume ranks
Tesla and TikTok above Google; and exactly 1 company of 455 had a pay figure. So the top band is
stated outright in `data/internships/company-tiers.json` (44 companies over three tiers) and
everything else is inferred.

| band | range |
|---|---|
| curated tier 1 / 2 / 3 | 1.00 / 0.88 / 0.78 |
| derived | 0.35–0.65, centred on the 0.5 `rank` substitutes for unknown |
| no evidence | **`None` — unknown, never zero** |

The bands do not overlap, and that is checked at compile time (`const _: () = assert!(...)`)
rather than by a test. The derived band is centred on 0.5 so that a company we know a little
about cannot score *below* one we know nothing about.

### `src/internships/rank.rs` — filtering and ranking

**Pure**: no SQL, no handlers. Three passes, separate on purpose.

1. **Hard filters exclude, never score.** Enforced structurally: `score_posting` does not
   receive `InternshipFilters` and therefore *cannot* read one. Filtering only deletes elements,
   so survivors keep the order they had in an unfiltered ranking — pinned by a test.
2. **Scoring** — a weighted composite over pay, posted date, deadline, location and prestige,
   each on its own `0.0..=1.0` scale. `RankedPosting::score` is by construction the sum of the
   contributions in `RankedPosting::breakdown`, so a composite is always decomposable.
3. **Ordering** — every posting is scored identically whatever the sort; only the order changes.

**Absent data is imputed to the exact midpoint of its own scale, never to zero and never to
best**, and `ScoreBasis` reports which happened per input per posting. Two entries earn their
reasoning:

- An **estimated `posted_at` decays normally but is capped at neutral.** The estimate is a
  *lower bound* on age, so freshness derived from it is an *upper bound* — taking it at face
  value is what makes an entire cold-start corpus read as "posted today".
- **Weights are fixed and never renormalized over the present inputs.** Renormalizing sounds
  more principled and is the worse failure: a posting with one known input and five unknowns
  has its whole score set by that one input, so anything with a single strong signal floats to
  the top — the "unknown ranks best" trap, arrived at while fixing "unknown ranks zero".

**Under a single-axis sort, unknowns are not imputed — they sort last, in every direction.**
Not a contradiction: the composite answers "how good is this overall", where refusing to guess
lets one missing field decide everything; a single-axis sort answers "order these by their
actual pay", where imputation is a lie with a number attached. A passed deadline is a third
case — known but not actionable — and sorts after every open deadline and before the unknowns.

`OnUnknown` has **no `Default`**, deliberately: a caller must say whether a filter keeps or
drops unknowns, because the silent default is the bug. This matters most for class year, which
§ B measured as absent from *every* source — so the policy decides the whole corpus rather than
an edge case.

### `src/internships/expiry.rs` — the sweep

The safety property is the **split between two functions**, not a condition inside one:

- `settle_source_run` is the **only** place `consecutive_misses` is ever advanced, and it
  advances nothing unless the run earned it.
- `sweep` reads counters and deadlines and **never looks at `source_runs` at all**.

So the sweep cannot get the successful-run rule wrong, because the sweep does not implement it.
There is no `AND outcome = 'success'` for a future edit to drop. A posting expires by
disappearance only when **every** one of its sightings has crossed `INTERNSHIP_MISS_THRESHOLD`
(default 3) — putting the counter on the posting row instead would let one source's outage
expire postings three other sources are still serving.

### `src/internships/collector.rs` — the coordinator

Fetch, QC, dedup, persist, settle, sweep. Everything that could lose data lives here, in one
readable place. Two rules that are easy to get subtly wrong:

- **`seen_external_ids` is every id the source returned, not every id that survived QC.** The
  miss counter answers "does the source still list this?", a question about the *source*. If our
  own parser rejects a row we already track, it has still been seen — counting it as missing
  lets our defect expire real postings.
- **An explicit closure flag does not need a complete enumeration; an absence does.** Simplify's
  `active: false` is a positive statement about a specific record, valid even from a `Partial`
  run. `mark_closed` still refuses to close a posting **another source still lists**.

Also here: `reconcile_interrupted_runs` (a run cannot outlive its process, so at startup every
unfinished run is dead by definition — reconciliation, not a timeout), `recompute_company_signals`,
and `emit_posting_alert` (Phase 8e — `docs/HUNT.md`).

### `src/internships/store.rs`

A translation layer rather than `#[derive(FromRow)]`, because the database cannot express the
grouping that makes "absent is not zero" a compile-time property. `PostingRow::into_posting`
refuses to build a `PayRange` without both currency and period, matching the CHECK constraints
rather than trusting them.

### `src/routes/internships.rs`

```
GET    /internships                        ranked + filtered list        CurrentUser
GET    /internships/sources                source names for the filter   CurrentUser
GET    /internships/runs                   run-health panel              CurrentUser
GET    /internships/runs/{id}/rejects      what a run threw away         CurrentUser
POST   /internships/collect                trigger a run                 RequireAdmin
GET    /internships/applications           the tracker                   CurrentUser
POST   /internships/applications
PATCH  /internships/applications/{id}
DELETE /internships/applications/{id}
```

`GET /internships` takes `sort`, `term_season`, `term_year`, `remote`, `location`, `class_year`,
`pay_min`, `pay_max`, `company`, `source`, and a `*_unknown` policy per filter. An unrecognized
policy value is a **400**, not a silent fallback — same reasoning as the blog's `?sort=oldset`.
The response carries `total_live` alongside `returned`, so the UI can say "12 of 1,881" rather
than leaving an empty list ambiguous between "nothing matched" and "nothing collected yet".

`source` is applied **in SQL against `posting_sightings`**, not in `rank`: a deduped posting can
be carried by several sources, so "which source" is a property of the sighting.

The tracker's read path uses a `LEFT JOIN` and a `CASE WHEN p.id IS NULL` — an `INNER JOIN`
would drop the application once the posting is gone (trap 1 by the back door), and
`p.expired_at IS NULL` alone would report a **vanished** posting as live, since `NULL IS NULL`
is true. `posting_is_live` is three-valued and a `None` must never render as "Closed".

## Frontend — `frontend/src/app/internships/`

| File | What it is |
|---|---|
| `page.tsx` | The ranked list: sort selector, filter controls, `PostingCard`s, "I applied" |
| `PostingCard.tsx` | One posting with its score breakdown |
| `CollectionStatus.tsx` | Shared progress banner, so an empty list can say a run is in flight |
| `applications/page.tsx` | The applied tracker: status transitions, notes, delete |
| `runs/page.tsx` | Run health — per-source outcomes, counts, `counts_for_expiry`, errors |

## Environment

| Variable | Effect |
|---|---|
| `INTERNSHIP_COLLECT_INTERVAL_SECS` | Collection cadence, default 21600 (6h). `0` disables scheduled collection |
| `INTERNSHIP_EXPIRY_INTERVAL_SECS` | Sweep cadence, default 3600. `0` disables |
| `INTERNSHIP_MISS_THRESHOLD` | Consecutive expiry-eligible misses before a sighting counts as gone, default 3 |
| `INTERNSHIP_MAX_BOARDS_PER_RUN` | Safety valve with a real cost — see below |
| `INTERNSHIP_DISABLED_SOURCES` | Comma-separated. A disabled source still writes a `Skipped` run record |
| `INTERNSHIP_REJECT_RETENTION_RUNS` | Collection runs whose `filtered` reject payloads are kept, default 3. `0` disables |
| `INTERNSHIP_REJECT_SAMPLES_PER_REASON` | Specimens kept per (source run, reason) inside that window, default 20. `0` disables |

**`INTERNSHIP_MAX_BOARDS_PER_RUN` makes expiry stop working.** The vendored directory holds
~2,084 board slugs; uncapped, a run is on the order of half an hour of continuous requests to
other people's servers. But a capped run **has not enumerated the source**, so it reports
`Partial`, and a `Partial` run can never expire anything. Convenient for development, wrong for
steady state.

All are documented in `.env.example` as of 2026-08-30 — they had been missing, despite
`apps/fridge-app/CLAUDE.md` saying that file documents every env var.

### Reject retention — added 2026-08-30

`posting_rejects` stores `raw_json` so a discarded row can be **diagnosed** rather than merely
counted. That argument is about `kind = 'rejected'` — the rows that should have parsed and did
not, each a potential defect. `kind = 'filtered'` is the opposite: correct exclusion, expected
in bulk, explicitly not a health signal.

With no retention policy, the bulk category ate the database. Measured on the real file:
**90,040 filtered rows averaging 8 KB of `raw_json` — 785 MB, 96% of an 801 MB database, and
zero `rejected` rows among them.** Everything the tab actually serves came to under a megabyte.

Two rules, applied at the end of every run by `collector::prune_rejects`:

1. **`rejected` payloads are never pruned, at any age.** They are the whole reason the table
   exists and they are rare.
2. **`filtered` payloads are kept for the last N runs, capped at M specimens per
   (source run, reason).** The window alone bounds nothing — one uncapped run filters tens of
   thousands of rows, and three of those was still 383 MB. The per-reason cap is what makes
   the table stop growing: twenty examples answer "what got filtered as `not_software`" as
   well as twenty thousand do.

**The counts are never touched.** `fetched = accepted + filtered + rejected` lives on
`source_runs`, and pruning removes evidence, never accounting — a run from last year still
reports exactly how many rows it filtered. `GET /internships/runs/{id}/rejects` returns
`filtered_count` and `rejected_count` alongside the specimens, plus `payloads_pruned`, so an
empty list can never be misread as "this run filtered nothing" — which is the exact ambiguity
the table was built to prevent, and would otherwise have been reintroduced by the housekeeping
that keeps it from eating the disk.

Result on the real database: **801 MB → 4.5 MB**, 407 specimens across 24 (run, reason) pairs,
with `source_runs` still reporting all 85,716 filtered rows.

## Verification (2026-08-20)

Against a **real collection run**, not fixtures: 2,746 rows from five sources in 22 seconds,
capped at 6 boards per source. Full detail is in `docs/PLAN.md` § Phase 7.

- **The others still land when a source doesn't.** LinkedIn and Indeed recorded `skipped` with
  honest `robots.txt` reasons and made zero requests; three ATS sources recorded `partial`;
  Simplify still returned 924 accepted postings. ✅
- **A posting in two sources appears once** — and the key also merged **65 postings one source
  had exploded per-location** (RTX 14, American Express 8, TikTok 7). ✅
- **Every fetched row is accounted for.** 2,746 = 926 + 1,820 + 0. **Zero rejected.** ✅
- **A posting with unknown pay is neither first nor last** — ranks 247 and 634 of 808, while
  both first and last place are unknown-pay postings. ✅
- **An applied posting survives expiry** through expiry *and* hard deletion, with
  `posting_is_live` going `true` → `false` → `null` and `null` rendering no badge at all. ✅
- **A failed run does not expire that source's postings.** ⚠️ **Verified by test, not live** —
  no source hard-failed during the real run.

### What the real run caught that 510 green tests could not

Both dedup bugs, both **over-merging** — the dangerous direction, since without an ATS key two
distinct jobs at one company sharing a title collapse into one row.

1. `job-boards.eu.greenhouse.io` is a **third** Greenhouse hostname, absent from the research
   doc, which lists only `boards.` and `job-boards.`.
2. `boards.greenhouse.io/embed/job_app?token=N` puts the job id **in the query string** — which
   every other URL shape treats as strippable tracking noise.

ATS-triple coverage went 266/804 → 285/808.

## Known gaps, recorded rather than fixed

- **ATS-triple coverage is ~35%, where the research predicted 73%.** The shortfall is entirely
  platforms `ats_identity` does not parse: **Workday** (`*.myworkdayjobs.com`, in the research
  table and never implemented), plus `apply.workable.com` and `ats.rippling.com`. Everything
  else in the fallback is a company's own careers page, which correctly has no ATS identity.
  **Note for Phase 8f:** `apps/hunt-extension/CLAUDE.md` says to reuse this host list for
  autofill and names seven hosts — the code parses **three platforms**. Take the list from the
  code, and adding a host should stay a one-place change.
- **Fuzzy company/title matching is unimplemented** (`dedup::FuzzyMatcher`), so `KLA` /
  `KLA Corporation` remain two postings.
- Pay coverage in the verified run was 2 of 808 — an artefact of the board cap, not a defect.
  An uncapped run is the only honest way to measure it.
- **A capped run can never expire anything**, by construction: the cap makes a source report
  `Partial`, and `Partial` may not advance disappearance counters. Steady state needs at least
  one uncapped run for expiry to function at all.
- The frontend's filters are **not URL-synced**, so a filtered view cannot be bookmarked. The
  backend accepts every parameter; only the page ignores them.
- `src/internships/audit.rs` holds failing tests from a 2026-08-22 adversarial audit — findings
  as executable evidence, not fixes. **It is being worked through as of 2026-08-29**, so read
  the module rather than any summary of it, and delete it once the findings are triaged.
