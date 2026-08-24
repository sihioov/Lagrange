SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

LOCK TABLE public.owner_beta_recommendation_runs IN ACCESS EXCLUSIVE MODE;

-- Dropping published target lineage would make a completed run unverifiable.
-- The table is access-exclusively locked above, so the owner can inspect every
-- row without leaving tenant identity in the pooled migration connection.
ALTER TABLE public.owner_beta_recommendation_runs NO FORCE ROW LEVEL SECURITY;
DO $rollback_guard$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM public.owner_beta_recommendation_runs AS run
         WHERE run.target_snapshot_sha256 IS NOT NULL
            OR run.cash_weight IS NOT NULL
    ) THEN
        RAISE EXCEPTION 'owner beta target publication rollback would discard lineage'
            USING ERRCODE = '55000';
    END IF;
END
$rollback_guard$;
ALTER TABLE public.owner_beta_recommendation_runs FORCE ROW LEVEL SECURITY;

REVOKE UPDATE (
    target_snapshot_sha256, cash_weight
) ON public.owner_beta_recommendation_runs FROM worker;

ALTER TABLE public.owner_beta_recommendation_runs
    DROP CONSTRAINT owner_beta_recommendation_runs_result_state_check,
    DROP CONSTRAINT owner_beta_recommendation_runs_cash_weight_check,
    DROP CONSTRAINT owner_beta_recommendation_runs_target_hash_check,
    DROP COLUMN cash_weight,
    DROP COLUMN target_snapshot_sha256,
    ADD CONSTRAINT owner_beta_recommendation_runs_success_factor_check CHECK (
        status <> 'SUCCEEDED' OR factor_snapshot_sha256 IS NOT NULL
    );
