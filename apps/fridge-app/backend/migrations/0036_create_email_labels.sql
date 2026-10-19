-- Hand labels for measuring the email classifier, entered one message at a time on
-- /internships/labelling (Phase 13b).
--
-- A table rather than the CSV `labelset export` writes, because labelling now happens in a
-- browser and has to survive being stopped halfway. The CSV is still the measurement artifact:
-- `labelset export-labels` writes these rows into one, and `labelset score` grades it exactly as
-- before, ledger and fingerprint included. This table is where labels are collected, not where
-- they are graded.
--
-- Deliberately NO column for what the classifier said. The labelling sheet never had one
-- either (`labelset.rs`, "anchoring"): a person shown the machine's answer agrees with it, and
-- the measurement then grades the classifier against itself.
CREATE TABLE email_labels (
    message_id  TEXT PRIMARY KEY NOT NULL REFERENCES email_messages (id),
    user_id     TEXT NOT NULL REFERENCES users (id),
    -- NULL means "looked at it and could not tell". Recorded so the message is not shown again,
    -- and exported as a blank label, which `labelset score` excludes rather than counts.
    label       TEXT CHECK (label IS NULL OR label IN (
                    'confirmation', 'oa', 'interview', 'offer',
                    'rejection', 'outreach', 'disregarded')),
    labelled_at TEXT NOT NULL
);

CREATE INDEX idx_email_labels_user ON email_labels (user_id);
