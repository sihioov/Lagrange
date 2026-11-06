-- Fixed source-owned descriptive database audit, not archive/stop authority.
-- Caller MUST hold one REPEATABLE READ READ ONLY transaction through dump/files.
-- No BEGIN/COMMIT/exported token, TEMP objects, persistent mutations or SET ROLE.
-- Execute every statement on that same connection; stop on first error; discard
-- all partial output on error/cancellation. Never log row output or SQL diagnostics.
-- Each statement's actor changes are transaction-local and roll back on failure;
-- successful row statements restore the incoming actor before returning output.
SET LOCAL search_path=pg_catalog,pg_temp;
DO $guard$
BEGIN
 IF current_user <> 'migration_owner' OR session_user <> current_user
    OR current_setting('role') <> 'none'
    OR current_setting('transaction_isolation') <> 'repeatable read'
    OR current_setting('transaction_read_only') <> 'on'
    OR current_setting('server_encoding') <> 'UTF8' OR current_setting('client_encoding') <> 'UTF8'
    OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_roles WHERE rolname=current_user
       AND rolcanlogin AND NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole
       AND NOT rolreplication AND NOT rolbypassrls)
    OR pg_catalog.row_security_active('public.users'::regclass)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_CONTEXT' USING ERRCODE='42501'; END IF;
 IF (SELECT count(*) FROM public._sqlx_migrations WHERE success AND version BETWEEN 1 AND 68) <> 68
    OR EXISTS(SELECT 1 FROM public._sqlx_migrations WHERE NOT success OR version NOT BETWEEN 1 AND 68)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_MIGRATIONS' USING ERRCODE='23514'; END IF;
END $guard$;


