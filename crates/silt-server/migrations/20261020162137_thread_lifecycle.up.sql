-- Deleted threads wait in the trash until purged; purged ones are
-- remembered so their deltas are never stored again (ADR 0013).
ALTER TABLE threads ADD COLUMN deleted_ms INTEGER;

CREATE TABLE purged_threads (
    thread_id BLOB PRIMARY KEY NOT NULL,
    family_id BLOB NOT NULL,
    purged_ms INTEGER NOT NULL
);
