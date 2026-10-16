-- Fixed backup audit. No persistent row/schema mutation. TEMP probes invoke
-- the existing installed consistency trigger; ROLLBACK removes every probe.
-- postgres is allowed only for the caller's disposable isolated restore DB.
-- The live maintenance transport uses an actual migration_owner login.
BEGIN;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '120s';
SET LOCAL idle_in_transaction_session_timeout = '15s';
SET LOCAL search_path = pg_catalog, pg_temp;
SET LOCAL TimeZone = 'UTC';
SET LOCAL DateStyle = 'ISO, YMD';
SET LOCAL extra_float_digits = 3;
DO $role$
BEGIN
    IF current_user <> session_user OR current_user NOT IN ('migration_owner','postgres') THEN
        RAISE EXCEPTION 'DAILY_BACKUP_ROLE' USING ERRCODE='42501';
    END IF;
END $role$;
LOCK TABLE public.jobs, public.job_attempts,
    public.owner_beta_recommendation_runs, public.owner_beta_recommendation_items,
    public.owner_beta_daily_reservations, public.owner_beta_daily_publication_claims,
    public.owner_beta_daily_control, public.owner_beta_daily_admissions,
    public.owner_beta_daily_admission_events, public.owner_beta_daily_calendar_sources,
    public.owner_beta_daily_calendar_days, public.owner_beta_daily_source_events,
    public.owner_beta_daily_poll_receipts, public.owner_beta_daily_poll_state
    IN SHARE MODE;
CREATE TEMP TABLE daily_backup_probe(owner_user_id uuid NOT NULL, job_id uuid PRIMARY KEY)
    ON COMMIT DROP;
CREATE TRIGGER daily_backup_probe AFTER INSERT ON daily_backup_probe
    FOR EACH ROW EXECUTE FUNCTION public.daily_0060_consistency();
CREATE TEMP TABLE daily_backup_summary(name text PRIMARY KEY, row_count bigint NOT NULL, sha256 text NOT NULL)
    ON COMMIT DROP;
CREATE TEMP TABLE daily_backup_history(name text NOT NULL, sha256 text NOT NULL)
    ON COMMIT DROP;

DO $audit$
DECLARE
    control public.owner_beta_daily_control;
    actor uuid;
    run_row public.owner_beta_recommendation_runs;
    job_row public.jobs;
    actors integer := 0;
    runs integer := 0;
    table_name text;
    row_count bigint;
    old_actor text := current_setting('app.actor_user_id',true);
