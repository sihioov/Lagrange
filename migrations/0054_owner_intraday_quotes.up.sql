-- 0054: owner-private, ephemeral intraday quote demand/cache state.
--
-- This migration deliberately contains no trigger, function, sequence, or
-- SECURITY DEFINER helper.  All state transitions and fencing checks live in
-- the typed job-queue repository.  The composite admission keys below keep a
-- quote tied to the exact 0053 owner/membership/generation lineage.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

CREATE TABLE public.owner_intraday_quote_demands (
    id uuid PRIMARY KEY DEFAULT pg_catalog.gen_random_uuid(),
    owner_user_id uuid NOT NULL
        REFERENCES public.users (id) ON DELETE RESTRICT,
    consumer_id uuid NOT NULL,
    membership_id uuid NOT NULL,
    generation_id uuid NOT NULL,
    instrument_id text NOT NULL,
    generation bigint NOT NULL,
    state text NOT NULL DEFAULT 'ACTIVE',
    renewal_sequence bigint NOT NULL DEFAULT 0,
    lease_expires_at timestamptz NOT NULL,
    released_at timestamptz,
    idempotency_key_sha256 text NOT NULL,
    request_sha256 text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    updated_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT owner_intraday_quote_demands_admission_fkey
        FOREIGN KEY (
            generation_id, owner_user_id, membership_id, instrument_id, generation
        ) REFERENCES public.owner_equity_generation_admissions (
            generation_id, owner_user_id, membership_id, instrument_id, generation
        ) ON DELETE RESTRICT,
    CONSTRAINT owner_intraday_quote_demands_consumer_key
        UNIQUE (owner_user_id, consumer_id),
    CONSTRAINT owner_intraday_quote_demands_instrument_check CHECK (
        instrument_id ~ '^[0-9]{6}[.]KRX$'
    ),
    CONSTRAINT owner_intraday_quote_demands_generation_check CHECK (
        generation > 0
    ),
    CONSTRAINT owner_intraday_quote_demands_state_check CHECK (
        state IN ('ACTIVE', 'RELEASED')
    ),
    CONSTRAINT owner_intraday_quote_demands_sequence_check CHECK (
        renewal_sequence >= 0
    ),
    CONSTRAINT owner_intraday_quote_demands_release_state_check CHECK (
        (state = 'ACTIVE' AND released_at IS NULL)
        OR (state = 'RELEASED' AND released_at IS NOT NULL)
    ),
    CONSTRAINT owner_intraday_quote_demands_idempotency_hash_check CHECK (
        idempotency_key_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_intraday_quote_demands_request_hash_check CHECK (
        request_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_intraday_quote_demands_expiry_check CHECK (
        lease_expires_at >= created_at
    )
);

CREATE INDEX owner_intraday_quote_demands_owner_state_expiry_idx
    ON public.owner_intraday_quote_demands (
        owner_user_id, state, lease_expires_at
    );
CREATE INDEX owner_intraday_quote_demands_merge_identity_idx
    ON public.owner_intraday_quote_demands (
        owner_user_id, membership_id, generation_id, instrument_id, generation,
        state, lease_expires_at
    );

CREATE TABLE public.owner_intraday_quote_cache (
    owner_user_id uuid NOT NULL
        REFERENCES public.users (id) ON DELETE RESTRICT,
    membership_id uuid NOT NULL,
    generation_id uuid NOT NULL,
    instrument_id text NOT NULL,
    generation bigint NOT NULL,
    session_date date,
    calendar_source text,
    calendar_source_version text,
    calendar_source_batch_id uuid,
    calendar_content_sha256 text,
    window_contract_sha256 text,
    price numeric(20,8),
    base_price numeric(20,8),
    change_amount numeric(20,8),
    change_percent numeric(20,8),
    direction text,
    halted boolean,
    received_at timestamptz,
    last_success_at timestamptz,
    quote_version bigint NOT NULL DEFAULT 0,
    last_attempt_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    last_failure_code text,
    last_failure_at timestamptz,
    producer_fence bigint NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    updated_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    PRIMARY KEY (owner_user_id, membership_id),
    CONSTRAINT owner_intraday_quote_cache_admission_fkey
        FOREIGN KEY (
            generation_id, owner_user_id, membership_id, instrument_id, generation
        ) REFERENCES public.owner_equity_generation_admissions (
            generation_id, owner_user_id, membership_id, instrument_id, generation
        ) ON DELETE RESTRICT,
    CONSTRAINT owner_intraday_quote_cache_instrument_check CHECK (
        instrument_id ~ '^[0-9]{6}[.]KRX$'
    ),
    CONSTRAINT owner_intraday_quote_cache_generation_check CHECK (
        generation > 0
    ),
    CONSTRAINT owner_intraday_quote_cache_quote_version_check CHECK (
        quote_version >= 0
    ),
    CONSTRAINT owner_intraday_quote_cache_producer_fence_check CHECK (
        producer_fence >= 0
    ),
    CONSTRAINT owner_intraday_quote_cache_session_all_or_none_check CHECK (
        (session_date IS NULL
         AND calendar_source IS NULL
         AND calendar_source_version IS NULL
         AND calendar_source_batch_id IS NULL
         AND calendar_content_sha256 IS NULL
         AND window_contract_sha256 IS NULL)
        OR
        (session_date IS NOT NULL
         AND calendar_source IS NOT NULL
         AND calendar_source_version IS NOT NULL
         AND calendar_source_batch_id IS NOT NULL
         AND calendar_content_sha256 IS NOT NULL
         AND window_contract_sha256 IS NOT NULL)
    ),
    CONSTRAINT owner_intraday_quote_cache_session_contract_check CHECK (
        session_date IS NULL
        OR (
            calendar_source = 'kis'
            AND calendar_source_version = 'kis-chk-holiday-v1:schema-1'
            AND calendar_content_sha256 ~ '^[0-9a-f]{64}$'
            AND window_contract_sha256 ~ '^sha256:[0-9a-f]{64}$'
        )
    ),
    CONSTRAINT owner_intraday_quote_cache_quote_all_or_none_check CHECK (
        (price IS NULL
         AND base_price IS NULL
         AND change_amount IS NULL
         AND change_percent IS NULL
         AND direction IS NULL
         AND halted IS NULL
         AND received_at IS NULL
         AND last_success_at IS NULL)
        OR
        (price IS NOT NULL
         AND base_price IS NOT NULL
         AND change_amount IS NOT NULL
         AND change_percent IS NOT NULL
         AND direction IS NOT NULL
         AND halted IS NOT NULL
         AND received_at IS NOT NULL
         AND last_success_at IS NOT NULL
         AND received_at = last_success_at
         AND price > 0
         AND base_price > 0)
    ),
    CONSTRAINT owner_intraday_quote_cache_direction_check CHECK (
        direction IS NULL
        OR direction IN ('LIMIT_UP', 'UP', 'FLAT', 'LIMIT_DOWN', 'DOWN')
    ),
    CONSTRAINT owner_intraday_quote_cache_failure_pair_check CHECK (
        (last_failure_code IS NULL AND last_failure_at IS NULL)
        OR (last_failure_code IS NOT NULL AND last_failure_at IS NOT NULL)
    ),
    CONSTRAINT owner_intraday_quote_cache_failure_code_check CHECK (
        last_failure_code IS NULL
        OR last_failure_code IN (
            'NO_ACTIVE_DEMAND', 'QUOTE_PENDING', 'QUOTE_STALE',
            'PROVIDER_TIMEOUT', 'PROVIDER_RATE_LIMITED',
            'PROVIDER_UNAVAILABLE', 'PROVIDER_RESPONSE_INVALID',
            'QUOTE_VALUE_INVALID', 'QUOTE_BUDGET_EXHAUSTED',
            'CALENDAR_UNAVAILABLE', 'SESSION_WINDOW_UNAVAILABLE',
            'SESSION_CLOSED', 'INSTRUMENT_HALTED',
            'PRODUCER_UNAVAILABLE', 'FEATURE_DISABLED'
        )
    )
);

CREATE INDEX owner_intraday_quote_cache_owner_attempt_idx
    ON public.owner_intraday_quote_cache (
        owner_user_id, last_attempt_at
    );
CREATE INDEX owner_intraday_quote_cache_identity_session_idx
    ON public.owner_intraday_quote_cache (
        owner_user_id, membership_id, generation_id, instrument_id,
        generation, session_date
    );

CREATE TABLE public.owner_intraday_quote_producers (
    owner_user_id uuid PRIMARY KEY
        REFERENCES public.users (id) ON DELETE RESTRICT,
    holder_id uuid NOT NULL,
    fencing_token bigint NOT NULL DEFAULT 0,
    lease_expires_at timestamptz NOT NULL,
    heartbeat_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT owner_intraday_quote_producers_fence_check CHECK (
        fencing_token >= 0
    ),
    CONSTRAINT owner_intraday_quote_producers_holder_check CHECK (
        holder_id <> '00000000-0000-0000-0000-000000000000'::uuid
    ),
    CONSTRAINT owner_intraday_quote_producers_lease_check CHECK (
        lease_expires_at > heartbeat_at
    )
);

ALTER TABLE public.owner_intraday_quote_demands OWNER TO migration_owner;
ALTER TABLE public.owner_intraday_quote_cache OWNER TO migration_owner;
ALTER TABLE public.owner_intraday_quote_producers OWNER TO migration_owner;

ALTER TABLE public.owner_intraday_quote_demands ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_intraday_quote_demands FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_intraday_quote_cache ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_intraday_quote_cache FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_intraday_quote_producers ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_intraday_quote_producers FORCE ROW LEVEL SECURITY;

REVOKE ALL ON TABLE public.owner_intraday_quote_demands
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON TABLE public.owner_intraday_quote_cache
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON TABLE public.owner_intraday_quote_producers
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

-- The app can mutate only actor-owned demand columns.  It has no cache or
-- producer write privilege and no delete privilege on demand tombstones.
GRANT SELECT ON TABLE public.owner_intraday_quote_demands TO app;
GRANT INSERT (
    owner_user_id, consumer_id, membership_id, generation_id, instrument_id,
    generation, state, renewal_sequence, lease_expires_at,
    idempotency_key_sha256, request_sha256
) ON public.owner_intraday_quote_demands TO app;
GRANT UPDATE (
    state, renewal_sequence, lease_expires_at, released_at,
    idempotency_key_sha256, request_sha256, updated_at
) ON public.owner_intraday_quote_demands TO app;
GRANT SELECT ON TABLE public.owner_intraday_quote_cache TO app;

-- Worker operations are explicit-column grants.  No worker privilege is
-- granted on EOD/Raw/Curated/snapshot tables by this migration.
GRANT SELECT ON TABLE public.owner_intraday_quote_demands,
    public.owner_intraday_quote_cache,
    public.owner_intraday_quote_producers TO worker;
GRANT INSERT (
    owner_user_id, consumer_id, membership_id, generation_id, instrument_id,
    generation, state, renewal_sequence, lease_expires_at, released_at,
    idempotency_key_sha256, request_sha256
) ON public.owner_intraday_quote_demands TO worker;
GRANT UPDATE (
    state, renewal_sequence, lease_expires_at, released_at,
    idempotency_key_sha256, request_sha256, updated_at
) ON public.owner_intraday_quote_demands TO worker;
GRANT DELETE ON TABLE public.owner_intraday_quote_demands TO worker;
GRANT INSERT (
    owner_user_id, membership_id, generation_id, instrument_id, generation,
    session_date, calendar_source, calendar_source_version,
    calendar_source_batch_id, calendar_content_sha256, window_contract_sha256,
    price, base_price, change_amount, change_percent, direction, halted,
    received_at, last_success_at, quote_version, last_attempt_at,
    last_failure_code, last_failure_at, producer_fence
) ON public.owner_intraday_quote_cache TO worker;
GRANT UPDATE (
    generation_id, instrument_id, generation, session_date, calendar_source,
    calendar_source_version, calendar_source_batch_id, calendar_content_sha256,
    window_contract_sha256, price, base_price, change_amount, change_percent,
    direction, halted, received_at, last_success_at, quote_version,
    last_attempt_at, last_failure_code, last_failure_at, producer_fence,
    updated_at
) ON public.owner_intraday_quote_cache TO worker;
GRANT DELETE ON TABLE public.owner_intraday_quote_cache TO worker;
GRANT INSERT (
    owner_user_id, holder_id, fencing_token, lease_expires_at, heartbeat_at,
    updated_at
) ON public.owner_intraday_quote_producers TO worker;
GRANT UPDATE (
    holder_id, fencing_token, lease_expires_at, heartbeat_at, updated_at
) ON public.owner_intraday_quote_producers TO worker;
GRANT DELETE ON TABLE public.owner_intraday_quote_producers TO worker;

CREATE POLICY owner_intraday_quote_demands_app_select
    ON public.owner_intraday_quote_demands FOR SELECT TO app
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );
CREATE POLICY owner_intraday_quote_demands_app_insert
    ON public.owner_intraday_quote_demands FOR INSERT TO app
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );
CREATE POLICY owner_intraday_quote_demands_app_update
    ON public.owner_intraday_quote_demands FOR UPDATE TO app
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    )
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );
CREATE POLICY owner_intraday_quote_demands_worker_all
    ON public.owner_intraday_quote_demands FOR ALL TO worker
    USING (true) WITH CHECK (true);
