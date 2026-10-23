-- Unused-only rollback. This file is executed by sqlx in one transaction.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
LOCK TABLE public.user_strategy_configs,public.owner_beta_recommendation_runs,
    public.owner_beta_recommendation_items,public.jobs,public.job_attempts,
    public.owner_beta_daily_control,public.owner_beta_daily_admissions,
    public.owner_beta_daily_admission_events,public.owner_beta_daily_reservations,
    public.owner_beta_daily_poll_state,public.owner_beta_daily_calendar_sources,
    public.owner_beta_daily_calendar_days,public.owner_beta_daily_source_events,
    public.owner_beta_daily_publication_claims,public.owner_beta_daily_poll_receipts
    IN ACCESS EXCLUSIVE MODE;
ALTER TABLE public.user_strategy_configs NO FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_beta_recommendation_runs NO FORCE ROW LEVEL SECURITY;
-- New tables have an explicit migration-owner policy, independent of actor.
DO $unused$
BEGIN
    IF EXISTS(SELECT 1 FROM public.owner_beta_recommendation_runs WHERE trigger_kind='DAILY')
       OR EXISTS(SELECT 1 FROM public.user_strategy_configs WHERE daily_enabled OR daily_epoch<>0)
       OR EXISTS(SELECT 1 FROM public.owner_beta_daily_control WHERE enabled OR activation_epoch<>0)
       OR EXISTS(SELECT 1 FROM public.owner_beta_daily_admissions)
       OR EXISTS(SELECT 1 FROM public.owner_beta_daily_admission_events)
       OR EXISTS(SELECT 1 FROM public.owner_beta_daily_reservations)
       OR EXISTS(SELECT 1 FROM public.owner_beta_daily_calendar_sources)
       OR EXISTS(SELECT 1 FROM public.owner_beta_daily_calendar_days)
       OR EXISTS(SELECT 1 FROM public.owner_beta_daily_source_events)
       OR EXISTS(SELECT 1 FROM public.owner_beta_daily_publication_claims)
       OR EXISTS(SELECT 1 FROM public.owner_beta_daily_poll_receipts)
       OR EXISTS(SELECT 1 FROM public.owner_beta_daily_poll_state WHERE latest_receipt_id IS NOT NULL OR last_completed_at IS NOT NULL OR last_result_code IS NOT NULL OR verified_admission_sha256 IS NOT NULL OR source_frontier_sha256 IS NOT NULL)
    THEN RAISE EXCEPTION 'daily rollback requires unused schema' USING ERRCODE='55000'; END IF;
END $unused$;
ALTER TABLE public.user_strategy_configs FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_beta_recommendation_runs FORCE ROW LEVEL SECURITY;

DROP TRIGGER daily_0060_consistency ON public.jobs;
DROP TRIGGER daily_0060_consistency ON public.job_attempts;
DROP TRIGGER daily_0060_consistency ON public.owner_beta_recommendation_runs;
DROP TRIGGER daily_0060_consistency ON public.owner_beta_recommendation_items;
DROP TRIGGER daily_0060_consistency ON public.owner_beta_daily_reservations;
DROP TRIGGER daily_0060_consistency ON public.owner_beta_daily_publication_claims;
DROP TRIGGER daily_0060_job_guard ON public.jobs;
DROP TRIGGER daily_0060_attempt_guard ON public.job_attempts;
DROP TRIGGER daily_0060_run_guard ON public.owner_beta_recommendation_runs;
DROP TRIGGER daily_0060_item_guard ON public.owner_beta_recommendation_items;
DROP TRIGGER daily_0060_config_epoch ON public.user_strategy_configs;
DROP TRIGGER daily_0060_control_epoch ON public.owner_beta_daily_control;
DO $drop_evidence_triggers$ DECLARE t text; BEGIN
    FOREACH t IN ARRAY ARRAY['admissions','admission_events','reservations','calendar_sources','calendar_days','source_events','publication_claims'] LOOP
        EXECUTE format('DROP TRIGGER daily_0060_immutable ON public.owner_beta_daily_%I',t);
    END LOOP;
END $drop_evidence_triggers$;

