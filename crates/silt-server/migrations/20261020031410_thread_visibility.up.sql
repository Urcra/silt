-- Visibility moves from the family's root thread to every thread
-- (ADR 0010, revised): a thread's own setting wins, a fork without one
-- follows its source, then the project, then `team`.
ALTER TABLE threads ADD COLUMN visibility TEXT CHECK (visibility IN ('private', 'team'));
UPDATE threads SET visibility = (SELECT f.visibility FROM families f WHERE f.root_thread = threads.id);
