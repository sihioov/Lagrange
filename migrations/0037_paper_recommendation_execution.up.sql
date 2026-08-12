-- 0037: exact recommendation lineage on Paper targets and worker-only lock
-- boundaries for close scheduling and execution preflight.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

ALTER TABLE public.pending_targets
    ADD COLUMN dataset_version_id uuid REFERENCES public.dataset_versions (id),
    ADD COLUMN dataset_manifest_sha256 text,
    ADD COLUMN non_execution_reason jsonb,
    ADD CONSTRAINT pending_targets_dataset_manifest_sha256_check CHECK (
        dataset_manifest_sha256 IS NULL
        OR dataset_manifest_sha256 ~ '^[0-9a-f]{64}$'
    ),
    ADD CONSTRAINT pending_targets_dataset_lineage_all_or_none_check CHECK (
        (dataset_version_id IS NULL AND dataset_manifest_sha256 IS NULL)
        OR (dataset_version_id IS NOT NULL AND dataset_manifest_sha256 IS NOT NULL)
    ),
    ADD CONSTRAINT pending_targets_non_execution_reason_shape_check CHECK (
        non_execution_reason IS NULL
        OR (
            status = 'SKIPPED'
            AND pg_catalog.jsonb_typeof(non_execution_reason) = 'object'
            AND non_execution_reason ? 'code'
            AND non_execution_reason ? 'message'
            AND pg_catalog.jsonb_typeof(non_execution_reason -> 'code') = 'string'
            AND pg_catalog.jsonb_typeof(non_execution_reason -> 'message') = 'string'
        )
    );

-- Scheduled targets may only enter through the validated bridge below. The
-- runner keeps SELECT/UPDATE for due scans and settlement, but cannot forge
-- an arbitrary target or its recommendation lineage.
REVOKE INSERT ON TABLE public.pending_targets FROM worker;

CREATE FUNCTION public.lock_recommendation_schedule_inputs(
    p_as_of date,
    p_dataset_version_id uuid,
    p_dataset_id text,
    p_dataset_version text,
    p_manifest_sha256 text
) RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF p_as_of IS NULL OR p_dataset_version_id IS NULL OR p_dataset_id IS NULL
       OR p_dataset_version IS NULL OR p_manifest_sha256 !~ '^[0-9a-f]{64}$' THEN
        RETURN false;
    END IF;

    PERFORM 1
      FROM public.trading_calendars AS calendar
     WHERE calendar.exchange = 'KRX'
       AND calendar.session_date = p_as_of
       AND calendar.session_type = 'TRADING'
       AND calendar.timezone = 'Asia/Seoul'
       AND calendar.source_batch_id IS NOT NULL
       AND calendar.content_sha256 IS NOT NULL
       AND calendar.retrieved_at IS NOT NULL
     FOR SHARE OF calendar;
    IF NOT FOUND THEN RETURN false; END IF;

    PERFORM 1
      FROM public.data_batches AS batch
     WHERE batch.provider = 'KRX'
       AND batch.market = 'KR'
       AND batch.kind = 'EOD'
       AND batch.batch_date = p_as_of
       AND batch.fetch_mode = 'credentialed'
       AND batch.source_batch_id IS NOT NULL
     LIMIT 1
     FOR SHARE OF batch;
    IF NOT FOUND THEN RETURN false; END IF;

    PERFORM 1
      FROM public.dataset_versions AS dataset
     WHERE dataset.id = p_dataset_version_id
       AND dataset.dataset_id = p_dataset_id
       AND dataset.version = p_dataset_version
       AND dataset.status IN ('READY', 'WARNING')
       AND dataset.manifest_sha256 = p_manifest_sha256
     FOR SHARE OF dataset;
    RETURN FOUND;
END;
$$;

ALTER FUNCTION public.lock_recommendation_schedule_inputs(date, uuid, text, text, text)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.lock_recommendation_schedule_inputs(date, uuid, text, text, text)
    FROM PUBLIC, app, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.lock_recommendation_schedule_inputs(date, uuid, text, text, text)
    TO worker;

