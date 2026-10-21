-- Files of merge requests this person has looked at (ADR 0024), by the
-- request's `DeltaId`; `pending` until the server has the mark.
CREATE TABLE seen_marks (
    mr_id TEXT NOT NULL,
    path TEXT NOT NULL,
    hash TEXT NOT NULL,
    seen INTEGER NOT NULL,
    at_ms INTEGER NOT NULL,
    pending INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (mr_id, path)
);
