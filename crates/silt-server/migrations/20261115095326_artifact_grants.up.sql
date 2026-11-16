-- What each person allowed or refused an artifact's page (ADR 0072), per
-- capability. A person's own, never synced into a family's log.
CREATE TABLE artifact_grants (
    user_id INTEGER NOT NULL REFERENCES users (id),
    thread_id BLOB NOT NULL,
    artifact_id TEXT NOT NULL,
    capability TEXT NOT NULL,
    state TEXT NOT NULL,
    at_ms INTEGER NOT NULL,
    PRIMARY KEY (user_id, thread_id, artifact_id, capability)
);
