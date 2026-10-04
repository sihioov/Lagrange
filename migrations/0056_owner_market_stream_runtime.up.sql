-- 0056: private runtime lifecycle metadata and an actor-scoped safe delivery read.
-- This is additive to 0055; all earlier migration sources remain immutable.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

ALTER TABLE public.owner_market_stream_producers
    ADD COLUMN status_code text,
    ADD COLUMN status_at timestamptz,
    ADD COLUMN gap_since timestamptz,
    ADD COLUMN session_has_gap boolean NOT NULL DEFAULT false,
    ADD CONSTRAINT owner_market_stream_producers_status_check CHECK (
        status_code IS NULL OR status_code IN (
            'FEATURE_DISABLED', 'NO_ACTIVE_DEMAND', 'CALENDAR_UNAVAILABLE',
            'SESSION_WINDOW_UNAVAILABLE', 'SESSION_CLOSED', 'AWAITING_FIRST_TRADE',
            'QUOTE_STALE', 'CONNECTION_LOST', 'RECONNECT_GAP',
            'SUBSCRIPTION_PENDING', 'SUBSCRIPTION_REJECTED',
            'SUBSCRIPTION_AMBIGUOUS', 'APPROVAL_UNAVAILABLE', 'BUDGET_EXHAUSTED',
            'PRODUCER_UNAVAILABLE', 'PIPELINE_LAG', 'WIRE_SCHEMA_MISMATCH',
            'PROVIDER_RESPONSE_INVALID', 'QUOTE_VALUE_INVALID',
            'MARKET_CLASS_UNSUPPORTED', 'LOCAL_INGRESS_LIMIT', 'RESYNC_REQUIRED',
            'ACCESS_REVOKED'
        )
    ),
    ADD CONSTRAINT owner_market_stream_producers_status_at_check CHECK (
        (status_code IS NULL AND status_at IS NULL)
        OR (status_code IS NOT NULL AND status_at IS NOT NULL)
    );

-- Existing table-level producer SELECT remains unchanged. Only the four
-- lifecycle columns are added to worker write authority.
GRANT UPDATE (status_code, status_at, gap_since, session_has_gap)
    ON public.owner_market_stream_producers TO worker;

-- The runtime compares only the three adopted grant pins. The worker still
-- cannot read entitlement payload, credentials, or other grant material.
GRANT SELECT (credential_generation, network_contract_sha256, identity_list_sha256)
    ON public.owner_market_stream_grants TO worker;

CREATE FUNCTION public.owner_market_stream_delivery_state(
    p_owner_id uuid,
    p_session_hash text,
    p_lease_id uuid
) RETURNS TABLE (
    owner_user_id uuid,
    membership_id uuid,
    generation_id uuid,
    instrument_id text,
    generation bigint,
    subscription_state text,
    subscription_epoch uuid,
    subscription_revision uuid,
    subscription_updated_at timestamptz,
    producer_connection_state text,
    producer_status_code text,
    producer_status_at timestamptz,
    producer_heartbeat_at timestamptz,
    producer_lease_expires_at timestamptz,
    producer_current_epoch uuid,
    producer_session_date date,
    producer_session_proof_id uuid,
    producer_session_proof_sha256 text,
    producer_calendar_source_batch_id uuid,
    producer_calendar_content_sha256 text,
    producer_window_contract_sha256 text,
    producer_gap_since timestamptz,
    producer_session_has_gap boolean,
    producer_gap_generation bigint,
    producer_state_version bigint
)
LANGUAGE plpgsql
VOLATILE
SECURITY DEFINER
SET search_path = pg_catalog
AS $owner_market_stream_delivery_state$
DECLARE
    v_now timestamptz := pg_catalog.clock_timestamp();
    v_session_date date := (v_now AT TIME ZONE 'Asia/Seoul')::date;
    v_item_count bigint;
    v_grant_id uuid;
    v_slot_id uuid;
