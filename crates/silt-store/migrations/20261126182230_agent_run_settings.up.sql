-- The settings the thread's agent runs with, such as its model, as a JSON
-- object of each setting's id and value: kept so the agent starts again
-- with them, whatever its own configuration says.
ALTER TABLE agent_runs ADD COLUMN settings TEXT NOT NULL DEFAULT '{}';
