-- Phase 7: the internship tab. See docs/PLAN.md § Phase 7.
--
-- Seven tables. Two of them (`posting_sightings` and the snapshot columns on
-- `internship_applications`) exist for no reason other than to make the phase's two named
-- data-loss traps unrepresentable rather than merely unlikely.
--
-- Timestamps are RFC3339 TEXT and ids are TEXT UUIDs, matching every other table here.
--
-- As in 0007: SQLite does not enforce `REFERENCES` unless `PRAGMA foreign_keys = ON` is set
-- per connection, which `db::init_pool` does not do. The clauses below are declared for
-- documentation. Nothing in this schema may *depend* on them firing — see the note on
-- `internship_applications.posting_id`, where that distinction is load-bearing.

-- ---------------------------------------------------------------------------------------
-- Collection runs
-- ---------------------------------------------------------------------------------------

-- One row per collection pass across all sources. Exists so "the last run" is a well-defined
-- thing the run-health panel can name, and so a source's outcome is always attributable to a
-- specific pass rather than floating free.
CREATE TABLE collection_runs (
    id TEXT PRIMARY KEY NOT NULL,
    started_at TEXT NOT NULL,
    -- NULL while in flight. A row that never gets a `finished_at` is itself a signal: the
    -- process died mid-run.
    finished_at TEXT,
    trigger TEXT NOT NULL CHECK (trigger IN ('startup', 'scheduled', 'manual'))
);

-- One row per (run, source). This is the "every failure lands somewhere a human will find it"
-- requirement from the root CLAUDE.md, made structural: a source cannot participate in a run
-- without writing a row here, so a source that silently returned nothing is distinguishable
-- from one that genuinely had nothing.
CREATE TABLE source_runs (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES collection_runs (id),
    source TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,

    -- Four outcomes, not two, because "did it work" is genuinely four-valued here:
    --
    --   'success' — the adapter completed its FULL enumeration. Every posting the source
    --               currently offers was seen. This is the only outcome that may expire
    --               anything, and `counts_for_expiry` below is what enforces that.
    --   'partial' — the adapter got some of the way (page 3 of 10, 4 boards of 30) and gave
    --               up. Postings exist and are worth keeping, but absence proves nothing.
    --               Folding this into 'success' is how a paginated fetch that dies early
    --               reports healthy while 80% of its postings appear to vanish at once.
    --   'failed'  — nothing usable. Blocked, rate-limited, reshaped, unparseable, timed out.
    --   'skipped' — deliberately not fetched: robots.txt disallowed it, or it is disabled.
    --               A correct outcome, not a failure, and it must not read as one in the UI.
    outcome TEXT NOT NULL CHECK (outcome IN ('success', 'partial', 'failed', 'skipped')),

    -- `filtered` and `rejected` are separate counts on purpose, and this is not pedantry.
    -- A source returning 3,000 non-internship jobs that we filter out is perfectly healthy;
    -- 14 rows that should have parsed and didn't is a defect. Summed into one number the
    -- defect is invisible, which is precisely the "a scraper that quietly discards half its
    -- input looks perfectly healthy" failure this phase is meant to prevent.
    --
    -- Invariant, pinned by a test: fetched = accepted + filtered + rejected.
    fetched_count INTEGER NOT NULL DEFAULT 0,
    accepted_count INTEGER NOT NULL DEFAULT 0,
    filtered_count INTEGER NOT NULL DEFAULT 0,
    rejected_count INTEGER NOT NULL DEFAULT 0,

    -- Whether this run is permitted to advance disappearance counters on its source's
    -- sightings. THE DEFAULT IS 0 AND THAT IS THE POINT: a source run has to affirmatively
    -- earn the right to expire postings, so every path that forgets to think about it — a new
    -- adapter, an early return, a panic caught upstream — fails closed.
    --
    -- Set to 1 only when outcome = 'success' AND the run is not a suspicious zero (a source
    -- returning 0 postings when its previous successful run returned many is a reshaped
    -- response, not a mass closure). `internships::runner` owns that decision and is the only
    -- writer. The expiry sweep never reads this column, or this table at all — see
    -- `posting_sightings.consecutive_misses`.
    counts_for_expiry INTEGER NOT NULL DEFAULT 0 CHECK (counts_for_expiry IN (0, 1)),

    -- Human-readable failure reason. Non-NULL whenever outcome is 'failed' or 'skipped';
    -- also populated on 'partial' to say where it stopped.
    error TEXT,

    -- One row per source per run. Makes a double-write a database error rather than two
    -- half-true rows in the health panel.
    UNIQUE (run_id, source)
);

