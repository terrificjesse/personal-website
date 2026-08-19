-- Blog tab: admin-authored posts, publicly readable once published.
--
-- `author_id` is NOT NULL (unlike the fridge/shopping/review tables) because there is no
-- pre-auth data to carry forward here — every post is created after Phase 5, by whoever the
-- `RequireAdmin` extractor let through, so there's no "unclaimed" state to represent.
--
-- `slug` is a separate stored column rather than derived from `title` at read time, so a
-- published URL survives a later title edit unchanged (`routes/blog.rs::update_post`
-- deliberately never rewrites it).
CREATE TABLE blog_posts (
    id TEXT PRIMARY KEY NOT NULL,
    author_id TEXT NOT NULL REFERENCES users (id),
    title TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    body TEXT NOT NULL,
    -- 0 = draft, visible only to admins; 1 = published, publicly readable.
    published INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_blog_posts_published ON blog_posts (published, created_at);
