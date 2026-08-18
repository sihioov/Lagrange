-- Fixed ETF price delivery has a narrower contract than the candidate-source
-- bridge.  Keep the candidate resolver strict, while allowing a price-only
-- release entitlement to cover the four downstream read surfaces.

-- The original 0042 ledger recorded the source-day entitlement date but not
-- the exact rights window used while deciding whether a Raw delivery could be
-- blocked.  Preserve that identity before installing the revalidation
-- procedure.  Existing rows are conservatively assigned their source day;
-- new block transitions replace this with the exact window supplied by the
-- caller.
ALTER TABLE public.candidate_raw_batch_publications
    ADD COLUMN rights_first_date date,
    ADD COLUMN rights_last_date date;
UPDATE public.candidate_raw_batch_publications
   SET rights_first_date = entitlement_date,
       rights_last_date = entitlement_date
 WHERE rights_first_date IS NULL OR rights_last_date IS NULL;
ALTER TABLE public.candidate_raw_batch_publications
    ALTER COLUMN rights_first_date SET NOT NULL,
    ALTER COLUMN rights_last_date SET NOT NULL;
ALTER TABLE public.candidate_raw_batch_publications
    ADD CONSTRAINT candidate_raw_rights_window_check
    CHECK (rights_first_date <= rights_last_date
           AND entitlement_date BETWEEN rights_first_date AND rights_last_date);

CREATE FUNCTION public.candidate_raw_rights_window_default()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $raw_rights_default$
BEGIN
    IF NEW.rights_first_date IS NULL THEN
        NEW.rights_first_date := NEW.entitlement_date;
    END IF;
    IF NEW.rights_last_date IS NULL THEN
        NEW.rights_last_date := NEW.entitlement_date;
    END IF;
    RETURN NEW;
END
$raw_rights_default$;
ALTER FUNCTION public.candidate_raw_rights_window_default() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.candidate_raw_rights_window_default() FROM PUBLIC;
CREATE TRIGGER candidate_raw_rights_window_default
    BEFORE INSERT ON public.candidate_raw_batch_publications
    FOR EACH ROW EXECUTE FUNCTION public.candidate_raw_rights_window_default();

-- Keep the existing Raw catalog API, while making its immutable coverage
-- identity explicit for both source and price rows.
CREATE OR REPLACE FUNCTION public.begin_candidate_raw_batch(
    p_batch_id uuid, p_surface text, p_raw_manifest_sha256 text,
    p_fetch_mode text, p_entitlement_reference text, p_entitlement_date date
)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $raw_begin_0046$
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
         entitlement_date,state,rights_first_date,rights_last_date)
    VALUES
        (p_batch_id,p_surface,p_raw_manifest_sha256,p_fetch_mode,
         p_entitlement_reference,p_entitlement_date,'CATALOGED',
         p_entitlement_date,p_entitlement_date)
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
$raw_begin_0046$;
ALTER FUNCTION public.begin_candidate_raw_batch(uuid,text,text,text,text,date)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.begin_candidate_raw_batch(uuid,text,text,text,text,date)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.begin_candidate_raw_batch(uuid,text,text,text,text,date)
    TO research_writer;
CREATE FUNCTION public.price_dataset_entitlement_is_valid(
    p_entitlement_id uuid,
    p_contract_reference text,
    p_first_date date,
    p_last_date date
)
RETURNS boolean
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $price_entitlement$
    SELECT EXISTS (
        SELECT 1
          FROM public.data_entitlements AS entitlement
         WHERE entitlement.id = p_entitlement_id
           AND entitlement.contract_reference = p_contract_reference
           AND entitlement.status = 'ACTIVE'
           AND entitlement.covered_datasets
                   @> '["krx_eod_bars"]'::jsonb
           AND entitlement.covered_uses
                   @> '["dataset","recommendation","backtest","paper_view"]'::jsonb
           AND entitlement.effective_from <= p_first_date
           AND entitlement.effective_until >= p_last_date
    )
$price_entitlement$;
ALTER FUNCTION public.price_dataset_entitlement_is_valid(uuid,text,date,date)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.price_dataset_entitlement_is_valid(uuid,text,date,date)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.price_dataset_entitlement_is_valid(uuid,text,date,date)
    TO research_writer;

