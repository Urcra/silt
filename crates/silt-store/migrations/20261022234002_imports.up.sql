-- Sessions that ran before Silt recorded them, imported whole (ADR 0042).
-- Kept after their thread is purged, so importing again doesn't bring the
-- session back.
CREATE TABLE imports (
    agent TEXT NOT NULL,
    session_id TEXT NOT NULL,
    thread_id BLOB NOT NULL,
    family_id BLOB NOT NULL,
    imported_ms INTEGER NOT NULL,
    PRIMARY KEY (agent, session_id)
);
