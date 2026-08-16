-- 0039: transactional auth audit outbox.
-- State-changing auth operations enqueue here in their existing transaction;
-- the audit writer later copies rows idempotently into audit_logs.

CREATE TABLE public.auth_audit_outbox (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    event_key      text NOT NULL UNIQUE,
    action         text NOT NULL,
    actor_role     text NOT NULL DEFAULT 'system',
    actor_user_id  uuid REFERENCES public.users (id),
    target_type    text,
    target_id      text,
    reason         text,
    created_at     timestamptz NOT NULL DEFAULT now(),
    attempts       integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    available_at   timestamptz NOT NULL DEFAULT now(),
    delivered_at   timestamptz
);
CREATE INDEX auth_audit_outbox_pending_idx
    ON public.auth_audit_outbox (available_at, id)
    WHERE delivered_at IS NULL;

ALTER TABLE public.auth_audit_outbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.auth_audit_outbox FORCE ROW LEVEL SECURITY;
-- Serving roles receive EXECUTE on the definer function only. The migration
-- owner policies are intentionally not paired with table grants, so they do
-- not expose the outbox as a direct application data surface.
CREATE POLICY auth_audit_outbox_enqueue ON public.auth_audit_outbox
    FOR INSERT TO migration_owner WITH CHECK (true);
CREATE POLICY auth_audit_outbox_owner_read ON public.auth_audit_outbox
    FOR SELECT TO migration_owner USING (true);
CREATE POLICY auth_audit_outbox_owner_update ON public.auth_audit_outbox
    FOR UPDATE TO migration_owner USING (true) WITH CHECK (true);
CREATE POLICY auth_audit_outbox_owner_delete ON public.auth_audit_outbox
    FOR DELETE TO migration_owner USING (true);
CREATE POLICY auth_audit_log_insert_migration_owner ON public.audit_logs
    FOR INSERT TO migration_owner WITH CHECK (true);
-- The definer delivery function needs a narrow migration-owner read policy for
-- `ON CONFLICT DO NOTHING` conflict checks and operational copy verification.
-- No serving role receives this policy or a direct audit table grant.
CREATE POLICY auth_audit_log_select_migration_owner ON public.audit_logs
    FOR SELECT TO migration_owner USING (true);

-- The function is the only serving-role write capability. It runs inside the
-- caller's transaction, validates actor attribution, and is idempotent only
-- when every immutable payload field matches the original event.
CREATE FUNCTION public.enqueue_auth_audit(
    p_event_key text,
    p_action text,
    p_actor_user_id uuid,
    p_target_type text,
    p_target_id text,
    p_reason text,
    p_created_at_epoch bigint
)
RETURNS uuid
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET lock_timeout = '1s'
SET statement_timeout = '5s'
AS $function$
DECLARE
    v_id uuid;
    v_existing public.auth_audit_outbox%ROWTYPE;
    v_created_at timestamptz;
    v_actor uuid;
BEGIN
    IF p_event_key IS NULL OR pg_catalog.length(p_event_key) = 0
        OR pg_catalog.length(p_event_key) > 256
        OR p_action IS NULL OR pg_catalog.length(p_action) = 0
        OR pg_catalog.length(p_action) > 128
        OR p_event_key ~ '[[:cntrl:]]'
        OR p_action ~ '[[:cntrl:]]'
        OR p_target_type ~ '[[:cntrl:]]'
        OR p_target_id ~ '[[:cntrl:]]'
        OR p_created_at_epoch IS NULL
    THEN
        RAISE EXCEPTION 'auth audit outbox input is invalid'
            USING ERRCODE = '22023';
    END IF;
    v_created_at := pg_catalog.to_timestamp(p_created_at_epoch::double precision);

    IF session_user NOT IN ('audit_writer', 'migration_owner') THEN
        v_actor := NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid;
        IF p_actor_user_id IS NULL OR p_actor_user_id IS DISTINCT FROM v_actor THEN
            RAISE EXCEPTION 'auth audit actor context is invalid'
                USING ERRCODE = '42501';
        END IF;
    END IF;

    INSERT INTO public.auth_audit_outbox (
        event_key, action, actor_role, actor_user_id,
        target_type, target_id, reason, created_at
    )
    VALUES (
        p_event_key, p_action, 'system', p_actor_user_id,
        p_target_type, p_target_id, p_reason, v_created_at
    )
    ON CONFLICT (event_key) DO NOTHING
    RETURNING id INTO v_id;
    IF FOUND THEN
        RETURN v_id;
    END IF;

    -- A concurrent insertion has now committed (the unique check waited for
    -- it). Lock and compare every immutable field before accepting replay.
    SELECT * INTO v_existing
    FROM public.auth_audit_outbox
    WHERE event_key = p_event_key
    FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'auth audit event disappeared during idempotent enqueue'
            USING ERRCODE = '40001';
    END IF;
    IF FOUND THEN
        IF v_existing.action IS DISTINCT FROM p_action
            OR v_existing.actor_user_id IS DISTINCT FROM p_actor_user_id
            OR v_existing.target_type IS DISTINCT FROM p_target_type
            OR v_existing.target_id IS DISTINCT FROM p_target_id
            OR v_existing.reason IS DISTINCT FROM p_reason
            OR v_existing.created_at IS DISTINCT FROM v_created_at
        THEN
            RAISE EXCEPTION 'auth audit event key payload mismatch'
                USING ERRCODE = '23505';
        END IF;
        RETURN v_existing.id;
    END IF;
END
$function$;

