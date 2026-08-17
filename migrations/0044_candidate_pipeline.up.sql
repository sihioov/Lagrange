-- 0044: Candidate scheduler and atomic publication capabilities. The worker
-- can call narrow functions but receives no direct DML on system output.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

INSERT INTO public.users (id, issuer, subject, email, display_name)
VALUES (
    '00000000-0000-4000-8000-000000000042'::uuid,
    'urn:lagrange:internal',
    'candidate-scheduler-v1',
    'candidate-scheduler@system.invalid',
    'Candidate Scheduler (non-login)'
);

CREATE TABLE public.candidate_scheduler_control (
    control_key        text PRIMARY KEY CHECK (control_key = 'scheduler'),
    active             boolean NOT NULL,
    service_user_id    uuid NOT NULL UNIQUE REFERENCES public.users(id) ON DELETE RESTRICT,
    wake_at_kst        time NOT NULL DEFAULT TIME '16:30',
    required_fetch_mode text NOT NULL DEFAULT 'credentialed'
        CHECK (required_fetch_mode IN ('credentialed','synthetic')),
    updated_at         timestamptz NOT NULL DEFAULT clock_timestamp()
);

INSERT INTO public.candidate_scheduler_control (
    control_key,
    active,
    service_user_id
)
VALUES (
    'scheduler',
    false,
    '00000000-0000-4000-8000-000000000042'::uuid
);

ALTER TABLE public.candidate_scheduler_control OWNER TO migration_owner;
ALTER TABLE public.candidate_scheduler_control ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.candidate_scheduler_control FORCE ROW LEVEL SECURITY;
REVOKE ALL ON TABLE public.candidate_scheduler_control
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT SELECT ON TABLE public.candidate_scheduler_control TO worker, admin;
CREATE POLICY candidate_scheduler_control_worker_select
    ON public.candidate_scheduler_control FOR SELECT TO worker USING (true);
CREATE POLICY candidate_scheduler_control_admin_select
    ON public.candidate_scheduler_control FOR SELECT TO admin USING (true);
CREATE POLICY candidate_scheduler_control_owner_all
    ON public.candidate_scheduler_control FOR ALL TO migration_owner
    USING (true) WITH CHECK (true);

-- Queue lifecycle fields remain mutable to the worker, while the immutable
-- service owner/type/key/payload identity is migration-owned.
CREATE FUNCTION public.jobs_reject_candidate_scheduled_mutation()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $guard$
DECLARE
    v_is_candidate boolean := false;
BEGIN
    IF CURRENT_USER = 'migration_owner' THEN
        IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
        RETURN NEW;
    END IF;

    IF TG_OP = 'INSERT' THEN
        IF NEW.job_type = 'candidate_compute'
            OR NEW.idempotency_key LIKE 'candidate:scheduled:%'
        THEN
            RAISE EXCEPTION 'scheduled candidate jobs are migration-owned'
                USING ERRCODE = '42501';
        END IF;
        RETURN NEW;
    END IF;

    SELECT EXISTS (
        SELECT 1 FROM public.stock_analysis_runs AS run WHERE run.job_id = OLD.id
    ) INTO v_is_candidate;

    IF TG_OP = 'DELETE' THEN
        IF v_is_candidate
            OR OLD.job_type = 'candidate_compute'
            OR OLD.idempotency_key LIKE 'candidate:scheduled:%'
        THEN
            RAISE EXCEPTION 'scheduled candidate job lineage is immutable'
                USING ERRCODE = '42501';
        END IF;
        RETURN OLD;
    END IF;

    IF NEW.job_type = 'candidate_compute'
        AND OLD.job_type <> 'candidate_compute'
    THEN
        RAISE EXCEPTION 'scheduled candidate jobs are migration-owned'
            USING ERRCODE = '42501';
    END IF;
    IF NEW.idempotency_key LIKE 'candidate:scheduled:%'
        AND OLD.idempotency_key NOT LIKE 'candidate:scheduled:%'
    THEN
        RAISE EXCEPTION 'scheduled candidate job namespace is migration-owned'
            USING ERRCODE = '42501';
    END IF;

    IF v_is_candidate
        OR OLD.job_type = 'candidate_compute'
        OR OLD.idempotency_key LIKE 'candidate:scheduled:%'
    THEN
        IF NEW.id IS DISTINCT FROM OLD.id
            OR NEW.owner_user_id IS DISTINCT FROM OLD.owner_user_id
            OR NEW.job_type IS DISTINCT FROM OLD.job_type
            OR NEW.idempotency_key IS DISTINCT FROM OLD.idempotency_key
            OR NEW.payload_json IS DISTINCT FROM OLD.payload_json
        THEN
            RAISE EXCEPTION 'scheduled candidate job lineage is immutable'
                USING ERRCODE = '42501';
        END IF;
    END IF;
    RETURN NEW;
END
$guard$;

ALTER FUNCTION public.jobs_reject_candidate_scheduled_mutation()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.jobs_reject_candidate_scheduled_mutation()
    FROM PUBLIC;

CREATE TRIGGER jobs_protect_candidate_scheduled_lineage
    BEFORE INSERT OR UPDATE OR DELETE ON public.jobs
    FOR EACH ROW EXECUTE FUNCTION public.jobs_reject_candidate_scheduled_mutation();

GRANT EXECUTE ON FUNCTION public.candidate_source_entitlement_is_valid(
    uuid, text, text, date, date
) TO worker;

