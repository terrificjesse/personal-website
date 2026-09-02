-- A collection run cannot survive the process that started it.
--
-- Before this column, a backend that died mid-run (a crash, a Ctrl-C, a `cargo run` restart)
-- left its `collection_runs` row with `finished_at IS NULL` forever, and nothing could tell
-- that apart from a run genuinely in progress. Observed on 2026-08-21: a row 46 minutes old
-- with zero `source_runs`, which produced three compounding failures at once —
--
--   1. the UI reported "Collecting… 0/9 sources" indefinitely;
--   2. that banner replaced the "Collect now" button, so the only way to start a real run was
--      hidden by a run that was not happening;
--   3. `collector::should_collect_on_startup` saw a recent run and skipped collecting, so
--      every subsequent restart did nothing either.
--
-- The fix is reconciliation at startup rather than a timeout: when the process boots, any run
-- still marked unfinished is by definition dead, because no run outlives its process. That is
-- a fact rather than a heuristic, which is why there is no "how old is too old" constant here.
--
-- `finished_at` alone was not enough to record this. Setting it would make an abandoned run
-- indistinguishable from a completed one, and a run that died halfway is worth seeing in the
-- health panel — a source that keeps getting interrupted is a real signal.
ALTER TABLE collection_runs ADD COLUMN interrupted INTEGER NOT NULL DEFAULT 0;

-- Any run currently unfinished predates this migration and is therefore already dead: the
-- process that owned it cannot still be running, or this migration would not be executing.
UPDATE collection_runs
   SET interrupted = 1,
       finished_at = COALESCE(finished_at, started_at)
 WHERE finished_at IS NULL;
