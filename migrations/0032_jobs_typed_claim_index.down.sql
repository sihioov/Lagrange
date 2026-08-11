-- no-transaction
-- Reverse 0032 without a blocking index-drop lock.

DROP INDEX CONCURRENTLY IF EXISTS jobs_typed_claim_idx;
