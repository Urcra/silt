-- What the views across families read of a family the daemon doesn't hold,
-- as of the store's position `position` in it (ADR 0115). `format` is the
-- digest's encoding; a digest of another one is taken again.
CREATE TABLE family_digests (
    family_id BLOB PRIMARY KEY NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    format INTEGER NOT NULL,
    position INTEGER NOT NULL,
    data BLOB NOT NULL
);
