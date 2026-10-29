SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- One closed predicate owns the contract/schema/strategy/config cross-product.
-- v1 deliberately retains its original jsonb equality semantics; the v2
-- decimal/integer text checks keep numeric spellings bound to the adopted
-- canonical snapshots as well as their tagged config hashes.
CREATE FUNCTION public.owner_beta_simulation_version_binding_matches(
    p_schema text,
    p_contract_sha256 text,
    p_strategy_id text,
    p_strategy_version text,
    p_config_json jsonb,
    p_config_sha256 text
) RETURNS boolean
LANGUAGE sql
IMMUTABLE
SET search_path = pg_catalog
AS $version_binding$
    SELECT COALESCE(
        CASE
            WHEN p_schema = 'owner-beta-price-only-simulation-job-v1'
             AND p_contract_sha256 =
                 'sha256:8c5c6bf7f07de09f14f1e5eaa2b6479be4b10e8b98ed5a89a280ac95568e87cd'
            THEN p_strategy_id = 'trend_following'
             AND p_strategy_version = '1.0.0'
             AND p_config_json =
                 '{"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200}'::jsonb
             AND p_config_sha256 =
                 'sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0'
            WHEN p_schema = 'owner-beta-price-only-simulation-job-v2'
             AND p_contract_sha256 =
                 'sha256:20dabf650deaff751d8d34a7fea98773e6843b879ed20e6002475fc70c243ee3'
            THEN CASE p_strategy_id
                WHEN 'buy_and_hold' THEN
                    p_strategy_version = '1.0.0'
                    AND p_config_json =
                        '{"benchmark_instrument":"069500.KRX","rebalance_cadence":"none","target_weight":1.0}'::jsonb
                    AND (p_config_json ->> 'target_weight') IS NOT DISTINCT FROM '1.0'
                    AND p_config_sha256 =
                        'sha256:4de76616ae1d72646bd695d0e5aa7df7d6544c2eef5bef7ca949536c1801fc06'
                WHEN 'trend_following' THEN
                    p_strategy_version = '1.0.0'
                    AND p_config_json =
                        '{"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200}'::jsonb
                    AND (p_config_json ->> 'fast_ma') IS NOT DISTINCT FROM '50'
                    AND (p_config_json ->> 'slow_ma') IS NOT DISTINCT FROM '200'
                    AND p_config_sha256 =
                        'sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0'
                WHEN 'relative_momentum' THEN
                    p_strategy_version = '1.0.0'
                    AND p_config_json = '{"lookback_months":12,"top_n":3}'::jsonb
                    AND (p_config_json ->> 'lookback_months') IS NOT DISTINCT FROM '12'
                    AND (p_config_json ->> 'top_n') IS NOT DISTINCT FROM '3'
                    AND p_config_sha256 =
                        'sha256:edace2510fb491084b142615a709f8ef689a5b207d7bae25ce7a886afceb8d07'
                WHEN 'dual_momentum' THEN
                    p_strategy_version = '1.0.0'
                    AND p_config_json =
                        '{"absolute_threshold":0.0,"lookback_months":12}'::jsonb
                    AND (p_config_json ->> 'absolute_threshold') IS NOT DISTINCT FROM '0.0'
                    AND (p_config_json ->> 'lookback_months') IS NOT DISTINCT FROM '12'
                    AND p_config_sha256 =
                        'sha256:ac6a1ac393e02952fe2db6308e5274436f766479f3e631ebe42e429a677426a6'
                WHEN 'inverse_volatility' THEN
                    p_strategy_version = '1.0.0'
                    AND p_config_json = '{"max_weight":0.3,"vol_window":60}'::jsonb
                    AND (p_config_json ->> 'max_weight') IS NOT DISTINCT FROM '0.3'
                    AND (p_config_json ->> 'vol_window') IS NOT DISTINCT FROM '60'
                    AND p_config_sha256 =
                        'sha256:6252f27903f5ea0a7755ea7fc6c05a935518a2dbe44edf371a1e71f127d1aa38'
                ELSE false
            END
            ELSE false
        END,
        false
    )
$version_binding$;
ALTER FUNCTION public.owner_beta_simulation_version_binding_matches(
    text, text, text, text, jsonb, text
) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_simulation_version_binding_matches(
    text, text, text, text, jsonb, text
) FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.owner_beta_simulation_version_binding_matches(
    text, text, text, text, jsonb, text
) TO app, worker;

