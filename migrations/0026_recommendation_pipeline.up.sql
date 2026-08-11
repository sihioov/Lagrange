-- 0026: recommendation execution lineage, deterministic scheduled submission,
-- and the minimum worker privileges needed to publish normalized results.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

ALTER TABLE recommendation_runs
    ADD COLUMN job_id uuid REFERENCES jobs (id),
    ADD COLUMN trigger_kind text NOT NULL DEFAULT 'MANUAL',
    ADD COLUMN dataset_version_id uuid REFERENCES dataset_versions (id),
    ADD COLUMN dataset_manifest_sha256 text;

ALTER TABLE recommendation_runs
    ADD CONSTRAINT recommendation_runs_trigger_check
        CHECK (trigger_kind IN ('MANUAL', 'SCHEDULED')),
    ADD CONSTRAINT recommendation_runs_dataset_manifest_sha256_check
        CHECK (
            dataset_manifest_sha256 IS NULL
            OR dataset_manifest_sha256 ~ '^[0-9a-f]{64}$'
        ),
    ADD CONSTRAINT recommendation_runs_scheduled_lineage_check
        CHECK (
            trigger_kind <> 'SCHEDULED'
            OR (
                strategy_config_id IS NOT NULL
                AND dataset_version_id IS NOT NULL
                AND dataset_manifest_sha256 IS NOT NULL
                AND job_id IS NOT NULL
            )
        );

ALTER TABLE account_strategy_bindings
    ADD COLUMN auto_apply_recommendations boolean NOT NULL DEFAULT false;

-- The function exists from 0026 onward, but scheduling stays inactive until
-- 0033 atomically activates this singleton after all supporting indexes exist.
CREATE TABLE recommendation_scheduler_control (
    control_key text PRIMARY KEY CHECK (control_key = 'scheduler'),
    active boolean NOT NULL
);

ALTER TABLE recommendation_scheduler_control OWNER TO migration_owner;
REVOKE ALL ON TABLE recommendation_scheduler_control
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
INSERT INTO recommendation_scheduler_control (control_key, active)
    VALUES ('scheduler', false);

-- The worker must be able to observe an opt-in, never manufacture or revoke
-- one. This tightens 0013's pre-automation grant; the down migration restores
-- that exact legacy privilege set.
REVOKE INSERT, UPDATE, DELETE ON TABLE account_strategy_bindings FROM worker;
GRANT SELECT ON TABLE account_strategy_bindings TO worker;

GRANT SELECT ON TABLE recommendation_runs TO worker;
GRANT UPDATE (status, summary_json) ON TABLE recommendation_runs TO worker;
GRANT SELECT, INSERT ON TABLE recommendation_items, target_portfolios TO worker;

-- Only the migration owner, through the scheduler function below, may create
-- a scheduled identity or rewrite its immutable lineage. App keeps its legacy
-- CRUD grant for manual runs and may still update non-lineage result fields.
CREATE FUNCTION public.recommendation_runs_reject_scheduled_lineage_mutation()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, public
AS $guard$
BEGIN
    IF CURRENT_USER <> 'migration_owner' THEN
        IF TG_OP = 'DELETE' AND OLD.trigger_kind = 'SCHEDULED' THEN
            RAISE EXCEPTION 'scheduled recommendation lineage is migration-owned'
                USING ERRCODE = '42501';
        ELSIF TG_OP = 'INSERT' AND NEW.trigger_kind = 'SCHEDULED' THEN
            RAISE EXCEPTION 'scheduled recommendation lineage is migration-owned'
                USING ERRCODE = '42501';
        ELSIF TG_OP = 'UPDATE'
            AND (
                (OLD.trigger_kind <> 'SCHEDULED' AND NEW.trigger_kind = 'SCHEDULED')
                OR (
                    OLD.trigger_kind = 'SCHEDULED'
                    AND (
                        NEW.owner_user_id IS DISTINCT FROM OLD.owner_user_id
                        OR NEW.strategy_config_id IS DISTINCT FROM OLD.strategy_config_id
                        OR NEW.as_of IS DISTINCT FROM OLD.as_of
                        OR NEW.job_id IS DISTINCT FROM OLD.job_id
                        OR NEW.trigger_kind IS DISTINCT FROM OLD.trigger_kind
                        OR NEW.dataset_version_id IS DISTINCT FROM OLD.dataset_version_id
                        OR NEW.dataset_manifest_sha256 IS DISTINCT FROM OLD.dataset_manifest_sha256
                    )
                )
            )
        THEN
            RAISE EXCEPTION 'scheduled recommendation lineage is migration-owned'
                USING ERRCODE = '42501';
        END IF;
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END
$guard$;

