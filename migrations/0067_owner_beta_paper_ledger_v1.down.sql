-- Only a genuinely unused 0067 install can be removed. The migration owner
-- policy is explicitly permissive while FORCE RLS is active, so this global
-- precheck sees the full beta history before the first destructive statement.
SET LOCAL lock_timeout='5s';
SET LOCAL statement_timeout='30s';

DO $$ BEGIN
  IF EXISTS(SELECT 1 FROM public.owner_beta_paper_control)
     OR EXISTS(SELECT 1 FROM public.owner_beta_paper_events)
     OR EXISTS(SELECT 1 FROM public.owner_beta_paper_lineage)
     OR EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions)
     OR EXISTS(SELECT 1 FROM public.owner_beta_paper_session_events)
     OR EXISTS(SELECT 1 FROM public.pending_targets WHERE beta_lineage_id IS NOT NULL)
     OR EXISTS(SELECT 1 FROM public.paper_rebalance_previews WHERE input_kind='OWNER_BETA_DAILY_V1' OR beta_lineage_id IS NOT NULL)
     OR EXISTS(SELECT 1 FROM public.jobs WHERE job_type='owner_beta_paper_preview' OR payload_json->>'input_kind'='OWNER_BETA_DAILY_V1')
  THEN RAISE EXCEPTION 'Owner-beta Paper ledger has durable history; 0067 down is forbidden' USING ERRCODE='23514'; END IF;
END $$;

DROP FUNCTION public.configure_owner_beta_paper_v1(uuid,text,jsonb,jsonb,text);
DROP FUNCTION public.record_owner_beta_paper_consent_v1(uuid,bigint,text,text,boolean,text);
DROP FUNCTION public.transition_owner_beta_paper_v1(bigint,text,jsonb,text);
DROP FUNCTION public.read_owner_beta_paper_state_v1();
DROP FUNCTION public.submit_owner_beta_paper_preview_v1(uuid,uuid,bigint,text,text);
DROP FUNCTION public.queue_owner_beta_paper_target_v1(uuid,bigint,text);
DROP FUNCTION public.snapshot_owner_beta_paper_preview_v1(uuid,uuid,text);
DROP FUNCTION public.publish_owner_beta_paper_preview_v1(uuid,uuid,jsonb,text);
DROP FUNCTION public.apply_owner_beta_paper_preview_v1(uuid,text,bigint,text);
DROP FUNCTION public.apply_owner_beta_paper_preview_auto_v1(uuid,bigint,text);
DROP FUNCTION public.fail_owner_beta_paper_preview_v1(uuid,uuid,jsonb,text);
DROP FUNCTION public.scan_owner_beta_paper_targets_v1(integer,text);
DROP FUNCTION public.snapshot_owner_beta_paper_target_v1(uuid,bigint,text);
DROP FUNCTION public.record_owner_beta_paper_wait_v1(uuid,bigint,text,text,text);
DROP FUNCTION public.commit_owner_beta_paper_settlement_v1(uuid,bigint,jsonb,text);
DROP FUNCTION public.commit_owner_beta_paper_valuation_v1(uuid,bigint,jsonb,text);

DROP FUNCTION public.lock_paper_rebalance_preview_submission(uuid,uuid,uuid,date);
DROP FUNCTION public.snapshot_paper_rebalance_preview(uuid,uuid,date);
DROP FUNCTION public.publish_paper_rebalance_preview(uuid,uuid,bigint,text,text,integer,date,text,jsonb,jsonb);
DROP FUNCTION public.fail_paper_rebalance_preview(uuid,uuid,jsonb);
DROP FUNCTION public.apply_paper_rebalance_preview(uuid,uuid,text,date);
DROP FUNCTION public.queue_scheduled_paper_targets(uuid,uuid,uuid,date,uuid,text,text,jsonb);
DROP FUNCTION public.preflight_paper_target(uuid,uuid);
DROP FUNCTION public.enqueue_paper_settlement_outbox(uuid,text,text,text,text,jsonb);
DROP FUNCTION public.paper_0067_worker_beta_preview(uuid);