CREATE FUNCTION public.schedule_candidate_run(
    p_as_of_date date,
    p_cutoff_at timestamptz,
    p_scoring_config_version text,
    p_scoring_config_sha256 text,
    p_universe_snapshot_id uuid,
    p_price_dataset_version_id uuid,
    p_price_curated_version integer,
    p_price_manifest_sha256 text,
    p_status_dataset_version_id uuid,
    p_status_manifest_sha256 text,
    p_flow_dataset_version_id uuid,
    p_flow_manifest_sha256 text,
    p_fundamental_dataset_version_id uuid,
    p_fundamental_manifest_sha256 text,
    p_sector_version_id uuid
)
RETURNS TABLE (run_id uuid, job_id uuid, computation_seq integer)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $schedule$
DECLARE
    v_service_user_id uuid;
    v_required_fetch_mode text;
    v_expected_key text;
    v_core_identity text;
    v_input_identity_sha256 text;
    v_run_id uuid;
    v_job_id uuid;
    v_seq integer;
    v_payload jsonb;
    v_dataset_id text;
    v_price_entitlement_id uuid;
    v_price_license_ref text;
    v_universe_entitlement_id uuid;
    v_universe_license_ref text;
    v_status_entitlement_id uuid;
    v_status_license_ref text;
    v_flow_entitlement_id uuid;
    v_flow_license_ref text;
    v_fundamental_entitlement_id uuid;
    v_fundamental_license_ref text;
    v_sector_entitlement_id uuid;
    v_sector_license_ref text;
    v_canonical_cutoff timestamptz;
    v_required_first_session date;
    v_required_session_count integer;
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock_shared(1815099521, 44);
    SELECT control.service_user_id, control.required_fetch_mode
    INTO v_service_user_id, v_required_fetch_mode
    FROM public.candidate_scheduler_control AS control
    WHERE control.control_key = 'scheduler' AND control.active;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate scheduler is unavailable'
            USING ERRCODE = '55000';
    END IF;
    -- `jobs` is FORCE-RLS and migration_owner is tenant-filtered. Bind the
    -- reserved service principal before any replay lookup or queue insert.
    PERFORM pg_catalog.set_config(
        'app.actor_user_id', v_service_user_id::text, true
    );

    IF p_as_of_date IS NULL OR p_cutoff_at IS NULL
        OR p_scoring_config_version IS NULL OR p_scoring_config_sha256 IS NULL
        OR p_universe_snapshot_id IS NULL
        OR p_price_dataset_version_id IS NULL OR p_price_curated_version IS NULL
        OR p_price_manifest_sha256 IS NULL
        OR p_status_dataset_version_id IS NULL OR p_status_manifest_sha256 IS NULL
        OR p_flow_dataset_version_id IS NULL OR p_flow_manifest_sha256 IS NULL
        OR p_fundamental_dataset_version_id IS NULL OR p_fundamental_manifest_sha256 IS NULL
        OR p_sector_version_id IS NULL
    THEN
        RAISE EXCEPTION 'candidate scheduled identity must be complete'
            USING ERRCODE = '22023';
    END IF;
    IF p_price_curated_version <= 0 THEN
        RAISE EXCEPTION 'candidate curated price version must be positive'
            USING ERRCODE = '22023';
    END IF;
    IF p_cutoff_at < (p_as_of_date::timestamp AT TIME ZONE 'Asia/Seoul')
        OR p_cutoff_at > ((p_as_of_date + 7)::timestamp AT TIME ZONE 'Asia/Seoul')
    THEN
        RAISE EXCEPTION 'candidate cutoff is outside the bounded as-of window'
            USING ERRCODE = '22023';
    END IF;
    IF p_scoring_config_sha256 !~ '^[0-9a-f]{64}$'
        OR p_price_manifest_sha256 !~ '^[0-9a-f]{64}$'
        OR p_status_manifest_sha256 !~ '^[0-9a-f]{64}$'
        OR p_flow_manifest_sha256 !~ '^[0-9a-f]{64}$'
        OR p_fundamental_manifest_sha256 !~ '^[0-9a-f]{64}$'
    THEN
        RAISE EXCEPTION 'candidate scheduled hash is invalid'
            USING ERRCODE = '22023';
    END IF;

    PERFORM 1
    FROM public.trading_calendars AS calendar
    WHERE calendar.exchange = 'KRX'
      AND calendar.session_date = p_as_of_date
      AND calendar.session_type = 'TRADING'
      AND calendar.timezone = 'Asia/Seoul'
      AND calendar.source_batch_id IS NOT NULL
      AND calendar.content_sha256 IS NOT NULL
      AND calendar.retrieved_at IS NOT NULL;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate run requires a confirmed KRX trading session'
            USING ERRCODE = '55000';
    END IF;

    SELECT min(required.session_date), count(*)
      INTO v_required_first_session, v_required_session_count
      FROM (
          SELECT calendar.session_date
            FROM public.trading_calendars AS calendar
           WHERE calendar.exchange = 'KRX'
             AND calendar.session_type = 'TRADING'
             AND calendar.timezone = 'Asia/Seoul'
             AND calendar.session_date <= p_as_of_date
             AND calendar.source_batch_id IS NOT NULL
             AND calendar.content_sha256 IS NOT NULL
             AND calendar.retrieved_at IS NOT NULL
           ORDER BY calendar.session_date DESC
           LIMIT 60
      ) AS required;
    IF v_required_session_count <> 60 THEN
        RAISE EXCEPTION 'candidate run requires 60 confirmed KRX sessions'
            USING ERRCODE = '55000';
    END IF;

    -- Validate exact source lineage before creating any queue state.
    PERFORM 1
    FROM public.candidate_scoring_configs AS config
    WHERE config.version = p_scoring_config_version
      AND config.content_sha256 = p_scoring_config_sha256;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate scoring configuration mismatch'
            USING ERRCODE = '23514';
    END IF;

    SELECT dataset.dataset_id, price.entitlement_id, price.license_ref
    INTO v_dataset_id, v_price_entitlement_id, v_price_license_ref
    FROM public.candidate_price_publications AS price
    JOIN public.dataset_versions AS dataset ON dataset.id = price.dataset_version_id
    WHERE price.dataset_version_id = p_price_dataset_version_id
      AND dataset.dataset_id = 'krx_eod_bars'
      AND dataset.manifest_sha256 = p_price_manifest_sha256
      AND dataset.status IN ('READY', 'WARNING')
      AND price.manifest_sha256 = p_price_manifest_sha256
      AND price.curated_generation = p_price_curated_version
      AND price.market = 'kr'
      AND price.first_session <= p_as_of_date
      AND price.last_session >= p_as_of_date
      AND price.available_at <= p_cutoff_at;
    IF NOT FOUND THEN RAISE EXCEPTION 'candidate price dataset is unavailable' USING ERRCODE = '55000'; END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_price_entitlement_id, v_price_license_ref, v_dataset_id,
        v_required_first_session, p_as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate price entitlement is inactive' USING ERRCODE = '42501';
    END IF;

    SELECT dataset.dataset_id, status.entitlement_id, status.license_ref
    INTO v_dataset_id, v_status_entitlement_id, v_status_license_ref
    FROM public.candidate_market_status_observations AS status
    JOIN public.dataset_versions AS dataset ON dataset.id = status.dataset_version_id
    WHERE status.dataset_version_id = p_status_dataset_version_id
      AND dataset.dataset_id = 'krx_market_status'
      AND dataset.manifest_sha256 = p_status_manifest_sha256
      AND dataset.status IN ('READY', 'WARNING')
      AND status.trade_date = p_as_of_date
      AND status.available_at <= p_cutoff_at
    ORDER BY status.available_at DESC, status.id
    LIMIT 1;
    IF NOT FOUND THEN RAISE EXCEPTION 'candidate market-status dataset is unavailable' USING ERRCODE = '55000'; END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_status_entitlement_id, v_status_license_ref, v_dataset_id, p_as_of_date, p_as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate market-status entitlement is inactive' USING ERRCODE = '42501';
    END IF;

    SELECT dataset.dataset_id, member.entitlement_id, member.license_ref
    INTO v_dataset_id, v_flow_entitlement_id, v_flow_license_ref
    FROM public.candidate_investor_flows AS flow
    JOIN public.candidate_investor_flow_snapshot_rows AS member
      ON member.flow_observation_id=flow.id
    JOIN public.dataset_versions AS dataset ON dataset.id = member.dataset_version_id
    WHERE member.dataset_version_id = p_flow_dataset_version_id
      AND dataset.dataset_id = 'krx_investor_flows'
      AND dataset.manifest_sha256 = p_flow_manifest_sha256
      AND dataset.status IN ('READY', 'WARNING')
      AND flow.trade_date = p_as_of_date
      AND flow.available_at <= p_cutoff_at
    ORDER BY flow.available_at DESC, flow.id
    LIMIT 1;
    IF NOT FOUND THEN RAISE EXCEPTION 'candidate flow dataset is unavailable' USING ERRCODE = '55000'; END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_flow_entitlement_id, v_flow_license_ref, v_dataset_id,
        v_required_first_session, p_as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate flow entitlement is inactive' USING ERRCODE = '42501';
    END IF;

    SELECT dataset.dataset_id, fact.entitlement_id, fact.license_ref
    INTO v_dataset_id, v_fundamental_entitlement_id, v_fundamental_license_ref
    FROM public.candidate_fundamental_observations AS fact
    JOIN public.dataset_versions AS dataset ON dataset.id = fact.dataset_version_id
    WHERE fact.dataset_version_id = p_fundamental_dataset_version_id
      AND dataset.dataset_id = 'krx_fundamentals'
      AND dataset.manifest_sha256 = p_fundamental_manifest_sha256
      AND dataset.status IN ('READY', 'WARNING')
      AND fact.fiscal_period_end <= p_as_of_date
      AND fact.available_at <= p_cutoff_at
    ORDER BY fact.available_at DESC, fact.id
    LIMIT 1;
    IF NOT FOUND THEN RAISE EXCEPTION 'candidate fundamental dataset is unavailable' USING ERRCODE = '55000'; END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_fundamental_entitlement_id, v_fundamental_license_ref, v_dataset_id, p_as_of_date, p_as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate fundamental entitlement is inactive' USING ERRCODE = '42501';
    END IF;

    SELECT dataset.dataset_id, universe.entitlement_id, universe.license_ref
    INTO v_dataset_id, v_universe_entitlement_id, v_universe_license_ref
    FROM public.candidate_universe_snapshots AS universe
    JOIN public.dataset_versions AS dataset ON dataset.id = universe.dataset_version_id
    WHERE universe.id = p_universe_snapshot_id
      AND universe.as_of_date <= p_as_of_date
      AND universe.available_at <= p_cutoff_at
      AND universe.member_count = (
          SELECT count(*) FROM public.candidate_universe_members AS member
           WHERE member.universe_snapshot_id = universe.id
             AND member.effective_from <= p_as_of_date
             AND (member.effective_until IS NULL OR member.effective_until >= p_as_of_date))
      AND dataset.dataset_id = 'krx_kospi200_membership'
      AND dataset.manifest_sha256 = universe.manifest_sha256
      AND dataset.status IN ('READY', 'WARNING');
    IF NOT FOUND THEN RAISE EXCEPTION 'candidate universe is unavailable at cutoff' USING ERRCODE = '55000'; END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_universe_entitlement_id, v_universe_license_ref, v_dataset_id, p_as_of_date, p_as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate universe entitlement is inactive' USING ERRCODE = '42501';
    END IF;
    SELECT dataset.dataset_id, sector.entitlement_id, sector.license_ref
    INTO v_dataset_id, v_sector_entitlement_id, v_sector_license_ref
    FROM public.candidate_sector_versions AS sector
    JOIN public.dataset_versions AS dataset ON dataset.id = sector.dataset_version_id
    WHERE sector.id = p_sector_version_id
      AND sector.effective_from <= p_as_of_date
      AND sector.available_at <= p_cutoff_at
      AND dataset.dataset_id = 'krx_sector_classification'
      AND dataset.manifest_sha256 = sector.manifest_sha256
      AND dataset.status IN ('READY', 'WARNING');
    IF NOT FOUND THEN RAISE EXCEPTION 'candidate sector dataset is unavailable at cutoff' USING ERRCODE = '55000'; END IF;
    IF NOT public.candidate_source_entitlement_is_valid(
        v_sector_entitlement_id, v_sector_license_ref, v_dataset_id, p_as_of_date, p_as_of_date
    ) THEN
        RAISE EXCEPTION 'candidate sector entitlement is inactive' USING ERRCODE = '42501';
    END IF;
    IF EXISTS (
        SELECT 1 FROM (VALUES
            ('bars'::text,p_price_dataset_version_id),
            ('market_status'::text,p_status_dataset_version_id),
            ('investor_flow'::text,p_flow_dataset_version_id),
            ('fundamentals'::text,p_fundamental_dataset_version_id),
            ('index_membership'::text,(
                SELECT universe.dataset_version_id
                  FROM public.candidate_universe_snapshots AS universe
                 WHERE universe.id=p_universe_snapshot_id)),
            ('sector_classification'::text,(
                SELECT sector.dataset_version_id
                  FROM public.candidate_sector_versions AS sector
                 WHERE sector.id=p_sector_version_id))
        ) AS required(response_kind,dataset_version_id)
        WHERE NOT EXISTS (
            SELECT 1 FROM public.candidate_raw_batch_datasets AS binding
            JOIN public.candidate_raw_batch_publications AS batch
              ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
           WHERE binding.dataset_version_id=required.dataset_version_id
             AND binding.response_kind=required.response_kind
             AND batch.state='PUBLISHED'
             AND batch.fetch_mode=v_required_fetch_mode)
    ) THEN
        RAISE EXCEPTION 'candidate source pins are not sealed under the required fetch mode'
            USING ERRCODE='55000';
    END IF;
    SELECT greatest(
        (SELECT calendar.retrieved_at FROM public.trading_calendars AS calendar
          WHERE calendar.exchange='KRX' AND calendar.session_date=p_as_of_date
            AND calendar.session_type='TRADING' AND calendar.timezone='Asia/Seoul'
          ORDER BY calendar.retrieved_at DESC LIMIT 1),
        (SELECT config.created_at FROM public.candidate_scoring_configs AS config
          WHERE config.version=p_scoring_config_version
            AND config.content_sha256=p_scoring_config_sha256),
        (SELECT price.available_at FROM public.candidate_price_publications AS price
          WHERE price.dataset_version_id=p_price_dataset_version_id),
        (SELECT max(status.available_at) FROM public.candidate_market_status_observations AS status
          WHERE status.dataset_version_id=p_status_dataset_version_id
            AND status.trade_date=p_as_of_date),
        (SELECT max(flow.available_at) FROM public.candidate_investor_flows AS flow
          JOIN public.candidate_investor_flow_snapshot_rows AS member
            ON member.flow_observation_id=flow.id
          WHERE member.dataset_version_id=p_flow_dataset_version_id
            AND flow.trade_date=p_as_of_date),
        (SELECT max(fact.available_at) FROM public.candidate_fundamental_observations AS fact
          WHERE fact.dataset_version_id=p_fundamental_dataset_version_id
            AND fact.fiscal_period_end <= p_as_of_date),
        (SELECT universe.available_at FROM public.candidate_universe_snapshots AS universe
          WHERE universe.id=p_universe_snapshot_id),
        (SELECT sector.available_at FROM public.candidate_sector_versions AS sector
          WHERE sector.id=p_sector_version_id)
    ) INTO v_canonical_cutoff;
    IF v_canonical_cutoff IS NULL OR p_cutoff_at <> v_canonical_cutoff THEN
        RAISE EXCEPTION 'candidate cutoff does not match exact pinned source availability'
            USING ERRCODE = '23514';
    END IF;
    IF (
        WITH required_sessions AS MATERIALIZED (
            SELECT calendar.session_date FROM public.trading_calendars AS calendar
             WHERE calendar.exchange='KRX' AND calendar.session_type='TRADING'
               AND calendar.timezone='Asia/Seoul' AND calendar.session_date <= p_as_of_date
               AND calendar.source_batch_id IS NOT NULL
               AND calendar.content_sha256 IS NOT NULL AND calendar.retrieved_at IS NOT NULL
             ORDER BY calendar.session_date DESC LIMIT 60
        )
        SELECT count(*) FROM public.candidate_universe_members AS member
         WHERE member.universe_snapshot_id=p_universe_snapshot_id
           AND member.effective_from <= p_as_of_date
           AND (member.effective_until IS NULL OR member.effective_until >= p_as_of_date)
           AND (SELECT count(*) FROM required_sessions)=60
           AND NOT EXISTS (
               SELECT 1 FROM required_sessions AS required WHERE NOT EXISTS (
                   SELECT 1 FROM public.candidate_price_instrument_sessions AS price_session
                    WHERE price_session.dataset_version_id=p_price_dataset_version_id
                      AND price_session.instrument_id=member.instrument_id
                      AND price_session.session_date=required.session_date))
           AND NOT EXISTS (
               SELECT 1 FROM required_sessions AS required
               CROSS JOIN (VALUES ('FOREIGN'),('INSTITUTION')) AS class(investor_class)
                WHERE NOT EXISTS (
                   SELECT 1 FROM public.candidate_investor_flows AS history
                   JOIN public.candidate_investor_flow_snapshot_rows AS flow_member
                     ON flow_member.flow_observation_id=history.id
                    WHERE flow_member.dataset_version_id=p_flow_dataset_version_id
                      AND history.instrument_id=member.instrument_id
                      AND history.trade_date=required.session_date
                      AND history.investor_class=class.investor_class
                      AND history.available_at <= p_cutoff_at))
           AND EXISTS (SELECT 1 FROM public.candidate_market_status_observations AS status
                        WHERE status.dataset_version_id=p_status_dataset_version_id
                          AND status.instrument_id=member.instrument_id
                          AND status.trade_date=p_as_of_date AND status.available_at <= p_cutoff_at)
           AND EXISTS (SELECT 1 FROM public.candidate_fundamental_observations AS fact
                        WHERE fact.dataset_version_id=p_fundamental_dataset_version_id
                          AND fact.instrument_id=member.instrument_id
                          AND fact.fiscal_period_end <= p_as_of_date
                          AND fact.available_at <= p_cutoff_at)
           AND EXISTS (SELECT 1 FROM public.candidate_sector_entries AS entry
                        WHERE entry.sector_version_id=p_sector_version_id
                          AND entry.instrument_id=member.instrument_id
                          AND entry.effective_from <= p_as_of_date
                          AND entry.available_at <= p_cutoff_at
                          AND (entry.effective_until IS NULL OR entry.effective_until >= p_as_of_date))
    ) < 5 THEN
        RAISE EXCEPTION 'fewer than five candidate members have complete 60-session inputs'
            USING ERRCODE = '55000';
    END IF;

    v_core_identity := pg_catalog.concat_ws(
        '|',
        pg_catalog.to_char(p_as_of_date, 'YYYY-MM-DD'),
        pg_catalog.to_char(
            p_cutoff_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'
        ),
        p_scoring_config_version,
        p_scoring_config_sha256,
        p_universe_snapshot_id::text,
        v_universe_entitlement_id::text,
        p_price_dataset_version_id::text,
        v_price_entitlement_id::text,
        p_price_curated_version::text,
        p_price_manifest_sha256,
        p_status_dataset_version_id::text,
        v_status_entitlement_id::text,
        p_status_manifest_sha256,
        p_flow_dataset_version_id::text,
        v_flow_entitlement_id::text,
        p_flow_manifest_sha256,
        p_fundamental_dataset_version_id::text,
        v_fundamental_entitlement_id::text,
        p_fundamental_manifest_sha256,
        p_sector_version_id::text,
        v_sector_entitlement_id::text
    );
    v_input_identity_sha256 := pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(v_core_identity, 'UTF8')),
        'hex'
    );
    v_expected_key := 'candidate:scheduled:' || pg_catalog.md5(v_core_identity);

    -- One lock serializes correction sequence allocation for a trading date.
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended('candidate|' || p_as_of_date::text, 0)
    );

    SELECT run.id, run.job_id, run.computation_seq
    INTO v_run_id, v_job_id, v_seq
    FROM public.stock_analysis_runs AS run
    WHERE run.input_identity_sha256 = v_input_identity_sha256
    FOR UPDATE OF run;
    IF FOUND THEN
        PERFORM 1
        FROM public.jobs AS job
        WHERE job.id = v_job_id
          AND job.owner_user_id = v_service_user_id
          AND job.job_type = 'candidate_compute'
          AND job.idempotency_key = v_expected_key;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'candidate scheduled replay conflicts with job lineage'
                USING ERRCODE = '23514';
        END IF;
        RETURN QUERY SELECT v_run_id, v_job_id, v_seq;
        RETURN;
    END IF;

    SELECT COALESCE(max(run.computation_seq), 0) + 1
    INTO v_seq
    FROM public.stock_analysis_runs AS run
    WHERE run.as_of_date = p_as_of_date;
    v_run_id := pg_catalog.gen_random_uuid();
    v_job_id := pg_catalog.gen_random_uuid();
    v_payload := pg_catalog.jsonb_build_object(
        'run_id', v_run_id,
        'as_of_date', pg_catalog.to_char(p_as_of_date, 'YYYY-MM-DD'),
        'cutoff_at', p_cutoff_at,
        'scoring_config_version', p_scoring_config_version,
        'scoring_config_sha256', p_scoring_config_sha256,
        'universe_snapshot_id', p_universe_snapshot_id,
        'universe_entitlement_id', v_universe_entitlement_id,
        'price_dataset_version_id', p_price_dataset_version_id,
        'price_entitlement_id', v_price_entitlement_id,
        'price_curated_version', p_price_curated_version,
        'price_manifest_sha256', p_price_manifest_sha256,
        'status_dataset_version_id', p_status_dataset_version_id,
        'status_entitlement_id', v_status_entitlement_id,
        'status_manifest_sha256', p_status_manifest_sha256,
        'flow_dataset_version_id', p_flow_dataset_version_id,
        'flow_entitlement_id', v_flow_entitlement_id,
        'flow_manifest_sha256', p_flow_manifest_sha256,
        'fundamental_dataset_version_id', p_fundamental_dataset_version_id,
        'fundamental_entitlement_id', v_fundamental_entitlement_id,
        'fundamental_manifest_sha256', p_fundamental_manifest_sha256,
        'sector_version_id', p_sector_version_id,
        'sector_entitlement_id', v_sector_entitlement_id,
        'input_identity_sha256', v_input_identity_sha256
    );

    INSERT INTO public.jobs (
        id, owner_user_id, job_type, status, idempotency_key,
        payload_json, max_attempts
    ) VALUES (
        v_job_id, v_service_user_id, 'candidate_compute', 'QUEUED',
        v_expected_key, v_payload, 3
    );

    INSERT INTO public.stock_analysis_runs (
        id, as_of_date, cutoff_at, computation_seq, status, job_id,
        scoring_config_version, scoring_config_sha256, universe_snapshot_id,
        universe_entitlement_id,
        price_dataset_version_id, price_entitlement_id,
        price_curated_version, price_manifest_sha256,
        status_dataset_version_id, status_entitlement_id, status_manifest_sha256,
        flow_dataset_version_id, flow_entitlement_id, flow_manifest_sha256,
        fundamental_dataset_version_id, fundamental_entitlement_id,
        fundamental_manifest_sha256,
        sector_version_id, sector_entitlement_id, input_identity_sha256
    ) VALUES (
        v_run_id, p_as_of_date, p_cutoff_at, v_seq, 'PENDING', v_job_id,
        p_scoring_config_version, p_scoring_config_sha256, p_universe_snapshot_id,
        v_universe_entitlement_id,
        p_price_dataset_version_id, v_price_entitlement_id,
        p_price_curated_version, p_price_manifest_sha256,
        p_status_dataset_version_id, v_status_entitlement_id, p_status_manifest_sha256,
        p_flow_dataset_version_id, v_flow_entitlement_id, p_flow_manifest_sha256,
        p_fundamental_dataset_version_id, v_fundamental_entitlement_id,
        p_fundamental_manifest_sha256,
        p_sector_version_id, v_sector_entitlement_id, v_input_identity_sha256
    );

    RETURN QUERY SELECT v_run_id, v_job_id, v_seq;