ALTER FUNCTION public.recommendation_runs_reject_scheduled_lineage_mutation()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION
    public.recommendation_runs_reject_scheduled_lineage_mutation()
    FROM PUBLIC;

CREATE TRIGGER recommendation_runs_protect_scheduled_lineage
    BEFORE INSERT OR UPDATE OR DELETE ON recommendation_runs
    FOR EACH ROW
    EXECUTE FUNCTION public.recommendation_runs_reject_scheduled_lineage_mutation();

-- Queue lifecycle fields remain mutable for claims, retries, and settlement;
-- the owner/type/key/payload identity of a scheduled recommendation never is.
CREATE FUNCTION public.jobs_reject_scheduled_recommendation_mutation()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, public
AS $job_guard$
DECLARE
    v_is_scheduled boolean := false;
BEGIN
    IF CURRENT_USER = 'migration_owner' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;

    IF TG_OP = 'INSERT' THEN
        IF NEW.idempotency_key LIKE 'recommendation:scheduled:%' THEN
            RAISE EXCEPTION 'scheduled recommendation jobs are migration-owned'
                USING ERRCODE = '42501';
        END IF;
        RETURN NEW;
    END IF;

    SELECT EXISTS(
        SELECT 1
        FROM public.recommendation_runs AS scheduled_run
        WHERE scheduled_run.job_id = OLD.id
          AND scheduled_run.trigger_kind = 'SCHEDULED'
    )
    INTO v_is_scheduled;

    IF TG_OP = 'UPDATE'
        AND NEW.idempotency_key LIKE 'recommendation:scheduled:%'
        AND NOT v_is_scheduled
    THEN
        RAISE EXCEPTION 'scheduled recommendation job namespace is migration-owned'
            USING ERRCODE = '42501';
    END IF;

    IF v_is_scheduled
        OR (
            OLD.job_type = 'recommendation'
            AND OLD.idempotency_key LIKE 'recommendation:scheduled:%'
        )
    THEN
        IF TG_OP = 'DELETE'
            OR NEW.id IS DISTINCT FROM OLD.id
            OR NEW.owner_user_id IS DISTINCT FROM OLD.owner_user_id
            OR NEW.job_type IS DISTINCT FROM OLD.job_type
            OR NEW.idempotency_key IS DISTINCT FROM OLD.idempotency_key
            OR NEW.payload_json IS DISTINCT FROM OLD.payload_json
        THEN
            RAISE EXCEPTION 'scheduled recommendation job lineage is immutable'
                USING ERRCODE = '42501';
        END IF;
    END IF;

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END
$job_guard$;

ALTER FUNCTION public.jobs_reject_scheduled_recommendation_mutation()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION
    public.jobs_reject_scheduled_recommendation_mutation()
    FROM PUBLIC;

CREATE TRIGGER jobs_protect_scheduled_recommendation_lineage
    BEFORE INSERT OR UPDATE OR DELETE ON jobs
    FOR EACH ROW
    EXECUTE FUNCTION public.jobs_reject_scheduled_recommendation_mutation();

