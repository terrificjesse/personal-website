-- Per-account ownership for the remaining user data. `reviews` already got this in
-- `0006_add_review_ownership.sql`; this is the same treatment for the three tables that
-- PLAN.md's Phase 5 checkpoint covers when it asks that "a fresh second test account sees an
-- empty fridge".
--
-- Nullable for the same reason `reviews.user_id` was: SQLite cannot add a NOT NULL column
-- without a default, and there is no honest default for "who owned this row before accounts
-- existed". NULL means **unclaimed** — written pre-Phase-5, belonging to nobody yet. The
-- first account to register claims every unclaimed row across all four tables in one
-- transaction (`routes::auth::claim_unowned_rows`); after that, nothing writes NULL again.
--
-- Unclaimed rows are invisible to every scoped read, so between this migration running and
-- the first registration the app shows an empty fridge. That is expected, and it is why the
-- backfill runs at registration rather than as a manual step someone can forget.

ALTER TABLE fridge_items ADD COLUMN user_id TEXT;
ALTER TABLE shopping_list_items ADD COLUMN user_id TEXT;
ALTER TABLE purchase_history ADD COLUMN user_id TEXT;

-- Every scoped read filters on `user_id` and most also sort by the table's timestamp, so the
-- indexes are composite in that order rather than on `user_id` alone.
CREATE INDEX idx_fridge_items_user_id ON fridge_items (user_id, added_at);
CREATE INDEX idx_shopping_list_items_user_id ON shopping_list_items (user_id, added_at);
CREATE INDEX idx_purchase_history_user_id ON purchase_history (user_id, purchased_at);
