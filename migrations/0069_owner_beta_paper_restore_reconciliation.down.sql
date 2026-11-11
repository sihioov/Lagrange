-- Reverting the old mutating reconciliation is safe only with no retained Paper history.
DO $$ BEGIN
  IF EXISTS(SELECT 1 FROM public.owner_beta_paper_operations)
     OR EXISTS(SELECT 1 FROM public.owner_beta_paper_operation_observations)
     OR EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions)
     OR EXISTS(SELECT 1 FROM public.owner_beta_paper_events)
  THEN RAISE EXCEPTION '0069 rollback blocked by retained Paper history' USING ERRCODE='55000'; END IF;
END $$;
CREATE OR REPLACE FUNCTION public.reconcile_owner_beta_paper_operation_v1(generation uuid,expected_epoch bigint,inventory_sha256 text,request_key text) RETURNS jsonb
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
