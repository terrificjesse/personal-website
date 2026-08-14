-- Phase 5 authentication. See docs/PLAN.md.
--
-- Three tables rather than one: an account, the ways you can prove you own it, and the
-- sessions that prove you already did. Keeping them apart is what makes "connect Google as
-- an *alternate* login method" (the PLAN.md checkpoint) a row insert rather than a schema
-- change.
--
-- Note SQLite does not enforce `REFERENCES` unless `PRAGMA foreign_keys = ON` is set per
-- connection, which `db::init_pool` does not currently do. The clauses below are declared
-- for documentation and so enabling the pragma later is a one-line change, not a migration.

CREATE TABLE users (
    id TEXT PRIMARY KEY NOT NULL,
    -- Stored lowercased and trimmed; the app normalizes before writing. UNIQUE is what makes
    -- "an account with this email already exists" a database guarantee rather than a race
    -- between two concurrent registrations that both passed a SELECT check.
    email TEXT NOT NULL UNIQUE,
    -- NULL is legal and meaningful: an account created through Google has no password until
    -- the user sets one. Password login must therefore treat NULL as "no password login for
    -- this account", never as "any password matches".
    password_hash TEXT,
    created_at TEXT NOT NULL
);

-- One row per external identity linked to an account. Separate from `users` so a single
-- account can accumulate several (Google today, anything else later) without widening the
-- users table each time.
CREATE TABLE oauth_identities (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id),
    -- 'google' for now. Text rather than an enum column so adding a provider is data, not DDL.
    provider TEXT NOT NULL,
    -- The provider's own stable id for the account — Google's `sub` claim, **not** the email.
    -- Emails at Google can change and be reassigned; `sub` cannot, which is why linking on
    -- email is the classic account-takeover bug here.
    provider_account_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    -- One external identity maps to at most one local account.
    UNIQUE (provider, provider_account_id)
);

-- Server-side sessions: the cookie carries an opaque random token, and everything about the
-- session lives here where it can be revoked. Chosen over a signed JWT specifically so
-- logout can actually end a session rather than merely asking the client to forget it.
CREATE TABLE sessions (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id),
    -- A **hash** of the session token, never the token itself. The cookie holds the only
    -- copy of the real token; this column is what a lookup compares against. If the database
    -- leaks, the attacker gets hashes rather than a set of live, ready-to-use sessions —
    -- the same reasoning that applies to `users.password_hash`.
    --
    -- UNIQUE because the lookup is by this column and two rows sharing one would make
    -- "which session is this?" ambiguous.
    token_hash TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    -- Absolute expiry, checked on every validation. Stored rather than derived so shortening
    -- the session lifetime later doesn't retroactively extend or revoke sessions already out
    -- in the wild.
    expires_at TEXT NOT NULL
);

CREATE INDEX idx_sessions_user_id ON sessions (user_id);
CREATE INDEX idx_oauth_identities_user_id ON oauth_identities (user_id);
