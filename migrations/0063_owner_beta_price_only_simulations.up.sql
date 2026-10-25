SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

CREATE OR REPLACE FUNCTION public.assert_candidate_publication_settlement()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $assert$
DECLARE
    v_job_id uuid;
    v_run_id uuid;
    v_run_status text;
    v_job_status text;
    v_attempt_count integer;
    v_service_user_id uuid;
    v_actor_before text := pg_catalog.current_setting('app.actor_user_id', true);
    v_actor_changed boolean := false;
BEGIN
    SELECT control.service_user_id INTO v_service_user_id
    FROM public.candidate_scheduler_control AS control
    WHERE control.control_key = 'scheduler';
    IF NOT FOUND THEN RETURN NULL; END IF;
    -- Constraint triggers run as migration_owner at COMMIT, outside any
    -- request actor scope. Bind the only owner candidate jobs may use before
    -- touching the FORCE-RLS queue tables.
    v_actor_changed := v_actor_before IS DISTINCT FROM v_service_user_id::text;
    PERFORM pg_catalog.set_config(
        'app.actor_user_id', v_service_user_id::text, true
    );
    IF TG_TABLE_NAME = 'stock_analysis_runs' THEN
        IF TG_OP = 'DELETE' THEN
            v_run_id := OLD.id;
            v_job_id := OLD.job_id;
        ELSE
            v_run_id := NEW.id;
            v_job_id := NEW.job_id;
        END IF;
    ELSIF TG_TABLE_NAME = 'jobs' THEN
        IF TG_OP = 'DELETE' THEN v_job_id := OLD.id; ELSE v_job_id := NEW.id; END IF;
        SELECT run.id INTO v_run_id
        FROM public.stock_analysis_runs AS run
        WHERE run.job_id = v_job_id;
    ELSE
        IF TG_OP = 'DELETE' THEN v_job_id := OLD.job_id; ELSE v_job_id := NEW.job_id; END IF;
        SELECT run.id INTO v_run_id
        FROM public.stock_analysis_runs AS run
        WHERE run.job_id = v_job_id;
    END IF;

    IF v_run_id IS NULL OR v_job_id IS NULL THEN
        IF v_actor_changed THEN
            PERFORM pg_catalog.set_config(
                'app.actor_user_id', COALESCE(v_actor_before, ''), true
            );
        END IF;
        RETURN NULL;
    END IF;
    SELECT run.status, job.status, job.attempt_count
    INTO v_run_status, v_job_status, v_attempt_count
    FROM public.stock_analysis_runs AS run
    JOIN public.jobs AS job ON job.id = run.job_id
    WHERE run.id = v_run_id;
    IF NOT FOUND THEN
        IF v_actor_changed THEN
            PERFORM pg_catalog.set_config(
                'app.actor_user_id', COALESCE(v_actor_before, ''), true
            );
        END IF;
        RETURN NULL;
    END IF;

    IF v_run_status = 'SUCCEEDED' OR v_job_status = 'SUCCEEDED' THEN
        IF v_run_status <> 'SUCCEEDED'
            OR v_job_status <> 'SUCCEEDED'
            OR NOT EXISTS (
                SELECT 1
                FROM public.job_attempts AS attempt
                WHERE attempt.job_id = v_job_id
                  AND attempt.attempt_no = v_attempt_count
                  AND attempt.outcome = 'SUCCEEDED'
                  AND attempt.finished_at IS NOT NULL
            )
            OR NOT EXISTS (
                SELECT 1
                FROM public.candidate_feed_snapshots AS feed
                WHERE feed.run_id = v_run_id
            )
        THEN
            RAISE EXCEPTION 'candidate publication and queue settlement must commit atomically'
                USING ERRCODE = '23514';
        END IF;
    ELSIF v_run_status IN ('FAILED', 'BLOCKED')
        OR v_job_status IN ('FAILED', 'CANCELED')
    THEN
        IF v_run_status NOT IN ('FAILED', 'BLOCKED')
            OR v_job_status NOT IN ('FAILED', 'CANCELED')
        THEN
            RAISE EXCEPTION 'candidate failure and queue settlement must commit atomically'
                USING ERRCODE = '23514';
        END IF;
    END IF;
    IF v_actor_changed THEN
        PERFORM pg_catalog.set_config(
            'app.actor_user_id', COALESCE(v_actor_before, ''), true
        );
    END IF;
    RETURN NULL;
END
$assert$;


