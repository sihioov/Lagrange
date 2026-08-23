SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

CREATE TABLE public.owner_beta_recommendation_runs (
    id uuid PRIMARY KEY DEFAULT pg_catalog.gen_random_uuid(),
    owner_user_id uuid NOT NULL
        REFERENCES public.users (id) ON DELETE RESTRICT,
    strategy_config_id uuid NOT NULL
        REFERENCES public.user_strategy_configs (id) ON DELETE RESTRICT,
    job_id uuid NOT NULL UNIQUE
        REFERENCES public.jobs (id) ON DELETE RESTRICT,
    as_of date NOT NULL,
    status text NOT NULL DEFAULT 'PENDING',
    input_kind text NOT NULL DEFAULT 'owner_beta_historical_price_only_v1',
    capability text NOT NULL DEFAULT 'PRICE_RETURN_ONLY',
    audience text NOT NULL DEFAULT 'OWNER_ONLY',
    vendor_snapshot boolean NOT NULL DEFAULT true,
    strict_pit boolean NOT NULL DEFAULT false,
    candidate_content_sha256 text NOT NULL,
    artifact_manifest_sha256 text NOT NULL,
    stage5_manifest_sha256 text NOT NULL,
    action_manifest_sha256 text NOT NULL,
    approval_registry_sha256 text NOT NULL,
    factor_snapshot_sha256 text,
    error_code text,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    started_at timestamptz,
    finished_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT owner_beta_recommendation_runs_status_check CHECK (
        status IN ('PENDING', 'RUNNING', 'SUCCEEDED', 'FAILED', 'CANCELED')
    ),
    CONSTRAINT owner_beta_recommendation_runs_input_kind_check CHECK (
        input_kind = 'owner_beta_historical_price_only_v1'
    ),
    CONSTRAINT owner_beta_recommendation_runs_capability_check CHECK (
        capability = 'PRICE_RETURN_ONLY'
    ),
    CONSTRAINT owner_beta_recommendation_runs_audience_check CHECK (
        audience = 'OWNER_ONLY'
    ),
    CONSTRAINT owner_beta_recommendation_runs_vendor_snapshot_check CHECK (
        vendor_snapshot
    ),
    CONSTRAINT owner_beta_recommendation_runs_strict_pit_check CHECK (
        NOT strict_pit
    ),
    CONSTRAINT owner_beta_recommendation_runs_candidate_hash_check CHECK (
        candidate_content_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_beta_recommendation_runs_artifact_hash_check CHECK (
        artifact_manifest_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_beta_recommendation_runs_stage5_hash_check CHECK (
        stage5_manifest_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_beta_recommendation_runs_action_hash_check CHECK (
        action_manifest_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_beta_recommendation_runs_approval_hash_check CHECK (
        approval_registry_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_beta_recommendation_runs_factor_hash_check CHECK (
        factor_snapshot_sha256 IS NULL
        OR factor_snapshot_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_beta_recommendation_runs_success_factor_check CHECK (
        status <> 'SUCCEEDED' OR factor_snapshot_sha256 IS NOT NULL
    ),
    CONSTRAINT owner_beta_recommendation_runs_error_code_check CHECK (
        error_code IS NULL OR error_code ~ '^[A-Z][A-Z0-9_]{0,63}$'
    ),
    CONSTRAINT owner_beta_recommendation_runs_id_owner_key
        UNIQUE (id, owner_user_id)
);

CREATE INDEX owner_beta_recommendation_runs_owner_created_idx
    ON public.owner_beta_recommendation_runs (owner_user_id, created_at DESC, id DESC);
CREATE INDEX owner_beta_recommendation_runs_worker_status_idx
    ON public.owner_beta_recommendation_runs (status, created_at, id)
    WHERE status IN ('PENDING', 'RUNNING');

CREATE TABLE public.owner_beta_recommendation_items (
    id uuid PRIMARY KEY DEFAULT pg_catalog.gen_random_uuid(),
    recommendation_run_id uuid NOT NULL,
    owner_user_id uuid NOT NULL
        REFERENCES public.users (id) ON DELETE RESTRICT,
    instrument_id text NOT NULL
        REFERENCES public.instruments (id) ON DELETE RESTRICT,
    rank integer,
    target_weight numeric(18, 6),
    reason_codes jsonb NOT NULL DEFAULT '[]'::jsonb,
    factors_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    excluded boolean NOT NULL DEFAULT false,
    exclusion_reason text,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT owner_beta_recommendation_items_run_owner_fkey
        FOREIGN KEY (recommendation_run_id, owner_user_id)
        REFERENCES public.owner_beta_recommendation_runs (id, owner_user_id)
        ON DELETE RESTRICT,
    CONSTRAINT owner_beta_recommendation_items_rank_check CHECK (
        rank IS NULL OR rank > 0
    ),
    CONSTRAINT owner_beta_recommendation_items_weight_check CHECK (
        target_weight IS NULL OR (target_weight >= 0 AND target_weight <= 1)
    ),
    CONSTRAINT owner_beta_recommendation_items_reason_codes_check CHECK (
        pg_catalog.jsonb_typeof(reason_codes) = 'array'
    ),
    CONSTRAINT owner_beta_recommendation_items_factors_check CHECK (
        pg_catalog.jsonb_typeof(factors_json) = 'object'
    ),
    CONSTRAINT owner_beta_recommendation_items_run_instrument_key
        UNIQUE (recommendation_run_id, instrument_id)
);

CREATE INDEX owner_beta_recommendation_items_owner_run_idx
    ON public.owner_beta_recommendation_items (owner_user_id, recommendation_run_id);

ALTER TABLE public.owner_beta_recommendation_runs OWNER TO migration_owner;
ALTER TABLE public.owner_beta_recommendation_items OWNER TO migration_owner;

ALTER TABLE public.owner_beta_recommendation_runs ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_beta_recommendation_runs FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_beta_recommendation_items ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_beta_recommendation_items FORCE ROW LEVEL SECURITY;

REVOKE ALL ON TABLE public.owner_beta_recommendation_runs
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON TABLE public.owner_beta_recommendation_items
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

GRANT SELECT ON TABLE public.owner_beta_recommendation_runs TO app, worker, admin;
GRANT INSERT (
    id, owner_user_id, strategy_config_id, job_id, as_of,
    candidate_content_sha256, artifact_manifest_sha256,
    stage5_manifest_sha256, action_manifest_sha256, approval_registry_sha256
) ON public.owner_beta_recommendation_runs TO app;
GRANT UPDATE (
    status, factor_snapshot_sha256, error_code,
    started_at, finished_at, updated_at
) ON public.owner_beta_recommendation_runs TO worker;

GRANT SELECT ON TABLE public.owner_beta_recommendation_items TO app, worker, admin;
GRANT INSERT (
    id, recommendation_run_id, owner_user_id, instrument_id, rank,
    target_weight, reason_codes, factors_json, excluded, exclusion_reason
) ON public.owner_beta_recommendation_items TO worker;

CREATE POLICY owner_beta_recommendation_runs_app_select
    ON public.owner_beta_recommendation_runs FOR SELECT TO app
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );
CREATE POLICY owner_beta_recommendation_runs_app_insert
    ON public.owner_beta_recommendation_runs FOR INSERT TO app
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );
CREATE POLICY owner_beta_recommendation_runs_worker_select
    ON public.owner_beta_recommendation_runs FOR SELECT TO worker USING (true);
CREATE POLICY owner_beta_recommendation_runs_worker_update
    ON public.owner_beta_recommendation_runs FOR UPDATE TO worker
    USING (true) WITH CHECK (true);
CREATE POLICY owner_beta_recommendation_runs_admin_select
    ON public.owner_beta_recommendation_runs FOR SELECT TO admin USING (true);
CREATE POLICY owner_beta_recommendation_runs_owner_all
    ON public.owner_beta_recommendation_runs FOR ALL TO migration_owner
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    )
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );

CREATE POLICY owner_beta_recommendation_items_app_select
    ON public.owner_beta_recommendation_items FOR SELECT TO app
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );
CREATE POLICY owner_beta_recommendation_items_worker_select
    ON public.owner_beta_recommendation_items FOR SELECT TO worker USING (true);
CREATE POLICY owner_beta_recommendation_items_worker_insert
    ON public.owner_beta_recommendation_items FOR INSERT TO worker
    WITH CHECK (true);
CREATE POLICY owner_beta_recommendation_items_admin_select
    ON public.owner_beta_recommendation_items FOR SELECT TO admin USING (true);
CREATE POLICY owner_beta_recommendation_items_owner_all
    ON public.owner_beta_recommendation_items FOR ALL TO migration_owner
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    )
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );

