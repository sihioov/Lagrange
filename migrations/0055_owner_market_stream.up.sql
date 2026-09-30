-- 0055: owner-private KIS market-stream demand, producer fencing, and
-- latest-value publication state.
--
-- This migration is intentionally independent from 0054.  The stream source
-- has no REST attempt reservation, no REST quote payload, and no tick history.
-- All six tables are FORCE-RLS and the serving roles receive only the narrow
-- columns needed by the typed repositories.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

CREATE TABLE public.owner_market_stream_grants (
    id uuid PRIMARY KEY DEFAULT pg_catalog.gen_random_uuid(),
    credential_slot_id uuid NOT NULL,
    credential_generation text NOT NULL,
    owner_user_id uuid NOT NULL
        REFERENCES public.users (id) ON DELETE RESTRICT,
    entitlement_id uuid NOT NULL
        REFERENCES public.data_entitlements (id) ON DELETE RESTRICT,
    entitlement_reference text NOT NULL,
    entitlement_document_sha256 text NOT NULL,
    tr_id text NOT NULL,
    wire_version text NOT NULL,
    network_contract_sha256 text NOT NULL,
    identity_list_sha256 text NOT NULL,
    effective_from date NOT NULL,
    effective_until date NOT NULL,
    activation_commit text NOT NULL,
    state text NOT NULL,
    grant_revision uuid NOT NULL DEFAULT pg_catalog.gen_random_uuid(),
    created_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    revoked_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT owner_market_stream_grants_slot_key
        UNIQUE (credential_slot_id, id),
    CONSTRAINT owner_market_stream_grants_revision_key
        UNIQUE (id, owner_user_id, credential_slot_id, grant_revision),
    CONSTRAINT owner_market_stream_grants_generation_check CHECK (
        credential_generation ~ '^[1-9][0-9]{0,19}$'
        AND credential_generation::numeric <= 18446744073709551615
    ),
    CONSTRAINT owner_market_stream_grants_reference_check CHECK (
        pg_catalog.btrim(entitlement_reference) <> ''
        AND pg_catalog.length(entitlement_reference) <= 512
        AND entitlement_reference !~ '[[:cntrl:]]'
    ),
    CONSTRAINT owner_market_stream_grants_entitlement_hash_check CHECK (
        entitlement_document_sha256 ~ '^[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_market_stream_grants_tr_id_check CHECK (
        tr_id = 'H0STCNT0'
    ),
    CONSTRAINT owner_market_stream_grants_wire_check CHECK (
        wire_version = 'kis-h0stcnt0-20260914-v1'
    ),
    CONSTRAINT owner_market_stream_grants_network_hash_check CHECK (
        network_contract_sha256 ~ '^[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_market_stream_grants_identity_hash_check CHECK (
        identity_list_sha256 ~ '^[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_market_stream_grants_window_check CHECK (
        effective_until >= effective_from
    ),
    CONSTRAINT owner_market_stream_grants_commit_check CHECK (
        activation_commit ~ '^[0-9a-f]{7,64}$'
    ),
    CONSTRAINT owner_market_stream_grants_state_check CHECK (
        state IN ('ACTIVE', 'REVOKED')
    ),
    CONSTRAINT owner_market_stream_grants_revoke_check CHECK (
        (state = 'ACTIVE' AND revoked_at IS NULL)
        OR (state = 'REVOKED' AND revoked_at IS NOT NULL)
    )
);

CREATE UNIQUE INDEX owner_market_stream_grants_active_slot_key
    ON public.owner_market_stream_grants (credential_slot_id)
    WHERE state = 'ACTIVE';
CREATE UNIQUE INDEX owner_market_stream_grants_active_owner_key
    ON public.owner_market_stream_grants (owner_user_id)
    WHERE state = 'ACTIVE';

