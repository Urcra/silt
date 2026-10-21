-- What a project is called and where to clone it from, and what a new
-- thread in it starts with (ADR 0023). NULL means no preference.
ALTER TABLE projects ADD COLUMN name TEXT;
ALTER TABLE projects ADD COLUMN origin_url TEXT;
ALTER TABLE projects ADD COLUMN default_agent TEXT;
ALTER TABLE projects ADD COLUMN default_preset TEXT;
