-- no-transaction
-- A clone tracked in place has a checkout per thread working there, so
-- `path` is unique per thread. SQLite rebuilds the table to drop a
-- constraint; with foreign keys on, dropping it would empty `disk_files`.
PRAGMA foreign_keys = OFF;
BEGIN;
CREATE TABLE checkouts_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id BLOB NOT NULL REFERENCES threads (id),
    path TEXT NOT NULL,
    actor BLOB NOT NULL,
    managed INTEGER NOT NULL,
    created_ms INTEGER NOT NULL,
    UNIQUE (path, thread_id)
);
INSERT INTO checkouts_new (id, thread_id, path, actor, managed, created_ms)
    SELECT id, thread_id, path, actor, managed, created_ms FROM checkouts;
-- Ids of removed checkouts stay used: `checkout_git_states` may still name them.
DELETE FROM sqlite_sequence WHERE name = 'checkouts_new';
INSERT INTO sqlite_sequence (name, seq)
    SELECT 'checkouts_new', seq FROM sqlite_sequence WHERE name = 'checkouts';
DROP TABLE checkouts;
ALTER TABLE checkouts_new RENAME TO checkouts;
CREATE INDEX checkouts_path ON checkouts (path);
COMMIT;
PRAGMA foreign_keys = ON;
