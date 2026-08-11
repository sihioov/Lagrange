-- Fail before 0032.down can remove the first recommendation-family index.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Drain calls holding the shared transaction fence. New calls queue behind
-- this exclusive waiter and must re-read active=false after this commits.
DO $deactivation$
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(1815099521, 33);
    UPDATE public.recommendation_scheduler_control
    SET active = false
    WHERE control_key = 'scheduler';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'recommendation scheduler control row is missing'
            USING ERRCODE = '55000';
    END IF;
END
$deactivation$;

REVOKE EXECUTE ON FUNCTION
    public.schedule_recommendation_run(uuid, uuid, date, uuid, text, integer, text)
    FROM worker;

SELECT public.assert_no_scheduled_recommendation_lineage();
DROP FUNCTION public.assert_no_scheduled_recommendation_lineage();
