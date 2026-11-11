# Public API — stress test

> **First pass, 2026-10-21.** Three confirmed, four refuted, one held.
>
> | | |
> |---|---|
> | **Confirmed & fixed** | S1 SQLite allocator lock collapses throughput (`65c393e`), S2 registration is unthrottled ("Rate-limit registration, before validation and before Argon2"), S4 a NaN pay bound is accepted (`c8aa917`) |
> | **Measured, by design** | S3 a listing request costs the same whatever it returns |
> | **Refuted** | S1 *as predicted* (no `/health` stall), S5 pool exhaustion, S6 oversized free text, S7 numeric extremes |
> | **Held** | S8 login limiter under a concurrent burst |
>
> Each confirmed finding has a test that failed against the code as it stood, committed in its
> own commit (`6263695`) **before** the commit that fixes it — so the bug is proven, not asserted.
> Every fix was then re-measured against the binary the deploy would build. Full suite after all
> three: 1,038 passed. Self-review only: no independent reviewer saw this pass.

**Scope: what an anonymous caller on the internet can reach.** `GET /internships` went public on
2026-10-19 (`9b2d5d8`), registration is open, and `deploy/Caddyfile` does no rate limiting — so
the backend's own throttles are the only ones in production. That is the surface this pass
attacks. The blog has had two passes of its own (`docs/BLOG_STRESS_TEST_PLAN.md`); this one
follows the same method.

**Method.** Hypotheses were written down *before* any load was run, so a refutation could not be
quietly reframed afterwards. Load ran against a release build — debug numbers are meaningless
for throughput — on an isolated copy of the database. Causes were established with a profiler,
not inferred from timings.

---

## Running it safely

`apps/fridge-app/backend/ops/stress/run-isolated.sh WORKDIR` starts a throwaway backend on
`:18080` against a `sqlite3 .backup` copy with credentials deleted, every background job off,
and no `.env` in reach (so Google OAuth never loads). Its header says why each of those is
there. **Never point load at `:8080` or the real `fridge.db`, and never call
`POST /internships/collect` against the copy** — that one is a real scrape of third-party boards.

`ops/stress/load.py` (needs `httpx` in a throwaway venv) has two modes: `load` (N workers for D
seconds) and `probe` (one cheap request every interval). Run a probe as a **separate process**:
one sharing the load generator's event loop queues behind it on the client, and a stall would
look identical to the client being busy.

Two traps this pass fell into, so the next one need not:

- **One Python process tops out near 155 requests/second.** Above that it measures itself, not
  the server. A post-fix "throughput dip at 56 clients" was entirely this — four processes at
  14 each showed the server doing ~290. Split load across processes before believing a ceiling.
- **`sample` on this machine is a Python package** that shadows macOS's profiler on `PATH`. It
  fails quietly. Use `/usr/bin/sample PID SECONDS -file out.txt`.

---

## Findings

Baseline at time of writing: 2,997 live postings, 14 cores, a 5-connection pool, and one listing
request taking 26 ms with the server otherwise idle.

### S1 — SQLite's memory-statistics lock collapses throughput under concurrency 🔴 CONFIRMED

**Predicted, wrongly:** the whole-corpus ranking runs on Tokio workers, so ~14 concurrent
listing requests would stall every route, `/health` included. `/health` p99 stayed under 60 ms
throughout. **That prediction is refuted.**

**What happened instead was worse.** Throughput *fell* as concurrency rose:

| Clients | Requests/s | p50 |
|---:|---:|---:|
| 1 | 38.3 | 26 ms |
| 5 | 11.9 | 355 ms |
| 14 | 15.1 | 633 ms |
| 56 | 9.9 | 6,862 ms |

At 56 clients the server burned ~870% CPU — eight cores — to finish ten requests a second: about
870 ms of CPU per request, against ~26 ms uncontended. Cores busy and work falling is contention,
not waiting. It began at **five** clients, inside the pool's limit and far below the core count,
which ruled out both.

**Cause, from `/usr/bin/sample` under load:** 99% of mutex waits were inside SQLite's allocator.

| Share | Stack |
|---:|---|
| 42.6% | `sqlite3Malloc` ← `sqlite3_value_dup` ← `sqlx_sqlite::SqliteRow::current` |
| 19.2% | `sqlite3DbFreeNN` ← `Arc::drop_slow` ← `drop_in_place<SqliteRow>` |
| 17.0% | `sqlite3DbFreeNN` ← `vdbeMemClear` ← `sqlite3_value_free` |
| 9.2% | `sqlite3Malloc` ← `sqlite3VdbeMemGrow` |

