-- R5 WP5 / P1: protected lifecycle journals and exact offline SQL readback.
-- 0067 stays immutable. Original configure/transition definitions are retained
-- privately for pristine down; successful 0068 INSERTs carry atomic receipts.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '15s';

CREATE TABLE public.owner_beta_paper_operations (
  generation uuid PRIMARY KEY CHECK(generation<>'00000000-0000-0000-0000-000000000000'::uuid),
  installation_id uuid NOT NULL CHECK(installation_id<>'00000000-0000-0000-0000-000000000000'::uuid),
  account_id uuid NOT NULL REFERENCES public.accounts(id) ON DELETE RESTRICT,
  request_key text NOT NULL CHECK(request_key ~ '^[A-Za-z0-9._:-]{1,128}$'),
  purpose text NOT NULL CHECK(purpose IN ('STOP','BACKUP','RESTORE')),
  scope text NOT NULL CHECK(scope IN ('PAPER_STOP','COMBINED_BACKUP')),
  expected_epoch bigint NOT NULL CHECK(expected_epoch>=0),
  local_epoch_floor bigint NOT NULL CHECK(local_epoch_floor>=0),
  fence_epoch bigint NOT NULL CHECK(fence_epoch>0),
  release_manifest_sha256 text NOT NULL CHECK(release_manifest_sha256 ~ '^sha256:[0-9a-f]{64}$'),
  intent jsonb NOT NULL CHECK(jsonb_typeof(intent)='object' AND octet_length(intent::text)<=65536),
  intent_sha256 text NOT NULL CHECK(intent_sha256 ~ '^sha256:[0-9a-f]{64}$'),
  prior_state text NOT NULL CHECK(prior_state IN ('OFF','ENABLED','DRAINING','RESTORED_OFF')),
  fence_event_id uuid NOT NULL REFERENCES public.owner_beta_paper_events(id) ON DELETE RESTRICT,
  created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  UNIQUE(account_id,request_key),
  UNIQUE(installation_id,generation),
  UNIQUE(account_id,generation),
  UNIQUE(account_id,fence_epoch)
);

CREATE TABLE public.owner_beta_paper_operation_observations (
  observation_id uuid PRIMARY KEY CHECK(observation_id<>'00000000-0000-0000-0000-000000000000'::uuid),
  generation uuid NOT NULL REFERENCES public.owner_beta_paper_operations(generation) ON DELETE RESTRICT,
  request_key text NOT NULL CHECK(request_key ~ '^[A-Za-z0-9._:-]{1,128}$'),
  summary jsonb NOT NULL CHECK(jsonb_typeof(summary)='object' AND octet_length(summary::text)<=65536),
  summary_sha256 text NOT NULL CHECK(summary_sha256 ~ '^sha256:[0-9a-f]{64}$'),
  observed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  UNIQUE(generation,request_key),
  UNIQUE(generation,observation_id)
);

