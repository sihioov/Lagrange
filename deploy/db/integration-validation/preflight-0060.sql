-- Hosted disposable staged lane only. Never a production seed.
\set ON_ERROR_STOP on
SET app.actor_user_id = '00000000-0000-4000-8000-000000000060';
DO $$ BEGIN
    IF current_user <> 'migration_owner' OR session_user <> current_user THEN
        RAISE EXCEPTION '0060 preflight requires direct migration login';
    END IF;
END $$;
\if :seed
DO $$
DECLARE v integer; state text; cfg uuid; rid uuid; jid uuid; pin text; payload jsonb;
BEGIN
    IF (SELECT count(*) FROM public._sqlx_migrations)<>59 THEN
        RAISE EXCEPTION '0060 seed requires exact 0059 frontier'; END IF;
    INSERT INTO public.users(id,issuer,subject,email) VALUES
      ('00000000-0000-4000-8000-000000000060','https://validation.invalid','daily-0060','daily0060@example.invalid');
    INSERT INTO public.roles(id) VALUES('owner') ON CONFLICT DO NOTHING;
    INSERT INTO public.user_roles(user_id,role_id) VALUES('00000000-0000-4000-8000-000000000060','owner');
    INSERT INTO public.strategies(id,display_name) VALUES('validation_daily_0060','0060 historical fixture');
    INSERT INTO public.instruments(id,symbol,venue,currency) VALUES('069500.KRX','069500','KRX','KRW') ON CONFLICT DO NOTHING;
    INSERT INTO public.user_strategy_configs(owner_user_id,strategy_id,strategy_version,config_json)
      VALUES('00000000-0000-4000-8000-000000000060','validation_daily_0060','1.0.0','{}') RETURNING id INTO cfg;
    FOREACH v IN ARRAY ARRAY[2,3] LOOP
      FOREACH state IN ARRAY ARRAY['PENDING','FAILED','CANCELED','SUCCEEDED'] LOOP
        rid:=gen_random_uuid(); jid:=gen_random_uuid(); pin:='sha256:'||repeat(v::text,64);
        payload:=jsonb_build_object('run_id',rid,'strategy_config_id',cfg,'as_of','2026-08-28',
          'pins',jsonb_build_object('candidate_content_sha256',pin,'artifact_manifest_sha256',pin,'stage5_manifest_sha256',pin,'action_manifest_sha256',pin,'approval_registry_sha256',pin),
          'strategy',jsonb_build_object('strategy_id','validation_daily_0060','strategy_version','1.0.0','config_json','{}'::jsonb,'config_sha256',pin));
        INSERT INTO public.jobs(id,owner_user_id,job_type,payload_json,idempotency_key)
          VALUES(jid,'00000000-0000-4000-8000-000000000060','owner_beta_price_recommendation',payload,'validation-0060-'||v||'-'||state);
        INSERT INTO public.owner_beta_recommendation_runs(id,owner_user_id,strategy_config_id,job_id,as_of,
          strategy_id,strategy_version,strategy_config_json,strategy_config_sha256,
          candidate_content_sha256,artifact_manifest_sha256,stage5_manifest_sha256,action_manifest_sha256,approval_registry_sha256)
          VALUES(rid,'00000000-0000-4000-8000-000000000060',cfg,jid,'2026-08-28','validation_daily_0060','1.0.0','{}',pin,pin,pin,pin,pin,pin);
        IF state='SUCCEEDED' THEN
          INSERT INTO public.owner_beta_recommendation_items(recommendation_run_id,owner_user_id,instrument_id,rank,target_weight)
            VALUES(rid,'00000000-0000-4000-8000-000000000060','069500.KRX',1,1);
          UPDATE public.owner_beta_recommendation_runs SET status=state,factor_snapshot_sha256=pin,target_snapshot_sha256=pin,cash_weight=0,finished_at='2026-08-28 09:00Z' WHERE id=rid;
        ELSIF state<>'PENDING' THEN
          UPDATE public.owner_beta_recommendation_runs SET status=state,finished_at='2026-08-28 09:00Z' WHERE id=rid;
        END IF;
        UPDATE public.jobs SET status=CASE WHEN state='PENDING' THEN 'QUEUED' ELSE state END WHERE id=jid;
      END LOOP;
    END LOOP;
