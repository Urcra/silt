-- Read-only links to one thread (ADR 0027). A frozen link shows the
-- thread's family as it was at `frozen_at`, the count of deltas then.
CREATE TABLE thread_links (
    id INTEGER PRIMARY KEY,
    token_hash BLOB NOT NULL UNIQUE,
    thread_id BLOB NOT NULL,
    owner_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_ms INTEGER NOT NULL,
    expires_ms INTEGER NOT NULL,
    frozen_at INTEGER,
    revoked_ms INTEGER
);

CREATE INDEX thread_links_thread ON thread_links (thread_id);

-- Whether an org's projects may have links, and when they were last turned
-- off: links made before then stay stopped.
ALTER TABLE orgs ADD COLUMN links INTEGER NOT NULL DEFAULT 1;
ALTER TABLE orgs ADD COLUMN links_off_ms INTEGER NOT NULL DEFAULT 0;
