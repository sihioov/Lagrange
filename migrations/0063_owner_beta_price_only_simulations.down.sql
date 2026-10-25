SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

LOCK TABLE public.owner_beta_simulation_runs IN ACCESS EXCLUSIVE MODE;
LOCK TABLE public.owner_beta_simulation_result_pairs IN ACCESS EXCLUSIVE MODE;

-- A rollback must inspect through FORCE RLS without borrowing an actor GUC.
-- The access-exclusive locks make this bounded emptiness check stable. If it
-- refuses, migration transaction rollback restores FORCE RLS and every object.
ALTER TABLE public.owner_beta_simulation_runs NO FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_beta_simulation_result_pairs NO FORCE ROW LEVEL SECURITY;
DO $owner_beta_simulation_empty_guard$
BEGIN
    IF EXISTS (SELECT 1 FROM public.owner_beta_simulation_result_pairs)
       OR EXISTS (SELECT 1 FROM public.owner_beta_simulation_runs)
    THEN
        RAISE EXCEPTION 'owner beta simulation migration requires empty run and pair history'
            USING ERRCODE = '55000';
    END IF;
END
$owner_beta_simulation_empty_guard$;

-- Restore the historical terminal callback after the empty-history refusal.
CREATE OR REPLACE FUNCTION public.sync_recommendation_run_from_terminal_job()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    PERFORM pg_catalog.set_config('app.actor_user_id', NEW.owner_user_id::text, true);
    IF NEW.job_type = 'recommendation'
       AND NEW.status = 'FAILED'
       AND OLD.status IS DISTINCT FROM NEW.status
       AND NEW.error_code = 'attempts_exhausted' THEN
        UPDATE public.recommendation_runs
           SET status = 'FAILED',
               summary_json = '{"code":"RECOMMENDATION_ATTEMPTS_EXHAUSTED","message":"recommendation worker attempts were exhausted"}'::jsonb
         WHERE job_id = NEW.id
           AND owner_user_id = NEW.owner_user_id
           AND status = 'PENDING';
    ELSIF NEW.job_type = 'recommendation'
       AND NEW.status = 'CANCELED'
       AND OLD.status IS DISTINCT FROM NEW.status THEN
        UPDATE public.recommendation_runs
           SET status = 'FAILED',
               summary_json = '{"code":"RECOMMENDATION_CANCELED","message":"recommendation was canceled"}'::jsonb
         WHERE job_id = NEW.id
           AND owner_user_id = NEW.owner_user_id
           AND status = 'PENDING';
    END IF;
    RETURN NEW;
END;
$$;

-- Restore the 0044 callback without replacing its owner or ACL.
CREATE OR REPLACE FUNCTION public.assert_candidate_publication_settlement()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $assert$
DECLARE
    v_job_id uuid;
    v_run_id uuid;
    v_run_status text;
    v_job_status text;
    v_attempt_count integer;
    v_service_user_id uuid;
