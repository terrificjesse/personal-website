-- Phase 8e: a credential the Firefox extension can actually send.
--
-- The extension was meant to reuse the `fridge_session` cookie, and that was tried first as
-- `apps/hunt-extension/CLAUDE.md` instructs. It does not work: the cookie is `SameSite=Lax`
-- and a request from a `moz-extension://` page is cross-site, so Firefox never attaches it.
-- The backend saw an anonymous request and answered 401, correctly, forever. This is the
-- fallback that same file names.
--
-- Timestamps are RFC3339 TEXT and ids are TEXT UUIDs, as everywhere else here.

CREATE TABLE hunt_tokens (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id),

    -- SHA-256 of the token, never the token itself. Same treatment as `sessions.token_hash`
    -- and computed by the same `auth::session_token_hash`, so there is one definition of
    -- "hash a bearer credential" in this codebase rather than two that can drift.
    --
    -- SHA-256 rather than Argon2 is right *here* and would be wrong for a password: the input
    -- is 32 bytes of CSPRNG output that only this backend ever generated, so there is no
    -- low-entropy guess to slow down.
    token_hash TEXT NOT NULL UNIQUE,

    -- Which device or browser profile this belongs to. Free text, shown when revoking — a
    -- list of indistinguishable tokens is a list nobody dares revoke anything from.
    label TEXT NOT NULL,

    created_at TEXT NOT NULL,
    -- Bumped on use, so an unused token is visibly unused before you delete it.
    last_used_at TEXT,

    -- NO EXPIRY COLUMN, deliberately. A session expires because it rides in a browser the user
    -- walks away from; this is a device credential that would silently stop a background
    -- notifier weeks later, and the failure would look exactly like a quiet job market. So the
    -- control is revocation, which is explicit and visible, rather than a clock nobody is
    -- watching.
    revoked_at TEXT
);

CREATE INDEX idx_hunt_tokens_user ON hunt_tokens (user_id, created_at);
