SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '15s';

DO $$ BEGIN
  IF EXISTS(SELECT 1 FROM public.owner_beta_paper_events WHERE num_nonnulls(request_canonical,request_sha256,result_canonical,result_sha256)>0)
     OR EXISTS(SELECT 1 FROM public.owner_beta_paper_operations)
     OR EXISTS(SELECT 1 FROM public.owner_beta_paper_operation_observations)
     OR EXISTS(SELECT 1 FROM public.owner_beta_paper_recovery_grants)
     OR EXISTS(SELECT 1 FROM public.owner_beta_paper_events
               WHERE kind IN ('DRAIN_STARTED','OFF','RESTORED_OFF','RECOVERY_AUTHORIZED')
                 AND request_key ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}:(begin|off|restore-off|recover)$')
  THEN RAISE EXCEPTION 'Owner-beta Paper operation/recovery history makes 0068 down unsafe' USING ERRCODE='23514'; END IF;
END $$;

DO $$ DECLARE definition text; BEGIN
  definition:=pg_get_functiondef('public.commit_owner_beta_paper_valuation_v1(uuid,bigint,jsonb,text)'::regprocedure);
  definition:=replace(definition,'event_seq bigint; inserted public.daily_equity; recovery_grant_id uuid; recovery_bound_admission text; recovery_authority jsonb; recovery boolean:=false; old_actor text:=current_setting(''app.actor_user_id'',true);','event_seq bigint; inserted public.daily_equity; old_actor text:=current_setting(''app.actor_user_id'',true);');
  definition:=replace(definition,'IF l.id IS NULL THEN RAISE EXCEPTION ''valuation lineage unavailable'' USING ERRCODE=''23514''; END IF; IF EXISTS(SELECT 1 FROM public.owner_beta_paper_recovery_grants g WHERE g.pending_target_id=t.id AND g.valuation_receipt_sha256 IS NULL) THEN recovery:=true; recovery_authority:=public.paper_0068_recovery_authority(t.id,expected_epoch,process_config_sha256,''VALUE''); IF recovery_authority IS NULL THEN PERFORM public.paper_0068_restore_actor(old_actor); RETURN public.paper_0067_action(''COMMIT_VALUATION'',''RECOVERY_AUTHORITY_UNAVAILABLE'',c.control_epoch,c.account_id,jsonb_build_object(''pending_target_id'',t.id,''reason_code'',''RECOVERY_AUTHORITY_UNAVAILABLE'')); END IF; recovery_grant_id:=(recovery_authority->>''grant_id'')::uuid; SELECT execution_admission_sha256 INTO recovery_bound_admission FROM public.owner_beta_paper_recovery_grants WHERE grant_id=recovery_grant_id; END IF; IF s.state NOT IN (''SETTLED_TRADE'',''SETTLED_NO_TRADE'') OR (valuation->>''control_epoch'')::bigint<>(CASE WHEN recovery THEN l.control_epoch ELSE c.control_epoch END)',
    'IF s.state NOT IN (''SETTLED_TRADE'',''SETTLED_NO_TRADE'') OR l.id IS NULL OR (valuation->>''control_epoch'')::bigint<>c.control_epoch');
  definition:=replace(definition,'IF recovery AND valuation->>''valuation_admission_sha256'' IS DISTINCT FROM recovery_bound_admission OR NOT public.daily_0060_admission_usable(d,valuation->>''valuation_admission_sha256'',false) OR NOT EXISTS(','IF NOT public.daily_0060_admission_usable(d,valuation->>''valuation_admission_sha256'',false) OR NOT EXISTS(');
  definition:=replace(definition,'  IF recovery_grant_id IS NOT NULL THEN UPDATE public.owner_beta_paper_recovery_grants SET valuation_receipt_sha256=valuation->>''calculation_sha256'' WHERE grant_id=recovery_grant_id AND valuation_receipt_sha256 IS NULL AND (step=''VALUE_ONLY'' OR settlement_receipt_sha256 IS NOT NULL); IF NOT FOUND THEN RAISE EXCEPTION ''recovery valuation step already consumed'' USING ERRCODE=''55000''; END IF; END IF; PERFORM public.paper_0068_restore_actor(old_actor); RETURN public.paper_0067_action(''COMMIT_VALUATION'',''OK''',
    '  PERFORM set_config(''app.actor_user_id'',COALESCE(old_actor,''''),true); RETURN public.paper_0067_action(''COMMIT_VALUATION'',''OK''');
  definition:=replace(definition,'PERFORM public.paper_0068_restore_actor(old_actor);','PERFORM set_config(''app.actor_user_id'',COALESCE(old_actor,''''),true);');
  IF position('paper_0068_recovery_authority' IN definition)>0 OR position('recovery_grant_id' IN definition)>0 THEN RAISE EXCEPTION '0068 valuation down restoration failed' USING ERRCODE='55000'; END IF;
  EXECUTE definition;
END $$;

DO $$ DECLARE definition text; BEGIN
  definition:=pg_get_functiondef('public.commit_owner_beta_paper_settlement_v1(uuid,bigint,jsonb,text)'::regprocedure);
  definition:=replace(definition,'recovery boolean:=false; recovery_grant_id uuid; recovery_bound_admission text; recovery_authority jsonb; old_actor text:=current_setting(''app.actor_user_id'',true);','recovery boolean:=false; old_actor text:=current_setting(''app.actor_user_id'',true);');
  definition:=replace(definition,'IF NOT FOUND THEN RAISE EXCEPTION ''settlement lineage unavailable'' USING ERRCODE=''23514''; END IF; IF EXISTS(SELECT 1 FROM public.owner_beta_paper_recovery_grants g WHERE g.pending_target_id=t.id AND g.valuation_receipt_sha256 IS NULL) THEN recovery:=true; recovery_authority:=public.paper_0068_recovery_authority(t.id,expected_epoch,process_config_sha256,''SETTLE''); IF recovery_authority IS NULL THEN PERFORM public.paper_0068_restore_actor(old_actor); RETURN public.paper_0067_action(''COMMIT_SETTLEMENT'',''RECOVERY_AUTHORITY_UNAVAILABLE'',c.control_epoch,c.account_id,jsonb_build_object(''pending_target_id'',t.id,''reason_code'',''RECOVERY_AUTHORITY_UNAVAILABLE'')); END IF; recovery_grant_id:=(recovery_authority->>''grant_id'')::uuid; SELECT execution_admission_sha256 INTO recovery_bound_admission FROM public.owner_beta_paper_recovery_grants WHERE grant_id=recovery_grant_id; IF settlement->>''execution_admission_sha256'' IS DISTINCT FROM recovery_bound_admission THEN RAISE EXCEPTION ''recovery execution admission differs from grant'' USING ERRCODE=''23514''; END IF; END IF; IF settlement->>''lineage_id''<>l.id::text OR (settlement->>''control_epoch'')::bigint<>(CASE WHEN recovery THEN l.control_epoch ELSE c.control_epoch END)',
    'IF NOT FOUND OR settlement->>''lineage_id''<>l.id::text OR (settlement->>''control_epoch'')::bigint<>c.control_epoch');
  definition:=replace(definition,'recovery:=recovery OR EXISTS(SELECT 1 FROM public.owner_beta_paper_events','recovery:=EXISTS(SELECT 1 FROM public.owner_beta_paper_events');
  definition:=replace(definition,'  IF recovery_grant_id IS NOT NULL THEN UPDATE public.owner_beta_paper_recovery_grants SET settlement_receipt_sha256=settlement->>''calculation_sha256'' WHERE grant_id=recovery_grant_id AND settlement_receipt_sha256 IS NULL AND valuation_receipt_sha256 IS NULL; IF NOT FOUND THEN RAISE EXCEPTION ''recovery settlement step already consumed'' USING ERRCODE=''55000''; END IF; END IF; PERFORM public.paper_0068_restore_actor(old_actor); RETURN public.paper_0067_action(''COMMIT_SETTLEMENT'',''OK''',
    '  PERFORM set_config(''app.actor_user_id'',COALESCE(old_actor,''''),true); RETURN public.paper_0067_action(''COMMIT_SETTLEMENT'',''OK''');
  definition:=replace(definition,'PERFORM public.paper_0068_restore_actor(old_actor);','PERFORM set_config(''app.actor_user_id'',COALESCE(old_actor,''''),true);');
  IF position('paper_0068_recovery_authority' IN definition)>0 OR position('recovery_grant_id' IN definition)>0 THEN RAISE EXCEPTION '0068 settlement down restoration failed' USING ERRCODE='55000'; END IF;
  EXECUTE definition;
END $$;

DO $$ DECLARE definition text; BEGIN
  definition:=pg_get_functiondef('public.snapshot_owner_beta_paper_target_v1(uuid,bigint,text)'::regprocedure);
  definition:=replace(definition,'e_admission text; d public.owner_beta_daily_control; recovery_authority jsonb;','e_admission text; d public.owner_beta_daily_control;');
  definition:=replace(definition,'IF NOT FOUND THEN RAISE EXCEPTION ''target lineage stale'' USING ERRCODE=''40001''; END IF; IF l.control_epoch<>c.control_epoch OR l.consent_event_id<>c.consent_event_id THEN recovery_authority:=public.paper_0068_recovery_authority(t.id,expected_epoch,process_config_sha256,CASE WHEN s.settlement_sha256 IS NULL THEN ''SETTLE'' ELSE ''VALUE'' END); IF recovery_authority IS NULL THEN PERFORM public.paper_0068_restore_actor(old_actor); RETURN public.paper_0067_action(''SNAPSHOT_TARGET'',''RECOVERY_AUTHORITY_UNAVAILABLE'',c.control_epoch,c.account_id,jsonb_build_object(''pending_target_id'',t.id,''reason_code'',''RECOVERY_AUTHORITY_UNAVAILABLE'')); END IF; END IF;','IF NOT FOUND OR l.control_epoch<>c.control_epoch OR l.consent_event_id<>c.consent_event_id THEN RAISE EXCEPTION ''target lineage stale'' USING ERRCODE=''40001''; END IF;');
  definition:=replace(definition,'   (jsonb_build_object(''pending_target_id''','   jsonb_build_object(''pending_target_id''');
  definition:=replace(definition,'''receipt_json'',s.receipt_json)||CASE WHEN recovery_authority IS NULL THEN ''{}''::jsonb ELSE jsonb_build_object(''recovery_authority'',recovery_authority) END));','''receipt_json'',s.receipt_json));');
  definition:=replace(definition,'PERFORM public.paper_0068_restore_actor(old_actor);','PERFORM set_config(''app.actor_user_id'',COALESCE(old_actor,''''),true);');
  IF position('paper_0068_recovery_authority' IN definition)>0 OR position('recovery_authority' IN definition)>0 THEN RAISE EXCEPTION '0068 target snapshot down restoration failed' USING ERRCODE='55000'; END IF;
  EXECUTE definition;
END $$;

DROP FUNCTION public.transition_owner_beta_paper_v1(bigint,text,jsonb,text);
ALTER FUNCTION public.paper_0068_transition_legacy(bigint,text,jsonb,text) RENAME TO transition_owner_beta_paper_v1;
REVOKE ALL ON FUNCTION public.transition_owner_beta_paper_v1(bigint,text,jsonb,text) FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter;
GRANT EXECUTE ON FUNCTION public.transition_owner_beta_paper_v1(bigint,text,jsonb,text) TO migration_owner;

DROP FUNCTION public.configure_owner_beta_paper_v1(uuid,text,jsonb,jsonb,text);
ALTER FUNCTION public.paper_0068_configure_legacy(uuid,text,jsonb,jsonb,text) RENAME TO configure_owner_beta_paper_v1;
REVOKE ALL ON FUNCTION public.configure_owner_beta_paper_v1(uuid,text,jsonb,jsonb,text) FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter;
GRANT EXECUTE ON FUNCTION public.configure_owner_beta_paper_v1(uuid,text,jsonb,jsonb,text) TO migration_owner;
DROP FUNCTION public.read_owner_beta_paper_operation_result_v1(text,text);
DROP TRIGGER paper_0068_receipt_guard ON public.owner_beta_paper_events;
DROP TRIGGER paper_0068_receipt_guard ON public.owner_beta_paper_operations;
DROP TRIGGER paper_0068_receipt_guard ON public.owner_beta_paper_operation_observations;
DROP FUNCTION public.paper_0068_receipt_guard();
DROP FUNCTION public.paper_0068_validate_receipt(jsonb,text);
DROP FUNCTION public.paper_0068_exact_request(text,text);
DROP FUNCTION public.paper_0068_request(text,uuid,jsonb);
DROP FUNCTION public.paper_0068_parse_canonical(text,integer);
DROP FUNCTION public.paper_0068_json_depth(jsonb);
DROP FUNCTION public.paper_0068_uuid(jsonb);
DROP FUNCTION public.paper_0068_reset(public.owner_beta_paper_control);
REVOKE SELECT (id,owner_user_id,account_id,control_epoch,kind,actor_user_id,occurred_at,payload,payload_sha256,request_key,pending_target_id) ON public.owner_beta_paper_events FROM app;
GRANT SELECT ON public.owner_beta_paper_events TO app;
ALTER TABLE public.owner_beta_paper_events DROP CONSTRAINT paper_0068_receipt_complete, DROP COLUMN request_canonical, DROP COLUMN request_sha256, DROP COLUMN result_canonical, DROP COLUMN result_sha256;

DROP FUNCTION public.paper_0068_recovery_authority(uuid,bigint,text,text);
DROP TRIGGER paper_0068_gate ON public.pending_targets;
DROP TRIGGER paper_0068_gate_apply ON public.paper_rebalance_previews;
DROP TRIGGER paper_0068_gate ON public.paper_rebalance_previews;
DROP FUNCTION public.paper_0068_gate_new_lineage_work();
DROP FUNCTION public.reconcile_owner_beta_paper_operation_v1(uuid,bigint,text,text);
DROP FUNCTION public.inspect_owner_beta_paper_operation_v1(uuid,uuid,integer);
DROP FUNCTION public.begin_owner_beta_paper_operation_v1(bigint,jsonb,text);
DROP FUNCTION public.paper_0068_utc(timestamptz);
DROP FUNCTION public.paper_0068_stream_digest(jsonb);
DROP FUNCTION public.paper_0068_canonical_row_json(jsonb);
DROP FUNCTION public.paper_0068_clock_fresh(jsonb);
DROP FUNCTION public.paper_0068_canonical_json(jsonb);
DROP FUNCTION public.paper_0068_nonnegative_i64(text);
DROP FUNCTION public.paper_0068_restore_actor(text);
DROP FUNCTION public.paper_0068_principal();

DROP TRIGGER owner_beta_paper_recovery_grant_guard ON public.owner_beta_paper_recovery_grants;
DROP TRIGGER owner_beta_paper_observations_immutable ON public.owner_beta_paper_operation_observations;
DROP TRIGGER owner_beta_paper_operations_immutable ON public.owner_beta_paper_operations;
DROP FUNCTION public.paper_0068_guard_grant();
DROP FUNCTION public.paper_0068_immutable();

DROP TABLE public.owner_beta_paper_recovery_grants;
DROP TABLE public.owner_beta_paper_operation_observations;
DROP TABLE public.owner_beta_paper_operations;