CREATE TABLE public.owner_market_stream_leases (
    id uuid PRIMARY KEY DEFAULT pg_catalog.gen_random_uuid(),
    owner_user_id uuid NOT NULL
        REFERENCES public.users (id) ON DELETE RESTRICT,
    consumer_id uuid NOT NULL,
    session_hash text NOT NULL
        REFERENCES public.web_sessions (session_hash) ON DELETE RESTRICT,
    kind text NOT NULL,
    renewal_sequence bigint NOT NULL DEFAULT 0,
    state text NOT NULL DEFAULT 'ACTIVE',
    lease_expires_at timestamptz NOT NULL,
    released_at timestamptz,
    idempotency_key_sha256 text NOT NULL,
    request_sha256 text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    updated_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT owner_market_stream_leases_owner_consumer_key
        UNIQUE (owner_user_id, consumer_id),
    CONSTRAINT owner_market_stream_leases_owner_id_key
        UNIQUE (id, owner_user_id),
    CONSTRAINT owner_market_stream_leases_consumer_check CHECK (
        consumer_id <> '00000000-0000-0000-0000-000000000000'::uuid
    ),
    CONSTRAINT owner_market_stream_leases_session_hash_check CHECK (
        session_hash ~ '^[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_market_stream_leases_kind_check CHECK (
        kind = 'BROWSER'
    ),
    CONSTRAINT owner_market_stream_leases_sequence_check CHECK (
        renewal_sequence >= 0
    ),
    CONSTRAINT owner_market_stream_leases_state_check CHECK (
        state IN ('ACTIVE', 'RELEASED')
    ),
    CONSTRAINT owner_market_stream_leases_release_check CHECK (
        (state = 'ACTIVE' AND released_at IS NULL)
        OR (state = 'RELEASED' AND released_at IS NOT NULL)
    ),
    CONSTRAINT owner_market_stream_leases_idempotency_hash_check CHECK (
        idempotency_key_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_market_stream_leases_request_hash_check CHECK (
        request_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_market_stream_leases_expiry_check CHECK (
        lease_expires_at >= created_at
    )
);

CREATE INDEX owner_market_stream_leases_owner_state_expiry_idx
    ON public.owner_market_stream_leases (owner_user_id, state, lease_expires_at);
CREATE INDEX owner_market_stream_leases_session_idx
    ON public.owner_market_stream_leases (session_hash, owner_user_id, state);

CREATE TABLE public.owner_market_stream_lease_items (
    lease_id uuid NOT NULL,
    owner_user_id uuid NOT NULL,
    membership_id uuid NOT NULL,
    generation_id uuid NOT NULL,
    instrument_id text NOT NULL,
    generation bigint NOT NULL,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    PRIMARY KEY (lease_id, membership_id),
    CONSTRAINT owner_market_stream_lease_items_lease_fkey
        FOREIGN KEY (lease_id, owner_user_id)
        REFERENCES public.owner_market_stream_leases (id, owner_user_id)
        ON DELETE RESTRICT,
    CONSTRAINT owner_market_stream_lease_items_admission_fkey
        FOREIGN KEY (
            generation_id, owner_user_id, membership_id, instrument_id, generation
        ) REFERENCES public.owner_equity_generation_admissions (
            generation_id, owner_user_id, membership_id, instrument_id, generation
        ) ON DELETE RESTRICT,
    CONSTRAINT owner_market_stream_lease_items_instrument_check CHECK (
        instrument_id ~ '^[0-9]{6}[.]KRX$'
    ),
    CONSTRAINT owner_market_stream_lease_items_generation_check CHECK (
        generation > 0
    ),
    CONSTRAINT owner_market_stream_lease_items_symbol_key
        UNIQUE (lease_id, instrument_id)
);

CREATE INDEX owner_market_stream_lease_items_owner_identity_idx
    ON public.owner_market_stream_lease_items (
        owner_user_id, membership_id, instrument_id, generation
    );

CREATE TABLE public.owner_market_stream_producers (
    credential_slot_id uuid PRIMARY KEY,
    grant_id uuid NOT NULL,
    grant_revision uuid NOT NULL,
    owner_user_id uuid NOT NULL
        REFERENCES public.users (id) ON DELETE RESTRICT,
    holder_id uuid NOT NULL,
    fencing_token bigint NOT NULL,
    current_epoch uuid,
    session_date date,
    session_proof_id uuid,
    session_proof_sha256 text,
    calendar_source text,
    calendar_source_version text,
    calendar_source_batch_id uuid,
    calendar_content_sha256 text,
    window_contract_sha256 text,
    lease_expires_at timestamptz NOT NULL,
    heartbeat_at timestamptz NOT NULL,
    connection_state text NOT NULL,
    gap_generation bigint NOT NULL DEFAULT 0,
    state_version bigint NOT NULL DEFAULT 1,
    updated_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT owner_market_stream_producers_grant_fkey
        FOREIGN KEY (
            grant_id, owner_user_id, credential_slot_id, grant_revision
        ) REFERENCES public.owner_market_stream_grants (
            id, owner_user_id, credential_slot_id, grant_revision
        ) ON DELETE RESTRICT,
    CONSTRAINT owner_market_stream_producers_holder_check CHECK (
        holder_id <> '00000000-0000-0000-0000-000000000000'::uuid
    ),
    CONSTRAINT owner_market_stream_producers_fence_check CHECK (
        fencing_token > 0
    ),
    CONSTRAINT owner_market_stream_producers_lineage_check CHECK (
        (session_date IS NULL
         AND session_proof_id IS NULL
         AND session_proof_sha256 IS NULL
         AND calendar_source IS NULL
         AND calendar_source_version IS NULL
         AND calendar_source_batch_id IS NULL
         AND calendar_content_sha256 IS NULL
         AND window_contract_sha256 IS NULL)
        OR
        (session_date IS NOT NULL
         AND session_proof_id IS NOT NULL
         AND session_proof_sha256 ~ '^sha256:[0-9a-f]{64}$'
         AND calendar_source = 'kis'
         AND calendar_source_version = 'kis-chk-holiday-v1:schema-1'
         AND calendar_source_batch_id IS NOT NULL
         AND calendar_content_sha256 ~ '^[0-9a-f]{64}$'
         AND window_contract_sha256 ~ '^sha256:[0-9a-f]{64}$')
    ),
    CONSTRAINT owner_market_stream_producers_lease_check CHECK (
        lease_expires_at > heartbeat_at
    ),
    CONSTRAINT owner_market_stream_producers_state_check CHECK (
        connection_state IN ('DISCONNECTED', 'CONNECTING', 'CONNECTED', 'BACKOFF', 'STOPPED')
    ),
    CONSTRAINT owner_market_stream_producers_gap_check CHECK (
        gap_generation >= 0
    ),
    CONSTRAINT owner_market_stream_producers_state_version_check CHECK (
        state_version > 0
    )
);

CREATE TABLE public.owner_market_stream_subscriptions (
    credential_slot_id uuid NOT NULL
        REFERENCES public.owner_market_stream_producers (credential_slot_id)
        ON DELETE RESTRICT,
    symbol text NOT NULL,
    provider text NOT NULL,
    environment text NOT NULL,
    venue text NOT NULL,
    tr_id text NOT NULL,
    grant_revision uuid NOT NULL,
    epoch uuid,
    state text NOT NULL,
    pending_operation text,
    pending_ordinal bigint,
    pending_reserved_at timestamptz,
    pending_deadline timestamptz,
    acked_at timestamptz,
    desired_reference_count integer NOT NULL DEFAULT 0,
    subscription_revision uuid NOT NULL DEFAULT pg_catalog.gen_random_uuid(),
    updated_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    PRIMARY KEY (credential_slot_id, symbol),
    CONSTRAINT owner_market_stream_subscriptions_market_key_check CHECK (
        provider = 'kis'
        AND environment = 'live'
        AND venue = 'KRX'
        AND tr_id = 'H0STCNT0'
        AND symbol ~ '^[0-9]{6}$'
    ),
    CONSTRAINT owner_market_stream_subscriptions_grant_revision_check CHECK (
        grant_revision <> '00000000-0000-0000-0000-000000000000'::uuid
    ),
    CONSTRAINT owner_market_stream_subscriptions_state_check CHECK (
        state IN (
            'DESIRED', 'PENDING_SUBSCRIBE', 'ACKED', 'PENDING_UNSUBSCRIBE',
            'ABSENT', 'REJECTED', 'AMBIGUOUS'
        )
    ),
    CONSTRAINT owner_market_stream_subscriptions_operation_check CHECK (
        pending_operation IS NULL OR pending_operation IN ('SUBSCRIBE', 'UNSUBSCRIBE')
    ),
    CONSTRAINT owner_market_stream_subscriptions_pending_check CHECK (
        (pending_operation IS NULL AND pending_ordinal IS NULL
            AND pending_reserved_at IS NULL AND pending_deadline IS NULL)
        OR
        (pending_operation IS NOT NULL AND pending_ordinal IS NOT NULL
            AND pending_ordinal > 0
            AND pending_reserved_at IS NOT NULL AND pending_deadline IS NOT NULL
            AND pending_reserved_at < pending_deadline)
    ),
    CONSTRAINT owner_market_stream_subscriptions_ack_check CHECK (
        (state = 'ACKED' AND epoch IS NOT NULL AND acked_at IS NOT NULL
            AND pending_operation IS NULL)
        OR (state <> 'ACKED')
    ),
    CONSTRAINT owner_market_stream_subscriptions_reference_check CHECK (
        desired_reference_count >= 0
    )
);

CREATE INDEX owner_market_stream_subscriptions_state_idx
    ON public.owner_market_stream_subscriptions (
        credential_slot_id, state, desired_reference_count, symbol
    );

CREATE TABLE public.owner_market_stream_cache (
    owner_user_id uuid NOT NULL
        REFERENCES public.users (id) ON DELETE RESTRICT,
    membership_id uuid NOT NULL,
    row_generation uuid NOT NULL DEFAULT pg_catalog.gen_random_uuid(),
    generation_id uuid NOT NULL,
    instrument_id text NOT NULL,
    generation bigint NOT NULL,
    grant_id uuid NOT NULL,
    credential_slot_id uuid NOT NULL,
    grant_revision uuid NOT NULL,
    source text NOT NULL,
    wire_version text NOT NULL,
    venue text NOT NULL,
    currency text NOT NULL,
    session_date date NOT NULL,
    calendar_source text NOT NULL,
    calendar_source_version text NOT NULL,
    session_proof_id uuid NOT NULL,
    session_proof_sha256 text NOT NULL,
    calendar_source_batch_id uuid NOT NULL,
    calendar_content_sha256 text NOT NULL,
    window_contract_sha256 text NOT NULL,
    epoch uuid,
    fencing_token bigint,
    subscription_revision uuid,
    receive_ordinal bigint,
    price numeric(20,8),
    base_price numeric(20,8),
    change_amount numeric(20,8),
    change_percent numeric(20,8),
    direction text,
    trade_volume bigint,
    cumulative_volume bigint,
    halted boolean,
    business_date date,
    trade_time time,
    provider_trade_at timestamptz,
    quote_version bigint NOT NULL DEFAULT 0,
    state_version bigint NOT NULL DEFAULT 1,
    status_code text,
    status_at timestamptz,
    connection_state text NOT NULL,
    market_state text NOT NULL,
    freshness text NOT NULL,
    availability text NOT NULL,
    gap_since timestamptz,
    gap_generation bigint NOT NULL DEFAULT 0,
    received_at timestamptz,
    committed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    updated_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    PRIMARY KEY (owner_user_id, membership_id),
    CONSTRAINT owner_market_stream_cache_admission_fkey
        FOREIGN KEY (
            generation_id, owner_user_id, membership_id, instrument_id, generation
        ) REFERENCES public.owner_equity_generation_admissions (
            generation_id, owner_user_id, membership_id, instrument_id, generation
        ) ON DELETE RESTRICT,
    CONSTRAINT owner_market_stream_cache_grant_fkey
        FOREIGN KEY (
            grant_id, owner_user_id, credential_slot_id, grant_revision
        ) REFERENCES public.owner_market_stream_grants (
            id, owner_user_id, credential_slot_id, grant_revision
        ) ON DELETE RESTRICT,
    CONSTRAINT owner_market_stream_cache_identity_check CHECK (
        instrument_id ~ '^[0-9]{6}[.]KRX$' AND generation > 0
    ),
    CONSTRAINT owner_market_stream_cache_contract_check CHECK (
        source = 'KIS_MARKET_WS'
        AND wire_version = 'kis-h0stcnt0-20260914-v1'
        AND venue = 'KRX'
        AND currency = 'KRW'
        AND calendar_source = 'kis'
        AND calendar_source_version = 'kis-chk-holiday-v1:schema-1'
        AND session_proof_sha256 ~ '^sha256:[0-9a-f]{64}$'
        AND calendar_content_sha256 ~ '^[0-9a-f]{64}$'
        AND window_contract_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_market_stream_cache_quote_all_or_none_check CHECK (
        (price IS NULL
         AND base_price IS NULL
         AND change_amount IS NULL
         AND change_percent IS NULL
         AND direction IS NULL
         AND trade_volume IS NULL
         AND cumulative_volume IS NULL
         AND halted IS NULL
         AND business_date IS NULL
         AND trade_time IS NULL
         AND provider_trade_at IS NULL
         AND epoch IS NULL
         AND fencing_token IS NULL
         AND subscription_revision IS NULL
         AND receive_ordinal IS NULL
         AND received_at IS NULL
         AND committed_at IS NULL
         AND quote_version = 0)
        OR
        (price IS NOT NULL
         AND base_price IS NULL
         AND change_amount IS NOT NULL
         AND change_percent IS NOT NULL
         AND direction IS NOT NULL
         AND trade_volume IS NOT NULL AND trade_volume >= 0
         AND cumulative_volume IS NOT NULL AND cumulative_volume >= 0
         AND halted IS NOT NULL
         AND business_date IS NOT NULL
         AND trade_time IS NOT NULL
         AND provider_trade_at IS NOT NULL
         AND epoch IS NOT NULL
         AND fencing_token > 0
         AND subscription_revision IS NOT NULL
         AND receive_ordinal > 0
         AND received_at IS NOT NULL
         AND committed_at IS NOT NULL
         AND quote_version > 0
         AND price > 0
         AND provider_trade_at AT TIME ZONE 'Asia/Seoul' =
             business_date + trade_time)
    ),
    CONSTRAINT owner_market_stream_cache_quote_precision_check CHECK (
        price IS NULL OR price <= 999999999999.99999999
    ),
    CONSTRAINT owner_market_stream_cache_direction_check CHECK (
        direction IS NULL OR direction IN ('UP', 'DOWN', 'FLAT', 'LIMIT_UP', 'LIMIT_DOWN')
    ),
    CONSTRAINT owner_market_stream_cache_versions_check CHECK (
        quote_version >= 0 AND state_version > 0
    ),
    CONSTRAINT owner_market_stream_cache_status_check CHECK (
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
    CONSTRAINT owner_market_stream_cache_connection_check CHECK (
        connection_state IN ('DISCONNECTED', 'CONNECTING', 'CONNECTED', 'BACKOFF', 'STOPPED')
    ),
    CONSTRAINT owner_market_stream_cache_market_check CHECK (
        market_state IN ('OPEN', 'CLOSED', 'UNKNOWN')
    ),
    CONSTRAINT owner_market_stream_cache_freshness_check CHECK (
        freshness IN ('RECENT', 'STALE', 'UNAVAILABLE')
    ),
    CONSTRAINT owner_market_stream_cache_availability_check CHECK (
        availability IN ('LIVE', 'LAST_KNOWN', 'AWAITING_FIRST_TRADE', 'UNAVAILABLE')
    ),
    CONSTRAINT owner_market_stream_cache_gap_check CHECK (
        gap_generation >= 0
    )
);

CREATE INDEX owner_market_stream_cache_owner_status_idx
    ON public.owner_market_stream_cache (owner_user_id, updated_at, membership_id);
CREATE INDEX owner_market_stream_cache_retention_idx
    ON public.owner_market_stream_cache (
        (COALESCE(received_at, status_at, updated_at))
    );

ALTER TABLE public.owner_market_stream_grants OWNER TO migration_owner;
ALTER TABLE public.owner_market_stream_leases OWNER TO migration_owner;
ALTER TABLE public.owner_market_stream_lease_items OWNER TO migration_owner;
ALTER TABLE public.owner_market_stream_producers OWNER TO migration_owner;
ALTER TABLE public.owner_market_stream_subscriptions OWNER TO migration_owner;
ALTER TABLE public.owner_market_stream_cache OWNER TO migration_owner;

ALTER TABLE public.owner_market_stream_grants ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_market_stream_grants FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_market_stream_leases ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_market_stream_leases FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_market_stream_lease_items ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_market_stream_lease_items FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_market_stream_producers ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_market_stream_producers FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_market_stream_subscriptions ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_market_stream_subscriptions FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_market_stream_cache ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_market_stream_cache FORCE ROW LEVEL SECURITY;

REVOKE ALL ON TABLE public.owner_market_stream_grants
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON TABLE public.owner_market_stream_leases
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON TABLE public.owner_market_stream_lease_items
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON TABLE public.owner_market_stream_producers
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON TABLE public.owner_market_stream_subscriptions
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON TABLE public.owner_market_stream_cache
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

-- The owner API owns only the demand surface.  It never receives a quote,
-- producer, subscription, or grant write privilege.
GRANT SELECT ON TABLE public.owner_market_stream_leases,
    public.owner_market_stream_lease_items TO app;
GRANT INSERT (
    id, owner_user_id, consumer_id, session_hash, kind, renewal_sequence,
    state, lease_expires_at, idempotency_key_sha256, request_sha256
) ON public.owner_market_stream_leases TO app;
GRANT UPDATE (
    state, renewal_sequence, lease_expires_at, released_at,
    idempotency_key_sha256, request_sha256, updated_at
) ON public.owner_market_stream_leases TO app;
GRANT INSERT (
    lease_id, owner_user_id, membership_id, generation_id, instrument_id, generation
) ON public.owner_market_stream_lease_items TO app;
GRANT DELETE ON public.owner_market_stream_lease_items TO app;
GRANT SELECT ON TABLE public.owner_market_stream_cache TO app;

-- The worker can observe only the active stream grant through its RLS policy;
-- it cannot install/revoke a grant or mutate 0053 admissions.
GRANT SELECT (id, credential_slot_id, owner_user_id, grant_revision, state)
    ON public.owner_market_stream_grants TO worker;
GRANT SELECT ON TABLE public.owner_market_stream_leases,
    public.owner_market_stream_lease_items,
    public.owner_market_stream_producers,
    public.owner_market_stream_subscriptions,
    public.owner_market_stream_cache TO worker;
GRANT INSERT (
    credential_slot_id, grant_id, grant_revision, owner_user_id, holder_id,
    fencing_token, current_epoch, session_date, session_proof_id,
    session_proof_sha256, calendar_source, calendar_source_version,
    calendar_source_batch_id, calendar_content_sha256, window_contract_sha256,
    lease_expires_at, heartbeat_at, connection_state, gap_generation,
    state_version, updated_at
) ON public.owner_market_stream_producers TO worker;
GRANT UPDATE (
    holder_id, fencing_token, current_epoch, session_date, session_proof_id,
    session_proof_sha256, calendar_source, calendar_source_version,
    calendar_source_batch_id, calendar_content_sha256, window_contract_sha256,
    lease_expires_at, heartbeat_at, connection_state, gap_generation,
    state_version, updated_at
) ON public.owner_market_stream_producers TO worker;
GRANT INSERT (
    credential_slot_id, symbol, provider, environment, venue, tr_id,
    grant_revision, epoch, state, pending_operation, pending_ordinal,
    pending_reserved_at, pending_deadline, acked_at, desired_reference_count,
    subscription_revision, updated_at
) ON public.owner_market_stream_subscriptions TO worker;
GRANT UPDATE (
    grant_revision, epoch, state, pending_operation, pending_ordinal,
    pending_reserved_at, pending_deadline, acked_at, desired_reference_count,
    subscription_revision, updated_at
) ON public.owner_market_stream_subscriptions TO worker;
GRANT INSERT (
    owner_user_id, membership_id, row_generation, generation_id, instrument_id,
    generation, grant_id, credential_slot_id, grant_revision, source, wire_version, venue, currency,
    session_date, calendar_source, calendar_source_version, session_proof_id,
    session_proof_sha256, calendar_source_batch_id, calendar_content_sha256,
    window_contract_sha256, epoch, fencing_token, subscription_revision,
    receive_ordinal, price, base_price, change_amount, change_percent,
    direction, trade_volume, cumulative_volume, halted, business_date, trade_time,
    provider_trade_at, quote_version, state_version, status_code, status_at,
    connection_state, market_state, freshness, availability, gap_since,
    gap_generation, received_at, committed_at, updated_at
) ON public.owner_market_stream_cache TO worker;
GRANT DELETE ON public.owner_market_stream_cache TO worker;
GRANT UPDATE (
    row_generation, generation_id, instrument_id, generation, grant_id,
    credential_slot_id, grant_revision, source, wire_version, venue, currency, session_date,
    calendar_source, calendar_source_version, session_proof_id,
    session_proof_sha256, calendar_source_batch_id, calendar_content_sha256,
    window_contract_sha256, epoch, fencing_token, subscription_revision,
    receive_ordinal, price, base_price, change_amount, change_percent,
    direction, trade_volume, cumulative_volume, halted, business_date, trade_time,
    provider_trade_at, quote_version, state_version, status_code, status_at,
    connection_state, market_state, freshness, availability, gap_since,
    gap_generation, received_at, committed_at, updated_at
) ON public.owner_market_stream_cache TO worker;

-- Define the nonlocking validation functions before any RLS policy that calls
-- them.  The locking variants are kept separate so RLS never takes a row
-- lock as a side effect of a read.
CREATE OR REPLACE FUNCTION public.owner_market_stream_rights_valid(
    p_grant_id uuid,
    p_owner_id uuid,
    p_session_date date
) RETURNS boolean
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog
AS $owner_market_stream_rights_valid$
    SELECT COALESCE(EXISTS (
        SELECT 1
          FROM public.owner_market_stream_grants AS stream_grant
          JOIN public.data_entitlements AS entitlement
            ON entitlement.id = stream_grant.entitlement_id
         WHERE stream_grant.id = p_grant_id
           AND stream_grant.owner_user_id = p_owner_id
           AND stream_grant.state = 'ACTIVE'
           AND stream_grant.effective_from <= p_session_date
           AND stream_grant.effective_until >= p_session_date
           AND stream_grant.tr_id = 'H0STCNT0'
           AND stream_grant.wire_version = 'kis-h0stcnt0-20260914-v1'
           AND stream_grant.network_contract_sha256 ~ '^[0-9a-f]{64}$'
           AND stream_grant.identity_list_sha256 =
               '0e6a9b3aef6b310685b9bd5594a39452c2902d11af623197699cf6dc46931e79'
           AND entitlement.id = stream_grant.entitlement_id
           AND entitlement.contract_reference = stream_grant.entitlement_reference
           AND entitlement.contract_document_sha256 = stream_grant.entitlement_document_sha256
           AND entitlement.status = 'ACTIVE'
           AND entitlement.effective_from <= p_session_date
           AND entitlement.effective_until >= p_session_date
           AND EXISTS (
               SELECT 1
                 FROM public.user_roles AS user_role
                WHERE user_role.user_id = p_owner_id
                  AND user_role.role_id = 'owner'
           )
    ), false)
$owner_market_stream_rights_valid$;

-- The immutable grant row is authoritative only when the coordinator-reviewed
-- approval artifact was copied by the separately pinned WS-5 installer.
-- Runtime callers cannot supply a replacement pin; syntax and fixtures grant
-- no live authority.

CREATE OR REPLACE FUNCTION public.owner_market_stream_session_valid(
    p_session_hash text,
    p_owner_id uuid
) RETURNS boolean
LANGUAGE plpgsql
VOLATILE
SECURITY DEFINER
SET search_path = pg_catalog
AS $owner_market_stream_session_valid$
DECLARE
    v_valid boolean;
    v_previous_actor text;
    v_previous_session_hash text;
    v_previous_owner_id text;
BEGIN
    IF p_session_hash IS NULL
       OR p_owner_id IS NULL
       OR p_session_hash !~ '^[0-9a-f]{64}$'
    THEN
        RETURN false;
    END IF;
    v_previous_session_hash := pg_catalog.current_setting(
        'app.market_stream_lookup_session_hash', true
    );
    v_previous_owner_id := pg_catalog.current_setting(
        'app.market_stream_lookup_owner_id', true
    );
    v_previous_actor := pg_catalog.current_setting('app.actor_user_id', true);
    PERFORM pg_catalog.set_config('app.actor_user_id', p_owner_id::text, true);
    PERFORM pg_catalog.set_config(
        'app.market_stream_lookup_session_hash', p_session_hash, true
    );
    PERFORM pg_catalog.set_config(
        'app.market_stream_lookup_owner_id', p_owner_id::text, true
    );
    SELECT EXISTS (
        SELECT 1
          FROM public.web_sessions AS session
          JOIN public.user_roles AS user_role
            ON user_role.user_id = session.user_id
           AND user_role.role_id = 'owner'
         WHERE session.session_hash = p_session_hash
           AND session.user_id = p_owner_id
           AND session.revoked_at IS NULL
           AND session.expires_at > pg_catalog.clock_timestamp()
    ) INTO v_valid;
    PERFORM pg_catalog.set_config(
        'app.market_stream_lookup_session_hash',
        COALESCE(v_previous_session_hash, ''), true
    );
    PERFORM pg_catalog.set_config(
        'app.market_stream_lookup_owner_id',
        COALESCE(v_previous_owner_id, ''), true
    );
    PERFORM pg_catalog.set_config(
        'app.actor_user_id', COALESCE(v_previous_actor, ''), true
    );
    RETURN COALESCE(v_valid, false);
END
$owner_market_stream_session_valid$;

CREATE OR REPLACE FUNCTION public.lock_owner_market_stream_rights(
    p_slot_id uuid,
    p_grant_id uuid,
    p_grant_revision uuid,
    p_owner_id uuid,
    p_session_date date
) RETURNS boolean
LANGUAGE plpgsql
VOLATILE
SECURITY DEFINER
SET search_path = pg_catalog
AS $lock_owner_market_stream_rights$
DECLARE
    v_valid boolean;
BEGIN
    IF session_user <> 'worker' THEN
        RAISE EXCEPTION USING ERRCODE = '42501', MESSAGE = 'permission denied';
    END IF;
    IF p_slot_id IS NULL OR p_slot_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_grant_id IS NULL OR p_grant_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_grant_revision IS NULL
       OR p_grant_revision = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_owner_id IS NULL OR p_owner_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_session_date IS NULL
    THEN
        RETURN false;
    END IF;
    SELECT true
      INTO v_valid
      FROM public.owner_market_stream_grants AS stream_grant
      JOIN public.data_entitlements AS entitlement
        ON entitlement.id = stream_grant.entitlement_id
     WHERE stream_grant.id = p_grant_id
       AND stream_grant.credential_slot_id = p_slot_id
       AND stream_grant.grant_revision = p_grant_revision
       AND stream_grant.owner_user_id = p_owner_id
       AND stream_grant.state = 'ACTIVE'
       AND stream_grant.effective_from <= p_session_date
       AND stream_grant.effective_until >= p_session_date
       AND stream_grant.tr_id = 'H0STCNT0'
       AND stream_grant.wire_version = 'kis-h0stcnt0-20260914-v1'
       AND stream_grant.network_contract_sha256 ~ '^[0-9a-f]{64}$'
       AND stream_grant.identity_list_sha256 =
           '0e6a9b3aef6b310685b9bd5594a39452c2902d11af623197699cf6dc46931e79'
       AND entitlement.id = stream_grant.entitlement_id
       AND entitlement.contract_reference = stream_grant.entitlement_reference
       AND entitlement.contract_document_sha256 = stream_grant.entitlement_document_sha256
       AND entitlement.status = 'ACTIVE'
       AND entitlement.effective_from <= p_session_date
       AND entitlement.effective_until >= p_session_date
       AND EXISTS (
           SELECT 1
             FROM public.user_roles AS user_role
            WHERE user_role.user_id = p_owner_id
              AND user_role.role_id = 'owner'
       )
     FOR SHARE OF stream_grant, entitlement;
    RETURN COALESCE(v_valid, false);
END
$lock_owner_market_stream_rights$;

CREATE OR REPLACE FUNCTION public.lock_owner_market_stream_session(
    p_slot_id uuid,
    p_grant_id uuid,
    p_grant_revision uuid,
    p_owner_id uuid,
    p_session_hash text
) RETURNS boolean
LANGUAGE plpgsql
VOLATILE
SECURITY DEFINER
SET search_path = pg_catalog
AS $lock_owner_market_stream_session$
DECLARE
    v_valid boolean;
    v_previous_actor text;
    v_previous_session_hash text;
    v_previous_owner_id text;
BEGIN
    IF session_user <> 'worker' THEN
        RAISE EXCEPTION USING ERRCODE = '42501', MESSAGE = 'permission denied';
    END IF;
    IF p_slot_id IS NULL OR p_slot_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_grant_id IS NULL OR p_grant_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_grant_revision IS NULL
       OR p_grant_revision = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_owner_id IS NULL OR p_owner_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_session_hash IS NULL OR p_session_hash !~ '^[0-9a-f]{64}$'
    THEN
        RETURN false;
    END IF;

    -- lock_owner_market_stream_rights already holds this immutable binding;
    -- verify it without acquiring grant/entitlement rows out of order.
    IF NOT EXISTS (
        SELECT 1
          FROM public.owner_market_stream_grants AS stream_grant
          JOIN public.data_entitlements AS entitlement
            ON entitlement.id = stream_grant.entitlement_id
         WHERE stream_grant.id = p_grant_id
           AND stream_grant.credential_slot_id = p_slot_id
           AND stream_grant.grant_revision = p_grant_revision
           AND stream_grant.owner_user_id = p_owner_id
           AND stream_grant.state = 'ACTIVE'
           AND stream_grant.tr_id = 'H0STCNT0'
           AND stream_grant.wire_version = 'kis-h0stcnt0-20260914-v1'
           AND stream_grant.network_contract_sha256 ~ '^[0-9a-f]{64}$'
           AND stream_grant.identity_list_sha256 =
               '0e6a9b3aef6b310685b9bd5594a39452c2902d11af623197699cf6dc46931e79'
           AND entitlement.id = stream_grant.entitlement_id
           AND entitlement.contract_reference = stream_grant.entitlement_reference
           AND entitlement.contract_document_sha256 = stream_grant.entitlement_document_sha256
           AND entitlement.status = 'ACTIVE'
           AND EXISTS (
               SELECT 1
                 FROM public.user_roles AS user_role
                WHERE user_role.user_id = p_owner_id
                  AND user_role.role_id = 'owner'
           )
    ) THEN
        RETURN false;
    END IF;
    v_previous_session_hash := pg_catalog.current_setting(
        'app.market_stream_lookup_session_hash', true
    );
    v_previous_owner_id := pg_catalog.current_setting(
        'app.market_stream_lookup_owner_id', true
    );
    v_previous_actor := pg_catalog.current_setting('app.actor_user_id', true);
    PERFORM pg_catalog.set_config('app.actor_user_id', p_owner_id::text, true);
    PERFORM pg_catalog.set_config(
        'app.market_stream_lookup_session_hash', p_session_hash, true
    );
    PERFORM pg_catalog.set_config(
        'app.market_stream_lookup_owner_id', p_owner_id::text, true
    );
    SELECT true
      INTO v_valid
      FROM public.web_sessions AS session
      JOIN public.user_roles AS user_role
        ON user_role.user_id = session.user_id
       AND user_role.role_id = 'owner'
     WHERE session.session_hash = p_session_hash
       AND session.user_id = p_owner_id
       AND session.revoked_at IS NULL
       AND session.expires_at > pg_catalog.clock_timestamp()
     FOR SHARE OF session, user_role;
    PERFORM pg_catalog.set_config(
        'app.market_stream_lookup_session_hash',
        COALESCE(v_previous_session_hash, ''), true
    );
    PERFORM pg_catalog.set_config(
        'app.market_stream_lookup_owner_id',
        COALESCE(v_previous_owner_id, ''), true
    );
    PERFORM pg_catalog.set_config(
        'app.actor_user_id', COALESCE(v_previous_actor, ''), true
    );
    RETURN COALESCE(v_valid, false);
END
$lock_owner_market_stream_session$;

CREATE FUNCTION public.lock_owner_market_stream_app_admission(
    p_owner_id uuid,
    p_session_hash text,
    p_membership_id uuid,
    p_instrument_id text,
    p_generation bigint
) RETURNS TABLE (
    owner_user_id uuid,
    membership_id uuid,
    generation_id uuid,
    instrument_id text,
    generation bigint
)
LANGUAGE plpgsql
VOLATILE
SECURITY DEFINER
SET search_path = pg_catalog
AS $lock_owner_market_stream_app_admission$
DECLARE
    v_previous_session_hash text;
    v_previous_owner_id text;
    v_session_valid boolean;
BEGIN
    IF session_user <> 'app' THEN
        RAISE EXCEPTION USING ERRCODE = '42501', MESSAGE = 'permission denied';
    END IF;
    IF p_owner_id IS NULL OR p_owner_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_session_hash IS NULL OR p_session_hash !~ '^[0-9a-f]{64}$'
       OR p_membership_id IS NULL
       OR p_membership_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_instrument_id IS NULL OR p_instrument_id !~ '^[0-9]{6}\.KRX$'
       OR p_generation IS NULL OR p_generation <= 0
       OR pg_catalog.current_setting('app.actor_user_id', true) IS DISTINCT FROM p_owner_id::text
       OR pg_catalog.current_setting('app.market_stream_session_hash', true)
            IS DISTINCT FROM p_session_hash
    THEN
        RETURN;
    END IF;

    -- Prove the requested current identity exists before taking the session
    -- or admission locks; a zero-row request is not a lock/oracle capability.
    IF NOT EXISTS (
        SELECT 1
          FROM public.owner_equity_memberships AS membership
          JOIN public.owner_equity_generation_admissions AS admission
            ON admission.owner_user_id = membership.owner_user_id
           AND admission.membership_id = membership.id
           AND admission.instrument_id = membership.instrument_id
         WHERE membership.owner_user_id = p_owner_id
           AND membership.id = p_membership_id
           AND membership.instrument_id = p_instrument_id
           AND membership.state = 'READY'
           AND admission.generation = p_generation
           AND NOT EXISTS (
               SELECT 1
                 FROM public.owner_equity_generation_admissions AS newer
                WHERE newer.owner_user_id = admission.owner_user_id
                  AND newer.membership_id = admission.membership_id
                  AND newer.instrument_id = admission.instrument_id
                  AND newer.generation > admission.generation
           )
    ) THEN
        RETURN;
    END IF;

    v_previous_session_hash := pg_catalog.current_setting(
        'app.market_stream_lookup_session_hash', true
    );
    v_previous_owner_id := pg_catalog.current_setting(
        'app.market_stream_lookup_owner_id', true
    );
    PERFORM pg_catalog.set_config(
        'app.market_stream_lookup_session_hash', p_session_hash, true
    );
    PERFORM pg_catalog.set_config(
        'app.market_stream_lookup_owner_id', p_owner_id::text, true
    );
    SELECT true
      INTO v_session_valid
      FROM public.web_sessions AS session
      JOIN public.user_roles AS user_role
        ON user_role.user_id = session.user_id
       AND user_role.role_id = 'owner'
     WHERE session.session_hash = p_session_hash
       AND session.user_id = p_owner_id
       AND session.revoked_at IS NULL
       AND session.expires_at > pg_catalog.clock_timestamp()
     FOR SHARE OF session, user_role;
    PERFORM pg_catalog.set_config(
        'app.market_stream_lookup_session_hash',
        COALESCE(v_previous_session_hash, ''), true
    );
    PERFORM pg_catalog.set_config(
        'app.market_stream_lookup_owner_id',
        COALESCE(v_previous_owner_id, ''), true
    );
    IF NOT COALESCE(v_session_valid, false) THEN
        RETURN;
    END IF;

    RETURN QUERY
    SELECT admission.owner_user_id, admission.membership_id,
           admission.generation_id, admission.instrument_id, admission.generation
      FROM public.owner_equity_memberships AS membership
      JOIN public.owner_equity_generation_admissions AS admission
        ON admission.owner_user_id = membership.owner_user_id
       AND admission.membership_id = membership.id
       AND admission.instrument_id = membership.instrument_id
     WHERE membership.owner_user_id = p_owner_id
       AND membership.id = p_membership_id
       AND membership.instrument_id = p_instrument_id
       AND membership.state = 'READY'
       AND admission.generation = p_generation
       AND NOT EXISTS (
           SELECT 1
             FROM public.owner_equity_generation_admissions AS newer
            WHERE newer.owner_user_id = admission.owner_user_id
              AND newer.membership_id = admission.membership_id
              AND newer.instrument_id = admission.instrument_id
              AND newer.generation > admission.generation
       )
     ORDER BY membership.id
     FOR SHARE OF membership, admission;
END
$lock_owner_market_stream_app_admission$;

CREATE FUNCTION public.lock_owner_market_stream_worker_admission(
    p_slot_id uuid,
    p_grant_id uuid,
    p_grant_revision uuid,
    p_owner_id uuid,
    p_membership_id uuid,
    p_generation_id uuid,
    p_instrument_id text,
    p_generation bigint
) RETURNS boolean
LANGUAGE plpgsql
VOLATILE
SECURITY DEFINER
SET search_path = pg_catalog
AS $lock_owner_market_stream_worker_admission$
DECLARE
    v_previous_actor text;
    v_locked boolean;
BEGIN
    IF session_user <> 'worker' THEN
        RAISE EXCEPTION USING ERRCODE = '42501', MESSAGE = 'permission denied';
    END IF;
    IF p_slot_id IS NULL OR p_slot_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_grant_id IS NULL OR p_grant_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_grant_revision IS NULL
       OR p_grant_revision = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_owner_id IS NULL OR p_owner_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_membership_id IS NULL
       OR p_membership_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_generation_id IS NULL
       OR p_generation_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_instrument_id IS NULL OR p_instrument_id !~ '^[0-9]{6}\.KRX$'
       OR p_generation IS NULL OR p_generation <= 0
    THEN
        RETURN false;
    END IF;
    IF NOT EXISTS (
        SELECT 1
          FROM public.owner_market_stream_grants AS stream_grant
          JOIN public.data_entitlements AS entitlement
            ON entitlement.id = stream_grant.entitlement_id
         WHERE stream_grant.id = p_grant_id
           AND stream_grant.credential_slot_id = p_slot_id
           AND stream_grant.grant_revision = p_grant_revision
           AND stream_grant.owner_user_id = p_owner_id
           AND stream_grant.state = 'ACTIVE'
           AND stream_grant.tr_id = 'H0STCNT0'
           AND stream_grant.wire_version = 'kis-h0stcnt0-20260914-v1'
           AND stream_grant.network_contract_sha256 ~ '^[0-9a-f]{64}$'
           AND stream_grant.identity_list_sha256 =
               '0e6a9b3aef6b310685b9bd5594a39452c2902d11af623197699cf6dc46931e79'
           AND entitlement.id = stream_grant.entitlement_id
           AND entitlement.contract_reference = stream_grant.entitlement_reference
           AND entitlement.contract_document_sha256 = stream_grant.entitlement_document_sha256
           AND entitlement.status = 'ACTIVE'
           AND EXISTS (
               SELECT 1
                 FROM public.user_roles AS user_role
                WHERE user_role.user_id = p_owner_id
                  AND user_role.role_id = 'owner'
           )
    ) THEN
        RETURN false;
    END IF;

    v_previous_actor := pg_catalog.current_setting('app.actor_user_id', true);
    PERFORM pg_catalog.set_config('app.actor_user_id', p_owner_id::text, true);
    SELECT true
      INTO v_locked
      FROM public.owner_equity_memberships AS membership
      JOIN public.owner_equity_generation_admissions AS admission
        ON admission.owner_user_id = membership.owner_user_id
       AND admission.membership_id = membership.id
       AND admission.instrument_id = membership.instrument_id
     WHERE membership.owner_user_id = p_owner_id
       AND membership.id = p_membership_id
       AND membership.instrument_id = p_instrument_id
       AND membership.state = 'READY'
       AND admission.generation_id = p_generation_id
       AND admission.generation = p_generation
       AND NOT EXISTS (
           SELECT 1
             FROM public.owner_equity_generation_admissions AS newer
            WHERE newer.owner_user_id = admission.owner_user_id
              AND newer.membership_id = admission.membership_id
              AND newer.instrument_id = admission.instrument_id
              AND newer.generation > admission.generation
       )
     ORDER BY membership.id
     FOR SHARE OF membership, admission;
    PERFORM pg_catalog.set_config(
        'app.actor_user_id', COALESCE(v_previous_actor, ''), true
    );
    RETURN COALESCE(v_locked, false);
END
$lock_owner_market_stream_worker_admission$;

CREATE FUNCTION public.lock_owner_market_stream_app_lease_items(
    p_owner_id uuid,
    p_session_hash text,
    p_lease_id uuid
) RETURNS integer
LANGUAGE plpgsql
VOLATILE
SECURITY DEFINER
SET search_path = pg_catalog
AS $lock_owner_market_stream_app_lease_items$
DECLARE
    v_item_id uuid;
    v_count integer := 0;
BEGIN
    IF session_user <> 'app' THEN
        RAISE EXCEPTION USING ERRCODE = '42501', MESSAGE = 'permission denied';
    END IF;
    IF p_owner_id IS NULL OR p_owner_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_session_hash IS NULL OR p_session_hash !~ '^[0-9a-f]{64}$'
       OR p_lease_id IS NULL OR p_lease_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR pg_catalog.current_setting('app.actor_user_id', true) IS DISTINCT FROM p_owner_id::text
       OR pg_catalog.current_setting('app.market_stream_session_hash', true)
            IS DISTINCT FROM p_session_hash
    THEN
        RETURN 0;
    END IF;
    IF NOT public.owner_market_stream_session_valid(p_session_hash, p_owner_id)
       OR NOT EXISTS (
           SELECT 1
             FROM public.owner_market_stream_leases AS lease
            WHERE lease.id = p_lease_id
              AND lease.owner_user_id = p_owner_id
              AND lease.session_hash = p_session_hash
              AND lease.state = 'ACTIVE'
              AND lease.lease_expires_at > pg_catalog.clock_timestamp()
       )
    THEN
        RETURN 0;
    END IF;
    FOR v_item_id IN
        SELECT item.membership_id
          FROM public.owner_market_stream_lease_items AS item
         WHERE item.lease_id = p_lease_id
           AND item.owner_user_id = p_owner_id
         ORDER BY item.membership_id
         FOR UPDATE OF item
    LOOP
        v_count := v_count + 1;
    END LOOP;
    IF v_count < 1 OR v_count > 30 THEN
        RETURN 0;
    END IF;
    RETURN v_count;
END
$lock_owner_market_stream_app_lease_items$;

CREATE FUNCTION public.lock_owner_market_stream_worker_demand(
    p_slot_id uuid,
    p_grant_id uuid,
    p_grant_revision uuid,
    p_owner_id uuid,
    p_membership_ids uuid[]
) RETURNS TABLE (
    lease_id uuid,
    owner_user_id uuid,
    session_hash text,
    lease_expires_at timestamptz,
    state text,
    membership_id uuid,
    generation_id uuid,
    instrument_id text,
    generation bigint
)
LANGUAGE plpgsql
VOLATILE
SECURITY DEFINER
SET search_path = pg_catalog
AS $lock_owner_market_stream_worker_demand$
DECLARE
    v_previous_actor text;
    v_current_count integer;
BEGIN
    IF session_user <> 'worker' THEN
        RAISE EXCEPTION USING ERRCODE = '42501', MESSAGE = 'permission denied';
    END IF;
    IF p_slot_id IS NULL OR p_slot_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_grant_id IS NULL OR p_grant_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_grant_revision IS NULL
       OR p_grant_revision = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_owner_id IS NULL OR p_owner_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_membership_ids IS NULL
       OR pg_catalog.cardinality(p_membership_ids) NOT BETWEEN 1 AND 30
       OR pg_catalog.array_position(p_membership_ids, NULL::uuid) IS NOT NULL
       OR EXISTS (
           SELECT 1 FROM pg_catalog.unnest(p_membership_ids) AS requested(id)
            WHERE requested.id = '00000000-0000-0000-0000-000000000000'::uuid
       )
       OR (SELECT pg_catalog.count(DISTINCT requested.id)
             FROM pg_catalog.unnest(p_membership_ids) AS requested(id))
            <> pg_catalog.cardinality(p_membership_ids)
    THEN
        RETURN;
    END IF;
    IF NOT EXISTS (
        SELECT 1
          FROM public.owner_market_stream_grants AS stream_grant
          JOIN public.data_entitlements AS entitlement
            ON entitlement.id = stream_grant.entitlement_id
         WHERE stream_grant.id = p_grant_id
           AND stream_grant.credential_slot_id = p_slot_id
           AND stream_grant.grant_revision = p_grant_revision
           AND stream_grant.owner_user_id = p_owner_id
           AND stream_grant.state = 'ACTIVE'
           AND stream_grant.tr_id = 'H0STCNT0'
           AND stream_grant.wire_version = 'kis-h0stcnt0-20260914-v1'
           AND stream_grant.network_contract_sha256 ~ '^[0-9a-f]{64}$'
           AND stream_grant.identity_list_sha256 =
               '0e6a9b3aef6b310685b9bd5594a39452c2902d11af623197699cf6dc46931e79'
           AND entitlement.id = stream_grant.entitlement_id
           AND entitlement.contract_reference = stream_grant.entitlement_reference
           AND entitlement.contract_document_sha256 = stream_grant.entitlement_document_sha256
           AND entitlement.status = 'ACTIVE'
           AND EXISTS (
               SELECT 1
                 FROM public.user_roles AS user_role
                WHERE user_role.user_id = p_owner_id
                  AND user_role.role_id = 'owner'
           )
    ) THEN
        RETURN;
    END IF;

    v_previous_actor := pg_catalog.current_setting('app.actor_user_id', true);
    PERFORM pg_catalog.set_config('app.actor_user_id', p_owner_id::text, true);
    SELECT pg_catalog.count(*)::integer
      INTO v_current_count
      FROM pg_catalog.unnest(p_membership_ids) AS requested(id)
      JOIN public.owner_equity_memberships AS membership
        ON membership.id = requested.id
       AND membership.owner_user_id = p_owner_id
       AND membership.state = 'READY'
     WHERE EXISTS (
         SELECT 1
           FROM public.owner_equity_generation_admissions AS admission
          WHERE admission.owner_user_id = membership.owner_user_id
            AND admission.membership_id = membership.id
            AND admission.instrument_id = membership.instrument_id
            AND NOT EXISTS (
                SELECT 1
                  FROM public.owner_equity_generation_admissions AS newer
                 WHERE newer.owner_user_id = admission.owner_user_id
                   AND newer.membership_id = admission.membership_id
                   AND newer.instrument_id = admission.instrument_id
                   AND newer.generation > admission.generation
            )
     );
    IF v_current_count <> pg_catalog.cardinality(p_membership_ids) THEN
        PERFORM pg_catalog.set_config(
            'app.actor_user_id', COALESCE(v_previous_actor, ''), true
        );
        RETURN;
    END IF;

    RETURN QUERY
    SELECT lease.id, lease.owner_user_id, lease.session_hash,
           lease.lease_expires_at, lease.state, item.membership_id,
           item.generation_id, item.instrument_id, item.generation
      FROM public.owner_market_stream_leases AS lease
      JOIN public.owner_market_stream_lease_items AS item
        ON item.lease_id = lease.id
       AND item.owner_user_id = lease.owner_user_id
      JOIN public.owner_equity_memberships AS membership
        ON membership.id = item.membership_id
       AND membership.owner_user_id = item.owner_user_id
       AND membership.instrument_id = item.instrument_id
       AND membership.state = 'READY'
      JOIN public.owner_equity_generation_admissions AS admission
        ON admission.owner_user_id = membership.owner_user_id
       AND admission.membership_id = membership.id
       AND admission.instrument_id = membership.instrument_id
       AND admission.generation_id = item.generation_id
       AND admission.generation = item.generation
     WHERE lease.owner_user_id = p_owner_id
       AND lease.state = 'ACTIVE'
       AND lease.lease_expires_at > pg_catalog.clock_timestamp()
       AND item.membership_id = ANY(p_membership_ids)
       AND NOT EXISTS (
           SELECT 1
             FROM public.owner_equity_generation_admissions AS newer
            WHERE newer.owner_user_id = admission.owner_user_id
              AND newer.membership_id = admission.membership_id
              AND newer.instrument_id = admission.instrument_id
              AND newer.generation > admission.generation
       )
     ORDER BY lease.session_hash, lease.id, item.membership_id
     FOR SHARE OF lease, item;
    PERFORM pg_catalog.set_config(
        'app.actor_user_id', COALESCE(v_previous_actor, ''), true
    );
END
$lock_owner_market_stream_worker_demand$;

CREATE POLICY owner_market_stream_grants_worker_select
    ON public.owner_market_stream_grants FOR SELECT TO worker
    USING (state = 'ACTIVE');
CREATE POLICY owner_market_stream_grants_owner_all
    ON public.owner_market_stream_grants FOR ALL TO migration_owner
    USING (true) WITH CHECK (true);

CREATE POLICY owner_market_stream_leases_app_select
    ON public.owner_market_stream_leases FOR SELECT TO app
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
        AND session_hash = NULLIF(
            pg_catalog.current_setting('app.market_stream_session_hash', true), ''
        )
        AND public.owner_market_stream_session_valid(session_hash, owner_user_id)
    );
CREATE POLICY owner_market_stream_leases_app_insert
    ON public.owner_market_stream_leases FOR INSERT TO app
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
        AND session_hash = NULLIF(
            pg_catalog.current_setting('app.market_stream_session_hash', true), ''
        )
        AND public.owner_market_stream_session_valid(session_hash, owner_user_id)
    );
CREATE POLICY owner_market_stream_leases_app_update
    ON public.owner_market_stream_leases FOR UPDATE TO app
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
        AND session_hash = NULLIF(
            pg_catalog.current_setting('app.market_stream_session_hash', true), ''
        )
        AND public.owner_market_stream_session_valid(session_hash, owner_user_id)
    )
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
        AND session_hash = NULLIF(
            pg_catalog.current_setting('app.market_stream_session_hash', true), ''
        )
        AND public.owner_market_stream_session_valid(session_hash, owner_user_id)
    );
