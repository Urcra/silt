-- The last stored delta of each family, kept by the store itself, so a
-- writer sees at a glance which families others appended to (ADR 0018).
CREATE TABLE family_positions (
    family_id BLOB PRIMARY KEY NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL
);
INSERT INTO family_positions (family_id, seq) SELECT family_id, MAX(seq) FROM deltas GROUP BY family_id;

CREATE TRIGGER family_positions_after_append AFTER INSERT ON deltas
BEGIN
    INSERT INTO family_positions (family_id, seq) VALUES (NEW.family_id, NEW.seq)
    ON CONFLICT (family_id) DO UPDATE SET seq = MAX(seq, excluded.seq);
END;

CREATE TRIGGER family_positions_after_clear AFTER DELETE ON deltas
BEGIN
    DELETE FROM family_positions
    WHERE family_id = OLD.family_id AND NOT EXISTS (SELECT 1 FROM deltas WHERE family_id = OLD.family_id);
END;
