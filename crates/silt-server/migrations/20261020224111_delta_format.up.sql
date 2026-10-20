-- How each stored delta is encoded, so a later Silt can change the encoding
-- and still read what is already stored. 0 is postcard as of this version.
ALTER TABLE deltas ADD COLUMN format INTEGER NOT NULL DEFAULT 0;