-- Exact 0050/0052 historical definitions and trigger column lists.
CREATE OR REPLACE FUNCTION public.owner_beta_recommendation_runs_validate_job_binding()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $binding$
DECLARE
    v_config_strategy_id text;
    v_config_strategy_version text;
    v_config_json jsonb;
    v_job_owner_user_id uuid;
    v_job_type text;
    v_payload jsonb;
    v_top_level_keys text[];
    v_pin_keys text[];
    v_strategy_keys text[];
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.id IS DISTINCT FROM OLD.id
            OR NEW.owner_user_id IS DISTINCT FROM OLD.owner_user_id
            OR NEW.strategy_config_id IS DISTINCT FROM OLD.strategy_config_id
            OR NEW.strategy_id IS DISTINCT FROM OLD.strategy_id
            OR NEW.strategy_version IS DISTINCT FROM OLD.strategy_version
            OR NEW.strategy_config_json IS DISTINCT FROM OLD.strategy_config_json
            OR NEW.strategy_config_sha256 IS DISTINCT FROM OLD.strategy_config_sha256
            OR NEW.job_id IS DISTINCT FROM OLD.job_id
            OR NEW.as_of IS DISTINCT FROM OLD.as_of
            OR NEW.input_kind IS DISTINCT FROM OLD.input_kind
            OR NEW.capability IS DISTINCT FROM OLD.capability
            OR NEW.audience IS DISTINCT FROM OLD.audience
            OR NEW.vendor_snapshot IS DISTINCT FROM OLD.vendor_snapshot
            OR NEW.strict_pit IS DISTINCT FROM OLD.strict_pit
            OR NEW.candidate_content_sha256 IS DISTINCT FROM OLD.candidate_content_sha256
            OR NEW.artifact_manifest_sha256 IS DISTINCT FROM OLD.artifact_manifest_sha256
            OR NEW.stage5_manifest_sha256 IS DISTINCT FROM OLD.stage5_manifest_sha256
            OR NEW.action_manifest_sha256 IS DISTINCT FROM OLD.action_manifest_sha256
            OR NEW.approval_registry_sha256 IS DISTINCT FROM OLD.approval_registry_sha256
        THEN
            RAISE EXCEPTION 'owner beta recommendation run identity is immutable'
                USING ERRCODE = '42501';
        END IF;
    ELSE
        SELECT config.strategy_id, config.strategy_version, config.config_json
          INTO v_config_strategy_id, v_config_strategy_version, v_config_json
          FROM public.user_strategy_configs AS config
         WHERE config.id = NEW.strategy_config_id
           AND config.owner_user_id = NEW.owner_user_id
           AND config.is_active
         FOR SHARE OF config;
        IF NOT FOUND
           OR v_config_strategy_id IS DISTINCT FROM NEW.strategy_id
           OR v_config_strategy_version IS DISTINCT FROM NEW.strategy_version
           OR v_config_json IS DISTINCT FROM NEW.strategy_config_json
        THEN
            RAISE EXCEPTION 'owner beta recommendation binding is invalid'
                USING ERRCODE = '23514';
        END IF;
    END IF;

    SELECT job.owner_user_id, job.job_type, job.payload_json
      INTO v_job_owner_user_id, v_job_type, v_payload
      FROM public.jobs AS job
     WHERE job.id = NEW.job_id
     FOR SHARE OF job;
    IF NOT FOUND
       OR v_job_owner_user_id IS DISTINCT FROM NEW.owner_user_id
       OR v_job_type IS DISTINCT FROM 'owner_beta_price_recommendation'
       OR pg_catalog.jsonb_typeof(v_payload) IS DISTINCT FROM 'object'
    THEN
        RAISE EXCEPTION 'owner beta recommendation binding is invalid'
            USING ERRCODE = '23514';
    END IF;

    SELECT pg_catalog.array_agg(payload_key ORDER BY payload_key)
      INTO v_top_level_keys
      FROM pg_catalog.jsonb_object_keys(v_payload) AS payload_keys(payload_key);
    IF v_top_level_keys IS DISTINCT FROM ARRAY[
        'as_of', 'pins', 'run_id', 'strategy', 'strategy_config_id'
    ]::text[]
       OR pg_catalog.jsonb_typeof(v_payload -> 'run_id') IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(v_payload -> 'strategy_config_id') IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(v_payload -> 'as_of') IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(v_payload -> 'pins') IS DISTINCT FROM 'object'
       OR pg_catalog.jsonb_typeof(v_payload -> 'strategy') IS DISTINCT FROM 'object'
    THEN
        RAISE EXCEPTION 'owner beta recommendation binding is invalid'
            USING ERRCODE = '23514';
    END IF;

    SELECT pg_catalog.array_agg(pin_key ORDER BY pin_key)
      INTO v_pin_keys
      FROM pg_catalog.jsonb_object_keys(v_payload -> 'pins') AS pin_keys(pin_key);
    IF v_pin_keys IS DISTINCT FROM ARRAY[
        'action_manifest_sha256',
        'approval_registry_sha256',
        'artifact_manifest_sha256',
        'candidate_content_sha256',
        'stage5_manifest_sha256'
    ]::text[]
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'pins' -> 'candidate_content_sha256'
          ) IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'pins' -> 'artifact_manifest_sha256'
          ) IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'pins' -> 'stage5_manifest_sha256'
          ) IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'pins' -> 'action_manifest_sha256'
          ) IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'pins' -> 'approval_registry_sha256'
          ) IS DISTINCT FROM 'string'
       OR v_payload ->> 'run_id' IS DISTINCT FROM NEW.id::text
       OR v_payload ->> 'strategy_config_id' IS DISTINCT FROM NEW.strategy_config_id::text
       OR v_payload ->> 'as_of' IS DISTINCT FROM pg_catalog.to_char(NEW.as_of, 'YYYY-MM-DD')
       OR v_payload -> 'pins' ->> 'candidate_content_sha256'
            IS DISTINCT FROM NEW.candidate_content_sha256
       OR v_payload -> 'pins' ->> 'artifact_manifest_sha256'
            IS DISTINCT FROM NEW.artifact_manifest_sha256
       OR v_payload -> 'pins' ->> 'stage5_manifest_sha256'
            IS DISTINCT FROM NEW.stage5_manifest_sha256
       OR v_payload -> 'pins' ->> 'action_manifest_sha256'
            IS DISTINCT FROM NEW.action_manifest_sha256
       OR v_payload -> 'pins' ->> 'approval_registry_sha256'
            IS DISTINCT FROM NEW.approval_registry_sha256
    THEN
        RAISE EXCEPTION 'owner beta recommendation binding is invalid'
            USING ERRCODE = '23514';
    END IF;

    SELECT pg_catalog.array_agg(strategy_key ORDER BY strategy_key)
      INTO v_strategy_keys
      FROM pg_catalog.jsonb_object_keys(v_payload -> 'strategy') AS strategy_keys(strategy_key);
    IF v_strategy_keys IS DISTINCT FROM ARRAY[
        'config_json', 'config_sha256', 'strategy_id', 'strategy_version'
    ]::text[]
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'strategy' -> 'strategy_id'
          ) IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'strategy' -> 'strategy_version'
          ) IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'strategy' -> 'config_json'
          ) IS DISTINCT FROM 'object'
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'strategy' -> 'config_sha256'
          ) IS DISTINCT FROM 'string'
       OR v_payload -> 'strategy' ->> 'strategy_id'
            IS DISTINCT FROM NEW.strategy_id
       OR v_payload -> 'strategy' ->> 'strategy_version'
            IS DISTINCT FROM NEW.strategy_version
       OR v_payload -> 'strategy' -> 'config_json'
            IS DISTINCT FROM NEW.strategy_config_json
       OR v_payload -> 'strategy' ->> 'config_sha256'
            IS DISTINCT FROM NEW.strategy_config_sha256
    THEN
        RAISE EXCEPTION 'owner beta recommendation binding is invalid'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END