-- The key namespace is selected by the already-closed schema/contract pair.
-- The strategy/config predicate is checked alongside it by the payload/run
-- binding paths; strategy alone never chooses a namespace.
CREATE FUNCTION public.owner_beta_simulation_version_namespace_matches(
    p_schema text,
    p_contract_sha256 text,
    p_idempotency_key text
) RETURNS boolean
LANGUAGE sql
IMMUTABLE
SET search_path = pg_catalog
AS $version_namespace$
    SELECT CASE
        WHEN p_schema = 'owner-beta-price-only-simulation-job-v1'
         AND p_contract_sha256 =
             'sha256:8c5c6bf7f07de09f14f1e5eaa2b6479be4b10e8b98ed5a89a280ac95568e87cd'
        THEN p_idempotency_key IS NOT NULL
         AND pg_catalog.left(p_idempotency_key, 36) =
             'owner-beta:price-only-simulation:v1:'
        WHEN p_schema = 'owner-beta-price-only-simulation-job-v2'
         AND p_contract_sha256 =
             'sha256:20dabf650deaff751d8d34a7fea98773e6843b879ed20e6002475fc70c243ee3'
        THEN p_idempotency_key IS NOT NULL
         AND pg_catalog.left(p_idempotency_key, 36) =
             'owner-beta:price-only-simulation:v2:'
        ELSE false
    END
$version_namespace$;
ALTER FUNCTION public.owner_beta_simulation_version_namespace_matches(text, text, text)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_simulation_version_namespace_matches(text, text, text)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

ALTER TABLE public.owner_beta_simulation_runs
    DROP CONSTRAINT owner_beta_simulation_runs_strategy_id_check,
    DROP CONSTRAINT owner_beta_simulation_runs_strategy_version_check,
    DROP CONSTRAINT owner_beta_simulation_runs_strategy_config_check,
    DROP CONSTRAINT owner_beta_simulation_runs_strategy_config_hash_check,
    DROP CONSTRAINT owner_beta_simulation_runs_contract_hash_check;
ALTER TABLE public.owner_beta_simulation_runs
    ADD CONSTRAINT owner_beta_simulation_runs_version_binding_check
    CHECK (public.owner_beta_simulation_version_binding_matches(
        CASE contract_document_sha256
            WHEN 'sha256:8c5c6bf7f07de09f14f1e5eaa2b6479be4b10e8b98ed5a89a280ac95568e87cd'
                THEN 'owner-beta-price-only-simulation-job-v1'
            WHEN 'sha256:20dabf650deaff751d8d34a7fea98773e6843b879ed20e6002475fc70c243ee3'
                THEN 'owner-beta-price-only-simulation-job-v2'
            ELSE NULL
        END,
        contract_document_sha256,
        strategy_id,
        strategy_version,
        strategy_config_json,
        strategy_config_sha256
    ));

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
    SELECT COALESCE(
        public.owner_beta_simulation_version_binding_matches(
            p_payload ->> 'schema',
            p_contract_sha256,
            p_strategy_id,
            p_strategy_version,
            p_strategy_config_json,
            p_strategy_config_sha256
        )
        AND p_payload = pg_catalog.jsonb_build_object(
            'schema', p_payload ->> 'schema',
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
        ),
        false
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
       OR NOT public.owner_beta_simulation_version_namespace_matches(
            NEW.payload_json ->> 'schema',
            NEW.payload_json ->> 'contract_document_sha256',
            NEW.idempotency_key
          )
       OR pg_catalog.jsonb_typeof(NEW.payload_json) IS DISTINCT FROM 'object'
       OR v_top_level_keys IS DISTINCT FROM ARRAY[
            'bars_sha256', 'contract_document_sha256', 'evaluation_end',
            'evaluation_start', 'pins', 'run_id', 'schema', 'strategy',
            'strategy_config_id'
          ]::text[]
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
       OR NEW.payload_json ->> 'bars_sha256'
            IS DISTINCT FROM 'sha256:23354c54708d7a458aadc4f2cb2fca77469969e91256b6d3b119101ebae98ffe'
       OR NOT public.owner_beta_simulation_version_binding_matches(
            NEW.payload_json ->> 'schema',
            NEW.payload_json ->> 'contract_document_sha256',
            NEW.payload_json -> 'strategy' ->> 'strategy_id',
            NEW.payload_json -> 'strategy' ->> 'strategy_version',
            NEW.payload_json -> 'strategy' -> 'config_json',
            NEW.payload_json -> 'strategy' ->> 'config_sha256'
          )
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
       OR NOT public.owner_beta_simulation_version_namespace_matches(
            v_job_payload ->> 'schema',
            v_job_payload ->> 'contract_document_sha256',
            v_key
       )
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
