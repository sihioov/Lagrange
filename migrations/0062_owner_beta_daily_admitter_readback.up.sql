-- R2 I4g2. Narrow readback for an independent daily admitter.
-- Returned metadata is neither a filesystem capability nor current publication authority.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

CREATE FUNCTION public.read_owner_beta_daily_admission_context(p_receipt_id uuid)
RETURNS TABLE (
    owner_user_id uuid, policy_sha256 text, activation_epoch bigint,
    enabled boolean, entitlement_reference text, entitlement_sha256 text,
    first_auto_session date, observed_at timestamptz,
    latest_admission_sha256 text, source_blocked boolean
)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
SET lock_timeout = '5s' SET statement_timeout = '30s'
AS $function$
DECLARE c public.owner_beta_daily_control; r public.owner_beta_daily_poll_receipts;
    latest text;
BEGIN
    IF session_user <> 'daily_input_admitter'
       OR current_setting('role', true) IS DISTINCT FROM 'none'
       OR NOT EXISTS (
           SELECT 1 FROM pg_catalog.pg_roles
           WHERE rolname = session_user AND rolcanlogin AND NOT rolsuper
             AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication
             AND NOT rolbypassrls
       )
    THEN
        RAISE EXCEPTION 'daily read principal denied' USING ERRCODE = '42501';
    END IF;
    IF p_receipt_id IS NULL OR p_receipt_id = '00000000-0000-0000-0000-000000000000'::uuid THEN
        RAISE EXCEPTION 'daily context identity invalid' USING ERRCODE = '23514';
    END IF;
    c := public.daily_0060_authority(NULL, false);
    PERFORM public.daily_0060_source_lock(c.owner_user_id, false);
    r := public.daily_0060_receipt(p_receipt_id, 'ADMISSION', c);
    IF r.completed_at IS NOT NULL THEN
        RAISE EXCEPTION 'daily context cycle completed' USING ERRCODE = '23514';
    END IF;
    -- A policy switch must not silently turn an existing chain into an empty base.
    IF EXISTS (
        SELECT 1 FROM public.owner_beta_daily_admissions a
        WHERE a.owner_user_id = c.owner_user_id AND a.policy_sha256 <> c.policy_sha256
    ) THEN
        RAISE EXCEPTION 'daily context history policy changed' USING ERRCODE = '23514';
    END IF;
    SELECT a.admission_sha256 INTO latest
      FROM public.owner_beta_daily_admissions a
      WHERE a.owner_user_id = c.owner_user_id AND a.policy_sha256 = c.policy_sha256
      ORDER BY a.range_end DESC, a.admission_sha256 LIMIT 1;
    RETURN QUERY SELECT c.owner_user_id, c.policy_sha256, c.activation_epoch,
        c.enabled, c.entitlement_reference, c.entitlement_sha256, c.first_auto_session,
        clock_timestamp(), latest,
        (EXISTS (SELECT 1 FROM public.owner_beta_daily_source_events e
                 WHERE e.owner_user_id = c.owner_user_id)
         OR EXISTS (SELECT 1 FROM public.owner_beta_daily_admission_events e
                    WHERE e.owner_user_id = c.owner_user_id));
END
$function$;

CREATE FUNCTION public.read_owner_beta_daily_admitted_input(p_admission_sha256 text)
RETURNS TABLE (
    owner_user_id uuid, policy_sha256 text, content_sha256 text, manifest_sha256 text,
    admission_sha256 text, parent_admission_sha256 text, source_frontier_sha256 text,
    range_start date, range_end date, session_count integer, bar_count integer,
    usable boolean
)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
SET lock_timeout = '5s' SET statement_timeout = '30s'
AS $function$
DECLARE c public.owner_beta_daily_control;
BEGIN
    IF session_user <> 'daily_input_admitter'
       OR current_setting('role', true) IS DISTINCT FROM 'none'
       OR NOT EXISTS (
           SELECT 1 FROM pg_catalog.pg_roles
           WHERE rolname = session_user AND rolcanlogin AND NOT rolsuper
             AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication
             AND NOT rolbypassrls
       )
    THEN
        RAISE EXCEPTION 'daily read principal denied' USING ERRCODE = '42501';
    END IF;
    IF p_admission_sha256 IS NULL OR p_admission_sha256 !~ '^sha256:[0-9a-f]{64}$' THEN
        RAISE EXCEPTION 'daily input read identity invalid' USING ERRCODE = '23514';
    END IF;
    c := public.daily_0060_authority(NULL, false);
    PERFORM public.daily_0060_source_lock(c.owner_user_id, false);
    RETURN QUERY SELECT a.owner_user_id, a.policy_sha256, a.content_sha256,
        a.manifest_sha256, a.admission_sha256, a.parent_admission_sha256,
        a.evidence_json->>'source_frontier_sha256', a.range_start, a.range_end,
        a.session_count, a.bar_count,
        public.daily_0060_admission_usable(c, a.admission_sha256, false)
      FROM public.owner_beta_daily_admissions a
      WHERE a.admission_sha256 = p_admission_sha256
        AND a.owner_user_id = c.owner_user_id AND a.policy_sha256 = c.policy_sha256;
