SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

DROP TRIGGER IF EXISTS jobs_sync_recommendation_terminal_run ON public.jobs;
DROP FUNCTION public.sync_recommendation_run_from_terminal_job();
REVOKE EXECUTE ON FUNCTION public.lock_recommendation_source_pins(uuid[], text[], text[]) FROM worker;
DROP FUNCTION public.lock_recommendation_source_pins(uuid[], text[], text[]);
REVOKE EXECUTE ON FUNCTION public.lock_recommendation_entitlement(uuid, text, date) FROM worker;
DROP FUNCTION public.lock_recommendation_entitlement(uuid, text, date);
