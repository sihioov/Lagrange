SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Hold the queue/run/pair/attempt write surface stable while checking every
-- v2 marker. Migration-owner ownership permits a complete scan only while
-- FORCE RLS is temporarily off; the guard runs before function, constraint,
-- or ACL restoration, and an exception rolls this migration transaction back.
LOCK TABLE public.jobs IN SHARE MODE;
LOCK TABLE public.owner_beta_simulation_runs IN ACCESS EXCLUSIVE MODE;
LOCK TABLE public.owner_beta_simulation_result_pairs IN ACCESS EXCLUSIVE MODE;
LOCK TABLE public.job_attempts IN SHARE MODE;

ALTER TABLE public.jobs NO FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_beta_simulation_runs NO FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_beta_simulation_result_pairs NO FORCE ROW LEVEL SECURITY;

DO $owner_beta_simulation_v2_history_guard$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM public.owner_beta_simulation_runs AS run
         WHERE run.contract_document_sha256 =
               'sha256:20dabf650deaff751d8d34a7fea98773e6843b879ed20e6002475fc70c243ee3'
    ) OR EXISTS (
        SELECT 1
          FROM public.jobs AS job
         WHERE job.job_type = 'owner_beta_price_only_simulation'
           AND (
                job.payload_json ->> 'schema' =
                    'owner-beta-price-only-simulation-job-v2'
                OR job.payload_json ->> 'contract_document_sha256' =
                    'sha256:20dabf650deaff751d8d34a7fea98773e6843b879ed20e6002475fc70c243ee3'
                OR job.idempotency_key LIKE 'owner-beta:price-only-simulation:v2:%'
           )
    ) OR EXISTS (
        SELECT 1
          FROM public.job_attempts AS attempt
          JOIN public.jobs AS job ON job.id = attempt.job_id
         WHERE job.job_type = 'owner_beta_price_only_simulation'
           AND (
                job.payload_json ->> 'schema' =
                    'owner-beta-price-only-simulation-job-v2'
                OR job.payload_json ->> 'contract_document_sha256' =
                    'sha256:20dabf650deaff751d8d34a7fea98773e6843b879ed20e6002475fc70c243ee3'
                OR job.idempotency_key LIKE 'owner-beta:price-only-simulation:v2:%'
           )
    ) OR EXISTS (
        SELECT 1
          FROM public.owner_beta_simulation_result_pairs AS pair
          JOIN public.owner_beta_simulation_runs AS run ON run.id = pair.run_id
         WHERE run.contract_document_sha256 =
               'sha256:20dabf650deaff751d8d34a7fea98773e6843b879ed20e6002475fc70c243ee3'
    ) THEN
        RAISE EXCEPTION 'owner beta simulation v2 history prevents migration downgrade'
            USING ERRCODE = '55000';
    END IF;
END
$owner_beta_simulation_v2_history_guard$;

-- No v2 history remains. Restore the exact 0063 payload function and its ACL.
CREATE OR REPLACE FUNCTION public.owner_beta_simulation_payload_matches(
    p_payload jsonb,
    p_run_id uuid,
    p_config_id uuid,
    p_evaluation_start date,
    p_evaluation_end date,
    p_candidate_sha256 text,
    p_artifact_sha256 text,
    p_stage5_sha256 text,
    p_action_sha256 text,
    p_approval_sha256 text,
    p_contract_sha256 text,
    p_bars_sha256 text,
    p_strategy_id text,
    p_strategy_version text,
    p_strategy_config_json jsonb,
    p_strategy_config_sha256 text
) RETURNS boolean
LANGUAGE sql
IMMUTABLE
SET search_path = pg_catalog
AS $payload$
    SELECT p_payload = pg_catalog.jsonb_build_object(
        'schema', 'owner-beta-price-only-simulation-job-v1',
        'run_id', p_run_id::text,
        'strategy_config_id', p_config_id::text,
        'evaluation_start', pg_catalog.to_char(p_evaluation_start, 'YYYY-MM-DD'),
        'evaluation_end', pg_catalog.to_char(p_evaluation_end, 'YYYY-MM-DD'),
        'pins', pg_catalog.jsonb_build_object(
            'candidate_content_sha256', p_candidate_sha256,
            'artifact_manifest_sha256', p_artifact_sha256,
            'stage5_manifest_sha256', p_stage5_sha256,
            'action_manifest_sha256', p_action_sha256,
            'approval_registry_sha256', p_approval_sha256
        ),
        'contract_document_sha256', p_contract_sha256,
        'bars_sha256', p_bars_sha256,
        'strategy', pg_catalog.jsonb_build_object(
            'strategy_id', p_strategy_id,
            'strategy_version', p_strategy_version,
            'config_json', p_strategy_config_json,
            'config_sha256', p_strategy_config_sha256
        )
    )
