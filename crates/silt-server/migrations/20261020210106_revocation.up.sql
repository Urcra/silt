-- Device tokens lapse after a long idle spell, and admins can disable a
-- user. Existing devices count as used now so nobody is signed out by the
-- upgrade.
ALTER TABLE devices ADD COLUMN last_used_ms INTEGER NOT NULL DEFAULT 0;
UPDATE devices SET last_used_ms = CAST(strftime('%s', 'now') AS INTEGER) * 1000;
ALTER TABLE users ADD COLUMN disabled INTEGER NOT NULL DEFAULT 0;