CREATE INDEX idx_source_runs_run ON source_runs (run_id);
CREATE INDEX idx_source_runs_source ON source_runs (source, started_at);

-- ---------------------------------------------------------------------------------------
-- Postings
-- ---------------------------------------------------------------------------------------

-- The deduped, normalized posting: one row per real-world job, however many sources carry it.
--
-- THE RULE THAT GOVERNS THIS WHOLE TABLE: every ranking input is nullable, and NULL means
-- *unknown*, never zero, never worst. Pay is absent from most sources; a posting with no
-- salary must not rank as though it pays nothing. `is_remote` follows the same rule at
-- smaller scale, and `company_signals.prestige` at larger.
CREATE TABLE internship_postings (
    id TEXT PRIMARY KEY NOT NULL,

    -- Identity. `dedup_key` is normalized(company)|normalized(title)|term|location, built by
    -- `internships::dedup`. UNIQUE makes "a posting present in two sources appears once" a
    -- database guarantee rather than application logic that a future upsert could forget —
    -- the same reasoning as `users.email UNIQUE` in 0007.
    --
    -- An expired posting keeps its key, so a reposted job resurrects on upsert (clearing
    -- `expired_at`) instead of inserting a duplicate alongside its own tombstone.
    dedup_key TEXT NOT NULL UNIQUE,

    -- Normalized for joining `company_signals`; `company_name` is what the UI renders. Two
    -- columns because "Google" / "Google LLC" / "google" must share a prestige signal while
    -- still displaying whatever the source actually called them.
    company_key TEXT NOT NULL,
    company_name TEXT NOT NULL,
    title TEXT NOT NULL,
    canonical_url TEXT NOT NULL,

    -- Term. NULL season or year = the source didn't say, which is common on the ATS APIs and
    -- must not be guessed from the posting date — a job posted in October is more often for
    -- next summer than this fall.
    term_season TEXT CHECK (term_season IN ('summer', 'fall', 'winter', 'spring')),
    term_year INTEGER,

    -- Location. `location_raw` is retained so a parse that produced nothing is inspectable
    -- rather than merely empty.
    location_raw TEXT,
    location_city TEXT,
    location_region TEXT,
    location_country TEXT,

    -- NULLABLE ON PURPOSE: NULL = unknown, 0 = onsite, 1 = remote. `NOT NULL DEFAULT 0` would
    -- silently assert "onsite" for every source that doesn't carry the field, and a remote
    -- filter would then quietly exclude postings that may well be remote.
    is_remote INTEGER CHECK (is_remote IN (0, 1)),

    -- Pay. All-NULL is the overwhelmingly common case and is fully legal.
    pay_min REAL,
    pay_max REAL,
    pay_currency TEXT,
    pay_period TEXT CHECK (pay_period IN ('hour', 'month', 'year')),
    -- What the source literally said ("$45-55/hr", "Competitive"). Kept so an unparsed figure
    -- can be diagnosed, and so "we couldn't parse it" stays distinct from "there wasn't one".
    pay_raw TEXT,
    -- The invariants that make these four columns a comparable quantity are enforced as
    -- table constraints at the foot of this table -- SQLite allows no further column
    -- definitions once a table-level constraint has appeared.

    -- Class-year eligibility as graduation years, which is the form that filters in SQL.
    -- "rising senior" / "graduating Dec 2026 - June 2027" both normalize into this range.
    class_year_min INTEGER,
    class_year_max INTEGER,
    class_year_raw TEXT,

    -- Dates. `posted_at` NULL means the source didn't say.
    posted_at TEXT,
    -- Set when `posted_at` was backfilled from `first_seen_at` rather than stated by the
    -- source. Without this flag the entire cold-start corpus dates to the day collection
    -- began and reads to the ranking as "all posted today" — thousands of postings tied at
    -- maximum freshness, which is worse than having no recency signal at all.
    posted_at_is_estimated INTEGER NOT NULL DEFAULT 0 CHECK (posted_at_is_estimated IN (0, 1)),

    -- NULL = no stated deadline. This must NOT be read as "expired" or as "closes now"; most
    -- sources have no deadline field, and those postings expire by disappearance instead.
    deadline TEXT,

    -- Our own observations, distinct from anything the source claims.
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,

    -- Expiry is a SOFT DELETE. NULL = live. The ranked list filters on this; nothing deletes
    -- the row. Trap 1 in PLAN.md depends on this: an applied posting stays joinable, so the
    -- tracker shows live data for as long as the row exists.
    expired_at TEXT,
    -- 'source_marked_closed' is deliberately first in preference order: several sources
    -- (Simplify's `active`, SmartRecruiters' `active`, a Greenhouse API 404) say outright
    -- that a posting is closed, which is strictly better evidence than waiting for it to
    -- fall off a feed. Disappearance is the fallback for sources that never say.
    expiry_reason TEXT CHECK (
        expiry_reason IN (
            'source_marked_closed',
            'deadline_passed',
            'vanished_from_sources',
            'manual'
        )
    ),

    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    -- --- table constraints ---
    --
    -- These live at the foot of the table because SQLite's CREATE TABLE grammar is
    -- column-defs-then-table-constraints: the first table-level CHECK ends the column list,
    -- and any column defined after it is a syntax error. Keep new columns above this line.

    -- Enforce the pay invariants by construction rather than trusting the parser. An amount
    -- with no currency and period is not a comparable quantity, and a range running backwards
    -- is a parse bug, not data.
    CHECK (pay_min IS NULL OR (pay_currency IS NOT NULL AND pay_period IS NOT NULL)),
    CHECK (pay_max IS NULL OR (pay_currency IS NOT NULL AND pay_period IS NOT NULL)),
    CHECK (pay_min IS NULL OR pay_max IS NULL OR pay_max >= pay_min),
    CHECK (pay_min IS NULL OR pay_min >= 0),

    -- A graduation-year range that runs backwards is likewise a parse bug.
    CHECK (
        class_year_min IS NULL
        OR class_year_max IS NULL
        OR class_year_max >= class_year_min
    ),

    -- Expiry is all-or-nothing: a tombstone with no reason, or a reason on a live posting,
    -- means the sweep wrote half a state transition.
    CHECK ((expired_at IS NULL) = (expiry_reason IS NULL))
);

