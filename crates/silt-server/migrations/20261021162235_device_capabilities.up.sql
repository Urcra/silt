-- What each device may do to the person's other devices (ADR 0026), as
-- JSON; NULL lets it do everything.
ALTER TABLE devices ADD COLUMN capabilities TEXT;
