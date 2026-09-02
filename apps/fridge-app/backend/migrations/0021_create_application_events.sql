-- Phase 10: append-only application status history. The mutable status column remains as a
-- read cache; application_events is the audit log it must fold from.

CREATE TABLE application_events (
    id TEXT PRIMARY KEY NOT NULL,
    application_id TEXT NOT NULL REFERENCES internship_applications (id),
    at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    from_status TEXT,
    to_status TEXT NOT NULL
        CHECK (to_status IN ('applied', 'oa', 'interview', 'offer', 'rejected')),
    actor TEXT NOT NULL
        CHECK (actor IN ('email', 'extension', 'manual', 'sweep', 'unknown')),
    cause_kind TEXT
        CHECK (cause_kind IN ('status_proposal', 'email_verdict', 'hunt_event')),
    cause_id TEXT,
    note TEXT,
    UNIQUE (application_id, cause_kind, cause_id, to_status)
);

CREATE INDEX idx_application_events_app ON application_events (application_id, at);
CREATE INDEX idx_application_events_at ON application_events (at);