CREATE POLICY owner_market_stream_leases_worker_select
    ON public.owner_market_stream_leases FOR SELECT TO worker
    USING (
        state = 'ACTIVE'
        AND lease_expires_at > pg_catalog.clock_timestamp()
        AND EXISTS (
            SELECT 1
              FROM public.owner_market_stream_grants AS stream_grant
             WHERE stream_grant.owner_user_id = owner_market_stream_leases.owner_user_id
               AND stream_grant.state = 'ACTIVE'
        )
    );
CREATE POLICY owner_market_stream_leases_owner_all
    ON public.owner_market_stream_leases FOR ALL TO migration_owner
    USING (true) WITH CHECK (true);

CREATE POLICY owner_market_stream_lease_items_app_select
    ON public.owner_market_stream_lease_items FOR SELECT TO app
    USING (
        EXISTS (
            SELECT 1
              FROM public.owner_market_stream_leases AS lease
             WHERE lease.id = owner_market_stream_lease_items.lease_id
               AND lease.owner_user_id = owner_market_stream_lease_items.owner_user_id
               AND lease.owner_user_id = NULLIF(
                   pg_catalog.current_setting('app.actor_user_id', true), ''
               )::uuid
               AND lease.session_hash = NULLIF(
                   pg_catalog.current_setting('app.market_stream_session_hash', true), ''
               )
               AND public.owner_market_stream_session_valid(
                   lease.session_hash, lease.owner_user_id
               )
               AND lease.state = 'ACTIVE'
        )
    );
