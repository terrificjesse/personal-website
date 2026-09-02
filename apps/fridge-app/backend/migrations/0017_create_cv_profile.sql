-- Phase 8f: the CV details the extension fills into ATS forms.
--
-- In the backend rather than the extension because it survives a browser reinstall, and
-- because it is the same SQLite file that already holds this user's password hash — so it is
-- not a new class of secret, just more of one that already exists. The extension caches it in
-- `browser.storage.local` so filling works offline.
--
-- Timestamps are RFC3339 TEXT and ids are TEXT UUIDs, as everywhere else here.

-- One row per user, so `user_id` IS the primary key. There is no "which profile" question to
-- answer and no second row to accidentally read.
CREATE TABLE cv_profile (
    user_id TEXT PRIMARY KEY NOT NULL REFERENCES users (id),

    -- EVERY FIELD IS NULLABLE, and that is the design rather than laziness.
    --
    -- A half-filled profile is the normal state — you fill in what an application asked for
    -- last time — and the autofill has to be able to tell "I have no phone number for you"
    -- from "your phone number is the empty string". The second would type a blank into a
    -- required field and let you submit a form you thought was complete. Same rule as
    -- `internship_postings.pay_min`: absent is not zero, one subsystem over.
    full_name TEXT,
    -- Some forms ask for these separately and splitting a full name on whitespace is how you
    -- get "Van Der Berg" wrong. Stored as asked for.
    first_name TEXT,
    last_name TEXT,
    preferred_name TEXT,

    email TEXT,
    phone TEXT,
    -- One string, as a form asks for it. Parsing it into city/region/country would be inventing
    -- structure no ATS field wants back.
    location TEXT,

    school TEXT,
    degree TEXT,
    major TEXT,
    gpa TEXT,
    graduation_month INTEGER CHECK (graduation_month BETWEEN 1 AND 12),
    graduation_year INTEGER,

    github_url TEXT,
    linkedin_url TEXT,
    portfolio_url TEXT,

    -- Free text, because the honest answers ("US citizen", "F-1 with OPT eligibility") do not
    -- fit an enum and getting this wrong on an application is expensive.
    work_authorization TEXT,
    -- Three-state on purpose: NULL = not stated, 0 = no, 1 = yes. Defaulting to 0 would answer
    -- a legally meaningful question on the user's behalf, which is the same mistake as
    -- defaulting `is_remote` to false, with worse consequences.
    needs_sponsorship INTEGER CHECK (needs_sponsorship IN (0, 1)),

    -- A REMINDER, NEVER AN UPLOAD. File inputs cannot be populated programmatically in any way
    -- that is both reliable and honest, so the extension shows this path and the user picks the
    -- file themselves. Synthesising a DataTransfer to fake a file selection is out of scope by
    -- decision, not by omission.
    resume_path TEXT,

    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- NO EEO / DEMOGRAPHIC COLUMNS, DELIBERATELY.
--
-- Rule 10 says race, gender, veteran and disability questions are opt-in and default off. The
-- strongest form of "default off" is having nothing to fill: data that is not stored cannot be
-- typed into a form by a bug, a bad label match, or a future refactor that forgets the flag.
-- If these are ever wanted they get their own table and their own explicit opt-in, so the
-- decision is visible in the schema rather than buried in a boolean.