DO $inventory$
DECLARE item record; actual text[]; qual text;
BEGIN

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
 WHERE n.nspname='public' AND c.relkind IN ('r','p','f') AND NOT c.relname=ANY(ARRAY['_sqlx_migrations','account_strategy_bindings','accounts','audit_logs','auth_audit_outbox','backtest_metrics','backtest_runs','backtest_warnings','broker_connections','broker_nodes','candidate_feed_items','candidate_feed_snapshots','candidate_fundamental_observations','candidate_instrument_registrations','candidate_investor_flow_snapshot_rows','candidate_investor_flows','candidate_market_status_observations','candidate_price_instrument_coverage','candidate_price_instrument_sessions','candidate_price_publications','candidate_price_revalidation_events','candidate_raw_batch_datasets','candidate_raw_batch_publications','candidate_scheduler_control','candidate_scoring_configs','candidate_sector_entries','candidate_sector_versions','candidate_universe_members','candidate_universe_registry','candidate_universe_snapshots','cash_ledger','corporate_actions','daily_equity','data_batches','data_entitlements','data_quality_issues','dataset_versions','factor_definitions','factor_snapshot_manifests','fills','identity_actor_capabilities','identity_invitation_expiry_capabilities','instrument_aliases','instruments','invitations','job_attempts','jobs','live_kill_switch','notification_deliveries','notification_subscriptions','notifications','order_intent_events','order_intents','orders','owner_beta_daily_admission_events','owner_beta_daily_admissions','owner_beta_daily_calendar_days','owner_beta_daily_calendar_sources','owner_beta_daily_control','owner_beta_daily_poll_receipts','owner_beta_daily_poll_state','owner_beta_daily_publication_claims','owner_beta_daily_reservations','owner_beta_daily_source_events','owner_beta_paper_control','owner_beta_paper_events','owner_beta_paper_lineage','owner_beta_paper_operation_observations','owner_beta_paper_operations','owner_beta_paper_recovery_grants','owner_beta_paper_session_events','owner_beta_paper_sessions','owner_beta_paper_source_admissions','owner_beta_paper_source_originals','owner_beta_paper_source_runs','owner_beta_paper_source_slots','owner_beta_recommendation_items','owner_beta_recommendation_runs','owner_beta_simulation_result_pairs','owner_beta_simulation_runs','owner_equity_generation_admissions','owner_equity_instrument_generations','owner_equity_membership_events','owner_equity_memberships','owner_equity_signal_snapshot_rows','owner_equity_signal_snapshots','owner_equity_universe_policies','owner_intraday_quote_cache','owner_intraday_quote_demands','owner_intraday_quote_producers','owner_market_stream_cache','owner_market_stream_grants','owner_market_stream_lease_items','owner_market_stream_leases','owner_market_stream_producers','owner_market_stream_subscriptions','paper_rebalance_previews','paper_settlement_outbox','paper_settlement_outbox_archive','pending_targets','positions','recommendation_items','recommendation_runs','recommendation_scheduler_control','reconciliation_runs','research_execution_status','result_artifacts','risk_events','risk_instrument_allowlist','risk_limits','robustness_children','robustness_suites','roles','screener_saved_screens','stock_analysis_runs','stock_analysis_snapshots','strategies','strategy_parameter_schemas','strategy_promotions','strategy_versions','system_flags','target_portfolios','trading_calendar_versions','trading_calendars','universe_snapshots','user_roles','user_strategy_configs','users','web_sessions','worker_heartbeats']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_UNKNOWN_TABLE' USING ERRCODE='23514'; END IF;
 IF EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c JOIN pg_catalog.pg_class t ON t.oid=c.conrelid
 JOIN pg_catalog.pg_namespace n ON n.oid=t.relnamespace WHERE n.nspname='public' AND NOT c.convalidated)
 OR EXISTS(SELECT 1 FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_class c ON c.oid=t.tgrelid
 JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND t.tgenabled NOT IN ('O','A'))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_DISABLED_CONSTRAINT' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.account_strategy_bindings'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['account_id','auto_apply_recommendations','bound_at','id','owner_user_id','strategy_config_id','strategy_id','strategy_version','unbound_at']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:account_strategy_bindings' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.accounts'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['account_type','cost_profile_id','cost_profile_version','created_at','currency','id','initial_cash','name','owner_user_id','paper_state_version','status','updated_at']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:accounts' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.cash_ledger'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['account_id','amount','balance','created_at','currency','event_type','id','owner_user_id','reference_id','seq','ts']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:cash_ledger' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.daily_equity'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['account_id','cash','created_at','currency','equity','id','owner_user_id','positions_value','trading_date']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:daily_equity' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.data_entitlements'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['contract_document_sha256','contract_reference','covered_datasets','covered_uses','created_at','effective_from','effective_until','id','managed_by','status','updated_at']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:data_entitlements' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.fills'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['account_id','created_at','fees','fill_ref','id','instrument_id','order_id','owner_user_id','price','quantity','side','ts']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:fills' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.job_attempts'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['attempt_no','claimed_by','created_at','error_code','error_message','finished_at','id','job_id','outcome','started_at']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:job_attempts' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.jobs'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['attempt_count','available_at','created_at','error_code','error_message','finished_at','id','idempotency_key','job_type','locked_at','locked_by','max_attempts','owner_user_id','payload_json','priority','started_at','status','updated_at']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:jobs' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.notification_deliveries'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['attempted_at','channel','delivery_attempts','delivery_lease_expires_at','delivery_token','error_detail','id','notification_id','owner_user_id','status']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:notification_deliveries' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.notifications'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['body','created_at','id','kind','owner_user_id','read_at','source_key','title']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:notifications' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.orders'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['account_id','created_at','id','instrument_id','order_ref','owner_user_id','price','quantity','side','status','submitted_at','updated_at']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:orders' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_daily_admission_events'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['admission_sha256','created_at','event_sha256','evidence_sha256','kind','owner_user_id']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_daily_admission_events' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_daily_admissions'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['admission_sha256','admitted_at','bar_count','content_sha256','evidence_json','manifest_sha256','owner_user_id','parent_admission_sha256','policy_sha256','range_end','range_start','session_count']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_daily_admissions' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_daily_calendar_days'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['calendar_evidence_sha256','evidence_kind','owner_user_id','session_date','session_type','source_row_ordinal','source_row_sha256']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_daily_calendar_days' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_daily_calendar_sources'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['calendar_evidence_sha256','copy_chain','evidence_json','normalizer_version','owner_user_id','policy_sha256','projected_at','requested_target','source_batch_date','source_batch_id','source_file_name','source_file_sha256','source_manifest_line_sha256','source_market','source_provider','source_retrieved_at']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_daily_calendar_sources' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_daily_control'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['activation_epoch','backoff_seconds','enabled','entitlement_reference','entitlement_sha256','first_auto_session','id','lease_seconds','max_jobs_per_owner','owner_user_id','policy_json','policy_sha256','trigger_version','updated_at']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_daily_control' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_daily_poll_receipts'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['activation_epoch','admission_sha256','completed_at','completion_binding_sha256','event_sha256','owner_user_id','policy_sha256','receipt_id','result_code','source_frontier_sha256','stage','started_at']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_daily_poll_receipts' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_daily_poll_state'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['last_completed_at','last_result_code','latest_receipt_id','next_poll_at','source_frontier_sha256','stage','verified_admission_sha256']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_daily_poll_state' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_daily_publication_claims'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['attempt_id','attempt_no','authorized_at','claimed_by','job_id','lease_seconds','locked_at','owner_user_id','run_id']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_daily_publication_claims' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_daily_reservations'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['activation_epoch','admission_sha256','config_epoch','created_at','job_id','owner_user_id','run_id','session_date','strategy_config_id','strategy_config_sha256','trigger_version']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_daily_reservations' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_daily_source_events'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['admission_sha256','code','event_sha256','evidence_sha256','owner_user_id','range_end','range_start','recorded_at','source_kind']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_daily_source_events' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_paper_control'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['account_id','activation_event_id','cohort_sha256','consent_event_id','contract_id','contract_json','contract_sha256','control_epoch','cost_profile_id','cost_profile_sha256','cost_profile_version','daily_cohort','id','last_signal_allocation_sha256','last_signal_date','owner_user_id','process_binding','process_binding_sha256','state','strategy_config_id','updated_at']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_paper_control' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_paper_events'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['account_id','actor_user_id','control_epoch','id','kind','occurred_at','owner_user_id','payload','payload_sha256','pending_target_id','request_canonical','request_key','request_sha256','result_canonical','result_sha256']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_paper_events' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_paper_lineage'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['account_id','activation_event_id','calendar_evidence_sha256','calendar_link_json','cohort_sha256','config_epoch','consent_event_id','control_epoch','created_at','daily_activation_epoch','effective_session','factor_snapshot_sha256','id','owner_user_id','recommendation_run_id','signal_admission_sha256','signal_date','strategy_config_id','target_snapshot_sha256','target_weights_json']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_paper_lineage' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_paper_operation_observations'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['generation','observation_id','observed_at','request_canonical','request_key','request_sha256','result_canonical','result_sha256','summary','summary_sha256']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_paper_operation_observations' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_paper_operations'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['account_id','created_at','expected_epoch','fence_epoch','fence_event_id','generation','installation_id','intent','intent_sha256','local_epoch_floor','prior_state','purpose','release_manifest_sha256','request_canonical','request_key','request_sha256','result_canonical','result_sha256','scope']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_paper_operations' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_paper_recovery_grants'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['account_id','active_epoch','consent_event_id','created_at','decision','decision_sha256','effective_session','execution_admission_sha256','expires_at','grant_id','lineage_epoch','lineage_id','original_receipt_sha256','pending_target_id','process_config_sha256','request_key','revoked_at','settlement_receipt_sha256','signal_admission_sha256','step','valuation_receipt_sha256']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_paper_recovery_grants' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_paper_session_events'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['event_seq','id','kind','occurred_at','payload','pending_target_id','reason_code']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_paper_session_events' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_paper_sessions'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['account_id','account_state_sha256','account_state_version','effective_session','execution_admission_sha256','execution_input_sha256','input_admitted_at','ledger_after_preimage','ledger_after_sha256','ledger_after_version','ledger_before_sha256','owner_user_id','pending_target_id','processing_origin','reason_code','receipt_json','settled_at','settlement_kind','settlement_sha256','source_original_retrieved_at','state','updated_at','usable_at','valuation_admission_sha256','valuation_input_sha256','valuation_sha256','valued_at']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_paper_sessions' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_paper_source_admissions'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['admission_sha256','linked_at','original_set_sha256','slot_id','source_event_sha256']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_paper_source_admissions' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_paper_source_originals'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['batch_id','file_name','market','provider','slot_id','wire_sha256']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_paper_source_originals' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_paper_source_runs'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['attempt_id','job_id','linked_at','run_id','slot_id']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_paper_source_runs' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_paper_source_slots'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['activation_epoch','cohort','cohort_sha256','owner_user_id','policy_sha256','reservation_id','reserved_at','root_event_sha256','session_date','slot_id']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_paper_source_slots' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_recommendation_items'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['created_at','excluded','exclusion_reason','factors_json','id','instrument_id','owner_user_id','rank','reason_codes','recommendation_run_id','target_weight']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_recommendation_items' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_recommendation_runs'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['action_manifest_sha256','approval_registry_sha256','artifact_manifest_sha256','as_of','audience','candidate_content_sha256','capability','cash_weight','created_at','daily_activation_epoch','daily_config_epoch','daily_pins','daily_trigger_version','error_code','factor_snapshot_sha256','finished_at','id','input_kind','job_id','owner_user_id','stage5_manifest_sha256','started_at','status','strategy_config_id','strategy_config_json','strategy_config_sha256','strategy_id','strategy_version','strict_pit','target_snapshot_sha256','trigger_kind','updated_at','vendor_snapshot']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_recommendation_runs' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_simulation_result_pairs'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['created_at','illustrative_canonical','illustrative_sha256','owner_user_id','run_id','zero_cost_canonical','zero_cost_sha256']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_simulation_result_pairs' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.owner_beta_simulation_runs'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['action_count','action_manifest_sha256','approval_registry_sha256','artifact_manifest_sha256','audience','bars_sha256','candidate_content_sha256','capability','contract_document_sha256','created_at','engine_json','error_code','evaluation_end','evaluation_start','finished_at','id','input_kind','job_id','owner_user_id','published_attempt_id','session_count','stage5_manifest_sha256','started_at','status','strategy_config_id','strategy_config_json','strategy_config_sha256','strategy_id','strategy_version','strict_pit','updated_at','vendor_snapshot']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:owner_beta_simulation_runs' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.paper_rebalance_previews'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['account_id','account_state_preimage','account_state_sha256','account_state_version','applied_at','beta_apply_mode','beta_lineage_id','beta_request_key','completed_at','cost_profile_id','cost_profile_version','created_at','dataset_manifest_sha256','dataset_version_id','error_json','id','input_kind','job_id','owner_user_id','pending_target_id','preview_seoul_date','preview_token','price_basis','price_date','proposed_effective_date','recommendation_run_id','result_json','signal_allocation_sha256','started_at','status','strategy_config_id','target_portfolio_id','target_portfolio_sha256','updated_at']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:paper_rebalance_previews' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.paper_settlement_outbox'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['attempts','available_at','body','claim_expires_at','claim_token','created_at','delivered_at','exhausted_at','id','kind','last_error','max_attempts','owner_user_id','parity_json','pending_target_id','severity','title']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:paper_settlement_outbox' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.paper_settlement_outbox_archive'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['archived_at','attempts','body','created_at','delivered_at','id','kind','max_attempts','owner_user_id','parity_json','pending_target_id','severity','title']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:paper_settlement_outbox_archive' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.pending_targets'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['account_id','beta_lineage_id','computed_on','created_at','dataset_manifest_sha256','dataset_version','dataset_version_id','effective_date','executed_at','id','non_execution_reason','owner_user_id','recommendation_run_id','source_kind','status','strategy_config_id','targets_json']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:pending_targets' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.positions'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['account_id','avg_price','id','instrument_id','owner_user_id','quantity','updated_at']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:positions' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.user_strategy_configs'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['config_json','created_at','daily_enabled','daily_epoch','id','is_active','owner_user_id','strategy_id','strategy_version','updated_at']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:user_strategy_configs' USING ERRCODE='23514'; END IF;

 SELECT array_agg(attname::text ORDER BY attname COLLATE "C") INTO actual FROM pg_catalog.pg_attribute
 WHERE attrelid='public.users'::regclass AND attnum>0 AND NOT attisdropped;
 IF actual IS DISTINCT FROM ARRAY['created_at','display_name','email','id','issuer','provisioned_by_user_id','subject','updated_at']::text[] THEN RAISE EXCEPTION 'PAPER_AUDIT_COLUMNS:users' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.account_strategy_bindings'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:account_strategy_bindings' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.account_strategy_bindings'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.account_strategy_bindings'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true','owner_user_id=current_setting''app.actor_user_id''::text,true::uuid','owner_user_id=NULLIFcurrent_setting''app.actor_user_id''::text,true,''''::text::uuid']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:account_strategy_bindings' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c
 JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attname='owner_user_id'
 JOIN pg_catalog.pg_attribute u ON u.attrelid=c.confrelid AND u.attname='id'
 WHERE c.conrelid='public.account_strategy_bindings'::regclass AND c.contype='f' AND c.convalidated
 AND c.confrelid='public.users'::regclass AND c.conkey=ARRAY[a.attnum] AND c.confkey=ARRAY[u.attnum])
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OWNER_FK:account_strategy_bindings' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.accounts'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:accounts' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.accounts'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.accounts'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true','owner_user_id=current_setting''app.actor_user_id''::text,true::uuid','owner_user_id=NULLIFcurrent_setting''app.actor_user_id''::text,true,''''::text::uuid']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:accounts' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c
 JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attname='owner_user_id'
 JOIN pg_catalog.pg_attribute u ON u.attrelid=c.confrelid AND u.attname='id'
 WHERE c.conrelid='public.accounts'::regclass AND c.contype='f' AND c.convalidated
 AND c.confrelid='public.users'::regclass AND c.conkey=ARRAY[a.attnum] AND c.confkey=ARRAY[u.attnum])
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OWNER_FK:accounts' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.cash_ledger'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:cash_ledger' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.cash_ledger'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.cash_ledger'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true','owner_user_id=current_setting''app.actor_user_id''::text,true::uuid','owner_user_id=NULLIFcurrent_setting''app.actor_user_id''::text,true,''''::text::uuid']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:cash_ledger' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c
 JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attname='owner_user_id'
 JOIN pg_catalog.pg_attribute u ON u.attrelid=c.confrelid AND u.attname='id'
 WHERE c.conrelid='public.cash_ledger'::regclass AND c.contype='f' AND c.convalidated
 AND c.confrelid='public.users'::regclass AND c.conkey=ARRAY[a.attnum] AND c.confkey=ARRAY[u.attnum])
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OWNER_FK:cash_ledger' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.daily_equity'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:daily_equity' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.daily_equity'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.daily_equity'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true','owner_user_id=current_setting''app.actor_user_id''::text,true::uuid','owner_user_id=NULLIFcurrent_setting''app.actor_user_id''::text,true,''''::text::uuid']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:daily_equity' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c
 JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attname='owner_user_id'
 JOIN pg_catalog.pg_attribute u ON u.attrelid=c.confrelid AND u.attname='id'
 WHERE c.conrelid='public.daily_equity'::regclass AND c.contype='f' AND c.convalidated
 AND c.confrelid='public.users'::regclass AND c.conkey=ARRAY[a.attnum] AND c.confkey=ARRAY[u.attnum])
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OWNER_FK:daily_equity' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.data_entitlements'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=false
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:data_entitlements' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.fills'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:fills' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.fills'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.fills'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true','owner_user_id=current_setting''app.actor_user_id''::text,true::uuid','owner_user_id=NULLIFcurrent_setting''app.actor_user_id''::text,true,''''::text::uuid']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:fills' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c
 JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attname='owner_user_id'
 JOIN pg_catalog.pg_attribute u ON u.attrelid=c.confrelid AND u.attname='id'
 WHERE c.conrelid='public.fills'::regclass AND c.contype='f' AND c.convalidated
 AND c.confrelid='public.users'::regclass AND c.conkey=ARRAY[a.attnum] AND c.confkey=ARRAY[u.attnum])
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OWNER_FK:fills' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.job_attempts'::regclass
 AND relrowsecurity=false AND relforcerowsecurity=false
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:job_attempts' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.jobs'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:jobs' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.jobs'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.jobs'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true','owner_user_id=current_setting''app.actor_user_id''::text,true::uuid','owner_user_id=NULLIFcurrent_setting''app.actor_user_id''::text,true,''''::text::uuid']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:jobs' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c
 JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attname='owner_user_id'
 JOIN pg_catalog.pg_attribute u ON u.attrelid=c.confrelid AND u.attname='id'
 WHERE c.conrelid='public.jobs'::regclass AND c.contype='f' AND c.convalidated
 AND c.confrelid='public.users'::regclass AND c.conkey=ARRAY[a.attnum] AND c.confkey=ARRAY[u.attnum])
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OWNER_FK:jobs' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.notification_deliveries'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:notification_deliveries' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.notification_deliveries'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.notification_deliveries'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true','owner_user_id=current_setting''app.actor_user_id''::text,true::uuid','owner_user_id=NULLIFcurrent_setting''app.actor_user_id''::text,true,''''::text::uuid']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:notification_deliveries' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c
 JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attname='owner_user_id'
 JOIN pg_catalog.pg_attribute u ON u.attrelid=c.confrelid AND u.attname='id'
 WHERE c.conrelid='public.notification_deliveries'::regclass AND c.contype='f' AND c.convalidated
 AND c.confrelid='public.users'::regclass AND c.conkey=ARRAY[a.attnum] AND c.confkey=ARRAY[u.attnum])
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OWNER_FK:notification_deliveries' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.notifications'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:notifications' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.notifications'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.notifications'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true','owner_user_id=current_setting''app.actor_user_id''::text,true::uuid','owner_user_id=NULLIFcurrent_setting''app.actor_user_id''::text,true,''''::text::uuid']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:notifications' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c
 JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attname='owner_user_id'
 JOIN pg_catalog.pg_attribute u ON u.attrelid=c.confrelid AND u.attname='id'
 WHERE c.conrelid='public.notifications'::regclass AND c.contype='f' AND c.convalidated
 AND c.confrelid='public.users'::regclass AND c.conkey=ARRAY[a.attnum] AND c.confkey=ARRAY[u.attnum])
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OWNER_FK:notifications' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.orders'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:orders' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.orders'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.orders'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true','owner_user_id=current_setting''app.actor_user_id''::text,true::uuid','owner_user_id=NULLIFcurrent_setting''app.actor_user_id''::text,true,''''::text::uuid']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:orders' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c
 JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attname='owner_user_id'
 JOIN pg_catalog.pg_attribute u ON u.attrelid=c.confrelid AND u.attname='id'
 WHERE c.conrelid='public.orders'::regclass AND c.contype='f' AND c.convalidated
 AND c.confrelid='public.users'::regclass AND c.conkey=ARRAY[a.attnum] AND c.confkey=ARRAY[u.attnum])
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OWNER_FK:orders' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_daily_admission_events'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_daily_admission_events' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_admission_events'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_admission_events'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_daily_admission_events' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.owner_beta_daily_admission_events t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:owner_beta_daily_admission_events' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_daily_admissions'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_daily_admissions' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_admissions'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_admissions'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_daily_admissions' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.owner_beta_daily_admissions t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:owner_beta_daily_admissions' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_daily_calendar_days'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_daily_calendar_days' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_calendar_days'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_calendar_days'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_daily_calendar_days' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.owner_beta_daily_calendar_days t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:owner_beta_daily_calendar_days' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_daily_calendar_sources'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_daily_calendar_sources' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_calendar_sources'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_calendar_sources'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_daily_calendar_sources' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.owner_beta_daily_calendar_sources t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:owner_beta_daily_calendar_sources' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_daily_control'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_daily_control' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_control'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_control'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_daily_control' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.owner_beta_daily_control t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:owner_beta_daily_control' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_daily_poll_receipts'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_daily_poll_receipts' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_poll_receipts'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_poll_receipts'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_daily_poll_receipts' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.owner_beta_daily_poll_receipts t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:owner_beta_daily_poll_receipts' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_daily_poll_state'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_daily_poll_state' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_poll_state'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_poll_state'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_daily_poll_state' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_daily_publication_claims'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_daily_publication_claims' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_publication_claims'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_publication_claims'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_daily_publication_claims' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.owner_beta_daily_publication_claims t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:owner_beta_daily_publication_claims' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_daily_reservations'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_daily_reservations' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_reservations'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_reservations'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_daily_reservations' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.owner_beta_daily_reservations t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:owner_beta_daily_reservations' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_daily_source_events'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_daily_source_events' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_source_events'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_daily_source_events'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_daily_source_events' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.owner_beta_daily_source_events t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:owner_beta_daily_source_events' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_paper_control'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_paper_control' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_control'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_control'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_paper_control' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.owner_beta_paper_control t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:owner_beta_paper_control' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_paper_events'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_paper_events' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_events'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_events'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_paper_events' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.owner_beta_paper_events t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:owner_beta_paper_events' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_paper_lineage'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_paper_lineage' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_lineage'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_lineage'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_paper_lineage' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.owner_beta_paper_lineage t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:owner_beta_paper_lineage' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_paper_operation_observations'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_paper_operation_observations' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_operation_observations'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_operation_observations'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_paper_operation_observations' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_paper_operations'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_paper_operations' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_operations'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_operations'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_paper_operations' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_paper_recovery_grants'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_paper_recovery_grants' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_recovery_grants'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_recovery_grants'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_paper_recovery_grants' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_paper_session_events'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_paper_session_events' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_session_events'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_session_events'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_paper_session_events' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_paper_sessions'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_paper_sessions' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_sessions'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_sessions'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_paper_sessions' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:owner_beta_paper_sessions' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_paper_source_admissions'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_paper_source_admissions' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_source_admissions'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_source_admissions'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_paper_source_admissions' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_paper_source_originals'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_paper_source_originals' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_source_originals'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_source_originals'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_paper_source_originals' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_paper_source_runs'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_paper_source_runs' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_source_runs'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_source_runs'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_paper_source_runs' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_paper_source_slots'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_paper_source_slots' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_source_slots'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_paper_source_slots'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_paper_source_slots' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.owner_beta_paper_source_slots t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:owner_beta_paper_source_slots' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_recommendation_items'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_recommendation_items' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_recommendation_items'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_recommendation_items'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true','owner_user_id=current_setting''app.actor_user_id''::text,true::uuid','owner_user_id=NULLIFcurrent_setting''app.actor_user_id''::text,true,''''::text::uuid']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_recommendation_items' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c
 JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attname='owner_user_id'
 JOIN pg_catalog.pg_attribute u ON u.attrelid=c.confrelid AND u.attname='id'
 WHERE c.conrelid='public.owner_beta_recommendation_items'::regclass AND c.contype='f' AND c.convalidated
 AND c.confrelid='public.users'::regclass AND c.conkey=ARRAY[a.attnum] AND c.confkey=ARRAY[u.attnum])
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OWNER_FK:owner_beta_recommendation_items' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_recommendation_runs'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_recommendation_runs' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_recommendation_runs'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_recommendation_runs'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true','owner_user_id=current_setting''app.actor_user_id''::text,true::uuid','owner_user_id=NULLIFcurrent_setting''app.actor_user_id''::text,true,''''::text::uuid']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_recommendation_runs' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c
 JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attname='owner_user_id'
 JOIN pg_catalog.pg_attribute u ON u.attrelid=c.confrelid AND u.attname='id'
 WHERE c.conrelid='public.owner_beta_recommendation_runs'::regclass AND c.contype='f' AND c.convalidated
 AND c.confrelid='public.users'::regclass AND c.conkey=ARRAY[a.attnum] AND c.confkey=ARRAY[u.attnum])
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OWNER_FK:owner_beta_recommendation_runs' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_simulation_result_pairs'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_simulation_result_pairs' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_simulation_result_pairs'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_simulation_result_pairs'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true','owner_user_id=current_setting''app.actor_user_id''::text,true::uuid','owner_user_id=NULLIFcurrent_setting''app.actor_user_id''::text,true,''''::text::uuid']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_simulation_result_pairs' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c
 JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attname='owner_user_id'
 JOIN pg_catalog.pg_attribute u ON u.attrelid=c.confrelid AND u.attname='id'
 WHERE c.conrelid='public.owner_beta_simulation_result_pairs'::regclass AND c.contype='f' AND c.convalidated
 AND c.confrelid='public.users'::regclass AND c.conkey=ARRAY[a.attnum] AND c.confkey=ARRAY[u.attnum])
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OWNER_FK:owner_beta_simulation_result_pairs' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.owner_beta_simulation_runs'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:owner_beta_simulation_runs' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_simulation_runs'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.owner_beta_simulation_runs'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true','owner_user_id=current_setting''app.actor_user_id''::text,true::uuid','owner_user_id=NULLIFcurrent_setting''app.actor_user_id''::text,true,''''::text::uuid']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:owner_beta_simulation_runs' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c
 JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attname='owner_user_id'
 JOIN pg_catalog.pg_attribute u ON u.attrelid=c.confrelid AND u.attname='id'
 WHERE c.conrelid='public.owner_beta_simulation_runs'::regclass AND c.contype='f' AND c.convalidated
 AND c.confrelid='public.users'::regclass AND c.conkey=ARRAY[a.attnum] AND c.confkey=ARRAY[u.attnum])
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OWNER_FK:owner_beta_simulation_runs' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.paper_rebalance_previews'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:paper_rebalance_previews' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.paper_rebalance_previews'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.paper_rebalance_previews'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:paper_rebalance_previews' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.paper_rebalance_previews t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:paper_rebalance_previews' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.paper_settlement_outbox'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:paper_settlement_outbox' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.paper_settlement_outbox'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.paper_settlement_outbox'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:paper_settlement_outbox' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.paper_settlement_outbox t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:paper_settlement_outbox' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.paper_settlement_outbox_archive'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:paper_settlement_outbox_archive' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.paper_settlement_outbox_archive'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.paper_settlement_outbox_archive'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:paper_settlement_outbox_archive' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM public.paper_settlement_outbox_archive t LEFT JOIN public.users u ON u.id=t.owner_user_id
 WHERE u.id IS NULL) THEN RAISE EXCEPTION 'PAPER_AUDIT_ORPHAN_OWNER:paper_settlement_outbox_archive' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.pending_targets'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:pending_targets' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.pending_targets'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.pending_targets'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true','owner_user_id=current_setting''app.actor_user_id''::text,true::uuid','owner_user_id=NULLIFcurrent_setting''app.actor_user_id''::text,true,''''::text::uuid']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:pending_targets' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c
 JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attname='owner_user_id'
 JOIN pg_catalog.pg_attribute u ON u.attrelid=c.confrelid AND u.attname='id'
 WHERE c.conrelid='public.pending_targets'::regclass AND c.contype='f' AND c.convalidated
 AND c.confrelid='public.users'::regclass AND c.conkey=ARRAY[a.attnum] AND c.confkey=ARRAY[u.attnum])
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OWNER_FK:pending_targets' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.positions'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:positions' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.positions'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.positions'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true','owner_user_id=current_setting''app.actor_user_id''::text,true::uuid','owner_user_id=NULLIFcurrent_setting''app.actor_user_id''::text,true,''''::text::uuid']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:positions' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c
 JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attname='owner_user_id'
 JOIN pg_catalog.pg_attribute u ON u.attrelid=c.confrelid AND u.attname='id'
 WHERE c.conrelid='public.positions'::regclass AND c.contype='f' AND c.convalidated
 AND c.confrelid='public.users'::regclass AND c.conkey=ARRAY[a.attnum] AND c.confkey=ARRAY[u.attnum])
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OWNER_FK:positions' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.user_strategy_configs'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=true
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:user_strategy_configs' USING ERRCODE='23514'; END IF;

 IF EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.user_strategy_configs'::regclass
 AND NOT polpermissive AND polcmd IN ('*','r') AND (0=ANY(polroles) OR
 (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)))
 OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy WHERE polrelid='public.user_strategy_configs'::regclass
 AND polpermissive AND polcmd IN ('*','r') AND (SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user)=ANY(polroles)
 AND regexp_replace(pg_catalog.pg_get_expr(polqual,polrelid),'[()[:space:]]','','g')=ANY(ARRAY['true','owner_user_id=current_setting''app.actor_user_id''::text,true::uuid','owner_user_id=NULLIFcurrent_setting''app.actor_user_id''::text,true,''''::text::uuid']::text[]))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_POLICY:user_strategy_configs' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c
 JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attname='owner_user_id'
 JOIN pg_catalog.pg_attribute u ON u.attrelid=c.confrelid AND u.attname='id'
 WHERE c.conrelid='public.user_strategy_configs'::regclass AND c.contype='f' AND c.convalidated
 AND c.confrelid='public.users'::regclass AND c.conkey=ARRAY[a.attnum] AND c.confkey=ARRAY[u.attnum])
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OWNER_FK:user_strategy_configs' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid='public.users'::regclass
 AND relrowsecurity=true AND relforcerowsecurity=false
 AND pg_catalog.pg_get_userbyid(relowner)='migration_owner')
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VISIBILITY_FLAGS:users' USING ERRCODE='23514'; END IF;