CREATE POLICY owner_market_stream_lease_items_app_insert
    ON public.owner_market_stream_lease_items FOR INSERT TO app
    WITH CHECK (
        EXISTS (
            SELECT 1
              FROM public.owner_market_stream_leases AS lease
             WHERE lease.id = owner_market_stream_lease_items.lease_id
               AND lease.owner_user_id = owner_market_stream_lease_items.owner_user_id
               AND lease.owner_user_id = NULLIF(
                   pg_catalog.current_setting('app.actor_user_id', true), ''
               )::uuid
               AND lease.session_hash = NULLIF(
                   pg_catalog.current_setting('app.market_stream_session_hash', true), ''
               )
               AND public.owner_market_stream_session_valid(
                   lease.session_hash, lease.owner_user_id
               )
               AND lease.state = 'ACTIVE'
        )
        AND EXISTS (
            SELECT 1
              FROM public.owner_equity_memberships AS membership
             WHERE membership.id = owner_market_stream_lease_items.membership_id
               AND membership.owner_user_id = owner_market_stream_lease_items.owner_user_id
               AND membership.instrument_id = owner_market_stream_lease_items.instrument_id
               AND membership.state = 'READY'
        )
    );
CREATE POLICY owner_market_stream_lease_items_app_delete
    ON public.owner_market_stream_lease_items FOR DELETE TO app
    USING (
        EXISTS (
            SELECT 1
              FROM public.owner_market_stream_leases AS lease
             WHERE lease.id = owner_market_stream_lease_items.lease_id
               AND lease.owner_user_id = owner_market_stream_lease_items.owner_user_id
               AND lease.owner_user_id = NULLIF(
                   pg_catalog.current_setting('app.actor_user_id', true), ''
               )::uuid
               AND lease.session_hash = NULLIF(
                   pg_catalog.current_setting('app.market_stream_session_hash', true), ''
               )
               AND public.owner_market_stream_session_valid(
                   lease.session_hash, lease.owner_user_id
               )
               AND lease.state = 'ACTIVE'
        )
    );
