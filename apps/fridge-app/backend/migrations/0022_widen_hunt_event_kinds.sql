-- Admit a third alert kind, `nudge` (Phase 11e).
--
-- **SQLite cannot ALTER a CHECK constraint**, so widening `kind` is the full table rebuild —
-- create, copy, drop, rename — and it is being done on the alert channel itself. Three things
-- therefore matter more here than in an ordinary migration:
--
-- 1. **`acked_at` must survive exactly.** It is a delivery receipt, and it is the only thing
--    stopping every historical alert from being raised again the next time the extension polls.
--    An MV3 background page remembers nothing across restarts, so if this column is lost the
--    user gets 60-odd notifications at once and mutes the channel — taking the OA alerts with
--    it. `hunt_event_rebuild_tests` pins the row count and the non-null `acked_at` count.
-- 2. **`UNIQUE (kind, subject_id)` must come back.** It is what makes "one alert per subject"
--    a property of the schema rather than of a caller remembering to check. A rebuild that
--    quietly drops it converts structural dedup into a convention.
-- 3. **Both indexes must come back** — dropping the table drops them, and nothing else in the
--    codebase would notice until the poll got slow.
--
-- The INSERT names its columns rather than using `SELECT *`: a rebuild that depends on column
-- order is one reordered column away from silently writing the wrong data into the right shape.

CREATE TABLE hunt_events_rebuilt (
    id TEXT PRIMARY KEY NOT NULL,

    -- The widened constraint, and the only reason this file exists. `nudge` is written by the
    -- follow-up sweep: an application that has had no response for N days. It is a third
    -- PRODUCER on the same table, poll and notification path — not a second pipeline.
    kind TEXT NOT NULL CHECK (kind IN ('posting', 'email', 'nudge')),

    -- NULL = from the shared posting corpus, visible to every signed-in user.
    -- NOT NULL = private to that user. The email and nudge producers must always set it.
    user_id TEXT REFERENCES users (id),

    -- What the event is about, and the idempotency key. A posting id, a Gmail message id, or
    -- for a nudge `"{application_id}:{threshold_days}"` — because a nudge at 14 days and one
    -- at 30 are different events, while one keyed on the application alone fires once ever.
    subject_id TEXT NOT NULL,

    title TEXT NOT NULL,
    body TEXT NOT NULL,
    url TEXT,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,

    -- A delivery receipt, not a user dismissal. See the note at the top of this file.
    acked_at TEXT,

    UNIQUE (kind, subject_id)
);

INSERT INTO hunt_events_rebuilt
    (id, kind, user_id, subject_id, title, body, url, payload_json, created_at, acked_at)
SELECT
     id, kind, user_id, subject_id, title, body, url, payload_json, created_at, acked_at
  FROM hunt_events;

DROP TABLE hunt_events;

ALTER TABLE hunt_events_rebuilt RENAME TO hunt_events;

CREATE INDEX idx_hunt_events_unacked ON hunt_events (created_at) WHERE acked_at IS NULL;
CREATE INDEX idx_hunt_events_created ON hunt_events (created_at);
