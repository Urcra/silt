-- The secrets this machine knows, to put back what redaction took out of
-- deltas (ADR 0015). Never synced.
CREATE TABLE secret_overlay (
    secret TEXT PRIMARY KEY NOT NULL
);
