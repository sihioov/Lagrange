SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Publication holds a shared lock on every entitlement row capable of
-- authorizing this exact dataset/use/date. Revocation/deletion must wait for
-- publication to commit, closing the preflight-to-publication race without
-- granting the worker UPDATE on contract metadata.
CREATE FUNCTION public.lock_recommendation_entitlement(
    p_owner_user_id uuid,
    p_dataset_id text,
    p_as_of date
) RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF p_owner_user_id IS NULL OR p_dataset_id IS NULL OR p_dataset_id = '' OR p_as_of IS NULL THEN
        RETURN false;
    END IF;

    -- The current DB entitlement mirror covers every platform user; retaining
    -- the owner argument makes the tenant boundary explicit and rejects an
    -- orphaned claim if its user row disappeared.
    PERFORM 1 FROM public.users WHERE id = p_owner_user_id FOR SHARE;
    IF NOT FOUND THEN RETURN false; END IF;

    PERFORM 1
      FROM public.data_entitlements
     WHERE status = 'ACTIVE'
       AND effective_from <= p_as_of
       AND effective_until >= p_as_of
       AND covered_datasets @> pg_catalog.jsonb_build_array(p_dataset_id)
       AND covered_uses @> '["recommendation"]'::jsonb
     FOR SHARE;
    IF NOT FOUND THEN RETURN false; END IF;

    RETURN true;
END;
$$;

ALTER FUNCTION public.lock_recommendation_entitlement(uuid, text, date)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.lock_recommendation_entitlement(uuid, text, date) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.lock_recommendation_entitlement(uuid, text, date)
    FROM app, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.lock_recommendation_entitlement(uuid, text, date) TO worker;

-- Production publication re-locks every exact Raw file pinned by the
-- immutable curated manifest. SECURITY DEFINER is required because PostgreSQL
-- row locks require UPDATE privilege, which the worker must never receive on
-- shared market-data metadata.
CREATE FUNCTION public.lock_recommendation_source_pins(
    p_source_batch_ids uuid[],
    p_source_file_names text[],
    p_content_sha256s text[]
) RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    v_expected integer;
    v_locked integer;
BEGIN
    v_expected := pg_catalog.cardinality(p_source_batch_ids);
    IF v_expected IS NULL OR v_expected = 0
       OR pg_catalog.cardinality(p_source_file_names) <> v_expected
       OR pg_catalog.cardinality(p_content_sha256s) <> v_expected THEN
        RETURN false;
    END IF;
    IF EXISTS (
        SELECT 1
          FROM ROWS FROM (
                   pg_catalog.unnest(p_source_batch_ids),
                   pg_catalog.unnest(p_source_file_names),
                   pg_catalog.unnest(p_content_sha256s)
               ) AS expected(source_batch_id, source_file_name, content_sha256)
         GROUP BY expected.source_batch_id, expected.source_file_name
        HAVING pg_catalog.count(*) > 1
            OR pg_catalog.bool_or(
                expected.source_batch_id IS NULL
                OR expected.source_file_name IS NULL
                OR expected.source_file_name = ''
                OR expected.content_sha256 !~ '^[0-9a-f]{64}$'
            )
    ) THEN
        RETURN false;
    END IF;

    PERFORM batch.id
      FROM ROWS FROM (
               pg_catalog.unnest(p_source_batch_ids),
               pg_catalog.unnest(p_source_file_names),
               pg_catalog.unnest(p_content_sha256s)
           ) AS expected(source_batch_id, source_file_name, content_sha256)
      JOIN public.data_batches AS batch
        ON batch.source_batch_id = expected.source_batch_id
       AND batch.source_file_name = expected.source_file_name
       AND batch.content_sha256 = expected.content_sha256
       AND batch.provider = 'KRX'
       AND batch.market = 'KR'
       AND batch.fetch_mode = 'credentialed'
     FOR SHARE OF batch;
    GET DIAGNOSTICS v_locked = ROW_COUNT;
    RETURN v_locked = v_expected;
END;
$$;

ALTER FUNCTION public.lock_recommendation_source_pins(uuid[], text[], text[])
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.lock_recommendation_source_pins(uuid[], text[], text[]) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.lock_recommendation_source_pins(uuid[], text[], text[])
    FROM app, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.lock_recommendation_source_pins(uuid[], text[], text[]) TO worker;

-- A runner crash can exhaust a lease through the generic queue sweeper, which
-- otherwise has no recommendation-specific hook. Keep the linked run from
-- remaining PENDING forever, and synchronize an authorized queue cancel too.
CREATE FUNCTION public.sync_recommendation_run_from_terminal_job()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    PERFORM pg_catalog.set_config('app.actor_user_id', NEW.owner_user_id::text, true);
    IF NEW.job_type = 'recommendation'
       AND NEW.status = 'FAILED'
       AND OLD.status IS DISTINCT FROM NEW.status
       AND NEW.error_code = 'attempts_exhausted' THEN
        UPDATE public.recommendation_runs
           SET status = 'FAILED',
               summary_json = '{"code":"RECOMMENDATION_ATTEMPTS_EXHAUSTED","message":"recommendation worker attempts were exhausted"}'::jsonb
         WHERE job_id = NEW.id
           AND owner_user_id = NEW.owner_user_id
           AND status = 'PENDING';
    ELSIF NEW.job_type = 'recommendation'
       AND NEW.status = 'CANCELED'
       AND OLD.status IS DISTINCT FROM NEW.status THEN
        UPDATE public.recommendation_runs
           SET status = 'FAILED',
               summary_json = '{"code":"RECOMMENDATION_CANCELED","message":"recommendation was canceled"}'::jsonb
         WHERE job_id = NEW.id
           AND owner_user_id = NEW.owner_user_id
           AND status = 'PENDING';
    END IF;
    RETURN NEW;
END;
$$;

ALTER FUNCTION public.sync_recommendation_run_from_terminal_job() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.sync_recommendation_run_from_terminal_job() FROM PUBLIC;

CREATE TRIGGER jobs_sync_recommendation_terminal_run
AFTER UPDATE OF status ON public.jobs
FOR EACH ROW
EXECUTE FUNCTION public.sync_recommendation_run_from_terminal_job();