$binding$;

ALTER FUNCTION public.owner_beta_recommendation_runs_validate_job_binding()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_recommendation_runs_validate_job_binding()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

DROP TRIGGER owner_beta_recommendation_runs_validate_job_binding
    ON public.owner_beta_recommendation_runs;
CREATE TRIGGER owner_beta_recommendation_runs_validate_job_binding
    BEFORE INSERT OR UPDATE OF
        id, owner_user_id, strategy_config_id, strategy_id, strategy_version,
        strategy_config_json, strategy_config_sha256, job_id, as_of,
        input_kind, capability, audience, vendor_snapshot, strict_pit,
        candidate_content_sha256, artifact_manifest_sha256,
        stage5_manifest_sha256, action_manifest_sha256, approval_registry_sha256
    ON public.owner_beta_recommendation_runs
    FOR EACH ROW
    EXECUTE FUNCTION public.owner_beta_recommendation_runs_validate_job_binding();

CREATE OR REPLACE FUNCTION public.owner_beta_recommendation_runs_lock_strategy_config_on_success()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $lock$
BEGIN
    IF NEW.status IS DISTINCT FROM 'SUCCEEDED'
       OR OLD.status IS NOT DISTINCT FROM 'SUCCEEDED'
    THEN
        RETURN NEW;
    END IF;

    PERFORM pg_catalog.set_config('app.actor_user_id', NEW.owner_user_id::text, true);
    PERFORM 1
      FROM public.user_strategy_configs AS config
     WHERE config.id = NEW.strategy_config_id
       AND config.owner_user_id = NEW.owner_user_id
       AND config.is_active
       AND config.strategy_id = NEW.strategy_id
       AND config.strategy_version = NEW.strategy_version
       AND config.config_json = NEW.strategy_config_json
     FOR SHARE OF config;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'owner beta recommendation strategy config is unavailable'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END
