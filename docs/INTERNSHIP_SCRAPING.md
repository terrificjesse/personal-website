# Internship posting sources — data acquisition reference

Reference for the internship-aggregator tab's collector. **This is not an implementation
plan** — it records what each source actually returns, verified by fetching it, so the
scraper design and the ranking don't get built against fields that never arrive.

All endpoint shapes, field names, and counts below were fetched live on **2026-08-20** unless
marked *unverified*. Where a number is a sample rather than a census, the sample size is given.

## Summary — the recommendation in five lines

1. **Build Simplify's `listings.json` first.** One conditional GET returns 1,881 active
   listings with term, degree, and an explicit `active` closure flag. Nothing else comes close
   on effort-to-coverage.
2. **Then mine its `url` field for board slugs** — it yields 2,244 distinct (ATS, slug) pairs,
   which is the answer to "how do I discover boards" and costs nothing extra.
3. **Poll those ATS boards directly for pay**, which Simplify has none of. Ashby is the only
   source with an unambiguous salary interval; Greenhouse needs an undocumented parameter.
4. **Do not build LinkedIn or Indeed.** LinkedIn is `Disallow: /`; Indeed answers a polite GET
   with a Cloudflare CAPTCHA. Both are dead ends under the no-evasion constraint.
5. **Expect pay on roughly a third of postings at best**, and expect the most prestigious
   employers to be the ones missing entirely. Design the ranking to degrade, not to require.

---

## A.1 — ATS public JSON APIs

All six below are unauthenticated. None required a key, a cookie, or a login.

### Endpoint shapes (all verified by fetching)

| ATS | Endpoint | Method | Response root | Pagination |
|---|---|---|---|---|
| Greenhouse | `https://boards-api.greenhouse.io/v1/boards/{token}/jobs` | GET | `{jobs:[…], meta:{total}}` | none — full board in one response |
| Greenhouse (one job) | `https://boards-api.greenhouse.io/v1/boards/{token}/jobs/{id}` | GET | job object | n/a |
| Lever | `https://api.lever.co/v0/postings/{site}?mode=json` | GET | bare array | `skip` / `limit` |
| Ashby | `https://api.ashbyhq.com/posting-api/job-board/{org}` | GET | `{apiVersion, jobs:[…]}` | none |
| SmartRecruiters | `https://api.smartrecruiters.com/v1/companies/{co}/postings` | GET | `{offset, limit, totalFound, content:[…]}` | `offset`/`limit`, **limit caps at 100** |
| SmartRecruiters (one) | `…/postings/{id}` | GET | posting object | n/a |
| Workday | `https://{tenant}.wd{N}.myworkdayjobs.com/wday/cxs/{tenant}/{site}/jobs` | **POST** | `{total, jobPostings:[…], facets}` | `limit`/`offset` in JSON body |
| Workday (one job) | `…/wday/cxs/{tenant}/{site}{externalPath}` | GET | `{jobPostingInfo:{…}}` | n/a |
| Recruitee | `https://{co}.recruitee.com/api/offers/` | GET | `{offers:[…]}` | none observed |
| Workable | `https://apply.workable.com/api/v1/widget/accounts/{acct}?details=true` | GET | `{name, description, jobs:[…]}` | *unverified — the account I tried returned 0 jobs, so the job-object shape is unconfirmed* |

### Per-ATS notes that matter

**Greenhouse** — the pay parameter is the important find.

- Salary lives in `pay_input_ranges`, and it is **absent unless you pass `pay_transparency=true`**.
  The similarly-named `pay_input_ranges=true` does *nothing* — verified in isolation on one job:
  no params → absent, `?pay_transparency=true` → 2 ranges, `?pay_input_ranges=true` → absent.
- **`pay_transparency=true` also works on the *list* endpoint**, which the official docs do not
  mention (they document it only under "Retrieve a job"). This is worth a lot: one request per
  board instead of N+1. Treat it as undocumented and therefore liable to change — if
  `pay_input_ranges` stops appearing on list responses, fall back to per-job fetches rather
  than assuming the board has no pay data.
- Population is per-company opt-in and varies enormously. Measured across whole boards:

  | board | jobs | with pay range |
  |---|---|---|
  | airtable | 16 | 16 (100%) |
  | robinhood | 127 | 113 (89%) |
  | anthropic | 484 | 430 (89%) |
  | databricks | 804 | 433 (54%) |
  | discord | 51 | 0 (0%) |

- **`min_cents`/`max_cents` carry no interval field.** Across the boards above, 32 ranges had
  `min_cents` under 100000 (under $1,000 — necessarily an hourly rate) and 1,533 were annual,
  in the same field with nothing distinguishing them. The only hints are magnitude and the
  free-text `title`/`blurb` on the range. A magnitude threshold is a heuristic, not a fact —
  and it is least reliable exactly where internships live, since a monthly intern stipend and
  a low annual salary can land in the same band. Store the raw cents plus the range `title`
  and decide late; do not normalize to an annual figure at ingest.
- `application_deadline` exists as a first-class field but is almost never set: 14/127 on
  robinhood, 0 on airtable, anthropic, databricks, discord.
- `content=true` inlines the full HTML description, department, and office on the list
  endpoint. It is expensive — Databricks' board with `content=true` is ~9 MB for 804 jobs.
- `metadata` is a per-company custom-field array. Null on Stripe's board, non-null on all 804
  Databricks jobs. Shape is company-defined; do not build a schema against it.
- A 404 is ambiguous between "no such board" and "no such job". Disambiguate by requesting the
  board's list endpoint: 200 there plus 404 on the job means the job is gone.

**Lever**

- `salaryRange` is a real structured field — `{interval, currency, min, max}`, e.g.
  `{"interval":"per-year-salary","currency":"USD","min":150000,"max":180000}`. Present on 48
  of 79 postings on the one board with pay data that I sampled. `salaryDescription` /
  `salaryDescriptionPlain` carry the free-text version alongside it.
- **There is no company-name field.** The response is a bare array of postings; the company is
  only knowable from the `{site}` slug you requested. Carry it through yourself.
- **Do not filter server-side on `commitment`.** The official docs use `Intern` as the example
  value; that returns **0** results on a board whose actual vocabulary is `Full-time`,
  `Contract`, `Fixed Term`, `Apprenticeship`, `Internship`. The vocabulary is employer-defined
  free text, so a server-side filter silently returns an empty set rather than an error. Fetch
  the whole board and filter locally.
- Documented filters (`location`, `commitment`, `team`, `department`) are all case-sensitive
  and OR'ed when repeated.
- `createdAt` is **epoch milliseconds**, unlike every other source here.
- The documented 429 rate limit (>2 req/s) applies to **application POSTs**, not to reads. No
  read limit is published.
