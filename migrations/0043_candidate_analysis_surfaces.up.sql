-- 0043: Immutable stock-analysis snapshots, common candidate feeds, and
-- tenant-owned saved screens. No table here is an ETF recommendation table.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

CREATE TABLE public.candidate_scoring_configs (
    version             text PRIMARY KEY,
    canonical_json      text NOT NULL,
    config_json         jsonb NOT NULL,
    content_sha256      text NOT NULL UNIQUE,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT candidate_scoring_version_check
        CHECK (version ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    CONSTRAINT candidate_scoring_json_object_check
        CHECK (jsonb_typeof(config_json) = 'object'),
    CONSTRAINT candidate_scoring_canonical_matches_check
        CHECK (canonical_json::jsonb = config_json),
    CONSTRAINT candidate_scoring_sha256_check
        CHECK (content_sha256 ~ '^[0-9a-f]{64}$')
);

INSERT INTO public.candidate_scoring_configs (
    version,
    canonical_json,
    config_json,
    content_sha256
)
VALUES (
    'candidate-score-v1',
    '{"context_sessions":[5,60],"evidence":{"axis_min_coverage":0.6,"strong_coverage":0.8},"financial_sector_profile":"candidate-financial-v1","min_average_trading_value_20":1000000000,"primary_horizon_sessions":20,"sector_min_members":8,"weights":{"flow":0.35,"fundamental":0.3,"technical":0.35},"winsorize":{"lower":0.01,"upper":0.99}}',
    '{"context_sessions":[5,60],"evidence":{"axis_min_coverage":0.6,"strong_coverage":0.8},"financial_sector_profile":"candidate-financial-v1","min_average_trading_value_20":1000000000,"primary_horizon_sessions":20,"sector_min_members":8,"weights":{"flow":0.35,"fundamental":0.3,"technical":0.35},"winsorize":{"lower":0.01,"upper":0.99}}'::jsonb,
    '1cd70f7a79af85896b015f265bea8ae931bbba29aef12a0b95f32c82ee056377'
);

CREATE TABLE public.stock_analysis_runs (
    id                          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    as_of_date                  date NOT NULL,
    cutoff_at                   timestamptz NOT NULL,
    computation_seq             integer NOT NULL,
    status                      text NOT NULL DEFAULT 'PENDING',
    job_id                      uuid UNIQUE REFERENCES public.jobs(id),
    scoring_config_version      text NOT NULL
        REFERENCES public.candidate_scoring_configs(version),
    scoring_config_sha256       text NOT NULL,
    universe_snapshot_id        uuid NOT NULL
        REFERENCES public.candidate_universe_snapshots(id),
    universe_entitlement_id     uuid NOT NULL REFERENCES public.data_entitlements(id),
    price_dataset_version_id    uuid NOT NULL REFERENCES public.dataset_versions(id),
    price_entitlement_id        uuid NOT NULL REFERENCES public.data_entitlements(id),
    price_curated_version       integer NOT NULL,
    price_manifest_sha256       text NOT NULL,
    status_dataset_version_id   uuid NOT NULL REFERENCES public.dataset_versions(id),
    status_entitlement_id       uuid NOT NULL REFERENCES public.data_entitlements(id),
    status_manifest_sha256      text NOT NULL,
    flow_dataset_version_id     uuid NOT NULL REFERENCES public.dataset_versions(id),
    flow_entitlement_id         uuid NOT NULL REFERENCES public.data_entitlements(id),
    flow_manifest_sha256        text NOT NULL,
    fundamental_dataset_version_id uuid NOT NULL REFERENCES public.dataset_versions(id),
    fundamental_entitlement_id  uuid NOT NULL REFERENCES public.data_entitlements(id),
    fundamental_manifest_sha256 text NOT NULL,
    sector_version_id           uuid NOT NULL
        REFERENCES public.candidate_sector_versions(id),
    sector_entitlement_id       uuid NOT NULL REFERENCES public.data_entitlements(id),
    input_identity_sha256       text NOT NULL UNIQUE,
    summary_json                jsonb NOT NULL DEFAULT '{}'::jsonb,
    error_code                  text,
    error_message               text,
    created_at                  timestamptz NOT NULL DEFAULT clock_timestamp(),
    published_at                timestamptz,
    CONSTRAINT stock_analysis_run_seq_check CHECK (computation_seq > 0),
    CONSTRAINT stock_analysis_run_price_curated_version_check
        CHECK (price_curated_version > 0),
    CONSTRAINT stock_analysis_run_status_check
        CHECK (status IN ('PENDING', 'RUNNING', 'SUCCEEDED', 'FAILED', 'BLOCKED')),
    CONSTRAINT stock_analysis_run_status_time_check
        CHECK (
            (status = 'SUCCEEDED' AND published_at IS NOT NULL)
            OR (status <> 'SUCCEEDED' AND published_at IS NULL)
        ),
    CONSTRAINT stock_analysis_run_summary_object_check
        CHECK (jsonb_typeof(summary_json) = 'object'),
    CONSTRAINT stock_analysis_run_error_check
        CHECK (
            (status IN ('FAILED', 'BLOCKED') AND error_code IS NOT NULL)
            OR (status NOT IN ('FAILED', 'BLOCKED') AND error_code IS NULL AND error_message IS NULL)
        ),
    CONSTRAINT stock_analysis_run_scoring_sha256_check
        CHECK (scoring_config_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT stock_analysis_run_price_sha256_check
        CHECK (price_manifest_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT stock_analysis_run_status_sha256_check
        CHECK (status_manifest_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT stock_analysis_run_flow_sha256_check
        CHECK (flow_manifest_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT stock_analysis_run_fundamental_sha256_check
        CHECK (fundamental_manifest_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT stock_analysis_run_identity_sha256_check
        CHECK (input_identity_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT stock_analysis_run_date_seq_key
        UNIQUE (as_of_date, computation_seq)
);

CREATE INDEX stock_analysis_runs_latest_idx
    ON public.stock_analysis_runs (as_of_date DESC, computation_seq DESC)
    WHERE status = 'SUCCEEDED';

CREATE TABLE public.stock_analysis_snapshots (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id              uuid NOT NULL REFERENCES public.stock_analysis_runs(id) ON DELETE CASCADE,
    instrument_id       text NOT NULL REFERENCES public.instruments(id),
    sector_code         text NOT NULL,
    fundamental_profile text NOT NULL,
    eligible            boolean NOT NULL,
    exclusion_codes     jsonb NOT NULL DEFAULT '[]'::jsonb,
    flow_score          numeric(12, 8),
    fundamental_score   numeric(12, 8),
    technical_score     numeric(12, 8),
    total_score         numeric(12, 8),
    flow_coverage       numeric(7, 6) NOT NULL,
    fundamental_coverage numeric(7, 6) NOT NULL,
    technical_coverage  numeric(7, 6) NOT NULL,
    evidence_strength   text NOT NULL,
    rank                integer,
    normalization_scope text NOT NULL,
    factors_json        jsonb NOT NULL,
    scenarios_json      jsonb NOT NULL,
    provenance_json     jsonb NOT NULL,
    content_sha256      text NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT stock_analysis_snapshot_run_instrument_key UNIQUE (run_id, instrument_id),
    CONSTRAINT stock_analysis_snapshot_identity_key UNIQUE (id, run_id, instrument_id),
    CONSTRAINT stock_analysis_snapshot_sector_check
        CHECK (sector_code ~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'),
    CONSTRAINT stock_analysis_snapshot_profile_check
        CHECK (fundamental_profile ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    CONSTRAINT stock_analysis_snapshot_exclusions_array_check
        CHECK (jsonb_typeof(exclusion_codes) = 'array'),
    CONSTRAINT stock_analysis_snapshot_coverage_check
        CHECK (
            flow_coverage BETWEEN 0 AND 1
            AND fundamental_coverage BETWEEN 0 AND 1
            AND technical_coverage BETWEEN 0 AND 1
        ),
    CONSTRAINT stock_analysis_snapshot_evidence_check
        CHECK (evidence_strength IN ('STRONG', 'MODERATE', 'WEAK')),
    CONSTRAINT stock_analysis_snapshot_rank_check CHECK (rank IS NULL OR rank > 0),
    CONSTRAINT stock_analysis_snapshot_normalization_check
        CHECK (normalization_scope IN ('SECTOR', 'UNIVERSE_FALLBACK', 'UNAVAILABLE')),
    CONSTRAINT stock_analysis_snapshot_json_check
        CHECK (
            jsonb_typeof(factors_json) = 'object'
            AND jsonb_typeof(scenarios_json) = 'object'
            AND jsonb_typeof(provenance_json) = 'object'
            AND jsonb_typeof(scenarios_json -> 'bullish') = 'object'
            AND jsonb_typeof(scenarios_json -> 'neutral') = 'object'
            AND jsonb_typeof(scenarios_json -> 'bearish') = 'object'
            AND NOT scenarios_json ?| ARRAY['probability', 'probabilities', 'target_price', 'expected_return']
        ),
    CONSTRAINT stock_analysis_snapshot_eligibility_check
        CHECK (
            (eligible AND jsonb_array_length(exclusion_codes) = 0 AND total_score IS NOT NULL)
            OR (NOT eligible AND jsonb_array_length(exclusion_codes) > 0 AND rank IS NULL)
        ),
    CONSTRAINT stock_analysis_snapshot_sha256_check
        CHECK (content_sha256 ~ '^[0-9a-f]{64}$')
);

CREATE INDEX stock_analysis_snapshots_screen_idx
    ON public.stock_analysis_snapshots (run_id, eligible, total_score DESC, instrument_id);

CREATE TABLE public.candidate_feed_snapshots (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id              uuid NOT NULL UNIQUE REFERENCES public.stock_analysis_runs(id),
    as_of_date          date NOT NULL,
    computation_seq     integer NOT NULL,
    status              text NOT NULL DEFAULT 'PUBLISHED',
    superseded_by       uuid,
    published_at        timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT candidate_feed_seq_check CHECK (computation_seq > 0),
    CONSTRAINT candidate_feed_status_check CHECK (status IN ('PUBLISHED', 'SUPERSEDED')),
    CONSTRAINT candidate_feed_supersession_check
        CHECK (
            (status = 'PUBLISHED' AND superseded_by IS NULL)
            OR (status = 'SUPERSEDED' AND superseded_by IS NOT NULL)
        ),
    CONSTRAINT candidate_feed_no_self_supersede_check
        CHECK (superseded_by IS NULL OR superseded_by <> id),
    CONSTRAINT candidate_feed_superseded_by_fk
        FOREIGN KEY (superseded_by)
        REFERENCES public.candidate_feed_snapshots(id)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT candidate_feed_date_seq_key UNIQUE (as_of_date, computation_seq),
    CONSTRAINT candidate_feed_identity_key UNIQUE (id, run_id)
);

CREATE UNIQUE INDEX candidate_feed_active_date_uq
    ON public.candidate_feed_snapshots(as_of_date)
    WHERE status = 'PUBLISHED';
CREATE INDEX candidate_feed_latest_idx
    ON public.candidate_feed_snapshots(as_of_date DESC, computation_seq DESC)
    WHERE status = 'PUBLISHED';

CREATE TABLE public.candidate_feed_items (
    feed_id             uuid NOT NULL,
    run_id              uuid NOT NULL,
    stock_analysis_snapshot_id uuid NOT NULL,
    instrument_id       text NOT NULL,
    rank                integer NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (feed_id, rank),
    CONSTRAINT candidate_feed_item_instrument_key UNIQUE (feed_id, instrument_id),
    CONSTRAINT candidate_feed_item_rank_check CHECK (rank BETWEEN 1 AND 5),
    CONSTRAINT candidate_feed_item_feed_run_fk
        FOREIGN KEY (feed_id, run_id)
        REFERENCES public.candidate_feed_snapshots(id, run_id) ON DELETE CASCADE,
    CONSTRAINT candidate_feed_item_snapshot_fk
        FOREIGN KEY (stock_analysis_snapshot_id, run_id, instrument_id)
        REFERENCES public.stock_analysis_snapshots(id, run_id, instrument_id)
);

CREATE TABLE public.screener_saved_screens (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id       uuid NOT NULL REFERENCES public.users(id) ON DELETE CASCADE,
    name                text NOT NULL,
    criteria_schema_version integer NOT NULL DEFAULT 1,
    criteria_json       jsonb NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT screener_saved_screen_name_check CHECK (length(btrim(name)) BETWEEN 1 AND 80),
    CONSTRAINT screener_saved_screen_schema_check CHECK (criteria_schema_version > 0),
    CONSTRAINT screener_saved_screen_criteria_check CHECK (jsonb_typeof(criteria_json) = 'object'),
    CONSTRAINT screener_saved_screen_owner_name_key UNIQUE (owner_user_id, name)
);

CREATE INDEX screener_saved_screens_owner_updated_idx
    ON public.screener_saved_screens(owner_user_id, updated_at DESC, id);

-- Validate every immutable run against exact, usable source pins. This trigger
-- is SECURITY DEFINER so serving roles never need UPDATE privileges merely to
-- validate a dataset row.
CREATE FUNCTION public.stock_analysis_validate_lineage()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $validate$
BEGIN
    IF TG_OP = 'UPDATE'
        AND NEW.as_of_date IS NOT DISTINCT FROM OLD.as_of_date
        AND NEW.cutoff_at IS NOT DISTINCT FROM OLD.cutoff_at
        AND NEW.computation_seq IS NOT DISTINCT FROM OLD.computation_seq
        AND NEW.job_id IS NOT DISTINCT FROM OLD.job_id
        AND NEW.scoring_config_version IS NOT DISTINCT FROM OLD.scoring_config_version
        AND NEW.scoring_config_sha256 IS NOT DISTINCT FROM OLD.scoring_config_sha256
        AND NEW.universe_snapshot_id IS NOT DISTINCT FROM OLD.universe_snapshot_id
        AND NEW.universe_entitlement_id IS NOT DISTINCT FROM OLD.universe_entitlement_id
        AND NEW.price_dataset_version_id IS NOT DISTINCT FROM OLD.price_dataset_version_id
        AND NEW.price_entitlement_id IS NOT DISTINCT FROM OLD.price_entitlement_id
        AND NEW.price_curated_version IS NOT DISTINCT FROM OLD.price_curated_version
        AND NEW.price_manifest_sha256 IS NOT DISTINCT FROM OLD.price_manifest_sha256
        AND NEW.status_dataset_version_id IS NOT DISTINCT FROM OLD.status_dataset_version_id
        AND NEW.status_entitlement_id IS NOT DISTINCT FROM OLD.status_entitlement_id
        AND NEW.status_manifest_sha256 IS NOT DISTINCT FROM OLD.status_manifest_sha256
        AND NEW.flow_dataset_version_id IS NOT DISTINCT FROM OLD.flow_dataset_version_id
        AND NEW.flow_entitlement_id IS NOT DISTINCT FROM OLD.flow_entitlement_id
        AND NEW.flow_manifest_sha256 IS NOT DISTINCT FROM OLD.flow_manifest_sha256
        AND NEW.fundamental_dataset_version_id IS NOT DISTINCT FROM OLD.fundamental_dataset_version_id
        AND NEW.fundamental_entitlement_id IS NOT DISTINCT FROM OLD.fundamental_entitlement_id
        AND NEW.fundamental_manifest_sha256 IS NOT DISTINCT FROM OLD.fundamental_manifest_sha256
        AND NEW.sector_version_id IS NOT DISTINCT FROM OLD.sector_version_id
        AND NEW.sector_entitlement_id IS NOT DISTINCT FROM OLD.sector_entitlement_id
        AND NEW.input_identity_sha256 IS NOT DISTINCT FROM OLD.input_identity_sha256
    THEN
        RETURN NEW;
    END IF;

    PERFORM 1
    FROM public.candidate_scoring_configs AS config
    WHERE config.version = NEW.scoring_config_version
      AND config.content_sha256 = NEW.scoring_config_sha256;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate run scoring configuration mismatch'
            USING ERRCODE = '23514';
    END IF;

    PERFORM 1
    FROM public.candidate_universe_snapshots AS universe
    WHERE universe.id = NEW.universe_snapshot_id
      AND universe.entitlement_id = NEW.universe_entitlement_id
      AND universe.as_of_date <= NEW.as_of_date
      AND universe.available_at <= NEW.cutoff_at
      AND universe.member_count = (
          SELECT count(*)
            FROM public.candidate_universe_members AS member
           WHERE member.universe_snapshot_id = universe.id
             AND member.effective_from <= NEW.as_of_date
             AND (member.effective_until IS NULL
                  OR member.effective_until >= NEW.as_of_date)
      );
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate run universe is unavailable at cutoff'
            USING ERRCODE = '23514';
    END IF;

    PERFORM 1
    FROM public.candidate_sector_versions AS sector
    WHERE sector.id = NEW.sector_version_id
      AND sector.entitlement_id = NEW.sector_entitlement_id
      AND sector.effective_from <= NEW.as_of_date
      AND sector.available_at <= NEW.cutoff_at;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate run sector version is unavailable at cutoff'
            USING ERRCODE = '23514';
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM public.candidate_price_publications AS price
        JOIN public.dataset_versions AS dataset ON dataset.id = price.dataset_version_id
        WHERE dataset.id = NEW.price_dataset_version_id
          AND price.entitlement_id = NEW.price_entitlement_id
          AND price.curated_generation = NEW.price_curated_version
          AND price.first_session <= NEW.as_of_date
          AND price.last_session >= NEW.as_of_date
          AND price.available_at <= NEW.cutoff_at
          AND dataset.manifest_sha256 = NEW.price_manifest_sha256
          AND dataset.status IN ('READY', 'WARNING')
    ) OR NOT EXISTS (
        SELECT 1
        FROM public.candidate_market_status_observations AS status
        JOIN public.dataset_versions AS dataset ON dataset.id = status.dataset_version_id
        WHERE dataset.id = NEW.status_dataset_version_id
          AND status.entitlement_id = NEW.status_entitlement_id
          AND status.trade_date = NEW.as_of_date
          AND status.available_at <= NEW.cutoff_at
          AND dataset.manifest_sha256 = NEW.status_manifest_sha256
          AND dataset.status IN ('READY', 'WARNING')
    ) OR NOT EXISTS (
        SELECT 1
        FROM public.candidate_investor_flows AS flow
        JOIN public.candidate_investor_flow_snapshot_rows AS member
          ON member.flow_observation_id=flow.id
        JOIN public.dataset_versions AS dataset ON dataset.id = member.dataset_version_id
        WHERE dataset.id = NEW.flow_dataset_version_id
          AND member.entitlement_id = NEW.flow_entitlement_id
          AND flow.trade_date = NEW.as_of_date
          AND flow.available_at <= NEW.cutoff_at
          AND dataset.manifest_sha256 = NEW.flow_manifest_sha256
          AND dataset.status IN ('READY', 'WARNING')
    ) OR NOT EXISTS (
        SELECT 1
        FROM public.candidate_fundamental_observations AS fact
        JOIN public.dataset_versions AS dataset ON dataset.id = fact.dataset_version_id
        WHERE dataset.id = NEW.fundamental_dataset_version_id
          AND fact.entitlement_id = NEW.fundamental_entitlement_id
          AND fact.fiscal_period_end <= NEW.as_of_date
          AND fact.available_at <= NEW.cutoff_at
          AND dataset.manifest_sha256 = NEW.fundamental_manifest_sha256
          AND dataset.status IN ('READY', 'WARNING')
    ) THEN
        RAISE EXCEPTION 'candidate run requires usable exact dataset pins'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END
$validate$;

ALTER FUNCTION public.stock_analysis_validate_lineage() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.stock_analysis_validate_lineage() FROM PUBLIC;

CREATE TRIGGER stock_analysis_run_lineage
    BEFORE INSERT OR UPDATE ON public.stock_analysis_runs
    FOR EACH ROW EXECUTE FUNCTION public.stock_analysis_validate_lineage();

CREATE FUNCTION public.candidate_analysis_reject_mutation()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $guard$
BEGIN
    IF CURRENT_USER <> 'migration_owner' THEN
        RAISE EXCEPTION 'candidate analysis publication is migration-owned'
            USING ERRCODE = '42501';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END
$guard$;

ALTER FUNCTION public.candidate_analysis_reject_mutation() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.candidate_analysis_reject_mutation() FROM PUBLIC;

CREATE FUNCTION public.candidate_feed_validate_item_count()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $validate$
DECLARE
    v_feed_id uuid;
    v_count bigint;
BEGIN
    IF TG_TABLE_NAME = 'candidate_feed_snapshots' THEN
        IF TG_OP = 'DELETE' THEN v_feed_id := OLD.id; ELSE v_feed_id := NEW.id; END IF;
    ELSE
        IF TG_OP = 'DELETE' THEN v_feed_id := OLD.feed_id; ELSE v_feed_id := NEW.feed_id; END IF;
    END IF;
    PERFORM 1 FROM public.candidate_feed_snapshots AS feed WHERE feed.id = v_feed_id;
    IF NOT FOUND THEN RETURN NULL; END IF;
    SELECT count(*) INTO v_count
    FROM public.candidate_feed_items AS item
    WHERE item.feed_id = v_feed_id;
    IF v_count <> 5 THEN
        RAISE EXCEPTION 'published candidate feed must contain exactly five items'
            USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END
$validate$;

ALTER FUNCTION public.candidate_feed_validate_item_count() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.candidate_feed_validate_item_count() FROM PUBLIC;

CREATE CONSTRAINT TRIGGER candidate_feed_snapshot_item_count
    AFTER INSERT OR UPDATE ON public.candidate_feed_snapshots
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION public.candidate_feed_validate_item_count();
CREATE CONSTRAINT TRIGGER candidate_feed_items_count
    AFTER INSERT OR UPDATE OR DELETE ON public.candidate_feed_items
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION public.candidate_feed_validate_item_count();

CREATE FUNCTION public.screener_saved_screen_touch_updated_at()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $touch$
BEGIN
    NEW.updated_at := clock_timestamp();
    RETURN NEW;
END
$touch$;

ALTER FUNCTION public.screener_saved_screen_touch_updated_at() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.screener_saved_screen_touch_updated_at() FROM PUBLIC;

CREATE TRIGGER screener_saved_screen_touch
    BEFORE UPDATE ON public.screener_saved_screens
    FOR EACH ROW EXECUTE FUNCTION public.screener_saved_screen_touch_updated_at();

DO $system_tables$
DECLARE
    t text;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'candidate_scoring_configs',
        'stock_analysis_runs',
        'stock_analysis_snapshots',
        'candidate_feed_snapshots',
        'candidate_feed_items'
    ] LOOP
        EXECUTE format(
            'CREATE TRIGGER %I BEFORE INSERT OR UPDATE OR DELETE ON public.%I '
            || 'FOR EACH ROW EXECUTE FUNCTION public.candidate_analysis_reject_mutation()',
            t || '_migration_owned',
            t
        );
        EXECUTE format('ALTER TABLE public.%I OWNER TO migration_owner', t);
        EXECUTE format('ALTER TABLE public.%I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('ALTER TABLE public.%I FORCE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY %I ON public.%I FOR SELECT TO app, worker, admin USING (true)',
            'candidate_analysis_select_' || t,
            t
        );
        EXECUTE format(
            'CREATE POLICY %I ON public.%I FOR ALL TO migration_owner USING (true) WITH CHECK (true)',
            'candidate_analysis_owner_' || t,
            t
        );
        EXECUTE format(
            'REVOKE ALL ON TABLE public.%I FROM PUBLIC, app, worker, admin, audit_writer, research_writer',
            t
        );
        EXECUTE format('GRANT SELECT ON TABLE public.%I TO app, worker, admin', t);
    END LOOP;
END
$system_tables$;

ALTER TABLE public.screener_saved_screens OWNER TO migration_owner;
ALTER TABLE public.screener_saved_screens ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.screener_saved_screens FORCE ROW LEVEL SECURITY;
REVOKE ALL ON TABLE public.screener_saved_screens
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE public.screener_saved_screens TO app;
GRANT SELECT ON TABLE public.screener_saved_screens TO admin;

CREATE POLICY screener_saved_screens_app
    ON public.screener_saved_screens FOR ALL TO app
    USING (
        owner_user_id = NULLIF(current_setting('app.actor_user_id', true), '')::uuid
    )
    WITH CHECK (
        owner_user_id = NULLIF(current_setting('app.actor_user_id', true), '')::uuid
    );
CREATE POLICY screener_saved_screens_owner
    ON public.screener_saved_screens FOR ALL TO migration_owner
    USING (true)
    WITH CHECK (true);
CREATE POLICY screener_saved_screens_admin
    ON public.screener_saved_screens FOR SELECT TO admin USING (true);
