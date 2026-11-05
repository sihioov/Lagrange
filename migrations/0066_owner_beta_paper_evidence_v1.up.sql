-- R5 WP-2A2. Identity links only. SQL is NOT filesystem/timer origin authority.
-- No new principal, no caller-supplied terminal/F/W, no mutable source history.
SET LOCAL lock_timeout='5s';
SET LOCAL statement_timeout='30s';

CREATE TABLE public.owner_beta_paper_source_slots (
    slot_id text PRIMARY KEY CHECK(slot_id ~ '^sha256:[0-9a-f]{64}$'),
    reservation_id uuid NOT NULL UNIQUE CHECK(reservation_id <> '00000000-0000-0000-0000-000000000000'),
    root_event_sha256 text NOT NULL UNIQUE CHECK(root_event_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    owner_user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE RESTRICT,
    policy_sha256 text NOT NULL CHECK(policy_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    activation_epoch bigint NOT NULL CHECK(activation_epoch>=0),
    session_date date NOT NULL,
    cohort_sha256 text NOT NULL CHECK(cohort_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    cohort jsonb NOT NULL,
    reserved_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(owner_user_id,policy_sha256,session_date)
);
CREATE TABLE public.owner_beta_paper_source_admissions (
    slot_id text PRIMARY KEY REFERENCES public.owner_beta_paper_source_slots(slot_id) ON DELETE RESTRICT,
    source_event_sha256 text NOT NULL UNIQUE CHECK(source_event_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    original_set_sha256 text NOT NULL UNIQUE CHECK(original_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    admission_sha256 text NOT NULL UNIQUE REFERENCES public.owner_beta_daily_admissions(admission_sha256) ON DELETE RESTRICT,
    linked_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE public.owner_beta_paper_source_originals (
    provider text NOT NULL CHECK(provider IN ('kis','kis-calendar')),
    market text NOT NULL CHECK(market='kr'),
    batch_id uuid NOT NULL,
    file_name text NOT NULL CHECK(file_name ~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,191}$' AND file_name<>'batch.json'),
    wire_sha256 text NOT NULL CHECK(wire_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    slot_id text NOT NULL REFERENCES public.owner_beta_paper_source_admissions(slot_id) ON DELETE RESTRICT,
    PRIMARY KEY(provider,market,batch_id,file_name)
);
CREATE TABLE public.owner_beta_paper_source_runs (
    slot_id text PRIMARY KEY REFERENCES public.owner_beta_paper_source_admissions(slot_id) ON DELETE RESTRICT,
    run_id uuid NOT NULL UNIQUE REFERENCES public.owner_beta_recommendation_runs(id) ON DELETE RESTRICT,
    job_id uuid NOT NULL UNIQUE REFERENCES public.jobs(id) ON DELETE RESTRICT,
    attempt_id uuid NOT NULL UNIQUE REFERENCES public.job_attempts(id) ON DELETE RESTRICT,
    linked_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

CREATE FUNCTION public.paper_0066_principal(expected text) RETURNS void
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$ BEGIN
    IF session_user IS DISTINCT FROM expected OR current_setting('role',true) IS DISTINCT FROM 'none'
       OR NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=session_user AND rolcanlogin
          AND NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication AND NOT rolbypassrls)
    THEN RAISE EXCEPTION 'paper evidence principal denied' USING ERRCODE='42501'; END IF;
END $$;

CREATE FUNCTION public.paper_0066_authority(cohort jsonb, session date)
RETURNS public.owner_beta_daily_control
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$
DECLARE c public.owner_beta_daily_control; sc public.user_strategy_configs; old_actor text:=current_setting('app.actor_user_id',true); k text;
BEGIN
    IF session IS NULL OR NOT public.daily_0060_keys(cohort,ARRAY['owner_user_id','policy_sha256','activation_epoch','strategy_config_id','config_epoch','strategy_config_sha256','entitlement_reference','entitlement_sha256','cohort_sha256','release_manifest_sha256','research_image_id','helper_image_id','semantic_config_sha256','calendar_sha256','verifier_revision']) THEN
        RAISE EXCEPTION 'paper evidence cohort shape' USING ERRCODE='23514'; END IF;
    FOREACH k IN ARRAY ARRAY['policy_sha256','strategy_config_sha256','entitlement_sha256','cohort_sha256','release_manifest_sha256','research_image_id','helper_image_id','semantic_config_sha256','calendar_sha256'] LOOP
        IF NOT public.daily_0060_hash(cohort->k) THEN RAISE EXCEPTION 'paper evidence cohort hash' USING ERRCODE='23514'; END IF;
    END LOOP;
    IF NOT public.daily_0060_uint(cohort->'activation_epoch',0,9223372036854775807)
       OR NOT public.daily_0060_uint(cohort->'config_epoch',0,9223372036854775807)
       OR cohort->>'verifier_revision' !~ '^[0-9a-f]{40}$' THEN
        RAISE EXCEPTION 'paper evidence cohort scalar' USING ERRCODE='23514'; END IF;
    c:=public.daily_0060_authority(session,true);
    IF cohort->>'owner_user_id' IS DISTINCT FROM c.owner_user_id::text
       OR cohort->>'policy_sha256' IS DISTINCT FROM c.policy_sha256
       OR cohort->'activation_epoch' IS DISTINCT FROM to_jsonb(c.activation_epoch)
       OR cohort->>'entitlement_reference' IS DISTINCT FROM c.entitlement_reference
       OR cohort->>'entitlement_sha256' IS DISTINCT FROM c.entitlement_sha256
       OR session<c.first_auto_session THEN
        RAISE EXCEPTION 'paper evidence current authority changed' USING ERRCODE='23514'; END IF;
    IF NOT EXISTS(SELECT 1 FROM public.data_entitlements e
       WHERE e.contract_reference=c.entitlement_reference AND e.contract_document_sha256=substring(c.entitlement_sha256,8)
       AND e.status='ACTIVE' AND e.effective_from<=LEAST(date '2016-08-29',(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date)
       AND e.effective_until>=GREATEST(session,(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date)
       AND e.covered_datasets @> '["krx_eod_bars"]'::jsonb AND e.covered_uses @> '["recommendation","paper_view"]'::jsonb)
    THEN RAISE EXCEPTION 'paper evidence rights unavailable' USING ERRCODE='42501'; END IF;
    PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
    SELECT * INTO sc FROM public.user_strategy_configs WHERE id=(cohort->>'strategy_config_id')::uuid AND owner_user_id=c.owner_user_id FOR SHARE;
    IF NOT FOUND OR NOT sc.is_active OR NOT sc.daily_enabled OR to_jsonb(sc.daily_epoch) IS DISTINCT FROM cohort->'config_epoch' THEN
        RAISE EXCEPTION 'paper evidence strategy changed' USING ERRCODE='23514'; END IF;
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
    PERFORM public.daily_0060_source_lock(c.owner_user_id,false);
    RETURN c;
END $$;

-- Only current-session bars/reference/calendar originals are exclusively owned.
-- Historical prefix/action dependencies remain legitimate shared dependencies.
CREATE FUNCTION public.paper_0066_originals(a public.owner_beta_daily_admissions)
RETURNS TABLE(provider text,market text,batch_id uuid,file_name text,wire_sha256 text)
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$ DECLARE s jsonb; f jsonb; n integer:=0; BEGIN
    SELECT value INTO STRICT s FROM jsonb_array_elements(a.evidence_json#>'{manifest,daily_sources}')
      WHERE value->>'session_date'=a.range_end::text;
    FOR f IN SELECT value FROM jsonb_array_elements((s->'bars')||(s->'reference'))
       UNION ALL SELECT value->'source_file' FROM jsonb_array_elements(s->'calendar') WHERE value->>'civil_date'=a.range_end::text LOOP
        -- Current source freshness excludes copied current-day originals here;
        -- the independent root/helper binds these IDs to child raw_committed.
        IF jsonb_array_length(f->'copied_from')>0 THEN f:=(f->'copied_from')->-1; END IF;
        IF f->>'batch_date' IS DISTINCT FROM a.range_end::text THEN
            RAISE EXCEPTION 'paper evidence current source reused' USING ERRCODE='23514'; END IF;
        n:=n+1;
        provider:=f->>'provider'; market:=f->>'market'; batch_id:=(f->>'batch_id')::uuid;
        file_name:=f->>'file_name'; wire_sha256:=f->>'wire_sha256'; RETURN NEXT;
    END LOOP;
    IF n<>23 THEN RAISE EXCEPTION 'paper evidence original count' USING ERRCODE='23514'; END IF;
END $$;

CREATE FUNCTION public.paper_0066_read(s public.owner_beta_paper_source_slots) RETURNS jsonb
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$
DECLARE c public.owner_beta_daily_control; a public.owner_beta_daily_admissions; link public.owner_beta_paper_source_admissions;
    rr public.owner_beta_paper_source_runs; res public.owner_beta_daily_reservations;
    r public.owner_beta_recommendation_runs; j public.jobs; attempt public.job_attempts; claim public.owner_beta_daily_publication_claims;
    result jsonb; old_actor text:=current_setting('app.actor_user_id',true); terminal text; attempts jsonb;
BEGIN
    c:=public.paper_0066_authority(s.cohort,s.session_date);
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
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
    RETURN result;
END $$;

CREATE FUNCTION public.reserve_owner_beta_paper_source_slot(slot text,reservation uuid,root_event_hash text,cohort jsonb,session date)
RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$
DECLARE c public.owner_beta_daily_control; saved public.owner_beta_paper_source_slots; now_kst timestamp;
BEGIN
    PERFORM public.paper_0066_principal('daily_input_admitter'); c:=public.paper_0066_authority(cohort,session);
    IF slot IS NULL OR slot !~ '^sha256:[0-9a-f]{64}$' OR reservation IS NULL OR reservation='00000000-0000-0000-0000-000000000000'
       OR root_event_hash IS NULL OR root_event_hash !~ '^sha256:[0-9a-f]{64}$' THEN
        RAISE EXCEPTION 'paper evidence reservation identity' USING ERRCODE='23514'; END IF;
    -- Serialize the owner/day independently of cohort, then recheck insert/readback.
    PERFORM pg_advisory_xact_lock(hashtextextended(c.owner_user_id::text||':paper-source:'||c.policy_sha256||':'||session::text,7966));
    SELECT * INTO saved FROM public.owner_beta_paper_source_slots WHERE owner_user_id=c.owner_user_id AND policy_sha256=c.policy_sha256 AND session_date=session;
    IF FOUND THEN
        IF ROW(saved.slot_id,saved.reservation_id,saved.root_event_sha256,saved.cohort) IS DISTINCT FROM ROW(slot,reservation,root_event_hash,cohort) THEN
            RAISE EXCEPTION 'paper evidence dispatch already reserved' USING ERRCODE='23514'; END IF;
    ELSE
        now_kst:=clock_timestamp() AT TIME ZONE 'Asia/Seoul';
        IF now_kst::date<>session OR now_kst::time<time '16:30:00' OR now_kst::time>time '16:31:00' THEN
            RAISE EXCEPTION 'paper evidence reservation window' USING ERRCODE='23514'; END IF;
        INSERT INTO public.owner_beta_paper_source_slots(slot_id,reservation_id,root_event_sha256,owner_user_id,policy_sha256,activation_epoch,session_date,cohort_sha256,cohort)
           VALUES(slot,reservation,root_event_hash,c.owner_user_id,c.policy_sha256,c.activation_epoch,session,cohort->>'cohort_sha256',cohort) RETURNING * INTO saved;
    END IF;
    RETURN public.paper_0066_read(saved);
END $$;

CREATE FUNCTION public.link_owner_beta_paper_source_admission(slot text,source_event_hash text,original_set_hash text,admission text)
RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$
DECLARE s public.owner_beta_paper_source_slots; c public.owner_beta_daily_control; a public.owner_beta_daily_admissions;
    saved public.owner_beta_paper_source_admissions; computed text; n integer;
BEGIN
    PERFORM public.paper_0066_principal('daily_input_admitter');
    SELECT * INTO s FROM public.owner_beta_paper_source_slots WHERE slot_id=slot FOR UPDATE;
    IF NOT FOUND THEN RAISE EXCEPTION 'paper evidence slot absent' USING ERRCODE='23514'; END IF;
    c:=public.paper_0066_authority(s.cohort,s.session_date);
    SELECT * INTO a FROM public.owner_beta_daily_admissions WHERE admission_sha256=admission AND owner_user_id=c.owner_user_id AND policy_sha256=c.policy_sha256 AND range_end=s.session_date;
    IF NOT FOUND OR NOT public.daily_0060_admission_usable(c,admission,false)
       OR source_event_hash IS NULL OR source_event_hash !~ '^sha256:[0-9a-f]{64}$'
       OR a.evidence_json#>'{admission,base}' IS DISTINCT FROM c.policy_json->'base' THEN
        RAISE EXCEPTION 'paper evidence admission unavailable' USING ERRCODE='23514'; END IF;
    SELECT public.daily_0060_digest(string_agg(provider||'|'||market||'|'||batch_id::text||'|'||file_name||'|'||wire_sha256,chr(10) ORDER BY provider COLLATE "C",market COLLATE "C",batch_id::text COLLATE "C",file_name COLLATE "C",wire_sha256 COLLATE "C")||chr(10)),count(*) INTO computed,n FROM public.paper_0066_originals(a);
    IF n<>23 OR computed IS DISTINCT FROM original_set_hash THEN RAISE EXCEPTION 'paper evidence originals mismatch' USING ERRCODE='23514'; END IF;
    SELECT * INTO saved FROM public.owner_beta_paper_source_admissions WHERE slot_id=slot;
    IF FOUND THEN
        IF ROW(saved.source_event_sha256,saved.original_set_sha256,saved.admission_sha256) IS DISTINCT FROM ROW(source_event_hash,original_set_hash,admission) THEN
            RAISE EXCEPTION 'paper evidence admission immutable' USING ERRCODE='23514'; END IF;
    ELSE
        INSERT INTO public.owner_beta_paper_source_admissions(slot_id,source_event_sha256,original_set_sha256,admission_sha256) VALUES(slot,source_event_hash,original_set_hash,admission);
        INSERT INTO public.owner_beta_paper_source_originals SELECT provider,market,batch_id,file_name,wire_sha256,slot FROM public.paper_0066_originals(a);
    END IF;
    RETURN public.paper_0066_read(s);
END $$;

CREATE FUNCTION public.link_owner_beta_paper_daily_run(slot text,run_id uuid) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$
DECLARE s public.owner_beta_paper_source_slots; result jsonb; saved public.owner_beta_paper_source_runs;
BEGIN
    PERFORM public.paper_0066_principal('worker');
    SELECT * INTO s FROM public.owner_beta_paper_source_slots WHERE slot_id=slot FOR UPDATE;
    IF NOT FOUND THEN RAISE EXCEPTION 'paper evidence slot absent' USING ERRCODE='23514'; END IF;
    result:=public.paper_0066_read(s);
    IF result->>'candidate_run_id' IS DISTINCT FROM run_id::text OR result->>'terminal' IS DISTINCT FROM 'SUCCEEDED' OR result->'admission'='null'::jsonb THEN
        RAISE EXCEPTION 'paper evidence run not terminal' USING ERRCODE='23514'; END IF;
    SELECT * INTO saved FROM public.owner_beta_paper_source_runs WHERE slot_id=slot;
    IF FOUND THEN
        IF saved.run_id IS DISTINCT FROM run_id THEN RAISE EXCEPTION 'paper evidence run immutable' USING ERRCODE='23514'; END IF;
    ELSE
        INSERT INTO public.owner_beta_paper_source_runs(slot_id,run_id,job_id,attempt_id) VALUES(slot,run_id,(result->>'job_id')::uuid,(result->>'attempt_id')::uuid);
    END IF;
    RETURN public.paper_0066_read(s);
END $$;

CREATE FUNCTION public.read_owner_beta_paper_session_evidence(session date,cohort jsonb) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$
DECLARE s public.owner_beta_paper_source_slots; c public.owner_beta_daily_control;
BEGIN
    PERFORM public.paper_0066_principal('worker'); c:=public.paper_0066_authority(cohort,session);
    SELECT * INTO s FROM public.owner_beta_paper_source_slots WHERE owner_user_id=c.owner_user_id AND policy_sha256=c.policy_sha256 AND session_date=session;
    IF NOT FOUND OR s.cohort IS DISTINCT FROM cohort THEN RAISE EXCEPTION 'paper evidence cohort missing' USING ERRCODE='23514'; END IF;
    RETURN public.paper_0066_read(s);
END $$;

DO $security$ DECLARE t text; f record; BEGIN
    FOREACH t IN ARRAY ARRAY['slots','admissions','originals','runs'] LOOP
        EXECUTE format('ALTER TABLE public.owner_beta_paper_source_%I OWNER TO migration_owner',t);
        EXECUTE format('ALTER TABLE public.owner_beta_paper_source_%I ENABLE ROW LEVEL SECURITY',t);
        EXECUTE format('ALTER TABLE public.owner_beta_paper_source_%I FORCE ROW LEVEL SECURITY',t);
        EXECUTE format('REVOKE ALL ON public.owner_beta_paper_source_%I FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter',t);
        EXECUTE format('CREATE POLICY paper_0066_internal ON public.owner_beta_paper_source_%I FOR ALL TO migration_owner USING(true) WITH CHECK(true)',t);
        EXECUTE format('CREATE TRIGGER paper_0066_immutable BEFORE UPDATE OR DELETE ON public.owner_beta_paper_source_%I FOR EACH ROW EXECUTE FUNCTION public.daily_0060_immutable()',t);
    END LOOP;
    FOR f IN SELECT p.oid::regprocedure AS signature FROM pg_proc p WHERE p.pronamespace='public'::regnamespace AND (p.proname LIKE 'paper\_0066\_%' ESCAPE '\' OR p.proname=ANY(ARRAY['reserve_owner_beta_paper_source_slot','link_owner_beta_paper_source_admission','link_owner_beta_paper_daily_run','read_owner_beta_paper_session_evidence'])) LOOP
        EXECUTE format('ALTER FUNCTION %s OWNER TO migration_owner',f.signature);
        EXECUTE format('ALTER FUNCTION %s SET search_path=pg_catalog,pg_temp',f.signature);
        EXECUTE format('ALTER FUNCTION %s SET lock_timeout=''5s''',f.signature);
        EXECUTE format('ALTER FUNCTION %s SET statement_timeout=''30s''',f.signature);
        EXECUTE format('REVOKE ALL ON FUNCTION %s FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter',f.signature);
    END LOOP;
END $security$;
GRANT EXECUTE ON FUNCTION public.reserve_owner_beta_paper_source_slot(text,uuid,text,jsonb,date),public.link_owner_beta_paper_source_admission(text,text,text,text) TO daily_input_admitter;
GRANT EXECUTE ON FUNCTION public.link_owner_beta_paper_daily_run(text,uuid),public.read_owner_beta_paper_session_evidence(date,jsonb) TO worker;
