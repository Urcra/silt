-- A grant holds for what the version the person saw declared of its
-- capability (ADR 0072): a later version declaring more asks again. Grants
-- kept before have no declaration, so they ask again too. Both tables go
-- with their person.
CREATE TABLE artifact_grants_declared (
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    thread_id BLOB NOT NULL,
    artifact_id TEXT NOT NULL,
    capability TEXT NOT NULL,
    state TEXT NOT NULL,
    declaration TEXT NOT NULL,
    at_ms INTEGER NOT NULL,
    PRIMARY KEY (user_id, thread_id, artifact_id, capability)
);
INSERT INTO artifact_grants_declared (user_id, thread_id, artifact_id, capability, state, declaration, at_ms)
    SELECT user_id, thread_id, artifact_id, capability, state, '', at_ms FROM artifact_grants;
DROP TABLE artifact_grants;
ALTER TABLE artifact_grants_declared RENAME TO artifact_grants;

CREATE TABLE artifact_pins_cascading (
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    thread_id BLOB NOT NULL,
    artifact_id TEXT NOT NULL,
    at_ms INTEGER NOT NULL,
    PRIMARY KEY (user_id, thread_id, artifact_id)
);
INSERT INTO artifact_pins_cascading SELECT user_id, thread_id, artifact_id, at_ms FROM artifact_pins;
DROP TABLE artifact_pins;
ALTER TABLE artifact_pins_cascading RENAME TO artifact_pins;
