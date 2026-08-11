-- no-transaction
-- 0032: preserve typed claim ordering while filtering availability from the
-- included value. Only queued rows occupy the hot worker index.

CREATE INDEX CONCURRENTLY jobs_typed_claim_idx
    ON jobs (job_type, priority DESC, created_at)
    INCLUDE (available_at)
    WHERE status = 'QUEUED';
