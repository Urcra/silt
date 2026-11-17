-- The artifacts each person pinned to the top of their Artifacts page
-- (ADR 0075). A person's own, never synced into a family's log.
CREATE TABLE artifact_pins (
    user_id INTEGER NOT NULL REFERENCES users (id),
    thread_id BLOB NOT NULL,
    artifact_id TEXT NOT NULL,
    at_ms INTEGER NOT NULL,
    PRIMARY KEY (user_id, thread_id, artifact_id)
);
