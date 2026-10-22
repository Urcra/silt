-- Deltas pulled from a server are stored before their search rows, which
-- take as long again to write. Each family whose rows are still to be
-- written is here, from the first delta without them, so a crash in
-- between leaves them to be written at the next start.
CREATE TABLE search_pending (
    family_id BLOB PRIMARY KEY NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    from_seq INTEGER NOT NULL
);
