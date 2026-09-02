-- Deadlines extracted from mail, and a fourth alert kind to warn about them (Phase 11f).
--
-- # Why `deadline` is its own kind rather than reusing `nudge`
--
-- The extension filters notifications on `kind`, and the options page has a checkbox per kind.
-- A follow-up nudge is chatty and low-stakes; an OA closing in 24 hours is the single thing
-- this tool exists to prevent losing. Filing them under one kind means the checkbox that mutes
-- the chatty one also mutes the important one — and the user who mutes it will be doing so
-- *because* of the chatty one.
--
-- The cost is that SQLite still cannot ALTER a CHECK, so this is the second `hunt_events`
-- rebuild in a week. It repeats migration `0022`'s guarantees exactly, and
-- `hunt_event_rebuild_tests` pins them for this file too: every row, every `acked_at` receipt,
-- `UNIQUE (kind, subject_id)`, and both indexes.
--
-- **Is the kind list now complete?** Probably: `posting`, `email`, `nudge`, `deadline` cover
-- every producer Phases 11–13 plan, and the derived states (dead, converted) are computed at
-- read time and alert nothing. If a fifth is ever needed, the answer is NOT to drop the CHECK —
-- it is what catches a typo'd kind at the write site — but to replace it with a `hunt_event_kinds`
-- lookup table and a foreign key, after which adding a kind is an INSERT rather than a rebuild.
-- Doing that now would be building for a fifth kind nobody has asked for.

CREATE TABLE hunt_events_rebuilt (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('posting', 'email', 'nudge', 'deadline')),
    user_id TEXT REFERENCES users (id),
    subject_id TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    url TEXT,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
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

-- ---------------------------------------------------------------------------------------
-- What was extracted, and from where
-- ---------------------------------------------------------------------------------------

-- One row per message that appears to carry a due date.
--
-- **Every row names the message it came from**, the way `status_proposals` names the verdict
-- that caused it: a deadline you cannot trace back to an email is one you cannot check, and
-- this is data parsed out of untrusted text by patterns that will sometimes be wrong.
CREATE TABLE application_deadlines (
    id TEXT PRIMARY KEY NOT NULL,

    -- The evidence. UNIQUE because re-running extraction over the same mailbox must not
    -- accumulate duplicates — the same structural-idempotency choice as `hunt_events`.
    message_id TEXT NOT NULL UNIQUE REFERENCES email_messages (id),

    user_id TEXT NOT NULL REFERENCES users (id),

    -- RULE 8: nullable on purpose. A pressing email that matches no application is still
    -- pressing, still labelled and still alerted — the matcher is fuzzy and will miss, and a
    -- miss must not swallow an OA deadline.
    application_id TEXT REFERENCES internship_applications (id),

    -- RFC3339 UTC, like every instant in this schema. How a bare date in an email becomes an
    -- instant is documented in `src/inbox/due_dates.rs`, and the choice leans EARLY.
    due_at TEXT NOT NULL,

    -- The phrase that produced it, verbatim. Without this a wrong date is unarguable.
    source_text TEXT NOT NULL,

    created_at TEXT NOT NULL
);

CREATE INDEX idx_deadlines_due ON application_deadlines (due_at);