CREATE FUNCTION public.owner_beta_recommendation_runs_validate_job_binding()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $binding$
DECLARE
    v_job_owner_user_id uuid;
    v_job_type text;
    v_payload jsonb;
    v_top_level_keys text[];
    v_pin_keys text[];
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.id IS DISTINCT FROM OLD.id
            OR NEW.owner_user_id IS DISTINCT FROM OLD.owner_user_id
            OR NEW.strategy_config_id IS DISTINCT FROM OLD.strategy_config_id
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
    END IF;

    PERFORM 1
      FROM public.user_strategy_configs AS config
     WHERE config.id = NEW.strategy_config_id
       AND config.owner_user_id = NEW.owner_user_id
     FOR SHARE OF config;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'owner beta recommendation binding is invalid'
            USING ERRCODE = '23514';
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
        'as_of', 'pins', 'run_id', 'strategy_config_id'
    ]::text[]
       OR pg_catalog.jsonb_typeof(v_payload -> 'run_id') IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(v_payload -> 'strategy_config_id') IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(v_payload -> 'as_of') IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(v_payload -> 'pins') IS DISTINCT FROM 'object'
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

    RETURN NEW;
END
$binding$;

ALTER FUNCTION public.owner_beta_recommendation_runs_validate_job_binding()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_recommendation_runs_validate_job_binding()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