CREATE POLICY owner_market_stream_lease_items_worker_select
    ON public.owner_market_stream_lease_items FOR SELECT TO worker
    USING (
        EXISTS (
            SELECT 1
              FROM public.owner_market_stream_leases AS lease
              JOIN public.owner_market_stream_grants AS stream_grant
                ON stream_grant.owner_user_id = lease.owner_user_id
               AND stream_grant.state = 'ACTIVE'
             WHERE lease.id = owner_market_stream_lease_items.lease_id
               AND lease.owner_user_id = owner_market_stream_lease_items.owner_user_id
               AND lease.state = 'ACTIVE'
               AND lease.lease_expires_at > pg_catalog.clock_timestamp()
        )
    );
CREATE POLICY owner_market_stream_lease_items_owner_all
    ON public.owner_market_stream_lease_items FOR ALL TO migration_owner
    USING (true) WITH CHECK (true);

CREATE POLICY owner_market_stream_producers_worker_all
    ON public.owner_market_stream_producers FOR ALL TO worker
    USING (
        EXISTS (
            SELECT 1
              FROM public.owner_market_stream_grants AS stream_grant
             WHERE stream_grant.id = owner_market_stream_producers.grant_id
               AND stream_grant.owner_user_id = owner_market_stream_producers.owner_user_id
               AND stream_grant.credential_slot_id = owner_market_stream_producers.credential_slot_id
               AND stream_grant.grant_revision = owner_market_stream_producers.grant_revision
               AND stream_grant.state = 'ACTIVE'
        )
    )
    WITH CHECK (
        EXISTS (
            SELECT 1
              FROM public.owner_market_stream_grants AS stream_grant
             WHERE stream_grant.id = owner_market_stream_producers.grant_id
               AND stream_grant.owner_user_id = owner_market_stream_producers.owner_user_id
               AND stream_grant.credential_slot_id = owner_market_stream_producers.credential_slot_id
               AND stream_grant.grant_revision = owner_market_stream_producers.grant_revision
               AND stream_grant.state = 'ACTIVE'
        )
    );
