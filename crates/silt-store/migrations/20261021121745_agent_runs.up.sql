-- The agent the web UI last ran in each thread on this machine, and how it
-- stands (ADR 0022): kept so every thread's state shows without its agent
-- running, and so a restart doesn't lose it. Personal to this machine, so
-- not in the family log.
CREATE TABLE agent_runs (
    thread_id BLOB PRIMARY KEY NOT NULL REFERENCES threads (id) ON DELETE CASCADE,
    agent TEXT NOT NULL,
    state TEXT NOT NULL,
    error TEXT,
    preset TEXT NOT NULL DEFAULT 'ask_before_commands',
    mode TEXT,
    updated_ms INTEGER NOT NULL
);
