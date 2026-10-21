-- Phones paired from a signed-in device or browser (ADR 0028): a one-time
-- code, and the session it starts, named, with what it may do (JSON;
-- NULL for a password's session, which may do everything).
CREATE TABLE phone_pairings (
    code_hash BLOB PRIMARY KEY NOT NULL,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    expires_ms INTEGER NOT NULL
);

ALTER TABLE web_sessions ADD COLUMN name TEXT;
ALTER TABLE web_sessions ADD COLUMN capabilities TEXT;
ALTER TABLE web_sessions ADD COLUMN created_ms INTEGER NOT NULL DEFAULT 0;

-- What the sender may do, fixed when it was relayed.
ALTER TABLE control_commands ADD COLUMN capabilities TEXT;
