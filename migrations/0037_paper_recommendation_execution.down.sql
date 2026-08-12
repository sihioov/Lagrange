SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

REVOKE EXECUTE ON FUNCTION public.queue_scheduled_paper_targets(uuid, uuid, date, uuid, text, text, jsonb)
    FROM worker;
DROP FUNCTION public.queue_scheduled_paper_targets(uuid, uuid, date, uuid, text, text, jsonb);

REVOKE EXECUTE ON FUNCTION public.preflight_paper_target(uuid, uuid) FROM worker;
DROP FUNCTION public.preflight_paper_target(uuid, uuid);

REVOKE EXECUTE ON FUNCTION public.lock_recommendation_schedule_inputs(date, uuid, text, text, text)
    FROM worker;
DROP FUNCTION public.lock_recommendation_schedule_inputs(date, uuid, text, text, text);

REVOKE EXECUTE ON FUNCTION public.lock_recommendation_calendar_coverage(date) FROM worker;
DROP FUNCTION public.lock_recommendation_calendar_coverage(date);

-- Restore the exact pre-0037 Paper runner grant from migration 0014.
GRANT INSERT ON TABLE public.pending_targets TO worker;

ALTER TABLE public.pending_targets
    DROP CONSTRAINT pending_targets_non_execution_reason_shape_check,
    DROP CONSTRAINT pending_targets_dataset_lineage_all_or_none_check,
    DROP CONSTRAINT pending_targets_dataset_manifest_sha256_check,
    DROP COLUMN non_execution_reason,
    DROP COLUMN dataset_manifest_sha256,
    DROP COLUMN dataset_version_id;