SQLite is built with memory statistics on by default, and updates them under **one mutex shared
by every connection in the process**. sqlx copies every column of every row out with
`sqlite3_value_dup` and frees it when the row drops, so one listing request — ~3,000 rows × ~30
columns — is ~90,000 allocations and as many frees through that one lock. `/health` stayed fast
because it never touches SQLite. **So an uptime monitor would report green while visitors waited
seven seconds.**

**Fix:** `-DSQLITE_DEFAULT_MEMSTATUS=0`, which SQLite's own *Recommended Compile-time Options*
lists for this reason. Nothing here reads the statistics it disables. A/B/A on the same database,
four client processes so the client could not be the ceiling:

| Clients | As shipped | Fixed | p50 shipped → fixed | p99 shipped → fixed |
|---:|---:|---:|---:|---:|
| 4 | 20.6 /s | **238.2 /s** | 192 → 16 ms | 235 → 23 ms |
| 16 | 22.2 /s | **295.3 /s** | 699 → 52 ms | 1,267 → 79 ms |
| 56 | 21.3 /s | **287.2 /s** | 2,504 → 193 ms | 3,794 → 230 ms |

About **13×** throughput and latency, with zero errors either way. Lock waits fall from dominant
to 0.3% of samples.

**The fix is a build setting, which is its weakness.** It lives in `.cargo/config.toml` **at the
repo root**, and must stay there: `docs/DEPLOY.md` builds with `cargo build --manifest-path
apps/fridge-app/backend/Cargo.toml` *from the root*, and Cargo finds config from the working
directory, not the manifest. A config beside `Cargo.toml` would pass CI — which runs inside the
backend directory — and be **silently ignored by the production build.** Two guards:
`db::tests::sqlite_is_built_without_its_global_memory_statistics_lock` fails any build that lost
the flag, and the backend logs a warning at startup if it is running without it, because the
deploy does not run the tests.

### S2 — Registration is unthrottled 🔴 CONFIRMED

`POST /auth/register` had no rate limit, and runs Argon2 (crate defaults: Argon2id, **19 MiB per
hash**) before any uniqueness check.

| Clients | Accounts created | Requests/s | `/health` p99 | Server memory |
|---:|---:|---:|---:|---:|
| 14 | 1,052 in 8 s | 132.0 | 52.8 ms | 123 → 390 MiB |
| 56 | 788 in 8 s | 82.3 | 163.3 ms | 123 → 390 MiB |

**1,840 accounts in 16 seconds, zero 429s** — about 470,000 an hour from one client, each with
a session row. `/health` degraded ~100× but did not stall, so the predicted stall is half-right.

**Do not "fix" the slow part by moving Argon2 to `spawn_blocking`.** Memory peaked at 390 MiB at
*both* concurrencies because running on the 14 worker threads accidentally caps concurrent
hashes at 14 (14 × 19 MiB ≈ 266 MiB). The blocking pool allows 512 threads: **512 × 19 MiB ≈ 9.7
GiB.** Moving the hash would turn slow into out-of-memory. The fix is the limit.

**Fix:** `POST /auth/register` gets the same two-bucket limiter as login — `REGISTER_MAX_ATTEMPTS
= 5` per `REGISTER_WINDOW` of one hour (`src/rate_limit.rs`), by client IP and by normalized
email — and the check runs **before**
validation and hashing, so a refused request costs nothing. Five an hour is a policy number,
picked to admit several people behind one NAT and nothing like a script; change the constant if
it is wrong.

The same burst against the fix, release build through the deploy's build path:

| 56 clients × 8 s | Before | After |
|---|---:|---:|
| Accounts created | 788 | **5** |
| Responses | 788 × 201 | 5 × 201, **3,156 × 429** |
| `/health` p99 | 163.3 ms | **1.8 ms** |
| Server memory growth | +267 MiB | +80 MiB |

The memory *baselines* differ — a warmed process before, a fresh one after — so only the growth
compares.

*Noted, not fixed:* registering an address that already exists answers differently from a fresh
one, so registration reveals whether an email has an account. Common, and out of scope here.

### S3 — A listing request costs the same whatever it returns — MEASURED, BY DESIGN

