-- 0038: asynchronous recommendation-to-Paper rebalance previews and explicit
-- manual application. Curated prices stay in the Paper worker; the app may
-- request/read a preview and apply only an already-published immutable result.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- A locked, monotonic database serialization point for every Paper cash or
-- position mutation. A preview records the version it observed; apply locks
-- the account row and requires the exact same value.
ALTER TABLE public.accounts
    ADD COLUMN paper_state_version bigint NOT NULL DEFAULT 0,
    ADD CONSTRAINT accounts_paper_state_version_check
        CHECK (paper_state_version >= 0);

CREATE FUNCTION public.bump_paper_account_state_version()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    v_old_account uuid;
    v_new_account uuid;
    v_owner_user_id uuid;
BEGIN
    IF TG_OP <> 'INSERT' THEN
        v_old_account := OLD.account_id;
    END IF;
    IF TG_OP <> 'DELETE' THEN
        v_new_account := NEW.account_id;
    END IF;
    v_owner_user_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.owner_user_id ELSE NEW.owner_user_id END;
    PERFORM pg_catalog.set_config('app.actor_user_id', v_owner_user_id::text, true);

    IF v_old_account IS NOT NULL AND v_old_account IS DISTINCT FROM v_new_account THEN
        UPDATE public.accounts
           SET paper_state_version = paper_state_version + 1
         WHERE id = v_old_account;
    END IF;
    IF v_new_account IS NOT NULL THEN
        UPDATE public.accounts
           SET paper_state_version = paper_state_version + 1
         WHERE id = v_new_account;
    END IF;

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$;

ALTER FUNCTION public.bump_paper_account_state_version() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.bump_paper_account_state_version()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

CREATE TRIGGER cash_ledger_bump_paper_state_version
    AFTER INSERT OR UPDATE OR DELETE ON public.cash_ledger
    FOR EACH ROW EXECUTE FUNCTION public.bump_paper_account_state_version();

CREATE TRIGGER positions_bump_paper_state_version
    AFTER INSERT OR UPDATE OR DELETE ON public.positions
    FOR EACH ROW EXECUTE FUNCTION public.bump_paper_account_state_version();

-- Recommendation-origin targets carry an explicit run and source. Existing
-- targets remain LEGACY; no provenance is inferred from old rows.
ALTER TABLE public.pending_targets
    ADD COLUMN source_kind text NOT NULL DEFAULT 'LEGACY',
    ADD COLUMN recommendation_run_id uuid REFERENCES public.recommendation_runs(id),
    ADD CONSTRAINT pending_targets_source_kind_check CHECK (
        source_kind IN ('LEGACY', 'SCHEDULED_RECOMMENDATION', 'MANUAL_RECOMMENDATION')
    ),
    ADD CONSTRAINT pending_targets_recommendation_source_lineage_check CHECK (
        (source_kind = 'LEGACY' AND recommendation_run_id IS NULL)
        OR (
            source_kind IN ('SCHEDULED_RECOMMENDATION', 'MANUAL_RECOMMENDATION')
            AND recommendation_run_id IS NOT NULL
            AND dataset_version IS NOT NULL
            AND dataset_version_id IS NOT NULL
            AND dataset_manifest_sha256 IS NOT NULL
        )
    );

CREATE INDEX pending_targets_recommendation_run_idx
    ON public.pending_targets (recommendation_run_id)
    WHERE recommendation_run_id IS NOT NULL;

CREATE FUNCTION public.pending_targets_protect_recommendation_origin()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, public
AS $$
BEGIN
    IF CURRENT_USER <> 'migration_owner' THEN
        IF TG_OP = 'INSERT' AND NEW.source_kind <> 'LEGACY' THEN
            RAISE EXCEPTION 'recommendation Paper target origin is migration-owned'
                USING ERRCODE = '42501';
        ELSIF TG_OP = 'DELETE' AND OLD.source_kind <> 'LEGACY' THEN
            RAISE EXCEPTION 'recommendation Paper target origin is migration-owned'
                USING ERRCODE = '42501';
        ELSIF TG_OP = 'UPDATE' AND (
            NEW.source_kind IS DISTINCT FROM OLD.source_kind
            OR NEW.recommendation_run_id IS DISTINCT FROM OLD.recommendation_run_id
            OR (
                OLD.source_kind <> 'LEGACY'
                AND (
                    NEW.account_id IS DISTINCT FROM OLD.account_id
                    OR NEW.owner_user_id IS DISTINCT FROM OLD.owner_user_id
                    OR NEW.strategy_config_id IS DISTINCT FROM OLD.strategy_config_id
                    OR NEW.computed_on IS DISTINCT FROM OLD.computed_on
                    OR NEW.effective_date IS DISTINCT FROM OLD.effective_date
                    OR NEW.targets_json IS DISTINCT FROM OLD.targets_json
                    OR NEW.dataset_version IS DISTINCT FROM OLD.dataset_version
                    OR NEW.dataset_version_id IS DISTINCT FROM OLD.dataset_version_id
                    OR NEW.dataset_manifest_sha256 IS DISTINCT FROM OLD.dataset_manifest_sha256
                )
            )
        ) THEN
            RAISE EXCEPTION 'recommendation Paper target origin is migration-owned'
                USING ERRCODE = '42501';
        END IF;
    END IF;
    IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
