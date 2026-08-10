-- no-transaction
-- 0024: build the populated-table publication-lineage uniqueness guarantee
-- without blocking writers behind a transactional index build.
-- Deployment must set a finite session lock_timeout externally, for example:
--   PGOPTIONS='-c lock_timeout=5s' sqlx migrate run
-- SQLx sends this file as one command and PostgreSQL requires concurrent DDL
-- to be its sole statement.

CREATE UNIQUE INDEX CONCURRENTLY data_batches_source_file_uq
    ON data_batches (provider, market, source_batch_id, source_file_name)
    WHERE source_batch_id IS NOT NULL;
