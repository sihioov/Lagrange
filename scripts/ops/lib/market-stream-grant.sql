BEGIN;

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
SET LOCAL search_path = pg_catalog;

SELECT pg_catalog.set_config(
    'lagrange.market_stream_grant.payload', :'grant_payload', true
) AS grant_payload \gset
SELECT pg_catalog.set_config(
    'lagrange.market_stream_grant.mode', :'grant_mode', true
) AS grant_mode \gset
SELECT pg_catalog.set_config(
    'lagrange.market_stream_grant.id', :'grant_id', true
) AS grant_id \gset

DO $grant_install$
DECLARE
    v_payload jsonb;
    v_mode text;
    v_grant_id uuid;
    v_slot_id uuid;
    v_owner_id uuid;
    v_entitlement_id uuid;
    v_from date;
    v_until date;
    v_existing public.owner_market_stream_grants%ROWTYPE;
    v_existing_found boolean;
    v_now timestamptz;
    v_result text;
BEGIN
    IF current_user <> 'migration_owner' OR session_user <> 'migration_owner' THEN
        RAISE EXCEPTION 'grant installer role mismatch';
    END IF;

    v_payload := pg_catalog.current_setting(
        'lagrange.market_stream_grant.payload', true
    )::jsonb;
    v_mode := pg_catalog.current_setting(
        'lagrange.market_stream_grant.mode', true
    );
    v_grant_id := pg_catalog.current_setting(
        'lagrange.market_stream_grant.id', true
    )::uuid;

    IF v_mode IS NULL OR v_mode NOT IN ('install', 'revoke') OR v_grant_id IS NULL
       OR v_grant_id = '00000000-0000-0000-0000-000000000000'::uuid THEN
        RAISE EXCEPTION 'grant installer input invalid';
    END IF;

    IF v_mode = 'install' THEN
        IF pg_catalog.jsonb_typeof(v_payload) <> 'object'
           OR (v_payload ->> 'schema_version') <> '1'
           OR (v_payload ->> 'scope') <> 'owner-market-stream-grant'
           OR (v_payload ->> 'grant_id') <> v_grant_id::text THEN
            RAISE EXCEPTION 'grant approval identity invalid';
        END IF;

        v_slot_id := (v_payload ->> 'credential_slot_id')::uuid;
        v_owner_id := (v_payload ->> 'owner_user_id')::uuid;
        v_entitlement_id := (v_payload ->> 'entitlement_id')::uuid;
        v_from := (v_payload ->> 'effective_from')::date;
        v_until := (v_payload ->> 'effective_until')::date;

        IF v_slot_id IS NULL OR v_owner_id IS NULL OR v_entitlement_id IS NULL
           OR v_from IS NULL OR v_until IS NULL OR v_until < v_from THEN
            RAISE EXCEPTION 'grant approval window invalid';
        END IF;

        PERFORM pg_catalog.pg_advisory_xact_lock(
            pg_catalog.hashtextextended(
                'owner-market-stream-grant:' || v_slot_id::text, 0
            )
        );

        SELECT grant_row.*
          INTO v_existing
          FROM public.owner_market_stream_grants AS grant_row
         WHERE grant_row.id = v_grant_id
         FOR UPDATE;
        v_existing_found := FOUND;

        PERFORM entitlement.id
          FROM public.data_entitlements AS entitlement
          JOIN public.user_roles AS owner_role
            ON owner_role.user_id = v_owner_id
           AND owner_role.role_id = 'owner'
         WHERE entitlement.id = v_entitlement_id
           AND entitlement.contract_reference = (v_payload ->> 'entitlement_reference')
           AND entitlement.contract_document_sha256 =
               (v_payload ->> 'entitlement_document_sha256')
           AND entitlement.status = 'ACTIVE'
           AND entitlement.effective_from <= v_from
           AND entitlement.effective_until >= v_until
         FOR SHARE OF entitlement, owner_role;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'grant entitlement or owner role is not eligible';
        END IF;

        IF v_existing_found THEN
            IF v_existing.state = 'ACTIVE'
               AND v_existing.id = v_grant_id
               AND v_existing.grant_revision = (v_payload ->> 'grant_revision')::uuid
               AND v_existing.credential_slot_id = v_slot_id
               AND v_existing.credential_generation =
                   (v_payload ->> 'credential_generation')
               AND v_existing.owner_user_id = v_owner_id
               AND v_existing.entitlement_id = v_entitlement_id
               AND v_existing.entitlement_reference =
                   (v_payload ->> 'entitlement_reference')
               AND v_existing.entitlement_document_sha256 =
                   (v_payload ->> 'entitlement_document_sha256')
               AND v_existing.tr_id = (v_payload ->> 'tr_id')
               AND v_existing.wire_version = (v_payload ->> 'wire_version')
               AND v_existing.network_contract_sha256 =
                   (v_payload ->> 'network_contract_sha256')
               AND v_existing.identity_list_sha256 =
                   (v_payload ->> 'identity_list_sha256')
               AND v_existing.effective_from = v_from
               AND v_existing.effective_until = v_until
               AND v_existing.activation_commit =
                   (v_payload ->> 'activation_commit') THEN
                v_result := 'ALREADY_APPLIED';
            ELSE
                RAISE EXCEPTION 'grant replay conflicts with immutable row';
            END IF;
        ELSE
            INSERT INTO public.owner_market_stream_grants (
                id,
                grant_revision,
                credential_slot_id,
                credential_generation,
                owner_user_id,
                entitlement_id,
                entitlement_reference,
                entitlement_document_sha256,
                tr_id,
                wire_version,
                network_contract_sha256,
                identity_list_sha256,
                effective_from,
                effective_until,
                activation_commit,
                state,
                revoked_at
            ) VALUES (
                v_grant_id,
                (v_payload ->> 'grant_revision')::uuid,
                v_slot_id,
                v_payload ->> 'credential_generation',
                v_owner_id,
                v_entitlement_id,
                v_payload ->> 'entitlement_reference',
                v_payload ->> 'entitlement_document_sha256',
                v_payload ->> 'tr_id',
                v_payload ->> 'wire_version',
                v_payload ->> 'network_contract_sha256',
                v_payload ->> 'identity_list_sha256',
                v_from,
                v_until,
                v_payload ->> 'activation_commit',
                'ACTIVE',
                NULL
            );
            v_result := 'APPLIED';
        END IF;
    ELSE
        SELECT grant_row.*
          INTO v_existing
          FROM public.owner_market_stream_grants AS grant_row
         WHERE grant_row.id = v_grant_id
         FOR UPDATE;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'grant row is absent';
        END IF;

        IF v_existing.state = 'ACTIVE' THEN
            v_now := pg_catalog.clock_timestamp();
            UPDATE public.owner_market_stream_grants
               SET state = 'REVOKED',
                   revoked_at = v_now,
                   updated_at = v_now
             WHERE id = v_grant_id;
            v_result := 'REVOKED';
        ELSIF v_existing.state = 'REVOKED' THEN
            v_result := 'ALREADY_REVOKED';
        ELSE
            RAISE EXCEPTION 'grant state is invalid';
        END IF;
    END IF;

    PERFORM pg_catalog.set_config(
        'lagrange.market_stream_grant.result', v_result, true
    );
END;
$grant_install$;

SELECT pg_catalog.current_setting(
    'lagrange.market_stream_grant.result', true
) AS grant_result \gset
COMMIT;

\echo :grant_result
