-- no-transaction
-- Reverse 0027 without a blocking index-drop lock.

DROP INDEX CONCURRENTLY IF EXISTS recommendation_runs_job_id_uq;