BEGIN
    IF (SELECT count(*) FROM public.owner_beta_daily_control) <> 1 THEN
        RAISE EXCEPTION 'DAILY_BACKUP_CONTROL' USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT control FROM public.owner_beta_daily_control WHERE id='etf11';
    IF control.enabled THEN
        RAISE EXCEPTION 'DAILY_BACKUP_ENABLED' USING ERRCODE='23514';
    END IF;
    -- users has RLS enabled but not forced (0010). Its actual table owner
    -- sees every identity; reject effective filtering for this caller, not
    -- the catalog flag that also protects normal serving-role reads.
    IF pg_catalog.row_security_active('public.users'::regclass) THEN
        RAISE EXCEPTION 'DAILY_BACKUP_IDENTITY_VISIBILITY' USING ERRCODE='23514';
    END IF;
    -- This literal identifier list is never populated from caller input. Hash
    -- each row first so a bounded aggregate does not concatenate full evidence.
    FOREACH table_name IN ARRAY ARRAY[
        'owner_beta_daily_control','owner_beta_daily_admissions',
        'owner_beta_daily_admission_events','owner_beta_daily_calendar_sources',
        'owner_beta_daily_calendar_days','owner_beta_daily_source_events',
        'owner_beta_daily_poll_receipts','owner_beta_daily_poll_state',
        'owner_beta_daily_reservations','owner_beta_daily_publication_claims'
    ] LOOP
        EXECUTE format('SELECT count(*) FROM (SELECT 1 FROM public.%I LIMIT 1000001) bounded',table_name)
            INTO row_count;
        IF row_count > 1000000 THEN
            RAISE EXCEPTION 'DAILY_BACKUP_LIMIT' USING ERRCODE='54000';
        END IF;
        IF table_name <> 'owner_beta_daily_poll_state' THEN
            EXECUTE format('SELECT count(*) FROM public.%I WHERE owner_user_id <> $1',table_name)
                INTO row_count USING control.owner_user_id;
            IF row_count <> 0 THEN
                RAISE EXCEPTION 'DAILY_BACKUP_OWNER' USING ERRCODE='23514';
            END IF;
        END IF;
        EXECUTE format(
            'INSERT INTO pg_temp.daily_backup_summary SELECT $1,count(*),'
            'public.daily_0060_digest(COALESCE(string_agg(h,chr(10) ORDER BY h),'''')) '
            'FROM (SELECT public.daily_0060_digest(to_jsonb(t)::text) h FROM public.%I t) hashes',
            table_name) USING table_name;
    END LOOP;
    IF (SELECT count(*) FROM public.owner_beta_daily_admissions) > 20000 THEN
        RAISE EXCEPTION 'DAILY_BACKUP_LIMIT' USING ERRCODE='54000';
    END IF;
    IF (SELECT count(*) FROM (
        SELECT key FROM (
            SELECT source_frontier_sha256 key FROM public.owner_beta_daily_poll_receipts
            WHERE stage='ADMISSION' AND result_code='VERIFIED' AND source_frontier_sha256 IS NOT NULL
            UNION SELECT evidence_json->>'source_frontier_sha256' FROM public.owner_beta_daily_admissions
        ) required_frontiers LIMIT 100001
    ) bounded_frontiers) > 100000 THEN
        RAISE EXCEPTION 'DAILY_BACKUP_LIMIT' USING ERRCODE='54000';
    END IF;
    FOR actor IN SELECT id FROM public.users ORDER BY id LIMIT 10001 LOOP
        actors := actors + 1;
        IF actors > 10000 THEN
            RAISE EXCEPTION 'DAILY_BACKUP_LIMIT' USING ERRCODE='54000';
        END IF;
        PERFORM set_config('app.actor_user_id',actor::text,true);
        FOR run_row IN
            SELECT * FROM public.owner_beta_recommendation_runs
            WHERE owner_user_id=actor AND
                  (input_kind='owner_beta_daily_etf11_price_only_v1' OR trigger_kind='DAILY')
            ORDER BY id LIMIT 100001
        LOOP
            runs := runs + 1;
            IF runs > 100000 THEN
                RAISE EXCEPTION 'DAILY_BACKUP_LIMIT' USING ERRCODE='54000';
            END IF;
            SELECT * INTO job_row FROM public.jobs WHERE id=run_row.job_id;
            IF NOT FOUND OR run_row.owner_user_id <> control.owner_user_id
               OR run_row.input_kind <> 'owner_beta_daily_etf11_price_only_v1'
               OR run_row.trigger_kind <> 'DAILY'
               OR job_row.owner_user_id IS DISTINCT FROM run_row.owner_user_id
               OR job_row.job_type <> 'owner_beta_price_recommendation'
               OR job_row.payload_json IS DISTINCT FROM public.daily_0060_payload(run_row)
               OR job_row.idempotency_key IS DISTINCT FROM
                  'owner-beta:daily:v1:' || run_row.owner_user_id::text || ':' ||
                  run_row.strategy_config_id::text || ':' || run_row.as_of::text
            THEN
                RAISE EXCEPTION 'DAILY_BACKUP_ORIGINAL_BINDING' USING ERRCODE='23514';
            END IF;
            INSERT INTO pg_temp.daily_backup_probe VALUES(actor,run_row.job_id);
            INSERT INTO pg_temp.daily_backup_history VALUES
                ('jobs',public.daily_0060_digest(to_jsonb(job_row)::text)),
                ('owner_beta_recommendation_runs',public.daily_0060_digest(to_jsonb(run_row)::text));
            INSERT INTO pg_temp.daily_backup_history
                SELECT 'job_attempts',public.daily_0060_digest(to_jsonb(a)::text)
                FROM public.job_attempts a WHERE job_id=run_row.job_id;
            INSERT INTO pg_temp.daily_backup_history
                SELECT 'owner_beta_recommendation_items',public.daily_0060_digest(to_jsonb(i)::text)
                FROM public.owner_beta_recommendation_items i WHERE recommendation_run_id=run_row.id;
        END LOOP;
        IF EXISTS (
            SELECT 1 FROM public.jobs j
            WHERE j.owner_user_id=actor AND
                  (j.payload_json->>'schema'='owner-beta-daily-recommendation-v1'
                   OR j.idempotency_key LIKE 'owner-beta:daily:v1:%')
              AND NOT EXISTS (SELECT 1 FROM pg_temp.daily_backup_probe p WHERE p.job_id=j.id)
        ) THEN
            RAISE EXCEPTION 'DAILY_BACKUP_ORPHAN_JOB' USING ERRCODE='23514';
        END IF;
    END LOOP;
    IF EXISTS (SELECT 1 FROM public.owner_beta_daily_reservations r
               WHERE NOT EXISTS (SELECT 1 FROM pg_temp.daily_backup_probe p WHERE p.job_id=r.job_id))
       OR EXISTS (SELECT 1 FROM public.owner_beta_daily_publication_claims c
                  WHERE NOT EXISTS (SELECT 1 FROM pg_temp.daily_backup_probe p WHERE p.job_id=c.job_id))
    THEN
        RAISE EXCEPTION 'DAILY_BACKUP_ORPHAN_HISTORY' USING ERRCODE='23514';
    END IF;
    INSERT INTO pg_temp.daily_backup_summary
        SELECT names.name,count(h.sha256),
               public.daily_0060_digest(COALESCE(string_agg(h.sha256,chr(10) ORDER BY h.sha256),''))
        FROM unnest(ARRAY['jobs','job_attempts','owner_beta_recommendation_runs',
                          'owner_beta_recommendation_items']) names(name)
        LEFT JOIN pg_temp.daily_backup_history h ON h.name=names.name
        GROUP BY names.name;
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
END $audit$;

-- Output is consumed in memory by the backup controller, never logged. This is
-- a bounded file-reference inventory, not a provider body or a secret export.
SELECT jsonb_build_object(
    'schema','lagrange-daily-backup-db-v1',
    'control',(SELECT jsonb_build_object(
        'owner_user_id',owner_user_id,'policy_sha256',policy_sha256,
        'activation_epoch',activation_epoch,'first_auto_session',first_auto_session,
        'trigger_version',trigger_version,'enabled',enabled)
        FROM public.owner_beta_daily_control WHERE id='etf11'),
    'tables',(SELECT jsonb_agg(to_jsonb(s) ORDER BY name) FROM pg_temp.daily_backup_summary s),
    'run_count',(SELECT count(*) FROM pg_temp.daily_backup_probe),
    'admissions',COALESCE((SELECT jsonb_agg(jsonb_build_object(
        'admission_sha256',admission_sha256,'content_sha256',content_sha256,
        'manifest_sha256',manifest_sha256,'parent_admission_sha256',parent_admission_sha256,
        'range_end',range_end,'source_frontier_sha256',evidence_json->>'source_frontier_sha256',
        'bars',evidence_json#>'{manifest,bars}',
        'normalization',evidence_json#>'{manifest,normalization}') ORDER BY range_end,admission_sha256)
        FROM public.owner_beta_daily_admissions),'[]'::jsonb),
    'verified_frontiers',COALESCE((SELECT jsonb_agg(key ORDER BY key) FROM (
        SELECT source_frontier_sha256 key FROM public.owner_beta_daily_poll_receipts
        WHERE stage='ADMISSION' AND result_code='VERIFIED' AND source_frontier_sha256 IS NOT NULL
        UNION SELECT evidence_json->>'source_frontier_sha256' FROM public.owner_beta_daily_admissions
    ) frontiers),'[]'::jsonb)
)::text AS daily_backup_result;
ROLLBACK;