- A company not on Lever returns **404**; a live board with nothing posted returns **200 with
  an empty array**. These are different conditions and the collector must not conflate them —
  404 means retire the slug, empty means keep polling.

**Ashby — the best source for pay, and the only one with an explicit interval**

- `?includeCompensation=true` adds a `compensation` object with `summaryComponents[]`, each
  `{compensationType, interval, currencyCode, minValue, maxValue}`. `interval` is explicit:
  `"1 YEAR"`, `"1 MONTH"`, `"NONE"`. No magnitude guessing.
- **`employmentType` has a first-class `Intern` value** — the cleanest internship filter of any
  source here, and far better than title matching. Observed vocabulary: `FullTime`, `Intern`,
  `Contract`, `Temporary`.
- Caveat that undercuts the above: a populated `summaryComponents` does **not** mean a known
  salary. Components can be `{"compensationType":"EquityPercentage","interval":"NONE",
  "currencyCode":null,"minValue":null,"maxValue":null}`. Require
  `compensationType == "Salary"` **and** a non-null `minValue`.
- Intern pay specifically is spotty even here. Ramp's one intern posting carried
  `{"interval":"1 MONTH","minValue":11700}`; Snowflake's six interns carried no compensation at
  all, on a board where 259/387 postings overall had components.
- Also has `isRemote` (boolean) and `secondaryLocations[]` — the only source with both a clean
  remote flag and structured multi-location data.
- No single-job endpoint and no filtering; you fetch the whole board every time.

**SmartRecruiters**

- **No salary field anywhere** — not on the list, not on the posting detail. Confirmed on both.
- `experienceLevel` (`{id,label}`, e.g. `mid_senior_level`) and `typeOfEmployment` are the
  useful structured filters. `location` has real `remote` and `hybrid` booleans.
- `limit` silently caps at 100 — requesting 200 returns `limit: 100` with no error. Page on
  `offset` against `totalFound`.
- The detail endpoint has an `active` boolean, which is a usable closure signal (see §D).
- Board slugs are ugly and non-obvious (`AECOM2`, `ATPCO1`, `3SBusinessCorporationInc1`) —
  guessing them from a company name will not work. Harvest them (§A.2).

**Workday — high volume, low field quality, expensive**

- The list endpoint is **POST** with a JSON body (`{"appliedFacets":{},"limit":N,"offset":N,
  "searchText":"…"}`). A GET returns 400.
- The path is `/wday/cxs/{tenant}/{site}/jobs` with **no locale segment**, even though the
  human-facing URL usually has one. `/wday/cxs/3m/Search/jobs` → 200;
  `/wday/cxs/3m/en-US/Search/jobs` → **405**. Strip `/en-US` when deriving the API path from a
  careers URL.
- The list returns only five fields, and two of them are close to useless:
  `postedOn` is a **relative string** (`"Posted 28 Days Ago"`), not a date, and `locationsText`
  collapses multi-location roles to `"2 Locations"`.
- The detail endpoint fixes both — real `location` + `additionalLocations`, and a `startDate`
  as an ISO date — but that means **one request per posting**. NVIDIA alone returned
  `total: 949` for `searchText: "intern"`. This is the one source where polite rate limiting
  makes full collection genuinely slow.
- No salary field on either endpoint.
- A wrong `{site}` returns 404, so the site id must be discovered, not guessed.
- Note there is a second Workday domain, `myworkdaysite.com`, alongside `myworkdayjobs.com`.

**Recruitee — richest schema, almost no US SWE presence**

- The schema is the most complete of any ATS here: `salary {min,max,period,currency}` (with a
  `period`, unlike Greenhouse), `close_at`, `remote`/`hybrid`/`on_site` booleans,
  `min_hours`/`max_hours`, `education_code`, `experience_code`, `employment_type_code`,
  `status`, `published_at`.
- On the one live board found, **`salary` was all-null on 12/12 and `close_at` on 12/12**. A
  field existing in the schema is not the same as an employer filling it in.
- Five of six slugs I tried 404'd, and Simplify's whole corpus contains exactly **one**
  Recruitee board. It is European-skewed and not worth early effort for US SWE internships.
- Minor data-quality note: the live board included a posting slugged `test-role-2`. Real
  boards contain test rows.

### How to discover a company's board slug

The honest answer is that you don't guess it — you harvest it. **Simplify's `listings.json`
`url` field is itself a board-slug directory**, and extracting it costs one file you are
already downloading:

| ATS | distinct slugs extractable | examples |
|---|---|---|
| Workday (tenant+site) | 1,141 | `3m.wd1`, `aaaie.wd1/CSAACareers2` |
| Greenhouse | 485 | `10xgenomics`, `1800contacts` |
| Ashby | 296 | `1password`, `8vc`, `AeroVect` |
| Lever | 157 | `AIFund`, `CesiumAstro` |
| SmartRecruiters | 121 | `AECOM2`, `ATPCO1` |
| Workable | 43 | `ascendis-pharma` |
| Recruitee | 1 | `1x` |
| **total** | **2,244** | |

**73% of all Simplify listings — and 58% of active ones — sit on an ATS with a public API.**
The rest are the problem; see §E.

Secondary discovery methods, all lower yield: a company careers page's embedded board iframe
usually exposes the token in its `src`; Greenhouse's `absolute_url` and Ashby's `jobUrl`
restate the slug on every record, so any single posting seeds the whole board.

---

## A.2 — GitHub internship-list repos

**The current cycle is Summer 2027, not 2026.** The Summer2026 repos are last season's; the
big two were renamed in place (GitHub redirects the old paths, so old links still resolve —
which makes them look current when they are not). Verify by cycle name, not by star count.

| repo | stars | machine-readable? | license | last push |
|---|---|---|---|---|
| `SimplifyJobs/Summer2027-Internships` | 46,607 | **yes** — `.github/scripts/listings.json` | **none** | 2026-08-20 |
| `vanshb03/Summer2027-Internships` | 8,881 | **yes** — same path, plus `archived/{2025,2026}/archived.json` | **MIT** | 2026-08-17 |
| `speedyapply/2027-SWE-College-Jobs` | 8,884 | no — markdown only | none | 2026-08-19 |
| `zapplyjobs/Internships-2027` | 5,116 | no — markdown only | NOASSERTION | 2026-08-20 |
| `northwesternfintech/2027QuantInternships` | 2,454 | *unverified* | none | 2026-07-30 |

**The default branch is `dev`, not `main`,** on both of the two that matter. A raw URL built
with `/main/` 404s.

```
https://raw.githubusercontent.com/SimplifyJobs/Summer2027-Internships/dev/.github/scripts/listings.json
https://raw.githubusercontent.com/vanshb03/Summer2027-Internships/dev/.github/scripts/listings.json
```

### Simplify — schema and the traps in it

14,444 records, 10.8 MB, flat array. All 15 fields present on 100% of records (nothing is
missing; plenty is uninformative):

