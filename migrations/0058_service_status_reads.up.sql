-- 0058: bounded service-status queue indexes and a read-only stream aggregate.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

CREATE INDEX jobs_service_status_owner_type_status_available_idx
    ON public.jobs (owner_user_id, job_type, status, available_at);

CREATE INDEX jobs_service_status_owner_type_finished_idx
    ON public.jobs (owner_user_id, job_type, finished_at DESC)
    WHERE finished_at IS NOT NULL;

CREATE INDEX owner_beta_service_status_owner_finished_idx
    ON public.owner_beta_recommendation_runs (owner_user_id, finished_at DESC)
    WHERE status = 'SUCCEEDED' AND finished_at IS NOT NULL;

CREATE FUNCTION public.owner_service_status_stream(
    p_owner_id uuid,
    p_session_hash text,
    p_slot_id uuid,
    p_grant_id uuid,
    p_contract_sha256 text,
    p_window_sha256 text,
    p_open_at timestamptz,
    p_close_at timestamptz
) RETURNS TABLE (
    observed_at timestamptz,
    demand_count bigint,
    live_count bigint,
    halted_count bigint,
    producer_present boolean,
    connection_state text,
    status_code text,
    producer_status_at timestamptz,
    producer_heartbeat_at timestamptz,
    producer_lease_expires_at timestamptz,
    oldest_received_at timestamptz,
    oldest_provider_trade_at timestamptz,
    last_received_at timestamptz,
    lineage_valid boolean,
    gap_present boolean,
    valid_until timestamptz
)
LANGUAGE plpgsql
VOLATILE
SECURITY DEFINER
SET search_path = pg_catalog
AS $owner_service_status_stream$
DECLARE
    v_now timestamptz := pg_catalog.clock_timestamp();
    v_session_date date := (v_now AT TIME ZONE 'Asia/Seoul')::date;
    v_active_lease_count bigint;
    v_demand_count bigint;
    v_grant_revision uuid;
    v_producer_present boolean := false;
    v_connection_state text;
    v_status_code text;
    v_producer_status_at timestamptz;
    v_producer_heartbeat_at timestamptz;
    v_producer_lease_expires_at timestamptz;
    v_current_epoch uuid;
    v_producer_session_date date;
    v_session_proof_id uuid;
    v_session_proof_sha256 text;
    v_calendar_source text;
    v_calendar_source_version text;
    v_calendar_source_batch_id uuid;
    v_calendar_content_sha256 text;
    v_window_contract_sha256 text;
    v_producer_gap_since timestamptz;
    v_producer_session_has_gap boolean := false;
    v_producer_gap_generation bigint;
    v_producer_lineage_valid boolean := false;
    v_window_open boolean := false;
    v_producer_ready boolean := false;
    v_calendar_valid_until timestamptz;