BEGIN
    SELECT control.service_user_id INTO v_service_user_id
    FROM public.candidate_scheduler_control AS control
    WHERE control.control_key = 'scheduler';
    IF NOT FOUND THEN RETURN NULL; END IF;
    -- Constraint triggers run as migration_owner at COMMIT, outside any
    -- request actor scope. Bind the only owner candidate jobs may use before
    -- touching the FORCE-RLS queue tables.
    PERFORM pg_catalog.set_config(
        'app.actor_user_id', v_service_user_id::text, true
    );
    IF TG_TABLE_NAME = 'stock_analysis_runs' THEN
        IF TG_OP = 'DELETE' THEN
            v_run_id := OLD.id;
            v_job_id := OLD.job_id;
        ELSE
            v_run_id := NEW.id;
            v_job_id := NEW.job_id;
        END IF;
    ELSIF TG_TABLE_NAME = 'jobs' THEN
        IF TG_OP = 'DELETE' THEN v_job_id := OLD.id; ELSE v_job_id := NEW.id; END IF;
        SELECT run.id INTO v_run_id
        FROM public.stock_analysis_runs AS run
        WHERE run.job_id = v_job_id;
    ELSE
        IF TG_OP = 'DELETE' THEN v_job_id := OLD.job_id; ELSE v_job_id := NEW.job_id; END IF;
        SELECT run.id INTO v_run_id
        FROM public.stock_analysis_runs AS run
        WHERE run.job_id = v_job_id;
    END IF;

    IF v_run_id IS NULL OR v_job_id IS NULL THEN RETURN NULL; END IF;
    SELECT run.status, job.status, job.attempt_count
    INTO v_run_status, v_job_status, v_attempt_count
    FROM public.stock_analysis_runs AS run
    JOIN public.jobs AS job ON job.id = run.job_id
    WHERE run.id = v_run_id;
    IF NOT FOUND THEN RETURN NULL; END IF;

    IF v_run_status = 'SUCCEEDED' OR v_job_status = 'SUCCEEDED' THEN
        IF v_run_status <> 'SUCCEEDED'
            OR v_job_status <> 'SUCCEEDED'
            OR NOT EXISTS (
                SELECT 1
                FROM public.job_attempts AS attempt
                WHERE attempt.job_id = v_job_id
                  AND attempt.attempt_no = v_attempt_count
                  AND attempt.outcome = 'SUCCEEDED'
                  AND attempt.finished_at IS NOT NULL
            )
            OR NOT EXISTS (
                SELECT 1
                FROM public.candidate_feed_snapshots AS feed
                WHERE feed.run_id = v_run_id
            )
        THEN
            RAISE EXCEPTION 'candidate publication and queue settlement must commit atomically'
                USING ERRCODE = '23514';
        END IF;
    ELSIF v_run_status IN ('FAILED', 'BLOCKED')
        OR v_job_status IN ('FAILED', 'CANCELED')
    THEN
        IF v_run_status NOT IN ('FAILED', 'BLOCKED')
            OR v_job_status NOT IN ('FAILED', 'CANCELED')
        THEN
            RAISE EXCEPTION 'candidate failure and queue settlement must commit atomically'
                USING ERRCODE = '23514';
        END IF;
    END IF;
    RETURN NULL;
END
$assert$;

DROP TRIGGER owner_beta_simulation_attempts_deferred_guard ON public.job_attempts;
DROP TRIGGER owner_beta_simulation_pairs_deferred_guard
    ON public.owner_beta_simulation_result_pairs;
DROP TRIGGER owner_beta_simulation_runs_deferred_guard
    ON public.owner_beta_simulation_runs;
DROP TRIGGER owner_beta_simulation_jobs_deferred_guard ON public.jobs;

DROP TRIGGER owner_beta_simulation_attempt_guard ON public.job_attempts;
DROP TRIGGER owner_beta_simulation_jobs_validate_insert_payload ON public.jobs;
DROP TRIGGER owner_beta_simulation_runs_lock_config_on_success
    ON public.owner_beta_simulation_runs;
DROP TRIGGER owner_beta_simulation_pairs_validate_insert
    ON public.owner_beta_simulation_result_pairs;
DROP TRIGGER owner_beta_simulation_runs_validate_binding
    ON public.owner_beta_simulation_runs;
DROP TRIGGER owner_beta_simulation_jobs_lineage_guard ON public.jobs;
DROP TRIGGER owner_beta_simulation_jobs_insert_guard ON public.jobs;

DROP POLICY owner_beta_simulation_pairs_app_select
    ON public.owner_beta_simulation_result_pairs;
DROP FUNCTION public.owner_beta_simulation_pair_read_allowed(uuid, uuid);

DROP POLICY owner_beta_simulation_jobs_trigger_select ON public.jobs;
ALTER POLICY tenant_all_owner_jobs ON public.jobs TO migration_owner
    USING (owner_user_id = current_setting('app.actor_user_id', true)::uuid)
    WITH CHECK (owner_user_id = current_setting('app.actor_user_id', true)::uuid);
DROP TABLE public.owner_beta_simulation_result_pairs;
DROP TABLE public.owner_beta_simulation_runs;

DROP FUNCTION public.owner_beta_simulation_deferred_guard();
DROP FUNCTION public.owner_beta_simulation_attempt_guard();
DROP FUNCTION public.owner_beta_simulation_pairs_validate_insert();
DROP FUNCTION public.owner_beta_simulation_jobs_validate_insert_payload();
DROP FUNCTION public.owner_beta_simulation_runs_lock_config_on_success();
DROP FUNCTION public.owner_beta_simulation_runs_validate_binding();
DROP FUNCTION public.owner_beta_simulation_jobs_lineage_guard();
DROP FUNCTION public.owner_beta_simulation_jobs_insert_guard();
DROP FUNCTION public.owner_beta_simulation_payload_matches(
    jsonb, uuid, uuid, date, date, text, text, text, text, text, text, text,
    text, text, jsonb, text
);
DROP FUNCTION public.lock_owner_beta_simulation_entitlement(uuid, date[]);