`source`, `category`, `company_name`, `id`, `title`, `active`, `terms[]`, `date_updated`,
`date_posted`, `url`, `locations[]`, `company_url`, `is_visible`, `sponsorship`, `degrees[]`

- **`active` is false on 12,563 of 14,444 — only 1,881 (13%) are live.** The file is a rolling
  archive, not a current-listings feed. Filter on `active` or 87% of your corpus is dead links.
- **`sponsorship` is 99.2% useless**: 14,335 records say `"Other"`. Only 109 carry real
  information (`Does Not Offer Sponsorship` 59, `U.S. Citizenship is Required` 27,
  `Offers Sponsorship` 23). Do not build a sponsorship filter on this.
- **`degrees` is degree *level*, not class year** — `Bachelor's`, `Master's`, `PhD`, `MBA`,
  and so on — and is empty on 22%. It does not answer "am I eligible as a 2028 grad".
- **`category` has two coexisting vocabularies**: `Software` (4,481) alongside
  `Software Engineering` (141); `AI/ML/Data` (6,159) alongside
  `Data Science, AI & Machine Learning` (73); likewise Hardware, Quant, Product. Normalize both
  generations or you will silently drop the newer labels.
- `terms[]` is the season field and is well populated (`N/A` on 1,581). Among *active*
  listings the distribution is Fall 2026 (630), Summer 2027 (515), Summer 2026 (420) — note the
  repo is named for Summer 2027 but is not restricted to it.
- `date_posted` / `date_updated` are **epoch seconds**. In this snapshot they were identical on
  the records inspected, so `date_updated` may not be an independent signal.
- `is_visible` is true on 14,443/14,444 — no filtering value.
- 1,409 records carry more than one location; the maximum observed was 52.
- **No salary field of any kind.**
- **No license file.** Absent a license, default copyright applies — there is no grant of
  reuse. vanshb03's MIT-licensed copy is the clean one to depend on if licensing matters to
  you, at the cost of coverage.

### vanshb03 — smaller, slower, genuinely additive

402 records, 13 fields. Differs from Simplify: **`season` is a singular string** (`"Winter"`)
where Simplify has `terms[]`; no `category`, no `degrees`.

Cadence is human/PR-driven — commits on 2026-08-17 and 2026-08-06, versus Simplify's
**every ~30 minutes** (verified across 10 consecutive commits). Freshness is not comparable.

Worth including anyway: **only 117 of its 402 URLs (29%) also appear in Simplify**, so roughly
285 listings are unique to it.

### Closures and deletions

Both repos represent closure as **`active: false`, not deletion** — records persist. This is
the single most useful closure signal available anywhere in this document (§D), and it is why
these files are worth ingesting even though they carry no pay data.

### Fetching politely

`raw.githubusercontent.com` honors **`If-None-Match`**: a conditional GET with the stored ETag
returned **304 with 0 bytes**, against 10,817,091 bytes unconditionally. `Cache-Control` is
`max-age=300`; there is no `Last-Modified`. Store the ETag and you can poll frequently for
almost nothing. Note the **GitHub REST API** (used for commit history, not raw files) is
limited to **60 requests/hour unauthenticated**.

---

## A.3 — RSS / JSON feeds

Mostly a dead end. Verified:

| source | result |
|---|---|
| Greenhouse RSS (`/embed/job_board/rss?for=…`) | **404 — no RSS** |
| SmartRecruiters RSS (`jobs.smartrecruiters.com/{co}/rss`) | **404 — no RSS** |
| Lever | no RSS; JSON is the feed |
| Ashby | no RSS; JSON is the feed |
| Recruitee `/api/offers/` | JSON, works (§A.1) |
| Workable widget JSON | 200, shape *unverified* (test account had 0 jobs) |
| WeWorkRemotely `…/remote-programming-jobs.rss` | **works** — 25 items, standard RSS |
| Handshake `joinhandshake.com/api/handshake-public-jobs.xml` | **works** — sitemap index (§A.4) |

**The modern ATSs replaced RSS with JSON and did not keep both.** Don't spend effort looking
for per-company feeds; the JSON APIs in §A.1 are the feed.

**WeWorkRemotely** is real but thin: `title`, `link`, `pubDate`, `category`, `region`, `guid`.
Title is `"Company: Role"` and must be split on the first colon. No pay, no season, no
structured location, and the feed is truncated to 25 items — it is a *new-postings* feed, so it
must be polled often or not at all. Remote-first and not internship-specific.

**A more general pattern worth knowing:** several job sites publish `schema.org` **JSON-LD
`JobPosting`** blocks in their HTML (Handshake does — §A.4). Where present this is far better
than parsing the page, because it is a stable, documented vocabulary with `baseSalary`,
`datePosted`, and `validThrough`. Checking for a JSON-LD block is cheap and worth doing on any
new source before writing a bespoke parser.

---

## A.4 — LinkedIn / Indeed / Handshake

The user chose to include these knowingly. Here is what they actually do.

### LinkedIn — do not build this

- `robots.txt`, `User-agent: *` block, in full: **`Disallow: /`**. Every path. Named bots
  (Googlebot, Bingbot, Applebot…) get specific allowances; a generic client gets nothing.
- The file itself carries a notice that using robots or other automated means to access
  LinkedIn without express permission is prohibited, and points to their user agreement and
  crawling terms. Access is by allowlist application only.
- The User Agreement §8.2 separately prohibits software, scripts, or bots to scrape or copy
  the service.
- **Recommendation: exclude entirely.** There is no polite configuration of this source —
  `Disallow: /` means every request is a violation, so "best effort" and "zero results" are the
  same outcome with different amounts of wasted code. Don't add a LinkedIn collector that is
  destined to be disabled; leave it out and record why.

### Indeed — technically permitted by robots, blocked in practice

- `robots.txt` is **`User-agent: * / Allow: /`** with ~468 targeted `Disallow` lines. Notably
  `Disallow: /*?rss` (the RSS feed is off-limits) and `Disallow: /jobs/{COUNTRY}/` for many
  countries. The US `/jobs?q=…` search path is **not** disallowed.
- But a single polite unauthenticated GET to `https://www.indeed.com/jobs?q=software+engineering+intern`
  with a normal identifying user agent returned **HTTP 403** and a Cloudflare
  "Security Check" interstitial with a CAPTCHA.
- Getting past that is exactly the fingerprint-spoofing / CAPTCHA-solving the constraints rule
  out. **Recommendation: exclude.** robots.txt permitting a path does not help when the edge
  refuses the request; the honest yield is zero.

### Handshake — the surprise; permitted, clean data, poor economics

Handshake is genuinely different from the other two, and the nuance is worth stating precisely
because the obvious conclusion is wrong in both directions.

