-- Phase 8a–8d: the inbox agent. See `apps/hunt-extension/CLAUDE.md`.
--
-- Five tables. Timestamps are RFC3339 TEXT and ids are TEXT UUIDs, as everywhere else here.
--
-- The shape follows one structural idea: **the four email categories already exist in the
-- database.** Phase 7 shipped `internship_applications.status` — applied → oa → interview →
-- offer → rejected — which is exactly "confirmation folder / OA folder / interview folder".
-- So the classifier's job is not "sort this into a folder", it is *match this email to an
-- application and propose a status transition*. Gmail labels are written afterwards as a
-- projection of application status. Built the other way round you get two taxonomies that
-- drift, and a tracker still reading `applied` for a job you already interviewed at.

-- ---------------------------------------------------------------------------------------
-- The connected account
-- ---------------------------------------------------------------------------------------

CREATE TABLE gmail_accounts (
    user_id TEXT PRIMARY KEY NOT NULL REFERENCES users (id),
    email TEXT NOT NULL,

    -- RULE 9. The refresh token lives here and never in a file the frontend or the extension
    -- can read, and never in the extension at all.
    --
    -- Stated plainly rather than left implied: this is a LIVE CREDENTIAL in plaintext, unlike
    -- `users.password_hash` next to it, which is a hash and useless if read. The security
    -- boundary is the database file itself. That is the same boundary the rest of this
    -- application already relies on, and it is worth knowing rather than discovering.
    refresh_token TEXT NOT NULL,

    -- Gmail's incremental cursor. NULL before the first full pass; after that, sync uses
    -- `history.list` from here rather than re-listing the mailbox (rule 4).
    history_id TEXT,

    connected_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- ---------------------------------------------------------------------------------------
-- Run records — RULE 5
-- ---------------------------------------------------------------------------------------

-- Mirrors `source_runs` deliberately: an expired refresh token must not look like a quiet
-- inbox. **A run that classified zero emails must be distinguishable from a run that failed
-- to authenticate**, which means the outcome and the error are recorded, not inferred from
-- the counts being zero.
CREATE TABLE inbox_runs (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id),
    started_at TEXT NOT NULL,
    finished_at TEXT,

    --   'success'   — the sync completed and every message it fetched was classified.
    --   'partial'   — some messages were fetched and something stopped it early.
    --   'failed'    — nothing usable. Auth expired, quota, network, reshaped response.
    --   'skipped'   — deliberately not run: no account connected, or disabled.
    outcome TEXT NOT NULL CHECK (outcome IN ('success', 'partial', 'failed', 'skipped')),
    error TEXT,

    fetched_count INTEGER NOT NULL DEFAULT 0,
    classified_count INTEGER NOT NULL DEFAULT 0,

    -- RULE 7: counted separately, never summed into one number.
    --
    -- The disregard branch is about to become the highest-volume path in the system. If a
    -- dropped email leaves no trace then "correctly ignored 400 newsletters" and "broken and
    -- ate an OA" produce identical output: a quiet inbox. This is `source_runs`'
    -- fetched = accepted + filtered + rejected, one subsystem over, and the invariant to pin
    -- with a test is:
    --
    --   classified = pressing + confirmation + outreach + disregarded
    pressing_count INTEGER NOT NULL DEFAULT 0,
    confirmation_count INTEGER NOT NULL DEFAULT 0,
    outreach_count INTEGER NOT NULL DEFAULT 0,
    disregarded_count INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_inbox_runs_user ON inbox_runs (user_id, started_at);

-- ---------------------------------------------------------------------------------------
-- Messages
-- ---------------------------------------------------------------------------------------

-- **Store the minimum.** This is a burner account, but it is still your mail: enough to
-- classify, match and show you what happened, and no body text.
CREATE TABLE email_messages (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id),

    -- RULE 4. Reprocessing a message is a NO-OP, not a second label write and a second
    -- notification. UNIQUE is what makes that a storage guarantee rather than application
    -- logic a future sync could forget.
    gmail_message_id TEXT NOT NULL UNIQUE,
    gmail_thread_id TEXT,

    from_address TEXT,
    subject TEXT,
    received_at TEXT,
    snippet TEXT,

    created_at TEXT NOT NULL
);

CREATE INDEX idx_email_messages_user ON email_messages (user_id, received_at);

-- ---------------------------------------------------------------------------------------
-- Verdicts — RULES 1, 7 and 8
-- ---------------------------------------------------------------------------------------

-- One row per classification pass, KEPT EVEN WHEN SUPERSEDED, so a bad call is diagnosable
-- rather than merely wrong. Same instinct as `posting_rejects`.
CREATE TABLE email_verdicts (
    id TEXT PRIMARY KEY NOT NULL,
    message_id TEXT NOT NULL REFERENCES email_messages (id),

    -- RULE 7: written for EVERY message, including disregarded ones. Disregarded means
    -- unlabelled, not unrecorded.
    category TEXT NOT NULL CHECK (category IN (
        'confirmation', 'oa', 'interview', 'offer', 'rejection', 'outreach', 'disregarded'
    )),
    confidence REAL,

    -- RULE 8. The match is ENRICHMENT, not a gate, and NULL is legal on a pressing category:
    -- the matcher is fuzzy and will miss, and if "unmatched" routed to disregard then one
    -- matcher miss silently eats an interview invite. Category is decided from the email
    -- alone, and only then is a match attempted.
    matched_application_id TEXT REFERENCES internship_applications (id),

    -- Which layer decided. The rules layer handles ~80% of this mail; the model is the
    -- fallback on ambiguity, and knowing which one was responsible is how you tell a bad rule
    -- from a bad prompt.
    classifier TEXT NOT NULL CHECK (classifier IN ('rules', 'llm')),

    -- RULE 1: email is untrusted content. If a body contains text addressed at the agent,
    -- that is data to classify and worth surfacing here — never an instruction.
    evidence TEXT,

    created_at TEXT NOT NULL
);

CREATE INDEX idx_verdicts_message ON email_verdicts (message_id, created_at);
CREATE INDEX idx_verdicts_category ON email_verdicts (category, created_at);

-- ---------------------------------------------------------------------------------------
-- Status proposals — RULE 2
-- ---------------------------------------------------------------------------------------

-- THE TRAP THIS TABLE EXISTS FOR: a misclassification must never silently rewrite the
-- tracker. Phase 7 made `status_changed_at` load-bearing — "how long have I been at this
-- stage" — and a false positive that flips applied → rejected destroys real state with no
-- record of why.
--
-- **The link from the change back to the email is what makes it reversible.**
CREATE TABLE status_proposals (
    id TEXT PRIMARY KEY NOT NULL,
    application_id TEXT NOT NULL REFERENCES internship_applications (id),
    verdict_id TEXT NOT NULL REFERENCES email_verdicts (id),

    from_status TEXT NOT NULL,
    to_status TEXT NOT NULL,

    -- Auto-apply only above the confidence threshold and only for FORWARD transitions, and
    -- never for `offer` or `rejected` — they end the story, so they always queue for review.
    applied_automatically INTEGER NOT NULL DEFAULT 0
        CHECK (applied_automatically IN (0, 1)),
    -- NULL until a human accepts or rejects it. An auto-applied proposal is still reviewable.
    reviewed_at TEXT,
    accepted INTEGER CHECK (accepted IN (0, 1)),

    created_at TEXT NOT NULL
);

CREATE INDEX idx_proposals_application ON status_proposals (application_id, created_at);
CREATE INDEX idx_proposals_pending ON status_proposals (created_at) WHERE reviewed_at IS NULL;