$payload$;
ALTER FUNCTION public.owner_beta_simulation_payload_matches(
    jsonb, uuid, uuid, date, date, text, text, text, text, text, text, text,
    text, text, jsonb, text
) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_simulation_payload_matches(
    jsonb, uuid, uuid, date, date, text, text, text, text, text, text, text,
    text, text, jsonb, text
) FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

-- Restore the exact 0063 insert guard, including the strict key/type checks.
CREATE OR REPLACE FUNCTION public.owner_beta_simulation_jobs_insert_guard()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $jobs_insert$
DECLARE
    v_top_level_keys text[];
    v_pin_keys text[];
    v_strategy_keys text[];
BEGIN
    IF NEW.job_type <> 'owner_beta_price_only_simulation' THEN
        RETURN NEW;
    END IF;

    SELECT pg_catalog.array_agg(key ORDER BY key)
      INTO v_top_level_keys
      FROM pg_catalog.jsonb_object_keys(NEW.payload_json) AS keys(key);
    SELECT pg_catalog.array_agg(key ORDER BY key)
      INTO v_pin_keys
      FROM pg_catalog.jsonb_object_keys(NEW.payload_json -> 'pins') AS keys(key);
    SELECT pg_catalog.array_agg(key ORDER BY key)
      INTO v_strategy_keys
      FROM pg_catalog.jsonb_object_keys(NEW.payload_json -> 'strategy') AS keys(key);

    IF NEW.status <> 'QUEUED'
       OR NEW.attempt_count <> 0
       OR NEW.max_attempts <> 3
       OR NEW.idempotency_key IS NULL
       OR NEW.idempotency_key NOT LIKE 'owner-beta:price-only-simulation:v1:%'
       OR pg_catalog.jsonb_typeof(NEW.payload_json) IS DISTINCT FROM 'object'
       OR v_top_level_keys IS DISTINCT FROM ARRAY[
            'bars_sha256', 'contract_document_sha256', 'evaluation_end',
            'evaluation_start', 'pins', 'run_id', 'schema', 'strategy',
            'strategy_config_id'
          ]::text[]
       OR NEW.payload_json ->> 'schema' IS DISTINCT FROM 'owner-beta-price-only-simulation-job-v1'
       OR pg_catalog.jsonb_typeof(NEW.payload_json -> 'run_id') IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(NEW.payload_json -> 'strategy_config_id') IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(NEW.payload_json -> 'evaluation_start') IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(NEW.payload_json -> 'evaluation_end') IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(NEW.payload_json -> 'pins') IS DISTINCT FROM 'object'
       OR pg_catalog.jsonb_typeof(NEW.payload_json -> 'strategy') IS DISTINCT FROM 'object'
       OR v_pin_keys IS DISTINCT FROM ARRAY[
            'action_manifest_sha256', 'approval_registry_sha256',
            'artifact_manifest_sha256', 'candidate_content_sha256',
            'stage5_manifest_sha256'
          ]::text[]
       OR v_strategy_keys IS DISTINCT FROM ARRAY[
            'config_json', 'config_sha256', 'strategy_id', 'strategy_version'
          ]::text[]
       OR NEW.payload_json -> 'pins' ->> 'candidate_content_sha256'
            IS DISTINCT FROM 'sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d'
       OR NEW.payload_json -> 'pins' ->> 'artifact_manifest_sha256'
            IS DISTINCT FROM 'sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6'
       OR NEW.payload_json -> 'pins' ->> 'stage5_manifest_sha256'
            IS DISTINCT FROM 'sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a'
       OR NEW.payload_json -> 'pins' ->> 'action_manifest_sha256'
            IS DISTINCT FROM 'sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550'
       OR NEW.payload_json -> 'pins' ->> 'approval_registry_sha256'
            IS DISTINCT FROM 'sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707'
       OR NEW.payload_json ->> 'contract_document_sha256'
            IS DISTINCT FROM 'sha256:8c5c6bf7f07de09f14f1e5eaa2b6479be4b10e8b98ed5a89a280ac95568e87cd'
       OR NEW.payload_json ->> 'bars_sha256'
            IS DISTINCT FROM 'sha256:23354c54708d7a458aadc4f2cb2fca77469969e91256b6d3b119101ebae98ffe'
       OR NEW.payload_json -> 'strategy' ->> 'strategy_id' IS DISTINCT FROM 'trend_following'
       OR NEW.payload_json -> 'strategy' ->> 'strategy_version' IS DISTINCT FROM '1.0.0'
       OR NEW.payload_json -> 'strategy' -> 'config_json'
            IS DISTINCT FROM '{"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200}'::jsonb
       OR NEW.payload_json -> 'strategy' ->> 'config_sha256'
            IS DISTINCT FROM 'sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0'
    THEN
        RAISE EXCEPTION 'owner beta simulation job binding is invalid'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END