-- Resolve one and only one exact active price entitlement.  In particular,
-- this function deliberately does not inspect candidate-universe registry
-- membership datasets or the candidate use.
CREATE FUNCTION public.resolve_price_dataset_entitlement(
    p_contract_reference text,
    p_first_date date,
    p_last_date date
)
RETURNS uuid
LANGUAGE plpgsql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $price_resolver$
DECLARE
    v_entitlement_id uuid;
    v_count bigint;
BEGIN
    IF p_contract_reference IS NULL
       OR btrim(p_contract_reference) = ''
       OR p_first_date IS NULL
       OR p_last_date IS NULL
       OR p_first_date > p_last_date
    THEN
        RAISE EXCEPTION 'price entitlement scope is invalid' USING ERRCODE = '42501';
    END IF;
    SELECT (array_agg(entitlement.id ORDER BY entitlement.id))[1], count(*)
      INTO v_entitlement_id, v_count
      FROM public.data_entitlements AS entitlement
     WHERE entitlement.contract_reference = p_contract_reference
       AND entitlement.status = 'ACTIVE'
       AND entitlement.covered_datasets
               @> '["krx_eod_bars"]'::jsonb
       AND entitlement.covered_uses
               @> '["dataset","recommendation","backtest","paper_view"]'::jsonb
       AND entitlement.effective_from <= p_first_date
       AND entitlement.effective_until >= p_last_date;
    IF v_count <> 1 THEN
        RAISE EXCEPTION 'price dataset requires one exact active entitlement'
            USING ERRCODE = '42501';
    END IF;
    RETURN v_entitlement_id;
END
$price_resolver$;
ALTER FUNCTION public.resolve_price_dataset_entitlement(text,date,date)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.resolve_price_dataset_entitlement(text,date,date)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.resolve_price_dataset_entitlement(text,date,date)
    TO research_writer;