CREATE FUNCTION public.lock_recommendation_calendar_coverage(
    p_session_date date
) RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF p_session_date IS NULL THEN
        RETURN false;
    END IF;

    PERFORM 1
      FROM public.trading_calendars AS calendar
     WHERE calendar.exchange = 'KRX'
       AND calendar.session_date = p_session_date
       AND calendar.session_type IN ('TRADING', 'CLOSED')
       AND calendar.timezone = 'Asia/Seoul'
       AND calendar.source_batch_id IS NOT NULL
       AND calendar.content_sha256 IS NOT NULL
       AND calendar.retrieved_at IS NOT NULL
     FOR SHARE OF calendar;
    RETURN FOUND;
END;
$$;

ALTER FUNCTION public.lock_recommendation_calendar_coverage(date)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.lock_recommendation_calendar_coverage(date)
    FROM PUBLIC, app, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.lock_recommendation_calendar_coverage(date)
    TO worker;

CREATE FUNCTION public.queue_scheduled_paper_targets(
    p_owner_user_id uuid,
    p_strategy_config_id uuid,
    p_as_of date,
    p_dataset_version_id uuid,
    p_dataset_version text,
    p_dataset_manifest_sha256 text,
    p_targets_json jsonb
) RETURNS TABLE (targets integer, missing_next_session boolean)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    v_effective_date date;
    v_binding record;
    v_dataset record;
    v_inserted_id uuid;
    v_existing record;
    v_count integer := 0;
BEGIN
    IF p_owner_user_id IS NULL OR p_strategy_config_id IS NULL OR p_as_of IS NULL
       OR p_dataset_version_id IS NULL OR p_dataset_version IS NULL
       OR p_dataset_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR pg_catalog.jsonb_typeof(p_targets_json) <> 'array'
       OR EXISTS (
            SELECT 1 FROM pg_catalog.jsonb_array_elements(p_targets_json) AS item
            WHERE pg_catalog.jsonb_typeof(item) <> 'object'
               OR item ->> 'instrument_id' IS NULL
               OR item ->> 'weight' IS NULL
               OR item ->> 'weight' !~ '^(0|[1-9][0-9]*)\.[0-9]{6}$'
               OR (item ->> 'weight')::numeric <= 0
       ) THEN
        RAISE EXCEPTION 'scheduled Paper target identity is invalid'
            USING ERRCODE = '22023';
    END IF;

    SELECT dataset.dataset_id
      INTO v_dataset
      FROM public.dataset_versions AS dataset
     WHERE dataset.id = p_dataset_version_id
       AND dataset.version = p_dataset_version
       AND dataset.status IN ('READY', 'WARNING')
       AND dataset.manifest_sha256 = p_dataset_manifest_sha256
     FOR SHARE OF dataset;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'scheduled Paper target dataset lineage is unavailable'
            USING ERRCODE = '22023';
    END IF;

    SELECT calendar.session_date INTO v_effective_date
      FROM public.trading_calendars AS calendar
     WHERE calendar.exchange = 'KRX'
       AND calendar.session_type = 'TRADING'
       AND calendar.timezone = 'Asia/Seoul'
       AND calendar.session_date > p_as_of
       AND calendar.source_batch_id IS NOT NULL
       AND calendar.content_sha256 IS NOT NULL
       AND calendar.retrieved_at IS NOT NULL
     ORDER BY calendar.session_date
     LIMIT 1
     FOR SHARE OF calendar;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 0, true;
        RETURN;
    END IF;

    PERFORM pg_catalog.set_config('app.actor_user_id', p_owner_user_id::text, true);
    FOR v_binding IN
        SELECT binding.account_id
          FROM public.account_strategy_bindings AS binding
          JOIN public.accounts AS account
            ON account.id = binding.account_id
           AND account.owner_user_id = binding.owner_user_id
          JOIN public.user_strategy_configs AS config
            ON config.id = binding.strategy_config_id
           AND config.owner_user_id = binding.owner_user_id
         WHERE binding.owner_user_id = p_owner_user_id
           AND binding.strategy_config_id = p_strategy_config_id
           AND binding.unbound_at IS NULL
           AND binding.auto_apply_recommendations
           AND account.account_type = 'PAPER'
           AND account.status = 'ACTIVE'
           AND config.is_active
         ORDER BY binding.account_id
         FOR SHARE OF binding, account, config
    LOOP
        v_inserted_id := NULL;
        INSERT INTO public.pending_targets (
            account_id, owner_user_id, strategy_config_id, computed_on,
            effective_date, targets_json, dataset_version,
            dataset_version_id, dataset_manifest_sha256
        ) VALUES (
            v_binding.account_id, p_owner_user_id, p_strategy_config_id, p_as_of,
            v_effective_date, p_targets_json, p_dataset_version,
            p_dataset_version_id, p_dataset_manifest_sha256
        )
        ON CONFLICT (account_id, effective_date) DO NOTHING
        RETURNING id INTO v_inserted_id;

        IF v_inserted_id IS NULL THEN
            SELECT target.strategy_config_id, target.computed_on,
                   target.targets_json, target.dataset_version,
                   target.dataset_version_id, target.dataset_manifest_sha256
              INTO v_existing
              FROM public.pending_targets AS target
             WHERE target.account_id = v_binding.account_id
               AND target.effective_date = v_effective_date
             FOR UPDATE OF target;
            IF NOT FOUND
               OR v_existing.strategy_config_id IS DISTINCT FROM p_strategy_config_id
               OR v_existing.computed_on IS DISTINCT FROM p_as_of
               OR v_existing.targets_json IS DISTINCT FROM p_targets_json
               OR v_existing.dataset_version IS DISTINCT FROM p_dataset_version
               OR v_existing.dataset_version_id IS DISTINCT FROM p_dataset_version_id
               OR v_existing.dataset_manifest_sha256 IS DISTINCT FROM p_dataset_manifest_sha256
            THEN
                RAISE EXCEPTION 'scheduled Paper target conflicts with existing lineage'
                    USING ERRCODE = '23514';
            END IF;
        END IF;
        v_count := v_count + 1;
    END LOOP;

    RETURN QUERY SELECT v_count, false;