CREATE POLICY owner_market_stream_producers_owner_all
    ON public.owner_market_stream_producers FOR ALL TO migration_owner
    USING (true) WITH CHECK (true);

CREATE POLICY owner_market_stream_subscriptions_worker_all
    ON public.owner_market_stream_subscriptions FOR ALL TO worker
    USING (
        EXISTS (
            SELECT 1
              FROM public.owner_market_stream_producers AS producer
              JOIN public.owner_market_stream_grants AS stream_grant
                ON stream_grant.id = producer.grant_id
               AND stream_grant.owner_user_id = producer.owner_user_id
               AND stream_grant.grant_revision = producer.grant_revision
               AND stream_grant.state = 'ACTIVE'
             WHERE producer.credential_slot_id = owner_market_stream_subscriptions.credential_slot_id
        )
    )
    WITH CHECK (
        EXISTS (
            SELECT 1
              FROM public.owner_market_stream_producers AS producer
              JOIN public.owner_market_stream_grants AS stream_grant
                ON stream_grant.id = producer.grant_id
               AND stream_grant.owner_user_id = producer.owner_user_id
               AND stream_grant.grant_revision = producer.grant_revision
               AND stream_grant.state = 'ACTIVE'
             WHERE producer.credential_slot_id = owner_market_stream_subscriptions.credential_slot_id
        )
    );