ALTER FUNCTION public.paper_0067_generic_lock_preview(uuid,uuid,uuid,date) RENAME TO lock_paper_rebalance_preview_submission;
ALTER FUNCTION public.paper_0067_generic_snapshot_preview(uuid,uuid,date) RENAME TO snapshot_paper_rebalance_preview;
ALTER FUNCTION public.paper_0067_generic_publish_preview(uuid,uuid,bigint,text,text,integer,date,text,jsonb,jsonb) RENAME TO publish_paper_rebalance_preview;
ALTER FUNCTION public.paper_0067_generic_fail_preview(uuid,uuid,jsonb) RENAME TO fail_paper_rebalance_preview;
ALTER FUNCTION public.paper_0067_generic_apply_preview(uuid,uuid,text,date) RENAME TO apply_paper_rebalance_preview;
ALTER FUNCTION public.paper_0067_generic_queue_targets(uuid,uuid,uuid,date,uuid,text,text,jsonb) RENAME TO queue_scheduled_paper_targets;
ALTER FUNCTION public.paper_0067_generic_preflight_target(uuid,uuid) RENAME TO preflight_paper_target;
ALTER FUNCTION public.paper_0067_generic_enqueue_outbox(uuid,text,text,text,text,jsonb) RENAME TO enqueue_paper_settlement_outbox;

DO $$ DECLARE f record; BEGIN
  FOR f IN SELECT * FROM (VALUES
    ('lock_paper_rebalance_preview_submission(uuid,uuid,uuid,date)','app'),
    ('snapshot_paper_rebalance_preview(uuid,uuid,date)','worker'),
    ('publish_paper_rebalance_preview(uuid,uuid,bigint,text,text,integer,date,text,jsonb,jsonb)','worker'),
    ('fail_paper_rebalance_preview(uuid,uuid,jsonb)','worker'),
    ('apply_paper_rebalance_preview(uuid,uuid,text,date)','app'),
    ('queue_scheduled_paper_targets(uuid,uuid,uuid,date,uuid,text,text,jsonb)','worker'),
    ('preflight_paper_target(uuid,uuid)','worker'),
    ('enqueue_paper_settlement_outbox(uuid,text,text,text,text,jsonb)','app')
  ) AS x(sig,grantee) LOOP
    EXECUTE format('ALTER FUNCTION public.%s OWNER TO migration_owner',f.sig);
    EXECUTE format('ALTER FUNCTION public.%s SET search_path=pg_catalog,pg_temp',f.sig);
    EXECUTE format('REVOKE ALL ON FUNCTION public.%s FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter',f.sig);
    EXECUTE format('GRANT EXECUTE ON FUNCTION public.%s TO %I',f.sig,f.grantee);
  END LOOP;
END $$;

-- These two 0041 implementations used public in their original fixed path;
-- restore their original definitions as well as their names and grants.
ALTER FUNCTION public.preflight_paper_target(uuid,uuid) SET search_path=pg_catalog,public;
ALTER FUNCTION public.enqueue_paper_settlement_outbox(uuid,text,text,text,text,jsonb) SET search_path=pg_catalog,public;

DROP POLICY paper_0067_worker_hide_beta_targets ON public.pending_targets;
DROP POLICY paper_0067_worker_hide_beta_previews ON public.paper_rebalance_previews;

DO $$ DECLARE t text; BEGIN
  FOREACH t IN ARRAY ARRAY['accounts','account_strategy_bindings','cash_ledger','positions','orders','fills','daily_equity','pending_targets','paper_rebalance_previews'] LOOP
    EXECUTE format('DROP TRIGGER paper_0067_guard ON public.%I',t);
  END LOOP;
END $$;

ALTER TABLE public.paper_rebalance_previews DROP CONSTRAINT paper_preview_beta_lineage_fkey;
ALTER TABLE public.paper_rebalance_previews DROP CONSTRAINT paper_preview_input_kind_check;
DROP INDEX public.paper_preview_beta_lineage_mode;
DROP INDEX public.paper_preview_beta_request_key;
ALTER TABLE public.pending_targets DROP CONSTRAINT pending_targets_beta_lineage_fkey;
ALTER TABLE public.pending_targets DROP CONSTRAINT pending_targets_source_kind_check;