END;
$$;

ALTER FUNCTION public.queue_scheduled_paper_targets(uuid, uuid, date, uuid, text, text, jsonb)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.queue_scheduled_paper_targets(uuid, uuid, date, uuid, text, text, jsonb)
    FROM PUBLIC, app, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.queue_scheduled_paper_targets(uuid, uuid, date, uuid, text, text, jsonb)
    TO worker;

CREATE FUNCTION public.preflight_paper_target(
    p_target_id uuid,
    p_owner_user_id uuid
) RETURNS TABLE (authorized boolean, reason jsonb)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    v_target record;
    v_dataset record;
    v_reason jsonb;
BEGIN
    IF p_target_id IS NULL OR p_owner_user_id IS NULL THEN
        RETURN QUERY SELECT false, pg_catalog.jsonb_build_object(
            'code', 'PAPER_TARGET_INVALID', 'message', 'Paper target identity is incomplete'
        );
        RETURN;
    END IF;

    PERFORM pg_catalog.set_config('app.actor_user_id', p_owner_user_id::text, true);
    SELECT target.account_id, target.strategy_config_id, target.computed_on,
           target.effective_date, target.dataset_version_id,
           target.dataset_manifest_sha256, target.status
      INTO v_target
      FROM public.pending_targets AS target
     WHERE target.id = p_target_id
       AND target.owner_user_id = p_owner_user_id
     FOR UPDATE OF target;
    IF NOT FOUND OR v_target.status <> 'PENDING' THEN
        RETURN QUERY SELECT false, pg_catalog.jsonb_build_object(
            'code', 'PAPER_TARGET_NOT_PENDING', 'message', 'Paper target is not pending'
        );
        RETURN;
    END IF;

    PERFORM 1
      FROM public.accounts AS account
      JOIN public.account_strategy_bindings AS binding
        ON binding.account_id = account.id
       AND binding.owner_user_id = account.owner_user_id
       AND binding.strategy_config_id = v_target.strategy_config_id
       AND binding.unbound_at IS NULL
       AND binding.auto_apply_recommendations
      JOIN public.user_strategy_configs AS config
        ON config.id = binding.strategy_config_id
       AND config.owner_user_id = binding.owner_user_id
       AND config.is_active
     WHERE account.id = v_target.account_id
       AND account.owner_user_id = p_owner_user_id
       AND account.account_type = 'PAPER'
       AND account.status = 'ACTIVE'
     FOR SHARE OF account, binding, config;
    IF NOT FOUND THEN
        v_reason := pg_catalog.jsonb_build_object(
            'code', 'PAPER_BINDING_INACTIVE',
            'message', 'Active opted-in Paper binding is no longer available'
        );
    ELSIF v_target.dataset_version_id IS NULL
       OR v_target.dataset_manifest_sha256 IS NULL THEN
        v_reason := pg_catalog.jsonb_build_object(
            'code', 'PAPER_DATA_LINEAGE_MISSING',
            'message', 'Queued target has no exact dataset lineage'
        );
    ELSE
        SELECT dataset.dataset_id, dataset.version, dataset.status,
               dataset.manifest_sha256
          INTO v_dataset
          FROM public.dataset_versions AS dataset
         WHERE dataset.id = v_target.dataset_version_id
         FOR SHARE OF dataset;
        IF NOT FOUND THEN
            v_reason := pg_catalog.jsonb_build_object(
                'code', 'PAPER_DATASET_MISSING',
                'message', 'Queued recommendation dataset no longer exists'
            );
        ELSIF v_dataset.status NOT IN ('READY', 'WARNING') THEN
            v_reason := pg_catalog.jsonb_build_object(
                'code', 'PAPER_DATASET_BLOCKED',
                'message', 'Queued recommendation dataset is blocked'
            );
        ELSIF v_dataset.manifest_sha256 IS DISTINCT FROM v_target.dataset_manifest_sha256
           OR v_dataset.version IS DISTINCT FROM (
                SELECT target.dataset_version FROM public.pending_targets AS target
                 WHERE target.id = p_target_id
           ) THEN
            v_reason := pg_catalog.jsonb_build_object(
                'code', 'PAPER_DATASET_LINEAGE_CHANGED',
                'message', 'Queued recommendation dataset lineage changed'
            );
        ELSE
            PERFORM 1
              FROM public.data_entitlements AS entitlement
             WHERE entitlement.status = 'ACTIVE'
               AND entitlement.effective_from <= v_target.effective_date
               AND entitlement.effective_until >= v_target.effective_date
               AND entitlement.covered_datasets
                   @> pg_catalog.jsonb_build_array(v_dataset.dataset_id)
               AND entitlement.covered_uses @> '["recommendation"]'::jsonb
             LIMIT 1
             FOR SHARE OF entitlement;
            IF NOT FOUND THEN
                v_reason := pg_catalog.jsonb_build_object(
                    'code', 'PAPER_ENTITLEMENT_INACTIVE',
                    'message', 'Recommendation entitlement is inactive for this session'
                );
            END IF;
        END IF;
    END IF;

    IF v_reason IS NOT NULL THEN
        UPDATE public.pending_targets
           SET status = 'SKIPPED', executed_at = pg_catalog.now(),
               non_execution_reason = v_reason
         WHERE id = p_target_id AND owner_user_id = p_owner_user_id
           AND status = 'PENDING';
        RETURN QUERY SELECT false, v_reason;
        RETURN;
    END IF;

    RETURN QUERY SELECT true, NULL::jsonb;
END;
$$;

ALTER FUNCTION public.preflight_paper_target(uuid, uuid) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.preflight_paper_target(uuid, uuid)
    FROM PUBLIC, app, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.preflight_paper_target(uuid, uuid) TO worker;
