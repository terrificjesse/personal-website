-- Phase 8e: the alert channel. See `apps/hunt-extension/CLAUDE.md`.
--
-- One table, two producers, one poll endpoint, one notification path:
--
--   internships::collector  -- a tier-1/2 company posted something new --.
--                                                                        >-- hunt_events
--   inbox::classify (8d)    -- an email means OA / interview / offer  --'
--
-- 8e writes only the first. The second is designed for here and lands later against this
-- same table; building a second pipeline for it is the thing this shape exists to prevent.
--
-- Timestamps are RFC3339 TEXT and ids are TEXT UUIDs, matching every other table here.

CREATE TABLE hunt_events (
    id TEXT PRIMARY KEY NOT NULL,

    -- Which producer wrote it. The extension switches on this to filter alert kinds, which
    -- is how the open "should Hunt/Outreach interrupt me" question stays a one-line change
    -- rather than a schema change.
    kind TEXT NOT NULL CHECK (kind IN ('posting', 'email')),

    -- Who may see this event.
    --
    --   NULL     = derived from the shared posting corpus. Not private to anyone; every
    --              signed-in user sees it. The collector has no user to attribute a posting
    --              to, and inventing one would be a lie about where the event came from.
    --   NOT NULL = private to that user. 8d's email producer ALWAYS sets it.
    --
    -- The read path is `user_id IS NULL OR user_id = :me`, so a private event cannot leak by
    -- construction: leaking it would require the email producer to write NULL, which is a
    -- visible bug at the write site rather than a forgotten predicate at the read site.
    --
    -- One consequence, accepted deliberately for a single-user tool: a NULL-user event has
    -- one `acked_at` shared by everyone, so a second registered user acking a posting alert
    -- acks it for you too. Per-user ack state would be a `hunt_event_acks (event_id,
    -- user_id)` join table, and the point at which that becomes worth it is the point at
    -- which a second person actually uses this.
    user_id TEXT REFERENCES users (id),

    -- What real-world thing this event is about, and the idempotency key:
    --
    --   'posting' -> internship_postings.id
    --   'email'   -> email_messages.gmail_message_id   (8d)
    --
    -- Polymorphic, so no REFERENCES clause is possible. Postings are soft-deleted
    -- (`expired_at`), never removed by the sweep, so a posting subject stays resolvable
    -- after it closes.
    subject_id TEXT NOT NULL,

    -- RENDERED, not structured. The extension has one notification path for both kinds and
    -- no per-kind template logic; each producer decides how its own event reads. Same
    -- instinct as the snapshot columns on `internship_applications`: the client renders from
    -- this row alone, with zero joins and nothing to look up.
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    -- Where clicking the notification should go. Nullable: not every future event kind has
    -- somewhere useful to send you.
    url TEXT,

    -- The structured facts behind those two lines (company, tier, term, source), for the
    -- popup and for anything the rendered strings cannot carry.
    payload_json TEXT NOT NULL,

    created_at TEXT NOT NULL,

    -- THIS COLUMN IS RULE 6.
    --
    -- Delivery receipt: set once a client has actually raised a desktop notification for
    -- this event. An MV3 background page is killed and restarted at the browser's
    -- convenience, so anything it remembers in memory is gone and every alert fires again.
    -- `browser.storage.local` is no better as the record — it is per-profile, clearable, and
    -- silently empty in a fresh profile.
    --
    -- So the server holds it. NULL means no client has taken delivery yet; the background
    -- poll asks for exactly those.
    acked_at TEXT,

    -- Notification dedup made STRUCTURAL rather than dependent on the producer getting its
    -- newness check right. Re-running collection over a posting we already alerted on cannot
    -- write a second event even if `upsert_posting`'s "is this new" answer later changes.
    UNIQUE (kind, subject_id)
);

-- The background poll's query, which is the hot one: unacked, newest first.
CREATE INDEX idx_hunt_events_unacked ON hunt_events (created_at) WHERE acked_at IS NULL;
-- The popup's recent-alerts list, which includes acked events.
CREATE INDEX idx_hunt_events_created ON hunt_events (created_at);
