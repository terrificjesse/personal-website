-- 0027 — delete postings that nothing points at and nothing can ever expire.
--
-- WHY
--
-- `expiry::sweep` deliberately refuses to touch a posting with zero sightings. The guard is
-- load-bearing: `NOT EXISTS (a sighting below threshold)` is **vacuously true** for a posting
-- with no sightings at all, so without it a brand-new posting whose sightings failed to record
-- would expire on the very next sweep. That guard is correct and stays.
--
-- Its consequence, traced in `docs/PLAN.md` § 12c finding 3: when a re-key makes a sighting
-- compute a different dedup key, the sighting is **moved onto** the row holding that key —
-- `UNIQUE (source, external_id)` means it is updated, not duplicated — and the row it left
-- keeps no sightings. Such a row can never be expired by any rule the sweep implements. It is
-- immortal, it is a duplicate of a row that is still live, and it is visible in the ranked list.
--
-- Four exist, all created 2026-08-21, all `co:` fallback-key rows vacated by migration 0025:
--
--   Nightwing Intelligence Solutions — Software / Hardware Engineering Intern
--   Nightwing                        — Software / Hardware Engineering Intern
--   Tower Research Capital           — Quantitative Developer Intern
--   Tower Research                   — Quantitative Developer Intern
--
-- WHAT THIS DELIBERATELY DOES NOT DO
--
-- It does not teach the sweep to expire sighting-less rows once they are old enough. That
-- reopens exactly the hazard the guard was written for, is a change to the one function that
-- decides what "closed" means, and deserves its own decision rather than being smuggled into a
-- cleanup.
--
-- EVERY REFERENCE, CHECKED RATHER THAN ASSUMED
--
-- Two declared foreign keys point at `internship_postings (id)` — `posting_sightings.posting_id`
-- and `internship_applications.posting_id` — and one soft reference does:
-- `hunt_events.subject_id`, for rows with `kind = 'posting'`, which is not a declared FK and so
-- would not have been caught by `PRAGMA foreign_keys`. All three are excluded below. A scan of
-- every column in the schema whose name mentions a posting, a subject or a job found nothing
-- else; `company_signals.live_postings` and `total_postings_seen` are aggregates, recomputed by
-- a full SELECT at the end of every collection run, so they self-correct rather than drift.
--
-- WHY THE DATE
--
-- The cutoff is the start of the first uncapped run (2026-09-02T20:41:48Z). It is what keeps
-- this from being a general rule that quietly deletes a posting created moments ago whose
-- sightings failed to write — the precise row the sweep's guard exists to protect. This
-- migration is a cleanup of a known historical mess, not a policy.
--
-- On a fresh database, and on any second application, this matches nothing and is a no-op.

DELETE FROM internship_postings AS p
 WHERE p.created_at < '2026-09-02T20:41:48'
   AND NOT EXISTS (SELECT 1 FROM posting_sightings s       WHERE s.posting_id = p.id)
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = p.id)
   AND NOT EXISTS (SELECT 1 FROM hunt_events h             WHERE h.subject_id = p.id);