-- The ranked list reads live postings filtered by term; the partial index keeps expired rows
-- out of the common path entirely rather than filtering them after the fact.
CREATE INDEX idx_postings_live ON internship_postings (term_year, term_season, posted_at)
WHERE expired_at IS NULL;
CREATE INDEX idx_postings_company ON internship_postings (company_key);
CREATE INDEX idx_postings_deadline ON internship_postings (deadline) WHERE expired_at IS NULL;

-- One row per (posting, source) sighting.
--
-- THIS TABLE IS TRAP 2. It is the only structure in which "LinkedIn stopped listing this but
-- Greenhouse still does" is representable at all. Put `consecutive_misses` on the posting row
-- instead and a single source's outage advances one shared counter, expiring postings that
-- three other sources are still actively serving — the named data-loss bug of this phase, one
-- level deeper than where it is usually looked for.
--
-- A posting expires by disappearance only when EVERY one of its sightings has crossed the
-- miss threshold.
CREATE TABLE posting_sightings (
    id TEXT PRIMARY KEY NOT NULL,
    posting_id TEXT NOT NULL REFERENCES internship_postings (id),
    source TEXT NOT NULL,
    -- The source's own identifier, whatever shape it takes. Paired with `source` this is the
    -- stable handle for "the same listing on the same site" across runs.
    external_id TEXT NOT NULL,
    -- The source-specific URL, which differs per source for one shared posting.
    url TEXT NOT NULL,

    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    last_seen_run_id TEXT REFERENCES collection_runs (id),

    -- Consecutive EXPIRY-ELIGIBLE runs of this source in which this sighting was absent.
    -- Advanced only by `internships::runner`, only for source runs with
    -- `counts_for_expiry = 1`, and reset to 0 on every sighting. A failed, partial, skipped
    -- or suspicious-zero run cannot touch it, because the increment never executes for one.
    --
    -- The expiry sweep reads this column and never looks at `source_runs`, so the
    -- successful-run rule lives at exactly one write site and cannot be forgotten at the
    -- read site.
    consecutive_misses INTEGER NOT NULL DEFAULT 0,

    -- One sighting per listing per source.
    UNIQUE (source, external_id)
);

