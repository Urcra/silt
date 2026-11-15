-- The project an agent runs in, which its history tools are kept to in a
-- Local only workspace (ADR 0066).
ALTER TABLE run_tokens ADD COLUMN project TEXT;