$lock$;

ALTER FUNCTION public.owner_beta_recommendation_runs_lock_strategy_config_on_success()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_recommendation_runs_lock_strategy_config_on_success()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;


DO $functions$ DECLARE f record; BEGIN
    FOR f IN SELECT p.oid::regprocedure AS signature FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
      WHERE n.nspname='public' AND (p.proname LIKE 'daily\_0060\_%' ESCAPE '\'
        OR p.proname=ANY(ARRAY['take_owner_beta_daily_admission_poll_slot','take_owner_beta_daily_schedule_poll_slot','project_owner_beta_daily_calendar','admit_owner_beta_daily_input','block_owner_beta_daily_input','finish_owner_beta_daily_admission_poll','finish_owner_beta_daily_schedule_poll','read_owner_beta_daily_source_evidence','owner_beta_daily_schedule_candidates','reserve_owner_beta_daily_run','lock_owner_beta_daily_run_gate','revoke_owner_beta_daily_input'])) LOOP
        EXECUTE format('DROP FUNCTION %s',f.signature);
    END LOOP;
END $functions$;
DROP TABLE public.owner_beta_daily_publication_claims;
DROP TABLE public.owner_beta_daily_reservations;
DROP TABLE public.owner_beta_daily_poll_state;
DROP TABLE public.owner_beta_daily_poll_receipts;
DROP TABLE public.owner_beta_daily_source_events;
DROP TABLE public.owner_beta_daily_calendar_days;
DROP TABLE public.owner_beta_daily_calendar_sources;
DROP TABLE public.owner_beta_daily_admission_events;
DROP TABLE public.owner_beta_daily_admissions;
DROP TABLE public.owner_beta_daily_control;

ALTER TABLE public.owner_beta_recommendation_runs
    DROP CONSTRAINT owner_beta_recommendation_runs_input_kind_check,
    DROP COLUMN trigger_kind,DROP COLUMN daily_pins,DROP COLUMN daily_config_epoch,
    DROP COLUMN daily_activation_epoch,DROP COLUMN daily_trigger_version,
    ALTER COLUMN candidate_content_sha256 SET NOT NULL,
    ALTER COLUMN artifact_manifest_sha256 SET NOT NULL,
    ALTER COLUMN stage5_manifest_sha256 SET NOT NULL,
    ALTER COLUMN action_manifest_sha256 SET NOT NULL,
    ALTER COLUMN approval_registry_sha256 SET NOT NULL,
    ADD CONSTRAINT owner_beta_recommendation_runs_input_kind_check CHECK (input_kind='owner_beta_historical_price_only_v1');
REVOKE INSERT(owner_user_id,strategy_id,strategy_version,config_json,is_active,daily_enabled),
    UPDATE(config_json,is_active,daily_enabled,updated_at) ON public.user_strategy_configs FROM app;
ALTER TABLE public.user_strategy_configs DROP COLUMN daily_enabled,DROP COLUMN daily_epoch;
GRANT INSERT,UPDATE ON public.user_strategy_configs TO app;
