-- R5 WP5 / P1: protected lifecycle journals and offline SQL readback.
-- 0067 remains immutable; its serving ABI is renamed privately below and
-- wrapped with the same public signature/grant boundary.
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
     OR (NEW.valuation_receipt_sha256 IS NOT NULL AND OLD.valuation_receipt_sha256 IS NULL)
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
  ELSIF typ IN ('true','false','null') THEN RETURN typ;
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
  ELSIF typ IN ('true','false','null') THEN
    RETURN typ;
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

ALTER FUNCTION public.transition_owner_beta_paper_v1(bigint,text,jsonb,text) RENAME TO paper_0068_transition_legacy;
REVOKE ALL ON FUNCTION public.paper_0068_transition_legacy(bigint,text,jsonb,text) FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter;
GRANT EXECUTE ON FUNCTION public.paper_0068_transition_legacy(bigint,text,jsonb,text) TO migration_owner;

CREATE FUNCTION public.begin_owner_beta_paper_operation_v1(expected_epoch bigint,intent jsonb,request_key text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE old_actor text:=current_setting('app.actor_user_id',true); c public.owner_beta_paper_control; prior public.owner_beta_paper_operations; latest_gen uuid; ev uuid; gen uuid; installation uuid; acct uuid; fence bigint; ihash text; replay jsonb;
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
  ihash:=public.daily_0060_digest(public.paper_0068_canonical_json(intent));
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE account_id=acct FOR UPDATE;
  IF NOT FOUND THEN RAISE EXCEPTION 'configured Owner-beta Paper account is missing' USING ERRCODE='55000'; END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  PERFORM pg_advisory_xact_lock(hashtextextended(c.account_id::text,381901));
  SELECT * INTO prior FROM public.owner_beta_paper_operations op WHERE op.generation=gen FOR SHARE;
  IF FOUND THEN
    IF prior.generation<>gen OR prior.account_id<>acct OR prior.intent_sha256<>ihash OR prior.intent IS DISTINCT FROM intent THEN RAISE EXCEPTION 'operation identity replay conflicts' USING ERRCODE='23505'; END IF;
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
  INSERT INTO public.owner_beta_paper_operations(generation,installation_id,account_id,request_key,purpose,scope,expected_epoch,local_epoch_floor,fence_epoch,release_manifest_sha256,intent,intent_sha256,prior_state,fence_event_id)
    VALUES(gen,installation,acct,request_key,intent->>'purpose',intent->>'scope',expected_epoch,(intent->>'local_epoch_floor')::bigint,fence,intent->>'release_manifest_sha256',intent,ihash,c.state,ev);
  UPDATE public.owner_beta_paper_control SET state='DRAINING',control_epoch=fence,updated_at=clock_timestamp() WHERE id='etf11';
  PERFORM public.paper_0068_restore_actor(old_actor);
  RETURN public.paper_0067_action('BEGIN_OPERATION','OK',fence,c.account_id,jsonb_build_object('generation',gen,'operation_event_id',ev,'intent_sha256',ihash,'fenced_epoch',fence));
EXCEPTION WHEN OTHERS THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
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
DECLARE old_actor text:=current_setting('app.actor_user_id',true); c public.owner_beta_paper_control; o public.owner_beta_paper_operations; prior public.owner_beta_paper_operation_observations; obs uuid; summary jsonb; dbhash text; ledgerhash text; pendinghash text; outboxhash text; jobshash text; unresolved bigint; claims bigint; held bigint;
BEGIN
  PERFORM public.paper_0068_principal();
  IF generation IS NULL OR inventory_sha256 IS NULL OR inventory_sha256 !~ '^sha256:[0-9a-f]{64}$' OR request_key IS NULL OR request_key !~ ('^'||generation::text||':reconcile:[0-9]{1,18}$') THEN RAISE EXCEPTION 'reconciliation identity invalid' USING ERRCODE='22023'; END IF;
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN RAISE EXCEPTION 'configured Paper account missing' USING ERRCODE='55000'; END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  PERFORM pg_advisory_xact_lock(hashtextextended(c.account_id::text,381901));
  SELECT * INTO o FROM public.owner_beta_paper_operations WHERE owner_beta_paper_operations.generation=reconcile_owner_beta_paper_operation_v1.generation AND account_id=c.account_id FOR SHARE;
  IF NOT FOUND OR c.state<>'DRAINING' OR expected_epoch IS DISTINCT FROM c.control_epoch OR o.fence_epoch<>c.control_epoch THEN RAISE EXCEPTION 'operation is not at expected durable fence' USING ERRCODE='40001'; END IF;
  SELECT * INTO prior FROM public.owner_beta_paper_operation_observations WHERE generation=o.generation AND owner_beta_paper_operation_observations.request_key=reconcile_owner_beta_paper_operation_v1.request_key;
  IF FOUND THEN
    IF prior.summary->>'inventory_sha256'<>inventory_sha256 THEN RAISE EXCEPTION 'reconciliation request replay conflicts' USING ERRCODE='23505'; END IF;
    PERFORM public.paper_0068_restore_actor(old_actor);
    RETURN public.paper_0067_action('RECONCILE_OPERATION','REPLAY',c.control_epoch,c.account_id,jsonb_build_object('observation_id',prior.observation_id,'summary',prior.summary,'summary_sha256',prior.summary_sha256));
  END IF;
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
        'payload',e.payload,'payload_sha256',e.payload_sha256,'request_key',e.request_key,'pending_target_id',e.pending_target_id)
      FROM public.owner_beta_paper_events e WHERE e.account_id=c.account_id AND e.owner_user_id=c.owner_user_id
      UNION ALL
      SELECT 'pending',c.account_id::text,to_jsonb(pendinghash)
    ) q(kind,row_key,row_value)
  ),'[]'::jsonb));
  obs:=gen_random_uuid();
  summary:=jsonb_build_object('schema','owner-beta-paper-reconciliation-v1','installation_id',o.installation_id,'generation',o.generation,'account_id',o.account_id,'observation_id',obs,'control_epoch',c.control_epoch,'inventory_sha256',inventory_sha256,'observed_at',to_char(clock_timestamp() AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS"Z"'),'database_sha256',dbhash,'ledger_sha256',ledgerhash,'pending_sha256',pendinghash,'outbox_sha256',outboxhash,'jobs_sha256',jobshash,'unresolved_commits',unresolved,'live_claims',claims,'held_targets',held,'quarantined_deliveries',(SELECT count(*) FROM public.notification_deliveries d JOIN public.notifications n ON n.id=d.notification_id AND n.owner_user_id=d.owner_user_id WHERE d.owner_user_id=c.owner_user_id AND d.delivery_token IS NOT NULL AND EXISTS(SELECT 1 FROM public.paper_settlement_outbox x WHERE x.owner_user_id=n.owner_user_id AND n.source_key=x.id::text AND EXISTS(SELECT 1 FROM public.pending_targets t WHERE t.id=x.pending_target_id AND t.account_id=c.account_id) UNION ALL SELECT 1 FROM public.paper_settlement_outbox_archive a WHERE a.owner_user_id=n.owner_user_id AND n.source_key=a.id::text AND EXISTS(SELECT 1 FROM public.pending_targets t WHERE t.id=a.pending_target_id AND t.account_id=c.account_id))));
  INSERT INTO public.owner_beta_paper_operation_observations(observation_id,generation,request_key,summary,summary_sha256)
    VALUES(obs,o.generation,request_key,summary,public.daily_0060_digest(public.paper_0068_canonical_json(summary)));
  PERFORM public.paper_0068_restore_actor(old_actor);
  RETURN public.paper_0067_action('RECONCILE_OPERATION','OK',c.control_epoch,c.account_id,jsonb_build_object('observation_id',obs,'summary',summary,'summary_sha256',public.daily_0060_digest(public.paper_0068_canonical_json(summary))));
EXCEPTION WHEN OTHERS THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
END $$;

CREATE FUNCTION public.paper_0068_recovery_authority(p_target uuid,p_epoch bigint,p_process_sha256 text,p_action text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE c public.owner_beta_paper_control; s public.owner_beta_paper_sessions; g public.owner_beta_paper_recovery_grants; t public.pending_targets; l public.owner_beta_paper_lineage; d public.owner_beta_daily_control; grant_found boolean; old_actor text:=current_setting('app.actor_user_id',true);
BEGIN
  -- This helper is intentionally usable only from a serving SECURITY DEFINER
  -- call or migration-owner recovery operation. The caller's public ABI still
  -- enforces app/worker and current process authority.
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11';
  IF NOT FOUND THEN RAISE EXCEPTION 'recovery authority unavailable' USING ERRCODE='55000'; END IF;
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
  IF g.account_id<>c.account_id OR g.expires_at<=clock_timestamp() OR c.state<>'ENABLED'
     OR c.control_epoch<>p_epoch OR c.consent_event_id IS DISTINCT FROM g.consent_event_id
     OR c.process_binding_sha256 IS DISTINCT FROM g.process_config_sha256 OR p_process_sha256 IS DISTINCT FROM g.process_config_sha256
     OR t.id IS NULL OR l.id IS NULL OR g.lineage_id IS DISTINCT FROM t.beta_lineage_id
     OR g.lineage_epoch IS DISTINCT FROM l.control_epoch OR g.effective_session IS DISTINCT FROM s.effective_session
     OR l.strategy_config_id IS DISTINCT FROM c.strategy_config_id
     OR g.signal_admission_sha256 IS DISTINCT FROM l.signal_admission_sha256
     OR g.original_receipt_sha256 IS DISTINCT FROM (CASE WHEN s.settlement_sha256 IS NULL
          THEN public.daily_0060_digest(public.paper_0068_canonical_json(COALESCE(s.receipt_json,'null'::jsonb)))
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
     OR p_action NOT IN ('SETTLE','VALUE')
     OR EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions earlier WHERE earlier.account_id=c.account_id AND earlier.effective_session<s.effective_session AND (earlier.state<>'VALUED' OR earlier.valuation_sha256 IS NULL))
  THEN
    UPDATE public.owner_beta_paper_recovery_grants SET revoked_at=clock_timestamp()
      WHERE grant_id=g.grant_id AND revoked_at IS NULL;
    PERFORM public.paper_0068_restore_actor(old_actor);
    RETURN NULL;
  END IF;
  PERFORM public.paper_0068_restore_actor(old_actor);
  RETURN jsonb_build_object('grant_id',g.grant_id,'active_epoch',g.active_epoch,'active_process_sha256',g.process_config_sha256,'active_consent_id',g.consent_event_id,'step',g.step,'expires_at',to_char(g.expires_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS"Z"'));
EXCEPTION WHEN OTHERS THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
END $$;

CREATE FUNCTION public.transition_owner_beta_paper_v1(expected_epoch bigint,transition text,evidence jsonb,request_key text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE old_actor text:=current_setting('app.actor_user_id',true); c public.owner_beta_paper_control; saved public.owner_beta_paper_events; o public.owner_beta_paper_operations; source_o public.owner_beta_paper_operations; ob public.owner_beta_paper_operation_observations; s public.owner_beta_paper_sessions; l public.owner_beta_paper_lineage; d public.owner_beta_daily_control; t public.pending_targets; ev uuid; next_epoch bigint; gen uuid; pid uuid; step_name text; decision_hash text; original_hash text; expected_expiry timestamptz; execution_admission text; result_state text; replay jsonb;
BEGIN
  PERFORM public.paper_0068_principal();
  IF transition IS DISTINCT FROM 'OFF' AND transition IS DISTINCT FROM 'RESTORED_OFF'
     AND NOT (transition='RECOVERY' AND evidence->>'schema'='owner-beta-paper-recovery-v2') THEN
    replay:=public.paper_0068_transition_legacy(expected_epoch,transition,evidence,request_key);
    PERFORM public.paper_0068_restore_actor(old_actor);
    RETURN replay;
  END IF;
  IF request_key IS NULL OR request_key !~ '^[A-Za-z0-9._:-]{1,128}$' OR evidence IS NULL OR jsonb_typeof(evidence)<>'object' OR octet_length(evidence::text)>65536 THEN RAISE EXCEPTION 'lifecycle receipt invalid' USING ERRCODE='22023'; END IF;
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN RETURN public.paper_0067_action('TRANSITION','NOT_FOUND',NULL,NULL); END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  PERFORM pg_advisory_xact_lock(hashtextextended(c.account_id::text,381901));
  SELECT * INTO saved FROM public.owner_beta_paper_events WHERE account_id=c.account_id AND owner_beta_paper_events.request_key=transition_owner_beta_paper_v1.request_key FOR SHARE;
  IF FOUND THEN
    IF saved.kind<>(CASE transition WHEN 'OFF' THEN 'OFF' WHEN 'RESTORED_OFF' THEN 'RESTORED_OFF' ELSE 'RECOVERY_AUTHORIZED' END) OR saved.payload IS DISTINCT FROM evidence THEN RAISE EXCEPTION 'transition request key conflicts with prior event' USING ERRCODE='23505'; END IF;
    PERFORM public.paper_0068_restore_actor(old_actor);
    replay:=jsonb_build_object('event_id',saved.id,'state',c.state,'pending_target_id',saved.pending_target_id);
    IF transition='RECOVERY' AND saved.payload->>'schema'='owner-beta-paper-recovery-v2' THEN
      replay:=replay||jsonb_build_object('grant_id',(SELECT g.grant_id FROM public.owner_beta_paper_recovery_grants g WHERE g.account_id=c.account_id AND g.request_key=transition_owner_beta_paper_v1.request_key));
    END IF;
    RETURN public.paper_0067_action('TRANSITION','REPLAY',c.control_epoch,c.account_id,replay);
  END IF;
  IF expected_epoch IS DISTINCT FROM c.control_epoch THEN RAISE EXCEPTION 'transition control epoch is stale' USING ERRCODE='40001'; END IF;

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
    INSERT INTO public.owner_beta_paper_events(id,owner_user_id,account_id,control_epoch,kind,payload,payload_sha256,request_key)
      VALUES(ev,c.owner_user_id,c.account_id,next_epoch,transition,evidence,public.daily_0060_digest(public.paper_0068_canonical_json(evidence)),request_key);
    UPDATE public.owner_beta_paper_control SET state=transition,control_epoch=next_epoch,process_binding=NULL,process_binding_sha256=NULL,cohort_sha256=NULL,daily_cohort=NULL,activation_event_id=NULL,updated_at=clock_timestamp() WHERE id='etf11';
    UPDATE public.owner_beta_paper_recovery_grants SET revoked_at=clock_timestamp() WHERE account_id=c.account_id AND revoked_at IS NULL AND valuation_receipt_sha256 IS NULL;
    PERFORM public.paper_0068_restore_actor(old_actor);
    RETURN public.paper_0067_action('TRANSITION','OK',next_epoch,c.account_id,jsonb_build_object('event_id',ev,'state',transition));
  END IF;

  IF evidence->>'schema'<>'owner-beta-paper-recovery-v2' THEN
    PERFORM public.paper_0068_restore_actor(old_actor);
    RETURN public.paper_0068_transition_legacy(expected_epoch,transition,evidence,request_key);
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
  THEN RAISE EXCEPTION 'recovery current control authority mismatch' USING ERRCODE='42501'; END IF;
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
  THEN RAISE EXCEPTION 'recovery current rights, consent or exact E admission unavailable' USING ERRCODE='42501'; END IF;
  UPDATE public.owner_beta_paper_recovery_grants SET revoked_at=clock_timestamp() WHERE pending_target_id=pid AND active_epoch=c.control_epoch AND revoked_at IS NULL AND valuation_receipt_sha256 IS NULL AND expires_at<=clock_timestamp();
  IF EXISTS(SELECT 1 FROM public.owner_beta_paper_recovery_grants g WHERE g.pending_target_id=pid AND g.active_epoch=c.control_epoch AND g.revoked_at IS NULL AND g.valuation_receipt_sha256 IS NULL) THEN RAISE EXCEPTION 'active recovery grant already exists for target and epoch' USING ERRCODE='23505'; END IF;
  INSERT INTO public.owner_beta_paper_recovery_grants(grant_id,account_id,pending_target_id,lineage_id,effective_session,original_receipt_sha256,lineage_epoch,active_epoch,consent_event_id,process_config_sha256,decision,decision_sha256,step,signal_admission_sha256,execution_admission_sha256,expires_at,request_key)
    VALUES(gen_random_uuid(),c.account_id,pid,l.id,s.effective_session,original_hash,l.control_epoch,c.control_epoch,c.consent_event_id,c.process_binding_sha256,evidence,decision_hash,evidence->>'step',l.signal_admission_sha256,execution_admission,(evidence->>'expires_at')::timestamptz,request_key);
  ev:=gen_random_uuid();
  INSERT INTO public.owner_beta_paper_events(id,owner_user_id,account_id,control_epoch,kind,payload,payload_sha256,request_key,pending_target_id)
    VALUES(ev,c.owner_user_id,c.account_id,c.control_epoch,'RECOVERY_AUTHORIZED',evidence,public.daily_0060_digest(public.paper_0068_canonical_json(evidence)),request_key,pid);
  IF s.settlement_sha256 IS NULL THEN
    UPDATE public.owner_beta_paper_sessions SET processing_origin='RECOVERY',state='RECOVERY_REQUIRED',reason_code='RECOVERY_REQUIRED',updated_at=clock_timestamp() WHERE pending_target_id=pid;
  END IF;
  INSERT INTO public.owner_beta_paper_session_events(pending_target_id,event_seq,kind,reason_code,payload)
    SELECT pid,COALESCE(max(event_seq),0)+1,'RECOVERY_AUTHORIZED','RECOVERY_REQUIRED',evidence FROM public.owner_beta_paper_session_events WHERE pending_target_id=pid;
  PERFORM public.paper_0068_restore_actor(old_actor);
  RETURN public.paper_0067_action('TRANSITION','OK',c.control_epoch,c.account_id,jsonb_build_object('event_id',ev,'state',c.state,'pending_target_id',pid,'grant_id',(SELECT grant_id FROM public.owner_beta_paper_recovery_grants WHERE account_id=c.account_id AND request_key=transition_owner_beta_paper_v1.request_key)));
EXCEPTION WHEN OTHERS THEN PERFORM public.paper_0068_restore_actor(old_actor); RAISE;
END $$;

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
    'IF NOT FOUND THEN RAISE EXCEPTION ''settlement lineage unavailable'' USING ERRCODE=''23514''; END IF; IF EXISTS(SELECT 1 FROM public.owner_beta_paper_recovery_grants g WHERE g.pending_target_id=t.id AND g.valuation_receipt_sha256 IS NULL) THEN recovery:=true; recovery_authority:=public.paper_0068_recovery_authority(t.id,expected_epoch,process_config_sha256,''SETTLE''); IF recovery_authority IS NULL THEN PERFORM public.paper_0068_restore_actor(old_actor); RETURN public.paper_0067_action(''COMMIT_SETTLEMENT'',''RECOVERY_AUTHORITY_UNAVAILABLE'',c.control_epoch,c.account_id,jsonb_build_object(''pending_target_id'',t.id,''reason_code'',''RECOVERY_AUTHORITY_UNAVAILABLE'')); END IF; recovery_grant_id:=(recovery_authority->>''grant_id'')::uuid; SELECT execution_admission_sha256 INTO recovery_bound_admission FROM public.owner_beta_paper_recovery_grants WHERE grant_id=recovery_grant_id; IF settlement->>''execution_admission_sha256'' IS DISTINCT FROM recovery_bound_admission THEN RAISE EXCEPTION ''recovery execution admission differs from grant'' USING ERRCODE=''23514''; END IF; END IF; IF settlement->>''lineage_id''<>l.id::text OR (settlement->>''control_epoch'')::bigint<>CASE WHEN recovery THEN l.control_epoch ELSE c.control_epoch END');
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
    'IF l.id IS NULL THEN RAISE EXCEPTION ''valuation lineage unavailable'' USING ERRCODE=''23514''; END IF; IF EXISTS(SELECT 1 FROM public.owner_beta_paper_recovery_grants g WHERE g.pending_target_id=t.id AND g.valuation_receipt_sha256 IS NULL) THEN recovery:=true; recovery_authority:=public.paper_0068_recovery_authority(t.id,expected_epoch,process_config_sha256,''VALUE''); IF recovery_authority IS NULL THEN PERFORM public.paper_0068_restore_actor(old_actor); RETURN public.paper_0067_action(''COMMIT_VALUATION'',''RECOVERY_AUTHORITY_UNAVAILABLE'',c.control_epoch,c.account_id,jsonb_build_object(''pending_target_id'',t.id,''reason_code'',''RECOVERY_AUTHORITY_UNAVAILABLE'')); END IF; recovery_grant_id:=(recovery_authority->>''grant_id'')::uuid; SELECT execution_admission_sha256 INTO recovery_bound_admission FROM public.owner_beta_paper_recovery_grants WHERE grant_id=recovery_grant_id; END IF; IF s.state NOT IN (''SETTLED_TRADE'',''SETTLED_NO_TRADE'') OR (valuation->>''control_epoch'')::bigint<>CASE WHEN recovery THEN l.control_epoch ELSE c.control_epoch END');
  definition:=replace(definition,'IF NOT public.daily_0060_admission_usable(d,valuation->>''valuation_admission_sha256'',false) OR NOT EXISTS(',
    'IF recovery AND valuation->>''valuation_admission_sha256'' IS DISTINCT FROM recovery_bound_admission OR NOT public.daily_0060_admission_usable(d,valuation->>''valuation_admission_sha256'',false) OR NOT EXISTS(');
  definition:=replace(definition,'  PERFORM set_config(''app.actor_user_id'',COALESCE(old_actor,''''),true); RETURN public.paper_0067_action(''COMMIT_VALUATION'',''OK''',
    '  IF recovery_grant_id IS NOT NULL THEN UPDATE public.owner_beta_paper_recovery_grants SET valuation_receipt_sha256=valuation->>''calculation_sha256'' WHERE grant_id=recovery_grant_id AND valuation_receipt_sha256 IS NULL AND (step=''VALUE_ONLY'' OR settlement_receipt_sha256 IS NOT NULL); IF NOT FOUND THEN RAISE EXCEPTION ''recovery valuation step already consumed'' USING ERRCODE=''55000''; END IF; END IF; PERFORM public.paper_0068_restore_actor(old_actor); RETURN public.paper_0067_action(''COMMIT_VALUATION'',''OK''');
  definition:=replace(definition,'PERFORM set_config(''app.actor_user_id'',COALESCE(old_actor,''''),true);','PERFORM public.paper_0068_restore_actor(old_actor);');
  IF position('recovery_authority:=public.paper_0068_recovery_authority' IN definition)=0 OR position('valuation_receipt_sha256=valuation->>''calculation_sha256''' IN definition)=0 THEN RAISE EXCEPTION '0067 valuation recovery hook failed to apply' USING ERRCODE='55000'; END IF;
  EXECUTE definition;
END $$;

DO $$ DECLARE f text; BEGIN
  FOREACH f IN ARRAY ARRAY['begin_owner_beta_paper_operation_v1(bigint,jsonb,text)','inspect_owner_beta_paper_operation_v1(uuid,uuid,integer)','reconcile_owner_beta_paper_operation_v1(uuid,bigint,text,text)'] LOOP
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
  FOREACH f IN ARRAY ARRAY['paper_0068_transition_legacy(bigint,text,jsonb,text)','paper_0068_principal()','paper_0068_nonnegative_i64(text)','paper_0068_restore_actor(text)','paper_0068_canonical_json(jsonb)','paper_0068_canonical_row_json(jsonb)','paper_0068_stream_digest(jsonb)','paper_0068_utc(timestamptz)','paper_0068_clock_fresh(jsonb)','paper_0068_immutable()','paper_0068_guard_grant()','paper_0068_recovery_authority(uuid,bigint,text,text)','paper_0068_gate_new_lineage_work()'] LOOP
    EXECUTE format('ALTER FUNCTION public.%s OWNER TO migration_owner',f);
    EXECUTE format('REVOKE ALL ON FUNCTION public.%s FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter',f);
  END LOOP;
END $$;
