-- An org's policy for agents started in its projects on members' machines
-- (ADR 0064), as JSON; '{}' restricts nothing.
ALTER TABLE orgs ADD COLUMN policy TEXT NOT NULL DEFAULT '{}';
