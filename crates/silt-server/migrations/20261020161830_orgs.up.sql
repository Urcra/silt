-- Orgs and teams (ADR 0013). Everything so far belongs to one org,
-- `default`, which every user joins; the old `team` visibility, which
-- meant everyone on the server, becomes `org`.
CREATE TABLE orgs (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    default_visibility TEXT NOT NULL DEFAULT 'org'
        CHECK (default_visibility = 'private' OR default_visibility = 'org' OR default_visibility LIKE 'team:%'),
    created_ms INTEGER NOT NULL DEFAULT 0
);
INSERT INTO orgs (id, name) VALUES (1, 'default');

CREATE TABLE org_members (
    org_id INTEGER NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('owner', 'member')),
    PRIMARY KEY (org_id, user_id)
);
INSERT INTO org_members (org_id, user_id, role)
SELECT 1, id, CASE WHEN admin = 1 THEN 'owner' ELSE 'member' END FROM users;

CREATE TABLE teams (
    id INTEGER PRIMARY KEY,
    org_id INTEGER NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    UNIQUE (org_id, name)
);

CREATE TABLE team_members (
    team_id INTEGER NOT NULL REFERENCES teams (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (team_id, user_id)
);

-- A project's visibility is now optional: without one its org's default
-- applies.
ALTER TABLE projects ADD COLUMN org_id INTEGER REFERENCES orgs (id);
ALTER TABLE projects ADD COLUMN level TEXT
    CHECK (level IS NULL OR level = 'private' OR level = 'org' OR level LIKE 'team:%');
UPDATE projects SET org_id = 1, level = CASE visibility WHEN 'team' THEN 'org' ELSE visibility END;
ALTER TABLE projects DROP COLUMN visibility;
ALTER TABLE projects RENAME COLUMN level TO visibility;
INSERT OR IGNORE INTO projects (repo, org_id) SELECT DISTINCT repo, 1 FROM threads;

ALTER TABLE threads ADD COLUMN level TEXT
    CHECK (level IS NULL OR level = 'private' OR level = 'org' OR level LIKE 'team:%');
UPDATE threads SET level = CASE visibility WHEN 'team' THEN 'org' ELSE visibility END;
ALTER TABLE threads DROP COLUMN visibility;
ALTER TABLE threads RENAME COLUMN level TO visibility;
