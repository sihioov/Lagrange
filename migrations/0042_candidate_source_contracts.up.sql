-- 0042: Point-in-time source contracts for the stock research-candidate
-- vertical. These tables are shared, system-owned, append-only observations.
-- They do not change the owner-scoped ETF recommendation contract.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- The dedicated publisher must attest the exact curated metadata row before
-- inserting source observations. It receives no dataset-version DML.
GRANT SELECT ON TABLE public.dataset_versions TO research_writer;
CREATE POLICY candidate_dataset_versions_select_research_writer
    ON public.dataset_versions
    FOR SELECT TO research_writer
    USING (true);

CREATE TABLE public.candidate_universe_snapshots (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    index_id            text NOT NULL,
    as_of_date          date NOT NULL,
    dataset_version_id  uuid NOT NULL REFERENCES public.dataset_versions(id),
    manifest_sha256     text NOT NULL,
    provider            text NOT NULL,
    entitlement_id      uuid NOT NULL REFERENCES public.data_entitlements(id),
    entitlement_date    date NOT NULL,
    license_ref         text NOT NULL,
    source_revision     text NOT NULL,
    available_at        timestamptz NOT NULL,
    retrieved_at        timestamptz NOT NULL,
    member_count        integer NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT candidate_universe_manifest_sha256_check
        CHECK (manifest_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT candidate_universe_index_id_check
        CHECK (index_id ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    CONSTRAINT candidate_universe_provider_check
        CHECK (provider ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    CONSTRAINT candidate_universe_license_ref_check
        CHECK (length(btrim(license_ref)) BETWEEN 1 AND 256),
    CONSTRAINT candidate_universe_source_revision_check
        CHECK (length(btrim(source_revision)) BETWEEN 1 AND 128),
    CONSTRAINT candidate_universe_member_count_check
        CHECK (member_count BETWEEN 1 AND 10000),
    CONSTRAINT candidate_universe_time_check
        CHECK (retrieved_at >= available_at),
    CONSTRAINT candidate_universe_identity_key
        UNIQUE (index_id, as_of_date, dataset_version_id)
);

CREATE TABLE public.candidate_universe_members (
    universe_snapshot_id uuid NOT NULL
        REFERENCES public.candidate_universe_snapshots(id) ON DELETE CASCADE,
    instrument_id       text NOT NULL REFERENCES public.instruments(id),
    announced_at        timestamptz NOT NULL,
    effective_from      date NOT NULL,
    effective_until     date,
    available_at        timestamptz NOT NULL,
    source_revision     text NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (universe_snapshot_id, instrument_id),
    CONSTRAINT candidate_universe_member_window_check
        CHECK (effective_until IS NULL OR effective_until >= effective_from),
    CONSTRAINT candidate_universe_member_availability_check
        CHECK (available_at >= announced_at),
    CONSTRAINT candidate_universe_member_source_revision_check
        CHECK (length(btrim(source_revision)) BETWEEN 1 AND 128)
);

CREATE INDEX candidate_universe_members_instrument_idx
    ON public.candidate_universe_members (instrument_id, effective_from);

CREATE TABLE public.candidate_investor_flows (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    instrument_id       text NOT NULL REFERENCES public.instruments(id),
    trade_date          date NOT NULL,
    investor_class      text NOT NULL,
    net_amount          numeric(28, 4) NOT NULL,
    net_volume          numeric(28, 4) NOT NULL,
    currency            text NOT NULL DEFAULT 'KRW',
    volume_unit         text NOT NULL DEFAULT 'SHARE',
    provider            text NOT NULL,
    source_revision     text NOT NULL,
    available_at        timestamptz NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT candidate_investor_flows_class_check
        CHECK (investor_class IN ('FOREIGN', 'INSTITUTION')),
    CONSTRAINT candidate_investor_flows_currency_check
        CHECK (currency ~ '^[A-Z]{3}$'),
    CONSTRAINT candidate_investor_flows_volume_unit_check
        CHECK (volume_unit IN ('SHARE')),
    CONSTRAINT candidate_investor_flows_provider_check
        CHECK (provider ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    CONSTRAINT candidate_investor_flows_source_revision_check
        CHECK (length(btrim(source_revision)) BETWEEN 1 AND 128),
    CONSTRAINT candidate_investor_flows_identity_key
        UNIQUE (instrument_id, trade_date, investor_class, source_revision)
);

CREATE INDEX candidate_investor_flows_pit_idx
    ON public.candidate_investor_flows
    (instrument_id, trade_date, investor_class, available_at DESC);

-- A daily rolling dataset is an immutable set of immutable observations.
-- The same exact historical fact may therefore participate in more than one
-- sealed 60-session snapshot without being copied or reassigned.
CREATE TABLE public.candidate_investor_flow_snapshot_rows (
    dataset_version_id  uuid NOT NULL REFERENCES public.dataset_versions(id),
    flow_observation_id uuid NOT NULL REFERENCES public.candidate_investor_flows(id),
    entitlement_id      uuid NOT NULL REFERENCES public.data_entitlements(id),
    entitlement_date    date NOT NULL,
    license_ref         text NOT NULL,
    retrieved_at        timestamptz NOT NULL,
    manifest_sha256     text NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (dataset_version_id, flow_observation_id),
    CONSTRAINT candidate_flow_snapshot_license_ref_check
        CHECK (length(btrim(license_ref)) BETWEEN 1 AND 256),
    CONSTRAINT candidate_flow_snapshot_manifest_check
        CHECK (manifest_sha256 ~ '^[0-9a-f]{64}$')
);
CREATE INDEX candidate_flow_snapshot_observation_idx
    ON public.candidate_investor_flow_snapshot_rows(flow_observation_id, dataset_version_id);

CREATE TABLE public.candidate_market_status_observations (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    instrument_id       text NOT NULL REFERENCES public.instruments(id),
    trade_date          date NOT NULL,
    suspended           boolean NOT NULL DEFAULT false,
    administrative      boolean NOT NULL DEFAULT false,
    liquidation         boolean NOT NULL DEFAULT false,
    inactive            boolean NOT NULL DEFAULT false,
    disqualifying_audit_opinion boolean NOT NULL DEFAULT false,
    complete_capital_impairment boolean NOT NULL DEFAULT false,
    provider            text NOT NULL,
    entitlement_id      uuid NOT NULL REFERENCES public.data_entitlements(id),
    entitlement_date    date NOT NULL,
    license_ref         text NOT NULL,
    source_revision     text NOT NULL,
    available_at        timestamptz NOT NULL,
    retrieved_at        timestamptz NOT NULL,
    dataset_version_id  uuid NOT NULL REFERENCES public.dataset_versions(id),
    manifest_sha256     text NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT candidate_market_status_provider_check
        CHECK (provider ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    CONSTRAINT candidate_market_status_license_ref_check
        CHECK (length(btrim(license_ref)) BETWEEN 1 AND 256),
    CONSTRAINT candidate_market_status_source_revision_check
        CHECK (length(btrim(source_revision)) BETWEEN 1 AND 128),
    CONSTRAINT candidate_market_status_time_check
        CHECK (retrieved_at >= available_at),
    CONSTRAINT candidate_market_status_manifest_sha256_check
        CHECK (manifest_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT candidate_market_status_identity_key
        UNIQUE (instrument_id, trade_date, source_revision, dataset_version_id)
);

CREATE INDEX candidate_market_status_pit_idx
    ON public.candidate_market_status_observations
    (instrument_id, trade_date DESC, available_at DESC);

CREATE TABLE public.candidate_fundamental_observations (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    instrument_id       text NOT NULL REFERENCES public.instruments(id),
    fiscal_period_start date NOT NULL,
    fiscal_period_end   date NOT NULL,
    period_kind         text NOT NULL,
    statement_scope     text NOT NULL,
    metric              text NOT NULL,
    value               numeric(38, 10) NOT NULL,
    currency            text,
    unit_scale          integer NOT NULL DEFAULT 1,
    audited             boolean,
    disclosed_at        timestamptz NOT NULL,
    available_at        timestamptz NOT NULL,
    retrieved_at        timestamptz NOT NULL,
    provider            text NOT NULL,
    entitlement_id      uuid NOT NULL REFERENCES public.data_entitlements(id),
    entitlement_date    date NOT NULL,
    license_ref         text NOT NULL,
    source_revision     text NOT NULL,
    restates_observation_id uuid
        REFERENCES public.candidate_fundamental_observations(id),
    dataset_version_id  uuid NOT NULL REFERENCES public.dataset_versions(id),
    manifest_sha256     text NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT candidate_fundamentals_period_check
        CHECK (fiscal_period_end >= fiscal_period_start),
    CONSTRAINT candidate_fundamentals_period_kind_check
        CHECK (period_kind IN ('QUARTER', 'HALF', 'NINE_MONTH', 'ANNUAL')),
    CONSTRAINT candidate_fundamentals_scope_check
        CHECK (statement_scope IN ('CONSOLIDATED', 'SEPARATE')),
    CONSTRAINT candidate_fundamentals_metric_check
        CHECK (metric ~ '^[a-z][a-z0-9_]{0,63}$'),
    CONSTRAINT candidate_fundamentals_currency_check
        CHECK (currency IS NULL OR currency ~ '^[A-Z]{3}$'),
    CONSTRAINT candidate_fundamentals_unit_scale_check
        CHECK (unit_scale IN (1, 1000, 1000000, 1000000000)),
    CONSTRAINT candidate_fundamentals_provider_check
        CHECK (provider ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    CONSTRAINT candidate_fundamentals_license_ref_check
        CHECK (length(btrim(license_ref)) BETWEEN 1 AND 256),
    CONSTRAINT candidate_fundamentals_source_revision_check
        CHECK (length(btrim(source_revision)) BETWEEN 1 AND 128),
    CONSTRAINT candidate_fundamentals_time_check
        CHECK (available_at >= disclosed_at AND retrieved_at >= available_at),
    CONSTRAINT candidate_fundamentals_no_self_restatement_check
        CHECK (restates_observation_id IS NULL OR restates_observation_id <> id),
    CONSTRAINT candidate_fundamentals_manifest_sha256_check
        CHECK (manifest_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT candidate_fundamentals_identity_key
        UNIQUE (
            instrument_id,
            fiscal_period_end,
            statement_scope,
            metric,
            disclosed_at,
            source_revision,
            dataset_version_id
        )
);

CREATE INDEX candidate_fundamentals_pit_idx
    ON public.candidate_fundamental_observations
    (instrument_id, metric, fiscal_period_end DESC, available_at DESC);

CREATE TABLE public.candidate_sector_versions (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    taxonomy_id         text NOT NULL,
    taxonomy_version    text NOT NULL,
    effective_from      date NOT NULL,
    available_at        timestamptz NOT NULL,
    retrieved_at        timestamptz NOT NULL,
    provider            text NOT NULL,
    entitlement_id      uuid NOT NULL REFERENCES public.data_entitlements(id),
    entitlement_date    date NOT NULL,
    license_ref         text NOT NULL,
    source_revision     text NOT NULL,
    dataset_version_id  uuid NOT NULL REFERENCES public.dataset_versions(id),
    manifest_sha256     text NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT candidate_sector_taxonomy_id_check
        CHECK (taxonomy_id ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    CONSTRAINT candidate_sector_taxonomy_version_check
        CHECK (length(btrim(taxonomy_version)) BETWEEN 1 AND 128),
    CONSTRAINT candidate_sector_provider_check
        CHECK (provider ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    CONSTRAINT candidate_sector_license_ref_check
        CHECK (length(btrim(license_ref)) BETWEEN 1 AND 256),
    CONSTRAINT candidate_sector_source_revision_check
        CHECK (length(btrim(source_revision)) BETWEEN 1 AND 128),
    CONSTRAINT candidate_sector_time_check
        CHECK (retrieved_at >= available_at),
    CONSTRAINT candidate_sector_manifest_sha256_check
        CHECK (manifest_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT candidate_sector_version_key
        UNIQUE (taxonomy_id, taxonomy_version, effective_from, source_revision,
                dataset_version_id)
);

CREATE TABLE public.candidate_sector_entries (
    sector_version_id   uuid NOT NULL
        REFERENCES public.candidate_sector_versions(id) ON DELETE CASCADE,
    instrument_id       text NOT NULL REFERENCES public.instruments(id),
    sector_code         text NOT NULL,
    sector_name         text NOT NULL,
    fundamental_profile text NOT NULL DEFAULT 'NON_FINANCIAL',
    effective_from      date NOT NULL,
    effective_until     date,
    available_at        timestamptz NOT NULL,
    source_revision     text NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (sector_version_id, instrument_id),
    CONSTRAINT candidate_sector_code_check
        CHECK (sector_code ~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'),
    CONSTRAINT candidate_sector_name_check
        CHECK (length(btrim(sector_name)) BETWEEN 1 AND 128),
    CONSTRAINT candidate_sector_profile_check
        CHECK (fundamental_profile IN ('NON_FINANCIAL', 'FINANCIAL', 'UNSUPPORTED')),
    CONSTRAINT candidate_sector_window_check
        CHECK (effective_until IS NULL OR effective_until >= effective_from),
    CONSTRAINT candidate_sector_entry_source_revision_check
        CHECK (length(btrim(source_revision)) BETWEEN 1 AND 128)
);

CREATE INDEX candidate_sector_entries_instrument_idx
    ON public.candidate_sector_entries (instrument_id, effective_from);

-- Exact price-lineage readiness attestation. The price curation boundary owns
-- the immutable dataset manifest; this row proves one curated generation
-- covers a closed-session range under one exact candidate-use entitlement.
CREATE TABLE public.candidate_price_publications (
    dataset_version_id  uuid PRIMARY KEY REFERENCES public.dataset_versions(id),
    dataset_version     text NOT NULL,
    manifest_sha256     text NOT NULL,
    market              text NOT NULL,
    curated_generation  bigint NOT NULL,
    first_session       date NOT NULL,
    last_session        date NOT NULL,
    provider            text NOT NULL,
    entitlement_id      uuid NOT NULL REFERENCES public.data_entitlements(id),
    license_ref         text NOT NULL,
    source_revision     text NOT NULL,
    available_at        timestamptz NOT NULL,
    retrieved_at        timestamptz NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT candidate_price_dataset_version_check
        CHECK (length(btrim(dataset_version)) BETWEEN 1 AND 128),
    CONSTRAINT candidate_price_manifest_sha256_check
        CHECK (manifest_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT candidate_price_market_check CHECK (market = 'kr'),
    CONSTRAINT candidate_price_generation_check CHECK (curated_generation > 0),
    CONSTRAINT candidate_price_session_window_check CHECK (last_session >= first_session),
    CONSTRAINT candidate_price_provider_check
        CHECK (provider ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    CONSTRAINT candidate_price_license_ref_check
        CHECK (length(btrim(license_ref)) BETWEEN 1 AND 256),
    CONSTRAINT candidate_price_source_revision_check
        CHECK (length(btrim(source_revision)) BETWEEN 1 AND 128),
    CONSTRAINT candidate_price_time_check CHECK (retrieved_at >= available_at)
);

CREATE INDEX candidate_price_publications_sessions_idx
    ON public.candidate_price_publications (last_session DESC, available_at DESC);

CREATE TABLE public.candidate_price_instrument_coverage (
    dataset_version_id  uuid NOT NULL
        REFERENCES public.candidate_price_publications(dataset_version_id) ON DELETE RESTRICT,
    instrument_id       text NOT NULL REFERENCES public.instruments(id),
    first_session       date NOT NULL,
    last_session        date NOT NULL,
    session_count       integer NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (dataset_version_id, instrument_id),
    CONSTRAINT candidate_price_coverage_window_check
        CHECK (last_session >= first_session),
    CONSTRAINT candidate_price_coverage_count_check
        CHECK (session_count > 0)
);

CREATE INDEX candidate_price_coverage_as_of_idx
    ON public.candidate_price_instrument_coverage
    (dataset_version_id, last_session, session_count);

-- One row per validated bar session. The aggregate above is useful for
-- diagnostics, while scheduling anti-joins these exact rows against the last
-- required KRX sessions so future dates or holes cannot inflate readiness.
CREATE TABLE public.candidate_price_instrument_sessions (
    dataset_version_id  uuid NOT NULL,
    instrument_id       text NOT NULL,
    session_date        date NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (dataset_version_id, instrument_id, session_date),
    FOREIGN KEY (dataset_version_id, instrument_id)
        REFERENCES public.candidate_price_instrument_coverage(
            dataset_version_id, instrument_id
        ) ON DELETE RESTRICT
);

CREATE INDEX candidate_price_sessions_date_idx
    ON public.candidate_price_instrument_sessions
    (dataset_version_id, session_date, instrument_id);

-- Durable Raw publication state. Recovery consults this exact immutable
-- identity before re-evaluating current rights, so revoking an old contract
-- cannot wedge later deliveries. CATALOGED is never readiness; only the same
-- transaction that publishes typed rows may seal a batch as PUBLISHED.
CREATE TABLE public.candidate_raw_batch_publications (
    batch_id             uuid NOT NULL,
    surface              text NOT NULL,
    raw_manifest_sha256  text NOT NULL,
    fetch_mode           text NOT NULL,
    entitlement_reference text NOT NULL,
    entitlement_date     date NOT NULL,
    state                text NOT NULL,
    reason_code          text,
    cataloged_at         timestamptz NOT NULL DEFAULT clock_timestamp(),
    published_at         timestamptz,
    PRIMARY KEY (batch_id, surface),
    CONSTRAINT candidate_raw_surface_check CHECK (surface IN ('source','price')),
    CONSTRAINT candidate_raw_manifest_check CHECK (raw_manifest_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT candidate_raw_fetch_mode_check CHECK (fetch_mode IN ('credentialed','synthetic')),
    CONSTRAINT candidate_raw_state_check CHECK (state IN ('CATALOGED','PUBLISHED','BLOCKED')),
    CONSTRAINT candidate_raw_reason_check CHECK (
        (state='BLOCKED') = (reason_code IS NOT NULL)
        AND (state='PUBLISHED') = (published_at IS NOT NULL)
    )
);

CREATE TABLE public.candidate_raw_batch_datasets (
    batch_id             uuid NOT NULL,
    surface              text NOT NULL,
    response_kind        text NOT NULL,
    dataset_version_id   uuid NOT NULL REFERENCES public.dataset_versions(id),
    reused_existing      boolean NOT NULL,
    created_at           timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (batch_id, surface, response_kind),
    FOREIGN KEY (batch_id, surface)
        REFERENCES public.candidate_raw_batch_publications(batch_id, surface)
        ON DELETE RESTRICT,
    CONSTRAINT candidate_raw_dataset_kind_check CHECK (
        response_kind IN ('bars','investor_flow','market_status','fundamentals',
                          'index_membership','sector_classification'))
);

CREATE UNIQUE INDEX candidate_raw_batch_dataset_exact_idx
    ON public.candidate_raw_batch_datasets
    (batch_id, surface, dataset_version_id, response_kind);
CREATE UNIQUE INDEX candidate_raw_dataset_single_origin_idx
    ON public.candidate_raw_batch_datasets(dataset_version_id)
    WHERE NOT reused_existing;

-- Auditable, append-only proof that a canonical instrument was introduced by
-- one verified Raw reference file under the exact candidate-use contract.
-- The shared instrument row is registered only through the narrow definer
-- below; research_writer never receives direct instruments DML.
CREATE TABLE public.candidate_instrument_registrations (
    instrument_id       text NOT NULL REFERENCES public.instruments(id),
    reference_sha256    text NOT NULL,
    source_revision     text NOT NULL,
    entitlement_id      uuid NOT NULL REFERENCES public.data_entitlements(id),
    license_ref         text NOT NULL,
    entitlement_date    date NOT NULL,
    retrieved_at        timestamptz NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    -- Identical immutable master bytes may be delivered repeatedly under the
    -- same contract, or lawfully reacquired under a renewed contract. Keep
    -- one append-only introduction proof per exact entitlement.
    PRIMARY KEY (instrument_id, reference_sha256, entitlement_id),
    CONSTRAINT candidate_instrument_reference_sha256_check
        CHECK (reference_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT candidate_instrument_source_revision_check
        CHECK (length(btrim(source_revision)) BETWEEN 1 AND 128),
    CONSTRAINT candidate_instrument_license_ref_check
        CHECK (length(btrim(license_ref)) BETWEEN 1 AND 256)
);

CREATE FUNCTION public.candidate_source_entitlement_is_valid(
    p_entitlement_id uuid,
    p_contract_reference text,
    p_dataset_id text,
    p_first_date date,
    p_last_date date
)
RETURNS boolean
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $entitlement$
    SELECT EXISTS (
        SELECT 1
        FROM public.data_entitlements AS entitlement
        WHERE entitlement.id = p_entitlement_id
          AND entitlement.contract_reference = p_contract_reference
          AND entitlement.status = 'ACTIVE'
          AND entitlement.covered_datasets
                @> pg_catalog.jsonb_build_array(p_dataset_id)
          AND entitlement.covered_uses @> '["candidate"]'::jsonb
          AND entitlement.effective_from <= p_first_date
          AND entitlement.effective_until >= p_last_date
    )
$entitlement$;

ALTER FUNCTION public.candidate_source_entitlement_is_valid(uuid, text, text, date, date)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.candidate_source_entitlement_is_valid(uuid, text, text, date, date)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.candidate_source_entitlement_is_valid(uuid, text, text, date, date)
    TO research_writer;

-- Resolve exactly one entitlement carried by the immutable Raw manifest. A
-- contract reference is never silently replaced with the newest grant.
CREATE FUNCTION public.resolve_candidate_contract_entitlement(
    p_contract_reference text,
    p_first_session date,
    p_last_session date
)
RETURNS uuid
LANGUAGE plpgsql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $resolver$
DECLARE
    v_entitlement_id uuid;
    v_count bigint;
BEGIN
    SELECT (array_agg(entitlement.id ORDER BY entitlement.id))[1], count(*)
      INTO v_entitlement_id, v_count
      FROM public.data_entitlements AS entitlement
     WHERE entitlement.contract_reference = p_contract_reference
       AND entitlement.status = 'ACTIVE'
       AND entitlement.covered_uses @> '["candidate"]'::jsonb
       AND entitlement.effective_from <= p_first_session
       AND entitlement.effective_until >= p_last_session
       AND NOT EXISTS (
           SELECT 1
             FROM (VALUES
                 ('krx_eod_bars'::text),
                 ('krx_investor_flows'::text),
                 ('krx_market_status'::text),
                 ('krx_fundamentals'::text),
                 ('krx_kospi200_membership'::text),
                 ('krx_sector_classification'::text)
             ) AS required(dataset_id)
            WHERE NOT entitlement.covered_datasets
                      @> pg_catalog.jsonb_build_array(required.dataset_id)
       );
    IF v_count <> 1 THEN
        RAISE EXCEPTION 'candidate price requires one exact active entitlement'
            USING ERRCODE = '42501';
    END IF;
    RETURN v_entitlement_id;
END
$resolver$;

ALTER FUNCTION public.resolve_candidate_contract_entitlement(text, date, date)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.resolve_candidate_contract_entitlement(text, date, date)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.resolve_candidate_contract_entitlement(text, date, date)
    TO research_writer;

CREATE FUNCTION public.begin_candidate_raw_batch(
    p_batch_id uuid, p_surface text, p_raw_manifest_sha256 text,
    p_fetch_mode text, p_entitlement_reference text, p_entitlement_date date
)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $raw_begin$
BEGIN
    IF p_batch_id IS NULL OR p_surface NOT IN ('source','price')
       OR p_raw_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_fetch_mode NOT IN ('credentialed','synthetic')
       OR p_entitlement_reference IS NULL OR btrim(p_entitlement_reference)=''
       OR p_entitlement_date IS NULL
    THEN
        RAISE EXCEPTION 'invalid candidate Raw batch identity' USING ERRCODE='23514';
    END IF;
    INSERT INTO public.candidate_raw_batch_publications
        (batch_id,surface,raw_manifest_sha256,fetch_mode,entitlement_reference,
         entitlement_date,state)
    VALUES
        (p_batch_id,p_surface,p_raw_manifest_sha256,p_fetch_mode,
         p_entitlement_reference,p_entitlement_date,'CATALOGED')
    ON CONFLICT (batch_id,surface) DO NOTHING;
    IF NOT EXISTS (
        SELECT 1 FROM public.candidate_raw_batch_publications AS batch
         WHERE batch.batch_id=p_batch_id AND batch.surface=p_surface
           AND batch.raw_manifest_sha256=p_raw_manifest_sha256
           AND batch.fetch_mode=p_fetch_mode
           AND batch.entitlement_reference=p_entitlement_reference
           AND batch.entitlement_date=p_entitlement_date
           AND batch.state IN ('CATALOGED','PUBLISHED')
    ) THEN
        RAISE EXCEPTION 'candidate Raw batch replay conflicts or is terminally blocked'
            USING ERRCODE='23514';
    END IF;
END
$raw_begin$;

CREATE FUNCTION public.bind_candidate_raw_dataset(
    p_batch_id uuid, p_surface text, p_response_kind text,
    p_dataset_version_id uuid, p_reused_existing boolean
)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $raw_bind$
DECLARE
    v_expected_dataset text;
BEGIN
    v_expected_dataset := CASE p_response_kind
        WHEN 'bars' THEN 'krx_eod_bars'
        WHEN 'investor_flow' THEN 'krx_investor_flows'
        WHEN 'market_status' THEN 'krx_market_status'
        WHEN 'fundamentals' THEN 'krx_fundamentals'
        WHEN 'index_membership' THEN 'krx_kospi200_membership'
        WHEN 'sector_classification' THEN 'krx_sector_classification'
        ELSE NULL END;
    IF v_expected_dataset IS NULL
       OR (p_surface='source' AND p_response_kind='bars')
       OR (p_surface='price' AND p_response_kind<>'bars')
       OR (p_reused_existing AND p_response_kind NOT IN (
            'fundamentals','index_membership','sector_classification'))
       OR p_surface NOT IN ('source','price')
       OR NOT EXISTS (
        SELECT 1 FROM public.candidate_raw_batch_publications AS batch
         WHERE batch.batch_id=p_batch_id AND batch.surface=p_surface
           AND batch.state IN ('CATALOGED','PUBLISHED')
    ) OR NOT EXISTS (
        SELECT 1 FROM public.dataset_versions AS dataset
         WHERE dataset.id=p_dataset_version_id AND dataset.dataset_id=v_expected_dataset
           AND dataset.status IN ('READY','WARNING')
    ) THEN
        RAISE EXCEPTION 'candidate Raw dataset binding is invalid' USING ERRCODE='23514';
    END IF;
    IF EXISTS (
        SELECT 1 FROM public.candidate_raw_batch_publications AS batch
         WHERE batch.batch_id=p_batch_id AND batch.surface=p_surface
           AND batch.state='PUBLISHED'
    ) THEN
        IF EXISTS (
            SELECT 1 FROM public.candidate_raw_batch_datasets AS binding
             WHERE binding.batch_id=p_batch_id AND binding.surface=p_surface
               AND binding.response_kind=p_response_kind
               AND binding.dataset_version_id=p_dataset_version_id
               AND binding.reused_existing=p_reused_existing
        ) THEN
            RETURN;
        END IF;
        RAISE EXCEPTION 'published candidate Raw batch cannot gain another dataset binding'
            USING ERRCODE='23514';
    END IF;
    IF p_reused_existing AND NOT EXISTS (
        SELECT 1
          FROM public.candidate_raw_batch_datasets AS origin
          JOIN public.candidate_raw_batch_publications AS origin_batch
            ON origin_batch.batch_id=origin.batch_id
           AND origin_batch.surface=origin.surface
         WHERE origin.dataset_version_id=p_dataset_version_id
           AND origin.response_kind=p_response_kind
           AND NOT origin.reused_existing
           AND origin_batch.state='PUBLISHED'
    ) THEN
        RAISE EXCEPTION 'candidate reusable dataset has no sealed immutable origin'
            USING ERRCODE='23514';
    END IF;
    IF NOT p_reused_existing AND EXISTS (
        SELECT 1 FROM public.candidate_raw_batch_datasets AS origin
         WHERE origin.dataset_version_id=p_dataset_version_id
           AND NOT origin.reused_existing
           AND (origin.batch_id<>p_batch_id OR origin.surface<>p_surface)
    ) THEN
        RAISE EXCEPTION 'candidate dataset already belongs to another immutable origin'
            USING ERRCODE='23514';
    END IF;
    INSERT INTO public.candidate_raw_batch_datasets
        (batch_id,surface,response_kind,dataset_version_id,reused_existing)
    VALUES
        (p_batch_id,p_surface,p_response_kind,p_dataset_version_id,p_reused_existing)
    ON CONFLICT (batch_id,surface,response_kind) DO NOTHING;
    IF NOT EXISTS (
        SELECT 1 FROM public.candidate_raw_batch_datasets AS binding
         WHERE binding.batch_id=p_batch_id AND binding.surface=p_surface
           AND binding.response_kind=p_response_kind
           AND binding.dataset_version_id=p_dataset_version_id
           AND binding.reused_existing=p_reused_existing
    ) THEN
        RAISE EXCEPTION 'candidate Raw dataset binding replay conflicts' USING ERRCODE='23514';
    END IF;
END
$raw_bind$;

CREATE FUNCTION public.seal_candidate_raw_batch(
    p_batch_id uuid, p_surface text, p_raw_manifest_sha256 text, p_fetch_mode text
)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $raw_seal$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM public.candidate_raw_batch_publications AS batch
         WHERE batch.batch_id=p_batch_id AND batch.surface=p_surface
           AND batch.raw_manifest_sha256=p_raw_manifest_sha256
           AND batch.fetch_mode=p_fetch_mode AND batch.state IN ('CATALOGED','PUBLISHED')
    ) THEN
        RAISE EXCEPTION 'candidate Raw batch cannot be sealed' USING ERRCODE='23514';
    END IF;
    IF p_surface='source' AND (
        (SELECT count(*) FROM public.candidate_raw_batch_datasets AS binding
          WHERE binding.batch_id=p_batch_id AND binding.surface='source') <> 5
        OR NOT EXISTS (
            SELECT 1 FROM public.candidate_raw_batch_datasets AS binding
            JOIN public.candidate_investor_flow_snapshot_rows AS member
              ON member.dataset_version_id=binding.dataset_version_id
            JOIN public.candidate_investor_flows AS flow
              ON flow.id=member.flow_observation_id
            JOIN public.candidate_raw_batch_publications AS batch
              ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
           WHERE binding.batch_id=p_batch_id AND binding.surface='source'
             AND binding.response_kind='investor_flow'
             AND flow.trade_date=batch.entitlement_date)
        OR NOT EXISTS (
            SELECT 1 FROM public.candidate_raw_batch_datasets AS binding
            JOIN public.candidate_market_status_observations AS status
              ON status.dataset_version_id=binding.dataset_version_id
            JOIN public.candidate_raw_batch_publications AS batch
              ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
           WHERE binding.batch_id=p_batch_id AND binding.surface='source'
             AND binding.response_kind='market_status'
             AND status.trade_date=batch.entitlement_date)
        OR NOT EXISTS (
            SELECT 1 FROM public.candidate_raw_batch_datasets AS binding
            JOIN public.candidate_fundamental_observations AS fundamental
              ON fundamental.dataset_version_id=binding.dataset_version_id
           WHERE binding.batch_id=p_batch_id AND binding.surface='source'
             AND binding.response_kind='fundamentals')
        OR NOT EXISTS (
            SELECT 1 FROM public.candidate_raw_batch_datasets AS binding
            JOIN public.candidate_universe_snapshots AS snapshot
              ON snapshot.dataset_version_id=binding.dataset_version_id
           WHERE binding.batch_id=p_batch_id AND binding.surface='source'
             AND binding.response_kind='index_membership'
             AND snapshot.member_count > 0
             AND snapshot.member_count=(
                 SELECT count(*) FROM public.candidate_universe_members AS member
                  WHERE member.universe_snapshot_id=snapshot.id))
        OR NOT EXISTS (
            SELECT 1 FROM public.candidate_raw_batch_datasets AS binding
            JOIN public.candidate_sector_versions AS version
              ON version.dataset_version_id=binding.dataset_version_id
           WHERE binding.batch_id=p_batch_id AND binding.surface='source'
             AND binding.response_kind='sector_classification'
             AND EXISTS (
                 SELECT 1 FROM public.candidate_sector_entries AS entry
                  WHERE entry.sector_version_id=version.id))
        OR EXISTS (
            SELECT 1 FROM public.candidate_raw_batch_datasets AS reused
             WHERE reused.batch_id=p_batch_id AND reused.surface='source'
               AND reused.reused_existing
               AND NOT EXISTS (
                   SELECT 1
                     FROM public.candidate_raw_batch_datasets AS origin
                     JOIN public.candidate_raw_batch_publications AS origin_batch
                       ON origin_batch.batch_id=origin.batch_id
                      AND origin_batch.surface=origin.surface
                    WHERE origin.surface='source'
                      AND origin.response_kind=reused.response_kind
                      AND origin.dataset_version_id=reused.dataset_version_id
                      AND NOT origin.reused_existing
                      AND origin_batch.state='PUBLISHED'
                      AND origin.batch_id<>p_batch_id))
    ) THEN
        RAISE EXCEPTION 'candidate source batch is incomplete and cannot be sealed'
            USING ERRCODE='23514';
    END IF;
    IF p_surface='price' AND NOT EXISTS (
        SELECT 1 FROM public.candidate_raw_batch_datasets AS binding
        JOIN public.candidate_price_publications AS price
          ON price.dataset_version_id=binding.dataset_version_id
       WHERE binding.batch_id=p_batch_id AND binding.surface='price'
         AND binding.response_kind='bars'
    ) THEN
        RAISE EXCEPTION 'candidate price batch is incomplete and cannot be sealed'
            USING ERRCODE='23514';
    END IF;
    UPDATE public.candidate_raw_batch_publications
       SET state='PUBLISHED', published_at=COALESCE(published_at,clock_timestamp())
     WHERE batch_id=p_batch_id AND surface=p_surface AND state='CATALOGED';
END
$raw_seal$;

CREATE FUNCTION public.block_candidate_raw_batch_for_inactive_rights(
    p_batch_id uuid, p_surface text, p_raw_manifest_sha256 text,
    p_fetch_mode text, p_entitlement_reference text, p_entitlement_date date,
    p_first_date date, p_last_date date
)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $raw_block$
BEGIN
    IF p_first_date IS NULL OR p_last_date IS NULL OR p_first_date > p_last_date
       OR p_entitlement_date NOT BETWEEN p_first_date AND p_last_date
    THEN
        RAISE EXCEPTION 'candidate Raw terminal block rights window is invalid'
            USING ERRCODE='23514';
    END IF;
    IF EXISTS (
        SELECT 1 FROM public.data_entitlements AS entitlement
         WHERE entitlement.contract_reference=p_entitlement_reference
           AND entitlement.status='ACTIVE'
           AND entitlement.covered_uses @> '["candidate"]'::jsonb
           AND entitlement.effective_from <= p_first_date
           AND entitlement.effective_until >= p_last_date
           AND NOT EXISTS (
               SELECT 1 FROM (VALUES
                   ('krx_eod_bars'::text),('krx_investor_flows'::text),
                   ('krx_market_status'::text),('krx_fundamentals'::text),
                   ('krx_kospi200_membership'::text),
                   ('krx_sector_classification'::text)
               ) AS required(dataset_id)
                WHERE NOT entitlement.covered_datasets
                          @> pg_catalog.jsonb_build_array(required.dataset_id))
    ) THEN
        RAISE EXCEPTION 'active rights cannot be recorded as a terminal Raw block'
            USING ERRCODE='23514';
    END IF;
    UPDATE public.candidate_raw_batch_publications
       SET state='BLOCKED', reason_code='ENTITLEMENT_INACTIVE', published_at=NULL
     WHERE batch_id=p_batch_id AND surface=p_surface AND state='CATALOGED'
       AND raw_manifest_sha256=p_raw_manifest_sha256 AND fetch_mode=p_fetch_mode
       AND entitlement_reference=p_entitlement_reference
       AND entitlement_date=p_entitlement_date;
    INSERT INTO public.candidate_raw_batch_publications
        (batch_id,surface,raw_manifest_sha256,fetch_mode,entitlement_reference,
         entitlement_date,state,reason_code)
    VALUES
        (p_batch_id,p_surface,p_raw_manifest_sha256,p_fetch_mode,
         p_entitlement_reference,p_entitlement_date,'BLOCKED','ENTITLEMENT_INACTIVE')
    ON CONFLICT (batch_id,surface) DO NOTHING;
    IF NOT EXISTS (
        SELECT 1 FROM public.candidate_raw_batch_publications AS batch
         WHERE batch.batch_id=p_batch_id AND batch.surface=p_surface
           AND batch.raw_manifest_sha256=p_raw_manifest_sha256
           AND batch.fetch_mode=p_fetch_mode
           AND batch.entitlement_reference=p_entitlement_reference
           AND batch.entitlement_date=p_entitlement_date
           AND batch.state='BLOCKED' AND batch.reason_code='ENTITLEMENT_INACTIVE'
    ) THEN
        RAISE EXCEPTION 'candidate Raw terminal block conflicts with durable state'
            USING ERRCODE='23514';
    END IF;
END
$raw_block$;

ALTER FUNCTION public.begin_candidate_raw_batch(uuid,text,text,text,text,date)
    OWNER TO migration_owner;
ALTER FUNCTION public.bind_candidate_raw_dataset(uuid,text,text,uuid,boolean)
    OWNER TO migration_owner;
ALTER FUNCTION public.seal_candidate_raw_batch(uuid,text,text,text)
    OWNER TO migration_owner;
ALTER FUNCTION public.block_candidate_raw_batch_for_inactive_rights(uuid,text,text,text,text,date,date,date)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.begin_candidate_raw_batch(uuid,text,text,text,text,date) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.bind_candidate_raw_dataset(uuid,text,text,uuid,boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.seal_candidate_raw_batch(uuid,text,text,text) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.block_candidate_raw_batch_for_inactive_rights(uuid,text,text,text,text,date,date,date) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.begin_candidate_raw_batch(uuid,text,text,text,text,date) TO research_writer;
GRANT EXECUTE ON FUNCTION public.bind_candidate_raw_dataset(uuid,text,text,uuid,boolean) TO research_writer;
GRANT EXECUTE ON FUNCTION public.seal_candidate_raw_batch(uuid,text,text,text) TO research_writer;
GRANT EXECUTE ON FUNCTION public.block_candidate_raw_batch_for_inactive_rights(uuid,text,text,text,text,date,date,date) TO research_writer;

CREATE FUNCTION public.register_candidate_instrument(
    p_instrument_id text,
    p_symbol text,
    p_name text,
    p_asset_class text,
    p_listed_at date,
    p_entitlement_id uuid,
    p_contract_reference text,
    p_entitlement_date date,
    p_reference_sha256 text,
    p_source_revision text,
    p_retrieved_at timestamptz
)
RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $instrument_catalog$
DECLARE
    v_inserted bigint;
BEGIN
    IF p_instrument_id !~ '^[0-9A-Z]{1,12}\.KRX$'
       OR p_symbol !~ '^[0-9A-Z]{1,12}$'
       OR p_instrument_id <> p_symbol || '.KRX'
       OR p_name IS NULL OR length(btrim(p_name)) NOT BETWEEN 1 AND 256
       OR p_asset_class NOT IN ('ETF', 'EQUITY')
       OR p_listed_at IS NULL
       OR p_reference_sha256 !~ '^[0-9a-f]{64}$'
       OR p_source_revision IS NULL OR length(btrim(p_source_revision)) NOT BETWEEN 1 AND 128
       OR p_retrieved_at IS NULL
    THEN
        RAISE EXCEPTION 'invalid candidate Raw reference instrument'
            USING ERRCODE = '23514';
    END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        p_entitlement_id,
        p_contract_reference,
        'krx_eod_bars',
        p_entitlement_date,
        p_entitlement_date
    ) THEN
        RAISE EXCEPTION 'candidate instrument requires exact Raw candidate-use rights'
            USING ERRCODE = '42501';
    END IF;

    INSERT INTO public.instruments
        (id, symbol, venue, currency, name, asset_class, status, listed_at, delisted_at)
    VALUES
        (p_instrument_id, p_symbol, 'KRX', 'KRW', p_name, p_asset_class,
         'ACTIVE', p_listed_at, NULL)
    ON CONFLICT (id) DO NOTHING;
    IF NOT EXISTS (
        SELECT 1
          FROM public.instruments AS instrument
         WHERE instrument.id = p_instrument_id
           AND instrument.symbol = p_symbol
           AND instrument.venue = 'KRX'
           AND instrument.currency = 'KRW'
           AND instrument.name = p_name
           AND instrument.asset_class = p_asset_class
           AND instrument.status = 'ACTIVE'
           AND instrument.listed_at = p_listed_at
           AND instrument.delisted_at IS NULL
    ) THEN
        RAISE EXCEPTION 'candidate instrument conflicts with canonical instrument master'
            USING ERRCODE = '23514';
    END IF;

    INSERT INTO public.candidate_instrument_registrations
        (instrument_id, reference_sha256, source_revision, entitlement_id,
         license_ref, entitlement_date, retrieved_at)
    VALUES
        (p_instrument_id, p_reference_sha256, p_source_revision, p_entitlement_id,
         p_contract_reference, p_entitlement_date, p_retrieved_at)
    ON CONFLICT ON CONSTRAINT candidate_instrument_registrations_pkey DO NOTHING;
    GET DIAGNOSTICS v_inserted = ROW_COUNT;
    IF v_inserted = 0 AND NOT EXISTS (
        SELECT 1
          FROM public.candidate_instrument_registrations AS registration
         WHERE registration.instrument_id = p_instrument_id
           AND registration.reference_sha256 = p_reference_sha256
           AND registration.entitlement_id = p_entitlement_id
           AND registration.license_ref = p_contract_reference
    ) THEN
        RAISE EXCEPTION 'candidate instrument Raw reference replay conflicts'
            USING ERRCODE = '23514';
    END IF;
    RETURN v_inserted = 1;
END
$instrument_catalog$;

ALTER FUNCTION public.register_candidate_instrument(
    text, text, text, text, date, uuid, text, date, text, text, timestamptz
) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.register_candidate_instrument(
    text, text, text, text, date, uuid, text, date, text, text, timestamptz
) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.register_candidate_instrument(
    text, text, text, text, date, uuid, text, date, text, text, timestamptz
) TO research_writer;

CREATE FUNCTION public.register_candidate_source_dataset(
    p_dataset_id text,
    p_dataset_version text,
    p_manifest_sha256 text,
    p_entitlement_id uuid,
    p_contract_reference text,
    p_entitlement_date date
)
RETURNS uuid
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $catalog$
DECLARE
    v_id uuid;
    v_manifest_sha256 text;
    v_storage_path text;
    v_expected_storage_path text;
    v_status text;
BEGIN
    IF p_dataset_id NOT IN (
        'krx_investor_flows',
        'krx_market_status',
        'krx_fundamentals',
        'krx_kospi200_membership',
        'krx_sector_classification'
    ) OR p_dataset_version IS NULL
      OR p_dataset_version !~ '^[a-z0-9:_-]{1,128}$'
      OR p_manifest_sha256 !~ '^[0-9a-f]{64}$'
    THEN
        RAISE EXCEPTION 'invalid candidate source dataset catalog entry'
            USING ERRCODE = '23514';
    END IF;
    v_expected_storage_path := 'db://candidate/' || p_dataset_id || '/' || p_dataset_version;
    IF NOT public.candidate_source_entitlement_is_valid(
        p_entitlement_id,
        p_contract_reference,
        p_dataset_id,
        p_entitlement_date,
        p_entitlement_date
    ) THEN
        RAISE EXCEPTION 'candidate source catalog requires exact active candidate-use rights'
            USING ERRCODE = '42501';
    END IF;
    INSERT INTO public.dataset_versions
        (dataset_id, version, status, manifest_sha256, storage_path)
    VALUES
        (p_dataset_id, p_dataset_version, 'READY', p_manifest_sha256, v_expected_storage_path)
    ON CONFLICT (dataset_id, version) DO NOTHING;
    SELECT dataset.id, dataset.manifest_sha256, dataset.storage_path, dataset.status
      INTO v_id, v_manifest_sha256, v_storage_path, v_status
      FROM public.dataset_versions AS dataset
     WHERE dataset.dataset_id = p_dataset_id
       AND dataset.version = p_dataset_version
     FOR SHARE OF dataset;
    IF v_id IS NULL
       OR v_manifest_sha256 <> p_manifest_sha256
       OR v_storage_path <> v_expected_storage_path
       OR v_status NOT IN ('READY', 'WARNING')
    THEN
        RAISE EXCEPTION 'candidate source dataset catalog conflicts with immutable Raw batch'
            USING ERRCODE = '23514';
    END IF;
    RETURN v_id;
END
$catalog$;

ALTER FUNCTION public.register_candidate_source_dataset(text, text, text, uuid, text, date)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.register_candidate_source_dataset(text, text, text, uuid, text, date)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.register_candidate_source_dataset(text, text, text, uuid, text, date)
    TO research_writer;

-- The research worker may publish only the fixed krx_eod_bars catalog shape.
-- It never receives broad dataset_versions DML. Exact retry is accepted;
-- conflicting bytes, paths, provenance, or rights fail closed.
CREATE FUNCTION public.publish_candidate_price_publication(
    p_dataset_version text,
    p_manifest_sha256 text,
    p_storage_path text,
    p_curated_generation bigint,
    p_first_session date,
    p_last_session date,
    p_instrument_coverage jsonb,
    p_provider text,
    p_entitlement_id uuid,
    p_license_ref text,
    p_source_revision text,
    p_raw_batch_id uuid,
    p_raw_manifest_sha256 text,
    p_fetch_mode text,
    p_entitlement_date date,
    p_available_at timestamptz,
    p_retrieved_at timestamptz
)
RETURNS TABLE (dataset_version_id uuid, published boolean)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $publisher$
DECLARE
    v_dataset_id uuid;
    v_manifest_sha256 text;
    v_storage_path text;
    v_status text;
    v_inserted bigint;
    v_coverage_count bigint;
BEGIN
    IF p_dataset_version IS NULL OR length(btrim(p_dataset_version)) NOT BETWEEN 1 AND 128
       OR p_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_storage_path IS NULL OR btrim(p_storage_path) = ''
       OR p_curated_generation NOT BETWEEN 1 AND 4294967295
       OR p_last_session < p_first_session
       OR jsonb_typeof(p_instrument_coverage) IS DISTINCT FROM 'array'
       OR jsonb_array_length(p_instrument_coverage) = 0
       OR p_retrieved_at < p_available_at
       OR p_source_revision IS NULL OR btrim(p_source_revision) = ''
       OR p_raw_batch_id IS NULL OR p_source_revision <> p_raw_batch_id::text
       OR p_raw_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_fetch_mode NOT IN ('credentialed','synthetic')
       OR p_entitlement_date IS NULL OR p_entitlement_date <> p_last_session
    THEN
        RAISE EXCEPTION 'invalid candidate price publication'
            USING ERRCODE = '23514';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM jsonb_array_elements(p_instrument_coverage) AS element(value)
         WHERE jsonb_typeof(element.value) IS DISTINCT FROM 'object'
            OR (element.value ->> 'instrument_id') IS NULL
            OR (element.value ->> 'first_session') IS NULL
            OR (element.value ->> 'last_session') IS NULL
            OR (element.value ->> 'session_count') IS NULL
            OR (element.value -> 'sessions') IS NULL
    ) OR EXISTS (
        SELECT 1
          FROM jsonb_to_recordset(p_instrument_coverage) AS coverage(
              instrument_id text,
              first_session date,
              last_session date,
              session_count integer,
              sessions jsonb
          )
         WHERE coverage.instrument_id !~ '^[0-9A-Z][0-9A-Z._-]{1,31}$'
            OR coverage.first_session < p_first_session
            OR coverage.last_session > p_last_session
            OR coverage.last_session < coverage.first_session
            OR coverage.session_count <= 0
            OR jsonb_typeof(coverage.sessions) IS DISTINCT FROM 'array'
            OR jsonb_array_length(coverage.sessions) <> coverage.session_count
    ) OR EXISTS (
        SELECT 1
          FROM jsonb_to_recordset(p_instrument_coverage) AS coverage(
              instrument_id text,
              first_session date,
              last_session date,
              session_count integer,
              sessions jsonb
          )
          CROSS JOIN LATERAL (
              SELECT count(*) AS row_count,
                     count(DISTINCT session.value::date) AS distinct_count,
                     min(session.value::date) AS first_session,
                     max(session.value::date) AS last_session
                FROM jsonb_array_elements_text(
                    CASE WHEN jsonb_typeof(coverage.sessions) = 'array'
                         THEN coverage.sessions ELSE '[]'::jsonb END
                ) AS session(value)
          ) AS exact_sessions
         WHERE exact_sessions.row_count <> coverage.session_count
            OR exact_sessions.distinct_count <> coverage.session_count
            OR exact_sessions.first_session IS DISTINCT FROM coverage.first_session
            OR exact_sessions.last_session IS DISTINCT FROM coverage.last_session
    ) OR (
        SELECT count(*) <> count(DISTINCT coverage.instrument_id)
          FROM jsonb_to_recordset(p_instrument_coverage) AS coverage(
              instrument_id text,
              first_session date,
              last_session date,
              session_count integer
          )
    ) OR (
        SELECT min(coverage.first_session) <> p_first_session
            OR max(coverage.last_session) <> p_last_session
          FROM jsonb_to_recordset(p_instrument_coverage) AS coverage(
              instrument_id text,
              first_session date,
              last_session date,
              session_count integer
          )
    ) THEN
        RAISE EXCEPTION 'candidate price coverage is invalid or does not match its publication'
            USING ERRCODE = '23514';
    END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        p_entitlement_id,
        p_license_ref,
        'krx_eod_bars',
        p_first_session,
        p_last_session
    ) THEN
        RAISE EXCEPTION 'candidate price requires an exact active candidate-use entitlement'
            USING ERRCODE = '42501';
    END IF;
    PERFORM public.begin_candidate_raw_batch(
        p_raw_batch_id,'price',p_raw_manifest_sha256,p_fetch_mode,
        p_license_ref,p_entitlement_date
    );

    INSERT INTO public.dataset_versions
        (dataset_id, version, status, manifest_sha256, storage_path)
    VALUES
        ('krx_eod_bars', p_dataset_version, 'READY', p_manifest_sha256, p_storage_path)
    ON CONFLICT (dataset_id, version) DO NOTHING;

    SELECT dataset.id, dataset.manifest_sha256, dataset.storage_path, dataset.status
      INTO v_dataset_id, v_manifest_sha256, v_storage_path, v_status
      FROM public.dataset_versions AS dataset
     WHERE dataset.dataset_id = 'krx_eod_bars'
       AND dataset.version = p_dataset_version
     FOR SHARE OF dataset;
    IF v_dataset_id IS NULL
       OR v_manifest_sha256 <> p_manifest_sha256
       OR v_storage_path <> p_storage_path
       OR v_status NOT IN ('READY', 'WARNING')
    THEN
        RAISE EXCEPTION 'candidate price dataset catalog conflicts with immutable generation'
            USING ERRCODE = '23514';
    END IF;

    INSERT INTO public.candidate_price_publications
        (dataset_version_id, dataset_version, manifest_sha256, market,
         curated_generation, first_session, last_session, provider,
         entitlement_id, license_ref, source_revision, available_at, retrieved_at)
    VALUES
        (v_dataset_id, p_dataset_version, p_manifest_sha256, 'kr',
         p_curated_generation, p_first_session, p_last_session, p_provider,
         p_entitlement_id, p_license_ref, p_source_revision, p_available_at, p_retrieved_at)
    ON CONFLICT ON CONSTRAINT candidate_price_publications_pkey DO NOTHING;
    GET DIAGNOSTICS v_inserted = ROW_COUNT;

    IF v_inserted = 0 AND NOT EXISTS (
        SELECT 1
          FROM public.candidate_price_publications AS price
         WHERE price.dataset_version_id = v_dataset_id
           AND price.dataset_version = p_dataset_version
           AND price.manifest_sha256 = p_manifest_sha256
           AND price.market = 'kr'
           AND price.curated_generation = p_curated_generation
           AND price.first_session = p_first_session
           AND price.last_session = p_last_session
           AND price.provider = p_provider
           AND price.entitlement_id = p_entitlement_id
           AND price.license_ref = p_license_ref
           AND price.source_revision = p_source_revision
           AND price.available_at = p_available_at
           AND price.retrieved_at = p_retrieved_at
    ) THEN
        RAISE EXCEPTION 'candidate price publication conflicts with immutable generation'
            USING ERRCODE = '23514';
    END IF;

    IF v_inserted = 1 THEN
        INSERT INTO public.candidate_price_instrument_coverage
            (dataset_version_id, instrument_id, first_session, last_session, session_count)
        SELECT v_dataset_id, coverage.instrument_id, coverage.first_session,
               coverage.last_session, coverage.session_count
          FROM jsonb_to_recordset(p_instrument_coverage) AS coverage(
              instrument_id text,
              first_session date,
              last_session date,
              session_count integer
          );
        INSERT INTO public.candidate_price_instrument_sessions
            (dataset_version_id, instrument_id, session_date)
        SELECT v_dataset_id, coverage.instrument_id, session.value::date
          FROM jsonb_to_recordset(p_instrument_coverage) AS coverage(
              instrument_id text,
              sessions jsonb
          )
          CROSS JOIN LATERAL jsonb_array_elements_text(coverage.sessions) AS session(value);
    END IF;

    SELECT count(*) INTO v_coverage_count
      FROM public.candidate_price_instrument_coverage AS stored
     WHERE stored.dataset_version_id = v_dataset_id;
    IF v_coverage_count <> jsonb_array_length(p_instrument_coverage)
       OR EXISTS (
           SELECT 1
             FROM jsonb_to_recordset(p_instrument_coverage) AS supplied(
                 instrument_id text,
                 first_session date,
                 last_session date,
                 session_count integer
             )
            WHERE NOT EXISTS (
                SELECT 1
                  FROM public.candidate_price_instrument_coverage AS stored
                 WHERE stored.dataset_version_id = v_dataset_id
                   AND stored.instrument_id = supplied.instrument_id
                   AND stored.first_session = supplied.first_session
                   AND stored.last_session = supplied.last_session
                   AND stored.session_count = supplied.session_count
            )
       )
       OR (
           SELECT count(*)
             FROM public.candidate_price_instrument_sessions AS stored_session
            WHERE stored_session.dataset_version_id = v_dataset_id
       ) <> (
           SELECT sum(supplied.session_count)
             FROM jsonb_to_recordset(p_instrument_coverage) AS supplied(
                 session_count integer
             )
       )
       OR EXISTS (
           SELECT 1
             FROM jsonb_to_recordset(p_instrument_coverage) AS supplied(
                 instrument_id text,
                 sessions jsonb
             )
             CROSS JOIN LATERAL jsonb_array_elements_text(supplied.sessions) AS session(value)
            WHERE NOT EXISTS (
                SELECT 1
                  FROM public.candidate_price_instrument_sessions AS stored_session
                 WHERE stored_session.dataset_version_id = v_dataset_id
                   AND stored_session.instrument_id = supplied.instrument_id
                   AND stored_session.session_date = session.value::date
            )
       )
    THEN
        RAISE EXCEPTION 'candidate price coverage conflicts with immutable generation'
            USING ERRCODE = '23514';
    END IF;
    PERFORM public.bind_candidate_raw_dataset(
        p_raw_batch_id,'price','bars',v_dataset_id,false
    );
    PERFORM public.seal_candidate_raw_batch(
        p_raw_batch_id,'price',p_raw_manifest_sha256,p_fetch_mode
    );
    RETURN QUERY SELECT v_dataset_id, v_inserted = 1;
END
$publisher$;

ALTER FUNCTION public.publish_candidate_price_publication(
    text, text, text, bigint, date, date, jsonb, text, uuid, text, text, uuid, text, text,
    date, timestamptz, timestamptz
) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.publish_candidate_price_publication(
    text, text, text, bigint, date, date, jsonb, text, uuid, text, text, uuid, text, text,
    date, timestamptz, timestamptz
) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.publish_candidate_price_publication(
    text, text, text, bigint, date, date, jsonb, text, uuid, text, text, uuid, text, text,
    date, timestamptz, timestamptz
) TO research_writer;

CREATE FUNCTION public.candidate_source_dataset_write_is_open(p_dataset_version_id uuid)
RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $gate$
    SELECT EXISTS (
        SELECT 1 FROM public.candidate_raw_batch_datasets AS binding
        JOIN public.candidate_raw_batch_publications AS batch
          ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
       WHERE binding.dataset_version_id=p_dataset_version_id
         AND binding.surface='source' AND NOT binding.reused_existing
         AND batch.state='CATALOGED'
         AND batch.batch_id=NULLIF(current_setting('app.candidate_raw_batch_id',true),'')::uuid)
$gate$;
ALTER FUNCTION public.candidate_source_dataset_write_is_open(uuid) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.candidate_source_dataset_write_is_open(uuid) FROM PUBLIC;

CREATE FUNCTION public.candidate_source_dataset_write_matches(
    p_dataset_version_id uuid,
    p_response_kind text,
    p_provider text,
    p_entitlement_id uuid,
    p_entitlement_date date,
    p_license_ref text,
    p_manifest_sha256 text
)
RETURNS boolean
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path=pg_catalog
AS $gate$
    SELECT EXISTS (
        SELECT 1
          FROM public.candidate_raw_batch_datasets AS binding
          JOIN public.candidate_raw_batch_publications AS batch
            ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
          JOIN public.dataset_versions AS dataset
            ON dataset.id=binding.dataset_version_id
          JOIN public.data_entitlements AS entitlement
            ON entitlement.id=p_entitlement_id
         WHERE binding.dataset_version_id=p_dataset_version_id
           AND binding.surface='source'
           AND NOT binding.reused_existing
           AND binding.response_kind=p_response_kind
           AND batch.state='CATALOGED'
           AND batch.batch_id=NULLIF(current_setting('app.candidate_raw_batch_id',true),'')::uuid
           AND batch.entitlement_reference=p_license_ref
           AND batch.entitlement_date=p_entitlement_date
           AND p_provider='krx'
           AND dataset.manifest_sha256=p_manifest_sha256
           AND dataset.dataset_id=CASE p_response_kind
                WHEN 'investor_flow' THEN 'krx_investor_flows'
                WHEN 'market_status' THEN 'krx_market_status'
                WHEN 'fundamentals' THEN 'krx_fundamentals'
                WHEN 'index_membership' THEN 'krx_kospi200_membership'
                WHEN 'sector_classification' THEN 'krx_sector_classification'
                ELSE NULL END
           AND entitlement.contract_reference=p_license_ref
           AND entitlement.status='ACTIVE'
           AND entitlement.covered_uses @> '["candidate"]'::jsonb
           AND entitlement.covered_datasets @> pg_catalog.jsonb_build_array(dataset.dataset_id)
           AND entitlement.effective_from <= p_entitlement_date
           AND entitlement.effective_until >= p_entitlement_date
    )
$gate$;
ALTER FUNCTION public.candidate_source_dataset_write_matches(
    uuid,text,text,uuid,date,text,text
) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.candidate_source_dataset_write_matches(
    uuid,text,text,uuid,date,text,text
) FROM PUBLIC;

CREATE FUNCTION public.insert_candidate_investor_flow(
    text,date,text,numeric,numeric,text,text,text,uuid,date,text,text,
    timestamptz,timestamptz,uuid,text
) RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $insert$
DECLARE n bigint; v_flow_id uuid;
BEGIN
    IF $2 > $10 OR $14 < $13
       OR NOT public.candidate_source_entitlement_is_valid(
           $9,$11,'krx_investor_flows',$2,$10
       )
       OR NOT public.candidate_source_dataset_write_matches(
           $15,'investor_flow',$8,$9,$10,$11,$16
       ) THEN
        RAISE EXCEPTION 'candidate source dataset is not open for typed publication' USING ERRCODE='42501';
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(
        pg_catalog.jsonb_build_array('candidate-investor-flow',$1,$2,$3,$12)::text, 0
    ));
    INSERT INTO public.candidate_investor_flows
        (instrument_id,trade_date,investor_class,net_amount,net_volume,currency,volume_unit,
         provider,source_revision,available_at)
    VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$12,$13)
    ON CONFLICT (instrument_id,trade_date,investor_class,source_revision) DO NOTHING
    RETURNING id INTO v_flow_id;
    IF v_flow_id IS NULL THEN
        SELECT flow.id INTO v_flow_id
          FROM public.candidate_investor_flows AS flow
         WHERE flow.instrument_id=$1 AND flow.trade_date=$2 AND flow.investor_class=$3
           AND flow.source_revision=$12 AND flow.net_amount=$4 AND flow.net_volume=$5
           AND flow.currency=$6 AND flow.volume_unit=$7 AND flow.provider=$8
           AND flow.available_at=$13;
        IF v_flow_id IS NULL THEN
            RAISE EXCEPTION 'candidate investor-flow natural identity is occupied by different content'
                USING ERRCODE='23514';
        END IF;
    END IF;
    INSERT INTO public.candidate_investor_flow_snapshot_rows
        (dataset_version_id,flow_observation_id,entitlement_id,entitlement_date,
         license_ref,retrieved_at,manifest_sha256)
    VALUES ($15,v_flow_id,$9,$10,$11,$14,$16)
    ON CONFLICT (dataset_version_id,flow_observation_id) DO NOTHING;
    GET DIAGNOSTICS n=ROW_COUNT;
    IF n=0 AND NOT EXISTS (
        SELECT 1 FROM public.candidate_investor_flow_snapshot_rows AS member
         WHERE member.dataset_version_id=$15 AND member.flow_observation_id=v_flow_id
           AND member.entitlement_id=$9 AND member.entitlement_date=$10
           AND member.license_ref=$11 AND member.retrieved_at=$14
           AND member.manifest_sha256=$16
    ) THEN
        RAISE EXCEPTION 'candidate investor-flow snapshot membership replay conflicts'
            USING ERRCODE='23514';
    END IF;
    RETURN n=1;
END $insert$;

CREATE FUNCTION public.insert_candidate_market_status(
    text,date,boolean,boolean,boolean,boolean,boolean,boolean,text,uuid,date,text,text,
    timestamptz,timestamptz,uuid,text
) RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $insert$
DECLARE n bigint;
BEGIN
    IF $2 <> $11 OR NOT public.candidate_source_dataset_write_matches(
        $16,'market_status',$9,$10,$11,$12,$17
    ) THEN
        RAISE EXCEPTION 'candidate source dataset is not open for typed publication' USING ERRCODE='42501';
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(
        pg_catalog.jsonb_build_array('candidate-market-status',$1,$2,$13)::text, 0
    ));
    IF EXISTS (
        SELECT 1 FROM public.candidate_market_status_observations AS existing
         WHERE existing.instrument_id=$1 AND existing.trade_date=$2
           AND existing.source_revision=$13
           AND ROW(existing.suspended,existing.administrative,existing.liquidation,
                   existing.inactive,existing.disqualifying_audit_opinion,
                   existing.complete_capital_impairment,existing.provider,existing.available_at)
               IS DISTINCT FROM ROW($3,$4,$5,$6,$7,$8,$9,$14)
    ) THEN
        RAISE EXCEPTION 'candidate market-status natural identity is occupied by different content'
            USING ERRCODE='23514';
    END IF;
    INSERT INTO public.candidate_market_status_observations
        (instrument_id,trade_date,suspended,administrative,liquidation,inactive,
         disqualifying_audit_opinion,complete_capital_impairment,provider,entitlement_id,
         entitlement_date,license_ref,source_revision,available_at,retrieved_at,
         dataset_version_id,manifest_sha256)
    VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
    ON CONFLICT (instrument_id,trade_date,source_revision,dataset_version_id) DO NOTHING;
    GET DIAGNOSTICS n=ROW_COUNT;
    IF n=0 AND NOT EXISTS (
        SELECT 1 FROM public.candidate_market_status_observations AS existing
         WHERE existing.instrument_id=$1 AND existing.trade_date=$2
           AND existing.suspended=$3 AND existing.administrative=$4
           AND existing.liquidation=$5 AND existing.inactive=$6
           AND existing.disqualifying_audit_opinion=$7
           AND existing.complete_capital_impairment=$8 AND existing.provider=$9
           AND existing.entitlement_id=$10 AND existing.entitlement_date=$11
           AND existing.license_ref=$12 AND existing.source_revision=$13
           AND existing.available_at=$14 AND existing.retrieved_at=$15
           AND existing.dataset_version_id=$16 AND existing.manifest_sha256=$17
    ) THEN
        RAISE EXCEPTION 'candidate market-status replay conflicts' USING ERRCODE='23514';
    END IF;
    RETURN n=1;
END $insert$;

CREATE FUNCTION public.insert_candidate_fundamental(
    text,date,date,text,text,text,numeric,text,integer,boolean,timestamptz,timestamptz,
    timestamptz,text,uuid,date,text,text,uuid,uuid,text
) RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $insert$
DECLARE n bigint;
BEGIN
    IF NOT public.candidate_source_dataset_write_matches(
        $20,'fundamentals',$14,$15,$16,$17,$21
    ) THEN
        RAISE EXCEPTION 'candidate source dataset is not open for typed publication' USING ERRCODE='42501';
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(
        pg_catalog.jsonb_build_array(
            'candidate-fundamental',$1,$3,$5,$6,$11,$18
        )::text, 0
    ));
    IF $19 IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM public.candidate_fundamental_observations AS prior
         WHERE prior.id=$19 AND prior.instrument_id=$1
           AND prior.fiscal_period_end=$3 AND prior.statement_scope=$5
           AND prior.metric=$6 AND prior.source_revision<>$18
           AND prior.disclosed_at <= $11 AND prior.available_at < $12
    ) THEN
        RAISE EXCEPTION 'candidate fundamental restatement lineage is invalid'
            USING ERRCODE='23514';
    END IF;
    IF EXISTS (
        SELECT 1 FROM public.candidate_fundamental_observations AS existing
         WHERE existing.instrument_id=$1 AND existing.fiscal_period_end=$3
           AND existing.statement_scope=$5 AND existing.metric=$6
           AND existing.disclosed_at=$11 AND existing.source_revision=$18
           AND (
               ROW(existing.fiscal_period_start,existing.period_kind,existing.value,
                       existing.currency,existing.unit_scale,existing.audited,
                       existing.available_at,existing.provider,
                       existing.restates_observation_id IS NULL)
                   IS DISTINCT FROM ROW($2,$4,$7,$8,$9,$10,$12,$14,$19 IS NULL)
               OR (existing.restates_observation_id IS NOT NULL AND $19 IS NOT NULL
                AND NOT EXISTS (
                    SELECT 1
                      FROM public.candidate_fundamental_observations AS old_restatement
                      JOIN public.candidate_fundamental_observations AS new_restatement
                        ON new_restatement.id=$19
                       AND ROW(new_restatement.instrument_id,
                               new_restatement.fiscal_period_end,
                               new_restatement.statement_scope,
                               new_restatement.metric,
                               new_restatement.disclosed_at,
                               new_restatement.source_revision)
                           = ROW(old_restatement.instrument_id,
                                 old_restatement.fiscal_period_end,
                                 old_restatement.statement_scope,
                                 old_restatement.metric,
                                 old_restatement.disclosed_at,
                                 old_restatement.source_revision)
                     WHERE old_restatement.id=existing.restates_observation_id
                ))
           )
    ) THEN
        RAISE EXCEPTION 'candidate fundamental natural identity is occupied by different content'
            USING ERRCODE='23514';
    END IF;
    INSERT INTO public.candidate_fundamental_observations
        (instrument_id,fiscal_period_start,fiscal_period_end,period_kind,statement_scope,
         metric,value,currency,unit_scale,audited,disclosed_at,available_at,retrieved_at,
         provider,entitlement_id,entitlement_date,license_ref,source_revision,
         restates_observation_id,dataset_version_id,manifest_sha256)
    VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)
    ON CONFLICT (instrument_id,fiscal_period_end,statement_scope,metric,disclosed_at,
                 source_revision,dataset_version_id)
    DO NOTHING;
    GET DIAGNOSTICS n=ROW_COUNT;
    IF n=0 AND NOT EXISTS (
        SELECT 1 FROM public.candidate_fundamental_observations AS existing
         WHERE existing.instrument_id=$1 AND existing.fiscal_period_start=$2
           AND existing.fiscal_period_end=$3 AND existing.period_kind=$4
           AND existing.statement_scope=$5 AND existing.metric=$6
           AND existing.value=$7 AND existing.currency IS NOT DISTINCT FROM $8
           AND existing.unit_scale=$9 AND existing.audited IS NOT DISTINCT FROM $10
           AND existing.disclosed_at=$11 AND existing.available_at=$12
           AND existing.retrieved_at=$13 AND existing.provider=$14
           AND existing.entitlement_id=$15 AND existing.entitlement_date=$16
           AND existing.license_ref=$17 AND existing.source_revision=$18
           AND existing.restates_observation_id IS NOT DISTINCT FROM $19
           AND existing.dataset_version_id=$20 AND existing.manifest_sha256=$21
    ) THEN
        RAISE EXCEPTION 'candidate fundamental replay conflicts' USING ERRCODE='23514';
    END IF;
    RETURN n=1;
END $insert$;

CREATE FUNCTION public.insert_candidate_universe_snapshot(
    date,uuid,text,text,uuid,date,text,text,timestamptz,timestamptz,integer
) RETURNS uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $insert$
DECLARE v_id uuid;
BEGIN
    IF $1 > $6 OR NOT public.candidate_source_dataset_write_matches(
        $2,'index_membership',$4,$5,$6,$7,$3
    ) THEN
        RAISE EXCEPTION 'candidate source dataset is not open for typed publication' USING ERRCODE='42501';
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(
        pg_catalog.jsonb_build_array('candidate-universe-snapshot','kospi200',$1,$8)::text, 0
    ));
    IF EXISTS (
        SELECT 1 FROM public.candidate_universe_snapshots AS existing
         WHERE existing.index_id='kospi200' AND existing.as_of_date=$1
           AND existing.source_revision=$8
           AND ROW(existing.available_at,existing.provider,existing.member_count)
               IS DISTINCT FROM ROW($9,$4,$11)
    ) THEN
        RAISE EXCEPTION 'candidate universe snapshot natural identity is occupied by different content'
            USING ERRCODE='23514';
    END IF;
    INSERT INTO public.candidate_universe_snapshots
        (index_id,as_of_date,dataset_version_id,manifest_sha256,provider,entitlement_id,
         entitlement_date,license_ref,source_revision,available_at,retrieved_at,member_count)
    VALUES ('kospi200',$1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
    ON CONFLICT (index_id,as_of_date,dataset_version_id) DO NOTHING RETURNING id INTO v_id;
    IF v_id IS NULL AND NOT EXISTS (
        SELECT 1 FROM public.candidate_universe_snapshots AS existing
         WHERE existing.index_id='kospi200' AND existing.as_of_date=$1
           AND existing.dataset_version_id=$2 AND existing.manifest_sha256=$3
           AND existing.provider=$4 AND existing.entitlement_id=$5
           AND existing.entitlement_date=$6 AND existing.license_ref=$7
           AND existing.source_revision=$8 AND existing.available_at=$9
           AND existing.retrieved_at=$10 AND existing.member_count=$11
    ) THEN
        RAISE EXCEPTION 'candidate universe snapshot replay conflicts' USING ERRCODE='23514';
    END IF;
    RETURN v_id;
END $insert$;

CREATE FUNCTION public.insert_candidate_universe_member(
    uuid,text,timestamptz,date,date,timestamptz,text
) RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $insert$
DECLARE n bigint; v_dataset uuid; v_index_id text;
BEGIN
    SELECT snapshot.dataset_version_id,snapshot.index_id INTO v_dataset,v_index_id
      FROM public.candidate_universe_snapshots AS snapshot WHERE snapshot.id=$1;
    IF NOT public.candidate_source_dataset_write_is_open(v_dataset) THEN
        RAISE EXCEPTION 'candidate source dataset is not open for typed publication' USING ERRCODE='42501';
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(
        pg_catalog.jsonb_build_array(
            'candidate-universe-member',v_index_id,$2,$4,$7
        )::text, 0
    ));
    IF EXISTS (
        SELECT 1 FROM public.candidate_universe_members AS existing
        JOIN public.candidate_universe_snapshots AS snapshot
          ON snapshot.id=existing.universe_snapshot_id
         WHERE snapshot.index_id=v_index_id AND existing.instrument_id=$2
           AND existing.effective_from=$4 AND existing.source_revision=$7
           AND ROW(existing.announced_at,existing.effective_until,existing.available_at)
               IS DISTINCT FROM ROW($3,$5,$6)
    ) THEN
        RAISE EXCEPTION 'candidate universe-member natural identity is occupied by different content'
            USING ERRCODE='23514';
    END IF;
    INSERT INTO public.candidate_universe_members
        (universe_snapshot_id,instrument_id,announced_at,effective_from,effective_until,
         available_at,source_revision)
    VALUES ($1,$2,$3,$4,$5,$6,$7)
    ON CONFLICT (universe_snapshot_id,instrument_id) DO NOTHING;
    GET DIAGNOSTICS n=ROW_COUNT;
    IF n=0 AND NOT EXISTS (
        SELECT 1 FROM public.candidate_universe_members AS existing
         WHERE existing.universe_snapshot_id=$1 AND existing.instrument_id=$2
           AND existing.announced_at=$3 AND existing.effective_from=$4
           AND existing.effective_until IS NOT DISTINCT FROM $5
           AND existing.available_at=$6 AND existing.source_revision=$7
    ) THEN
        RAISE EXCEPTION 'candidate universe-member replay conflicts' USING ERRCODE='23514';
    END IF;
    RETURN n=1;
END $insert$;

CREATE FUNCTION public.insert_candidate_sector_version(
    text,text,date,timestamptz,timestamptz,text,uuid,date,text,text,uuid,text
) RETURNS uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $insert$
DECLARE v_id uuid;
BEGIN
    IF $3 > $8 OR NOT public.candidate_source_dataset_write_matches(
        $11,'sector_classification',$6,$7,$8,$9,$12
    ) THEN
        RAISE EXCEPTION 'candidate source dataset is not open for typed publication' USING ERRCODE='42501';
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(
        pg_catalog.jsonb_build_array(
            'candidate-sector-version',$1,$2,$3,$10
        )::text, 0
    ));
    IF EXISTS (
        SELECT 1 FROM public.candidate_sector_versions AS existing
         WHERE existing.taxonomy_id=$1 AND existing.taxonomy_version=$2
           AND existing.effective_from=$3 AND existing.source_revision=$10
           AND ROW(existing.available_at,existing.provider)
               IS DISTINCT FROM ROW($4,$6)
    ) THEN
        RAISE EXCEPTION 'candidate sector-version natural identity is occupied by different content'
            USING ERRCODE='23514';
    END IF;
    INSERT INTO public.candidate_sector_versions
        (taxonomy_id,taxonomy_version,effective_from,available_at,retrieved_at,provider,
         entitlement_id,entitlement_date,license_ref,source_revision,dataset_version_id,
         manifest_sha256)
    VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
    ON CONFLICT (taxonomy_id,taxonomy_version,effective_from,source_revision,dataset_version_id)
    DO NOTHING RETURNING id INTO v_id;
    IF v_id IS NULL THEN
        SELECT existing.id INTO v_id FROM public.candidate_sector_versions AS existing
         WHERE existing.taxonomy_id=$1 AND existing.taxonomy_version=$2
           AND existing.effective_from=$3 AND existing.available_at=$4
           AND existing.retrieved_at=$5 AND existing.provider=$6
           AND existing.entitlement_id=$7 AND existing.entitlement_date=$8
           AND existing.license_ref=$9 AND existing.source_revision=$10
           AND existing.dataset_version_id=$11 AND existing.manifest_sha256=$12;
        IF v_id IS NULL THEN
            RAISE EXCEPTION 'candidate sector-version replay conflicts' USING ERRCODE='23514';
        END IF;
    END IF;
    RETURN v_id;
END $insert$;

CREATE FUNCTION public.insert_candidate_sector_entry(
    uuid,text,text,text,text,date,date,timestamptz,text
) RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $insert$
DECLARE
    n bigint;
    v_dataset uuid;
    v_taxonomy_id text;
    v_taxonomy_version text;
    v_version_effective_from date;
    v_version_source_revision text;
BEGIN
    SELECT version.dataset_version_id,version.taxonomy_id,version.taxonomy_version,
           version.effective_from,version.source_revision
      INTO v_dataset,v_taxonomy_id,v_taxonomy_version,
           v_version_effective_from,v_version_source_revision
      FROM public.candidate_sector_versions AS version WHERE version.id=$1;
    IF NOT public.candidate_source_dataset_write_is_open(v_dataset) THEN
        RAISE EXCEPTION 'candidate source dataset is not open for typed publication' USING ERRCODE='42501';
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(
        pg_catalog.jsonb_build_array(
            'candidate-sector-entry',v_taxonomy_id,v_taxonomy_version,
            v_version_effective_from,v_version_source_revision,$2,$9
        )::text, 0
    ));
    IF EXISTS (
        SELECT 1 FROM public.candidate_sector_entries AS existing
        JOIN public.candidate_sector_versions AS version
          ON version.id=existing.sector_version_id
         WHERE version.taxonomy_id=v_taxonomy_id
           AND version.taxonomy_version=v_taxonomy_version
           AND version.effective_from=v_version_effective_from
           AND version.source_revision=v_version_source_revision
           AND existing.instrument_id=$2 AND existing.source_revision=$9
           AND ROW(existing.sector_code,existing.sector_name,
                   existing.fundamental_profile,existing.effective_from,
                   existing.effective_until,existing.available_at)
               IS DISTINCT FROM ROW($3,$4,$5,$6,$7,$8)
    ) THEN
        RAISE EXCEPTION 'candidate sector-entry natural identity is occupied by different content'
            USING ERRCODE='23514';
    END IF;
    INSERT INTO public.candidate_sector_entries
        (sector_version_id,instrument_id,sector_code,sector_name,fundamental_profile,
         effective_from,effective_until,available_at,source_revision)
    VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
    ON CONFLICT (sector_version_id,instrument_id) DO NOTHING;
    GET DIAGNOSTICS n=ROW_COUNT;
    IF n=0 AND NOT EXISTS (
        SELECT 1 FROM public.candidate_sector_entries AS existing
         WHERE existing.sector_version_id=$1 AND existing.instrument_id=$2
           AND existing.sector_code=$3 AND existing.sector_name=$4
           AND existing.fundamental_profile=$5 AND existing.effective_from=$6
           AND existing.effective_until IS NOT DISTINCT FROM $7
           AND existing.available_at=$8 AND existing.source_revision=$9
    ) THEN
        RAISE EXCEPTION 'candidate sector-entry replay conflicts' USING ERRCODE='23514';
    END IF;
    RETURN n=1;
END $insert$;

DO $typed_publishers$
DECLARE signature text;
BEGIN
    FOREACH signature IN ARRAY ARRAY[
      'public.insert_candidate_investor_flow(text,date,text,numeric,numeric,text,text,text,uuid,date,text,text,timestamptz,timestamptz,uuid,text)',
      'public.insert_candidate_market_status(text,date,boolean,boolean,boolean,boolean,boolean,boolean,text,uuid,date,text,text,timestamptz,timestamptz,uuid,text)',
      'public.insert_candidate_fundamental(text,date,date,text,text,text,numeric,text,integer,boolean,timestamptz,timestamptz,timestamptz,text,uuid,date,text,text,uuid,uuid,text)',
      'public.insert_candidate_universe_snapshot(date,uuid,text,text,uuid,date,text,text,timestamptz,timestamptz,integer)',
      'public.insert_candidate_universe_member(uuid,text,timestamptz,date,date,timestamptz,text)',
      'public.insert_candidate_sector_version(text,text,date,timestamptz,timestamptz,text,uuid,date,text,text,uuid,text)',
      'public.insert_candidate_sector_entry(uuid,text,text,text,text,date,date,timestamptz,text)'
    ] LOOP
        EXECUTE 'ALTER FUNCTION ' || signature || ' OWNER TO migration_owner';
        EXECUTE 'REVOKE ALL ON FUNCTION ' || signature || ' FROM PUBLIC';
        EXECUTE 'GRANT EXECUTE ON FUNCTION ' || signature || ' TO research_writer';
    END LOOP;
END $typed_publishers$;

-- The repeated hash is deliberate: every observation is independently bound
-- to the exact READY/WARNING curated manifest it claims to represent.
CREATE FUNCTION public.candidate_source_validate_dataset_pin()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $validate$
DECLARE
    v_dataset_id text;
    v_first_date date;
    v_last_date date;
BEGIN
    v_dataset_id := CASE TG_TABLE_NAME
        WHEN 'candidate_universe_snapshots' THEN 'krx_kospi200_membership'
        WHEN 'candidate_investor_flows' THEN 'krx_investor_flows'
        WHEN 'candidate_market_status_observations' THEN 'krx_market_status'
        WHEN 'candidate_fundamental_observations' THEN 'krx_fundamentals'
        WHEN 'candidate_sector_versions' THEN 'krx_sector_classification'
        WHEN 'candidate_price_publications' THEN 'krx_eod_bars'
        ELSE '__invalid_candidate_source_table__'
    END;
    PERFORM 1
    FROM public.dataset_versions AS dataset
    WHERE dataset.id = NEW.dataset_version_id
      AND dataset.manifest_sha256 = NEW.manifest_sha256
      AND dataset.status IN ('READY', 'WARNING')
      AND dataset.dataset_id = v_dataset_id
      AND (
          TG_TABLE_NAME <> 'candidate_price_publications'
          OR dataset.version = (to_jsonb(NEW) ->> 'dataset_version')
      )
    FOR SHARE OF dataset;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate source requires a usable exact dataset pin'
            USING ERRCODE = '23514';
    END IF;
    v_first_date := CASE TG_TABLE_NAME
        WHEN 'candidate_price_publications' THEN (to_jsonb(NEW) ->> 'first_session')::date
        ELSE (to_jsonb(NEW) ->> 'entitlement_date')::date
    END;
    v_last_date := CASE
        WHEN TG_TABLE_NAME = 'candidate_price_publications' THEN
            (to_jsonb(NEW) ->> 'last_session')::date
        ELSE v_first_date
    END;
    IF NOT public.candidate_source_entitlement_is_valid(
        NEW.entitlement_id,
        NEW.license_ref,
        v_dataset_id,
        v_first_date,
        v_last_date
    ) THEN
        RAISE EXCEPTION 'candidate source requires an exact active candidate-use entitlement'
            USING ERRCODE = '42501';
    END IF;
    RETURN NEW;
END
$validate$;

ALTER FUNCTION public.candidate_source_validate_dataset_pin()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.candidate_source_validate_dataset_pin()
    FROM PUBLIC;

CREATE TRIGGER candidate_universe_dataset_pin
    BEFORE INSERT ON public.candidate_universe_snapshots
    FOR EACH ROW EXECUTE FUNCTION public.candidate_source_validate_dataset_pin();
CREATE TRIGGER candidate_market_status_dataset_pin
    BEFORE INSERT ON public.candidate_market_status_observations
    FOR EACH ROW EXECUTE FUNCTION public.candidate_source_validate_dataset_pin();
CREATE TRIGGER candidate_fundamental_dataset_pin
    BEFORE INSERT ON public.candidate_fundamental_observations
    FOR EACH ROW EXECUTE FUNCTION public.candidate_source_validate_dataset_pin();
CREATE TRIGGER candidate_sector_dataset_pin
    BEFORE INSERT ON public.candidate_sector_versions
    FOR EACH ROW EXECUTE FUNCTION public.candidate_source_validate_dataset_pin();
CREATE TRIGGER candidate_price_dataset_pin
    BEFORE INSERT ON public.candidate_price_publications
    FOR EACH ROW EXECUTE FUNCTION public.candidate_source_validate_dataset_pin();

CREATE FUNCTION public.candidate_universe_validate_members()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $validate$
DECLARE
    v_snapshot_id uuid;
    v_expected integer;
    v_available_at timestamptz;
    v_actual bigint;
BEGIN
    IF TG_TABLE_NAME = 'candidate_universe_snapshots' THEN
        IF TG_OP = 'DELETE' THEN
            v_snapshot_id := OLD.id;
        ELSE
            v_snapshot_id := NEW.id;
        END IF;
    ELSE
        IF TG_OP = 'DELETE' THEN
            v_snapshot_id := OLD.universe_snapshot_id;
        ELSE
            v_snapshot_id := NEW.universe_snapshot_id;
        END IF;
    END IF;

    SELECT snapshot.member_count, snapshot.available_at
    INTO v_expected, v_available_at
    FROM public.candidate_universe_snapshots AS snapshot
    WHERE snapshot.id = v_snapshot_id;
    IF NOT FOUND THEN
        RETURN NULL;
    END IF;

    SELECT count(*) INTO v_actual
    FROM public.candidate_universe_members AS member
    WHERE member.universe_snapshot_id = v_snapshot_id;
    IF v_actual <> v_expected THEN
        RAISE EXCEPTION 'candidate universe member count mismatch'
            USING ERRCODE = '23514';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM public.candidate_universe_members AS member
        WHERE member.universe_snapshot_id = v_snapshot_id
          AND member.available_at > v_available_at
    ) THEN
        RAISE EXCEPTION 'candidate universe contains a future-only member'
            USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END
$validate$;

ALTER FUNCTION public.candidate_universe_validate_members()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.candidate_universe_validate_members()
    FROM PUBLIC;

CREATE CONSTRAINT TRIGGER candidate_universe_snapshot_members_match
    AFTER INSERT OR UPDATE ON public.candidate_universe_snapshots
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION public.candidate_universe_validate_members();
CREATE CONSTRAINT TRIGGER candidate_universe_member_count_matches
    AFTER INSERT OR UPDATE OR DELETE ON public.candidate_universe_members
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION public.candidate_universe_validate_members();

-- Immutable source rows: serving roles cannot mutate them even if a later
-- grant is accidentally broadened. The migration owner retains explicit
-- maintenance capability for migrations and guarded rollback.
CREATE FUNCTION public.candidate_source_reject_mutation()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $guard$
BEGIN
    IF CURRENT_USER <> 'migration_owner' THEN
        RAISE EXCEPTION 'candidate source observations are append-only'
            USING ERRCODE = '42501';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END
$guard$;

ALTER FUNCTION public.candidate_source_reject_mutation() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.candidate_source_reject_mutation() FROM PUBLIC;

DO $raw_security$
DECLARE
    t text;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'candidate_raw_batch_publications',
        'candidate_raw_batch_datasets'
    ] LOOP
        EXECUTE format(
            'CREATE TRIGGER %I BEFORE UPDATE OR DELETE ON public.%I '
            || 'FOR EACH ROW EXECUTE FUNCTION public.candidate_source_reject_mutation()',
            t || '_immutable', t
        );
        EXECUTE format('ALTER TABLE public.%I OWNER TO migration_owner', t);
        EXECUTE format('ALTER TABLE public.%I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('ALTER TABLE public.%I FORCE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY %I ON public.%I FOR SELECT TO worker, admin, research_writer USING (true)',
            'candidate_source_select_' || t, t
        );
        EXECUTE format(
            'CREATE POLICY %I ON public.%I FOR ALL TO migration_owner USING (true) WITH CHECK (true)',
            'candidate_source_owner_' || t, t
        );
        EXECUTE format(
            'REVOKE ALL ON TABLE public.%I FROM PUBLIC, app, worker, admin, audit_writer, research_writer',
            t
        );
        EXECUTE format('GRANT SELECT ON TABLE public.%I TO worker, admin, research_writer', t);
    END LOOP;
END
$raw_security$;

DO $triggers$
DECLARE
    t text;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'candidate_universe_snapshots',
        'candidate_universe_members',
        'candidate_investor_flows',
        'candidate_investor_flow_snapshot_rows',
        'candidate_market_status_observations',
        'candidate_fundamental_observations',
        'candidate_sector_versions',
        'candidate_sector_entries'
    ] LOOP
        EXECUTE format(
            'CREATE TRIGGER %I BEFORE UPDATE OR DELETE ON public.%I '
            || 'FOR EACH ROW EXECUTE FUNCTION public.candidate_source_reject_mutation()',
            t || '_immutable',
            t
        );
        EXECUTE format('ALTER TABLE public.%I OWNER TO migration_owner', t);
        EXECUTE format('ALTER TABLE public.%I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('ALTER TABLE public.%I FORCE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY %I ON public.%I FOR SELECT TO worker, admin, research_writer USING (true)',
            'candidate_source_select_' || t,
            t
        );
        EXECUTE format(
            'CREATE POLICY %I ON public.%I FOR ALL TO migration_owner USING (true) WITH CHECK (true)',
            'candidate_source_owner_' || t,
            t
        );
        EXECUTE format(
            'REVOKE ALL ON TABLE public.%I FROM PUBLIC, app, worker, admin, audit_writer, research_writer',
            t
        );
        EXECUTE format(
            'GRANT SELECT ON TABLE public.%I TO worker, admin, research_writer',
            t
        );
    END LOOP;
END
$triggers$;

-- Price publication and its per-instrument coverage are one procedure-owned
-- append-only aggregate. research_writer may read the attestation but has no
-- direct INSERT privilege with which it could create a parent without its
-- complete coverage evidence (or attach evidence to the wrong parent).
DO $price_security$
DECLARE
    t text;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'candidate_price_publications',
        'candidate_price_instrument_coverage',
        'candidate_price_instrument_sessions'
    ] LOOP
        EXECUTE format(
            'CREATE TRIGGER %I BEFORE UPDATE OR DELETE ON public.%I '
            || 'FOR EACH ROW EXECUTE FUNCTION public.candidate_source_reject_mutation()',
            t || '_immutable',
            t
        );
        EXECUTE format('ALTER TABLE public.%I OWNER TO migration_owner', t);
        EXECUTE format('ALTER TABLE public.%I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('ALTER TABLE public.%I FORCE ROW LEVEL SECURITY', t);
        EXECUTE format(
            'CREATE POLICY %I ON public.%I FOR SELECT TO worker, admin, research_writer USING (true)',
            'candidate_source_select_' || t,
            t
        );
        EXECUTE format(
            'CREATE POLICY %I ON public.%I FOR ALL TO migration_owner USING (true) WITH CHECK (true)',
            'candidate_source_owner_' || t,
            t
        );
        EXECUTE format(
            'REVOKE ALL ON TABLE public.%I FROM PUBLIC, app, worker, admin, audit_writer, research_writer',
            t
        );
        EXECUTE format(
            'GRANT SELECT ON TABLE public.%I TO worker, admin, research_writer',
            t
        );
    END LOOP;
END
$price_security$;

CREATE TRIGGER candidate_instrument_registrations_immutable
    BEFORE UPDATE OR DELETE ON public.candidate_instrument_registrations
    FOR EACH ROW EXECUTE FUNCTION public.candidate_source_reject_mutation();
ALTER TABLE public.candidate_instrument_registrations OWNER TO migration_owner;
ALTER TABLE public.candidate_instrument_registrations ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.candidate_instrument_registrations FORCE ROW LEVEL SECURITY;
CREATE POLICY candidate_instrument_registrations_select
    ON public.candidate_instrument_registrations
    FOR SELECT TO worker, admin, research_writer
    USING (true);
CREATE POLICY candidate_instrument_registrations_owner
    ON public.candidate_instrument_registrations
    FOR ALL TO migration_owner
    USING (true) WITH CHECK (true);
REVOKE ALL ON TABLE public.candidate_instrument_registrations
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT SELECT ON TABLE public.candidate_instrument_registrations
    TO worker, admin, research_writer;
