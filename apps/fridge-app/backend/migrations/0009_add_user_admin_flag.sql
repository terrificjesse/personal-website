-- Admin flag for accounts. Lets a small number of trusted accounts (initially just the site
-- owner's) reach admin-only features — starting with the blog editor — without a separate
-- roles/permissions table. A single boolean is enough for "is this me," which is the whole
-- requirement right now; revisit if a second kind of privileged account ever shows up.
--
-- No API grants this. It is deliberately not self-service: set it directly in the database
-- for your own account after registering —
--
--   sqlite3 fridge.db "UPDATE users SET is_admin = 1 WHERE email = 'you@example.com';"
--
-- New accounts default to 0 (see `routes::auth::register` and `resolve_google_identity`,
-- neither of which sets this column, so the schema default applies).

ALTER TABLE users ADD COLUMN is_admin INTEGER NOT NULL DEFAULT 0;
