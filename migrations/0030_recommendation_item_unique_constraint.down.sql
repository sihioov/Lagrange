-- Revert 0030. PostgreSQL drops the constraint-owned backing index with it;
-- 0029 down therefore uses DROP INDEX IF EXISTS.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

ALTER TABLE recommendation_items
    DROP CONSTRAINT IF EXISTS recommendation_items_run_instrument_key;
