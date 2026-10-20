-- Accounts (ADR 0013): passwords, sign-in identities, browser sessions,
-- device sign-in requests and invites. The first user, from before
-- accounts existed, administers the server.
ALTER TABLE users ADD COLUMN display_name TEXT;
ALTER TABLE users ADD COLUMN email TEXT;
ALTER TABLE users ADD COLUMN password_hash TEXT;
ALTER TABLE users ADD COLUMN admin INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN created_ms INTEGER NOT NULL DEFAULT 0;
UPDATE users SET admin = 1 WHERE id = 1;

CREATE TABLE identities (
    provider TEXT NOT NULL,
    subject TEXT NOT NULL,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (provider, subject)
);

CREATE TABLE web_sessions (
    token_hash BLOB PRIMARY KEY NOT NULL,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    expires_ms INTEGER NOT NULL
);

CREATE TABLE device_requests (
    code_hash BLOB PRIMARY KEY NOT NULL,
    user_code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    expires_ms INTEGER NOT NULL,
    user_id INTEGER REFERENCES users (id) ON DELETE CASCADE
);

CREATE TABLE invites (
    token_hash BLOB PRIMARY KEY NOT NULL,
    created_by INTEGER REFERENCES users (id) ON DELETE SET NULL,
    org TEXT,
    expires_ms INTEGER NOT NULL,
    used_by INTEGER REFERENCES users (id) ON DELETE SET NULL
);

-- An OAuth or OIDC sign-in under way: what to check and where to go next.
CREATE TABLE sign_in_states (
    state_hash BLOB PRIMARY KEY NOT NULL,
    provider TEXT NOT NULL,
    verifier TEXT NOT NULL,
    invite TEXT,
    next TEXT NOT NULL,
    expires_ms INTEGER NOT NULL
);