CREATE INDEX idx_sightings_posting ON posting_sightings (posting_id);
CREATE INDEX idx_sightings_source ON posting_sightings (source, consecutive_misses);

-- Rows the QC pass did not accept. The whole reason this table exists is that a silent drop
-- is indistinguishable from an empty source: a scraper discarding half its input looks
-- perfectly healthy right up until someone counts.
CREATE TABLE posting_rejects (
    id TEXT PRIMARY KEY NOT NULL,
    source_run_id TEXT NOT NULL REFERENCES source_runs (id),
    source TEXT NOT NULL,

    -- 'filtered' — correctly excluded (not an internship, not SWE, wrong term). Expected in
    --              bulk, and NOT a health signal.
    -- 'rejected' — should have been usable and wasn't (unparseable required field, missing
    --              company or URL, malformed record). Every one of these is a potential bug.
    kind TEXT NOT NULL CHECK (kind IN ('filtered', 'rejected')),

    -- Machine-readable cause, e.g. 'not_an_internship', 'unparseable_pay', 'missing_company'.
    reason TEXT NOT NULL,
    -- Which field, when the cause is about one.
    field TEXT,
    detail TEXT,

    external_id TEXT,
    url TEXT,

    -- The raw record as the source gave it. Retained so a reject can be DIAGNOSED rather than
    -- merely counted — a reject count with no payload tells you something is wrong and
    -- nothing about what.
    raw_json TEXT NOT NULL,

    created_at TEXT NOT NULL
);

CREATE INDEX idx_rejects_run ON posting_rejects (source_run_id, kind);
CREATE INDEX idx_rejects_reason ON posting_rejects (kind, reason);

-- ---------------------------------------------------------------------------------------
-- Applied tracker
-- ---------------------------------------------------------------------------------------

