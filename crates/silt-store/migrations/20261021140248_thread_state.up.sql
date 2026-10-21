-- This person's state of each thread (ADR 0024), as a
-- `silt_proto::ThreadState` in JSON; `pending` until the server has it.
CREATE TABLE thread_state (
    thread_id BLOB PRIMARY KEY NOT NULL,
    state TEXT NOT NULL,
    pending INTEGER NOT NULL DEFAULT 0
);