- `app.joinhandshake.com/robots.txt` is `Disallow: /` **with an allowlist**, and that allowlist
  contains **`Allow: /public`**. Job pages live at `/public/jobs/{id}`. Under longest-match
  robots semantics the specific `Allow` wins over the blanket `Disallow`, so **these pages are
  permitted**. (The job *search* is not — only `/public` and a few named paths.)
- `joinhandshake.com/robots.txt` is `Disallow:` (allow all) and advertises
  `https://joinhandshake.com/api/handshake-public-jobs.xml` — a sitemap index pointing at three
  child sitemaps of **25,000 URLs each (~75,000 public jobs)**.
- Each page returned **200** unauthenticated and carries a `schema.org` **JSON-LD JobPosting**.
  Across a 39-page random sample every page had `title`, `description`, `datePosted`,
  **`validThrough`**, `identifier`, `hiringOrganization`, `jobLocation`, `employmentType`, and
  `industry`. `validThrough` is a real expiry date and is **the only reliable deadline field in
  this entire document**.

So why isn't this the top recommendation? Three measured reasons:

1. **The corpus is not technical.** In 39 random samples: **2 internships (5%)**, **1
   engineering-adjacent title (3%)**, and **zero that were both**. The visible corpus is
   teachers, nurses, retail, lab technicians. The two internships found were "Business
   Operations" and "Facilities Operations".
2. **The sitemap carries no metadata to filter on** — `<loc>` only, no `<lastmod>`, no title.
   You cannot tell which URLs are new or which are software internships without fetching each
   one. At a polite ~1 req/s a full sweep of 75,000 URLs is **~21 hours** for a yield that the
   sample suggests is well under 1%.
3. **`baseSalary` present ≠ salary known.** Present on 6/12 in the first sample but with a
   *numeric* value on only 4/39 overall (10%) — the rest are `MonetaryAmount` shells carrying a
   `unitText` and no `value`.

Also note the JSON-LD `title` is polluted with site branding —
`"Warehouse Order Selector | Albertsons Companies | Handshake"` — and must be split on `|`
rather than used directly.

**Recommendation: permitted, so not excluded on principle — but not swept.** Treat it as a
low-priority enrichment source: if a posting already collected elsewhere has a Handshake URL,
fetching that one page is a cheap way to obtain a `validThrough` deadline nothing else
provides. A full crawl is not worth 21 hours for a handful of non-technical internships.

---

## B. Field-availability matrix

Legend: **Y** present and generally populated · **~** sometimes (see note) · **D** derivable
from other fields or free text · **N** absent · **!** present but effectively uninformative

| Source | Company | Title | URL | Location | Remote | **Pay** | Term/season | Posted date | Deadline | Class year | Sponsorship |
|---|---|---|---|---|---|---|---|---|---|---|---|
| **Greenhouse** | Y | Y | Y | Y | D | **~** ¹ | D | Y | ~ ² | N | N |
| **Lever** | D ³ | Y | Y | Y | Y | **~** ⁴ | Y ⁵ | Y ⁶ | N | N | N |
| **Ashby** | D ³ | Y | Y | Y | **Y** | **~** ⁷ | **Y** ⁸ | Y | N | N | N |
| **SmartRecruiters** | Y | Y | Y | Y | Y | **N** | D ⁹ | Y | N | D ⁹ | N |
| **Workday** (list) | D | Y | D | **N** ¹⁰ | N | **N** | D | **N** ¹¹ | N | N | N |
| **Workday** (detail) | D | Y | Y | Y | D | **N** | D | ~ ¹² | N | N | N |
| **Recruitee** | Y | Y | Y | Y | Y | ~ ¹³ | D | Y | ~ ¹³ | D | N |
| **Simplify JSON** | Y | Y | Y | Y | D | **N** | **Y** | Y | N | ! ¹⁴ | ! ¹⁵ |
| **vanshb03 JSON** | Y | Y | Y | Y | D | **N** | Y ¹⁶ | Y | N | N | ! ¹⁵ |
| **Handshake JSON-LD** | Y | ~ ¹⁷ | Y | Y | N | ~ ¹⁸ | N | Y | **Y** ¹⁹ | N | N |
| **WeWorkRemotely RSS** | D ²⁰ | Y | Y | ~ | Y | N | N | Y | N | N | N |
| **LinkedIn / Indeed** | — | — | — | — | — | — | — | — | — | — | — ²¹ |

1. Requires `pay_transparency=true`. 0–100% by board (0% discord, 54% databricks, 89%
   anthropic/robinhood, 100% airtable). **No interval field** — hourly and annual share
   `min_cents`.
2. `application_deadline` exists; set on 14/127 on one board, 0 on four others.
3. No company field in the payload — carry the requested slug.
4. `salaryRange {interval,currency,min,max}` on 48/79 on the sampled board.
5. `categories.commitment`, but employer-defined free text — filter locally, not server-side.
6. `createdAt` in **epoch milliseconds**.
7. `compensation.summaryComponents` with explicit `interval`; require
   `compensationType=="Salary"` and non-null `minValue`. Intern rows often carry none.
8. `employmentType == "Intern"` — the cleanest internship signal in this document.
9. `typeOfEmployment` and `experienceLevel` are structured but describe seniority, not season.
10. `locationsText` is `"2 Locations"` for multi-location roles.
11. `postedOn` is a relative string (`"Posted 28 Days Ago"`).
12. `startDate` is an ISO date but is the posting's start, not necessarily the posted date.
13. Schema has `salary {min,max,period,currency}` and `close_at`; both were null on 12/12.
14. `degrees[]` is degree *level*, not graduation year; empty on 22%.
15. 99.2% `"Other"` — only 109/14,444 informative.
16. `season` is a singular string, not an array.
17. Polluted with `" | Company | Handshake"`.
18. Numeric value on 4/39 sampled (10%).
19. `validThrough` on 39/39 — the only dependable deadline field found anywhere.
20. Title is `"Company: Role"`; split on the first colon.
21. Not collectable under the stated constraints — see §A.4.

### What this means for the ranking

- **Pay is the sparsest field that the ranking most depends on.** No GitHub list has it at all,
  SmartRecruiters and Workday have none, and the ATSs that do have it are per-company opt-in.
  A generous estimate for *internship* postings specifically, after intersecting "board has pay
  enabled" with "this particular intern row has a value", is well under half.
- **Only Ashby gives an unambiguous pay figure.** Everything else needs either a magnitude
  heuristic (Greenhouse) or free-text parsing (Lever's `salaryDescription`, Greenhouse's
  `blurb`).
- **Term/season is best from the GitHub lists**, not from the ATSs, where it is derivable from
  the title at best.
- **Sponsorship and class-year eligibility are effectively unavailable.** Do not design ranking
  inputs that require them. If they matter, they must come from parsing description text, which
  is a separate project with its own error rate.

---

## C. Deduplication

