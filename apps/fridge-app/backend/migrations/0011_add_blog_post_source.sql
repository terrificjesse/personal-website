-- Where a blog post came from: 'db' = written in the browser through /blog/admin,
-- 'file' = ingested from a .md file under content/blog/ by `blog_files::sync`.
--
-- The point of putting file-sourced posts in this same table, rather than merging two stores
-- at request time, is that sort and search then have exactly **one query path** — a file post
-- and a browser post are both just rows, so `ORDER BY` and `LIKE` cover them identically and
-- no endpoint has to reconcile two orderings.
--
-- DEFAULT 'db' is what backfills the existing rows: every post that predates this migration
-- was written in the browser, so the default is not a placeholder but the correct value.
--
-- File-sourced rows are owned by the sync, not by the API: `routes/blog.rs::update_post` and
-- `delete_post` refuse them with 409, since an edit would be silently reverted on the next
-- sync, and `sync` deletes rows whose file has disappeared. Both are scoped to source =
-- 'file' and never touch a 'db' row.
ALTER TABLE blog_posts ADD COLUMN source TEXT NOT NULL DEFAULT 'db';

CREATE INDEX idx_blog_posts_source ON blog_posts (source);
