-- 0033: install the guard at the recommendation migration family boundary.
-- The paired down migration is intentionally the first reverse step and calls
-- it before any earlier schema/index/privilege teardown. FORCE RLS requires
-- inspecting one tenant at a time under migration_owner's owner policies.

CREATE FUNCTION public.assert_no_scheduled_recommendation_lineage()
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $rollback_guard$
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
            RAISE EXCEPTION 'recommendation rollback blocked by scheduled recommendation lineage'
                USING ERRCODE = '55000';
        END IF;
    END LOOP;
END
$rollback_guard$;

ALTER FUNCTION public.assert_no_scheduled_recommendation_lineage()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.assert_no_scheduled_recommendation_lineage()
    FROM PUBLIC;

-- The complete 0026..0032 support family now exists. Activate and grant in
-- this transaction so worker can never observe one without the other.
DO $activation$
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(1815099521, 33);
    UPDATE public.recommendation_scheduler_control
    SET active = true
    WHERE control_key = 'scheduler';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'recommendation scheduler control row is missing'
            USING ERRCODE = '55000';
    END IF;
END
$activation$;

GRANT EXECUTE ON FUNCTION
    public.schedule_recommendation_run(uuid, uuid, date, uuid, text, integer, text)
    TO worker;
