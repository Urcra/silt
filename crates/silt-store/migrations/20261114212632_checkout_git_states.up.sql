-- The git state each checkout was in when its thread last looked, and the
-- thread's head then: when git moved since, the person's edits recorded
-- after that head are git's (ADR 0066). A `silt_git::GitState` as text,
-- the head as `Frontier`'s text.
CREATE TABLE checkout_git_states (
    checkout_id INTEGER NOT NULL,
    thread_id BLOB NOT NULL,
    state TEXT NOT NULL,
    head TEXT NOT NULL,
    PRIMARY KEY (checkout_id, thread_id)
);
