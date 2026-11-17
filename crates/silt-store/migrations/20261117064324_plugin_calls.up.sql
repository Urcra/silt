-- Each call a sandboxed run made to a plugin (ADR 0086): what Silt sent or
-- would send, as its canonical request, and how it ended. One that asked
-- the person is an approval, answered by the request's hash.
CREATE TABLE plugin_calls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id BLOB NOT NULL,
    -- The hash of the token of the run that made it.
    run_hash BLOB NOT NULL,
    workspace TEXT NOT NULL,
    -- The canonical request, as JSON.
    request TEXT NOT NULL,
    hash TEXT NOT NULL,
    -- Whether it may change something, rather than only read.
    change INTEGER NOT NULL,
    -- Whether it waited for the person.
    asked INTEGER NOT NULL,
    -- pending, running, rejected, expired or ended.
    state TEXT NOT NULL,
    -- How it ended, as JSON; none while pending or running.
    outcome TEXT,
    -- The grant it ran under, if it asked no one because of one.
    grant_id INTEGER,
    created_ms INTEGER NOT NULL,
    answered_ms INTEGER
);
CREATE INDEX plugin_calls_thread ON plugin_calls (thread_id, id);

-- What the person allowed beyond one call: a plugin's environment's reads
-- or changes, for a run or a thread, until a time. A fork starts with none.
CREATE TABLE plugin_grants (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id BLOB NOT NULL,
    -- The run's token hash, for a grant to that run only.
    run_hash BLOB,
    plugin TEXT NOT NULL,
    environment TEXT NOT NULL,
    change INTEGER NOT NULL,
    created_ms INTEGER NOT NULL,
    until_ms INTEGER,
    revoked_ms INTEGER,
    -- The approval it was given with.
    from_call INTEGER NOT NULL
);
CREATE INDEX plugin_grants_thread ON plugin_grants (thread_id);
