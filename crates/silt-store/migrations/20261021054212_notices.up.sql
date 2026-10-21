-- Messages for the agent in a thread, handed over once with its next prompt.
-- Stored, so whichever process sees that prompt delivers them (ADR 0018).
CREATE TABLE notices (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id BLOB NOT NULL REFERENCES threads (id) ON DELETE CASCADE,
    message TEXT NOT NULL,
    created_ms INTEGER NOT NULL
);
CREATE INDEX notices_by_thread ON notices (thread_id);
