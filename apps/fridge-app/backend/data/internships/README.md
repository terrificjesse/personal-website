# Internship source data

Vendored snapshots backing `src/internships/sources/` (Phase 7). Same pattern as
`data/themealdb/` and `data/foodkeeper/`: fetch once, commit the snapshot, document when it was
taken — **no live API calls at test time, and none from a request handler ever**.

Two different kinds of file live here and they are not interchangeable:

- **`board-slugs.json`** is *operational data*. It is compiled into the binary with
  `include_str!` and is what the Greenhouse / Lever / Ashby adapters actually poll.
- **`fixtures/`** are *test data*. They exist so the whole parsing layer runs offline. A test
  suite that needs the network is a test suite that fails in CI and teaches you nothing.

## Provenance

Everything below was fetched on **2026-08-20**, over HTTPS, unauthenticated, one request per
URL, with this user agent:

```
personal-website-internship-collector/0.1 (+https://github.com/terrificjesse/personal-website; contact via GitHub)
```

`robots.txt` was fetched and checked for every host before any other request. **Nothing was
fetched from LinkedIn or Indeed, at all** — see the "Not fetched" section.

### `board-slugs.json` — the board directory

Derived, not downloaded. Source: one GET of

```
https://raw.githubusercontent.com/SimplifyJobs/Summer2027-Internships/dev/.github/scripts/listings.json
```

which returned **10,826,186 bytes**, `ETag:
"7b225bc0360fcac16e5ac0ce6466482804aa49125669ad3f854f5fe8d68c7f97"`, `Cache-Control: max-age=300`,
containing **14,456 records of which 1,873 were `active`**.

The file itself is not committed (10.8 MB, and it changes every ~30 minutes). What is committed
is the `(ATS, board slug)` directory extracted from its `url` field by
`simplify::extract_board_slugs`, which is the answer to "how do I discover boards" and costs
nothing extra — see `docs/INTERNSHIP_SCRAPING.md` § A.2.

| ATS | slugs | of which had ≥1 *active* listing |
|---|---|---|
| workday | 979 | 199 |
| greenhouse | 485 | 97 |
| ashby | 297 | 121 |
| lever | 157 | 27 |
| smartrecruiters | 122 | 37 |
| workable | 43 | 27 |
| recruitee | 1 | 0 |
| **total** | **2,084** | |

10,741 of 14,456 listings (**74%**) and 1,101 of 1,873 active ones (**59%**) sit on one of these
boards. Only `greenhouse`, `lever` and `ashby` are polled today; the rest are recorded because
harvesting them was free and re-deriving them is not.

**All slugs are kept, not just the ones with an active listing today.** A board with nothing
open this week routinely has something open next week, and — more importantly — the polled list
must be **stable across runs**. Drop a slug and every posting on that board goes unobserved,
then expires together after the miss threshold, indistinguishable from the board genuinely
closing. Grow this file; do not prune it on the strength of one quiet run.

**A slug that 404s is the one exception, and even that takes evidence.** A 404 on a board's list
endpoint is unambiguous — "no such board", so it offers nothing — and since 12r each one is
recorded in `source_run_scopes.gone` rather than only printed. On the first run under migration
`0032` (2026-09-03) that was **37 dead slugs: 22 greenhouse, 13 ashby, 2 lever**. They are still
in this file, deliberately. A 404 can be a deploy, a rename in flight, or a CDN with an opinion,
and retiring a live board costs exactly what the paragraph above describes. Ask for candidates
instead, from `apps/fridge-app/backend/`:

```
cargo run --release -- boards retire
```

It names only slugs whose last three verdicts were all 404 with no answer in between, and says
how far from full that window is when it cannot answer yet. Retiring is still a hand edit to
this file; record in the commit which runs the slug was absent on.

### `fixtures/` — offline test data

All five are **real responses**, trimmed. Trimming is only ever deletion (whole records
dropped, long HTML replaced with a marked placeholder); no field value was edited, so a test
asserting on a shape is asserting on a shape the server really sent.

| file | source | contents |
|---|---|---|
| `simplify-listings.sample.json` | the `listings.json` above | 24 records — 20 `active`, 4 closed — chosen to cover the traps: both Greenhouse hosts, a `/apply` suffix, a percent-encoded Ashby slug, `?gh_jid=`/`?mobile=` query strings, a multi-location record, both `category` vocabularies, an informative `sponsorship`, an empty `degrees`, a `terms: ["N/A"]` |
| `greenhouse-board.sample.json` | `boards-api.greenhouse.io/v1/boards/{anthropic,airtable}/jobs?pay_transparency=true` | 6 jobs. The 4 Anthropic rows are **every** `intern` substring hit on that 485-job board — and all 6 are false positives ("Director, US International Tax", "Internal Communications Manager, Tech"), which is why they are the fixture. Pay-range `blurb`/`title` truncated |
| `lever-board.sample.json` | `api.lever.co/v0/postings/belvederetrading?mode=json` | 4 postings: 2 with `categories.commitment == "Intern"`, 2 without. All carry a real `salaryRange`. Description bodies truncated |
| `ashby-board.sample.json` | `api.ashbyhq.com/posting-api/job-board/Etched?includeCompensation=true` | 5 jobs: 3 `employmentType == "Intern"` (all with **empty** `summaryComponents`, which is the point), 2 `FullTime` with real compensation. Descriptions truncated |
| `weworkremotely.sample.rss` | `weworkremotely.com/categories/remote-programming-jobs.rss` | the first 5 of the feed's 25 items, verbatim except that `<description>` bodies are replaced with a placeholder |

