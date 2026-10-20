-- How far each family has been pushed to and pulled from the server.
CREATE TABLE sync_state (
    family_id BLOB PRIMARY KEY NOT NULL REFERENCES families (id),
    generation INTEGER,
    pushed INTEGER NOT NULL,
    pulled INTEGER NOT NULL,
    last_pulled BLOB
);

-- The repository's normalized origin URL, to find a local clone on another machine.
ALTER TABLE threads ADD COLUMN repo TEXT NOT NULL DEFAULT '';
