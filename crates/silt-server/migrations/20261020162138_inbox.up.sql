-- What each user should hear about: merge requests, comments and mentions
-- (ADR 0013). `delta` is the delta that caused it, as text.
CREATE TABLE notifications (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    thread_id BLOB NOT NULL,
    delta TEXT NOT NULL,
    actor TEXT NOT NULL,
    text TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    read_ms INTEGER,
    UNIQUE (user_id, kind, delta)
);
CREATE INDEX notifications_unread ON notifications (user_id, read_ms);
