-- The user owning the thread on the server; empty when unknown.
ALTER TABLE threads ADD COLUMN owner TEXT NOT NULL DEFAULT '';