END $inventory$;

-- Pure read-only counterparts of existing consistency predicates.
-- No consistency trigger invocation: the original Daily trigger locks rows.
DO $relations$
#variable_conflict use_column
DECLARE actor uuid; old_actor text:=current_setting('app.actor_user_id',true);
 j public.jobs; r public.owner_beta_recommendation_runs;
 res public.owner_beta_daily_reservations; claim public.owner_beta_daily_publication_claims;
 a public.job_attempts; admitted public.owner_beta_daily_admissions;
 item_count integer; running integer; weight numeric; item_ids jsonb;
BEGIN
 FOR actor IN SELECT id FROM public.users ORDER BY id LOOP
 PERFORM pg_catalog.set_config('app.actor_user_id',actor::text,true);

IF EXISTS(SELECT 1 FROM public.account_strategy_bindings t LEFT JOIN public.accounts p
 ON p.id=t.account_id AND p.owner_user_id=t.owner_user_id
 WHERE t.owner_user_id=actor AND t.account_id IS NOT NULL AND p.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_ACCOUNT:account_strategy_bindings' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.owner_beta_paper_control t LEFT JOIN public.accounts p
 ON p.id=t.account_id AND p.owner_user_id=t.owner_user_id
 WHERE t.owner_user_id=actor AND t.account_id IS NOT NULL AND p.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_ACCOUNT:owner_beta_paper_control' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.owner_beta_paper_events t LEFT JOIN public.accounts p
 ON p.id=t.account_id AND p.owner_user_id=t.owner_user_id
 WHERE t.owner_user_id=actor AND t.account_id IS NOT NULL AND p.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_ACCOUNT:owner_beta_paper_events' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.paper_rebalance_previews t LEFT JOIN public.accounts p
 ON p.id=t.account_id AND p.owner_user_id=t.owner_user_id
 WHERE t.owner_user_id=actor AND t.account_id IS NOT NULL AND p.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_ACCOUNT:paper_rebalance_previews' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.pending_targets t LEFT JOIN public.accounts p
 ON p.id=t.account_id AND p.owner_user_id=t.owner_user_id
 WHERE t.owner_user_id=actor AND t.account_id IS NOT NULL AND p.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_ACCOUNT:pending_targets' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.owner_beta_paper_lineage t LEFT JOIN public.accounts p
 ON p.id=t.account_id AND p.owner_user_id=t.owner_user_id
 WHERE t.owner_user_id=actor AND t.account_id IS NOT NULL AND p.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_ACCOUNT:owner_beta_paper_lineage' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions t LEFT JOIN public.accounts p
 ON p.id=t.account_id AND p.owner_user_id=t.owner_user_id
 WHERE t.owner_user_id=actor AND t.account_id IS NOT NULL AND p.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_ACCOUNT:owner_beta_paper_sessions' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.cash_ledger t LEFT JOIN public.accounts p
 ON p.id=t.account_id AND p.owner_user_id=t.owner_user_id
 WHERE t.owner_user_id=actor AND t.account_id IS NOT NULL AND p.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_ACCOUNT:cash_ledger' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.positions t LEFT JOIN public.accounts p
 ON p.id=t.account_id AND p.owner_user_id=t.owner_user_id
 WHERE t.owner_user_id=actor AND t.account_id IS NOT NULL AND p.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_ACCOUNT:positions' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.orders t LEFT JOIN public.accounts p
 ON p.id=t.account_id AND p.owner_user_id=t.owner_user_id
 WHERE t.owner_user_id=actor AND t.account_id IS NOT NULL AND p.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_ACCOUNT:orders' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.fills t LEFT JOIN public.accounts p
 ON p.id=t.account_id AND p.owner_user_id=t.owner_user_id
 WHERE t.owner_user_id=actor AND t.account_id IS NOT NULL AND p.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_ACCOUNT:fills' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.daily_equity t LEFT JOIN public.accounts p
 ON p.id=t.account_id AND p.owner_user_id=t.owner_user_id
 WHERE t.owner_user_id=actor AND t.account_id IS NOT NULL AND p.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_ACCOUNT:daily_equity' USING ERRCODE='23514'; END IF;
FOR j IN SELECT jobs.id,jobs.owner_user_id,jobs.job_type,jobs.status,jobs.priority,jobs.idempotency_key,jobs.payload_json,jobs.max_attempts,jobs.attempt_count,jobs.available_at,jobs.locked_by,jobs.locked_at,jobs.started_at,jobs.finished_at,jobs.error_code,jobs.error_message,jobs.created_at,jobs.updated_at FROM public.jobs WHERE owner_user_id=actor AND
 (payload_json->>'schema'='owner-beta-daily-recommendation-v1' OR EXISTS
 (SELECT 1 FROM public.owner_beta_daily_reservations WHERE job_id=jobs.id)) LOOP
    SELECT owner_beta_daily_reservations.owner_user_id,owner_beta_daily_reservations.strategy_config_id,owner_beta_daily_reservations.session_date,owner_beta_daily_reservations.trigger_version,owner_beta_daily_reservations.run_id,owner_beta_daily_reservations.job_id,owner_beta_daily_reservations.admission_sha256,owner_beta_daily_reservations.strategy_config_sha256,owner_beta_daily_reservations.config_epoch,owner_beta_daily_reservations.activation_epoch,owner_beta_daily_reservations.created_at INTO res FROM public.owner_beta_daily_reservations WHERE job_id=j.id;
    IF j.payload_json->>'schema' IS DISTINCT FROM 'owner-beta-daily-recommendation-v1' AND res.job_id IS NULL THEN
        CONTINUE;
    END IF;
    SELECT job_attempts.id,job_attempts.job_id,job_attempts.attempt_no,job_attempts.outcome,job_attempts.claimed_by,job_attempts.error_code,job_attempts.error_message,job_attempts.started_at,job_attempts.finished_at,job_attempts.created_at INTO a FROM public.job_attempts WHERE job_id=j.id AND attempt_no=j.attempt_count;
    SELECT owner_beta_recommendation_runs.id,owner_beta_recommendation_runs.owner_user_id,owner_beta_recommendation_runs.strategy_config_id,owner_beta_recommendation_runs.job_id,owner_beta_recommendation_runs.as_of,owner_beta_recommendation_runs.status,owner_beta_recommendation_runs.input_kind,owner_beta_recommendation_runs.capability,owner_beta_recommendation_runs.audience,owner_beta_recommendation_runs.vendor_snapshot,owner_beta_recommendation_runs.strict_pit,owner_beta_recommendation_runs.candidate_content_sha256,owner_beta_recommendation_runs.artifact_manifest_sha256,owner_beta_recommendation_runs.stage5_manifest_sha256,owner_beta_recommendation_runs.action_manifest_sha256,owner_beta_recommendation_runs.approval_registry_sha256,owner_beta_recommendation_runs.factor_snapshot_sha256,owner_beta_recommendation_runs.error_code,owner_beta_recommendation_runs.created_at,owner_beta_recommendation_runs.started_at,owner_beta_recommendation_runs.finished_at,owner_beta_recommendation_runs.updated_at,owner_beta_recommendation_runs.strategy_id,owner_beta_recommendation_runs.strategy_version,owner_beta_recommendation_runs.strategy_config_json,owner_beta_recommendation_runs.strategy_config_sha256,owner_beta_recommendation_runs.target_snapshot_sha256,owner_beta_recommendation_runs.cash_weight,owner_beta_recommendation_runs.trigger_kind,owner_beta_recommendation_runs.daily_pins,owner_beta_recommendation_runs.daily_config_epoch,owner_beta_recommendation_runs.daily_activation_epoch,owner_beta_recommendation_runs.daily_trigger_version INTO r FROM public.owner_beta_recommendation_runs WHERE job_id=j.id;
    IF r.id IS NULL OR res.run_id IS DISTINCT FROM r.id OR res.owner_user_id IS DISTINCT FROM r.owner_user_id OR j.owner_user_id IS DISTINCT FROM r.owner_user_id
       OR r.trigger_kind IS DISTINCT FROM 'DAILY' OR res.strategy_config_id IS DISTINCT FROM r.strategy_config_id OR res.session_date IS DISTINCT FROM r.as_of
       OR res.trigger_version IS DISTINCT FROM r.daily_trigger_version OR res.config_epoch IS DISTINCT FROM r.daily_config_epoch
       OR res.activation_epoch IS DISTINCT FROM r.daily_activation_epoch OR res.strategy_config_sha256 IS DISTINCT FROM r.strategy_config_sha256
       OR res.admission_sha256 IS DISTINCT FROM r.daily_pins->>'admission_sha256' OR j.payload_json IS DISTINCT FROM public.daily_0060_payload(r)
       OR j.job_type IS DISTINCT FROM 'owner_beta_price_recommendation' OR j.max_attempts<>3 OR j.attempt_count NOT BETWEEN 0 AND 3
    THEN RAISE EXCEPTION 'daily reservation consistency' USING ERRCODE='23514'; END IF;
    SELECT owner_beta_daily_admissions.admission_sha256,owner_beta_daily_admissions.owner_user_id,owner_beta_daily_admissions.policy_sha256,owner_beta_daily_admissions.content_sha256,owner_beta_daily_admissions.manifest_sha256,owner_beta_daily_admissions.parent_admission_sha256,owner_beta_daily_admissions.range_start,owner_beta_daily_admissions.range_end,owner_beta_daily_admissions.session_count,owner_beta_daily_admissions.bar_count,owner_beta_daily_admissions.evidence_json,owner_beta_daily_admissions.admitted_at INTO admitted FROM public.owner_beta_daily_admissions WHERE admission_sha256=res.admission_sha256;
    IF admitted.owner_user_id IS DISTINCT FROM r.owner_user_id OR admitted.range_end IS DISTINCT FROM r.as_of OR r.daily_pins IS DISTINCT FROM
       jsonb_build_object('admission_sha256',admitted.admission_sha256,'content_sha256',admitted.content_sha256,'manifest_sha256',admitted.manifest_sha256,'policy_sha256',admitted.policy_sha256)
    THEN RAISE EXCEPTION 'daily admission pin consistency' USING ERRCODE='23514'; END IF;
    SELECT count(*) INTO running FROM public.job_attempts WHERE job_id=j.id AND outcome='RUNNING';
    IF (SELECT count(*) FROM public.job_attempts WHERE job_id=j.id)<>j.attempt_count
       OR EXISTS(SELECT 1 FROM public.job_attempts WHERE job_id=j.id AND (attempt_no<1 OR attempt_no>j.attempt_count)) THEN
        RAISE EXCEPTION 'daily attempt sequence' USING ERRCODE='23514'; END IF;
    SELECT owner_beta_daily_publication_claims.run_id,owner_beta_daily_publication_claims.job_id,owner_beta_daily_publication_claims.owner_user_id,owner_beta_daily_publication_claims.attempt_id,owner_beta_daily_publication_claims.attempt_no,owner_beta_daily_publication_claims.claimed_by,owner_beta_daily_publication_claims.locked_at,owner_beta_daily_publication_claims.lease_seconds,owner_beta_daily_publication_claims.authorized_at INTO claim FROM public.owner_beta_daily_publication_claims WHERE job_id=j.id;
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
END LOOP;
   IF EXISTS (
     SELECT 1 FROM public.owner_beta_simulation_runs r
     LEFT JOIN public.jobs j ON j.id=r.job_id AND j.owner_user_id=r.owner_user_id
     LEFT JOIN public.user_strategy_configs c ON c.id=r.strategy_config_id AND c.owner_user_id=r.owner_user_id
     LEFT JOIN public.owner_beta_simulation_result_pairs p ON p.run_id=r.id AND p.owner_user_id=r.owner_user_id
     LEFT JOIN public.job_attempts a ON a.id=r.published_attempt_id AND a.job_id=r.job_id
     WHERE r.owner_user_id=actor AND (j.id IS NULL OR c.id IS NULL OR
       j.job_type <> 'owner_beta_price_only_simulation' OR
       NOT public.owner_beta_simulation_version_namespace_matches(j.payload_json->>'schema',
           j.payload_json->>'contract_document_sha256',j.idempotency_key) OR
       NOT public.owner_beta_simulation_payload_matches(j.payload_json,r.id,r.strategy_config_id,
           r.evaluation_start,r.evaluation_end,r.candidate_content_sha256,r.artifact_manifest_sha256,
           r.stage5_manifest_sha256,r.action_manifest_sha256,r.approval_registry_sha256,
           r.contract_document_sha256,r.bars_sha256,r.strategy_id,r.strategy_version,
           r.strategy_config_json,r.strategy_config_sha256) OR
       (r.status='SUCCEEDED' AND (p.run_id IS NULL OR a.id IS NULL OR a.outcome <> 'SUCCEEDED')) OR
       (r.status <> 'SUCCEEDED' AND p.run_id IS NOT NULL) OR
       (p.run_id IS NOT NULL AND (p.illustrative_sha256 <> 'sha256:' ||
          pg_catalog.encode(pg_catalog.sha256(p.illustrative_canonical),'hex') OR
          p.zero_cost_sha256 <> 'sha256:' || pg_catalog.encode(pg_catalog.sha256(p.zero_cost_canonical),'hex'))))
   ) OR EXISTS (
     SELECT 1 FROM public.jobs j LEFT JOIN public.owner_beta_simulation_runs r
       ON r.job_id=j.id AND r.owner_user_id=j.owner_user_id
     WHERE j.owner_user_id=actor AND j.job_type='owner_beta_price_only_simulation' AND
       (r.id IS NULL OR j.status='RUNNING' OR j.locked_by IS NOT NULL OR j.locked_at IS NOT NULL OR
        (j.status='QUEUED' AND r.status <> 'PENDING') OR
        (j.status IN ('SUCCEEDED','FAILED','CANCELED') AND r.status <> j.status))
   ) OR EXISTS (
     SELECT 1 FROM public.job_attempts a JOIN public.jobs j ON j.id=a.job_id
     WHERE j.owner_user_id=actor AND j.job_type='owner_beta_price_only_simulation' AND a.outcome='RUNNING'
   ) THEN RAISE EXCEPTION 'SIMULATION_BACKUP_BINDING'; END IF;

IF EXISTS(SELECT 1 FROM public.pending_targets t LEFT JOIN public.owner_beta_paper_lineage l
 ON l.id=t.beta_lineage_id AND l.owner_user_id=t.owner_user_id AND l.account_id=t.account_id
 AND l.strategy_config_id=t.strategy_config_id AND l.effective_session=t.effective_date
 AND l.recommendation_run_id=t.recommendation_run_id
 LEFT JOIN public.owner_beta_paper_sessions s ON s.pending_target_id=t.id
 AND s.owner_user_id=t.owner_user_id AND s.account_id=t.account_id AND s.effective_session=t.effective_date
 WHERE t.owner_user_id=actor AND t.beta_lineage_id IS NOT NULL AND (l.id IS NULL OR s.pending_target_id IS NULL))
 OR EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions s LEFT JOIN public.pending_targets t
 ON t.id=s.pending_target_id AND t.beta_lineage_id IS NOT NULL AND t.owner_user_id=s.owner_user_id
 AND t.account_id=s.account_id AND t.effective_date=s.effective_session
 WHERE s.owner_user_id=actor AND (t.id IS NULL
 OR (s.settlement_sha256 IS NOT NULL AND s.settlement_sha256 IS DISTINCT FROM s.receipt_json#>>'{settlement,calculation_sha256}')
 OR (s.valuation_sha256 IS NOT NULL AND s.valuation_sha256 IS DISTINCT FROM s.receipt_json#>>'{valuation,calculation_sha256}')))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_SESSION' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.fills f LEFT JOIN public.orders o
 ON o.id=f.order_id AND o.account_id=f.account_id AND o.owner_user_id=f.owner_user_id
 AND o.instrument_id=f.instrument_id AND o.side=f.side
 WHERE f.owner_user_id=actor AND o.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_FILL' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.notification_deliveries d LEFT JOIN public.notifications n
 ON n.id=d.notification_id AND n.owner_user_id=d.owner_user_id
 WHERE d.owner_user_id=actor AND n.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_DELIVERY' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.owner_beta_daily_reservations s LEFT JOIN public.owner_beta_recommendation_runs r
 ON r.id=s.run_id AND r.job_id=s.job_id AND r.owner_user_id=s.owner_user_id
 WHERE s.owner_user_id=actor AND r.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_RESERVATION' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.owner_beta_paper_source_slots s
 JOIN public.owner_beta_paper_source_admissions a ON a.slot_id=s.slot_id
 LEFT JOIN public.owner_beta_daily_admissions d ON d.admission_sha256=a.admission_sha256
 AND d.owner_user_id=s.owner_user_id AND d.range_end=s.session_date
 LEFT JOIN public.owner_beta_paper_source_runs sr ON sr.slot_id=s.slot_id
 LEFT JOIN public.owner_beta_recommendation_runs r ON r.id=sr.run_id
 AND r.owner_user_id=s.owner_user_id AND r.job_id=sr.job_id
 LEFT JOIN public.job_attempts ja ON ja.id=sr.attempt_id AND ja.job_id=sr.job_id
 WHERE s.owner_user_id=actor AND (d.admission_sha256 IS NULL OR
 (sr.slot_id IS NOT NULL AND (r.id IS NULL OR ja.id IS NULL))))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_SOURCE_CHAIN' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.pending_targets t WHERE t.owner_user_id=actor AND t.status<>'PENDING'
 AND NOT EXISTS(SELECT 1 FROM public.paper_settlement_outbox o WHERE o.pending_target_id=t.id AND o.owner_user_id=t.owner_user_id)
 AND NOT EXISTS(SELECT 1 FROM public.paper_settlement_outbox_archive o WHERE o.pending_target_id=t.id AND o.owner_user_id=t.owner_user_id))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OBLIGATION' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.paper_settlement_outbox o LEFT JOIN public.pending_targets t
 ON t.id=o.pending_target_id AND t.owner_user_id=o.owner_user_id
 WHERE o.owner_user_id=actor AND (t.id IS NULL OR t.status='PENDING'))
 OR EXISTS(SELECT 1 FROM public.paper_settlement_outbox_archive o LEFT JOIN public.pending_targets t
 ON t.id=o.pending_target_id AND t.owner_user_id=o.owner_user_id
 WHERE o.owner_user_id=actor AND (t.id IS NULL OR t.status='PENDING'))
 OR EXISTS(SELECT 1 FROM public.paper_settlement_outbox o JOIN public.paper_settlement_outbox_archive a
 ON a.pending_target_id=o.pending_target_id WHERE o.owner_user_id=actor)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_OUTBOX' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM (
 SELECT id,owner_user_id,delivered_at FROM public.paper_settlement_outbox
 UNION ALL SELECT id,owner_user_id,delivered_at FROM public.paper_settlement_outbox_archive) o
 LEFT JOIN public.notifications n ON n.source_key=o.id::text AND n.owner_user_id=o.owner_user_id
 WHERE o.owner_user_id=actor AND o.delivered_at IS NOT NULL AND n.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_NOTIFICATION' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.owner_beta_paper_lineage l LEFT JOIN public.owner_beta_recommendation_runs r
 ON r.id=l.recommendation_run_id AND r.owner_user_id=l.owner_user_id
 AND r.strategy_config_id=l.strategy_config_id AND r.as_of=l.signal_date
 AND r.factor_snapshot_sha256=l.factor_snapshot_sha256 AND r.target_snapshot_sha256=l.target_snapshot_sha256
 LEFT JOIN public.owner_beta_paper_events c ON c.id=l.consent_event_id AND c.owner_user_id=l.owner_user_id AND c.account_id=l.account_id
 LEFT JOIN public.owner_beta_paper_events e ON e.id=l.activation_event_id AND e.owner_user_id=l.owner_user_id AND e.account_id=l.account_id
 WHERE l.owner_user_id=actor AND (r.id IS NULL OR c.id IS NULL OR e.id IS NULL))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_LINEAGE' USING ERRCODE='23514'; END IF;
