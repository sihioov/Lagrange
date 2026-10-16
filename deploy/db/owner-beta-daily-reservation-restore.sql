-- Fixed maintenance transaction, invoked only after installed-release binding,
-- DB disable and owned-process drain, inside a complete filesystem snapshot.
-- File/DB reconciliation must pass afterwards before enable. No enqueue,
-- reseal, history deletion, trigger bypass or grants. Caller uses ON_ERROR_STOP.
BEGIN;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '120s';
SET LOCAL idle_in_transaction_session_timeout = '15s';
SET LOCAL search_path = pg_catalog, pg_temp;
SET LOCAL TimeZone = 'UTC';
SET LOCAL DateStyle = 'ISO, YMD';
DO $role$
BEGIN
    IF current_user <> 'migration_owner' OR session_user <> 'migration_owner' THEN
        RAISE EXCEPTION 'DAILY_RESTORE_ROLE' USING ERRCODE='42501';
    END IF;
END $role$;

-- Block mutations for the complete repair. The ordinary job/attempt/run lock
-- order is retained; lock timeout fails instead of bypassing a live writer.
LOCK TABLE public.jobs, public.job_attempts,
    public.owner_beta_recommendation_runs, public.owner_beta_recommendation_items,
    public.owner_beta_daily_reservations, public.owner_beta_daily_publication_claims,
    public.owner_beta_daily_control, public.owner_beta_daily_admissions
    IN SHARE ROW EXCLUSIVE MODE;

-- Reuse the installed deferred consistency implementation without rewriting
-- any existing row merely to fire its trigger. This table/trigger are TEMP only.
CREATE TEMP TABLE daily_restore_probe(owner_user_id uuid NOT NULL, job_id uuid PRIMARY KEY)
    ON COMMIT DROP;
CREATE TRIGGER daily_restore_probe AFTER INSERT ON daily_restore_probe
    FOR EACH ROW EXECUTE FUNCTION public.daily_0060_consistency();

DO $repair$
DECLARE
    control public.owner_beta_daily_control;
    actor uuid;
    run_row public.owner_beta_recommendation_runs;
    job_row public.jobs;
    old_actor text := current_setting('app.actor_user_id', true);
    actors integer := 0;
    runs integer := 0;
BEGIN
    IF (SELECT count(*) FROM public.owner_beta_daily_control) <> 1 THEN
        RAISE EXCEPTION 'DAILY_RESTORE_CONTROL' USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT control FROM public.owner_beta_daily_control WHERE id='etf11';
    IF control.enabled OR EXISTS (
        SELECT 1 FROM public.owner_beta_daily_reservations
        WHERE owner_user_id <> control.owner_user_id
    ) OR EXISTS (
        SELECT 1 FROM public.owner_beta_daily_publication_claims
        WHERE owner_user_id <> control.owner_user_id
    ) THEN
        RAISE EXCEPTION 'DAILY_RESTORE_DISABLED_OWNER' USING ERRCODE='23514';
    END IF;
    -- users has RLS enabled but not forced (0010). Require this actual
    -- migration_owner session to see the complete identity set without
    -- changing policy, role attributes or row_security configuration.
    IF pg_catalog.row_security_active('public.users'::regclass) THEN
        RAISE EXCEPTION 'DAILY_RESTORE_IDENTITY_VISIBILITY' USING ERRCODE='23514';
    END IF;
    FOR actor IN SELECT id FROM public.users ORDER BY id LIMIT 10001 LOOP
        actors := actors + 1;
        IF actors > 10000 THEN
            RAISE EXCEPTION 'DAILY_RESTORE_LIMIT' USING ERRCODE='54000';
        END IF;
        PERFORM set_config('app.actor_user_id',actor::text,true);
        FOR run_row IN
            SELECT * FROM public.owner_beta_recommendation_runs
            WHERE owner_user_id=actor AND
                  (input_kind='owner_beta_daily_etf11_price_only_v1' OR trigger_kind='DAILY')
            ORDER BY id LIMIT 100001
        LOOP
            runs := runs + 1;
            IF runs > 100000 THEN
                RAISE EXCEPTION 'DAILY_RESTORE_LIMIT' USING ERRCODE='54000';
            END IF;
            SELECT * INTO job_row FROM public.jobs WHERE id=run_row.job_id;
            IF NOT FOUND OR run_row.owner_user_id <> control.owner_user_id
               OR job_row.owner_user_id IS DISTINCT FROM run_row.owner_user_id
               OR run_row.input_kind <> 'owner_beta_daily_etf11_price_only_v1'
               OR run_row.trigger_kind <> 'DAILY'
               OR job_row.job_type <> 'owner_beta_price_recommendation'
               OR job_row.payload_json IS DISTINCT FROM public.daily_0060_payload(run_row)
               OR job_row.idempotency_key IS DISTINCT FROM
                  'owner-beta:daily:v1:' || run_row.owner_user_id::text || ':' ||
                  run_row.strategy_config_id::text || ':' || run_row.as_of::text
            THEN
                RAISE EXCEPTION 'DAILY_RESTORE_ORIGINAL_BINDING' USING ERRCODE='23514';
            END IF;
            IF NOT EXISTS (SELECT 1 FROM public.owner_beta_daily_reservations
                           WHERE run_id=run_row.id) THEN
                -- Every field comes from the surviving immutable run/job. A
                -- conflicting permanent key/job fails the whole transaction.
                -- created_at is the actual repair time, never an invented past.
                INSERT INTO public.owner_beta_daily_reservations
                    (owner_user_id,strategy_config_id,session_date,trigger_version,
                     run_id,job_id,admission_sha256,strategy_config_sha256,
                     config_epoch,activation_epoch)
                VALUES (run_row.owner_user_id,run_row.strategy_config_id,run_row.as_of,
                        run_row.daily_trigger_version,run_row.id,run_row.job_id,
                        run_row.daily_pins->>'admission_sha256',run_row.strategy_config_sha256,
                        run_row.daily_config_epoch,run_row.daily_activation_epoch);
            END IF;
            INSERT INTO pg_temp.daily_restore_probe VALUES(actor,run_row.job_id);
        END LOOP;
        IF EXISTS (
            SELECT 1 FROM public.jobs j
            WHERE j.owner_user_id=actor AND
                  (j.payload_json->>'schema'='owner-beta-daily-recommendation-v1'
                   OR j.idempotency_key LIKE 'owner-beta:daily:v1:%')
              AND NOT EXISTS (SELECT 1 FROM pg_temp.daily_restore_probe p WHERE p.job_id=j.id)
        ) THEN
            RAISE EXCEPTION 'DAILY_RESTORE_ORPHAN_JOB' USING ERRCODE='23514';
        END IF;
    END LOOP;
    IF EXISTS (SELECT 1 FROM public.owner_beta_daily_reservations r
               WHERE NOT EXISTS (SELECT 1 FROM pg_temp.daily_restore_probe p WHERE p.job_id=r.job_id))
       OR EXISTS (SELECT 1 FROM public.owner_beta_daily_publication_claims c
                  WHERE NOT EXISTS (SELECT 1 FROM pg_temp.daily_restore_probe p WHERE p.job_id=c.job_id))
    THEN
        RAISE EXCEPTION 'DAILY_RESTORE_ORPHAN_HISTORY' USING ERRCODE='23514';
    END IF;
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
END $repair$;
SET CONSTRAINTS ALL IMMEDIATE;
COMMIT;
SELECT 'DAILY_BACKUP_RESERVATIONS: VERIFIED' AS daily_backup_result;
