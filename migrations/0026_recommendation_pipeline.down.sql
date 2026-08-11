-- Revert only 0026's recommendation pipeline schema and privileges.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Fail before changing privileges, functions, triggers, constraints, or
-- columns. FORCE RLS means migration_owner must inspect one tenant at a time
-- under the same actor context used by its existing owner policies.
DO $rollback_guard$
DECLARE
    v_owner_user_id uuid;
BEGIN
    FOR v_owner_user_id IN SELECT id FROM public.users LOOP
        PERFORM pg_catalog.set_config(
            'app.actor_user_id',
            v_owner_user_id::text,
            true
        );
        IF EXISTS (
            SELECT 1
            FROM public.recommendation_runs AS scheduled_run
            WHERE scheduled_run.owner_user_id = v_owner_user_id
              AND scheduled_run.trigger_kind = 'SCHEDULED'
        ) OR EXISTS (
            SELECT 1
            FROM public.jobs AS scheduled_job
            WHERE scheduled_job.owner_user_id = v_owner_user_id
              AND scheduled_job.idempotency_key LIKE 'recommendation:scheduled:%'
        ) THEN
            RAISE EXCEPTION '0026 rollback blocked by scheduled recommendation lineage'
                USING ERRCODE = '55000';
        END IF;
    END LOOP;
END
$rollback_guard$;

REVOKE EXECUTE ON FUNCTION
    public.schedule_recommendation_run(uuid, uuid, date, uuid, text, integer, text)
    FROM worker;
DROP FUNCTION IF EXISTS
    public.schedule_recommendation_run(uuid, uuid, date, uuid, text, integer, text);

DROP TRIGGER IF EXISTS jobs_protect_scheduled_recommendation_lineage
    ON jobs;
DROP FUNCTION IF EXISTS
    public.jobs_reject_scheduled_recommendation_mutation();

DROP TRIGGER IF EXISTS recommendation_runs_protect_scheduled_lineage
    ON recommendation_runs;
DROP FUNCTION IF EXISTS
    public.recommendation_runs_reject_scheduled_lineage_mutation();

REVOKE SELECT ON TABLE recommendation_runs FROM worker;
REVOKE UPDATE (status, summary_json) ON TABLE recommendation_runs FROM worker;
REVOKE SELECT, INSERT ON TABLE recommendation_items, target_portfolios FROM worker;

-- Restore migration 0013's exact worker privilege set on bindings.
GRANT SELECT, INSERT, UPDATE ON TABLE account_strategy_bindings TO worker;

ALTER TABLE recommendation_runs
    DROP CONSTRAINT IF EXISTS recommendation_runs_scheduled_lineage_check,
    DROP CONSTRAINT IF EXISTS recommendation_runs_dataset_manifest_sha256_check,
    DROP CONSTRAINT IF EXISTS recommendation_runs_trigger_check,
    DROP CONSTRAINT IF EXISTS recommendation_runs_dataset_version_id_fkey,
    DROP CONSTRAINT IF EXISTS recommendation_runs_job_id_fkey;

ALTER TABLE recommendation_runs
    DROP COLUMN IF EXISTS dataset_manifest_sha256,
    DROP COLUMN IF EXISTS dataset_version_id,
    DROP COLUMN IF EXISTS trigger_kind,
    DROP COLUMN IF EXISTS job_id;

ALTER TABLE account_strategy_bindings
    DROP COLUMN IF EXISTS auto_apply_recommendations;