### Recommended merge key: the canonical ATS triple

The strongest key is not the company name or the title — it is the **(ats, board_slug,
job_id)** triple, which is recoverable from *both* sides of the join because the GitHub lists
link directly at the ATS:

| ATS | Observed posting-URL shape | Identity recovered from the URL |
|---|---|---|
| Greenhouse | `job-boards.greenhouse.io/{slug}/jobs/{id}` | `{slug}` + `id` |
| Lever | `jobs.lever.co/{site}/{uuid}[/apply]` | `{site}` + `id` (uuid) |
| Ashby | `jobs.ashbyhq.com/{org}/{uuid}[/application]` | `{org}` + `id` (uuid) |
| SmartRecruiters | `jobs.smartrecruiters.com/{co}/{id}-{slug}` | `{co}` + `id` |
| Workday jobs host | `{tenant}.wd{N}.myworkdayjobs.com/[locale/]{site}/job/…/{slug}_{id}` | `{tenant}.wd{N}` + the full suffix after the first `_` |
| Workday site host | `wd{N}.myworkdaysite.com/[locale/]recruiting/{tenant}/{site}/job/…/{slug}_{id}` | `{tenant}.wd{N}` + the full suffix after the first `_` |
| Workable | `apply.workable.com/{account}/j/{10-hex-id}[/apply]` | `{account}` + `id` |
| Rippling | `ats.rippling.com/[locale/]{company}/jobs/{uuid}` | `{company}` + `uuid` |

This is an exact key with no fuzzy matching. The original source/host count projected that it
could cover **73% of Simplify listings (58% of active ones)**, but the Phase 7 real run measured
only ~35% before the Workday, Workable and Rippling parsers existed. Task 12b owns the new
measurement; do not turn the projection into a result before that independent check runs.

**Normalize the URL before extracting**, or the join silently misses:

- **Trailing action segments**: Lever and Workable append `/apply`; Ashby appends
  `/application`. Lever accounts for the previously measured 579 records, and Workable has
  both forms for the same job in the current database.
- **Query strings**: 544 records carry `?gh_jid=…`, 574 `?mobile=…`, 339 `?ats=…`, plus
  `?embed`, `?job`, `?nl`. Strip the query entirely before comparing.
- **Two Greenhouse hosts**: `job-boards.greenhouse.io` (1,244) and `boards.greenhouse.io` (179)
  are the same board. Canonicalize the host.
- **Workday's locale segment** (`/en-US`) appears in browse URLs and must come off.

#### URL shapes verified while adding the missing identities (2026-09-02)

These parsers were built from the read-only local posting corpus, not invented examples. It
contained **384 Workday**, **19 Workable**, and **18 Rippling** stored posting URLs. Workable's
job token is an observed ten-character uppercase hexadecimal string; Rippling uses a UUID and
appears both with and without `/en-GB`; identical Rippling and Workable jobs occur in both URL
forms, which verifies that locale and trailing-action removal are identity-preserving.

Workday needs two cautions that the earlier table missed:

- `myworkdaysite.com` does not necessarily put a tenant before the shard. The observed form is
  `wd3.myworkdaysite.com/recruiting/magna/Magna/job/…`; the tenant has to come from the
  `recruiting/{tenant}` path. The parser also accepts the tenant-hosted form already recognized
  by board discovery. Task 12a changes identity only: `sources::simplify::board_of` still reads
  the first two host labels for the bare-shard form and therefore derives
  `wd3.myworkdaysite`, not `magna.wd3`. That discovery gap must be fixed separately before
  those boards can be polled by tenant.
- All 384 observed final path segments have one display/id separator `_`; 11 have another
  underscore **inside** the identity (`R_12318`, `REQ_0000080335-1`). Split on the first,
  never the last. Same-title URLs in the corpus whose suffixes differ only by `-1` or `-2`
  suggest those endings can be route disambiguation, but the URL alone cannot tell that case
  from a requisition id that genuinely contains a final hyphen-number. The parser preserves it
  and keeps the shard in `{tenant}.wd{N}`. Both choices can under-merge; neither can collapse
  two jobs on an assumption the URL does not prove. That is the deliberate side of the error
  direction for this table.

Task 12b can measure the blast radius without collecting or touching the live database. The
fixture is the consistent SQLite backup `/tmp/fridge-12b-copy.db`; the ignored test opens it
read-only, reports one-new-key/multiple-row merge candidates, and separately reports
one-stored-row/multiple-key split candidates. Run exactly from the `pw-lane-ab` worktree root
(the live, gitignored database remains in the sibling main worktree):

```sh
sqlite3 -readonly ../personal-website/apps/fridge-app/backend/fridge.db ".backup '/tmp/fridge-12b-copy.db'"
cd apps/fridge-app/backend
REKEY_FIXTURE_DB=/tmp/fridge-12b-copy.db cargo test internships::dedup::tests::report_new_ats_key_merge_and_split_candidates -- --ignored --nocapture
```

The 12a author deliberately did **not** run that ignored test: 12b is the independent
merge/split measurement. Normal unit tests use the real URL strings above without reading a
database.

**Verified caveat:** the join is not lossless. An `active: true` Simplify record pointed at
Greenhouse board `mcghealth` job `8350486002`; the board's list endpoint returned 200 while
the job returned `{"status":404,"error":"Job not found"}`. The URL key was correct — the
posting was simply gone and Simplify hadn't caught up. **A missing join is evidence about
freshness, not proof of a bad key.**

### The fallback key, and what breaks it

For the ~27% not on a pollable ATS, you need `(normalized_company, normalized_title,
normalized_location)`. Every part of it is fragile:

- **Company-name variants.** Measured *within Simplify alone*, 18 groups of distinct
  `company_name` values collapse to the same normalized key. Real examples: `KLA` /
  `KLA Corporation`; `DRW` / `DRW Holdings`; `Medpace` / `Medpace, Inc.`; `Astera` /
  `Astera Labs`; `Curtiss-Wright` / `Curtiss-Wright Corporation`; `Brain Co.` / `Brain Corp`;
  and — the ones that show pure hygiene problems — **`WhatNot` / `Whatnot`** (case) and
  **`Moog` / `Moog `** (trailing whitespace). Across sources this gets worse: parent/subsidiary
  pairs ("Google" vs "Alphabet", "Instagram" vs "Meta") are not string problems at all and need
  a small hand-maintained alias table. There is no algorithmic fix for those.
- **Title variants.** "SWE Intern" vs "Software Engineer Intern, Summer 2026" share almost no
  surface form. Strip the season, the requisition id, and any parenthetical before comparing;
  even then, expect this key to under-merge rather than over-merge, which is the safer failure.
- **Multi-location postings.** 1,409 Simplify records list more than one location (max 52).
  If one source explodes a posting per location and another keeps it as one row, a
  location-bearing key double-counts. Prefer keying on the *set* of locations, or drop location
  from the key and use it only as a tie-breaker.
- **Reposts with new ids.** A closed-and-relisted role gets a fresh ATS id and is genuinely
  indistinguishable from a new posting by the primary key. Only the fallback key catches it,
  and only if the title didn't change.

### A title-matching trap worth hard-coding against

**Do not use a naive `title.contains("intern")` filter.** Measured on a 1,404-title real
corpus from the Greenhouse boards fetched here: 21 titles contain `intern` as a substring, and
**18 of them (86%) are false positives** — "Product Marketing Manager, International",
"Director, US International Tax", "Internal Communications Manager, Tech". Only 3 were genuine
internships. Use a word-boundary match (`\bintern(ship|s)?\b`) and prefer a structured field
where one exists (Ashby's `employmentType == "Intern"`, Simplify's `terms`). This bit during
research: a substring filter written to find an internship returned "Manager, International
Statutory & Technical Accounting".

### Is `src/nlp.rs` reusable here?

**Mostly no — it is the wrong shape for posting dedup, with one narrow exception.**

`suggest_item_names(query, candidates, limit) -> Vec<Suggestion>` is a *ranked typeahead*: it
scores every candidate into non-overlapping bands (exact 1.00 / prefix 0.80 / substring 0.60 /
fuzzy 0.30), sorts, and truncates to `limit`. Dedup needs a *pairwise boolean* — "are these the
same posting?" — which is a different question with a different output type.

Concretely, three things break on job titles:

1. **The fuzzy tier compares the whole query against each single token of the candidate:**
   `candidate.name_lower.split_whitespace().map(|token| normalized_damerau_levenshtein(query,
   token)).fold(0.0, f32::max)`. For "software engineer intern summer 2026" against tokens like
   "swe" and "intern", every comparison scores near zero. The design point is one- and two-word
   grocery names, and it is well-tuned for that.
2. **The substring band would fire constantly.** `BAND_SUBSTRING` triggers when the candidate
   contains the query; among job titles, "Engineer" is a substring of a large fraction of the
   corpus, so unrelated roles would collide at 0.60+ and outrank genuine fuzzy matches by
   construction — the band ordering that makes it correct for groceries makes it wrong here.
3. **`Candidate` is fridge-shaped** — it carries `foodkeeper_product_id` and a
   `SuggestionSource {Fridge, Foodkeeper}` enum. And `SCORE_FLOOR` is a *display* floor for a
   dropdown, not an identity threshold; there is no calibrated "same entity" cutoff.

**The exception is company-name canonicalization**, which is genuinely the same problem it was
built for: short strings, a known candidate list, typo and suffix tolerance. `KLA` →
`KLA Corporation` is exactly a prefix-band hit; `WhatNot` → `Whatnot` is exactly the
normalization it already does. Calling `suggest_item_names` with a company-name candidate list
and taking only `BAND_EXACT`/`BAND_PREFIX` results would work — though the fridge-specific
fields on `Candidate` make it awkward, and it would not handle "Google"/"Alphabet", which is a
knowledge problem rather than a string problem.

**Important:** `src/nlp.rs` is a `[learn]` file. Generalizing it — making `Candidate` generic,
adding a pairwise entry point, retuning the bands for long strings — would mean *editing* it,
which is out of scope for Claude per `CLAUDE.md`. The right shape is a **separate matcher
module for postings** that leaves `nlp.rs` untouched, optionally calling into it for the
company-name case only.

---

## D. Freshness and closure detection

Most sources never say a posting closed. Ranked by reliability:

**1. Explicit status flags — use these first where they exist.**

| source | field | notes |
|---|---|---|
| Simplify / vanshb03 | `active` | Records persist; 12,563/14,444 are `false`. The best closure signal available. |
| SmartRecruiters (detail) | `active` | Boolean on the posting detail endpoint. |
| Recruitee | `status` | Observed `"published"`. |
| Handshake JSON-LD | `validThrough` | A real expiry *date* — 39/39 in sample. Genuinely predictive rather than reactive. |
| Greenhouse | `application_deadline` | Exists but rarely set (14/127 on one board, 0 on four others). |

**2. API 404 on the posting — reliable, but only via the API.**

`GET boards-api.greenhouse.io/v1/boards/{slug}/jobs/{id}` returns a clean
`{"status":404,"error":"Job not found"}` for a dead job.

**The critical trap: the public HTML URL does not 404.** The same dead job at
`https://job-boards.greenhouse.io/mcghealth/jobs/8350486002` **redirects to the board root with
HTTP 200** (`…/mcghealth?error=true`). A collector that checks liveness by HTTP status on the
public URL will conclude every dead posting is alive, forever, with no error to alert on. Check
the **API** endpoint, and treat a redirect away from the job path as a closure signal in its
own right.

Disambiguate the two 404 causes: a 404 on the job **plus 200 on the board's list** means the
job closed; 404 on both means the board itself is gone and the whole slug should be retired.

**3. Disappearance from the feed — the fallback, and it needs hysteresis.**

For sources with no status field (Lever, Ashby, Workday), absence from a full board fetch is
the only signal. Do not act on one miss: a partial fetch, a timeout, a transient 5xx, or a
paginated read that stopped early all look identical to a closure. **Require 2–3 consecutive
clean runs** — where "clean" means the board fetch itself succeeded, not merely that the
posting was absent — before marking a posting closed, and record the miss count so the decision
is auditable. Ashby and Greenhouse return the whole board in one response, so "the fetch
succeeded" is unambiguous there; Workday's paginated POST needs the page count checked against
`total` before any absence is trusted.

**Never let a failed fetch mark postings closed.** This is where per-source failure isolation
and closure detection interact: a blocked source must record "run failed" and leave every
posting's state untouched, rather than observing zero postings and retiring the whole board.

**4. Cross-source disagreement is data.** Simplify said `active: true` for a posting Greenhouse
had already 404'd. Where the ATS is authoritative — it is the system of record — prefer it, and
treat a list's `active` flag as a *lagging* indicator useful mainly for postings you cannot
reach directly.

### D.4 — Scopes: "the fetch succeeded" at board granularity (added 2026-09-02, migration 0026)

Rule 3 above says absence counts only from a run where the fetch itself succeeded, and rule
"never let a failed fetch mark postings closed" says one broken board makes the whole source
untrusted. Both are right, and together they made the largest ATS source unable to expire
anything at all.

Measured on the first uncapped run (2026-09-02): Greenhouse polled **485 boards, 484 read
cleanly, one — `designmehair` — returned a network error**, and the source-level verdict was
therefore `partial`. Not one of those 484 complete enumerations counted toward closure. At 485
boards a fully clean sweep is improbable, so that was the steady state, not an unlucky run.

A **scope** is a sub-unit of a source that can be enumerated completely on its own. The rule is
unchanged — absence is evidence only from a complete enumeration — but completeness is now
answerable per scope instead of per source.

| source | scope | why |
|---|---|---|
| Greenhouse | the board slug | Whole board in one request, no pagination, so "this board was completely read" is unambiguous. 485 of them under one source name. |
| Lever, Ashby | *none yet* | Also multi-board and the obvious next candidates. The mechanism is source-agnostic; only the adapter half is missing. Deliberately not done in the same change. |
| Simplify, vanshb03, WeWorkRemotely | *none, and none needed* | One endpoint each. A scope would be the whole source. |
| LinkedIn, Indeed, Handshake | *none* | Never enumerate at all — see A.4. |

Three properties worth knowing before touching this:

- **A scope is all-or-nothing.** There is no partial scope. If a sub-unit can be half-read it is
  not a scope; split it further or leave it unscoped. This is why Workday, whose paginated POST
  can stop mid-board, is not a scope candidate as-is.
- **A board that 404s on its list endpoint is a *completed* scope with zero postings**, not a
  failed one. Per rule 2 above, "no such board" is an unambiguous statement that it offers
  nothing, and its postings should expire. Before scopes they could only expire on a run where
  all 485 boards succeeded.
- **A scope's verdict and its posting ids must come from one parse.** `greenhouse::board_result`
  returns both in a tuple for exactly this reason. A scope reported complete whose ids went
  missing gets its whole board's miss counters incremented with nothing reset — every posting on
  it expiring after three runs. That is the one way this mechanism loses data.

`expiry.rs`'s module doc carries the soundness analysis, including the single narrow case where
scoped expiry can over-expire where the source-level rule would not.

**Scopes are forward-looking, and migration `0028` is what reaches backwards.** A sighting is
tagged when a run *sees* it, so a sighting whose job is already gone can never be tagged — and
an untagged sighting does not advance on a partial run. Measured in 12j: of 42 legacy sightings
on 100 completely enumerated boards, 37 were tagged and the 5 that were not were already dead.

`posting_sightings.url` already records the board, and `upsert_posting` rewrites it every time
the sighting is seen, so its slug is the same fact the tag carries. 0028 backfills from it, and
is **generated from `dedup::ats_identity`** rather than parsing URLs in SQL — that parser
already knows Greenhouse's three host forms and its one case-foldable path, and a second
implementation would diverge exactly where it hurts. The generator is
`src/internships/scope_backfill.rs`; regenerate and re-review with:

```
sqlite3 fridge.db ".backup '/tmp/scope-backfill.db'"
SCOPE_FIXTURE_DB=/tmp/scope-backfill.db SCOPE_BACKFILL_OUT=/tmp/0028_body.sql \
  cargo test -p fridge_backend scope_backfill -- --ignored --nocapture
```

