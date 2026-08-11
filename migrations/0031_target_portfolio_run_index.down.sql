-- no-transaction
-- Reverse 0031 without a blocking index-drop lock.

DROP INDEX CONCURRENTLY IF EXISTS target_portfolios_one_per_run;
