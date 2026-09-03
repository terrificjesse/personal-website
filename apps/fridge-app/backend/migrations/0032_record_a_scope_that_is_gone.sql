-- A board that has been deleted, told apart from a board that is simply empty.
--
-- Phase 12r. `source_run_scopes.outcome` deliberately records a 404'd board as 'completed' with
-- zero postings — "no such board" is an unambiguous statement that it offers nothing, so absence
-- from it is evidence and its sightings must advance toward expiry. That is correct and 0026
-- says so on purpose. It is also lossy: after it, a board that no longer exists and a board that
-- enumerated fine with zero internships are the same row.
--
-- The collector already knows the difference. It prints
--
--     internships: 6 Greenhouse board(s) 404'd and should be retired: 10xgenomics, ...
--
-- on every run and nothing anywhere consumes it, so the finding dies with the log. This repo's
-- scraping rules require a source silently returning zero to be distinguishable from a source
-- that genuinely had zero; that rule held at the source level and was violated one level down.
--
-- A flag rather than a third `outcome` value, and the reason is the invariant it preserves:
-- every expiry decision in `internships::expiry` keys off `outcome = 'completed'`, so a new enum
-- value would silently remove gone boards from expiry — the exact opposite of what 0026 built.
-- A column orthogonal to `outcome` cannot change expiry eligibility however it is later used.
-- The CHECK enforces the other direction: a failed read proves nothing, least of all that the
-- board is gone.
--
-- Existing rows get 0, which is honest rather than convenient: the runs before this migration
-- did not record the distinction, so we do not know which of their empty completed scopes were
-- 404s. This column starts the clock at zero, and `internships::board_retirement` reports how
-- far from full its window is rather than answering from a short history.
ALTER TABLE source_run_scopes
    ADD COLUMN gone INTEGER NOT NULL DEFAULT 0
        CHECK (gone IN (0, 1) AND (gone = 0 OR outcome = 'completed'));
