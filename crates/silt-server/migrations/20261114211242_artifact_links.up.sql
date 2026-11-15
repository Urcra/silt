-- A read-only link to one artifact of the thread (ADR 0067), at a version or,
-- with none, at whichever is the latest when it's opened.
ALTER TABLE thread_links ADD COLUMN artifact_id TEXT;
ALTER TABLE thread_links ADD COLUMN artifact_version INTEGER;
