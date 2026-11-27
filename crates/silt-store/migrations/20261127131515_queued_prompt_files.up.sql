-- The files a queued prompt goes with, as a JSON array of their names and
-- types in order; the daemon keeps the files themselves beside the store.
ALTER TABLE prompt_queue ADD COLUMN files TEXT NOT NULL DEFAULT '[]';