ALTER FUNCTION public.enqueue_auth_audit(text, text, uuid, text, text, text, bigint)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.enqueue_auth_audit(text, text, uuid, text, text, text, bigint)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.enqueue_auth_audit(text, text, uuid, text, text, text, bigint)
    TO app, audit_writer;

-- A single SECURITY DEFINER call owns the lock/copy/mark transaction. The
-- migration-owner policy is deliberate: audit_writer has EXECUTE only and no
-- direct outbox table visibility or DML capability.
CREATE FUNCTION public.deliver_auth_audit_batch(p_limit integer DEFAULT 64)
RETURNS TABLE (delivered_count integer, failed_count integer)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET lock_timeout = '1s'
SET statement_timeout = '5s'
AS $function$
DECLARE
    v_id uuid;
    v_delivered integer := 0;
    v_failed integer := 0;
BEGIN
    PERFORM pg_catalog.set_config('statement_timeout', '5000', true);
    PERFORM pg_catalog.set_config('lock_timeout', '1000', true);
    IF p_limit IS NULL OR p_limit < 1 OR p_limit > 1000 THEN
        RAISE EXCEPTION 'audit batch limit is invalid' USING ERRCODE = '22023';
    END IF;
    FOR v_id IN
        SELECT id
        FROM public.auth_audit_outbox
        WHERE delivered_at IS NULL AND available_at <= pg_catalog.clock_timestamp()
        ORDER BY available_at, id
        FOR UPDATE SKIP LOCKED
        LIMIT p_limit
    LOOP
        BEGIN
            INSERT INTO public.audit_logs (
                id, action, actor_role, actor_user_id, target_type, target_id,
                reason, created_at
            )
            SELECT id, action, actor_role, actor_user_id, target_type, target_id,
                   reason, created_at
            FROM public.auth_audit_outbox
            WHERE id = v_id
            ON CONFLICT (id) DO NOTHING;
            UPDATE public.auth_audit_outbox
            SET delivered_at = pg_catalog.clock_timestamp()
            WHERE id = v_id AND delivered_at IS NULL;
            v_delivered := v_delivered + 1;
        EXCEPTION WHEN OTHERS THEN
            -- The failed copy is retried with bounded backoff. The exception
            -- block is a PL/pgSQL savepoint; successful rows and this retry
            -- state still commit together only when the outer statement does.
            UPDATE public.auth_audit_outbox
            SET attempts = attempts + 1,
                available_at = pg_catalog.clock_timestamp() +
                    pg_catalog.make_interval(secs => LEAST(300, (attempts + 1) * 5))
            WHERE id = v_id;
            v_failed := v_failed + 1;
        END;
    END LOOP;
    RETURN QUERY SELECT v_delivered, v_failed;
END
$function$;

ALTER FUNCTION public.deliver_auth_audit_batch(integer) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.deliver_auth_audit_batch(integer)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.deliver_auth_audit_batch(integer) TO audit_writer;

-- Read-only operational view through a definer function; no table grant is
-- required by the audit connection. The age is NULL when no row is pending.
CREATE FUNCTION public.auth_audit_outbox_stats()
RETURNS TABLE (pending_count bigint, oldest_pending_age_secs bigint)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET lock_timeout = '1s'
SET statement_timeout = '3s'
AS $function$
BEGIN
    PERFORM pg_catalog.set_config('statement_timeout', '3000', true);
    PERFORM pg_catalog.set_config('lock_timeout', '1000', true);
    RETURN QUERY
    SELECT count(*)::bigint,
           COALESCE(
               EXTRACT(EPOCH FROM (pg_catalog.clock_timestamp() - min(created_at)))::bigint,
               0::bigint
           )
    FROM public.auth_audit_outbox
    WHERE delivered_at IS NULL;
END
$function$;

ALTER FUNCTION public.auth_audit_outbox_stats() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.auth_audit_outbox_stats()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.auth_audit_outbox_stats() TO audit_writer;

-- Delivered rows are retained only for a bounded operational window after
-- their immutable audit_logs copy exists. The function never targets pending
-- rows and is callable only by the dispatcher role through EXECUTE.
CREATE FUNCTION public.prune_auth_audit_outbox(
    p_keep_seconds bigint DEFAULT 604800,
    p_limit integer DEFAULT 256
)
RETURNS bigint
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET lock_timeout = '1s'
SET statement_timeout = '5s'
AS $function$
DECLARE
    v_deleted bigint;
BEGIN
    PERFORM pg_catalog.set_config('statement_timeout', '5000', true);
    PERFORM pg_catalog.set_config('lock_timeout', '1000', true);
    IF p_keep_seconds IS NULL OR p_keep_seconds < 86400
        OR p_keep_seconds > 31536000
        OR p_limit IS NULL OR p_limit < 1 OR p_limit > 10000
    THEN
        RAISE EXCEPTION 'audit retention parameters are invalid' USING ERRCODE = '22023';
    END IF;
    WITH candidates AS (
        SELECT id
        FROM public.auth_audit_outbox
        WHERE delivered_at IS NOT NULL
          AND delivered_at < pg_catalog.clock_timestamp()
              - pg_catalog.make_interval(secs => p_keep_seconds)
        ORDER BY delivered_at, id
        FOR UPDATE SKIP LOCKED
        LIMIT p_limit
    )
    DELETE FROM public.auth_audit_outbox AS outbox
    USING candidates
    WHERE outbox.id = candidates.id;
    GET DIAGNOSTICS v_deleted = ROW_COUNT;
    RETURN v_deleted;
END
$function$;

ALTER FUNCTION public.prune_auth_audit_outbox(bigint, integer) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.prune_auth_audit_outbox(bigint, integer)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.prune_auth_audit_outbox(bigint, integer) TO audit_writer;
