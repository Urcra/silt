-- Files of threads this person has looked at (ADR 0067), with the thread's
-- version each was seen at; `pending` until the server has the mark.
CREATE TABLE thread_seen_marks (
    thread_id BLOB NOT NULL,
    path TEXT NOT NULL,
    hash TEXT NOT NULL,
    version TEXT,
    seen INTEGER NOT NULL,
    at_ms INTEGER NOT NULL,
    pending INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (thread_id, path)
);
