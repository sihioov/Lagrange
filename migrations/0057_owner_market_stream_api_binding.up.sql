-- 0057: the app can attest immutable deployment pins, without reading a
-- private grant, provider credential, entitlement document, or producer fence.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

CREATE FUNCTION public.owner_market_stream_api_binding_valid(
    p_owner_id uuid,
    p_session_hash text,
    p_slot_id uuid,
    p_grant_id uuid,
    p_contract_sha256 text
) RETURNS boolean
LANGUAGE plpgsql
VOLATILE
SECURITY DEFINER
SET search_path = pg_catalog
AS $owner_market_stream_api_binding_valid$
BEGIN
    IF session_user <> 'app' THEN
        RAISE EXCEPTION USING ERRCODE = '42501', MESSAGE = 'permission denied';
    END IF;
    IF p_owner_id IS NULL OR p_owner_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_slot_id IS NULL OR p_slot_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_grant_id IS NULL OR p_grant_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_session_hash IS NULL OR p_session_hash !~ '^[0-9a-f]{64}$'
       OR p_contract_sha256 IS NULL OR p_contract_sha256 !~ '^[0-9a-f]{64}$'
       OR pg_catalog.current_setting('app.actor_user_id', true) IS DISTINCT FROM p_owner_id::text
       OR pg_catalog.current_setting('app.market_stream_session_hash', true) IS DISTINCT FROM p_session_hash
       OR NOT public.owner_market_stream_session_valid(p_session_hash, p_owner_id)
    THEN
        RETURN false;
    END IF;
    RETURN EXISTS (
        SELECT 1
          FROM public.owner_market_stream_grants AS stream_grant
         WHERE stream_grant.id = p_grant_id
           AND stream_grant.owner_user_id = p_owner_id
           AND stream_grant.credential_slot_id = p_slot_id
           AND stream_grant.network_contract_sha256 = p_contract_sha256
           AND stream_grant.state = 'ACTIVE'
           AND public.owner_market_stream_rights_valid(
               stream_grant.id, p_owner_id,
               (pg_catalog.clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date
           )
    );
END
$owner_market_stream_api_binding_valid$;

ALTER FUNCTION public.owner_market_stream_api_binding_valid(uuid, text, uuid, uuid, text)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_market_stream_api_binding_valid(uuid, text, uuid, uuid, text)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.owner_market_stream_api_binding_valid(uuid, text, uuid, uuid, text)
    TO app;
