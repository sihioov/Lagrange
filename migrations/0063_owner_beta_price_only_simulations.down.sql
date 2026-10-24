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

DROP TRIGGER owner_beta_simulation_attempts_deferred_guard ON public.job_attempts;
DROP TRIGGER owner_beta_simulation_pairs_deferred_guard
    ON public.owner_beta_simulation_result_pairs;
DROP TRIGGER owner_beta_simulation_runs_deferred_guard
    ON public.owner_beta_simulation_runs;
DROP TRIGGER owner_beta_simulation_jobs_deferred_guard ON public.jobs;

DROP TRIGGER owner_beta_simulation_attempt_guard ON public.job_attempts;
DROP TRIGGER owner_beta_simulation_jobs_set_actor ON public.jobs;
DROP TRIGGER owner_beta_simulation_jobs_validate_insert_payload ON public.jobs;
DROP TRIGGER owner_beta_simulation_runs_lock_config_on_success
    ON public.owner_beta_simulation_runs;
DROP TRIGGER owner_beta_simulation_pairs_validate_insert
    ON public.owner_beta_simulation_result_pairs;
DROP TRIGGER owner_beta_simulation_runs_validate_binding
    ON public.owner_beta_simulation_runs;
DROP TRIGGER owner_beta_simulation_jobs_lineage_guard ON public.jobs;
DROP TRIGGER owner_beta_simulation_jobs_insert_guard ON public.jobs;

DROP POLICY owner_beta_simulation_jobs_trigger_select ON public.jobs;
DROP TABLE public.owner_beta_simulation_result_pairs;
DROP TABLE public.owner_beta_simulation_runs;

DROP FUNCTION public.owner_beta_simulation_deferred_guard();
DROP FUNCTION public.owner_beta_simulation_attempt_guard();
DROP FUNCTION public.owner_beta_simulation_jobs_set_actor();
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
