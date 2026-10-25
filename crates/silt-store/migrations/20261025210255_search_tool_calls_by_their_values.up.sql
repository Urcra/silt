-- Tool calls were indexed with their input as JSON, so a search for
-- "command" matched the key of every shell command. Every search row comes
-- from a delta, so they are all written again, now from the inputs'
-- values, as each family's are while pending.
DELETE FROM search;
INSERT INTO search_pending (family_id, from_seq)
SELECT id, 0 FROM families WHERE true
ON CONFLICT (family_id) DO UPDATE SET from_seq = 0;
