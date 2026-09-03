-- 0031 — record the messages an inbox pass skipped as already seen.
--
-- WHY: the top-level accounting does not close without it.
--
-- Migration 0019 built `inbox_runs` explicitly as `source_runs`' accounting invariant "one
-- subsystem over", and quoted the rule it was pinning:
--
--     classified = pressing + confirmation + outreach + disregarded
--
-- That half works. It holds on all 152 rows in the live database, checked 2026-09-03, and a
-- test pins it. What is missing is the half above it. `SyncReport` carries `already_seen` —
-- messages stored by a previous pass, which rule 4 makes a no-op — and no column receives it.
--
-- So a run row reads `fetched 44, classified 0`, and there is nothing in the table that
-- distinguishes:
--
--   * 44 messages we had already processed, which is a healthy idle pass, from
--   * 44 messages the classifier silently dropped.
--
-- 142 of those 152 rows have `fetched <> classified`; across all of them 2,989 were fetched and
-- 108 classified. The 2,881 difference is entirely unaccounted for in the schema. That is the
-- exact failure this table's own comment says it exists to prevent — a quiet inbox that looks
-- identical whether nothing happened or everything was eaten — and `source_runs` avoids it by
-- accounting for every fetched row rather than most of them.
--
-- After this, the whole pass balances:
--
--     fetched = classified + already_seen
--     classified = pressing + confirmation + outreach + disregarded
--
-- BACKFILL: existing rows get `fetched_count - classified_count`, which is what `already_seen`
-- was for every run that completed — the only other way a fetched message leaves the pass is a
-- failure, and a failed pass records its reason in `outcome`/`error`. Rows where that would be
-- negative are left at 0 rather than given a fabricated number; there are none today, and a
-- negative would mean the invariant was already broken and should be visible, not smoothed.

ALTER TABLE inbox_runs ADD COLUMN already_seen_count INTEGER NOT NULL DEFAULT 0;

UPDATE inbox_runs
   SET already_seen_count = fetched_count - classified_count
 WHERE fetched_count - classified_count > 0;