CREATE TRIGGER owner_beta_recommendation_runs_validate_job_binding
    BEFORE INSERT OR UPDATE OF
        id, owner_user_id, strategy_config_id, job_id, as_of,
        input_kind, capability, audience, vendor_snapshot, strict_pit,
        candidate_content_sha256, artifact_manifest_sha256,
        stage5_manifest_sha256, action_manifest_sha256, approval_registry_sha256
    ON public.owner_beta_recommendation_runs
    FOR EACH ROW
    EXECUTE FUNCTION public.owner_beta_recommendation_runs_validate_job_binding();

CREATE FUNCTION public.jobs_protect_owner_beta_recommendation_lineage()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $job_guard$
DECLARE
    v_is_owner_beta boolean;
BEGIN
    SELECT EXISTS (
        SELECT 1
          FROM public.owner_beta_recommendation_runs AS run
         WHERE run.job_id = OLD.id
    ) INTO v_is_owner_beta;

    IF TG_OP = 'DELETE' THEN
        IF v_is_owner_beta THEN
            RAISE EXCEPTION 'owner beta recommendation job lineage is immutable'
                USING ERRCODE = '42501';
        END IF;
        RETURN OLD;
    END IF;

    IF v_is_owner_beta AND (
        NEW.id IS DISTINCT FROM OLD.id
        OR NEW.owner_user_id IS DISTINCT FROM OLD.owner_user_id
        OR NEW.job_type IS DISTINCT FROM OLD.job_type
        OR NEW.idempotency_key IS DISTINCT FROM OLD.idempotency_key
        OR NEW.payload_json IS DISTINCT FROM OLD.payload_json
    ) THEN
        RAISE EXCEPTION 'owner beta recommendation job lineage is immutable'
            USING ERRCODE = '42501';
    END IF;

    RETURN NEW;
END
$job_guard$;

ALTER FUNCTION public.jobs_protect_owner_beta_recommendation_lineage()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.jobs_protect_owner_beta_recommendation_lineage()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

CREATE TRIGGER jobs_protect_owner_beta_recommendation_lineage
    BEFORE UPDATE OR DELETE ON public.jobs
    FOR EACH ROW
    EXECUTE FUNCTION public.jobs_protect_owner_beta_recommendation_lineage();
