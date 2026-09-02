-- Phase 8g: the answer library. Questions you have already answered well, so the next form
-- asking the same thing is a choice rather than a rewrite.
--
-- Timestamps are RFC3339 TEXT and ids are TEXT UUIDs, as everywhere else here.

CREATE TABLE application_answers (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id),

    -- The question as the form asked it, kept verbatim so you can recognise it later, and a
    -- normalized form for matching. Both, because the one you read and the one the matcher
    -- compares are different jobs — the same split as `company_name`/`company_key` next door.
    question_text TEXT NOT NULL,
    question_normalized TEXT NOT NULL,

    answer_text TEXT NOT NULL,

    -- THIS COLUMN IS THE TRAP THIS TABLE EXISTS AROUND.
    --
    -- "Why do you want to work at X" is near-identical across applications by every measure a
    -- similarity score can see, and is the single worst thing to reuse verbatim. Pasting
    -- "I'm excited about Stripe's mission" into a Datadog form is a uniquely bad way to lose
    -- an application — worse than an empty box, because you will not notice.
    --
    -- Set when the question is inherently about the employer, or when a company is named in
    -- the question or the answer. Flagged generously on purpose: a false positive costs one
    -- suggestion, a false negative costs the application.
    is_company_specific INTEGER NOT NULL DEFAULT 0 CHECK (is_company_specific IN (0, 1)),

    -- Who it was written for, when known. NULL means "not written for anyone in particular",
    -- which is what makes a company-specific answer safe to offer back for the SAME company
    -- and never for a different one.
    company_name TEXT,

    -- Free text, comma-separated. Yours to organise by; nothing reads them but you.
    tags TEXT,

    -- Bumped when you actually use an answer, not when it is merely shown. A suggestion you
    -- ignored is not evidence the answer is good.
    use_count INTEGER NOT NULL DEFAULT 0,
    last_used_at TEXT,

    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_answers_user ON application_answers (user_id, updated_at);

-- The previous text of an answer, written every time one is edited.
--
-- You improve an answer over time and want the current one — but a rewrite you regret should
-- be recoverable, and "I had a better version of this two months ago" is a real and
-- unrecoverable loss otherwise. Cheap: a few hundred bytes per edit.
CREATE TABLE answer_revisions (
    id TEXT PRIMARY KEY NOT NULL,
    answer_id TEXT NOT NULL REFERENCES application_answers (id),
    -- The text as it was BEFORE the edit that created this row.
    answer_text TEXT NOT NULL,
    replaced_at TEXT NOT NULL
);

CREATE INDEX idx_answer_revisions ON answer_revisions (answer_id, replaced_at);
