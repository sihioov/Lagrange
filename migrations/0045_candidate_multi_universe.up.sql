-- 0045: Candidate multi-universe identity and source-binding boundary.
--
-- 0042--0044 remain immutable.  This migration adds the registry and makes
-- the existing source/run/feed identities universe-aware while retaining all
-- public function signatures for rolling deployment compatibility.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Serialize this DDL with the candidate scheduler's fence.  The scheduler
-- function takes the shared form of this lock before inspecting its control
-- row, so no schedule can observe a half-transitioned identity.
SELECT pg_catalog.pg_advisory_xact_lock(1815099521, 44);

CREATE TABLE public.candidate_universe_registry (
    universe_key          text PRIMARY KEY,
    membership_dataset_id text NOT NULL UNIQUE,
    display_name          text NOT NULL,
    market                text NOT NULL CHECK (market = 'kr'),
    sort_order            integer NOT NULL UNIQUE CHECK (sort_order > 0),
    enabled               boolean NOT NULL,
    created_at            timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK (universe_key ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    CHECK (membership_dataset_id ~ '^krx_[a-z0-9_]+_membership$')
);

INSERT INTO public.candidate_universe_registry (
    universe_key, membership_dataset_id, display_name, market, sort_order, enabled
)
VALUES
    ('kospi200', 'krx_kospi200_membership', 'KOSPI 200', 'kr', 10, true),
    ('kosdaq150', 'krx_kosdaq150_membership', 'KOSDAQ 150', 'kr', 20, true);

ALTER TABLE public.candidate_universe_registry OWNER TO migration_owner;
ALTER TABLE public.candidate_universe_registry ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.candidate_universe_registry FORCE ROW LEVEL SECURITY;
REVOKE ALL ON TABLE public.candidate_universe_registry
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT SELECT ON TABLE public.candidate_universe_registry
    TO app, worker, admin, research_writer;
CREATE POLICY candidate_universe_registry_select_app
    ON public.candidate_universe_registry FOR SELECT TO app USING (true);
CREATE POLICY candidate_universe_registry_select_worker
    ON public.candidate_universe_registry FOR SELECT TO worker USING (true);
CREATE POLICY candidate_universe_registry_select_admin
    ON public.candidate_universe_registry FOR SELECT TO admin USING (true);
CREATE POLICY candidate_universe_registry_select_research_writer
    ON public.candidate_universe_registry FOR SELECT TO research_writer USING (true);
CREATE POLICY candidate_universe_registry_owner_all
    ON public.candidate_universe_registry FOR ALL TO migration_owner
    USING (true) WITH CHECK (true);

-- The Raw binding identity now contains the logical dataset.  Existing rows
-- are backfilled from the immutable dataset_versions row before constraints
-- are tightened; no 0042 row is rewritten beyond this denormalized key.
ALTER TABLE public.candidate_raw_batch_datasets
    ADD COLUMN dataset_id text;
UPDATE public.candidate_raw_batch_datasets AS binding
SET dataset_id = dataset.dataset_id
FROM public.dataset_versions AS dataset
WHERE dataset.id = binding.dataset_version_id;
ALTER TABLE public.candidate_raw_batch_datasets
    ALTER COLUMN dataset_id SET NOT NULL;
ALTER TABLE public.candidate_raw_batch_datasets
    ADD CONSTRAINT candidate_raw_dataset_id_check
    CHECK (dataset_id ~ '^krx_[a-z0-9_]+$');
ALTER TABLE public.candidate_raw_batch_datasets
    DROP CONSTRAINT candidate_raw_batch_datasets_pkey;
ALTER TABLE public.candidate_raw_batch_datasets
    ADD CONSTRAINT candidate_raw_batch_datasets_pkey
    PRIMARY KEY (batch_id, surface, dataset_id);
DROP INDEX public.candidate_raw_batch_dataset_exact_idx;
CREATE UNIQUE INDEX candidate_raw_batch_dataset_exact_idx
    ON public.candidate_raw_batch_datasets
    (batch_id, surface, response_kind, dataset_id);

CREATE FUNCTION public.candidate_raw_dataset_id_matches()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $raw_identity$
DECLARE
    v_dataset_id text;
BEGIN
    SELECT dataset.dataset_id
      INTO v_dataset_id
      FROM public.dataset_versions AS dataset
     WHERE dataset.id = NEW.dataset_version_id;
    IF v_dataset_id IS NULL OR NEW.dataset_id IS DISTINCT FROM v_dataset_id THEN
        RAISE EXCEPTION 'candidate Raw binding dataset identity does not match dataset_versions'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END
$raw_identity$;
ALTER FUNCTION public.candidate_raw_dataset_id_matches() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.candidate_raw_dataset_id_matches() FROM PUBLIC;
CREATE TRIGGER candidate_raw_dataset_id_identity
    BEFORE INSERT OR UPDATE ON public.candidate_raw_batch_datasets
    FOR EACH ROW EXECUTE FUNCTION public.candidate_raw_dataset_id_matches();

-- Every published universe snapshot is tied to a registry key.  The explicit
-- (id,index_id) key is needed by the composite run lineage foreign key.
ALTER TABLE public.candidate_universe_snapshots
    ADD CONSTRAINT candidate_universe_registry_fk
    FOREIGN KEY (index_id) REFERENCES public.candidate_universe_registry(universe_key);
ALTER TABLE public.candidate_universe_snapshots
    ADD CONSTRAINT candidate_universe_id_index_uq UNIQUE (id, index_id);

ALTER TABLE public.stock_analysis_runs
    ADD COLUMN universe_key text;
UPDATE public.stock_analysis_runs AS run
SET universe_key = snapshot.index_id
FROM public.candidate_universe_snapshots AS snapshot
WHERE snapshot.id = run.universe_snapshot_id;
ALTER TABLE public.stock_analysis_runs
    ALTER COLUMN universe_key SET NOT NULL;
ALTER TABLE public.stock_analysis_runs
    ADD CONSTRAINT stock_analysis_run_universe_key_check
    CHECK (universe_key ~ '^[a-z0-9][a-z0-9._-]{0,63}$');
ALTER TABLE public.stock_analysis_runs
    ADD CONSTRAINT stock_analysis_run_universe_fk
    FOREIGN KEY (universe_key) REFERENCES public.candidate_universe_registry(universe_key);
ALTER TABLE public.stock_analysis_runs
    DROP CONSTRAINT stock_analysis_run_date_seq_key;
ALTER TABLE public.stock_analysis_runs
    ADD CONSTRAINT stock_analysis_run_date_seq_key
    UNIQUE (universe_key, as_of_date, computation_seq);
ALTER TABLE public.stock_analysis_runs
    ADD CONSTRAINT stock_analysis_run_id_universe_uq UNIQUE (id, universe_key);
ALTER TABLE public.stock_analysis_runs
    DROP CONSTRAINT stock_analysis_runs_universe_snapshot_id_fkey;
ALTER TABLE public.stock_analysis_runs
    ADD CONSTRAINT stock_analysis_run_snapshot_universe_fk
    FOREIGN KEY (universe_snapshot_id, universe_key)
    REFERENCES public.candidate_universe_snapshots(id, index_id);

DROP INDEX public.stock_analysis_runs_latest_idx;
CREATE INDEX stock_analysis_runs_latest_idx
    ON public.stock_analysis_runs
    (universe_key, as_of_date DESC, computation_seq DESC)
    WHERE status = 'SUCCEEDED';

ALTER TABLE public.candidate_feed_snapshots
    ADD COLUMN universe_key text;
UPDATE public.candidate_feed_snapshots AS feed
SET universe_key = run.universe_key
FROM public.stock_analysis_runs AS run
WHERE run.id = feed.run_id;
ALTER TABLE public.candidate_feed_snapshots
    ALTER COLUMN universe_key SET NOT NULL;
ALTER TABLE public.candidate_feed_snapshots
    ADD CONSTRAINT candidate_feed_universe_key_check
    CHECK (universe_key ~ '^[a-z0-9][a-z0-9._-]{0,63}$');
ALTER TABLE public.candidate_feed_snapshots
    ADD CONSTRAINT candidate_feed_universe_fk
    FOREIGN KEY (universe_key) REFERENCES public.candidate_universe_registry(universe_key);
ALTER TABLE public.candidate_feed_snapshots
    DROP CONSTRAINT candidate_feed_date_seq_key;
ALTER TABLE public.candidate_feed_snapshots
    ADD CONSTRAINT candidate_feed_date_seq_key
    UNIQUE (universe_key, as_of_date, computation_seq);
ALTER TABLE public.candidate_feed_snapshots
    DROP CONSTRAINT candidate_feed_snapshots_run_id_fkey;
ALTER TABLE public.candidate_feed_snapshots
    ADD CONSTRAINT candidate_feed_run_universe_fk
    FOREIGN KEY (run_id, universe_key)
    REFERENCES public.stock_analysis_runs(id, universe_key);

DROP INDEX public.candidate_feed_active_date_uq;
CREATE UNIQUE INDEX candidate_feed_active_date_uq
    ON public.candidate_feed_snapshots(universe_key, as_of_date)
    WHERE status = 'PUBLISHED';
DROP INDEX public.candidate_feed_latest_idx;
CREATE INDEX candidate_feed_latest_idx
    ON public.candidate_feed_snapshots
    (universe_key, as_of_date DESC, computation_seq DESC)
    WHERE status = 'PUBLISHED';

-- The old helper remains valid for a single dataset check.  Contract
-- resolution now requires every common source and every enabled registry
-- membership dataset in one exact candidate-use entitlement.
CREATE OR REPLACE FUNCTION public.resolve_candidate_contract_entitlement(
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
             FROM (
                 VALUES
                     ('krx_eod_bars'::text),
                     ('krx_investor_flows'::text),
                     ('krx_market_status'::text),
                     ('krx_fundamentals'::text),
                     ('krx_sector_classification'::text)
                 UNION ALL
                 SELECT registry.membership_dataset_id
                   FROM public.candidate_universe_registry AS registry
                  WHERE registry.enabled
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
ALTER FUNCTION public.resolve_candidate_contract_entitlement(text,date,date)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.resolve_candidate_contract_entitlement(text,date,date)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.resolve_candidate_contract_entitlement(text,date,date)
    TO research_writer;

-- Source catalog accepts only the fixed common source set plus enabled
-- registry membership datasets.
CREATE OR REPLACE FUNCTION public.register_candidate_source_dataset(
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
    IF p_dataset_id IS NULL
       OR (
           p_dataset_id NOT IN (
               'krx_investor_flows',
               'krx_market_status',
               'krx_fundamentals',
               'krx_sector_classification'
           )
           AND NOT EXISTS (
               SELECT 1 FROM public.candidate_universe_registry AS registry
                WHERE registry.membership_dataset_id = p_dataset_id
                  AND registry.enabled
           )
       )
       OR p_dataset_version IS NULL
       OR p_dataset_version !~ '^[a-z0-9:_-]{1,128}$'
       OR p_manifest_sha256 !~ '^[0-9a-f]{64}$'
    THEN
        RAISE EXCEPTION 'invalid candidate source dataset catalog entry'
            USING ERRCODE = '23514';
    END IF;
    v_expected_storage_path := 'db://candidate/' || p_dataset_id || '/' || p_dataset_version;
    IF NOT public.candidate_source_entitlement_is_valid(
        p_entitlement_id, p_contract_reference, p_dataset_id,
        p_entitlement_date, p_entitlement_date
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
ALTER FUNCTION public.register_candidate_source_dataset(text,text,text,uuid,text,date)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.register_candidate_source_dataset(text,text,text,uuid,text,date)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.register_candidate_source_dataset(text,text,text,uuid,text,date)
    TO research_writer;

CREATE OR REPLACE FUNCTION public.bind_candidate_raw_dataset(
    p_batch_id uuid, p_surface text, p_response_kind text,
    p_dataset_version_id uuid, p_reused_existing boolean
)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $raw_bind$
DECLARE
    v_dataset_id text;
    v_expected_dataset text;
BEGIN
    SELECT dataset.dataset_id
      INTO v_dataset_id
      FROM public.dataset_versions AS dataset
     WHERE dataset.id = p_dataset_version_id
       AND dataset.status IN ('READY', 'WARNING');
    v_expected_dataset := CASE p_response_kind
        WHEN 'bars' THEN 'krx_eod_bars'
        WHEN 'investor_flow' THEN 'krx_investor_flows'
        WHEN 'market_status' THEN 'krx_market_status'
        WHEN 'fundamentals' THEN 'krx_fundamentals'
        WHEN 'sector_classification' THEN 'krx_sector_classification'
        WHEN 'index_membership' THEN v_dataset_id
        ELSE NULL
    END;
    IF p_surface NOT IN ('source', 'price')
       OR v_dataset_id IS NULL
       OR v_expected_dataset IS NULL
       OR (p_surface = 'source' AND p_response_kind = 'bars')
       OR (p_surface = 'price' AND p_response_kind <> 'bars')
       OR (p_response_kind <> 'index_membership' AND v_dataset_id <> v_expected_dataset)
       OR (p_response_kind = 'index_membership' AND NOT EXISTS (
           SELECT 1 FROM public.candidate_universe_registry AS registry
            WHERE registry.membership_dataset_id = v_dataset_id
              AND registry.enabled
       ))
       OR (p_reused_existing AND p_response_kind NOT IN (
           'fundamentals', 'index_membership', 'sector_classification'
       ))
       OR NOT EXISTS (
           SELECT 1
             FROM public.candidate_raw_batch_publications AS batch
            WHERE batch.batch_id = p_batch_id
              AND batch.surface = p_surface
              AND batch.state IN ('CATALOGED', 'PUBLISHED')
       )
    THEN
        RAISE EXCEPTION 'candidate Raw dataset binding is invalid' USING ERRCODE = '23514';
    END IF;

    IF EXISTS (
        SELECT 1 FROM public.candidate_raw_batch_publications AS batch
         WHERE batch.batch_id = p_batch_id
           AND batch.surface = p_surface
           AND batch.state = 'PUBLISHED'
    ) THEN
        IF EXISTS (
            SELECT 1
              FROM public.candidate_raw_batch_datasets AS binding
             WHERE binding.batch_id = p_batch_id
               AND binding.surface = p_surface
               AND binding.dataset_id = v_dataset_id
               AND binding.response_kind = p_response_kind
               AND binding.dataset_version_id = p_dataset_version_id
               AND binding.reused_existing = p_reused_existing
        ) THEN
            RETURN;
        END IF;
        RAISE EXCEPTION 'published candidate Raw batch cannot gain another dataset binding'
            USING ERRCODE = '23514';
    END IF;

    IF p_reused_existing AND NOT EXISTS (
        SELECT 1
          FROM public.candidate_raw_batch_datasets AS origin
          JOIN public.candidate_raw_batch_publications AS origin_batch
            ON origin_batch.batch_id = origin.batch_id
           AND origin_batch.surface = origin.surface
         WHERE origin.dataset_version_id = p_dataset_version_id
           AND origin.dataset_id = v_dataset_id
           AND origin.response_kind = p_response_kind
           AND NOT origin.reused_existing
           AND origin_batch.state = 'PUBLISHED'
    ) THEN
        RAISE EXCEPTION 'candidate reusable dataset has no sealed immutable origin'
            USING ERRCODE = '23514';
    END IF;
    IF NOT p_reused_existing AND EXISTS (
        SELECT 1
          FROM public.candidate_raw_batch_datasets AS origin
         WHERE origin.dataset_version_id = p_dataset_version_id
           AND NOT origin.reused_existing
           AND (origin.batch_id <> p_batch_id OR origin.surface <> p_surface)
    ) THEN
        RAISE EXCEPTION 'candidate dataset already belongs to another immutable origin'
            USING ERRCODE = '23514';
    END IF;

    INSERT INTO public.candidate_raw_batch_datasets (
        batch_id, surface, response_kind, dataset_id,
        dataset_version_id, reused_existing
    )
    VALUES (
        p_batch_id, p_surface, p_response_kind, v_dataset_id,
        p_dataset_version_id, p_reused_existing
    )
    ON CONFLICT (batch_id, surface, dataset_id) DO NOTHING;
    IF NOT EXISTS (
        SELECT 1
          FROM public.candidate_raw_batch_datasets AS binding
         WHERE binding.batch_id = p_batch_id
           AND binding.surface = p_surface
           AND binding.dataset_id = v_dataset_id
           AND binding.response_kind = p_response_kind
           AND binding.dataset_version_id = p_dataset_version_id
           AND binding.reused_existing = p_reused_existing
    ) THEN
        RAISE EXCEPTION 'candidate Raw dataset binding replay conflicts'
            USING ERRCODE = '23514';
    END IF;
END
$raw_bind$;
ALTER FUNCTION public.bind_candidate_raw_dataset(uuid,text,text,uuid,boolean)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.bind_candidate_raw_dataset(uuid,text,text,uuid,boolean)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.bind_candidate_raw_dataset(uuid,text,text,uuid,boolean)
    TO research_writer;

CREATE OR REPLACE FUNCTION public.seal_candidate_raw_batch(
    p_batch_id uuid, p_surface text, p_raw_manifest_sha256 text, p_fetch_mode text
)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $raw_seal$
DECLARE
    v_expected_count bigint;
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM public.candidate_raw_batch_publications AS batch
         WHERE batch.batch_id = p_batch_id
           AND batch.surface = p_surface
           AND batch.raw_manifest_sha256 = p_raw_manifest_sha256
           AND batch.fetch_mode = p_fetch_mode
           AND batch.state IN ('CATALOGED', 'PUBLISHED')
    ) THEN
        RAISE EXCEPTION 'candidate Raw batch cannot be sealed' USING ERRCODE = '23514';
    END IF;

    IF p_surface = 'source' THEN
        SELECT 4 + count(*) INTO v_expected_count
          FROM public.candidate_universe_registry AS registry
         WHERE registry.enabled;
        IF (
            (SELECT count(*)
               FROM public.candidate_raw_batch_datasets AS binding
              WHERE binding.batch_id = p_batch_id AND binding.surface = 'source')
              <> v_expected_count
            OR EXISTS (
                SELECT 1
                  FROM (
                      VALUES
                          ('investor_flow'::text, 'krx_investor_flows'::text),
                          ('market_status'::text, 'krx_market_status'::text),
                          ('fundamentals'::text, 'krx_fundamentals'::text),
                          ('sector_classification'::text, 'krx_sector_classification'::text)
                      UNION ALL
                      SELECT 'index_membership', registry.membership_dataset_id
                        FROM public.candidate_universe_registry AS registry
                       WHERE registry.enabled
                  ) AS required(response_kind, dataset_id)
                 WHERE NOT EXISTS (
                     SELECT 1
                       FROM public.candidate_raw_batch_datasets AS binding
                      WHERE binding.batch_id = p_batch_id
                        AND binding.surface = 'source'
                        AND binding.response_kind = required.response_kind
                        AND binding.dataset_id = required.dataset_id
                 )
            )
            OR EXISTS (
                SELECT 1
                  FROM public.candidate_raw_batch_datasets AS binding
                 WHERE binding.batch_id = p_batch_id
                   AND binding.surface = 'source'
                   AND NOT EXISTS (
                       SELECT 1
                         FROM (
                             VALUES
                                 ('investor_flow'::text, 'krx_investor_flows'::text),
                                 ('market_status'::text, 'krx_market_status'::text),
                                 ('fundamentals'::text, 'krx_fundamentals'::text),
                                 ('sector_classification'::text, 'krx_sector_classification'::text)
                             UNION ALL
                             SELECT 'index_membership', registry.membership_dataset_id
                               FROM public.candidate_universe_registry AS registry
                              WHERE registry.enabled
                         ) AS required(response_kind, dataset_id)
                        WHERE required.response_kind = binding.response_kind
                          AND required.dataset_id = binding.dataset_id
                   )
            )
            OR NOT EXISTS (
                SELECT 1
                  FROM public.candidate_raw_batch_datasets AS binding
                  JOIN public.candidate_investor_flow_snapshot_rows AS member
                    ON member.dataset_version_id = binding.dataset_version_id
                  JOIN public.candidate_investor_flows AS flow
                    ON flow.id = member.flow_observation_id
                  JOIN public.candidate_raw_batch_publications AS batch
                    ON batch.batch_id = binding.batch_id AND batch.surface = binding.surface
                 WHERE binding.batch_id = p_batch_id AND binding.surface = 'source'
                   AND binding.response_kind = 'investor_flow'
                   AND flow.trade_date = batch.entitlement_date
            )
            OR NOT EXISTS (
                SELECT 1
                  FROM public.candidate_raw_batch_datasets AS binding
                  JOIN public.candidate_market_status_observations AS status
                    ON status.dataset_version_id = binding.dataset_version_id
                  JOIN public.candidate_raw_batch_publications AS batch
                    ON batch.batch_id = binding.batch_id AND batch.surface = binding.surface
                 WHERE binding.batch_id = p_batch_id AND binding.surface = 'source'
                   AND binding.response_kind = 'market_status'
                   AND status.trade_date = batch.entitlement_date
            )
            OR NOT EXISTS (
                SELECT 1
                  FROM public.candidate_raw_batch_datasets AS binding
                  JOIN public.candidate_fundamental_observations AS fundamental
                    ON fundamental.dataset_version_id = binding.dataset_version_id
                 WHERE binding.batch_id = p_batch_id AND binding.surface = 'source'
                   AND binding.response_kind = 'fundamentals'
            )
            OR EXISTS (
                SELECT 1
                  FROM public.candidate_universe_registry AS registry
                 WHERE registry.enabled
                   AND NOT EXISTS (
                       SELECT 1
                         FROM public.candidate_raw_batch_datasets AS binding
                         JOIN public.candidate_universe_snapshots AS snapshot
                           ON snapshot.dataset_version_id = binding.dataset_version_id
                          AND snapshot.index_id = registry.universe_key
                         JOIN public.candidate_universe_members AS member
                           ON member.universe_snapshot_id = snapshot.id
                        WHERE binding.batch_id = p_batch_id
                          AND binding.surface = 'source'
                          AND binding.response_kind = 'index_membership'
                          AND binding.dataset_id = registry.membership_dataset_id
                          AND snapshot.member_count > 0
                          AND snapshot.member_count = (
                              SELECT count(*)
                                FROM public.candidate_universe_members AS exact_member
                               WHERE exact_member.universe_snapshot_id = snapshot.id
                          )
                   )
            )
            OR NOT EXISTS (
                SELECT 1
                  FROM public.candidate_raw_batch_datasets AS binding
                  JOIN public.candidate_sector_versions AS version
                    ON version.dataset_version_id = binding.dataset_version_id
                  JOIN public.candidate_sector_entries AS entry
                    ON entry.sector_version_id = version.id
                 WHERE binding.batch_id = p_batch_id AND binding.surface = 'source'
                   AND binding.response_kind = 'sector_classification'
            )
            OR EXISTS (
                SELECT 1
                  FROM public.candidate_raw_batch_datasets AS reused
                 WHERE reused.batch_id = p_batch_id
                   AND reused.surface = 'source'
                   AND reused.reused_existing
                   AND NOT EXISTS (
                       SELECT 1
                         FROM public.candidate_raw_batch_datasets AS origin
                         JOIN public.candidate_raw_batch_publications AS origin_batch
                           ON origin_batch.batch_id = origin.batch_id
                          AND origin_batch.surface = origin.surface
                        WHERE origin.surface = 'source'
                          AND origin.dataset_id = reused.dataset_id
                          AND origin.response_kind = reused.response_kind
                          AND origin.dataset_version_id = reused.dataset_version_id
                          AND NOT origin.reused_existing
                          AND origin_batch.state = 'PUBLISHED'
                          AND origin.batch_id <> p_batch_id
                   )
            )
        ) THEN
            RAISE EXCEPTION 'candidate source batch is incomplete and cannot be sealed'
                USING ERRCODE = '23514';
        END IF;
    END IF;

    IF p_surface = 'price' AND NOT EXISTS (
        SELECT 1
          FROM public.candidate_raw_batch_datasets AS binding
          JOIN public.candidate_price_publications AS price
            ON price.dataset_version_id = binding.dataset_version_id
         WHERE binding.batch_id = p_batch_id
           AND binding.surface = 'price'
           AND binding.response_kind = 'bars'
    ) THEN
        RAISE EXCEPTION 'candidate price batch is incomplete and cannot be sealed'
            USING ERRCODE = '23514';
    END IF;
    UPDATE public.candidate_raw_batch_publications
       SET state = 'PUBLISHED', published_at = COALESCE(published_at, clock_timestamp())
     WHERE batch_id = p_batch_id AND surface = p_surface AND state = 'CATALOGED';
END
$raw_seal$;
ALTER FUNCTION public.seal_candidate_raw_batch(uuid,text,text,text)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.seal_candidate_raw_batch(uuid,text,text,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.seal_candidate_raw_batch(uuid,text,text,text)
    TO research_writer;

CREATE OR REPLACE FUNCTION public.block_candidate_raw_batch_for_inactive_rights(
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
            USING ERRCODE = '23514';
    END IF;
    IF EXISTS (
        SELECT 1 FROM public.data_entitlements AS entitlement
         WHERE entitlement.contract_reference = p_entitlement_reference
           AND entitlement.status = 'ACTIVE'
           AND entitlement.covered_uses @> '["candidate"]'::jsonb
           AND entitlement.effective_from <= p_first_date
           AND entitlement.effective_until >= p_last_date
           AND NOT EXISTS (
               SELECT 1
                 FROM (
                     VALUES
                         ('krx_eod_bars'::text),
                         ('krx_investor_flows'::text),
                         ('krx_market_status'::text),
                         ('krx_fundamentals'::text),
                         ('krx_sector_classification'::text)
                     UNION ALL
                     SELECT registry.membership_dataset_id
                       FROM public.candidate_universe_registry AS registry
                      WHERE registry.enabled
                 ) AS required(dataset_id)
                WHERE NOT entitlement.covered_datasets
                          @> pg_catalog.jsonb_build_array(required.dataset_id)
           )
    ) THEN
        RAISE EXCEPTION 'active rights cannot be recorded as a terminal Raw block'
            USING ERRCODE = '23514';
    END IF;
    UPDATE public.candidate_raw_batch_publications
       SET state = 'BLOCKED', reason_code = 'ENTITLEMENT_INACTIVE', published_at = NULL
     WHERE batch_id = p_batch_id AND surface = p_surface AND state = 'CATALOGED'
       AND raw_manifest_sha256 = p_raw_manifest_sha256
       AND fetch_mode = p_fetch_mode
       AND entitlement_reference = p_entitlement_reference
       AND entitlement_date = p_entitlement_date;
    INSERT INTO public.candidate_raw_batch_publications (
        batch_id, surface, raw_manifest_sha256, fetch_mode,
        entitlement_reference, entitlement_date, state, reason_code
    )
    VALUES (
        p_batch_id, p_surface, p_raw_manifest_sha256, p_fetch_mode,
        p_entitlement_reference, p_entitlement_date, 'BLOCKED', 'ENTITLEMENT_INACTIVE'
    )
    ON CONFLICT (batch_id, surface) DO NOTHING;
    IF NOT EXISTS (
        SELECT 1 FROM public.candidate_raw_batch_publications AS batch
         WHERE batch.batch_id = p_batch_id AND batch.surface = p_surface
           AND batch.raw_manifest_sha256 = p_raw_manifest_sha256
           AND batch.fetch_mode = p_fetch_mode
           AND batch.entitlement_reference = p_entitlement_reference
           AND batch.entitlement_date = p_entitlement_date
           AND batch.state = 'BLOCKED'
           AND batch.reason_code = 'ENTITLEMENT_INACTIVE'
    ) THEN
        RAISE EXCEPTION 'candidate Raw terminal block conflicts with durable state'
            USING ERRCODE = '23514';
    END IF;
END
$raw_block$;
ALTER FUNCTION public.block_candidate_raw_batch_for_inactive_rights(
    uuid,text,text,text,text,date,date,date
) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.block_candidate_raw_batch_for_inactive_rights(
    uuid,text,text,text,text,date,date,date
) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.block_candidate_raw_batch_for_inactive_rights(
    uuid,text,text,text,text,date,date,date
) TO research_writer;

CREATE OR REPLACE FUNCTION public.candidate_source_dataset_write_matches(
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
SET search_path = pg_catalog
AS $gate$
    SELECT EXISTS (
        SELECT 1
          FROM public.candidate_raw_batch_datasets AS binding
          JOIN public.candidate_raw_batch_publications AS batch
            ON batch.batch_id = binding.batch_id AND batch.surface = binding.surface
          JOIN public.dataset_versions AS dataset
            ON dataset.id = binding.dataset_version_id
          JOIN public.data_entitlements AS entitlement
            ON entitlement.id = p_entitlement_id
         WHERE binding.dataset_version_id = p_dataset_version_id
           AND binding.surface = 'source'
           AND NOT binding.reused_existing
           AND binding.response_kind = p_response_kind
           AND batch.state = 'CATALOGED'
           AND batch.batch_id = NULLIF(
               current_setting('app.candidate_raw_batch_id', true), ''
           )::uuid
           AND batch.entitlement_reference = p_license_ref
           AND batch.entitlement_date = p_entitlement_date
           AND p_provider = 'krx'
           AND dataset.manifest_sha256 = p_manifest_sha256
           AND dataset.dataset_id = CASE p_response_kind
               WHEN 'investor_flow' THEN 'krx_investor_flows'
               WHEN 'market_status' THEN 'krx_market_status'
               WHEN 'fundamentals' THEN 'krx_fundamentals'
               WHEN 'sector_classification' THEN 'krx_sector_classification'
               WHEN 'index_membership' THEN dataset.dataset_id
               ELSE NULL
           END
           AND (
               p_response_kind <> 'index_membership'
               OR EXISTS (
                   SELECT 1
                     FROM public.candidate_universe_registry AS registry
                    WHERE registry.membership_dataset_id = dataset.dataset_id
                      AND registry.enabled
               )
           )
           AND entitlement.contract_reference = p_license_ref
           AND entitlement.status = 'ACTIVE'
           AND entitlement.covered_uses @> '["candidate"]'::jsonb
           AND entitlement.covered_datasets
                   @> pg_catalog.jsonb_build_array(dataset.dataset_id)
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

CREATE OR REPLACE FUNCTION public.insert_candidate_universe_snapshot(
    date,uuid,text,text,uuid,date,text,text,timestamptz,timestamptz,integer
)
RETURNS uuid
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $insert$
DECLARE
    v_id uuid;
    v_universe_key text;
BEGIN
    SELECT registry.universe_key
      INTO v_universe_key
      FROM public.dataset_versions AS dataset
      JOIN public.candidate_universe_registry AS registry
        ON registry.membership_dataset_id = dataset.dataset_id
       AND registry.enabled
     WHERE dataset.id = $2;
    IF v_universe_key IS NULL
       OR $1 > $6
       OR NOT public.candidate_source_dataset_write_matches(
           $2, 'index_membership', $4, $5, $6, $7, $3
       )
    THEN
        RAISE EXCEPTION 'candidate source dataset is not open for typed publication'
            USING ERRCODE = '42501';
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(
        pg_catalog.jsonb_build_array(
            'candidate-universe-snapshot', v_universe_key, $1, $8
        )::text, 0
    ));
    IF EXISTS (
        SELECT 1
          FROM public.candidate_universe_snapshots AS existing
         WHERE existing.index_id = v_universe_key
           AND existing.as_of_date = $1
           AND existing.source_revision = $8
           AND ROW(existing.available_at, existing.provider, existing.member_count)
               IS DISTINCT FROM ROW($9, $4, $11)
    ) THEN
        RAISE EXCEPTION 'candidate universe snapshot natural identity is occupied by different content'
            USING ERRCODE = '23514';
    END IF;
    INSERT INTO public.candidate_universe_snapshots (
        index_id, as_of_date, dataset_version_id, manifest_sha256, provider,
        entitlement_id, entitlement_date, license_ref, source_revision,
        available_at, retrieved_at, member_count
    )
    VALUES (
        v_universe_key, $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11
    )
    ON CONFLICT (index_id, as_of_date, dataset_version_id)
    DO NOTHING RETURNING id INTO v_id;
    IF v_id IS NULL AND NOT EXISTS (
        SELECT 1
          FROM public.candidate_universe_snapshots AS existing
         WHERE existing.index_id = v_universe_key
           AND existing.as_of_date = $1
           AND existing.dataset_version_id = $2
           AND existing.manifest_sha256 = $3
           AND existing.provider = $4
           AND existing.entitlement_id = $5
           AND existing.entitlement_date = $6
           AND existing.license_ref = $7
           AND existing.source_revision = $8
           AND existing.available_at = $9
           AND existing.retrieved_at = $10
           AND existing.member_count = $11
    ) THEN
        RAISE EXCEPTION 'candidate universe snapshot replay conflicts'
            USING ERRCODE = '23514';
    END IF;
    RETURN v_id;
END
$insert$;
ALTER FUNCTION public.insert_candidate_universe_snapshot(
    date,uuid,text,text,uuid,date,text,text,timestamptz,timestamptz,integer
) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.insert_candidate_universe_snapshot(
    date,uuid,text,text,uuid,date,text,text,timestamptz,timestamptz,integer
) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.insert_candidate_universe_snapshot(
    date,uuid,text,text,uuid,date,text,text,timestamptz,timestamptz,integer
) TO research_writer;

CREATE OR REPLACE FUNCTION public.candidate_source_validate_dataset_pin()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $validate$
DECLARE
    v_dataset_id text;
    v_first_date date;
    v_last_date date;
    v_registry_key text;
BEGIN
    SELECT dataset.dataset_id
      INTO v_dataset_id
      FROM public.dataset_versions AS dataset
     WHERE dataset.id = NEW.dataset_version_id;
    IF TG_TABLE_NAME = 'candidate_universe_snapshots' THEN
        SELECT registry.universe_key
          INTO v_registry_key
          FROM public.candidate_universe_registry AS registry
         WHERE registry.membership_dataset_id = v_dataset_id
           AND registry.universe_key = (pg_catalog.to_jsonb(NEW) ->> 'index_id')
           AND registry.enabled;
        IF v_registry_key IS NULL THEN
            RAISE EXCEPTION 'candidate universe snapshot dataset does not match registry'
                USING ERRCODE = '23514';
        END IF;
    ELSE
        v_dataset_id := CASE TG_TABLE_NAME
            WHEN 'candidate_investor_flows' THEN 'krx_investor_flows'
            WHEN 'candidate_market_status_observations' THEN 'krx_market_status'
            WHEN 'candidate_fundamental_observations' THEN 'krx_fundamentals'
            WHEN 'candidate_sector_versions' THEN 'krx_sector_classification'
            WHEN 'candidate_price_publications' THEN 'krx_eod_bars'
            ELSE '__invalid_candidate_source_table__'
        END;
    END IF;
    PERFORM 1
      FROM public.dataset_versions AS dataset
     WHERE dataset.id = NEW.dataset_version_id
       AND dataset.manifest_sha256 = NEW.manifest_sha256
       AND dataset.status IN ('READY', 'WARNING')
       AND dataset.dataset_id = v_dataset_id
       AND (
           TG_TABLE_NAME <> 'candidate_price_publications'
           OR dataset.version = (pg_catalog.to_jsonb(NEW) ->> 'dataset_version')
       )
    FOR SHARE OF dataset;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate source requires a usable exact dataset pin'
            USING ERRCODE = '23514';
    END IF;
    v_first_date := CASE TG_TABLE_NAME
        WHEN 'candidate_price_publications'
            THEN (pg_catalog.to_jsonb(NEW) ->> 'first_session')::date
        ELSE (pg_catalog.to_jsonb(NEW) ->> 'entitlement_date')::date
    END;
    v_last_date := CASE
        WHEN TG_TABLE_NAME = 'candidate_price_publications'
            THEN (pg_catalog.to_jsonb(NEW) ->> 'last_session')::date
        ELSE v_first_date
    END;
    IF NOT public.candidate_source_entitlement_is_valid(
        NEW.entitlement_id, NEW.license_ref, v_dataset_id,
        v_first_date, v_last_date
    ) THEN
        RAISE EXCEPTION 'candidate source requires an exact active candidate-use entitlement'
            USING ERRCODE = '42501';
    END IF;
    RETURN NEW;
END
$validate$;
ALTER FUNCTION public.candidate_source_validate_dataset_pin() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.candidate_source_validate_dataset_pin() FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.stock_analysis_validate_lineage()
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
       AND NEW.universe_key IS NOT DISTINCT FROM OLD.universe_key
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
      JOIN public.candidate_universe_registry AS registry
        ON registry.universe_key = NEW.universe_key
       AND registry.enabled
     WHERE universe.id = NEW.universe_snapshot_id
       AND universe.index_id = NEW.universe_key
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
          JOIN public.dataset_versions AS dataset
            ON dataset.id = price.dataset_version_id
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
          JOIN public.dataset_versions AS dataset
            ON dataset.id = status.dataset_version_id
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
            ON member.flow_observation_id = flow.id
          JOIN public.dataset_versions AS dataset
            ON dataset.id = member.dataset_version_id
         WHERE dataset.id = NEW.flow_dataset_version_id
           AND member.entitlement_id = NEW.flow_entitlement_id
           AND flow.trade_date = NEW.as_of_date
           AND flow.available_at <= NEW.cutoff_at
           AND dataset.manifest_sha256 = NEW.flow_manifest_sha256
           AND dataset.status IN ('READY', 'WARNING')
    ) OR NOT EXISTS (
        SELECT 1
          FROM public.candidate_fundamental_observations AS fact
          JOIN public.dataset_versions AS dataset
            ON dataset.id = fact.dataset_version_id
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

CREATE OR REPLACE FUNCTION public.schedule_candidate_run(
    p_as_of_date date,
    p_cutoff_at timestamptz,
    p_scoring_config_version text,
    p_scoring_config_sha256 text,
    p_universe_snapshot_id uuid,
    p_price_dataset_version_id uuid,
    p_price_curated_version integer,
    p_price_manifest_sha256 text,
    p_status_dataset_version_id uuid,
    p_status_manifest_sha256 text,
    p_flow_dataset_version_id uuid,
    p_flow_manifest_sha256 text,
    p_fundamental_dataset_version_id uuid,
    p_fundamental_manifest_sha256 text,
    p_sector_version_id uuid
)
RETURNS TABLE (run_id uuid, job_id uuid, computation_seq integer)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $schedule$
DECLARE
    v_service_user_id uuid;
    v_required_fetch_mode text;
    v_expected_key text;
    v_core_identity text;
    v_input_identity_sha256 text;
    v_run_id uuid;
    v_job_id uuid;
    v_seq integer;
    v_payload jsonb;
    v_dataset_id text;
    v_universe_key text;
    v_price_entitlement_id uuid;
    v_price_license_ref text;
    v_universe_entitlement_id uuid;
    v_universe_license_ref text;
    v_status_entitlement_id uuid;
    v_status_license_ref text;
    v_flow_entitlement_id uuid;
    v_flow_license_ref text;
    v_fundamental_entitlement_id uuid;
    v_fundamental_license_ref text;
    v_sector_entitlement_id uuid;
    v_sector_license_ref text;
    v_canonical_cutoff timestamptz;
    v_required_first_session date;
    v_required_session_count integer;
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock_shared(1815099521, 44);
    SELECT control.service_user_id, control.required_fetch_mode
      INTO v_service_user_id, v_required_fetch_mode
      FROM public.candidate_scheduler_control AS control
     WHERE control.control_key = 'scheduler' AND control.active;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate scheduler is unavailable' USING ERRCODE = '55000';
    END IF;
    PERFORM pg_catalog.set_config('app.actor_user_id', v_service_user_id::text, true);

    IF p_as_of_date IS NULL OR p_cutoff_at IS NULL
       OR p_scoring_config_version IS NULL OR p_scoring_config_sha256 IS NULL
       OR p_universe_snapshot_id IS NULL
       OR p_price_dataset_version_id IS NULL OR p_price_curated_version IS NULL
       OR p_price_manifest_sha256 IS NULL
       OR p_status_dataset_version_id IS NULL OR p_status_manifest_sha256 IS NULL
       OR p_flow_dataset_version_id IS NULL OR p_flow_manifest_sha256 IS NULL
       OR p_fundamental_dataset_version_id IS NULL
       OR p_fundamental_manifest_sha256 IS NULL
       OR p_sector_version_id IS NULL
    THEN
        RAISE EXCEPTION 'candidate scheduled identity must be complete'
            USING ERRCODE = '22023';
    END IF;
    IF p_price_curated_version <= 0
       OR p_scoring_config_sha256 !~ '^[0-9a-f]{64}$'
       OR p_price_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_status_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_flow_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_fundamental_manifest_sha256 !~ '^[0-9a-f]{64}$'
    THEN
        RAISE EXCEPTION 'candidate scheduled hash is invalid' USING ERRCODE = '22023';
    END IF;
    IF p_cutoff_at < (p_as_of_date::timestamp AT TIME ZONE 'Asia/Seoul')
       OR p_cutoff_at > ((p_as_of_date + 7)::timestamp AT TIME ZONE 'Asia/Seoul')
    THEN
        RAISE EXCEPTION 'candidate cutoff is outside the bounded as-of window'
            USING ERRCODE = '22023';
    END IF;

    PERFORM 1
      FROM public.trading_calendars AS calendar
     WHERE calendar.exchange = 'KRX'
       AND calendar.session_date = p_as_of_date
       AND calendar.session_type = 'TRADING'
       AND calendar.timezone = 'Asia/Seoul'
       AND calendar.source_batch_id IS NOT NULL
       AND calendar.content_sha256 IS NOT NULL
       AND calendar.retrieved_at IS NOT NULL;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate run requires a confirmed KRX trading session'
            USING ERRCODE = '55000';
    END IF;
    SELECT min(required.session_date), count(*)
      INTO v_required_first_session, v_required_session_count
      FROM (
          SELECT calendar.session_date
            FROM public.trading_calendars AS calendar
           WHERE calendar.exchange = 'KRX'
             AND calendar.session_type = 'TRADING'
             AND calendar.timezone = 'Asia/Seoul'
             AND calendar.session_date <= p_as_of_date
             AND calendar.source_batch_id IS NOT NULL
             AND calendar.content_sha256 IS NOT NULL
             AND calendar.retrieved_at IS NOT NULL
           ORDER BY calendar.session_date DESC
           LIMIT 60
      ) AS required;
    IF v_required_session_count <> 60 THEN
        RAISE EXCEPTION 'candidate run requires 60 confirmed KRX sessions'
            USING ERRCODE = '55000';
    END IF;

    PERFORM 1
      FROM public.candidate_scoring_configs AS config
     WHERE config.version = p_scoring_config_version
       AND config.content_sha256 = p_scoring_config_sha256;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate scoring configuration mismatch' USING ERRCODE = '23514';
    END IF;

    SELECT dataset.dataset_id, price.entitlement_id, price.license_ref
      INTO v_dataset_id, v_price_entitlement_id, v_price_license_ref
      FROM public.candidate_price_publications AS price
      JOIN public.dataset_versions AS dataset ON dataset.id = price.dataset_version_id
     WHERE price.dataset_version_id = p_price_dataset_version_id
       AND dataset.dataset_id = 'krx_eod_bars'
       AND dataset.manifest_sha256 = p_price_manifest_sha256
       AND dataset.status IN ('READY', 'WARNING')
       AND price.manifest_sha256 = p_price_manifest_sha256
       AND price.curated_generation = p_price_curated_version
       AND price.market = 'kr'
       AND price.first_session <= p_as_of_date
       AND price.last_session >= p_as_of_date
       AND price.available_at <= p_cutoff_at;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate price dataset is unavailable' USING ERRCODE = '55000';
    END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_price_entitlement_id, v_price_license_ref, v_dataset_id,
        v_required_first_session, p_as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate price entitlement is inactive' USING ERRCODE = '42501';
    END IF;

    SELECT dataset.dataset_id, status.entitlement_id, status.license_ref
      INTO v_dataset_id, v_status_entitlement_id, v_status_license_ref
      FROM public.candidate_market_status_observations AS status
      JOIN public.dataset_versions AS dataset ON dataset.id = status.dataset_version_id
     WHERE status.dataset_version_id = p_status_dataset_version_id
       AND dataset.dataset_id = 'krx_market_status'
       AND dataset.manifest_sha256 = p_status_manifest_sha256
       AND dataset.status IN ('READY', 'WARNING')
       AND status.trade_date = p_as_of_date
       AND status.available_at <= p_cutoff_at
     ORDER BY status.available_at DESC, status.id
     LIMIT 1;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate market-status dataset is unavailable' USING ERRCODE = '55000';
    END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_status_entitlement_id, v_status_license_ref, v_dataset_id,
        p_as_of_date, p_as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate market-status entitlement is inactive' USING ERRCODE = '42501';
    END IF;

    SELECT dataset.dataset_id, member.entitlement_id, member.license_ref
      INTO v_dataset_id, v_flow_entitlement_id, v_flow_license_ref
      FROM public.candidate_investor_flows AS flow
      JOIN public.candidate_investor_flow_snapshot_rows AS member
        ON member.flow_observation_id = flow.id
      JOIN public.dataset_versions AS dataset ON dataset.id = member.dataset_version_id
     WHERE member.dataset_version_id = p_flow_dataset_version_id
       AND dataset.dataset_id = 'krx_investor_flows'
       AND dataset.manifest_sha256 = p_flow_manifest_sha256
       AND dataset.status IN ('READY', 'WARNING')
       AND flow.trade_date = p_as_of_date
       AND flow.available_at <= p_cutoff_at
     ORDER BY flow.available_at DESC, flow.id
     LIMIT 1;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate flow dataset is unavailable' USING ERRCODE = '55000';
    END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_flow_entitlement_id, v_flow_license_ref, v_dataset_id,
        v_required_first_session, p_as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate flow entitlement is inactive' USING ERRCODE = '42501';
    END IF;

    SELECT dataset.dataset_id, fact.entitlement_id, fact.license_ref
      INTO v_dataset_id, v_fundamental_entitlement_id, v_fundamental_license_ref
      FROM public.candidate_fundamental_observations AS fact
      JOIN public.dataset_versions AS dataset ON dataset.id = fact.dataset_version_id
     WHERE fact.dataset_version_id = p_fundamental_dataset_version_id
       AND dataset.dataset_id = 'krx_fundamentals'
       AND dataset.manifest_sha256 = p_fundamental_manifest_sha256
       AND dataset.status IN ('READY', 'WARNING')
       AND fact.fiscal_period_end <= p_as_of_date
       AND fact.available_at <= p_cutoff_at
     ORDER BY fact.available_at DESC, fact.id
     LIMIT 1;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate fundamental dataset is unavailable' USING ERRCODE = '55000';
    END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_fundamental_entitlement_id, v_fundamental_license_ref, v_dataset_id,
        p_as_of_date, p_as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate fundamental entitlement is inactive' USING ERRCODE = '42501';
    END IF;

    SELECT dataset.dataset_id, registry.universe_key,
           universe.entitlement_id, universe.license_ref
      INTO v_dataset_id, v_universe_key,
           v_universe_entitlement_id, v_universe_license_ref
      FROM public.candidate_universe_snapshots AS universe
      JOIN public.dataset_versions AS dataset ON dataset.id = universe.dataset_version_id
      JOIN public.candidate_universe_registry AS registry
        ON registry.universe_key = universe.index_id
       AND registry.membership_dataset_id = dataset.dataset_id
       AND registry.enabled
     WHERE universe.id = p_universe_snapshot_id
       AND universe.as_of_date <= p_as_of_date
       AND universe.available_at <= p_cutoff_at
       AND universe.member_count = (
           SELECT count(*) FROM public.candidate_universe_members AS member
            WHERE member.universe_snapshot_id = universe.id
              AND member.effective_from <= p_as_of_date
              AND (member.effective_until IS NULL
                   OR member.effective_until >= p_as_of_date)
       )
       AND dataset.manifest_sha256 = universe.manifest_sha256
       AND dataset.status IN ('READY', 'WARNING');
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate universe is unavailable at cutoff' USING ERRCODE = '55000';
    END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_universe_entitlement_id, v_universe_license_ref, v_dataset_id,
        p_as_of_date, p_as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate universe entitlement is inactive' USING ERRCODE = '42501';
    END IF;

    SELECT dataset.dataset_id, sector.entitlement_id, sector.license_ref
      INTO v_dataset_id, v_sector_entitlement_id, v_sector_license_ref
      FROM public.candidate_sector_versions AS sector
      JOIN public.dataset_versions AS dataset ON dataset.id = sector.dataset_version_id
     WHERE sector.id = p_sector_version_id
       AND sector.effective_from <= p_as_of_date
       AND sector.available_at <= p_cutoff_at
       AND dataset.dataset_id = 'krx_sector_classification'
       AND dataset.manifest_sha256 = sector.manifest_sha256
       AND dataset.status IN ('READY', 'WARNING');
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate sector dataset is unavailable at cutoff' USING ERRCODE = '55000';
    END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_sector_entitlement_id, v_sector_license_ref, v_dataset_id,
        p_as_of_date, p_as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate sector entitlement is inactive' USING ERRCODE = '42501';
    END IF;

    IF EXISTS (
        SELECT 1
          FROM (
              VALUES
                  ('bars'::text, p_price_dataset_version_id, NULL::text),
                  ('market_status', p_status_dataset_version_id, NULL),
                  ('investor_flow', p_flow_dataset_version_id, NULL),
                  ('fundamentals', p_fundamental_dataset_version_id, NULL),
                  ('sector_classification', (
                      SELECT sector.dataset_version_id
                        FROM public.candidate_sector_versions AS sector
                       WHERE sector.id = p_sector_version_id
                  ), NULL),
                  ('index_membership', p_universe_snapshot_id, v_universe_key)
          ) AS required(response_kind, identity_id, identity_key)
         WHERE NOT EXISTS (
             SELECT 1
               FROM public.candidate_raw_batch_datasets AS binding
               JOIN public.candidate_raw_batch_publications AS batch
                 ON batch.batch_id = binding.batch_id AND batch.surface = binding.surface
              WHERE binding.response_kind = required.response_kind
                AND (
                    binding.dataset_version_id = required.identity_id
                    OR (required.response_kind = 'index_membership' AND EXISTS (
                        SELECT 1
                          FROM public.candidate_universe_snapshots AS snapshot
                         WHERE snapshot.id = required.identity_id
                           AND snapshot.dataset_version_id = binding.dataset_version_id
                           AND snapshot.index_id = required.identity_key
                    ))
                )
                AND batch.state = 'PUBLISHED'
                AND batch.fetch_mode = v_required_fetch_mode
         )
    ) THEN
        RAISE EXCEPTION 'candidate source pins are not sealed under the required fetch mode'
            USING ERRCODE = '55000';
    END IF;

    SELECT greatest(
        (SELECT calendar.retrieved_at FROM public.trading_calendars AS calendar
          WHERE calendar.exchange = 'KRX' AND calendar.session_date = p_as_of_date
            AND calendar.session_type = 'TRADING' AND calendar.timezone = 'Asia/Seoul'
          ORDER BY calendar.retrieved_at DESC LIMIT 1),
        (SELECT config.created_at FROM public.candidate_scoring_configs AS config
          WHERE config.version = p_scoring_config_version
            AND config.content_sha256 = p_scoring_config_sha256),
        (SELECT price.available_at FROM public.candidate_price_publications AS price
          WHERE price.dataset_version_id = p_price_dataset_version_id),
        (SELECT max(status.available_at) FROM public.candidate_market_status_observations AS status
          WHERE status.dataset_version_id = p_status_dataset_version_id
            AND status.trade_date = p_as_of_date),
        (SELECT max(flow.available_at) FROM public.candidate_investor_flows AS flow
          JOIN public.candidate_investor_flow_snapshot_rows AS member
            ON member.flow_observation_id = flow.id
          WHERE member.dataset_version_id = p_flow_dataset_version_id
            AND flow.trade_date = p_as_of_date),
        (SELECT max(fact.available_at) FROM public.candidate_fundamental_observations AS fact
          WHERE fact.dataset_version_id = p_fundamental_dataset_version_id
            AND fact.fiscal_period_end <= p_as_of_date),
        (SELECT universe.available_at FROM public.candidate_universe_snapshots AS universe
          WHERE universe.id = p_universe_snapshot_id),
        (SELECT sector.available_at FROM public.candidate_sector_versions AS sector
          WHERE sector.id = p_sector_version_id)
    ) INTO v_canonical_cutoff;
    IF v_canonical_cutoff IS NULL OR p_cutoff_at <> v_canonical_cutoff THEN
        RAISE EXCEPTION 'candidate cutoff does not match exact pinned source availability'
            USING ERRCODE = '23514';
    END IF;

    IF (
        WITH required_sessions AS MATERIALIZED (
            SELECT calendar.session_date
              FROM public.trading_calendars AS calendar
             WHERE calendar.exchange = 'KRX'
               AND calendar.session_type = 'TRADING'
               AND calendar.timezone = 'Asia/Seoul'
               AND calendar.session_date <= p_as_of_date
               AND calendar.source_batch_id IS NOT NULL
               AND calendar.content_sha256 IS NOT NULL
               AND calendar.retrieved_at IS NOT NULL
             ORDER BY calendar.session_date DESC
             LIMIT 60
        )
        SELECT count(*) FROM public.candidate_universe_members AS member
         WHERE member.universe_snapshot_id = p_universe_snapshot_id
           AND member.effective_from <= p_as_of_date
           AND (member.effective_until IS NULL OR member.effective_until >= p_as_of_date)
           AND NOT EXISTS (
               SELECT 1 FROM required_sessions AS required
                WHERE NOT EXISTS (
                    SELECT 1 FROM public.candidate_price_instrument_sessions AS price_session
                     WHERE price_session.dataset_version_id = p_price_dataset_version_id
                       AND price_session.instrument_id = member.instrument_id
                       AND price_session.session_date = required.session_date
                )
           )
           AND NOT EXISTS (
               SELECT 1 FROM required_sessions AS required
               CROSS JOIN (VALUES ('FOREIGN'), ('INSTITUTION')) AS class(investor_class)
                WHERE NOT EXISTS (
                    SELECT 1
                      FROM public.candidate_investor_flows AS history
                      JOIN public.candidate_investor_flow_snapshot_rows AS flow_member
                        ON flow_member.flow_observation_id = history.id
                     WHERE flow_member.dataset_version_id = p_flow_dataset_version_id
                       AND history.instrument_id = member.instrument_id
                       AND history.trade_date = required.session_date
                       AND history.investor_class = class.investor_class
                       AND history.available_at <= p_cutoff_at
                )
           )
           AND EXISTS (
               SELECT 1 FROM public.candidate_market_status_observations AS status
                WHERE status.dataset_version_id = p_status_dataset_version_id
                  AND status.instrument_id = member.instrument_id
                  AND status.trade_date = p_as_of_date
                  AND status.available_at <= p_cutoff_at
           )
           AND EXISTS (
               SELECT 1 FROM public.candidate_fundamental_observations AS fact
                WHERE fact.dataset_version_id = p_fundamental_dataset_version_id
                  AND fact.instrument_id = member.instrument_id
                  AND fact.fiscal_period_end <= p_as_of_date
                  AND fact.available_at <= p_cutoff_at
           )
           AND EXISTS (
               SELECT 1 FROM public.candidate_sector_entries AS entry
                WHERE entry.sector_version_id = p_sector_version_id
                  AND entry.instrument_id = member.instrument_id
                  AND entry.effective_from <= p_as_of_date
                  AND entry.available_at <= p_cutoff_at
                  AND (entry.effective_until IS NULL OR entry.effective_until >= p_as_of_date)
           )
    ) < 5 THEN
        RAISE EXCEPTION 'fewer than five candidate members have complete 60-session inputs'
            USING ERRCODE = '55000';
    END IF;

    v_core_identity := pg_catalog.concat_ws(
        '|', v_universe_key, pg_catalog.to_char(p_as_of_date, 'YYYY-MM-DD'),
        pg_catalog.to_char(p_cutoff_at AT TIME ZONE 'UTC',
                           'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
        p_scoring_config_version, p_scoring_config_sha256,
        p_universe_snapshot_id::text, v_universe_entitlement_id::text,
        p_price_dataset_version_id::text, v_price_entitlement_id::text,
        p_price_curated_version::text, p_price_manifest_sha256,
        p_status_dataset_version_id::text, v_status_entitlement_id::text,
        p_status_manifest_sha256, p_flow_dataset_version_id::text,
        v_flow_entitlement_id::text, p_flow_manifest_sha256,
        p_fundamental_dataset_version_id::text, v_fundamental_entitlement_id::text,
        p_fundamental_manifest_sha256, p_sector_version_id::text,
        v_sector_entitlement_id::text
    );
    v_input_identity_sha256 := pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(v_core_identity, 'UTF8')), 'hex'
    );
    v_expected_key := 'candidate:scheduled:' || pg_catalog.md5(v_core_identity);
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended('candidate|' || v_universe_key || '|' || p_as_of_date::text, 0)
    );

    SELECT run.id, run.job_id, run.computation_seq
      INTO v_run_id, v_job_id, v_seq
      FROM public.stock_analysis_runs AS run
     WHERE run.input_identity_sha256 = v_input_identity_sha256
     FOR UPDATE OF run;
    IF FOUND THEN
        PERFORM 1 FROM public.jobs AS job
         WHERE job.id = v_job_id
           AND job.owner_user_id = v_service_user_id
           AND job.job_type = 'candidate_compute'
           AND job.idempotency_key = v_expected_key;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'candidate scheduled replay conflicts with job lineage'
                USING ERRCODE = '23514';
        END IF;
        RETURN QUERY SELECT v_run_id, v_job_id, v_seq;
        RETURN;
    END IF;
    SELECT COALESCE(max(run.computation_seq), 0) + 1
      INTO v_seq
      FROM public.stock_analysis_runs AS run
     WHERE run.universe_key = v_universe_key AND run.as_of_date = p_as_of_date;
    v_run_id := pg_catalog.gen_random_uuid();
    v_job_id := pg_catalog.gen_random_uuid();
    v_payload := pg_catalog.jsonb_build_object(
        'run_id', v_run_id, 'universe_key', v_universe_key,
        'as_of_date', pg_catalog.to_char(p_as_of_date, 'YYYY-MM-DD'),
        'cutoff_at', p_cutoff_at,
        'scoring_config_version', p_scoring_config_version,
        'scoring_config_sha256', p_scoring_config_sha256,
        'universe_snapshot_id', p_universe_snapshot_id,
        'universe_entitlement_id', v_universe_entitlement_id,
        'price_dataset_version_id', p_price_dataset_version_id,
        'price_entitlement_id', v_price_entitlement_id,
        'price_curated_version', p_price_curated_version,
        'price_manifest_sha256', p_price_manifest_sha256,
        'status_dataset_version_id', p_status_dataset_version_id,
        'status_entitlement_id', v_status_entitlement_id,
        'status_manifest_sha256', p_status_manifest_sha256,
        'flow_dataset_version_id', p_flow_dataset_version_id,
        'flow_entitlement_id', v_flow_entitlement_id,
        'flow_manifest_sha256', p_flow_manifest_sha256,
        'fundamental_dataset_version_id', p_fundamental_dataset_version_id,
        'fundamental_entitlement_id', v_fundamental_entitlement_id,
        'fundamental_manifest_sha256', p_fundamental_manifest_sha256,
        'sector_version_id', p_sector_version_id,
        'sector_entitlement_id', v_sector_entitlement_id,
        'input_identity_sha256', v_input_identity_sha256
    );
    INSERT INTO public.jobs (
        id, owner_user_id, job_type, status, idempotency_key, payload_json, max_attempts
    ) VALUES (
        v_job_id, v_service_user_id, 'candidate_compute', 'QUEUED',
        v_expected_key, v_payload, 3
    );
    INSERT INTO public.stock_analysis_runs (
        id, as_of_date, cutoff_at, computation_seq, status, job_id,
        scoring_config_version, scoring_config_sha256, universe_snapshot_id,
        universe_key, universe_entitlement_id,
        price_dataset_version_id, price_entitlement_id, price_curated_version,
        price_manifest_sha256, status_dataset_version_id, status_entitlement_id,
        status_manifest_sha256, flow_dataset_version_id, flow_entitlement_id,
        flow_manifest_sha256, fundamental_dataset_version_id,
        fundamental_entitlement_id, fundamental_manifest_sha256, sector_version_id,
        sector_entitlement_id, input_identity_sha256
    ) VALUES (
        v_run_id, p_as_of_date, p_cutoff_at, v_seq, 'PENDING', v_job_id,
        p_scoring_config_version, p_scoring_config_sha256, p_universe_snapshot_id,
        v_universe_key, v_universe_entitlement_id, p_price_dataset_version_id,
        v_price_entitlement_id, p_price_curated_version, p_price_manifest_sha256,
        p_status_dataset_version_id, v_status_entitlement_id, p_status_manifest_sha256,
        p_flow_dataset_version_id, v_flow_entitlement_id, p_flow_manifest_sha256,
        p_fundamental_dataset_version_id, v_fundamental_entitlement_id,
        p_fundamental_manifest_sha256, p_sector_version_id, v_sector_entitlement_id,
        v_input_identity_sha256
    );
    RETURN QUERY SELECT v_run_id, v_job_id, v_seq;
END
$schedule$;
ALTER FUNCTION public.schedule_candidate_run(
    date,timestamptz,text,text,uuid,uuid,integer,text,uuid,text,uuid,text,uuid,text,uuid
) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.schedule_candidate_run(
    date,timestamptz,text,text,uuid,uuid,integer,text,uuid,text,uuid,text,uuid,text,uuid
) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.schedule_candidate_run(
    date,timestamptz,text,text,uuid,uuid,integer,text,uuid,text,uuid,text,uuid,text,uuid
) TO worker;

CREATE OR REPLACE FUNCTION public.publish_candidate_analysis(
    p_run_id uuid,
    p_job_id uuid,
    p_attempt_no integer,
    p_worker_id text,
    p_snapshots jsonb,
    p_summary jsonb
)
RETURNS uuid
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $publish$
DECLARE
    v_service_user_id uuid;
    v_run public.stock_analysis_runs%ROWTYPE;
    v_feed_id uuid;
    v_snapshot_count integer;
    v_member_count integer;
    v_eligible_count integer;
    v_dataset_id text;
    v_license_ref text;
    v_universe_key text;
    v_required_first_session date;
    v_required_session_count integer;
BEGIN
    IF p_run_id IS NULL OR p_job_id IS NULL OR p_attempt_no <= 0
       OR length(btrim(COALESCE(p_worker_id, ''))) = 0
       OR jsonb_typeof(p_snapshots) <> 'array'
       OR jsonb_array_length(p_snapshots) = 0
       OR jsonb_array_length(p_snapshots) > 10000
       OR jsonb_typeof(p_summary) <> 'object'
    THEN
        RAISE EXCEPTION 'candidate publication payload is invalid' USING ERRCODE = '22023';
    END IF;
    SELECT control.service_user_id
      INTO v_service_user_id
      FROM public.candidate_scheduler_control AS control
     WHERE control.control_key = 'scheduler';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate scheduler is unavailable' USING ERRCODE = '55000';
    END IF;
    PERFORM pg_catalog.set_config('app.actor_user_id', v_service_user_id::text, true);

    SELECT run.* INTO v_run
      FROM public.stock_analysis_runs AS run
     WHERE run.id = p_run_id AND run.job_id = p_job_id
     FOR UPDATE OF run;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate publication run is missing' USING ERRCODE = '23514';
    END IF;
    v_universe_key := v_run.universe_key;
    IF v_run.status = 'SUCCEEDED' THEN
        SELECT feed.id INTO v_feed_id
          FROM public.candidate_feed_snapshots AS feed
         WHERE feed.run_id = p_run_id AND feed.universe_key = v_universe_key;
        IF v_feed_id IS NULL
           OR p_summary IS DISTINCT FROM v_run.summary_json
           OR jsonb_array_length(p_snapshots) <> (
               SELECT count(*) FROM public.stock_analysis_snapshots AS snapshot
                WHERE snapshot.run_id = p_run_id
           )
           OR jsonb_array_length(p_snapshots) <> (
               SELECT count(DISTINCT supplied.value ->> 'instrument_id')
                 FROM jsonb_array_elements(p_snapshots) AS supplied(value)
           )
           OR EXISTS (
               WITH supplied AS (
                   SELECT value ->> 'instrument_id' AS instrument_id,
                          pg_catalog.encode(
                              pg_catalog.sha256(pg_catalog.jsonb_send(value)), 'hex'
                          ) AS content_sha256
                     FROM jsonb_array_elements(p_snapshots)
               ), stored AS (
                   SELECT snapshot.instrument_id, snapshot.content_sha256
                     FROM public.stock_analysis_snapshots AS snapshot
                    WHERE snapshot.run_id = p_run_id
               )
               SELECT 1 FROM stored FULL JOIN supplied USING (instrument_id)
                WHERE stored.instrument_id IS NULL
                   OR supplied.instrument_id IS NULL
                   OR supplied.content_sha256 IS DISTINCT FROM stored.content_sha256
           )
        THEN
            RAISE EXCEPTION 'candidate publication replay payload mismatch'
                USING ERRCODE = '23514';
        END IF;
        RETURN v_feed_id;
    END IF;
    IF v_run.status NOT IN ('PENDING', 'RUNNING') THEN
        RAISE EXCEPTION 'candidate publication run is not publishable' USING ERRCODE = '55000';
    END IF;

    SELECT min(required.session_date), count(*)
      INTO v_required_first_session, v_required_session_count
      FROM (
          SELECT calendar.session_date
            FROM public.trading_calendars AS calendar
           WHERE calendar.exchange = 'KRX'
             AND calendar.session_type = 'TRADING'
             AND calendar.timezone = 'Asia/Seoul'
             AND calendar.session_date <= v_run.as_of_date
             AND calendar.source_batch_id IS NOT NULL
             AND calendar.content_sha256 IS NOT NULL
             AND calendar.retrieved_at IS NOT NULL
           ORDER BY calendar.session_date DESC
           LIMIT 60
      ) AS required;
    IF v_required_session_count <> 60 THEN
        RAISE EXCEPTION 'candidate publication requires 60 confirmed KRX sessions'
            USING ERRCODE = '55000';
    END IF;
    PERFORM 1
      FROM public.jobs AS job
      JOIN public.job_attempts AS attempt
        ON attempt.job_id = job.id AND attempt.attempt_no = p_attempt_no
     WHERE job.id = p_job_id
       AND job.owner_user_id = v_service_user_id
       AND job.job_type = 'candidate_compute'
       AND job.status = 'RUNNING'
       AND job.locked_by = p_worker_id
       AND job.locked_at IS NOT NULL
       AND job.attempt_count = p_attempt_no
       AND attempt.outcome = 'RUNNING'
       AND attempt.claimed_by = p_worker_id
     FOR UPDATE OF job, attempt;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate publication does not hold the queue claim'
            USING ERRCODE = '55000';
    END IF;

    -- Re-attest each exact source and entitlement inside the publication
    -- transaction.  The universe lookup is registry-derived; a KOSPI
    -- entitlement can never be reused for a KOSDAQ snapshot.
    SELECT dataset.dataset_id, price.license_ref INTO v_dataset_id, v_license_ref
      FROM public.candidate_price_publications AS price
      JOIN public.dataset_versions AS dataset ON dataset.id = price.dataset_version_id
     WHERE price.dataset_version_id = v_run.price_dataset_version_id
       AND price.entitlement_id = v_run.price_entitlement_id
       AND price.curated_generation = v_run.price_curated_version
       AND price.first_session <= v_run.as_of_date
       AND price.last_session >= v_run.as_of_date
       AND price.available_at <= v_run.cutoff_at
       AND dataset.dataset_id = 'krx_eod_bars'
       AND dataset.manifest_sha256 = v_run.price_manifest_sha256
       AND dataset.status IN ('READY', 'WARNING')
     FOR SHARE OF price, dataset;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate price dataset became unavailable' USING ERRCODE = '55000';
    END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_run.price_entitlement_id, v_license_ref, v_dataset_id,
        v_required_first_session, v_run.as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate price entitlement became inactive before publication'
            USING ERRCODE = '42501';
    END IF;

    SELECT dataset.dataset_id, status.license_ref INTO v_dataset_id, v_license_ref
      FROM public.candidate_market_status_observations AS status
      JOIN public.dataset_versions AS dataset ON dataset.id = status.dataset_version_id
     WHERE status.dataset_version_id = v_run.status_dataset_version_id
       AND status.entitlement_id = v_run.status_entitlement_id
       AND status.trade_date = v_run.as_of_date
       AND status.available_at <= v_run.cutoff_at
       AND dataset.dataset_id = 'krx_market_status'
       AND dataset.manifest_sha256 = v_run.status_manifest_sha256
       AND dataset.status IN ('READY', 'WARNING')
     ORDER BY status.available_at DESC, status.id
     LIMIT 1
     FOR SHARE OF status, dataset;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate market-status dataset became unavailable'
            USING ERRCODE = '55000';
    END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_run.status_entitlement_id, v_license_ref, v_dataset_id,
        v_run.as_of_date, v_run.as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate market-status entitlement became inactive before publication'
            USING ERRCODE = '42501';
    END IF;

    SELECT dataset.dataset_id, member.license_ref INTO v_dataset_id, v_license_ref
      FROM public.candidate_investor_flows AS flow
      JOIN public.candidate_investor_flow_snapshot_rows AS member
        ON member.flow_observation_id = flow.id
      JOIN public.dataset_versions AS dataset ON dataset.id = member.dataset_version_id
     WHERE member.dataset_version_id = v_run.flow_dataset_version_id
       AND member.entitlement_id = v_run.flow_entitlement_id
       AND flow.trade_date = v_run.as_of_date
       AND flow.available_at <= v_run.cutoff_at
       AND dataset.dataset_id = 'krx_investor_flows'
       AND dataset.manifest_sha256 = v_run.flow_manifest_sha256
       AND dataset.status IN ('READY', 'WARNING')
     ORDER BY flow.available_at DESC, flow.id
     LIMIT 1
     FOR SHARE OF flow, dataset;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate flow dataset became unavailable' USING ERRCODE = '55000';
    END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_run.flow_entitlement_id, v_license_ref, v_dataset_id,
        v_required_first_session, v_run.as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate flow entitlement became inactive before publication'
            USING ERRCODE = '42501';
    END IF;

    SELECT dataset.dataset_id, fact.license_ref INTO v_dataset_id, v_license_ref
      FROM public.candidate_fundamental_observations AS fact
      JOIN public.dataset_versions AS dataset ON dataset.id = fact.dataset_version_id
     WHERE fact.dataset_version_id = v_run.fundamental_dataset_version_id
       AND fact.entitlement_id = v_run.fundamental_entitlement_id
       AND fact.fiscal_period_end <= v_run.as_of_date
       AND fact.available_at <= v_run.cutoff_at
       AND dataset.dataset_id = 'krx_fundamentals'
       AND dataset.manifest_sha256 = v_run.fundamental_manifest_sha256
       AND dataset.status IN ('READY', 'WARNING')
     ORDER BY fact.available_at DESC, fact.id
     LIMIT 1
     FOR SHARE OF fact, dataset;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate fundamental dataset became unavailable'
            USING ERRCODE = '55000';
    END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_run.fundamental_entitlement_id, v_license_ref, v_dataset_id,
        v_run.as_of_date, v_run.as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate fundamental entitlement became inactive before publication'
            USING ERRCODE = '42501';
    END IF;

    SELECT dataset.dataset_id, universe.license_ref INTO v_dataset_id, v_license_ref
      FROM public.candidate_universe_snapshots AS universe
      JOIN public.dataset_versions AS dataset ON dataset.id = universe.dataset_version_id
      JOIN public.candidate_universe_registry AS registry
        ON registry.universe_key = universe.index_id
       AND registry.membership_dataset_id = dataset.dataset_id
       AND registry.enabled
     WHERE universe.id = v_run.universe_snapshot_id
       AND universe.index_id = v_run.universe_key
       AND universe.entitlement_id = v_run.universe_entitlement_id
       AND universe.as_of_date <= v_run.as_of_date
       AND universe.available_at <= v_run.cutoff_at
       AND universe.member_count = (
           SELECT count(*) FROM public.candidate_universe_members AS member
            WHERE member.universe_snapshot_id = universe.id
              AND member.effective_from <= v_run.as_of_date
              AND (member.effective_until IS NULL OR member.effective_until >= v_run.as_of_date)
       )
       AND dataset.manifest_sha256 = universe.manifest_sha256
       AND dataset.status IN ('READY', 'WARNING')
     FOR SHARE OF universe, dataset;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate universe became unavailable' USING ERRCODE = '55000';
    END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_run.universe_entitlement_id, v_license_ref, v_dataset_id,
        v_run.as_of_date, v_run.as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate universe entitlement became inactive before publication'
            USING ERRCODE = '42501';
    END IF;

    SELECT dataset.dataset_id, sector.license_ref INTO v_dataset_id, v_license_ref
      FROM public.candidate_sector_versions AS sector
      JOIN public.dataset_versions AS dataset ON dataset.id = sector.dataset_version_id
     WHERE sector.id = v_run.sector_version_id
       AND sector.entitlement_id = v_run.sector_entitlement_id
       AND sector.effective_from <= v_run.as_of_date
       AND sector.available_at <= v_run.cutoff_at
       AND dataset.dataset_id = 'krx_sector_classification'
       AND dataset.manifest_sha256 = sector.manifest_sha256
       AND dataset.status IN ('READY', 'WARNING')
     FOR SHARE OF sector, dataset;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate sector dataset became unavailable' USING ERRCODE = '55000';
    END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_run.sector_entitlement_id, v_license_ref, v_dataset_id,
        v_run.as_of_date, v_run.as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate sector entitlement became inactive before publication'
            USING ERRCODE = '42501';
    END IF;

    SELECT universe.member_count
      INTO v_member_count
      FROM public.candidate_universe_snapshots AS universe
     WHERE universe.id = v_run.universe_snapshot_id
       AND universe.index_id = v_run.universe_key;
    IF v_member_count <> jsonb_array_length(p_snapshots) THEN
        RAISE EXCEPTION 'candidate publication must cover the exact universe'
            USING ERRCODE = '23514';
    END IF;

    WITH supplied AS (
        SELECT *
          FROM jsonb_to_recordset(p_snapshots) AS input(
              instrument_id text,
              sector_code text,
              fundamental_profile text,
              eligible boolean,
              exclusion_codes jsonb,
              flow_score numeric,
              fundamental_score numeric,
              technical_score numeric,
              total_score numeric,
              flow_coverage numeric,
              fundamental_coverage numeric,
              technical_coverage numeric,
              evidence_strength text,
              normalization_scope text,
              factors_json jsonb,
              scenarios_json jsonb,
              provenance_json jsonb
          )
    )
    SELECT count(*)::integer INTO v_snapshot_count FROM supplied;
    IF v_snapshot_count <> v_member_count OR EXISTS (
        WITH supplied AS (
            SELECT value ->> 'instrument_id' AS instrument_id
              FROM jsonb_array_elements(p_snapshots)
        ), members AS (
            SELECT member.instrument_id
              FROM public.candidate_universe_members AS member
             WHERE member.universe_snapshot_id = v_run.universe_snapshot_id
        )
        SELECT 1 FROM supplied FULL JOIN members USING (instrument_id)
         WHERE supplied.instrument_id IS NULL OR members.instrument_id IS NULL
    ) THEN
        RAISE EXCEPTION 'candidate publication membership mismatch'
            USING ERRCODE = '23514';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM jsonb_array_elements(p_snapshots) AS supplied(value)
         WHERE supplied.value ? 'content_sha256'
            OR supplied.value - ARRAY[
                'instrument_id', 'sector_code', 'fundamental_profile', 'eligible',
                'exclusion_codes', 'flow_score', 'fundamental_score',
                'technical_score', 'total_score', 'flow_coverage',
                'fundamental_coverage', 'technical_coverage', 'evidence_strength',
                'normalization_scope', 'factors_json', 'scenarios_json',
                'provenance_json'
              ]::text[] <> '{}'::jsonb
            OR supplied.value -> 'provenance_json' ->> 'input_identity_sha256'
                   IS DISTINCT FROM v_run.input_identity_sha256
            OR supplied.value -> 'provenance_json' ->> 'as_of_date'
                   IS DISTINCT FROM pg_catalog.to_char(v_run.as_of_date, 'YYYY-MM-DD')
            OR (supplied.value -> 'scenarios_json') ?| ARRAY[
                'probability', 'probabilities', 'target_price', 'expected_return'
              ]::text[]
            OR EXISTS (
                SELECT 1
                  FROM pg_catalog.jsonb_path_query(
                      supplied.value -> 'scenarios_json', '$.**'
                  ) AS descendant(value)
                 WHERE pg_catalog.jsonb_typeof(descendant.value) = 'object'
                   AND descendant.value ?| ARRAY[
                       'probability', 'probabilities', 'target_price', 'expected_return'
                   ]::text[]
            )
    ) THEN
        RAISE EXCEPTION 'candidate publication schema or provenance mismatch'
            USING ERRCODE = '23514';
    END IF;

    WITH supplied AS (
        SELECT *
          FROM jsonb_to_recordset(p_snapshots) AS input(
              instrument_id text,
              sector_code text,
              fundamental_profile text,
              eligible boolean,
              exclusion_codes jsonb,
              flow_score numeric,
              fundamental_score numeric,
              technical_score numeric,
              total_score numeric,
              flow_coverage numeric,
              fundamental_coverage numeric,
              technical_coverage numeric,
              evidence_strength text,
              normalization_scope text,
              factors_json jsonb,
              scenarios_json jsonb,
              provenance_json jsonb
          )
    ), hashes AS (
        SELECT value ->> 'instrument_id' AS instrument_id,
               pg_catalog.encode(
                   pg_catalog.sha256(pg_catalog.jsonb_send(value)), 'hex'
               ) AS content_sha256
          FROM jsonb_array_elements(p_snapshots)
    ), eligible_ranks AS (
        SELECT supplied.instrument_id,
               (row_number() OVER (
                   ORDER BY supplied.total_score DESC NULLS LAST, supplied.instrument_id
               ))::integer AS rank
          FROM supplied
         WHERE supplied.eligible
    )
    INSERT INTO public.stock_analysis_snapshots (
        run_id, instrument_id, sector_code, fundamental_profile, eligible,
        exclusion_codes, flow_score, fundamental_score, technical_score,
        total_score, flow_coverage, fundamental_coverage, technical_coverage,
        evidence_strength, rank, normalization_scope, factors_json,
        scenarios_json, provenance_json, content_sha256
    )
    SELECT p_run_id, supplied.instrument_id, supplied.sector_code,
           supplied.fundamental_profile, supplied.eligible,
           supplied.exclusion_codes, supplied.flow_score,
           supplied.fundamental_score, supplied.technical_score,
           supplied.total_score, supplied.flow_coverage,
           supplied.fundamental_coverage, supplied.technical_coverage,
           supplied.evidence_strength, eligible_ranks.rank,
           supplied.normalization_scope, supplied.factors_json,
           supplied.scenarios_json, supplied.provenance_json,
           hashes.content_sha256
      FROM supplied
      JOIN hashes USING (instrument_id)
      LEFT JOIN eligible_ranks USING (instrument_id);

    SELECT count(*)::integer
      INTO v_eligible_count
      FROM public.stock_analysis_snapshots AS snapshot
     WHERE snapshot.run_id = p_run_id
       AND snapshot.eligible
       AND snapshot.evidence_strength IN ('STRONG', 'MODERATE')
       AND snapshot.flow_coverage >= 0.6
       AND snapshot.fundamental_coverage >= 0.6
       AND snapshot.technical_coverage >= 0.6;
    IF v_eligible_count < 5 THEN
        RAISE EXCEPTION 'candidate publication has fewer than five supported candidates'
            USING ERRCODE = '23514';
    END IF;

    v_feed_id := pg_catalog.gen_random_uuid();
    UPDATE public.candidate_feed_snapshots AS previous
       SET status = 'SUPERSEDED', superseded_by = v_feed_id
     WHERE previous.as_of_date = v_run.as_of_date
       AND previous.universe_key = v_run.universe_key
       AND previous.status = 'PUBLISHED';
    INSERT INTO public.candidate_feed_snapshots (
        id, run_id, universe_key, as_of_date, computation_seq, status, published_at
    ) VALUES (
        v_feed_id, p_run_id, v_run.universe_key, v_run.as_of_date,
        v_run.computation_seq, 'PUBLISHED', clock_timestamp()
    );
    INSERT INTO public.candidate_feed_items (
        feed_id, run_id, stock_analysis_snapshot_id, instrument_id, rank
    )
    SELECT v_feed_id, p_run_id, snapshot.id, snapshot.instrument_id,
           (row_number() OVER (
               ORDER BY snapshot.total_score DESC, snapshot.instrument_id
           ))::integer
      FROM public.stock_analysis_snapshots AS snapshot
     WHERE snapshot.run_id = p_run_id
       AND snapshot.eligible
       AND snapshot.evidence_strength IN ('STRONG', 'MODERATE')
       AND snapshot.flow_coverage >= 0.6
       AND snapshot.fundamental_coverage >= 0.6
       AND snapshot.technical_coverage >= 0.6
     ORDER BY snapshot.total_score DESC, snapshot.instrument_id
     LIMIT 5;
    UPDATE public.stock_analysis_runs
       SET status = 'SUCCEEDED', summary_json = p_summary,
           published_at = clock_timestamp(), error_code = NULL, error_message = NULL
     WHERE id = p_run_id AND universe_key = v_run.universe_key;
    RETURN v_feed_id;
END
$publish$;
ALTER FUNCTION public.publish_candidate_analysis(uuid,uuid,integer,text,jsonb,jsonb)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.publish_candidate_analysis(uuid,uuid,integer,text,jsonb,jsonb)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.publish_candidate_analysis(uuid,uuid,integer,text,jsonb,jsonb)
    TO worker;

-- A corrected feed supersedes the historical feed row, but the succeeded run
-- remains the frozen source-of-truth for ScreenCursor v2 attribution reads.
CREATE OR REPLACE FUNCTION public.candidate_published_source_attributions(p_run_id uuid)
RETURNS TABLE (
    source text, dataset_id text, license_ref text, entitlement_id uuid,
    contract_reference text, contract_document_sha256 text
)
LANGUAGE sql
SECURITY DEFINER
STABLE
SET search_path = pg_catalog
AS $attribution$
    WITH run AS (
        SELECT run.* FROM public.stock_analysis_runs AS run
         WHERE run.id=p_run_id AND run.status='SUCCEEDED'
           AND EXISTS (SELECT 1 FROM public.candidate_feed_snapshots AS feed
                        WHERE feed.run_id=run.id
                          AND feed.universe_key=run.universe_key
                          AND feed.status IN ('PUBLISHED','SUPERSEDED'))
    ), refs AS (
        SELECT 'price'::text AS source, dataset.dataset_id, price.license_ref,
               run.price_entitlement_id AS entitlement_id,
               price.first_session AS first_use_date,
               run.as_of_date AS last_use_date
          FROM run JOIN public.candidate_price_publications AS price
            ON price.dataset_version_id=run.price_dataset_version_id
           AND price.entitlement_id=run.price_entitlement_id
           AND price.curated_generation=run.price_curated_version
           AND price.manifest_sha256=run.price_manifest_sha256
          JOIN public.dataset_versions AS dataset ON dataset.id=price.dataset_version_id
        UNION ALL
        SELECT 'universe', dataset.dataset_id, universe.license_ref,
               run.universe_entitlement_id, run.as_of_date, run.as_of_date
          FROM run JOIN public.candidate_universe_snapshots AS universe
            ON universe.id=run.universe_snapshot_id
           AND universe.entitlement_id=run.universe_entitlement_id
          JOIN public.dataset_versions AS dataset ON dataset.id=universe.dataset_version_id
        UNION ALL
        SELECT 'market_status', dataset.dataset_id, status.license_ref,
               run.status_entitlement_id, run.as_of_date, run.as_of_date
          FROM run JOIN public.candidate_market_status_observations AS status
            ON status.dataset_version_id=run.status_dataset_version_id
           AND status.manifest_sha256=run.status_manifest_sha256
           AND status.entitlement_id=run.status_entitlement_id
          JOIN public.dataset_versions AS dataset ON dataset.id=status.dataset_version_id
        UNION ALL
        SELECT 'flow', dataset.dataset_id, member.license_ref,
               run.flow_entitlement_id,
               min(flow.trade_date) OVER (PARTITION BY member.dataset_version_id),
               run.as_of_date
          FROM run JOIN public.candidate_investor_flows AS flow ON true
          JOIN public.candidate_investor_flow_snapshot_rows AS member
            ON member.flow_observation_id=flow.id
           AND member.dataset_version_id=run.flow_dataset_version_id
           AND member.entitlement_id=run.flow_entitlement_id
          JOIN public.dataset_versions AS dataset ON dataset.id=member.dataset_version_id
           AND dataset.manifest_sha256=run.flow_manifest_sha256
        UNION ALL
        SELECT 'fundamental', dataset.dataset_id, fact.license_ref,
               run.fundamental_entitlement_id, run.as_of_date, run.as_of_date
          FROM run JOIN public.candidate_fundamental_observations AS fact
            ON fact.dataset_version_id=run.fundamental_dataset_version_id
           AND fact.manifest_sha256=run.fundamental_manifest_sha256
           AND fact.entitlement_id=run.fundamental_entitlement_id
          JOIN public.dataset_versions AS dataset ON dataset.id=fact.dataset_version_id
        UNION ALL
        SELECT 'sector', dataset.dataset_id, sector.license_ref,
               run.sector_entitlement_id, run.as_of_date, run.as_of_date
          FROM run JOIN public.candidate_sector_versions AS sector
            ON sector.id=run.sector_version_id
           AND sector.entitlement_id=run.sector_entitlement_id
          JOIN public.dataset_versions AS dataset ON dataset.id=sector.dataset_version_id
    )
    SELECT DISTINCT refs.source, refs.dataset_id, refs.license_ref,
           entitlement.id, entitlement.contract_reference,
           entitlement.contract_document_sha256
      FROM refs JOIN public.data_entitlements AS entitlement
       ON entitlement.id=refs.entitlement_id
       AND entitlement.contract_reference=refs.license_ref
       AND public.candidate_source_entitlement_is_valid(
           refs.entitlement_id, refs.license_ref, refs.dataset_id,
           refs.first_use_date, refs.last_use_date)
     ORDER BY refs.source, refs.dataset_id
$attribution$;
ALTER FUNCTION public.candidate_published_source_attributions(uuid)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.candidate_published_source_attributions(uuid)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.candidate_published_source_attributions(uuid)
    TO app;
