-- Where each family syncs (ADR 0032): a server's URL, or '' for this
-- machine only. A family without a row syncs with the default server.
-- A family listed by a server has its home before any of it is stored, so
-- this doesn't reference `families`.
CREATE TABLE family_homes (
    family_id BLOB PRIMARY KEY NOT NULL,
    home TEXT NOT NULL
);

-- Where a project's new families go, by normalized repository.
CREATE TABLE project_homes (
    repo TEXT PRIMARY KEY NOT NULL,
    home TEXT NOT NULL
);
