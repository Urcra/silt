-- Clones this machine opened the web UI from, by normalized repository.
CREATE TABLE clones (
    repo TEXT PRIMARY KEY NOT NULL,
    path TEXT NOT NULL
);
