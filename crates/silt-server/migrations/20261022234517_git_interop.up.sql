-- Whether a project's commits get Silt's git notes, and whether its
-- threads' pull requests get a comment from the server (ADR 0042).
ALTER TABLE projects ADD COLUMN git_notes TEXT NOT NULL DEFAULT 'off';
ALTER TABLE projects ADD COLUMN forge_comments INTEGER NOT NULL DEFAULT 0;
