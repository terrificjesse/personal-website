-- 0026 — scoped expiry: let one dead board out of 485 stop disqualifying Greenhouse.
--
-- WHY
--
-- `source_runs.counts_for_expiry` is granted per SOURCE, and a multi-board source is
-- `success` only if every board was enumerated. That rule is correct — an unreachable board's
-- postings are unobserved, not gone — but Greenhouse is 485 boards under one name, so a single
-- network error disqualifies the whole source. On the 2026-09-02 uncapped run exactly that
-- happened: 484 boards read cleanly, `designmehair` errored, and the largest ATS source
-- advanced no disappearance counters at all. At that board count a clean sweep is improbable,
-- so this was not an unlucky run; it was the steady state.
--
-- A SCOPE is a sub-unit of a source that can be enumerated completely on its own. Greenhouse's
-- scope is the board slug. Absence is still evidence only from a complete enumeration — the
-- change is that completeness is now answerable per board instead of per source.
--
-- NO TABLE REBUILD IS NEEDED HERE. SQLite cannot ALTER a CHECK constraint, so touching one
-- means rebuilding the table (see 0025, which had to). `posting_sightings` carries no CHECK
-- and this only appends a nullable column, so `ADD COLUMN` is sufficient and cheap.

-- Which scope this sighting was last seen in. NULL means "this source has one implicit scope
-- covering the whole source", which is the correct and complete reading for every row that
-- exists today — so there is nothing to backfill and nothing to guess.
--
-- Written only by `expiry::settle_source_run`, at the same moment it resets the sighting's
-- miss counter, because "where we last saw it" and "when we last saw it" are one fact and must
-- not be updatable independently.
ALTER TABLE posting_sightings ADD COLUMN scope TEXT;

-- The increment half of settle filters on exactly this pair.
CREATE INDEX idx_sightings_scope ON posting_sightings (source, scope);

-- What each scope did on one source run.
--
-- This is the record that makes a partial run's expiry decision auditable: which boards were
-- trusted, which were not, and why. It is written inside the same transaction as the counter
-- increments, so a crash can never leave counters advanced with no record of which scopes
-- earned it.
CREATE TABLE source_run_scopes (
    source_run_id TEXT NOT NULL REFERENCES source_runs (id) ON DELETE CASCADE,
    -- Stable within the source. For Greenhouse this is the board slug, and it is a database
    -- key: renaming the scheme orphans every sighting tagged under the old one.
    scope TEXT NOT NULL,

    -- Two-valued, unlike `source_runs.outcome`. A scope that can be half-read is not a scope.
    -- 'completed' — everything this scope offers was seen; absence from it is evidence.
    --               A board that 404s on its list endpoint is 'completed' with zero postings:
    --               "no such board" says it offers nothing, and its postings should expire.
    -- 'failed'    — unreachable, unparseable, or never attempted. Proves nothing.
    outcome TEXT NOT NULL CHECK (outcome IN ('completed', 'failed')),

    fetched_count INTEGER NOT NULL DEFAULT 0,

    -- A failure with no stated reason is indistinguishable from a success in a health panel,
    -- and this table is the only place a run's 400th failed board is legible at all — the
    -- source-level error string shows three failures and a count. So the pairing is enforced
    -- rather than merely intended.
    error TEXT,
    CHECK ((outcome = 'failed') = (error IS NOT NULL)),

    PRIMARY KEY (source_run_id, scope)
);

CREATE INDEX idx_source_run_scopes_outcome ON source_run_scopes (source_run_id, outcome);