END
$function$;

CREATE FUNCTION public.read_owner_beta_daily_admission_receipt(p_receipt_id uuid)
RETURNS TABLE (
    receipt_id uuid, owner_user_id uuid, policy_sha256 text, activation_epoch bigint,
    started_at timestamptz, completed_at timestamptz, result_code text,
    source_frontier_sha256 text, admission_sha256 text, event_sha256 text,
    event_code text, matches_current_cycle boolean
)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
SET lock_timeout = '5s' SET statement_timeout = '30s'
AS $function$
DECLARE c public.owner_beta_daily_control; r public.owner_beta_daily_poll_receipts;
    event_reason text; latest uuid;
BEGIN
    IF session_user <> 'daily_input_admitter'
       OR current_setting('role', true) IS DISTINCT FROM 'none'
       OR NOT EXISTS (
           SELECT 1 FROM pg_catalog.pg_roles
           WHERE rolname = session_user AND rolcanlogin AND NOT rolsuper
             AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication
             AND NOT rolbypassrls
       )
    THEN
        RAISE EXCEPTION 'daily read principal denied' USING ERRCODE = '42501';
    END IF;
    IF p_receipt_id IS NULL OR p_receipt_id = '00000000-0000-0000-0000-000000000000'::uuid THEN
        RAISE EXCEPTION 'daily receipt read identity invalid' USING ERRCODE = '23514';
    END IF;
    c := public.daily_0060_authority(NULL, false);
    PERFORM public.daily_0060_source_lock(c.owner_user_id, false);
    SELECT * INTO r FROM public.owner_beta_daily_poll_receipts pr
      WHERE pr.receipt_id = p_receipt_id AND pr.stage = 'ADMISSION'
        AND pr.owner_user_id = c.owner_user_id AND pr.policy_sha256 = c.policy_sha256
      FOR SHARE;
    IF NOT FOUND THEN RETURN; END IF;
    -- Completed old-epoch receipts may be inspected, but never become current
    -- merely because they were previously VERIFIED.
    SELECT s.latest_receipt_id INTO latest FROM public.owner_beta_daily_poll_state s
      WHERE s.stage = 'ADMISSION';
    IF r.result_code = 'VERIFIED' THEN
        IF r.completed_at IS NULL OR r.event_sha256 IS NOT NULL
           OR NOT EXISTS (
              SELECT 1 FROM public.owner_beta_daily_admissions a
              WHERE a.admission_sha256 = r.admission_sha256
                AND a.owner_user_id = r.owner_user_id AND a.policy_sha256 = r.policy_sha256
                AND a.evidence_json->>'source_frontier_sha256' = r.source_frontier_sha256
                AND r.completion_binding_sha256 =
                    public.daily_0060_digest('sql-admission-v1' || a.evidence_json::text)
           )
        THEN
            RAISE EXCEPTION 'daily receipt read binding invalid' USING ERRCODE = '23514';
        END IF;
    END IF;
    IF r.event_sha256 IS NOT NULL THEN
        SELECT e.code INTO event_reason FROM public.owner_beta_daily_source_events e
          WHERE e.event_sha256 = r.event_sha256 AND e.owner_user_id = c.owner_user_id;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'daily receipt read event invalid' USING ERRCODE = '23514';
        END IF;
    END IF;
    RETURN QUERY SELECT r.receipt_id, r.owner_user_id, r.policy_sha256, r.activation_epoch,
        r.started_at, r.completed_at, r.result_code, r.source_frontier_sha256,
        r.admission_sha256, r.event_sha256, event_reason,
        COALESCE(latest = r.receipt_id AND r.activation_epoch = c.activation_epoch, false);
END
$function$;

ALTER FUNCTION public.read_owner_beta_daily_admission_context(uuid) OWNER TO migration_owner;
ALTER FUNCTION public.read_owner_beta_daily_admitted_input(text) OWNER TO migration_owner;
ALTER FUNCTION public.read_owner_beta_daily_admission_receipt(uuid) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.read_owner_beta_daily_admission_context(uuid),
    public.read_owner_beta_daily_admitted_input(text),
    public.read_owner_beta_daily_admission_receipt(uuid)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer, daily_input_admitter;
GRANT EXECUTE ON FUNCTION public.read_owner_beta_daily_admission_context(uuid),
    public.read_owner_beta_daily_admitted_input(text),
    public.read_owner_beta_daily_admission_receipt(uuid) TO daily_input_admitter;