-- THIS TABLE IS TRAP 1.
--
-- The rule: **the applied list must render correctly from this table alone, with zero joins.**
-- Every column the tracker view displays is snapshotted here at apply time. `posting_id` is an
-- enrichment — it makes the live posting joinable while it exists — and never a dependency.
--
-- Why both mechanisms rather than either one:
--
--   Soft-delete alone is not enough. FOREIGN KEYS ARE NOT ENFORCED in this database (see the
--   header note), so `ON DELETE SET NULL` below is documentation, not a guarantee. A future
--   hard delete — a cleanup script, a source purge, a schema migration — leaves `posting_id`
--   dangling at a row that no longer exists, and a tracker that joins renders a blank.
--
--   Snapshot alone is not enough either. It drifts: while the posting is live and the company
--   edits the title or publishes a pay range, the tracker would keep showing the old values.
--
-- So: snapshot is the source of truth, the join is enrichment layered on top when available.
--
-- `user_id` is NOT NULL, unlike the fridge/shopping/review tables. Same reasoning as
-- `blog_posts.author_id` in 0010: there is no pre-auth internship data, so there is no
-- "unclaimed" state to represent.
CREATE TABLE internship_applications (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id),

    -- Nullable on purpose. See above: this is the enrichment, not the record.
    --
    -- Verified against a throwaway database rather than assumed: with foreign keys
    -- unenforced, hard-deleting the posting leaves this column DANGLING at an id that no
    -- longer resolves -- `ON DELETE SET NULL` does not fire. So the read path must LEFT JOIN
    -- and treat a non-resolving `posting_id` exactly like a NULL one. An INNER JOIN here
    -- silently drops the application from the tracker, which is trap 1 arriving by the back
    -- door after the snapshot was supposed to have closed it.
    posting_id TEXT REFERENCES internship_postings (id) ON DELETE SET NULL,

    -- --- snapshot, written once at apply time, never rewritten by the collector ---
    company_name TEXT NOT NULL,
    title TEXT NOT NULL,
    url TEXT NOT NULL,
    location_raw TEXT,
    pay_min REAL,
    pay_max REAL,
    pay_currency TEXT,
    pay_period TEXT,
    term_season TEXT,
    term_year INTEGER,
    source TEXT,
    -- The full posting record as JSON at the moment of application. The columns above are what
    -- the view renders; this is so a field nobody thought to snapshot is still recoverable.
    snapshot_json TEXT NOT NULL,
    snapshot_at TEXT NOT NULL,

    -- applied -> oa -> interview -> offer/rejected. TEXT with a CHECK rather than an integer
    -- enum so the stored value is legible in `sqlite3` and adding a stage later is a migration
    -- rather than a reinterpretation of existing rows.
    status TEXT NOT NULL CHECK (status IN ('applied', 'oa', 'interview', 'offer', 'rejected')),
    applied_at TEXT NOT NULL,
    -- When `status` last changed, kept distinct from `updated_at` (which any edit bumps,
    -- including a notes edit) so "how long have I been waiting on this stage" is answerable.
    status_changed_at TEXT NOT NULL,
    notes TEXT,

    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    -- One application per posting per user. NULL `posting_id` values are distinct to SQLite,
    -- which is the behavior we want: two applications whose postings were both hard-deleted
    -- must not collide.
    UNIQUE (user_id, posting_id)
);

CREATE INDEX idx_applications_user ON internship_applications (user_id, applied_at);
CREATE INDEX idx_applications_posting ON internship_applications (posting_id);

-- ---------------------------------------------------------------------------------------
-- Derived company signals
-- ---------------------------------------------------------------------------------------

-- Backs the ranking's prestige input. The user chose DERIVED signals over a hand-maintained
-- tier list, so this table is computed from what collection actually observed and is
-- recomputed after each run.
--
-- It is a stored table rather than a subquery so the derivation is INSPECTABLE — you can ask
-- why a company scored what it did, which a hand-maintained list gives you for free and a
-- derived signal otherwise does not.
CREATE TABLE company_signals (
    company_key TEXT PRIMARY KEY NOT NULL,
    company_name TEXT NOT NULL,

    -- Inputs, stored alongside the output so the score is reproducible by hand.
    distinct_sources INTEGER NOT NULL DEFAULT 0,
    live_postings INTEGER NOT NULL DEFAULT 0,
    total_postings_seen INTEGER NOT NULL DEFAULT 0,
    pay_observations INTEGER NOT NULL DEFAULT 0,
    -- Normalized to one currency and period so companies are comparable; NULL when
    -- `pay_observations` is 0, which is the common case.
    median_pay_hourly_usd REAL,

    first_seen_at TEXT NOT NULL,

    -- NULL = NOT ENOUGH EVIDENCE, and the ranking must read that as unknown rather than as
    -- worst. Same rule as `internship_postings.pay_min`: a company we know nothing about is
    -- not a company we know to be bad. A derived signal makes this trap easier to fall into
    -- than a tier list does, because the absence looks like a computed 0.0.
    prestige REAL,
    computed_at TEXT NOT NULL
);
