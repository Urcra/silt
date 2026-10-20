CREATE TABLE families (
    id BLOB PRIMARY KEY NOT NULL,
    created_ms INTEGER NOT NULL
);

-- Every delta of a family, in the order this replica applied them.
CREATE TABLE deltas (
    family_id BLOB NOT NULL REFERENCES families (id),
    seq INTEGER NOT NULL,
    actor BLOB NOT NULL,
    lamport INTEGER NOT NULL,
    len INTEGER NOT NULL,
    data BLOB NOT NULL,
    PRIMARY KEY (family_id, seq),
    UNIQUE (family_id, actor, lamport)
);

CREATE TABLE threads (
    id BLOB PRIMARY KEY NOT NULL,
    family_id BLOB NOT NULL REFERENCES families (id),
    repo_root TEXT NOT NULL,
    created_ms INTEGER NOT NULL
);

CREATE TABLE checkouts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id BLOB NOT NULL REFERENCES threads (id),
    path TEXT NOT NULL UNIQUE,
    actor BLOB NOT NULL,
    managed INTEGER NOT NULL,
    created_ms INTEGER NOT NULL
);

-- What each checkout last read from or wrote to disk, per file.
CREATE TABLE disk_files (
    checkout_id INTEGER NOT NULL REFERENCES checkouts (id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    hash BLOB NOT NULL,
    size INTEGER NOT NULL,
    mtime_ns INTEGER NOT NULL,
    version BLOB NOT NULL,
    PRIMARY KEY (checkout_id, path)
);

CREATE TABLE sessions (
    agent TEXT NOT NULL,
    session_id TEXT NOT NULL,
    thread_id BLOB NOT NULL REFERENCES threads (id),
    checkout_id INTEGER NOT NULL REFERENCES checkouts (id),
    created_ms INTEGER NOT NULL,
    PRIMARY KEY (agent, session_id)
);

-- How far each session file has been read.
CREATE TABLE session_files (
    agent TEXT NOT NULL,
    session_id TEXT NOT NULL,
    file TEXT NOT NULL,
    path TEXT NOT NULL,
    offset INTEGER NOT NULL,
    PRIMARY KEY (agent, session_id, file),
    FOREIGN KEY (agent, session_id) REFERENCES sessions (agent, session_id)
);

CREATE VIRTUAL TABLE search USING fts5 (
    body,
    thread_id UNINDEXED,
    actor UNINDEXED,
    lamport UNINDEXED,
    kind UNINDEXED,
    tokenize = 'unicode61'
);
