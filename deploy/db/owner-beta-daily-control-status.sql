-- Fixed read-only bootstrap/maintenance observation. No Owner/data values print.
\set ON_ERROR_STOP on
BEGIN READ ONLY;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '15s';
SET LOCAL idle_in_transaction_session_timeout = '20s';
SET LOCAL search_path = pg_catalog, pg_temp;
DO $daily_control_role$
DECLARE
    table_name text;
    present_count integer := 0;
    history_present boolean := false;
    has_rows boolean;
    control_present boolean := false;
    actor uuid;
    actor_count integer := 0;
    old_actor text := current_setting('app.actor_user_id',true);
BEGIN
    IF current_user <> 'migration_owner' OR session_user <> 'migration_owner' THEN
        RAISE EXCEPTION 'DAILY_CONTROL_ROLE' USING ERRCODE='42501';
    END IF;
    FOREACH table_name IN ARRAY ARRAY[
        'owner_beta_daily_control','owner_beta_daily_admissions',
        'owner_beta_daily_admission_events','owner_beta_daily_calendar_sources',
        'owner_beta_daily_calendar_days','owner_beta_daily_source_events',
        'owner_beta_daily_poll_receipts','owner_beta_daily_poll_state',
        'owner_beta_daily_reservations','owner_beta_daily_publication_claims'
    ] LOOP
        IF to_regclass('public.' || table_name) IS NOT NULL THEN
            present_count := present_count + 1;
            EXECUTE format('SELECT EXISTS (SELECT 1 FROM public.%I)',table_name) INTO has_rows;
            IF table_name='owner_beta_daily_control' THEN
                control_present := has_rows;
            ELSE
                history_present := history_present OR has_rows;
            END IF;
        END IF;
    END LOOP;
    -- Runs use actor-scoped migration-owner RLS, unlike the ten daily tables.
    -- Enumerate identities only when this direct owner is effectively unfiltered;
    -- a NULL actor must never be mistaken for an empty history.
    IF NOT control_present AND to_regclass('public.owner_beta_recommendation_runs') IS NOT NULL THEN
        IF EXISTS (SELECT 1 FROM pg_attribute
                   WHERE attrelid='public.owner_beta_recommendation_runs'::regclass
                     AND attname='input_kind' AND NOT attisdropped) THEN
            IF to_regclass('public.users') IS NULL OR
               pg_catalog.row_security_active(to_regclass('public.users')) THEN
                RAISE EXCEPTION 'DAILY_CONTROL_IDENTITY_VISIBILITY' USING ERRCODE='23514';
            END IF;
            FOR actor IN SELECT id FROM public.users ORDER BY id LIMIT 10001 LOOP
                actor_count := actor_count + 1;
                IF actor_count > 10000 THEN
                    RAISE EXCEPTION 'DAILY_CONTROL_IDENTITY_LIMIT' USING ERRCODE='23514';
                END IF;
                PERFORM set_config('app.actor_user_id',actor::text,true);
                EXECUTE 'SELECT EXISTS (SELECT 1 FROM public.owner_beta_recommendation_runs
                         WHERE input_kind=''owner_beta_daily_etf11_price_only_v1'')' INTO has_rows;
                history_present := history_present OR has_rows;
                EXIT WHEN has_rows;
            END LOOP;
            PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
        END IF;
    END IF;
    IF present_count NOT IN (0,10) OR (history_present AND NOT control_present) THEN
        RAISE EXCEPTION 'DAILY_CONTROL_INCOMPLETE_HISTORY' USING ERRCODE='23514';
    END IF;
END $daily_control_role$;
SELECT to_regclass('public.owner_beta_daily_control') IS NOT NULL AS daily_present \gset
\if :daily_present
SELECT CASE
    WHEN count(*)=0 THEN 'DAILY_CONTROL_STATUS: ABSENT_CONTROL'
    WHEN count(*)=1 AND bool_and(id='etf11' AND NOT enabled)
        THEN 'DAILY_CONTROL_STATUS: DISABLED'
    WHEN count(*)=1 AND bool_and(id='etf11' AND enabled)
        THEN 'DAILY_CONTROL_STATUS: ENABLED'
    ELSE 'DAILY_CONTROL_STATUS: INVALID'
END FROM public.owner_beta_daily_control;
\else
\echo DAILY_CONTROL_STATUS: ABSENT_SCHEMA
\endif
COMMIT;