END;
$$;

ALTER FUNCTION public.pending_targets_protect_recommendation_origin()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.pending_targets_protect_recommendation_origin()
    FROM PUBLIC;

CREATE TRIGGER pending_targets_protect_recommendation_origin
    BEFORE INSERT OR UPDATE OR DELETE ON public.pending_targets
    FOR EACH ROW EXECUTE FUNCTION public.pending_targets_protect_recommendation_origin();

CREATE TABLE public.paper_rebalance_previews (
    id uuid PRIMARY KEY DEFAULT pg_catalog.gen_random_uuid(),
    owner_user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE CASCADE,
    account_id uuid NOT NULL REFERENCES public.accounts(id) ON DELETE CASCADE,
    recommendation_run_id uuid NOT NULL REFERENCES public.recommendation_runs(id),
    target_portfolio_id uuid NOT NULL REFERENCES public.target_portfolios(id),
    strategy_config_id uuid NOT NULL REFERENCES public.user_strategy_configs(id),
    job_id uuid NOT NULL UNIQUE REFERENCES public.jobs(id),
    status text NOT NULL DEFAULT 'PENDING',
    price_basis text NOT NULL DEFAULT 'RECOMMENDATION_CLOSE',
    price_date date NOT NULL,
    proposed_effective_date date,
    dataset_version_id uuid NOT NULL REFERENCES public.dataset_versions(id),
    dataset_manifest_sha256 text NOT NULL,
    cost_profile_id text,
    cost_profile_version integer,
    account_state_version bigint,
    account_state_sha256 text,
    target_portfolio_sha256 text NOT NULL,
    preview_token text,
    result_json jsonb,
    error_json jsonb,
    pending_target_id uuid REFERENCES public.pending_targets(id),
    created_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    started_at timestamptz,
    completed_at timestamptz,
    applied_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT paper_rebalance_previews_status_check CHECK (
        status IN ('PENDING', 'RUNNING', 'READY', 'FAILED', 'APPLIED')
    ),
    CONSTRAINT paper_rebalance_previews_price_basis_check
        CHECK (price_basis = 'RECOMMENDATION_CLOSE'),
    CONSTRAINT paper_rebalance_previews_dates_check CHECK (
        proposed_effective_date IS NULL OR proposed_effective_date > price_date
    ),
    CONSTRAINT paper_rebalance_previews_manifest_hash_check
        CHECK (dataset_manifest_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT paper_rebalance_previews_account_hash_check CHECK (
        account_state_sha256 IS NULL OR account_state_sha256 ~ '^[0-9a-f]{64}$'
    ),
    CONSTRAINT paper_rebalance_previews_target_hash_check
        CHECK (target_portfolio_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT paper_rebalance_previews_token_check CHECK (
        preview_token IS NULL OR preview_token ~ '^[0-9a-f]{64}$'
    ),
    CONSTRAINT paper_rebalance_previews_result_shape_check CHECK (
        result_json IS NULL OR (
            pg_catalog.jsonb_typeof(result_json) = 'object'
            AND pg_catalog.octet_length(result_json::text) <= 262144
        )
    ),
    CONSTRAINT paper_rebalance_previews_error_shape_check CHECK (
        error_json IS NULL OR (
            pg_catalog.jsonb_typeof(error_json) = 'object'
            AND error_json ? 'code'
            AND error_json ? 'message'
            AND pg_catalog.jsonb_typeof(error_json -> 'code') = 'string'
            AND pg_catalog.jsonb_typeof(error_json -> 'message') = 'string'
            AND pg_catalog.octet_length(error_json::text) <= 4096
        )
    ),
    CONSTRAINT paper_rebalance_previews_lifecycle_check CHECK (
        (
            status IN ('PENDING', 'RUNNING')
            AND result_json IS NULL AND error_json IS NULL
            AND preview_token IS NULL AND pending_target_id IS NULL
            AND completed_at IS NULL AND applied_at IS NULL
        ) OR (
            status = 'READY'
            AND result_json IS NOT NULL AND error_json IS NULL
            AND preview_token IS NOT NULL AND pending_target_id IS NULL
            AND completed_at IS NOT NULL AND applied_at IS NULL
            AND proposed_effective_date IS NOT NULL
            AND cost_profile_id IS NOT NULL AND cost_profile_version IS NOT NULL
            AND account_state_version IS NOT NULL AND account_state_sha256 IS NOT NULL
        ) OR (
            status = 'FAILED'
            AND result_json IS NULL AND error_json IS NOT NULL
            AND preview_token IS NULL AND pending_target_id IS NULL
            AND completed_at IS NOT NULL AND applied_at IS NULL
        ) OR (
            status = 'APPLIED'
            AND result_json IS NOT NULL AND error_json IS NULL
            AND preview_token IS NOT NULL AND pending_target_id IS NOT NULL
            AND completed_at IS NOT NULL AND applied_at IS NOT NULL
            AND proposed_effective_date IS NOT NULL
            AND cost_profile_id IS NOT NULL AND cost_profile_version IS NOT NULL
            AND account_state_version IS NOT NULL AND account_state_sha256 IS NOT NULL
        )
    )
);

CREATE INDEX paper_rebalance_previews_owner_created_idx
    ON public.paper_rebalance_previews (owner_user_id, created_at DESC, id DESC);
CREATE INDEX paper_rebalance_previews_worker_pending_idx
    ON public.paper_rebalance_previews (created_at, id)
    WHERE status IN ('PENDING', 'RUNNING');

ALTER TABLE public.paper_rebalance_previews ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.paper_rebalance_previews FORCE ROW LEVEL SECURITY;

CREATE POLICY paper_rebalance_previews_app_select
    ON public.paper_rebalance_previews FOR SELECT TO app
    USING (owner_user_id = pg_catalog.current_setting('app.actor_user_id', true)::uuid);
CREATE POLICY paper_rebalance_previews_app_insert
    ON public.paper_rebalance_previews FOR INSERT TO app
    WITH CHECK (owner_user_id = pg_catalog.current_setting('app.actor_user_id', true)::uuid);
CREATE POLICY paper_rebalance_previews_owner_all
    ON public.paper_rebalance_previews FOR ALL TO migration_owner
    USING (true)
    WITH CHECK (true);
CREATE POLICY paper_rebalance_previews_worker_select
    ON public.paper_rebalance_previews FOR SELECT TO worker USING (true);
CREATE POLICY paper_rebalance_previews_admin_select
    ON public.paper_rebalance_previews FOR SELECT TO admin USING (true);

REVOKE ALL ON TABLE public.paper_rebalance_previews
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT SELECT ON TABLE public.paper_rebalance_previews TO app, worker, admin;
GRANT INSERT (
    owner_user_id, account_id, recommendation_run_id, target_portfolio_id,
    strategy_config_id, job_id, price_date, dataset_version_id,
    dataset_manifest_sha256, target_portfolio_sha256
) ON public.paper_rebalance_previews TO app;

-- Worker snapshot. The queue payload supplies only preview/job identity; owner,
-- account, target, and dataset identity all come from locked database rows.
CREATE FUNCTION public.snapshot_paper_rebalance_preview(
    p_preview_id uuid,
    p_job_id uuid,
    p_seoul_today date
) RETURNS TABLE (
    owner_user_id uuid,
    account_id uuid,
    recommendation_run_id uuid,
    target_portfolio_id uuid,
    strategy_config_id uuid,
    price_date date,
    proposed_effective_date date,
    dataset_version_id uuid,
    dataset_id text,
    dataset_version text,
    curated_version integer,
    dataset_manifest_sha256 text,
    target_portfolio_sha256 text,
    cost_profile_id text,
    cost_profile_version integer,
    account_state_version bigint,
    cash_balance text,
    positions_json jsonb,
    weights_json jsonb
)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    v_owner_user_id uuid;
    v_preview record;
    v_cash_running text;
    v_cash_replayed text;
    v_positions jsonb;
    v_effective date;
BEGIN
    IF p_preview_id IS NULL OR p_job_id IS NULL OR p_seoul_today IS NULL THEN
        RETURN;
    END IF;

    SELECT preview.owner_user_id
      INTO v_owner_user_id
      FROM public.paper_rebalance_previews AS preview
     WHERE preview.id = p_preview_id;
    IF NOT FOUND THEN RETURN; END IF;
    PERFORM pg_catalog.set_config('app.actor_user_id', v_owner_user_id::text, true);

    SELECT preview.owner_user_id, preview.account_id,
           preview.recommendation_run_id, preview.target_portfolio_id,
           preview.strategy_config_id, preview.price_date,
           preview.dataset_version_id, preview.dataset_manifest_sha256,
           preview.target_portfolio_sha256,
           account.cost_profile_id, account.cost_profile_version,
           account.paper_state_version,
           dataset.dataset_id, dataset.version AS dataset_version,
           (source_job.payload_json #>> '{dataset,curated_version}')::integer
               AS curated_version,
           portfolio.weights_json
      INTO v_preview
      FROM public.paper_rebalance_previews AS preview
      JOIN public.jobs AS job
        ON job.id = preview.job_id
       AND job.id = p_job_id
       AND job.owner_user_id = preview.owner_user_id
       AND job.job_type = 'paper_rebalance_preview'
       AND job.status = 'RUNNING'
      JOIN public.accounts AS account
        ON account.id = preview.account_id
       AND account.owner_user_id = preview.owner_user_id
       AND account.account_type = 'PAPER'
       AND account.status = 'ACTIVE'
      JOIN public.account_strategy_bindings AS binding
        ON binding.account_id = account.id
       AND binding.owner_user_id = account.owner_user_id
       AND binding.strategy_config_id = preview.strategy_config_id
       AND binding.unbound_at IS NULL
      JOIN public.user_strategy_configs AS config
        ON config.id = binding.strategy_config_id
       AND config.owner_user_id = binding.owner_user_id
       AND config.is_active
      JOIN public.recommendation_runs AS run
        ON run.id = preview.recommendation_run_id
       AND run.owner_user_id = preview.owner_user_id
       AND run.strategy_config_id = preview.strategy_config_id
       AND run.status = 'SUCCEEDED'
       AND run.as_of = preview.price_date
       AND run.dataset_version_id = preview.dataset_version_id
       AND run.dataset_manifest_sha256 = preview.dataset_manifest_sha256
      JOIN public.dataset_versions AS dataset
        ON dataset.id = preview.dataset_version_id
       AND dataset.status IN ('READY', 'WARNING')
       AND dataset.manifest_sha256 = preview.dataset_manifest_sha256
      JOIN public.jobs AS source_job
        ON source_job.id = run.job_id
       AND source_job.owner_user_id = run.owner_user_id
       AND source_job.job_type = 'recommendation'
       AND source_job.status = 'SUCCEEDED'
       AND source_job.payload_json #>> '{dataset,id}' = run.dataset_version_id::text
       AND source_job.payload_json #>> '{dataset,dataset_id}' = dataset.dataset_id
       AND source_job.payload_json #>> '{dataset,version}' = dataset.version
       AND source_job.payload_json #>> '{dataset,manifest_sha256}' = run.dataset_manifest_sha256
       AND source_job.payload_json #>> '{dataset,curated_version}' ~ '^[1-9][0-9]{0,9}$'
       AND (source_job.payload_json #>> '{dataset,curated_version}')::numeric <= 2147483647
      JOIN public.target_portfolios AS portfolio
        ON portfolio.id = preview.target_portfolio_id
       AND portfolio.owner_user_id = preview.owner_user_id
       AND portfolio.recommendation_run_id = run.id
       AND portfolio.as_of = run.as_of
     WHERE preview.id = p_preview_id
       AND preview.status IN ('PENDING', 'RUNNING')
     FOR UPDATE OF preview
     FOR SHARE OF account, binding, config, run, source_job, portfolio, dataset;
    IF NOT FOUND THEN RETURN; END IF;

    PERFORM 1
      FROM public.data_entitlements AS entitlement
     WHERE entitlement.status = 'ACTIVE'
       AND entitlement.effective_from <= v_preview.price_date
       AND entitlement.effective_until >= v_preview.price_date
       AND entitlement.covered_datasets
           @> pg_catalog.jsonb_build_array(v_preview.dataset_id)
       AND entitlement.covered_uses @> '["recommendation"]'::jsonb
     LIMIT 1
     FOR SHARE OF entitlement;
    IF NOT FOUND THEN RETURN; END IF;

    SELECT calendar.session_date
      INTO v_effective
      FROM public.trading_calendars AS calendar
     WHERE calendar.exchange = 'KRX'
       AND calendar.session_type = 'TRADING'
       AND calendar.timezone = 'Asia/Seoul'
       AND calendar.session_date > GREATEST(v_preview.price_date, p_seoul_today)
       AND calendar.source_batch_id IS NOT NULL
       AND calendar.content_sha256 IS NOT NULL
       AND calendar.retrieved_at IS NOT NULL
     ORDER BY calendar.session_date
     LIMIT 1
     FOR SHARE OF calendar;
    IF NOT FOUND THEN RETURN; END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(v_preview.account_id::text, 381901)
    );

    SELECT (
             SELECT ledger.balance::text
               FROM public.cash_ledger AS ledger
              WHERE ledger.account_id = v_preview.account_id
                AND ledger.owner_user_id = v_preview.owner_user_id
              ORDER BY ledger.seq DESC LIMIT 1
           ),
           COALESCE(pg_catalog.sum(replay.amount), 0)::text
      INTO v_cash_running, v_cash_replayed
      FROM public.cash_ledger AS replay
     WHERE replay.account_id = v_preview.account_id
       AND replay.owner_user_id = v_preview.owner_user_id;
    IF v_cash_running IS NULL OR v_cash_running IS DISTINCT FROM v_cash_replayed THEN
        RAISE EXCEPTION 'Paper cash ledger is unavailable or inconsistent'
            USING ERRCODE = '23514';
    END IF;

    SELECT COALESCE(
               pg_catalog.jsonb_object_agg(position.instrument_id, position.quantity::text
                                            ORDER BY position.instrument_id),
               '{}'::jsonb
           )
      INTO v_positions
      FROM public.positions AS position
     WHERE position.account_id = v_preview.account_id
       AND position.owner_user_id = v_preview.owner_user_id
       AND position.quantity <> 0;

    UPDATE public.paper_rebalance_previews
       SET status = 'RUNNING', started_at = COALESCE(started_at, pg_catalog.now()),
           updated_at = pg_catalog.now()
     WHERE id = p_preview_id;

    RETURN QUERY SELECT
        v_preview.owner_user_id, v_preview.account_id,
        v_preview.recommendation_run_id, v_preview.target_portfolio_id,
        v_preview.strategy_config_id, v_preview.price_date, v_effective,
        v_preview.dataset_version_id, v_preview.dataset_id,
        v_preview.dataset_version, v_preview.curated_version,
        v_preview.dataset_manifest_sha256,
        v_preview.target_portfolio_sha256, v_preview.cost_profile_id,
        v_preview.cost_profile_version, v_preview.paper_state_version,
        v_cash_running, v_positions, v_preview.weights_json;
END;
$$;

ALTER FUNCTION public.snapshot_paper_rebalance_preview(uuid, uuid, date)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.snapshot_paper_rebalance_preview(uuid, uuid, date)
    FROM PUBLIC, app, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.snapshot_paper_rebalance_preview(uuid, uuid, date)
    TO worker;

CREATE FUNCTION public.publish_paper_rebalance_preview(
    p_preview_id uuid,
    p_job_id uuid,
    p_account_state_version bigint,
    p_account_state_sha256 text,
    p_cost_profile_id text,
    p_cost_profile_version integer,
    p_proposed_effective_date date,
    p_preview_token text,
    p_result_json jsonb
) RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    v_owner_user_id uuid;
    v_preview record;
    v_current_version bigint;
BEGIN
    IF p_preview_id IS NULL OR p_job_id IS NULL OR p_account_state_version IS NULL
       OR p_account_state_sha256 !~ '^[0-9a-f]{64}$'
       OR p_cost_profile_id IS NULL OR p_cost_profile_version IS NULL
       OR p_proposed_effective_date IS NULL
       OR p_preview_token !~ '^[0-9a-f]{64}$'
       OR pg_catalog.jsonb_typeof(p_result_json) <> 'object'
       OR pg_catalog.octet_length(p_result_json::text) > 262144 THEN
        RETURN false;
    END IF;

    SELECT preview.owner_user_id
      INTO v_owner_user_id
      FROM public.paper_rebalance_previews AS preview
     WHERE preview.id = p_preview_id;
    IF NOT FOUND THEN RETURN false; END IF;
    PERFORM pg_catalog.set_config('app.actor_user_id', v_owner_user_id::text, true);

    SELECT preview.account_id, preview.price_date
      INTO v_preview
      FROM public.paper_rebalance_previews AS preview
      JOIN public.jobs AS job
        ON job.id = preview.job_id AND job.id = p_job_id
       AND job.job_type = 'paper_rebalance_preview' AND job.status = 'RUNNING'
     WHERE preview.id = p_preview_id AND preview.status = 'RUNNING'
     FOR UPDATE OF preview, job;
    IF NOT FOUND OR p_proposed_effective_date <= v_preview.price_date THEN
        RETURN false;
    END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(v_preview.account_id::text, 381901)
    );
    SELECT account.paper_state_version
      INTO v_current_version
      FROM public.accounts AS account
     WHERE account.id = v_preview.account_id
     FOR SHARE OF account;
    IF NOT FOUND OR v_current_version IS DISTINCT FROM p_account_state_version THEN
        RETURN false;
    END IF;

    UPDATE public.paper_rebalance_previews
       SET status = 'READY', proposed_effective_date = p_proposed_effective_date,
           cost_profile_id = p_cost_profile_id,
           cost_profile_version = p_cost_profile_version,
           account_state_version = p_account_state_version,
           account_state_sha256 = p_account_state_sha256,
           preview_token = p_preview_token, result_json = p_result_json,
           error_json = NULL, completed_at = pg_catalog.now(),
           updated_at = pg_catalog.now()
     WHERE id = p_preview_id AND status = 'RUNNING';
    RETURN FOUND;
END;
$$;

ALTER FUNCTION public.publish_paper_rebalance_preview(uuid, uuid, bigint, text, text, integer, date, text, jsonb)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.publish_paper_rebalance_preview(uuid, uuid, bigint, text, text, integer, date, text, jsonb)
    FROM PUBLIC, app, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.publish_paper_rebalance_preview(uuid, uuid, bigint, text, text, integer, date, text, jsonb)
    TO worker;

CREATE FUNCTION public.fail_paper_rebalance_preview(
    p_preview_id uuid,
    p_job_id uuid,
    p_error_json jsonb
) RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    v_owner_user_id uuid;
BEGIN
    IF pg_catalog.jsonb_typeof(p_error_json) <> 'object'
       OR NOT (p_error_json ? 'code') OR NOT (p_error_json ? 'message')
       OR pg_catalog.jsonb_typeof(p_error_json -> 'code') <> 'string'
       OR pg_catalog.jsonb_typeof(p_error_json -> 'message') <> 'string'
       OR pg_catalog.octet_length(p_error_json::text) > 4096 THEN
        RETURN false;
    END IF;
    SELECT preview.owner_user_id
      INTO v_owner_user_id
      FROM public.paper_rebalance_previews AS preview
     WHERE preview.id = p_preview_id;
    IF NOT FOUND THEN RETURN false; END IF;
    PERFORM pg_catalog.set_config('app.actor_user_id', v_owner_user_id::text, true);
    UPDATE public.paper_rebalance_previews AS preview
       SET status = 'FAILED', error_json = p_error_json,
           completed_at = pg_catalog.now(), updated_at = pg_catalog.now()
      FROM public.jobs AS job
     WHERE preview.id = p_preview_id
       AND preview.job_id = p_job_id
       AND job.id = preview.job_id
       AND job.job_type = 'paper_rebalance_preview'
       AND preview.status IN ('PENDING', 'RUNNING');
    RETURN FOUND;
END;
$$;

ALTER FUNCTION public.fail_paper_rebalance_preview(uuid, uuid, jsonb)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.fail_paper_rebalance_preview(uuid, uuid, jsonb)
    FROM PUBLIC, app, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.fail_paper_rebalance_preview(uuid, uuid, jsonb)
    TO worker;

-- The new scheduled bridge carries exact run identity. The 0037 overload stays
-- installed for reversible rollback but is no longer callable by worker.
REVOKE EXECUTE ON FUNCTION
    public.queue_scheduled_paper_targets(uuid, uuid, date, uuid, text, text, jsonb)
    FROM worker;

CREATE FUNCTION public.queue_scheduled_paper_targets(
    p_recommendation_run_id uuid,
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
    IF p_recommendation_run_id IS NULL OR p_owner_user_id IS NULL
       OR p_strategy_config_id IS NULL OR p_as_of IS NULL
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

    PERFORM pg_catalog.set_config('app.actor_user_id', p_owner_user_id::text, true);

    PERFORM 1
      FROM public.recommendation_runs AS run
     WHERE run.id = p_recommendation_run_id
       AND run.owner_user_id = p_owner_user_id
       AND run.strategy_config_id = p_strategy_config_id
       AND run.as_of = p_as_of
       AND run.trigger_kind = 'SCHEDULED'
       AND run.status = 'PENDING'
       AND run.dataset_version_id = p_dataset_version_id
       AND run.dataset_manifest_sha256 = p_dataset_manifest_sha256
     FOR SHARE OF run;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'scheduled Paper target run lineage is unavailable'
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
            dataset_version_id, dataset_manifest_sha256,
            source_kind, recommendation_run_id
        ) VALUES (
            v_binding.account_id, p_owner_user_id, p_strategy_config_id, p_as_of,
            v_effective_date, p_targets_json, p_dataset_version,
            p_dataset_version_id, p_dataset_manifest_sha256,
            'SCHEDULED_RECOMMENDATION', p_recommendation_run_id
        )
        ON CONFLICT (account_id, effective_date) DO NOTHING
        RETURNING id INTO v_inserted_id;

        IF v_inserted_id IS NULL THEN
            SELECT target.strategy_config_id, target.computed_on,
                   target.targets_json, target.dataset_version,
                   target.dataset_version_id, target.dataset_manifest_sha256,
                   target.source_kind, target.recommendation_run_id
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
               OR v_existing.source_kind IS DISTINCT FROM 'SCHEDULED_RECOMMENDATION'
               OR v_existing.recommendation_run_id IS DISTINCT FROM p_recommendation_run_id
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

ALTER FUNCTION public.queue_scheduled_paper_targets(uuid, uuid, uuid, date, uuid, text, text, jsonb)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.queue_scheduled_paper_targets(uuid, uuid, uuid, date, uuid, text, text, jsonb)
    FROM PUBLIC, app, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.queue_scheduled_paper_targets(uuid, uuid, uuid, date, uuid, text, text, jsonb)
    TO worker;

-- Manual targets require an active exact binding but not the automatic consent
-- flag. Scheduled and legacy recommendation targets retain the 0037 opt-in.
CREATE OR REPLACE FUNCTION public.preflight_paper_target(
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
           target.dataset_manifest_sha256, target.status, target.source_kind
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

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(v_target.account_id::text, 381901)
    );

    PERFORM 1
      FROM public.accounts AS account
      JOIN public.account_strategy_bindings AS binding
        ON binding.account_id = account.id
       AND binding.owner_user_id = account.owner_user_id
       AND binding.strategy_config_id = v_target.strategy_config_id
       AND binding.unbound_at IS NULL
       AND (
            v_target.source_kind = 'MANUAL_RECOMMENDATION'
            OR binding.auto_apply_recommendations
       )
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
            'message', 'Active permitted Paper binding is no longer available'
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

-- App-only consent boundary. Domain failures are returned as outcomes so the
-- HTTP layer never branches on database error text.
CREATE FUNCTION public.apply_paper_rebalance_preview(
    p_owner_user_id uuid,
    p_preview_id uuid,
    p_preview_token text,
    p_seoul_today date
) RETURNS TABLE (
    outcome text,
    pending_target_id uuid,
    effective_date date,
    source_kind text
)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    v_preview record;
    v_account record;
    v_dataset record;
    v_targets jsonb;
    v_existing record;
    v_target_id uuid;
BEGIN
    IF p_owner_user_id IS NULL OR p_preview_id IS NULL OR p_seoul_today IS NULL THEN
        RETURN QUERY SELECT 'NOT_FOUND', NULL::uuid, NULL::date, NULL::text;
        RETURN;
    END IF;

    PERFORM pg_catalog.set_config('app.actor_user_id', p_owner_user_id::text, true);
    SELECT preview.*, portfolio.weights_json
      INTO v_preview
      FROM public.paper_rebalance_previews AS preview
      JOIN public.target_portfolios AS portfolio
        ON portfolio.id = preview.target_portfolio_id
       AND portfolio.owner_user_id = preview.owner_user_id
       AND portfolio.recommendation_run_id = preview.recommendation_run_id
     WHERE preview.id = p_preview_id
       AND preview.owner_user_id = p_owner_user_id
     FOR UPDATE OF preview
     FOR SHARE OF portfolio;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'NOT_FOUND', NULL::uuid, NULL::date, NULL::text;
        RETURN;
    END IF;
    IF v_preview.status = 'APPLIED' THEN
        RETURN QUERY SELECT 'REPLAY', v_preview.pending_target_id,
            v_preview.proposed_effective_date, 'MANUAL_RECOMMENDATION';
        RETURN;
    END IF;
    IF v_preview.status <> 'READY' THEN
        RETURN QUERY SELECT 'NOT_READY', NULL::uuid, NULL::date, NULL::text;
        RETURN;
    END IF;
    IF p_preview_token IS NULL OR p_preview_token IS DISTINCT FROM v_preview.preview_token THEN
        RETURN QUERY SELECT 'STALE', NULL::uuid, NULL::date, NULL::text;
        RETURN;
    END IF;
    IF v_preview.proposed_effective_date IS NULL
       OR v_preview.proposed_effective_date <= p_seoul_today THEN
        RETURN QUERY SELECT 'STALE', NULL::uuid, NULL::date, NULL::text;
        RETURN;
    END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(v_preview.account_id::text, 381901)
    );
    SELECT account.paper_state_version, account.status, account.account_type,
           account.cost_profile_id, account.cost_profile_version
      INTO v_account
      FROM public.accounts AS account
     WHERE account.id = v_preview.account_id
       AND account.owner_user_id = p_owner_user_id
     FOR UPDATE OF account;
    IF NOT FOUND OR v_account.status <> 'ACTIVE' OR v_account.account_type <> 'PAPER'
       OR v_account.paper_state_version IS DISTINCT FROM v_preview.account_state_version
       OR v_account.cost_profile_id IS DISTINCT FROM v_preview.cost_profile_id
       OR v_account.cost_profile_version IS DISTINCT FROM v_preview.cost_profile_version THEN
        RETURN QUERY SELECT 'STALE', NULL::uuid, NULL::date, NULL::text;
        RETURN;
    END IF;

    PERFORM 1
      FROM public.account_strategy_bindings AS binding
      JOIN public.user_strategy_configs AS config
        ON config.id = binding.strategy_config_id
       AND config.owner_user_id = binding.owner_user_id
       AND config.is_active
     WHERE binding.account_id = v_preview.account_id
       AND binding.owner_user_id = p_owner_user_id
       AND binding.strategy_config_id = v_preview.strategy_config_id
       AND binding.unbound_at IS NULL
     FOR SHARE OF binding, config;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'STALE', NULL::uuid, NULL::date, NULL::text;
        RETURN;
    END IF;

    PERFORM 1
      FROM public.recommendation_runs AS run
     WHERE run.id = v_preview.recommendation_run_id
       AND run.owner_user_id = p_owner_user_id
       AND run.strategy_config_id = v_preview.strategy_config_id
       AND run.status = 'SUCCEEDED'
       AND run.as_of = v_preview.price_date
       AND run.dataset_version_id = v_preview.dataset_version_id
       AND run.dataset_manifest_sha256 = v_preview.dataset_manifest_sha256
     FOR SHARE OF run;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'STALE', NULL::uuid, NULL::date, NULL::text;
        RETURN;
    END IF;

    SELECT dataset.dataset_id, dataset.version
      INTO v_dataset
      FROM public.dataset_versions AS dataset
     WHERE dataset.id = v_preview.dataset_version_id
       AND dataset.status IN ('READY', 'WARNING')
       AND dataset.manifest_sha256 = v_preview.dataset_manifest_sha256
     FOR SHARE OF dataset;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'STALE', NULL::uuid, NULL::date, NULL::text;
        RETURN;
    END IF;

    PERFORM 1
      FROM public.data_entitlements AS entitlement
     WHERE entitlement.status = 'ACTIVE'
       AND entitlement.effective_from <= v_preview.proposed_effective_date
       AND entitlement.effective_until >= v_preview.proposed_effective_date
       AND entitlement.covered_datasets @> pg_catalog.jsonb_build_array(v_dataset.dataset_id)
       AND entitlement.covered_uses @> '["recommendation"]'::jsonb
     LIMIT 1
     FOR SHARE OF entitlement;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'STALE', NULL::uuid, NULL::date, NULL::text;
        RETURN;
    END IF;

    PERFORM 1
      FROM public.trading_calendars AS calendar
     WHERE calendar.exchange = 'KRX'
       AND calendar.session_date = v_preview.proposed_effective_date
       AND calendar.session_type = 'TRADING'
       AND calendar.timezone = 'Asia/Seoul'
       AND calendar.source_batch_id IS NOT NULL
       AND calendar.content_sha256 IS NOT NULL
       AND calendar.retrieved_at IS NOT NULL
     FOR SHARE OF calendar;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'STALE', NULL::uuid, NULL::date, NULL::text;
        RETURN;
    END IF;

    SELECT pg_catalog.jsonb_agg(
               pg_catalog.jsonb_build_object('instrument_id', weight.key, 'weight', weight.value)
               ORDER BY weight.key
           )
      INTO v_targets
      FROM pg_catalog.jsonb_each_text(v_preview.weights_json) AS weight
     WHERE weight.value ~ '^(0|[1-9][0-9]*)\.[0-9]{6}$'
       AND weight.value::numeric > 0;
    IF v_targets IS NULL OR pg_catalog.jsonb_array_length(v_targets) = 0 THEN
        RETURN QUERY SELECT 'CONFLICT', NULL::uuid, NULL::date, NULL::text;
        RETURN;
    END IF;

    SELECT target.id, target.strategy_config_id, target.computed_on,
           target.targets_json, target.dataset_version,
           target.dataset_version_id, target.dataset_manifest_sha256,
           target.source_kind, target.recommendation_run_id
      INTO v_existing
      FROM public.pending_targets AS target
     WHERE target.account_id = v_preview.account_id
       AND target.effective_date = v_preview.proposed_effective_date
     FOR UPDATE OF target;
    IF FOUND THEN
        IF v_existing.strategy_config_id IS DISTINCT FROM v_preview.strategy_config_id
           OR v_existing.computed_on IS DISTINCT FROM v_preview.price_date
           OR v_existing.targets_json IS DISTINCT FROM v_targets
           OR v_existing.dataset_version IS DISTINCT FROM v_dataset.version
           OR v_existing.dataset_version_id IS DISTINCT FROM v_preview.dataset_version_id
           OR v_existing.dataset_manifest_sha256 IS DISTINCT FROM v_preview.dataset_manifest_sha256
           OR v_existing.source_kind IS DISTINCT FROM 'MANUAL_RECOMMENDATION'
           OR v_existing.recommendation_run_id IS DISTINCT FROM v_preview.recommendation_run_id THEN
            RETURN QUERY SELECT 'CONFLICT', NULL::uuid, NULL::date, NULL::text;
            RETURN;
        END IF;
        v_target_id := v_existing.id;
    ELSE
        INSERT INTO public.pending_targets (
            account_id, owner_user_id, strategy_config_id, computed_on,
            effective_date, targets_json, dataset_version,
            dataset_version_id, dataset_manifest_sha256,
            source_kind, recommendation_run_id
        ) VALUES (
            v_preview.account_id, p_owner_user_id, v_preview.strategy_config_id,
            v_preview.price_date, v_preview.proposed_effective_date, v_targets,
            v_dataset.version, v_preview.dataset_version_id,
            v_preview.dataset_manifest_sha256,
            'MANUAL_RECOMMENDATION', v_preview.recommendation_run_id
        ) RETURNING id INTO v_target_id;
    END IF;

    UPDATE public.paper_rebalance_previews
       SET status = 'APPLIED', pending_target_id = v_target_id,
           applied_at = pg_catalog.now(), updated_at = pg_catalog.now()
     WHERE id = p_preview_id AND owner_user_id = p_owner_user_id
       AND status = 'READY';
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'CONFLICT', NULL::uuid, NULL::date, NULL::text;
        RETURN;
    END IF;

    RETURN QUERY SELECT 'APPLIED', v_target_id,
        v_preview.proposed_effective_date, 'MANUAL_RECOMMENDATION';
END;
$$;

ALTER FUNCTION public.apply_paper_rebalance_preview(uuid, uuid, text, date)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.apply_paper_rebalance_preview(uuid, uuid, text, date)
    FROM PUBLIC, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.apply_paper_rebalance_preview(uuid, uuid, text, date)
    TO app;
