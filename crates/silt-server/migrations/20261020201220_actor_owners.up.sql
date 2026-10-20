-- Which user's devices write as each actor: the first to push one owns it,
-- so nobody else can take its next ids and wedge its sync (ADR 0016).
CREATE TABLE actor_owners (
    actor BLOB PRIMARY KEY NOT NULL,
    user_id INTEGER NOT NULL REFERENCES users (id)
);
