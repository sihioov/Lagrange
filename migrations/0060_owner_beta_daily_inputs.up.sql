-- R2 I2. The complete migration is transactional; no role or runtime is created.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

DO $preflight$
DECLARE r pg_catalog.pg_roles%ROWTYPE;
BEGIN
    SELECT * INTO r FROM pg_catalog.pg_roles WHERE rolname = 'daily_input_admitter';
    IF NOT FOUND OR NOT r.rolcanlogin OR r.rolsuper OR r.rolcreatedb
       OR r.rolcreaterole OR r.rolreplication OR r.rolbypassrls OR r.rolinherit
       OR NOT pg_catalog.has_database_privilege(r.oid, pg_catalog.current_database(), 'CONNECT')
       OR pg_catalog.has_database_privilege(r.oid, pg_catalog.current_database(), 'CREATE')
       OR NOT pg_catalog.has_schema_privilege(r.oid, 'public', 'USAGE')
       OR pg_catalog.has_schema_privilege(r.oid, 'public', 'CREATE')
       OR EXISTS (SELECT 1 FROM pg_catalog.pg_auth_members m
                  WHERE m.member = r.oid OR m.roleid = r.oid)
       OR EXISTS(WITH RECURSIVE paths(roleid) AS (
            SELECT oid FROM pg_catalog.pg_roles WHERE rolname IN ('app','worker','admin','research_writer','audit_writer')
            UNION SELECT m.roleid FROM pg_catalog.pg_auth_members m JOIN paths p ON m.member=p.roleid)
          SELECT 1 FROM paths p JOIN pg_catalog.pg_roles target ON target.oid=p.roleid
          WHERE target.rolname IN ('migration_owner','daily_input_admitter'))
    THEN
        RAISE EXCEPTION 'daily role preflight denied' USING ERRCODE = '42501';
    END IF;
END
$preflight$;

LOCK TABLE public.user_strategy_configs, public.owner_beta_recommendation_runs,
    public.owner_beta_recommendation_items, public.jobs, public.job_attempts
    IN ACCESS EXCLUSIVE MODE;

-- Private predicates deliberately return false for NULL, including JSON null.
CREATE FUNCTION public.daily_0060_keys(j jsonb, keys text[]) RETURNS boolean
LANGUAGE sql IMMUTABLE SET search_path = pg_catalog, pg_temp
AS $$ SELECT COALESCE(jsonb_typeof(j) = 'object' AND
    (SELECT array_agg(k ORDER BY k) FROM jsonb_object_keys(
        CASE WHEN jsonb_typeof(j) = 'object' THEN j ELSE '{}'::jsonb END) k)
    = (SELECT array_agg(k ORDER BY k) FROM unnest(keys) k), false) $$;

