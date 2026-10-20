-- A fingerprint of what this user may see of the family on the server.
ALTER TABLE sync_state ADD COLUMN view INTEGER NOT NULL DEFAULT 0;
