ALTER TABLE threads ADD COLUMN level TEXT CHECK (level IN ('private', 'team'));
UPDATE threads SET level = CASE WHEN visibility = 'private' THEN 'private' WHEN visibility IS NULL THEN NULL ELSE 'team' END;
ALTER TABLE threads DROP COLUMN visibility;
ALTER TABLE threads RENAME COLUMN level TO visibility;

ALTER TABLE projects ADD COLUMN level TEXT NOT NULL DEFAULT 'team' CHECK (level IN ('private', 'team'));
UPDATE projects SET level = CASE WHEN visibility = 'private' THEN 'private' ELSE 'team' END;
ALTER TABLE projects DROP COLUMN visibility;
ALTER TABLE projects RENAME COLUMN level TO visibility;
ALTER TABLE projects DROP COLUMN org_id;

DROP TABLE team_members;
DROP TABLE teams;
DROP TABLE org_members;
DROP TABLE orgs;
