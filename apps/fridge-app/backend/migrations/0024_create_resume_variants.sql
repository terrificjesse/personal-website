-- Which résumé went with which application (Phase 12f).
--
-- A variant is a LABEL for a résumé the user maintains outside this app, never the file. The
-- reasoning is in `docs/HUNT.md` § Résumé variants and is worth not re-deciding casually: a
-- résumé is the most identifying document most people own, this database already holds a Gmail
-- refresh token, and its backups are already credential material. Storing the PDF raises what
-- a leak costs without changing what this answers.

CREATE TABLE resume_variants (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id),

    -- What the user calls it: "one-page, systems". Unique per user because two variants
    -- sharing a name makes every report that groups by variant ambiguous, and the ambiguity
    -- would appear months later in the one number this feature exists to produce.
    label TEXT NOT NULL,

    -- What is different about it. Free text, never parsed.
    notes TEXT,

    created_at TEXT NOT NULL,

    -- Retiring is this column. **Never a DELETE**: a retired résumé is exactly the thing you
    -- want to compare a new one against, and deleting it would remove the evidence for the
    -- comparison. Archived variants stop being offered for new applications and keep appearing
    -- in every report covering a window they were used in.
    archived_at TEXT,

    UNIQUE (user_id, label)
);

CREATE INDEX idx_resume_variants_user ON resume_variants (user_id, archived_at);

-- **Nullable, and deliberately not backfilled.** Every application that already exists was sent
-- with a résumé nobody recorded. Assigning them a guess — the oldest variant, the most used —
-- would put invented data into the only number this feature produces, and it would be
-- indistinguishable from measurement. They stay unattributed, and reports must give them their
-- own bucket rather than dropping them from the denominator.
--
-- The reference is to the ID, not the label. That is what makes renaming a variant an UPDATE
-- rather than an event that orphans every application which used it.
ALTER TABLE internship_applications
    ADD COLUMN resume_variant_id TEXT REFERENCES resume_variants (id);