CREATE POLICY owner_intraday_quote_demands_owner_all
    ON public.owner_intraday_quote_demands FOR ALL TO migration_owner
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    )
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );

CREATE POLICY owner_intraday_quote_cache_app_select
    ON public.owner_intraday_quote_cache FOR SELECT TO app
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
        AND EXISTS (
            SELECT 1
              FROM public.owner_equity_memberships AS membership
              JOIN public.owner_equity_generation_admissions AS admission
                ON admission.generation_id = owner_intraday_quote_cache.generation_id
               AND admission.owner_user_id = owner_intraday_quote_cache.owner_user_id
               AND admission.membership_id = owner_intraday_quote_cache.membership_id
               AND admission.instrument_id = owner_intraday_quote_cache.instrument_id
               AND admission.generation = owner_intraday_quote_cache.generation
             WHERE membership.id = owner_intraday_quote_cache.membership_id
               AND membership.owner_user_id = owner_intraday_quote_cache.owner_user_id
               AND membership.instrument_id = owner_intraday_quote_cache.instrument_id
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
CREATE POLICY owner_intraday_quote_cache_worker_all
    ON public.owner_intraday_quote_cache FOR ALL TO worker
    USING (true) WITH CHECK (true);
CREATE POLICY owner_intraday_quote_cache_owner_all
    ON public.owner_intraday_quote_cache FOR ALL TO migration_owner
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    )
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );

CREATE POLICY owner_intraday_quote_producers_worker_all
    ON public.owner_intraday_quote_producers FOR ALL TO worker
    USING (true) WITH CHECK (true);
CREATE POLICY owner_intraday_quote_producers_owner_all
    ON public.owner_intraday_quote_producers FOR ALL TO migration_owner
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    )
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );
