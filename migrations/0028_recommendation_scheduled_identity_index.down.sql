-- no-transaction
-- Reverse 0028 without a blocking index-drop lock.

DROP INDEX CONCURRENTLY IF EXISTS recommendation_runs_scheduled_identity_uq;