END
$schedule$;

ALTER FUNCTION public.schedule_candidate_run(
    date, timestamptz, text, text, uuid, uuid, integer, text, uuid, text, uuid, text,
    uuid, text, uuid
) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.schedule_candidate_run(
    date, timestamptz, text, text, uuid, uuid, integer, text, uuid, text, uuid, text,
    uuid, text, uuid
) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.schedule_candidate_run(
    date, timestamptz, text, text, uuid, uuid, integer, text, uuid, text, uuid, text,
    uuid, text, uuid
) TO worker;

CREATE FUNCTION public.publish_candidate_analysis(
    p_run_id uuid,
    p_job_id uuid,
    p_attempt_no integer,
    p_worker_id text,
    p_snapshots jsonb,
    p_summary jsonb
)
RETURNS uuid
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $publish$
DECLARE
    v_service_user_id uuid;
    v_run public.stock_analysis_runs%ROWTYPE;
    v_feed_id uuid;
    v_snapshot_count integer;
    v_member_count integer;
    v_eligible_count integer;
    v_dataset_id text;
    v_license_ref text;
    v_required_first_session date;
    v_required_session_count integer;
BEGIN
    IF p_run_id IS NULL OR p_job_id IS NULL OR p_attempt_no <= 0
        OR length(btrim(COALESCE(p_worker_id, ''))) = 0
        OR jsonb_typeof(p_snapshots) <> 'array'
        OR jsonb_array_length(p_snapshots) = 0
        OR jsonb_array_length(p_snapshots) > 10000
        OR jsonb_typeof(p_summary) <> 'object'
    THEN
        RAISE EXCEPTION 'candidate publication payload is invalid'
            USING ERRCODE = '22023';
    END IF;

    SELECT control.service_user_id INTO v_service_user_id
    FROM public.candidate_scheduler_control AS control
    WHERE control.control_key = 'scheduler';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate scheduler is unavailable'
            USING ERRCODE = '55000';
    END IF;
    -- The shared jobs table is FORCE-RLS and migration_owner is scoped by
    -- app.actor_user_id. Bind the reserved service principal before reading
    -- or settling the exact queue claim.
    PERFORM pg_catalog.set_config(
        'app.actor_user_id', v_service_user_id::text, true
    );

    SELECT run.* INTO v_run
    FROM public.stock_analysis_runs AS run
    WHERE run.id = p_run_id AND run.job_id = p_job_id
    FOR UPDATE OF run;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate publication run is missing'
            USING ERRCODE = '23514';
    END IF;

    IF v_run.status = 'SUCCEEDED' THEN
        SELECT feed.id INTO v_feed_id
        FROM public.candidate_feed_snapshots AS feed
        WHERE feed.run_id = p_run_id;
        IF v_feed_id IS NULL
            OR p_summary IS DISTINCT FROM v_run.summary_json
            OR jsonb_array_length(p_snapshots) <> (
                SELECT count(*)
                FROM public.stock_analysis_snapshots AS snapshot
                WHERE snapshot.run_id = p_run_id
            )
            OR jsonb_array_length(p_snapshots) <> (
                SELECT count(DISTINCT supplied.value ->> 'instrument_id')
                FROM jsonb_array_elements(p_snapshots) AS supplied(value)
            )
            OR EXISTS (
                WITH supplied AS (
                    SELECT value ->> 'instrument_id' AS instrument_id,
                           pg_catalog.encode(
                               pg_catalog.sha256(pg_catalog.jsonb_send(value)),
                               'hex'
                           ) AS content_sha256
                    FROM jsonb_array_elements(p_snapshots)
                ), stored AS (
                    SELECT snapshot.instrument_id, snapshot.content_sha256
                    FROM public.stock_analysis_snapshots AS snapshot
                    WHERE snapshot.run_id = p_run_id
                )
                SELECT 1
                FROM stored
                FULL JOIN supplied USING (instrument_id)
                WHERE stored.instrument_id IS NULL
                   OR supplied.instrument_id IS NULL
                   OR supplied.content_sha256 IS DISTINCT FROM stored.content_sha256
            )
        THEN
            RAISE EXCEPTION 'candidate publication replay payload mismatch'
                USING ERRCODE = '23514';
        END IF;
        RETURN v_feed_id;
    END IF;
    IF v_run.status NOT IN ('PENDING', 'RUNNING') THEN
        RAISE EXCEPTION 'candidate publication run is not publishable'
            USING ERRCODE = '55000';
    END IF;

    SELECT min(required.session_date), count(*)
      INTO v_required_first_session, v_required_session_count
      FROM (
          SELECT calendar.session_date
            FROM public.trading_calendars AS calendar
           WHERE calendar.exchange = 'KRX'
             AND calendar.session_type = 'TRADING'
             AND calendar.timezone = 'Asia/Seoul'
             AND calendar.session_date <= v_run.as_of_date
             AND calendar.source_batch_id IS NOT NULL
             AND calendar.content_sha256 IS NOT NULL
             AND calendar.retrieved_at IS NOT NULL
           ORDER BY calendar.session_date DESC
           LIMIT 60
      ) AS required;
    IF v_required_session_count <> 60 THEN
        RAISE EXCEPTION 'candidate publication requires 60 confirmed KRX sessions'
            USING ERRCODE = '55000';
    END IF;

    PERFORM 1
    FROM public.jobs AS job
    JOIN public.job_attempts AS attempt
      ON attempt.job_id = job.id AND attempt.attempt_no = p_attempt_no
    WHERE job.id = p_job_id
      AND job.owner_user_id = v_service_user_id
      AND job.job_type = 'candidate_compute'
      AND job.status = 'RUNNING'
      AND job.locked_by = p_worker_id
      AND job.locked_at IS NOT NULL
      AND job.attempt_count = p_attempt_no
      AND attempt.outcome = 'RUNNING'
      AND attempt.claimed_by = p_worker_id
    FOR UPDATE OF job, attempt;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate publication does not hold the queue claim'
            USING ERRCODE = '55000';
    END IF;

    -- Re-attest every exact source and candidate-use entitlement in the same
    -- transaction that publishes. A block or contract revocation after
    -- computation must race in favor of failing closed, never publication.
    SELECT dataset.dataset_id, price.license_ref INTO v_dataset_id, v_license_ref
    FROM public.candidate_price_publications AS price
    JOIN public.dataset_versions AS dataset ON dataset.id = price.dataset_version_id
    WHERE price.dataset_version_id = v_run.price_dataset_version_id
      AND price.entitlement_id = v_run.price_entitlement_id
      AND price.curated_generation = v_run.price_curated_version
      AND price.first_session <= v_run.as_of_date
      AND price.last_session >= v_run.as_of_date
      AND price.available_at <= v_run.cutoff_at
      AND dataset.dataset_id = 'krx_eod_bars'
      AND dataset.manifest_sha256 = v_run.price_manifest_sha256
      AND dataset.status IN ('READY', 'WARNING')
    FOR SHARE OF price, dataset;
    IF NOT FOUND THEN RAISE EXCEPTION 'candidate price dataset became unavailable' USING ERRCODE = '55000'; END IF;
    IF NOT public.candidate_source_entitlement_is_valid(v_run.price_entitlement_id, v_license_ref, v_dataset_id, v_required_first_session, v_run.as_of_date)
    THEN RAISE EXCEPTION 'candidate price entitlement became inactive before publication' USING ERRCODE = '42501'; END IF;

    SELECT dataset.dataset_id, status.license_ref INTO v_dataset_id, v_license_ref
    FROM public.candidate_market_status_observations AS status
    JOIN public.dataset_versions AS dataset ON dataset.id = status.dataset_version_id
    WHERE status.dataset_version_id = v_run.status_dataset_version_id
      AND status.entitlement_id = v_run.status_entitlement_id
      AND status.trade_date = v_run.as_of_date
      AND status.available_at <= v_run.cutoff_at
      AND dataset.dataset_id = 'krx_market_status'
      AND dataset.manifest_sha256 = v_run.status_manifest_sha256
      AND dataset.status IN ('READY', 'WARNING')
    ORDER BY status.available_at DESC, status.id LIMIT 1
    FOR SHARE OF status, dataset;
    IF NOT FOUND THEN RAISE EXCEPTION 'candidate market-status dataset became unavailable' USING ERRCODE = '55000'; END IF;
    IF NOT public.candidate_source_entitlement_is_valid(v_run.status_entitlement_id, v_license_ref, v_dataset_id, v_run.as_of_date, v_run.as_of_date)
    THEN RAISE EXCEPTION 'candidate market-status entitlement became inactive before publication' USING ERRCODE = '42501'; END IF;

    SELECT dataset.dataset_id, member.license_ref INTO v_dataset_id, v_license_ref
    FROM public.candidate_investor_flows AS flow
    JOIN public.candidate_investor_flow_snapshot_rows AS member
      ON member.flow_observation_id=flow.id
    JOIN public.dataset_versions AS dataset ON dataset.id = member.dataset_version_id
    WHERE member.dataset_version_id = v_run.flow_dataset_version_id
      AND member.entitlement_id = v_run.flow_entitlement_id
      AND flow.trade_date = v_run.as_of_date
      AND flow.available_at <= v_run.cutoff_at
      AND dataset.dataset_id = 'krx_investor_flows'
      AND dataset.manifest_sha256 = v_run.flow_manifest_sha256
      AND dataset.status IN ('READY', 'WARNING')
    ORDER BY flow.available_at DESC, flow.id LIMIT 1
    FOR SHARE OF flow, dataset;
    IF NOT FOUND THEN RAISE EXCEPTION 'candidate flow dataset became unavailable' USING ERRCODE = '55000'; END IF;
    IF NOT public.candidate_source_entitlement_is_valid(v_run.flow_entitlement_id, v_license_ref, v_dataset_id, v_required_first_session, v_run.as_of_date)
    THEN RAISE EXCEPTION 'candidate flow entitlement became inactive before publication' USING ERRCODE = '42501'; END IF;

    SELECT dataset.dataset_id, fact.license_ref INTO v_dataset_id, v_license_ref
    FROM public.candidate_fundamental_observations AS fact
    JOIN public.dataset_versions AS dataset ON dataset.id = fact.dataset_version_id
    WHERE fact.dataset_version_id = v_run.fundamental_dataset_version_id
      AND fact.entitlement_id = v_run.fundamental_entitlement_id
      AND fact.fiscal_period_end <= v_run.as_of_date
      AND fact.available_at <= v_run.cutoff_at
      AND dataset.dataset_id = 'krx_fundamentals'
      AND dataset.manifest_sha256 = v_run.fundamental_manifest_sha256
      AND dataset.status IN ('READY', 'WARNING')
    ORDER BY fact.available_at DESC, fact.id LIMIT 1
    FOR SHARE OF fact, dataset;
    IF NOT FOUND THEN RAISE EXCEPTION 'candidate fundamental dataset became unavailable' USING ERRCODE = '55000'; END IF;
    IF NOT public.candidate_source_entitlement_is_valid(v_run.fundamental_entitlement_id, v_license_ref, v_dataset_id, v_run.as_of_date, v_run.as_of_date)
    THEN RAISE EXCEPTION 'candidate fundamental entitlement became inactive before publication' USING ERRCODE = '42501'; END IF;

    SELECT dataset.dataset_id, universe.license_ref INTO v_dataset_id, v_license_ref
    FROM public.candidate_universe_snapshots AS universe
    JOIN public.dataset_versions AS dataset ON dataset.id = universe.dataset_version_id
    WHERE universe.id = v_run.universe_snapshot_id
      AND universe.entitlement_id = v_run.universe_entitlement_id
      AND universe.as_of_date <= v_run.as_of_date
      AND universe.available_at <= v_run.cutoff_at
      AND universe.member_count = (
          SELECT count(*) FROM public.candidate_universe_members AS member
           WHERE member.universe_snapshot_id = universe.id
             AND member.effective_from <= v_run.as_of_date
             AND (member.effective_until IS NULL OR member.effective_until >= v_run.as_of_date))
      AND dataset.dataset_id = 'krx_kospi200_membership'
      AND dataset.manifest_sha256 = universe.manifest_sha256
      AND dataset.status IN ('READY', 'WARNING')
    FOR SHARE OF universe, dataset;
    IF NOT FOUND THEN RAISE EXCEPTION 'candidate universe became unavailable' USING ERRCODE = '55000'; END IF;
    IF NOT public.candidate_source_entitlement_is_valid(v_run.universe_entitlement_id, v_license_ref, v_dataset_id, v_run.as_of_date, v_run.as_of_date)
    THEN RAISE EXCEPTION 'candidate universe entitlement became inactive before publication' USING ERRCODE = '42501'; END IF;

    SELECT dataset.dataset_id, sector.license_ref INTO v_dataset_id, v_license_ref
    FROM public.candidate_sector_versions AS sector
    JOIN public.dataset_versions AS dataset ON dataset.id = sector.dataset_version_id
    WHERE sector.id = v_run.sector_version_id
      AND sector.entitlement_id = v_run.sector_entitlement_id
      AND sector.effective_from <= v_run.as_of_date
      AND sector.available_at <= v_run.cutoff_at
      AND dataset.dataset_id = 'krx_sector_classification'
      AND dataset.manifest_sha256 = sector.manifest_sha256
      AND dataset.status IN ('READY', 'WARNING')
    FOR SHARE OF sector, dataset;
    IF NOT FOUND THEN RAISE EXCEPTION 'candidate sector dataset became unavailable' USING ERRCODE = '55000'; END IF;
    IF NOT public.candidate_source_entitlement_is_valid(v_run.sector_entitlement_id, v_license_ref, v_dataset_id, v_run.as_of_date, v_run.as_of_date)
    THEN RAISE EXCEPTION 'candidate sector entitlement became inactive before publication' USING ERRCODE = '42501'; END IF;

    SELECT universe.member_count INTO v_member_count
    FROM public.candidate_universe_snapshots AS universe
    WHERE universe.id = v_run.universe_snapshot_id;
    IF v_member_count <> jsonb_array_length(p_snapshots) THEN
        RAISE EXCEPTION 'candidate publication must cover the exact universe'
            USING ERRCODE = '23514';
    END IF;

    WITH supplied AS (
        SELECT *
        FROM jsonb_to_recordset(p_snapshots) AS input(
            instrument_id text,
            sector_code text,
            fundamental_profile text,
            eligible boolean,
            exclusion_codes jsonb,
            flow_score numeric,
            fundamental_score numeric,
            technical_score numeric,
            total_score numeric,
            flow_coverage numeric,
            fundamental_coverage numeric,
            technical_coverage numeric,
            evidence_strength text,
            normalization_scope text,
            factors_json jsonb,
            scenarios_json jsonb,
            provenance_json jsonb
        )
    )
    SELECT count(*)::integer INTO v_snapshot_count FROM supplied;
    IF v_snapshot_count <> v_member_count OR EXISTS (
        WITH supplied AS (
            SELECT value ->> 'instrument_id' AS instrument_id
            FROM jsonb_array_elements(p_snapshots)
        ), members AS (
            SELECT member.instrument_id
            FROM public.candidate_universe_members AS member
            WHERE member.universe_snapshot_id = v_run.universe_snapshot_id
        )
        SELECT 1
        FROM supplied
        FULL JOIN members USING (instrument_id)
        WHERE supplied.instrument_id IS NULL OR members.instrument_id IS NULL
    ) THEN
        RAISE EXCEPTION 'candidate publication membership mismatch'
            USING ERRCODE = '23514';
    END IF;
    IF EXISTS (
        SELECT 1
        FROM jsonb_array_elements(p_snapshots) AS supplied(value)
        WHERE supplied.value ? 'content_sha256'
           OR supplied.value - ARRAY[
                'instrument_id', 'sector_code', 'fundamental_profile', 'eligible',
                'exclusion_codes', 'flow_score', 'fundamental_score',
                'technical_score', 'total_score', 'flow_coverage',
                'fundamental_coverage', 'technical_coverage', 'evidence_strength',
                'normalization_scope', 'factors_json', 'scenarios_json',
                'provenance_json'
              ]::text[] <> '{}'::jsonb
           OR supplied.value -> 'provenance_json' ->> 'input_identity_sha256'
                  IS DISTINCT FROM v_run.input_identity_sha256
           OR supplied.value -> 'provenance_json' ->> 'as_of_date'
                  IS DISTINCT FROM pg_catalog.to_char(v_run.as_of_date, 'YYYY-MM-DD')
           OR (supplied.value -> 'scenarios_json') ?| ARRAY[
                'probability', 'probabilities', 'target_price', 'expected_return'
              ]::text[]
           OR EXISTS (
                SELECT 1
                FROM pg_catalog.jsonb_path_query(
                    supplied.value -> 'scenarios_json', '$.**'
                ) AS descendant(value)
                WHERE pg_catalog.jsonb_typeof(descendant.value) = 'object'
                  AND descendant.value ?| ARRAY[
                        'probability', 'probabilities',
                        'target_price', 'expected_return'
                      ]::text[]
              )
    ) THEN
        RAISE EXCEPTION 'candidate publication schema or provenance mismatch'
            USING ERRCODE = '23514';
    END IF;

    WITH supplied AS (
        SELECT *
        FROM jsonb_to_recordset(p_snapshots) AS input(
            instrument_id text,
            sector_code text,
            fundamental_profile text,
            eligible boolean,
            exclusion_codes jsonb,
            flow_score numeric,
            fundamental_score numeric,
            technical_score numeric,
            total_score numeric,
            flow_coverage numeric,
            fundamental_coverage numeric,
            technical_coverage numeric,
            evidence_strength text,
            normalization_scope text,
            factors_json jsonb,
            scenarios_json jsonb,
            provenance_json jsonb
        )
    ), hashes AS (
        SELECT value ->> 'instrument_id' AS instrument_id,
               pg_catalog.encode(
                   pg_catalog.sha256(pg_catalog.jsonb_send(value)),
                   'hex'
               ) AS content_sha256
        FROM jsonb_array_elements(p_snapshots)
    ), eligible_ranks AS (
        SELECT supplied.instrument_id,
               (row_number() OVER (
                   ORDER BY supplied.total_score DESC NULLS LAST, supplied.instrument_id
               ))::integer AS rank
        FROM supplied
        WHERE supplied.eligible
    )
    INSERT INTO public.stock_analysis_snapshots (
        run_id, instrument_id, sector_code, fundamental_profile, eligible,
        exclusion_codes, flow_score, fundamental_score, technical_score,
        total_score, flow_coverage, fundamental_coverage, technical_coverage,
        evidence_strength, rank, normalization_scope, factors_json,
        scenarios_json, provenance_json, content_sha256
    )
    SELECT
        p_run_id, supplied.instrument_id, supplied.sector_code,
        supplied.fundamental_profile, supplied.eligible,
        supplied.exclusion_codes, supplied.flow_score,
        supplied.fundamental_score, supplied.technical_score,
        supplied.total_score, supplied.flow_coverage,
        supplied.fundamental_coverage, supplied.technical_coverage,
        supplied.evidence_strength, eligible_ranks.rank,
        supplied.normalization_scope, supplied.factors_json,
        supplied.scenarios_json, supplied.provenance_json,
        hashes.content_sha256
    FROM supplied
    JOIN hashes USING (instrument_id)
    LEFT JOIN eligible_ranks USING (instrument_id);

    SELECT count(*)::integer INTO v_eligible_count
    FROM public.stock_analysis_snapshots AS snapshot
    WHERE snapshot.run_id = p_run_id
      AND snapshot.eligible
      AND snapshot.evidence_strength IN ('STRONG', 'MODERATE')
      AND snapshot.flow_coverage >= 0.6
      AND snapshot.fundamental_coverage >= 0.6
      AND snapshot.technical_coverage >= 0.6;
    IF v_eligible_count < 5 THEN
        RAISE EXCEPTION 'candidate publication has fewer than five supported candidates'
            USING ERRCODE = '23514';
    END IF;

    v_feed_id := pg_catalog.gen_random_uuid();
    UPDATE public.candidate_feed_snapshots AS previous
    SET status = 'SUPERSEDED', superseded_by = v_feed_id
    WHERE previous.as_of_date = v_run.as_of_date
      AND previous.status = 'PUBLISHED';

    INSERT INTO public.candidate_feed_snapshots (
        id, run_id, as_of_date, computation_seq, status, published_at
    ) VALUES (
        v_feed_id, p_run_id, v_run.as_of_date, v_run.computation_seq,
        'PUBLISHED', clock_timestamp()
    );

    INSERT INTO public.candidate_feed_items (
        feed_id, run_id, stock_analysis_snapshot_id, instrument_id, rank
    )
    SELECT v_feed_id, p_run_id, snapshot.id, snapshot.instrument_id,
           (row_number() OVER (
               ORDER BY snapshot.total_score DESC, snapshot.instrument_id
           ))::integer
    FROM public.stock_analysis_snapshots AS snapshot
    WHERE snapshot.run_id = p_run_id
      AND snapshot.eligible
      AND snapshot.evidence_strength IN ('STRONG', 'MODERATE')
      AND snapshot.flow_coverage >= 0.6
      AND snapshot.fundamental_coverage >= 0.6
      AND snapshot.technical_coverage >= 0.6
    ORDER BY snapshot.total_score DESC, snapshot.instrument_id
    LIMIT 5;

    UPDATE public.stock_analysis_runs
    SET status = 'SUCCEEDED', summary_json = p_summary,
        published_at = clock_timestamp(), error_code = NULL, error_message = NULL
    WHERE id = p_run_id;

    RETURN v_feed_id;