CREATE POLICY owner_market_stream_subscriptions_owner_all
    ON public.owner_market_stream_subscriptions FOR ALL TO migration_owner
    USING (true) WITH CHECK (true);

CREATE POLICY owner_market_stream_cache_app_select
    ON public.owner_market_stream_cache FOR SELECT TO app
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
        AND session_date = (
            pg_catalog.clock_timestamp() AT TIME ZONE 'Asia/Seoul'
        )::date
        AND public.owner_market_stream_rights_valid(
            grant_id,
            owner_user_id,
            session_date
        )
        AND EXISTS (
            SELECT 1
              FROM public.owner_market_stream_leases AS lease
              JOIN public.owner_market_stream_lease_items AS item
               ON item.lease_id = lease.id
               AND item.owner_user_id = lease.owner_user_id
               AND item.membership_id = owner_market_stream_cache.membership_id
               AND item.generation_id = owner_market_stream_cache.generation_id
               AND item.instrument_id = owner_market_stream_cache.instrument_id
               AND item.generation = owner_market_stream_cache.generation
             WHERE lease.owner_user_id = owner_market_stream_cache.owner_user_id
               AND lease.session_hash = NULLIF(
                   pg_catalog.current_setting('app.market_stream_session_hash', true), ''
               )
               AND public.owner_market_stream_session_valid(
                   lease.session_hash, lease.owner_user_id
               )
               AND lease.state = 'ACTIVE'
               AND lease.lease_expires_at > pg_catalog.clock_timestamp()
        )
        AND EXISTS (
            SELECT 1
              FROM public.owner_equity_memberships AS membership
              JOIN public.owner_equity_generation_admissions AS admission
                ON admission.generation_id = owner_market_stream_cache.generation_id
               AND admission.owner_user_id = owner_market_stream_cache.owner_user_id
               AND admission.membership_id = owner_market_stream_cache.membership_id
               AND admission.instrument_id = owner_market_stream_cache.instrument_id
               AND admission.generation = owner_market_stream_cache.generation
             WHERE membership.id = owner_market_stream_cache.membership_id
               AND membership.owner_user_id = owner_market_stream_cache.owner_user_id
               AND membership.instrument_id = owner_market_stream_cache.instrument_id
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
    );
