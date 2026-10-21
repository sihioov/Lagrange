-- Separate fixed inspection input: never send install/revoke SQL on this path.
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
SET LOCAL idle_in_transaction_session_timeout = '30s';
SET LOCAL search_path = pg_catalog;
SET LOCAL DateStyle = 'ISO, YMD';

WITH expected AS (SELECT :'grant_payload'::jsonb AS p)
SELECT CASE
    WHEN current_user <> 'migration_owner' OR session_user <> 'migration_owner'
      OR pg_catalog.current_setting('transaction_read_only') <> 'on'
      OR :'grant_mode' <> 'inspect'
      OR (p ->> 'grant_id') IS DISTINCT FROM :'grant_id'
        THEN 'DB_OUTCOME_UNKNOWN'
    WHEN g.id IS NULL THEN 'INSPECT_MISSING'
    WHEN pg_catalog.jsonb_build_object(
        'grant_id', g.id::text,
        'grant_revision', g.grant_revision::text,
        'credential_slot_id', g.credential_slot_id::text,
        'credential_generation', g.credential_generation::text,
        'owner_user_id', g.owner_user_id::text,
        'entitlement_id', g.entitlement_id::text,
        'entitlement_reference', g.entitlement_reference::text,
        'entitlement_document_sha256', g.entitlement_document_sha256::text,
        'tr_id', g.tr_id::text,
        'wire_version', g.wire_version::text,
        'network_contract_sha256', g.network_contract_sha256::text,
        'identity_list_sha256', g.identity_list_sha256::text,
        'effective_from', g.effective_from::text,
        'effective_until', g.effective_until::text,
        'activation_commit', g.activation_commit::text
    ) IS DISTINCT FROM (p - 'schema_version' - 'scope')
      OR NOT EXISTS (
          SELECT 1 FROM public.user_roles AS owner_role
           WHERE owner_role.user_id = g.owner_user_id AND owner_role.role_id = 'owner'
      )
      OR NOT EXISTS (
          SELECT 1 FROM public.data_entitlements AS entitlement
           WHERE entitlement.id = g.entitlement_id AND entitlement.status = 'ACTIVE'
             AND entitlement.contract_reference = g.entitlement_reference
             AND entitlement.contract_document_sha256 = g.entitlement_document_sha256
             AND entitlement.effective_from <= g.effective_from
             AND entitlement.effective_until >= g.effective_until
      ) THEN 'INSPECT_MISMATCH'
    WHEN g.state = 'REVOKED' THEN 'INSPECT_REVOKED'
    WHEN g.state <> 'ACTIVE' THEN 'INSPECT_MISMATCH'
    WHEN (pg_catalog.transaction_timestamp() AT TIME ZONE 'Asia/Seoul')::date
         < g.effective_from THEN 'INSPECT_NOT_YET_VALID'
    WHEN (pg_catalog.transaction_timestamp() AT TIME ZONE 'Asia/Seoul')::date
         > g.effective_until THEN 'INSPECT_EXPIRED'
    ELSE 'INSPECT_VALID'
END AS grant_inspection
FROM expected
LEFT JOIN public.owner_market_stream_grants AS g ON g.id = :'grant_id'::uuid
\gset
COMMIT;
\echo :grant_inspection