-- 0067 settlement receipt names concrete economic/outbox rows; deletion is not
-- normalized away. Numeric comparisons are exact PostgreSQL numeric operations.
IF EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions s WHERE s.owner_user_id=actor
 AND s.settlement_sha256 IS NOT NULL AND (
 jsonb_typeof(s.receipt_json->'order_fill_ids') IS DISTINCT FROM 'array'
 OR jsonb_array_length(s.receipt_json->'order_fill_ids') IS DISTINCT FROM jsonb_array_length(s.receipt_json#>'{settlement,orders}')
 OR NOT EXISTS(SELECT 1 FROM public.paper_settlement_outbox o WHERE o.id::text=s.receipt_json->>'outbox_id' AND o.pending_target_id=s.pending_target_id AND o.owner_user_id=s.owner_user_id
 UNION ALL SELECT 1 FROM public.paper_settlement_outbox_archive o WHERE o.id::text=s.receipt_json->>'outbox_id' AND o.pending_target_id=s.pending_target_id AND o.owner_user_id=s.owner_user_id)
 OR EXISTS(SELECT 1 FROM jsonb_array_elements(s.receipt_json->'order_fill_ids') x
 LEFT JOIN public.orders o ON o.id::text=x->>'order_id' AND o.account_id=s.account_id AND o.owner_user_id=s.owner_user_id AND o.order_ref::text=x->>'order_ref'
 LEFT JOIN public.fills f ON f.id::text=x->>'fill_id' AND f.order_id=o.id AND f.account_id=s.account_id AND f.owner_user_id=s.owner_user_id AND f.fill_ref::text=x->>'fill_ref'
 LEFT JOIN public.cash_ledger c ON c.reference_id=f.id AND c.account_id=s.account_id AND c.owner_user_id=s.owner_user_id
 WHERE o.id IS NULL OR f.id IS NULL OR c.id IS NULL)
 OR NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_session_events e WHERE e.pending_target_id=s.pending_target_id AND e.kind IN ('SETTLED_TRADE','SETTLED_NO_TRADE')
 AND e.payload->>'settlement_sha256'=s.settlement_sha256)))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_SETTLEMENT_ROWS' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions s LEFT JOIN public.daily_equity e
 ON e.account_id=s.account_id AND e.owner_user_id=s.owner_user_id AND e.trading_date=s.effective_session
 WHERE s.owner_user_id=actor AND s.valuation_sha256 IS NOT NULL AND (e.id IS NULL
 OR e.equity IS DISTINCT FROM (s.receipt_json#>>'{valuation,equity}')::numeric
 OR e.cash IS DISTINCT FROM (s.receipt_json#>>'{valuation,cash}')::numeric
 OR e.positions_value IS DISTINCT FROM (s.receipt_json#>>'{valuation,positions_value}')::numeric
 OR NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_session_events v WHERE v.pending_target_id=s.pending_target_id AND v.kind='VALUED'
 AND v.payload->>'valuation_sha256'=s.valuation_sha256 AND v.payload->>'daily_equity_id'=e.id::text)))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_VALUATION_ROWS' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.owner_beta_daily_publication_claims c LEFT JOIN public.owner_beta_daily_reservations d ON d.run_id=c.run_id AND d.job_id=c.job_id AND d.owner_user_id=c.owner_user_id
 LEFT JOIN public.job_attempts a ON a.id=c.attempt_id AND a.job_id=c.job_id AND a.attempt_no=c.attempt_no AND a.claimed_by=c.claimed_by
 WHERE c.owner_user_id=actor AND (d.run_id IS NULL OR a.id IS NULL))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_PUBLICATION_ORPHAN' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.owner_beta_paper_lineage l WHERE l.owner_user_id=actor
 AND NOT EXISTS(SELECT 1 FROM public.pending_targets t WHERE t.beta_lineage_id=l.id AND t.owner_user_id=l.owner_user_id AND t.account_id=l.account_id))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_LINEAGE_ORPHAN' USING ERRCODE='23514'; END IF;
 END LOOP;

PERFORM public.paper_0068_validate_receipt(jsonb_build_object('id',t.id,'owner_user_id',t.owner_user_id,'account_id',t.account_id,'control_epoch',t.control_epoch,'kind',t.kind,'actor_user_id',t.actor_user_id,'occurred_at',t.occurred_at,'payload',t.payload,'payload_sha256',t.payload_sha256,'request_key',t.request_key,'pending_target_id',t.pending_target_id,'request_canonical',t.request_canonical,'request_sha256',t.request_sha256,'result_canonical',t.result_canonical,'result_sha256',t.result_sha256),'owner_beta_paper_events') FROM public.owner_beta_paper_events t;
IF EXISTS(SELECT 1 FROM public.owner_beta_paper_events WHERE
 num_nonnulls(request_canonical,request_sha256,result_canonical,result_sha256) NOT IN (0,4)
 OR (request_canonical IS NOT NULL AND (request_sha256 IS DISTINCT FROM public.daily_0060_digest(request_canonical)
 OR result_sha256 IS DISTINCT FROM public.daily_0060_digest(result_canonical))))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_RECEIPT' USING ERRCODE='23514'; END IF;
PERFORM public.paper_0068_validate_receipt(jsonb_build_object('generation',t.generation,'installation_id',t.installation_id,'account_id',t.account_id,'request_key',t.request_key,'purpose',t.purpose,'scope',t.scope,'expected_epoch',t.expected_epoch,'local_epoch_floor',t.local_epoch_floor,'fence_epoch',t.fence_epoch,'release_manifest_sha256',t.release_manifest_sha256,'intent',t.intent,'intent_sha256',t.intent_sha256,'prior_state',t.prior_state,'fence_event_id',t.fence_event_id,'created_at',t.created_at,'request_canonical',t.request_canonical,'request_sha256',t.request_sha256,'result_canonical',t.result_canonical,'result_sha256',t.result_sha256),'owner_beta_paper_operations') FROM public.owner_beta_paper_operations t;
IF EXISTS(SELECT 1 FROM public.owner_beta_paper_operations WHERE
 num_nonnulls(request_canonical,request_sha256,result_canonical,result_sha256) NOT IN (0,4)
 OR (request_canonical IS NOT NULL AND (request_sha256 IS DISTINCT FROM public.daily_0060_digest(request_canonical)
 OR result_sha256 IS DISTINCT FROM public.daily_0060_digest(result_canonical))))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_RECEIPT' USING ERRCODE='23514'; END IF;
