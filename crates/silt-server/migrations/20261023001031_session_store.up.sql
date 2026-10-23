-- Agent SDK sessions kept as threads (ADR 0042): each person's sessions by
-- the SDK's project key and session id.
CREATE TABLE sdk_sessions (
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    project_key TEXT NOT NULL,
    session_id TEXT NOT NULL,
    family_id BLOB NOT NULL,
    thread_id BLOB NOT NULL UNIQUE,
    mtime_ms INTEGER NOT NULL,
    PRIMARY KEY (user_id, project_key, session_id)
);

-- The entries recorded so far by their uuid, per transcript ('' for the
-- main one, else the subagent's name), so a retried append adds nothing.
CREATE TABLE sdk_entries (
    thread_id BLOB NOT NULL REFERENCES sdk_sessions (thread_id) ON DELETE CASCADE,
    file TEXT NOT NULL,
    uuid TEXT NOT NULL,
    PRIMARY KEY (thread_id, file, uuid)
);

-- How many of a subagent transcript's entries a delete hid: the log only
-- grows, so a delete hides what was there and later appends show again.
CREATE TABLE sdk_cleared (
    thread_id BLOB NOT NULL REFERENCES sdk_sessions (thread_id) ON DELETE CASCADE,
    file TEXT NOT NULL,
    hidden INTEGER NOT NULL,
    PRIMARY KEY (thread_id, file)
);
