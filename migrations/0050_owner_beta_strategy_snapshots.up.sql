SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

LOCK TABLE public.owner_beta_recommendation_runs IN ACCESS EXCLUSIVE MODE;

-- A pre-0050 row has no enqueue-time strategy snapshot. It cannot be
-- backfilled from the later-mutable strategy config without inventing lineage.
-- The access-exclusive lock makes it safe to let the table owner inspect the
-- complete table for this guard. Do not impersonate each tenant with a custom
-- GUC here: PostgreSQL retains an empty custom GUC on the pooled migration
-- connection, which can break older strict RLS policies during rollback.
ALTER TABLE public.owner_beta_recommendation_runs NO FORCE ROW LEVEL SECURITY;
DO $legacy_guard$
BEGIN
    IF EXISTS (
        SELECT 1 FROM public.owner_beta_recommendation_runs
    ) THEN
        RAISE EXCEPTION 'owner beta strategy snapshot migration requires an empty run table'
            USING ERRCODE = '55000';
    END IF;
END
$legacy_guard$;
ALTER TABLE public.owner_beta_recommendation_runs FORCE ROW LEVEL SECURITY;

ALTER TABLE public.owner_beta_recommendation_runs
    ADD COLUMN strategy_id text NOT NULL,
    ADD COLUMN strategy_version text NOT NULL,
    ADD COLUMN strategy_config_json jsonb NOT NULL,
    ADD COLUMN strategy_config_sha256 text NOT NULL,
    ADD CONSTRAINT owner_beta_recommendation_runs_strategy_id_check CHECK (
        strategy_id <> ''
    ),
    ADD CONSTRAINT owner_beta_recommendation_runs_strategy_version_check CHECK (
        strategy_version <> ''
    ),
    ADD CONSTRAINT owner_beta_recommendation_runs_strategy_config_check CHECK (
        pg_catalog.jsonb_typeof(strategy_config_json) = 'object'
    ),
    ADD CONSTRAINT owner_beta_recommendation_runs_strategy_config_hash_check CHECK (
        strategy_config_sha256 ~ '^sha256:[0-9a-f]{64}$'
    );

GRANT INSERT (
    strategy_id, strategy_version, strategy_config_json, strategy_config_sha256
) ON public.owner_beta_recommendation_runs TO app;

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
