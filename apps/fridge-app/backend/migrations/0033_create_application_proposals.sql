-- Applications the mailbox knows about and the tracker does not.
--
-- Phase 12s. The inbox agent matches mail to an application and proposes a transition; it has
-- never had a path that *creates* one, so a confirmation email for a job you applied to outside
-- the extension is classified, labelled, and then has nowhere to go. Measured on the live
-- mailbox 2026-09-04: 24 confirmations and 4 OA invitations naming ~16 companies, against 2
-- tracked applications. See docs/HUNT.md § "Untracked applications".
--
-- This proposes; it never creates. Accepting is a click, for the same reason
-- INBOX_AUTO_APPLY_CONFIDENCE is unset until Checkpoint 13 measures the classifier — of the 12
-- companies this would have surfaced, one is the generic word `internship`, which
-- data/internships/company-aliases.json already refuses as a company name.
--
-- A separate table rather than a nullable `status_proposals.application_id`: that column is NOT
-- NULL and the review queue's user scoping rides on its join to internship_applications.
-- `fetch_proposals` says outright that widening it would leak other users' rows. So this table
-- carries its own user_id.
CREATE TABLE application_proposals (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id),
    -- The email that caused it. Same reversibility guarantee status_proposals makes: a bad
    -- call can always be traced back to the message that produced it.
    verdict_id TEXT NOT NULL REFERENCES email_verdicts (id),

    -- Normalized by internships::normalize::company_key — the same function the matcher uses,
    -- so "Stripe", "stripe," and "Stripe Inc" collapse the way they do everywhere else. A
    -- second normalizer here would drift from the one that decides whether a match exists.
    company_key TEXT NOT NULL,
    company_name TEXT NOT NULL,
    -- NULL is a legal answer and an honest one. A subject line often names no role, and
    -- inventing "Software Engineer Intern" would put a guess where the panel shows a fact.
    title TEXT,

    -- Terminal statuses are excluded here, not by convention upstream. An email cannot create
    -- an application that already ended: `offer` and `rejected` presuppose a history this row
    -- does not have.
    implied_status TEXT NOT NULL CHECK (implied_status IN ('applied', 'oa', 'interview')),

    reviewed_at TEXT,
    accepted INTEGER CHECK (accepted IN (0, 1)),
    created_at TEXT NOT NULL,

    -- One question per company per user, ever asked once.
    --
    -- `reviewed_at` is deliberately NOT part of this key. Five Stripe confirmations must
    -- produce one proposal rather than five; rejecting means "I did not apply there" and must
    -- not be re-asked on the next email; accepting creates the application, after which the
    -- ordinary matcher finds it and this path never fires for that company again. A partial
    -- index over unreviewed rows would re-ask a question that has already been answered.
    UNIQUE (user_id, company_key)
);

CREATE INDEX idx_application_proposals_open
    ON application_proposals (user_id, created_at)
    WHERE reviewed_at IS NULL;
