-- Drop the presence table.
--
-- The presence feature (online/offline/busy status tracking) has been
-- removed from echat. The table had no remaining writers or readers
-- after the application code was cleaned up, so the rows here are dead
-- data only.
--
-- No other table has a foreign key pointing to `presence`, so a plain
-- DROP (without CASCADE-on-dependents) is enough. `IF EXISTS` keeps the
-- migration idempotent for environments where the table was already
-- dropped manually.

DROP TABLE IF EXISTS presence;
