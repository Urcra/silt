-- Every device belongs to a user; devices paired before users existed
-- belong to `owner` (ADR 0010).
CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);
INSERT INTO users (id, name) VALUES (1, 'owner');

ALTER TABLE devices ADD COLUMN user_id INTEGER NOT NULL DEFAULT 1;
ALTER TABLE pairing_codes ADD COLUMN user_id INTEGER NOT NULL DEFAULT 1;
ALTER TABLE threads ADD COLUMN owner_id INTEGER NOT NULL DEFAULT 1;

-- Visibility is enforced per family. `visibility` is the root thread's
-- override of its project's setting.
CREATE TABLE families (
    id BLOB PRIMARY KEY NOT NULL,
    owner_id INTEGER NOT NULL REFERENCES users (id),
    root_thread BLOB,
    repo TEXT NOT NULL DEFAULT '',
    visibility TEXT CHECK (visibility IN ('private', 'team'))
);
INSERT INTO families (id, owner_id, root_thread, repo)
SELECT t.family_id, 1,
       (SELECT first.id FROM threads first WHERE first.family_id = t.family_id ORDER BY first.created_ms LIMIT 1),
       MIN(t.repo)
FROM threads t
GROUP BY t.family_id;
INSERT OR IGNORE INTO families (id, owner_id)
SELECT DISTINCT family_id, 1 FROM deltas;

CREATE TABLE projects (
    repo TEXT PRIMARY KEY NOT NULL,
    visibility TEXT NOT NULL CHECK (visibility IN ('private', 'team'))
);
