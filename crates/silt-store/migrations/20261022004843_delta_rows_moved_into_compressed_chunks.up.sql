-- A chunk holds its deltas' bytes, compressed or not, and the rows it holds
-- keep only their place (ADR 0035).
ALTER TABLE delta_chunks ADD COLUMN codec INTEGER NOT NULL DEFAULT 0;
UPDATE deltas SET data = x''
WHERE seq <= (SELECT MAX(last_seq) FROM delta_chunks WHERE delta_chunks.family_id = deltas.family_id);