$jobs_insert$;
ALTER FUNCTION public.owner_beta_simulation_jobs_insert_guard() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_simulation_jobs_insert_guard()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

-- Restore the exact 0063 owner/actor, queue-first lock, and active-config guard.
CREATE OR REPLACE FUNCTION public.owner_beta_simulation_runs_validate_binding()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $run_binding$
DECLARE
    v_actor_before text := pg_catalog.current_setting('app.actor_user_id', true);
    v_actor_changed boolean := false;
    v_job_owner uuid;
    v_job_type text;
    v_job_payload jsonb;
    v_key text;
    v_config_owner uuid;
    v_config_strategy text;
    v_config_version text;
    v_config_json jsonb;
    v_config_active boolean;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'owner beta simulation run history is immutable'
            USING ERRCODE = '42501';
    END IF;

    PERFORM NULLIF(v_actor_before, '')::uuid;
    IF session_user <> 'app'
       AND v_actor_before IS DISTINCT FROM NEW.owner_user_id::text THEN
        PERFORM pg_catalog.set_config('app.actor_user_id', NEW.owner_user_id::text, true);
        v_actor_changed := true;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.id IS DISTINCT FROM OLD.id
           OR NEW.owner_user_id IS DISTINCT FROM OLD.owner_user_id
           OR NEW.strategy_config_id IS DISTINCT FROM OLD.strategy_config_id
           OR NEW.job_id IS DISTINCT FROM OLD.job_id
           OR NEW.strategy_id IS DISTINCT FROM OLD.strategy_id
           OR NEW.strategy_version IS DISTINCT FROM OLD.strategy_version
           OR NEW.strategy_config_json IS DISTINCT FROM OLD.strategy_config_json
           OR NEW.strategy_config_sha256 IS DISTINCT FROM OLD.strategy_config_sha256
           OR NEW.evaluation_start IS DISTINCT FROM OLD.evaluation_start
           OR NEW.evaluation_end IS DISTINCT FROM OLD.evaluation_end
           OR NEW.session_count IS DISTINCT FROM OLD.session_count
           OR NEW.input_kind IS DISTINCT FROM OLD.input_kind
           OR NEW.capability IS DISTINCT FROM OLD.capability
           OR NEW.audience IS DISTINCT FROM OLD.audience
           OR NEW.vendor_snapshot IS DISTINCT FROM OLD.vendor_snapshot
           OR NEW.strict_pit IS DISTINCT FROM OLD.strict_pit
           OR NEW.action_count IS DISTINCT FROM OLD.action_count
           OR NEW.candidate_content_sha256 IS DISTINCT FROM OLD.candidate_content_sha256
           OR NEW.artifact_manifest_sha256 IS DISTINCT FROM OLD.artifact_manifest_sha256
           OR NEW.stage5_manifest_sha256 IS DISTINCT FROM OLD.stage5_manifest_sha256
           OR NEW.action_manifest_sha256 IS DISTINCT FROM OLD.action_manifest_sha256
           OR NEW.approval_registry_sha256 IS DISTINCT FROM OLD.approval_registry_sha256
           OR NEW.contract_document_sha256 IS DISTINCT FROM OLD.contract_document_sha256
           OR NEW.bars_sha256 IS DISTINCT FROM OLD.bars_sha256
        THEN
            RAISE EXCEPTION 'owner beta simulation run identity is immutable'
                USING ERRCODE = '42501';
        END IF;
        IF OLD.status <> 'PENDING'
           OR (NEW.status = 'PENDING' AND (
                NEW.error_code IS NOT NULL OR NEW.engine_json IS NOT NULL
                OR NEW.published_attempt_id IS NOT NULL OR NEW.finished_at IS NOT NULL
           ))
        THEN
            RAISE EXCEPTION 'owner beta simulation run terminal state is immutable'
                USING ERRCODE = '42501';
        END IF;
    ELSE
        IF NEW.status <> 'PENDING'
           OR NEW.error_code IS NOT NULL
           OR NEW.engine_json IS NOT NULL
           OR NEW.published_attempt_id IS NOT NULL
           OR NEW.finished_at IS NOT NULL
        THEN
            RAISE EXCEPTION 'owner beta simulation run must begin pending'
                USING ERRCODE = '42501';
        END IF;
    END IF;

    -- Preserve the global lock order: queue job first, then the run row. The
    -- run row itself is not locked yet during BEFORE INSERT.
    SELECT job.owner_user_id, job.job_type, job.payload_json, job.idempotency_key
      INTO v_job_owner, v_job_type, v_job_payload, v_key
      FROM public.jobs AS job
     WHERE job.id = NEW.job_id
     FOR UPDATE OF job;
    IF NOT FOUND
       OR v_job_owner IS DISTINCT FROM NEW.owner_user_id
       OR v_job_type IS DISTINCT FROM 'owner_beta_price_only_simulation'
       OR v_key IS NULL
       OR v_key NOT LIKE 'owner-beta:price-only-simulation:v1:%'
       OR NOT public.owner_beta_simulation_payload_matches(
            v_job_payload, NEW.id, NEW.strategy_config_id,
            NEW.evaluation_start, NEW.evaluation_end,
            NEW.candidate_content_sha256, NEW.artifact_manifest_sha256,
            NEW.stage5_manifest_sha256, NEW.action_manifest_sha256,
            NEW.approval_registry_sha256, NEW.contract_document_sha256,
            NEW.bars_sha256, NEW.strategy_id, NEW.strategy_version,
            NEW.strategy_config_json, NEW.strategy_config_sha256
       )
    THEN
        RAISE EXCEPTION 'owner beta simulation job and run binding is invalid'
            USING ERRCODE = '23514';
    END IF;

    IF TG_OP = 'INSERT' THEN
        SELECT config.owner_user_id, config.strategy_id, config.strategy_version,
               config.config_json, config.is_active
          INTO v_config_owner, v_config_strategy, v_config_version,
               v_config_json, v_config_active
          FROM public.user_strategy_configs AS config
         WHERE config.id = NEW.strategy_config_id;
    ELSE
        SELECT config.owner_user_id
          INTO v_config_owner
          FROM public.user_strategy_configs AS config
         WHERE config.id = NEW.strategy_config_id;
    END IF;
    IF NOT FOUND OR v_config_owner IS DISTINCT FROM NEW.owner_user_id
    THEN
        RAISE EXCEPTION 'owner beta simulation strategy config identity is unavailable'
            USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'INSERT'
       AND (v_config_strategy IS DISTINCT FROM NEW.strategy_id
            OR v_config_version IS DISTINCT FROM NEW.strategy_version
            OR v_config_json IS DISTINCT FROM NEW.strategy_config_json
            OR NOT v_config_active)
    THEN
        RAISE EXCEPTION 'owner beta simulation strategy snapshot is not active at admission'
            USING ERRCODE = '23514';
    END IF;
    IF v_actor_changed THEN
        PERFORM pg_catalog.set_config(
            'app.actor_user_id', COALESCE(v_actor_before, ''), true
        );
    END IF;
    RETURN NEW;
