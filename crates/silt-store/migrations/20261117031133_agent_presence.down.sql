DROP TABLE agent_messages;
DROP TABLE session_touches;
ALTER TABLE sessions DROP COLUMN doing;
ALTER TABLE sessions DROP COLUMN idle;
ALTER TABLE sessions DROP COLUMN ended_ms;
ALTER TABLE sessions DROP COLUMN seen_ms;
