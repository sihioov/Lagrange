-- Revert only 0026's recommendation pipeline schema and privileges.

REVOKE EXECUTE ON FUNCTION
    public.schedule_recommendation_run(uuid, uuid, date, uuid, text, text)
    FROM worker;
DROP FUNCTION IF EXISTS
    public.schedule_recommendation_run(uuid, uuid, date, uuid, text, text);

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

DROP INDEX IF EXISTS jobs_typed_claim_idx;
DROP INDEX IF EXISTS recommendation_runs_scheduled_identity_uq;
DROP INDEX IF EXISTS recommendation_runs_job_id_uq;
DROP INDEX IF EXISTS target_portfolios_one_per_run;

ALTER TABLE recommendation_items
    DROP CONSTRAINT IF EXISTS recommendation_items_run_instrument_key;

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
