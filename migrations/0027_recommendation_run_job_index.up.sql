-- no-transaction
-- 0027: non-blocking uniqueness for populated recommendation run lineage.
-- One concurrent statement per SQLx migration is mandatory: SQLx executes a
-- migration file as one command, and PostgreSQL rejects multiple concurrent
-- index statements in the resulting implicit transaction block.

CREATE UNIQUE INDEX CONCURRENTLY recommendation_runs_job_id_uq
    ON recommendation_runs (job_id) WHERE job_id IS NOT NULL;
