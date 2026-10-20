-- Derived from the deltas and rebuilt on every start (ADR 0011).
CREATE VIRTUAL TABLE search USING fts5(
    body,
    thread_id UNINDEXED,
    kind UNINDEXED,
    time_ms UNINDEXED,
    tokenize = 'unicode61'
);

CREATE TABLE thread_activity (
    thread_id BLOB PRIMARY KEY NOT NULL,
    branch TEXT,
    last_ms INTEGER NOT NULL DEFAULT 0,
    merged INTEGER NOT NULL DEFAULT 0
);

-- The paths each thread changed, as named when it changed them.
CREATE TABLE touches (
    thread_id BLOB NOT NULL,
    path TEXT NOT NULL,
    last_ms INTEGER NOT NULL,
    PRIMARY KEY (thread_id, path)
);