-- 0042's price publisher remains the single publication transaction, but its
-- exact entitlement predicate is replaced explicitly here.  Candidate-source
-- callers keep the original candidate-only helper below.
CREATE OR REPLACE FUNCTION public.publish_candidate_price_publication(
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
AS $publisher_0046$
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
    IF NOT public.price_dataset_entitlement_is_valid(
        p_entitlement_id, p_license_ref, p_first_session, p_last_session
    ) THEN
        RAISE EXCEPTION 'candidate price requires an exact active price dataset entitlement'
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
$publisher_0046$;
ALTER FUNCTION public.publish_candidate_price_publication(
    text, text, text, bigint, date, date, jsonb, text, uuid, text, text, uuid,
    text, text, date, timestamptz, timestamptz
) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.publish_candidate_price_publication(
    text, text, text, bigint, date, date, jsonb, text, uuid, text, text, uuid,
    text, text, date, timestamptz, timestamptz
) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.publish_candidate_price_publication(
    text, text, text, bigint, date, date, jsonb, text, uuid, text, text, uuid,
    text, text, date, timestamptz, timestamptz
) TO research_writer;

-- Preserve the original BLOCKED row and record every rights-based re-open as
-- an immutable event.  The publication ledger row is moved back to CATALOGED
-- only after this event is durably appended; CATALOGED is the existing
-- retryable/pending state consumed by the publication procedures.
CREATE TABLE public.candidate_price_revalidation_events (
    event_id               uuid NOT NULL DEFAULT pg_catalog.gen_random_uuid(),
    batch_id               uuid NOT NULL,
    surface                text NOT NULL,
    raw_manifest_sha256    text NOT NULL,
    fetch_mode             text NOT NULL,
    entitlement_reference  text NOT NULL,
    entitlement_date       date NOT NULL,
    blocked_first_date     date NOT NULL,
    blocked_last_date      date NOT NULL,
    revalidated_first_date date NOT NULL,
    revalidated_last_date  date NOT NULL,
    previous_state         text NOT NULL,
    reopened_state         text NOT NULL,
    reason_code             text NOT NULL,
    entitlement_id         uuid NOT NULL REFERENCES public.data_entitlements(id),
    created_at              timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (event_id),
    CONSTRAINT candidate_price_revalidation_batch_fk
        FOREIGN KEY (batch_id, surface)
        REFERENCES public.candidate_raw_batch_publications(batch_id, surface)
        ON DELETE RESTRICT,
    CONSTRAINT candidate_price_revalidation_surface_check
        CHECK (surface = 'price'),
    CONSTRAINT candidate_price_revalidation_hash_check
        CHECK (raw_manifest_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT candidate_price_revalidation_mode_check
        CHECK (fetch_mode IN ('credentialed','synthetic')),
    CONSTRAINT candidate_price_revalidation_state_check
        CHECK (previous_state = 'BLOCKED' AND reopened_state = 'CATALOGED'),
    CONSTRAINT candidate_price_revalidation_reason_check
        CHECK (reason_code = 'ENTITLEMENT_REVALIDATED'),
    CONSTRAINT candidate_price_revalidation_window_check
        CHECK (blocked_first_date <= blocked_last_date
               AND revalidated_first_date <= revalidated_last_date
               AND blocked_first_date >= revalidated_first_date
               AND blocked_last_date <= revalidated_last_date
               AND entitlement_date BETWEEN blocked_first_date AND blocked_last_date
               AND entitlement_date BETWEEN revalidated_first_date
                                          AND revalidated_last_date)
);

CREATE FUNCTION public.revalidate_candidate_price_raw_batch(
    p_batch_id uuid,
    p_surface text,
    p_raw_manifest_sha256 text,
    p_fetch_mode text,
    p_entitlement_reference text,
    p_entitlement_date date,
    p_first_date date,
    p_last_date date,
    p_entitlement_id uuid
)
RETURNS uuid
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $price_revalidate$
DECLARE
    v_state text;
    v_reason_code text;
    v_raw_manifest_sha256 text;
    v_fetch_mode text;
    v_entitlement_reference text;
    v_entitlement_date date;
    v_rights_first_date date;
    v_rights_last_date date;
BEGIN
    IF p_batch_id IS NULL
       OR p_surface IS DISTINCT FROM 'price'
       OR p_raw_manifest_sha256 IS NULL
       OR p_raw_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_fetch_mode IS NULL
       OR p_fetch_mode NOT IN ('credentialed','synthetic')
       OR p_entitlement_reference IS NULL
       OR btrim(p_entitlement_reference) = ''
       OR p_entitlement_date IS NULL
       OR p_first_date IS NULL
       OR p_last_date IS NULL
       OR p_first_date > p_last_date
       OR p_entitlement_date NOT BETWEEN p_first_date AND p_last_date
       OR p_entitlement_id IS NULL
    THEN
        RAISE EXCEPTION 'price Raw revalidation identity is invalid'
            USING ERRCODE = '23514';
    END IF;
    SELECT batch.state, batch.reason_code, batch.raw_manifest_sha256,
           batch.fetch_mode, batch.entitlement_reference, batch.entitlement_date,
           batch.rights_first_date, batch.rights_last_date
      INTO v_state, v_reason_code, v_raw_manifest_sha256,
           v_fetch_mode, v_entitlement_reference, v_entitlement_date,
           v_rights_first_date, v_rights_last_date
      FROM public.candidate_raw_batch_publications AS batch
     WHERE batch.batch_id = p_batch_id
       AND batch.surface = p_surface
     FOR UPDATE;
    -- A freshly ingested Raw entry has no publication-ledger row yet; the
    -- normal publish procedure will create it.  Only an existing terminal
    -- BLOCKED row is eligible for an audited re-open.
    IF NOT FOUND THEN
        RETURN p_entitlement_id;
    END IF;
    IF v_raw_manifest_sha256 <> p_raw_manifest_sha256
       OR v_fetch_mode <> p_fetch_mode
       OR v_entitlement_reference <> p_entitlement_reference
       OR v_entitlement_date <> p_entitlement_date
    THEN
        RAISE EXCEPTION 'price Raw revalidation identity conflicts with durable ledger'
            USING ERRCODE = '23514';
    END IF;

    -- An already pending or published exact replay is a no-op.  This keeps
    -- crash recovery idempotent without manufacturing audit events.  Their
    -- immutable publication identity is checked above; coverage revalidation
    -- applies only to a terminal BLOCKED row.
    IF v_state IN ('CATALOGED','PUBLISHED') THEN
        RETURN p_entitlement_id;
    END IF;
    IF v_state <> 'BLOCKED' OR v_reason_code <> 'ENTITLEMENT_INACTIVE' THEN
        RAISE EXCEPTION 'only an entitlement-inactive price Raw block may be revalidated'
            USING ERRCODE = '42501';
    END IF;
    IF p_first_date > v_rights_first_date
       OR p_last_date < v_rights_last_date
    THEN
        RAISE EXCEPTION 'price Raw revalidation request omits original blocked coverage'
            USING ERRCODE = '23514';
    END IF;
    IF NOT public.price_dataset_entitlement_is_valid(
        p_entitlement_id, p_entitlement_reference, p_first_date, p_last_date
    ) THEN
        RAISE EXCEPTION 'price entitlement is inactive or does not cover the exact window'
            USING ERRCODE = '42501';
    END IF;
    IF public.resolve_price_dataset_entitlement(
           p_entitlement_reference, p_first_date, p_last_date
       ) IS DISTINCT FROM p_entitlement_id
    THEN
        RAISE EXCEPTION 'price revalidation entitlement is not the exact resolved contract'
            USING ERRCODE = '42501';
    END IF;
    INSERT INTO public.candidate_price_revalidation_events (
        batch_id, surface, raw_manifest_sha256, fetch_mode,
        entitlement_reference, entitlement_date, blocked_first_date,
        blocked_last_date, revalidated_first_date, revalidated_last_date,
        previous_state, reopened_state, reason_code, entitlement_id
    ) VALUES (
        p_batch_id, p_surface, p_raw_manifest_sha256, p_fetch_mode,
        p_entitlement_reference, p_entitlement_date, v_rights_first_date,
        v_rights_last_date, p_first_date, p_last_date, 'BLOCKED',
        'CATALOGED', 'ENTITLEMENT_REVALIDATED', p_entitlement_id
    );

    UPDATE public.candidate_raw_batch_publications
       SET state = 'CATALOGED', reason_code = NULL, published_at = NULL
     WHERE batch_id = p_batch_id
       AND surface = p_surface
       AND state = 'BLOCKED'
       AND reason_code = 'ENTITLEMENT_INACTIVE';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'price Raw revalidation state transition was lost'
            USING ERRCODE = '40001';
    END IF;
    RETURN p_entitlement_id;
END
$price_revalidate$;
ALTER FUNCTION public.revalidate_candidate_price_raw_batch(
    uuid,text,text,text,text,date,date,date,uuid
) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.revalidate_candidate_price_raw_batch(
    uuid,text,text,text,text,date,date,date,uuid
) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.revalidate_candidate_price_raw_batch(
    uuid,text,text,text,text,date,date,date,uuid
) TO research_writer;

-- The 0042 blocker was candidate-source-wide.  Keep that contract for source
-- rows, but give the fixed ETF price surface its own exact four-use check so a
-- price-only entitlement is not mistaken for candidate-source permission.
CREATE OR REPLACE FUNCTION public.block_candidate_raw_batch_for_inactive_rights(
    p_batch_id uuid, p_surface text, p_raw_manifest_sha256 text,
    p_fetch_mode text, p_entitlement_reference text, p_entitlement_date date,
    p_first_date date, p_last_date date
)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $raw_block_0046$
BEGIN
    IF p_batch_id IS NULL OR p_surface NOT IN ('source','price')
       OR p_raw_manifest_sha256 !~ '^[0-9a-f]{64}$'
       OR p_fetch_mode NOT IN ('credentialed','synthetic')
       OR p_entitlement_reference IS NULL OR btrim(p_entitlement_reference)=''
       OR p_entitlement_date IS NULL
       OR p_first_date IS NULL OR p_last_date IS NULL OR p_first_date > p_last_date
       OR p_entitlement_date NOT BETWEEN p_first_date AND p_last_date
    THEN
        RAISE EXCEPTION 'candidate Raw terminal block rights window is invalid'
            USING ERRCODE='23514';
    END IF;
    IF p_surface = 'price' THEN
        IF EXISTS (
            SELECT 1 FROM public.data_entitlements AS entitlement
             WHERE entitlement.contract_reference=p_entitlement_reference
               AND entitlement.status='ACTIVE'
               AND entitlement.covered_datasets @> '["krx_eod_bars"]'::jsonb
               AND entitlement.covered_uses
                     @> '["dataset","recommendation","backtest","paper_view"]'::jsonb
               AND entitlement.effective_from <= p_first_date
               AND entitlement.effective_until >= p_last_date
        ) THEN
            RAISE EXCEPTION 'active price rights cannot be recorded as a terminal Raw block'
                USING ERRCODE='23514';
        END IF;
    ELSIF EXISTS (
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
       SET state='BLOCKED', reason_code='ENTITLEMENT_INACTIVE', published_at=NULL,
           rights_first_date=p_first_date, rights_last_date=p_last_date
     WHERE batch_id=p_batch_id AND surface=p_surface AND state='CATALOGED'
       AND raw_manifest_sha256=p_raw_manifest_sha256 AND fetch_mode=p_fetch_mode
       AND entitlement_reference=p_entitlement_reference
       AND entitlement_date=p_entitlement_date;
    INSERT INTO public.candidate_raw_batch_publications
        (batch_id,surface,raw_manifest_sha256,fetch_mode,entitlement_reference,
         entitlement_date,state,reason_code,rights_first_date,rights_last_date)
    VALUES
        (p_batch_id,p_surface,p_raw_manifest_sha256,p_fetch_mode,
         p_entitlement_reference,p_entitlement_date,'BLOCKED','ENTITLEMENT_INACTIVE',
         p_first_date,p_last_date)
    ON CONFLICT (batch_id,surface) DO NOTHING;
    IF NOT EXISTS (
        SELECT 1 FROM public.candidate_raw_batch_publications AS batch
         WHERE batch.batch_id=p_batch_id AND batch.surface=p_surface
           AND batch.raw_manifest_sha256=p_raw_manifest_sha256
           AND batch.fetch_mode=p_fetch_mode
           AND batch.entitlement_reference=p_entitlement_reference
           AND batch.entitlement_date=p_entitlement_date
           AND batch.rights_first_date=p_first_date
           AND batch.rights_last_date=p_last_date
           AND batch.state='BLOCKED' AND batch.reason_code='ENTITLEMENT_INACTIVE'
    ) THEN
        RAISE EXCEPTION 'candidate Raw terminal block conflicts with durable state'
            USING ERRCODE='23514';
    END IF;
END
$raw_block_0046$;
ALTER FUNCTION public.block_candidate_raw_batch_for_inactive_rights(
    uuid,text,text,text,text,date,date,date
) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.block_candidate_raw_batch_for_inactive_rights(
    uuid,text,text,text,text,date,date,date
) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.block_candidate_raw_batch_for_inactive_rights(
    uuid,text,text,text,text,date,date,date
) TO research_writer;

-- The old helper remains candidate-use strict for every candidate source and
-- analysis caller.  The one fixed-ETF price publisher call site was routed
-- above to the narrower four-use price entitlement and never changes this
-- candidate contract.
CREATE OR REPLACE FUNCTION public.candidate_source_entitlement_is_valid(
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
ALTER FUNCTION public.candidate_source_entitlement_is_valid(uuid,text,text,date,date)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.candidate_source_entitlement_is_valid(uuid,text,text,date,date)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.candidate_source_entitlement_is_valid(uuid,text,text,date,date)
    TO research_writer;

-- Revalidation events are append-only evidence.  Direct DML remains denied;
-- only the SECURITY DEFINER transition above may append one.
CREATE TRIGGER candidate_price_revalidation_events_immutable
    BEFORE UPDATE OR DELETE ON public.candidate_price_revalidation_events
    FOR EACH ROW EXECUTE FUNCTION public.candidate_source_reject_mutation();
ALTER TABLE public.candidate_price_revalidation_events OWNER TO migration_owner;
ALTER TABLE public.candidate_price_revalidation_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.candidate_price_revalidation_events FORCE ROW LEVEL SECURITY;
CREATE POLICY candidate_price_revalidation_events_select
    ON public.candidate_price_revalidation_events
    FOR SELECT TO research_writer
    USING (true);
CREATE POLICY candidate_price_revalidation_events_select_worker_admin
    ON public.candidate_price_revalidation_events
    FOR SELECT TO worker, admin
    USING (true);
CREATE POLICY candidate_price_revalidation_events_owner
    ON public.candidate_price_revalidation_events
    FOR ALL TO migration_owner
    USING (true) WITH CHECK (true);
REVOKE ALL ON TABLE public.candidate_price_revalidation_events
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT SELECT ON TABLE public.candidate_price_revalidation_events
    TO worker, admin, research_writer;
