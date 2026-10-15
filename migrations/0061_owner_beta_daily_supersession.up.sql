-- R2 I4f. Additive cleanup authority; 0060 publication/queue guards stay unchanged.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

CREATE FUNCTION public.cancel_superseded_owner_beta_daily_claim(
    p_run_id uuid, p_job_id uuid, p_attempt_id uuid, p_attempt_no integer,
    p_worker text
) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET lock_timeout = '5s' SET statement_timeout = '30s'
AS $function$
DECLARE
    owner_id uuid;
    old_actor text := current_setting('app.actor_user_id', true);
    j public.jobs;
    a public.job_attempts;
    r public.owner_beta_recommendation_runs;
    c public.owner_beta_daily_control;
    res public.owner_beta_daily_reservations;
    expected date;
BEGIN
    -- The grant is not enough: an elevated login using SET ROLE is not a worker.
    IF session_user <> 'worker'
       OR current_setting('role', true) IS DISTINCT FROM 'none'
       OR NOT EXISTS (
           SELECT 1 FROM pg_catalog.pg_roles
           WHERE rolname = session_user AND rolcanlogin AND NOT rolsuper
             AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication
             AND NOT rolbypassrls
       )
    THEN
        RAISE EXCEPTION 'daily cancellation principal denied' USING ERRCODE = '42501';
    END IF;
    IF p_run_id IS NULL OR p_job_id IS NULL OR p_attempt_id IS NULL
       OR p_attempt_no IS NULL OR p_attempt_no NOT BETWEEN 1 AND 3
       OR p_worker IS NULL OR p_worker = ''
    THEN
        RAISE EXCEPTION 'daily cancellation identity invalid' USING ERRCODE = '23514';
    END IF;

    SELECT owner_user_id INTO owner_id
    FROM public.owner_beta_daily_control WHERE id = 'etf11';
    IF owner_id IS NULL THEN
        RAISE EXCEPTION 'daily cancellation control unavailable' USING ERRCODE = '23514';
    END IF;
    PERFORM set_config('app.actor_user_id', owner_id::text, true);

    -- Same order as publication: job, attempt, run, control, source fence.
    SELECT * INTO j FROM public.jobs
      WHERE id = p_job_id AND owner_user_id = owner_id FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'daily cancellation job unavailable' USING ERRCODE = '23514';
    END IF;
    SELECT * INTO a FROM public.job_attempts
      WHERE id = p_attempt_id AND job_id = j.id FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'daily cancellation attempt unavailable' USING ERRCODE = '23514';
    END IF;
    SELECT * INTO r FROM public.owner_beta_recommendation_runs
      WHERE id = p_run_id AND job_id = j.id AND owner_user_id = owner_id FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'daily cancellation run unavailable' USING ERRCODE = '23514';
    END IF;
    SELECT * INTO c FROM public.owner_beta_daily_control
      WHERE id = 'etf11' AND owner_user_id = owner_id FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'daily cancellation control changed' USING ERRCODE = '23514';
    END IF;
    SELECT * INTO res FROM public.owner_beta_daily_reservations
      WHERE run_id = r.id AND job_id = j.id;
    IF NOT FOUND
       OR j.job_type IS DISTINCT FROM 'owner_beta_price_recommendation'
       OR j.payload_json IS DISTINCT FROM public.daily_0060_payload(r)
       OR j.max_attempts <> 3 OR j.attempt_count IS DISTINCT FROM p_attempt_no
       OR a.attempt_no IS DISTINCT FROM p_attempt_no
       OR a.claimed_by IS DISTINCT FROM p_worker
       OR r.trigger_kind IS DISTINCT FROM 'DAILY'
       OR ROW(res.owner_user_id, res.strategy_config_id, res.session_date,
              res.trigger_version, res.config_epoch, res.activation_epoch,
              res.strategy_config_sha256, res.admission_sha256)
          IS DISTINCT FROM
          ROW(r.owner_user_id, r.strategy_config_id, r.as_of,
              r.daily_trigger_version, r.daily_config_epoch, r.daily_activation_epoch,
              r.strategy_config_sha256, r.daily_pins->>'admission_sha256')
       OR r.factor_snapshot_sha256 IS NOT NULL OR r.target_snapshot_sha256 IS NOT NULL
       OR r.cash_weight IS NOT NULL
       OR EXISTS (SELECT 1 FROM public.owner_beta_recommendation_items WHERE recommendation_run_id = r.id)
       OR EXISTS (SELECT 1 FROM public.owner_beta_daily_publication_claims WHERE run_id = r.id)
    THEN
        RAISE EXCEPTION 'daily cancellation binding invalid' USING ERRCODE = '23514';
    END IF;

    -- Only an exactly completed supersession is replayable, never a user cancel.
    IF j.status = 'CANCELED' AND r.status = 'CANCELED' AND a.outcome = 'FAILED'
       AND r.error_code = 'DAILY_SESSION_SUPERSEDED'
       AND a.error_code = 'DAILY_SESSION_SUPERSEDED'
       AND j.locked_by IS NULL AND j.locked_at IS NULL
       AND j.finished_at IS NOT NULL AND a.finished_at IS NOT NULL AND r.finished_at IS NOT NULL
    THEN
        PERFORM set_config('app.actor_user_id', COALESCE(old_actor, ''), true);
        RETURN true;
    END IF;
    IF j.status IS DISTINCT FROM 'RUNNING' OR r.status IS DISTINCT FROM 'PENDING'
       OR a.outcome IS DISTINCT FROM 'RUNNING'
       OR j.locked_by IS DISTINCT FROM p_worker OR j.locked_at IS NULL
       OR j.finished_at IS NOT NULL OR a.finished_at IS NOT NULL
       OR r.error_code IS NOT NULL OR r.finished_at IS NOT NULL
    THEN
        RAISE EXCEPTION 'daily cancellation claim stale' USING ERRCODE = '55000';
    END IF;

    PERFORM public.daily_0060_source_lock(owner_id, false);
    IF j.locked_at <= clock_timestamp() - make_interval(secs => c.lease_seconds) THEN
        RAISE EXCEPTION 'daily cancellation lease stale' USING ERRCODE = '55000';
    END IF;
    -- Cleanup needs trustworthy newer-session evidence, not permission to publish.
    -- Disabled control/config/rights cannot force an older claim to publish.
    IF r.daily_pins->>'policy_sha256' IS DISTINCT FROM c.policy_sha256 THEN
        PERFORM set_config('app.actor_user_id', COALESCE(old_actor, ''), true);
        RETURN false;
    END IF;
    expected := public.daily_0060_expected(c);
    IF expected IS NULL OR expected <= r.as_of OR expected < c.first_auto_session THEN
        PERFORM set_config('app.actor_user_id', COALESCE(old_actor, ''), true);
        RETURN false;
    END IF;

    -- The unchanged 0060 triggers fence every transition and the deferred matrix
    -- verifies the final four-row relationship at commit. Reservation is permanent.
    UPDATE public.jobs SET status = 'CANCELED', updated_at = clock_timestamp()
      WHERE id = j.id;
    UPDATE public.job_attempts SET outcome = 'FAILED',
      error_code = 'DAILY_SESSION_SUPERSEDED',
      error_message = 'A newer eligible daily session superseded this claim.',
      finished_at = clock_timestamp()
      WHERE id = a.id;
    UPDATE public.jobs SET locked_by = NULL, locked_at = NULL, updated_at = clock_timestamp()
      WHERE id = j.id;
    UPDATE public.owner_beta_recommendation_runs
      SET status = 'CANCELED', error_code = 'DAILY_SESSION_SUPERSEDED',
          started_at = COALESCE(started_at, j.started_at),
          finished_at = clock_timestamp(), updated_at = clock_timestamp()
      WHERE id = r.id;

    PERFORM set_config('app.actor_user_id', COALESCE(old_actor, ''), true);
    RETURN true;
EXCEPTION WHEN OTHERS THEN
    PERFORM set_config('app.actor_user_id', COALESCE(old_actor, ''), true);
    RAISE;
END
$function$;

ALTER FUNCTION public.cancel_superseded_owner_beta_daily_claim(uuid,uuid,uuid,integer,text)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.cancel_superseded_owner_beta_daily_claim(uuid,uuid,uuid,integer,text)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer, daily_input_admitter;
GRANT EXECUTE ON FUNCTION public.cancel_superseded_owner_beta_daily_claim(uuid,uuid,uuid,integer,text)
    TO worker;
