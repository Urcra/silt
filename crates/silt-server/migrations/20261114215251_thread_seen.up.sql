-- Files of a thread each person has looked at (ADR 0067), as `seen` keeps
-- a merge request's, with the thread's version each was seen at.
CREATE TABLE thread_seen (
    user_id INTEGER NOT NULL REFERENCES users (id),
    thread_id BLOB NOT NULL,
    path TEXT NOT NULL,
    hash TEXT NOT NULL,
    version TEXT,
    seen INTEGER NOT NULL,
    at_ms INTEGER NOT NULL,
    PRIMARY KEY (user_id, thread_id, path)
);