BEGIN
    -- This capability is bound to the authenticated app actor and its current
    -- browser session. Invalid input returns no row and reveals no state.
    IF session_user <> 'app'
       OR p_owner_id IS NULL
       OR p_owner_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_session_hash IS NULL
       OR p_session_hash !~ '^[0-9a-f]{64}$'
       OR p_slot_id IS NULL
       OR p_slot_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_grant_id IS NULL
       OR p_grant_id = '00000000-0000-0000-0000-000000000000'::uuid
       OR p_contract_sha256 IS NULL
       OR p_contract_sha256 !~ '^[0-9a-f]{64}$'
       OR p_window_sha256 IS NULL
       OR p_window_sha256 !~ '^sha256:[0-9a-f]{64}$'
       OR (p_open_at IS NULL) <> (p_close_at IS NULL)
       OR (p_open_at IS NOT NULL AND (
           p_open_at >= p_close_at
           OR (p_open_at AT TIME ZONE 'Asia/Seoul')::date IS DISTINCT FROM v_session_date
           OR (p_close_at AT TIME ZONE 'Asia/Seoul')::date IS DISTINCT FROM v_session_date
       ))
       OR pg_catalog.current_setting('app.actor_user_id', true)
            IS DISTINCT FROM p_owner_id::text
       OR pg_catalog.current_setting('app.market_stream_session_hash', true)
            IS DISTINCT FROM p_session_hash
    THEN
        RETURN;
    END IF;

    IF NOT public.owner_market_stream_api_binding_valid(
        p_owner_id, p_session_hash, p_slot_id, p_grant_id, p_contract_sha256
    ) THEN
        RETURN;
    END IF;

    -- Re-pin the active grant revision for the current KST session date.
    SELECT stream_grant.grant_revision
      INTO v_grant_revision
      FROM public.owner_market_stream_grants AS stream_grant
     WHERE stream_grant.id = p_grant_id
       AND stream_grant.owner_user_id = p_owner_id
       AND stream_grant.credential_slot_id = p_slot_id
       AND stream_grant.network_contract_sha256 = p_contract_sha256
       AND stream_grant.state = 'ACTIVE'
       AND stream_grant.grant_revision <> '00000000-0000-0000-0000-000000000000'::uuid
       AND public.owner_market_stream_rights_valid(
           stream_grant.id, p_owner_id, v_session_date
       );
    IF v_grant_revision IS NULL THEN
        RETURN;
    END IF;

    -- Bound work before materializing identities. Overflow returns no row so
    -- a partial prefix can never be mistaken for complete demand.
    SELECT pg_catalog.count(*)
      INTO v_active_lease_count
      FROM (
          SELECT lease.id
            FROM public.owner_market_stream_leases AS lease
           WHERE lease.owner_user_id = p_owner_id
             AND lease.kind = 'BROWSER'
             AND lease.state = 'ACTIVE'
             AND lease.lease_expires_at > v_now
             AND public.owner_market_stream_session_valid(
                 lease.session_hash, p_owner_id
             )
           ORDER BY lease.id
           LIMIT 21
      ) AS bounded_leases;
    IF v_active_lease_count > 20 THEN
        RETURN;
    END IF;

    WITH valid_leases AS MATERIALIZED (
        SELECT lease.id, lease.owner_user_id
          FROM public.owner_market_stream_leases AS lease
         WHERE lease.owner_user_id = p_owner_id
           AND lease.kind = 'BROWSER'
           AND lease.state = 'ACTIVE'
           AND lease.lease_expires_at > v_now
           AND public.owner_market_stream_session_valid(
               lease.session_hash, p_owner_id
           )
    ),
    bounded_identities AS (
        SELECT DISTINCT item.owner_user_id, item.membership_id,
               item.generation_id, item.instrument_id, item.generation
          FROM valid_leases AS lease
          JOIN public.owner_market_stream_lease_items AS item
            ON item.lease_id = lease.id
           AND item.owner_user_id = lease.owner_user_id
         ORDER BY item.owner_user_id, item.membership_id, item.generation_id,
                  item.instrument_id, item.generation
         LIMIT 31
    )
    SELECT pg_catalog.count(*)
      INTO v_demand_count
      FROM bounded_identities;
    IF v_demand_count > 30 THEN
        RETURN;
    END IF;

    SELECT producer.connection_state,
           producer.status_code,
           producer.status_at,
           producer.heartbeat_at,
           producer.lease_expires_at,
           producer.current_epoch,
           producer.session_date,
           producer.session_proof_id,
           producer.session_proof_sha256,
           producer.calendar_source,
           producer.calendar_source_version,
           producer.calendar_source_batch_id,
           producer.calendar_content_sha256,
           producer.window_contract_sha256,
           producer.gap_since,
           producer.session_has_gap,
           producer.gap_generation
      INTO v_connection_state,
           v_status_code,
           v_producer_status_at,
           v_producer_heartbeat_at,
           v_producer_lease_expires_at,
           v_current_epoch,
           v_producer_session_date,
           v_session_proof_id,
           v_session_proof_sha256,
           v_calendar_source,
           v_calendar_source_version,
           v_calendar_source_batch_id,
           v_calendar_content_sha256,
           v_window_contract_sha256,
           v_producer_gap_since,
           v_producer_session_has_gap,
           v_producer_gap_generation
      FROM public.owner_market_stream_producers AS producer
     WHERE producer.credential_slot_id = p_slot_id
       AND producer.grant_id = p_grant_id
       AND producer.grant_revision = v_grant_revision
       AND producer.owner_user_id = p_owner_id;
    v_producer_present := FOUND;

    -- NULL bounds are authenticated, non-positive observation mode. The caller
    -- independently validates CLOSED; NULL itself is not a closure claim.
    v_window_open := COALESCE(
        p_open_at IS NOT NULL AND p_close_at IS NOT NULL
        AND v_now >= p_open_at AND v_now < p_close_at, false
    );

    IF v_producer_present THEN
        SELECT COALESCE(
            v_window_open
            AND v_producer_session_date = v_session_date
            AND v_session_proof_id IS NOT NULL
            AND v_session_proof_id <> '00000000-0000-0000-0000-000000000000'::uuid
            AND v_session_proof_sha256 ~ '^sha256:[0-9a-f]{64}$'
            AND v_calendar_source = 'kis'
            AND v_calendar_source_version = 'kis-chk-holiday-v1:schema-1'
            AND v_calendar_source_batch_id IS NOT NULL
            AND v_calendar_source_batch_id <> '00000000-0000-0000-0000-000000000000'::uuid
            AND v_calendar_content_sha256 ~ '^[0-9a-f]{64}$'
            AND v_window_contract_sha256 = p_window_sha256
            AND v_window_contract_sha256 ~ '^sha256:[0-9a-f]{64}$'
            AND v_session_proof_sha256 = 'sha256:' || pg_catalog.encode(
                pg_catalog.sha256(pg_catalog.convert_to(
                    'owner-market-stream-day-v1' || E'\n'
                    || pg_catalog.to_char(v_producer_session_date, 'YYYY-MM-DD') || E'\n'
                    || v_calendar_source_batch_id::text || E'\n'
                    || pg_catalog.lower(v_calendar_content_sha256) || E'\n'
                    || v_window_contract_sha256 || E'\n'
                    || pg_catalog.to_char(
                        p_open_at AT TIME ZONE 'UTC',
                        'YYYY-MM-DD"T"HH24:MI:SS"Z"'
                    ) || E'\n'
                    || pg_catalog.to_char(
                        p_close_at AT TIME ZONE 'UTC',
                        'YYYY-MM-DD"T"HH24:MI:SS"Z"'
                    ) || E'\n',
                    'UTF8'
                )),
                'hex'
            )
            AND verified_calendar.valid_until IS NOT NULL,
            false
        ), verified_calendar.valid_until
        INTO v_producer_lineage_valid, v_calendar_valid_until
        FROM (
                SELECT pg_catalog.min(LEAST(
                    calendar.retrieved_at + INTERVAL '36 hours',
                    version.retrieved_at + INTERVAL '36 hours',
                    batch.retrieved_at + INTERVAL '36 hours'
                )) AS valid_until
                  FROM public.trading_calendars AS calendar
                  JOIN public.trading_calendar_versions AS version
                    ON version.exchange = calendar.exchange
                   AND version.session_date = calendar.session_date
                   AND version.session_type = calendar.session_type
                   AND version.source_version = calendar.source_version
                   AND version.source = v_calendar_source
                   AND version.timezone = 'Asia/Seoul'
                   AND version.source_batch_id = calendar.source_batch_id
                   AND version.content_sha256 = calendar.content_sha256
                  JOIN public.data_batches AS batch
                    ON batch.source_batch_id = calendar.source_batch_id
                   AND batch.provider = 'KRX'
                   AND batch.market = 'KR'
                   AND batch.kind = 'CALENDAR'
                   AND batch.batch_date = calendar.session_date
                   AND batch.content_sha256 = calendar.content_sha256
                   AND batch.source_file_name = 'calendar.json'
                   AND batch.fetch_mode = 'credentialed'
                 WHERE calendar.exchange = 'KRX'
                   AND calendar.session_date = v_producer_session_date
                   AND calendar.session_date = v_session_date
                   AND calendar.session_type = 'TRADING'
                   AND calendar.timezone = 'Asia/Seoul'
                   AND calendar.source = v_calendar_source
                   AND calendar.source = 'kis'
                   AND calendar.source_version = v_calendar_source_version
                   AND calendar.source_version = 'kis-chk-holiday-v1:schema-1'
                   AND calendar.source_batch_id = v_calendar_source_batch_id
                   AND calendar.content_sha256 = v_calendar_content_sha256
                   AND calendar.retrieved_at <= v_now
                   AND calendar.retrieved_at >= v_now - INTERVAL '36 hours'
                   AND version.retrieved_at <= v_now
                   AND version.retrieved_at >= v_now - INTERVAL '36 hours'
                   AND batch.retrieved_at <= v_now
                   AND batch.retrieved_at >= v_now - INTERVAL '36 hours'
        ) AS verified_calendar;
    END IF;

    -- Reuse the exact producer/window eligibility for coverage and pending
    -- readiness assurance. Already-failed prerequisites are diagnostic clocks.
    v_producer_ready := COALESCE(
        v_producer_present AND v_producer_lineage_valid
        AND v_connection_state = 'CONNECTED' AND v_status_code IS NULL
        AND v_current_epoch IS NOT NULL
        AND v_current_epoch <> '00000000-0000-0000-0000-000000000000'::uuid
        AND v_producer_session_date = v_session_date
        AND v_producer_lease_expires_at > v_now + INTERVAL '5 seconds'
        AND v_producer_heartbeat_at <= v_now
        AND v_producer_heartbeat_at >= v_now - INTERVAL '10 seconds'
        AND v_producer_gap_since IS NULL AND NOT v_producer_session_has_gap
        AND v_window_contract_sha256 = p_window_sha256
        AND v_window_open, false
    );

    RETURN QUERY
    WITH bounded_leases AS MATERIALIZED (
        SELECT lease.id, lease.owner_user_id, lease.lease_expires_at
          FROM public.owner_market_stream_leases AS lease
         WHERE lease.owner_user_id = p_owner_id
           AND lease.kind = 'BROWSER'
           AND lease.state = 'ACTIVE'
           AND lease.lease_expires_at > v_now
           AND public.owner_market_stream_session_valid(
               lease.session_hash, p_owner_id
           )
         ORDER BY lease.id
         LIMIT 21
    ),
    lease_summary AS (
        SELECT pg_catalog.count(*) AS active_lease_count,
               pg_catalog.min(lease_expires_at) AS earliest_lease_expiry
          FROM bounded_leases
    ),
    demand_identities AS MATERIALIZED (
        SELECT DISTINCT item.owner_user_id, item.membership_id,
               item.generation_id, item.instrument_id, item.generation
          FROM bounded_leases AS lease
          JOIN public.owner_market_stream_lease_items AS item
            ON item.lease_id = lease.id
           AND item.owner_user_id = lease.owner_user_id
         ORDER BY item.owner_user_id, item.membership_id, item.generation_id,
                  item.instrument_id, item.generation
         LIMIT 31
    ),
    demand_summary AS (
        SELECT pg_catalog.count(*) AS demand_count
          FROM demand_identities
    ),
    demand_evidence AS (
        SELECT demand.owner_user_id,
               demand.membership_id,
               demand.generation_id,
               demand.instrument_id,
               demand.generation,
               EXISTS (
                   SELECT 1
                     FROM public.owner_equity_memberships AS membership
                     JOIN public.owner_equity_generation_admissions AS admission
                       ON admission.owner_user_id = membership.owner_user_id
                      AND admission.membership_id = membership.id
                      AND admission.instrument_id = membership.instrument_id
                      AND admission.generation_id = demand.generation_id
                      AND admission.generation = demand.generation
                    WHERE membership.id = demand.membership_id
                      AND membership.owner_user_id = demand.owner_user_id
                      AND membership.instrument_id = demand.instrument_id
                      AND membership.state = 'READY'
                      AND NOT EXISTS (
                          SELECT 1
                            FROM public.owner_equity_generation_admissions AS newer
                           WHERE newer.owner_user_id = admission.owner_user_id
                             AND newer.membership_id = admission.membership_id
                             AND newer.instrument_id = admission.instrument_id
                             AND newer.generation > admission.generation
                      )
               ) AS current_admission,
               cache.halted,
               cache.received_at,
               cache.provider_trade_at,
               cache.gap_since AS cache_gap_since,
               cache.gap_generation AS cache_gap_generation,
               (
                   v_producer_present
                   AND v_producer_lineage_valid
                   AND EXISTS (
                       SELECT 1
                         FROM public.owner_equity_memberships AS membership
                         JOIN public.owner_equity_generation_admissions AS admission
                           ON admission.owner_user_id = membership.owner_user_id
                          AND admission.membership_id = membership.id
                          AND admission.instrument_id = membership.instrument_id
                          AND admission.generation_id = demand.generation_id
                          AND admission.generation = demand.generation
                        WHERE membership.id = demand.membership_id
                          AND membership.owner_user_id = demand.owner_user_id
                          AND membership.instrument_id = demand.instrument_id
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
                   AND EXISTS (
                       SELECT 1
                         FROM public.owner_market_stream_subscriptions AS subscription
                         JOIN public.owner_market_stream_cache AS quote
                           ON quote.owner_user_id = demand.owner_user_id
                          AND quote.membership_id = demand.membership_id
                          AND quote.generation_id = demand.generation_id
                          AND quote.instrument_id = demand.instrument_id
                          AND quote.generation = demand.generation
                          AND quote.grant_id = p_grant_id
                          AND quote.credential_slot_id = p_slot_id
                          AND quote.grant_revision = v_grant_revision
                          AND quote.grant_revision <> '00000000-0000-0000-0000-000000000000'::uuid
                          AND quote.source = 'KIS_MARKET_WS'
                          AND quote.wire_version = 'kis-h0stcnt0-20260914-v1'
                          AND quote.venue = 'KRX'
                          AND quote.currency = 'KRW'
                          AND quote.session_date = v_session_date
                          AND quote.calendar_source = 'kis'
                          AND quote.calendar_source_version = 'kis-chk-holiday-v1:schema-1'
                          AND quote.session_proof_id IS NOT NULL
                          AND quote.session_proof_id <> '00000000-0000-0000-0000-000000000000'::uuid
                          AND quote.session_proof_id = v_session_proof_id
                          AND quote.session_proof_sha256 = v_session_proof_sha256
                          AND quote.calendar_source_batch_id = v_calendar_source_batch_id
                          AND quote.calendar_content_sha256 = v_calendar_content_sha256
                          AND quote.window_contract_sha256 = v_window_contract_sha256
                          AND quote.epoch IS NOT NULL
                          AND quote.epoch <> '00000000-0000-0000-0000-000000000000'::uuid
                          AND quote.epoch = v_current_epoch
                          AND quote.fencing_token > 0
                          AND quote.subscription_revision IS NOT NULL
                          AND quote.subscription_revision <> '00000000-0000-0000-0000-000000000000'::uuid
                          AND quote.receive_ordinal > 0
                          AND quote.quote_version > 0
                          AND quote.base_price IS NULL
                          AND quote.price IS NOT NULL
                          AND quote.price > 0
                          AND quote.change_amount IS NOT NULL
                          AND quote.change_percent IS NOT NULL
                          AND quote.direction IS NOT NULL
                          AND quote.trade_volume IS NOT NULL
                          AND quote.trade_volume >= 0
                          AND quote.cumulative_volume IS NOT NULL
                          AND quote.cumulative_volume >= 0
                          AND quote.halted IS NOT NULL
                          AND quote.business_date = v_session_date
                          AND quote.trade_time IS NOT NULL
                          AND quote.provider_trade_at IS NOT NULL
                          AND quote.provider_trade_at AT TIME ZONE 'Asia/Seoul'
                                = quote.business_date + quote.trade_time
                          AND quote.received_at IS NOT NULL
                          AND quote.committed_at IS NOT NULL
                          AND quote.received_at <= v_now
                          AND quote.provider_trade_at <= v_now
                          AND quote.committed_at <= v_now
                          AND quote.received_at >= v_now - INTERVAL '30 seconds'
                          AND quote.provider_trade_at >= v_now - INTERVAL '30 seconds'
                          AND quote.received_at >= p_open_at
                          AND quote.received_at < p_close_at
                          AND quote.provider_trade_at >= p_open_at
                          AND quote.provider_trade_at < p_close_at
                          AND quote.committed_at >= p_open_at
                          AND quote.committed_at < p_close_at
                          AND quote.committed_at >= quote.received_at
                          AND quote.committed_at <= quote.received_at + INTERVAL '3 seconds'
                          AND quote.provider_trade_at <= quote.received_at + INTERVAL '2 seconds'
                          AND quote.gap_since IS NULL
                          AND quote.gap_generation = v_producer_gap_generation
                          AND quote.connection_state = 'CONNECTED'
                          AND quote.market_state = 'OPEN'
                          AND quote.freshness = 'RECENT'
                          AND quote.availability = 'LIVE'
                          AND quote.status_code IS NULL
                          AND subscription.credential_slot_id = p_slot_id
                          AND subscription.symbol = pg_catalog.left(demand.instrument_id, 6)
                          AND subscription.provider = 'kis'
                          AND subscription.environment = 'live'
                          AND subscription.venue = 'KRX'
                          AND subscription.tr_id = 'H0STCNT0'
                          AND subscription.grant_revision = v_grant_revision
                          AND subscription.state = 'ACKED'
                          AND subscription.epoch = v_current_epoch
                          AND subscription.subscription_revision = quote.subscription_revision
                   )
                   AND v_producer_ready
               ) AS is_live
          FROM demand_identities AS demand
          LEFT JOIN public.owner_market_stream_cache AS cache
            ON cache.owner_user_id = demand.owner_user_id
           AND cache.membership_id = demand.membership_id
           AND cache.generation_id = demand.generation_id
           AND cache.instrument_id = demand.instrument_id
           AND cache.generation = demand.generation
    ),
    quote_summary AS (
        SELECT pg_catalog.min(received_at) AS oldest_received_at,
               pg_catalog.min(provider_trade_at) AS oldest_provider_trade_at,
               pg_catalog.max(received_at) AS last_received_at,
               -- Inclusive SQL coverage; halted is a subset. Only the API
               -- adapter converts these to exclusive evaluation counts.
               pg_catalog.count(*) FILTER (WHERE is_live) AS live_count,
               pg_catalog.count(*) FILTER (WHERE is_live AND halted) AS halted_count,
               pg_catalog.min(received_at + INTERVAL '30 seconds')
                   FILTER (WHERE is_live) AS live_receipt_valid_until,
               pg_catalog.min(provider_trade_at + INTERVAL '30 seconds')
                   FILTER (WHERE is_live) AS live_provider_valid_until,
               pg_catalog.bool_or(
                   cache_gap_since IS NOT NULL
                   OR (cache_gap_generation IS NOT NULL
                       AND cache_gap_generation <> v_producer_gap_generation)
               ) AS cache_gap_present,
               pg_catalog.bool_and(current_admission) AS all_admissions_current
          FROM demand_evidence
    )
    SELECT v_now,
           demand_summary.demand_count,
           quote_summary.live_count,
           quote_summary.halted_count,
           v_producer_present,
           CASE WHEN v_connection_state IN (
               'DISCONNECTED', 'CONNECTING', 'CONNECTED', 'BACKOFF', 'STOPPED'
           ) THEN v_connection_state ELSE NULL END,
           CASE WHEN v_status_code IN (
               'FEATURE_DISABLED', 'NO_ACTIVE_DEMAND', 'CALENDAR_UNAVAILABLE',
               'SESSION_WINDOW_UNAVAILABLE', 'SESSION_CLOSED', 'AWAITING_FIRST_TRADE',
               'QUOTE_STALE', 'CONNECTION_LOST', 'RECONNECT_GAP',
               'SUBSCRIPTION_PENDING', 'SUBSCRIPTION_REJECTED',
               'SUBSCRIPTION_AMBIGUOUS', 'APPROVAL_UNAVAILABLE', 'BUDGET_EXHAUSTED',
               'PRODUCER_UNAVAILABLE', 'PIPELINE_LAG', 'WIRE_SCHEMA_MISMATCH',
               'PROVIDER_RESPONSE_INVALID', 'QUOTE_VALUE_INVALID',
               'MARKET_CLASS_UNSUPPORTED', 'LOCAL_INGRESS_LIMIT', 'RESYNC_REQUIRED',
               'ACCESS_REVOKED'
           ) THEN v_status_code ELSE NULL END,
           v_producer_status_at,
           v_producer_heartbeat_at,
           v_producer_lease_expires_at,
           quote_summary.oldest_received_at,
           quote_summary.oldest_provider_trade_at,
           quote_summary.last_received_at,
           CASE
               WHEN NOT v_window_open THEN false
               WHEN NOT v_producer_present AND demand_summary.demand_count = 0 THEN true
               WHEN v_producer_present
                    AND v_producer_lineage_valid
                    AND COALESCE(quote_summary.all_admissions_current, true)
               THEN true
               ELSE false
           END,
           COALESCE(
               v_producer_gap_since IS NOT NULL
               OR v_producer_session_has_gap
               OR quote_summary.cache_gap_present,
               false
           ),
           LEAST(
               v_now + INTERVAL '30 seconds',
               ((v_session_date + 1)::timestamp AT TIME ZONE 'Asia/Seoul'),
               CASE WHEN p_open_at > v_now THEN p_open_at END,
               CASE WHEN p_close_at > v_now THEN p_close_at END,
               lease_summary.earliest_lease_expiry,
               v_calendar_valid_until,
               CASE WHEN readiness.used THEN v_producer_heartbeat_at + INTERVAL '10 seconds' END,
               CASE WHEN readiness.used THEN v_producer_lease_expires_at - INTERVAL '5 seconds' END,
               quote_summary.live_receipt_valid_until,
               quote_summary.live_provider_valid_until
           )
      FROM quote_summary
      CROSS JOIN lease_summary
      CROSS JOIN demand_summary
      CROSS JOIN LATERAL (
          SELECT v_producer_ready AND demand_summary.demand_count > 0
                 AND (quote_summary.live_count > 0 OR (
                     COALESCE(quote_summary.all_admissions_current, true)
                     AND NOT COALESCE(quote_summary.cache_gap_present, false)
                 )) AS used
      ) AS readiness
     WHERE lease_summary.active_lease_count <= 20
       AND demand_summary.demand_count <= 30;
END
$owner_service_status_stream$;

ALTER FUNCTION public.owner_service_status_stream(
    uuid, text, uuid, uuid, text, text, timestamptz, timestamptz
) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_service_status_stream(
    uuid, text, uuid, uuid, text, text, timestamptz, timestamptz
) FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.owner_service_status_stream(
    uuid, text, uuid, uuid, text, text, timestamptz, timestamptz
) TO app;