| Request | Median | Bytes |
|---|---:|---:|
| `limit=200` | 15.9 ms | 263,696 |
| `limit=50` (default) | 11.6 ms | 65,438 |
| `limit=1` | 11.6 ms | 1,492 |
| `limit=1&offset=999999` | 11.4 ms | **130** |

(Fixed build, single requests.) About 11 ms of every request is loading and ranking the whole
corpus; paging happens after. That is deliberate — rank order is a property of the whole corpus —
and it is the amplifier behind S1: a 130-byte response costs the server what a 65 KB one does.
**The structural fix is to cache the ranked corpus** and invalidate it when a collection or
expiry sweep changes it. Not done here: invalidation correctness is the whole difficulty, and it
deserves its own change rather than a footnote in a stress pass.

### S4 — A NaN pay bound is accepted 🟠 CONFIRMED

`list_postings` answers an impossible pay window with a 400, because "an empty list would look
like 'nothing pays that' rather than 'you asked for nothing'". NaN got past both checks — every
comparison against NaN is false:

| Query | Before |
|---|---|
| `pay_min=NaN&pay_unknown=drop` | **200, 0 of 2,997** |
| `pay_min=inf&pay_unknown=drop` | **200, 0 of 2,997** |
| `pay_min=NaN` | 200, 2,771 — every known-pay posting silently dropped |

**Fix:** both bounds must be finite. That rejects `pay_max=inf` too, which could be read as "no
ceiling"; it is rejected anyway, because a bound is an amount of money and omitting it already
means no ceiling.

### S5 — Pool exhaustion turns into 500s — REFUTED

Not one 500 or timeout in any run, up to 56 clients against a 5-connection pool, before or after
the S1 fix. The post-fix "dip at 56 clients" this was briefly blamed for was the load client (see
*Running it safely*).

### S6 — Unbounded free-text filters multiply matching cost — REFUTED

The HTTP layer bounds the request line: a 64 KiB `location` gets **414**, 512 KiB gets **431**,
and up to 8 KiB the cost stays flat at ~25 ms.

### S7 — Year parameters overflow at numeric extremes — REFUTED

`class_year` at ±`i32::MAX` and beyond answers 400. `term_year` at `i64::MAX` and `i64::MIN`
returns 839 — exactly what `term_year=1900` returns, because those are the unknown-term postings
`keep` retains (`term_unknown=drop` gives 0). No overflow, no panic, consistent semantics.
`offset=u64::MAX` is an empty page, as documented.

### S8 — The login limiter holds under a concurrent burst — HELD

50 simultaneous logins for one account from one address: **exactly 10 × 401 and 40 × 429**. The
check-and-record happens under one lock, so a burst cannot race past it.

---

## Second pass, 2026-11-11 — hypotheses, written before any load ran

The owner asked for more stress testing after the internships site was split out
(`docs/HUNT.md` § "The internships site, split out"). Since the first pass, the corpus grew from
2,997 live postings to 4,514 (7,937 rows including expired ones), the listing gained paging and
gzip, the extension gained alert paging, and two Next servers front the one backend. Same method
as the first pass: these predictions are committed before any load runs, against a release build
on an isolated copy (`ops/stress/run-isolated.sh`).

| | Hypothesis | Prediction |
|---|---|---|
| S9 | Listing throughput fell with the corpus: S3 says every request ranks the whole corpus | About two-thirds of the post-S1-fix numbers: ~190 req/s at 16 clients, zero errors |
| S10 | Gzip costs real CPU on the largest page | At `limit=200`, gzip ≥ 20% below identity in req/s; at the default `limit=50`, under 10% |
| S11 | Per-address limits key on the full IP, so one IPv6 /64 bypasses them | A registration burst rotating addresses inside one /64 gets **no** 429s, against all-but-five for one address |
| S12 | Alert paging rejects every malformed `cursor` with 400 | Holds: no 500s and no panics |
| S13 | The extension's page lookup (`/hunt/posting-for`) scans every row and parses each URL | Over 20 ms per request at 7,937 rows, and below the listing's throughput. Its code comment's "~1,400 rows" is stale |
| S14 | One account can grow the database without a ceiling: answers have a size cap and no count cap | 2,000 maximum-length answers are all accepted and the database grows by about 40 MB |
| S15 | Neither Next server is the bottleneck after the split | The internships site's `/internships` page and the personal site's `/internships` redirect each sustain over 500 req/s at 16 clients |