CREATE FUNCTION public.daily_0060_hash(j jsonb) RETURNS boolean
LANGUAGE sql IMMUTABLE SET search_path = pg_catalog, pg_temp
AS $$ SELECT COALESCE(jsonb_typeof(j) = 'string' AND j #>> '{}' ~ '^sha256:[0-9a-f]{64}$', false) $$;

CREATE FUNCTION public.daily_0060_uint(j jsonb, lo numeric, hi numeric) RETURNS boolean
LANGUAGE plpgsql IMMUTABLE SET search_path = pg_catalog, pg_temp
AS $$ BEGIN
    IF jsonb_typeof(j) IS DISTINCT FROM 'number' OR j::text !~ '^(0|[1-9][0-9]*)$' THEN
        RETURN false;
    END IF;
    RETURN (j::text)::numeric BETWEEN lo AND hi;
END $$;

CREATE FUNCTION public.daily_0060_date(j jsonb) RETURNS date
LANGUAGE plpgsql IMMUTABLE SET search_path = pg_catalog, pg_temp
AS $$ DECLARE d date; BEGIN
    IF jsonb_typeof(j) IS DISTINCT FROM 'string' OR j #>> '{}' !~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}$' THEN
        RAISE EXCEPTION 'daily date shape' USING ERRCODE = '23514';
    END IF;
    BEGIN d := (j #>> '{}')::date;
    EXCEPTION WHEN datetime_field_overflow OR invalid_datetime_format THEN
        RAISE EXCEPTION 'daily date shape' USING ERRCODE = '23514';
    END;
    IF to_char(d, 'YYYY-MM-DD') <> j #>> '{}' THEN
        RAISE EXCEPTION 'daily date shape' USING ERRCODE = '23514';
    END IF;
    RETURN d;
END $$;

CREATE FUNCTION public.daily_0060_digest(preimage text) RETURNS text
LANGUAGE sql IMMUTABLE STRICT SET search_path = pg_catalog, pg_temp
AS $$ SELECT 'sha256:' || encode(sha256(convert_to(preimage, 'UTF8')), 'hex') $$;

CREATE TABLE public.owner_beta_daily_control (
    id text PRIMARY KEY DEFAULT 'etf11' CHECK (id = 'etf11'),
    owner_user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE RESTRICT,
    enabled boolean NOT NULL DEFAULT false,
    activation_epoch bigint NOT NULL DEFAULT 0 CHECK (activation_epoch >= 0),
    policy_sha256 text NOT NULL CHECK (policy_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    policy_json jsonb NOT NULL CHECK (jsonb_typeof(policy_json) = 'object'),
    entitlement_reference text NOT NULL CHECK (length(entitlement_reference) BETWEEN 1 AND 512 AND entitlement_reference !~ '[[:cntrl:]]'),
    entitlement_sha256 text NOT NULL CHECK (entitlement_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    first_auto_session date NOT NULL CHECK (first_auto_session > '2026-08-28'),
    trigger_version text NOT NULL DEFAULT 'owner-beta-daily-trigger-v1' CHECK (trigger_version = 'owner-beta-daily-trigger-v1'),
    max_jobs_per_owner integer NOT NULL CHECK (max_jobs_per_owner > 0),
    lease_seconds integer NOT NULL CHECK (lease_seconds > 0),
    backoff_seconds bigint NOT NULL CHECK (backoff_seconds > 0),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

CREATE TABLE public.owner_beta_daily_admissions (
    admission_sha256 text PRIMARY KEY CHECK (admission_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    owner_user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE RESTRICT,
    policy_sha256 text NOT NULL CHECK (policy_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    content_sha256 text NOT NULL CHECK (content_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    manifest_sha256 text NOT NULL CHECK (manifest_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    parent_admission_sha256 text REFERENCES public.owner_beta_daily_admissions(admission_sha256) ON DELETE RESTRICT,
    range_start date NOT NULL CHECK (range_start = '2016-08-29'),
    range_end date NOT NULL CHECK (range_end > '2026-08-28'),
    session_count integer NOT NULL CHECK (session_count BETWEEN 2453 AND 20000),
    bar_count integer NOT NULL CHECK (bar_count = 11 * session_count),
    evidence_json jsonb NOT NULL CHECK (jsonb_typeof(evidence_json) = 'object'),
    admitted_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(owner_user_id, range_end)
);

CREATE TABLE public.owner_beta_daily_admission_events (
    event_sha256 text PRIMARY KEY CHECK (event_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    admission_sha256 text NOT NULL REFERENCES public.owner_beta_daily_admissions(admission_sha256) ON DELETE RESTRICT,
    owner_user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE RESTRICT,
    kind text NOT NULL CHECK (kind IN ('CONFLICT','REVOKED')),
    evidence_sha256 text NOT NULL CHECK (evidence_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

CREATE TABLE public.owner_beta_daily_calendar_sources (
    calendar_evidence_sha256 text PRIMARY KEY CHECK (calendar_evidence_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    owner_user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE RESTRICT,
    policy_sha256 text NOT NULL CHECK (policy_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    normalizer_version text NOT NULL CHECK (normalizer_version = 'daily-raw-reconstruction-v1'),
    source_provider text NOT NULL CHECK (source_provider IN ('kis','kis-calendar')),
    source_market text NOT NULL CHECK (source_market = 'kr'),
    source_batch_id uuid NOT NULL,
    source_batch_date date NOT NULL,
    requested_target date NOT NULL CHECK (requested_target = source_batch_date),
    source_file_name text NOT NULL,
    source_file_sha256 text NOT NULL CHECK (source_file_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    source_manifest_line_sha256 text NOT NULL CHECK (source_manifest_line_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    source_retrieved_at timestamptz NOT NULL,
    copy_chain jsonb NOT NULL CHECK (jsonb_typeof(copy_chain) = 'array'),
    evidence_json jsonb NOT NULL CHECK (jsonb_typeof(evidence_json) = 'object'),
    projected_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(owner_user_id, calendar_evidence_sha256)
);

CREATE TABLE public.owner_beta_daily_calendar_days (
    owner_user_id uuid NOT NULL,
    calendar_evidence_sha256 text NOT NULL,
    session_date date NOT NULL,
    session_type text NOT NULL CHECK (session_type IN ('TRADING','CLOSED')),
    evidence_kind text NOT NULL CHECK (evidence_kind IN ('REQUESTED_TARGET','DERIVED_SURROUNDING')),
    source_row_ordinal integer NOT NULL CHECK (source_row_ordinal >= 0),
    source_row_sha256 text NOT NULL CHECK (source_row_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(owner_user_id, calendar_evidence_sha256, session_date),
    FOREIGN KEY(owner_user_id, calendar_evidence_sha256) REFERENCES
        public.owner_beta_daily_calendar_sources(owner_user_id, calendar_evidence_sha256) ON DELETE RESTRICT
);

CREATE TABLE public.owner_beta_daily_source_events (
    event_sha256 text PRIMARY KEY CHECK (event_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    owner_user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE RESTRICT,
    source_kind text NOT NULL CHECK (source_kind IN ('ACTION','CALENDAR','BAR','REFERENCE')),
    range_start date NOT NULL,
    range_end date NOT NULL CHECK (range_end >= range_start),
    code text NOT NULL CHECK (code IN ('SOURCE_CONFLICT','SEALED_SOURCE_CHANGED','SOURCE_REVOKED','SOURCE_INTEGRITY','CALENDAR_CONFLICT','NONCASH_ACTION')),
    evidence_sha256 text NOT NULL CHECK (evidence_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    admission_sha256 text REFERENCES public.owner_beta_daily_admissions(admission_sha256) ON DELETE RESTRICT,
    recorded_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

CREATE TABLE public.owner_beta_daily_poll_receipts (
    receipt_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    stage text NOT NULL CHECK (stage IN ('ADMISSION','SCHEDULE')),
    owner_user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE RESTRICT,
    policy_sha256 text NOT NULL CHECK (policy_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    activation_epoch bigint NOT NULL CHECK (activation_epoch >= 0),
    started_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    completed_at timestamptz,
    result_code text CHECK (result_code IN ('VERIFIED','INPUT_PENDING','INPUT_BLOCKED','IO_FAILED','CHECKED','DEFERRED','BLOCKED')),
    source_frontier_sha256 text CHECK (source_frontier_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    admission_sha256 text REFERENCES public.owner_beta_daily_admissions(admission_sha256) ON DELETE RESTRICT,
    event_sha256 text REFERENCES public.owner_beta_daily_source_events(event_sha256) ON DELETE RESTRICT,
    -- SQL operation identity only; this is never a D4 canonical artifact hash.
    completion_binding_sha256 text CHECK (completion_binding_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    CHECK ((completed_at IS NULL) = (result_code IS NULL)),
    CHECK (completed_at IS NULL OR completed_at >= started_at)
);

CREATE TABLE public.owner_beta_daily_poll_state (
    stage text PRIMARY KEY CHECK (stage IN ('ADMISSION','SCHEDULE')),
    next_poll_at timestamptz NOT NULL,
    last_completed_at timestamptz,
    last_result_code text,
    verified_admission_sha256 text REFERENCES public.owner_beta_daily_admissions(admission_sha256) ON DELETE RESTRICT,
    source_frontier_sha256 text CHECK (source_frontier_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    latest_receipt_id uuid REFERENCES public.owner_beta_daily_poll_receipts(receipt_id) ON DELETE RESTRICT
);

CREATE TABLE public.owner_beta_daily_reservations (
    owner_user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE RESTRICT,
    strategy_config_id uuid NOT NULL REFERENCES public.user_strategy_configs(id) ON DELETE RESTRICT,
    session_date date NOT NULL,
    trigger_version text NOT NULL CHECK (trigger_version = 'owner-beta-daily-trigger-v1'),
    run_id uuid NOT NULL UNIQUE REFERENCES public.owner_beta_recommendation_runs(id) ON DELETE RESTRICT,
    job_id uuid NOT NULL UNIQUE REFERENCES public.jobs(id) ON DELETE RESTRICT,
    admission_sha256 text NOT NULL REFERENCES public.owner_beta_daily_admissions(admission_sha256) ON DELETE RESTRICT,
    strategy_config_sha256 text NOT NULL CHECK (strategy_config_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    config_epoch bigint NOT NULL CHECK (config_epoch >= 0),
    activation_epoch bigint NOT NULL CHECK (activation_epoch >= 0),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY(owner_user_id, strategy_config_id, session_date, trigger_version)
);

CREATE TABLE public.owner_beta_daily_publication_claims (
    run_id uuid PRIMARY KEY REFERENCES public.owner_beta_recommendation_runs(id) ON DELETE RESTRICT,
    job_id uuid NOT NULL UNIQUE REFERENCES public.jobs(id) ON DELETE RESTRICT,
    owner_user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE RESTRICT,
    attempt_id uuid NOT NULL UNIQUE REFERENCES public.job_attempts(id) ON DELETE RESTRICT,
    attempt_no integer NOT NULL CHECK (attempt_no > 0),
    claimed_by text NOT NULL CHECK (claimed_by <> ''),
    locked_at timestamptz NOT NULL,
    lease_seconds integer NOT NULL CHECK (lease_seconds > 0),
    authorized_at timestamptz NOT NULL
);

ALTER TABLE public.user_strategy_configs
    ADD COLUMN daily_enabled boolean NOT NULL DEFAULT false,
    ADD COLUMN daily_epoch bigint NOT NULL DEFAULT 0 CHECK (daily_epoch >= 0);
REVOKE INSERT, UPDATE ON public.user_strategy_configs FROM app;
GRANT INSERT(owner_user_id,strategy_id,strategy_version,config_json,is_active,daily_enabled)
    ON public.user_strategy_configs TO app;
GRANT UPDATE(config_json,is_active,daily_enabled,updated_at)
    ON public.user_strategy_configs TO app;

CREATE FUNCTION public.daily_0060_config_epoch() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
SET lock_timeout = '5s' SET statement_timeout = '30s'
AS $$ BEGIN
    IF TG_OP = 'INSERT' THEN
        IF NEW.daily_epoch <> 0 THEN RAISE EXCEPTION 'daily epoch is private' USING ERRCODE='42501'; END IF;
    ELSE
        IF ROW(NEW.id,NEW.owner_user_id,NEW.created_at,NEW.strategy_id,NEW.strategy_version,NEW.daily_epoch)
           IS DISTINCT FROM ROW(OLD.id,OLD.owner_user_id,OLD.created_at,OLD.strategy_id,OLD.strategy_version,OLD.daily_epoch)
        THEN RAISE EXCEPTION 'daily config identity is immutable' USING ERRCODE='42501'; END IF;
        IF ROW(NEW.config_json,NEW.is_active,NEW.daily_enabled)
           IS DISTINCT FROM ROW(OLD.config_json,OLD.is_active,OLD.daily_enabled) THEN
            IF OLD.daily_epoch = 9223372036854775807 THEN RAISE EXCEPTION 'daily epoch overflow' USING ERRCODE='23514'; END IF;
            NEW.daily_epoch := OLD.daily_epoch + 1;
        END IF;
    END IF;
    IF NEW.daily_enabled THEN
        IF NEW.owner_user_id::text IS DISTINCT FROM NULLIF(current_setting('app.actor_user_id',true),'') THEN
            RAISE EXCEPTION 'daily opt in actor denied' USING ERRCODE='42501';
        END IF;
        PERFORM 1 FROM public.users u WHERE u.id=NEW.owner_user_id FOR SHARE;
        IF NOT FOUND THEN RAISE EXCEPTION 'daily owner denied' USING ERRCODE='42501'; END IF;
        PERFORM 1 FROM public.user_roles ur WHERE ur.user_id=NEW.owner_user_id AND ur.role_id='owner' FOR SHARE;
        IF NOT FOUND THEN RAISE EXCEPTION 'daily owner denied' USING ERRCODE='42501'; END IF;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER daily_0060_config_epoch BEFORE INSERT OR UPDATE ON public.user_strategy_configs
    FOR EACH ROW EXECUTE FUNCTION public.daily_0060_config_epoch();

CREATE FUNCTION public.daily_0060_control_epoch() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp
AS $$ BEGIN
    IF TG_OP = 'DELETE' THEN RAISE EXCEPTION 'daily control is permanent' USING ERRCODE='42501'; END IF;
    IF TG_OP = 'INSERT' THEN
        IF NEW.enabled OR NEW.activation_epoch <> 0 THEN RAISE EXCEPTION 'daily control starts disabled' USING ERRCODE='23514'; END IF;
    ELSE
        IF ROW(NEW.id,NEW.owner_user_id,NEW.activation_epoch) IS DISTINCT FROM ROW(OLD.id,OLD.owner_user_id,OLD.activation_epoch) THEN
            RAISE EXCEPTION 'daily control identity is immutable' USING ERRCODE='42501';
        END IF;
        IF (to_jsonb(NEW)-'updated_at') IS DISTINCT FROM (to_jsonb(OLD)-'updated_at') THEN
            IF OLD.activation_epoch=9223372036854775807 THEN RAISE EXCEPTION 'daily epoch overflow' USING ERRCODE='23514'; END IF;
            NEW.activation_epoch := OLD.activation_epoch+1;
        END IF;
    END IF;
    PERFORM public.daily_0060_validate_policy(NEW.policy_json);
    NEW.updated_at := clock_timestamp();
    RETURN NEW;
END $$;
CREATE TRIGGER daily_0060_control_epoch BEFORE INSERT OR UPDATE OR DELETE ON public.owner_beta_daily_control
    FOR EACH ROW EXECUTE FUNCTION public.daily_0060_control_epoch();

CREATE FUNCTION public.daily_0060_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp
AS $$ BEGIN RAISE EXCEPTION 'daily evidence is immutable' USING ERRCODE='42501'; END $$;

-- All ACL/RLS and helper EXECUTE revocations are completed at the end of UP.

CREATE FUNCTION public.daily_0060_validate_policy(j jsonb) RETURNS void
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp
AS $$ DECLARE v jsonb; BEGIN
    IF NOT public.daily_0060_keys(j,ARRAY['schema','capability','audience','vendor_snapshot','strict_pit','base','instruments','price_semantics','cash_action_policy','normalizer_version','materializer_version','source_kinds','limits'])
       OR (j - ARRAY['base','limits','source_kinds']) IS DISTINCT FROM
        '{"schema":"owner-beta-daily-etf11-policy-v1","capability":"PRICE_RETURN_ONLY","audience":"OWNER_ONLY","vendor_snapshot":true,"strict_pit":false,"instruments":["069500.KRX","102110.KRX","114260.KRX","132030.KRX","133690.KRX","143850.KRX","148070.KRX","153130.KRX","192090.KRX","195930.KRX","229200.KRX"],"price_semantics":"KIS_FID_ORG_ADJ_PRC_1_RAW_OHLC","cash_action_policy":"CASH_DIVIDENDS_EXCLUDED_NONCASH_BLOCKED_V1","normalizer_version":"daily-raw-reconstruction-v1","materializer_version":"daily-price-only-v1"}'::jsonb
       OR NOT public.daily_0060_keys(j->'base',ARRAY['kind','pins'])
       OR j#>>'{base,kind}' IS DISTINCT FROM 'historical_price_only_v3'
       OR NOT public.daily_0060_keys(j#>'{base,pins}',ARRAY['candidate_content_sha256','artifact_manifest_sha256','stage5_manifest_sha256','action_manifest_sha256','approval_registry_sha256'])
       OR j->'limits' IS DISTINCT FROM '{"max_policy_bytes":262144,"max_manifest_bytes":16777216,"max_admission_bytes":1048576,"max_normalization_bytes":16777216,"max_bars_bytes":134217728,"max_sessions":20000,"max_civil_dates_per_addition":370,"max_action_chains_per_session":32768,"max_file_evidence_per_session":262144,"max_source_file_bytes":67108864,"max_normalized_document_bytes":67108864,"max_copy_depth":16,"max_pages_per_action_chain":10,"max_cash_rows_per_chain":100000,"max_json_depth":32}'::jsonb
       OR j->'source_kinds' IS DISTINCT FROM '[{"provider":"kis","market":"kr","kind":"bars"},{"provider":"kis","market":"kr","kind":"calendar"},{"provider":"kis","market":"kr","kind":"corporate_actions"},{"provider":"kis","market":"kr","kind":"reference"},{"provider":"kis-calendar","market":"kr","kind":"calendar"}]'::jsonb
    THEN RAISE EXCEPTION 'daily policy shape' USING ERRCODE='23514'; END IF;
    FOR v IN SELECT value FROM jsonb_each(j#>'{base,pins}') LOOP
        IF NOT public.daily_0060_hash(v) THEN RAISE EXCEPTION 'daily policy pin' USING ERRCODE='23514'; END IF;
    END LOOP;
END $$;

-- A verified FileEvidence is a claim from the narrow admitter, not file IO.
-- Validate every hop and the exact redacted request before storing any JSON.
CREATE FUNCTION public.daily_0060_file(j jsonb, kind text) RETURNS jsonb
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp
AS $$
DECLARE o jsonb; prev jsonb; terminal jsonb; req jsonb; q jsonb; h jsonb; pair jsonb;
    seen text[] := '{}'; key text; prior_key text; stamp timestamptz; previous_stamp timestamptz := clock_timestamp();
    origin_keys text[] := ARRAY['provider','market','batch_id','batch_date','batch_json_sha256','manifest_line_sha256','file_name','kind','wire_sha256','size_bytes','request','response_continuation','retrieved_at','retrieval_precision'];
BEGIN
    IF NOT public.daily_0060_keys(j,origin_keys || ARRAY['original_retrieved_at','original_retrieval_precision','copied_from'])
       OR jsonb_typeof(j->'copied_from') IS DISTINCT FROM 'array'
       OR j->>'original_retrieval_precision' IS DISTINCT FROM 'batch'
    THEN RAISE EXCEPTION 'daily file shape' USING ERRCODE='23514'; END IF;
    IF jsonb_array_length(j->'copied_from') > 16 THEN RAISE EXCEPTION 'daily copy limit' USING ERRCODE='23514'; END IF;
    FOR o IN SELECT value FROM jsonb_array_elements(jsonb_build_array(j-ARRAY['original_retrieved_at','original_retrieval_precision','copied_from']) || (j->'copied_from')) LOOP
        IF NOT public.daily_0060_keys(o,origin_keys)
           OR o->>'provider' IS NULL OR o->>'provider' NOT IN ('kis','kis-calendar') OR o->>'market' IS DISTINCT FROM 'kr'
           OR o->>'kind' IS DISTINCT FROM kind OR (o->>'provider'='kis-calendar' AND kind<>'calendar')
           OR jsonb_typeof(o->'batch_id') IS DISTINCT FROM 'string'
           OR o->>'batch_id' !~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
           OR jsonb_typeof(o->'file_name') IS DISTINCT FROM 'string'
           OR o->>'file_name' !~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$' OR position('..' IN o->>'file_name')>0
           OR NOT public.daily_0060_hash(o->'batch_json_sha256') OR NOT public.daily_0060_hash(o->'manifest_line_sha256')
           OR NOT public.daily_0060_hash(o->'wire_sha256') OR NOT public.daily_0060_uint(o->'size_bytes',1,67108864)
           OR o->>'retrieval_precision' IS DISTINCT FROM 'batch'
           OR jsonb_typeof(o->'retrieved_at') IS DISTINCT FROM 'string'
           OR o->>'retrieved_at' !~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-5][0-9]\.[0-9]{9}Z$'
           OR NOT (o->'response_continuation' = 'null'::jsonb OR
                 (jsonb_typeof(o->'response_continuation')='string' AND o->>'response_continuation' ~ '^[A-Za-z0-9]{0,8}$'))
        THEN RAISE EXCEPTION 'daily origin shape' USING ERRCODE='23514'; END IF;
        PERFORM public.daily_0060_date(o->'batch_date');
        stamp := (o->>'retrieved_at')::timestamptz;
        key := (o->>'provider')||'/'||(o->>'market')||'/'||(o->>'batch_id')||'/'||(o->>'file_name');
        IF stamp>previous_stamp OR key=ANY(seen) OR (prev IS NOT NULL AND
            ROW(o->'kind',o->'wire_sha256',o->'size_bytes',o->'request',o->'response_continuation') IS DISTINCT FROM
            ROW(prev->'kind',prev->'wire_sha256',prev->'size_bytes',prev->'request',prev->'response_continuation'))
        THEN RAISE EXCEPTION 'daily copy lineage' USING ERRCODE='23514'; END IF;
        seen := array_append(seen,key); previous_stamp:=stamp; prev:=o; terminal:=o;
        req:=o->'request';
        IF NOT public.daily_0060_keys(req,ARRAY['endpoint','query','headers','mode'])
           OR req->>'mode' IS DISTINCT FROM 'credentialed'
           OR jsonb_typeof(req->'query') IS DISTINCT FROM 'array' OR jsonb_typeof(req->'headers') IS DISTINCT FROM 'array'
        THEN RAISE EXCEPTION 'daily request shape' USING ERRCODE='23514'; END IF;
        q:='{}'; h:='{}'; prior_key:=NULL;
        FOR pair IN SELECT value FROM jsonb_array_elements(req->'query') LOOP
            IF jsonb_typeof(pair) IS DISTINCT FROM 'array' OR jsonb_array_length(pair)<>2
               OR jsonb_typeof(pair->0) IS DISTINCT FROM 'string' OR jsonb_typeof(pair->1) IS DISTINCT FROM 'string'
               OR q ? (pair->>0) OR (prior_key IS NOT NULL AND prior_key COLLATE "C">=(pair->>0) COLLATE "C") THEN RAISE EXCEPTION 'daily query shape' USING ERRCODE='23514'; END IF;
            q:=q||jsonb_build_object(pair->>0,pair->1); prior_key:=pair->>0;
        END LOOP;
        prior_key:=NULL;
        FOR pair IN SELECT value FROM jsonb_array_elements(req->'headers') LOOP
            IF jsonb_typeof(pair) IS DISTINCT FROM 'array' OR jsonb_array_length(pair)<>2
               OR jsonb_typeof(pair->0) IS DISTINCT FROM 'string' OR jsonb_typeof(pair->1) IS DISTINCT FROM 'string'
               OR h ? (pair->>0) OR (prior_key IS NOT NULL AND prior_key COLLATE "C">=(pair->>0) COLLATE "C") THEN RAISE EXCEPTION 'daily header shape' USING ERRCODE='23514'; END IF;
            h:=h||jsonb_build_object(pair->>0,pair->1); prior_key:=pair->>0;
        END LOOP;
        IF NOT public.daily_0060_keys(h,ARRAY['appkey','appsecret','authorization','tr_cont','tr_id'])
           OR (h-ARRAY['tr_cont','tr_id']) <> '{"appkey":"[REDACTED]","appsecret":"[REDACTED]","authorization":"[REDACTED]"}'::jsonb
        THEN RAISE EXCEPTION 'daily credentials forbidden' USING ERRCODE='23514'; END IF;
        IF kind='calendar' THEN
            IF req->>'endpoint' IS DISTINCT FROM '/uapi/domestic-stock/v1/quotations/chk-holiday'
               OR h->>'tr_id' IS DISTINCT FROM 'CTCA0903R' OR h->>'tr_cont' IS DISTINCT FROM ''
               OR NOT public.daily_0060_keys(q,ARRAY['BASS_DT','CTX_AREA_FK','CTX_AREA_NK'])
               OR q->>'BASS_DT' !~ '^[0-9]{8}$' OR q->>'CTX_AREA_FK' IS DISTINCT FROM '' OR q->>'CTX_AREA_NK' IS DISTINCT FROM ''
            THEN RAISE EXCEPTION 'daily calendar request' USING ERRCODE='23514'; END IF;
        ELSIF kind IN ('bars','reference') THEN
            IF h->>'tr_cont' IS DISTINCT FROM '' OR q->>'FID_COND_MRKT_DIV_CODE' IS DISTINCT FROM 'J'
               OR q->>'FID_INPUT_ISCD' NOT IN ('069500','102110','114260','132030','133690','143850','148070','153130','192090','195930','229200')
            THEN RAISE EXCEPTION 'daily price request' USING ERRCODE='23514'; END IF;
            IF kind='bars' THEN
                IF req->>'endpoint' IS DISTINCT FROM '/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice'
                   OR h->>'tr_id' IS DISTINCT FROM 'FHKST03010100'
                   OR NOT public.daily_0060_keys(q,ARRAY['FID_COND_MRKT_DIV_CODE','FID_INPUT_ISCD','FID_INPUT_DATE_1','FID_INPUT_DATE_2','FID_PERIOD_DIV_CODE','FID_ORG_ADJ_PRC'])
                   OR q->>'FID_INPUT_DATE_1' !~ '^[0-9]{8}$' OR q->'FID_INPUT_DATE_1' IS DISTINCT FROM q->'FID_INPUT_DATE_2'
                   OR q->>'FID_PERIOD_DIV_CODE' IS DISTINCT FROM 'D' OR q->>'FID_ORG_ADJ_PRC' IS DISTINCT FROM '1'
                THEN RAISE EXCEPTION 'daily bars request' USING ERRCODE='23514'; END IF;
            ELSE
                IF req->>'endpoint' IS DISTINCT FROM '/uapi/domestic-stock/v1/quotations/inquire-price'
                   OR h->>'tr_id' IS DISTINCT FROM 'FHKST01010100'
                   OR NOT public.daily_0060_keys(q,ARRAY['FID_COND_MRKT_DIV_CODE','FID_INPUT_ISCD'])
                THEN RAISE EXCEPTION 'daily reference request' USING ERRCODE='23514'; END IF;
            END IF;
        ELSE
            IF kind<>'corporate_actions' OR h->>'tr_cont' NOT IN ('','N')
               OR q->>'CTS' IS DISTINCT FROM '' OR q->>'F_DT' !~ '^[0-9]{8}$' OR q->>'T_DT' !~ '^[0-9]{8}$'
               OR q->>'SHT_CD' !~ '^([0-9]{6})?$'
               OR NOT EXISTS (SELECT 1 FROM (VALUES
                    ('paidin-capin','HHKDB669100C0','{"GB1":"1"}'::jsonb),('paidin-capin','HHKDB669100C0','{"GB1":"2"}'::jsonb),
                    ('bonus-issue','HHKDB669101C0','{}'::jsonb),('dividend','HHKDB669102C0','{"GB1":"0","HIGH_GB":""}'::jsonb),
                    ('merger-split','HHKDB669104C0','{}'::jsonb),('rev-split','HHKDB669105C0','{"MARKET_GB":"0"}'::jsonb),
                    ('cap-dcrs','HHKDB669106C0','{}'::jsonb)) allowed(path,tr,extras)
                 WHERE req->>'endpoint'='/uapi/domestic-stock/v1/ksdinfo/'||path AND h->>'tr_id'=tr
                   AND q-ARRAY['CTS','F_DT','T_DT','SHT_CD']=extras
                   AND q ?& ARRAY['CTS','F_DT','T_DT','SHT_CD'])
            THEN RAISE EXCEPTION 'daily action request' USING ERRCODE='23514'; END IF;
        END IF;
    END LOOP;
    IF j->'original_retrieved_at' IS DISTINCT FROM terminal->'retrieved_at'
       OR (kind='calendar' AND q->>'BASS_DT' <> replace(terminal->>'batch_date','-',''))
       OR (kind='bars' AND q->>'FID_INPUT_DATE_1' <> replace(terminal->>'batch_date','-',''))
    THEN RAISE EXCEPTION 'daily original binding' USING ERRCODE='23514'; END IF;
    RETURN terminal;
EXCEPTION WHEN invalid_parameter_value OR invalid_text_representation OR datetime_field_overflow OR invalid_datetime_format OR numeric_value_out_of_range THEN
    RAISE EXCEPTION 'daily file scalar shape' USING ERRCODE='23514';
END $$;

CREATE FUNCTION public.daily_0060_authority(p_end date, must_enable boolean)
RETURNS public.owner_beta_daily_control
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE c public.owner_beta_daily_control; n integer; today date;
BEGIN
    SELECT * INTO c FROM public.owner_beta_daily_control WHERE id='etf11' FOR SHARE;
    IF NOT FOUND OR (must_enable AND NOT c.enabled) THEN RAISE EXCEPTION 'daily control unavailable' USING ERRCODE='23514'; END IF;
    PERFORM 1 FROM public.users WHERE id=c.owner_user_id FOR SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'daily owner unavailable' USING ERRCODE='42501'; END IF;
    PERFORM 1 FROM public.user_roles WHERE user_id=c.owner_user_id AND role_id='owner' FOR SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'daily owner unavailable' USING ERRCODE='42501'; END IF;
    -- Lock every exact witness before evaluating time. No unrelated ACTIVE row suffices.
    PERFORM 1 FROM public.data_entitlements e
      WHERE e.contract_reference=c.entitlement_reference
        AND e.contract_document_sha256=substring(c.entitlement_sha256,8)
      ORDER BY e.id FOR SHARE;
    today:=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date;
    SELECT count(*) INTO n FROM public.data_entitlements e
      WHERE e.contract_reference=c.entitlement_reference
        AND e.contract_document_sha256=substring(c.entitlement_sha256,8)
        AND e.status='ACTIVE' AND e.effective_from<=LEAST(date '2016-08-29',today)
        AND e.effective_until>=GREATEST(COALESCE(p_end,today),today)
        AND e.covered_datasets @> '["krx_eod_bars"]'::jsonb
        AND e.covered_uses @> '["recommendation"]'::jsonb;
    IF n=0 THEN RAISE EXCEPTION 'daily rights unavailable' USING ERRCODE='42501'; END IF;
    RETURN c;
END $$;

CREATE FUNCTION public.daily_0060_source_lock(owner_id uuid, exclusive_lock boolean) RETURNS void
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp
AS $$ BEGIN
    IF exclusive_lock THEN PERFORM pg_advisory_xact_lock(hashtextextended(owner_id::text||':daily-source',7920));
    ELSE PERFORM pg_advisory_xact_lock_shared(hashtextextended(owner_id::text||':daily-source',7920)); END IF;
END $$;

CREATE FUNCTION public.daily_0060_receipt(p_id uuid, p_stage text, c public.owner_beta_daily_control)
RETURNS public.owner_beta_daily_poll_receipts
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp
AS $$ DECLARE r public.owner_beta_daily_poll_receipts; latest uuid; BEGIN
    SELECT * INTO r FROM public.owner_beta_daily_poll_receipts WHERE receipt_id=p_id FOR UPDATE;
    IF NOT FOUND OR ROW(r.stage,r.owner_user_id,r.policy_sha256,r.activation_epoch)
        IS DISTINCT FROM ROW(p_stage,c.owner_user_id,c.policy_sha256,c.activation_epoch)
    THEN RAISE EXCEPTION 'daily receipt authority' USING ERRCODE='23514'; END IF;
    SELECT latest_receipt_id INTO latest FROM public.owner_beta_daily_poll_state WHERE stage=p_stage FOR UPDATE;
    IF r.completed_at IS NULL AND latest IS DISTINCT FROM p_id THEN
        RAISE EXCEPTION 'daily receipt superseded' USING ERRCODE='23514';
    END IF;
    RETURN r;
END $$;

CREATE FUNCTION public.daily_0060_complete(p_id uuid, result text, frontier text, admitted text, event_id text, binding text DEFAULT NULL)
RETURNS text LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp
AS $$ DECLARE r public.owner_beta_daily_poll_receipts; t timestamptz; BEGIN
    SELECT * INTO STRICT r FROM public.owner_beta_daily_poll_receipts WHERE receipt_id=p_id FOR UPDATE;
    IF r.completed_at IS NOT NULL THEN
        IF ROW(r.result_code,r.source_frontier_sha256,r.admission_sha256,r.event_sha256,r.completion_binding_sha256)
           IS DISTINCT FROM ROW(result,frontier,admitted,event_id,binding) THEN
            RAISE EXCEPTION 'daily completion mismatch' USING ERRCODE='23514';
        END IF;
        RETURN 'REPLAY';
    END IF;
    PERFORM 1 FROM public.owner_beta_daily_poll_state WHERE stage=r.stage FOR UPDATE;
    t:=clock_timestamp();
    UPDATE public.owner_beta_daily_poll_state SET last_completed_at=t,last_result_code=result,
        verified_admission_sha256=admitted,source_frontier_sha256=frontier
      WHERE stage=r.stage AND latest_receipt_id=p_id;
    IF NOT FOUND THEN RAISE EXCEPTION 'daily receipt superseded' USING ERRCODE='23514'; END IF;
    UPDATE public.owner_beta_daily_poll_receipts SET completed_at=t,result_code=result,
        source_frontier_sha256=frontier,admission_sha256=admitted,event_sha256=event_id,completion_binding_sha256=binding WHERE receipt_id=p_id;
    RETURN 'COMPLETED';
END $$;

CREATE FUNCTION public.daily_0060_take_slot(p_stage text) RETURNS TABLE(receipt_id uuid)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE c public.owner_beta_daily_control; t timestamptz; rid uuid; BEGIN
    IF NOT EXISTS (SELECT 1 FROM public.owner_beta_daily_control WHERE id='etf11' AND (p_stage='ADMISSION' OR enabled)) THEN RETURN; END IF;
    c:=public.daily_0060_authority(NULL,p_stage='SCHEDULE');
    PERFORM pg_advisory_xact_lock(hashtextextended('daily-poll:'||p_stage,7921));
    t:=clock_timestamp();
    INSERT INTO public.owner_beta_daily_poll_state(stage,next_poll_at) VALUES(p_stage,t) ON CONFLICT DO NOTHING;
    PERFORM 1 FROM public.owner_beta_daily_poll_state WHERE stage=p_stage AND next_poll_at<=t FOR UPDATE;
    IF NOT FOUND THEN RETURN; END IF;
    INSERT INTO public.owner_beta_daily_poll_receipts(stage,owner_user_id,policy_sha256,activation_epoch,started_at)
      VALUES(p_stage,c.owner_user_id,c.policy_sha256,c.activation_epoch,t) RETURNING owner_beta_daily_poll_receipts.receipt_id INTO rid;
    UPDATE public.owner_beta_daily_poll_state SET next_poll_at=t+interval '60 seconds',latest_receipt_id=rid,
        last_completed_at=NULL,last_result_code=NULL,verified_admission_sha256=NULL,source_frontier_sha256=NULL WHERE stage=p_stage;
    RETURN QUERY SELECT rid;
END $$;

CREATE FUNCTION public.take_owner_beta_daily_admission_poll_slot() RETURNS TABLE(receipt_id uuid)
LANGUAGE sql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ SELECT * FROM public.daily_0060_take_slot('ADMISSION') $$;
CREATE FUNCTION public.take_owner_beta_daily_schedule_poll_slot() RETURNS TABLE(receipt_id uuid)
LANGUAGE sql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ SELECT * FROM public.daily_0060_take_slot('SCHEDULE') $$;

CREATE FUNCTION public.finish_owner_beta_daily_admission_poll(p_receipt_id uuid,p_result_code text,p_source_frontier_sha256 text)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE c public.owner_beta_daily_control; BEGIN
    IF p_result_code IS NULL OR p_result_code NOT IN ('INPUT_PENDING','INPUT_BLOCKED','IO_FAILED')
       OR (p_source_frontier_sha256 IS NOT NULL AND p_source_frontier_sha256 !~ '^sha256:[0-9a-f]{64}$')
    THEN RAISE EXCEPTION 'daily completion shape' USING ERRCODE='23514'; END IF;
    c:=public.daily_0060_authority(NULL,false);
    PERFORM public.daily_0060_source_lock(c.owner_user_id,true);
    PERFORM public.daily_0060_receipt(p_receipt_id,'ADMISSION',c);
    RETURN public.daily_0060_complete(p_receipt_id,p_result_code,p_source_frontier_sha256,NULL,NULL);
END $$;

CREATE FUNCTION public.finish_owner_beta_daily_schedule_poll(p_receipt_id uuid,p_result_code text)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE c public.owner_beta_daily_control; BEGIN
    IF p_result_code IS NULL OR p_result_code NOT IN ('CHECKED','DEFERRED','BLOCKED') THEN RAISE EXCEPTION 'daily completion shape' USING ERRCODE='23514'; END IF;
    c:=public.daily_0060_authority(NULL,false);
    PERFORM public.daily_0060_source_lock(c.owner_user_id,false);
    PERFORM public.daily_0060_receipt(p_receipt_id,'SCHEDULE',c);
    RETURN public.daily_0060_complete(p_receipt_id,p_result_code,NULL,NULL,NULL);
END $$;

CREATE FUNCTION public.daily_0060_lock_admissions(owner_id uuid) RETURNS void
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$ BEGIN
    PERFORM 1 FROM public.owner_beta_daily_admissions WHERE owner_user_id=owner_id
        ORDER BY range_end,admission_sha256 FOR SHARE;
END $$;

CREATE FUNCTION public.daily_0060_event(c public.owner_beta_daily_control,kind text,lo date,hi date,code text,evidence text)
RETURNS text LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp
AS $$ DECLARE eid text; first_admission text; a record; BEGIN
    eid:=public.daily_0060_digest('owner-beta-daily-source-event-v1'||chr(10)||c.owner_user_id::text||chr(10)||kind||chr(10)||code||chr(10)||lo::text||chr(10)||hi::text||chr(10)||evidence||chr(10));
    SELECT admission_sha256 INTO first_admission FROM public.owner_beta_daily_admissions
      WHERE owner_user_id=c.owner_user_id AND range_start<=hi AND range_end>=lo ORDER BY range_end,admission_sha256 LIMIT 1;
    INSERT INTO public.owner_beta_daily_source_events(event_sha256,owner_user_id,source_kind,range_start,range_end,code,evidence_sha256,admission_sha256)
      VALUES(eid,c.owner_user_id,kind,lo,hi,code,evidence,first_admission) ON CONFLICT DO NOTHING;
    FOR a IN SELECT admission_sha256 FROM public.owner_beta_daily_admissions
      WHERE owner_user_id=c.owner_user_id AND range_start<=hi AND range_end>=lo ORDER BY range_end,admission_sha256 LOOP
        INSERT INTO public.owner_beta_daily_admission_events(event_sha256,admission_sha256,owner_user_id,kind,evidence_sha256)
          VALUES(public.daily_0060_digest('owner-beta-daily-admission-event-v1'||chr(10)||'CONFLICT'||chr(10)||a.admission_sha256||chr(10)||eid||chr(10)),
                 a.admission_sha256,c.owner_user_id,'CONFLICT',eid) ON CONFLICT DO NOTHING;
    END LOOP;
    RETURN eid;
END $$;

CREATE FUNCTION public.block_owner_beta_daily_input(p_receipt_id uuid,p_event jsonb)
RETURNS TABLE(disposition text,admission_sha256 text,reason_code text,event_sha256 text)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE c public.owner_beta_daily_control; r public.owner_beta_daily_poll_receipts; lo date; hi date; eid text; BEGIN
    IF NOT public.daily_0060_keys(p_event,ARRAY['source_kind','range_start','range_end','code','evidence_sha256'])
       OR p_event->>'source_kind' IS NULL OR p_event->>'source_kind' NOT IN ('ACTION','CALENDAR','BAR','REFERENCE')
       OR p_event->>'code' IS NULL OR p_event->>'code' NOT IN ('SOURCE_CONFLICT','SEALED_SOURCE_CHANGED','SOURCE_INTEGRITY','CALENDAR_CONFLICT','NONCASH_ACTION')
       OR NOT public.daily_0060_hash(p_event->'evidence_sha256')
    THEN RAISE EXCEPTION 'daily event shape' USING ERRCODE='23514'; END IF;
    lo:=public.daily_0060_date(p_event->'range_start'); hi:=public.daily_0060_date(p_event->'range_end');
    IF hi<lo THEN RAISE EXCEPTION 'daily event range' USING ERRCODE='23514'; END IF;
    c:=public.daily_0060_authority(hi,false);
    PERFORM public.daily_0060_source_lock(c.owner_user_id,true);
    PERFORM public.daily_0060_lock_admissions(c.owner_user_id);
    r:=public.daily_0060_receipt(p_receipt_id,'ADMISSION',c);
    eid:=public.daily_0060_digest('owner-beta-daily-source-event-v1'||chr(10)||c.owner_user_id::text||chr(10)||(p_event->>'source_kind')||chr(10)||(p_event->>'code')||chr(10)||lo::text||chr(10)||hi::text||chr(10)||(p_event->>'evidence_sha256')||chr(10));
    IF r.completed_at IS NOT NULL THEN
        IF r.result_code IS DISTINCT FROM 'INPUT_BLOCKED' OR r.event_sha256 IS DISTINCT FROM eid
           OR r.completion_binding_sha256 IS DISTINCT FROM public.daily_0060_digest('sql-block-v1'||p_event::text) THEN
            RAISE EXCEPTION 'daily completion mismatch' USING ERRCODE='23514'; END IF;
    ELSE
        eid:=public.daily_0060_event(c,p_event->>'source_kind',lo,hi,p_event->>'code',p_event->>'evidence_sha256');
        PERFORM public.daily_0060_complete(p_receipt_id,'INPUT_BLOCKED',p_event->>'evidence_sha256',NULL,eid,public.daily_0060_digest('sql-block-v1'||p_event::text));
    END IF;
    RETURN QUERY SELECT 'BLOCKED'::text,NULL::text,p_event->>'code',eid;
END $$;

CREATE FUNCTION public.revoke_owner_beta_daily_input(p_admission_sha256 text,p_evidence_sha256 text)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE c public.owner_beta_daily_control; a public.owner_beta_daily_admissions; n integer; BEGIN
    SELECT * INTO c FROM public.owner_beta_daily_control WHERE id='etf11' FOR SHARE;
    IF NOT FOUND OR p_evidence_sha256 IS NULL OR p_evidence_sha256 !~ '^sha256:[0-9a-f]{64}$' THEN
        RAISE EXCEPTION 'daily revocation shape' USING ERRCODE='23514'; END IF;
    -- Release revocation must still work after loss of Owner/entitlement.
    PERFORM public.daily_0060_source_lock(c.owner_user_id,true);
    PERFORM public.daily_0060_lock_admissions(c.owner_user_id);
    SELECT * INTO a FROM public.owner_beta_daily_admissions WHERE admission_sha256=p_admission_sha256 AND owner_user_id=c.owner_user_id;
    IF NOT FOUND THEN RAISE EXCEPTION 'daily admission unavailable' USING ERRCODE='23514'; END IF;
    INSERT INTO public.owner_beta_daily_admission_events(event_sha256,admission_sha256,owner_user_id,kind,evidence_sha256)
      VALUES(public.daily_0060_digest('owner-beta-daily-admission-event-v1'||chr(10)||'REVOKED'||chr(10)||p_admission_sha256||chr(10)||p_evidence_sha256||chr(10)),
             p_admission_sha256,c.owner_user_id,'REVOKED',p_evidence_sha256) ON CONFLICT DO NOTHING;
    GET DIAGNOSTICS n=ROW_COUNT;
    RETURN n=1;
END $$;

-- Compact encoding is used only for validated FileEvidence observation identities.
-- D4 objects have prescribed field order, not jsonb's storage/display order.
CREATE FUNCTION public.daily_0060_compact(j jsonb) RETURNS text
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,pg_temp
AS $$ DECLARE s text; BEGIN
    IF jsonb_typeof(j)='array' THEN
        SELECT '['||COALESCE(string_agg(public.daily_0060_compact(value),',' ORDER BY ord),'')||']' INTO s FROM jsonb_array_elements(j) WITH ORDINALITY x(value,ord);
    ELSIF jsonb_typeof(j)='object' THEN
        SELECT '{'||COALESCE(string_agg(to_jsonb(key)::text||':'||public.daily_0060_compact(value),',' ORDER BY key COLLATE "C"),'')||'}' INTO s FROM jsonb_each(j);
    ELSE s:=j::text; END IF;
    RETURN s;
END $$;
CREATE FUNCTION public.daily_0060_origin_bytes(j jsonb) RETURNS text
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,pg_temp
AS $$ DECLARE k text; parts text[]:='{}'; req text[]:='{}'; BEGIN
    FOREACH k IN ARRAY ARRAY['endpoint','query','headers','mode'] LOOP
        req:=array_append(req,to_jsonb(k)::text||':'||public.daily_0060_compact(j->'request'->k));
    END LOOP;
    FOREACH k IN ARRAY ARRAY['provider','market','batch_id','batch_date','batch_json_sha256','manifest_line_sha256','file_name','kind','wire_sha256','size_bytes','request','response_continuation','retrieved_at','retrieval_precision'] LOOP
        parts:=array_append(parts,to_jsonb(k)::text||':'||CASE WHEN k='request' THEN '{'||array_to_string(req,',')||'}' ELSE public.daily_0060_compact(j->k) END);
    END LOOP;
    RETURN '{'||array_to_string(parts,',')||'}';
END $$;
CREATE FUNCTION public.daily_0060_file_bytes(j jsonb) RETURNS text
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,pg_temp
AS $$ DECLARE copies text; base text; BEGIN
    SELECT COALESCE(string_agg(public.daily_0060_origin_bytes(value),',' ORDER BY ord),'') INTO copies FROM jsonb_array_elements(j->'copied_from') WITH ORDINALITY x(value,ord);
    base:=public.daily_0060_origin_bytes(j);
    RETURN left(base,length(base)-1)||',"original_retrieved_at":'||(j->'original_retrieved_at')::text||',"original_retrieval_precision":"batch","copied_from":['||copies||']}';
END $$;

CREATE FUNCTION public.project_owner_beta_daily_calendar(p_receipt_id uuid,p_calendar_evidence_sha256 text,p_file_evidence jsonb,p_days jsonb)
RETURNS TABLE(disposition text,calendar_evidence_sha256 text,reason_code text,event_sha256 text)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE c public.owner_beta_daily_control; r public.owner_beta_daily_poll_receipts; o jsonb; day jsonb;
    target date; d date; lo date; hi date; seen date[]:='{}'; ords integer[]:='{}'; targets integer:=0;
    existing public.owner_beta_daily_calendar_sources; stored jsonb; eid text; observation text; expected_key text; binding text;
BEGIN
    o:=public.daily_0060_file(p_file_evidence,'calendar'); target:=public.daily_0060_date(o->'batch_date');
    IF jsonb_typeof(p_days) IS DISTINCT FROM 'array' THEN RAISE EXCEPTION 'daily calendar shape' USING ERRCODE='23514'; END IF;
    IF jsonb_array_length(p_days) NOT BETWEEN 1 AND 370 THEN RAISE EXCEPTION 'daily calendar bound' USING ERRCODE='23514'; END IF;
    FOR day IN SELECT value FROM jsonb_array_elements(p_days) LOOP
        IF NOT public.daily_0060_keys(day,ARRAY['session_date','session_type','evidence_kind','source_row_ordinal','source_row_sha256'])
           OR day->>'session_type' IS NULL OR day->>'session_type' NOT IN ('TRADING','CLOSED')
           OR NOT public.daily_0060_uint(day->'source_row_ordinal',0,2147483647) OR NOT public.daily_0060_hash(day->'source_row_sha256')
        THEN RAISE EXCEPTION 'daily calendar row' USING ERRCODE='23514'; END IF;
        d:=public.daily_0060_date(day->'session_date');
        IF d=ANY(seen) OR (day->>'source_row_ordinal')::int=ANY(ords)
           OR day->>'evidence_kind' IS DISTINCT FROM (CASE WHEN d=target THEN 'REQUESTED_TARGET' ELSE 'DERIVED_SURROUNDING' END)
        THEN RAISE EXCEPTION 'daily calendar binding' USING ERRCODE='23514'; END IF;
        seen:=array_append(seen,d); ords:=array_append(ords,(day->>'source_row_ordinal')::int);
        lo:=LEAST(lo,d); hi:=GREATEST(hi,d); IF d=target THEN targets:=targets+1; END IF;
    END LOOP;
    IF targets<>1 OR hi-lo>=370 THEN RAISE EXCEPTION 'daily requested target missing' USING ERRCODE='23514'; END IF;
    c:=public.daily_0060_authority(hi,false);
    expected_key:=public.daily_0060_digest('owner-beta-daily-calendar-source-v1'||chr(10)||c.owner_user_id::text||chr(10)||c.policy_sha256||chr(10)||'daily-raw-reconstruction-v1'||chr(10)||public.daily_0060_file_bytes(p_file_evidence));
    IF expected_key IS DISTINCT FROM p_calendar_evidence_sha256 THEN RAISE EXCEPTION 'daily calendar identity' USING ERRCODE='23514'; END IF;
    binding:=public.daily_0060_digest('sql-calendar-v1'||jsonb_build_array(p_calendar_evidence_sha256,p_file_evidence,p_days)::text);
    observation:=public.daily_0060_digest('owner-beta-daily-source-observation-v1'||chr(10)||'['||public.daily_0060_file_bytes(p_file_evidence)||']');
    PERFORM public.daily_0060_source_lock(c.owner_user_id,true);
    PERFORM public.daily_0060_lock_admissions(c.owner_user_id);
    r:=public.daily_0060_receipt(p_receipt_id,'ADMISSION',c);
    SELECT * INTO existing FROM public.owner_beta_daily_calendar_sources WHERE owner_beta_daily_calendar_sources.calendar_evidence_sha256=p_calendar_evidence_sha256;
    IF FOUND THEN
        SELECT jsonb_agg(jsonb_build_object('session_date',session_date,'session_type',session_type,'evidence_kind',evidence_kind,'source_row_ordinal',source_row_ordinal,'source_row_sha256',source_row_sha256) ORDER BY session_date)
          INTO stored FROM public.owner_beta_daily_calendar_days WHERE owner_beta_daily_calendar_days.calendar_evidence_sha256=p_calendar_evidence_sha256;
        IF existing.evidence_json IS DISTINCT FROM p_file_evidence OR stored IS DISTINCT FROM
            (SELECT jsonb_agg(value ORDER BY value->>'session_date') FROM jsonb_array_elements(p_days)) THEN
            RAISE EXCEPTION 'daily calendar replay mismatch' USING ERRCODE='23514'; END IF;
        RETURN QUERY SELECT 'REPLAY'::text,p_calendar_evidence_sha256,NULL::text,NULL::text; RETURN;
    END IF;
    IF r.completed_at IS NOT NULL THEN
        IF r.result_code='INPUT_BLOCKED' AND r.completion_binding_sha256=binding AND EXISTS(SELECT 1 FROM public.owner_beta_daily_source_events e WHERE e.event_sha256=r.event_sha256 AND e.evidence_sha256=observation AND e.code IN ('CALENDAR_CONFLICT','SEALED_SOURCE_CHANGED')) THEN
            RETURN QUERY SELECT 'BLOCKED'::text,NULL::text,e.code,r.event_sha256 FROM public.owner_beta_daily_source_events e WHERE e.event_sha256=r.event_sha256; RETURN;
        END IF;
        RAISE EXCEPTION 'daily receipt completed' USING ERRCODE='23514';
    END IF;
    IF EXISTS (SELECT 1 FROM jsonb_array_elements(p_days) x JOIN public.owner_beta_daily_calendar_days cd
      ON cd.owner_user_id=c.owner_user_id AND cd.session_date=(x->>'session_date')::date WHERE cd.session_type<>x->>'session_type') THEN
        eid:=public.daily_0060_event(c,'CALENDAR',lo,hi,'CALENDAR_CONFLICT',observation);
        PERFORM public.daily_0060_complete(p_receipt_id,'INPUT_BLOCKED',observation,NULL,eid,binding);
        RETURN QUERY SELECT 'BLOCKED'::text,NULL::text,'CALENDAR_CONFLICT'::text,eid; RETURN;
    END IF;
    IF EXISTS(SELECT 1 FROM public.owner_beta_daily_calendar_sources s
        WHERE s.owner_user_id=c.owner_user_id AND s.policy_sha256=c.policy_sha256
          AND s.source_provider=o->>'provider' AND s.source_market=o->>'market'
          AND s.source_batch_id=(o->>'batch_id')::uuid AND s.source_file_name=o->>'file_name'
          AND public.daily_0060_file(s.evidence_json,'calendar') IS DISTINCT FROM o) THEN
        eid:=public.daily_0060_event(c,'CALENDAR',lo,hi,'SEALED_SOURCE_CHANGED',observation);
        PERFORM public.daily_0060_complete(p_receipt_id,'INPUT_BLOCKED',observation,NULL,eid,binding);
        RETURN QUERY SELECT 'BLOCKED'::text,NULL::text,'SEALED_SOURCE_CHANGED'::text,eid; RETURN;
    END IF;
    INSERT INTO public.owner_beta_daily_calendar_sources VALUES(p_calendar_evidence_sha256,c.owner_user_id,c.policy_sha256,'daily-raw-reconstruction-v1',
        o->>'provider',o->>'market',(o->>'batch_id')::uuid,target,target,o->>'file_name',o->>'wire_sha256',o->>'manifest_line_sha256',
        (o->>'retrieved_at')::timestamptz,p_file_evidence->'copied_from',p_file_evidence,clock_timestamp());
    INSERT INTO public.owner_beta_daily_calendar_days SELECT c.owner_user_id,p_calendar_evidence_sha256,(x->>'session_date')::date,
        x->>'session_type',x->>'evidence_kind',(x->>'source_row_ordinal')::int,x->>'source_row_sha256' FROM jsonb_array_elements(p_days) x;
    RETURN QUERY SELECT 'PROJECTED'::text,p_calendar_evidence_sha256,NULL::text,NULL::text;
END $$;

-- Caller already holds the common source fence. Never guess weekdays or fall back
-- across a missing civil date. Wall time is sampled after all authority locks.
CREATE FUNCTION public.daily_0060_expected_at(c public.owner_beta_daily_control,local_now timestamp) RETURNS date
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp
AS $$ DECLARE civil date; expected date; n integer; BEGIN
    civil:=local_now::date - CASE WHEN local_now::time<time '16:30' THEN 1 ELSE 0 END;
    SELECT max(d.session_date) INTO expected FROM public.owner_beta_daily_calendar_days d
      JOIN public.owner_beta_daily_calendar_sources s USING(owner_user_id,calendar_evidence_sha256)
      WHERE d.owner_user_id=c.owner_user_id AND s.policy_sha256=c.policy_sha256 AND d.session_type='TRADING'
        AND d.session_date BETWEEN civil-369 AND civil;
    IF expected IS NULL THEN RETURN NULL; END IF;
    SELECT count(DISTINCT d.session_date) INTO n FROM public.owner_beta_daily_calendar_days d
      JOIN public.owner_beta_daily_calendar_sources s USING(owner_user_id,calendar_evidence_sha256)
      WHERE d.owner_user_id=c.owner_user_id AND s.policy_sha256=c.policy_sha256 AND d.session_date BETWEEN expected AND civil;
    IF n<>civil-expected+1 OR EXISTS(SELECT 1 FROM public.owner_beta_daily_source_events WHERE owner_user_id=c.owner_user_id AND range_start<=civil AND range_end>=expected)
       OR EXISTS(SELECT 1 FROM public.owner_beta_daily_calendar_days WHERE owner_user_id=c.owner_user_id AND session_date BETWEEN expected AND civil GROUP BY session_date HAVING count(DISTINCT session_type)>1)
    THEN RETURN NULL; END IF;
    RETURN expected;
END $$;

CREATE FUNCTION public.daily_0060_expected(c public.owner_beta_daily_control) RETURNS date
LANGUAGE sql SET search_path=pg_catalog,pg_temp
AS $$ SELECT public.daily_0060_expected_at(c,clock_timestamp() AT TIME ZONE 'Asia/Seoul') $$;

CREATE FUNCTION public.daily_0060_admission_usable(c public.owner_beta_daily_control,aid text,require_fresh boolean) RETURNS boolean
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp
AS $$ DECLARE a public.owner_beta_daily_admissions; BEGIN
    SELECT * INTO a FROM public.owner_beta_daily_admissions WHERE admission_sha256=aid AND owner_user_id=c.owner_user_id AND policy_sha256=c.policy_sha256;
    IF NOT FOUND THEN RETURN false; END IF;
    IF EXISTS(WITH RECURSIVE chain AS (SELECT admission_sha256,parent_admission_sha256 FROM public.owner_beta_daily_admissions WHERE admission_sha256=aid
         UNION ALL SELECT p.admission_sha256,p.parent_admission_sha256 FROM public.owner_beta_daily_admissions p JOIN chain q ON p.admission_sha256=q.parent_admission_sha256)
       SELECT 1 FROM chain JOIN public.owner_beta_daily_admission_events USING(admission_sha256))
       OR EXISTS(SELECT 1 FROM public.owner_beta_daily_source_events WHERE owner_user_id=c.owner_user_id AND range_start<=a.range_end AND range_end>=a.range_start)
    THEN RETURN false; END IF;
    IF require_fresh THEN
        -- Lock first, then sample wall time: waiting cannot preserve old freshness.
        PERFORM 1 FROM public.owner_beta_daily_poll_receipts r JOIN public.owner_beta_daily_poll_state s ON s.latest_receipt_id=r.receipt_id
          WHERE s.stage='ADMISSION' FOR SHARE OF r,s;
        IF NOT EXISTS(SELECT 1 FROM public.owner_beta_daily_poll_receipts r JOIN public.owner_beta_daily_poll_state s ON s.latest_receipt_id=r.receipt_id
          WHERE s.stage='ADMISSION' AND r.owner_user_id=c.owner_user_id AND r.policy_sha256=c.policy_sha256
            AND r.activation_epoch=c.activation_epoch AND r.result_code='VERIFIED' AND r.admission_sha256=aid
            AND s.verified_admission_sha256=aid AND r.completed_at>=clock_timestamp()-interval '180 seconds'
            AND r.completed_at<=clock_timestamp()) THEN RETURN false; END IF;
    END IF;
    RETURN true;
END $$;

CREATE FUNCTION public.daily_0060_file_key(f jsonb) RETURNS jsonb
LANGUAGE sql IMMUTABLE SET search_path=pg_catalog,pg_temp
AS $$ SELECT jsonb_build_object('provider',f->'provider','market',f->'market','batch_id',f->'batch_id','file_name',f->'file_name','wire_sha256',f->'wire_sha256') $$;

-- SQL validates the complete projection. Canonical artifact hashes and physical
-- source bytes remain the narrow verifier's responsibility, not jsonb::text hashes.
CREATE FUNCTION public.daily_0060_manifest(j jsonb, policy jsonb) RETURNS jsonb
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp
AS $$ DECLARE s jsonb; x jsonb; f jsonb; o jsonb; a jsonb; page jsonb; first_page jsonb; first_origin jsonb; prior_page jsonb; doc jsonb; k jsonb;
    dt date; prev date:=date '2026-08-28'; d date; last_date date; lo date; hi date; seen date[]:='{}'; dates date[]:='{}';
    files jsonb:='{}'; file_id text; session_files jsonb; kinds text[]; symbols text[]; symbol text;
    n integer; ns integer; nc integer:=0; na integer:=0; np integer:=0; cash_count bigint:=0; session_cash bigint; nd integer:=0;
    required_keys jsonb; actual_keys jsonb; docs_seen text[]; q jsonb; endpoint text; trid text; logical_class text; cash jsonb;
BEGIN
    IF NOT public.daily_0060_keys(j,ARRAY['schema','capability','audience','vendor_snapshot','strict_pit','base','parent_admission_sha256','range_start','range_end','instruments','sessions','daily_sources','bars','normalization','policy_sha256','materializer_version','code_commit','content_sha256'])
       OR j->>'schema' IS DISTINCT FROM 'owner-beta-daily-etf11-input-v1' OR j->>'capability' IS DISTINCT FROM 'PRICE_RETURN_ONLY'
       OR j->>'audience' IS DISTINCT FROM 'OWNER_ONLY' OR j->'vendor_snapshot' IS DISTINCT FROM 'true'::jsonb OR j->'strict_pit' IS DISTINCT FROM 'false'::jsonb
       OR j->'base' IS DISTINCT FROM policy->'base' OR j->'instruments' IS DISTINCT FROM policy->'instruments'
       OR j->>'materializer_version' IS DISTINCT FROM 'daily-price-only-v1'
       OR jsonb_typeof(j->'code_commit') IS DISTINCT FROM 'string' OR j->>'code_commit' !~ '^[0-9a-f]{40}$'
       OR NOT public.daily_0060_hash(j->'content_sha256') OR NOT public.daily_0060_hash(j->'policy_sha256')
       OR NOT (j->'parent_admission_sha256'='null'::jsonb OR public.daily_0060_hash(j->'parent_admission_sha256'))
       OR jsonb_typeof(j->'sessions') IS DISTINCT FROM 'array' OR jsonb_typeof(j->'daily_sources') IS DISTINCT FROM 'array'
    THEN RAISE EXCEPTION 'daily manifest shape' USING ERRCODE='23514'; END IF;
    ns:=jsonb_array_length(j->'sessions'); n:=jsonb_array_length(j->'daily_sources');
    IF ns NOT BETWEEN 2453 AND 20000 OR n<>ns-2452 OR (j->'parent_admission_sha256'='null'::jsonb) IS DISTINCT FROM (n=1)
       OR public.daily_0060_date(j->'range_start')<>date '2016-08-29'
    THEN RAISE EXCEPTION 'daily manifest counts' USING ERRCODE='23514'; END IF;
    FOR x IN SELECT value FROM jsonb_array_elements(j->'sessions') LOOP
        d:=public.daily_0060_date(x);
        IF last_date IS NOT NULL AND d<=last_date THEN RAISE EXCEPTION 'daily session order' USING ERRCODE='23514'; END IF;
        dates:=array_append(dates,d); last_date:=d;
    END LOOP;
    IF dates[1]<>date '2016-08-29' OR dates[2452]<>date '2026-08-28' OR last_date<>public.daily_0060_date(j->'range_end') THEN
        RAISE EXCEPTION 'daily base range' USING ERRCODE='23514'; END IF;
    IF NOT public.daily_0060_keys(j->'bars',ARRAY['file','sha256','size_bytes','row_count'])
       OR j#>>'{bars,file}' IS DISTINCT FROM 'bars.ndjson' OR NOT public.daily_0060_hash(j#>'{bars,sha256}')
       OR NOT public.daily_0060_uint(j#>'{bars,size_bytes}',1,134217728) OR j#>'{bars,row_count}' IS DISTINCT FROM to_jsonb(11*ns)
       OR NOT public.daily_0060_keys(j->'normalization',ARRAY['file','sha256','size_bytes','row_count'])
       OR j#>>'{normalization,file}' IS DISTINCT FROM 'normalization.json' OR NOT public.daily_0060_hash(j#>'{normalization,sha256}')
       OR NOT public.daily_0060_uint(j#>'{normalization,size_bytes}',1,16777216)
    THEN RAISE EXCEPTION 'daily descriptor shape' USING ERRCODE='23514'; END IF;
    FOR s IN SELECT value FROM jsonb_array_elements(j->'daily_sources') LOOP
        IF NOT public.daily_0060_keys(s,ARRAY['session_date','previous_frontier','instruments','bars','reference','calendar','actions','normalized_documents'])
           OR s->'instruments' IS DISTINCT FROM policy->'instruments' THEN RAISE EXCEPTION 'daily session shape' USING ERRCODE='23514'; END IF;
        dt:=public.daily_0060_date(s->'session_date');
        IF public.daily_0060_date(s->'previous_frontier')<>prev OR dt<=prev OR dt-prev>370 OR dt<>dates[2453+cardinality(seen)] THEN
            RAISE EXCEPTION 'daily extension gap' USING ERRCODE='23514'; END IF;
        seen:=array_append(seen,dt); session_files:='[]'; docs_seen:='{}'; session_cash:=0;
        FOREACH symbol IN ARRAY ARRAY['bars','reference','calendar','actions','normalized_documents'] LOOP
            IF jsonb_typeof(s->symbol) IS DISTINCT FROM 'array' THEN RAISE EXCEPTION 'daily evidence array' USING ERRCODE='23514'; END IF;
        END LOOP;
        IF jsonb_array_length(s->'bars')<>11 OR jsonb_array_length(s->'reference')<>11
           OR jsonb_array_length(s->'calendar')<>dt-prev OR jsonb_array_length(s->'actions') NOT BETWEEN 7 AND 32768
           OR jsonb_array_length(s->'normalized_documents')<>3+dt-prev
        THEN RAISE EXCEPTION 'daily evidence count' USING ERRCODE='23514'; END IF;
        FOREACH symbol IN ARRAY ARRAY['bars','reference'] LOOP
            symbols:='{}';
            FOR f IN SELECT value FROM jsonb_array_elements(s->symbol) LOOP
                o:=public.daily_0060_file(f,symbol);
                IF public.daily_0060_date(o->'batch_date')<>dt THEN RAISE EXCEPTION 'daily price date' USING ERRCODE='23514'; END IF;
                SELECT value->>1 INTO file_id FROM jsonb_array_elements(f#>'{request,query}') WHERE value->>0='FID_INPUT_ISCD';
                IF file_id=ANY(symbols) THEN RAISE EXCEPTION 'daily duplicate symbol' USING ERRCODE='23514'; END IF;
                symbols:=array_append(symbols,file_id); session_files:=session_files||jsonb_build_array(f);
            END LOOP;
        END LOOP;
        d:=prev;
        FOR x IN SELECT value FROM jsonb_array_elements(s->'calendar') LOOP
            d:=d+1;
            IF NOT public.daily_0060_keys(x,ARRAY['civil_date','session_type','exchange','timezone','source_file','normalized_document_sha256','legacy_projection'])
               OR public.daily_0060_date(x->'civil_date')<>d OR x->>'session_type' IS DISTINCT FROM (CASE WHEN d=dt THEN 'TRADING' ELSE 'CLOSED' END)
               OR x->>'exchange' IS DISTINCT FROM 'KRX' OR x->>'timezone' IS DISTINCT FROM 'Asia/Seoul'
               OR NOT public.daily_0060_hash(x->'normalized_document_sha256')
            THEN RAISE EXCEPTION 'daily civil binding' USING ERRCODE='23514'; END IF;
            PERFORM public.daily_0060_file(x->'source_file','calendar');
            session_files:=session_files||jsonb_build_array(x->'source_file');
            k:=x->'legacy_projection';
            IF k<>'null'::jsonb THEN
                IF NOT public.daily_0060_keys(k,ARRAY['source','source_version','source_batch_id','source_file_name','document_sha256','retrieved_at'])
                   OR k->>'source' IS DISTINCT FROM 'kis' OR k->>'source_version' IS DISTINCT FROM 'kis-chk-holiday-v1:schema-1'
                   OR k->>'source_file_name' IS DISTINCT FROM 'calendar.json' OR NOT public.daily_0060_hash(k->'document_sha256')
                   OR jsonb_typeof(k->'source_batch_id') IS DISTINCT FROM 'string'
                   OR k->>'source_batch_id' !~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                   OR jsonb_typeof(k->'retrieved_at') IS DISTINCT FROM 'string'
                   OR k->>'retrieved_at' !~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-5][0-9]\.[0-9]{9}Z$'
                THEN RAISE EXCEPTION 'daily legacy binding' USING ERRCODE='23514'; END IF;
            END IF;
            nc:=nc+1;
        END LOOP;
        FOR a IN SELECT value FROM jsonb_array_elements(s->'actions') LOOP
            IF NOT public.daily_0060_keys(a,ARRAY['logical_class','scope','symbol','range_start','range_end','pages','cash_dividends','cash_dividend_rows_sha256'])
               OR a->>'scope' IS NULL OR a->>'scope' NOT IN ('whole_market','etf11_symbol')
               OR NOT public.daily_0060_hash(a->'cash_dividend_rows_sha256')
               OR jsonb_typeof(a->'pages') IS DISTINCT FROM 'array' OR jsonb_typeof(a->'cash_dividends') IS DISTINCT FROM 'array'
            THEN RAISE EXCEPTION 'daily action shape' USING ERRCODE='23514'; END IF;
            logical_class:=a->>'logical_class';
            SELECT path,tr INTO endpoint,trid FROM (VALUES ('paidin-subscription','paidin-capin','HHKDB669100C0'),('paidin-record','paidin-capin','HHKDB669100C0'),('bonus-issue','bonus-issue','HHKDB669101C0'),('dividend','dividend','HHKDB669102C0'),('merger-split','merger-split','HHKDB669104C0'),('reverse-split','rev-split','HHKDB669105C0'),('capital-decrease','cap-dcrs','HHKDB669106C0')) allowed(class,path,tr) WHERE class=logical_class;
            IF NOT FOUND THEN RAISE EXCEPTION 'daily action class' USING ERRCODE='23514'; END IF;
            IF (a->>'scope'='whole_market' AND a->'symbol' IS DISTINCT FROM 'null'::jsonb)
               OR (a->>'scope'='etf11_symbol' AND (jsonb_typeof(a->'symbol') IS DISTINCT FROM 'string' OR NOT (policy->'instruments') ? ((a->>'symbol')||'.KRX')))
               OR jsonb_array_length(a->'pages') NOT BETWEEN 1 AND 10 OR jsonb_array_length(a->'cash_dividends')>100000
               OR (logical_class<>'dividend' AND a->'cash_dividends'<>'[]'::jsonb)
            THEN RAISE EXCEPTION 'daily action scope' USING ERRCODE='23514'; END IF;
            lo:=public.daily_0060_date(a->'range_start'); hi:=public.daily_0060_date(a->'range_end');
            IF lo>hi THEN RAISE EXCEPTION 'daily action range' USING ERRCODE='23514'; END IF;
            first_page:=NULL; first_origin:=NULL; prior_page:=NULL; symbols:='{}';
            FOR page IN SELECT value FROM jsonb_array_elements(a->'pages') LOOP
                o:=public.daily_0060_file(page,'corporate_actions');
                SELECT jsonb_object_agg(value->>0,value->1) INTO q FROM jsonb_array_elements(page#>'{request,query}');
                IF page#>>'{request,endpoint}' IS DISTINCT FROM '/uapi/domestic-stock/v1/ksdinfo/'||endpoint
                   OR q->>'F_DT' IS DISTINCT FROM to_char(lo,'YYYYMMDD') OR q->>'T_DT' IS DISTINCT FROM to_char(hi,'YYYYMMDD')
                   OR q->>'SHT_CD' IS DISTINCT FROM COALESCE(a->>'symbol','')
                   OR (logical_class='paidin-subscription' AND q->>'GB1' IS DISTINCT FROM '1')
                   OR (logical_class='paidin-record' AND q->>'GB1' IS DISTINCT FROM '2')
                   OR (SELECT value->>1 FROM jsonb_array_elements(page#>'{request,headers}') WHERE value->>0='tr_cont')
                      IS DISTINCT FROM (CASE WHEN first_page IS NULL THEN '' ELSE 'N' END)
                   OR page->>'wire_sha256'=ANY(symbols)
                   OR (prior_page IS NOT NULL AND prior_page->>'response_continuation' IS DISTINCT FROM 'M')
                   OR (first_origin IS NOT NULL AND ROW(o->'provider',o->'market',o->'batch_id',o->'batch_date',o->'retrieved_at')
                        IS DISTINCT FROM ROW(first_origin->'provider',first_origin->'market',first_origin->'batch_id',first_origin->'batch_date',first_origin->'retrieved_at'))
                   OR (first_page IS NOT NULL AND ROW(page->'provider',page->'batch_id',page->'batch_date',page#>'{request,query}')
                        IS DISTINCT FROM ROW(first_page->'provider',first_page->'batch_id',first_page->'batch_date',first_page#>'{request,query}'))
                THEN RAISE EXCEPTION 'daily page binding' USING ERRCODE='23514'; END IF;
                first_page:=COALESCE(first_page,page); first_origin:=COALESCE(first_origin,o); prior_page:=page; symbols:=array_append(symbols,page->>'wire_sha256');
                session_files:=session_files||jsonb_build_array(page); np:=np+1;
            END LOOP;
            IF prior_page->>'response_continuation'='M' THEN RAISE EXCEPTION 'daily final page' USING ERRCODE='23514'; END IF;
            FOR cash IN SELECT value FROM jsonb_array_elements(a->'cash_dividends') LOOP
                IF NOT public.daily_0060_keys(cash,ARRAY['symbol','record_date','dividend_kind','face_value','cash_dividend_amount','cash_dividend_rate','stock_dividend_rate','cash_payment_date','stock_payment_date','odd_lot_payment_date','stock_kind','high_dividend_flag','source_file'])
                   OR jsonb_typeof(cash->'symbol') IS DISTINCT FROM 'string' OR NOT (policy->'instruments') ? ((cash->>'symbol')||'.KRX')
                   OR (a->>'scope'='etf11_symbol' AND cash->'symbol' IS DISTINCT FROM a->'symbol')
                   OR cash->>'stock_dividend_rate' IS DISTINCT FROM '0' OR cash->'stock_payment_date' IS DISTINCT FROM 'null'::jsonb OR cash->'odd_lot_payment_date' IS DISTINCT FROM 'null'::jsonb
                   OR NOT EXISTS(SELECT 1 FROM jsonb_array_elements(a->'pages') p WHERE public.daily_0060_file_key(p)=cash->'source_file')
                THEN RAISE EXCEPTION 'daily cash row' USING ERRCODE='23514'; END IF;
                d:=public.daily_0060_date(cash->'record_date');
                IF d NOT BETWEEN GREATEST(lo,prev+1) AND LEAST(hi,dt) THEN RAISE EXCEPTION 'daily cash date' USING ERRCODE='23514'; END IF;
                IF cash->'cash_payment_date'<>'null'::jsonb THEN PERFORM public.daily_0060_date(cash->'cash_payment_date'); END IF;
                FOREACH symbol IN ARRAY ARRAY['dividend_kind','stock_kind','high_dividend_flag'] LOOP
                    IF jsonb_typeof(cash->symbol) IS DISTINCT FROM 'string' OR cash->>symbol !~ '^[A-Za-z0-9_-]{0,32}$' THEN RAISE EXCEPTION 'daily cash code' USING ERRCODE='23514'; END IF;
                END LOOP;
                FOREACH symbol IN ARRAY ARRAY['face_value','cash_dividend_amount','cash_dividend_rate'] LOOP
                    IF jsonb_typeof(cash->symbol) IS DISTINCT FROM 'string' OR cash->>symbol !~ '^(0|[1-9][0-9]*)(\.[0-9]*[1-9])?$'
                       OR length(split_part(cash->>symbol,'.',2))>28 OR replace(cash->>symbol,'.','')::numeric>170141183460469231731687303715884105727
                    THEN RAISE EXCEPTION 'daily cash decimal' USING ERRCODE='23514'; END IF;
                END LOOP;
            END LOOP;
            IF (SELECT count(*) FROM jsonb_array_elements(a->'cash_dividends'))<>(SELECT count(DISTINCT value) FROM jsonb_array_elements(a->'cash_dividends')) THEN
                RAISE EXCEPTION 'daily duplicate cash row' USING ERRCODE='23514'; END IF;
            session_cash:=session_cash+jsonb_array_length(a->'cash_dividends');
            cash_count:=cash_count+jsonb_array_length(a->'cash_dividends'); na:=na+1;
        END LOOP;
        IF EXISTS(SELECT 1 FROM unnest(ARRAY['paidin-subscription','paidin-record','bonus-issue','dividend','merger-split','reverse-split','capital-decrease']) cl,
           jsonb_array_elements_text(policy->'instruments') ins, generate_series(1,dt-prev) delta
           WHERE NOT EXISTS(SELECT 1 FROM jsonb_array_elements(s->'actions') ax WHERE ax->>'logical_class'=cl
             AND (ax->>'scope'='whole_market' OR ax->>'symbol'=split_part(ins,'.',1))
             AND (ax->>'range_start')::date<=prev+delta AND (ax->>'range_end')::date>=prev+delta))
           OR (SELECT count(*) FROM jsonb_array_elements(s->'actions'))<>(SELECT count(DISTINCT value) FROM jsonb_array_elements(s->'actions'))
        THEN RAISE EXCEPTION 'daily action coverage' USING ERRCODE='23514'; END IF;
        IF jsonb_array_length(session_files)>262144 THEN RAISE EXCEPTION 'daily source limit' USING ERRCODE='23514'; END IF;
        FOR f IN SELECT value FROM jsonb_array_elements(session_files) LOOP
            file_id:=(public.daily_0060_file_key(f)-'wire_sha256')::text;
            IF files ? file_id AND files->file_id IS DISTINCT FROM f THEN RAISE EXCEPTION 'daily source contradiction' USING ERRCODE='23514'; END IF;
            files:=files||jsonb_build_object(file_id,f);
        END LOOP;
        FOR doc IN SELECT value FROM jsonb_array_elements(s->'normalized_documents') LOOP
            IF NOT public.daily_0060_keys(doc,ARRAY['kind','session_date','normalizer_version','sha256','size_bytes','row_count','source_files','source_evidence_sha256'])
               OR doc->>'normalizer_version' IS DISTINCT FROM 'daily-raw-reconstruction-v1' OR NOT public.daily_0060_hash(doc->'sha256')
               OR NOT public.daily_0060_hash(doc->'source_evidence_sha256') OR NOT public.daily_0060_uint(doc->'size_bytes',1,67108864)
               OR NOT public.daily_0060_uint(doc->'row_count',0,3276800000) OR jsonb_typeof(doc->'source_files') IS DISTINCT FROM 'array'
               OR doc->>'kind' IS NULL OR doc->>'kind' NOT IN ('bars','reference','calendar','corporate_actions')
            THEN RAISE EXCEPTION 'daily document shape' USING ERRCODE='23514'; END IF;
            d:=public.daily_0060_date(doc->'session_date'); file_id:=(doc->>'kind')||':'||d::text;
            IF file_id=ANY(docs_seen) OR (doc->>'kind'<>'calendar' AND d<>dt) OR (doc->>'kind'='calendar' AND d NOT BETWEEN prev+1 AND dt)
               OR (doc->>'kind'='corporate_actions' AND doc->'row_count' IS DISTINCT FROM to_jsonb(session_cash))
               OR (doc->>'kind' IN ('bars','reference') AND doc->'row_count'<>'11'::jsonb) OR (doc->>'kind'='calendar' AND doc->'row_count'<>'1'::jsonb)
            THEN RAISE EXCEPTION 'daily document binding' USING ERRCODE='23514'; END IF;
            docs_seen:=array_append(docs_seen,file_id);
            IF doc->>'kind'='calendar' THEN
                SELECT jsonb_build_array(public.daily_0060_file_key(value->'source_file')) INTO required_keys
                    FROM jsonb_array_elements(s->'calendar') WHERE value->>'civil_date'=d::text AND value->'normalized_document_sha256'=doc->'sha256';
            ELSE
                SELECT jsonb_agg(u.source_key ORDER BY u.source_key) INTO required_keys FROM (SELECT DISTINCT public.daily_0060_file_key(value) AS source_key FROM jsonb_array_elements(session_files) WHERE value->>'kind'=doc->>'kind') u;
            END IF;
            SELECT jsonb_agg(value ORDER BY value) INTO actual_keys FROM jsonb_array_elements(doc->'source_files');
            IF required_keys IS NULL OR actual_keys IS DISTINCT FROM required_keys THEN RAISE EXCEPTION 'daily document sources' USING ERRCODE='23514'; END IF;
            nd:=nd+1;
        END LOOP;
        prev:=dt;
    END LOOP;
    IF j#>'{normalization,row_count}' IS DISTINCT FROM to_jsonb(nd) THEN RAISE EXCEPTION 'daily normalization count' USING ERRCODE='23514'; END IF;
    RETURN jsonb_build_object('instrument_count',11,'session_count',ns,'bar_count',11*ns,'added_session_count',n,
       'source_file_count',(SELECT count(*) FROM jsonb_each(files)),'calendar_row_count',nc,'action_chain_count',na,
       'action_page_count',np,'cash_dividend_row_count',cash_count,'normalized_document_count',nd);
EXCEPTION WHEN invalid_parameter_value OR invalid_text_representation OR datetime_field_overflow OR invalid_datetime_format OR numeric_value_out_of_range THEN
    RAISE EXCEPTION 'daily manifest scalar shape' USING ERRCODE='23514';
END $$;

CREATE FUNCTION public.admit_owner_beta_daily_input(p_receipt_id uuid,p_admission jsonb)
RETURNS TABLE(disposition text,admission_sha256 text,reason_code text,event_sha256 text)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE c public.owner_beta_daily_control; r public.owner_beta_daily_poll_receipts; a jsonb; m jsonb; counts jsonb;
    parent public.owner_beta_daily_admissions; existing public.owner_beta_daily_admissions; aid text; frontier text; eid text;
    stop_date date; prior date:=date '2026-08-28'; ns integer; nd integer; code text; binding text; blocked_event public.owner_beta_daily_source_events;
BEGIN
    IF NOT public.daily_0060_keys(p_admission,ARRAY['admission_sha256','source_frontier_sha256','admission','manifest'])
       OR NOT public.daily_0060_hash(p_admission->'admission_sha256') OR NOT public.daily_0060_hash(p_admission->'source_frontier_sha256')
    THEN RAISE EXCEPTION 'daily admission transport' USING ERRCODE='23514'; END IF;
    binding:=public.daily_0060_digest('sql-admission-v1'||p_admission::text);
    a:=p_admission->'admission'; m:=p_admission->'manifest'; aid:=p_admission->>'admission_sha256'; frontier:=p_admission->>'source_frontier_sha256';
    stop_date:=public.daily_0060_date(m->'range_end'); c:=public.daily_0060_authority(stop_date,false);
    IF NOT public.daily_0060_keys(a,ARRAY['schema','content_sha256','manifest_sha256','policy_sha256','parent_admission_sha256','base','verifier_code_commit','normalizer_version','evidence_sha256','counts'])
       OR a->>'schema' IS DISTINCT FROM 'owner-beta-daily-etf11-admission-v1' OR a->>'normalizer_version' IS DISTINCT FROM 'daily-raw-reconstruction-v1'
       OR NOT public.daily_0060_hash(a->'content_sha256') OR NOT public.daily_0060_hash(a->'manifest_sha256') OR NOT public.daily_0060_hash(a->'evidence_sha256')
       OR a->>'policy_sha256' IS DISTINCT FROM c.policy_sha256 OR m->'policy_sha256' IS DISTINCT FROM a->'policy_sha256'
       OR a->'content_sha256' IS DISTINCT FROM m->'content_sha256' OR a->'parent_admission_sha256' IS DISTINCT FROM m->'parent_admission_sha256'
       OR a->'base' IS DISTINCT FROM c.policy_json->'base' OR jsonb_typeof(a->'verifier_code_commit') IS DISTINCT FROM 'string' OR a->>'verifier_code_commit' !~ '^[0-9a-f]{40}$'
    THEN RAISE EXCEPTION 'daily admission shape' USING ERRCODE='23514'; END IF;
    counts:=public.daily_0060_manifest(m,c.policy_json);
    IF a->'counts' IS DISTINCT FROM counts THEN RAISE EXCEPTION 'daily admission counts' USING ERRCODE='23514'; END IF;
    ns:=(counts->>'session_count')::int; nd:=(counts->>'added_session_count')::int;
    PERFORM public.daily_0060_source_lock(c.owner_user_id,true);
    PERFORM public.daily_0060_lock_admissions(c.owner_user_id);
    IF a->>'parent_admission_sha256' IS NOT NULL THEN
        SELECT * INTO parent FROM public.owner_beta_daily_admissions pa WHERE pa.admission_sha256=a->>'parent_admission_sha256' AND pa.owner_user_id=c.owner_user_id AND pa.policy_sha256=c.policy_sha256;
        IF NOT FOUND OR parent.session_count<>ns-1 OR parent.range_end>=stop_date
           OR parent.evidence_json#>'{manifest,sessions}' IS DISTINCT FROM (m->'sessions')-(ns-1)
           OR parent.evidence_json#>'{manifest,daily_sources}' IS DISTINCT FROM (m->'daily_sources')-(nd-1)
        THEN RAISE EXCEPTION 'daily parent prefix' USING ERRCODE='23514'; END IF;
        prior:=parent.range_end;
    ELSIF ns<>2453 OR nd<>1 THEN RAISE EXCEPTION 'daily initial base' USING ERRCODE='23514'; END IF;
    -- Explicit consecutive projection facts for the new civil interval, not weekdays.
    IF EXISTS(SELECT 1 FROM generate_series(1,stop_date-prior) delta WHERE NOT EXISTS(
        SELECT 1 FROM public.owner_beta_daily_calendar_days d JOIN public.owner_beta_daily_calendar_sources s USING(owner_user_id,calendar_evidence_sha256)
        WHERE d.owner_user_id=c.owner_user_id AND s.policy_sha256=c.policy_sha256 AND d.session_date=prior+delta
          AND d.session_type=CASE WHEN prior+delta=stop_date THEN 'TRADING' ELSE 'CLOSED' END))
       OR stop_date>(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date
    THEN RAISE EXCEPTION 'daily extension proof' USING ERRCODE='23514'; END IF;
    r:=public.daily_0060_receipt(p_receipt_id,'ADMISSION',c);
    SELECT * INTO existing FROM public.owner_beta_daily_admissions ex WHERE ex.owner_user_id=c.owner_user_id AND ex.range_end=stop_date;
    IF r.completed_at IS NOT NULL THEN
        IF r.result_code='VERIFIED' AND r.admission_sha256=aid AND r.source_frontier_sha256=frontier
           AND existing.evidence_json=p_admission AND r.completion_binding_sha256=binding THEN
            RETURN QUERY SELECT 'REPLAY'::text,aid,NULL::text,NULL::text; RETURN;
        ELSIF r.result_code='INPUT_BLOCKED' AND r.source_frontier_sha256=frontier AND r.completion_binding_sha256=binding THEN
            SELECT * INTO blocked_event FROM public.owner_beta_daily_source_events e WHERE e.event_sha256=r.event_sha256;
            IF FOUND THEN RETURN QUERY SELECT 'BLOCKED'::text,NULL::text,blocked_event.code,blocked_event.event_sha256; RETURN; END IF;
        END IF;
        RAISE EXCEPTION 'daily completion mismatch' USING ERRCODE='23514';
    END IF;
    code:=NULL;
    IF existing.admission_sha256 IS NOT NULL AND
       (existing.admission_sha256<>aid OR existing.evidence_json IS DISTINCT FROM p_admission) THEN
        code:='SEALED_SOURCE_CHANGED';
    ELSIF EXISTS(SELECT 1 FROM public.owner_beta_daily_source_events e WHERE e.owner_user_id=c.owner_user_id AND e.range_start<=stop_date AND e.range_end>=date '2016-08-29')
       OR (parent.admission_sha256 IS NOT NULL AND NOT public.daily_0060_admission_usable(c,parent.admission_sha256,false))
       OR (existing.admission_sha256 IS NOT NULL AND NOT public.daily_0060_admission_usable(c,existing.admission_sha256,false)) THEN
        code:='SOURCE_CONFLICT';
    END IF;
    IF code IS NOT NULL THEN
        eid:=public.daily_0060_event(c,'BAR',stop_date,stop_date,code,frontier);
        PERFORM public.daily_0060_complete(p_receipt_id,'INPUT_BLOCKED',frontier,NULL,eid,binding);
        RETURN QUERY SELECT 'BLOCKED'::text,NULL::text,code,eid; RETURN;
    END IF;
    IF existing.admission_sha256 IS NULL THEN
        INSERT INTO public.owner_beta_daily_admissions VALUES(aid,c.owner_user_id,c.policy_sha256,a->>'content_sha256',a->>'manifest_sha256',a->>'parent_admission_sha256',
            date '2016-08-29',stop_date,ns,11*ns,p_admission,clock_timestamp());
    END IF;
    PERFORM public.daily_0060_complete(p_receipt_id,'VERIFIED',frontier,aid,NULL,binding);
    RETURN QUERY SELECT CASE WHEN existing.admission_sha256 IS NULL THEN 'ADMITTED' ELSE 'REPLAY' END,aid,NULL::text,NULL::text;
END $$;

CREATE FUNCTION public.read_owner_beta_daily_source_evidence(p_start date,p_end date)
RETURNS TABLE(evidence jsonb) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE c public.owner_beta_daily_control; BEGIN
    IF p_start IS NULL OR p_end IS NULL OR p_end<p_start OR p_end-p_start>=370 THEN RAISE EXCEPTION 'daily read range' USING ERRCODE='23514'; END IF;
    c:=public.daily_0060_authority(p_end,false);
    PERFORM public.daily_0060_source_lock(c.owner_user_id,false);
    RETURN QUERY SELECT jsonb_build_object('kind','calendar','session_date',d.session_date,'session_type',d.session_type,'evidence_kind',d.evidence_kind,
       'calendar_evidence_sha256',s.calendar_evidence_sha256,'requested_target',s.requested_target,'source_batch_date',s.source_batch_date,
       'source_retrieved_at',s.source_retrieved_at,'projected_at',s.projected_at)
       FROM public.owner_beta_daily_calendar_days d JOIN public.owner_beta_daily_calendar_sources s USING(owner_user_id,calendar_evidence_sha256)
       WHERE d.owner_user_id=c.owner_user_id AND s.policy_sha256=c.policy_sha256 AND d.session_date BETWEEN p_start AND p_end
       ORDER BY d.session_date,s.calendar_evidence_sha256;
    RETURN QUERY SELECT jsonb_build_object('kind','normalized_source','provider',b.provider,'market',b.market,'batch_date',b.batch_date,
       'source_kind',b.kind,'source_batch_id',b.source_batch_id,'source_file_name',b.source_file_name,'content_sha256',b.content_sha256,'retrieved_at',b.retrieved_at)
       FROM public.data_batches b WHERE b.provider='KRX' AND b.market='KR' AND b.fetch_mode='credentialed'
         AND b.kind IN ('EOD','REFERENCE','CORPORATE_ACTIONS') AND b.batch_date BETWEEN p_start AND p_end ORDER BY b.batch_date,b.id;
    RETURN QUERY SELECT jsonb_build_object('kind','source_event','source_kind',e.source_kind,'range_start',e.range_start,'range_end',e.range_end,'code',e.code,'event_sha256',e.event_sha256)
       FROM public.owner_beta_daily_source_events e WHERE e.owner_user_id=c.owner_user_id AND e.range_start<=p_end AND e.range_end>=p_start ORDER BY e.event_sha256;
END $$;

ALTER TABLE public.owner_beta_recommendation_runs
    ADD COLUMN trigger_kind text NOT NULL DEFAULT 'MANUAL',
    ADD COLUMN daily_pins jsonb,
    ADD COLUMN daily_config_epoch bigint,
    ADD COLUMN daily_activation_epoch bigint,
    ADD COLUMN daily_trigger_version text,
    ALTER COLUMN candidate_content_sha256 DROP NOT NULL,
    ALTER COLUMN artifact_manifest_sha256 DROP NOT NULL,
    ALTER COLUMN stage5_manifest_sha256 DROP NOT NULL,
    ALTER COLUMN action_manifest_sha256 DROP NOT NULL,
    ALTER COLUMN approval_registry_sha256 DROP NOT NULL,
    DROP CONSTRAINT owner_beta_recommendation_runs_input_kind_check,
    ADD CONSTRAINT owner_beta_recommendation_runs_input_kind_check CHECK (
      (input_kind='owner_beta_historical_price_only_v1' AND trigger_kind='MANUAL'
       AND candidate_content_sha256 IS NOT NULL AND artifact_manifest_sha256 IS NOT NULL
       AND stage5_manifest_sha256 IS NOT NULL AND action_manifest_sha256 IS NOT NULL AND approval_registry_sha256 IS NOT NULL
       AND daily_pins IS NULL AND daily_config_epoch IS NULL AND daily_activation_epoch IS NULL AND daily_trigger_version IS NULL)
      OR (input_kind='owner_beta_daily_etf11_price_only_v1' AND trigger_kind='DAILY'
       AND candidate_content_sha256 IS NULL AND artifact_manifest_sha256 IS NULL AND stage5_manifest_sha256 IS NULL
       AND action_manifest_sha256 IS NULL AND approval_registry_sha256 IS NULL
       AND daily_pins IS NOT NULL AND daily_config_epoch IS NOT NULL AND daily_activation_epoch IS NOT NULL AND daily_trigger_version IS NOT NULL
       AND daily_config_epoch>=0 AND daily_activation_epoch>=0 AND daily_trigger_version='owner-beta-daily-trigger-v1'
       AND jsonb_typeof(daily_pins)='object' AND daily_pins ?& ARRAY['content_sha256','manifest_sha256','admission_sha256','policy_sha256']
       AND daily_pins-ARRAY['content_sha256','manifest_sha256','admission_sha256','policy_sha256']='{}'::jsonb
       AND COALESCE(daily_pins->>'content_sha256' ~ '^sha256:[0-9a-f]{64}$',false)
       AND COALESCE(daily_pins->>'manifest_sha256' ~ '^sha256:[0-9a-f]{64}$',false)
       AND COALESCE(daily_pins->>'admission_sha256' ~ '^sha256:[0-9a-f]{64}$',false)
       AND COALESCE(daily_pins->>'policy_sha256' ~ '^sha256:[0-9a-f]{64}$',false)));

CREATE FUNCTION public.daily_0060_payload(r public.owner_beta_recommendation_runs) RETURNS jsonb
LANGUAGE sql IMMUTABLE SET search_path=pg_catalog,pg_temp
AS $$ SELECT jsonb_build_object('schema','owner-beta-daily-recommendation-v1','run_id',r.id,'strategy_config_id',r.strategy_config_id,'as_of',r.as_of,
    'input',r.daily_pins||'{"kind":"owner_beta_daily_etf11_price_only_v1"}'::jsonb,
    'strategy',jsonb_build_object('strategy_id',r.strategy_id,'strategy_version',r.strategy_version,'config_json',r.strategy_config_json,'config_sha256',r.strategy_config_sha256),
    'trigger',jsonb_build_object('kind','DAILY','version',r.daily_trigger_version,'config_epoch',r.daily_config_epoch,'activation_epoch',r.daily_activation_epoch)) $$;

CREATE FUNCTION public.daily_0060_payload_shape(j jsonb) RETURNS boolean
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp
AS $$ DECLARE k text; BEGIN
    IF NOT public.daily_0060_keys(j,ARRAY['schema','run_id','strategy_config_id','as_of','input','strategy','trigger'])
       OR j->>'schema' IS DISTINCT FROM 'owner-beta-daily-recommendation-v1'
       OR NOT public.daily_0060_keys(j->'input',ARRAY['kind','content_sha256','manifest_sha256','admission_sha256','policy_sha256'])
       OR j#>>'{input,kind}' IS DISTINCT FROM 'owner_beta_daily_etf11_price_only_v1'
       OR NOT public.daily_0060_keys(j->'strategy',ARRAY['strategy_id','strategy_version','config_json','config_sha256'])
       OR jsonb_typeof(j#>'{strategy,strategy_id}') IS DISTINCT FROM 'string' OR j#>>'{strategy,strategy_id}'=''
       OR jsonb_typeof(j#>'{strategy,strategy_version}') IS DISTINCT FROM 'string' OR j#>>'{strategy,strategy_version}'=''
       OR jsonb_typeof(j#>'{strategy,config_json}') IS DISTINCT FROM 'object' OR NOT public.daily_0060_hash(j#>'{strategy,config_sha256}')
       OR NOT public.daily_0060_keys(j->'trigger',ARRAY['kind','version','config_epoch','activation_epoch'])
       OR j#>>'{trigger,kind}' IS DISTINCT FROM 'DAILY' OR j#>>'{trigger,version}' IS DISTINCT FROM 'owner-beta-daily-trigger-v1'
       OR NOT public.daily_0060_uint(j#>'{trigger,config_epoch}',0,9223372036854775807) OR NOT public.daily_0060_uint(j#>'{trigger,activation_epoch}',0,9223372036854775807)
    THEN RETURN false; END IF;
    FOREACH k IN ARRAY ARRAY['content_sha256','manifest_sha256','admission_sha256','policy_sha256'] LOOP
        IF NOT public.daily_0060_hash(j->'input'->k) THEN RETURN false; END IF;
    END LOOP;
    FOREACH k IN ARRAY ARRAY['run_id','strategy_config_id'] LOOP
        IF jsonb_typeof(j->k) IS DISTINCT FROM 'string' OR j->>k !~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' THEN RETURN false; END IF;
    END LOOP;
    PERFORM public.daily_0060_date(j->'as_of');
    RETURN true;
END $$;

CREATE FUNCTION public.owner_beta_daily_schedule_candidates()
RETURNS TABLE(strategy_config_id uuid,expected_session date,admission_sha256 text,config_epoch bigint,strategy_id text,strategy_version text,config_json jsonb)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE c public.owner_beta_daily_control; expected date; aid text; old_actor text:=current_setting('app.actor_user_id',true); BEGIN
    IF NOT EXISTS(SELECT 1 FROM public.owner_beta_daily_control WHERE enabled) THEN RETURN; END IF;
    c:=public.daily_0060_authority(NULL,true);
    PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
    PERFORM 1 FROM public.user_strategy_configs WHERE owner_user_id=c.owner_user_id ORDER BY id FOR SHARE;
    PERFORM public.daily_0060_source_lock(c.owner_user_id,false);
    PERFORM public.daily_0060_lock_admissions(c.owner_user_id);
    expected:=public.daily_0060_expected(c);
    SELECT a.admission_sha256 INTO aid FROM public.owner_beta_daily_admissions a WHERE a.owner_user_id=c.owner_user_id AND a.range_end=expected;
    IF expected>=c.first_auto_session AND public.daily_0060_admission_usable(c,aid,true) THEN
        RETURN QUERY SELECT sc.id,expected,aid,sc.daily_epoch,sc.strategy_id,sc.strategy_version,sc.config_json
           FROM public.user_strategy_configs sc WHERE sc.owner_user_id=c.owner_user_id AND sc.is_active AND sc.daily_enabled ORDER BY sc.id;
    END IF;
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.reserve_owner_beta_daily_run(p_config_id uuid,p_expected_session date,p_admission_sha256 text,p_config_epoch bigint,p_strategy jsonb)
RETURNS TABLE(run_id uuid,job_id uuid,disposition text)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE c public.owner_beta_daily_control; sc public.user_strategy_configs; a public.owner_beta_daily_admissions;
    r public.owner_beta_recommendation_runs; saved public.owner_beta_daily_reservations; expected date; owner_id uuid; n bigint;
    old_actor text:=current_setting('app.actor_user_id',true);
BEGIN
    SELECT owner_user_id INTO owner_id FROM public.owner_beta_daily_control WHERE id='etf11';
    IF owner_id IS NULL THEN RETURN QUERY SELECT NULL::uuid,NULL::uuid,'BLOCKED'::text; RETURN; END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended(owner_id::text,7919));
    c:=public.daily_0060_authority(p_expected_session,false);
    PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
    SELECT * INTO saved FROM public.owner_beta_daily_reservations WHERE owner_user_id=c.owner_user_id AND strategy_config_id=p_config_id
      AND session_date=p_expected_session AND trigger_version=c.trigger_version;
    IF FOUND THEN
        RETURN QUERY SELECT saved.run_id,saved.job_id,'REPLAY'::text;
        PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN;
    END IF;
    SELECT * INTO sc FROM public.user_strategy_configs WHERE id=p_config_id AND owner_user_id=c.owner_user_id FOR SHARE;
    IF NOT FOUND OR NOT c.enabled OR NOT sc.daily_enabled OR NOT sc.is_active OR sc.daily_epoch IS DISTINCT FROM p_config_epoch
       OR NOT public.daily_0060_keys(p_strategy,ARRAY['strategy_id','strategy_version','config_json','config_sha256'])
       OR p_strategy->>'strategy_id' IS DISTINCT FROM sc.strategy_id OR p_strategy->>'strategy_version' IS DISTINCT FROM sc.strategy_version
       OR p_strategy->'config_json' IS DISTINCT FROM sc.config_json OR NOT public.daily_0060_hash(p_strategy->'config_sha256')
    THEN
        RETURN QUERY SELECT NULL::uuid,NULL::uuid,'BLOCKED'::text;
        PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN;
    END IF;
    PERFORM public.daily_0060_source_lock(c.owner_user_id,false);
    PERFORM public.daily_0060_lock_admissions(c.owner_user_id);
    SELECT * INTO a FROM public.owner_beta_daily_admissions WHERE admission_sha256=p_admission_sha256 AND owner_user_id=c.owner_user_id;
    expected:=public.daily_0060_expected(c);
    IF expected IS NULL OR expected IS DISTINCT FROM p_expected_session OR expected<c.first_auto_session OR a.range_end IS DISTINCT FROM expected
       OR NOT public.daily_0060_admission_usable(c,p_admission_sha256,true) THEN
        RETURN QUERY SELECT NULL::uuid,NULL::uuid,'BLOCKED'::text;
        PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN;
    END IF;
    SELECT count(*) INTO n FROM public.jobs WHERE owner_user_id=c.owner_user_id AND status IN ('QUEUED','RUNNING');
    IF n>=c.max_jobs_per_owner THEN
        RETURN QUERY SELECT NULL::uuid,NULL::uuid,'DEFERRED'::text;
        PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN;
    END IF;
    r.id:=gen_random_uuid(); r.job_id:=gen_random_uuid(); r.owner_user_id:=c.owner_user_id; r.strategy_config_id:=sc.id;
    r.as_of:=expected; r.input_kind:='owner_beta_daily_etf11_price_only_v1'; r.trigger_kind:='DAILY';
    r.strategy_id:=sc.strategy_id; r.strategy_version:=sc.strategy_version; r.strategy_config_json:=sc.config_json; r.strategy_config_sha256:=p_strategy->>'config_sha256';
    r.daily_pins:=jsonb_build_object('content_sha256',a.content_sha256,'manifest_sha256',a.manifest_sha256,'admission_sha256',a.admission_sha256,'policy_sha256',a.policy_sha256);
    r.daily_config_epoch:=sc.daily_epoch; r.daily_activation_epoch:=c.activation_epoch; r.daily_trigger_version:=c.trigger_version;
    INSERT INTO public.jobs(id,owner_user_id,job_type,payload_json,idempotency_key,max_attempts)
       VALUES(r.job_id,c.owner_user_id,'owner_beta_price_recommendation',public.daily_0060_payload(r),
              'owner-beta:daily:v1:'||c.owner_user_id::text||':'||sc.id::text||':'||expected::text,3);
    INSERT INTO public.owner_beta_recommendation_runs(id,owner_user_id,strategy_config_id,job_id,as_of,input_kind,trigger_kind,
       strategy_id,strategy_version,strategy_config_json,strategy_config_sha256,daily_pins,daily_config_epoch,daily_activation_epoch,daily_trigger_version)
       VALUES(r.id,c.owner_user_id,sc.id,r.job_id,expected,r.input_kind,r.trigger_kind,sc.strategy_id,sc.strategy_version,sc.config_json,r.strategy_config_sha256,
              r.daily_pins,r.daily_config_epoch,r.daily_activation_epoch,r.daily_trigger_version);
    INSERT INTO public.owner_beta_daily_reservations VALUES(c.owner_user_id,sc.id,expected,c.trigger_version,r.id,r.job_id,a.admission_sha256,
        r.strategy_config_sha256,sc.daily_epoch,c.activation_epoch,clock_timestamp());
    RETURN QUERY SELECT r.id,r.job_id,'RESERVED'::text;
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.daily_0060_gate(p_run_id uuid) RETURNS public.owner_beta_daily_publication_claims
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp
AS $$ DECLARE c public.owner_beta_daily_control; r public.owner_beta_recommendation_runs; j public.jobs; a public.job_attempts;
    sc public.user_strategy_configs; claim public.owner_beta_daily_publication_claims; expected date;
BEGIN
    -- The enclosing definer derives its actor from control before entering here.
    SELECT * INTO j FROM public.jobs WHERE id=(SELECT job_id FROM public.owner_beta_recommendation_runs WHERE id=p_run_id) FOR UPDATE;
    IF NOT FOUND THEN RAISE EXCEPTION 'daily job unavailable' USING ERRCODE='23514'; END IF;
    SELECT * INTO a FROM public.job_attempts WHERE job_id=j.id AND attempt_no=j.attempt_count FOR UPDATE;
    IF NOT FOUND THEN RAISE EXCEPTION 'daily attempt unavailable' USING ERRCODE='23514'; END IF;
    SELECT * INTO r FROM public.owner_beta_recommendation_runs WHERE id=p_run_id FOR UPDATE;
    c:=public.daily_0060_authority(r.as_of,true);
    SELECT * INTO sc FROM public.user_strategy_configs WHERE id=r.strategy_config_id AND owner_user_id=c.owner_user_id FOR SHARE;
    IF NOT FOUND OR NOT sc.is_active OR NOT sc.daily_enabled OR sc.daily_epoch IS DISTINCT FROM r.daily_config_epoch
       OR ROW(sc.strategy_id,sc.strategy_version,sc.config_json) IS DISTINCT FROM ROW(r.strategy_id,r.strategy_version,r.strategy_config_json)
       OR r.daily_activation_epoch IS DISTINCT FROM c.activation_epoch OR r.daily_trigger_version IS DISTINCT FROM c.trigger_version
       OR r.daily_pins->>'policy_sha256' IS DISTINCT FROM c.policy_sha256
       OR r.owner_user_id IS DISTINCT FROM c.owner_user_id OR r.trigger_kind IS DISTINCT FROM 'DAILY' OR r.status IS DISTINCT FROM 'PENDING'
       OR j.owner_user_id IS DISTINCT FROM r.owner_user_id OR j.job_type IS DISTINCT FROM 'owner_beta_price_recommendation'
       OR j.payload_json IS DISTINCT FROM public.daily_0060_payload(r) OR j.status IS DISTINCT FROM 'RUNNING'
       OR a.outcome IS DISTINCT FROM 'RUNNING' OR a.claimed_by IS DISTINCT FROM j.locked_by OR j.locked_by IS NULL OR j.locked_at IS NULL
       OR j.max_attempts<>3 OR j.attempt_count NOT BETWEEN 1 AND 3
       OR NOT EXISTS(SELECT 1 FROM public.owner_beta_daily_reservations res WHERE res.run_id=r.id AND res.job_id=j.id AND res.admission_sha256=r.daily_pins->>'admission_sha256')
    THEN RAISE EXCEPTION 'daily publication authority' USING ERRCODE='23514'; END IF;
    PERFORM public.daily_0060_source_lock(c.owner_user_id,false);
    PERFORM public.daily_0060_lock_admissions(c.owner_user_id);
    IF NOT public.daily_0060_admission_usable(c,r.daily_pins->>'admission_sha256',true) THEN RAISE EXCEPTION 'daily admission unavailable' USING ERRCODE='23514'; END IF;
    expected:=public.daily_0060_expected(c);
    IF expected IS DISTINCT FROM r.as_of OR expected<c.first_auto_session OR j.locked_at<=clock_timestamp()-make_interval(secs=>c.lease_seconds)
    THEN RAISE EXCEPTION 'daily publication expired' USING ERRCODE='23514'; END IF;
    claim.run_id:=r.id; claim.job_id:=j.id; claim.owner_user_id:=r.owner_user_id; claim.attempt_id:=a.id; claim.attempt_no:=a.attempt_no;
    claim.claimed_by:=a.claimed_by; claim.locked_at:=j.locked_at; claim.lease_seconds:=c.lease_seconds; claim.authorized_at:=clock_timestamp();
    RETURN claim;
END $$;

CREATE FUNCTION public.lock_owner_beta_daily_run_gate(p_run_id uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE owner_id uuid; old_actor text:=current_setting('app.actor_user_id',true); BEGIN
    SELECT owner_user_id INTO owner_id FROM public.owner_beta_daily_control WHERE id='etf11';
    IF owner_id IS NULL THEN RAISE EXCEPTION 'daily control unavailable' USING ERRCODE='23514'; END IF;
    PERFORM set_config('app.actor_user_id',owner_id::text,true);
    PERFORM public.daily_0060_gate(p_run_id);
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN true;
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.daily_0060_run_guard() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE claim public.owner_beta_daily_publication_claims; j public.jobs;
    old_actor text:=current_setting('app.actor_user_id',true); BEGIN
    IF TG_OP='DELETE' THEN
        IF OLD.trigger_kind='DAILY' THEN RAISE EXCEPTION 'daily run is permanent' USING ERRCODE='42501'; END IF;
        RETURN OLD;
    END IF;
    IF TG_OP='UPDATE' AND (NEW.trigger_kind IS DISTINCT FROM OLD.trigger_kind OR (OLD.trigger_kind='DAILY' AND NEW.input_kind IS DISTINCT FROM OLD.input_kind)) THEN
        RAISE EXCEPTION 'daily discriminant is immutable' USING ERRCODE='42501'; END IF;
    IF NEW.trigger_kind<>'DAILY' THEN RETURN NEW; END IF;
    PERFORM set_config('app.actor_user_id',NEW.owner_user_id::text,true);
    IF TG_OP='UPDATE' THEN
        IF (to_jsonb(NEW)-ARRAY['status','factor_snapshot_sha256','target_snapshot_sha256','cash_weight','error_code','started_at','finished_at','updated_at'])
           IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['status','factor_snapshot_sha256','target_snapshot_sha256','cash_weight','error_code','started_at','finished_at','updated_at'])
           OR (OLD.status IN ('SUCCEEDED','FAILED','CANCELED') AND NEW IS DISTINCT FROM OLD)
        THEN RAISE EXCEPTION 'daily run is immutable' USING ERRCODE='42501'; END IF;
    ELSE
        IF NEW.status<>'PENDING' OR NEW.factor_snapshot_sha256 IS NOT NULL OR NEW.target_snapshot_sha256 IS NOT NULL OR NEW.cash_weight IS NOT NULL
           OR NEW.error_code IS NOT NULL OR NEW.started_at IS NOT NULL OR NEW.finished_at IS NOT NULL
        THEN RAISE EXCEPTION 'daily run initial state' USING ERRCODE='23514'; END IF;
    END IF;
    SELECT * INTO j FROM public.jobs WHERE id=NEW.job_id FOR SHARE;
    IF NOT FOUND OR j.owner_user_id IS DISTINCT FROM NEW.owner_user_id OR j.job_type IS DISTINCT FROM 'owner_beta_price_recommendation'
       OR j.payload_json IS DISTINCT FROM public.daily_0060_payload(NEW) OR NEW.status='RUNNING'
    THEN RAISE EXCEPTION 'daily run binding' USING ERRCODE='23514'; END IF;
    IF TG_OP='UPDATE' AND OLD.status='PENDING' AND NEW.status='SUCCEEDED' THEN
        claim:=public.daily_0060_gate(NEW.id);
        INSERT INTO public.owner_beta_daily_publication_claims SELECT (claim).*;
        NEW.finished_at:=claim.authorized_at; NEW.updated_at:=claim.authorized_at;
        NEW.started_at:=j.started_at;
    ELSIF TG_OP='UPDATE' AND OLD.status='PENDING' AND NEW.status IN ('FAILED','CANCELED') THEN
        NEW.finished_at:=clock_timestamp(); NEW.updated_at:=NEW.finished_at;
    ELSIF TG_OP='UPDATE' AND NEW IS DISTINCT FROM OLD THEN
        RAISE EXCEPTION 'daily pending mutation' USING ERRCODE='23514';
    END IF;
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN NEW;
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;
CREATE TRIGGER daily_0060_run_guard BEFORE INSERT OR UPDATE OR DELETE ON public.owner_beta_recommendation_runs
    FOR EACH ROW EXECUTE FUNCTION public.daily_0060_run_guard();

CREATE FUNCTION public.daily_0060_item_guard() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE r public.owner_beta_recommendation_runs; run_id uuid; owner_id uuid;
    old_actor text:=current_setting('app.actor_user_id',true); BEGIN
    IF TG_OP='INSERT' THEN run_id:=NEW.recommendation_run_id; owner_id:=NEW.owner_user_id;
    ELSE run_id:=OLD.recommendation_run_id; owner_id:=OLD.owner_user_id; END IF;
    PERFORM set_config('app.actor_user_id',owner_id::text,true);
    SELECT * INTO r FROM public.owner_beta_recommendation_runs WHERE id=run_id;
    IF FOUND AND r.trigger_kind='DAILY' THEN
        IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'daily items are immutable' USING ERRCODE='42501'; END IF;
        PERFORM public.daily_0060_gate(run_id);
    ELSIF TG_OP='UPDATE' AND NEW.recommendation_run_id IS DISTINCT FROM OLD.recommendation_run_id THEN
        -- Do not let a privileged mover enter a daily result through a legacy row.
        IF EXISTS(SELECT 1 FROM public.owner_beta_daily_reservations WHERE owner_beta_daily_reservations.run_id=NEW.recommendation_run_id) THEN
            RAISE EXCEPTION 'daily item identity' USING ERRCODE='42501'; END IF;
    END IF;
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
    IF TG_OP='DELETE' THEN RETURN OLD; ELSE RETURN NEW; END IF;
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;
CREATE TRIGGER daily_0060_item_guard BEFORE INSERT OR UPDATE OR DELETE ON public.owner_beta_recommendation_items
    FOR EACH ROW EXECUTE FUNCTION public.daily_0060_item_guard();

CREATE FUNCTION public.daily_0060_job_guard() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE is_daily boolean; c public.owner_beta_daily_control; a public.job_attempts;
    receipt public.owner_beta_daily_publication_claims; t timestamptz:=clock_timestamp(); live boolean; delay bigint;
BEGIN
    IF TG_OP='INSERT' THEN is_daily:=NEW.payload_json ? 'schema' AND NEW.job_type='owner_beta_price_recommendation';
    ELSE is_daily:=OLD.payload_json->>'schema'='owner-beta-daily-recommendation-v1' OR EXISTS(SELECT 1 FROM public.owner_beta_daily_reservations WHERE job_id=OLD.id); END IF;
    IF NOT COALESCE(is_daily,false) THEN
        IF TG_OP='UPDATE' AND NEW.payload_json ? 'schema' AND NEW.job_type='owner_beta_price_recommendation' THEN RAISE EXCEPTION 'daily job cannot be retagged' USING ERRCODE='23514'; END IF;
        IF TG_OP='DELETE' THEN RETURN OLD; ELSE RETURN NEW; END IF;
    END IF;
    IF TG_OP='DELETE' THEN RAISE EXCEPTION 'daily job is permanent' USING ERRCODE='42501'; END IF;
    IF NOT public.daily_0060_payload_shape(NEW.payload_json) OR NEW.job_type<>'owner_beta_price_recommendation' OR NEW.max_attempts<>3
       OR NEW.idempotency_key IS DISTINCT FROM 'owner-beta:daily:v1:'||NEW.owner_user_id::text||':'||(NEW.payload_json->>'strategy_config_id')||':'||(NEW.payload_json->>'as_of')
    THEN RAISE EXCEPTION 'daily job shape' USING ERRCODE='23514'; END IF;
    IF TG_OP='INSERT' THEN
        IF NEW.status<>'QUEUED' OR NEW.attempt_count<>0 OR NEW.locked_by IS NOT NULL OR NEW.locked_at IS NOT NULL OR NEW.started_at IS NOT NULL OR NEW.finished_at IS NOT NULL THEN
            RAISE EXCEPTION 'daily initial job' USING ERRCODE='23514'; END IF;
        NEW.available_at:=t; NEW.created_at:=t; NEW.updated_at:=t; RETURN NEW;
    END IF;
    IF ROW(NEW.id,NEW.owner_user_id,NEW.job_type,NEW.payload_json,NEW.max_attempts,NEW.idempotency_key,NEW.created_at,NEW.priority)
       IS DISTINCT FROM ROW(OLD.id,OLD.owner_user_id,OLD.job_type,OLD.payload_json,OLD.max_attempts,OLD.idempotency_key,OLD.created_at,OLD.priority)
       OR (OLD.status IN ('SUCCEEDED','FAILED') AND NEW IS DISTINCT FROM OLD)
    THEN RAISE EXCEPTION 'daily job identity is immutable' USING ERRCODE='42501'; END IF;
    IF NEW IS NOT DISTINCT FROM OLD THEN RETURN NEW; END IF;
    -- Job then attempt then control: use the same order as publication.
    SELECT * INTO a FROM public.job_attempts WHERE job_id=OLD.id AND attempt_no=OLD.attempt_count FOR UPDATE;
    SELECT * INTO c FROM public.owner_beta_daily_control WHERE owner_user_id=OLD.owner_user_id FOR SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'daily control missing' USING ERRCODE='23514'; END IF;
    t:=clock_timestamp();
    live:=OLD.locked_at IS NOT NULL AND OLD.locked_at>t-make_interval(secs=>c.lease_seconds);
    IF OLD.status='QUEUED' AND NEW.status='RUNNING' THEN
        IF NEW.attempt_count<>OLD.attempt_count+1 OR NEW.attempt_count>3 OR OLD.available_at>t OR NEW.locked_by IS NULL OR NEW.locked_by='' THEN
            RAISE EXCEPTION 'daily claim invalid' USING ERRCODE='23514'; END IF;
        NEW.locked_at:=t; NEW.started_at:=COALESCE(OLD.started_at,t); NEW.finished_at:=NULL;
        NEW.available_at:=OLD.available_at;
    ELSIF OLD.status='RUNNING' AND NEW.status='RUNNING' THEN
        IF NOT live OR a.outcome IS DISTINCT FROM 'RUNNING' OR a.claimed_by IS DISTINCT FROM OLD.locked_by
           OR NEW.locked_by IS DISTINCT FROM OLD.locked_by OR NEW.attempt_count<>OLD.attempt_count
           OR (to_jsonb(NEW)-ARRAY['locked_at','updated_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['locked_at','updated_at'])
        THEN RAISE EXCEPTION 'daily heartbeat invalid' USING ERRCODE='23514'; END IF;
        NEW.locked_at:=t;
    ELSIF OLD.status='RUNNING' AND NEW.status IN ('QUEUED','FAILED','SUCCEEDED') THEN
        IF a.id IS NULL OR a.claimed_by IS DISTINCT FROM OLD.locked_by OR NEW.attempt_count<>OLD.attempt_count
           OR NEW.locked_by IS NOT NULL OR NEW.locked_at IS NOT NULL THEN RAISE EXCEPTION 'daily settlement identity' USING ERRCODE='23514'; END IF;
        IF NEW.status='SUCCEEDED' THEN
            SELECT * INTO receipt FROM public.owner_beta_daily_publication_claims WHERE job_id=OLD.id;
            IF NOT FOUND OR NOT live OR a.outcome IS DISTINCT FROM 'RUNNING'
               OR ROW(receipt.attempt_id,receipt.attempt_no,receipt.claimed_by,receipt.locked_at)
                  IS DISTINCT FROM ROW(a.id,OLD.attempt_count,OLD.locked_by,OLD.locked_at)
            THEN RAISE EXCEPTION 'daily success receipt missing' USING ERRCODE='23514'; END IF;
            NEW.finished_at:=t;
        ELSE
            IF NOT ((live AND a.outcome IN ('RUNNING','FAILED')) OR (NOT live AND a.outcome='ORPHANED'))
               OR (NEW.status='QUEUED' AND OLD.attempt_count>=3) OR (NEW.status='FAILED' AND NOT live AND OLD.attempt_count<3)
            THEN RAISE EXCEPTION 'daily failure claim invalid' USING ERRCODE='23514'; END IF;
            IF NEW.status='QUEUED' THEN
                BEGIN delay:=c.backoff_seconds*CASE WHEN OLD.attempt_count=1 THEN 1 ELSE 2 END;
                    NEW.available_at:=t+make_interval(secs=>delay::double precision);
                EXCEPTION WHEN numeric_value_out_of_range OR datetime_field_overflow THEN RAISE EXCEPTION 'daily backoff overflow' USING ERRCODE='23514'; END;
                NEW.finished_at:=NULL;
            ELSE NEW.finished_at:=t; END IF;
        END IF;
        NEW.started_at:=OLD.started_at;
    ELSIF OLD.status IN ('QUEUED','RUNNING') AND NEW.status='CANCELED' THEN
        IF NEW.attempt_count<>OLD.attempt_count OR NEW.locked_by IS DISTINCT FROM OLD.locked_by OR NEW.locked_at IS DISTINCT FROM OLD.locked_at THEN
            RAISE EXCEPTION 'daily cancel identity' USING ERRCODE='23514'; END IF;
        NEW.finished_at:=t; NEW.started_at:=OLD.started_at; NEW.available_at:=OLD.available_at;
    ELSIF OLD.status='CANCELED' AND NEW.status='CANCELED' THEN
        IF OLD.locked_by IS NULL OR OLD.locked_at IS NULL OR NEW.locked_by IS NOT NULL OR NEW.locked_at IS NOT NULL
           OR a.claimed_by IS DISTINCT FROM OLD.locked_by OR NOT ((live AND a.outcome IN ('RUNNING','FAILED')) OR (NOT live AND a.outcome='ORPHANED'))
           OR (to_jsonb(NEW)-ARRAY['locked_by','locked_at','updated_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['locked_by','locked_at','updated_at'])
        THEN RAISE EXCEPTION 'daily canceled cleanup invalid' USING ERRCODE='23514'; END IF;
    ELSE RAISE EXCEPTION 'daily job transition invalid' USING ERRCODE='23514'; END IF;
    NEW.updated_at:=t;
    RETURN NEW;
END $$;
CREATE TRIGGER daily_0060_job_guard BEFORE INSERT OR UPDATE OR DELETE ON public.jobs
    FOR EACH ROW EXECUTE FUNCTION public.daily_0060_job_guard();

CREATE FUNCTION public.daily_0060_attempt_guard() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE j public.jobs; jid uuid; owner_id uuid; t timestamptz:=clock_timestamp(); c public.owner_beta_daily_control;
    live boolean; old_actor text:=current_setting('app.actor_user_id',true); receipt public.owner_beta_daily_publication_claims;
BEGIN
    IF TG_OP='INSERT' THEN jid:=NEW.job_id; ELSE jid:=OLD.job_id; END IF;
    SELECT owner_user_id INTO owner_id FROM public.owner_beta_daily_control WHERE id='etf11';
    IF owner_id IS NULL THEN IF TG_OP='DELETE' THEN RETURN OLD; ELSE RETURN NEW; END IF; END IF;
    PERFORM set_config('app.actor_user_id',owner_id::text,true);
    SELECT * INTO j FROM public.jobs WHERE id=jid FOR UPDATE;
    IF NOT FOUND OR j.payload_json->>'schema' IS DISTINCT FROM 'owner-beta-daily-recommendation-v1' THEN
        PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
        IF TG_OP='DELETE' THEN RETURN OLD; ELSE RETURN NEW; END IF;
    END IF;
    IF TG_OP='DELETE' THEN RAISE EXCEPTION 'daily attempt is permanent' USING ERRCODE='42501'; END IF;
    SELECT * INTO c FROM public.owner_beta_daily_control WHERE id='etf11' FOR SHARE;
    t:=clock_timestamp();
    live:=j.locked_at IS NOT NULL AND j.locked_at>t-make_interval(secs=>c.lease_seconds);
    IF TG_OP='INSERT' THEN
        IF j.status<>'RUNNING' OR NEW.outcome<>'RUNNING' OR NEW.attempt_no<>j.attempt_count
           OR NEW.claimed_by IS DISTINCT FROM j.locked_by OR NEW.claimed_by IS NULL OR NEW.finished_at IS NOT NULL OR NOT live
        THEN RAISE EXCEPTION 'daily attempt initial state' USING ERRCODE='23514'; END IF;
        NEW.started_at:=j.locked_at; NEW.created_at:=t;
    ELSE
        IF ROW(NEW.id,NEW.job_id,NEW.attempt_no,NEW.claimed_by,NEW.started_at,NEW.created_at)
           IS DISTINCT FROM ROW(OLD.id,OLD.job_id,OLD.attempt_no,OLD.claimed_by,OLD.started_at,OLD.created_at)
           OR (OLD.outcome<>'RUNNING' AND NEW IS DISTINCT FROM OLD) THEN
            RAISE EXCEPTION 'daily attempt is immutable' USING ERRCODE='42501'; END IF;
        IF NEW IS NOT DISTINCT FROM OLD THEN
            PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN NEW;
        END IF;
        IF NEW.attempt_no<>j.attempt_count THEN RAISE EXCEPTION 'daily stale attempt' USING ERRCODE='23514'; END IF;
        IF NEW.outcome='ORPHANED' THEN
            IF j.status NOT IN ('RUNNING','CANCELED') OR live OR j.locked_at IS NULL OR j.locked_by IS DISTINCT FROM OLD.claimed_by THEN
                RAISE EXCEPTION 'daily orphan claim invalid' USING ERRCODE='23514'; END IF;
        ELSIF NEW.outcome='SUCCEEDED' THEN
            SELECT * INTO receipt FROM public.owner_beta_daily_publication_claims WHERE job_id=j.id;
            IF NOT FOUND OR j.status<>'SUCCEEDED' OR receipt.attempt_id<>OLD.id OR receipt.claimed_by IS DISTINCT FROM OLD.claimed_by THEN
                RAISE EXCEPTION 'daily attempt success denied' USING ERRCODE='23514'; END IF;
        ELSIF NEW.outcome='FAILED' THEN
            -- A job-first settlement clears its lease. A locked job in one of
            -- these states with its current attempt still RUNNING cannot have
            -- committed: the deferred matrix forbids that intermediate state.
            -- The job guard already checked this attempt's immutable identity.
            IF NOT ((j.status IN ('RUNNING','CANCELED') AND live AND j.locked_by=OLD.claimed_by)
                OR (j.status IN ('QUEUED','FAILED','CANCELED') AND j.locked_by IS NULL AND j.locked_at IS NULL
                    AND OLD.outcome='RUNNING')) THEN
                RAISE EXCEPTION 'daily attempt failure denied' USING ERRCODE='23514'; END IF;
        ELSE RAISE EXCEPTION 'daily attempt transition invalid' USING ERRCODE='23514'; END IF;
        NEW.finished_at:=t;
    END IF;
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN NEW;
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;
CREATE TRIGGER daily_0060_attempt_guard BEFORE INSERT OR UPDATE OR DELETE ON public.job_attempts
    FOR EACH ROW EXECUTE FUNCTION public.daily_0060_attempt_guard();

CREATE FUNCTION public.daily_0060_consistency() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$ DECLARE row_data jsonb; jid uuid; owner_id uuid; j public.jobs; r public.owner_beta_recommendation_runs;
    res public.owner_beta_daily_reservations; claim public.owner_beta_daily_publication_claims; a public.job_attempts;
    admitted public.owner_beta_daily_admissions; item_count integer; running integer; weight numeric; item_ids jsonb;
    old_actor text:=current_setting('app.actor_user_id',true);
BEGIN
    IF TG_OP='DELETE' THEN row_data:=to_jsonb(OLD); ELSE row_data:=to_jsonb(NEW); END IF;
    -- Unrelated queue traffic must not acquire daily authority or lock job rows.
    -- The migration-owned reservation is permanent and its internal policy sees
    -- every tenant, so a reserved job cannot evade checks by losing its tag.
    IF TG_TABLE_NAME='jobs'
       AND row_data#>>'{payload_json,schema}' IS DISTINCT FROM 'owner-beta-daily-recommendation-v1'
       AND NOT EXISTS(SELECT 1 FROM public.owner_beta_daily_reservations WHERE job_id=(row_data->>'id')::uuid)
    THEN RETURN NULL; END IF;
    owner_id:=(row_data->>'owner_user_id')::uuid;
    IF owner_id IS NULL THEN SELECT owner_user_id INTO owner_id FROM public.owner_beta_daily_control WHERE id='etf11'; END IF;
    IF owner_id IS NULL THEN RETURN NULL; END IF;
    PERFORM set_config('app.actor_user_id',owner_id::text,true);
    CASE TG_TABLE_NAME
      WHEN 'jobs' THEN jid:=(row_data->>'id')::uuid;
      WHEN 'owner_beta_recommendation_items' THEN
        SELECT job_id INTO jid FROM public.owner_beta_recommendation_runs WHERE id=(row_data->>'recommendation_run_id')::uuid;
      ELSE jid:=(row_data->>'job_id')::uuid;
    END CASE;
    SELECT * INTO j FROM public.jobs WHERE id=jid FOR UPDATE;
    IF NOT FOUND THEN
        IF TG_TABLE_NAME='jobs' AND row_data#>>'{payload_json,schema}'='owner-beta-daily-recommendation-v1' THEN
            RAISE EXCEPTION 'daily missing job' USING ERRCODE='23514'; END IF;
        PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN NULL;
    END IF;
    SELECT * INTO res FROM public.owner_beta_daily_reservations WHERE job_id=j.id;
    IF j.payload_json->>'schema' IS DISTINCT FROM 'owner-beta-daily-recommendation-v1' AND res.job_id IS NULL THEN
        PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN NULL;
    END IF;
    SELECT * INTO a FROM public.job_attempts WHERE job_id=j.id AND attempt_no=j.attempt_count FOR UPDATE;
    SELECT * INTO r FROM public.owner_beta_recommendation_runs WHERE job_id=j.id FOR UPDATE;
    IF r.id IS NULL OR res.run_id IS DISTINCT FROM r.id OR res.owner_user_id IS DISTINCT FROM r.owner_user_id OR j.owner_user_id IS DISTINCT FROM r.owner_user_id
       OR r.trigger_kind IS DISTINCT FROM 'DAILY' OR res.strategy_config_id IS DISTINCT FROM r.strategy_config_id OR res.session_date IS DISTINCT FROM r.as_of
       OR res.trigger_version IS DISTINCT FROM r.daily_trigger_version OR res.config_epoch IS DISTINCT FROM r.daily_config_epoch
       OR res.activation_epoch IS DISTINCT FROM r.daily_activation_epoch OR res.strategy_config_sha256 IS DISTINCT FROM r.strategy_config_sha256
       OR res.admission_sha256 IS DISTINCT FROM r.daily_pins->>'admission_sha256' OR j.payload_json IS DISTINCT FROM public.daily_0060_payload(r)
       OR j.job_type IS DISTINCT FROM 'owner_beta_price_recommendation' OR j.max_attempts<>3 OR j.attempt_count NOT BETWEEN 0 AND 3
    THEN RAISE EXCEPTION 'daily reservation consistency' USING ERRCODE='23514'; END IF;
    SELECT * INTO admitted FROM public.owner_beta_daily_admissions WHERE admission_sha256=res.admission_sha256;
    IF admitted.owner_user_id IS DISTINCT FROM r.owner_user_id OR admitted.range_end IS DISTINCT FROM r.as_of OR r.daily_pins IS DISTINCT FROM
       jsonb_build_object('admission_sha256',admitted.admission_sha256,'content_sha256',admitted.content_sha256,'manifest_sha256',admitted.manifest_sha256,'policy_sha256',admitted.policy_sha256)
    THEN RAISE EXCEPTION 'daily admission pin consistency' USING ERRCODE='23514'; END IF;
    SELECT count(*) INTO running FROM public.job_attempts WHERE job_id=j.id AND outcome='RUNNING';
    IF (SELECT count(*) FROM public.job_attempts WHERE job_id=j.id)<>j.attempt_count
       OR EXISTS(SELECT 1 FROM public.job_attempts WHERE job_id=j.id AND (attempt_no<1 OR attempt_no>j.attempt_count)) THEN
        RAISE EXCEPTION 'daily attempt sequence' USING ERRCODE='23514'; END IF;
    SELECT * INTO claim FROM public.owner_beta_daily_publication_claims WHERE job_id=j.id;
    SELECT count(*),COALESCE(sum(target_weight),0),jsonb_agg(instrument_id ORDER BY instrument_id)
       INTO item_count,weight,item_ids FROM public.owner_beta_recommendation_items WHERE recommendation_run_id=r.id;
    IF j.status<>'SUCCEEDED' AND (item_count<>0 OR claim.run_id IS NOT NULL OR r.factor_snapshot_sha256 IS NOT NULL OR r.target_snapshot_sha256 IS NOT NULL OR r.cash_weight IS NOT NULL) THEN
        RAISE EXCEPTION 'daily premature output' USING ERRCODE='23514'; END IF;
    CASE j.status
    WHEN 'QUEUED' THEN
        IF r.status<>'PENDING' OR running<>0 OR j.locked_by IS NOT NULL OR j.locked_at IS NOT NULL OR j.finished_at IS NOT NULL
           OR (j.attempt_count=0 AND a.id IS NOT NULL) OR (j.attempt_count>0 AND (a.outcome IS NULL OR a.outcome NOT IN ('FAILED','ORPHANED') OR j.attempt_count>=3))
        THEN RAISE EXCEPTION 'daily queued consistency' USING ERRCODE='23514'; END IF;
    WHEN 'RUNNING' THEN
        IF r.status<>'PENDING' OR running<>1 OR a.outcome IS DISTINCT FROM 'RUNNING' OR a.claimed_by IS DISTINCT FROM j.locked_by
           OR j.locked_at IS NULL OR j.locked_by IS NULL OR j.finished_at IS NOT NULL THEN
            RAISE EXCEPTION 'daily running consistency' USING ERRCODE='23514'; END IF;
    WHEN 'SUCCEEDED' THEN
        IF r.status<>'SUCCEEDED' OR running<>0 OR a.outcome IS DISTINCT FROM 'SUCCEEDED' OR claim.run_id IS DISTINCT FROM r.id
           OR ROW(claim.owner_user_id,claim.attempt_id,claim.attempt_no,claim.claimed_by) IS DISTINCT FROM ROW(r.owner_user_id,a.id,j.attempt_count,a.claimed_by)
           OR r.finished_at IS DISTINCT FROM claim.authorized_at OR r.updated_at IS DISTINCT FROM claim.authorized_at
           OR j.finished_at IS NULL OR a.finished_at IS NULL OR j.finished_at<claim.authorized_at OR a.finished_at<j.finished_at
           OR j.locked_by IS NOT NULL OR j.locked_at IS NOT NULL OR item_count<>11
           OR item_ids IS DISTINCT FROM admitted.evidence_json#>'{manifest,instruments}' OR weight+r.cash_weight IS DISTINCT FROM 1::numeric
           OR EXISTS(SELECT 1 FROM public.owner_beta_recommendation_items i WHERE i.recommendation_run_id=r.id
              AND (i.target_weight IS NULL OR (i.excluded AND (i.target_weight<>0 OR i.rank IS NOT NULL OR i.exclusion_reason IS NULL OR i.exclusion_reason=''))
                   OR (NOT i.excluded AND (i.rank IS NULL OR i.exclusion_reason IS NOT NULL))))
           OR EXISTS(SELECT 1 FROM public.owner_beta_recommendation_items i WHERE i.recommendation_run_id=r.id AND NOT i.excluded GROUP BY rank HAVING count(*)<>1)
        THEN RAISE EXCEPTION 'daily success consistency' USING ERRCODE='23514'; END IF;
    WHEN 'FAILED' THEN
        IF r.status<>'FAILED' OR running<>0 OR a.outcome IS NULL OR a.outcome NOT IN ('FAILED','ORPHANED') OR j.finished_at IS NULL OR j.locked_by IS NOT NULL OR j.locked_at IS NOT NULL THEN
            RAISE EXCEPTION 'daily failed consistency' USING ERRCODE='23514'; END IF;
    WHEN 'CANCELED' THEN
        IF r.status NOT IN ('PENDING','CANCELED') OR running>1 OR j.finished_at IS NULL
           OR (running=1 AND (a.outcome IS DISTINCT FROM 'RUNNING' OR a.claimed_by IS DISTINCT FROM j.locked_by OR j.locked_at IS NULL OR r.status<>'PENDING'))
           OR (running=0 AND (j.locked_by IS NOT NULL OR j.locked_at IS NOT NULL OR (a.id IS NOT NULL AND a.outcome NOT IN ('FAILED','ORPHANED'))))
        THEN RAISE EXCEPTION 'daily cancel consistency' USING ERRCODE='23514'; END IF;
    ELSE RAISE EXCEPTION 'daily final state' USING ERRCODE='23514'; END CASE;
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN NULL;
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE CONSTRAINT TRIGGER daily_0060_consistency AFTER INSERT OR UPDATE OR DELETE ON public.jobs
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.daily_0060_consistency();
CREATE CONSTRAINT TRIGGER daily_0060_consistency AFTER INSERT OR UPDATE OR DELETE ON public.job_attempts
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.daily_0060_consistency();
CREATE CONSTRAINT TRIGGER daily_0060_consistency AFTER INSERT OR UPDATE OR DELETE ON public.owner_beta_recommendation_runs
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.daily_0060_consistency();
CREATE CONSTRAINT TRIGGER daily_0060_consistency AFTER INSERT OR UPDATE OR DELETE ON public.owner_beta_recommendation_items
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.daily_0060_consistency();
CREATE CONSTRAINT TRIGGER daily_0060_consistency AFTER INSERT OR UPDATE OR DELETE ON public.owner_beta_daily_reservations
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.daily_0060_consistency();
CREATE CONSTRAINT TRIGGER daily_0060_consistency AFTER INSERT OR UPDATE OR DELETE ON public.owner_beta_daily_publication_claims
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.daily_0060_consistency();

-- Exact historical branches from 0050/0052, preceded only by daily dispatch.
CREATE OR REPLACE FUNCTION public.owner_beta_recommendation_runs_validate_job_binding()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $binding$
DECLARE
    v_config_strategy_id text;
    v_config_strategy_version text;
    v_config_json jsonb;
    v_job_owner_user_id uuid;
    v_job_type text;
    v_payload jsonb;
    v_top_level_keys text[];
    v_pin_keys text[];
    v_strategy_keys text[];
BEGIN
    -- Daily validation is in daily_0060_run_guard; legacy branch below is unchanged.
    IF NEW.trigger_kind = 'DAILY' THEN RETURN NEW; END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.id IS DISTINCT FROM OLD.id
            OR NEW.owner_user_id IS DISTINCT FROM OLD.owner_user_id
            OR NEW.strategy_config_id IS DISTINCT FROM OLD.strategy_config_id
            OR NEW.strategy_id IS DISTINCT FROM OLD.strategy_id
            OR NEW.strategy_version IS DISTINCT FROM OLD.strategy_version
            OR NEW.strategy_config_json IS DISTINCT FROM OLD.strategy_config_json
            OR NEW.strategy_config_sha256 IS DISTINCT FROM OLD.strategy_config_sha256
            OR NEW.job_id IS DISTINCT FROM OLD.job_id
            OR NEW.as_of IS DISTINCT FROM OLD.as_of
            OR NEW.input_kind IS DISTINCT FROM OLD.input_kind
            OR NEW.capability IS DISTINCT FROM OLD.capability
            OR NEW.audience IS DISTINCT FROM OLD.audience
            OR NEW.vendor_snapshot IS DISTINCT FROM OLD.vendor_snapshot
            OR NEW.strict_pit IS DISTINCT FROM OLD.strict_pit
            OR NEW.candidate_content_sha256 IS DISTINCT FROM OLD.candidate_content_sha256
            OR NEW.artifact_manifest_sha256 IS DISTINCT FROM OLD.artifact_manifest_sha256
            OR NEW.stage5_manifest_sha256 IS DISTINCT FROM OLD.stage5_manifest_sha256
            OR NEW.action_manifest_sha256 IS DISTINCT FROM OLD.action_manifest_sha256
            OR NEW.approval_registry_sha256 IS DISTINCT FROM OLD.approval_registry_sha256
        THEN
            RAISE EXCEPTION 'owner beta recommendation run identity is immutable'
                USING ERRCODE = '42501';
        END IF;
    ELSE
        SELECT config.strategy_id, config.strategy_version, config.config_json
          INTO v_config_strategy_id, v_config_strategy_version, v_config_json
          FROM public.user_strategy_configs AS config
         WHERE config.id = NEW.strategy_config_id
           AND config.owner_user_id = NEW.owner_user_id
           AND config.is_active
         FOR SHARE OF config;
        IF NOT FOUND
           OR v_config_strategy_id IS DISTINCT FROM NEW.strategy_id
           OR v_config_strategy_version IS DISTINCT FROM NEW.strategy_version
           OR v_config_json IS DISTINCT FROM NEW.strategy_config_json
        THEN
            RAISE EXCEPTION 'owner beta recommendation binding is invalid'
                USING ERRCODE = '23514';
        END IF;
    END IF;

    SELECT job.owner_user_id, job.job_type, job.payload_json
      INTO v_job_owner_user_id, v_job_type, v_payload
      FROM public.jobs AS job
     WHERE job.id = NEW.job_id
     FOR SHARE OF job;
    IF NOT FOUND
       OR v_job_owner_user_id IS DISTINCT FROM NEW.owner_user_id
       OR v_job_type IS DISTINCT FROM 'owner_beta_price_recommendation'
       OR pg_catalog.jsonb_typeof(v_payload) IS DISTINCT FROM 'object'
    THEN
        RAISE EXCEPTION 'owner beta recommendation binding is invalid'
            USING ERRCODE = '23514';
    END IF;

    SELECT pg_catalog.array_agg(payload_key ORDER BY payload_key)
      INTO v_top_level_keys
      FROM pg_catalog.jsonb_object_keys(v_payload) AS payload_keys(payload_key);
    IF v_top_level_keys IS DISTINCT FROM ARRAY[
        'as_of', 'pins', 'run_id', 'strategy', 'strategy_config_id'
    ]::text[]
       OR pg_catalog.jsonb_typeof(v_payload -> 'run_id') IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(v_payload -> 'strategy_config_id') IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(v_payload -> 'as_of') IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(v_payload -> 'pins') IS DISTINCT FROM 'object'
       OR pg_catalog.jsonb_typeof(v_payload -> 'strategy') IS DISTINCT FROM 'object'
    THEN
        RAISE EXCEPTION 'owner beta recommendation binding is invalid'
            USING ERRCODE = '23514';
    END IF;

    SELECT pg_catalog.array_agg(pin_key ORDER BY pin_key)
      INTO v_pin_keys
      FROM pg_catalog.jsonb_object_keys(v_payload -> 'pins') AS pin_keys(pin_key);
    IF v_pin_keys IS DISTINCT FROM ARRAY[
        'action_manifest_sha256',
        'approval_registry_sha256',
        'artifact_manifest_sha256',
        'candidate_content_sha256',
        'stage5_manifest_sha256'
    ]::text[]
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'pins' -> 'candidate_content_sha256'
          ) IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'pins' -> 'artifact_manifest_sha256'
          ) IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'pins' -> 'stage5_manifest_sha256'
          ) IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'pins' -> 'action_manifest_sha256'
          ) IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'pins' -> 'approval_registry_sha256'
          ) IS DISTINCT FROM 'string'
       OR v_payload ->> 'run_id' IS DISTINCT FROM NEW.id::text
       OR v_payload ->> 'strategy_config_id' IS DISTINCT FROM NEW.strategy_config_id::text
       OR v_payload ->> 'as_of' IS DISTINCT FROM pg_catalog.to_char(NEW.as_of, 'YYYY-MM-DD')
       OR v_payload -> 'pins' ->> 'candidate_content_sha256'
            IS DISTINCT FROM NEW.candidate_content_sha256
       OR v_payload -> 'pins' ->> 'artifact_manifest_sha256'
            IS DISTINCT FROM NEW.artifact_manifest_sha256
       OR v_payload -> 'pins' ->> 'stage5_manifest_sha256'
            IS DISTINCT FROM NEW.stage5_manifest_sha256
       OR v_payload -> 'pins' ->> 'action_manifest_sha256'
            IS DISTINCT FROM NEW.action_manifest_sha256
       OR v_payload -> 'pins' ->> 'approval_registry_sha256'
            IS DISTINCT FROM NEW.approval_registry_sha256
    THEN
        RAISE EXCEPTION 'owner beta recommendation binding is invalid'
            USING ERRCODE = '23514';
    END IF;

    SELECT pg_catalog.array_agg(strategy_key ORDER BY strategy_key)
      INTO v_strategy_keys
      FROM pg_catalog.jsonb_object_keys(v_payload -> 'strategy') AS strategy_keys(strategy_key);
    IF v_strategy_keys IS DISTINCT FROM ARRAY[
        'config_json', 'config_sha256', 'strategy_id', 'strategy_version'
    ]::text[]
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'strategy' -> 'strategy_id'
          ) IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'strategy' -> 'strategy_version'
          ) IS DISTINCT FROM 'string'
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'strategy' -> 'config_json'
          ) IS DISTINCT FROM 'object'
       OR pg_catalog.jsonb_typeof(
            v_payload -> 'strategy' -> 'config_sha256'
          ) IS DISTINCT FROM 'string'
       OR v_payload -> 'strategy' ->> 'strategy_id'
            IS DISTINCT FROM NEW.strategy_id
       OR v_payload -> 'strategy' ->> 'strategy_version'
            IS DISTINCT FROM NEW.strategy_version
       OR v_payload -> 'strategy' -> 'config_json'
            IS DISTINCT FROM NEW.strategy_config_json
       OR v_payload -> 'strategy' ->> 'config_sha256'
            IS DISTINCT FROM NEW.strategy_config_sha256
    THEN
        RAISE EXCEPTION 'owner beta recommendation binding is invalid'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END
$binding$;

ALTER FUNCTION public.owner_beta_recommendation_runs_validate_job_binding()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_recommendation_runs_validate_job_binding()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

DROP TRIGGER owner_beta_recommendation_runs_validate_job_binding
    ON public.owner_beta_recommendation_runs;
CREATE TRIGGER owner_beta_recommendation_runs_validate_job_binding
    BEFORE INSERT OR UPDATE OF
        id, owner_user_id, strategy_config_id, strategy_id, strategy_version,
        strategy_config_json, strategy_config_sha256, job_id, as_of,
        trigger_kind, daily_pins, daily_config_epoch, daily_activation_epoch, daily_trigger_version,
        input_kind, capability, audience, vendor_snapshot, strict_pit,
        candidate_content_sha256, artifact_manifest_sha256,
        stage5_manifest_sha256, action_manifest_sha256, approval_registry_sha256
    ON public.owner_beta_recommendation_runs
    FOR EACH ROW
    EXECUTE FUNCTION public.owner_beta_recommendation_runs_validate_job_binding();

CREATE OR REPLACE FUNCTION public.owner_beta_recommendation_runs_lock_strategy_config_on_success()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $lock$
BEGIN
    -- Daily success is authorized and stamped by daily_0060_run_guard.
    IF NEW.trigger_kind = 'DAILY' THEN RETURN NEW; END IF;
    IF NEW.status IS DISTINCT FROM 'SUCCEEDED'
       OR OLD.status IS NOT DISTINCT FROM 'SUCCEEDED'
    THEN
        RETURN NEW;
    END IF;

    PERFORM pg_catalog.set_config('app.actor_user_id', NEW.owner_user_id::text, true);
    PERFORM 1
      FROM public.user_strategy_configs AS config
     WHERE config.id = NEW.strategy_config_id
       AND config.owner_user_id = NEW.owner_user_id
       AND config.is_active
       AND config.strategy_id = NEW.strategy_id
       AND config.strategy_version = NEW.strategy_version
       AND config.config_json = NEW.strategy_config_json
     FOR SHARE OF config;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'owner beta recommendation strategy config is unavailable'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END
$lock$;

ALTER FUNCTION public.owner_beta_recommendation_runs_lock_strategy_config_on_success()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_recommendation_runs_lock_strategy_config_on_success()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

-- Literal migration-owned object sets. No serving function executes dynamic SQL.
DO $security$
DECLARE t text; f record; BEGIN
    FOREACH t IN ARRAY ARRAY['control','admissions','admission_events','reservations','poll_state','calendar_sources','calendar_days','source_events','publication_claims','poll_receipts'] LOOP
        EXECUTE format('ALTER TABLE public.owner_beta_daily_%I OWNER TO migration_owner',t);
        EXECUTE format('ALTER TABLE public.owner_beta_daily_%I ENABLE ROW LEVEL SECURITY',t);
        EXECUTE format('ALTER TABLE public.owner_beta_daily_%I FORCE ROW LEVEL SECURITY',t);
        EXECUTE format('REVOKE ALL ON public.owner_beta_daily_%I FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter',t);
        EXECUTE format('CREATE POLICY daily_0060_internal ON public.owner_beta_daily_%I FOR ALL TO migration_owner USING (true) WITH CHECK (true)',t);
        IF t<>'publication_claims' THEN
            EXECUTE format('GRANT SELECT ON public.owner_beta_daily_%I TO app',t);
            IF t='poll_state' THEN
                EXECUTE format('CREATE POLICY daily_0060_owner_read ON public.owner_beta_daily_%I FOR SELECT TO app USING (EXISTS(SELECT 1 FROM public.owner_beta_daily_control c JOIN public.user_roles ur ON ur.user_id=c.owner_user_id AND ur.role_id=''owner'' WHERE c.owner_user_id=NULLIF(current_setting(''app.actor_user_id'',true),'''')::uuid))',t);
            ELSE
                EXECUTE format('CREATE POLICY daily_0060_owner_read ON public.owner_beta_daily_%I FOR SELECT TO app USING (owner_user_id=NULLIF(current_setting(''app.actor_user_id'',true),'''')::uuid AND EXISTS(SELECT 1 FROM public.user_roles ur WHERE ur.user_id=owner_user_id AND ur.role_id=''owner''))',t);
            END IF;
        END IF;
        IF t IN ('admissions','admission_events','reservations','calendar_sources','calendar_days','source_events','publication_claims') THEN
            EXECUTE format('CREATE TRIGGER daily_0060_immutable BEFORE UPDATE OR DELETE ON public.owner_beta_daily_%I FOR EACH ROW EXECUTE FUNCTION public.daily_0060_immutable()',t);
        END IF;
    END LOOP;
    FOR f IN SELECT p.oid::regprocedure AS signature FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
      WHERE n.nspname='public' AND (p.proname LIKE 'daily\_0060\_%' ESCAPE '\'
        OR p.proname=ANY(ARRAY['take_owner_beta_daily_admission_poll_slot','take_owner_beta_daily_schedule_poll_slot','project_owner_beta_daily_calendar','admit_owner_beta_daily_input','block_owner_beta_daily_input','finish_owner_beta_daily_admission_poll','finish_owner_beta_daily_schedule_poll','read_owner_beta_daily_source_evidence','owner_beta_daily_schedule_candidates','reserve_owner_beta_daily_run','lock_owner_beta_daily_run_gate','revoke_owner_beta_daily_input'])) LOOP
        EXECUTE format('ALTER FUNCTION %s OWNER TO migration_owner',f.signature);
        EXECUTE format('ALTER FUNCTION %s SET search_path=pg_catalog,pg_temp',f.signature);
        EXECUTE format('ALTER FUNCTION %s SET lock_timeout=''5s''',f.signature);
        EXECUTE format('ALTER FUNCTION %s SET statement_timeout=''30s''',f.signature);
        EXECUTE format('REVOKE ALL ON FUNCTION %s FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter',f.signature);
    END LOOP;
END $security$;

GRANT EXECUTE ON FUNCTION public.take_owner_beta_daily_admission_poll_slot(),
    public.project_owner_beta_daily_calendar(uuid,text,jsonb,jsonb),
    public.admit_owner_beta_daily_input(uuid,jsonb),public.block_owner_beta_daily_input(uuid,jsonb),
    public.finish_owner_beta_daily_admission_poll(uuid,text,text),
    public.read_owner_beta_daily_source_evidence(date,date) TO daily_input_admitter;
GRANT EXECUTE ON FUNCTION public.take_owner_beta_daily_schedule_poll_slot(),
    public.finish_owner_beta_daily_schedule_poll(uuid,text),public.owner_beta_daily_schedule_candidates(),
    public.reserve_owner_beta_daily_run(uuid,date,text,bigint,jsonb),
    public.lock_owner_beta_daily_run_gate(uuid) TO worker;
-- revoke_owner_beta_daily_input remains migration/release authority only.