END
$run_binding$;
ALTER FUNCTION public.owner_beta_simulation_runs_validate_binding() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_simulation_runs_validate_binding()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

ALTER TABLE public.owner_beta_simulation_runs
    DROP CONSTRAINT owner_beta_simulation_runs_version_binding_check;
ALTER TABLE public.owner_beta_simulation_runs
    ADD CONSTRAINT owner_beta_simulation_runs_strategy_id_check
        CHECK (strategy_id = 'trend_following'),
    ADD CONSTRAINT owner_beta_simulation_runs_strategy_version_check
        CHECK (strategy_version = '1.0.0'),
    ADD CONSTRAINT owner_beta_simulation_runs_strategy_config_check
        CHECK (strategy_config_json = '{"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200}'::jsonb),
    ADD CONSTRAINT owner_beta_simulation_runs_strategy_config_hash_check
        CHECK (strategy_config_sha256 = 'sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0'),
    ADD CONSTRAINT owner_beta_simulation_runs_contract_hash_check
        CHECK (contract_document_sha256 = 'sha256:8c5c6bf7f07de09f14f1e5eaa2b6479be4b10e8b98ed5a89a280ac95568e87cd');

ALTER TABLE public.jobs FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_beta_simulation_runs FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_beta_simulation_result_pairs FORCE ROW LEVEL SECURITY;

-- The original callers and check constraint are restored before either new
-- pure predicate is removed.
DROP FUNCTION public.owner_beta_simulation_version_namespace_matches(text, text, text);
DROP FUNCTION public.owner_beta_simulation_version_binding_matches(
    text, text, text, text, jsonb, text
);
