-- Disable and remove the candidate scheduler only when no durable candidate
-- job/run lineage or reserved-principal dependency remains.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

SELECT pg_catalog.pg_advisory_xact_lock(1815099521, 44);
UPDATE public.candidate_scheduler_control
SET active = false, updated_at = clock_timestamp()
WHERE control_key = 'scheduler';

DO $rollback_guard$
DECLARE
    v_service_user_id uuid;
BEGIN
    SELECT service_user_id INTO v_service_user_id
    FROM public.candidate_scheduler_control
    WHERE control_key = 'scheduler';
    IF EXISTS (SELECT 1 FROM public.stock_analysis_runs)
        OR EXISTS (
            SELECT 1 FROM public.jobs
            WHERE job_type = 'candidate_compute'
               OR idempotency_key LIKE 'candidate:scheduled:%'
        )
    THEN
        RAISE EXCEPTION '0044 rollback blocked by candidate job or run lineage'
            USING ERRCODE = '55000';
    END IF;
    IF EXISTS (SELECT 1 FROM public.user_roles WHERE user_id = v_service_user_id)
        OR EXISTS (SELECT 1 FROM public.web_sessions WHERE user_id = v_service_user_id)
        OR EXISTS (SELECT 1 FROM public.invitations WHERE user_id = v_service_user_id)
        OR EXISTS (
            SELECT 1 FROM public.screener_saved_screens
            WHERE owner_user_id = v_service_user_id
        )
    THEN
        RAISE EXCEPTION '0044 rollback blocked by reserved service principal dependencies'
            USING ERRCODE = '55000';
    END IF;
END
$rollback_guard$;

DROP FUNCTION public.candidate_published_source_attributions(uuid);

DROP TRIGGER candidate_attempt_requires_publication ON public.job_attempts;
DROP TRIGGER candidate_job_requires_publication ON public.jobs;
DROP TRIGGER stock_analysis_run_requires_settlement ON public.stock_analysis_runs;
DROP FUNCTION public.assert_candidate_publication_settlement();

REVOKE ALL ON FUNCTION public.fail_candidate_analysis_run(uuid, uuid, text, text, text, jsonb)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
DROP FUNCTION public.fail_candidate_analysis_run(uuid, uuid, text, text, text, jsonb);
REVOKE ALL ON FUNCTION public.publish_candidate_analysis(uuid, uuid, integer, text, jsonb, jsonb)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
DROP FUNCTION public.publish_candidate_analysis(uuid, uuid, integer, text, jsonb, jsonb);
REVOKE ALL ON FUNCTION public.schedule_candidate_run(
    date, timestamptz, text, text, uuid, uuid, integer, text, uuid, text, uuid, text,
    uuid, text, uuid
) FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
DROP FUNCTION public.schedule_candidate_run(
    date, timestamptz, text, text, uuid, uuid, integer, text, uuid, text, uuid, text,
    uuid, text, uuid
);
REVOKE EXECUTE ON FUNCTION public.candidate_source_entitlement_is_valid(
    uuid, text, text, date, date
) FROM worker;

DROP TRIGGER jobs_protect_candidate_scheduled_lineage ON public.jobs;
DROP FUNCTION public.jobs_reject_candidate_scheduled_mutation();
DROP TABLE public.candidate_scheduler_control;

DELETE FROM public.users
WHERE id = '00000000-0000-4000-8000-000000000042'::uuid
  AND issuer = 'urn:lagrange:internal'
  AND subject = 'candidate-scheduler-v1'
  AND email = 'candidate-scheduler@system.invalid';