DROP FUNCTION public.paper_0067_apply_auto_preview(uuid,bigint,text);
DROP FUNCTION public.paper_0067_apply_target(uuid,text,bigint,text,jsonb,text);
DROP FUNCTION public.paper_0067_next_session(uuid,text,date);
DROP FUNCTION public.paper_0067_authority(text,date,bigint,text,boolean);
DROP FUNCTION public.paper_0067_account_state_sha256(bigint,text,jsonb);
DROP FUNCTION public.paper_0067_action(text,text,bigint,uuid,jsonb);
DROP FUNCTION public.paper_0067_guard_beta_write();
DROP TRIGGER owner_beta_paper_events_immutable ON public.owner_beta_paper_events;
DROP TRIGGER owner_beta_paper_lineage_immutable ON public.owner_beta_paper_lineage;
DROP TRIGGER owner_beta_paper_session_events_immutable ON public.owner_beta_paper_session_events;
DROP FUNCTION public.paper_0067_immutable();
DROP FUNCTION public.paper_0067_principal(text);
DROP FUNCTION public.paper_0067_is_beta_account(uuid);

DROP TABLE public.owner_beta_paper_session_events;
DROP TABLE public.owner_beta_paper_sessions;
ALTER TABLE public.owner_beta_paper_control DROP CONSTRAINT owner_beta_paper_control_consent_fkey;
ALTER TABLE public.owner_beta_paper_control DROP CONSTRAINT owner_beta_paper_control_activation_fkey;
ALTER TABLE public.owner_beta_paper_control DROP CONSTRAINT owner_beta_paper_control_account_fkey;
ALTER TABLE public.owner_beta_paper_control DROP CONSTRAINT owner_beta_paper_control_strategy_fkey;
DROP TABLE public.owner_beta_paper_lineage;
DROP TABLE public.owner_beta_paper_control;
DROP TABLE public.owner_beta_paper_events;

-- Beta session and event foreign keys depend on these shared-table unique
-- indexes. Remove their referencing beta tables before removing the keys.
ALTER TABLE public.pending_targets DROP CONSTRAINT pending_targets_id_owner_beta_key;
ALTER TABLE public.pending_targets DROP CONSTRAINT pending_targets_exact_beta_target_key;
ALTER TABLE public.pending_targets DROP CONSTRAINT pending_targets_id_owner_account_beta_key;
ALTER TABLE public.pending_targets DROP COLUMN beta_lineage_id;

ALTER TABLE public.paper_rebalance_previews
  ALTER COLUMN recommendation_run_id SET NOT NULL,
  ALTER COLUMN target_portfolio_id SET NOT NULL,
  ALTER COLUMN dataset_version_id SET NOT NULL,
  ALTER COLUMN dataset_manifest_sha256 SET NOT NULL;
ALTER TABLE public.paper_rebalance_previews DROP COLUMN beta_apply_mode;
ALTER TABLE public.paper_rebalance_previews DROP COLUMN beta_lineage_id;
ALTER TABLE public.paper_rebalance_previews DROP COLUMN beta_request_key;
ALTER TABLE public.paper_rebalance_previews DROP COLUMN account_state_preimage;
ALTER TABLE public.paper_rebalance_previews DROP COLUMN signal_allocation_sha256;
ALTER TABLE public.paper_rebalance_previews DROP COLUMN input_kind;
ALTER TABLE public.pending_targets ADD CONSTRAINT pending_targets_source_kind_check CHECK (
  source_kind IN ('LEGACY','SCHEDULED_RECOMMENDATION','MANUAL_RECOMMENDATION')
);
ALTER TABLE public.pending_targets ADD CONSTRAINT pending_targets_recommendation_source_lineage_check CHECK (
  (source_kind='LEGACY' AND recommendation_run_id IS NULL)
  OR (source_kind IN ('SCHEDULED_RECOMMENDATION','MANUAL_RECOMMENDATION') AND recommendation_run_id IS NOT NULL
      AND dataset_version IS NOT NULL AND dataset_version_id IS NOT NULL AND dataset_manifest_sha256 IS NOT NULL)
);
ALTER TABLE public.accounts DROP CONSTRAINT accounts_id_owner_beta_key;
ALTER TABLE public.user_strategy_configs DROP CONSTRAINT user_strategy_configs_id_owner_beta_key;
ALTER TABLE public.owner_beta_daily_admissions DROP CONSTRAINT owner_beta_daily_admissions_hash_owner_beta_key;
