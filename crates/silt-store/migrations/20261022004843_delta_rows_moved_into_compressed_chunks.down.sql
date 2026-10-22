-- Can't be undone in SQL: the packed rows' bytes now live only in chunks,
-- some of them compressed (ADR 0035). Restore a backup of the data dir to
-- go back to a build from before this.
SELECT 1;
