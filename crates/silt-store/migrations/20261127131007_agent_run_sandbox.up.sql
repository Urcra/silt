-- The thread's own choice of Silt's sandbox, as JSON: whether its agent
-- runs in it and what it reaches, in place of its workspace's rules.
-- NULL follows the workspace.
ALTER TABLE agent_runs ADD COLUMN sandbox TEXT;