END
$publish$;

ALTER FUNCTION public.publish_candidate_analysis(uuid, uuid, integer, text, jsonb, jsonb)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.publish_candidate_analysis(uuid, uuid, integer, text, jsonb, jsonb)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.publish_candidate_analysis(uuid, uuid, integer, text, jsonb, jsonb)
    TO worker;

CREATE FUNCTION public.fail_candidate_analysis_run(
    p_run_id uuid,
    p_job_id uuid,
    p_status text,
    p_error_code text,
    p_error_message text,
    p_summary jsonb
)
RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $fail$
DECLARE
    v_changed_count bigint;
    v_service_user_id uuid;
BEGIN
    IF p_status NOT IN ('FAILED', 'BLOCKED')
        OR p_error_code !~ '^[A-Z][A-Z0-9_]{0,63}$'
        OR jsonb_typeof(p_summary) <> 'object'
    THEN
        RAISE EXCEPTION 'candidate failure payload is invalid'
            USING ERRCODE = '22023';
    END IF;
    SELECT control.service_user_id INTO v_service_user_id
    FROM public.candidate_scheduler_control AS control
    WHERE control.control_key = 'scheduler';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'candidate scheduler is unavailable'
            USING ERRCODE = '55000';
    END IF;
    PERFORM pg_catalog.set_config(
        'app.actor_user_id', v_service_user_id::text, true
    );
    UPDATE public.stock_analysis_runs AS run
    SET status = p_status,
        error_code = p_error_code,
        error_message = left(COALESCE(p_error_message, ''), 2048),
        summary_json = p_summary,
        published_at = NULL
    FROM public.jobs AS job,
         public.candidate_scheduler_control AS control
    WHERE run.id = p_run_id
      AND run.job_id = p_job_id
      AND run.status IN ('PENDING', 'RUNNING')
      AND job.id = p_job_id
      AND job.owner_user_id = control.service_user_id
      AND job.job_type = 'candidate_compute'
      AND control.control_key = 'scheduler';
    GET DIAGNOSTICS v_changed_count = ROW_COUNT;
    RETURN v_changed_count = 1;
