-- Prompts waiting for a thread's agent, in `position` order (ADR 0022):
-- the next is sent when a turn ends. Held here until the server holds the
-- queue (ADR 0020).
CREATE TABLE prompt_queue (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id BLOB NOT NULL REFERENCES threads (id) ON DELETE CASCADE,
    author TEXT NOT NULL,
    text TEXT NOT NULL,
    position INTEGER NOT NULL,
    created_ms INTEGER NOT NULL
);

CREATE INDEX prompt_queue_order ON prompt_queue (thread_id, position);