CREATE POLICY owner_market_stream_cache_worker_all
    ON public.owner_market_stream_cache FOR ALL TO worker
    USING (
        public.owner_market_stream_rights_valid(
            grant_id, owner_user_id, session_date
        )
    )
    WITH CHECK (
        public.owner_market_stream_rights_valid(
            grant_id, owner_user_id, session_date
        )
    );
CREATE POLICY owner_market_stream_cache_owner_all
    ON public.owner_market_stream_cache FOR ALL TO migration_owner
    USING (true) WITH CHECK (true);

-- The helper functions are the only migration-owner path that needs to look
-- up a web session while FORCE RLS is enabled on that legacy tenant table.
-- The custom settings are transaction-local and match one exact hash/owner;
-- worker receives no direct web_sessions privilege.
CREATE POLICY owner_market_stream_session_lookup
    ON public.web_sessions FOR SELECT TO migration_owner
    USING (
        session_hash = NULLIF(
            pg_catalog.current_setting('app.market_stream_lookup_session_hash', true), ''
        )
        AND user_id = NULLIF(
            pg_catalog.current_setting('app.market_stream_lookup_owner_id', true), ''
        )::uuid
    );

CREATE FUNCTION public.owner_market_stream_grants_guard()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $owner_market_stream_grants_guard$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'owner market stream grants are immutable'
            USING ERRCODE = '55000';
    END IF;
    IF OLD.id IS DISTINCT FROM NEW.id
       OR OLD.credential_slot_id IS DISTINCT FROM NEW.credential_slot_id
       OR OLD.credential_generation IS DISTINCT FROM NEW.credential_generation
       OR OLD.owner_user_id IS DISTINCT FROM NEW.owner_user_id
       OR OLD.entitlement_id IS DISTINCT FROM NEW.entitlement_id
       OR OLD.entitlement_reference IS DISTINCT FROM NEW.entitlement_reference
       OR OLD.entitlement_document_sha256 IS DISTINCT FROM NEW.entitlement_document_sha256
       OR OLD.tr_id IS DISTINCT FROM NEW.tr_id
       OR OLD.wire_version IS DISTINCT FROM NEW.wire_version
       OR OLD.network_contract_sha256 IS DISTINCT FROM NEW.network_contract_sha256
       OR OLD.identity_list_sha256 IS DISTINCT FROM NEW.identity_list_sha256
       OR OLD.effective_from IS DISTINCT FROM NEW.effective_from
       OR OLD.effective_until IS DISTINCT FROM NEW.effective_until
       OR OLD.activation_commit IS DISTINCT FROM NEW.activation_commit
       OR OLD.grant_revision IS DISTINCT FROM NEW.grant_revision
       OR OLD.created_at IS DISTINCT FROM NEW.created_at
    THEN
        RAISE EXCEPTION 'owner market stream grant payload is immutable'
            USING ERRCODE = '55000';
    END IF;
    IF OLD.state = 'REVOKED'
       OR (OLD.state = 'ACTIVE' AND NEW.state <> 'REVOKED')
       OR (OLD.state = 'REVOKED' AND NEW.revoked_at IS NULL)
       OR (OLD.state = 'ACTIVE' AND NEW.revoked_at IS NULL)
    THEN
        RAISE EXCEPTION 'owner market stream grant state transition is invalid'
            USING ERRCODE = '55000';
    END IF;
    RETURN NEW;
END
$owner_market_stream_grants_guard$;

ALTER FUNCTION public.owner_market_stream_grants_guard() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_market_stream_grants_guard() FROM PUBLIC, app, worker, admin, research_writer;
CREATE TRIGGER owner_market_stream_grants_guard
    BEFORE UPDATE OR DELETE ON public.owner_market_stream_grants
    FOR EACH ROW EXECUTE FUNCTION public.owner_market_stream_grants_guard();

ALTER FUNCTION public.owner_market_stream_rights_valid(uuid, uuid, date)
    OWNER TO migration_owner;
ALTER FUNCTION public.owner_market_stream_session_valid(text, uuid)
    OWNER TO migration_owner;
ALTER FUNCTION public.lock_owner_market_stream_rights(uuid, uuid, uuid, uuid, date)
    OWNER TO migration_owner;
ALTER FUNCTION public.lock_owner_market_stream_session(uuid, uuid, uuid, uuid, text)
    OWNER TO migration_owner;
ALTER FUNCTION public.lock_owner_market_stream_app_admission(uuid, text, uuid, text, bigint)
    OWNER TO migration_owner;
ALTER FUNCTION public.lock_owner_market_stream_worker_admission(
    uuid, uuid, uuid, uuid, uuid, uuid, text, bigint
) OWNER TO migration_owner;
ALTER FUNCTION public.lock_owner_market_stream_app_lease_items(uuid, text, uuid)
    OWNER TO migration_owner;
ALTER FUNCTION public.lock_owner_market_stream_worker_demand(uuid, uuid, uuid, uuid, uuid[])
    OWNER TO migration_owner;

REVOKE ALL ON FUNCTION public.owner_market_stream_rights_valid(uuid, uuid, date)
    FROM PUBLIC, admin, audit_writer, research_writer;
REVOKE ALL ON FUNCTION public.owner_market_stream_session_valid(text, uuid)
    FROM PUBLIC, admin, audit_writer, research_writer;
REVOKE ALL ON FUNCTION public.lock_owner_market_stream_rights(uuid, uuid, uuid, uuid, date)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON FUNCTION public.lock_owner_market_stream_session(uuid, uuid, uuid, uuid, text)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON FUNCTION public.lock_owner_market_stream_app_admission(uuid, text, uuid, text, bigint)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON FUNCTION public.lock_owner_market_stream_worker_admission(
    uuid, uuid, uuid, uuid, uuid, uuid, text, bigint
) FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON FUNCTION public.lock_owner_market_stream_app_lease_items(uuid, text, uuid)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON FUNCTION public.lock_owner_market_stream_worker_demand(uuid, uuid, uuid, uuid, uuid[])
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.owner_market_stream_rights_valid(uuid, uuid, date)
    TO app, worker;
GRANT EXECUTE ON FUNCTION public.owner_market_stream_session_valid(text, uuid)
    TO app, worker;
GRANT EXECUTE ON FUNCTION public.lock_owner_market_stream_rights(uuid, uuid, uuid, uuid, date)
    TO worker;
GRANT EXECUTE ON FUNCTION public.lock_owner_market_stream_session(uuid, uuid, uuid, uuid, text)
    TO worker;
GRANT EXECUTE ON FUNCTION public.lock_owner_market_stream_app_admission(uuid, text, uuid, text, bigint)
    TO app;
GRANT EXECUTE ON FUNCTION public.lock_owner_market_stream_worker_admission(
    uuid, uuid, uuid, uuid, uuid, uuid, text, bigint
) TO worker;
GRANT EXECUTE ON FUNCTION public.lock_owner_market_stream_app_lease_items(uuid, text, uuid)
    TO app;
GRANT EXECUTE ON FUNCTION public.lock_owner_market_stream_worker_demand(uuid, uuid, uuid, uuid, uuid[])
    TO worker;
