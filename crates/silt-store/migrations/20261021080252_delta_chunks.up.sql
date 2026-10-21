-- A family's older deltas packed together, from `first_seq` to `last_seq`,
-- so a family is read in a few rows rather than one per delta: row by row,
-- a long family takes a hook capturing on its own most of its budget (ADR
-- 0018). The rows stay where they are; a chunk repeats them.
CREATE TABLE delta_chunks (
    family_id BLOB NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    first_seq INTEGER NOT NULL,
    last_seq INTEGER NOT NULL,
    data BLOB NOT NULL,
    PRIMARY KEY (family_id, first_seq)
);
