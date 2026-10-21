-- What each person keeps about a thread for themselves (ADR 0024): a
-- `silt_proto::ThreadState` as JSON, merged field by field in Rust.
CREATE TABLE thread_state (
    user_id INTEGER NOT NULL REFERENCES users (id),
    thread_id BLOB NOT NULL,
    state TEXT NOT NULL,
    PRIMARY KEY (user_id, thread_id)
);
