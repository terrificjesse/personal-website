-- 0030 — delete `posting` alerts whose posting no longer exists.
--
-- First migration from Claude Code's reserved block, `0030–0059`. See CLAUDE.md rule 3, which
-- moved to per-agent blocks on 2026-09-03 because the previous per-lane one ran out.
--
-- WHY
--
-- Migration 0025 merged 58 duplicate posting groups and guarded its DELETE on
-- `internship_applications` only. `hunt_events.subject_id` holds a posting id for rows with
-- `kind = 'posting'`, but it is a **soft reference and not a declared foreign key**, so neither
-- the migration nor `PRAGMA foreign_keys` caught it. One alert — "New at Jane Street",
-- 2026-08-30, already acked — was left pointing at a posting 0025 had deleted.
--
-- Found by a QC pass on 2026-09-03, not by anything that was watching for it. Migration 0029
-- guards all three references (sightings, applications, hunt_events); this cleans up what 0025
-- left behind.
--
-- WHY DELETE RATHER THAN REPOINT
--
-- The merge survivor already carries its own `posting` alert, and `hunt_events` has
-- `UNIQUE (kind, subject_id)` — so repointing would collide, and would be redundant if it
-- could not: the surviving alert is about the same real job. The orphan is already `acked`,
-- so nothing re-notifies either way.
--
-- WHY NO DATE CUTOFF, UNLIKE 0027
--
-- 0027 needed one because "a posting with no sightings" also describes a brand-new posting
-- whose sightings have not been written yet. This predicate has no such window:
-- `collector::emit_posting_alert` runs *after* `upsert_posting` returns the stored id, so an
-- alert whose posting does not exist was never a legitimate intermediate state. Anything this
-- matches is a leftover.
--
-- SCOPE
--
-- `kind = 'posting'` only. The other three kinds do not hold posting ids: `nudge` and
-- `deadline` are keyed on applications, and `email` on a Gmail message id. Matching on all
-- kinds would delete every one of them.
--
-- On a fresh database this matches nothing, and a second application matches nothing.

DELETE FROM hunt_events
 WHERE kind = 'posting'
   AND NOT EXISTS (
         SELECT 1 FROM internship_postings p WHERE p.id = hunt_events.subject_id
       );
