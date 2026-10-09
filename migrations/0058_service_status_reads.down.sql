SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

REVOKE ALL ON FUNCTION public.owner_service_status_stream(
    uuid, text, uuid, uuid, text, text, timestamptz, timestamptz
) FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
DROP FUNCTION public.owner_service_status_stream(
    uuid, text, uuid, uuid, text, text, timestamptz, timestamptz
);

DROP INDEX public.owner_beta_service_status_owner_finished_idx;
DROP INDEX public.jobs_service_status_owner_type_finished_idx;
DROP INDEX public.jobs_service_status_owner_type_status_available_idx;
