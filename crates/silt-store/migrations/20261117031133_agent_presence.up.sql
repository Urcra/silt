-- Whether each session's agent is still around and what it's doing, kept
-- alike with or without a daemon, so agents in one project can be told of
-- each other (ADR 0087).
ALTER TABLE sessions ADD COLUMN seen_ms INTEGER;
ALTER TABLE sessions ADD COLUMN ended_ms INTEGER;
-- Set after the agent's turn stopped, until its next prompt.
ALTER TABLE sessions ADD COLUMN idle INTEGER NOT NULL DEFAULT 0;
-- The first line of the session's last prompt.
ALTER TABLE sessions ADD COLUMN doing TEXT;

-- The files each session edited, last edit first.
CREATE TABLE session_touches (
    agent TEXT NOT NULL,
    session_id TEXT NOT NULL,
    path TEXT NOT NULL,
    touched_ms INTEGER NOT NULL,
    PRIMARY KEY (agent, session_id, path),
    FOREIGN KEY (agent, session_id) REFERENCES sessions (agent, session_id) ON DELETE CASCADE
);

-- What one thread's agent has to tell another's, handed over once: a
-- message it sent, or that it arrived in the project.
CREATE TABLE agent_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    to_thread BLOB NOT NULL REFERENCES threads (id) ON DELETE CASCADE,
    from_thread BLOB NOT NULL REFERENCES threads (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    body TEXT NOT NULL,
    created_ms INTEGER NOT NULL
);
CREATE INDEX agent_messages_to ON agent_messages (to_thread);
