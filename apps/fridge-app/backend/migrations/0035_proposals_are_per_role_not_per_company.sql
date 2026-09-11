-- A repeat application to the same company is a separate application.
--
-- Phase 12s, correcting 0033's own key. That table carried `UNIQUE (user_id, company_key)` with
-- a comment defending it: "Five Stripe confirmations must produce one proposal rather than
-- five." That was the right observation about the wrong unit. Those five Stripe emails were
-- about ONE application. Microsoft sent NINE confirmations for nine different roles, and under a
-- per-company key eight of them can never be proposed at all.
--
-- Measured 2026-09-11 on the live mailbox: nine Microsoft confirmations, nine distinct roles
-- ("Software Engineer: AI/ML & LLM Intern", "… Data Platform/Analytics", "… Cloud & Distributed
-- Backend", "Firmware Engineering INTERN", and more), against ONE tracked Microsoft application.
-- The user reported the tracker holding 14 entries "when there should be far more"; this is half
-- the reason. The other half was a classifier that filed all nine as `disregarded`.
--
-- The unit is a (company, role) pair. `role_key` is the normalized role, or the EMPTY STRING
-- when the email names none — and empty is what preserves the original intent: an email that
-- names no role is not evidence of a *different* application, so every roleless autoresponder at
-- one company still collapses to a single proposal. Stripe stays one. Microsoft becomes nine.
--
-- SQLite cannot alter a UNIQUE constraint, so the table is rebuilt. Existing rows keep their
-- answers: a company already accepted or rejected stays that way, and the new column defaults to
-- the empty string, which is exactly "we did not know the role when we asked".

CREATE TABLE application_proposals_new (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id),
    verdict_id TEXT NOT NULL REFERENCES email_verdicts (id),

    company_key TEXT NOT NULL,
    company_name TEXT NOT NULL,
    title TEXT,

    -- The normalized role, '' when the email named none. Part of the key; `title` is what is
    -- shown and may be spelled differently across two emails about the same opening.
    role_key TEXT NOT NULL DEFAULT '',

    implied_status TEXT NOT NULL CHECK (implied_status IN ('applied', 'oa', 'interview')),

    reviewed_at TEXT,
    accepted INTEGER CHECK (accepted IN (0, 1)),
    created_at TEXT NOT NULL,

    -- One question per (company, role), still asked once and never re-asked after an answer.
    UNIQUE (user_id, company_key, role_key)
);

INSERT INTO application_proposals_new
    (id, user_id, verdict_id, company_key, company_name, title, role_key,
     implied_status, reviewed_at, accepted, created_at)
SELECT id, user_id, verdict_id, company_key, company_name, title, '',
       implied_status, reviewed_at, accepted, created_at
  FROM application_proposals;

DROP TABLE application_proposals;
ALTER TABLE application_proposals_new RENAME TO application_proposals;

CREATE INDEX idx_application_proposals_open
    ON application_proposals (user_id, created_at)
    WHERE reviewed_at IS NULL;