CREATE TABLE public.owner_beta_paper_recovery_grants (
  grant_id uuid PRIMARY KEY CHECK(grant_id<>'00000000-0000-0000-0000-000000000000'::uuid),
  account_id uuid NOT NULL REFERENCES public.accounts(id) ON DELETE RESTRICT,
  pending_target_id uuid NOT NULL REFERENCES public.pending_targets(id) ON DELETE RESTRICT,
  lineage_id uuid NOT NULL REFERENCES public.owner_beta_paper_lineage(id) ON DELETE RESTRICT,
  effective_session date NOT NULL,
  original_receipt_sha256 text NOT NULL CHECK(original_receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
  lineage_epoch bigint NOT NULL CHECK(lineage_epoch>=0),
  active_epoch bigint NOT NULL CHECK(active_epoch>lineage_epoch),
  consent_event_id uuid NOT NULL REFERENCES public.owner_beta_paper_events(id) ON DELETE RESTRICT,
  process_config_sha256 text NOT NULL CHECK(process_config_sha256 ~ '^sha256:[0-9a-f]{64}$'),
  decision jsonb NOT NULL CHECK(jsonb_typeof(decision)='object' AND octet_length(decision::text)<=65536),
  decision_sha256 text NOT NULL CHECK(decision_sha256 ~ '^sha256:[0-9a-f]{64}$'),
  step text NOT NULL CHECK(step IN ('SETTLE_THEN_VALUE','VALUE_ONLY')),
  signal_admission_sha256 text NOT NULL CHECK(signal_admission_sha256 ~ '^sha256:[0-9a-f]{64}$'),
  execution_admission_sha256 text NOT NULL CHECK(execution_admission_sha256 ~ '^sha256:[0-9a-f]{64}$'),
  expires_at timestamptz NOT NULL,
  request_key text NOT NULL CHECK(request_key ~ '^[A-Za-z0-9._:-]{1,128}$'),
  settlement_receipt_sha256 text CHECK(settlement_receipt_sha256 IS NULL OR settlement_receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
  valuation_receipt_sha256 text CHECK(valuation_receipt_sha256 IS NULL OR valuation_receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
  revoked_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  UNIQUE(account_id,request_key)
);
CREATE UNIQUE INDEX owner_beta_paper_recovery_active_target_epoch
  ON public.owner_beta_paper_recovery_grants(pending_target_id,active_epoch)
  WHERE revoked_at IS NULL AND valuation_receipt_sha256 IS NULL;

DO $$ DECLARE t text; BEGIN
  FOREACH t IN ARRAY ARRAY['owner_beta_paper_operations','owner_beta_paper_operation_observations','owner_beta_paper_recovery_grants'] LOOP
    EXECUTE format('ALTER TABLE public.%I OWNER TO migration_owner',t);
    EXECUTE format('ALTER TABLE public.%I ENABLE ROW LEVEL SECURITY',t);
    EXECUTE format('ALTER TABLE public.%I FORCE ROW LEVEL SECURITY',t);
    EXECUTE format('CREATE POLICY paper_0068_internal ON public.%I FOR ALL TO migration_owner USING(true) WITH CHECK(true)',t);
    EXECUTE format('REVOKE ALL ON public.%I FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter',t);
  END LOOP;
END $$;

CREATE FUNCTION public.paper_0068_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$
BEGIN RAISE EXCEPTION 'Owner-beta Paper operation history is append-only' USING ERRCODE='55000'; END $$;
CREATE TRIGGER owner_beta_paper_operations_immutable BEFORE UPDATE OR DELETE ON public.owner_beta_paper_operations FOR EACH ROW EXECUTE FUNCTION public.paper_0068_immutable();
CREATE TRIGGER owner_beta_paper_observations_immutable BEFORE UPDATE OR DELETE ON public.owner_beta_paper_operation_observations FOR EACH ROW EXECUTE FUNCTION public.paper_0068_immutable();

CREATE FUNCTION public.paper_0068_guard_grant() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$
BEGIN
  IF TG_OP='DELETE' OR (to_jsonb(NEW)-ARRAY['settlement_receipt_sha256','valuation_receipt_sha256','revoked_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['settlement_receipt_sha256','valuation_receipt_sha256','revoked_at'])
     OR (OLD.settlement_receipt_sha256 IS NOT NULL AND NEW.settlement_receipt_sha256 IS DISTINCT FROM OLD.settlement_receipt_sha256)
     OR (OLD.valuation_receipt_sha256 IS NOT NULL AND NEW.valuation_receipt_sha256 IS DISTINCT FROM OLD.valuation_receipt_sha256)
     OR (OLD.revoked_at IS NOT NULL AND NEW.revoked_at IS DISTINCT FROM OLD.revoked_at)
     OR (NEW.settlement_receipt_sha256 IS NOT NULL AND OLD.settlement_receipt_sha256 IS NULL AND NEW.step<>'SETTLE_THEN_VALUE')
     OR (NEW.valuation_receipt_sha256 IS NOT NULL AND OLD.valuation_receipt_sha256 IS NULL AND
         (NEW.revoked_at IS NOT NULL OR NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions s
          WHERE s.pending_target_id=NEW.pending_target_id AND s.account_id=NEW.account_id
            AND s.state='VALUED' AND s.valuation_sha256=NEW.valuation_receipt_sha256
            AND s.settlement_sha256 IS NOT NULL
            AND ((NEW.step='SETTLE_THEN_VALUE' AND NEW.settlement_receipt_sha256=s.settlement_sha256)
              OR (NEW.step='VALUE_ONLY' AND NEW.original_receipt_sha256=s.settlement_sha256)))))
  THEN RAISE EXCEPTION 'recovery grant identity/consumption is immutable' USING ERRCODE='55000'; END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER owner_beta_paper_recovery_grant_guard BEFORE UPDATE OR DELETE ON public.owner_beta_paper_recovery_grants FOR EACH ROW EXECUTE FUNCTION public.paper_0068_guard_grant();

CREATE FUNCTION public.paper_0068_principal() RETURNS void
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$ BEGIN
  IF session_user IS DISTINCT FROM 'migration_owner' OR current_setting('role',true) IS DISTINCT FROM 'none'
     OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_roles r WHERE r.rolname=session_user AND r.rolcanlogin AND NOT r.rolsuper AND NOT r.rolcreatedb AND NOT r.rolcreaterole AND NOT r.rolreplication AND NOT r.rolbypassrls)
  THEN RAISE EXCEPTION 'Owner-beta Paper maintenance principal denied' USING ERRCODE='42501'; END IF;
END $$;

CREATE FUNCTION public.paper_0068_nonnegative_i64(value_text text) RETURNS boolean
LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog,pg_temp AS $$
BEGIN
  IF value_text !~ '^(0|[1-9][0-9]*)$' OR length(value_text)>19 THEN RETURN false; END IF;
  IF length(value_text)=19 AND value_text COLLATE "C" > '9223372036854775807' COLLATE "C" THEN RETURN false; END IF;
  RETURN true;
END $$;

CREATE FUNCTION public.paper_0068_restore_actor(saved_actor text) RETURNS void
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$
BEGIN
  IF current_setting('app.actor_user_id',true) IS NOT DISTINCT FROM saved_actor THEN RETURN; END IF;
  IF saved_actor IS NULL THEN
    EXECUTE 'RESET app.actor_user_id';
  ELSE
    PERFORM set_config('app.actor_user_id',saved_actor,true);
  END IF;
END $$;

-- New wire canonicalization is independent of jsonb::text. P2 still rejects
-- duplicate keys and noncanonical original bytes before binding jsonb.
CREATE FUNCTION public.paper_0068_canonical_json(v jsonb) RETURNS text
LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog,pg_temp AS $$
DECLARE typ text:=jsonb_typeof(v); out_text text;
BEGIN
  IF typ='object' THEN
    IF EXISTS(SELECT 1 FROM jsonb_object_keys(v) AS e(k) WHERE k !~ '^[ -~]*$') THEN
      RAISE EXCEPTION 'wire object key is not printable ASCII' USING ERRCODE='22023';
    END IF;
    SELECT '{'||COALESCE(string_agg(to_jsonb(k)::text||':'||public.paper_0068_canonical_json(val),',' ORDER BY k COLLATE "C"),'')||'}'
      INTO out_text FROM jsonb_each(v) AS e(k,val);
    RETURN out_text;
  ELSIF typ='array' THEN
    SELECT '['||COALESCE(string_agg(public.paper_0068_canonical_json(val),',' ORDER BY ord), '')||']'
      INTO out_text FROM jsonb_array_elements(v) WITH ORDINALITY AS e(val,ord);
    RETURN out_text;
  ELSIF typ='string' THEN
    IF (v #>> '{}') !~ '^[ -~]*$' THEN RAISE EXCEPTION 'wire string is not printable ASCII' USING ERRCODE='22023'; END IF;
    RETURN to_jsonb(v #>> '{}')::text;
  ELSIF typ='number' THEN
    out_text:=v::text;
    IF NOT public.paper_0068_nonnegative_i64(out_text) THEN
      RAISE EXCEPTION 'wire integer is not canonical signed i64' USING ERRCODE='22023';
    END IF;
    RETURN out_text;
  ELSIF typ IN ('boolean','null') THEN RETURN v::text;
  END IF;
  RAISE EXCEPTION 'wire value type unsupported' USING ERRCODE='22023';
END $$;

CREATE FUNCTION public.paper_0068_canonical_row_json(v jsonb) RETURNS text
LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog,pg_temp AS $$
DECLARE typ text:=jsonb_typeof(v); out_text text; n numeric;
BEGIN
  IF typ='object' THEN
    SELECT '{'||COALESCE(string_agg(to_jsonb(k)::text||':'||public.paper_0068_canonical_row_json(val),',' ORDER BY k COLLATE "C"),'')||'}'
      INTO out_text FROM jsonb_each(v) AS e(k,val);
    RETURN out_text;
  ELSIF typ='array' THEN
    SELECT '['||COALESCE(string_agg(public.paper_0068_canonical_row_json(val),',' ORDER BY ord),'')||']'
      INTO out_text FROM jsonb_array_elements(v) WITH ORDINALITY AS e(val,ord);
    RETURN out_text;
  ELSIF typ='string' THEN
    RETURN to_jsonb(v #>> '{}')::text;
  ELSIF typ='number' THEN
    n:=trim_scale((v::text)::numeric);
    IF n=0 THEN RETURN '0'; END IF;
    RETURN n::text;
  ELSIF typ IN ('boolean','null') THEN
    RETURN v::text;
  END IF;
  RAISE EXCEPTION 'audit row value type unsupported' USING ERRCODE='22023';
END $$;

CREATE FUNCTION public.paper_0068_stream_digest(rows jsonb) RETURNS text
LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog,pg_temp AS $$
DECLARE item jsonb; name text; row_key text; canonical text; frames text:='';
BEGIN
  IF jsonb_typeof(rows)<>'array' THEN RAISE EXCEPTION 'audit stream is not an array' USING ERRCODE='22023'; END IF;
  IF current_setting('server_encoding')<>'UTF8' THEN RAISE EXCEPTION 'audit stream requires UTF-8 database encoding' USING ERRCODE='22023'; END IF;
  FOR item IN SELECT value FROM jsonb_array_elements(rows) ORDER BY value->>'table' COLLATE "C",value->>'row_key' COLLATE "C" LOOP
    IF NOT public.daily_0060_keys(item,ARRAY['table','row_key','row']) OR jsonb_typeof(item->'table')<>'string' OR jsonb_typeof(item->'row_key')<>'string' THEN RAISE EXCEPTION 'audit frame shape invalid' USING ERRCODE='22023'; END IF;
    name:=item->>'table'; row_key:=item->>'row_key';
    IF name !~ '^[ -~]+$' OR row_key !~ '^[ -~]*$' THEN RAISE EXCEPTION 'audit frame identity is not printable ASCII' USING ERRCODE='22023'; END IF;
    canonical:=public.paper_0068_canonical_row_json(item->'row');
    frames:=frames||octet_length(name)::text||':'||name||octet_length(row_key)::text||':'||row_key||octet_length(canonical)::text||':'||canonical;
  END LOOP;
  RETURN public.daily_0060_digest(frames);
END $$;

CREATE FUNCTION public.paper_0068_utc(ts timestamptz) RETURNS text
LANGUAGE sql IMMUTABLE STRICT SET search_path=pg_catalog,pg_temp
AS $$ SELECT to_char(ts AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') $$;

CREATE FUNCTION public.paper_0068_clock_fresh(observed jsonb) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$
DECLARE rt numeric; now_ns numeric;
BEGIN
  IF NOT public.daily_0060_keys(observed,ARRAY['realtime_ns','monotonic_ns','boottime_ns'])
     OR jsonb_typeof(observed->'realtime_ns')<>'number' OR jsonb_typeof(observed->'monotonic_ns')<>'number' OR jsonb_typeof(observed->'boottime_ns')<>'number'
     OR NOT public.paper_0068_nonnegative_i64(observed->>'realtime_ns')
     OR NOT public.paper_0068_nonnegative_i64(observed->>'monotonic_ns')
     OR NOT public.paper_0068_nonnegative_i64(observed->>'boottime_ns') THEN RETURN false; END IF;
  rt:=(observed->>'realtime_ns')::numeric;
  now_ns:=extract(epoch FROM clock_timestamp())*1000000000;
  RETURN rt<=now_ns AND now_ns-rt<30000000000;
END $$;

-- P0-B receipts are additive. Historical NULLs are never rewritten.
DO $$ DECLARE t text; BEGIN
  FOREACH t IN ARRAY ARRAY['owner_beta_paper_events','owner_beta_paper_operations','owner_beta_paper_operation_observations'] LOOP
    EXECUTE format('ALTER TABLE public.%I ADD COLUMN request_canonical text, ADD COLUMN request_sha256 text, ADD COLUMN result_canonical text, ADD COLUMN result_sha256 text',t);
    EXECUTE format('ALTER TABLE public.%I ADD CONSTRAINT paper_0068_receipt_complete CHECK (num_nonnulls(request_canonical,request_sha256,result_canonical,result_sha256) IN (0,4))',t);
  END LOOP;
END $$;

-- Preserve app's original 0067 column visibility without exposing private receipts.
REVOKE SELECT ON public.owner_beta_paper_events FROM app;
GRANT SELECT (id,owner_user_id,account_id,control_epoch,kind,actor_user_id,occurred_at,payload,payload_sha256,request_key,pending_target_id) ON public.owner_beta_paper_events TO app;

CREATE FUNCTION public.paper_0068_json_depth(v jsonb) RETURNS integer
LANGUAGE sql IMMUTABLE STRICT SET search_path=pg_catalog,pg_temp AS $$
  WITH RECURSIVE nodes(v,d) AS (
    SELECT v,1 UNION ALL
    SELECT child, d+1 FROM nodes CROSS JOIN LATERAL (
      SELECT value AS child FROM jsonb_each(CASE WHEN jsonb_typeof(nodes.v)='object' THEN nodes.v ELSE '{}'::jsonb END)
      UNION ALL SELECT value FROM jsonb_array_elements(CASE WHEN jsonb_typeof(nodes.v)='array' THEN nodes.v ELSE '[]'::jsonb END)
    ) c WHERE d<=16
  ) SELECT max(d) FROM nodes
$$;

CREATE FUNCTION public.paper_0068_parse_canonical(wire text,byte_limit integer) RETURNS jsonb
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,pg_temp AS $$
DECLARE v jsonb;
BEGIN
  IF wire IS NULL OR octet_length(wire)>byte_limit OR wire !~ '^[ -~]+$' THEN
    RAISE EXCEPTION 'canonical wire size or encoding invalid' USING ERRCODE='22023';
  END IF;
  BEGIN v:=wire::jsonb;
  EXCEPTION WHEN OTHERS THEN RAISE EXCEPTION 'canonical JSON invalid' USING ERRCODE='22023'; END;
  IF public.paper_0068_json_depth(v)>16 OR public.paper_0068_canonical_json(v) IS DISTINCT FROM wire THEN
    RAISE EXCEPTION 'canonical JSON bytes or depth invalid' USING ERRCODE='22023';
  END IF;
  RETURN v;
END $$;

CREATE FUNCTION public.paper_0068_uuid(v jsonb) RETURNS boolean
LANGUAGE sql IMMUTABLE SET search_path=pg_catalog,pg_temp AS $$
 SELECT COALESCE(jsonb_typeof(v)='string' AND v#>>'{}' ~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$',false)
$$;

CREATE FUNCTION public.paper_0068_request(p_kind text,p_account uuid,p_args jsonb) RETURNS text
LANGUAGE sql IMMUTABLE SET search_path=pg_catalog,pg_temp AS $$
 SELECT public.paper_0068_canonical_json(jsonb_build_object('schema','owner-beta-paper-exact-request-v1','kind',p_kind,'account_id',p_account,'args',p_args))
$$;

-- Static shape validation only: no freshness, serving authority, locks or Current.
CREATE FUNCTION public.paper_0068_exact_request(wire text,digest text) RETURNS jsonb
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,pg_temp AS $$
DECLARE q jsonb; a jsonb; e jsonb; b jsonb; x jsonb; k text; variant text; keys text[]; v jsonb;
BEGIN
  q:=public.paper_0068_parse_canonical(wire,1048576); a:=q->'args';
  IF digest IS NULL OR digest !~ '^sha256:[0-9a-f]{64}$' OR public.daily_0060_digest(wire)<>digest
     OR NOT public.daily_0060_keys(q,ARRAY['schema','kind','account_id','args'])
     OR q->>'schema' IS DISTINCT FROM 'owner-beta-paper-exact-request-v1'
     OR q->>'kind' IS NULL OR q->>'kind' NOT IN ('CONFIGURE','BEGIN','RECONCILE','TRANSITION','RESET')
     OR jsonb_typeof(a) IS DISTINCT FROM 'object'
     OR (CASE WHEN q->>'kind'='CONFIGURE' THEN q->'account_id' IS DISTINCT FROM 'null'::jsonb ELSE NOT public.paper_0068_uuid(q->'account_id') END)
  THEN RAISE EXCEPTION 'exact request envelope invalid' USING ERRCODE='22023'; END IF;
  keys:=CASE q->>'kind'
    WHEN 'CONFIGURE' THEN ARRAY['strategy_config_id','account_name','contract_canonical','process_binding','request_key']
    WHEN 'BEGIN' THEN ARRAY['expected_epoch','intent_canonical','request_key']
    WHEN 'RECONCILE' THEN ARRAY['generation','expected_epoch','inventory_sha256','request_key']
    WHEN 'TRANSITION' THEN ARRAY['expected_epoch','transition','evidence_canonical','request_key']
    ELSE ARRAY['expected_epoch'] END;
  IF NOT public.daily_0060_keys(a,keys)
     OR (q->>'kind'<>'RESET' AND (jsonb_typeof(a->'request_key') IS DISTINCT FROM 'string' OR a->>'request_key' !~ '^[A-Za-z0-9._:-]{1,128}$'))
     OR (q->>'kind'<>'CONFIGURE' AND (jsonb_typeof(a->'expected_epoch') IS DISTINCT FROM 'number' OR NOT public.paper_0068_nonnegative_i64(a->>'expected_epoch')))
  THEN RAISE EXCEPTION 'exact argument set invalid' USING ERRCODE='22023'; END IF;
  IF q->>'kind'='RESET' THEN RETURN q; END IF;
  IF q->>'kind'='RECONCILE' THEN
    IF NOT public.paper_0068_uuid(a->'generation') OR NOT public.daily_0060_hash(a->'inventory_sha256')
       OR a->>'request_key' !~ ('^'||(a->>'generation')||':reconcile:[0-9]{1,18}$')
    THEN RAISE EXCEPTION 'exact reconciliation arguments invalid' USING ERRCODE='22023'; END IF;
    RETURN q;
  END IF;
  k:=CASE q->>'kind' WHEN 'CONFIGURE' THEN 'contract_canonical' WHEN 'BEGIN' THEN 'intent_canonical' ELSE 'evidence_canonical' END;
  IF jsonb_typeof(a->k) IS DISTINCT FROM 'string' THEN RAISE EXCEPTION 'embedded canonical argument must be text' USING ERRCODE='22023'; END IF;
  e:=public.paper_0068_parse_canonical(a->>k,CASE WHEN q->>'kind'='BEGIN' THEN 65536 ELSE 262144 END);
  IF q->>'kind'='CONFIGURE' THEN
    IF NOT public.paper_0068_uuid(a->'strategy_config_id') OR jsonb_typeof(a->'account_name') IS DISTINCT FROM 'string'
       OR a->>'account_name' !~ '^[A-Za-z0-9 _.-]{1,64}$' OR a->'process_binding' IS DISTINCT FROM 'null'::jsonb
       OR e IS DISTINCT FROM jsonb_build_object('contract_id','owner-beta-paper-v1-draft-1','contract_sha256','sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0','decision_sha256','sha256:8ef602a35586d3e2ca532c5fbb5efc6f2e727eb25ebeb45a44dad0ea0d0ca394','cost_profile_id','KRX_ETF_DEFAULT','cost_profile_version',1)
    THEN RAISE EXCEPTION 'exact configuration arguments invalid' USING ERRCODE='22023'; END IF;
    RETURN q;
  ELSIF q->>'kind'='BEGIN' THEN
    keys:=ARRAY['schema','installation_id','generation','account_id','predecessor_generation','request_key','purpose','scope','expected_epoch','local_epoch_floor','release_manifest_sha256','artifact_sha256','resume_target_id'];
    IF e->>'schema' IS DISTINCT FROM 'owner-beta-paper-operation-intent-v1' OR e->>'purpose' IS NULL OR e->>'purpose' NOT IN ('STOP','BACKUP','RESTORE') OR e->>'scope' IS NULL OR e->>'scope' NOT IN ('PAPER_STOP','COMBINED_BACKUP')
    THEN RAISE EXCEPTION 'exact intent variant invalid' USING ERRCODE='22023'; END IF;
  ELSE
    variant:=a->>'transition';
    IF variant IS NULL OR variant NOT IN ('ENABLE','DRAIN','OFF','RESTORED_OFF','RECOVERY') THEN RAISE EXCEPTION 'exact transition variant invalid' USING ERRCODE='22023'; END IF;
    IF variant='ENABLE' THEN
      keys:=ARRAY['current_cohort','process_binding','process_binding_sha256','evaluated_at','expires_at','session_evidence','expected_epoch','new_epoch'];
      b:=e->'process_binding';
      IF NOT public.daily_0060_keys(b,ARRAY['schema','owner_user_id','account_id','strategy_config_id','consent_event_id','consent_version','control_epoch','daily_cohort_sha256','financial_contract_sha256','code_commit','release_manifest_sha256','api_binary_sha256','worker_binary_sha256'])
         OR b->>'schema' IS DISTINCT FROM 'owner-beta-paper-process-v1' OR b->>'consent_version' IS DISTINCT FROM 'owner-beta-paper-v1-draft-1'
         OR b->>'code_commit' IS NULL OR b->>'code_commit' !~ '^[0-9a-f]{40}$'
      THEN RAISE EXCEPTION 'exact process binding shape invalid' USING ERRCODE='22023'; END IF;
      FOR k,v IN SELECT key,value FROM jsonb_each(b) LOOP
        IF (k LIKE '%\_id' ESCAPE '\' AND NOT public.paper_0068_uuid(v)) OR (k LIKE '%\_sha256' ESCAPE '\' AND NOT public.daily_0060_hash(v))
           OR (k='control_epoch' AND (jsonb_typeof(v) IS DISTINCT FROM 'number' OR NOT public.paper_0068_nonnegative_i64(v#>>'{}')))
        THEN RAISE EXCEPTION 'exact process binding scalar invalid' USING ERRCODE='22023'; END IF;
      END LOOP;
      b:=e->'current_cohort';
      IF NOT public.daily_0060_keys(b,ARRAY['owner_user_id','policy_sha256','activation_epoch','strategy_config_id','config_epoch','strategy_config_sha256','entitlement_reference','entitlement_sha256','cohort_sha256','release_manifest_sha256','research_image_id','helper_image_id','semantic_config_sha256','calendar_sha256','verifier_revision'])
         OR jsonb_typeof(b->'entitlement_reference') IS DISTINCT FROM 'string' OR length(b->>'entitlement_reference') NOT BETWEEN 1 AND 512
         OR b->>'verifier_revision' IS NULL OR b->>'verifier_revision' !~ '^[0-9a-f]{40}$'
      THEN RAISE EXCEPTION 'exact cohort shape invalid' USING ERRCODE='22023'; END IF;
      FOR k,v IN SELECT key,value FROM jsonb_each(b) LOOP
        IF (k IN ('owner_user_id','strategy_config_id') AND NOT public.paper_0068_uuid(v))
           OR ((k LIKE '%\_sha256' ESCAPE '\' OR k IN ('research_image_id','helper_image_id')) AND NOT public.daily_0060_hash(v))
           OR (k IN ('activation_epoch','config_epoch') AND (jsonb_typeof(v) IS DISTINCT FROM 'number' OR NOT public.paper_0068_nonnegative_i64(v#>>'{}')))
        THEN RAISE EXCEPTION 'exact cohort scalar invalid' USING ERRCODE='22023'; END IF;
      END LOOP;
      IF jsonb_typeof(e->'session_evidence') IS DISTINCT FROM 'array' OR jsonb_array_length(e->'session_evidence')<>3 THEN RAISE EXCEPTION 'exact sessions shape invalid' USING ERRCODE='22023'; END IF;
      FOR x IN SELECT value FROM jsonb_array_elements(e->'session_evidence') LOOP
        IF NOT public.daily_0060_keys(x,ARRAY['session_date','run_id','admission_sha256','source_event_sha256','root_event_sha256']) OR NOT public.paper_0068_uuid(x->'run_id')
           OR NOT public.daily_0060_hash(x->'admission_sha256') OR NOT public.daily_0060_hash(x->'source_event_sha256') OR NOT public.daily_0060_hash(x->'root_event_sha256')
           OR jsonb_typeof(x->'session_date') IS DISTINCT FROM 'string' OR x->>'session_date' !~ '^20[0-9]{2}-[0-9]{2}-[0-9]{2}$'
        THEN RAISE EXCEPTION 'exact session scalar invalid' USING ERRCODE='22023'; END IF;
        BEGIN PERFORM (x->>'session_date')::date; EXCEPTION WHEN OTHERS THEN RAISE EXCEPTION 'exact session date invalid' USING ERRCODE='22023'; END;
      END LOOP;
    ELSIF variant='DRAIN' THEN
      keys:=ARRAY['schema','expected_epoch','reason_code'];
      IF e->>'schema' IS DISTINCT FROM 'owner-beta-paper-drain-v1' THEN RAISE EXCEPTION 'exact drain schema invalid' USING ERRCODE='22023'; END IF;
    ELSIF variant IN ('OFF','RESTORED_OFF') THEN
      IF octet_length(e::text)>65536 THEN RAISE EXCEPTION 'exact lifecycle argument too large' USING ERRCODE='22023'; END IF;
      keys:=ARRAY['schema','installation_id','generation','account_id','request_key','scope','expected_epoch','resulting_epoch','intent_sha256','stage_sha256','inventory_sha256','reconciliation_sha256','observation_id','release_manifest_sha256','boot_id','observed'];
      IF variant='RESTORED_OFF' THEN keys:=keys||ARRAY['source_installation_id','source_generation','source_epoch','archive_sha256','source_audit_sha256','restored_audit_sha256','destination_epoch_floor']; END IF;
      IF e->>'schema' IS DISTINCT FROM (CASE variant WHEN 'OFF' THEN 'owner-beta-paper-drain-receipt-v1' ELSE 'owner-beta-paper-restore-receipt-v1' END)
         OR e->>'scope' IS NULL OR e->>'scope' NOT IN ('PAPER_STOP','COMBINED_BACKUP')
         OR NOT public.daily_0060_keys(e->'observed',ARRAY['realtime_ns','monotonic_ns','boottime_ns'])
      THEN RAISE EXCEPTION 'exact lifecycle shape invalid' USING ERRCODE='22023'; END IF;
      FOR k,v IN SELECT key,value FROM jsonb_each(e->'observed') LOOP
        IF jsonb_typeof(v) IS DISTINCT FROM 'number' OR NOT public.paper_0068_nonnegative_i64(v#>>'{}') THEN RAISE EXCEPTION 'exact clock scalar invalid' USING ERRCODE='22023'; END IF;
      END LOOP;
    ELSIF e->>'schema'='owner-beta-paper-recovery-v2' THEN
      IF octet_length(e::text)>65536 THEN RAISE EXCEPTION 'exact recovery argument too large' USING ERRCODE='22023'; END IF;
      keys:=ARRAY['schema','installation_id','generation','account_id','request_key','pending_target_id','lineage_id','effective_session','lineage_epoch','active_epoch','original_receipt_sha256','consent_event_id','process_config_sha256','step','signal_admission_sha256','execution_admission_sha256','decision_sha256','reason_code','expires_at'];
      IF e->>'step' IS NULL OR e->>'step' NOT IN ('SETTLE_THEN_VALUE','VALUE_ONLY') OR e->>'reason_code' IS DISTINCT FROM 'EXPLICIT_RETAINED_RECOVERY' THEN RAISE EXCEPTION 'exact recovery decision invalid' USING ERRCODE='22023'; END IF;
    ELSE
      keys:=ARRAY['schema','pending_target_id','effective_session','reason_code','decision_sha256'];
      IF e->>'schema' IS DISTINCT FROM 'owner-beta-paper-recovery-v1' THEN RAISE EXCEPTION 'exact recovery schema invalid' USING ERRCODE='22023'; END IF;
    END IF;
  END IF;
  IF NOT public.daily_0060_keys(e,keys) THEN RAISE EXCEPTION 'embedded argument key set invalid' USING ERRCODE='22023'; END IF;
  FOR k,v IN SELECT key,value FROM jsonb_each(e) LOOP
    IF k IN ('predecessor_generation','resume_target_id','artifact_sha256') AND v='null'::jsonb THEN CONTINUE; END IF;
    IF v='null'::jsonb
       OR ((k LIKE '%\_id' ESCAPE '\' OR k IN ('generation','predecessor_generation','source_generation')) AND NOT public.paper_0068_uuid(v))
       OR (k LIKE '%\_sha256' ESCAPE '\' AND NOT public.daily_0060_hash(v))
       OR (k IN ('expected_epoch','new_epoch','local_epoch_floor','resulting_epoch','source_epoch','destination_epoch_floor','lineage_epoch','active_epoch') AND (jsonb_typeof(v) IS DISTINCT FROM 'number' OR NOT public.paper_0068_nonnegative_i64(v#>>'{}')))
       OR (k IN ('schema','purpose','scope','request_key','reason_code','step','effective_session','evaluated_at','expires_at') AND jsonb_typeof(v) IS DISTINCT FROM 'string')
       OR (k='request_key' AND v#>>'{}' !~ '^[A-Za-z0-9._:-]{1,128}$')
       OR (k='reason_code' AND v#>>'{}' !~ '^[A-Z][A-Z0-9_]{0,63}$')
       OR (k='effective_session' AND v#>>'{}' !~ '^20[0-9]{2}-[0-9]{2}-[0-9]{2}$')
    THEN RAISE EXCEPTION 'embedded argument scalar invalid' USING ERRCODE='22023'; END IF;
    IF k IN ('evaluated_at','expires_at','effective_session') THEN
      BEGIN
        IF k='effective_session' THEN PERFORM (v#>>'{}')::date;
        ELSIF NOT isfinite((v#>>'{}')::timestamptz) THEN RAISE EXCEPTION 'nonfinite'; END IF;
      EXCEPTION WHEN OTHERS THEN RAISE EXCEPTION 'embedded timestamp invalid' USING ERRCODE='22023'; END;
    END IF;
  END LOOP;
  RETURN q;
END $$;

CREATE FUNCTION public.paper_0068_validate_receipt(r jsonb,family text) RETURNS void
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$
DECLARE q jsonb; a jsonb; e jsonb; result jsonb; expected jsonb; o public.owner_beta_paper_operations; c public.owner_beta_paper_control; event public.owner_beta_paper_events; expected_kind text; legacy_hash text;
BEGIN
  IF num_nonnulls(NULLIF(r->'request_canonical','null'::jsonb),NULLIF(r->'request_sha256','null'::jsonb),NULLIF(r->'result_canonical','null'::jsonb),NULLIF(r->'result_sha256','null'::jsonb))=0 THEN RETURN; END IF;
  BEGIN
    q:=public.paper_0068_exact_request(r->>'request_canonical',r->>'request_sha256'); a:=q->'args';
    result:=public.paper_0068_parse_canonical(r->>'result_canonical',262144);
    IF r->>'result_sha256' IS NULL OR r->>'result_sha256' !~ '^sha256:[0-9a-f]{64}$' OR public.daily_0060_digest(r->>'result_canonical')<>r->>'result_sha256' THEN RAISE EXCEPTION 'receipt result digest invalid' USING ERRCODE='23514'; END IF;
  EXCEPTION WHEN OTHERS THEN RAISE EXCEPTION 'stored exact receipt invalid' USING ERRCODE='23514'; END;
  IF q->>'kind'='RESET' OR r->>'request_key' IS DISTINCT FROM a->>'request_key' THEN RAISE EXCEPTION 'receipt key linkage invalid' USING ERRCODE='23514'; END IF;
  IF family='owner_beta_paper_events' THEN
    SELECT * INTO c FROM public.owner_beta_paper_control WHERE account_id=(r->>'account_id')::uuid AND owner_user_id=(r->>'owner_user_id')::uuid;
    IF NOT FOUND THEN RAISE EXCEPTION 'receipt Owner/account linkage invalid' USING ERRCODE='23514'; END IF;
    IF q->>'kind'='CONFIGURE' THEN
      e:=(a->>'contract_canonical')::jsonb;
      IF r->>'kind' IS DISTINCT FROM 'CONFIGURED' OR r->>'control_epoch'<>'0'
         OR r->'payload' IS DISTINCT FROM jsonb_build_object('schema','owner-beta-paper-configured-v1','account_name',a->>'account_name','strategy_config_id',a->>'strategy_config_id','contract',e,'initial_cash','10000000.0000','currency','KRW')
         OR r->'pending_target_id' IS DISTINCT FROM 'null'::jsonb
      THEN RAISE EXCEPTION 'configure receipt linkage invalid' USING ERRCODE='23514'; END IF;
      legacy_hash:=public.daily_0060_digest((r->'payload')::text);
      expected:=public.paper_0067_action('CONFIGURE','OK',0,c.account_id,jsonb_build_object('account_id',c.account_id,'event_id',r->'id','state','OFF','initial_cash','10000000.0000','currency','KRW'));
    ELSIF q->>'kind'='TRANSITION' THEN
      e:=(a->>'evidence_canonical')::jsonb;
      expected_kind:=CASE a->>'transition' WHEN 'ENABLE' THEN 'ENABLED' WHEN 'DRAIN' THEN 'DRAIN_STARTED' WHEN 'OFF' THEN 'OFF' WHEN 'RESTORED_OFF' THEN 'RESTORED_OFF' ELSE 'RECOVERY_AUTHORIZED' END;
      IF q->>'account_id' IS DISTINCT FROM r->>'account_id' OR r->>'kind' IS DISTINCT FROM expected_kind OR r->'payload' IS DISTINCT FROM e
         OR (r->>'control_epoch')::numeric IS DISTINCT FROM (a->>'expected_epoch')::numeric+(CASE WHEN a->>'transition'='RECOVERY' THEN 0 ELSE 1 END)
         OR r->'pending_target_id' IS DISTINCT FROM (CASE WHEN a->>'transition'='RECOVERY' THEN e->'pending_target_id' ELSE 'null'::jsonb END)
      THEN RAISE EXCEPTION 'transition receipt linkage invalid' USING ERRCODE='23514'; END IF;
      legacy_hash:=public.daily_0060_digest(CASE WHEN a->>'transition' IN ('OFF','RESTORED_OFF') OR e->>'schema'='owner-beta-paper-recovery-v2' THEN public.paper_0068_canonical_json(e) ELSE e::text END);
      expected:=public.paper_0067_action('TRANSITION','OK',(r->>'control_epoch')::bigint,c.account_id,jsonb_build_object('event_id',r->'id','state',CASE a->>'transition' WHEN 'ENABLE' THEN 'ENABLED' WHEN 'DRAIN' THEN 'DRAINING' WHEN 'OFF' THEN 'OFF' WHEN 'RESTORED_OFF' THEN 'RESTORED_OFF' ELSE result#>>'{data,state}' END));
      IF a->>'transition'='RECOVERY' THEN
        IF result#>>'{data,state}' IS NULL OR result#>>'{data,state}' NOT IN ('OFF','ENABLED','DRAINING','RESTORED_OFF') THEN RAISE EXCEPTION 'recovery result state invalid' USING ERRCODE='23514'; END IF;
        IF e->>'schema'='owner-beta-paper-recovery-v2' THEN
          expected:=jsonb_set(expected,'{data}',(expected->'data')||jsonb_build_object('pending_target_id',e->'pending_target_id','grant_id',(SELECT g.grant_id FROM public.owner_beta_paper_recovery_grants g WHERE g.account_id=c.account_id AND g.request_key=a->>'request_key' AND g.pending_target_id=(e->>'pending_target_id')::uuid AND g.active_epoch=(r->>'control_epoch')::bigint)));
          IF expected#>'{data,grant_id}'='null'::jsonb THEN RAISE EXCEPTION 'recovery receipt grant missing' USING ERRCODE='23514'; END IF;
        END IF;
      END IF;
    ELSE RAISE EXCEPTION 'event receipt kind invalid' USING ERRCODE='23514'; END IF;
    IF r->>'payload_sha256' IS DISTINCT FROM legacy_hash THEN RAISE EXCEPTION 'event payload digest invalid' USING ERRCODE='23514'; END IF;
  ELSIF family='owner_beta_paper_operations' THEN
    e:=(a->>'intent_canonical')::jsonb;
    IF q->>'kind' IS DISTINCT FROM 'BEGIN' OR q->>'account_id' IS DISTINCT FROM r->>'account_id' OR r->'intent' IS DISTINCT FROM e
       OR r->>'expected_epoch' IS DISTINCT FROM a->>'expected_epoch' OR e->>'expected_epoch' IS DISTINCT FROM a->>'expected_epoch'
       OR r->>'account_id' IS DISTINCT FROM e->>'account_id' OR r->>'generation' IS DISTINCT FROM e->>'generation'
       OR r->>'installation_id' IS DISTINCT FROM e->>'installation_id' OR r->>'request_key' IS DISTINCT FROM e->>'request_key'
       OR r->>'purpose' IS DISTINCT FROM e->>'purpose' OR r->>'scope' IS DISTINCT FROM e->>'scope'
       OR r->>'local_epoch_floor' IS DISTINCT FROM e->>'local_epoch_floor' OR r->>'release_manifest_sha256' IS DISTINCT FROM e->>'release_manifest_sha256'
       OR r->>'intent_sha256' IS DISTINCT FROM public.daily_0060_digest(public.paper_0068_canonical_json(e))
       OR (r->>'fence_epoch')::numeric<>greatest((r->>'expected_epoch')::numeric,(r->>'local_epoch_floor')::numeric)+1
    THEN RAISE EXCEPTION 'operation receipt linkage invalid' USING ERRCODE='23514'; END IF;
    SELECT * INTO event FROM public.owner_beta_paper_events WHERE id=(r->>'fence_event_id')::uuid AND account_id=(r->>'account_id')::uuid AND kind='DRAIN_STARTED' AND control_epoch=(r->>'fence_epoch')::bigint AND payload=e AND payload_sha256=r->>'intent_sha256' AND request_key=(r->>'generation')||':begin';
    IF NOT FOUND THEN RAISE EXCEPTION 'operation receipt fence missing' USING ERRCODE='23514'; END IF;
    expected:=public.paper_0067_action('BEGIN_OPERATION','OK',(r->>'fence_epoch')::bigint,(r->>'account_id')::uuid,jsonb_build_object('generation',r->'generation','operation_event_id',r->'fence_event_id','intent_sha256',r->'intent_sha256','fenced_epoch',r->'fence_epoch'));
  ELSIF family='owner_beta_paper_operation_observations' THEN
    SELECT * INTO o FROM public.owner_beta_paper_operations WHERE generation=(r->>'generation')::uuid;
    IF NOT FOUND OR q->>'kind' IS DISTINCT FROM 'RECONCILE' OR q->>'account_id' IS DISTINCT FROM o.account_id::text
       OR a->>'generation' IS DISTINCT FROM o.generation::text OR (a->>'expected_epoch')::bigint<>o.fence_epoch
       OR r#>>'{summary,inventory_sha256}' IS DISTINCT FROM a->>'inventory_sha256' OR r#>>'{summary,observation_id}' IS DISTINCT FROM r->>'observation_id'
       OR r#>>'{summary,generation}' IS DISTINCT FROM o.generation::text OR r#>>'{summary,account_id}' IS DISTINCT FROM o.account_id::text
       OR r#>>'{summary,installation_id}' IS DISTINCT FROM o.installation_id::text OR r#>>'{summary,control_epoch}' IS DISTINCT FROM o.fence_epoch::text
       OR r->>'summary_sha256' IS DISTINCT FROM public.daily_0060_digest(public.paper_0068_canonical_json(r->'summary'))
    THEN RAISE EXCEPTION 'observation receipt linkage invalid' USING ERRCODE='23514'; END IF;
    PERFORM public.paper_0068_validate_receipt(to_jsonb(o),'owner_beta_paper_operations');
    expected:=public.paper_0067_action('RECONCILE_OPERATION','OK',o.fence_epoch,o.account_id,jsonb_build_object('observation_id',r->'observation_id','summary',r->'summary','summary_sha256',r->'summary_sha256'));
  ELSE RAISE EXCEPTION 'receipt family invalid' USING ERRCODE='23514'; END IF;
  IF result IS DISTINCT FROM expected THEN RAISE EXCEPTION 'receipt original result linkage invalid' USING ERRCODE='23514'; END IF;
END $$;

CREATE FUNCTION public.paper_0068_receipt_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$
BEGIN PERFORM public.paper_0068_validate_receipt(to_jsonb(NEW),TG_TABLE_NAME); RETURN NEW; END $$;
CREATE TRIGGER paper_0068_receipt_guard BEFORE INSERT ON public.owner_beta_paper_events FOR EACH ROW EXECUTE FUNCTION public.paper_0068_receipt_guard();
CREATE TRIGGER paper_0068_receipt_guard BEFORE INSERT ON public.owner_beta_paper_operations FOR EACH ROW EXECUTE FUNCTION public.paper_0068_receipt_guard();
CREATE TRIGGER paper_0068_receipt_guard BEFORE INSERT ON public.owner_beta_paper_operation_observations FOR EACH ROW EXECUTE FUNCTION public.paper_0068_receipt_guard();

CREATE FUNCTION public.paper_0068_reset(c public.owner_beta_paper_control) RETURNS jsonb
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$
DECLARE e public.owner_beta_paper_events; n bigint; max_epoch bigint; micros numeric;
BEGIN
  SELECT max(control_epoch) INTO max_epoch FROM public.owner_beta_paper_events
    WHERE account_id=c.account_id AND owner_user_id=c.owner_user_id AND kind IN ('CONFIGURED','OFF','RESTORED_OFF');
  SELECT count(*) INTO n FROM public.owner_beta_paper_events WHERE account_id=c.account_id AND owner_user_id=c.owner_user_id AND kind IN ('CONFIGURED','OFF','RESTORED_OFF') AND control_epoch=max_epoch;
  IF n<>1 OR max_epoch>c.control_epoch THEN RAISE EXCEPTION 'reset boundary missing or ambiguous' USING ERRCODE='55000'; END IF;
  SELECT * INTO STRICT e FROM public.owner_beta_paper_events WHERE account_id=c.account_id AND owner_user_id=c.owner_user_id AND kind IN ('CONFIGURED','OFF','RESTORED_OFF') AND control_epoch=max_epoch;
  IF NOT isfinite(e.occurred_at) OR (e.kind='CONFIGURED' AND e.control_epoch<>0) THEN RAISE EXCEPTION 'reset timestamp invalid' USING ERRCODE='55000'; END IF;
  micros:=extract(epoch FROM e.occurred_at)*1000000;
  IF micros<0 OR micros>9223372036854775807 OR micros<>trunc(micros) THEN RAISE EXCEPTION 'reset timestamp outside microsecond range' USING ERRCODE='55000'; END IF;
  RETURN jsonb_build_object('account_id',c.account_id,'event_id',e.id,'kind',e.kind,'control_epoch',e.control_epoch,'occurred_at_us',micros::bigint,'payload_sha256',e.payload_sha256);
END $$;

CREATE FUNCTION public.read_owner_beta_paper_operation_result_v1(request_canonical text,request_sha256 text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE old_actor text:=current_setting('app.actor_user_id',true); q jsonb; a jsonb; c public.owner_beta_paper_control; r jsonb; e jsonb; b jsonb; o public.owner_beta_paper_operations; status text:='ABSENT'; event_id uuid; event_epoch bigint; result_text text; result_hash text; reset jsonb; family text; n bigint; expected_kind text; legacy_epoch text;
BEGIN
  PERFORM public.paper_0068_principal();
  q:=public.paper_0068_exact_request(request_canonical,request_sha256); a:=q->'args';
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11';
  IF q->>'kind'<>'CONFIGURE' AND (c.account_id IS NULL OR c.account_id::text IS DISTINCT FROM q->>'account_id') THEN RAISE EXCEPTION 'exact read account denied' USING ERRCODE='42501'; END IF;
  IF c.account_id IS NOT NULL THEN PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true); END IF;
  IF q->>'kind'='RESET' THEN
    IF (a->>'expected_epoch')::bigint<>c.control_epoch THEN RAISE EXCEPTION 'reset expected epoch stale' USING ERRCODE='40001'; END IF;
    reset:=public.paper_0068_reset(c); status:='RESET';
  ELSIF c.account_id IS NULL THEN
    -- Only CONFIGURE reaches this branch; no tenant is selected before setup.
    NULL;
  ELSIF q->>'kind' IN ('CONFIGURE','TRANSITION') THEN
    family:='owner_beta_paper_events';
    SELECT to_jsonb(t) INTO r FROM public.owner_beta_paper_events t WHERE t.account_id=c.account_id AND t.owner_user_id=c.owner_user_id AND t.request_key=a->>'request_key';
    IF r IS NOT NULL AND r->'request_canonical'='null'::jsonb THEN
      expected_kind:=CASE WHEN q->>'kind'='CONFIGURE' THEN 'CONFIGURED' ELSE (CASE a->>'transition' WHEN 'ENABLE' THEN 'ENABLED' WHEN 'DRAIN' THEN 'DRAIN_STARTED' WHEN 'OFF' THEN 'OFF' WHEN 'RESTORED_OFF' THEN 'RESTORED_OFF' ELSE 'RECOVERY_AUTHORIZED' END) END;
      IF r->>'kind' IS DISTINCT FROM expected_kind THEN RAISE EXCEPTION 'legacy event kind conflicts' USING ERRCODE='23505'; END IF;
      IF q->>'kind'='CONFIGURE' THEN
        IF r#>>'{payload,account_name}' IS DISTINCT FROM a->>'account_name' OR r#>>'{payload,strategy_config_id}' IS DISTINCT FROM a->>'strategy_config_id' OR r#>'{payload,contract}' IS DISTINCT FROM (a->>'contract_canonical')::jsonb THEN RAISE EXCEPTION 'legacy configuration conflicts' USING ERRCODE='23505'; END IF;
      ELSE
        e:=(a->>'evidence_canonical')::jsonb;
        legacy_epoch:=CASE a->>'transition' WHEN 'ENABLE' THEN e->>'expected_epoch' WHEN 'DRAIN' THEN e->>'expected_epoch' WHEN 'OFF' THEN e->>'resulting_epoch' WHEN 'RESTORED_OFF' THEN e->>'resulting_epoch' ELSE e->>'active_epoch' END;
        IF r->'payload' IS DISTINCT FROM e OR (legacy_epoch IS NOT NULL AND legacy_epoch IS DISTINCT FROM a->>'expected_epoch') THEN RAISE EXCEPTION 'legacy transition conflicts' USING ERRCODE='23505'; END IF;
      END IF;
    END IF;
  ELSIF q->>'kind'='BEGIN' THEN
    family:='owner_beta_paper_operations'; e:=(a->>'intent_canonical')::jsonb;
    SELECT count(*) INTO n FROM public.owner_beta_paper_operations t WHERE t.generation=(e->>'generation')::uuid OR (t.account_id=c.account_id AND t.request_key=a->>'request_key');
    IF n>1 THEN RAISE EXCEPTION 'operation identities collide' USING ERRCODE='23505'; END IF;
    SELECT to_jsonb(t) INTO r FROM public.owner_beta_paper_operations t WHERE t.generation=(e->>'generation')::uuid OR (t.account_id=c.account_id AND t.request_key=a->>'request_key');
    IF r IS NOT NULL AND (r->>'account_id' IS DISTINCT FROM c.account_id::text OR r->>'generation' IS DISTINCT FROM e->>'generation' OR r->>'request_key' IS DISTINCT FROM a->>'request_key') THEN RAISE EXCEPTION 'operation identity conflicts' USING ERRCODE='23505'; END IF;
    IF r IS NOT NULL AND r->'request_canonical'='null'::jsonb AND (r->'intent' IS DISTINCT FROM e OR r->>'expected_epoch' IS DISTINCT FROM a->>'expected_epoch') THEN RAISE EXCEPTION 'legacy operation conflicts' USING ERRCODE='23505'; END IF;
  ELSE
    family:='owner_beta_paper_operation_observations';
    SELECT * INTO o FROM public.owner_beta_paper_operations WHERE generation=(a->>'generation')::uuid;
    IF o.generation IS NOT NULL AND o.account_id<>c.account_id THEN RAISE EXCEPTION 'reconciliation account denied' USING ERRCODE='42501'; END IF;
    SELECT to_jsonb(t) INTO r FROM public.owner_beta_paper_operation_observations t WHERE t.generation=o.generation AND t.request_key=a->>'request_key';
    IF r IS NOT NULL AND r->'request_canonical'='null'::jsonb AND (r#>>'{summary,inventory_sha256}' IS DISTINCT FROM a->>'inventory_sha256' OR a->>'expected_epoch' IS DISTINCT FROM o.fence_epoch::text) THEN RAISE EXCEPTION 'legacy observation conflicts' USING ERRCODE='23505'; END IF;
  END IF;
  IF r IS NOT NULL THEN
    PERFORM public.paper_0068_validate_receipt(r,family);
    IF r->'request_canonical'='null'::jsonb THEN status:='UNPROVEN';
    ELSE
      IF r->>'request_canonical' IS DISTINCT FROM request_canonical OR r->>'request_sha256' IS DISTINCT FROM request_sha256 THEN RAISE EXCEPTION 'occupied exact request conflicts' USING ERRCODE='23505'; END IF;
      status:='COMMITTED'; result_text:=r->>'result_canonical'; result_hash:=r->>'result_sha256';
      event_id:=(CASE family WHEN 'owner_beta_paper_events' THEN r->>'id' WHEN 'owner_beta_paper_operations' THEN r->>'fence_event_id' ELSE r->>'observation_id' END)::uuid;
      event_epoch:=(CASE family WHEN 'owner_beta_paper_events' THEN r->>'control_epoch' WHEN 'owner_beta_paper_operations' THEN r->>'fence_epoch' ELSE o.fence_epoch::text END)::bigint;
    END IF;
  END IF;
  IF r IS NULL AND q->>'kind'='BEGIN' THEN
    e:=(a->>'intent_canonical')::jsonb;
    IF e->>'account_id' IS DISTINCT FROM q->>'account_id' OR e->>'expected_epoch' IS DISTINCT FROM a->>'expected_epoch' OR e->>'request_key' IS DISTINCT FROM a->>'request_key' THEN RAISE EXCEPTION 'absent intent argument linkage invalid' USING ERRCODE='22023'; END IF;
  ELSIF r IS NULL AND q->>'kind'='TRANSITION' THEN
    e:=(a->>'evidence_canonical')::jsonb;
    IF (e ? 'account_id' AND e->>'account_id' IS DISTINCT FROM q->>'account_id')
       OR (e ? 'request_key' AND e->>'request_key' IS DISTINCT FROM a->>'request_key')
       OR (a->>'transition'='ENABLE' AND e#>>'{process_binding,account_id}' IS DISTINCT FROM q->>'account_id')
    THEN RAISE EXCEPTION 'absent transition argument linkage invalid' USING ERRCODE='22023'; END IF;
  END IF;
  b:=jsonb_build_object('schema','owner-beta-paper-exact-readback-v1','kind',q->'kind','request_sha256',request_sha256,'status',status,'account_id',c.account_id,'current_epoch',c.control_epoch,'current_state',c.state,'event_id',event_id,'event_epoch',event_epoch,'result_canonical',result_text,'result_sha256',result_hash,'reset',reset);
  IF octet_length(public.paper_0068_canonical_json(b))>1048576 THEN RAISE EXCEPTION 'exact readback too large' USING ERRCODE='22023'; END IF;
  PERFORM public.paper_0068_restore_actor(old_actor); RETURN b;
EXCEPTION WHEN query_canceled OR assert_failure THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
WHEN OTHERS THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
END $$;


ALTER FUNCTION public.transition_owner_beta_paper_v1(bigint,text,jsonb,text) RENAME TO paper_0068_transition_legacy;
REVOKE ALL ON FUNCTION public.paper_0068_transition_legacy(bigint,text,jsonb,text) FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter;
GRANT EXECUTE ON FUNCTION public.paper_0068_transition_legacy(bigint,text,jsonb,text) TO migration_owner;

ALTER FUNCTION public.configure_owner_beta_paper_v1(uuid,text,jsonb,jsonb,text) RENAME TO paper_0068_configure_legacy;
REVOKE ALL ON FUNCTION public.paper_0068_configure_legacy(uuid,text,jsonb,jsonb,text) FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter;

CREATE FUNCTION public.configure_owner_beta_paper_v1(strategy_config_id uuid,account_name text,contract jsonb,process_binding jsonb,request_key text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE daily public.owner_beta_daily_control; sc public.user_strategy_configs; acct uuid; ev uuid; saved public.owner_beta_paper_events; c public.owner_beta_paper_control; old_actor text:=current_setting('app.actor_user_id',true); cost_hash text; req text; ok jsonb; receipt_result text;
BEGIN
  PERFORM public.paper_0067_principal('migration_owner');
  IF process_binding IS NOT NULL THEN RAISE EXCEPTION 'initial process binding must be null while OFF' USING ERRCODE='23514'; END IF;
  IF account_name IS NULL OR length(account_name) NOT BETWEEN 1 AND 64 OR account_name !~ '^[A-Za-z0-9 _.-]+$' OR request_key IS NULL OR length(request_key) NOT BETWEEN 1 AND 128 THEN RAISE EXCEPTION 'configuration request shape' USING ERRCODE='23514'; END IF;
  IF NOT public.daily_0060_keys(contract,ARRAY['contract_id','contract_sha256','decision_sha256','cost_profile_id','cost_profile_version'])
     OR contract->>'contract_id'<>'owner-beta-paper-v1-draft-1'
     OR contract->>'contract_sha256'<>'sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0'
     OR contract->>'decision_sha256'<>'sha256:8ef602a35586d3e2ca532c5fbb5efc6f2e727eb25ebeb45a44dad0ea0d0ca394'
     OR contract->>'cost_profile_id'<>'KRX_ETF_DEFAULT' OR contract->>'cost_profile_version'<>'1' THEN RAISE EXCEPTION 'financial contract mismatch' USING ERRCODE='23514'; END IF;
  IF public.paper_0068_json_depth(contract)>16 THEN RAISE EXCEPTION 'contract depth invalid' USING ERRCODE='22023'; END IF;
  req:=public.paper_0068_request('CONFIGURE',NULL,jsonb_build_object('strategy_config_id',strategy_config_id,'account_name',account_name,'contract_canonical',public.paper_0068_canonical_json(contract),'process_binding',NULL,'request_key',request_key));
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11';
  IF c.account_id IS NOT NULL THEN
    PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
    SELECT * INTO saved FROM public.owner_beta_paper_events e WHERE e.account_id=c.account_id AND e.request_key=configure_owner_beta_paper_v1.request_key;
    IF FOUND AND saved.request_canonical IS NOT NULL THEN
      PERFORM public.paper_0068_validate_receipt(to_jsonb(saved),'owner_beta_paper_events');
      IF saved.request_canonical IS DISTINCT FROM req OR saved.request_sha256 IS DISTINCT FROM public.daily_0060_digest(req) THEN RAISE EXCEPTION 'configuration exact request conflicts' USING ERRCODE='23505'; END IF;
      PERFORM public.paper_0068_restore_actor(old_actor);
      RETURN public.paper_0067_action('CONFIGURE','REPLAY',c.control_epoch,c.account_id,jsonb_build_object('event_id',saved.id,'state',c.state));
    END IF;
  END IF;
  SELECT * INTO STRICT daily FROM public.owner_beta_daily_control WHERE id='etf11' FOR SHARE;
  PERFORM set_config('app.actor_user_id',daily.owner_user_id::text,true);
  SELECT * INTO STRICT sc FROM public.user_strategy_configs sc0 WHERE sc0.id=configure_owner_beta_paper_v1.strategy_config_id AND sc0.owner_user_id=daily.owner_user_id FOR SHARE;
  IF NOT sc.is_active OR NOT sc.daily_enabled OR sc.strategy_id<>'trend_following' OR sc.strategy_version<>'1.0.0'
     OR sc.config_json->>'benchmark_instrument'<>'069500.KRX' OR sc.config_json->>'fast_ma'<>'50' OR sc.config_json->>'slow_ma'<>'200' THEN RAISE EXCEPTION 'adopted strategy config required' USING ERRCODE='23514'; END IF;
  IF EXISTS(SELECT 1 FROM public.owner_beta_paper_control) THEN
    SELECT * INTO saved FROM public.owner_beta_paper_events e WHERE e.account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') AND e.request_key=configure_owner_beta_paper_v1.request_key AND e.kind='CONFIGURED';
    IF FOUND THEN
      SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11';
      acct:=c.account_id;
      IF saved.owner_user_id<>daily.owner_user_id OR saved.payload->>'account_name' IS DISTINCT FROM account_name OR saved.payload->>'strategy_config_id' IS DISTINCT FROM strategy_config_id::text OR saved.payload->'contract' IS DISTINCT FROM contract OR c.strategy_config_id<>strategy_config_id OR c.contract_json IS DISTINCT FROM contract THEN RAISE EXCEPTION 'configuration request key conflicts with existing binding' USING ERRCODE='23505'; END IF;
      PERFORM public.paper_0068_restore_actor(old_actor);
      RETURN public.paper_0067_action('CONFIGURE','REPLAY',c.control_epoch,acct,jsonb_build_object('event_id',saved.id,'state',c.state));
    END IF;
    IF EXISTS(SELECT 1 FROM public.owner_beta_paper_events e WHERE e.account_id=(SELECT account_id FROM public.owner_beta_paper_control WHERE id='etf11') AND e.request_key=configure_owner_beta_paper_v1.request_key) THEN RAISE EXCEPTION 'configuration request key conflicts with an existing event' USING ERRCODE='23505'; END IF;
    RAISE EXCEPTION 'Owner-beta Paper binding already configured' USING ERRCODE='23505';
  END IF;
  acct:=gen_random_uuid();
  INSERT INTO public.accounts(id,owner_user_id,account_type,name,currency,status,initial_cash,cost_profile_id,cost_profile_version) VALUES(acct,daily.owner_user_id,'PAPER',account_name,'KRW','ACTIVE',10000000.0000,'KRX_ETF_DEFAULT',1);
  INSERT INTO public.account_strategy_bindings(account_id,owner_user_id,strategy_config_id,strategy_id,strategy_version) VALUES(acct,daily.owner_user_id,strategy_config_id,sc.strategy_id,sc.strategy_version);
  INSERT INTO public.cash_ledger(account_id,owner_user_id,seq,event_type,amount,balance,currency,ts,created_at)
    VALUES(acct,daily.owner_user_id,1,'DEPOSIT',10000000.0000,10000000.0000,'KRW',clock_timestamp(),clock_timestamp());
  cost_hash:=public.daily_0060_digest('KRX_ETF_DEFAULT@1|buy_commission=0.00015|sell_commission=0.00015|min_commission=1000.0000|slippage=10bp|min_trade=100000.0000|rebalance_threshold=0.005000|sell_tax=0.00000');
  ev:=gen_random_uuid();
  INSERT INTO public.owner_beta_paper_control(id,owner_user_id,account_id,strategy_config_id,contract_id,contract_sha256,contract_json,cost_profile_id,cost_profile_version,cost_profile_sha256)
   VALUES('etf11',daily.owner_user_id,acct,strategy_config_id,contract->>'contract_id',contract->>'contract_sha256',contract,'KRX_ETF_DEFAULT',1,cost_hash);
  ok:=public.paper_0067_action('CONFIGURE','OK',0,acct,jsonb_build_object('account_id',acct,'event_id',ev,'state','OFF','initial_cash','10000000.0000','currency','KRW'));
  receipt_result:=public.paper_0068_canonical_json(ok);
  INSERT INTO public.owner_beta_paper_events(id,owner_user_id,account_id,control_epoch,kind,payload,payload_sha256,request_key,request_canonical,request_sha256,result_canonical,result_sha256)
   VALUES(ev,daily.owner_user_id,acct,0,'CONFIGURED',jsonb_build_object('schema','owner-beta-paper-configured-v1','account_name',account_name,'strategy_config_id',strategy_config_id,'contract',contract,'initial_cash','10000000.0000','currency','KRW'),public.daily_0060_digest(jsonb_build_object('schema','owner-beta-paper-configured-v1','account_name',account_name,'strategy_config_id',strategy_config_id,'contract',contract,'initial_cash','10000000.0000','currency','KRW')::text),request_key,req,public.daily_0060_digest(req),receipt_result,public.daily_0060_digest(receipt_result));
  PERFORM public.paper_0068_restore_actor(old_actor);
  RETURN ok;
EXCEPTION WHEN query_canceled OR assert_failure THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
WHEN OTHERS THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
END $$;

CREATE FUNCTION public.begin_owner_beta_paper_operation_v1(expected_epoch bigint,intent jsonb,request_key text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE old_actor text:=current_setting('app.actor_user_id',true); c public.owner_beta_paper_control; prior public.owner_beta_paper_operations; latest_gen uuid; ev uuid; gen uuid; installation uuid; acct uuid; fence bigint; ihash text; replay jsonb; req text; receipt_result text; ok jsonb;
BEGIN
  PERFORM public.paper_0068_principal();
  IF intent IS NULL OR jsonb_typeof(intent)<>'object' OR octet_length(intent::text)>65536
     OR NOT public.daily_0060_keys(intent,ARRAY['schema','installation_id','generation','account_id','predecessor_generation','request_key','purpose','scope','expected_epoch','local_epoch_floor','release_manifest_sha256','artifact_sha256','resume_target_id'])
     OR intent->>'schema'<>'owner-beta-paper-operation-intent-v1'
     OR request_key IS NULL OR request_key !~ '^[A-Za-z0-9._:-]{1,128}$'
     OR intent->>'request_key' IS DISTINCT FROM request_key
     OR intent->>'installation_id' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR intent->>'generation' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR jsonb_typeof(intent->'account_id')<>'string' OR intent->>'account_id' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR jsonb_typeof(intent->'predecessor_generation') NOT IN ('string','null') OR jsonb_typeof(intent->'artifact_sha256') NOT IN ('string','null') OR jsonb_typeof(intent->'resume_target_id') NOT IN ('string','null')
     OR intent->'predecessor_generation'<>'null'::jsonb AND intent->>'predecessor_generation' !~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR intent->'resume_target_id'<>'null'::jsonb AND intent->>'resume_target_id' !~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR intent->'artifact_sha256'<>'null'::jsonb AND intent->>'artifact_sha256' !~ '^sha256:[0-9a-f]{64}$'
     OR intent->>'purpose' NOT IN ('STOP','BACKUP','RESTORE') OR intent->>'scope' NOT IN ('PAPER_STOP','COMBINED_BACKUP')
     OR intent->>'release_manifest_sha256' !~ '^sha256:[0-9a-f]{64}$'
     OR jsonb_typeof(intent->'expected_epoch')<>'number' OR jsonb_typeof(intent->'local_epoch_floor')<>'number'
     OR NOT public.paper_0068_nonnegative_i64(intent->>'expected_epoch')
     OR NOT public.paper_0068_nonnegative_i64(intent->>'local_epoch_floor')
  THEN RAISE EXCEPTION 'operation intent wire invalid' USING ERRCODE='22023'; END IF;
  installation:=(intent->>'installation_id')::uuid; gen:=(intent->>'generation')::uuid;
  acct:=CASE WHEN jsonb_typeof(intent->'account_id')='null' THEN NULL ELSE (intent->>'account_id')::uuid END;
  IF acct IS NULL OR acct='00000000-0000-0000-0000-000000000000'::uuid OR intent->>'purpose'='RESTORE' AND intent->>'scope'<>'COMBINED_BACKUP' THEN RAISE EXCEPTION 'operation account or purpose invalid' USING ERRCODE='23514'; END IF;
  IF public.paper_0068_json_depth(intent)>16 THEN RAISE EXCEPTION 'intent depth invalid' USING ERRCODE='22023'; END IF;
  ihash:=public.daily_0060_digest(public.paper_0068_canonical_json(intent));
  req:=public.paper_0068_request('BEGIN',acct,jsonb_build_object('expected_epoch',expected_epoch,'intent_canonical',public.paper_0068_canonical_json(intent),'request_key',request_key));
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE account_id=acct;
  IF c.account_id IS NOT NULL THEN
    PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
    SELECT * INTO prior FROM public.owner_beta_paper_operations op WHERE op.generation=gen OR (op.account_id=acct AND op.request_key=begin_owner_beta_paper_operation_v1.request_key);
    IF FOUND AND prior.request_canonical IS NOT NULL THEN
      PERFORM public.paper_0068_validate_receipt(to_jsonb(prior),'owner_beta_paper_operations');
      IF prior.request_canonical IS DISTINCT FROM req OR prior.request_sha256 IS DISTINCT FROM public.daily_0060_digest(req)
         OR (SELECT count(*) FROM public.owner_beta_paper_operations op WHERE op.generation=gen OR (op.account_id=acct AND op.request_key=begin_owner_beta_paper_operation_v1.request_key))<>1
      THEN RAISE EXCEPTION 'operation exact identity conflicts' USING ERRCODE='23505'; END IF;
      PERFORM public.paper_0068_restore_actor(old_actor);
      RETURN public.paper_0067_action('BEGIN_OPERATION','REPLAY',c.control_epoch,c.account_id,jsonb_build_object('generation',prior.generation,'operation_event_id',prior.fence_event_id,'intent_sha256',prior.intent_sha256,'fenced_epoch',prior.fence_epoch));
    END IF;
  END IF;
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE account_id=acct FOR UPDATE;
  IF NOT FOUND THEN RAISE EXCEPTION 'configured Owner-beta Paper account is missing' USING ERRCODE='55000'; END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  PERFORM pg_advisory_xact_lock(hashtextextended(c.account_id::text,381901));
  SELECT * INTO prior FROM public.owner_beta_paper_operations op WHERE op.generation=gen FOR SHARE;
  IF FOUND THEN
    PERFORM public.paper_0068_validate_receipt(to_jsonb(prior),'owner_beta_paper_operations');
    IF (prior.request_canonical IS NOT NULL AND (prior.request_canonical IS DISTINCT FROM req OR prior.request_sha256 IS DISTINCT FROM public.daily_0060_digest(req)))
       OR prior.expected_epoch IS DISTINCT FROM expected_epoch OR prior.request_key IS DISTINCT FROM request_key
       OR prior.generation<>gen OR prior.account_id<>acct OR prior.intent_sha256<>ihash OR prior.intent IS DISTINCT FROM intent THEN RAISE EXCEPTION 'operation identity replay conflicts' USING ERRCODE='23505'; END IF;
    PERFORM public.paper_0068_restore_actor(old_actor);
    RETURN public.paper_0067_action('BEGIN_OPERATION','REPLAY',c.control_epoch,c.account_id,jsonb_build_object('generation',prior.generation,'operation_event_id',prior.fence_event_id,'intent_sha256',prior.intent_sha256,'fenced_epoch',prior.fence_epoch));
  END IF;
  SELECT * INTO prior FROM public.owner_beta_paper_operations op WHERE op.account_id=acct AND op.request_key=begin_owner_beta_paper_operation_v1.request_key FOR SHARE;
  IF FOUND THEN RAISE EXCEPTION 'operation request key belongs to another generation' USING ERRCODE='23505'; END IF;
  IF expected_epoch IS DISTINCT FROM c.control_epoch OR (intent->>'expected_epoch')::bigint<>expected_epoch THEN RAISE EXCEPTION 'operation expected epoch stale' USING ERRCODE='40001'; END IF;
  IF c.state NOT IN ('OFF','RESTORED_OFF','ENABLED','DRAINING') THEN RAISE EXCEPTION 'operation cannot fence blocked Paper state' USING ERRCODE='55000'; END IF;
  IF c.control_epoch>=9223372036854775806 OR greatest(c.control_epoch,(intent->>'local_epoch_floor')::bigint)>=9223372036854775806 THEN RAISE EXCEPTION 'control epoch overflow' USING ERRCODE='22023'; END IF;
  IF intent->>'purpose'='RESTORE' AND intent->>'scope'<>'COMBINED_BACKUP' THEN RAISE EXCEPTION 'restore must use combined backup scope' USING ERRCODE='23514'; END IF;
  IF intent->>'purpose'<>'RESTORE' AND (intent->>'local_epoch_floor')::bigint<>expected_epoch THEN RAISE EXCEPTION 'ordinary operation floor must equal observed epoch' USING ERRCODE='23514'; END IF;
  IF intent->>'purpose'='STOP' AND intent->>'scope'<>'PAPER_STOP' OR intent->>'purpose'='BACKUP' AND intent->>'scope'<>'COMBINED_BACKUP' THEN RAISE EXCEPTION 'operation purpose/scope mismatch' USING ERRCODE='23514'; END IF;
  SELECT op.generation INTO latest_gen FROM public.owner_beta_paper_operations op WHERE op.account_id=acct ORDER BY op.created_at DESC,op.fence_epoch DESC,op.generation DESC LIMIT 1;
  IF NULLIF(intent->>'predecessor_generation','null')::uuid IS DISTINCT FROM latest_gen THEN RAISE EXCEPTION 'operation predecessor must name the latest retained account generation' USING ERRCODE='23514'; END IF;
  IF c.state='DRAINING' AND NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_events e WHERE e.account_id=acct AND e.kind='DRAIN_STARTED' AND e.control_epoch=c.control_epoch) THEN RAISE EXCEPTION 'draining state has no durable fence event' USING ERRCODE='55000'; END IF;
  fence:=greatest(c.control_epoch,(intent->>'local_epoch_floor')::bigint)+1;
  ev:=gen_random_uuid();
  INSERT INTO public.owner_beta_paper_events(id,owner_user_id,account_id,control_epoch,kind,payload,payload_sha256,request_key)
    VALUES(ev,c.owner_user_id,c.account_id,fence,'DRAIN_STARTED',intent,public.daily_0060_digest(public.paper_0068_canonical_json(intent)),gen::text||':begin');
  ok:=public.paper_0067_action('BEGIN_OPERATION','OK',fence,c.account_id,jsonb_build_object('generation',gen,'operation_event_id',ev,'intent_sha256',ihash,'fenced_epoch',fence));
  receipt_result:=public.paper_0068_canonical_json(ok);
  INSERT INTO public.owner_beta_paper_operations(generation,installation_id,account_id,request_key,purpose,scope,expected_epoch,local_epoch_floor,fence_epoch,release_manifest_sha256,intent,intent_sha256,prior_state,fence_event_id,request_canonical,request_sha256,result_canonical,result_sha256)
    VALUES(gen,installation,acct,request_key,intent->>'purpose',intent->>'scope',expected_epoch,(intent->>'local_epoch_floor')::bigint,fence,intent->>'release_manifest_sha256',intent,ihash,c.state,ev,req,public.daily_0060_digest(req),receipt_result,public.daily_0060_digest(receipt_result));
  UPDATE public.owner_beta_paper_control SET state='DRAINING',control_epoch=fence,updated_at=clock_timestamp() WHERE id='etf11';
  PERFORM public.paper_0068_restore_actor(old_actor);
  RETURN ok;
EXCEPTION WHEN query_canceled OR assert_failure THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
WHEN OTHERS THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
END $$;

CREATE FUNCTION public.paper_0068_gate_new_lineage_work() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE acct uuid; c public.owner_beta_paper_control;
BEGIN
  IF TG_TABLE_NAME='paper_rebalance_previews' THEN
    IF NEW.input_kind<>'OWNER_BETA_DAILY_V1' THEN RETURN NEW; END IF;
    IF TG_OP='UPDATE' AND (NEW.status IS DISTINCT FROM 'APPLIED' OR OLD.status IS NOT DISTINCT FROM NEW.status) THEN RETURN NEW; END IF;
    acct:=NEW.account_id;
  ELSE
    IF NEW.beta_lineage_id IS NULL THEN RETURN NEW; END IF;
    acct:=NEW.account_id;
  END IF;
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE account_id=acct;
  IF NOT FOUND THEN RETURN NEW; END IF;
  IF EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions s
            JOIN public.owner_beta_paper_lineage l ON l.id=(SELECT t.beta_lineage_id FROM public.pending_targets t WHERE t.id=s.pending_target_id)
            WHERE s.account_id=acct AND (s.state<>'VALUED' OR s.valuation_sha256 IS NULL) AND l.control_epoch<c.control_epoch)
  THEN RAISE EXCEPTION 'old-epoch retained Owner-beta lineage must be valued before new target work' USING ERRCODE='55000'; END IF;
  RETURN NEW;
END $$;
ALTER FUNCTION public.paper_0068_gate_new_lineage_work() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.paper_0068_gate_new_lineage_work() FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter;
CREATE TRIGGER paper_0068_gate BEFORE INSERT ON public.paper_rebalance_previews FOR EACH ROW EXECUTE FUNCTION public.paper_0068_gate_new_lineage_work();
CREATE TRIGGER paper_0068_gate_apply BEFORE UPDATE OF status ON public.paper_rebalance_previews FOR EACH ROW EXECUTE FUNCTION public.paper_0068_gate_new_lineage_work();
CREATE TRIGGER paper_0068_gate BEFORE INSERT ON public.pending_targets FOR EACH ROW EXECUTE FUNCTION public.paper_0068_gate_new_lineage_work();

CREATE FUNCTION public.inspect_owner_beta_paper_operation_v1(generation uuid,after_target uuid,page_limit integer) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE old_actor text:=current_setting('app.actor_user_id',true); c public.owner_beta_paper_control; o public.owner_beta_paper_operations; rows jsonb; nxt uuid; rec jsonb; response jsonb; has_more boolean:=false;
BEGIN
  PERFORM public.paper_0068_principal();
  IF current_setting('transaction_isolation')<>'repeatable read' OR current_setting('transaction_read_only')<>'on' THEN RAISE EXCEPTION 'operation readback requires read-only repeatable-read transaction' USING ERRCODE='25001'; END IF;
  IF page_limit IS NULL OR page_limit NOT BETWEEN 1 AND 64 THEN RAISE EXCEPTION 'operation page limit outside 1..64' USING ERRCODE='22023'; END IF;
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11';
  IF NOT FOUND THEN
    IF generation IS NOT NULL OR after_target IS NOT NULL THEN RAISE EXCEPTION 'unconfigured Paper accepts only empty history probe' USING ERRCODE='55000'; END IF;
    IF EXISTS(SELECT 1 FROM public.owner_beta_paper_operations UNION ALL SELECT 1 FROM public.owner_beta_paper_operation_observations UNION ALL SELECT 1 FROM public.owner_beta_paper_recovery_grants UNION ALL SELECT 1 FROM public.owner_beta_paper_events WHERE kind IN ('DRAIN_STARTED','OFF','RESTORED_OFF','RECOVERY_AUTHORIZED')) THEN RAISE EXCEPTION 'unconfigured account has retained Paper history' USING ERRCODE='55000'; END IF;
    PERFORM public.paper_0068_restore_actor(old_actor);
    RETURN public.paper_0067_action('INSPECT_OPERATION','OK',NULL,NULL,jsonb_build_object('generation',NULL,'state',NULL,'control_epoch',NULL,'history_present',false,'operation_event_id',NULL,'reconciliation',NULL,'targets','[]'::jsonb,'next_target',NULL));
  END IF;
    PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  IF generation IS NULL THEN
    IF after_target IS NOT NULL THEN RAISE EXCEPTION 'history probe cursor requires a generation' USING ERRCODE='22023'; END IF;
    PERFORM public.paper_0068_restore_actor(old_actor);
    RETURN public.paper_0067_action('INSPECT_OPERATION','OK',c.control_epoch,c.account_id,jsonb_build_object('generation',NULL,'state',c.state,'control_epoch',c.control_epoch,'history_present',EXISTS(SELECT 1 FROM public.owner_beta_paper_operations UNION ALL SELECT 1 FROM public.owner_beta_paper_operation_observations UNION ALL SELECT 1 FROM public.owner_beta_paper_recovery_grants UNION ALL SELECT 1 FROM public.owner_beta_paper_events WHERE kind IN ('DRAIN_STARTED','OFF','RESTORED_OFF','RECOVERY_AUTHORIZED')),'operation_event_id',NULL,'reconciliation',NULL,'targets','[]'::jsonb,'next_target',NULL));
  END IF;
  SELECT * INTO o FROM public.owner_beta_paper_operations WHERE owner_beta_paper_operations.generation=inspect_owner_beta_paper_operation_v1.generation AND account_id=c.account_id;
  IF NOT FOUND THEN RAISE EXCEPTION 'operation generation is unknown for configured account' USING ERRCODE='55000'; END IF;
  SELECT jsonb_agg(q.target_row ORDER BY q.target_id) INTO rows FROM (
    SELECT t.id AS target_id,jsonb_build_object('target_id',t.id,'lineage_id',t.beta_lineage_id,'effective_session',s.effective_session,'lineage_epoch',l.control_epoch,'target_status',t.status,'session_state',s.state,'settlement_sha256',s.settlement_sha256,'valuation_sha256',s.valuation_sha256,'ledger_before_sha256',s.ledger_before_sha256,'ledger_after_sha256',s.ledger_after_sha256,'account_state_version',s.ledger_after_version,'outbox_id',coalesce(x.id,xa.id)) AS target_row
    FROM public.owner_beta_paper_sessions s JOIN public.pending_targets t ON t.id=s.pending_target_id
    JOIN public.owner_beta_paper_lineage l ON l.id=t.beta_lineage_id
    LEFT JOIN public.paper_settlement_outbox x ON x.pending_target_id=t.id
    LEFT JOIN public.paper_settlement_outbox_archive xa ON xa.pending_target_id=t.id
    WHERE s.account_id=c.account_id AND s.owner_user_id=c.owner_user_id AND (after_target IS NULL OR t.id>after_target)
    ORDER BY t.id LIMIT page_limit+1
  ) q;
  has_more:=COALESCE(jsonb_array_length(rows),0)>page_limit;
  IF has_more THEN
    SELECT jsonb_agg(q.target_row ORDER BY q.target_id) INTO rows FROM (
      SELECT t.id AS target_id,jsonb_build_object('target_id',t.id,'lineage_id',t.beta_lineage_id,'effective_session',s.effective_session,'lineage_epoch',l.control_epoch,'target_status',t.status,'session_state',s.state,'settlement_sha256',s.settlement_sha256,'valuation_sha256',s.valuation_sha256,'ledger_before_sha256',s.ledger_before_sha256,'ledger_after_sha256',s.ledger_after_sha256,'account_state_version',s.ledger_after_version,'outbox_id',coalesce(x.id,xa.id)) AS target_row
      FROM public.owner_beta_paper_sessions s JOIN public.pending_targets t ON t.id=s.pending_target_id JOIN public.owner_beta_paper_lineage l ON l.id=t.beta_lineage_id LEFT JOIN public.paper_settlement_outbox x ON x.pending_target_id=t.id LEFT JOIN public.paper_settlement_outbox_archive xa ON xa.pending_target_id=t.id
      WHERE s.account_id=c.account_id AND s.owner_user_id=c.owner_user_id AND (after_target IS NULL OR t.id>after_target) ORDER BY t.id LIMIT page_limit
    ) q;
    nxt:=(rows->(page_limit-1)->>'target_id')::uuid;
  ELSE nxt:=NULL; END IF;
  SELECT jsonb_build_object('schema','owner-beta-paper-reconciliation-v1','installation_id',o.installation_id,'generation',o.generation,'account_id',o.account_id,'observation_id',x.observation_id,'control_epoch',o.fence_epoch,'inventory_sha256',x.summary->>'inventory_sha256','observed_at',x.summary->>'observed_at','database_sha256',x.summary->>'database_sha256','ledger_sha256',x.summary->>'ledger_sha256','pending_sha256',x.summary->>'pending_sha256','outbox_sha256',x.summary->>'outbox_sha256','jobs_sha256',x.summary->>'jobs_sha256','unresolved_commits',x.summary->'unresolved_commits','live_claims',x.summary->'live_claims','held_targets',x.summary->'held_targets','quarantined_deliveries',x.summary->'quarantined_deliveries') INTO rec
    FROM public.owner_beta_paper_operation_observations x WHERE x.generation=o.generation ORDER BY x.observed_at DESC,x.observation_id DESC LIMIT 1;
  PERFORM public.paper_0068_restore_actor(old_actor);
  response:=public.paper_0067_action('INSPECT_OPERATION','OK',c.control_epoch,c.account_id,jsonb_build_object('generation',o.generation,'state',c.state,'control_epoch',c.control_epoch,'history_present',true,'operation_event_id',o.fence_event_id,'reconciliation',rec,'targets',COALESCE(rows,'[]'::jsonb),'next_target',nxt));
  IF octet_length(response::text)>262144 THEN RAISE EXCEPTION 'operation readback exceeds 256KiB' USING ERRCODE='22023'; END IF;
  RETURN response;
EXCEPTION WHEN OTHERS THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
END $$;

CREATE FUNCTION public.reconcile_owner_beta_paper_operation_v1(generation uuid,expected_epoch bigint,inventory_sha256 text,request_key text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE old_actor text:=current_setting('app.actor_user_id',true); c public.owner_beta_paper_control; o public.owner_beta_paper_operations; prior public.owner_beta_paper_operation_observations; obs uuid; summary jsonb; dbhash text; ledgerhash text; pendinghash text; outboxhash text; jobshash text; unresolved bigint; claims bigint; held bigint; req text; receipt_result text; ok jsonb;
BEGIN
  PERFORM public.paper_0068_principal();
  IF generation IS NULL OR inventory_sha256 IS NULL OR inventory_sha256 !~ '^sha256:[0-9a-f]{64}$' OR request_key IS NULL OR request_key !~ ('^'||generation::text||':reconcile:[0-9]{1,18}$') THEN RAISE EXCEPTION 'reconciliation identity invalid' USING ERRCODE='22023'; END IF;
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11';
  req:=public.paper_0068_request('RECONCILE',c.account_id,jsonb_build_object('generation',generation,'expected_epoch',expected_epoch,'inventory_sha256',inventory_sha256,'request_key',request_key));
  IF c.account_id IS NOT NULL THEN
    PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
    SELECT * INTO prior FROM public.owner_beta_paper_operation_observations t WHERE t.generation=reconcile_owner_beta_paper_operation_v1.generation AND t.request_key=reconcile_owner_beta_paper_operation_v1.request_key;
    IF FOUND AND prior.request_canonical IS NOT NULL THEN
      PERFORM public.paper_0068_validate_receipt(to_jsonb(prior),'owner_beta_paper_operation_observations');
      IF prior.request_canonical IS DISTINCT FROM req OR prior.request_sha256 IS DISTINCT FROM public.daily_0060_digest(req) THEN RAISE EXCEPTION 'reconciliation exact request conflicts' USING ERRCODE='23505'; END IF;
      PERFORM public.paper_0068_restore_actor(old_actor);
      RETURN public.paper_0067_action('RECONCILE_OPERATION','REPLAY',c.control_epoch,c.account_id,jsonb_build_object('observation_id',prior.observation_id,'summary',prior.summary,'summary_sha256',prior.summary_sha256));
    END IF;
  END IF;
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN RAISE EXCEPTION 'configured Paper account missing' USING ERRCODE='55000'; END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  PERFORM pg_advisory_xact_lock(hashtextextended(c.account_id::text,381901));
  SELECT * INTO o FROM public.owner_beta_paper_operations WHERE owner_beta_paper_operations.generation=reconcile_owner_beta_paper_operation_v1.generation AND account_id=c.account_id FOR SHARE;
  IF NOT FOUND THEN RAISE EXCEPTION 'operation is not at expected durable fence' USING ERRCODE='40001'; END IF;
  SELECT * INTO prior FROM public.owner_beta_paper_operation_observations WHERE owner_beta_paper_operation_observations.generation=o.generation AND owner_beta_paper_operation_observations.request_key=reconcile_owner_beta_paper_operation_v1.request_key;
  IF FOUND THEN
    PERFORM public.paper_0068_validate_receipt(to_jsonb(prior),'owner_beta_paper_operation_observations');
    IF (prior.request_canonical IS NOT NULL AND (prior.request_canonical IS DISTINCT FROM req OR prior.request_sha256 IS DISTINCT FROM public.daily_0060_digest(req)))
       OR expected_epoch IS DISTINCT FROM o.fence_epoch OR prior.summary->>'inventory_sha256'<>inventory_sha256 THEN RAISE EXCEPTION 'reconciliation request replay conflicts' USING ERRCODE='23505'; END IF;
    PERFORM public.paper_0068_restore_actor(old_actor);
    RETURN public.paper_0067_action('RECONCILE_OPERATION','REPLAY',c.control_epoch,c.account_id,jsonb_build_object('observation_id',prior.observation_id,'summary',prior.summary,'summary_sha256',prior.summary_sha256));
  END IF;
  IF c.state<>'DRAINING' OR expected_epoch IS DISTINCT FROM c.control_epoch OR o.fence_epoch<>c.control_epoch THEN RAISE EXCEPTION 'operation is not at expected durable fence' USING ERRCODE='40001'; END IF;
  IF EXISTS(SELECT 1 FROM pg_catalog.pg_stat_activity a
            WHERE a.datname=current_database() AND a.pid<>pg_backend_pid()
              AND a.usename IN ('app','worker')
              AND (a.state IS DISTINCT FROM 'idle' OR a.xact_start IS NOT NULL))
     OR EXISTS(SELECT 1 FROM pg_catalog.pg_prepared_xacts p WHERE p.database=current_database())
  THEN RAISE EXCEPTION 'unclassified writer backend or prepared transaction remains' USING ERRCODE='55000'; END IF;

  -- The root controller has already proved process/cgroup/container death and
  -- terminated captured writer backends. Recheck the database boundary above,
  -- then retain attempt identities while making only unfinished work terminal.
  WITH dead_jobs AS MATERIALIZED (
    SELECT j.id,j.attempt_count FROM public.jobs j
    WHERE j.job_type='owner_beta_paper_preview' AND j.owner_user_id=c.owner_user_id AND j.status='RUNNING'
      AND EXISTS(SELECT 1 FROM public.job_attempts a WHERE a.job_id=j.id AND a.attempt_no=j.attempt_count AND a.outcome='RUNNING')
    FOR UPDATE
  )
  UPDATE public.job_attempts a SET outcome='ORPHANED',error_code='owner_beta_paper_fenced',error_message=NULL,finished_at=clock_timestamp()
  FROM dead_jobs j WHERE a.job_id=j.id AND a.attempt_no=j.attempt_count AND a.outcome='RUNNING';
  UPDATE public.jobs j SET status='FAILED',error_code='owner_beta_paper_fenced',
    error_message='Owner-beta Paper operation fenced before preview completion',
    locked_by=NULL,locked_at=NULL,finished_at=clock_timestamp(),updated_at=clock_timestamp()
  WHERE j.job_type='owner_beta_paper_preview' AND j.owner_user_id=c.owner_user_id AND j.status='RUNNING'
    AND EXISTS(SELECT 1 FROM public.job_attempts a WHERE a.job_id=j.id AND a.attempt_no=j.attempt_count AND a.outcome='ORPHANED' AND a.error_code='owner_beta_paper_fenced');

  -- Preserve claim tokens and attempts; expiry returns the dead outbox claim to
  -- the existing retry scan without inventing a new delivery identity.
  UPDATE public.paper_settlement_outbox x SET claim_expires_at=clock_timestamp()
  WHERE x.claim_token IS NOT NULL AND x.delivered_at IS NULL AND x.exhausted_at IS NULL
    AND EXISTS(SELECT 1 FROM public.pending_targets t WHERE t.id=x.pending_target_id AND t.account_id=c.account_id AND t.beta_lineage_id IS NOT NULL);

  WITH changed AS (
    UPDATE public.owner_beta_paper_sessions s SET state='HELD',reason_code='RECOVERY_REQUIRED',updated_at=clock_timestamp()
    FROM public.pending_targets t
    WHERE t.id=s.pending_target_id AND t.account_id=c.account_id AND t.owner_user_id=c.owner_user_id AND t.beta_lineage_id IS NOT NULL
      AND s.settlement_sha256 IS NULL AND (s.state IS DISTINCT FROM 'HELD' OR s.reason_code IS DISTINCT FROM 'RECOVERY_REQUIRED')
    RETURNING s.pending_target_id
  )
  INSERT INTO public.owner_beta_paper_session_events(pending_target_id,event_seq,kind,reason_code,payload)
  SELECT changed.pending_target_id,
    COALESCE((SELECT max(e.event_seq) FROM public.owner_beta_paper_session_events e WHERE e.pending_target_id=changed.pending_target_id),0)+1,
    'RECOVERY_REQUIRED','RECOVERY_REQUIRED',jsonb_build_object('schema','owner-beta-paper-operation-hold-v1','generation',o.generation,'request_key',request_key)
  FROM changed;

  -- Re-read all database-owned beta state under the control/advisory barrier.
  ledgerhash:=public.paper_0068_stream_digest(COALESCE((SELECT jsonb_agg(jsonb_build_object('table',q.kind,'row_key',q.row_key,'row',q.row_value) ORDER BY q.kind,q.row_key) FROM (
    SELECT 'cash_ledger'::text kind,cl.account_id::text||':'||cl.seq::text row_key,jsonb_build_object('id',cl.id,'account_id',cl.account_id,'owner_user_id',cl.owner_user_id,'seq',cl.seq,'event_type',cl.event_type,'amount',to_char(cl.amount,'FM99999999999990.0000'),'balance',to_char(cl.balance,'FM99999999999990.0000'),'currency',cl.currency,'reference_id',cl.reference_id,'ts',public.paper_0068_utc(cl.ts),'created_at',public.paper_0068_utc(cl.created_at)) row_value FROM public.cash_ledger cl WHERE cl.account_id=c.account_id AND cl.owner_user_id=c.owner_user_id
    UNION ALL SELECT 'positions',p.account_id::text||':'||p.instrument_id,jsonb_build_object('id',p.id,'account_id',p.account_id,'owner_user_id',p.owner_user_id,'instrument_id',p.instrument_id,'quantity',to_char(p.quantity,'FM99999999999990.0000'),'avg_price',to_char(p.avg_price,'FM99999999999990.0000'),'updated_at',public.paper_0068_utc(p.updated_at)) FROM public.positions p WHERE p.account_id=c.account_id AND p.owner_user_id=c.owner_user_id
    UNION ALL SELECT 'orders',o0.account_id::text||':'||o0.order_ref,jsonb_build_object('id',o0.id,'account_id',o0.account_id,'owner_user_id',o0.owner_user_id,'order_ref',o0.order_ref,'instrument_id',o0.instrument_id,'side',o0.side,'quantity',to_char(o0.quantity,'FM99999999999990.0000'),'price',to_char(o0.price,'FM99999999999990.0000'),'status',o0.status,'submitted_at',public.paper_0068_utc(o0.submitted_at),'created_at',public.paper_0068_utc(o0.created_at),'updated_at',public.paper_0068_utc(o0.updated_at)) FROM public.orders o0 WHERE o0.account_id=c.account_id AND o0.owner_user_id=c.owner_user_id
    UNION ALL SELECT 'fills',f.account_id::text||':'||f.fill_ref,jsonb_build_object('id',f.id,'account_id',f.account_id,'owner_user_id',f.owner_user_id,'order_id',f.order_id,'instrument_id',f.instrument_id,'fill_ref',f.fill_ref,'side',f.side,'quantity',to_char(f.quantity,'FM99999999999990.0000'),'price',to_char(f.price,'FM99999999999990.0000'),'fees',to_char(f.fees,'FM99999999999990.0000'),'ts',public.paper_0068_utc(f.ts),'created_at',public.paper_0068_utc(f.created_at)) FROM public.fills f WHERE f.account_id=c.account_id AND f.owner_user_id=c.owner_user_id
    UNION ALL SELECT 'daily_equity',e.account_id::text||':'||e.trading_date::text,jsonb_build_object('id',e.id,'account_id',e.account_id,'owner_user_id',e.owner_user_id,'trading_date',e.trading_date,'equity',to_char(e.equity,'FM99999999999990.0000'),'cash',to_char(e.cash,'FM99999999999990.0000'),'positions_value',to_char(e.positions_value,'FM99999999999990.0000'),'currency',e.currency,'created_at',public.paper_0068_utc(e.created_at)) FROM public.daily_equity e WHERE e.account_id=c.account_id AND e.owner_user_id=c.owner_user_id
  ) q(kind,row_key,row_value)), '[]'::jsonb));
  pendinghash:=public.paper_0068_stream_digest(COALESCE((
    SELECT jsonb_agg(jsonb_build_object('table',q.kind,'row_key',q.row_key,'row',q.row_value) ORDER BY q.kind,q.row_key)
    FROM (
      SELECT 'pending_targets'::text kind,t.id::text row_key,jsonb_build_object(
        'id',t.id,'account_id',t.account_id,'owner_user_id',t.owner_user_id,'strategy_config_id',t.strategy_config_id,
        'computed_on',t.computed_on,'effective_date',t.effective_date,'targets_json',t.targets_json,'status',t.status,
        'executed_at',public.paper_0068_utc(t.executed_at),'created_at',public.paper_0068_utc(t.created_at),
        'dataset_version',t.dataset_version,'recommendation_run_id',t.recommendation_run_id,'source_kind',t.source_kind,
        'dataset_version_id',t.dataset_version_id,'dataset_manifest_sha256',t.dataset_manifest_sha256,
        'non_execution_reason',t.non_execution_reason,'beta_lineage_id',t.beta_lineage_id) row_value
      FROM public.pending_targets t WHERE t.account_id=c.account_id AND t.owner_user_id=c.owner_user_id AND t.beta_lineage_id IS NOT NULL
      UNION ALL
      SELECT 'owner_beta_paper_sessions',s.pending_target_id::text,jsonb_build_object(
        'pending_target_id',s.pending_target_id,'owner_user_id',s.owner_user_id,'account_id',s.account_id,
        'effective_session',s.effective_session,'account_state_version',s.account_state_version,'account_state_sha256',s.account_state_sha256,
        'state',s.state,'settlement_kind',s.settlement_kind,'reason_code',s.reason_code,
        'execution_admission_sha256',s.execution_admission_sha256,'valuation_admission_sha256',s.valuation_admission_sha256,
        'execution_input_sha256',s.execution_input_sha256,'valuation_input_sha256',s.valuation_input_sha256,
        'ledger_before_sha256',s.ledger_before_sha256,'ledger_after_sha256',s.ledger_after_sha256,
        'ledger_after_version',s.ledger_after_version,'ledger_after_preimage',s.ledger_after_preimage,
        'settlement_sha256',s.settlement_sha256,'valuation_sha256',s.valuation_sha256,
        'source_original_retrieved_at',public.paper_0068_utc(s.source_original_retrieved_at),
        'input_admitted_at',public.paper_0068_utc(s.input_admitted_at),'usable_at',public.paper_0068_utc(s.usable_at),
        'settled_at',public.paper_0068_utc(s.settled_at),'valued_at',public.paper_0068_utc(s.valued_at),
        'processing_origin',s.processing_origin,'receipt_json',s.receipt_json,'updated_at',public.paper_0068_utc(s.updated_at)) row_value
      FROM public.owner_beta_paper_sessions s JOIN public.pending_targets t ON t.id=s.pending_target_id
      WHERE s.account_id=c.account_id AND s.owner_user_id=c.owner_user_id AND t.beta_lineage_id IS NOT NULL
      UNION ALL
      SELECT 'owner_beta_paper_lineage',l.id::text,jsonb_build_object(
        'id',l.id,'owner_user_id',l.owner_user_id,'account_id',l.account_id,'strategy_config_id',l.strategy_config_id,
        'recommendation_run_id',l.recommendation_run_id,'signal_date',l.signal_date,'effective_session',l.effective_session,
        'signal_admission_sha256',l.signal_admission_sha256,'factor_snapshot_sha256',l.factor_snapshot_sha256,
        'target_snapshot_sha256',l.target_snapshot_sha256,'target_weights_json',l.target_weights_json,
        'config_epoch',l.config_epoch,'daily_activation_epoch',l.daily_activation_epoch,'control_epoch',l.control_epoch,
        'consent_event_id',l.consent_event_id,'activation_event_id',l.activation_event_id,'cohort_sha256',l.cohort_sha256,
        'calendar_evidence_sha256',l.calendar_evidence_sha256,'calendar_link_json',l.calendar_link_json,
        'created_at',public.paper_0068_utc(l.created_at)) row_value
      FROM public.owner_beta_paper_lineage l WHERE l.account_id=c.account_id AND l.owner_user_id=c.owner_user_id
      UNION ALL
      SELECT 'owner_beta_paper_session_events',e.pending_target_id::text||':'||lpad(e.event_seq::text,20,'0'),jsonb_build_object(
        'id',e.id,'pending_target_id',e.pending_target_id,'event_seq',e.event_seq,'kind',e.kind,
        'reason_code',e.reason_code,'payload',e.payload,'occurred_at',public.paper_0068_utc(e.occurred_at))
      FROM public.owner_beta_paper_session_events e
      JOIN public.owner_beta_paper_sessions s ON s.pending_target_id=e.pending_target_id
      JOIN public.pending_targets t ON t.id=s.pending_target_id
      WHERE s.account_id=c.account_id AND s.owner_user_id=c.owner_user_id AND t.beta_lineage_id IS NOT NULL
    ) q
  ),'[]'::jsonb));
  outboxhash:=public.paper_0068_stream_digest(COALESCE((SELECT jsonb_agg(jsonb_build_object('table',q.kind,'row_key',q.row_key,'row',q.row_value) ORDER BY q.kind,q.row_key) FROM (
    SELECT 'paper_settlement_outbox'::text kind,x.id::text row_key,jsonb_build_object('id',x.id,'pending_target_id',x.pending_target_id,'owner_user_id',x.owner_user_id,'severity',x.severity,'kind',x.kind,'title',x.title,'body',x.body,'parity_json',x.parity_json,'attempts',x.attempts,'max_attempts',x.max_attempts,'available_at',public.paper_0068_utc(x.available_at),'delivered_at',public.paper_0068_utc(x.delivered_at),'exhausted_at',public.paper_0068_utc(x.exhausted_at),'last_error_sha256',CASE WHEN x.last_error IS NULL THEN NULL ELSE public.daily_0060_digest(x.last_error) END,'claim_token',x.claim_token,'claim_expires_at',public.paper_0068_utc(x.claim_expires_at),'created_at',public.paper_0068_utc(x.created_at)) row_value FROM public.paper_settlement_outbox x WHERE x.owner_user_id=c.owner_user_id AND EXISTS(SELECT 1 FROM public.pending_targets t WHERE t.id=x.pending_target_id AND t.account_id=c.account_id)
    UNION ALL SELECT 'paper_settlement_outbox_archive',a.id::text,jsonb_build_object('id',a.id,'pending_target_id',a.pending_target_id,'owner_user_id',a.owner_user_id,'severity',a.severity,'kind',a.kind,'title',a.title,'body',a.body,'parity_json',a.parity_json,'attempts',a.attempts,'max_attempts',a.max_attempts,'delivered_at',public.paper_0068_utc(a.delivered_at),'created_at',public.paper_0068_utc(a.created_at),'archived_at',public.paper_0068_utc(a.archived_at)) FROM public.paper_settlement_outbox_archive a WHERE a.owner_user_id=c.owner_user_id AND EXISTS(SELECT 1 FROM public.pending_targets t WHERE t.id=a.pending_target_id AND t.account_id=c.account_id)
    UNION ALL SELECT 'notifications',n.id::text,jsonb_build_object('id',n.id,'owner_user_id',n.owner_user_id,'kind',n.kind,'title',n.title,'body',n.body,'read_at',public.paper_0068_utc(n.read_at),'created_at',public.paper_0068_utc(n.created_at),'source_key',n.source_key) FROM public.notifications n WHERE n.owner_user_id=c.owner_user_id AND EXISTS(SELECT 1 FROM public.paper_settlement_outbox x WHERE x.owner_user_id=n.owner_user_id AND n.source_key=x.id::text AND EXISTS(SELECT 1 FROM public.pending_targets t WHERE t.id=x.pending_target_id AND t.account_id=c.account_id) UNION ALL SELECT 1 FROM public.paper_settlement_outbox_archive a WHERE a.owner_user_id=n.owner_user_id AND n.source_key=a.id::text AND EXISTS(SELECT 1 FROM public.pending_targets t WHERE t.id=a.pending_target_id AND t.account_id=c.account_id))
    UNION ALL SELECT 'notification_deliveries',d.notification_id::text||':'||d.channel,jsonb_build_object('id',d.id,'notification_id',d.notification_id,'owner_user_id',d.owner_user_id,'channel',d.channel,'status',d.status,'error_detail_sha256',CASE WHEN d.error_detail IS NULL THEN NULL ELSE public.daily_0060_digest(d.error_detail) END,'attempted_at',public.paper_0068_utc(d.attempted_at),'delivery_token',d.delivery_token,'delivery_lease_expires_at',public.paper_0068_utc(d.delivery_lease_expires_at),'delivery_attempts',d.delivery_attempts) FROM public.notification_deliveries d JOIN public.notifications n ON n.id=d.notification_id AND n.owner_user_id=d.owner_user_id WHERE d.owner_user_id=c.owner_user_id AND EXISTS(SELECT 1 FROM public.paper_settlement_outbox x WHERE x.owner_user_id=n.owner_user_id AND n.source_key=x.id::text AND EXISTS(SELECT 1 FROM public.pending_targets t WHERE t.id=x.pending_target_id AND t.account_id=c.account_id) UNION ALL SELECT 1 FROM public.paper_settlement_outbox_archive a WHERE a.owner_user_id=n.owner_user_id AND n.source_key=a.id::text AND EXISTS(SELECT 1 FROM public.pending_targets t WHERE t.id=a.pending_target_id AND t.account_id=c.account_id))
  ) q(kind,row_key,row_value)),'[]'::jsonb));
  jobshash:=public.paper_0068_stream_digest(COALESCE((SELECT jsonb_agg(jsonb_build_object('table',q.kind,'row_key',q.row_key,'row',q.row_value) ORDER BY q.kind,q.row_key) FROM (
    SELECT 'jobs'::text kind,j.id::text row_key,jsonb_build_object('id',j.id,'owner_user_id',j.owner_user_id,'job_type',j.job_type,'status',j.status,'priority',j.priority,'idempotency_key',j.idempotency_key,'payload_json',j.payload_json,'max_attempts',j.max_attempts,'attempt_count',j.attempt_count,'available_at',public.paper_0068_utc(j.available_at),'locked_by',j.locked_by,'locked_at',public.paper_0068_utc(j.locked_at),'started_at',public.paper_0068_utc(j.started_at),'finished_at',public.paper_0068_utc(j.finished_at),'error_code',j.error_code,'error_message_sha256',CASE WHEN j.error_message IS NULL THEN NULL ELSE public.daily_0060_digest(j.error_message) END,'created_at',public.paper_0068_utc(j.created_at),'updated_at',public.paper_0068_utc(j.updated_at)) row_value FROM public.jobs j WHERE j.job_type='owner_beta_paper_preview' AND j.owner_user_id=c.owner_user_id
    UNION ALL SELECT 'job_attempts',a.id::text,jsonb_build_object('id',a.id,'job_id',a.job_id,'attempt_no',a.attempt_no,'outcome',a.outcome,'claimed_by',a.claimed_by,'error_code',a.error_code,'error_message_sha256',CASE WHEN a.error_message IS NULL THEN NULL ELSE public.daily_0060_digest(a.error_message) END,'started_at',public.paper_0068_utc(a.started_at),'finished_at',public.paper_0068_utc(a.finished_at),'created_at',public.paper_0068_utc(a.created_at)) FROM public.job_attempts a JOIN public.jobs j ON j.id=a.job_id WHERE j.job_type='owner_beta_paper_preview' AND j.owner_user_id=c.owner_user_id
    UNION ALL SELECT 'paper_rebalance_previews',p.id::text,jsonb_build_object('id',p.id,'owner_user_id',p.owner_user_id,'account_id',p.account_id,'recommendation_run_id',p.recommendation_run_id,'target_portfolio_id',p.target_portfolio_id,'strategy_config_id',p.strategy_config_id,'job_id',p.job_id,'status',p.status,'price_basis',p.price_basis,'price_date',p.price_date,'preview_seoul_date',p.preview_seoul_date,'proposed_effective_date',p.proposed_effective_date,'dataset_version_id',p.dataset_version_id,'dataset_manifest_sha256',p.dataset_manifest_sha256,'cost_profile_id',p.cost_profile_id,'cost_profile_version',p.cost_profile_version,'account_state_version',p.account_state_version,'account_state_sha256',p.account_state_sha256,'target_portfolio_sha256',p.target_portfolio_sha256,'preview_token',p.preview_token,'result_json',p.result_json,'error_json',p.error_json,'pending_target_id',p.pending_target_id,'created_at',public.paper_0068_utc(p.created_at),'started_at',public.paper_0068_utc(p.started_at),'completed_at',public.paper_0068_utc(p.completed_at),'applied_at',public.paper_0068_utc(p.applied_at),'updated_at',public.paper_0068_utc(p.updated_at),'input_kind',p.input_kind,'beta_apply_mode',p.beta_apply_mode,'beta_lineage_id',p.beta_lineage_id,'beta_request_key',p.beta_request_key,'account_state_preimage',p.account_state_preimage,'signal_allocation_sha256',p.signal_allocation_sha256) FROM public.paper_rebalance_previews p WHERE p.account_id=c.account_id AND p.owner_user_id=c.owner_user_id AND p.input_kind='OWNER_BETA_DAILY_V1'
  ) q(kind,row_key,row_value)),'[]'::jsonb));
  unresolved:=(SELECT count(*) FROM public.pending_targets t
    LEFT JOIN public.owner_beta_paper_sessions s ON s.pending_target_id=t.id
    LEFT JOIN public.owner_beta_paper_lineage l ON l.id=t.beta_lineage_id
    WHERE t.account_id=c.account_id AND t.owner_user_id=c.owner_user_id AND t.beta_lineage_id IS NOT NULL
      AND (s.pending_target_id IS NULL OR l.id IS NULL OR l.account_id IS DISTINCT FROM t.account_id
        OR l.owner_user_id IS DISTINCT FROM t.owner_user_id
        OR (s.settlement_sha256 IS NULL AND (t.status<>'PENDING' OR s.state NOT IN ('PENDING','WAITING_PRICE','WAITING_ADMISSION','HELD','BLOCKED','RECOVERY_REQUIRED')))
        OR (s.settlement_sha256 IS NOT NULL AND NOT ((s.state IN ('SETTLED_TRADE','VALUED') AND t.status='EXECUTED') OR (s.state IN ('SETTLED_NO_TRADE','VALUED') AND t.status='SKIPPED')))
        OR (s.valuation_sha256 IS NOT NULL AND s.state<>'VALUED') OR (s.state='VALUED' AND s.valuation_sha256 IS NULL)))
    +(SELECT count(*) FROM public.owner_beta_paper_lineage l
      LEFT JOIN public.pending_targets t ON t.beta_lineage_id=l.id
      WHERE l.account_id=c.account_id AND l.owner_user_id=c.owner_user_id AND t.id IS NULL);
  claims:=(SELECT count(*) FROM public.jobs j WHERE j.job_type='owner_beta_paper_preview' AND j.owner_user_id=c.owner_user_id AND j.status='RUNNING')
    +(SELECT count(*) FROM public.paper_settlement_outbox x WHERE x.claim_token IS NOT NULL AND x.claim_expires_at>clock_timestamp()
       AND EXISTS(SELECT 1 FROM public.pending_targets t WHERE t.id=x.pending_target_id AND t.account_id=c.account_id AND t.beta_lineage_id IS NOT NULL));
  held:=(SELECT count(*) FROM public.owner_beta_paper_sessions s WHERE s.account_id=c.account_id AND s.owner_user_id=c.owner_user_id AND s.state IN ('HELD','BLOCKED','RECOVERY_REQUIRED'));
  dbhash:=public.paper_0068_stream_digest(COALESCE((
    SELECT jsonb_agg(jsonb_build_object('table',q.kind,'row_key',q.row_key,'row',q.row_value) ORDER BY q.kind,q.row_key)
    FROM (
      SELECT 'account_strategy_bindings'::text kind,b.id::text row_key,jsonb_build_object(
        'id',b.id,'account_id',b.account_id,'owner_user_id',b.owner_user_id,'strategy_config_id',b.strategy_config_id,
        'strategy_id',b.strategy_id,'strategy_version',b.strategy_version,'bound_at',public.paper_0068_utc(b.bound_at),
        'unbound_at',public.paper_0068_utc(b.unbound_at)) row_value
      FROM public.account_strategy_bindings b WHERE b.account_id=c.account_id AND b.owner_user_id=c.owner_user_id
      UNION ALL
      SELECT 'accounts',a.id::text,jsonb_build_object(
        'id',a.id,'owner_user_id',a.owner_user_id,'account_type',a.account_type,'name',a.name,'currency',a.currency,
        'status',a.status,'created_at',public.paper_0068_utc(a.created_at),'updated_at',public.paper_0068_utc(a.updated_at),
        'initial_cash',to_char(a.initial_cash,'FM99999999999990.0000'),'cost_profile_id',a.cost_profile_id,
        'cost_profile_version',a.cost_profile_version,'paper_state_version',a.paper_state_version)
      FROM public.accounts a WHERE a.id=c.account_id AND a.owner_user_id=c.owner_user_id
      UNION ALL
      SELECT 'jobs',c.account_id::text,to_jsonb(jobshash)
      UNION ALL
      SELECT 'ledger',c.account_id::text,to_jsonb(ledgerhash)
      UNION ALL
      SELECT 'outbox',c.account_id::text,to_jsonb(outboxhash)
      UNION ALL
      SELECT 'owner_beta_paper_control',c.id,jsonb_build_object(
        'id',c.id,'owner_user_id',c.owner_user_id,'account_id',c.account_id,'strategy_config_id',c.strategy_config_id,
        'state',c.state,'control_epoch',c.control_epoch,'contract_id',c.contract_id,'contract_sha256',c.contract_sha256,
        'contract_json',c.contract_json,'cost_profile_id',c.cost_profile_id,'cost_profile_version',c.cost_profile_version,
        'cost_profile_sha256',c.cost_profile_sha256,'cohort_sha256',c.cohort_sha256,'daily_cohort',c.daily_cohort,
        'process_binding',c.process_binding,'process_binding_sha256',c.process_binding_sha256,
        'consent_event_id',c.consent_event_id,'activation_event_id',c.activation_event_id,
        'last_signal_allocation_sha256',c.last_signal_allocation_sha256,'last_signal_date',c.last_signal_date,
        'updated_at',public.paper_0068_utc(c.updated_at))
      UNION ALL
      SELECT 'owner_beta_paper_events',e.id::text,jsonb_build_object(
        'id',e.id,'owner_user_id',e.owner_user_id,'account_id',e.account_id,'control_epoch',e.control_epoch,
        'kind',e.kind,'actor_user_id',e.actor_user_id,'occurred_at',public.paper_0068_utc(e.occurred_at),
        'payload',e.payload,'payload_sha256',e.payload_sha256,'request_key',e.request_key,'pending_target_id',e.pending_target_id,'request_canonical',e.request_canonical,'request_sha256',e.request_sha256,'result_canonical',e.result_canonical,'result_sha256',e.result_sha256)
      FROM public.owner_beta_paper_events e WHERE e.account_id=c.account_id AND e.owner_user_id=c.owner_user_id
      UNION ALL
      SELECT 'pending',c.account_id::text,to_jsonb(pendinghash)
    ) q(kind,row_key,row_value)
  ),'[]'::jsonb));
  obs:=gen_random_uuid();
  summary:=jsonb_build_object('schema','owner-beta-paper-reconciliation-v1','installation_id',o.installation_id,'generation',o.generation,'account_id',o.account_id,'observation_id',obs,'control_epoch',c.control_epoch,'inventory_sha256',inventory_sha256,'observed_at',to_char(clock_timestamp() AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS"Z"'),'database_sha256',dbhash,'ledger_sha256',ledgerhash,'pending_sha256',pendinghash,'outbox_sha256',outboxhash,'jobs_sha256',jobshash,'unresolved_commits',unresolved,'live_claims',claims,'held_targets',held,'quarantined_deliveries',(SELECT count(*) FROM public.notification_deliveries d JOIN public.notifications n ON n.id=d.notification_id AND n.owner_user_id=d.owner_user_id WHERE d.owner_user_id=c.owner_user_id AND d.delivery_token IS NOT NULL AND EXISTS(SELECT 1 FROM public.paper_settlement_outbox x WHERE x.owner_user_id=n.owner_user_id AND n.source_key=x.id::text AND EXISTS(SELECT 1 FROM public.pending_targets t WHERE t.id=x.pending_target_id AND t.account_id=c.account_id) UNION ALL SELECT 1 FROM public.paper_settlement_outbox_archive a WHERE a.owner_user_id=n.owner_user_id AND n.source_key=a.id::text AND EXISTS(SELECT 1 FROM public.pending_targets t WHERE t.id=a.pending_target_id AND t.account_id=c.account_id))));
  ok:=public.paper_0067_action('RECONCILE_OPERATION','OK',c.control_epoch,c.account_id,jsonb_build_object('observation_id',obs,'summary',summary,'summary_sha256',public.daily_0060_digest(public.paper_0068_canonical_json(summary))));
  receipt_result:=public.paper_0068_canonical_json(ok);
  INSERT INTO public.owner_beta_paper_operation_observations(observation_id,generation,request_key,summary,summary_sha256,request_canonical,request_sha256,result_canonical,result_sha256)
    VALUES(obs,o.generation,request_key,summary,public.daily_0060_digest(public.paper_0068_canonical_json(summary)),req,public.daily_0060_digest(req),receipt_result,public.daily_0060_digest(receipt_result));
  PERFORM public.paper_0068_restore_actor(old_actor);
  RETURN ok;
EXCEPTION WHEN query_canceled OR assert_failure THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
WHEN OTHERS THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
END $$;

CREATE FUNCTION public.paper_0068_recovery_authority(p_target uuid,p_epoch bigint,p_process_sha256 text,p_action text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE c public.owner_beta_paper_control; s public.owner_beta_paper_sessions; g public.owner_beta_paper_recovery_grants; t public.pending_targets; l public.owner_beta_paper_lineage; d public.owner_beta_daily_control; grant_found boolean; dispatch_event public.owner_beta_paper_events; old_actor text:=current_setting('app.actor_user_id',true);
BEGIN
  -- This helper is intentionally usable only from a serving SECURITY DEFINER
  -- call or migration-owner recovery operation. The caller's public ABI still
  -- enforces app/worker and current process authority.
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11';
  IF NOT FOUND THEN RAISE EXCEPTION 'recovery authority unavailable' USING ERRCODE='55000'; END IF;
  IF NULLIF(old_actor,'') IS NOT NULL AND old_actor<>c.owner_user_id::text THEN RAISE EXCEPTION 'recovery actor mismatch' USING ERRCODE='42501'; END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT * INTO s FROM public.owner_beta_paper_sessions WHERE pending_target_id=p_target AND account_id=c.account_id AND owner_user_id=c.owner_user_id FOR UPDATE;
  SELECT * INTO g FROM public.owner_beta_paper_recovery_grants WHERE pending_target_id=p_target AND active_epoch=p_epoch ORDER BY created_at DESC,grant_id DESC LIMIT 1 FOR UPDATE;
  grant_found:=FOUND;
  IF NOT grant_found OR g.revoked_at IS NOT NULL THEN
    PERFORM public.paper_0068_restore_actor(old_actor);
    RETURN NULL;
  END IF;
  SELECT * INTO t FROM public.pending_targets WHERE id=p_target AND account_id=c.account_id AND owner_user_id=c.owner_user_id AND beta_lineage_id=g.lineage_id;
  SELECT * INTO l FROM public.owner_beta_paper_lineage WHERE id=g.lineage_id AND account_id=c.account_id AND owner_user_id=c.owner_user_id FOR SHARE;
  SELECT * INTO d FROM public.owner_beta_daily_control WHERE id='etf11' AND owner_user_id=c.owner_user_id FOR SHARE;
  SELECT * INTO dispatch_event FROM public.owner_beta_paper_events e
    WHERE e.account_id=c.account_id AND e.request_key=g.request_key;
  IF NOT FOUND THEN RAISE EXCEPTION 'recovery dispatch missing' USING ERRCODE='42501'; END IF;
  PERFORM public.paper_0068_validate_receipt(to_jsonb(dispatch_event),'owner_beta_paper_events');
  IF dispatch_event.kind IS DISTINCT FROM 'RECOVERY_AUTHORIZED' OR dispatch_event.payload IS DISTINCT FROM g.decision
     OR dispatch_event.pending_target_id IS DISTINCT FROM p_target OR dispatch_event.control_epoch IS DISTINCT FROM g.active_epoch
     OR dispatch_event.owner_user_id IS DISTINCT FROM c.owner_user_id OR dispatch_event.actor_user_id IS DISTINCT FROM c.owner_user_id
     OR dispatch_event.result_canonical::jsonb#>>'{data,grant_id}' IS DISTINCT FROM g.grant_id::text
     OR dispatch_event.request_canonical IS DISTINCT FROM public.paper_0068_request('TRANSITION',c.account_id,
       jsonb_build_object('expected_epoch',p_epoch,'transition','RECOVERY','evidence_canonical',public.paper_0068_canonical_json(g.decision),'request_key',g.request_key))
     OR g.decision->>'request_key' IS DISTINCT FROM g.request_key OR g.decision->>'account_id' IS DISTINCT FROM c.account_id::text
     OR g.decision->>'step' IS DISTINCT FROM g.step OR (g.decision->>'expires_at')::timestamptz IS DISTINCT FROM g.expires_at
     OR g.decision->>'decision_sha256' IS DISTINCT FROM g.decision_sha256
     OR g.decision_sha256 IS DISTINCT FROM public.daily_0060_digest(public.paper_0068_canonical_json(jsonb_build_object(
        'effective_session',g.decision->'effective_session','pending_target_id',g.decision->'pending_target_id','step',g.decision->'step')))
  THEN RAISE EXCEPTION 'recovery dispatch identity mismatch' USING ERRCODE='42501'; END IF;
  IF g.account_id<>c.account_id OR g.expires_at<=clock_timestamp() OR c.state<>'ENABLED'
     OR c.control_epoch<>p_epoch OR c.consent_event_id IS DISTINCT FROM g.consent_event_id
     OR c.process_binding_sha256 IS DISTINCT FROM g.process_config_sha256 OR p_process_sha256 IS DISTINCT FROM g.process_config_sha256
     OR t.id IS NULL OR l.id IS NULL OR g.lineage_id IS DISTINCT FROM t.beta_lineage_id
     OR g.lineage_epoch IS DISTINCT FROM l.control_epoch OR g.effective_session IS DISTINCT FROM s.effective_session
     OR l.strategy_config_id IS DISTINCT FROM c.strategy_config_id
     OR g.signal_admission_sha256 IS DISTINCT FROM l.signal_admission_sha256
     OR g.original_receipt_sha256 IS DISTINCT FROM (CASE WHEN s.settlement_sha256 IS NULL
          THEN public.daily_0060_digest(public.paper_0068_canonical_json(COALESCE(s.receipt_json,'null'::jsonb)))
          WHEN g.step='SETTLE_THEN_VALUE' AND g.settlement_receipt_sha256=s.settlement_sha256
            THEN g.original_receipt_sha256 -- immutable grant links original pre-settlement receipt to its consumed settlement
          ELSE s.settlement_sha256 END)
     OR g.decision->>'pending_target_id' IS DISTINCT FROM p_target::text
     OR g.decision->>'lineage_id' IS DISTINCT FROM l.id::text
     OR g.decision->>'lineage_epoch' IS DISTINCT FROM g.lineage_epoch::text
     OR g.decision->>'active_epoch' IS DISTINCT FROM p_epoch::text
     OR g.decision->>'original_receipt_sha256' IS DISTINCT FROM g.original_receipt_sha256
     OR g.decision->>'consent_event_id' IS DISTINCT FROM g.consent_event_id::text
     OR g.decision->>'process_config_sha256' IS DISTINCT FROM g.process_config_sha256
     OR g.decision->>'signal_admission_sha256' IS DISTINCT FROM g.signal_admission_sha256
     OR g.decision->>'execution_admission_sha256' IS DISTINCT FROM g.execution_admission_sha256
     OR NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_events e WHERE e.id=c.consent_event_id AND e.kind='CONSENT_GRANTED' AND e.account_id=c.account_id)
     OR NOT EXISTS(SELECT 1 FROM public.data_entitlements de WHERE de.contract_reference=c.daily_cohort->>'entitlement_reference' AND de.contract_document_sha256=substring(c.daily_cohort->>'entitlement_sha256',8) AND de.status='ACTIVE' AND de.effective_from<=s.effective_session AND de.effective_until>=s.effective_session AND de.effective_from<=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date AND de.effective_until>=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date AND de.covered_datasets @> '["krx_eod_bars"]'::jsonb AND de.covered_uses @> '["recommendation","paper_view"]'::jsonb)
     OR d.owner_user_id IS NULL OR NOT COALESCE(d.enabled,false) OR NOT public.daily_0060_admission_usable(d,g.signal_admission_sha256,false) OR NOT public.daily_0060_admission_usable(d,g.execution_admission_sha256,false)
     OR s.pending_target_id IS NULL OR s.effective_session<>g.effective_session
     OR (p_action='SETTLE' AND (g.step<>'SETTLE_THEN_VALUE' OR g.settlement_receipt_sha256 IS NOT NULL OR s.settlement_sha256 IS NOT NULL))
     OR (p_action='SETTLE' AND (t.status<>'PENDING' OR s.state NOT IN ('PENDING','WAITING_PRICE','WAITING_ADMISSION','HELD','BLOCKED','RECOVERY_REQUIRED')))
     OR (p_action='VALUE' AND (g.step NOT IN ('SETTLE_THEN_VALUE','VALUE_ONLY') OR g.step='SETTLE_THEN_VALUE' AND (g.settlement_receipt_sha256 IS NULL OR g.settlement_receipt_sha256 IS DISTINCT FROM s.settlement_sha256) OR s.settlement_sha256 IS NULL OR g.valuation_receipt_sha256 IS NOT NULL))
     OR (p_action='VALUE' AND NOT ((s.state='SETTLED_TRADE' AND t.status='EXECUTED') OR (s.state='SETTLED_NO_TRADE' AND t.status='SKIPPED')))
     OR (p_action='SETTLE_READBACK' AND (g.step<>'SETTLE_THEN_VALUE' OR g.settlement_receipt_sha256 IS NULL
          OR g.settlement_receipt_sha256 IS DISTINCT FROM s.settlement_sha256 OR g.valuation_receipt_sha256 IS NOT NULL
          OR NOT ((s.state='SETTLED_TRADE' AND t.status='EXECUTED') OR (s.state='SETTLED_NO_TRADE' AND t.status='SKIPPED'))))
     OR (p_action='VALUE_READBACK' AND (s.state<>'VALUED' OR g.valuation_receipt_sha256 IS NULL
          OR g.valuation_receipt_sha256 IS DISTINCT FROM s.valuation_sha256 OR s.settlement_sha256 IS NULL
          OR (g.step='SETTLE_THEN_VALUE' AND g.settlement_receipt_sha256 IS DISTINCT FROM s.settlement_sha256)
          OR (g.step='VALUE_ONLY' AND g.original_receipt_sha256 IS DISTINCT FROM s.settlement_sha256)))
     OR p_action NOT IN ('SETTLE','VALUE','SETTLE_READBACK','VALUE_READBACK')
     OR EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions earlier WHERE earlier.account_id=c.account_id AND earlier.effective_session<s.effective_session AND (earlier.state<>'VALUED' OR earlier.valuation_sha256 IS NULL))
  THEN
    UPDATE public.owner_beta_paper_recovery_grants SET revoked_at=clock_timestamp()
      WHERE grant_id=g.grant_id AND revoked_at IS NULL;
    PERFORM public.paper_0068_restore_actor(old_actor);
    RETURN NULL;
  END IF;
  PERFORM public.paper_0068_restore_actor(old_actor);
  RETURN jsonb_build_object('grant_id',g.grant_id,'active_epoch',g.active_epoch,'active_process_sha256',g.process_config_sha256,'active_consent_id',g.consent_event_id,'step',g.step,'expires_at',to_char(g.expires_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS"Z"'));
EXCEPTION WHEN query_canceled OR assert_failure THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
WHEN OTHERS THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
END $$;

CREATE FUNCTION public.transition_owner_beta_paper_v1(expected_epoch bigint,transition text,evidence jsonb,request_key text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE old_actor text:=current_setting('app.actor_user_id',true); c public.owner_beta_paper_control; saved public.owner_beta_paper_events; o public.owner_beta_paper_operations; source_o public.owner_beta_paper_operations; ob public.owner_beta_paper_operation_observations; s public.owner_beta_paper_sessions; l public.owner_beta_paper_lineage; d public.owner_beta_daily_control; t public.pending_targets; ev uuid; next_epoch bigint; gen uuid; pid uuid; step_name text; decision_hash text; original_hash text; expected_expiry timestamptz; execution_admission text; result_state text; replay jsonb; req text; receipt_result text; ok jsonb; reset jsonb; reset_at timestamptz; checked_at timestamptz;
BEGIN
  PERFORM public.paper_0068_principal();
  IF request_key IS NULL OR request_key !~ '^[A-Za-z0-9._:-]{1,128}$' OR evidence IS NULL OR jsonb_typeof(evidence)<>'object' OR octet_length(evidence::text)>262144 OR transition IS NULL OR transition NOT IN ('ENABLE','DRAIN','OFF','RESTORED_OFF','RECOVERY') THEN RAISE EXCEPTION 'lifecycle receipt invalid' USING ERRCODE='22023'; END IF;
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11';
  IF public.paper_0068_json_depth(evidence)>16 THEN RAISE EXCEPTION 'evidence depth invalid' USING ERRCODE='22023'; END IF;
  req:=public.paper_0068_request('TRANSITION',c.account_id,jsonb_build_object('expected_epoch',expected_epoch,'transition',transition,'evidence_canonical',public.paper_0068_canonical_json(evidence),'request_key',request_key));
  IF c.account_id IS NOT NULL THEN
    PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
    -- Distinct aliases avoid PL/pgSQL record-variable collisions (t and d).
    SELECT * INTO saved FROM public.owner_beta_paper_events event_row WHERE event_row.account_id=c.account_id AND event_row.request_key=transition_owner_beta_paper_v1.request_key;
    IF FOUND AND saved.request_canonical IS NOT NULL THEN
      PERFORM public.paper_0068_validate_receipt(to_jsonb(saved),'owner_beta_paper_events');
      IF saved.request_canonical IS DISTINCT FROM req OR saved.request_sha256 IS DISTINCT FROM public.daily_0060_digest(req) THEN RAISE EXCEPTION 'transition exact request conflicts' USING ERRCODE='23505'; END IF;
    replay:=jsonb_build_object('event_id',saved.id,'state',c.state,'pending_target_id',saved.pending_target_id);
    IF transition='RECOVERY' AND saved.payload->>'schema'='owner-beta-paper-recovery-v2' THEN
      replay:=replay||jsonb_build_object('grant_id',(SELECT g.grant_id FROM public.owner_beta_paper_recovery_grants g WHERE g.account_id=c.account_id AND g.request_key=transition_owner_beta_paper_v1.request_key));
    END IF;
    PERFORM public.paper_0068_restore_actor(old_actor);
    RETURN public.paper_0067_action('TRANSITION','REPLAY',c.control_epoch,c.account_id,replay);
    END IF;
  END IF;
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN RETURN public.paper_0067_action('TRANSITION','NOT_FOUND',NULL,NULL); END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  PERFORM pg_advisory_xact_lock(hashtextextended(c.account_id::text,381901));
  SELECT * INTO saved FROM public.owner_beta_paper_events WHERE account_id=c.account_id AND owner_beta_paper_events.request_key=transition_owner_beta_paper_v1.request_key FOR SHARE;
  IF FOUND THEN
    PERFORM public.paper_0068_validate_receipt(to_jsonb(saved),'owner_beta_paper_events');
    IF (saved.request_canonical IS NOT NULL AND (saved.request_canonical IS DISTINCT FROM req OR saved.request_sha256 IS DISTINCT FROM public.daily_0060_digest(req)))
       OR saved.kind<>(CASE transition WHEN 'ENABLE' THEN 'ENABLED' WHEN 'DRAIN' THEN 'DRAIN_STARTED' WHEN 'OFF' THEN 'OFF' WHEN 'RESTORED_OFF' THEN 'RESTORED_OFF' ELSE 'RECOVERY_AUTHORIZED' END) OR saved.payload IS DISTINCT FROM evidence THEN RAISE EXCEPTION 'transition request key conflicts with prior event' USING ERRCODE='23505'; END IF;
    replay:=jsonb_build_object('event_id',saved.id,'state',c.state,'pending_target_id',saved.pending_target_id);
    IF transition='RECOVERY' AND saved.payload->>'schema'='owner-beta-paper-recovery-v2' THEN
      replay:=replay||jsonb_build_object('grant_id',(SELECT g.grant_id FROM public.owner_beta_paper_recovery_grants g WHERE g.account_id=c.account_id AND g.request_key=transition_owner_beta_paper_v1.request_key));
    END IF;
    PERFORM public.paper_0068_restore_actor(old_actor);
    RETURN public.paper_0067_action('TRANSITION','REPLAY',c.control_epoch,c.account_id,replay);
  END IF;
  IF expected_epoch IS DISTINCT FROM c.control_epoch THEN RAISE EXCEPTION 'transition control epoch is stale' USING ERRCODE='40001'; END IF;

  IF transition NOT IN ('OFF','RESTORED_OFF') AND NOT (transition='RECOVERY' AND evidence->>'schema'='owner-beta-paper-recovery-v2') THEN
    DECLARE b jsonb; v_cohort jsonb; now_kst date:=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date; target public.pending_targets; session_row public.owner_beta_paper_sessions; recovery_target uuid; seq bigint; daily public.owner_beta_daily_control; expected_session date; session_dates date[]; verified_session date; i integer;
    BEGIN
  IF transition='ENABLE' THEN
    -- DB creation times do not authenticate ARM origin. Native must still
    -- prove the protected reset selection and each session's kernel anchor.
    reset:=public.paper_0068_reset(c);
    SELECT occurred_at INTO STRICT reset_at FROM public.owner_beta_paper_events WHERE id=(reset->>'event_id')::uuid;
    checked_at:=clock_timestamp();
    IF NOT public.daily_0060_keys(evidence,ARRAY['current_cohort','process_binding','process_binding_sha256','evaluated_at','expires_at','session_evidence','expected_epoch','new_epoch']) THEN RAISE EXCEPTION 'enable evidence shape' USING ERRCODE='23514'; END IF;
    v_cohort:=evidence->'current_cohort'; b:=evidence->'process_binding';
    IF NOT public.daily_0060_keys(b,ARRAY['schema','owner_user_id','account_id','strategy_config_id','consent_event_id','consent_version','control_epoch','daily_cohort_sha256','financial_contract_sha256','code_commit','release_manifest_sha256','api_binary_sha256','worker_binary_sha256'])
       OR b->>'schema'<>'owner-beta-paper-process-v1' OR b->>'owner_user_id'<>c.owner_user_id::text OR b->>'account_id'<>c.account_id::text OR b->>'strategy_config_id'<>c.strategy_config_id::text OR b->>'consent_event_id'<>c.consent_event_id::text OR b->>'consent_version'<>c.contract_id OR b->>'financial_contract_sha256'<>c.contract_sha256 OR b->>'daily_cohort_sha256'!~'^sha256:[0-9a-f]{64}$' OR b->>'code_commit'!~'^[0-9a-f]{40}$' OR evidence->>'process_binding_sha256'!~'^sha256:[0-9a-f]{64}$' THEN RAISE EXCEPTION 'process binding mismatch' USING ERRCODE='23514'; END IF;
    IF c.state NOT IN ('OFF','RESTORED_OFF') OR expected_epoch=9223372036854775807 OR evidence->>'expected_epoch' !~ '^(0|[1-9][0-9]{0,18})$' OR evidence->>'new_epoch' !~ '^[1-9][0-9]{0,18}$' OR b->>'control_epoch' !~ '^[1-9][0-9]{0,18}$' OR (evidence->>'expected_epoch')::bigint<>expected_epoch OR (evidence->>'new_epoch')::bigint<>expected_epoch+1 OR (b->>'control_epoch')::bigint<>expected_epoch+1 OR jsonb_typeof(evidence->'evaluated_at')<>'string' OR jsonb_typeof(evidence->'expires_at')<>'string' OR (evidence->>'expires_at')::timestamptz<=clock_timestamp() OR (evidence->>'evaluated_at')::timestamptz>clock_timestamp() OR clock_timestamp()-(evidence->>'evaluated_at')::timestamptz>interval '180 seconds' THEN RAISE EXCEPTION 'enable evidence expired or transition state invalid' USING ERRCODE='23514'; END IF;
    IF b->>'release_manifest_sha256' !~ '^sha256:[0-9a-f]{64}$' OR b->>'api_binary_sha256' !~ '^sha256:[0-9a-f]{64}$' OR b->>'worker_binary_sha256' !~ '^sha256:[0-9a-f]{64}$' OR b->>'financial_contract_sha256' IS DISTINCT FROM c.contract_sha256
      OR b->>'owner_user_id' IS DISTINCT FROM c.owner_user_id::text OR b->>'account_id' IS DISTINCT FROM c.account_id::text OR b->>'strategy_config_id' IS DISTINCT FROM c.strategy_config_id::text
      OR b->>'consent_event_id' IS DISTINCT FROM c.consent_event_id::text THEN RAISE EXCEPTION 'process binding identity hashes invalid' USING ERRCODE='23514'; END IF;
    IF jsonb_typeof(evidence->'session_evidence')<>'array' OR jsonb_array_length(evidence->'session_evidence')<>3 OR c.consent_event_id IS NULL OR NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_events WHERE id=c.consent_event_id AND kind='CONSENT_GRANTED' AND owner_user_id=c.owner_user_id AND account_id=c.account_id) THEN RAISE EXCEPTION 'enable needs consent and three sessions' USING ERRCODE='23514'; END IF;
    daily:=public.paper_0066_authority(v_cohort,(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date);
    expected_session:=public.daily_0060_expected_at(daily,clock_timestamp() AT TIME ZONE 'Asia/Seoul');
    IF expected_session IS NULL OR (SELECT count(DISTINCT x->>'session_date') FROM jsonb_array_elements(evidence->'session_evidence') x WHERE x->>'session_date' ~ '^20[0-9]{2}-[0-9]{2}-[0-9]{2}$' AND (x->>'session_date')::date<=expected_session)<>3
       OR EXISTS(SELECT 1 FROM jsonb_array_elements(evidence->'session_evidence') x WHERE NOT public.daily_0060_keys(x,ARRAY['session_date','run_id','admission_sha256','source_event_sha256','root_event_sha256']) OR x->>'admission_sha256' !~ '^sha256:[0-9a-f]{64}$' OR x->>'source_event_sha256' !~ '^sha256:[0-9a-f]{64}$' OR x->>'root_event_sha256' !~ '^sha256:[0-9a-f]{64}$'
         OR NOT EXISTS(SELECT 1 FROM public.owner_beta_recommendation_runs r JOIN public.owner_beta_daily_reservations dr ON dr.run_id=r.id JOIN public.owner_beta_paper_source_runs sr ON sr.run_id=r.id JOIN public.owner_beta_paper_source_slots ss ON ss.slot_id=sr.slot_id JOIN public.owner_beta_paper_source_admissions sa ON sa.slot_id=ss.slot_id JOIN public.owner_beta_daily_admissions a ON a.admission_sha256=sa.admission_sha256 JOIN public.jobs j ON j.id=r.job_id JOIN public.job_attempts ja ON ja.id=sr.attempt_id AND ja.job_id=j.id AND ja.attempt_no=j.attempt_count JOIN public.owner_beta_daily_publication_claims pc ON pc.run_id=r.id AND pc.job_id=j.id AND pc.owner_user_id=r.owner_user_id AND pc.attempt_id=ja.id AND pc.attempt_no=ja.attempt_no AND pc.claimed_by=ja.claimed_by WHERE r.id=(x->>'run_id')::uuid AND r.owner_user_id=c.owner_user_id AND r.status='SUCCEEDED' AND r.trigger_kind='DAILY' AND r.strategy_config_id=c.strategy_config_id
         AND r.strategy_id='trend_following' AND r.strategy_version='1.0.0'
         AND r.strategy_config_sha256=v_cohort->>'strategy_config_sha256'
         AND r.daily_config_epoch=(v_cohort->>'config_epoch')::bigint AND r.daily_activation_epoch=(v_cohort->>'activation_epoch')::bigint
         AND r.input_kind='owner_beta_daily_etf11_price_only_v1' AND r.daily_trigger_version=dr.trigger_version
         AND j.owner_user_id=c.owner_user_id AND j.job_type='owner_beta_price_recommendation' AND j.status='SUCCEEDED'
         AND j.attempt_count BETWEEN 1 AND 3 AND ja.outcome='SUCCEEDED' AND sr.job_id=r.job_id
         AND r.daily_pins IS NOT DISTINCT FROM jsonb_build_object('admission_sha256',a.admission_sha256,'content_sha256',a.content_sha256,'manifest_sha256',a.manifest_sha256,'policy_sha256',a.policy_sha256)
         AND j.payload_json IS NOT DISTINCT FROM public.daily_0060_payload(r)
         AND r.finished_at=pc.authorized_at AND j.finished_at>=pc.authorized_at AND ja.finished_at>=j.finished_at
         AND (SELECT count(*) FROM public.job_attempts attempt WHERE attempt.job_id=j.id)=j.attempt_count
         AND NOT EXISTS(SELECT 1 FROM public.job_attempts attempt WHERE attempt.job_id=j.id AND attempt.attempt_no<j.attempt_count AND attempt.outcome NOT IN ('FAILED','ORPHANED'))
         AND ss.reserved_at>reset_at AND ss.reserved_at<=checked_at
         AND dr.created_at>reset_at AND dr.created_at<=checked_at
         AND r.created_at>reset_at AND r.created_at<=checked_at
         AND sa.linked_at>reset_at AND sa.linked_at<=checked_at
         AND sr.linked_at>reset_at AND sr.linked_at<=checked_at
         AND r.as_of=(x->>'session_date')::date AND dr.owner_user_id=c.owner_user_id AND dr.strategy_config_id=c.strategy_config_id AND dr.session_date=r.as_of AND dr.run_id=r.id AND dr.job_id=r.job_id AND dr.admission_sha256=sa.admission_sha256 AND dr.config_epoch=(v_cohort->>'config_epoch')::bigint AND dr.activation_epoch=(v_cohort->>'activation_epoch')::bigint AND dr.strategy_config_sha256=v_cohort->>'strategy_config_sha256' AND r.daily_pins->>'admission_sha256'=a.admission_sha256 AND a.owner_user_id=c.owner_user_id AND a.range_end=r.as_of AND ss.owner_user_id=c.owner_user_id AND ss.policy_sha256=v_cohort->>'policy_sha256' AND ss.activation_epoch=(v_cohort->>'activation_epoch')::bigint AND ss.session_date=r.as_of AND ss.cohort_sha256=v_cohort->>'cohort_sha256' AND ss.cohort IS NOT DISTINCT FROM v_cohort AND sa.admission_sha256=x->>'admission_sha256' AND sa.source_event_sha256=x->>'source_event_sha256' AND ss.root_event_sha256=x->>'root_event_sha256')) THEN RAISE EXCEPTION 'pre-session evidence list is invalid' USING ERRCODE='23514'; END IF;
    SELECT array_agg((x->>'session_date')::date ORDER BY (x->>'session_date')::date) INTO session_dates FROM jsonb_array_elements(evidence->'session_evidence') x;
    IF session_dates[3] IS DISTINCT FROM expected_session THEN RAISE EXCEPTION 'pre-session evidence does not end at current expected session' USING ERRCODE='23514'; END IF;
    FOR i IN 1..3 LOOP
      SELECT e.effective_session INTO verified_session FROM public.paper_0067_next_session(c.owner_user_id,daily.policy_sha256,CASE WHEN i=1 THEN session_dates[1]-1 ELSE session_dates[i-1] END) e;
      IF verified_session IS DISTINCT FROM session_dates[i] THEN RAISE EXCEPTION 'pre-session evidence is not three consecutive verified sessions' USING ERRCODE='23514'; END IF;
    END LOOP;
    IF NOT EXISTS(SELECT 1 FROM public.owner_beta_daily_control daily_row WHERE daily_row.owner_user_id=c.owner_user_id AND daily_row.enabled AND v_cohort->>'cohort_sha256'=b->>'daily_cohort_sha256') THEN RAISE EXCEPTION 'Daily witness cohort not current' USING ERRCODE='23514'; END IF;
    next_epoch:=expected_epoch+1;
  ELSE
    IF transition='DRAIN' THEN
      IF NOT public.daily_0060_keys(evidence,ARRAY['schema','expected_epoch','reason_code']) OR evidence->>'schema'<>'owner-beta-paper-drain-v1' OR evidence->>'expected_epoch' !~ '^(0|[1-9][0-9]{0,18})$' OR (evidence->>'expected_epoch')::bigint<>expected_epoch OR evidence->>'reason_code' !~ '^[A-Z][A-Z0-9_]{0,63}$' OR c.state NOT IN ('ENABLED','DRAINING') OR expected_epoch=9223372036854775807 THEN RAISE EXCEPTION 'invalid drain transition' USING ERRCODE='23514'; END IF;
    END IF;
    IF transition IN ('OFF','RESTORED_OFF') THEN RAISE EXCEPTION 'WP5 protected drain/restore receipt contract is not part of the SQL ABI yet' USING ERRCODE='55000'; END IF;
    IF transition='RECOVERY' THEN
      IF NOT public.daily_0060_keys(evidence,ARRAY['schema','pending_target_id','effective_session','reason_code','decision_sha256']) OR evidence->>'schema'<>'owner-beta-paper-recovery-v1' OR evidence->>'pending_target_id' !~ '^[0-9a-f-]{36}$' OR evidence->>'effective_session' !~ '^20[0-9]{2}-[0-9]{2}-[0-9]{2}$' OR evidence->>'reason_code' !~ '^[A-Z][A-Z0-9_]{0,63}$' OR evidence->>'decision_sha256' !~ '^sha256:[0-9a-f]{64}$' THEN RAISE EXCEPTION 'recovery authorization shape invalid' USING ERRCODE='23514'; END IF;
      recovery_target:=(evidence->>'pending_target_id')::uuid;
      SELECT * INTO target FROM public.pending_targets WHERE id=recovery_target AND account_id=c.account_id AND owner_user_id=c.owner_user_id AND beta_lineage_id IS NOT NULL FOR UPDATE;
      SELECT * INTO session_row FROM public.owner_beta_paper_sessions WHERE pending_target_id=recovery_target AND account_id=c.account_id AND owner_user_id=c.owner_user_id FOR UPDATE;
      IF NOT FOUND OR target.id IS NULL OR session_row.valuation_sha256 IS NOT NULL
         OR NOT ((target.status='PENDING' AND session_row.settlement_sha256 IS NULL AND session_row.state IN ('PENDING','WAITING_PRICE','WAITING_ADMISSION','HELD','RECOVERY_REQUIRED'))
           OR (session_row.settlement_sha256 IS NOT NULL AND ((target.status='EXECUTED' AND session_row.state='SETTLED_TRADE') OR (target.status='SKIPPED' AND session_row.state='SETTLED_NO_TRADE'))))
         OR session_row.effective_session<>(evidence->>'effective_session')::date OR session_row.effective_session>=now_kst OR EXISTS(SELECT 1 FROM public.owner_beta_paper_events e WHERE e.account_id=c.account_id AND e.kind='RECOVERY_AUTHORIZED' AND e.pending_target_id=recovery_target) OR EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions earlier WHERE earlier.account_id=c.account_id AND earlier.effective_session<session_row.effective_session AND earlier.state<>'VALUED') THEN RAISE EXCEPTION 'recovery target is not the earliest unresolved expired session' USING ERRCODE='23514'; END IF;
      c:=public.paper_0067_authority(CASE WHEN session_row.settlement_sha256 IS NULL THEN 'SETTLE' ELSE 'VALUE' END,session_row.effective_session,expected_epoch,c.process_binding_sha256,true);
    END IF;
    next_epoch:=expected_epoch+CASE WHEN transition='DRAIN' THEN 1 ELSE 0 END;
  END IF;
  ev:=gen_random_uuid();
  ok:=public.paper_0067_action('TRANSITION','OK',next_epoch,c.account_id,jsonb_build_object('event_id',ev,'state',CASE transition WHEN 'ENABLE' THEN 'ENABLED' WHEN 'DRAIN' THEN 'DRAINING' WHEN 'OFF' THEN 'OFF' WHEN 'RESTORED_OFF' THEN 'RESTORED_OFF' ELSE c.state END));
  receipt_result:=public.paper_0068_canonical_json(ok);
  INSERT INTO public.owner_beta_paper_events(id,owner_user_id,account_id,control_epoch,kind,payload,payload_sha256,request_key,pending_target_id,request_canonical,request_sha256,result_canonical,result_sha256)
   VALUES(ev,c.owner_user_id,c.account_id,next_epoch,CASE transition WHEN 'ENABLE' THEN 'ENABLED' WHEN 'DRAIN' THEN 'DRAIN_STARTED' WHEN 'OFF' THEN 'OFF' WHEN 'RESTORED_OFF' THEN 'RESTORED_OFF' ELSE 'RECOVERY_AUTHORIZED' END,evidence,public.daily_0060_digest(evidence::text),request_key,recovery_target,req,public.daily_0060_digest(req),receipt_result,public.daily_0060_digest(receipt_result));
  UPDATE public.owner_beta_paper_control SET state=CASE transition WHEN 'ENABLE' THEN 'ENABLED' WHEN 'DRAIN' THEN 'DRAINING' WHEN 'OFF' THEN 'OFF' WHEN 'RESTORED_OFF' THEN 'RESTORED_OFF' ELSE state END,
    control_epoch=next_epoch,process_binding=CASE WHEN transition='ENABLE' THEN b ELSE process_binding END,process_binding_sha256=CASE WHEN transition='ENABLE' THEN evidence->>'process_binding_sha256' ELSE process_binding_sha256 END,
    cohort_sha256=CASE WHEN transition='ENABLE' THEN b->>'daily_cohort_sha256' ELSE cohort_sha256 END,daily_cohort=CASE WHEN transition='ENABLE' THEN v_cohort ELSE daily_cohort END,
    activation_event_id=CASE WHEN transition='ENABLE' THEN ev ELSE activation_event_id END,updated_at=clock_timestamp() WHERE id='etf11';
  IF transition='RECOVERY' THEN
    -- A committed settlement remains immutable while only its valuation resumes.
    -- Pre-settlement recovery keeps the existing recovery execution path.
    IF session_row.settlement_sha256 IS NULL THEN
      UPDATE public.owner_beta_paper_sessions SET processing_origin='RECOVERY',state='RECOVERY_REQUIRED',reason_code='RECOVERY_REQUIRED',updated_at=clock_timestamp() WHERE pending_target_id=recovery_target;
    END IF;
    SELECT COALESCE(max(event_seq),0)+1 INTO seq FROM public.owner_beta_paper_session_events WHERE pending_target_id=recovery_target;
    INSERT INTO public.owner_beta_paper_session_events(pending_target_id,event_seq,kind,reason_code,payload) VALUES(recovery_target,seq,'RECOVERY_AUTHORIZED','RECOVERY_REQUIRED',evidence);
  END IF;
  PERFORM public.paper_0068_restore_actor(old_actor); RETURN ok;
    END;
  END IF;
  IF octet_length(evidence::text)>65536 THEN RAISE EXCEPTION 'lifecycle receipt invalid' USING ERRCODE='22023'; END IF;

  IF transition IN ('OFF','RESTORED_OFF') THEN
    IF transition='OFF' AND NOT public.daily_0060_keys(evidence,ARRAY['schema','installation_id','generation','account_id','request_key','scope','expected_epoch','resulting_epoch','intent_sha256','stage_sha256','inventory_sha256','reconciliation_sha256','observation_id','release_manifest_sha256','boot_id','observed'])
       OR transition='RESTORED_OFF' AND NOT public.daily_0060_keys(evidence,ARRAY['schema','installation_id','generation','account_id','request_key','scope','expected_epoch','resulting_epoch','intent_sha256','stage_sha256','inventory_sha256','reconciliation_sha256','observation_id','release_manifest_sha256','boot_id','observed','source_installation_id','source_generation','source_epoch','archive_sha256','source_audit_sha256','restored_audit_sha256','destination_epoch_floor'])
       OR EXISTS(SELECT 1 FROM jsonb_each(evidence) e WHERE e.value='null'::jsonb)
       OR jsonb_typeof(evidence->'installation_id') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'generation') IS DISTINCT FROM 'string'
       OR jsonb_typeof(evidence->'account_id') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'request_key') IS DISTINCT FROM 'string'
       OR jsonb_typeof(evidence->'scope') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'expected_epoch') IS DISTINCT FROM 'number'
       OR jsonb_typeof(evidence->'resulting_epoch') IS DISTINCT FROM 'number' OR jsonb_typeof(evidence->'intent_sha256') IS DISTINCT FROM 'string'
       OR jsonb_typeof(evidence->'stage_sha256') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'inventory_sha256') IS DISTINCT FROM 'string'
       OR jsonb_typeof(evidence->'reconciliation_sha256') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'observation_id') IS DISTINCT FROM 'string'
       OR jsonb_typeof(evidence->'release_manifest_sha256') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'boot_id') IS DISTINCT FROM 'string'
       OR jsonb_typeof(evidence->'observed') IS DISTINCT FROM 'object'
       OR transition='RESTORED_OFF' AND (jsonb_typeof(evidence->'source_installation_id') IS DISTINCT FROM 'string'
          OR jsonb_typeof(evidence->'source_generation') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'source_epoch') IS DISTINCT FROM 'number'
          OR jsonb_typeof(evidence->'archive_sha256') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'source_audit_sha256') IS DISTINCT FROM 'string'
          OR jsonb_typeof(evidence->'restored_audit_sha256') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'destination_epoch_floor') IS DISTINCT FROM 'number')
    THEN RAISE EXCEPTION 'OFF/restore receipt schema invalid' USING ERRCODE='22023'; END IF;
    IF evidence->>'schema' IS DISTINCT FROM (CASE transition WHEN 'OFF' THEN 'owner-beta-paper-drain-receipt-v1' ELSE 'owner-beta-paper-restore-receipt-v1' END)
       OR evidence->>'account_id' IS DISTINCT FROM c.account_id::text OR evidence->>'request_key' IS DISTINCT FROM request_key
       OR evidence->>'scope' NOT IN ('PAPER_STOP','COMBINED_BACKUP') OR evidence->>'generation' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
       OR evidence->>'installation_id' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
       OR NOT public.paper_0068_nonnegative_i64(evidence->>'expected_epoch') OR evidence->>'expected_epoch'='9223372036854775807'
       OR NOT public.paper_0068_nonnegative_i64(evidence->>'resulting_epoch') OR evidence->>'resulting_epoch'='0'
       OR evidence->>'intent_sha256' !~ '^sha256:[0-9a-f]{64}$' OR evidence->>'stage_sha256' !~ '^sha256:[0-9a-f]{64}$'
       OR evidence->>'inventory_sha256' !~ '^sha256:[0-9a-f]{64}$' OR evidence->>'reconciliation_sha256' !~ '^sha256:[0-9a-f]{64}$'
       OR evidence->>'release_manifest_sha256' !~ '^sha256:[0-9a-f]{64}$'
       OR evidence->>'boot_id' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
       OR NOT public.paper_0068_clock_fresh(evidence->'observed') THEN RAISE EXCEPTION 'OFF/restore receipt fields or freshness invalid' USING ERRCODE='23514'; END IF;
    gen:=(evidence->>'generation')::uuid;
    IF request_key<>gen::text||(CASE transition WHEN 'OFF' THEN ':off' ELSE ':restore-off' END) THEN RAISE EXCEPTION 'receipt request key does not bind generation' USING ERRCODE='23514'; END IF;
    SELECT * INTO o FROM public.owner_beta_paper_operations WHERE generation=gen AND account_id=c.account_id FOR SHARE;
    IF NOT FOUND OR o.installation_id<>(evidence->>'installation_id')::uuid OR o.scope<>evidence->>'scope'
       OR o.fence_epoch<>(evidence->>'resulting_epoch')::bigint OR o.intent_sha256<>evidence->>'intent_sha256'
       OR o.release_manifest_sha256<>evidence->>'release_manifest_sha256' OR c.state<>'DRAINING' OR o.fence_epoch<>c.control_epoch
       OR (evidence->>'expected_epoch')::bigint<>o.expected_epoch
       OR transition='OFF' AND (o.purpose NOT IN ('STOP','BACKUP') OR o.scope='PAPER_STOP' AND o.purpose<>'STOP')
       OR transition='RESTORED_OFF' AND (o.purpose<>'RESTORE' OR o.scope<>'COMBINED_BACKUP')
    THEN RAISE EXCEPTION 'OFF/restore receipt does not match durable operation fence' USING ERRCODE='23514'; END IF;
    SELECT * INTO ob FROM public.owner_beta_paper_operation_observations WHERE generation=gen AND observation_id=(evidence->>'observation_id')::uuid AND summary->>'schema'='owner-beta-paper-reconciliation-v1' FOR SHARE;
    IF FOUND AND EXISTS(SELECT 1 FROM public.owner_beta_paper_operation_observations newer WHERE newer.generation=gen AND (newer.observed_at>ob.observed_at OR newer.observed_at=ob.observed_at AND newer.observation_id>ob.observation_id)) THEN RAISE EXCEPTION 'receipt does not name latest reconciliation observation' USING ERRCODE='40001'; END IF;
    IF NOT FOUND OR ob.summary_sha256<>evidence->>'reconciliation_sha256' OR ob.summary->>'inventory_sha256'<>evidence->>'inventory_sha256'
       OR (ob.summary->>'control_epoch')::bigint<>c.control_epoch OR (ob.summary->>'unresolved_commits')::bigint<>0 OR (ob.summary->>'live_claims')::bigint<>0
    THEN RAISE EXCEPTION 'OFF/restore reconciliation proof is missing or unresolved' USING ERRCODE='55000'; END IF;
    IF transition='RESTORED_OFF' THEN
      IF NOT public.paper_0068_nonnegative_i64(evidence->>'source_epoch')
         OR NOT public.paper_0068_nonnegative_i64(evidence->>'destination_epoch_floor') THEN
        RAISE EXCEPTION 'restore source/destination epoch is invalid' USING ERRCODE='23514';
      END IF;
      SELECT * INTO source_o FROM public.owner_beta_paper_operations WHERE generation=(evidence->>'source_generation')::uuid AND account_id=c.account_id FOR SHARE;
      IF NOT FOUND OR source_o.installation_id IS DISTINCT FROM (evidence->>'source_installation_id')::uuid
         OR source_o.installation_id=o.installation_id OR source_o.generation IS DISTINCT FROM NULLIF(o.intent->>'predecessor_generation','null')::uuid
         OR o.intent->>'artifact_sha256' IS DISTINCT FROM evidence->>'archive_sha256'
         OR (evidence->>'destination_epoch_floor')::bigint<>o.local_epoch_floor
         OR (evidence->>'source_epoch')::bigint>(evidence->>'destination_epoch_floor')::bigint
         OR NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_events se WHERE se.account_id=c.account_id AND se.request_key=source_o.generation::text||':off' AND se.kind='OFF' AND (se.payload->>'resulting_epoch')::bigint=(evidence->>'source_epoch')::bigint)
         OR evidence->>'scope'<>'COMBINED_BACKUP' OR evidence->>'archive_sha256' !~ '^sha256:[0-9a-f]{64}$'
       OR evidence->>'source_audit_sha256' !~ '^sha256:[0-9a-f]{64}$' OR evidence->>'restored_audit_sha256' IS DISTINCT FROM evidence->>'source_audit_sha256'
       OR evidence->>'source_installation_id' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
       OR evidence->>'source_generation' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
         OR (evidence->>'destination_epoch_floor')::bigint>=c.control_epoch
      THEN RAISE EXCEPTION 'restore source/destination audit binding invalid' USING ERRCODE='23514'; END IF;
    END IF;
    IF c.control_epoch=9223372036854775807 THEN RAISE EXCEPTION 'control epoch overflow' USING ERRCODE='22023'; END IF;
    next_epoch:=c.control_epoch+1; ev:=gen_random_uuid();
    ok:=public.paper_0067_action('TRANSITION','OK',next_epoch,c.account_id,jsonb_build_object('event_id',ev,'state',transition));
    receipt_result:=public.paper_0068_canonical_json(ok);
    INSERT INTO public.owner_beta_paper_events(id,owner_user_id,account_id,control_epoch,kind,payload,payload_sha256,request_key,request_canonical,request_sha256,result_canonical,result_sha256)
      VALUES(ev,c.owner_user_id,c.account_id,next_epoch,transition,evidence,public.daily_0060_digest(public.paper_0068_canonical_json(evidence)),request_key,req,public.daily_0060_digest(req),receipt_result,public.daily_0060_digest(receipt_result));
    UPDATE public.owner_beta_paper_control SET state=transition,control_epoch=next_epoch,process_binding=NULL,process_binding_sha256=NULL,cohort_sha256=NULL,daily_cohort=NULL,activation_event_id=NULL,updated_at=clock_timestamp() WHERE id='etf11';
    UPDATE public.owner_beta_paper_recovery_grants SET revoked_at=clock_timestamp() WHERE account_id=c.account_id AND revoked_at IS NULL AND valuation_receipt_sha256 IS NULL;
    PERFORM public.paper_0068_restore_actor(old_actor);
    RETURN ok;
  END IF;

  IF NOT public.daily_0060_keys(evidence,ARRAY['schema','installation_id','generation','account_id','request_key','pending_target_id','lineage_id','effective_session','lineage_epoch','active_epoch','original_receipt_sha256','consent_event_id','process_config_sha256','step','signal_admission_sha256','execution_admission_sha256','decision_sha256','reason_code','expires_at'])
     OR EXISTS(SELECT 1 FROM jsonb_each(evidence) e WHERE e.value='null'::jsonb)
     OR jsonb_typeof(evidence->'installation_id') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'generation') IS DISTINCT FROM 'string'
     OR jsonb_typeof(evidence->'account_id') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'request_key') IS DISTINCT FROM 'string'
     OR jsonb_typeof(evidence->'pending_target_id') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'lineage_id') IS DISTINCT FROM 'string'
     OR jsonb_typeof(evidence->'effective_session') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'lineage_epoch') IS DISTINCT FROM 'number'
     OR jsonb_typeof(evidence->'active_epoch') IS DISTINCT FROM 'number' OR jsonb_typeof(evidence->'original_receipt_sha256') IS DISTINCT FROM 'string'
     OR jsonb_typeof(evidence->'consent_event_id') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'process_config_sha256') IS DISTINCT FROM 'string'
     OR jsonb_typeof(evidence->'step') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'signal_admission_sha256') IS DISTINCT FROM 'string'
     OR jsonb_typeof(evidence->'execution_admission_sha256') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'decision_sha256') IS DISTINCT FROM 'string'
     OR jsonb_typeof(evidence->'reason_code') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'expires_at') IS DISTINCT FROM 'string'
     OR request_key<>evidence->>'generation'||':recover' OR evidence->>'reason_code'<>'EXPLICIT_RETAINED_RECOVERY'
     OR evidence->>'account_id'<>c.account_id::text OR evidence->>'installation_id' !~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR evidence->>'generation' !~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR evidence->>'pending_target_id' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR evidence->>'lineage_id' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR evidence->>'consent_event_id' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR evidence->>'effective_session' !~ '^20[0-9]{2}-[0-9]{2}-[0-9]{2}$'
     OR NOT public.paper_0068_nonnegative_i64(evidence->>'lineage_epoch')
     OR NOT public.paper_0068_nonnegative_i64(evidence->>'active_epoch')
     OR evidence->>'original_receipt_sha256' !~ '^sha256:[0-9a-f]{64}$' OR evidence->>'process_config_sha256' !~ '^sha256:[0-9a-f]{64}$'
     OR evidence->>'signal_admission_sha256' !~ '^sha256:[0-9a-f]{64}$' OR evidence->>'execution_admission_sha256' !~ '^sha256:[0-9a-f]{64}$'
     OR evidence->>'decision_sha256' !~ '^sha256:[0-9a-f]{64}$' OR evidence->>'step' NOT IN ('SETTLE_THEN_VALUE','VALUE_ONLY')
  THEN RAISE EXCEPTION 'recovery-v2 decision schema invalid' USING ERRCODE='23514'; END IF;
  IF (evidence->>'active_epoch')::bigint<>c.control_epoch OR (evidence->>'lineage_epoch')::bigint>=c.control_epoch
     OR evidence->>'process_config_sha256'<>c.process_binding_sha256 OR evidence->>'consent_event_id'<>c.consent_event_id::text
     OR c.state<>'ENABLED' OR evidence->>'expires_at' !~ '^20[0-9]{2}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$'
  THEN RAISE EXCEPTION 'recovery current control authority mismatch' USING ERRCODE='42501',CONSTRAINT='paper_recovery_v2_control'; END IF;
  expected_expiry:=((clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date+1)::timestamp AT TIME ZONE 'Asia/Seoul';
  IF (evidence->>'expires_at')::timestamptz<>expected_expiry THEN RAISE EXCEPTION 'recovery grant must expire at next KST midnight' USING ERRCODE='23514'; END IF;
  decision_hash:=public.daily_0060_digest(public.paper_0068_canonical_json(jsonb_build_object('effective_session',evidence->'effective_session','pending_target_id',evidence->'pending_target_id','step',evidence->'step')));
  IF decision_hash<>evidence->>'decision_sha256' THEN RAISE EXCEPTION 'recovery decision hash mismatch' USING ERRCODE='23514'; END IF;
  pid:=(evidence->>'pending_target_id')::uuid;
  SELECT * INTO t FROM public.pending_targets WHERE id=pid AND account_id=c.account_id AND owner_user_id=c.owner_user_id AND beta_lineage_id=(evidence->>'lineage_id')::uuid FOR UPDATE;
  SELECT * INTO s FROM public.owner_beta_paper_sessions WHERE pending_target_id=pid AND account_id=c.account_id AND owner_user_id=c.owner_user_id FOR UPDATE;
  SELECT * INTO l FROM public.owner_beta_paper_lineage WHERE id=(evidence->>'lineage_id')::uuid AND account_id=c.account_id AND owner_user_id=c.owner_user_id FOR SHARE;
  IF NOT FOUND OR s.pending_target_id IS NULL OR t.id IS NULL OR l.id IS NULL OR s.effective_session<>(evidence->>'effective_session')::date OR l.effective_session<>s.effective_session OR l.control_epoch<>(evidence->>'lineage_epoch')::bigint
     OR s.valuation_sha256 IS NOT NULL OR s.effective_session>=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date
     OR NOT ((s.settlement_sha256 IS NULL AND t.status='PENDING' AND s.state IN ('PENDING','WAITING_PRICE','WAITING_ADMISSION','HELD','BLOCKED','RECOVERY_REQUIRED') AND evidence->>'step'='SETTLE_THEN_VALUE') OR (s.settlement_sha256 IS NOT NULL AND ((s.state='SETTLED_TRADE' AND t.status='EXECUTED') OR (s.state='SETTLED_NO_TRADE' AND t.status='SKIPPED')) AND evidence->>'step'='VALUE_ONLY'))
     OR EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions earlier WHERE earlier.account_id=c.account_id AND earlier.effective_session<s.effective_session AND (earlier.state<>'VALUED' OR earlier.valuation_sha256 IS NULL))
  THEN RAISE EXCEPTION 'recovery target is not earliest retained unresolved lineage' USING ERRCODE='23514'; END IF;
  original_hash:=CASE WHEN s.settlement_sha256 IS NOT NULL THEN s.settlement_sha256 ELSE public.daily_0060_digest(public.paper_0068_canonical_json(COALESCE(s.receipt_json,'null'::jsonb))) END;
  IF original_hash<>evidence->>'original_receipt_sha256' OR l.signal_admission_sha256<>evidence->>'signal_admission_sha256' THEN RAISE EXCEPTION 'recovery original lineage receipt mismatch' USING ERRCODE='23514'; END IF;
  SELECT * INTO d FROM public.owner_beta_daily_control WHERE id='etf11' AND owner_user_id=c.owner_user_id FOR SHARE;
  execution_admission:=evidence->>'execution_admission_sha256';
  IF NOT d.enabled OR NOT public.daily_0060_admission_usable(d,l.signal_admission_sha256,false) OR NOT public.daily_0060_admission_usable(d,execution_admission,false)
     OR NOT EXISTS(SELECT 1 FROM public.owner_beta_daily_admissions a WHERE a.owner_user_id=c.owner_user_id AND a.admission_sha256=execution_admission AND a.range_end=s.effective_session AND a.policy_sha256=d.policy_sha256)
     OR NOT EXISTS(SELECT 1 FROM public.owner_beta_daily_admissions e JOIN public.owner_beta_daily_admissions t0 ON t0.admission_sha256=l.signal_admission_sha256 WHERE e.admission_sha256=execution_admission AND e.owner_user_id=c.owner_user_id AND e.range_end=s.effective_session AND e.policy_sha256=t0.policy_sha256 AND e.range_start=t0.range_start AND e.range_end>=t0.range_end AND (WITH RECURSIVE ancestry(admission_sha256,parent_admission_sha256) AS (SELECT admission_sha256,parent_admission_sha256 FROM public.owner_beta_daily_admissions WHERE admission_sha256=e.admission_sha256 UNION ALL SELECT p.admission_sha256,p.parent_admission_sha256 FROM public.owner_beta_daily_admissions p JOIN ancestry a ON p.admission_sha256=a.parent_admission_sha256) SELECT bool_or(admission_sha256=l.signal_admission_sha256) FROM ancestry))
     OR NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_source_slots sl JOIN public.owner_beta_paper_source_admissions sa USING(slot_id) WHERE sl.owner_user_id=c.owner_user_id AND sl.session_date=s.effective_session AND sa.admission_sha256=execution_admission)
     OR NOT EXISTS(SELECT 1 FROM public.data_entitlements de WHERE de.contract_reference=d.entitlement_reference AND de.contract_document_sha256=substring(d.entitlement_sha256,8) AND de.status='ACTIVE' AND de.effective_from<=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date AND de.effective_until>=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date AND de.covered_datasets @> '["krx_eod_bars"]'::jsonb AND de.covered_uses @> '["recommendation","paper_view"]'::jsonb)
     OR NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_events e WHERE e.id=c.consent_event_id AND e.kind='CONSENT_GRANTED' AND e.account_id=c.account_id)
  THEN RAISE EXCEPTION 'recovery current rights, consent or exact E admission unavailable' USING ERRCODE='42501',CONSTRAINT='paper_recovery_v2_rights'; END IF;
  UPDATE public.owner_beta_paper_recovery_grants SET revoked_at=clock_timestamp() WHERE pending_target_id=pid AND active_epoch=c.control_epoch AND revoked_at IS NULL AND valuation_receipt_sha256 IS NULL AND expires_at<=clock_timestamp();
  IF EXISTS(SELECT 1 FROM public.owner_beta_paper_recovery_grants g WHERE g.pending_target_id=pid AND g.active_epoch=c.control_epoch AND g.revoked_at IS NULL AND g.valuation_receipt_sha256 IS NULL) THEN RAISE EXCEPTION 'active recovery grant already exists for target and epoch' USING ERRCODE='23505'; END IF;
  INSERT INTO public.owner_beta_paper_recovery_grants(grant_id,account_id,pending_target_id,lineage_id,effective_session,original_receipt_sha256,lineage_epoch,active_epoch,consent_event_id,process_config_sha256,decision,decision_sha256,step,signal_admission_sha256,execution_admission_sha256,expires_at,request_key)
    VALUES(gen_random_uuid(),c.account_id,pid,l.id,s.effective_session,original_hash,l.control_epoch,c.control_epoch,c.consent_event_id,c.process_binding_sha256,evidence,decision_hash,evidence->>'step',l.signal_admission_sha256,execution_admission,(evidence->>'expires_at')::timestamptz,request_key);
  ev:=gen_random_uuid();
  ok:=public.paper_0067_action('TRANSITION','OK',c.control_epoch,c.account_id,jsonb_build_object('event_id',ev,'state',c.state,'pending_target_id',pid,'grant_id',(SELECT grant_row.grant_id FROM public.owner_beta_paper_recovery_grants grant_row WHERE grant_row.account_id=c.account_id AND grant_row.request_key=transition_owner_beta_paper_v1.request_key)));
  receipt_result:=public.paper_0068_canonical_json(ok);
  INSERT INTO public.owner_beta_paper_events(id,owner_user_id,account_id,control_epoch,kind,actor_user_id,payload,payload_sha256,request_key,pending_target_id,request_canonical,request_sha256,result_canonical,result_sha256)
    VALUES(ev,c.owner_user_id,c.account_id,c.control_epoch,'RECOVERY_AUTHORIZED',c.owner_user_id,evidence,public.daily_0060_digest(public.paper_0068_canonical_json(evidence)),request_key,pid,req,public.daily_0060_digest(req),receipt_result,public.daily_0060_digest(receipt_result));
  IF s.settlement_sha256 IS NULL THEN
    UPDATE public.owner_beta_paper_sessions SET processing_origin='RECOVERY',state='RECOVERY_REQUIRED',reason_code='RECOVERY_REQUIRED',updated_at=clock_timestamp() WHERE pending_target_id=pid;
  END IF;
  INSERT INTO public.owner_beta_paper_session_events(pending_target_id,event_seq,kind,reason_code,payload)
    SELECT pid,COALESCE(max(event_seq),0)+1,'RECOVERY_AUTHORIZED','RECOVERY_REQUIRED',evidence FROM public.owner_beta_paper_session_events WHERE pending_target_id=pid;
  PERFORM public.paper_0068_restore_actor(old_actor);
  RETURN ok;
EXCEPTION WHEN query_canceled OR assert_failure THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
WHEN OTHERS THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
END $$;

-- Private historical maintenance read. Kept separate from all ordinary 0066
-- readers; pre-dispatch verification cannot require a grant that does not exist.
CREATE FUNCTION public.paper_0068_read_recovery_session(evidence jsonb, session date, cohort jsonb) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE c public.owner_beta_daily_control; a public.owner_beta_daily_admissions; link public.owner_beta_paper_source_admissions;
    rr public.owner_beta_paper_source_runs; res public.owner_beta_daily_reservations;
    r public.owner_beta_recommendation_runs; j public.jobs; attempt public.job_attempts; claim public.owner_beta_daily_publication_claims;
    s public.owner_beta_paper_source_slots; pc public.owner_beta_paper_control;
    target public.pending_targets; session_row public.owner_beta_paper_sessions;
    lineage public.owner_beta_paper_lineage; original_event public.owner_beta_paper_events;
    dispatch_event public.owner_beta_paper_events;
    config public.user_strategy_configs; account public.accounts; grant_row public.owner_beta_paper_recovery_grants;
    request_key text:=evidence->>'request_key'; selected_admission text; k text;
    expected_expiry timestamptz; decision_hash text; original_hash text;
    result jsonb; old_actor text:=current_setting('app.actor_user_id',true); terminal text; attempts jsonb;
BEGIN
    -- This is a read, never a grant, transition or substitute for live Current/Source.
    -- Root's retained R/A/E/Q and actual Source custody are checked by the native
    -- consumer. JSON or this SQL result alone cannot authenticate filesystem origin.
    PERFORM public.paper_0068_principal();
    IF evidence IS NULL OR octet_length(public.paper_0068_canonical_json(evidence))>65536
       OR public.paper_0068_json_depth(evidence)>16
       OR evidence->>'schema' IS DISTINCT FROM 'owner-beta-paper-recovery-v2'
       OR session IS NULL OR cohort IS NULL
       OR octet_length(public.paper_0068_canonical_json(cohort))>65536
       OR public.paper_0068_json_depth(cohort)>16 THEN
      RAISE EXCEPTION 'recovery reader shape' USING ERRCODE='23514';
    END IF;
    -- Do not lock Paper control/target/session: the consuming transaction
    -- already holds their write fence during repeated reads. A second connection
    -- locking them would deadlock against itself. Immutable source rows and the
    -- current Daily/source locks below remain protected; the consumer independently
    -- repeats original/control identity under its write fence before COMMIT.
    SELECT * INTO pc FROM public.owner_beta_paper_control WHERE id='etf11';
    IF NOT FOUND OR (NULLIF(old_actor,'') IS NOT NULL AND old_actor<>pc.owner_user_id::text) THEN
      RAISE EXCEPTION 'recovery reader actor/control' USING ERRCODE='42501';
    END IF;
  IF NOT public.daily_0060_keys(evidence,ARRAY['schema','installation_id','generation','account_id','request_key','pending_target_id','lineage_id','effective_session','lineage_epoch','active_epoch','original_receipt_sha256','consent_event_id','process_config_sha256','step','signal_admission_sha256','execution_admission_sha256','decision_sha256','reason_code','expires_at'])
     OR EXISTS(SELECT 1 FROM jsonb_each(evidence) e WHERE e.value='null'::jsonb)
     OR jsonb_typeof(evidence->'installation_id') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'generation') IS DISTINCT FROM 'string'
     OR jsonb_typeof(evidence->'account_id') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'request_key') IS DISTINCT FROM 'string'
     OR jsonb_typeof(evidence->'pending_target_id') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'lineage_id') IS DISTINCT FROM 'string'
     OR jsonb_typeof(evidence->'effective_session') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'lineage_epoch') IS DISTINCT FROM 'number'
     OR jsonb_typeof(evidence->'active_epoch') IS DISTINCT FROM 'number' OR jsonb_typeof(evidence->'original_receipt_sha256') IS DISTINCT FROM 'string'
     OR jsonb_typeof(evidence->'consent_event_id') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'process_config_sha256') IS DISTINCT FROM 'string'
     OR jsonb_typeof(evidence->'step') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'signal_admission_sha256') IS DISTINCT FROM 'string'
     OR jsonb_typeof(evidence->'execution_admission_sha256') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'decision_sha256') IS DISTINCT FROM 'string'
     OR jsonb_typeof(evidence->'reason_code') IS DISTINCT FROM 'string' OR jsonb_typeof(evidence->'expires_at') IS DISTINCT FROM 'string'
     OR request_key<>evidence->>'generation'||':recover' OR evidence->>'reason_code'<>'EXPLICIT_RETAINED_RECOVERY'
     OR evidence->>'account_id'<>pc.account_id::text OR evidence->>'installation_id' !~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR evidence->>'generation' !~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR evidence->>'pending_target_id' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR evidence->>'lineage_id' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR evidence->>'consent_event_id' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR evidence->>'effective_session' !~ '^20[0-9]{2}-[0-9]{2}-[0-9]{2}$'
     OR NOT public.paper_0068_nonnegative_i64(evidence->>'lineage_epoch')
     OR NOT public.paper_0068_nonnegative_i64(evidence->>'active_epoch')
     OR evidence->>'original_receipt_sha256' !~ '^sha256:[0-9a-f]{64}$' OR evidence->>'process_config_sha256' !~ '^sha256:[0-9a-f]{64}$'
     OR evidence->>'signal_admission_sha256' !~ '^sha256:[0-9a-f]{64}$' OR evidence->>'execution_admission_sha256' !~ '^sha256:[0-9a-f]{64}$'
     OR evidence->>'decision_sha256' !~ '^sha256:[0-9a-f]{64}$' OR evidence->>'step' NOT IN ('SETTLE_THEN_VALUE','VALUE_ONLY')
  THEN RAISE EXCEPTION 'recovery-v2 decision schema invalid' USING ERRCODE='23514'; END IF;
  IF (evidence->>'active_epoch')::bigint<>pc.control_epoch OR (evidence->>'lineage_epoch')::bigint>=pc.control_epoch
     OR evidence->>'process_config_sha256'<>pc.process_binding_sha256 OR evidence->>'consent_event_id'<>pc.consent_event_id::text
     OR pc.state<>'ENABLED' OR evidence->>'expires_at' !~ '^20[0-9]{2}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$'
  THEN RAISE EXCEPTION 'recovery current control authority mismatch' USING ERRCODE='42501',CONSTRAINT='paper_recovery_v2_control'; END IF;
  expected_expiry:=((clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date+1)::timestamp AT TIME ZONE 'Asia/Seoul';
  IF (evidence->>'expires_at')::timestamptz<>expected_expiry THEN RAISE EXCEPTION 'recovery grant must expire at next KST midnight' USING ERRCODE='23514'; END IF;
  decision_hash:=public.daily_0060_digest(public.paper_0068_canonical_json(jsonb_build_object('effective_session',evidence->'effective_session','pending_target_id',evidence->'pending_target_id','step',evidence->'step')));
  IF decision_hash<>evidence->>'decision_sha256' THEN RAISE EXCEPTION 'recovery decision hash mismatch' USING ERRCODE='23514'; END IF;

    PERFORM set_config('app.actor_user_id',pc.owner_user_id::text,true);
    SELECT * INTO account FROM public.accounts WHERE id=pc.account_id AND owner_user_id=pc.owner_user_id FOR SHARE;
    PERFORM 1 FROM public.account_strategy_bindings b WHERE b.account_id=pc.account_id
      AND b.owner_user_id=pc.owner_user_id AND b.strategy_config_id=pc.strategy_config_id AND b.unbound_at IS NULL FOR SHARE;
    IF account.id IS NULL OR account.account_type<>'PAPER' OR account.status<>'ACTIVE' OR account.currency<>'KRW'
       OR account.initial_cash<>10000000.0000 OR account.cost_profile_id<>pc.cost_profile_id
       OR account.cost_profile_version<>pc.cost_profile_version
       OR pc.cost_profile_sha256 IS DISTINCT FROM public.daily_0060_digest('KRX_ETF_DEFAULT@1|buy_commission=0.00015|sell_commission=0.00015|min_commission=1000.0000|slippage=10bp|min_trade=100000.0000|rebalance_threshold=0.005000|sell_tax=0.00000')
       OR NOT EXISTS(SELECT 1 FROM public.account_strategy_bindings b WHERE b.account_id=pc.account_id
          AND b.owner_user_id=pc.owner_user_id AND b.strategy_config_id=pc.strategy_config_id AND b.unbound_at IS NULL)
       OR NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_events e WHERE e.id=pc.consent_event_id
          AND e.account_id=pc.account_id AND e.owner_user_id=pc.owner_user_id
          AND e.actor_user_id=pc.owner_user_id AND e.kind='CONSENT_GRANTED')
    THEN RAISE EXCEPTION 'recovery reader account/consent' USING ERRCODE='42501'; END IF;
    -- Ordinary current authority remains exact, including CURRENT activation,
    -- config epoch, enabled state, first-auto-session, owner and present rights.
    c:=public.paper_0066_authority(pc.daily_cohort,(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date);
    IF c.owner_user_id IS DISTINCT FROM pc.owner_user_id
       OR pc.process_binding->>'owner_user_id' IS DISTINCT FROM pc.owner_user_id::text
       OR pc.process_binding->>'account_id' IS DISTINCT FROM pc.account_id::text
       OR pc.process_binding->>'strategy_config_id' IS DISTINCT FROM pc.strategy_config_id::text
       OR pc.process_binding->>'consent_event_id' IS DISTINCT FROM pc.consent_event_id::text
       OR pc.process_binding->>'consent_version' IS DISTINCT FROM pc.contract_id
       OR pc.process_binding->'control_epoch' IS DISTINCT FROM to_jsonb(pc.control_epoch)
       OR pc.process_binding->>'daily_cohort_sha256' IS DISTINCT FROM pc.cohort_sha256
       OR pc.daily_cohort->>'cohort_sha256' IS DISTINCT FROM pc.cohort_sha256
       OR pc.process_binding->>'financial_contract_sha256' IS DISTINCT FROM pc.contract_sha256
       OR public.daily_0060_digest(public.paper_0068_canonical_json(pc.process_binding)) IS DISTINCT FROM pc.process_binding_sha256
    THEN RAISE EXCEPTION 'recovery reader current identity' USING ERRCODE='42501'; END IF;
    SELECT * INTO target FROM public.pending_targets WHERE id=(evidence->>'pending_target_id')::uuid
      AND account_id=pc.account_id AND owner_user_id=pc.owner_user_id;
    SELECT * INTO lineage FROM public.owner_beta_paper_lineage WHERE id=target.beta_lineage_id
      AND account_id=pc.account_id AND owner_user_id=pc.owner_user_id AND strategy_config_id=pc.strategy_config_id;
    SELECT * INTO session_row FROM public.owner_beta_paper_sessions WHERE pending_target_id=target.id
      AND account_id=pc.account_id AND owner_user_id=pc.owner_user_id;
    IF target.id IS NULL OR lineage.id IS NULL OR session_row.pending_target_id IS NULL
       OR lineage.id::text IS DISTINCT FROM evidence->>'lineage_id'
       OR lineage.control_epoch IS DISTINCT FROM (evidence->>'lineage_epoch')::bigint
       OR lineage.effective_session IS DISTINCT FROM (evidence->>'effective_session')::date
       OR session_row.effective_session IS DISTINCT FROM lineage.effective_session
       OR lineage.signal_admission_sha256 IS DISTINCT FROM evidence->>'signal_admission_sha256'
       OR target.recommendation_run_id IS NOT NULL
       OR target.source_kind NOT IN ('OWNER_BETA_DAILY_MANUAL','OWNER_BETA_DAILY_AUTO')
       OR ROW(target.strategy_config_id,target.computed_on,target.effective_date,target.targets_json)
          IS DISTINCT FROM ROW(lineage.strategy_config_id,lineage.signal_date,lineage.effective_session,lineage.target_weights_json)
       OR session NOT IN (lineage.signal_date,lineage.effective_session)
       OR lineage.signal_date>=lineage.effective_session
       OR lineage.effective_session>=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date
       OR session_row.valuation_sha256 IS NOT NULL
       OR NOT ((session_row.settlement_sha256 IS NULL AND target.status='PENDING'
            AND session_row.state IN ('PENDING','WAITING_PRICE','WAITING_ADMISSION','HELD','BLOCKED','RECOVERY_REQUIRED')
            AND evidence->>'step'='SETTLE_THEN_VALUE')
          OR (session_row.settlement_sha256 IS NOT NULL AND evidence->>'step'='VALUE_ONLY'
            AND ((target.status='EXECUTED' AND session_row.state='SETTLED_TRADE')
              OR (target.status='SKIPPED' AND session_row.state='SETTLED_NO_TRADE'))))
       OR EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions earlier WHERE earlier.account_id=pc.account_id
          AND earlier.effective_session<lineage.effective_session AND (earlier.state<>'VALUED' OR earlier.valuation_sha256 IS NULL))
    THEN RAISE EXCEPTION 'recovery reader original lineage' USING ERRCODE='23514'; END IF;
    original_hash:=COALESCE(session_row.settlement_sha256,public.daily_0060_digest(public.paper_0068_canonical_json(COALESCE(session_row.receipt_json,'null'::jsonb))));
    IF original_hash IS DISTINCT FROM evidence->>'original_receipt_sha256' THEN
      RAISE EXCEPTION 'recovery reader original receipt' USING ERRCODE='23514'; END IF;
    SELECT * INTO original_event FROM public.owner_beta_paper_events WHERE id=lineage.activation_event_id
      AND account_id=pc.account_id AND owner_user_id=pc.owner_user_id AND control_epoch=lineage.control_epoch AND kind='ENABLED';
    IF NOT FOUND OR original_event.payload#>>'{process_binding,owner_user_id}' IS DISTINCT FROM pc.owner_user_id::text
       OR original_event.payload#>>'{process_binding,account_id}' IS DISTINCT FROM pc.account_id::text
       OR original_event.payload#>>'{process_binding,strategy_config_id}' IS DISTINCT FROM pc.strategy_config_id::text
       OR original_event.payload#>>'{process_binding,consent_event_id}' IS DISTINCT FROM lineage.consent_event_id::text
       OR original_event.payload#>'{process_binding,control_epoch}' IS DISTINCT FROM to_jsonb(lineage.control_epoch)
       OR original_event.payload#>>'{process_binding,daily_cohort_sha256}' IS DISTINCT FROM lineage.cohort_sha256
       OR original_event.payload#>>'{process_binding,financial_contract_sha256}' IS DISTINCT FROM pc.contract_sha256
       OR original_event.payload#>>'{process_binding,consent_version}' IS DISTINCT FROM pc.contract_id
       OR public.daily_0060_digest(public.paper_0068_canonical_json(original_event.payload->'process_binding'))
          IS DISTINCT FROM original_event.payload->>'process_binding_sha256'
    THEN RAISE EXCEPTION 'recovery reader original economics' USING ERRCODE='23514'; END IF;
    SELECT * INTO config FROM public.user_strategy_configs WHERE id=pc.strategy_config_id AND owner_user_id=pc.owner_user_id;
    IF NOT FOUND OR NOT EXISTS(SELECT 1 FROM public.owner_beta_recommendation_runs r0
      WHERE r0.id=lineage.recommendation_run_id AND r0.owner_user_id=pc.owner_user_id
        AND r0.strategy_config_id=config.id AND r0.strategy_id=config.strategy_id
        AND r0.strategy_version=config.strategy_version AND r0.strategy_config_json=config.config_json
        AND r0.strategy_config_sha256=pc.daily_cohort->>'strategy_config_sha256'
        AND r0.factor_snapshot_sha256=lineage.factor_snapshot_sha256 AND r0.target_snapshot_sha256=lineage.target_snapshot_sha256)
    THEN RAISE EXCEPTION 'recovery reader changed strategy semantics' USING ERRCODE='23514'; END IF;
    selected_admission:=CASE WHEN session=lineage.signal_date THEN lineage.signal_admission_sha256 ELSE evidence->>'execution_admission_sha256' END;
    SELECT sl.* INTO s FROM public.owner_beta_paper_source_slots sl
      JOIN public.owner_beta_paper_source_admissions sa USING(slot_id)
      WHERE sl.owner_user_id=pc.owner_user_id AND sl.policy_sha256=c.policy_sha256
        AND sl.session_date=session AND sa.admission_sha256=selected_admission;
    IF NOT FOUND OR s.cohort IS DISTINCT FROM cohort
       OR (session=lineage.signal_date AND (s.cohort_sha256 IS DISTINCT FROM lineage.cohort_sha256
          OR s.activation_epoch IS DISTINCT FROM lineage.daily_activation_epoch
          OR s.cohort->'config_epoch' IS DISTINCT FROM to_jsonb(lineage.config_epoch)
          OR NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_source_runs sr WHERE sr.slot_id=s.slot_id AND sr.run_id=lineage.recommendation_run_id)))
    THEN RAISE EXCEPTION 'recovery reader original Source slot' USING ERRCODE='23514'; END IF;
    FOREACH k IN ARRAY ARRAY['owner_user_id','policy_sha256','strategy_config_id','strategy_config_sha256','entitlement_reference','entitlement_sha256'] LOOP
      IF cohort->k IS NULL OR cohort->k IS DISTINCT FROM pc.daily_cohort->k THEN
        RAISE EXCEPTION 'recovery reader changed cohort semantics' USING ERRCODE='23514';
      END IF;
    END LOOP;
    IF NOT public.daily_0060_admission_usable(c,lineage.signal_admission_sha256,false)
       OR NOT public.daily_0060_admission_usable(c,evidence->>'execution_admission_sha256',false)
       OR NOT EXISTS(SELECT 1 FROM public.owner_beta_daily_admissions e
          JOIN public.owner_beta_daily_admissions t0 ON t0.admission_sha256=lineage.signal_admission_sha256
          WHERE e.admission_sha256=evidence->>'execution_admission_sha256'
            AND e.owner_user_id=pc.owner_user_id AND e.range_end=lineage.effective_session
            AND t0.range_end=lineage.signal_date AND e.policy_sha256=t0.policy_sha256 AND e.range_start=t0.range_start
            AND e.evidence_json#>'{admission,base}'=c.policy_json->'base'
            AND t0.evidence_json#>'{admission,base}'=c.policy_json->'base'
            AND (WITH RECURSIVE ancestry(admission_sha256,parent_admission_sha256) AS (
              SELECT admission_sha256,parent_admission_sha256 FROM public.owner_beta_daily_admissions WHERE admission_sha256=e.admission_sha256
              UNION ALL SELECT p.admission_sha256,p.parent_admission_sha256 FROM public.owner_beta_daily_admissions p JOIN ancestry a0 ON p.admission_sha256=a0.parent_admission_sha256)
              SELECT bool_or(admission_sha256=lineage.signal_admission_sha256) FROM ancestry))
    THEN RAISE EXCEPTION 'recovery reader admission/prefix' USING ERRCODE='23514'; END IF;
    SELECT * INTO grant_row FROM public.owner_beta_paper_recovery_grants g
      WHERE g.account_id=pc.account_id AND g.request_key=paper_0068_read_recovery_session.request_key;
    IF FOUND THEN
      SELECT * INTO dispatch_event FROM public.owner_beta_paper_events e WHERE e.account_id=pc.account_id
        AND e.request_key=paper_0068_read_recovery_session.request_key;
      IF NOT FOUND THEN RAISE EXCEPTION 'recovery reader missing dispatch' USING ERRCODE='42501'; END IF;
      PERFORM public.paper_0068_validate_receipt(to_jsonb(dispatch_event),'owner_beta_paper_events');
      IF dispatch_event.request_canonical IS DISTINCT FROM public.paper_0068_request('TRANSITION',pc.account_id,
          jsonb_build_object('expected_epoch',pc.control_epoch,'transition','RECOVERY',
            'evidence_canonical',public.paper_0068_canonical_json(evidence),'request_key',request_key))
         OR ROW(grant_row.pending_target_id,grant_row.lineage_id,grant_row.effective_session,grant_row.lineage_epoch,grant_row.active_epoch,
             grant_row.consent_event_id,grant_row.process_config_sha256,grant_row.original_receipt_sha256,
             grant_row.signal_admission_sha256,grant_row.execution_admission_sha256,grant_row.step,grant_row.decision_sha256)
           IS DISTINCT FROM ROW(target.id,lineage.id,lineage.effective_session,lineage.control_epoch,pc.control_epoch,
             pc.consent_event_id,pc.process_binding_sha256,original_hash,lineage.signal_admission_sha256,
             evidence->>'execution_admission_sha256',evidence->>'step',decision_hash)
         OR grant_row.decision IS DISTINCT FROM evidence OR grant_row.revoked_at IS NOT NULL
         OR grant_row.expires_at<=clock_timestamp() OR grant_row.valuation_receipt_sha256 IS NOT NULL
         OR NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_events e WHERE e.account_id=pc.account_id
            AND e.request_key=paper_0068_read_recovery_session.request_key AND e.kind='RECOVERY_AUTHORIZED'
            AND e.payload=evidence AND e.pending_target_id=target.id AND e.control_epoch=pc.control_epoch
            AND e.result_canonical::jsonb#>>'{data,grant_id}'=grant_row.grant_id::text)
      THEN RAISE EXCEPTION 'recovery reader grant/dispatch mismatch' USING ERRCODE='42501'; END IF;
    ELSIF EXISTS(SELECT 1 FROM public.owner_beta_paper_events e WHERE e.account_id=pc.account_id
       AND e.request_key=paper_0068_read_recovery_session.request_key)
       OR EXISTS(SELECT 1 FROM public.owner_beta_paper_recovery_grants g WHERE g.pending_target_id=target.id
          AND g.active_epoch=pc.control_epoch AND g.revoked_at IS NULL AND g.valuation_receipt_sha256 IS NULL
          AND g.expires_at>clock_timestamp()) THEN
      RAISE EXCEPTION 'recovery reader conflicting or incomplete dispatch' USING ERRCODE='42501';
    END IF;
    -- Original 0066 DB wire/join checks below, with no epoch rewriting.

    SELECT * INTO a FROM public.owner_beta_daily_admissions WHERE owner_user_id=c.owner_user_id AND range_end=s.session_date;
    IF a.admission_sha256 IS NOT NULL AND (a.policy_sha256<>c.policy_sha256 OR NOT public.daily_0060_admission_usable(c,a.admission_sha256,false)) THEN
        RAISE EXCEPTION 'paper evidence admission unavailable' USING ERRCODE='23514'; END IF;
    SELECT * INTO link FROM public.owner_beta_paper_source_admissions WHERE slot_id=s.slot_id;
    IF link.slot_id IS NOT NULL AND link.admission_sha256 IS DISTINCT FROM a.admission_sha256 THEN
        RAISE EXCEPTION 'paper evidence admission join' USING ERRCODE='23514'; END IF;
    SELECT * INTO rr FROM public.owner_beta_paper_source_runs WHERE slot_id=s.slot_id;
    PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
    SELECT * INTO res FROM public.owner_beta_daily_reservations
        WHERE owner_user_id=c.owner_user_id AND strategy_config_id=(s.cohort->>'strategy_config_id')::uuid AND session_date=s.session_date AND trigger_version=c.trigger_version;
    IF res.run_id IS NOT NULL THEN
        SELECT * INTO r FROM public.owner_beta_recommendation_runs WHERE id=res.run_id;
        SELECT * INTO j FROM public.jobs WHERE id=res.job_id;
        SELECT * INTO attempt FROM public.job_attempts WHERE job_id=res.job_id AND attempt_no=j.attempt_count;
        SELECT * INTO claim FROM public.owner_beta_daily_publication_claims WHERE run_id=res.run_id;
        IF r.id IS NULL OR j.id IS NULL OR r.trigger_kind IS DISTINCT FROM 'DAILY'
           OR ROW(r.owner_user_id,j.owner_user_id,res.owner_user_id) IS DISTINCT FROM ROW(c.owner_user_id,c.owner_user_id,c.owner_user_id)
           OR ROW(r.as_of,res.session_date) IS DISTINCT FROM ROW(s.session_date,s.session_date)
           OR ROW(r.job_id,r.strategy_config_id,r.daily_config_epoch,r.daily_activation_epoch,r.strategy_config_sha256)
              IS DISTINCT FROM ROW(j.id,res.strategy_config_id,res.config_epoch,res.activation_epoch,res.strategy_config_sha256)
           OR res.config_epoch IS DISTINCT FROM (s.cohort->>'config_epoch')::bigint
           OR res.activation_epoch IS DISTINCT FROM s.activation_epoch
           OR res.strategy_config_sha256 IS DISTINCT FROM s.cohort->>'strategy_config_sha256'
           OR res.admission_sha256 IS DISTINCT FROM a.admission_sha256
           OR r.daily_pins IS DISTINCT FROM jsonb_build_object('admission_sha256',a.admission_sha256,'content_sha256',a.content_sha256,'manifest_sha256',a.manifest_sha256,'policy_sha256',a.policy_sha256)
           OR j.payload_json IS DISTINCT FROM public.daily_0060_payload(r)
           OR j.job_type IS DISTINCT FROM 'owner_beta_price_recommendation'
        THEN RAISE EXCEPTION 'paper evidence run join' USING ERRCODE='23514'; END IF;
        SELECT jsonb_agg(jsonb_build_object('id',id,'attempt_no',attempt_no,'outcome',outcome,'started_at',started_at,'finished_at',finished_at) ORDER BY attempt_no) INTO attempts FROM public.job_attempts WHERE job_id=j.id;
        IF COALESCE(jsonb_array_length(attempts),0) IS DISTINCT FROM j.attempt_count OR j.attempt_count NOT BETWEEN 0 AND 3 THEN
            RAISE EXCEPTION 'paper evidence attempts incomplete' USING ERRCODE='23514'; END IF;
        terminal:=j.status;
        IF terminal='SUCCEEDED' AND (r.status IS DISTINCT FROM 'SUCCEEDED' OR attempt.outcome IS DISTINCT FROM 'SUCCEEDED'
           OR ROW(claim.run_id,claim.job_id,claim.owner_user_id,claim.attempt_id,claim.attempt_no,claim.claimed_by)
              IS DISTINCT FROM ROW(r.id,j.id,c.owner_user_id,attempt.id,j.attempt_count,attempt.claimed_by)
           OR r.finished_at IS DISTINCT FROM claim.authorized_at OR j.finished_at IS NULL OR attempt.finished_at IS NULL
           OR j.finished_at<claim.authorized_at OR attempt.finished_at<j.finished_at
           OR r.factor_snapshot_sha256 IS NULL OR r.target_snapshot_sha256 IS NULL
           OR r.finished_at >= ((s.session_date+1)::timestamp AT TIME ZONE 'Asia/Seoul')
           OR j.finished_at >= ((s.session_date+1)::timestamp AT TIME ZONE 'Asia/Seoul')
           OR attempt.finished_at >= ((s.session_date+1)::timestamp AT TIME ZONE 'Asia/Seoul')
           OR EXISTS(SELECT 1 FROM public.job_attempts x WHERE x.job_id=j.id AND x.attempt_no<j.attempt_count AND x.outcome NOT IN ('FAILED','ORPHANED')))
        THEN RAISE EXCEPTION 'paper evidence terminal mismatch' USING ERRCODE='23514'; END IF;
        IF rr.run_id IS NOT NULL AND (ROW(rr.run_id,rr.job_id,rr.attempt_id) IS DISTINCT FROM ROW(r.id,j.id,attempt.id) OR terminal<>'SUCCEEDED') THEN
            RAISE EXCEPTION 'paper evidence linked terminal changed' USING ERRCODE='23514'; END IF;
    END IF;
    result:=jsonb_build_object('schema','owner-beta-paper-db-v1','slot',to_jsonb(s),
       'admission',CASE WHEN link.slot_id IS NULL THEN NULL ELSE to_jsonb(link) END,
       'run',CASE WHEN rr.slot_id IS NULL THEN NULL ELSE to_jsonb(rr) END,
       'candidate_admission_sha256',a.admission_sha256,'candidate_run_id',res.run_id,
       'terminal',terminal,'job_id',j.id,'attempt_id',attempt.id,'attempts',attempts,
       'factor_snapshot_sha256',r.factor_snapshot_sha256,'target_snapshot_sha256',r.target_snapshot_sha256,
       'candidate_pins',CASE WHEN a.admission_sha256 IS NULL THEN NULL ELSE jsonb_build_object('content_sha256',a.content_sha256,'manifest_sha256',a.manifest_sha256,'admission_sha256',a.admission_sha256,'policy_sha256',a.policy_sha256) END,
       'pins',r.daily_pins,'admitted_at',a.admitted_at,'finished_at',attempt.finished_at);
    IF octet_length(public.paper_0068_canonical_json(result))>262144 THEN
      RAISE EXCEPTION 'recovery reader result bound' USING ERRCODE='23514';
    END IF;
    PERFORM public.paper_0068_restore_actor(old_actor);
    RETURN result;
EXCEPTION WHEN query_canceled OR assert_failure THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
WHEN OTHERS THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
END $$;

ALTER FUNCTION public.paper_0068_read_recovery_session(jsonb,date,jsonb) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.paper_0068_read_recovery_session(jsonb,date,jsonb) FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter;
GRANT EXECUTE ON FUNCTION public.paper_0068_read_recovery_session(jsonb,date,jsonb) TO migration_owner;

-- Worker-only historical input read. Every argument is an untrusted selector.
-- No native Root privilege/secret or caller-supplied checked capability crosses here.
CREATE FUNCTION public.paper_0068_read_worker_recovery(p_target uuid,p_grant uuid,p_epoch bigint,p_process text,p_action text,p_session date,p_receipt text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE pc public.owner_beta_paper_control; c public.owner_beta_daily_control;
    target public.pending_targets; session_row public.owner_beta_paper_sessions;
    lineage public.owner_beta_paper_lineage; g public.owner_beta_paper_recovery_grants;
    original_event public.owner_beta_paper_events; config public.user_strategy_configs;
    a public.owner_beta_daily_admissions; link public.owner_beta_paper_source_admissions;
    rr public.owner_beta_paper_source_runs; res public.owner_beta_daily_reservations;
    r public.owner_beta_recommendation_runs; j public.jobs; attempt public.job_attempts;
    claim public.owner_beta_daily_publication_claims; s public.owner_beta_paper_source_slots;
    ts public.owner_beta_paper_source_slots; es public.owner_beta_paper_source_slots;
    old_actor text:=current_setting('app.actor_user_id',true); authority jsonb;
    selected_admission text; cohort jsonb; k text; result jsonb; selection jsonb; terminal text; attempts jsonb;
BEGIN
    PERFORM public.paper_0067_principal('worker');
    IF p_target IS NULL OR p_grant IS NULL OR p_epoch IS NULL OR p_process IS NULL
       OR p_action IS NULL OR p_action NOT IN ('SETTLE','VALUE') OR p_session IS NULL
       OR (p_receipt IS NOT NULL AND p_receipt !~ '^sha256:[0-9a-f]{64}$') THEN
      RAISE EXCEPTION 'worker recovery selector invalid' USING ERRCODE='23514';
    END IF;
    -- Existing serving order: control -> account advisory/account -> current
    -- Daily/config/rights -> target -> session -> grant. Same transaction as economics.
    SELECT * INTO pc FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
    IF NOT FOUND OR (NULLIF(old_actor,'') IS NOT NULL AND old_actor<>pc.owner_user_id::text) THEN
      RAISE EXCEPTION 'worker recovery owner/actor' USING ERRCODE='42501';
    END IF;
    pc:=public.paper_0067_authority(p_action,(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date,p_epoch,p_process,true);
    PERFORM set_config('app.actor_user_id',pc.owner_user_id::text,true);
    SELECT * INTO target FROM public.pending_targets WHERE id=p_target AND owner_user_id=pc.owner_user_id AND account_id=pc.account_id FOR UPDATE;
    SELECT * INTO session_row FROM public.owner_beta_paper_sessions WHERE pending_target_id=p_target AND owner_user_id=pc.owner_user_id AND account_id=pc.account_id FOR UPDATE;
    SELECT * INTO g FROM public.owner_beta_paper_recovery_grants WHERE grant_id=p_grant AND pending_target_id=p_target AND account_id=pc.account_id AND active_epoch=p_epoch FOR UPDATE;
    IF NOT FOUND OR target.id IS NULL OR session_row.pending_target_id IS NULL THEN
      RAISE EXCEPTION 'worker recovery exact grant missing' USING ERRCODE='42501';
    END IF;
    authority:=public.paper_0068_recovery_authority(p_target,p_epoch,p_process,
      CASE WHEN p_receipt IS NULL THEN p_action ELSE p_action||'_READBACK' END);
    IF authority IS NULL OR authority->>'grant_id' IS DISTINCT FROM p_grant::text
       OR (p_receipt IS NOT NULL AND p_receipt IS DISTINCT FROM
         CASE p_action WHEN 'SETTLE' THEN g.settlement_receipt_sha256 ELSE g.valuation_receipt_sha256 END) THEN
      RAISE EXCEPTION 'worker recovery live grant unavailable' USING ERRCODE='42501';
    END IF;
    SELECT * INTO lineage FROM public.owner_beta_paper_lineage WHERE id=target.beta_lineage_id AND account_id=pc.account_id AND owner_user_id=pc.owner_user_id FOR SHARE;
    IF lineage.id IS NULL OR lineage.id IS DISTINCT FROM g.lineage_id OR lineage.control_epoch>=pc.control_epoch
       OR target.recommendation_run_id IS NOT NULL OR target.source_kind NOT IN ('OWNER_BETA_DAILY_MANUAL','OWNER_BETA_DAILY_AUTO')
       OR ROW(target.strategy_config_id,target.computed_on,target.effective_date,target.targets_json)
          IS DISTINCT FROM ROW(lineage.strategy_config_id,lineage.signal_date,lineage.effective_session,lineage.target_weights_json)
       OR p_session NOT IN (lineage.signal_date,lineage.effective_session)
       OR lineage.signal_date>=lineage.effective_session OR lineage.effective_session>=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date
       OR pc.cost_profile_sha256 IS DISTINCT FROM public.daily_0060_digest('KRX_ETF_DEFAULT@1|buy_commission=0.00015|sell_commission=0.00015|min_commission=1000.0000|slippage=10bp|min_trade=100000.0000|rebalance_threshold=0.005000|sell_tax=0.00000')
       OR public.daily_0060_digest(public.paper_0068_canonical_json(pc.process_binding)) IS DISTINCT FROM pc.process_binding_sha256 THEN
      RAISE EXCEPTION 'worker recovery original target changed' USING ERRCODE='23514';
    END IF;
    c:=public.paper_0066_authority(pc.daily_cohort,(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date);
    SELECT sl.* INTO ts FROM public.owner_beta_paper_source_slots sl
      JOIN public.owner_beta_paper_source_admissions sa USING(slot_id)
      JOIN public.owner_beta_paper_source_runs sr USING(slot_id)
      WHERE sl.owner_user_id=pc.owner_user_id AND sl.session_date=lineage.signal_date
        AND sa.admission_sha256=g.signal_admission_sha256 AND sr.run_id=lineage.recommendation_run_id;
    SELECT sl.* INTO es FROM public.owner_beta_paper_source_slots sl
      JOIN public.owner_beta_paper_source_admissions sa USING(slot_id)
      WHERE sl.owner_user_id=pc.owner_user_id AND sl.session_date=lineage.effective_session AND sa.admission_sha256=g.execution_admission_sha256;
    IF ts.slot_id IS NULL OR es.slot_id IS NULL OR ts.cohort_sha256 IS DISTINCT FROM lineage.cohort_sha256
       OR ts.activation_epoch IS DISTINCT FROM lineage.daily_activation_epoch
       OR ts.cohort->'config_epoch' IS DISTINCT FROM to_jsonb(lineage.config_epoch) THEN
      RAISE EXCEPTION 'worker recovery retained Source missing' USING ERRCODE='23514';
    END IF;
    cohort:=CASE WHEN p_session=lineage.signal_date THEN ts.cohort ELSE es.cohort END;
    SELECT * INTO original_event FROM public.owner_beta_paper_events WHERE id=lineage.activation_event_id
      AND account_id=pc.account_id AND owner_user_id=pc.owner_user_id AND control_epoch=lineage.control_epoch AND kind='ENABLED';
    IF NOT FOUND OR original_event.payload#>>'{process_binding,owner_user_id}' IS DISTINCT FROM pc.owner_user_id::text
       OR original_event.payload#>>'{process_binding,account_id}' IS DISTINCT FROM pc.account_id::text
       OR original_event.payload#>>'{process_binding,strategy_config_id}' IS DISTINCT FROM pc.strategy_config_id::text
       OR original_event.payload#>>'{process_binding,consent_event_id}' IS DISTINCT FROM lineage.consent_event_id::text
       OR original_event.payload#>'{process_binding,control_epoch}' IS DISTINCT FROM to_jsonb(lineage.control_epoch)
       OR original_event.payload#>>'{process_binding,daily_cohort_sha256}' IS DISTINCT FROM lineage.cohort_sha256
       OR original_event.payload#>>'{process_binding,financial_contract_sha256}' IS DISTINCT FROM pc.contract_sha256
       OR original_event.payload#>>'{process_binding,consent_version}' IS DISTINCT FROM pc.contract_id
       OR public.daily_0060_digest(public.paper_0068_canonical_json(original_event.payload->'process_binding'))
          IS DISTINCT FROM original_event.payload->>'process_binding_sha256'
    THEN RAISE EXCEPTION 'recovery reader original economics' USING ERRCODE='23514'; END IF;
    SELECT * INTO config FROM public.user_strategy_configs WHERE id=pc.strategy_config_id AND owner_user_id=pc.owner_user_id;
    IF NOT FOUND OR NOT EXISTS(SELECT 1 FROM public.owner_beta_recommendation_runs r0
      WHERE r0.id=lineage.recommendation_run_id AND r0.owner_user_id=pc.owner_user_id
        AND r0.strategy_config_id=config.id AND r0.strategy_id=config.strategy_id
        AND r0.strategy_version=config.strategy_version AND r0.strategy_config_json=config.config_json
        AND r0.strategy_config_sha256=pc.daily_cohort->>'strategy_config_sha256'
        AND r0.factor_snapshot_sha256=lineage.factor_snapshot_sha256 AND r0.target_snapshot_sha256=lineage.target_snapshot_sha256)
    THEN RAISE EXCEPTION 'recovery reader changed strategy semantics' USING ERRCODE='23514'; END IF;
    selected_admission:=CASE WHEN p_session=lineage.signal_date THEN lineage.signal_admission_sha256 ELSE g.execution_admission_sha256 END;
    SELECT sl.* INTO s FROM public.owner_beta_paper_source_slots sl
      JOIN public.owner_beta_paper_source_admissions sa USING(slot_id)
      WHERE sl.owner_user_id=pc.owner_user_id AND sl.policy_sha256=c.policy_sha256
        AND sl.session_date=p_session AND sa.admission_sha256=selected_admission;
    IF NOT FOUND OR s.cohort IS DISTINCT FROM cohort
       OR (p_session=lineage.signal_date AND (s.cohort_sha256 IS DISTINCT FROM lineage.cohort_sha256
          OR s.activation_epoch IS DISTINCT FROM lineage.daily_activation_epoch
          OR s.cohort->'config_epoch' IS DISTINCT FROM to_jsonb(lineage.config_epoch)
          OR NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_source_runs sr WHERE sr.slot_id=s.slot_id AND sr.run_id=lineage.recommendation_run_id)))
    THEN RAISE EXCEPTION 'recovery reader original Source slot' USING ERRCODE='23514'; END IF;
    FOREACH k IN ARRAY ARRAY['owner_user_id','policy_sha256','strategy_config_id','strategy_config_sha256','entitlement_reference','entitlement_sha256'] LOOP
      IF cohort->k IS NULL OR cohort->k IS DISTINCT FROM pc.daily_cohort->k THEN
        RAISE EXCEPTION 'recovery reader changed cohort semantics' USING ERRCODE='23514';
      END IF;
    END LOOP;
    IF NOT public.daily_0060_admission_usable(c,lineage.signal_admission_sha256,false)
       OR NOT public.daily_0060_admission_usable(c,g.execution_admission_sha256,false)
       OR NOT EXISTS(SELECT 1 FROM public.owner_beta_daily_admissions e
          JOIN public.owner_beta_daily_admissions t0 ON t0.admission_sha256=lineage.signal_admission_sha256
          WHERE e.admission_sha256=g.execution_admission_sha256
            AND e.owner_user_id=pc.owner_user_id AND e.range_end=lineage.effective_session
            AND t0.range_end=lineage.signal_date AND e.policy_sha256=t0.policy_sha256 AND e.range_start=t0.range_start
            AND e.evidence_json#>'{admission,base}'=c.policy_json->'base'
            AND t0.evidence_json#>'{admission,base}'=c.policy_json->'base'
            AND (WITH RECURSIVE ancestry(admission_sha256,parent_admission_sha256) AS (
              SELECT admission_sha256,parent_admission_sha256 FROM public.owner_beta_daily_admissions WHERE admission_sha256=e.admission_sha256
              UNION ALL SELECT p.admission_sha256,p.parent_admission_sha256 FROM public.owner_beta_daily_admissions p JOIN ancestry a0 ON p.admission_sha256=a0.parent_admission_sha256)
              SELECT bool_or(admission_sha256=lineage.signal_admission_sha256) FROM ancestry))
    THEN RAISE EXCEPTION 'recovery reader admission/prefix' USING ERRCODE='23514'; END IF;
    SELECT * INTO a FROM public.owner_beta_daily_admissions WHERE owner_user_id=c.owner_user_id AND range_end=s.session_date;
    IF a.admission_sha256 IS NOT NULL AND (a.policy_sha256<>c.policy_sha256 OR NOT public.daily_0060_admission_usable(c,a.admission_sha256,false)) THEN
        RAISE EXCEPTION 'paper evidence admission unavailable' USING ERRCODE='23514'; END IF;
    SELECT * INTO link FROM public.owner_beta_paper_source_admissions WHERE slot_id=s.slot_id;
    IF link.slot_id IS NOT NULL AND link.admission_sha256 IS DISTINCT FROM a.admission_sha256 THEN
        RAISE EXCEPTION 'paper evidence admission join' USING ERRCODE='23514'; END IF;
    SELECT * INTO rr FROM public.owner_beta_paper_source_runs WHERE slot_id=s.slot_id;
    PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
    SELECT * INTO res FROM public.owner_beta_daily_reservations
        WHERE owner_user_id=c.owner_user_id AND strategy_config_id=(s.cohort->>'strategy_config_id')::uuid AND session_date=s.session_date AND trigger_version=c.trigger_version;
    IF res.run_id IS NOT NULL THEN
        SELECT * INTO r FROM public.owner_beta_recommendation_runs WHERE id=res.run_id;
        SELECT * INTO j FROM public.jobs WHERE id=res.job_id;
        SELECT * INTO attempt FROM public.job_attempts WHERE job_id=res.job_id AND attempt_no=j.attempt_count;
        SELECT * INTO claim FROM public.owner_beta_daily_publication_claims WHERE run_id=res.run_id;
        IF r.id IS NULL OR j.id IS NULL OR r.trigger_kind IS DISTINCT FROM 'DAILY'
           OR ROW(r.owner_user_id,j.owner_user_id,res.owner_user_id) IS DISTINCT FROM ROW(c.owner_user_id,c.owner_user_id,c.owner_user_id)
           OR ROW(r.as_of,res.session_date) IS DISTINCT FROM ROW(s.session_date,s.session_date)
           OR ROW(r.job_id,r.strategy_config_id,r.daily_config_epoch,r.daily_activation_epoch,r.strategy_config_sha256)
              IS DISTINCT FROM ROW(j.id,res.strategy_config_id,res.config_epoch,res.activation_epoch,res.strategy_config_sha256)
           OR res.config_epoch IS DISTINCT FROM (s.cohort->>'config_epoch')::bigint
           OR res.activation_epoch IS DISTINCT FROM s.activation_epoch
           OR res.strategy_config_sha256 IS DISTINCT FROM s.cohort->>'strategy_config_sha256'
           OR res.admission_sha256 IS DISTINCT FROM a.admission_sha256
           OR r.daily_pins IS DISTINCT FROM jsonb_build_object('admission_sha256',a.admission_sha256,'content_sha256',a.content_sha256,'manifest_sha256',a.manifest_sha256,'policy_sha256',a.policy_sha256)
           OR j.payload_json IS DISTINCT FROM public.daily_0060_payload(r)
           OR j.job_type IS DISTINCT FROM 'owner_beta_price_recommendation'
        THEN RAISE EXCEPTION 'paper evidence run join' USING ERRCODE='23514'; END IF;
        SELECT jsonb_agg(jsonb_build_object('id',id,'attempt_no',attempt_no,'outcome',outcome,'started_at',started_at,'finished_at',finished_at) ORDER BY attempt_no) INTO attempts FROM public.job_attempts WHERE job_id=j.id;
        IF COALESCE(jsonb_array_length(attempts),0) IS DISTINCT FROM j.attempt_count OR j.attempt_count NOT BETWEEN 0 AND 3 THEN
            RAISE EXCEPTION 'paper evidence attempts incomplete' USING ERRCODE='23514'; END IF;
        terminal:=j.status;
        IF terminal='SUCCEEDED' AND (r.status IS DISTINCT FROM 'SUCCEEDED' OR attempt.outcome IS DISTINCT FROM 'SUCCEEDED'
           OR ROW(claim.run_id,claim.job_id,claim.owner_user_id,claim.attempt_id,claim.attempt_no,claim.claimed_by)
              IS DISTINCT FROM ROW(r.id,j.id,c.owner_user_id,attempt.id,j.attempt_count,attempt.claimed_by)
           OR r.finished_at IS DISTINCT FROM claim.authorized_at OR j.finished_at IS NULL OR attempt.finished_at IS NULL
           OR j.finished_at<claim.authorized_at OR attempt.finished_at<j.finished_at
           OR r.factor_snapshot_sha256 IS NULL OR r.target_snapshot_sha256 IS NULL
           OR r.finished_at >= ((s.session_date+1)::timestamp AT TIME ZONE 'Asia/Seoul')
           OR j.finished_at >= ((s.session_date+1)::timestamp AT TIME ZONE 'Asia/Seoul')
           OR attempt.finished_at >= ((s.session_date+1)::timestamp AT TIME ZONE 'Asia/Seoul')
           OR EXISTS(SELECT 1 FROM public.job_attempts x WHERE x.job_id=j.id AND x.attempt_no<j.attempt_count AND x.outcome NOT IN ('FAILED','ORPHANED')))
        THEN RAISE EXCEPTION 'paper evidence terminal mismatch' USING ERRCODE='23514'; END IF;
        IF rr.run_id IS NOT NULL AND (ROW(rr.run_id,rr.job_id,rr.attempt_id) IS DISTINCT FROM ROW(r.id,j.id,attempt.id) OR terminal<>'SUCCEEDED') THEN
            RAISE EXCEPTION 'paper evidence linked terminal changed' USING ERRCODE='23514'; END IF;
    END IF;
    result:=jsonb_build_object('schema','owner-beta-paper-db-v1','slot',to_jsonb(s),
       'admission',CASE WHEN link.slot_id IS NULL THEN NULL ELSE to_jsonb(link) END,
       'run',CASE WHEN rr.slot_id IS NULL THEN NULL ELSE to_jsonb(rr) END,
       'candidate_admission_sha256',a.admission_sha256,'candidate_run_id',res.run_id,
       'terminal',terminal,'job_id',j.id,'attempt_id',attempt.id,'attempts',attempts,
       'factor_snapshot_sha256',r.factor_snapshot_sha256,'target_snapshot_sha256',r.target_snapshot_sha256,
       'candidate_pins',CASE WHEN a.admission_sha256 IS NULL THEN NULL ELSE jsonb_build_object('content_sha256',a.content_sha256,'manifest_sha256',a.manifest_sha256,'admission_sha256',a.admission_sha256,'policy_sha256',a.policy_sha256) END,
       'pins',r.daily_pins,'admitted_at',a.admitted_at,'finished_at',attempt.finished_at);
    selection:=jsonb_build_object('target_id',target.id,'grant_id',g.grant_id,'lineage_id',lineage.id,
      'run_id',lineage.recommendation_run_id,'signal_date',lineage.signal_date,'effective_session',lineage.effective_session,
      'signal_cohort',ts.cohort,'execution_cohort',es.cohort,'decision',g.decision,
      'active_epoch',pc.control_epoch,'active_process_sha256',pc.process_binding_sha256,'active_consent_id',pc.consent_event_id,
      'action',p_action);
    result:=jsonb_build_object('selection',selection,'readback',result);
    IF octet_length(public.paper_0068_canonical_json(result))>262144 OR public.paper_0068_json_depth(result)>16 THEN
      RAISE EXCEPTION 'worker recovery read bound' USING ERRCODE='23514';
    END IF;
    PERFORM public.paper_0068_restore_actor(old_actor);
    RETURN result;
EXCEPTION WHEN query_canceled OR assert_failure THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
WHEN OTHERS THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
END $$;
ALTER FUNCTION public.paper_0068_read_worker_recovery(uuid,uuid,bigint,text,text,date,text) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.paper_0068_read_worker_recovery(uuid,uuid,bigint,text,text,date,text) FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter;
GRANT EXECUTE ON FUNCTION public.paper_0068_read_worker_recovery(uuid,uuid,bigint,text,text,date,text) TO worker;

-- Patch only the three existing serving seams that must consume a v2 grant.
-- Every anchor is checked so a future ABI change fails the migration closed.
DO $$ DECLARE definition text; BEGIN
  definition:=pg_get_functiondef('public.snapshot_owner_beta_paper_target_v1(uuid,bigint,text)'::regprocedure);
  IF position('e_admission text; d public.owner_beta_daily_control;' IN definition)=0 OR position('IF NOT FOUND OR l.control_epoch<>c.control_epoch OR l.consent_event_id<>c.consent_event_id THEN RAISE EXCEPTION ''target lineage stale''' IN definition)=0 OR position('   jsonb_build_object(''pending_target_id''' IN definition)=0 OR position('''receipt_json'',s.receipt_json));' IN definition)=0 THEN RAISE EXCEPTION '0067 target snapshot source seam changed' USING ERRCODE='55000'; END IF;
  definition:=replace(definition,'e_admission text; d public.owner_beta_daily_control;','e_admission text; d public.owner_beta_daily_control; recovery_authority jsonb;');
  definition:=replace(definition,'IF NOT FOUND OR l.control_epoch<>c.control_epoch OR l.consent_event_id<>c.consent_event_id THEN RAISE EXCEPTION ''target lineage stale'' USING ERRCODE=''40001''; END IF;','IF NOT FOUND THEN RAISE EXCEPTION ''target lineage stale'' USING ERRCODE=''40001''; END IF; IF l.control_epoch<>c.control_epoch OR l.consent_event_id<>c.consent_event_id THEN recovery_authority:=public.paper_0068_recovery_authority(t.id,expected_epoch,process_config_sha256,CASE WHEN s.settlement_sha256 IS NULL THEN ''SETTLE'' ELSE ''VALUE'' END); IF recovery_authority IS NULL THEN PERFORM public.paper_0068_restore_actor(old_actor); RETURN public.paper_0067_action(''SNAPSHOT_TARGET'',''RECOVERY_AUTHORITY_UNAVAILABLE'',c.control_epoch,c.account_id,jsonb_build_object(''pending_target_id'',t.id,''reason_code'',''RECOVERY_AUTHORITY_UNAVAILABLE'')); END IF; END IF;');
  definition:=replace(definition,'   jsonb_build_object(''pending_target_id''','   (jsonb_build_object(''pending_target_id''');
  definition:=replace(definition,'''receipt_json'',s.receipt_json));','''receipt_json'',s.receipt_json)||CASE WHEN recovery_authority IS NULL THEN ''{}''::jsonb ELSE jsonb_build_object(''recovery_authority'',recovery_authority) END));');
  definition:=replace(definition,'PERFORM set_config(''app.actor_user_id'',COALESCE(old_actor,''''),true);','PERFORM public.paper_0068_restore_actor(old_actor);');
  IF position('recovery_authority:=public.paper_0068_recovery_authority' IN definition)=0 OR position('''recovery_authority'',recovery_authority' IN definition)=0 THEN RAISE EXCEPTION '0067 target snapshot recovery hook failed to apply' USING ERRCODE='55000'; END IF;
  EXECUTE definition;
END $$;

DO $$ DECLARE definition text; BEGIN
  definition:=pg_get_functiondef('public.commit_owner_beta_paper_settlement_v1(uuid,bigint,jsonb,text)'::regprocedure);
  IF position('recovery boolean:=false;' IN definition)=0 OR position('SELECT * INTO l FROM public.owner_beta_paper_lineage WHERE id=t.beta_lineage_id AND account_id=c.account_id AND owner_user_id=c.owner_user_id FOR SHARE;' IN definition)=0 OR position('(settlement->>''control_epoch'')::bigint<>c.control_epoch' IN definition)=0 OR position('recovery:=EXISTS(SELECT 1 FROM public.owner_beta_paper_events' IN definition)=0 THEN RAISE EXCEPTION '0067 settlement source seam changed' USING ERRCODE='55000'; END IF;
  definition:=replace(definition,'recovery boolean:=false; old_actor text:=current_setting(''app.actor_user_id'',true);','recovery boolean:=false; recovery_grant_id uuid; recovery_bound_admission text; recovery_authority jsonb; old_actor text:=current_setting(''app.actor_user_id'',true);');
  definition:=replace(definition,'IF NOT FOUND OR settlement->>''lineage_id''<>l.id::text OR (settlement->>''control_epoch'')::bigint<>c.control_epoch',
    'IF NOT FOUND THEN RAISE EXCEPTION ''settlement lineage unavailable'' USING ERRCODE=''23514''; END IF; IF EXISTS(SELECT 1 FROM public.owner_beta_paper_recovery_grants g WHERE g.pending_target_id=t.id AND g.valuation_receipt_sha256 IS NULL) THEN recovery:=true; recovery_authority:=public.paper_0068_recovery_authority(t.id,expected_epoch,process_config_sha256,''SETTLE''); IF recovery_authority IS NULL THEN PERFORM public.paper_0068_restore_actor(old_actor); RETURN public.paper_0067_action(''COMMIT_SETTLEMENT'',''RECOVERY_AUTHORITY_UNAVAILABLE'',c.control_epoch,c.account_id,jsonb_build_object(''pending_target_id'',t.id,''reason_code'',''RECOVERY_AUTHORITY_UNAVAILABLE'')); END IF; recovery_grant_id:=(recovery_authority->>''grant_id'')::uuid; SELECT execution_admission_sha256 INTO recovery_bound_admission FROM public.owner_beta_paper_recovery_grants WHERE grant_id=recovery_grant_id; IF settlement->>''execution_admission_sha256'' IS DISTINCT FROM recovery_bound_admission THEN RAISE EXCEPTION ''recovery execution admission differs from grant'' USING ERRCODE=''23514''; END IF; END IF; IF settlement->>''lineage_id''<>l.id::text OR (settlement->>''control_epoch'')::bigint<>(CASE WHEN recovery THEN l.control_epoch ELSE c.control_epoch END)');
  definition:=replace(definition,'recovery:=EXISTS(SELECT 1 FROM public.owner_beta_paper_events','recovery:=recovery OR EXISTS(SELECT 1 FROM public.owner_beta_paper_events');
  definition:=replace(definition,'  PERFORM set_config(''app.actor_user_id'',COALESCE(old_actor,''''),true); RETURN public.paper_0067_action(''COMMIT_SETTLEMENT'',''OK''',
    '  IF recovery_grant_id IS NOT NULL THEN UPDATE public.owner_beta_paper_recovery_grants SET settlement_receipt_sha256=settlement->>''calculation_sha256'' WHERE grant_id=recovery_grant_id AND settlement_receipt_sha256 IS NULL AND valuation_receipt_sha256 IS NULL; IF NOT FOUND THEN RAISE EXCEPTION ''recovery settlement step already consumed'' USING ERRCODE=''55000''; END IF; END IF; PERFORM public.paper_0068_restore_actor(old_actor); RETURN public.paper_0067_action(''COMMIT_SETTLEMENT'',''OK''');
  definition:=replace(definition,'PERFORM set_config(''app.actor_user_id'',COALESCE(old_actor,''''),true);','PERFORM public.paper_0068_restore_actor(old_actor);');
  IF position('recovery_authority:=public.paper_0068_recovery_authority' IN definition)=0 OR position('settlement_receipt_sha256=settlement->>''calculation_sha256''' IN definition)=0 THEN RAISE EXCEPTION '0067 settlement recovery hook failed to apply' USING ERRCODE='55000'; END IF;
  EXECUTE definition;
END $$;

DO $$ DECLARE definition text; BEGIN
  definition:=pg_get_functiondef('public.commit_owner_beta_paper_valuation_v1(uuid,bigint,jsonb,text)'::regprocedure);
  IF position('event_seq bigint; inserted public.daily_equity; old_actor' IN definition)=0 OR position('IF s.state NOT IN (''SETTLED_TRADE'',''SETTLED_NO_TRADE'') OR l.id IS NULL OR (valuation->>''control_epoch'')::bigint<>c.control_epoch' IN definition)=0 OR position('INSERT INTO public.owner_beta_paper_session_events(pending_target_id,event_seq,kind,payload,occurred_at) VALUES(t.id,event_seq,''VALUED''' IN definition)=0 THEN RAISE EXCEPTION '0067 valuation source seam changed' USING ERRCODE='55000'; END IF;
  definition:=replace(definition,'event_seq bigint; inserted public.daily_equity; old_actor text:=current_setting(''app.actor_user_id'',true);','event_seq bigint; inserted public.daily_equity; recovery_grant_id uuid; recovery_bound_admission text; recovery_authority jsonb; recovery boolean:=false; old_actor text:=current_setting(''app.actor_user_id'',true);');
  definition:=replace(definition,'IF s.state NOT IN (''SETTLED_TRADE'',''SETTLED_NO_TRADE'') OR l.id IS NULL OR (valuation->>''control_epoch'')::bigint<>c.control_epoch',
    'IF l.id IS NULL THEN RAISE EXCEPTION ''valuation lineage unavailable'' USING ERRCODE=''23514''; END IF; IF EXISTS(SELECT 1 FROM public.owner_beta_paper_recovery_grants g WHERE g.pending_target_id=t.id AND g.valuation_receipt_sha256 IS NULL) THEN recovery:=true; recovery_authority:=public.paper_0068_recovery_authority(t.id,expected_epoch,process_config_sha256,''VALUE''); IF recovery_authority IS NULL THEN PERFORM public.paper_0068_restore_actor(old_actor); RETURN public.paper_0067_action(''COMMIT_VALUATION'',''RECOVERY_AUTHORITY_UNAVAILABLE'',c.control_epoch,c.account_id,jsonb_build_object(''pending_target_id'',t.id,''reason_code'',''RECOVERY_AUTHORITY_UNAVAILABLE'')); END IF; recovery_grant_id:=(recovery_authority->>''grant_id'')::uuid; SELECT execution_admission_sha256 INTO recovery_bound_admission FROM public.owner_beta_paper_recovery_grants WHERE grant_id=recovery_grant_id; END IF; IF s.state NOT IN (''SETTLED_TRADE'',''SETTLED_NO_TRADE'') OR (valuation->>''control_epoch'')::bigint<>(CASE WHEN recovery THEN l.control_epoch ELSE c.control_epoch END)');
  definition:=replace(definition,'IF NOT public.daily_0060_admission_usable(d,valuation->>''valuation_admission_sha256'',false) OR NOT EXISTS(',
    'IF recovery AND valuation->>''valuation_admission_sha256'' IS DISTINCT FROM recovery_bound_admission OR NOT public.daily_0060_admission_usable(d,valuation->>''valuation_admission_sha256'',false) OR NOT EXISTS(');
  definition:=replace(definition,'  PERFORM set_config(''app.actor_user_id'',COALESCE(old_actor,''''),true); RETURN public.paper_0067_action(''COMMIT_VALUATION'',''OK''',
    '  IF recovery_grant_id IS NOT NULL THEN UPDATE public.owner_beta_paper_recovery_grants SET valuation_receipt_sha256=valuation->>''calculation_sha256'' WHERE grant_id=recovery_grant_id AND valuation_receipt_sha256 IS NULL AND (step=''VALUE_ONLY'' OR settlement_receipt_sha256 IS NOT NULL); IF NOT FOUND THEN RAISE EXCEPTION ''recovery valuation step already consumed'' USING ERRCODE=''55000''; END IF; END IF; PERFORM public.paper_0068_restore_actor(old_actor); RETURN public.paper_0067_action(''COMMIT_VALUATION'',''OK''');
  definition:=replace(definition,'PERFORM set_config(''app.actor_user_id'',COALESCE(old_actor,''''),true);','PERFORM public.paper_0068_restore_actor(old_actor);');
  IF position('recovery_authority:=public.paper_0068_recovery_authority' IN definition)=0 OR position('valuation_receipt_sha256=valuation->>''calculation_sha256''' IN definition)=0 THEN RAISE EXCEPTION '0067 valuation recovery hook failed to apply' USING ERRCODE='55000'; END IF;
  EXECUTE definition;
END $$;

DO $$ DECLARE f text; BEGIN
  FOREACH f IN ARRAY ARRAY['begin_owner_beta_paper_operation_v1(bigint,jsonb,text)','inspect_owner_beta_paper_operation_v1(uuid,uuid,integer)','reconcile_owner_beta_paper_operation_v1(uuid,bigint,text,text)','read_owner_beta_paper_operation_result_v1(text,text)','configure_owner_beta_paper_v1(uuid,text,jsonb,jsonb,text)'] LOOP
    EXECUTE format('ALTER FUNCTION public.%s OWNER TO migration_owner',f);
    EXECUTE format('ALTER FUNCTION public.%s SET search_path=pg_catalog,pg_temp',f);
    EXECUTE format('ALTER FUNCTION public.%s SET lock_timeout=''5s''',f);
    EXECUTE format('ALTER FUNCTION public.%s SET statement_timeout=''15s''',f);
    EXECUTE format('REVOKE ALL ON FUNCTION public.%s FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter',f);
    EXECUTE format('GRANT EXECUTE ON FUNCTION public.%s TO migration_owner',f);
  END LOOP;
  FOREACH f IN ARRAY ARRAY['transition_owner_beta_paper_v1(bigint,text,jsonb,text)'] LOOP
    EXECUTE format('ALTER FUNCTION public.%s OWNER TO migration_owner',f);
    EXECUTE format('ALTER FUNCTION public.%s SET search_path=pg_catalog,pg_temp',f);
    EXECUTE format('ALTER FUNCTION public.%s SET lock_timeout=''5s''',f);
    EXECUTE format('ALTER FUNCTION public.%s SET statement_timeout=''15s''',f);
    EXECUTE format('REVOKE ALL ON FUNCTION public.%s FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter',f);
    EXECUTE format('GRANT EXECUTE ON FUNCTION public.%s TO migration_owner',f);
  END LOOP;
  FOREACH f IN ARRAY ARRAY['paper_0068_configure_legacy(uuid,text,jsonb,jsonb,text)','paper_0068_json_depth(jsonb)','paper_0068_parse_canonical(text,integer)','paper_0068_uuid(jsonb)','paper_0068_request(text,uuid,jsonb)','paper_0068_exact_request(text,text)','paper_0068_validate_receipt(jsonb,text)','paper_0068_receipt_guard()','paper_0068_reset(public.owner_beta_paper_control)','paper_0068_transition_legacy(bigint,text,jsonb,text)','paper_0068_principal()','paper_0068_nonnegative_i64(text)','paper_0068_restore_actor(text)','paper_0068_canonical_json(jsonb)','paper_0068_canonical_row_json(jsonb)','paper_0068_stream_digest(jsonb)','paper_0068_utc(timestamptz)','paper_0068_clock_fresh(jsonb)','paper_0068_immutable()','paper_0068_guard_grant()','paper_0068_recovery_authority(uuid,bigint,text,text)','paper_0068_gate_new_lineage_work()'] LOOP
    EXECUTE format('ALTER FUNCTION public.%s OWNER TO migration_owner',f);
    EXECUTE format('REVOKE ALL ON FUNCTION public.%s FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter',f);
  END LOOP;
END $$;