Two pseudo-slugs must never become scopes: `embed` (Greenhouse's embed form, which carries no
board) and `gh_jid` (a job id in a query parameter on a company's own careers page). Neither is
a board, neither is ever polled, and a sighting tagged with one would wait forever.

---

## E. Prestige signals

The user chose to derive prestige rather than maintain a tier list. Here is what is actually
derivable from the sources above, with an honest read on each.

| Signal | Derivable from | Strength |
|---|---|---|
| **Pay percentile** | Ashby (clean), Greenhouse (`pay_transparency`), Lever (`salaryRange`) | **Best available**, but see the selection bias below. |
| **Runs its own ATS / no public API** | Which host the posting URL points at | Weak but real — see below. |
| **Posting volume** | Board `total` (Greenhouse `meta.total`, Workday `total`, SmartRecruiters `totalFound`) | **Weak and inverted.** Big boards mean big companies, not selective ones. |
| **Speed of disappearance** | Consecutive-miss tracking from §D | **Plausible but confounded.** |
| **Appearance in curated lists** | Presence in Simplify *and* vanshb03 | Weak, circular. |
| **Headcount** | Not available from any source here | **Not derivable** — would need an external dataset. |

**Pay percentile** is the only signal with a direct causal story, and it has a nasty selection
bias: pay is disclosed largely because a *jurisdiction* requires it, not because a company pays
well. Companies with Colorado, NYC, or California postings disclose; others don't. So a pay
percentile computed over disclosed postings ranks *disclosure-heavy* employers, which
correlates with US-coastal presence more than with prestige. Compute it within a location
cohort if you use it at all.

**"Runs its own ATS" is a real signal but points the wrong way for coverage.** The 42% of
active Simplify listings *not* on a pollable ATS are disproportionately the most prestigious
employers — the measured tail includes TikTok, ByteDance, Tesla, Citadel, Citadel Securities,
Apple, Jane Street, Goldman Sachs, Optiver, Meta, Amazon, Google, Epic Games, Jump/Quantbot,
plus a long list of Oracle Cloud and iCIMS tenants. Large companies buy or build bespoke
career sites. This means two things: the signal has some validity, **and the collector will
systematically under-cover exactly the companies the ranking most wants to rank highly.**
That bias is worth surfacing in the UI rather than hiding in a score.

**Posting volume is close to useless and arguably inverted.** Databricks' board has 804 jobs,
Airtable's 16. A high count reflects company size and hiring velocity, not selectivity — and if
anything the most selective internship programs post *one* requisition for hundreds of hires.

**Speed of disappearance** sounds appealing — competitive roles fill fast — but is confounded
by ATS hygiene (some companies prune aggressively, some leave stale rows for months), by
requisition-vs-role modeling, and by your own polling interval, which sets the floor on the
resolution you can measure. At a daily poll you cannot distinguish "closed in 4 hours" from
"closed in 20 hours".

**Presence in curated lists is circular** — those lists are themselves curated partly by
perceived prestige, so using them as a prestige input just relaunders the maintainers' opinions
as derived data. If that is acceptable, say so explicitly rather than letting it look like an
independent measurement.

**Honest overall read:** of the six, only pay percentile carries real information, and it
carries a location bias that has to be corrected for. A derived prestige score built from the
rest will mostly encode company size and ATS-vendor choice. Consider ranking on pay and
freshness alone, and treating "prestige" as an explicit user-supplied preference rather than
something the data supports inferring.

---

## F. Legal / ToS summary

Factual. The user has already decided to include the restricted sources on a best-effort basis;
this section exists so that decision is informed, not to re-litigate it.

| Source | Programmatic access | Basis |
|---|---|---|
| Greenhouse Job Board API | **Permitted** | Public documented unauthenticated API; `boards.greenhouse.io/robots.txt` disallows only `/embed/`. No published rate limit; docs and community reports agree that abusive callers get throttled. |
| Lever Postings API | **Permitted** | Officially documented public API intended for building career sites. Published 429 limit applies to application POSTs, not reads. |
| Ashby Posting API | **Permitted** | Officially documented public job-posting API. No documented rate limit; ~100 req/min is community lore, *unverified*. |
| SmartRecruiters Postings API | **Permitted** | Public unauthenticated API under `api.smartrecruiters.com`. |
| Workday CXS | **Silent** | Undocumented internal endpoint powering the public site. `nvidia.wd5.myworkdayjobs.com/robots.txt` allows the career-site path and disallows only `/talentcommunity/` and `/refreshFacet/`; `/wday/cxs/` is not disallowed. Not a published API — treat as liable to change without notice. |
| Recruitee `/api/offers/` | **Silent** | Public JSON endpoint, widely used, not formally documented as public. |
| GitHub lists (Simplify) | **No license** — default copyright | No LICENSE file; there is no grant of reuse. Content is community-contributed. |
| GitHub lists (vanshb03) | **MIT** | Explicit permissive license. |
| Handshake `/public/jobs/*` | **Permitted by robots** | `app.joinhandshake.com` is `Disallow: /` *with* `Allow: /public`; the public jobs sitemap is advertised in `joinhandshake.com/robots.txt`. Their broader ToS was **not** reviewed — *unverified*. |
| WeWorkRemotely RSS | **Permitted** | Published RSS feed. |
| **Indeed** | **robots permits, edge blocks** | `User-agent: * / Allow: /` with targeted disallows (incl. `/*?rss`), but a polite GET returns 403 + Cloudflare CAPTCHA. |
| **LinkedIn** | **Prohibited** | `User-agent: * / Disallow: /`, an explicit notice in `robots.txt`, and User Agreement §8.2 barring scripts/bots to scrape or copy the service. Access is allowlist-only by application. |

Two practical notes rather than advice: an ATS API being public says nothing about the
*employer's* terms for their own postings, and the GitHub lists redistribute data scraped from
those same employers, so licensing the file does not launder the contents. For a personal,
non-commercial, non-republishing project this is the same posture as `data/themealdb/`.

---

## G. Recommended collection order and cadence

Ordered by coverage-per-effort, not by data quality.

| # | Source | Why this order | Cadence | Est. requests/run |
|---|---|---|---|---|
| 1 | **Simplify `listings.json`** | 1,881 active listings, term + degree + `active` closure flag, from **one** conditional GET. Nothing else is close. | **hourly** (upstream commits ~every 30 min; ETag makes a no-change poll free) | 1 |
| 2 | **Slug extraction from #1** | Free — it's the file you already have. Produces the 2,244-slug board directory that makes #3 possible. | same run as #1 | 0 |
| 3 | **Ashby boards** | Best pay data (explicit interval) and `employmentType=="Intern"`. Whole board per request. | **6–12 h** | ~296 |
| 4 | **Greenhouse boards** | Largest slug count after Workday; pay via `pay_transparency=true` on the list endpoint. | **6–12 h** | ~485 |
| 5 | **Lever boards** | Structured `salaryRange`; whole board per request. | **12–24 h** | ~157 |
| 6 | **vanshb03 `listings.json`** | 29% URL overlap with Simplify → ~285 unique listings, and MIT-licensed. | **daily** (commits are days apart) | 1 |
| 7 | **SmartRecruiters boards** | Good structured fields, but **no pay at all**. | **daily** | ~121 + paging |
| 8 | **WeWorkRemotely RSS** | Cheap, but truncated to 25 items — must be frequent or skipped. | **hourly** or drop | 1 |
| 9 | **Workday tenants** | Highest slug count (1,141) but N+1 fetches, no pay, and a useless list payload. Build last, and only for tenants that actually carry internships. | **daily**, tenant subset | expensive — budget it |
| 10 | **Recruitee** | One board in the entire corpus. | **weekly** or skip | ~1 |
| 11 | **Handshake** | Permitted, and the only `validThrough`. Enrichment only — never a sweep. | **on demand**, per known URL | 1 per posting |
| — | **LinkedIn, Indeed** | Not built. See §A.4. | — | — |

### Operating rules

- **Fail fast per source, never per run.** Each source is one unit: a 4xx, a timeout, a parse
  error, or an unexpected shape records the reason and moves on. A blocked Workday tenant must
  not prevent Greenhouse from being written.
- **Distinguish "run failed" from "zero results."** These have opposite consequences for
  closure detection (§D). A failed run must leave posting state untouched.
- **Rate limit per host, not globally.** 485 Greenhouse boards are all one host; a global
  limiter makes the whole run serial for no benefit, while a per-host limiter keeps you polite
  where it matters.
- **Conditional GETs everywhere they work.** Verified on `raw.githubusercontent.com` (304, 0
  bytes vs 10.8 MB). Check for `ETag` on ATS responses too — *unverified* whether they send one.
- **Send a normal identifying user agent with contact info** and stop on 403/429 rather than
  retrying harder. A source that pushes back is a source to drop for that run and log.
- **Surface failures where a human sees them.** A per-run summary — source, status, count,
  reason — is the minimum. In this repo the natural fit is the pattern `blog_files::sync`
  already uses: a returned report struct with per-source counts, logged at the end of the run.
  Note the Phase 6 bug worth not repeating — `sync` accumulated its skip count in a local that
  never reached the returned report, so `skipped` always read `0`. **A failure counter that is
  never read is the same as no failure reporting at all**, and it will look like it's working.

### Realistic expected yield

From #1–#5 (one file plus ~940 board fetches), expect on the order of **1,000–1,500 distinct
active internship postings** after dedup, of which perhaps **a third carry any pay figure** and
fewer carry an unambiguous one. That is the corpus the ranking has to work with. Everything
below #5 adds coverage at rapidly worsening cost per posting.

---

## Known gaps in this document

- **Workable's job-object shape is unverified** — the endpoint returned 200 but the account I
  tried had zero postings.
- **No ATS was tested for `ETag` support**, so the conditional-GET recommendation is verified
  only for `raw.githubusercontent.com`.
- **Rate limits are largely unpublished.** No vendor here documents a read limit for their
  public board API except Lever, whose documented limit covers application POSTs only. The
  cadences in §G are chosen to be conservative, not because a limit is known.
- **Handshake's general ToS was not reviewed** — only `robots.txt`, which permits `/public`.
- **The Handshake density estimate is 39 samples.** 2 internships and 1 engineering title is
  enough to rule it out as a primary source, not enough to state a precise rate.
- **`northwesternfintech/2027QuantInternships` was not opened** — listed for completeness only.
- **Salary population rates are per-board samples**, not a census. They varied from 0% to 100%
  across five Greenhouse boards; do not treat any single figure as the platform rate.
- This is a point-in-time snapshot (**2026-08-20**). The GitHub repos roll over each cycle and
  were renamed from `Summer2026` to `Summer2027` in place — **re-verify the repo names before
  the next cycle**, because the old URLs redirect and will look alive long after they go stale.
