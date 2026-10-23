-- Closed control transaction for one verified immutable release.
-- Values arrive as bounded hex data. No serving role gains a new function/grant.
BEGIN;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '15s';
SET LOCAL idle_in_transaction_session_timeout = '20s';
SET LOCAL search_path = pg_catalog, pg_temp;
DO $daily_control_role$
BEGIN
    IF current_user <> 'migration_owner' OR session_user <> 'migration_owner' THEN
        RAISE EXCEPTION 'DAILY_CONTROL_ROLE' USING ERRCODE='42501';
    END IF;
END $daily_control_role$;
CREATE TEMP TABLE daily_release_control_input (
    action text NOT NULL,
    config jsonb NOT NULL,
    policy_bytes text NOT NULL,
    expected_commit text NOT NULL,
    capacity integer,
    lease_seconds integer,
    backoff_seconds bigint
) ON COMMIT DROP;
INSERT INTO pg_temp.daily_release_control_input VALUES (
    '__ACTION__',
    convert_from(decode('__CONFIG_HEX__','hex'),'UTF8')::jsonb,
    convert_from(decode('__POLICY_HEX__','hex'),'UTF8'),
    '__COMMIT__', __CAPACITY__, __LEASE__, __BACKOFF__
);
DO $daily_control$
DECLARE
    i pg_temp.daily_release_control_input;
    c public.owner_beta_daily_control;
    owner_id uuid;
BEGIN
    SELECT * INTO STRICT i FROM pg_temp.daily_release_control_input;
    IF i.action NOT IN ('prepare','check','check-disabled','enable','disable')
       OR i.expected_commit !~ '^[0-9a-f]{40}$'
       OR i.expected_commit = repeat('0',40)
       OR i.config->>'code_commit' IS DISTINCT FROM i.expected_commit
       OR i.config->>'schema' IS DISTINCT FROM 'owner-beta-daily-process-v1'
       OR i.config->>'policy_sha256' IS DISTINCT FROM public.daily_0060_digest(i.policy_bytes)
       OR (i.action <> 'disable' AND
           (i.capacity IS NULL OR i.lease_seconds IS NULL OR i.backoff_seconds IS NULL
            OR i.capacity <= 0 OR i.lease_seconds <= 0 OR i.backoff_seconds <= 0))
       OR (i.action = 'disable' AND
           (i.capacity IS NOT NULL OR i.lease_seconds IS NOT NULL OR i.backoff_seconds IS NOT NULL))
    THEN
        RAISE EXCEPTION 'DAILY_CONTROL_INPUT' USING ERRCODE='23514';
    END IF;
    PERFORM public.daily_0060_validate_policy(i.policy_bytes::jsonb);
    owner_id := (i.config->>'owner')::uuid;
    LOCK TABLE public.owner_beta_daily_control IN SHARE ROW EXCLUSIVE MODE;
    SELECT * INTO c FROM public.owner_beta_daily_control WHERE id='etf11' FOR UPDATE;
    IF NOT FOUND THEN
        IF i.action <> 'prepare' THEN
            RAISE EXCEPTION 'DAILY_CONTROL_ABSENT' USING ERRCODE='23514';
        END IF;
        INSERT INTO public.owner_beta_daily_control (
            owner_user_id,policy_sha256,policy_json,entitlement_reference,
            entitlement_sha256,first_auto_session,max_jobs_per_owner,
            lease_seconds,backoff_seconds
        ) VALUES (
            owner_id,i.config->>'policy_sha256',i.policy_bytes::jsonb,
            i.config->>'entitlement_reference',i.config->>'entitlement_sha256',
            (i.config->>'first_auto_session')::date,i.capacity,
            i.lease_seconds,i.backoff_seconds
        ) RETURNING * INTO c;
    END IF;
    IF ROW(c.owner_user_id,c.policy_sha256,c.policy_json,c.entitlement_reference,
           c.entitlement_sha256,c.first_auto_session,c.trigger_version)
       IS DISTINCT FROM
       ROW(owner_id,i.config->>'policy_sha256',i.policy_bytes::jsonb,
           i.config->>'entitlement_reference',i.config->>'entitlement_sha256',
           (i.config->>'first_auto_session')::date,'owner-beta-daily-trigger-v1')
       OR (i.action <> 'disable' AND
           ROW(c.max_jobs_per_owner,c.lease_seconds,c.backoff_seconds)
           IS DISTINCT FROM ROW(i.capacity,i.lease_seconds,i.backoff_seconds))
    THEN
        RAISE EXCEPTION 'DAILY_CONTROL_BINDING' USING ERRCODE='23514';
    END IF;
    IF i.action IN ('prepare','check-disabled') AND c.enabled THEN
        RAISE EXCEPTION 'DAILY_CONTROL_NOT_DISABLED' USING ERRCODE='23514';
    END IF;
    -- Rights/Owner revocation and changed capacity must not block disabling.
    -- Disable preserves those stored fields; it never repairs or resets them.
    IF i.action IN ('prepare','enable') THEN
        PERFORM public.daily_0060_authority(NULL,false);
    END IF;
    IF i.action='enable' AND NOT c.enabled THEN
        UPDATE public.owner_beta_daily_control SET enabled=true WHERE id='etf11';
    ELSIF i.action='disable' AND c.enabled THEN
        UPDATE public.owner_beta_daily_control SET enabled=false WHERE id='etf11';
    END IF;
    -- Repeated operations leave updated_at and activation_epoch unchanged.
END $daily_control$;
SELECT CASE enabled WHEN true THEN 'DAILY_CONTROL: ENABLED'
                   ELSE 'DAILY_CONTROL: DISABLED' END
FROM public.owner_beta_daily_control WHERE id='etf11';
COMMIT;