PERFORM public.paper_0068_validate_receipt(jsonb_build_object('observation_id',t.observation_id,'generation',t.generation,'request_key',t.request_key,'summary',t.summary,'summary_sha256',t.summary_sha256,'observed_at',t.observed_at,'request_canonical',t.request_canonical,'request_sha256',t.request_sha256,'result_canonical',t.result_canonical,'result_sha256',t.result_sha256),'owner_beta_paper_operation_observations') FROM public.owner_beta_paper_operation_observations t;
IF EXISTS(SELECT 1 FROM public.owner_beta_paper_operation_observations WHERE
 num_nonnulls(request_canonical,request_sha256,result_canonical,result_sha256) NOT IN (0,4)
 OR (request_canonical IS NOT NULL AND (request_sha256 IS DISTINCT FROM public.daily_0060_digest(request_canonical)
 OR result_sha256 IS DISTINCT FROM public.daily_0060_digest(result_canonical))))
 THEN RAISE EXCEPTION 'PAPER_AUDIT_RECEIPT' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.owner_beta_paper_source_admissions a LEFT JOIN public.owner_beta_paper_source_slots s ON s.slot_id=a.slot_id
 LEFT JOIN public.owner_beta_daily_admissions d ON d.admission_sha256=a.admission_sha256 AND d.owner_user_id=s.owner_user_id
 WHERE s.slot_id IS NULL OR d.admission_sha256 IS NULL)
 OR EXISTS(SELECT 1 FROM public.owner_beta_paper_source_originals o LEFT JOIN public.owner_beta_paper_source_admissions a ON a.slot_id=o.slot_id WHERE a.slot_id IS NULL)
 OR EXISTS(SELECT 1 FROM public.owner_beta_paper_source_runs r LEFT JOIN public.owner_beta_paper_source_admissions a ON a.slot_id=r.slot_id WHERE a.slot_id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_SOURCE_ORPHAN' USING ERRCODE='23514'; END IF;
IF EXISTS(SELECT 1 FROM public.owner_beta_paper_session_events e LEFT JOIN public.owner_beta_paper_sessions s ON s.pending_target_id=e.pending_target_id WHERE s.pending_target_id IS NULL)
 OR EXISTS(SELECT 1 FROM public.owner_beta_paper_operation_observations o LEFT JOIN public.owner_beta_paper_operations p ON p.generation=o.generation WHERE p.generation IS NULL)
 OR EXISTS(SELECT 1 FROM public.owner_beta_paper_recovery_grants g LEFT JOIN public.owner_beta_paper_sessions s
 ON s.pending_target_id=g.pending_target_id AND s.account_id=g.account_id AND s.effective_session=g.effective_session
 LEFT JOIN public.owner_beta_paper_lineage l ON l.id=g.lineage_id AND l.account_id=g.account_id AND l.effective_session=g.effective_session
 LEFT JOIN public.owner_beta_paper_events e ON e.id=g.consent_event_id AND e.account_id=g.account_id
 WHERE s.pending_target_id IS NULL OR l.id IS NULL OR e.id IS NULL)
 THEN RAISE EXCEPTION 'PAPER_AUDIT_HISTORY_ORPHAN' USING ERRCODE='23514'; END IF;

PERFORM pg_catalog.set_config('app.actor_user_id',coalesce(old_actor,''),true);
EXCEPTION WHEN query_canceled OR OTHERS THEN
 PERFORM pg_catalog.set_config('app.actor_user_id',coalesce(old_actor,''),true);
 RAISE;
END $relations$;


SELECT '["header","owner-beta-paper-db-stream-v1","sha256:56dbff10f7e1738c08778758b250057631c7dc8c02155cd669515bfaebd08ff4"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.event_sha256::text row_key, jsonb_build_object('event_sha256',t.event_sha256, 'admission_sha256',t.admission_sha256, 'owner_user_id',t.owner_user_id, 'kind',t.kind, 'evidence_sha256',t.evidence_sha256, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_daily_admission_events t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','daily','owner_beta_daily_admission_events',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","daily","owner_beta_daily_admission_events"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.admission_sha256::text row_key, jsonb_build_object('admission_sha256',t.admission_sha256, 'owner_user_id',t.owner_user_id, 'policy_sha256',t.policy_sha256, 'content_sha256',t.content_sha256, 'manifest_sha256',t.manifest_sha256, 'parent_admission_sha256',t.parent_admission_sha256, 'range_start',to_char(t.range_start,'YYYY-MM-DD'), 'range_end',to_char(t.range_end,'YYYY-MM-DD'), 'session_count',t.session_count, 'bar_count',t.bar_count, 'evidence_json',t.evidence_json, 'admitted_at',to_char(t.admitted_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_daily_admissions t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','daily','owner_beta_daily_admissions',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","daily","owner_beta_daily_admissions"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.owner_user_id::text||':'||t.calendar_evidence_sha256::text||':'||t.session_date::text row_key, jsonb_build_object('owner_user_id',t.owner_user_id, 'calendar_evidence_sha256',t.calendar_evidence_sha256, 'session_date',to_char(t.session_date,'YYYY-MM-DD'), 'session_type',t.session_type, 'evidence_kind',t.evidence_kind, 'source_row_ordinal',t.source_row_ordinal, 'source_row_sha256',t.source_row_sha256) row_value FROM public.owner_beta_daily_calendar_days t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','daily','owner_beta_daily_calendar_days',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","daily","owner_beta_daily_calendar_days"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.calendar_evidence_sha256::text row_key, jsonb_build_object('calendar_evidence_sha256',t.calendar_evidence_sha256, 'owner_user_id',t.owner_user_id, 'policy_sha256',t.policy_sha256, 'normalizer_version',t.normalizer_version, 'source_provider',t.source_provider, 'source_market',t.source_market, 'source_batch_id',t.source_batch_id, 'source_batch_date',to_char(t.source_batch_date,'YYYY-MM-DD'), 'requested_target',to_char(t.requested_target,'YYYY-MM-DD'), 'source_file_name',t.source_file_name, 'source_file_sha256',t.source_file_sha256, 'source_manifest_line_sha256',t.source_manifest_line_sha256, 'source_retrieved_at',to_char(t.source_retrieved_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'copy_chain',t.copy_chain, 'evidence_json',t.evidence_json, 'projected_at',to_char(t.projected_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_daily_calendar_sources t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','daily','owner_beta_daily_calendar_sources',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","daily","owner_beta_daily_calendar_sources"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'enabled',t.enabled, 'activation_epoch',t.activation_epoch, 'policy_sha256',t.policy_sha256, 'policy_json',t.policy_json, 'entitlement_reference',t.entitlement_reference, 'entitlement_sha256',t.entitlement_sha256, 'first_auto_session',to_char(t.first_auto_session,'YYYY-MM-DD'), 'trigger_version',t.trigger_version, 'max_jobs_per_owner',t.max_jobs_per_owner, 'lease_seconds',t.lease_seconds, 'backoff_seconds',t.backoff_seconds, 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_daily_control t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','daily','owner_beta_daily_control',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","daily","owner_beta_daily_control"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.receipt_id::text row_key, jsonb_build_object('receipt_id',t.receipt_id, 'stage',t.stage, 'owner_user_id',t.owner_user_id, 'policy_sha256',t.policy_sha256, 'activation_epoch',t.activation_epoch, 'started_at',to_char(t.started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'completed_at',to_char(t.completed_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'result_code',t.result_code, 'source_frontier_sha256',t.source_frontier_sha256, 'admission_sha256',t.admission_sha256, 'event_sha256',t.event_sha256, 'completion_binding_sha256',t.completion_binding_sha256) row_value FROM public.owner_beta_daily_poll_receipts t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','daily','owner_beta_daily_poll_receipts',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","daily","owner_beta_daily_poll_receipts"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.stage::text row_key, jsonb_build_object('stage',t.stage, 'next_poll_at',to_char(t.next_poll_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'last_completed_at',to_char(t.last_completed_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'last_result_code',t.last_result_code, 'verified_admission_sha256',t.verified_admission_sha256, 'source_frontier_sha256',t.source_frontier_sha256, 'latest_receipt_id',t.latest_receipt_id) row_value FROM public.owner_beta_daily_poll_state t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','daily','owner_beta_daily_poll_state',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","daily","owner_beta_daily_poll_state"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.run_id::text row_key, jsonb_build_object('run_id',t.run_id, 'job_id',t.job_id, 'owner_user_id',t.owner_user_id, 'attempt_id',t.attempt_id, 'attempt_no',t.attempt_no, 'claimed_by',t.claimed_by, 'locked_at',to_char(t.locked_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'lease_seconds',t.lease_seconds, 'authorized_at',to_char(t.authorized_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_daily_publication_claims t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','daily','owner_beta_daily_publication_claims',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","daily","owner_beta_daily_publication_claims"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.owner_user_id::text||':'||t.strategy_config_id::text||':'||t.session_date::text||':'||t.trigger_version::text row_key, jsonb_build_object('owner_user_id',t.owner_user_id, 'strategy_config_id',t.strategy_config_id, 'session_date',to_char(t.session_date,'YYYY-MM-DD'), 'trigger_version',t.trigger_version, 'run_id',t.run_id, 'job_id',t.job_id, 'admission_sha256',t.admission_sha256, 'strategy_config_sha256',t.strategy_config_sha256, 'config_epoch',t.config_epoch, 'activation_epoch',t.activation_epoch, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_daily_reservations t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','daily','owner_beta_daily_reservations',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","daily","owner_beta_daily_reservations"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.event_sha256::text row_key, jsonb_build_object('event_sha256',t.event_sha256, 'owner_user_id',t.owner_user_id, 'source_kind',t.source_kind, 'range_start',to_char(t.range_start,'YYYY-MM-DD'), 'range_end',to_char(t.range_end,'YYYY-MM-DD'), 'code',t.code, 'evidence_sha256',t.evidence_sha256, 'admission_sha256',t.admission_sha256, 'recorded_at',to_char(t.recorded_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_daily_source_events t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','daily','owner_beta_daily_source_events',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","daily","owner_beta_daily_source_events"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'recommendation_run_id',t.recommendation_run_id, 'owner_user_id',t.owner_user_id, 'instrument_id',t.instrument_id, 'rank',t.rank, 'target_weight',t.target_weight, 'reason_codes',t.reason_codes, 'factors_json',t.factors_json, 'excluded',t.excluded, 'exclusion_reason',t.exclusion_reason, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value
   FROM public.owner_beta_recommendation_items t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','daily','owner_beta_recommendation_items',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","daily","owner_beta_recommendation_items"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'strategy_config_id',t.strategy_config_id, 'job_id',t.job_id, 'as_of',to_char(t.as_of,'YYYY-MM-DD'), 'status',t.status, 'input_kind',t.input_kind, 'capability',t.capability, 'audience',t.audience, 'vendor_snapshot',t.vendor_snapshot, 'strict_pit',t.strict_pit, 'candidate_content_sha256',t.candidate_content_sha256, 'artifact_manifest_sha256',t.artifact_manifest_sha256, 'stage5_manifest_sha256',t.stage5_manifest_sha256, 'action_manifest_sha256',t.action_manifest_sha256, 'approval_registry_sha256',t.approval_registry_sha256, 'factor_snapshot_sha256',t.factor_snapshot_sha256, 'error_code',t.error_code, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'started_at',to_char(t.started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'finished_at',to_char(t.finished_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'strategy_id',t.strategy_id, 'strategy_version',t.strategy_version, 'strategy_config_json',t.strategy_config_json, 'strategy_config_sha256',t.strategy_config_sha256, 'target_snapshot_sha256',t.target_snapshot_sha256, 'cash_weight',t.cash_weight, 'trigger_kind',t.trigger_kind, 'daily_pins',t.daily_pins, 'daily_config_epoch',t.daily_config_epoch, 'daily_activation_epoch',t.daily_activation_epoch, 'daily_trigger_version',t.daily_trigger_version) row_value
   FROM public.owner_beta_recommendation_runs t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','daily','owner_beta_recommendation_runs',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","daily","owner_beta_recommendation_runs"]'::text AS audit_record;

SELECT '["group_end","daily"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'account_id',t.account_id, 'owner_user_id',t.owner_user_id, 'strategy_config_id',t.strategy_config_id, 'strategy_id',t.strategy_id, 'strategy_version',t.strategy_version, 'bound_at',to_char(t.bound_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'unbound_at',to_char(t.unbound_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'auto_apply_recommendations',t.auto_apply_recommendations) row_value
   FROM public.account_strategy_bindings t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_control','account_strategy_bindings',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_control","account_strategy_bindings"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'account_type',t.account_type, 'name',t.name, 'currency',t.currency, 'status',t.status, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'initial_cash',t.initial_cash::text, 'cost_profile_id',t.cost_profile_id, 'cost_profile_version',t.cost_profile_version, 'paper_state_version',t.paper_state_version) row_value
   FROM public.accounts t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_control','accounts',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_control","accounts"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'account_id',t.account_id, 'strategy_config_id',t.strategy_config_id, 'state',t.state, 'control_epoch',t.control_epoch, 'contract_id',t.contract_id, 'contract_sha256',t.contract_sha256, 'contract_json',t.contract_json, 'cost_profile_id',t.cost_profile_id, 'cost_profile_version',t.cost_profile_version, 'cost_profile_sha256',t.cost_profile_sha256, 'cohort_sha256',t.cohort_sha256, 'daily_cohort',t.daily_cohort, 'process_binding',t.process_binding, 'process_binding_sha256',t.process_binding_sha256, 'consent_event_id',t.consent_event_id, 'activation_event_id',t.activation_event_id, 'last_signal_allocation_sha256',t.last_signal_allocation_sha256, 'last_signal_date',to_char(t.last_signal_date,'YYYY-MM-DD'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_paper_control t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_control','owner_beta_paper_control',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_control","owner_beta_paper_control"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'account_id',t.account_id, 'control_epoch',t.control_epoch, 'kind',t.kind, 'actor_user_id',t.actor_user_id, 'occurred_at',to_char(t.occurred_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'payload',t.payload, 'payload_sha256',t.payload_sha256, 'request_key',t.request_key, 'pending_target_id',t.pending_target_id, 'request_canonical',t.request_canonical, 'request_sha256',t.request_sha256, 'result_canonical',t.result_canonical, 'result_sha256',t.result_sha256) row_value FROM public.owner_beta_paper_events t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_control','owner_beta_paper_events',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_control","owner_beta_paper_events"]'::text AS audit_record;

SELECT '["group_end","paper_control"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'job_id',t.job_id, 'attempt_no',t.attempt_no, 'outcome',t.outcome, 'claimed_by',t.claimed_by, 'error_code',t.error_code, 'error_message',CASE WHEN t.error_message IS NULL THEN NULL ELSE 'sha256:'||encode(sha256(convert_to(t.error_message,'UTF8')),'hex') END, 'started_at',to_char(t.started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'finished_at',to_char(t.finished_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.job_attempts t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_jobs','job_attempts',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_jobs","job_attempts"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'job_type',t.job_type, 'status',t.status, 'priority',t.priority, 'idempotency_key',t.idempotency_key, 'payload_json',t.payload_json, 'max_attempts',t.max_attempts, 'attempt_count',t.attempt_count, 'available_at',to_char(t.available_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'locked_by',t.locked_by, 'locked_at',to_char(t.locked_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'started_at',to_char(t.started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'finished_at',to_char(t.finished_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'error_code',t.error_code, 'error_message',CASE WHEN t.error_message IS NULL THEN NULL ELSE 'sha256:'||encode(sha256(convert_to(t.error_message,'UTF8')),'hex') END, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value
   FROM public.jobs t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_jobs','jobs',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_jobs","jobs"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'account_id',t.account_id, 'recommendation_run_id',t.recommendation_run_id, 'target_portfolio_id',t.target_portfolio_id, 'strategy_config_id',t.strategy_config_id, 'job_id',t.job_id, 'status',t.status, 'price_basis',t.price_basis, 'price_date',to_char(t.price_date,'YYYY-MM-DD'), 'preview_seoul_date',to_char(t.preview_seoul_date,'YYYY-MM-DD'), 'proposed_effective_date',to_char(t.proposed_effective_date,'YYYY-MM-DD'), 'dataset_version_id',t.dataset_version_id, 'dataset_manifest_sha256',t.dataset_manifest_sha256, 'cost_profile_id',t.cost_profile_id, 'cost_profile_version',t.cost_profile_version, 'account_state_version',t.account_state_version, 'account_state_sha256',t.account_state_sha256, 'target_portfolio_sha256',t.target_portfolio_sha256, 'preview_token',t.preview_token, 'result_json',t.result_json, 'error_json',CASE WHEN t.error_json IS NULL THEN NULL ELSE jsonb_build_object('sha256','sha256:'||encode(sha256(convert_to(public.paper_0068_canonical_row_json(t.error_json),'UTF8')),'hex')) END, 'pending_target_id',t.pending_target_id, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'started_at',to_char(t.started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'completed_at',to_char(t.completed_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'applied_at',to_char(t.applied_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'input_kind',t.input_kind, 'beta_apply_mode',t.beta_apply_mode, 'beta_lineage_id',t.beta_lineage_id, 'beta_request_key',t.beta_request_key, 'account_state_preimage',t.account_state_preimage, 'signal_allocation_sha256',t.signal_allocation_sha256) row_value FROM public.paper_rebalance_previews t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_jobs','paper_rebalance_previews',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_jobs","paper_rebalance_previews"]'::text AS audit_record;

SELECT '["group_end","paper_jobs"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.account_id::text||':'||t.seq::text row_key, jsonb_build_object('id',t.id, 'account_id',t.account_id, 'owner_user_id',t.owner_user_id, 'seq',t.seq, 'event_type',t.event_type, 'amount',t.amount::text, 'balance',t.balance::text, 'currency',t.currency, 'reference_id',t.reference_id, 'ts',to_char(t.ts AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value
   FROM public.cash_ledger t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_ledger','cash_ledger',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_ledger","cash_ledger"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.account_id::text||':'||t.trading_date::text row_key, jsonb_build_object('id',t.id, 'account_id',t.account_id, 'owner_user_id',t.owner_user_id, 'trading_date',to_char(t.trading_date,'YYYY-MM-DD'), 'equity',t.equity::text, 'cash',t.cash::text, 'positions_value',t.positions_value::text, 'currency',t.currency, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value
   FROM public.daily_equity t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_ledger','daily_equity',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_ledger","daily_equity"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.account_id::text||':'||t.fill_ref::text row_key, jsonb_build_object('id',t.id, 'account_id',t.account_id, 'owner_user_id',t.owner_user_id, 'order_id',t.order_id, 'instrument_id',t.instrument_id, 'fill_ref',t.fill_ref, 'side',t.side, 'quantity',t.quantity::text, 'price',t.price::text, 'fees',t.fees::text, 'ts',to_char(t.ts AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value
   FROM public.fills t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_ledger','fills',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_ledger","fills"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.account_id::text||':'||t.order_ref::text row_key, jsonb_build_object('id',t.id, 'account_id',t.account_id, 'owner_user_id',t.owner_user_id, 'order_ref',t.order_ref, 'instrument_id',t.instrument_id, 'side',t.side, 'quantity',t.quantity::text, 'price',t.price::text, 'status',t.status, 'submitted_at',to_char(t.submitted_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value
   FROM public.orders t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_ledger','orders',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_ledger","orders"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.account_id::text||':'||t.instrument_id::text row_key, jsonb_build_object('id',t.id, 'account_id',t.account_id, 'owner_user_id',t.owner_user_id, 'instrument_id',t.instrument_id, 'quantity',t.quantity::text, 'avg_price',t.avg_price::text, 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value
   FROM public.positions t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_ledger','positions',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_ledger","positions"]'::text AS audit_record;

SELECT '["group_end","paper_ledger"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'account_id',t.account_id, 'strategy_config_id',t.strategy_config_id, 'recommendation_run_id',t.recommendation_run_id, 'signal_date',to_char(t.signal_date,'YYYY-MM-DD'), 'effective_session',to_char(t.effective_session,'YYYY-MM-DD'), 'signal_admission_sha256',t.signal_admission_sha256, 'factor_snapshot_sha256',t.factor_snapshot_sha256, 'target_snapshot_sha256',t.target_snapshot_sha256, 'target_weights_json',t.target_weights_json, 'config_epoch',t.config_epoch, 'daily_activation_epoch',t.daily_activation_epoch, 'control_epoch',t.control_epoch, 'consent_event_id',t.consent_event_id, 'activation_event_id',t.activation_event_id, 'cohort_sha256',t.cohort_sha256, 'calendar_evidence_sha256',t.calendar_evidence_sha256, 'calendar_link_json',t.calendar_link_json, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_paper_lineage t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_lineage','owner_beta_paper_lineage',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_lineage","owner_beta_paper_lineage"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.pending_target_id::text||':'||lpad(t.event_seq::text,20,'0') row_key, jsonb_build_object('id',t.id, 'pending_target_id',t.pending_target_id, 'event_seq',t.event_seq, 'kind',t.kind, 'reason_code',t.reason_code, 'payload',t.payload, 'occurred_at',to_char(t.occurred_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_paper_session_events t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_lineage','owner_beta_paper_session_events',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_lineage","owner_beta_paper_session_events"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.pending_target_id::text row_key, jsonb_build_object('pending_target_id',t.pending_target_id, 'owner_user_id',t.owner_user_id, 'account_id',t.account_id, 'effective_session',to_char(t.effective_session,'YYYY-MM-DD'), 'account_state_version',t.account_state_version, 'account_state_sha256',t.account_state_sha256, 'state',t.state, 'settlement_kind',t.settlement_kind, 'reason_code',t.reason_code, 'execution_admission_sha256',t.execution_admission_sha256, 'valuation_admission_sha256',t.valuation_admission_sha256, 'execution_input_sha256',t.execution_input_sha256, 'valuation_input_sha256',t.valuation_input_sha256, 'ledger_before_sha256',t.ledger_before_sha256, 'ledger_after_sha256',t.ledger_after_sha256, 'ledger_after_version',t.ledger_after_version, 'ledger_after_preimage',t.ledger_after_preimage, 'settlement_sha256',t.settlement_sha256, 'valuation_sha256',t.valuation_sha256, 'source_original_retrieved_at',to_char(t.source_original_retrieved_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'input_admitted_at',to_char(t.input_admitted_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'usable_at',to_char(t.usable_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'settled_at',to_char(t.settled_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'valued_at',to_char(t.valued_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'processing_origin',t.processing_origin, 'receipt_json',t.receipt_json, 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_paper_sessions t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_lineage','owner_beta_paper_sessions',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_lineage","owner_beta_paper_sessions"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.slot_id::text row_key, jsonb_build_object('slot_id',t.slot_id, 'source_event_sha256',t.source_event_sha256, 'original_set_sha256',t.original_set_sha256, 'admission_sha256',t.admission_sha256, 'linked_at',to_char(t.linked_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_paper_source_admissions t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_lineage','owner_beta_paper_source_admissions',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_lineage","owner_beta_paper_source_admissions"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.provider::text||':'||t.market::text||':'||t.batch_id::text||':'||t.file_name::text row_key, jsonb_build_object('provider',t.provider, 'market',t.market, 'batch_id',t.batch_id, 'file_name',t.file_name, 'wire_sha256',t.wire_sha256, 'slot_id',t.slot_id) row_value FROM public.owner_beta_paper_source_originals t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_lineage','owner_beta_paper_source_originals',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_lineage","owner_beta_paper_source_originals"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.slot_id::text row_key, jsonb_build_object('slot_id',t.slot_id, 'run_id',t.run_id, 'job_id',t.job_id, 'attempt_id',t.attempt_id, 'linked_at',to_char(t.linked_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_paper_source_runs t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_lineage','owner_beta_paper_source_runs',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_lineage","owner_beta_paper_source_runs"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.slot_id::text row_key, jsonb_build_object('slot_id',t.slot_id, 'reservation_id',t.reservation_id, 'root_event_sha256',t.root_event_sha256, 'owner_user_id',t.owner_user_id, 'policy_sha256',t.policy_sha256, 'activation_epoch',t.activation_epoch, 'session_date',to_char(t.session_date,'YYYY-MM-DD'), 'cohort_sha256',t.cohort_sha256, 'cohort',t.cohort, 'reserved_at',to_char(t.reserved_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_paper_source_slots t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_lineage','owner_beta_paper_source_slots',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_lineage","owner_beta_paper_source_slots"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'account_id',t.account_id, 'recommendation_run_id',t.recommendation_run_id, 'target_portfolio_id',t.target_portfolio_id, 'strategy_config_id',t.strategy_config_id, 'job_id',t.job_id, 'status',t.status, 'price_basis',t.price_basis, 'price_date',to_char(t.price_date,'YYYY-MM-DD'), 'preview_seoul_date',to_char(t.preview_seoul_date,'YYYY-MM-DD'), 'proposed_effective_date',to_char(t.proposed_effective_date,'YYYY-MM-DD'), 'dataset_version_id',t.dataset_version_id, 'dataset_manifest_sha256',t.dataset_manifest_sha256, 'cost_profile_id',t.cost_profile_id, 'cost_profile_version',t.cost_profile_version, 'account_state_version',t.account_state_version, 'account_state_sha256',t.account_state_sha256, 'target_portfolio_sha256',t.target_portfolio_sha256, 'preview_token',t.preview_token, 'result_json',t.result_json, 'error_json',CASE WHEN t.error_json IS NULL THEN NULL ELSE jsonb_build_object('sha256','sha256:'||encode(sha256(convert_to(public.paper_0068_canonical_row_json(t.error_json),'UTF8')),'hex')) END, 'pending_target_id',t.pending_target_id, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'started_at',to_char(t.started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'completed_at',to_char(t.completed_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'applied_at',to_char(t.applied_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'input_kind',t.input_kind, 'beta_apply_mode',t.beta_apply_mode, 'beta_lineage_id',t.beta_lineage_id, 'beta_request_key',t.beta_request_key, 'account_state_preimage',t.account_state_preimage, 'signal_allocation_sha256',t.signal_allocation_sha256) row_value FROM public.paper_rebalance_previews t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_lineage','paper_rebalance_previews',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_lineage","paper_rebalance_previews"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'account_id',t.account_id, 'owner_user_id',t.owner_user_id, 'strategy_config_id',t.strategy_config_id, 'computed_on',to_char(t.computed_on,'YYYY-MM-DD'), 'effective_date',to_char(t.effective_date,'YYYY-MM-DD'), 'targets_json',t.targets_json, 'status',t.status, 'executed_at',to_char(t.executed_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'dataset_version',t.dataset_version, 'dataset_version_id',t.dataset_version_id, 'dataset_manifest_sha256',t.dataset_manifest_sha256, 'non_execution_reason',t.non_execution_reason, 'source_kind',t.source_kind, 'recommendation_run_id',t.recommendation_run_id, 'beta_lineage_id',t.beta_lineage_id) row_value
   FROM public.pending_targets t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_lineage','pending_targets',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_lineage","pending_targets"]'::text AS audit_record;

SELECT '["group_end","paper_lineage"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.observation_id::text row_key, jsonb_build_object('observation_id',t.observation_id, 'generation',t.generation, 'request_key',t.request_key, 'summary',t.summary, 'summary_sha256',t.summary_sha256, 'observed_at',to_char(t.observed_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'request_canonical',t.request_canonical, 'request_sha256',t.request_sha256, 'result_canonical',t.result_canonical, 'result_sha256',t.result_sha256) row_value FROM public.owner_beta_paper_operation_observations t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_operations','owner_beta_paper_operation_observations',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_operations","owner_beta_paper_operation_observations"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.generation::text row_key, jsonb_build_object('generation',t.generation, 'installation_id',t.installation_id, 'account_id',t.account_id, 'request_key',t.request_key, 'purpose',t.purpose, 'scope',t.scope, 'expected_epoch',t.expected_epoch, 'local_epoch_floor',t.local_epoch_floor, 'fence_epoch',t.fence_epoch, 'release_manifest_sha256',t.release_manifest_sha256, 'intent',t.intent, 'intent_sha256',t.intent_sha256, 'prior_state',t.prior_state, 'fence_event_id',t.fence_event_id, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'request_canonical',t.request_canonical, 'request_sha256',t.request_sha256, 'result_canonical',t.result_canonical, 'result_sha256',t.result_sha256) row_value FROM public.owner_beta_paper_operations t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_operations','owner_beta_paper_operations',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_operations","owner_beta_paper_operations"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.grant_id::text row_key, jsonb_build_object('grant_id',t.grant_id, 'account_id',t.account_id, 'pending_target_id',t.pending_target_id, 'lineage_id',t.lineage_id, 'effective_session',to_char(t.effective_session,'YYYY-MM-DD'), 'original_receipt_sha256',t.original_receipt_sha256, 'lineage_epoch',t.lineage_epoch, 'active_epoch',t.active_epoch, 'consent_event_id',t.consent_event_id, 'process_config_sha256',t.process_config_sha256, 'decision',t.decision, 'decision_sha256',t.decision_sha256, 'step',t.step, 'signal_admission_sha256',t.signal_admission_sha256, 'execution_admission_sha256',t.execution_admission_sha256, 'expires_at',to_char(t.expires_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'request_key',t.request_key, 'settlement_receipt_sha256',t.settlement_receipt_sha256, 'valuation_receipt_sha256',t.valuation_receipt_sha256, 'revoked_at',to_char(t.revoked_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_paper_recovery_grants t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_operations','owner_beta_paper_recovery_grants',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_operations","owner_beta_paper_recovery_grants"]'::text AS audit_record;

SELECT '["group_end","paper_operations"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.notification_id::text||':'||t.channel::text row_key, jsonb_build_object('id',t.id, 'notification_id',t.notification_id, 'owner_user_id',t.owner_user_id, 'channel',t.channel, 'status',t.status, 'error_detail',CASE WHEN t.error_detail IS NULL THEN NULL ELSE 'sha256:'||encode(sha256(convert_to(t.error_detail,'UTF8')),'hex') END, 'attempted_at',to_char(t.attempted_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'delivery_token',t.delivery_token, 'delivery_lease_expires_at',to_char(t.delivery_lease_expires_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'delivery_attempts',t.delivery_attempts) row_value
   FROM public.notification_deliveries t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_outbox','notification_deliveries',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_outbox","notification_deliveries"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'kind',t.kind, 'title',t.title, 'body',t.body, 'read_at',to_char(t.read_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'source_key',t.source_key) row_value
   FROM public.notifications t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_outbox','notifications',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_outbox","notifications"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'pending_target_id',t.pending_target_id, 'owner_user_id',t.owner_user_id, 'severity',t.severity, 'kind',t.kind, 'title',t.title, 'body',t.body, 'parity_json',t.parity_json, 'attempts',t.attempts, 'max_attempts',t.max_attempts, 'available_at',to_char(t.available_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'delivered_at',to_char(t.delivered_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'exhausted_at',to_char(t.exhausted_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'last_error',CASE WHEN t.last_error IS NULL THEN NULL ELSE 'sha256:'||encode(sha256(convert_to(t.last_error,'UTF8')),'hex') END, 'claim_token',t.claim_token, 'claim_expires_at',to_char(t.claim_expires_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.paper_settlement_outbox t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_outbox','paper_settlement_outbox',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_outbox","paper_settlement_outbox"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'pending_target_id',t.pending_target_id, 'owner_user_id',t.owner_user_id, 'severity',t.severity, 'kind',t.kind, 'title',t.title, 'body',t.body, 'parity_json',t.parity_json, 'attempts',t.attempts, 'max_attempts',t.max_attempts, 'delivered_at',to_char(t.delivered_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'archived_at',to_char(t.archived_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.paper_settlement_outbox_archive t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','paper_outbox','paper_settlement_outbox_archive',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","paper_outbox","paper_settlement_outbox_archive"]'::text AS audit_record;

SELECT '["group_end","paper_outbox"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT m.version::text row_key,jsonb_build_object('version',m.version, 'description',m.description, 'installed_on',to_char(m.installed_on AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'success',m.success, 'checksum',encode(m.checksum,'hex'), 'execution_time',m.execution_time) row_value FROM public._sqlx_migrations m) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','schema_security','_sqlx_migrations',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","schema_security","_sqlx_migrations"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT c.relname||':'||lpad(((SELECT count(*) FROM pg_attribute z WHERE z.attrelid=a.attrelid AND z.attnum>0 AND NOT z.attisdropped AND z.attnum<=a.attnum))::text,6,'0') row_key,jsonb_build_object('table',c.relname, 'column',a.attname, 'ordinal',(SELECT count(*) FROM pg_attribute z WHERE z.attrelid=a.attrelid AND z.attnum>0 AND NOT z.attisdropped AND z.attnum<=a.attnum), 'type',format_type(a.atttypid,a.atttypmod), 'not_null',a.attnotnull, 'identity',a.attidentity, 'generated',a.attgenerated, 'acl',(SELECT jsonb_agg(v::text ORDER BY v::text COLLATE "C") FROM unnest(a.attacl) v), 'default',pg_get_expr(d.adbin,d.adrelid), 'collation',CASE WHEN a.attcollation=0 THEN NULL ELSE a.attcollation::regcollation::text END) row_value FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum WHERE n.nspname='public' AND a.attnum>0 AND NOT a.attisdropped) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','schema_security','pg_attribute',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","schema_security","pg_attribute"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.rolname||':'||u.rolname||':'||g.rolname row_key,jsonb_build_object('role',r.rolname, 'member',u.rolname, 'grantor',g.rolname, 'admin_option',m.admin_option, 'inherit_option',m.inherit_option, 'set_option',m.set_option) row_value FROM pg_auth_members m JOIN pg_roles r ON r.oid=m.roleid JOIN pg_roles u ON u.oid=m.member JOIN pg_roles g ON g.oid=m.grantor) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','schema_security','pg_auth_members',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","schema_security","pg_auth_members"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT c.relname row_key,jsonb_build_object('name',c.relname, 'kind',c.relkind, 'owner',pg_get_userbyid(c.relowner), 'acl',(SELECT jsonb_agg(v::text ORDER BY v::text COLLATE "C") FROM unnest(c.relacl) v), 'row_security',c.relrowsecurity, 'force_row_security',c.relforcerowsecurity, 'options',c.reloptions) row_value FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public') r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','schema_security','pg_class',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","schema_security","pg_class"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT c.relname||':'||x.conname row_key,jsonb_build_object('table',c.relname, 'name',x.conname, 'type',x.contype, 'definition',pg_get_constraintdef(x.oid), 'validated',x.convalidated, 'deferrable',x.condeferrable, 'deferred',x.condeferred) row_value FROM pg_constraint x JOIN pg_class c ON c.oid=x.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public') r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','schema_security','pg_constraint',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","schema_security","pg_constraint"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT 'database'::text row_key,jsonb_build_object('owner',pg_get_userbyid(d.datdba), 'acl',(SELECT jsonb_agg(v::text ORDER BY v::text COLLATE "C") FROM unnest(d.datacl) v), 'encoding',pg_encoding_to_char(d.encoding), 'collate',d.datcollate, 'ctype',d.datctype, 'allow_connections',d.datallowconn, 'connection_limit',d.datconnlimit) row_value FROM pg_database d WHERE d.datname=current_database()) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','schema_security','pg_database',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","schema_security","pg_database"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT pg_get_userbyid(a.defaclrole)||':'||coalesce(n.nspname,'')||':'||a.defaclobjtype row_key,jsonb_build_object('owner',pg_get_userbyid(a.defaclrole), 'schema',n.nspname, 'kind',a.defaclobjtype, 'acl',(SELECT jsonb_agg(v::text ORDER BY v::text COLLATE "C") FROM unnest(a.defaclacl) v)) row_value FROM pg_default_acl a LEFT JOIN pg_namespace n ON n.oid=a.defaclnamespace WHERE a.defaclnamespace=0 OR n.nspname='public') r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','schema_security','pg_default_acl',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","schema_security","pg_default_acl"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT c.relname row_key,jsonb_build_object('name',c.relname, 'definition',pg_get_indexdef(i.indexrelid), 'valid',i.indisvalid, 'ready',i.indisready, 'unique',i.indisunique, 'primary',i.indisprimary) row_value FROM pg_index i JOIN pg_class c ON c.oid=i.indexrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public') r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','schema_security','pg_index',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","schema_security","pg_index"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT n.nspname row_key,jsonb_build_object('name',n.nspname, 'owner',pg_get_userbyid(n.nspowner), 'acl',(SELECT jsonb_agg(v::text ORDER BY v::text COLLATE "C") FROM unnest(n.nspacl) v)) row_value FROM pg_namespace n WHERE n.nspname='public') r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','schema_security','pg_namespace',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","schema_security","pg_namespace"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT c.relname||':'||p.polname row_key,jsonb_build_object('table',c.relname, 'name',p.polname, 'command',p.polcmd, 'permissive',p.polpermissive, 'roles',(SELECT jsonb_agg(CASE WHEN r=0 THEN 'PUBLIC' ELSE pg_get_userbyid(r) END ORDER BY CASE WHEN r=0 THEN 'PUBLIC' ELSE pg_get_userbyid(r) END COLLATE "C") FROM unnest(p.polroles) r), 'using',pg_get_expr(p.polqual,p.polrelid), 'check',pg_get_expr(p.polwithcheck,p.polrelid)) row_value FROM pg_policy p JOIN pg_class c ON c.oid=p.polrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public') r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','schema_security','pg_policy',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","schema_security","pg_policy"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT p.proname||'('||pg_get_function_identity_arguments(p.oid)||')' row_key,jsonb_build_object('identity',p.proname||'('||pg_get_function_identity_arguments(p.oid)||')', 'definition',pg_get_functiondef(p.oid), 'owner',pg_get_userbyid(p.proowner), 'acl',(SELECT jsonb_agg(v::text ORDER BY v::text COLLATE "C") FROM unnest(p.proacl) v), 'security_definer',p.prosecdef, 'config',p.proconfig) row_value FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' AND p.prokind IN ('f','p')) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','schema_security','pg_proc',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","schema_security","pg_proc"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.rolname row_key,jsonb_build_object('rolname',r.rolname, 'rolsuper',r.rolsuper, 'rolinherit',r.rolinherit, 'rolcreaterole',r.rolcreaterole, 'rolcreatedb',r.rolcreatedb, 'rolcanlogin',r.rolcanlogin, 'rolreplication',r.rolreplication, 'rolbypassrls',r.rolbypassrls, 'rolconnlimit',r.rolconnlimit, 'rolconfig',r.rolconfig, 'rolvaliduntil',CASE WHEN r.rolvaliduntil IS NULL THEN NULL WHEN isfinite(r.rolvaliduntil) THEN to_char(r.rolvaliduntil AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') ELSE r.rolvaliduntil::text END) row_value FROM pg_roles r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','schema_security','pg_roles',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","schema_security","pg_roles"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT c.relname||':'||CASE WHEN t.tgisinternal THEN t.tgfoid::regproc::text||':'||coalesce((SELECT conname FROM pg_constraint WHERE oid=t.tgconstraint),'')||':'||t.tgtype::text ELSE t.tgname END row_key,jsonb_build_object('table',c.relname, 'name',CASE WHEN t.tgisinternal THEN NULL ELSE t.tgname END, 'enabled',t.tgenabled, 'internal',t.tgisinternal, 'function',t.tgfoid::regproc::text, 'type',t.tgtype, 'constraint',(SELECT conname FROM pg_constraint WHERE oid=t.tgconstraint), 'definition',CASE WHEN t.tgisinternal THEN NULL ELSE pg_get_triggerdef(t.oid) END) row_value FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public') r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','schema_security','pg_trigger',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","schema_security","pg_trigger"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.id row_key,jsonb_build_object('id',r.id, 'description',r.description, 'created_at',to_char(r.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.roles r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','schema_security','roles',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","schema_security","roles"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.user_id::text||':'||r.role_id row_key,jsonb_build_object('user_id',r.user_id, 'role_id',r.role_id, 'granted_by',r.granted_by, 'granted_at',to_char(r.granted_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.user_roles r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','schema_security','user_roles',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","schema_security","user_roles"]'::text AS audit_record;

SELECT '["group_end","schema_security"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'contract_document_sha256',t.contract_document_sha256, 'contract_reference',t.contract_reference, 'status',t.status, 'covered_datasets',t.covered_datasets, 'covered_uses',t.covered_uses, 'effective_from',to_char(t.effective_from,'YYYY-MM-DD'), 'effective_until',to_char(t.effective_until,'YYYY-MM-DD'), 'managed_by',t.managed_by, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.data_entitlements t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','simulation','data_entitlements',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","simulation","data_entitlements"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.run_id::text row_key, jsonb_build_object('run_id',t.run_id, 'owner_user_id',t.owner_user_id, 'illustrative_canonical',encode(t.illustrative_canonical,'hex'), 'illustrative_sha256',t.illustrative_sha256, 'zero_cost_canonical',encode(t.zero_cost_canonical,'hex'), 'zero_cost_sha256',t.zero_cost_sha256, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value
   FROM public.owner_beta_simulation_result_pairs t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','simulation','owner_beta_simulation_result_pairs',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","simulation","owner_beta_simulation_result_pairs"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'strategy_config_id',t.strategy_config_id, 'job_id',t.job_id, 'strategy_id',t.strategy_id, 'strategy_version',t.strategy_version, 'strategy_config_json',t.strategy_config_json, 'strategy_config_sha256',t.strategy_config_sha256, 'evaluation_start',to_char(t.evaluation_start,'YYYY-MM-DD'), 'evaluation_end',to_char(t.evaluation_end,'YYYY-MM-DD'), 'session_count',t.session_count, 'input_kind',t.input_kind, 'capability',t.capability, 'audience',t.audience, 'vendor_snapshot',t.vendor_snapshot, 'strict_pit',t.strict_pit, 'action_count',t.action_count, 'candidate_content_sha256',t.candidate_content_sha256, 'artifact_manifest_sha256',t.artifact_manifest_sha256, 'stage5_manifest_sha256',t.stage5_manifest_sha256, 'action_manifest_sha256',t.action_manifest_sha256, 'approval_registry_sha256',t.approval_registry_sha256, 'contract_document_sha256',t.contract_document_sha256, 'bars_sha256',t.bars_sha256, 'status',t.status, 'error_code',t.error_code, 'engine_json',t.engine_json, 'published_attempt_id',t.published_attempt_id, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'started_at',to_char(t.started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'finished_at',to_char(t.finished_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value
   FROM public.owner_beta_simulation_runs t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','simulation','owner_beta_simulation_runs',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","simulation","owner_beta_simulation_runs"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'strategy_id',t.strategy_id, 'strategy_version',t.strategy_version, 'config_json',t.config_json, 'is_active',t.is_active, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'daily_enabled',t.daily_enabled, 'daily_epoch',t.daily_epoch) row_value
   FROM public.user_strategy_configs t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','simulation','user_strategy_configs',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","simulation","user_strategy_configs"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'issuer',t.issuer, 'subject',t.subject, 'email',t.email, 'display_name',t.display_name, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'provisioned_by_user_id',t.provisioned_by_user_id) row_value FROM public.users t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('row','simulation','users',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["table_end","simulation","users"]'::text AS audit_record;

SELECT '["group_end","simulation"]'::text AS audit_record;

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'account_id',t.account_id, 'owner_user_id',t.owner_user_id, 'strategy_config_id',t.strategy_config_id, 'strategy_id',t.strategy_id, 'strategy_version',t.strategy_version, 'bound_at',to_char(t.bound_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'unbound_at',to_char(t.unbound_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'auto_apply_recommendations',t.auto_apply_recommendations) row_value
   FROM public.account_strategy_bindings t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','account_strategy_bindings',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'account_type',t.account_type, 'name',t.name, 'currency',t.currency, 'status',t.status, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'initial_cash',t.initial_cash::text, 'cost_profile_id',t.cost_profile_id, 'cost_profile_version',t.cost_profile_version, 'paper_state_version',t.paper_state_version) row_value
   FROM public.accounts t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','accounts',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'contract_document_sha256',t.contract_document_sha256, 'contract_reference',t.contract_reference, 'status',t.status, 'covered_datasets',t.covered_datasets, 'covered_uses',t.covered_uses, 'effective_from',to_char(t.effective_from,'YYYY-MM-DD'), 'effective_until',to_char(t.effective_until,'YYYY-MM-DD'), 'managed_by',t.managed_by, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.data_entitlements t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','data_entitlements',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.event_sha256::text row_key, jsonb_build_object('event_sha256',t.event_sha256, 'admission_sha256',t.admission_sha256, 'owner_user_id',t.owner_user_id, 'kind',t.kind, 'evidence_sha256',t.evidence_sha256, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_daily_admission_events t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_daily_admission_events',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.admission_sha256::text row_key, jsonb_build_object('admission_sha256',t.admission_sha256, 'owner_user_id',t.owner_user_id, 'policy_sha256',t.policy_sha256, 'content_sha256',t.content_sha256, 'manifest_sha256',t.manifest_sha256, 'parent_admission_sha256',t.parent_admission_sha256, 'range_start',to_char(t.range_start,'YYYY-MM-DD'), 'range_end',to_char(t.range_end,'YYYY-MM-DD'), 'session_count',t.session_count, 'bar_count',t.bar_count, 'evidence_json',t.evidence_json, 'admitted_at',to_char(t.admitted_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_daily_admissions t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_daily_admissions',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.owner_user_id::text||':'||t.calendar_evidence_sha256::text||':'||t.session_date::text row_key, jsonb_build_object('owner_user_id',t.owner_user_id, 'calendar_evidence_sha256',t.calendar_evidence_sha256, 'session_date',to_char(t.session_date,'YYYY-MM-DD'), 'session_type',t.session_type, 'evidence_kind',t.evidence_kind, 'source_row_ordinal',t.source_row_ordinal, 'source_row_sha256',t.source_row_sha256) row_value FROM public.owner_beta_daily_calendar_days t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_daily_calendar_days',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.calendar_evidence_sha256::text row_key, jsonb_build_object('calendar_evidence_sha256',t.calendar_evidence_sha256, 'owner_user_id',t.owner_user_id, 'policy_sha256',t.policy_sha256, 'normalizer_version',t.normalizer_version, 'source_provider',t.source_provider, 'source_market',t.source_market, 'source_batch_id',t.source_batch_id, 'source_batch_date',to_char(t.source_batch_date,'YYYY-MM-DD'), 'requested_target',to_char(t.requested_target,'YYYY-MM-DD'), 'source_file_name',t.source_file_name, 'source_file_sha256',t.source_file_sha256, 'source_manifest_line_sha256',t.source_manifest_line_sha256, 'source_retrieved_at',to_char(t.source_retrieved_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'copy_chain',t.copy_chain, 'evidence_json',t.evidence_json, 'projected_at',to_char(t.projected_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_daily_calendar_sources t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_daily_calendar_sources',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'enabled',t.enabled, 'activation_epoch',t.activation_epoch, 'policy_sha256',t.policy_sha256, 'policy_json',t.policy_json, 'entitlement_reference',t.entitlement_reference, 'entitlement_sha256',t.entitlement_sha256, 'first_auto_session',to_char(t.first_auto_session,'YYYY-MM-DD'), 'trigger_version',t.trigger_version, 'max_jobs_per_owner',t.max_jobs_per_owner, 'lease_seconds',t.lease_seconds, 'backoff_seconds',t.backoff_seconds, 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_daily_control t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_daily_control',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.receipt_id::text row_key, jsonb_build_object('receipt_id',t.receipt_id, 'stage',t.stage, 'owner_user_id',t.owner_user_id, 'policy_sha256',t.policy_sha256, 'activation_epoch',t.activation_epoch, 'started_at',to_char(t.started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'completed_at',to_char(t.completed_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'result_code',t.result_code, 'source_frontier_sha256',t.source_frontier_sha256, 'admission_sha256',t.admission_sha256, 'event_sha256',t.event_sha256, 'completion_binding_sha256',t.completion_binding_sha256) row_value FROM public.owner_beta_daily_poll_receipts t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_daily_poll_receipts',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.stage::text row_key, jsonb_build_object('stage',t.stage, 'next_poll_at',to_char(t.next_poll_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'last_completed_at',to_char(t.last_completed_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'last_result_code',t.last_result_code, 'verified_admission_sha256',t.verified_admission_sha256, 'source_frontier_sha256',t.source_frontier_sha256, 'latest_receipt_id',t.latest_receipt_id) row_value FROM public.owner_beta_daily_poll_state t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_daily_poll_state',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.run_id::text row_key, jsonb_build_object('run_id',t.run_id, 'job_id',t.job_id, 'owner_user_id',t.owner_user_id, 'attempt_id',t.attempt_id, 'attempt_no',t.attempt_no, 'claimed_by',t.claimed_by, 'locked_at',to_char(t.locked_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'lease_seconds',t.lease_seconds, 'authorized_at',to_char(t.authorized_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_daily_publication_claims t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_daily_publication_claims',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.owner_user_id::text||':'||t.strategy_config_id::text||':'||t.session_date::text||':'||t.trigger_version::text row_key, jsonb_build_object('owner_user_id',t.owner_user_id, 'strategy_config_id',t.strategy_config_id, 'session_date',to_char(t.session_date,'YYYY-MM-DD'), 'trigger_version',t.trigger_version, 'run_id',t.run_id, 'job_id',t.job_id, 'admission_sha256',t.admission_sha256, 'strategy_config_sha256',t.strategy_config_sha256, 'config_epoch',t.config_epoch, 'activation_epoch',t.activation_epoch, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_daily_reservations t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_daily_reservations',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.event_sha256::text row_key, jsonb_build_object('event_sha256',t.event_sha256, 'owner_user_id',t.owner_user_id, 'source_kind',t.source_kind, 'range_start',to_char(t.range_start,'YYYY-MM-DD'), 'range_end',to_char(t.range_end,'YYYY-MM-DD'), 'code',t.code, 'evidence_sha256',t.evidence_sha256, 'admission_sha256',t.admission_sha256, 'recorded_at',to_char(t.recorded_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_daily_source_events t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_daily_source_events',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'account_id',t.account_id, 'strategy_config_id',t.strategy_config_id, 'state',t.state, 'control_epoch',t.control_epoch, 'contract_id',t.contract_id, 'contract_sha256',t.contract_sha256, 'contract_json',t.contract_json, 'cost_profile_id',t.cost_profile_id, 'cost_profile_version',t.cost_profile_version, 'cost_profile_sha256',t.cost_profile_sha256, 'cohort_sha256',t.cohort_sha256, 'daily_cohort',t.daily_cohort, 'process_binding',t.process_binding, 'process_binding_sha256',t.process_binding_sha256, 'consent_event_id',t.consent_event_id, 'activation_event_id',t.activation_event_id, 'last_signal_allocation_sha256',t.last_signal_allocation_sha256, 'last_signal_date',to_char(t.last_signal_date,'YYYY-MM-DD'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_paper_control t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_paper_control',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'account_id',t.account_id, 'control_epoch',t.control_epoch, 'kind',t.kind, 'actor_user_id',t.actor_user_id, 'occurred_at',to_char(t.occurred_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'payload',t.payload, 'payload_sha256',t.payload_sha256, 'request_key',t.request_key, 'pending_target_id',t.pending_target_id, 'request_canonical',t.request_canonical, 'request_sha256',t.request_sha256, 'result_canonical',t.result_canonical, 'result_sha256',t.result_sha256) row_value FROM public.owner_beta_paper_events t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_paper_events',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'account_id',t.account_id, 'strategy_config_id',t.strategy_config_id, 'recommendation_run_id',t.recommendation_run_id, 'signal_date',to_char(t.signal_date,'YYYY-MM-DD'), 'effective_session',to_char(t.effective_session,'YYYY-MM-DD'), 'signal_admission_sha256',t.signal_admission_sha256, 'factor_snapshot_sha256',t.factor_snapshot_sha256, 'target_snapshot_sha256',t.target_snapshot_sha256, 'target_weights_json',t.target_weights_json, 'config_epoch',t.config_epoch, 'daily_activation_epoch',t.daily_activation_epoch, 'control_epoch',t.control_epoch, 'consent_event_id',t.consent_event_id, 'activation_event_id',t.activation_event_id, 'cohort_sha256',t.cohort_sha256, 'calendar_evidence_sha256',t.calendar_evidence_sha256, 'calendar_link_json',t.calendar_link_json, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_paper_lineage t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_paper_lineage',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.observation_id::text row_key, jsonb_build_object('observation_id',t.observation_id, 'generation',t.generation, 'request_key',t.request_key, 'summary',t.summary, 'summary_sha256',t.summary_sha256, 'observed_at',to_char(t.observed_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'request_canonical',t.request_canonical, 'request_sha256',t.request_sha256, 'result_canonical',t.result_canonical, 'result_sha256',t.result_sha256) row_value FROM public.owner_beta_paper_operation_observations t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_paper_operation_observations',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.generation::text row_key, jsonb_build_object('generation',t.generation, 'installation_id',t.installation_id, 'account_id',t.account_id, 'request_key',t.request_key, 'purpose',t.purpose, 'scope',t.scope, 'expected_epoch',t.expected_epoch, 'local_epoch_floor',t.local_epoch_floor, 'fence_epoch',t.fence_epoch, 'release_manifest_sha256',t.release_manifest_sha256, 'intent',t.intent, 'intent_sha256',t.intent_sha256, 'prior_state',t.prior_state, 'fence_event_id',t.fence_event_id, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'request_canonical',t.request_canonical, 'request_sha256',t.request_sha256, 'result_canonical',t.result_canonical, 'result_sha256',t.result_sha256) row_value FROM public.owner_beta_paper_operations t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_paper_operations',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.grant_id::text row_key, jsonb_build_object('grant_id',t.grant_id, 'account_id',t.account_id, 'pending_target_id',t.pending_target_id, 'lineage_id',t.lineage_id, 'effective_session',to_char(t.effective_session,'YYYY-MM-DD'), 'original_receipt_sha256',t.original_receipt_sha256, 'lineage_epoch',t.lineage_epoch, 'active_epoch',t.active_epoch, 'consent_event_id',t.consent_event_id, 'process_config_sha256',t.process_config_sha256, 'decision',t.decision, 'decision_sha256',t.decision_sha256, 'step',t.step, 'signal_admission_sha256',t.signal_admission_sha256, 'execution_admission_sha256',t.execution_admission_sha256, 'expires_at',to_char(t.expires_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'request_key',t.request_key, 'settlement_receipt_sha256',t.settlement_receipt_sha256, 'valuation_receipt_sha256',t.valuation_receipt_sha256, 'revoked_at',to_char(t.revoked_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_paper_recovery_grants t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_paper_recovery_grants',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.pending_target_id::text row_key, jsonb_build_object('pending_target_id',t.pending_target_id, 'owner_user_id',t.owner_user_id, 'account_id',t.account_id, 'effective_session',to_char(t.effective_session,'YYYY-MM-DD'), 'account_state_version',t.account_state_version, 'account_state_sha256',t.account_state_sha256, 'state',t.state, 'settlement_kind',t.settlement_kind, 'reason_code',t.reason_code, 'execution_admission_sha256',t.execution_admission_sha256, 'valuation_admission_sha256',t.valuation_admission_sha256, 'execution_input_sha256',t.execution_input_sha256, 'valuation_input_sha256',t.valuation_input_sha256, 'ledger_before_sha256',t.ledger_before_sha256, 'ledger_after_sha256',t.ledger_after_sha256, 'ledger_after_version',t.ledger_after_version, 'ledger_after_preimage',t.ledger_after_preimage, 'settlement_sha256',t.settlement_sha256, 'valuation_sha256',t.valuation_sha256, 'source_original_retrieved_at',to_char(t.source_original_retrieved_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'input_admitted_at',to_char(t.input_admitted_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'usable_at',to_char(t.usable_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'settled_at',to_char(t.settled_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'valued_at',to_char(t.valued_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'processing_origin',t.processing_origin, 'receipt_json',t.receipt_json, 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_paper_sessions t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_paper_sessions',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.slot_id::text row_key, jsonb_build_object('slot_id',t.slot_id, 'source_event_sha256',t.source_event_sha256, 'original_set_sha256',t.original_set_sha256, 'admission_sha256',t.admission_sha256, 'linked_at',to_char(t.linked_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_paper_source_admissions t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_paper_source_admissions',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.provider::text||':'||t.market::text||':'||t.batch_id::text||':'||t.file_name::text row_key, jsonb_build_object('provider',t.provider, 'market',t.market, 'batch_id',t.batch_id, 'file_name',t.file_name, 'wire_sha256',t.wire_sha256, 'slot_id',t.slot_id) row_value FROM public.owner_beta_paper_source_originals t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_paper_source_originals',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.slot_id::text row_key, jsonb_build_object('slot_id',t.slot_id, 'run_id',t.run_id, 'job_id',t.job_id, 'attempt_id',t.attempt_id, 'linked_at',to_char(t.linked_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_paper_source_runs t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_paper_source_runs',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.slot_id::text row_key, jsonb_build_object('slot_id',t.slot_id, 'reservation_id',t.reservation_id, 'root_event_sha256',t.root_event_sha256, 'owner_user_id',t.owner_user_id, 'policy_sha256',t.policy_sha256, 'activation_epoch',t.activation_epoch, 'session_date',to_char(t.session_date,'YYYY-MM-DD'), 'cohort_sha256',t.cohort_sha256, 'cohort',t.cohort, 'reserved_at',to_char(t.reserved_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value FROM public.owner_beta_paper_source_slots t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_paper_source_slots',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'strategy_config_id',t.strategy_config_id, 'job_id',t.job_id, 'as_of',to_char(t.as_of,'YYYY-MM-DD'), 'status',t.status, 'input_kind',t.input_kind, 'capability',t.capability, 'audience',t.audience, 'vendor_snapshot',t.vendor_snapshot, 'strict_pit',t.strict_pit, 'candidate_content_sha256',t.candidate_content_sha256, 'artifact_manifest_sha256',t.artifact_manifest_sha256, 'stage5_manifest_sha256',t.stage5_manifest_sha256, 'action_manifest_sha256',t.action_manifest_sha256, 'approval_registry_sha256',t.approval_registry_sha256, 'factor_snapshot_sha256',t.factor_snapshot_sha256, 'error_code',t.error_code, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'started_at',to_char(t.started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'finished_at',to_char(t.finished_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'strategy_id',t.strategy_id, 'strategy_version',t.strategy_version, 'strategy_config_json',t.strategy_config_json, 'strategy_config_sha256',t.strategy_config_sha256, 'target_snapshot_sha256',t.target_snapshot_sha256, 'cash_weight',t.cash_weight, 'trigger_kind',t.trigger_kind, 'daily_pins',t.daily_pins, 'daily_config_epoch',t.daily_config_epoch, 'daily_activation_epoch',t.daily_activation_epoch, 'daily_trigger_version',t.daily_trigger_version) row_value
   FROM public.owner_beta_recommendation_runs t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_recommendation_runs',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'strategy_config_id',t.strategy_config_id, 'job_id',t.job_id, 'strategy_id',t.strategy_id, 'strategy_version',t.strategy_version, 'strategy_config_json',t.strategy_config_json, 'strategy_config_sha256',t.strategy_config_sha256, 'evaluation_start',to_char(t.evaluation_start,'YYYY-MM-DD'), 'evaluation_end',to_char(t.evaluation_end,'YYYY-MM-DD'), 'session_count',t.session_count, 'input_kind',t.input_kind, 'capability',t.capability, 'audience',t.audience, 'vendor_snapshot',t.vendor_snapshot, 'strict_pit',t.strict_pit, 'action_count',t.action_count, 'candidate_content_sha256',t.candidate_content_sha256, 'artifact_manifest_sha256',t.artifact_manifest_sha256, 'stage5_manifest_sha256',t.stage5_manifest_sha256, 'action_manifest_sha256',t.action_manifest_sha256, 'approval_registry_sha256',t.approval_registry_sha256, 'contract_document_sha256',t.contract_document_sha256, 'bars_sha256',t.bars_sha256, 'status',t.status, 'error_code',t.error_code, 'engine_json',t.engine_json, 'published_attempt_id',t.published_attempt_id, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'started_at',to_char(t.started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'finished_at',to_char(t.finished_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')) row_value
   FROM public.owner_beta_simulation_runs t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','owner_beta_simulation_runs',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'account_id',t.account_id, 'recommendation_run_id',t.recommendation_run_id, 'target_portfolio_id',t.target_portfolio_id, 'strategy_config_id',t.strategy_config_id, 'job_id',t.job_id, 'status',t.status, 'price_basis',t.price_basis, 'price_date',to_char(t.price_date,'YYYY-MM-DD'), 'preview_seoul_date',to_char(t.preview_seoul_date,'YYYY-MM-DD'), 'proposed_effective_date',to_char(t.proposed_effective_date,'YYYY-MM-DD'), 'dataset_version_id',t.dataset_version_id, 'dataset_manifest_sha256',t.dataset_manifest_sha256, 'cost_profile_id',t.cost_profile_id, 'cost_profile_version',t.cost_profile_version, 'account_state_version',t.account_state_version, 'account_state_sha256',t.account_state_sha256, 'target_portfolio_sha256',t.target_portfolio_sha256, 'preview_token',t.preview_token, 'result_json',t.result_json, 'error_json',CASE WHEN t.error_json IS NULL THEN NULL ELSE jsonb_build_object('sha256','sha256:'||encode(sha256(convert_to(public.paper_0068_canonical_row_json(t.error_json),'UTF8')),'hex')) END, 'pending_target_id',t.pending_target_id, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'started_at',to_char(t.started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'completed_at',to_char(t.completed_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'applied_at',to_char(t.applied_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'input_kind',t.input_kind, 'beta_apply_mode',t.beta_apply_mode, 'beta_lineage_id',t.beta_lineage_id, 'beta_request_key',t.beta_request_key, 'account_state_preimage',t.account_state_preimage, 'signal_allocation_sha256',t.signal_allocation_sha256) row_value FROM public.paper_rebalance_previews t WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','paper_rebalance_previews',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'account_id',t.account_id, 'owner_user_id',t.owner_user_id, 'strategy_config_id',t.strategy_config_id, 'computed_on',to_char(t.computed_on,'YYYY-MM-DD'), 'effective_date',to_char(t.effective_date,'YYYY-MM-DD'), 'targets_json',t.targets_json, 'status',t.status, 'executed_at',to_char(t.executed_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'dataset_version',t.dataset_version, 'dataset_version_id',t.dataset_version_id, 'dataset_manifest_sha256',t.dataset_manifest_sha256, 'non_execution_reason',t.non_execution_reason, 'source_kind',t.source_kind, 'recommendation_run_id',t.recommendation_run_id, 'beta_lineage_id',t.beta_lineage_id) row_value
   FROM public.pending_targets t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','pending_targets',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

WITH saved AS MATERIALIZED (SELECT current_setting('app.actor_user_id',true) actor),
 rows AS MATERIALIZED (SELECT r.row_key,r.row_value FROM saved CROSS JOIN LATERAL (SELECT r.row_key,r.row_value FROM (SELECT id FROM public.users WHERE length(coalesce(saved.actor,''))>=0 OFFSET 0) u
 CROSS JOIN LATERAL (SELECT set_config('app.actor_user_id',u.id::text,true) actor OFFSET 0) ctx
 CROSS JOIN LATERAL (SELECT t.id::text row_key, jsonb_build_object('id',t.id, 'owner_user_id',t.owner_user_id, 'strategy_id',t.strategy_id, 'strategy_version',t.strategy_version, 'config_json',t.config_json, 'is_active',t.is_active, 'created_at',to_char(t.created_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'updated_at',to_char(t.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), 'daily_enabled',t.daily_enabled, 'daily_epoch',t.daily_epoch) row_value
   FROM public.user_strategy_configs t WHERE t.owner_user_id=ctx.actor::uuid OFFSET 0) r) r),
 restored AS MATERIALIZED (SELECT set_config('app.actor_user_id',CASE WHEN consumed.n>=0 THEN coalesce(saved.actor,'') ELSE NULL END,true) actor
 FROM saved CROSS JOIN (SELECT count(*) n FROM rows) consumed)
SELECT jsonb_build_array('reference','user_strategy_configs',r.row_key,r.row_value)::text AS audit_record
 FROM restored CROSS JOIN LATERAL
 (SELECT row_key,row_value FROM rows WHERE length(restored.actor)>=0 OFFSET 0) r
 ORDER BY r.row_key COLLATE "C";

SELECT '["references_end"]'::text AS audit_record;
SELECT '["end"]'::text AS audit_record;
