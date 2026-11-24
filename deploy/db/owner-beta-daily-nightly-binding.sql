-- Check the exact selected Daily control before beginning a nightly attempt.
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL statement_timeout = '10s';
SET LOCAL lock_timeout = '5s';
SET LOCAL search_path = pg_catalog, pg_temp;
DO $binding$
DECLARE
    config jsonb := convert_from(decode('__CONFIG_HEX__','hex'),'UTF8')::jsonb;
    control public.owner_beta_daily_control%ROWTYPE;
BEGIN
    IF current_user <> 'migration_owner' OR session_user <> 'migration_owner'
       OR (SELECT count(*) FROM public.owner_beta_daily_control) <> 1 THEN
        RAISE EXCEPTION 'DAILY_BACKUP_NIGHTLY_BINDING' USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT control FROM public.owner_beta_daily_control WHERE id='etf11';
    IF control.activation_epoch < 1
       OR ROW(control.owner_user_id,control.policy_sha256,control.entitlement_reference,
              control.entitlement_sha256,control.first_auto_session,control.trigger_version,
              control.max_jobs_per_owner,control.lease_seconds,control.backoff_seconds)
        IS DISTINCT FROM ROW((config->>'owner')::uuid,config->>'policy_sha256',
              config->>'entitlement_reference',config->>'entitlement_sha256',
              (config->>'first_auto_session')::date,'owner-beta-daily-trigger-v1',
              __CAPACITY__::integer,__LEASE__::integer,__BACKOFF__::bigint) THEN
        RAISE EXCEPTION 'DAILY_BACKUP_NIGHTLY_BINDING' USING ERRCODE='23514';
    END IF;
END
$binding$;
SELECT CASE WHEN enabled THEN 'DAILY_NIGHTLY_CONTROL: ENABLED' ELSE 'DAILY_NIGHTLY_CONTROL: DISABLED' END
FROM public.owner_beta_daily_control WHERE id='etf11';
ROLLBACK;