END
$fail$;

ALTER FUNCTION public.fail_candidate_analysis_run(uuid, uuid, text, text, text, jsonb)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.fail_candidate_analysis_run(uuid, uuid, text, text, text, jsonb)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.fail_candidate_analysis_run(uuid, uuid, text, text, text, jsonb)
    TO worker;

-- A worker must commit analysis publication and queue settlement together.
-- The deferred fence rejects a SUCCEEDED run without its exact successful
-- queue attempt/feed, and rejects a successful candidate job without output.
CREATE FUNCTION public.assert_candidate_publication_settlement()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $assert$
DECLARE
    v_job_id uuid;
    v_run_id uuid;
    v_run_status text;
    v_job_status text;
    v_attempt_count integer;
    v_service_user_id uuid;
BEGIN
    SELECT control.service_user_id INTO v_service_user_id
    FROM public.candidate_scheduler_control AS control
    WHERE control.control_key = 'scheduler';
    IF NOT FOUND THEN RETURN NULL; END IF;
    -- Constraint triggers run as migration_owner at COMMIT, outside any
    -- request actor scope. Bind the only owner candidate jobs may use before
    -- touching the FORCE-RLS queue tables.
    PERFORM pg_catalog.set_config(
        'app.actor_user_id', v_service_user_id::text, true
    );
    IF TG_TABLE_NAME = 'stock_analysis_runs' THEN
        IF TG_OP = 'DELETE' THEN
            v_run_id := OLD.id;
            v_job_id := OLD.job_id;
        ELSE
            v_run_id := NEW.id;
            v_job_id := NEW.job_id;
        END IF;
    ELSIF TG_TABLE_NAME = 'jobs' THEN
        IF TG_OP = 'DELETE' THEN v_job_id := OLD.id; ELSE v_job_id := NEW.id; END IF;
        SELECT run.id INTO v_run_id
        FROM public.stock_analysis_runs AS run
        WHERE run.job_id = v_job_id;
    ELSE
        IF TG_OP = 'DELETE' THEN v_job_id := OLD.job_id; ELSE v_job_id := NEW.job_id; END IF;
        SELECT run.id INTO v_run_id
        FROM public.stock_analysis_runs AS run
        WHERE run.job_id = v_job_id;
    END IF;

    IF v_run_id IS NULL OR v_job_id IS NULL THEN RETURN NULL; END IF;
    SELECT run.status, job.status, job.attempt_count
    INTO v_run_status, v_job_status, v_attempt_count
    FROM public.stock_analysis_runs AS run
    JOIN public.jobs AS job ON job.id = run.job_id
    WHERE run.id = v_run_id;
    IF NOT FOUND THEN RETURN NULL; END IF;

    IF v_run_status = 'SUCCEEDED' OR v_job_status = 'SUCCEEDED' THEN
        IF v_run_status <> 'SUCCEEDED'
            OR v_job_status <> 'SUCCEEDED'
            OR NOT EXISTS (
                SELECT 1
                FROM public.job_attempts AS attempt
                WHERE attempt.job_id = v_job_id
                  AND attempt.attempt_no = v_attempt_count
                  AND attempt.outcome = 'SUCCEEDED'
                  AND attempt.finished_at IS NOT NULL
            )
            OR NOT EXISTS (
                SELECT 1
                FROM public.candidate_feed_snapshots AS feed
                WHERE feed.run_id = v_run_id
            )
        THEN
            RAISE EXCEPTION 'candidate publication and queue settlement must commit atomically'
                USING ERRCODE = '23514';
        END IF;
    ELSIF v_run_status IN ('FAILED', 'BLOCKED')
        OR v_job_status IN ('FAILED', 'CANCELED')
    THEN
        IF v_run_status NOT IN ('FAILED', 'BLOCKED')
            OR v_job_status NOT IN ('FAILED', 'CANCELED')
        THEN
            RAISE EXCEPTION 'candidate failure and queue settlement must commit atomically'
                USING ERRCODE = '23514';
        END IF;
    END IF;
    RETURN NULL;
