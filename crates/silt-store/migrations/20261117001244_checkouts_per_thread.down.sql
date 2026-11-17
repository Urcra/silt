-- no-transaction
-- Back to one checkout per path: each path keeps its newest checkout, and
-- the sessions of the others move to it.
PRAGMA foreign_keys = OFF;
BEGIN;
UPDATE sessions SET checkout_id = (
    SELECT MAX(kept.id) FROM checkouts kept
    WHERE kept.path = (SELECT path FROM checkouts WHERE id = sessions.checkout_id)
);
DELETE FROM disk_files WHERE checkout_id NOT IN (SELECT MAX(id) FROM checkouts GROUP BY path);
DELETE FROM checkouts WHERE id NOT IN (SELECT MAX(id) FROM checkouts GROUP BY path);
CREATE TABLE checkouts_old (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id BLOB NOT NULL REFERENCES threads (id),
    path TEXT NOT NULL UNIQUE,
    actor BLOB NOT NULL,
    managed INTEGER NOT NULL,
    created_ms INTEGER NOT NULL
);
INSERT INTO checkouts_old (id, thread_id, path, actor, managed, created_ms)
    SELECT id, thread_id, path, actor, managed, created_ms FROM checkouts;
DELETE FROM sqlite_sequence WHERE name = 'checkouts_old';
INSERT INTO sqlite_sequence (name, seq)
    SELECT 'checkouts_old', seq FROM sqlite_sequence WHERE name = 'checkouts';
DROP INDEX checkouts_path;
DROP TABLE checkouts;
ALTER TABLE checkouts_old RENAME TO checkouts;
COMMIT;
PRAGMA foreign_keys = ON;
