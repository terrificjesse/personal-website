-- Ownership + visibility columns for the global review aggregator (see docs/PLAN.md Phase 5).
-- Added ahead of auth on purpose: retrofitting ownership onto rows that never had it is far
-- more painful than carrying nullable columns for a phase.
--
-- `user_id` is NULL for every review written before Phase 5 — the app was single-user, so
-- there was no one else those reviews could belong to. Phase 5 backfills them with the real
-- account id once one exists.
ALTER TABLE reviews ADD COLUMN user_id TEXT;

-- Opt-in, not opt-out: defaults to 0 so nothing already written silently becomes world-
-- readable the moment the global aggregator is switched on.
ALTER TABLE reviews ADD COLUMN is_public INTEGER NOT NULL DEFAULT 0;

-- Moderation tombstone. Hidden rows stay in the table (a review is evidence even when it
-- shouldn't be displayed) but are excluded from every read path.
ALTER TABLE reviews ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;

CREATE INDEX idx_reviews_recipe_id ON reviews (recipe_id);
CREATE INDEX idx_reviews_user_id ON reviews (user_id);
