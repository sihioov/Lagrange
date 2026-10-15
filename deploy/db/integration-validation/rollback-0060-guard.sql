-- Only the hosted disposable staged lane. No deletion of used daily evidence.
\set ON_ERROR_STOP on
SET app.actor_user_id = '00000000-0000-4000-8000-000000000060';
\if :unused
BEGIN;
\i /validation/migrations/60/0060_owner_beta_daily_inputs.down.sql
DO $$ BEGIN
    IF public.validation_0060_history() IS DISTINCT FROM (SELECT history FROM public.validation_0060_snapshot)
       OR (SELECT jsonb_agg(pg_get_functiondef(oid) ORDER BY proname) FROM pg_proc WHERE pronamespace='public'::regnamespace AND proname IN
         ('owner_beta_recommendation_runs_validate_job_binding','owner_beta_recommendation_runs_lock_strategy_config_on_success')) IS DISTINCT FROM (SELECT functions FROM public.validation_0060_snapshot)
       OR NOT has_table_privilege('app','public.user_strategy_configs','INSERT')
       OR NOT has_table_privilege('app','public.user_strategy_configs','UPDATE')
       OR NOT (SELECT relforcerowsecurity FROM pg_class WHERE oid='public.user_strategy_configs'::regclass)
       OR to_regclass('public.owner_beta_daily_control') IS NOT NULL THEN
        RAISE EXCEPTION '0060 unused rollback did not restore baseline'; END IF;
END $$;
-- The migration ledger is intentionally untouched by this atomic rehearsal.
\i /validation/migrations/60/0060_owner_beta_daily_inputs.up.sql
COMMIT;
SELECT 'DAILY_0060_HARNESS:unused_down_up:PASS';
\elif :seed_hidden
BEGIN;
INSERT INTO public.users(id,issuer,subject,email) VALUES
 ('00000000-0000-4000-8000-000000000061','https://validation.invalid','hidden-0060','hidden0060@example.invalid');
SET LOCAL app.actor_user_id = '00000000-0000-4000-8000-000000000061';
INSERT INTO public.user_strategy_configs(owner_user_id,strategy_id,strategy_version,config_json)
 VALUES('00000000-0000-4000-8000-000000000061','validation_daily_0060','1.0.0','{}');
UPDATE public.user_strategy_configs SET config_json='{"edit":true}';
UPDATE public.user_strategy_configs SET config_json='{}';
COMMIT;
SELECT 'DAILY_0060_HARNESS:hidden_epoch_seed:PASS';
\elif :attempt
BEGIN;
-- Expected SQLSTATE 55000. Disconnect rolls back every DDL/RLS change.
\i /validation/migrations/60/0060_owner_beta_daily_inputs.down.sql
ROLLBACK;
\else
DO $$ BEGIN
    IF public.validation_0060_history() IS DISTINCT FROM (SELECT history FROM public.validation_0060_snapshot)
      OR (SELECT count(*) FROM public._sqlx_migrations)<>60
      OR (SELECT count(*) FROM pg_class WHERE relnamespace='public'::regnamespace AND relkind='r' AND relname LIKE 'owner_beta_daily_%' AND relrowsecurity AND relforcerowsecurity)<>10
      OR NOT (SELECT relforcerowsecurity FROM pg_class WHERE oid='public.user_strategy_configs'::regclass)
      OR EXISTS(SELECT 1 FROM public.user_strategy_configs WHERE owner_user_id='00000000-0000-4000-8000-000000000061') THEN
        RAISE EXCEPTION '0060 refused rollback was not atomic'; END IF;
    PERFORM set_config('app.actor_user_id','00000000-0000-4000-8000-000000000061',true);
    IF (SELECT daily_epoch FROM public.user_strategy_configs) IS DISTINCT FROM 2::bigint THEN
        RAISE EXCEPTION '0060 hidden history was lost'; END IF;
END $$;
SELECT 'DAILY_0060_HARNESS:hidden_down_refused_atomic:PASS';
\endif