END
$assert$;

ALTER FUNCTION public.assert_candidate_publication_settlement()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.assert_candidate_publication_settlement()
    FROM PUBLIC;

CREATE CONSTRAINT TRIGGER stock_analysis_run_requires_settlement
    AFTER INSERT OR UPDATE OR DELETE ON public.stock_analysis_runs
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION public.assert_candidate_publication_settlement();
CREATE CONSTRAINT TRIGGER candidate_job_requires_publication
    AFTER INSERT OR UPDATE OR DELETE ON public.jobs
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION public.assert_candidate_publication_settlement();
CREATE CONSTRAINT TRIGGER candidate_attempt_requires_publication
    AFTER INSERT OR UPDATE OR DELETE ON public.job_attempts
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION public.assert_candidate_publication_settlement();

-- Serving applications receive only the six attribution rows of one fully
-- published run. They have no direct SELECT on licensed Raw candidate tables.
CREATE FUNCTION public.candidate_published_source_attributions(p_run_id uuid)
RETURNS TABLE (
    source text, dataset_id text, license_ref text, entitlement_id uuid,
    contract_reference text, contract_document_sha256 text
)
LANGUAGE sql
SECURITY DEFINER
STABLE
SET search_path = pg_catalog
AS $attribution$
    WITH run AS (
        SELECT run.* FROM public.stock_analysis_runs AS run
         WHERE run.id=p_run_id AND run.status='SUCCEEDED'
           AND EXISTS (SELECT 1 FROM public.candidate_feed_snapshots AS feed
                        WHERE feed.run_id=run.id AND feed.status='PUBLISHED')
    ), refs AS (
        SELECT 'price'::text AS source, dataset.dataset_id, price.license_ref,
               run.price_entitlement_id AS entitlement_id,
               price.first_session AS first_use_date,
               run.as_of_date AS last_use_date
          FROM run JOIN public.candidate_price_publications AS price
            ON price.dataset_version_id=run.price_dataset_version_id
           AND price.entitlement_id=run.price_entitlement_id
           AND price.curated_generation=run.price_curated_version
           AND price.manifest_sha256=run.price_manifest_sha256
          JOIN public.dataset_versions AS dataset ON dataset.id=price.dataset_version_id
        UNION ALL
        SELECT 'universe', dataset.dataset_id, universe.license_ref,
               run.universe_entitlement_id, run.as_of_date, run.as_of_date
          FROM run JOIN public.candidate_universe_snapshots AS universe
            ON universe.id=run.universe_snapshot_id
           AND universe.entitlement_id=run.universe_entitlement_id
          JOIN public.dataset_versions AS dataset ON dataset.id=universe.dataset_version_id
        UNION ALL
        SELECT 'market_status', dataset.dataset_id, status.license_ref,
               run.status_entitlement_id, run.as_of_date, run.as_of_date
          FROM run JOIN public.candidate_market_status_observations AS status
            ON status.dataset_version_id=run.status_dataset_version_id
           AND status.manifest_sha256=run.status_manifest_sha256
           AND status.entitlement_id=run.status_entitlement_id
          JOIN public.dataset_versions AS dataset ON dataset.id=status.dataset_version_id
        UNION ALL
        SELECT 'flow', dataset.dataset_id, member.license_ref,
               run.flow_entitlement_id,
               min(flow.trade_date) OVER (PARTITION BY member.dataset_version_id),
               run.as_of_date
          FROM run JOIN public.candidate_investor_flows AS flow ON true
          JOIN public.candidate_investor_flow_snapshot_rows AS member
            ON member.flow_observation_id=flow.id
           AND member.dataset_version_id=run.flow_dataset_version_id
           AND member.entitlement_id=run.flow_entitlement_id
          JOIN public.dataset_versions AS dataset ON dataset.id=member.dataset_version_id
           AND dataset.manifest_sha256=run.flow_manifest_sha256
        UNION ALL
        SELECT 'fundamental', dataset.dataset_id, fact.license_ref,
               run.fundamental_entitlement_id, run.as_of_date, run.as_of_date
          FROM run JOIN public.candidate_fundamental_observations AS fact
            ON fact.dataset_version_id=run.fundamental_dataset_version_id
           AND fact.manifest_sha256=run.fundamental_manifest_sha256
           AND fact.entitlement_id=run.fundamental_entitlement_id
          JOIN public.dataset_versions AS dataset ON dataset.id=fact.dataset_version_id
        UNION ALL
        SELECT 'sector', dataset.dataset_id, sector.license_ref,
               run.sector_entitlement_id, run.as_of_date, run.as_of_date
          FROM run JOIN public.candidate_sector_versions AS sector
            ON sector.id=run.sector_version_id
           AND sector.entitlement_id=run.sector_entitlement_id
          JOIN public.dataset_versions AS dataset ON dataset.id=sector.dataset_version_id
    )
    SELECT DISTINCT refs.source, refs.dataset_id, refs.license_ref,
           entitlement.id, entitlement.contract_reference,
           entitlement.contract_document_sha256
      FROM refs JOIN public.data_entitlements AS entitlement
       ON entitlement.id=refs.entitlement_id
       AND entitlement.contract_reference=refs.license_ref
       AND public.candidate_source_entitlement_is_valid(
           refs.entitlement_id, refs.license_ref, refs.dataset_id,
           refs.first_use_date, refs.last_use_date)
     ORDER BY refs.source, refs.dataset_id
$attribution$;

ALTER FUNCTION public.candidate_published_source_attributions(uuid) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.candidate_published_source_attributions(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.candidate_published_source_attributions(uuid) TO app;

-- Activate only after every guard and capability is installed.
UPDATE public.candidate_scheduler_control
SET active = true, updated_at = clock_timestamp()
WHERE control_key = 'scheduler';