CREATE TABLE public.owner_beta_simulation_runs (
    id uuid PRIMARY KEY DEFAULT pg_catalog.gen_random_uuid(),
    owner_user_id uuid NOT NULL
        REFERENCES public.users (id) ON DELETE RESTRICT,
    strategy_config_id uuid NOT NULL
        REFERENCES public.user_strategy_configs (id) ON DELETE RESTRICT,
    job_id uuid NOT NULL UNIQUE
        REFERENCES public.jobs (id) ON DELETE RESTRICT,
    strategy_id text NOT NULL,
    strategy_version text NOT NULL,
    strategy_config_json jsonb NOT NULL,
    strategy_config_sha256 text NOT NULL,
    evaluation_start date NOT NULL,
    evaluation_end date NOT NULL,
    session_count integer NOT NULL,
    input_kind text NOT NULL DEFAULT 'owner_beta_historical_price_only_v1',
    capability text NOT NULL DEFAULT 'PRICE_RETURN_ONLY',
    audience text NOT NULL DEFAULT 'OWNER_ONLY',
    vendor_snapshot boolean NOT NULL DEFAULT true,
    strict_pit boolean NOT NULL DEFAULT false,
    action_count integer NOT NULL DEFAULT 0,
    candidate_content_sha256 text NOT NULL,
    artifact_manifest_sha256 text NOT NULL,
    stage5_manifest_sha256 text NOT NULL,
    action_manifest_sha256 text NOT NULL,
    approval_registry_sha256 text NOT NULL,
    contract_document_sha256 text NOT NULL,
    bars_sha256 text NOT NULL,
    status text NOT NULL DEFAULT 'PENDING',
    error_code text,
    engine_json jsonb,
    published_attempt_id uuid
        REFERENCES public.job_attempts (id) ON DELETE RESTRICT,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    started_at timestamptz,
    finished_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT owner_beta_simulation_runs_id_owner_key
        UNIQUE (id, owner_user_id),
    CONSTRAINT owner_beta_simulation_runs_strategy_id_check
        CHECK (strategy_id = 'trend_following'),
    CONSTRAINT owner_beta_simulation_runs_strategy_version_check
        CHECK (strategy_version = '1.0.0'),
    CONSTRAINT owner_beta_simulation_runs_strategy_config_check
        CHECK (strategy_config_json = '{"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200}'::jsonb),
    CONSTRAINT owner_beta_simulation_runs_strategy_config_hash_check
        CHECK (strategy_config_sha256 = 'sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0'),
    CONSTRAINT owner_beta_simulation_runs_window_check
        CHECK (evaluation_start >= DATE '2016-08-29'
           AND evaluation_end <= DATE '2026-08-28'
           AND evaluation_end >= evaluation_start),
    CONSTRAINT owner_beta_simulation_runs_session_count_check
        CHECK (session_count BETWEEN 202 AND 2452),
    CONSTRAINT owner_beta_simulation_runs_input_kind_check
        CHECK (input_kind = 'owner_beta_historical_price_only_v1'),
    CONSTRAINT owner_beta_simulation_runs_capability_check
        CHECK (capability = 'PRICE_RETURN_ONLY'),
    CONSTRAINT owner_beta_simulation_runs_audience_check
        CHECK (audience = 'OWNER_ONLY'),
    CONSTRAINT owner_beta_simulation_runs_vendor_snapshot_check
        CHECK (vendor_snapshot),
    CONSTRAINT owner_beta_simulation_runs_strict_pit_check
        CHECK (NOT strict_pit),
    CONSTRAINT owner_beta_simulation_runs_action_count_check
        CHECK (action_count = 0),
    CONSTRAINT owner_beta_simulation_runs_candidate_hash_check
        CHECK (candidate_content_sha256 = 'sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d'),
    CONSTRAINT owner_beta_simulation_runs_artifact_hash_check
        CHECK (artifact_manifest_sha256 = 'sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6'),
    CONSTRAINT owner_beta_simulation_runs_stage5_hash_check
        CHECK (stage5_manifest_sha256 = 'sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a'),
    CONSTRAINT owner_beta_simulation_runs_action_hash_check
        CHECK (action_manifest_sha256 = 'sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550'),
    CONSTRAINT owner_beta_simulation_runs_approval_hash_check
        CHECK (approval_registry_sha256 = 'sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707'),
    CONSTRAINT owner_beta_simulation_runs_contract_hash_check
        CHECK (contract_document_sha256 = 'sha256:8c5c6bf7f07de09f14f1e5eaa2b6479be4b10e8b98ed5a89a280ac95568e87cd'),
    CONSTRAINT owner_beta_simulation_runs_bars_hash_check
        CHECK (bars_sha256 = 'sha256:23354c54708d7a458aadc4f2cb2fca77469969e91256b6d3b119101ebae98ffe'),
    CONSTRAINT owner_beta_simulation_runs_status_check
        CHECK (status IN ('PENDING', 'SUCCEEDED', 'FAILED', 'CANCELED')),
    CONSTRAINT owner_beta_simulation_runs_error_code_check
        CHECK (error_code IS NULL OR error_code ~ '^[A-Z][A-Z0-9_]{0,63}$'),
    CONSTRAINT owner_beta_simulation_runs_engine_check
        CHECK (engine_json IS NULL OR pg_catalog.jsonb_typeof(engine_json) = 'object'),
    CONSTRAINT owner_beta_simulation_runs_terminal_shape_check CHECK (
        (status = 'PENDING'
            AND error_code IS NULL
            AND engine_json IS NULL
            AND published_attempt_id IS NULL
            AND finished_at IS NULL)
        OR (status = 'SUCCEEDED'
            AND error_code IS NULL
            AND engine_json IS NOT NULL
            AND published_attempt_id IS NOT NULL
            AND started_at IS NOT NULL
            AND finished_at IS NOT NULL)
        OR (status IN ('FAILED', 'CANCELED')
            AND error_code IS NOT NULL
            AND engine_json IS NULL
            AND published_attempt_id IS NULL
            AND finished_at IS NOT NULL)
    )
);

CREATE INDEX owner_beta_simulation_runs_owner_created_idx
    ON public.owner_beta_simulation_runs (owner_user_id, created_at DESC, id DESC);
CREATE INDEX owner_beta_simulation_runs_worker_status_idx
    ON public.owner_beta_simulation_runs (status, created_at, id)
    WHERE status = 'PENDING';

