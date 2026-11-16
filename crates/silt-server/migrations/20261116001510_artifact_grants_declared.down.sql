CREATE TABLE artifact_grants_undeclared (
    user_id INTEGER NOT NULL REFERENCES users (id),
    thread_id BLOB NOT NULL,
    artifact_id TEXT NOT NULL,
    capability TEXT NOT NULL,
    state TEXT NOT NULL,
    at_ms INTEGER NOT NULL,
    PRIMARY KEY (user_id, thread_id, artifact_id, capability)
);
INSERT INTO artifact_grants_undeclared SELECT user_id, thread_id, artifact_id, capability, state, at_ms FROM artifact_grants;
DROP TABLE artifact_grants;
ALTER TABLE artifact_grants_undeclared RENAME TO artifact_grants;

CREATE TABLE artifact_pins_plain (
    user_id INTEGER NOT NULL REFERENCES users (id),
    thread_id BLOB NOT NULL,
    artifact_id TEXT NOT NULL,
    at_ms INTEGER NOT NULL,
    PRIMARY KEY (user_id, thread_id, artifact_id)
);
INSERT INTO artifact_pins_plain SELECT user_id, thread_id, artifact_id, at_ms FROM artifact_pins;
DROP TABLE artifact_pins;
ALTER TABLE artifact_pins_plain RENAME TO artifact_pins;
