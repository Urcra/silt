-- Each person's own themes for Silt's UI, as their devices sync them
-- (ADR 0099): one row a theme file, its JSON and fonts in `body`, a removed
-- one kept as a tombstone so the removal reaches every device.
CREATE TABLE themes (
    user_id INTEGER NOT NULL REFERENCES users (id),
    name TEXT NOT NULL,
    at_ms INTEGER NOT NULL,
    deleted INTEGER NOT NULL DEFAULT 0,
    body BLOB NOT NULL,
    PRIMARY KEY (user_id, name)
);