CREATE TABLE public.owner_beta_simulation_result_pairs (
    run_id uuid PRIMARY KEY,
    owner_user_id uuid NOT NULL
        REFERENCES public.users (id) ON DELETE RESTRICT,
    illustrative_canonical bytea NOT NULL,
    illustrative_sha256 text NOT NULL,
    zero_cost_canonical bytea NOT NULL,
    zero_cost_sha256 text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT owner_beta_simulation_result_pairs_run_owner_fkey
        FOREIGN KEY (run_id, owner_user_id)
        REFERENCES public.owner_beta_simulation_runs (id, owner_user_id)
        ON DELETE RESTRICT,
    CONSTRAINT owner_beta_simulation_result_pairs_illustrative_hash_check
        CHECK (illustrative_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    CONSTRAINT owner_beta_simulation_result_pairs_zero_cost_hash_check
        CHECK (zero_cost_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    CONSTRAINT owner_beta_simulation_result_pairs_size_check
        CHECK (pg_catalog.octet_length(illustrative_canonical)::bigint
             + pg_catalog.octet_length(zero_cost_canonical)::bigint <= 67108864)
);

CREATE INDEX owner_beta_simulation_result_pairs_owner_run_idx
    ON public.owner_beta_simulation_result_pairs (owner_user_id, run_id);

ALTER TABLE public.owner_beta_simulation_runs OWNER TO migration_owner;
ALTER TABLE public.owner_beta_simulation_result_pairs OWNER TO migration_owner;

ALTER TABLE public.owner_beta_simulation_runs ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_beta_simulation_runs FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_beta_simulation_result_pairs ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_beta_simulation_result_pairs FORCE ROW LEVEL SECURITY;

REVOKE ALL ON TABLE public.owner_beta_simulation_runs
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON TABLE public.owner_beta_simulation_result_pairs
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

GRANT SELECT ON TABLE public.owner_beta_simulation_runs TO app, worker;
GRANT INSERT (
    id, owner_user_id, strategy_config_id, job_id,
    strategy_id, strategy_version, strategy_config_json, strategy_config_sha256,
    evaluation_start, evaluation_end, session_count,
    input_kind, capability, audience, vendor_snapshot, strict_pit, action_count,
    candidate_content_sha256, artifact_manifest_sha256, stage5_manifest_sha256,
    action_manifest_sha256, approval_registry_sha256,
    contract_document_sha256, bars_sha256
) ON public.owner_beta_simulation_runs TO app;
GRANT UPDATE (
    status, error_code, engine_json, published_attempt_id,
    started_at, finished_at, updated_at
) ON public.owner_beta_simulation_runs TO worker;
GRANT SELECT (
    id, owner_user_id, strategy_config_id, job_id,
    strategy_id, strategy_version, strategy_config_json, strategy_config_sha256,
    evaluation_start, evaluation_end, session_count,
    input_kind, capability, audience, vendor_snapshot, strict_pit, action_count,
    candidate_content_sha256, artifact_manifest_sha256, stage5_manifest_sha256,
    action_manifest_sha256, approval_registry_sha256,
    contract_document_sha256, bars_sha256, status, error_code,
    engine_json, published_attempt_id, created_at, started_at, finished_at, updated_at
) ON public.owner_beta_simulation_runs TO admin;

GRANT SELECT ON TABLE public.owner_beta_simulation_result_pairs TO app, worker;
GRANT INSERT (
    run_id, owner_user_id,
    illustrative_canonical, illustrative_sha256,
    zero_cost_canonical, zero_cost_sha256
) ON public.owner_beta_simulation_result_pairs TO worker;

CREATE POLICY owner_beta_simulation_runs_app_select
    ON public.owner_beta_simulation_runs FOR SELECT TO app
    USING (owner_user_id = NULLIF(
        pg_catalog.current_setting('app.actor_user_id', true), ''
    )::uuid);
CREATE POLICY owner_beta_simulation_runs_app_insert
    ON public.owner_beta_simulation_runs FOR INSERT TO app
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
        AND status = 'PENDING'
        AND error_code IS NULL
        AND engine_json IS NULL
        AND published_attempt_id IS NULL
    );
CREATE POLICY owner_beta_simulation_runs_worker_select
    ON public.owner_beta_simulation_runs FOR SELECT TO worker USING (true);
CREATE POLICY owner_beta_simulation_runs_worker_update
    ON public.owner_beta_simulation_runs FOR UPDATE TO worker
    USING (true) WITH CHECK (true);
CREATE POLICY owner_beta_simulation_runs_admin_select
    ON public.owner_beta_simulation_runs FOR SELECT TO admin USING (true);
CREATE POLICY owner_beta_simulation_runs_migration_owner_actor
    ON public.owner_beta_simulation_runs FOR ALL TO migration_owner
    USING (owner_user_id = NULLIF(
        pg_catalog.current_setting('app.actor_user_id', true), ''
    )::uuid)
    WITH CHECK (owner_user_id = NULLIF(
        pg_catalog.current_setting('app.actor_user_id', true), ''
    )::uuid);

CREATE FUNCTION public.owner_beta_simulation_pair_read_allowed(
    p_run_id uuid,
    p_owner_id uuid
)
RETURNS boolean
LANGUAGE plpgsql
STABLE
SECURITY INVOKER
CALLED ON NULL INPUT
PARALLEL UNSAFE
SET search_path = pg_catalog
AS $pair_read$
DECLARE
    v_actor_raw text;
    v_actor_id uuid;
BEGIN
    v_actor_raw := pg_catalog.current_setting('app.actor_user_id', true);
    IF NULLIF(v_actor_raw, '') IS NULL THEN
        RETURN false;
    END IF;

    v_actor_id := v_actor_raw::uuid;
    IF p_run_id IS NULL OR p_owner_id IS DISTINCT FROM v_actor_id THEN
        RETURN false;
    END IF;

    RETURN EXISTS (
        SELECT 1
        FROM public.owner_beta_simulation_runs AS run
        JOIN public.jobs AS job ON job.id = run.job_id
        WHERE run.id = p_run_id
          AND run.owner_user_id = p_owner_id
          AND run.status = 'SUCCEEDED'
          AND job.owner_user_id = p_owner_id
          AND job.job_type = 'owner_beta_price_only_simulation'
          AND job.status = 'SUCCEEDED'
    );
END
$pair_read$;
ALTER FUNCTION public.owner_beta_simulation_pair_read_allowed(uuid, uuid)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_simulation_pair_read_allowed(uuid, uuid)
    FROM PUBLIC, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.owner_beta_simulation_pair_read_allowed(uuid, uuid)
    TO app;

CREATE POLICY owner_beta_simulation_pairs_app_select
    ON public.owner_beta_simulation_result_pairs FOR SELECT TO app
    USING (public.owner_beta_simulation_pair_read_allowed(
        owner_beta_simulation_result_pairs.run_id,
        owner_beta_simulation_result_pairs.owner_user_id
    )
    );
CREATE POLICY owner_beta_simulation_pairs_worker_select
    ON public.owner_beta_simulation_result_pairs FOR SELECT TO worker USING (true);
CREATE POLICY owner_beta_simulation_pairs_worker_insert
    ON public.owner_beta_simulation_result_pairs FOR INSERT TO worker
    WITH CHECK (true);
CREATE POLICY owner_beta_simulation_pairs_migration_owner_actor
    ON public.owner_beta_simulation_result_pairs FOR ALL TO migration_owner
    USING (owner_user_id = NULLIF(
        pg_catalog.current_setting('app.actor_user_id', true), ''
    )::uuid)
    WITH CHECK (owner_user_id = NULLIF(
        pg_catalog.current_setting('app.actor_user_id', true), ''
    )::uuid);

-- Trigger reads of generic jobs can start with no actor; keep the owner policy null-safe.
ALTER POLICY tenant_all_owner_jobs ON public.jobs TO migration_owner
    USING (owner_user_id = NULLIF(current_setting('app.actor_user_id', true), '')::uuid)
    WITH CHECK (owner_user_id = NULLIF(current_setting('app.actor_user_id', true), '')::uuid);

-- Trigger-only JSON binding helper. The caller's exact payload is compared as
-- jsonb, so extra keys, wrong JSON types, dates, pins, or strategy fields fail.
CREATE FUNCTION public.owner_beta_simulation_payload_matches(
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

CREATE FUNCTION public.owner_beta_simulation_jobs_insert_guard()
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
CREATE TRIGGER owner_beta_simulation_jobs_insert_guard
    BEFORE INSERT ON public.jobs
    FOR EACH ROW EXECUTE FUNCTION public.owner_beta_simulation_jobs_insert_guard();

-- Jobs are inserted before their linked run. The actor is taken from the
-- actual job row before any child trigger inspects its owner-scoped lineage.
CREATE FUNCTION public.owner_beta_simulation_jobs_lineage_guard()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $jobs_lineage$
DECLARE
    v_actor_before text := pg_catalog.current_setting('app.actor_user_id', true);
    v_actor_changed boolean := false;
    v_has_run boolean;
BEGIN
    PERFORM NULLIF(v_actor_before, '')::uuid;
    IF v_actor_before IS DISTINCT FROM OLD.owner_user_id::text THEN
        PERFORM pg_catalog.set_config('app.actor_user_id', OLD.owner_user_id::text, true);
        v_actor_changed := true;
    END IF;
    SELECT EXISTS (
        SELECT 1
          FROM public.owner_beta_simulation_runs AS run
         WHERE run.job_id = OLD.id
           AND run.owner_user_id = OLD.owner_user_id
    ) INTO v_has_run;

    IF TG_OP = 'DELETE' THEN
        IF OLD.job_type = 'owner_beta_price_only_simulation' OR v_has_run THEN
            RAISE EXCEPTION 'owner beta simulation job lineage is immutable'
                USING ERRCODE = '42501';
        END IF;
    IF v_actor_changed THEN
        PERFORM pg_catalog.set_config(
            'app.actor_user_id', COALESCE(v_actor_before, ''), true
        );
    END IF;
        RETURN OLD;
    END IF;

    IF (OLD.job_type = 'owner_beta_price_only_simulation'
        OR NEW.job_type = 'owner_beta_price_only_simulation'
        OR v_has_run)
       AND (
            NEW.id IS DISTINCT FROM OLD.id
            OR NEW.owner_user_id IS DISTINCT FROM OLD.owner_user_id
            OR NEW.job_type IS DISTINCT FROM OLD.job_type
            OR NEW.idempotency_key IS DISTINCT FROM OLD.idempotency_key
            OR NEW.payload_json IS DISTINCT FROM OLD.payload_json
            OR NEW.max_attempts IS DISTINCT FROM OLD.max_attempts
       )
    THEN
        RAISE EXCEPTION 'owner beta simulation job lineage is immutable'
            USING ERRCODE = '42501';
    END IF;
    IF v_actor_changed THEN
        PERFORM pg_catalog.set_config(
            'app.actor_user_id', COALESCE(v_actor_before, ''), true
        );
    END IF;
    RETURN NEW;
END
$jobs_lineage$;
ALTER FUNCTION public.owner_beta_simulation_jobs_lineage_guard() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_simulation_jobs_lineage_guard()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
CREATE TRIGGER owner_beta_simulation_jobs_lineage_guard
    BEFORE UPDATE OR DELETE ON public.jobs
    FOR EACH ROW EXECUTE FUNCTION public.owner_beta_simulation_jobs_lineage_guard();

CREATE POLICY owner_beta_simulation_jobs_trigger_select
    ON public.jobs FOR SELECT TO migration_owner
    USING (
        job_type = 'owner_beta_price_only_simulation'
        AND pg_catalog.pg_trigger_depth() > 0
    );

CREATE FUNCTION public.owner_beta_simulation_runs_validate_binding()
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
CREATE TRIGGER owner_beta_simulation_runs_validate_binding
    BEFORE INSERT OR UPDATE OR DELETE ON public.owner_beta_simulation_runs
    FOR EACH ROW EXECUTE FUNCTION public.owner_beta_simulation_runs_validate_binding();

-- This mirrors the trigger-only 0052 pattern: config locking comes from the
-- immutable run row, with no worker UPDATE grant or caller-selected owner.
CREATE FUNCTION public.owner_beta_simulation_runs_lock_config_on_success()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $config_lock$
DECLARE
    v_actor_before text := pg_catalog.current_setting('app.actor_user_id', true);
    v_actor_changed boolean := false;
BEGIN
    IF NEW.status IS DISTINCT FROM 'SUCCEEDED'
       OR OLD.status IS NOT DISTINCT FROM 'SUCCEEDED'
    THEN
        RETURN NEW;
    END IF;
    PERFORM NULLIF(v_actor_before, '')::uuid;
    IF v_actor_before IS DISTINCT FROM NEW.owner_user_id::text THEN
        PERFORM pg_catalog.set_config('app.actor_user_id', NEW.owner_user_id::text, true);
        v_actor_changed := true;
    END IF;
    PERFORM 1
      FROM public.user_strategy_configs AS config
     WHERE config.id = NEW.strategy_config_id
       AND config.owner_user_id = NEW.owner_user_id
     FOR SHARE OF config;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'owner beta simulation strategy config identity is unavailable'
            USING ERRCODE = '23514';
    END IF;
    IF v_actor_changed THEN
        PERFORM pg_catalog.set_config(
            'app.actor_user_id', COALESCE(v_actor_before, ''), true
        );
    END IF;
    RETURN NEW;
END
$config_lock$;
ALTER FUNCTION public.owner_beta_simulation_runs_lock_config_on_success()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_simulation_runs_lock_config_on_success()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
CREATE TRIGGER owner_beta_simulation_runs_lock_config_on_success
    BEFORE UPDATE OF status ON public.owner_beta_simulation_runs
    FOR EACH ROW EXECUTE FUNCTION public.owner_beta_simulation_runs_lock_config_on_success();

CREATE FUNCTION public.owner_beta_simulation_jobs_validate_insert_payload()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $job_payload$
DECLARE
    v_run_id uuid;
    v_config_id uuid;
    v_start date;
    v_end date;
    v_candidate text;
    v_artifact text;
    v_stage5 text;
    v_action text;
    v_approval text;
    v_contract text;
    v_bars text;
    v_strategy_id text;
    v_strategy_version text;
    v_strategy_config jsonb;
    v_strategy_sha text;
BEGIN
    IF NEW.job_type <> 'owner_beta_price_only_simulation' THEN
        RETURN NEW;
    END IF;
    BEGIN
        v_run_id := (NEW.payload_json ->> 'run_id')::uuid;
        v_config_id := (NEW.payload_json ->> 'strategy_config_id')::uuid;
        v_start := (NEW.payload_json ->> 'evaluation_start')::date;
        v_end := (NEW.payload_json ->> 'evaluation_end')::date;
    EXCEPTION WHEN OTHERS THEN
        RAISE EXCEPTION 'owner beta simulation job payload is invalid'
            USING ERRCODE = '23514';
    END;
    v_candidate := NEW.payload_json -> 'pins' ->> 'candidate_content_sha256';
    v_artifact := NEW.payload_json -> 'pins' ->> 'artifact_manifest_sha256';
    v_stage5 := NEW.payload_json -> 'pins' ->> 'stage5_manifest_sha256';
    v_action := NEW.payload_json -> 'pins' ->> 'action_manifest_sha256';
    v_approval := NEW.payload_json -> 'pins' ->> 'approval_registry_sha256';
    v_contract := NEW.payload_json ->> 'contract_document_sha256';
    v_bars := NEW.payload_json ->> 'bars_sha256';
    v_strategy_id := NEW.payload_json -> 'strategy' ->> 'strategy_id';
    v_strategy_version := NEW.payload_json -> 'strategy' ->> 'strategy_version';
    v_strategy_config := NEW.payload_json -> 'strategy' -> 'config_json';
    v_strategy_sha := NEW.payload_json -> 'strategy' ->> 'config_sha256';

    IF NOT public.owner_beta_simulation_payload_matches(
        NEW.payload_json, v_run_id, v_config_id, v_start, v_end,
        v_candidate, v_artifact, v_stage5, v_action, v_approval,
        v_contract, v_bars, v_strategy_id, v_strategy_version,
        v_strategy_config, v_strategy_sha
    )
       OR v_start < DATE '2016-08-29'
       OR v_end > DATE '2026-08-28'
       OR v_end < v_start
    THEN
        RAISE EXCEPTION 'owner beta simulation job payload is invalid'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END
$job_payload$;
ALTER FUNCTION public.owner_beta_simulation_jobs_validate_insert_payload()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_simulation_jobs_validate_insert_payload()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
CREATE TRIGGER owner_beta_simulation_jobs_validate_insert_payload
    BEFORE INSERT ON public.jobs
    FOR EACH ROW EXECUTE FUNCTION public.owner_beta_simulation_jobs_validate_insert_payload();

CREATE FUNCTION public.owner_beta_simulation_pairs_validate_insert()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $pair_binding$
DECLARE
    v_owner uuid;
    v_job_id uuid;
    v_job_owner uuid;
    v_job_type text;
    v_payload jsonb;
    v_run_owner uuid;
    v_run_strategy_config_id uuid;
    v_run_strategy_id text;
    v_run_strategy_version text;
    v_run_strategy_json jsonb;
    v_run_strategy_sha text;
    v_run_start date;
    v_run_end date;
    v_run_candidate text;
    v_run_artifact text;
    v_run_stage5 text;
    v_run_action text;
    v_run_approval text;
    v_run_contract text;
    v_run_bars text;
    v_actor_before text := pg_catalog.current_setting('app.actor_user_id', true);
    v_actor_changed boolean := false;
BEGIN
    IF TG_OP <> 'INSERT' THEN
        RAISE EXCEPTION 'owner beta simulation result pair is immutable'
            USING ERRCODE = '42501';
    END IF;

    PERFORM NULLIF(v_actor_before, '')::uuid;
    IF v_actor_before IS DISTINCT FROM NEW.owner_user_id::text THEN
        PERFORM pg_catalog.set_config('app.actor_user_id', NEW.owner_user_id::text, true);
        v_actor_changed := true;
    END IF;
    SELECT run.owner_user_id, run.job_id, run.strategy_config_id,
           run.strategy_id, run.strategy_version, run.strategy_config_json,
           run.strategy_config_sha256, run.evaluation_start, run.evaluation_end,
           run.candidate_content_sha256, run.artifact_manifest_sha256,
           run.stage5_manifest_sha256, run.action_manifest_sha256,
           run.approval_registry_sha256, run.contract_document_sha256, run.bars_sha256
      INTO v_owner, v_job_id, v_run_strategy_config_id,
           v_run_strategy_id, v_run_strategy_version, v_run_strategy_json,
           v_run_strategy_sha, v_run_start, v_run_end,
           v_run_candidate, v_run_artifact, v_run_stage5, v_run_action,
           v_run_approval, v_run_contract, v_run_bars
      FROM public.owner_beta_simulation_runs AS run
     WHERE run.id = NEW.run_id
       AND run.owner_user_id = NEW.owner_user_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'owner beta simulation result pair binding is invalid'
            USING ERRCODE = '23514';
    END IF;

    -- Lock in the canonical order before the linked run row.
    SELECT job.owner_user_id, job.job_type, job.payload_json
      INTO v_job_owner, v_job_type, v_payload
      FROM public.jobs AS job
     WHERE job.id = v_job_id
     FOR UPDATE OF job;
    IF NOT FOUND
       OR v_owner IS DISTINCT FROM NEW.owner_user_id
       OR v_job_owner IS DISTINCT FROM NEW.owner_user_id
       OR v_job_type IS DISTINCT FROM 'owner_beta_price_only_simulation'
       OR NOT public.owner_beta_simulation_payload_matches(
            v_payload, NEW.run_id, v_run_strategy_config_id,
            v_run_start, v_run_end, v_run_candidate, v_run_artifact,
            v_run_stage5, v_run_action, v_run_approval, v_run_contract,
            v_run_bars, v_run_strategy_id, v_run_strategy_version,
            v_run_strategy_json, v_run_strategy_sha
       )
    THEN
        RAISE EXCEPTION 'owner beta simulation result pair binding is invalid'
            USING ERRCODE = '23514';
    END IF;

    PERFORM 1
      FROM public.owner_beta_simulation_runs AS run
     WHERE run.id = NEW.run_id
       AND run.owner_user_id = NEW.owner_user_id
     FOR UPDATE OF run;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'owner beta simulation result pair binding is invalid'
            USING ERRCODE = '23514';
    END IF;
    IF v_actor_changed THEN
        PERFORM pg_catalog.set_config(
            'app.actor_user_id', COALESCE(v_actor_before, ''), true
        );
    END IF;
    RETURN NEW;
END
$pair_binding$;
ALTER FUNCTION public.owner_beta_simulation_pairs_validate_insert()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_simulation_pairs_validate_insert()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
CREATE TRIGGER owner_beta_simulation_pairs_validate_insert
    BEFORE INSERT OR UPDATE OR DELETE ON public.owner_beta_simulation_result_pairs
    FOR EACH ROW EXECUTE FUNCTION public.owner_beta_simulation_pairs_validate_insert();

-- Every attempt transition for the new type is bound back to its concrete job
-- row. The migration-owner policy above exposes only this row during protected
-- trigger execution; the owner GUC is then derived from that row before the
-- run/pair policy is consulted.
CREATE FUNCTION public.owner_beta_simulation_attempt_guard()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $attempt_guard$
DECLARE
    v_actor_before text := pg_catalog.current_setting('app.actor_user_id', true);
    v_actor_changed boolean := false;
    v_old_is_r4 boolean := false;
    v_new_is_r4 boolean := false;
    v_owner uuid;
    v_job_type text;
    v_job_status text;
    v_attempt_count integer;
    v_locked_by text;
    v_run_status text;
    v_published_attempt_id uuid;
    v_pair_exists boolean;
BEGIN
    -- Reject malformed caller context before RLS discovery, even on the ORed trigger policy.
    PERFORM NULLIF(v_actor_before, '')::uuid;
    IF TG_OP = 'INSERT' THEN
        SELECT job.owner_user_id, job.job_type, job.status, job.attempt_count, job.locked_by
          INTO v_owner, v_job_type, v_job_status, v_attempt_count, v_locked_by
          FROM public.jobs AS job
         WHERE job.id = NEW.job_id
           AND job.job_type = 'owner_beta_price_only_simulation';
        IF NOT FOUND THEN
            RETURN NEW;
        END IF;
    ELSIF TG_OP = 'DELETE' THEN
        SELECT job.owner_user_id, job.job_type, job.status, job.attempt_count, job.locked_by
          INTO v_owner, v_job_type, v_job_status, v_attempt_count, v_locked_by
          FROM public.jobs AS job
         WHERE job.id = OLD.job_id
           AND job.job_type = 'owner_beta_price_only_simulation';
        IF NOT FOUND THEN
            RETURN OLD;
        END IF;
    ELSE
        PERFORM 1
          FROM public.jobs AS job
         WHERE job.id = OLD.job_id
           AND job.job_type = 'owner_beta_price_only_simulation';
        v_old_is_r4 := FOUND;
        PERFORM 1
          FROM public.jobs AS job
         WHERE job.id = NEW.job_id
           AND job.job_type = 'owner_beta_price_only_simulation';
        v_new_is_r4 := FOUND;

        IF NEW.job_id IS DISTINCT FROM OLD.job_id THEN
            IF v_old_is_r4 OR v_new_is_r4 THEN
                RAISE EXCEPTION 'owner beta simulation attempt binding is immutable'
                    USING ERRCODE = '42501';
            END IF;
            RETURN NEW;
        END IF;
        IF NOT v_old_is_r4 AND NOT v_new_is_r4 THEN
            RETURN NEW;
        END IF;

        -- The unchanged attempt remains anchored to its OLD parent identity.
        SELECT job.owner_user_id, job.job_type, job.status, job.attempt_count, job.locked_by
          INTO v_owner, v_job_type, v_job_status, v_attempt_count, v_locked_by
          FROM public.jobs AS job
         WHERE job.id = OLD.job_id
           AND job.job_type = 'owner_beta_price_only_simulation';
        IF NOT FOUND THEN
            RAISE EXCEPTION 'owner beta simulation attempt binding is immutable'
                USING ERRCODE = '42501';
        END IF;
    END IF;

    IF v_actor_before IS DISTINCT FROM v_owner::text THEN
        PERFORM pg_catalog.set_config('app.actor_user_id', v_owner::text, true);
        v_actor_changed := true;
    END IF;
    IF TG_OP = 'INSERT' THEN
        IF NEW.attempt_no <> v_attempt_count
           OR NEW.outcome <> 'RUNNING'
           OR NEW.claimed_by IS DISTINCT FROM v_locked_by
           OR v_job_status <> 'RUNNING'
        THEN
            RAISE EXCEPTION 'owner beta simulation attempt binding is invalid'
                USING ERRCODE = '23514';
        END IF;
        IF v_actor_changed THEN
            PERFORM pg_catalog.set_config(
                'app.actor_user_id', COALESCE(v_actor_before, ''), true
            );
        END IF;
        RETURN NEW;
    END IF;

    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'owner beta simulation attempts are immutable'
            USING ERRCODE = '42501';
    END IF;
    IF NEW.id IS DISTINCT FROM OLD.id
       OR NEW.job_id IS DISTINCT FROM OLD.job_id
       OR NEW.attempt_no IS DISTINCT FROM OLD.attempt_no
       OR NEW.claimed_by IS DISTINCT FROM OLD.claimed_by
       OR NEW.started_at IS DISTINCT FROM OLD.started_at
       OR OLD.outcome <> 'RUNNING'
       OR NEW.outcome NOT IN ('SUCCEEDED', 'FAILED', 'ORPHANED')
       OR NEW.finished_at IS NULL
       OR (NEW.outcome = 'SUCCEEDED'
           AND (NEW.error_code IS NOT NULL OR NEW.error_message IS NOT NULL))
    THEN
        RAISE EXCEPTION 'owner beta simulation attempt binding is immutable'
            USING ERRCODE = '42501';
    END IF;

    SELECT run.status, run.published_attempt_id
      INTO v_run_status, v_published_attempt_id
      FROM public.owner_beta_simulation_runs AS run
     WHERE run.job_id = OLD.job_id
       AND run.owner_user_id = v_owner;
    IF FOUND AND NEW.outcome = 'SUCCEEDED'
       AND (v_run_status <> 'SUCCEEDED'
            OR v_published_attempt_id IS DISTINCT FROM OLD.id)
    THEN
        RAISE EXCEPTION 'owner beta simulation success attempt is not published'
            USING ERRCODE = '23514';
    END IF;
    IF FOUND AND v_run_status = 'SUCCEEDED'
       AND (NEW.outcome <> 'SUCCEEDED'
            OR v_published_attempt_id IS DISTINCT FROM OLD.id)
    THEN
        RAISE EXCEPTION 'owner beta simulation published attempt is immutable'
            USING ERRCODE = '42501';
    END IF;
    IF v_actor_changed THEN
        PERFORM pg_catalog.set_config(
            'app.actor_user_id', COALESCE(v_actor_before, ''), true
        );
    END IF;
    RETURN NEW;
END
$attempt_guard$;
ALTER FUNCTION public.owner_beta_simulation_attempt_guard() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_simulation_attempt_guard()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
CREATE TRIGGER owner_beta_simulation_attempt_guard
    BEFORE INSERT OR UPDATE OR DELETE ON public.job_attempts
    FOR EACH ROW EXECUTE FUNCTION public.owner_beta_simulation_attempt_guard();

CREATE FUNCTION public.owner_beta_simulation_deferred_guard()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $deferred_guard$
DECLARE
    v_job_id uuid;
    v_run_id uuid;
    v_owner uuid;
    v_job_owner uuid;
    v_job_type text;
    v_job_status text;
    v_attempt_count integer;
    v_max_attempts integer;
    v_locked_by text;
    v_locked_at timestamptz;
    v_run_owner uuid;
    v_run_status text;
    v_run_strategy_config_id uuid;
    v_run_strategy_id text;
    v_run_strategy_version text;
    v_run_strategy_json jsonb;
    v_run_strategy_sha text;
    v_run_start date;
    v_run_end date;
    v_run_candidate text;
    v_run_artifact text;
    v_run_stage5 text;
    v_run_action text;
    v_run_approval text;
    v_run_contract text;
    v_run_bars text;
    v_run_session_count integer;
    v_run_input_kind text;
    v_run_capability text;
    v_run_audience text;
    v_run_vendor_snapshot boolean;
    v_run_strict_pit boolean;
    v_run_action_count integer;
    v_run_error_code text;
    v_engine_json jsonb;
    v_published_attempt_id uuid;
    v_pair_owner uuid;
    v_pair_exists boolean;
    v_payload jsonb;
    v_attempt_job_id uuid;
    v_attempt_no integer;
    v_attempt_outcome text;
    v_attempt_claimed_by text;
    v_actor_before text := pg_catalog.current_setting('app.actor_user_id', true);
    v_actor_changed boolean := false;
BEGIN
    PERFORM NULLIF(v_actor_before, '')::uuid;
    IF TG_TABLE_NAME = 'jobs' THEN
        v_job_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.id ELSE NEW.id END;
        v_owner := CASE WHEN TG_OP = 'DELETE' THEN OLD.owner_user_id ELSE NEW.owner_user_id END;
        v_job_type := CASE WHEN TG_OP = 'DELETE' THEN OLD.job_type ELSE NEW.job_type END;
        IF v_job_type <> 'owner_beta_price_only_simulation' THEN
            RETURN NULL;
        END IF;
        v_run_id := NULL;
    ELSIF TG_TABLE_NAME = 'owner_beta_simulation_runs' THEN
        v_run_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.id ELSE NEW.id END;
        v_job_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.job_id ELSE NEW.job_id END;
        v_owner := CASE WHEN TG_OP = 'DELETE' THEN OLD.owner_user_id ELSE NEW.owner_user_id END;
    ELSIF TG_TABLE_NAME = 'owner_beta_simulation_result_pairs' THEN
        v_run_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.run_id ELSE NEW.run_id END;
        v_owner := CASE WHEN TG_OP = 'DELETE' THEN OLD.owner_user_id ELSE NEW.owner_user_id END;
        IF v_actor_before IS DISTINCT FROM v_owner::text THEN
            PERFORM pg_catalog.set_config('app.actor_user_id', v_owner::text, true);
            v_actor_changed := true;
        END IF;
        SELECT run.job_id INTO v_job_id
          FROM public.owner_beta_simulation_runs AS run
         WHERE run.id = v_run_id AND run.owner_user_id = v_owner;
    ELSE
        v_job_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.job_id ELSE NEW.job_id END;
        v_owner := NULL;
        v_job_type := NULL;
        SELECT job.owner_user_id, job.job_type INTO v_owner, v_job_type
          FROM public.jobs AS job
         WHERE job.id = v_job_id;
        IF NOT FOUND THEN
            RETURN NULL;
        END IF;
        IF v_job_type <> 'owner_beta_price_only_simulation' THEN
            RETURN NULL;
        END IF;
        IF v_actor_before IS DISTINCT FROM v_owner::text THEN
            PERFORM pg_catalog.set_config('app.actor_user_id', v_owner::text, true);
            v_actor_changed := true;
        END IF;
        SELECT run.id INTO v_run_id
          FROM public.owner_beta_simulation_runs AS run
         WHERE run.job_id = v_job_id AND run.owner_user_id = v_owner;
    END IF;

    IF v_owner IS NULL OR v_job_id IS NULL THEN
        RAISE EXCEPTION 'owner beta simulation deferred binding is incomplete'
            USING ERRCODE = '23514';
    END IF;
    IF NOT v_actor_changed
       AND v_actor_before IS DISTINCT FROM v_owner::text THEN
        PERFORM pg_catalog.set_config('app.actor_user_id', v_owner::text, true);
        v_actor_changed := true;
    END IF;

    -- The committed-state inspection follows job -> run -> pair lock order.
    SELECT job.owner_user_id, job.job_type, job.status, job.attempt_count,
           job.max_attempts, job.locked_by, job.locked_at, job.payload_json
      INTO v_job_owner, v_job_type, v_job_status, v_attempt_count,
           v_max_attempts, v_locked_by, v_locked_at, v_payload
      FROM public.jobs AS job
     WHERE job.id = v_job_id
     FOR UPDATE OF job;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'owner beta simulation job is missing'
            USING ERRCODE = '23514';
    END IF;
    IF v_job_type <> 'owner_beta_price_only_simulation' THEN
        IF EXISTS (
            SELECT 1 FROM public.owner_beta_simulation_runs AS run
             WHERE run.job_id = v_job_id AND run.owner_user_id = v_owner
        ) THEN
            RAISE EXCEPTION 'owner beta simulation job was retagged'
                USING ERRCODE = '23514';
        END IF;
        IF v_actor_changed THEN
            PERFORM pg_catalog.set_config(
                'app.actor_user_id', COALESCE(v_actor_before, ''), true
            );
        END IF;
        RETURN NULL;
    END IF;
    IF v_job_owner IS DISTINCT FROM v_owner THEN
        RAISE EXCEPTION 'owner beta simulation job owner binding is invalid'
            USING ERRCODE = '23514';
    END IF;

    SELECT run.id, run.owner_user_id, run.status, run.strategy_config_id,
           run.strategy_id, run.strategy_version, run.strategy_config_json,
           run.strategy_config_sha256, run.evaluation_start, run.evaluation_end,
           run.candidate_content_sha256, run.artifact_manifest_sha256,
           run.stage5_manifest_sha256, run.action_manifest_sha256,
           run.approval_registry_sha256, run.contract_document_sha256, run.bars_sha256,
           run.session_count, run.input_kind, run.capability, run.audience,
           run.vendor_snapshot, run.strict_pit, run.action_count, run.error_code,
           run.engine_json, run.published_attempt_id
      INTO v_run_id, v_run_owner, v_run_status, v_run_strategy_config_id,
           v_run_strategy_id, v_run_strategy_version, v_run_strategy_json,
           v_run_strategy_sha, v_run_start, v_run_end,
           v_run_candidate, v_run_artifact, v_run_stage5, v_run_action,
           v_run_approval, v_run_contract, v_run_bars,
           v_run_session_count, v_run_input_kind, v_run_capability, v_run_audience,
           v_run_vendor_snapshot, v_run_strict_pit, v_run_action_count, v_run_error_code,
           v_engine_json, v_published_attempt_id
      FROM public.owner_beta_simulation_runs AS run
     WHERE run.job_id = v_job_id AND run.owner_user_id = v_owner
     FOR UPDATE OF run;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'owner beta simulation run is missing'
            USING ERRCODE = '23514';
    END IF;
    IF v_run_owner IS DISTINCT FROM v_job_owner
       OR NOT public.owner_beta_simulation_payload_matches(
            v_payload, v_run_id, v_run_strategy_config_id,
            v_run_start, v_run_end, v_run_candidate, v_run_artifact,
            v_run_stage5, v_run_action, v_run_approval, v_run_contract,
            v_run_bars, v_run_strategy_id, v_run_strategy_version,
            v_run_strategy_json, v_run_strategy_sha
       )
       OR v_run_input_kind <> 'owner_beta_historical_price_only_v1'
       OR v_run_capability <> 'PRICE_RETURN_ONLY'
       OR v_run_audience <> 'OWNER_ONLY'
       OR NOT v_run_vendor_snapshot
       OR v_run_strict_pit
       OR v_run_action_count <> 0
       OR v_run_session_count NOT BETWEEN 202 AND 2452
    THEN
        RAISE EXCEPTION 'owner beta simulation run/job binding is invalid'
            USING ERRCODE = '23514';
    END IF;

    IF v_max_attempts <> 3
       OR v_attempt_count NOT BETWEEN 0 AND v_max_attempts
       OR ((v_locked_at IS NULL) <> (v_locked_by IS NULL))
       OR (v_job_status = 'RUNNING'
           AND (v_locked_at IS NULL OR v_locked_by IS NULL OR btrim(v_locked_by) = ''))
       OR (v_job_status IN ('QUEUED', 'SUCCEEDED', 'FAILED')
           AND v_locked_at IS NOT NULL)
    THEN
        RAISE EXCEPTION 'owner beta simulation queue lease or retry bound is invalid'
            USING ERRCODE = '23514';
    END IF;

    IF v_attempt_count = 0 THEN
        IF EXISTS (
            SELECT 1 FROM public.job_attempts AS attempt
             WHERE attempt.job_id = v_job_id
        ) THEN
            RAISE EXCEPTION 'owner beta simulation attempt count has no matching history'
                USING ERRCODE = '23514';
        END IF;
    ELSE
        SELECT attempt.job_id, attempt.attempt_no, attempt.outcome, attempt.claimed_by
          INTO v_attempt_job_id, v_attempt_no, v_attempt_outcome, v_attempt_claimed_by
          FROM public.job_attempts AS attempt
         WHERE attempt.job_id = v_job_id
           AND attempt.attempt_no = v_attempt_count;
        IF NOT FOUND
           OR v_attempt_job_id IS DISTINCT FROM v_job_id
           OR v_attempt_no IS DISTINCT FROM v_attempt_count
           OR (v_job_status = 'RUNNING'
               AND (v_attempt_outcome <> 'RUNNING'
                    OR v_attempt_claimed_by IS DISTINCT FROM v_locked_by))
           OR (v_job_status = 'SUCCEEDED' AND v_attempt_outcome <> 'SUCCEEDED')
           OR (v_job_status IN ('QUEUED', 'FAILED')
               AND v_attempt_outcome NOT IN ('FAILED', 'ORPHANED'))
           OR (v_job_status = 'CANCELED'
               AND v_attempt_outcome NOT IN ('RUNNING', 'FAILED', 'ORPHANED'))
           OR (v_job_status = 'CANCELED' AND v_locked_by IS NOT NULL
               AND v_attempt_claimed_by IS DISTINCT FROM v_locked_by)
        THEN
            RAISE EXCEPTION 'owner beta simulation current attempt binding is invalid'
                USING ERRCODE = '23514';
        END IF;
    END IF;

    SELECT pair.owner_user_id INTO v_pair_owner
      FROM public.owner_beta_simulation_result_pairs AS pair
     WHERE pair.run_id = v_run_id
     FOR UPDATE OF pair;
    v_pair_exists := FOUND;
    IF v_pair_exists AND v_pair_owner IS DISTINCT FROM v_owner THEN
        RAISE EXCEPTION 'owner beta simulation pair owner binding is invalid'
            USING ERRCODE = '23514';
    END IF;

    IF v_pair_exists THEN
        IF v_run_status <> 'SUCCEEDED'
           OR v_job_status <> 'SUCCEEDED'
           OR v_engine_json IS NULL
           OR v_published_attempt_id IS NULL
           OR v_run_error_code IS NOT NULL
        THEN
            RAISE EXCEPTION 'owner beta simulation partial result pair is forbidden'
                USING ERRCODE = '23514';
        END IF;
    ELSE
        IF v_run_status = 'SUCCEEDED' OR v_job_status = 'SUCCEEDED'
           OR v_engine_json IS NOT NULL OR v_published_attempt_id IS NOT NULL
        THEN
            RAISE EXCEPTION 'owner beta simulation success requires a whole pair'
                USING ERRCODE = '23514';
        END IF;
    END IF;

    IF v_job_status IN ('QUEUED', 'RUNNING') AND v_run_status <> 'PENDING' THEN
        RAISE EXCEPTION 'owner beta simulation active job must have a pending run'
            USING ERRCODE = '23514';
    ELSIF v_job_status = 'FAILED' AND v_run_status <> 'FAILED' THEN
        RAISE EXCEPTION 'owner beta simulation failed job mirror is invalid'
            USING ERRCODE = '23514';
    ELSIF v_job_status = 'CANCELED' AND v_run_status NOT IN ('PENDING', 'CANCELED') THEN
        RAISE EXCEPTION 'owner beta simulation canceled job mirror is invalid'
            USING ERRCODE = '23514';
    ELSIF v_job_status = 'SUCCEEDED' AND v_run_status <> 'SUCCEEDED' THEN
        RAISE EXCEPTION 'owner beta simulation success job mirror is invalid'
            USING ERRCODE = '23514';
    END IF;

    IF v_run_status IN ('FAILED', 'CANCELED')
       AND (v_run_error_code IS NULL OR v_pair_exists)
    THEN
        RAISE EXCEPTION 'owner beta simulation terminal failure shape is invalid'
            USING ERRCODE = '23514';
    END IF;

    IF v_run_status = 'SUCCEEDED' THEN
        SELECT attempt.job_id, attempt.attempt_no, attempt.outcome
          INTO v_attempt_job_id, v_attempt_no, v_attempt_outcome
          FROM public.job_attempts AS attempt
         WHERE attempt.id = v_published_attempt_id;
        IF NOT FOUND
           OR v_attempt_job_id IS DISTINCT FROM v_job_id
           OR v_attempt_no IS DISTINCT FROM v_attempt_count
           OR v_attempt_outcome IS DISTINCT FROM 'SUCCEEDED'
        THEN
            RAISE EXCEPTION 'owner beta simulation published attempt binding is invalid'
                USING ERRCODE = '23514';
        END IF;
    END IF;
    IF v_actor_changed THEN
        PERFORM pg_catalog.set_config(
            'app.actor_user_id', COALESCE(v_actor_before, ''), true
        );
    END IF;
    RETURN NULL;
END
$deferred_guard$;
ALTER FUNCTION public.owner_beta_simulation_deferred_guard() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_simulation_deferred_guard()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

CREATE CONSTRAINT TRIGGER owner_beta_simulation_jobs_deferred_guard
    AFTER INSERT OR UPDATE OR DELETE ON public.jobs
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
    EXECUTE FUNCTION public.owner_beta_simulation_deferred_guard();
CREATE CONSTRAINT TRIGGER owner_beta_simulation_runs_deferred_guard
    AFTER INSERT OR UPDATE OR DELETE ON public.owner_beta_simulation_runs
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
    EXECUTE FUNCTION public.owner_beta_simulation_deferred_guard();
CREATE CONSTRAINT TRIGGER owner_beta_simulation_pairs_deferred_guard
    AFTER INSERT OR UPDATE OR DELETE ON public.owner_beta_simulation_result_pairs
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
    EXECUTE FUNCTION public.owner_beta_simulation_deferred_guard();
CREATE CONSTRAINT TRIGGER owner_beta_simulation_attempts_deferred_guard
    AFTER INSERT OR UPDATE OR DELETE ON public.job_attempts
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
    EXECUTE FUNCTION public.owner_beta_simulation_deferred_guard();

CREATE FUNCTION public.lock_owner_beta_simulation_entitlement(
    p_owner_user_id uuid,
    p_required_dates date[]
) RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $entitlement$
DECLARE
    v_expected integer;
    v_covered integer;
BEGIN
    v_expected := pg_catalog.cardinality(p_required_dates);
    IF p_owner_user_id IS NULL
       OR v_expected IS NULL
       OR v_expected < 202
       OR v_expected > 2452
       OR pg_catalog.array_position(p_required_dates, NULL) IS NOT NULL
       OR EXISTS (
            SELECT 1
              FROM pg_catalog.unnest(p_required_dates) WITH ORDINALITY AS d(session_date, n)
             WHERE d.session_date < DATE '2016-08-29'
                OR d.session_date > DATE '2026-08-28'
                OR EXTRACT(ISODOW FROM d.session_date) > 5
       )
       OR EXISTS (
            SELECT 1
              FROM pg_catalog.unnest(p_required_dates) WITH ORDINALITY AS d(session_date, n)
              JOIN pg_catalog.unnest(p_required_dates) WITH ORDINALITY AS previous(session_date, n)
                ON previous.n = d.n - 1
             WHERE previous.session_date >= d.session_date
       )
       OR (SELECT pg_catalog.count(DISTINCT d.session_date)
             FROM pg_catalog.unnest(p_required_dates) AS d(session_date)) <> v_expected
    THEN
        RETURN false;
    END IF;

    PERFORM 1 FROM public.users WHERE id = p_owner_user_id FOR SHARE;
    IF NOT FOUND THEN RETURN false; END IF;

    -- Lock the union of all ACTIVE grants that cover at least one requested
    -- session. Revocation/deletion waits until publication commits.
    PERFORM entitlement.id
      FROM public.data_entitlements AS entitlement
     WHERE entitlement.status = 'ACTIVE'
       AND entitlement.covered_datasets @> '["krx_eod_bars"]'::jsonb
       AND entitlement.covered_uses @> '["backtest"]'::jsonb
       AND EXISTS (
            SELECT 1
              FROM pg_catalog.unnest(p_required_dates) AS required(session_date)
             WHERE entitlement.effective_from <= required.session_date
               AND entitlement.effective_until >= required.session_date
       )
     ORDER BY entitlement.id
     FOR SHARE OF entitlement;

    SELECT pg_catalog.count(DISTINCT required.session_date)::integer
      INTO v_covered
      FROM pg_catalog.unnest(p_required_dates) AS required(session_date)
     WHERE EXISTS (
            SELECT 1
              FROM public.data_entitlements AS entitlement
             WHERE entitlement.status = 'ACTIVE'
               AND entitlement.covered_datasets @> '["krx_eod_bars"]'::jsonb
               AND entitlement.covered_uses @> '["backtest"]'::jsonb
               AND entitlement.effective_from <= required.session_date
               AND entitlement.effective_until >= required.session_date
       );
    RETURN v_covered = v_expected;
END
$entitlement$;
ALTER FUNCTION public.lock_owner_beta_simulation_entitlement(uuid, date[])
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.lock_owner_beta_simulation_entitlement(uuid, date[])
    FROM PUBLIC, app, admin, audit_writer, research_writer, worker;
GRANT EXECUTE ON FUNCTION public.lock_owner_beta_simulation_entitlement(uuid, date[])
    TO worker;
