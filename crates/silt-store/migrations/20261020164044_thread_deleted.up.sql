-- A thread in the trash stays until it is restored or purged (ADR 0013).
ALTER TABLE threads ADD COLUMN deleted_ms INTEGER;
