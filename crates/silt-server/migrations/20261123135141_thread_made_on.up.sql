-- The device that registered each thread, so a person's other machines can
-- say where a thread no device has taken over lives (ADR 0115). Threads
-- registered before this, and by the hosted app, have none. Not a foreign
-- key: a device signed out leaves the thread where it was.
ALTER TABLE threads ADD COLUMN made_on BLOB;
