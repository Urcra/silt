-- The actor the server writes as for each person, for what they do in the
-- hosted web app (ADR 0026).
CREATE TABLE server_actors (
    user_id INTEGER PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    actor BLOB NOT NULL UNIQUE
);