BEGIN
    -- This is an app-only read capability. Reject malformed or context-free
    -- calls before touching tenant rows; invalid authorization returns no rows.
    IF session_user <> 'app'
       OR p_owner_id IS NULL
       OR p_owner_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_session_hash IS NULL
       OR p_session_hash !~ '^[0-9a-f]{64}$'
       OR p_lease_id IS NULL
       OR p_lease_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR pg_catalog.current_setting('app.actor_user_id', true)
            IS DISTINCT FROM p_owner_id::text
       OR pg_catalog.current_setting('app.market_stream_session_hash', true)
            IS DISTINCT FROM p_session_hash
    THEN
        RETURN;
    END IF;

    IF NOT public.owner_market_stream_session_valid(p_session_hash, p_owner_id)
       OR NOT EXISTS (
           SELECT 1
             FROM public.owner_market_stream_leases AS lease
            WHERE lease.id = p_lease_id
              AND lease.owner_user_id = p_owner_id
              AND lease.session_hash = p_session_hash
              AND lease.state = 'ACTIVE'
              AND lease.lease_expires_at > v_now
       )
    THEN
        RETURN;
    END IF;

    SELECT stream_grant.id, stream_grant.credential_slot_id
      INTO v_grant_id, v_slot_id
      FROM public.owner_market_stream_grants AS stream_grant
     WHERE stream_grant.owner_user_id = p_owner_id
       AND stream_grant.state = 'ACTIVE'
       AND public.owner_market_stream_rights_valid(
            stream_grant.id, p_owner_id, v_session_date
       )
     ORDER BY stream_grant.id
     LIMIT 1;
    IF v_grant_id IS NULL OR v_slot_id IS NULL THEN
        RETURN;
    END IF;

    SELECT pg_catalog.count(*)
      INTO v_item_count
      FROM public.owner_market_stream_lease_items AS item
     WHERE item.lease_id = p_lease_id
       AND item.owner_user_id = p_owner_id;
    IF v_item_count < 1 OR v_item_count > 30 THEN
        RETURN;
    END IF;

    -- A lease is valid only as a complete set of current READY identities.
    -- Never return a subset that could make a stale lease look complete.
    IF EXISTS (
        SELECT 1
          FROM public.owner_market_stream_lease_items AS item
         WHERE item.lease_id = p_lease_id
           AND item.owner_user_id = p_owner_id
           AND NOT EXISTS (
               SELECT 1
                 FROM public.owner_equity_memberships AS membership
                 JOIN public.owner_equity_generation_admissions AS admission
                   ON admission.owner_user_id = membership.owner_user_id
                  AND admission.membership_id = membership.id
                  AND admission.instrument_id = membership.instrument_id
                  AND admission.generation_id = item.generation_id
                  AND admission.generation = item.generation
                 WHERE membership.id = item.membership_id
                   AND membership.owner_user_id = item.owner_user_id
                   AND membership.instrument_id = item.instrument_id
                   AND membership.state = 'READY'
                   AND NOT EXISTS (
                       SELECT 1
                         FROM public.owner_equity_generation_admissions AS newer
                        WHERE newer.owner_user_id = admission.owner_user_id
                          AND newer.membership_id = admission.membership_id
                          AND newer.instrument_id = admission.instrument_id
                          AND newer.generation > admission.generation
                   )
           )
    ) THEN
        RETURN;
    END IF;

    RETURN QUERY
    SELECT item.owner_user_id,
           item.membership_id,
           item.generation_id,
           item.instrument_id,
           item.generation,
           subscription.state,
           subscription.epoch,
           subscription.subscription_revision,
           subscription.updated_at,
           producer.connection_state,
           producer.status_code,
           producer.status_at,
           producer.heartbeat_at,
           producer.lease_expires_at,
           producer.current_epoch,
           producer.session_date,
           producer.session_proof_id,
           producer.session_proof_sha256,
           producer.calendar_source_batch_id,
           producer.calendar_content_sha256,
           producer.window_contract_sha256,
           producer.gap_since,
           producer.session_has_gap,
           producer.gap_generation,
           producer.state_version
      FROM public.owner_market_stream_lease_items AS item
      JOIN public.owner_market_stream_leases AS lease
        ON lease.id = item.lease_id
       AND lease.owner_user_id = item.owner_user_id
      JOIN public.owner_equity_memberships AS membership
        ON membership.id = item.membership_id
       AND membership.owner_user_id = item.owner_user_id
       AND membership.instrument_id = item.instrument_id
       AND membership.state = 'READY'
      JOIN public.owner_equity_generation_admissions AS admission
        ON admission.generation_id = item.generation_id
       AND admission.owner_user_id = item.owner_user_id
       AND admission.membership_id = item.membership_id
       AND admission.instrument_id = item.instrument_id
       AND admission.generation = item.generation
      JOIN public.owner_market_stream_grants AS stream_grant
        ON stream_grant.id = v_grant_id
       AND stream_grant.credential_slot_id = v_slot_id
       AND stream_grant.owner_user_id = p_owner_id
       AND stream_grant.state = 'ACTIVE'
      LEFT JOIN public.owner_market_stream_subscriptions AS subscription
        ON subscription.credential_slot_id = v_slot_id
       AND subscription.symbol = pg_catalog.left(item.instrument_id, 6)
      LEFT JOIN public.owner_market_stream_producers AS producer
        ON producer.credential_slot_id = v_slot_id
       AND producer.grant_id = stream_grant.id
       AND producer.grant_revision = stream_grant.grant_revision
       AND producer.owner_user_id = p_owner_id
     WHERE item.lease_id = p_lease_id
       AND item.owner_user_id = p_owner_id
       AND lease.session_hash = p_session_hash
       AND lease.state = 'ACTIVE'
       AND lease.lease_expires_at > v_now
       AND admission.generation_id = item.generation_id
       AND admission.generation = item.generation
       AND NOT EXISTS (
           SELECT 1
             FROM public.owner_equity_generation_admissions AS newer
            WHERE newer.owner_user_id = admission.owner_user_id
              AND newer.membership_id = admission.membership_id
              AND newer.instrument_id = admission.instrument_id
              AND newer.generation > admission.generation
       )
     ORDER BY item.instrument_id, item.membership_id;
END
$owner_market_stream_delivery_state$;

ALTER FUNCTION public.owner_market_stream_delivery_state(uuid, text, uuid)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_market_stream_delivery_state(uuid, text, uuid)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.owner_market_stream_delivery_state(uuid, text, uuid)
    TO app;
