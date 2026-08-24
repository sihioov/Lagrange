SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

LOCK TABLE public.owner_beta_recommendation_runs IN ACCESS EXCLUSIVE MODE;

-- Dropping published target lineage would make a completed run unverifiable.
DO $rollback_guard$
DECLARE
    v_owner_user_id uuid;
BEGIN
    FOR v_owner_user_id IN
        SELECT users.id FROM public.users AS users ORDER BY users.id
    LOOP
        PERFORM pg_catalog.set_config(
            'app.actor_user_id', v_owner_user_id::text, true
        );
        IF EXISTS (
            SELECT 1
              FROM public.owner_beta_recommendation_runs AS run
             WHERE run.owner_user_id = v_owner_user_id
               AND (
                    run.target_snapshot_sha256 IS NOT NULL
                    OR run.cash_weight IS NOT NULL
               )
        ) THEN
            RAISE EXCEPTION 'owner beta target publication rollback would discard lineage'
                USING ERRCODE = '55000';
        END IF;
    END LOOP;
    PERFORM pg_catalog.set_config('app.actor_user_id', '', true);
END
$rollback_guard$;

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
