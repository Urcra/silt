-- Commands relayed to the daemon a thread runs on (ADR 0026), kept until
-- answered or expired, as a record of who sent what from where.
CREATE TABLE control_commands (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id BLOB NOT NULL,
    thread_id BLOB NOT NULL,
    sender_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    from_device BLOB,
    command TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    expires_ms INTEGER NOT NULL,
    taken_ms INTEGER,
    answered_ms INTEGER,
    answer TEXT
);

CREATE INDEX control_commands_waiting ON control_commands (device_id, taken_ms);