-- Narrow scheduler capability. Callers supply the deterministic key so the
-- Rust scheduler and database can agree on the identity, but the function
-- recomputes and verifies it before any privileged table access.
CREATE FUNCTION public.schedule_recommendation_run(
    p_owner_user_id uuid,
    p_strategy_config_id uuid,
    p_as_of date,
    p_dataset_version_id uuid,
    p_manifest_hash text,
    p_curated_version integer,
    p_idempotency_key text
)
RETURNS TABLE (run_id uuid, job_id uuid)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $function$
DECLARE
    v_identity text;
    v_expected_key text;
    v_run_id uuid;
    v_job_id uuid;
    v_dataset_id text;
    v_dataset_version text;
    v_existing_manifest text;
    v_inserted_run_id uuid;
    v_payload jsonb;
BEGIN
    -- Hold a shared transaction fence for the entire scheduling operation.
    -- 0033.down takes the matching exclusive fence before deactivation, so a
    -- call authorized before REVOKE must re-read inactive state after waiting.
    PERFORM pg_catalog.pg_advisory_xact_lock_shared(1815099521, 33);
    PERFORM 1
    FROM public.recommendation_scheduler_control AS control
    WHERE control.control_key = 'scheduler'
      AND control.active;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'recommendation scheduler is unavailable'
            USING ERRCODE = '55000';
    END IF;

    IF p_owner_user_id IS NULL
        OR p_strategy_config_id IS NULL
        OR p_as_of IS NULL
        OR p_dataset_version_id IS NULL
        OR p_manifest_hash IS NULL
        OR p_curated_version IS NULL
    THEN
        RAISE EXCEPTION 'scheduled recommendation identity must be complete'
            USING ERRCODE = '22023';
    END IF;

    IF p_manifest_hash !~ '^[0-9a-f]{64}$' THEN
        RAISE EXCEPTION 'dataset manifest hash is invalid'
            USING ERRCODE = '22023';
    END IF;

    IF p_curated_version <= 0 THEN
        RAISE EXCEPTION 'curated dataset version must be positive'
            USING ERRCODE = '22023';
    END IF;

    v_identity := pg_catalog.concat_ws(
        '|',
        p_owner_user_id::text,
        p_strategy_config_id::text,
        pg_catalog.to_char(p_as_of, 'YYYY-MM-DD'),
        p_dataset_version_id::text
    );
    v_expected_key := 'recommendation:scheduled:' || pg_catalog.md5(v_identity);
    IF p_idempotency_key IS DISTINCT FROM v_expected_key THEN
        RAISE EXCEPTION 'scheduled recommendation idempotency key is invalid'
            USING ERRCODE = '22023';
    END IF;

    -- recommendation_runs and the authorization inputs use RLS. The function
    -- owner is also subject to FORCE RLS, so bind it to exactly the requested
    -- owner before reading or writing tenant rows.
    PERFORM pg_catalog.set_config('app.actor_user_id', p_owner_user_id::text, true);

    PERFORM 1
    FROM public.user_strategy_configs AS config
    WHERE config.id = p_strategy_config_id
      AND config.owner_user_id = p_owner_user_id
      AND config.is_active
    FOR SHARE OF config;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'active owned strategy configuration is required'
            USING ERRCODE = '42501';
    END IF;

    PERFORM 1
    FROM public.account_strategy_bindings AS binding
    JOIN public.accounts AS account ON account.id = binding.account_id
    WHERE binding.owner_user_id = p_owner_user_id
      AND binding.strategy_config_id = p_strategy_config_id
      AND binding.unbound_at IS NULL
      AND binding.auto_apply_recommendations
      AND account.owner_user_id = p_owner_user_id
      AND account.account_type = 'PAPER'
      AND account.status = 'ACTIVE'
    LIMIT 1
    FOR SHARE OF binding, account;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'active opted-in Paper binding is required'
            USING ERRCODE = '42501';
    END IF;

    SELECT dataset.dataset_id, dataset.version
    INTO v_dataset_id, v_dataset_version
    FROM public.dataset_versions AS dataset
    WHERE dataset.id = p_dataset_version_id
      AND dataset.status IN ('READY', 'WARNING')
      AND dataset.manifest_sha256 = p_manifest_hash
    FOR SHARE OF dataset;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'usable matching dataset version is required'
            USING ERRCODE = '22023';
    END IF;

    -- Serialize all callers for the immutable owner/config/close/dataset
    -- identity. The partial unique index remains authoritative even for writes
    -- that do not enter through this function.
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(v_identity, 0)
    );

    SELECT existing.id, existing.job_id, existing.dataset_manifest_sha256
    INTO v_run_id, v_job_id, v_existing_manifest
    FROM public.recommendation_runs AS existing
    WHERE existing.owner_user_id = p_owner_user_id
      AND existing.strategy_config_id = p_strategy_config_id
      AND existing.as_of = p_as_of
      AND existing.dataset_version_id = p_dataset_version_id
      AND existing.trigger_kind = 'SCHEDULED'
    FOR UPDATE OF existing;

    IF FOUND AND v_existing_manifest IS DISTINCT FROM p_manifest_hash THEN
        RAISE EXCEPTION 'scheduled recommendation identity conflicts with lineage'
            USING ERRCODE = '23514';
    END IF;

    IF v_run_id IS NULL THEN
        v_run_id := pg_catalog.gen_random_uuid();
    END IF;

    v_payload := pg_catalog.jsonb_build_object(
        'run_id', v_run_id,
        'strategy_config_id', p_strategy_config_id,
        'as_of', pg_catalog.to_char(p_as_of, 'YYYY-MM-DD'),
        'dataset', pg_catalog.jsonb_build_object(
            'id', p_dataset_version_id,
            'dataset_id', v_dataset_id,
            'version', v_dataset_version,
            'curated_version', p_curated_version,
            'manifest_sha256', p_manifest_hash
        )
    );

    IF v_job_id IS NOT NULL THEN
        PERFORM 1
        FROM public.jobs AS existing_job
        WHERE existing_job.id = v_job_id
          AND existing_job.owner_user_id = p_owner_user_id
          AND existing_job.job_type = 'recommendation'
          AND existing_job.idempotency_key = v_expected_key
          AND existing_job.payload_json = v_payload;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'scheduled recommendation job lineage is invalid'
                USING ERRCODE = '23514';
        END IF;
        RETURN QUERY SELECT v_run_id, v_job_id;
        RETURN;
    END IF;

    INSERT INTO public.jobs (
        owner_user_id,
        job_type,
        status,
        idempotency_key,
        payload_json
    )
    VALUES (
        p_owner_user_id,
        'recommendation',
        'QUEUED',
        v_expected_key,
        v_payload
    )
    ON CONFLICT (owner_user_id, idempotency_key) DO NOTHING
    RETURNING jobs.id INTO v_job_id;

    IF v_job_id IS NULL THEN
        RAISE EXCEPTION 'job idempotency identity conflicts without a scheduled run'
            USING ERRCODE = '23514';
    END IF;

    INSERT INTO public.recommendation_runs (
        id,
        owner_user_id,
        strategy_config_id,
        as_of,
        status,
        job_id,
        trigger_kind,
        dataset_version_id,
        dataset_manifest_sha256
    )
    VALUES (
        v_run_id,
        p_owner_user_id,
        p_strategy_config_id,
        p_as_of,
        'PENDING',
        v_job_id,
        'SCHEDULED',
        p_dataset_version_id,
        p_manifest_hash
    )
    ON CONFLICT (
        owner_user_id,
        strategy_config_id,
        as_of,
        dataset_version_id
    ) WHERE trigger_kind = 'SCHEDULED'
    DO NOTHING
    RETURNING recommendation_runs.id INTO v_inserted_run_id;

    IF v_inserted_run_id IS NULL THEN
        RAISE EXCEPTION 'scheduled recommendation identity changed concurrently'
            USING ERRCODE = '23514';
    END IF;

    RETURN QUERY SELECT v_run_id, v_job_id;
END
$function$;

ALTER FUNCTION public.schedule_recommendation_run(uuid, uuid, date, uuid, text, integer, text)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION
    public.schedule_recommendation_run(uuid, uuid, date, uuid, text, integer, text)
    FROM PUBLIC;
