SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

DROP TRIGGER IF EXISTS jobs_protect_owner_beta_recommendation_lineage
    ON public.jobs;
DROP TRIGGER IF EXISTS owner_beta_recommendation_runs_validate_job_binding
    ON public.owner_beta_recommendation_runs;

DROP FUNCTION IF EXISTS public.jobs_protect_owner_beta_recommendation_lineage();
DROP FUNCTION IF EXISTS public.owner_beta_recommendation_runs_validate_job_binding();

DROP POLICY IF EXISTS owner_beta_recommendation_items_owner_all
    ON public.owner_beta_recommendation_items;
DROP POLICY IF EXISTS owner_beta_recommendation_items_admin_select
    ON public.owner_beta_recommendation_items;
DROP POLICY IF EXISTS owner_beta_recommendation_items_worker_insert
    ON public.owner_beta_recommendation_items;
DROP POLICY IF EXISTS owner_beta_recommendation_items_worker_select
    ON public.owner_beta_recommendation_items;
DROP POLICY IF EXISTS owner_beta_recommendation_items_app_select
    ON public.owner_beta_recommendation_items;

DROP POLICY IF EXISTS owner_beta_recommendation_runs_owner_all
    ON public.owner_beta_recommendation_runs;
DROP POLICY IF EXISTS owner_beta_recommendation_runs_admin_select
    ON public.owner_beta_recommendation_runs;
DROP POLICY IF EXISTS owner_beta_recommendation_runs_worker_update
    ON public.owner_beta_recommendation_runs;
DROP POLICY IF EXISTS owner_beta_recommendation_runs_worker_select
    ON public.owner_beta_recommendation_runs;
DROP POLICY IF EXISTS owner_beta_recommendation_runs_app_insert
    ON public.owner_beta_recommendation_runs;
DROP POLICY IF EXISTS owner_beta_recommendation_runs_app_select
    ON public.owner_beta_recommendation_runs;

DROP TABLE IF EXISTS public.owner_beta_recommendation_items;
DROP TABLE IF EXISTS public.owner_beta_recommendation_runs;
