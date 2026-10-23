-- One explicit first-session transition. No grants, history deletion or epoch reset.
BEGIN;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '15s';
SET LOCAL idle_in_transaction_session_timeout = '20s';
SET LOCAL search_path = pg_catalog, pg_temp;
CREATE TEMP TABLE daily_first_session_input (
    apply boolean NOT NULL, old_config jsonb NOT NULL, new_config jsonb NOT NULL,
    policy_bytes text NOT NULL, capacity integer NOT NULL,
    lease_seconds integer NOT NULL, backoff_seconds bigint NOT NULL
) ON COMMIT DROP;
INSERT INTO pg_temp.daily_first_session_input VALUES (
    __APPLY__, convert_from(decode('__OLD_CONFIG_HEX__','hex'),'UTF8')::jsonb,
    convert_from(decode('__NEW_CONFIG_HEX__','hex'),'UTF8')::jsonb,
    convert_from(decode('__POLICY_HEX__','hex'),'UTF8'), __CAPACITY__, __LEASE__, __BACKOFF__
);
DO $first_session$
DECLARE
    i pg_temp.daily_first_session_input;
    c public.owner_beta_daily_control;
    previous_session date;
    next_session date;
    relation text;
    has_rows boolean;
    actor uuid;
    actor_count integer := 0;
    old_actor text := current_setting('app.actor_user_id',true);
BEGIN
    IF current_user <> 'migration_owner' OR session_user <> 'migration_owner' THEN
        RAISE EXCEPTION 'DAILY_FIRST_SESSION_ROLE' USING ERRCODE='42501';
    END IF;
    SELECT * INTO STRICT i FROM pg_temp.daily_first_session_input;
    previous_session := (i.old_config->>'first_auto_session')::date;
    next_session := (i.new_config->>'first_auto_session')::date;
    IF (i.old_config-ARRAY['code_commit','first_auto_session']) IS DISTINCT FROM
       (i.new_config-ARRAY['code_commit','first_auto_session'])
       OR i.old_config->>'code_commit' IS NOT DISTINCT FROM i.new_config->>'code_commit'
       OR COALESCE(i.old_config->>'code_commit','') !~ '^[0-9a-f]{40}$'
       OR COALESCE(i.new_config->>'code_commit','') !~ '^[0-9a-f]{40}$'
       OR i.old_config->>'code_commit'=repeat('0',40)
       OR i.new_config->>'code_commit'=repeat('0',40)
       OR i.new_config->>'schema' IS DISTINCT FROM 'owner-beta-daily-process-v1'
       OR i.new_config->>'policy_sha256' IS DISTINCT FROM public.daily_0060_digest(i.policy_bytes)
       OR previous_session IS NULL OR next_session IS NULL
       OR previous_session > (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date
       OR next_session <= (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date
       OR i.capacity <= 0 OR i.lease_seconds <= 0 OR i.backoff_seconds <= 0 THEN
        RAISE EXCEPTION 'DAILY_FIRST_SESSION_INPUT' USING ERRCODE='23514';
    END IF;
    PERFORM public.daily_0060_validate_policy(i.policy_bytes::jsonb);
    LOCK TABLE public.owner_beta_daily_control IN SHARE ROW EXCLUSIVE MODE;
    SELECT * INTO STRICT c FROM public.owner_beta_daily_control WHERE id='etf11' FOR UPDATE;
    IF c.enabled OR
       ROW(c.owner_user_id,c.policy_sha256,c.policy_json,c.entitlement_reference,
           c.entitlement_sha256,c.trigger_version,c.max_jobs_per_owner,c.lease_seconds,c.backoff_seconds)
       IS DISTINCT FROM ROW((i.new_config->>'owner')::uuid,i.new_config->>'policy_sha256',
           i.policy_bytes::jsonb,i.new_config->>'entitlement_reference',
           i.new_config->>'entitlement_sha256','owner-beta-daily-trigger-v1',
           i.capacity,i.lease_seconds,i.backoff_seconds)
       OR NOT ((c.first_auto_session=previous_session AND c.activation_epoch=0)
               OR (c.first_auto_session=next_session AND c.activation_epoch=1)) THEN
        RAISE EXCEPTION 'DAILY_FIRST_SESSION_BINDING' USING ERRCODE='23514';
    END IF;
    -- All daily durable history must be absent. Keep every original row intact.
    FOREACH relation IN ARRAY ARRAY[
        'owner_beta_daily_admissions','owner_beta_daily_admission_events',
        'owner_beta_daily_calendar_sources','owner_beta_daily_calendar_days',
        'owner_beta_daily_source_events','owner_beta_daily_poll_receipts',
        'owner_beta_daily_poll_state','owner_beta_daily_reservations',
        'owner_beta_daily_publication_claims'
    ] LOOP
        EXECUTE format('LOCK TABLE public.%I IN SHARE ROW EXCLUSIVE MODE',relation);
        EXECUTE format('SELECT EXISTS (SELECT 1 FROM public.%I)',relation) INTO has_rows;
        IF has_rows THEN
            RAISE EXCEPTION 'DAILY_FIRST_SESSION_HISTORY' USING ERRCODE='23514';
        END IF;
    END LOOP;
    -- Daily runs use actor-scoped RLS. A NULL actor is not proof of no history.
    IF pg_catalog.row_security_active('public.users'::regclass) THEN
        RAISE EXCEPTION 'DAILY_FIRST_SESSION_VISIBILITY' USING ERRCODE='23514';
    END IF;
    LOCK TABLE public.owner_beta_recommendation_runs IN SHARE ROW EXCLUSIVE MODE;
    FOR actor IN SELECT id FROM public.users ORDER BY id LIMIT 10001 LOOP
        actor_count := actor_count+1;
        IF actor_count>10000 THEN
            RAISE EXCEPTION 'DAILY_FIRST_SESSION_IDENTITY_LIMIT' USING ERRCODE='23514';
        END IF;
        PERFORM set_config('app.actor_user_id',actor::text,true);
        IF EXISTS (SELECT 1 FROM public.owner_beta_recommendation_runs
                   WHERE input_kind='owner_beta_daily_etf11_price_only_v1') THEN
            RAISE EXCEPTION 'DAILY_FIRST_SESSION_HISTORY' USING ERRCODE='23514';
        END IF;
    END LOOP;
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
    PERFORM public.daily_0060_authority(NULL,false);
    IF i.apply AND c.first_auto_session=previous_session THEN
        UPDATE public.owner_beta_daily_control SET first_auto_session=next_session WHERE id='etf11';
        -- Existing immutable trigger advances epoch 0 -> 1; never set it ourselves.
        IF NOT EXISTS (SELECT 1 FROM public.owner_beta_daily_control
                       WHERE id='etf11' AND NOT enabled AND activation_epoch=1
                         AND first_auto_session=next_session) THEN
            RAISE EXCEPTION 'DAILY_FIRST_SESSION_RESULT' USING ERRCODE='23514';
        END IF;
    END IF;
END $first_session$;
SELECT CASE apply WHEN true THEN 'DAILY_FIRST_SESSION: REBOUND_DISABLED'
                  ELSE 'DAILY_FIRST_SESSION: PREFLIGHT' END
FROM pg_temp.daily_first_session_input;
COMMIT;