END $$;
CREATE FUNCTION public.validation_0060_history() RETURNS jsonb LANGUAGE sql
SET search_path=pg_catalog,pg_temp AS $$
SELECT jsonb_build_object(
 'runs',(SELECT jsonb_agg(to_jsonb(r)-ARRAY['trigger_kind','daily_pins','daily_config_epoch','daily_activation_epoch','daily_trigger_version'] ORDER BY id) FROM public.owner_beta_recommendation_runs r),
 'items',(SELECT jsonb_agg(to_jsonb(i) ORDER BY id) FROM public.owner_beta_recommendation_items i),
 'jobs',(SELECT jsonb_agg(to_jsonb(j) ORDER BY id) FROM public.jobs j WHERE owner_user_id='00000000-0000-4000-8000-000000000060'),
 'configs',(SELECT jsonb_agg(to_jsonb(c)-ARRAY['daily_enabled','daily_epoch'] ORDER BY id) FROM public.user_strategy_configs c))
$$;
REVOKE ALL ON FUNCTION public.validation_0060_history() FROM PUBLIC;
CREATE TABLE public.validation_0060_snapshot AS SELECT public.validation_0060_history() AS history,
  (SELECT jsonb_agg(pg_get_functiondef(oid) ORDER BY proname) FROM pg_proc WHERE pronamespace='public'::regnamespace AND proname IN
   ('owner_beta_recommendation_runs_validate_job_binding','owner_beta_recommendation_runs_lock_strategy_config_on_success')) AS functions,
  (SELECT jsonb_agg(jsonb_build_array(version,success,encode(checksum,'hex')) ORDER BY version) FROM public._sqlx_migrations) AS ledger;
REVOKE ALL ON public.validation_0060_snapshot FROM PUBLIC;
SELECT 'DAILY_0060_HARNESS:history_seed_0059:PASS';
\else
DO $$ BEGIN
    IF (SELECT count(*) FROM public._sqlx_migrations)<>60 OR EXISTS(SELECT 1 FROM public._sqlx_migrations WHERE NOT success) THEN
        RAISE EXCEPTION '0060 ledger incomplete'; END IF;
    IF public.validation_0060_history() IS DISTINCT FROM (SELECT history FROM public.validation_0060_snapshot)
      OR (SELECT jsonb_agg(jsonb_build_array(version,success,encode(checksum,'hex')) ORDER BY version) FROM public._sqlx_migrations WHERE version<=59)
        IS DISTINCT FROM (SELECT ledger FROM public.validation_0060_snapshot) THEN
        RAISE EXCEPTION '0060 changed historical rows or checksums'; END IF;
    IF (SELECT count(*) FROM public.owner_beta_recommendation_runs)<>8
      OR EXISTS(SELECT 1 FROM public.user_strategy_configs WHERE daily_enabled OR daily_epoch<>0)
      OR EXISTS(SELECT 1 FROM public.owner_beta_daily_control) THEN
        RAISE EXCEPTION '0060 auto activation or historical fixture loss'; END IF;
    IF (SELECT count(*) FROM pg_class WHERE relnamespace='public'::regnamespace AND relkind='r' AND relname LIKE 'owner_beta_daily_%'
        AND relrowsecurity AND relforcerowsecurity AND pg_get_userbyid(relowner)='migration_owner')<>10 THEN
        RAISE EXCEPTION '0060 catalog authority'; END IF;
END $$;
SELECT 'DAILY_0060_HARNESS:history_preserved_0060:PASS';
\endif
