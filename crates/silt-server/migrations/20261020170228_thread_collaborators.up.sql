-- People the owner lets write into a thread (ADR 0014).
CREATE TABLE thread_collaborators (
    thread_id BLOB NOT NULL REFERENCES threads (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (thread_id, user_id)
);
