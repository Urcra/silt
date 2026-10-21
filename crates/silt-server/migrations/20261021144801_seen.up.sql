-- Files of a merge request each person has looked at (ADR 0024), at the
-- version `hash` names; the newer mark wins.
CREATE TABLE seen (
    user_id INTEGER NOT NULL REFERENCES users (id),
    -- The merge request's id as `DeltaId` writes it.
    mr_id TEXT NOT NULL,
    path TEXT NOT NULL,
    hash TEXT NOT NULL,
    seen INTEGER NOT NULL,
    at_ms INTEGER NOT NULL,
    PRIMARY KEY (user_id, mr_id, path)
);
