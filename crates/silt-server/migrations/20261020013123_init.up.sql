CREATE TABLE devices (
    id BLOB PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    token_hash BLOB NOT NULL UNIQUE,
    created_ms INTEGER NOT NULL
);

CREATE TABLE pairing_codes (
    code_hash BLOB PRIMARY KEY NOT NULL,
    expires_ms INTEGER NOT NULL
);

-- Every delta of every family, in the order the server stored them.
CREATE TABLE deltas (
    family_id BLOB NOT NULL,
    seq INTEGER NOT NULL,
    data BLOB NOT NULL,
    PRIMARY KEY (family_id, seq)
);

CREATE TABLE threads (
    id BLOB PRIMARY KEY NOT NULL,
    family_id BLOB NOT NULL,
    repo TEXT NOT NULL,
    created_ms INTEGER NOT NULL
);

CREATE TABLE leases (
    thread_id BLOB PRIMARY KEY NOT NULL REFERENCES threads (id),
    device_id BLOB NOT NULL REFERENCES devices (id),
    epoch INTEGER NOT NULL
);