There is deliberately **no Handshake fixture**. Handshake is never swept (see below), so there
was no reason to fetch a page; `best_effort.rs` tests against a synthetic JSON-LD block built
from the field list in `docs/INTERNSHIP_SCRAPING.md` § A.4, and says so in the test.

## `robots.txt` as actually observed

Checked before fetching, and worth recording because one of them contradicts the research doc:

| host | `User-agent: *` group |
|---|---|
| `raw.githubusercontent.com` | **404** — no file, so allow-all |
| `boards-api.greenhouse.io` | `Disallow: /embed/` and nothing else |
| `api.lever.co` | `Allow: /` plus **`Crawl-delay: 1`** |
| `api.ashbyhq.com` | **401 Unauthorized** |
| `weworkremotely.com` | `Allow: /` with targeted disallows (`/admin/`, `/account/`, …) |

Two of those needed a decision:

- **Lever publishes `Crawl-delay: 1`.** § A.1 says "no read limit is published", which is true
  of their *documentation* — but their `robots.txt` publishes one. `http.rs` honours it, and a
  published `Crawl-delay` only ever *raises* our per-host delay, never lowers it.
- **Ashby's `robots.txt` answers 401.** Under RFC 9309 § 2.3.1.3 a 4xx is "unavailable" and the
  crawler may access the server; only a 5xx or an unreachable host (§ 2.3.1.4) means complete
  disallow. That is how `http.rs` reads it, and it agrees with § F, which records the Ashby
  posting API as publicly documented and permitted.

## Not fetched

- **LinkedIn** — `robots.txt` is `User-agent: * / Disallow: /` for every path, the file carries
  an explicit notice against automated access, and User Agreement § 8.2 bars scripts and bots.
  No request was made to any LinkedIn host at any point, including for `robots.txt`; the rule
  above is quoted from `docs/INTERNSHIP_SCRAPING.md` § A.4, which recorded it.
- **Indeed** — `robots.txt` permits `/jobs`, but § A.4 recorded that a polite unauthenticated
  GET returns HTTP 403 behind a Cloudflare CAPTCHA. Passing that requires fingerprint spoofing
  or CAPTCHA solving, which the root `CLAUDE.md` forbids outright. Not re-verified here, because
  re-verifying it means making the request.
- **Handshake** — permitted (`Allow: /public` beats the blanket `Disallow: /` under longest-match
  robots semantics) and its pages carry a clean `schema.org` `JobPosting` with `validThrough`,
  the only dependable deadline field in the whole research document. It is nonetheless **not
  swept**: the sitemap has `<loc>` only, so filtering ~75,000 URLs down to software internships
  means fetching every one — ~21 hours at a polite 1 req/s, for a yield measured at well under
  1%. Implemented as per-URL enrichment instead.

## Licensing / ToS position

Factual summary of `docs/INTERNSHIP_SCRAPING.md` § F, which is the place to look for detail.

| source | position |
|---|---|
| Greenhouse job-board API | Permitted — public documented unauthenticated API; robots disallows only `/embed/` |
| Lever postings API | Permitted — officially documented public API for building career sites |
| Ashby posting API | Permitted — officially documented public job-posting API |
| SimplifyJobs `listings.json` | **No LICENSE file**, so default copyright applies and there is no grant of reuse |
| vanshb03 `listings.json` | MIT |
| WeWorkRemotely RSS | Permitted — published feed |
| Handshake `/public/jobs/*` | Permitted by robots; their broader ToS was **not** reviewed |

Two things this does not launder, stated plainly rather than as advice:

1. An ATS API being public says nothing about the **employer's** terms for their own postings.
2. The GitHub lists redistribute data scraped from those same employers, so vanshb03's MIT
   licence covers the file, not its contents.

This is a personal, non-commercial project that does not republish the data — the same posture
already taken for `data/themealdb/`. `board-slugs.json` is additionally a *derived index* (a
list of company board identifiers), not a copy of the listings.

## Known gaps and drift

- **This is a point-in-time snapshot.** SimplifyJobs commits roughly every 30 minutes; the
  counts above were true at the moment of the fetch and are not true now.
- **The cycle rolls over, and the repos are renamed in place.** `Summer2026` became `Summer2027`
  and GitHub redirects the old paths, so a stale URL keeps resolving and *looks* current long
  after it goes stale. Re-verify the repo names by cycle before the next season, not by whether
  the URL still works. The default branch is **`dev`**, not `main`.
- **The Workday slugs are tenant + shard only** (`nvidia.wd5`), not tenant + site. A Workday
  adapter needs the site id too, and that is not recoverable from a browse URL. § A.2 counts
  1,141 tenant+site pairs against the 979 tenant+shard pairs here — the difference is tenants
  running more than one site.
- **Only Greenhouse, Lever and Ashby are polled.** SmartRecruiters, Workday, Workable and
  Recruitee slugs are stored but unused.
- **Ashby slugs arrive percent-encoded** in Simplify's `url` field (`Hippocratic%20AI`) and are
  decoded before being stored here. This is not recorded in the research doc; it was found while
  building the extractor. Polling the encoded form asks for a board that does not exist.
