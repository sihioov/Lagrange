SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- The app may read dataset metadata but must not receive UPDATE solely to
-- take a row lock. This narrow definer function attests and locks exactly the
-- deployment-configured READY pin inside the submission transaction.
CREATE FUNCTION public.lock_recommendation_submission_dataset(
    p_dataset_version_id uuid,
    p_dataset_id text,
    p_dataset_version text,
    p_dataset_manifest_sha256 text
) RETURNS boolean
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT EXISTS (
        SELECT 1
          FROM public.dataset_versions AS dataset
         WHERE dataset.id = p_dataset_version_id
           AND dataset.dataset_id = p_dataset_id
           AND dataset.version = p_dataset_version
           AND dataset.status = 'READY'
           AND dataset.manifest_sha256 = p_dataset_manifest_sha256
         FOR SHARE OF dataset
    );
$$;

ALTER FUNCTION public.lock_recommendation_submission_dataset(uuid, text, text, text)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.lock_recommendation_submission_dataset(uuid, text, text, text)
    FROM PUBLIC;
REVOKE ALL ON FUNCTION public.lock_recommendation_submission_dataset(uuid, text, text, text)
    FROM admin, audit_writer, research_writer, worker;
GRANT EXECUTE ON FUNCTION public.lock_recommendation_submission_dataset(uuid, text, text, text)
    TO app;
