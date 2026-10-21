-- A family's projection as of its first `covers` deltas, so loading it
-- doesn't replay them (ADR 0018). `format` is the projection's encoding; a
-- snapshot of another one is replayed instead.
CREATE TABLE family_snapshots (
    family_id BLOB PRIMARY KEY NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    format INTEGER NOT NULL,
    covers INTEGER NOT NULL,
    data BLOB NOT NULL
);
