SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

CREATE FUNCTION public.lock_recommendation_publication_inputs(
    p_owner_user_id uuid,
    p_strategy_config_id uuid,
    p_strategy_id text,
    p_strategy_version text,
    p_config_json jsonb,
    p_dataset_version_id uuid,
    p_dataset_id text,
    p_dataset_version text,
    p_dataset_status text,
    p_dataset_manifest_sha256 text,
    p_dataset_storage_path text,
    p_universe_snapshot_id text,
    p_universe_members jsonb
) RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF p_owner_user_id IS NULL
       OR p_strategy_config_id IS NULL
       OR p_dataset_version_id IS NULL
       OR p_dataset_status NOT IN ('READY', 'WARNING')
       OR jsonb_typeof(p_universe_members) <> 'array'
       OR jsonb_array_length(p_universe_members) <> 11 THEN
        RETURN false;
    END IF;

    PERFORM pg_catalog.set_config('app.actor_user_id', p_owner_user_id::text, true);

    PERFORM 1
      FROM public.user_strategy_configs
     WHERE id = p_strategy_config_id
       AND owner_user_id = p_owner_user_id
       AND is_active
       AND strategy_id = p_strategy_id
       AND strategy_version = p_strategy_version
       AND config_json = p_config_json
     FOR SHARE;
    IF NOT FOUND THEN RETURN false; END IF;

    PERFORM 1
      FROM public.dataset_versions
     WHERE id = p_dataset_version_id
       AND dataset_id = p_dataset_id
       AND version = p_dataset_version
       AND status = p_dataset_status
       AND manifest_sha256 = p_dataset_manifest_sha256
       AND storage_path = p_dataset_storage_path
     FOR SHARE;
    IF NOT FOUND THEN RETURN false; END IF;

    PERFORM 1
      FROM public.universe_snapshots
     WHERE snapshot_id = p_universe_snapshot_id
       AND instruments_json = p_universe_members
     FOR SHARE;
    IF NOT FOUND THEN RETURN false; END IF;

    RETURN true;
END;
$$;

ALTER FUNCTION public.lock_recommendation_publication_inputs(uuid, uuid, text, text, jsonb, uuid, text, text, text, text, text, text, jsonb) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.lock_recommendation_publication_inputs(uuid, uuid, text, text, jsonb, uuid, text, text, text, text, text, text, jsonb) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.lock_recommendation_publication_inputs(uuid, uuid, text, text, jsonb, uuid, text, text, text, text, text, text, jsonb) FROM app, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.lock_recommendation_publication_inputs(uuid, uuid, text, text, jsonb, uuid, text, text, text, text, text, text, jsonb) TO worker;
