-- R5-03 / WP-3: isolated Owner-beta Paper ledger and action ABI.
-- This migration creates no configured account, consent, process binding, or ON state.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

ALTER TABLE public.accounts ADD CONSTRAINT accounts_id_owner_beta_key UNIQUE (id,owner_user_id);
ALTER TABLE public.user_strategy_configs ADD CONSTRAINT user_strategy_configs_id_owner_beta_key UNIQUE (id,owner_user_id);
ALTER TABLE public.owner_beta_daily_admissions ADD CONSTRAINT owner_beta_daily_admissions_hash_owner_beta_key UNIQUE(admission_sha256,owner_user_id);

CREATE TABLE public.owner_beta_paper_control (
    id text PRIMARY KEY DEFAULT 'etf11' CHECK (id = 'etf11'),
    owner_user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE RESTRICT,
    account_id uuid NOT NULL UNIQUE,
    strategy_config_id uuid NOT NULL,
    state text NOT NULL DEFAULT 'OFF' CHECK (state IN ('OFF','ENABLED','DRAINING','RESTORED_OFF','BLOCKED')),
    control_epoch bigint NOT NULL DEFAULT 0 CHECK (control_epoch >= 0),
    contract_id text NOT NULL,
    contract_sha256 text NOT NULL CHECK (contract_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    contract_json jsonb NOT NULL CHECK (jsonb_typeof(contract_json) = 'object'),
    cost_profile_id text NOT NULL CHECK (cost_profile_id = 'KRX_ETF_DEFAULT'),
    cost_profile_version integer NOT NULL CHECK (cost_profile_version = 1),
    cost_profile_sha256 text NOT NULL CHECK (cost_profile_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    cohort_sha256 text CHECK (cohort_sha256 IS NULL OR cohort_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    daily_cohort jsonb CHECK (daily_cohort IS NULL OR jsonb_typeof(daily_cohort)='object'),
    process_binding jsonb,
    process_binding_sha256 text CHECK (process_binding_sha256 IS NULL OR process_binding_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    consent_event_id uuid,
    activation_event_id uuid,
    last_signal_allocation_sha256 text CHECK (last_signal_allocation_sha256 IS NULL OR last_signal_allocation_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    last_signal_date date,
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT owner_beta_paper_control_account_fkey FOREIGN KEY (account_id,owner_user_id)
        REFERENCES public.accounts(id,owner_user_id) ON DELETE RESTRICT,
    CONSTRAINT owner_beta_paper_control_strategy_fkey FOREIGN KEY (strategy_config_id,owner_user_id)
        REFERENCES public.user_strategy_configs(id,owner_user_id) ON DELETE RESTRICT,
    CONSTRAINT owner_beta_paper_control_signal_pair CHECK ((last_signal_allocation_sha256 IS NULL) = (last_signal_date IS NULL)),
    CONSTRAINT owner_beta_paper_control_binding_pair CHECK (
      (process_binding IS NULL AND process_binding_sha256 IS NULL AND cohort_sha256 IS NULL AND daily_cohort IS NULL)
      OR (process_binding IS NOT NULL AND process_binding_sha256 IS NOT NULL AND cohort_sha256 IS NOT NULL AND daily_cohort IS NOT NULL)
    ),
    CONSTRAINT owner_beta_paper_control_off_binding_check CHECK (state <> 'ENABLED' OR process_binding IS NOT NULL)
);
CREATE TABLE public.owner_beta_paper_events (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE RESTRICT,
    account_id uuid NOT NULL,
    control_epoch bigint NOT NULL CHECK (control_epoch >= 0),
    kind text NOT NULL CHECK (kind IN ('CONFIGURED','CONSENT_GRANTED','CONSENT_REVOKED','ENABLED','DRAIN_STARTED','OFF','RESTORED_OFF','BLOCKED','RECOVERY_AUTHORIZED')),
    actor_user_id uuid REFERENCES public.users(id) ON DELETE RESTRICT,
    occurred_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    payload jsonb NOT NULL CHECK (jsonb_typeof(payload)='object' AND octet_length(payload::text)<=262144),
    payload_sha256 text NOT NULL CHECK (payload_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    request_key text,
    pending_target_id uuid,
    FOREIGN KEY(account_id,owner_user_id) REFERENCES public.accounts(id,owner_user_id) ON DELETE RESTRICT,
    UNIQUE(id,owner_user_id,account_id),
    CHECK(request_key IS NULL OR (length(request_key) BETWEEN 1 AND 128 AND request_key !~ '[[:cntrl:]]'))
);
CREATE UNIQUE INDEX owner_beta_paper_events_request_key ON public.owner_beta_paper_events(account_id,request_key) WHERE request_key IS NOT NULL;

CREATE TABLE public.owner_beta_paper_lineage (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE RESTRICT,
    account_id uuid NOT NULL,
    strategy_config_id uuid NOT NULL,
    recommendation_run_id uuid NOT NULL,
    signal_date date NOT NULL,
    effective_session date NOT NULL CHECK (effective_session > signal_date),
    signal_admission_sha256 text NOT NULL,
    factor_snapshot_sha256 text NOT NULL CHECK (factor_snapshot_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    target_snapshot_sha256 text NOT NULL CHECK (target_snapshot_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    target_weights_json jsonb NOT NULL CHECK (jsonb_typeof(target_weights_json)='array' AND octet_length(target_weights_json::text)<=65536),
    config_epoch bigint NOT NULL CHECK (config_epoch>=0),
    daily_activation_epoch bigint NOT NULL CHECK (daily_activation_epoch>=0),
    control_epoch bigint NOT NULL CHECK (control_epoch>=0),
    consent_event_id uuid NOT NULL,
    activation_event_id uuid NOT NULL,
    cohort_sha256 text NOT NULL CHECK (cohort_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    calendar_evidence_sha256 text NOT NULL,
    calendar_link_json jsonb NOT NULL CHECK (jsonb_typeof(calendar_link_json)='object' AND octet_length(calendar_link_json::text)<=65536),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY(account_id,owner_user_id) REFERENCES public.accounts(id,owner_user_id) ON DELETE RESTRICT,
    FOREIGN KEY(strategy_config_id,owner_user_id) REFERENCES public.user_strategy_configs(id,owner_user_id) ON DELETE RESTRICT,
    FOREIGN KEY(recommendation_run_id,owner_user_id) REFERENCES public.owner_beta_recommendation_runs(id,owner_user_id) ON DELETE RESTRICT,
    FOREIGN KEY(signal_admission_sha256,owner_user_id) REFERENCES public.owner_beta_daily_admissions(admission_sha256,owner_user_id) ON DELETE RESTRICT,
    FOREIGN KEY(owner_user_id,calendar_evidence_sha256) REFERENCES public.owner_beta_daily_calendar_sources(owner_user_id,calendar_evidence_sha256) ON DELETE RESTRICT,
    FOREIGN KEY(consent_event_id,owner_user_id,account_id) REFERENCES public.owner_beta_paper_events(id,owner_user_id,account_id) ON DELETE RESTRICT,
    FOREIGN KEY(activation_event_id,owner_user_id,account_id) REFERENCES public.owner_beta_paper_events(id,owner_user_id,account_id) ON DELETE RESTRICT,
    UNIQUE(id,owner_user_id,account_id,strategy_config_id,signal_date,effective_session),
    UNIQUE(account_id,recommendation_run_id,effective_session,control_epoch,consent_event_id)
);

ALTER TABLE public.pending_targets ADD COLUMN beta_lineage_id uuid;
ALTER TABLE public.pending_targets ADD CONSTRAINT pending_targets_beta_lineage_fkey
  FOREIGN KEY(beta_lineage_id,owner_user_id,account_id,strategy_config_id,computed_on,effective_date)
  REFERENCES public.owner_beta_paper_lineage(id,owner_user_id,account_id,strategy_config_id,signal_date,effective_session) ON DELETE RESTRICT;
ALTER TABLE public.pending_targets DROP CONSTRAINT pending_targets_source_kind_check;
ALTER TABLE public.pending_targets DROP CONSTRAINT pending_targets_recommendation_source_lineage_check;
ALTER TABLE public.pending_targets ADD CONSTRAINT pending_targets_source_kind_check CHECK (
  (source_kind='LEGACY' AND recommendation_run_id IS NULL AND beta_lineage_id IS NULL)
  OR (source_kind IN ('SCHEDULED_RECOMMENDATION','MANUAL_RECOMMENDATION') AND recommendation_run_id IS NOT NULL AND beta_lineage_id IS NULL AND dataset_version IS NOT NULL AND dataset_version_id IS NOT NULL AND dataset_manifest_sha256 IS NOT NULL)
  OR (source_kind IN ('OWNER_BETA_DAILY_MANUAL','OWNER_BETA_DAILY_AUTO') AND recommendation_run_id IS NULL AND dataset_version IS NULL AND dataset_version_id IS NULL AND dataset_manifest_sha256 IS NULL AND beta_lineage_id IS NOT NULL)
);

ALTER TABLE public.paper_rebalance_previews
  ADD COLUMN input_kind text NOT NULL DEFAULT 'GENERIC_DATASET_V1',
  ADD COLUMN beta_apply_mode text,
  ADD COLUMN beta_lineage_id uuid,
  ADD COLUMN beta_request_key text,
  ADD COLUMN account_state_preimage jsonb,
  ADD COLUMN signal_allocation_sha256 text,
  ALTER COLUMN recommendation_run_id DROP NOT NULL,
  ALTER COLUMN target_portfolio_id DROP NOT NULL,
  ALTER COLUMN dataset_version_id DROP NOT NULL,
  ALTER COLUMN dataset_manifest_sha256 DROP NOT NULL;
ALTER TABLE public.paper_rebalance_previews ADD CONSTRAINT paper_preview_beta_lineage_fkey
  FOREIGN KEY(beta_lineage_id,owner_user_id,account_id,strategy_config_id,price_date,proposed_effective_date)
  REFERENCES public.owner_beta_paper_lineage(id,owner_user_id,account_id,strategy_config_id,signal_date,effective_session) ON DELETE RESTRICT;
ALTER TABLE public.paper_rebalance_previews ADD CONSTRAINT paper_preview_input_kind_check CHECK (
  (input_kind='GENERIC_DATASET_V1' AND beta_apply_mode IS NULL AND beta_lineage_id IS NULL AND recommendation_run_id IS NOT NULL AND target_portfolio_id IS NOT NULL AND dataset_version_id IS NOT NULL AND dataset_manifest_sha256 IS NOT NULL)
  OR (input_kind='OWNER_BETA_DAILY_V1' AND beta_apply_mode IN ('MANUAL','AUTO') AND beta_lineage_id IS NOT NULL AND recommendation_run_id IS NULL AND target_portfolio_id IS NULL AND dataset_version_id IS NULL AND dataset_manifest_sha256 IS NULL AND proposed_effective_date IS NOT NULL)
);
CREATE UNIQUE INDEX paper_preview_beta_request_key ON public.paper_rebalance_previews(owner_user_id,beta_request_key) WHERE beta_request_key IS NOT NULL;
CREATE UNIQUE INDEX paper_preview_beta_lineage_mode ON public.paper_rebalance_previews(beta_lineage_id,beta_apply_mode) WHERE input_kind='OWNER_BETA_DAILY_V1';

ALTER TABLE public.pending_targets ADD CONSTRAINT pending_targets_id_owner_beta_key UNIQUE(id,owner_user_id);
ALTER TABLE public.pending_targets ADD CONSTRAINT pending_targets_exact_beta_target_key UNIQUE(id,owner_user_id,account_id,effective_date);
ALTER TABLE public.pending_targets ADD CONSTRAINT pending_targets_id_owner_account_beta_key UNIQUE(id,owner_user_id,account_id);
ALTER TABLE public.owner_beta_paper_events ADD CONSTRAINT owner_beta_paper_event_target_fkey
  FOREIGN KEY(pending_target_id,owner_user_id,account_id) REFERENCES public.pending_targets(id,owner_user_id,account_id) ON DELETE RESTRICT;
CREATE TABLE public.owner_beta_paper_sessions (
    pending_target_id uuid PRIMARY KEY REFERENCES public.pending_targets(id) ON DELETE RESTRICT,
    owner_user_id uuid NOT NULL,
    account_id uuid NOT NULL,
    effective_session date NOT NULL,
    account_state_version bigint NOT NULL CHECK(account_state_version>=0),
    account_state_sha256 text NOT NULL CHECK(account_state_sha256 ~ '^[0-9a-f]{64}$'),
    state text NOT NULL CHECK(state IN ('PENDING','WAITING_PRICE','WAITING_ADMISSION','HELD','BLOCKED','RECOVERY_REQUIRED','SETTLED_TRADE','SETTLED_NO_TRADE','VALUED')),
    settlement_kind text CHECK(settlement_kind IS NULL OR settlement_kind IN ('TRADE','NO_TRADE')),
    reason_code text CHECK(reason_code IS NULL OR reason_code ~ '^[A-Z][A-Z0-9_]{0,63}$'),
    execution_admission_sha256 text,
    valuation_admission_sha256 text,
    execution_input_sha256 text CHECK(execution_input_sha256 IS NULL OR execution_input_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    valuation_input_sha256 text CHECK(valuation_input_sha256 IS NULL OR valuation_input_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    ledger_before_sha256 text CHECK(ledger_before_sha256 IS NULL OR ledger_before_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    ledger_after_sha256 text CHECK(ledger_after_sha256 IS NULL OR ledger_after_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    ledger_after_version bigint CHECK(ledger_after_version IS NULL OR ledger_after_version>=0),
    ledger_after_preimage jsonb,
    settlement_sha256 text CHECK(settlement_sha256 IS NULL OR settlement_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    valuation_sha256 text CHECK(valuation_sha256 IS NULL OR valuation_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    source_original_retrieved_at timestamptz,
    input_admitted_at timestamptz,
    usable_at timestamptz,
    settled_at timestamptz,
    valued_at timestamptz,
    processing_origin text NOT NULL CHECK(processing_origin IN ('MANUAL','AUTO','RECOVERY')),
    receipt_json jsonb CHECK(receipt_json IS NULL OR (jsonb_typeof(receipt_json)='object' AND octet_length(receipt_json::text)<=262144)),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY(pending_target_id,owner_user_id,account_id,effective_session) REFERENCES public.pending_targets(id,owner_user_id,account_id,effective_date) ON DELETE RESTRICT,
    FOREIGN KEY(account_id,owner_user_id) REFERENCES public.accounts(id,owner_user_id) ON DELETE RESTRICT,
    FOREIGN KEY(execution_admission_sha256,owner_user_id) REFERENCES public.owner_beta_daily_admissions(admission_sha256,owner_user_id) ON DELETE RESTRICT,
    FOREIGN KEY(valuation_admission_sha256,owner_user_id) REFERENCES public.owner_beta_daily_admissions(admission_sha256,owner_user_id) ON DELETE RESTRICT,
    UNIQUE(account_id,effective_session),
    CHECK((state IN ('SETTLED_TRADE','SETTLED_NO_TRADE','VALUED')) = (settlement_sha256 IS NOT NULL)),
    CHECK(state<>'VALUED' OR valuation_sha256 IS NOT NULL)
);

CREATE TABLE public.owner_beta_paper_session_events (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    pending_target_id uuid NOT NULL REFERENCES public.owner_beta_paper_sessions(pending_target_id) ON DELETE RESTRICT,
    event_seq bigint NOT NULL CHECK(event_seq>0),
    kind text NOT NULL CHECK(kind IN ('WAITING','BLOCKED','HELD','RECOVERY_REQUIRED','RECOVERY_AUTHORIZED','SETTLED_TRADE','SETTLED_NO_TRADE','VALUED')),
    reason_code text,
    payload jsonb NOT NULL CHECK(jsonb_typeof(payload)='object' AND octet_length(payload::text)<=262144),
    occurred_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(pending_target_id,event_seq)
);

CREATE FUNCTION public.paper_0067_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$
BEGIN RAISE EXCEPTION 'Owner-beta Paper history is immutable' USING ERRCODE='55000'; END $$;
CREATE TRIGGER owner_beta_paper_events_immutable BEFORE UPDATE OR DELETE ON public.owner_beta_paper_events FOR EACH ROW EXECUTE FUNCTION public.paper_0067_immutable();
CREATE TRIGGER owner_beta_paper_lineage_immutable BEFORE UPDATE OR DELETE ON public.owner_beta_paper_lineage FOR EACH ROW EXECUTE FUNCTION public.paper_0067_immutable();
CREATE TRIGGER owner_beta_paper_session_events_immutable BEFORE UPDATE OR DELETE ON public.owner_beta_paper_session_events FOR EACH ROW EXECUTE FUNCTION public.paper_0067_immutable();

CREATE FUNCTION public.paper_0067_principal(expected text) RETURNS void
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$ BEGIN
  IF session_user IS DISTINCT FROM expected OR current_setting('role',true) IS DISTINCT FROM 'none'
     OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_roles WHERE rolname=session_user AND rolcanlogin AND NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication AND NOT rolbypassrls)
  THEN RAISE EXCEPTION 'Owner-beta Paper principal denied' USING ERRCODE='42501'; END IF;
  IF expected='app' AND NOT EXISTS(SELECT 1 FROM public.user_roles ur
      WHERE ur.user_id=NULLIF(current_setting('app.actor_user_id',true),'')::uuid AND ur.role_id='owner')
  THEN RAISE EXCEPTION 'Owner-beta Paper Owner membership required' USING ERRCODE='42501'; END IF;
END $$;

CREATE FUNCTION public.paper_0067_is_beta_account(p_account uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$
  SELECT EXISTS(SELECT 1 FROM public.owner_beta_paper_control WHERE account_id=p_account)
$$;
ALTER FUNCTION public.paper_0067_is_beta_account(uuid) OWNER TO migration_owner;

CREATE FUNCTION public.paper_0067_guard_beta_write() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog,pg_temp AS $$
DECLARE old_id uuid; new_id uuid;
BEGIN
  IF TG_OP<>'INSERT' THEN old_id:=CASE WHEN TG_TABLE_NAME='accounts' THEN (to_jsonb(OLD)->>'id')::uuid ELSE (to_jsonb(OLD)->>'account_id')::uuid END; END IF;
  IF TG_OP<>'DELETE' THEN new_id:=CASE WHEN TG_TABLE_NAME='accounts' THEN (to_jsonb(NEW)->>'id')::uuid ELSE (to_jsonb(NEW)->>'account_id')::uuid END; END IF;
  IF current_user<>'migration_owner' AND (public.paper_0067_is_beta_account(old_id) OR public.paper_0067_is_beta_account(new_id)) THEN
    RAISE EXCEPTION 'bound Owner-beta Paper rows are writable only through the beta ledger ABI' USING ERRCODE='42501';
  END IF;
  RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
END $$;

DO $$ DECLARE t text; BEGIN
  FOREACH t IN ARRAY ARRAY['accounts','account_strategy_bindings','cash_ledger','positions','orders','fills','daily_equity','pending_targets','paper_rebalance_previews'] LOOP
    EXECUTE format('CREATE TRIGGER paper_0067_guard BEFORE INSERT OR UPDATE OR DELETE ON public.%I FOR EACH ROW EXECUTE FUNCTION public.paper_0067_guard_beta_write()',t);
  END LOOP;
END $$;
REVOKE ALL ON FUNCTION public.paper_0067_is_beta_account(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.paper_0067_is_beta_account(uuid) TO app,worker,admin,research_writer,audit_writer;

DO $$ DECLARE t text; BEGIN
  FOREACH t IN ARRAY ARRAY['owner_beta_paper_control','owner_beta_paper_events','owner_beta_paper_lineage','owner_beta_paper_sessions','owner_beta_paper_session_events'] LOOP
    EXECUTE format('ALTER TABLE public.%I OWNER TO migration_owner',t);
    EXECUTE format('ALTER TABLE public.%I ENABLE ROW LEVEL SECURITY',t);
    EXECUTE format('ALTER TABLE public.%I FORCE ROW LEVEL SECURITY',t);
    EXECUTE format('CREATE POLICY paper_0067_internal ON public.%I FOR ALL TO migration_owner USING(true) WITH CHECK(true)',t);
    IF t<>'owner_beta_paper_session_events' THEN
      EXECUTE format('CREATE POLICY paper_0067_app_read ON public.%I FOR SELECT TO app USING(owner_user_id = NULLIF(current_setting(''app.actor_user_id'',true),'''')::uuid AND EXISTS(SELECT 1 FROM public.user_roles ur WHERE ur.user_id=owner_user_id AND ur.role_id=''owner''))',t);
    END IF;
    EXECUTE format('REVOKE ALL ON public.%I FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter',t);
    EXECUTE format('GRANT SELECT ON public.%I TO app',t);
  END LOOP;
END $$;
ALTER TABLE public.owner_beta_paper_control ADD CONSTRAINT owner_beta_paper_control_consent_fkey FOREIGN KEY(consent_event_id,owner_user_id,account_id) REFERENCES public.owner_beta_paper_events(id,owner_user_id,account_id) ON DELETE RESTRICT;
ALTER TABLE public.owner_beta_paper_control ADD CONSTRAINT owner_beta_paper_control_activation_fkey FOREIGN KEY(activation_event_id,owner_user_id,account_id) REFERENCES public.owner_beta_paper_events(id,owner_user_id,account_id) ON DELETE RESTRICT;
CREATE POLICY paper_0067_app_read ON public.owner_beta_paper_session_events FOR SELECT TO app USING(EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions s WHERE s.pending_target_id=owner_beta_paper_session_events.pending_target_id AND s.owner_user_id=NULLIF(current_setting('app.actor_user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM public.user_roles ur WHERE ur.user_id=s.owner_user_id AND ur.role_id='owner')));
CREATE POLICY paper_0067_worker_hide_beta_targets ON public.pending_targets AS RESTRICTIVE FOR SELECT TO worker USING(beta_lineage_id IS NULL);
CREATE POLICY paper_0067_worker_hide_beta_previews ON public.paper_rebalance_previews AS RESTRICTIVE FOR SELECT TO worker USING(input_kind='GENERIC_DATASET_V1');

CREATE FUNCTION public.paper_0067_action(p_action text,p_outcome text,p_epoch bigint,p_account uuid,p_data jsonb DEFAULT '{}'::jsonb) RETURNS jsonb
LANGUAGE sql IMMUTABLE SET search_path=pg_catalog,pg_temp AS $$
  SELECT jsonb_build_object('schema','owner-beta-paper-action-v1','action',p_action,'outcome',p_outcome,'control_epoch',p_epoch,'account_id',p_account,'data',COALESCE(p_data,'{}'::jsonb))
$$;

-- This emits the exact compact serde_json v1 object used by
-- paper_preview::account_state_sha256. Keys and position symbols use bytewise
-- order; money and quantities remain strings exactly as supplied by NUMERIC.
CREATE FUNCTION public.paper_0067_account_state_sha256(state_version bigint,cash_text text,positions_json jsonb) RETURNS text
LANGUAGE sql IMMUTABLE STRICT SET search_path=pg_catalog,pg_temp AS $$
  SELECT encode(sha256(convert_to(
    '{"cash":'||to_json(cash_text)::text||',"paper_state_version":'||state_version::text||',"positions":{'||
      COALESCE((SELECT string_agg(to_json(k)::text||':'||to_json(v)::text,',' ORDER BY k COLLATE "C") FROM jsonb_each_text(positions_json) AS p(k,v)),'')||
    '},"schema_version":1}', 'UTF8')), 'hex')
$$;

CREATE FUNCTION public.configure_owner_beta_paper_v1(strategy_config_id uuid,account_name text,contract jsonb,process_binding jsonb,request_key text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE daily public.owner_beta_daily_control; sc public.user_strategy_configs; acct uuid; ev uuid; saved public.owner_beta_paper_events; c public.owner_beta_paper_control; old_actor text:=current_setting('app.actor_user_id',true); cost_hash text;
BEGIN
  PERFORM public.paper_0067_principal('migration_owner');
  IF process_binding IS NOT NULL THEN RAISE EXCEPTION 'initial process binding must be null while OFF' USING ERRCODE='23514'; END IF;
  IF account_name IS NULL OR length(account_name) NOT BETWEEN 1 AND 64 OR account_name !~ '^[A-Za-z0-9 _.-]+$' OR request_key IS NULL OR length(request_key) NOT BETWEEN 1 AND 128 THEN RAISE EXCEPTION 'configuration request shape' USING ERRCODE='23514'; END IF;
  IF NOT public.daily_0060_keys(contract,ARRAY['contract_id','contract_sha256','decision_sha256','cost_profile_id','cost_profile_version'])
     OR contract->>'contract_id'<>'owner-beta-paper-v1-draft-1'
     OR contract->>'contract_sha256'<>'sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0'
     OR contract->>'decision_sha256'<>'sha256:8ef602a35586d3e2ca532c5fbb5efc6f2e727eb25ebeb45a44dad0ea0d0ca394'
     OR contract->>'cost_profile_id'<>'KRX_ETF_DEFAULT' OR contract->>'cost_profile_version'<>'1' THEN RAISE EXCEPTION 'financial contract mismatch' USING ERRCODE='23514'; END IF;
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
      PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
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
  INSERT INTO public.owner_beta_paper_events(id,owner_user_id,account_id,control_epoch,kind,payload,payload_sha256,request_key)
   VALUES(ev,daily.owner_user_id,acct,0,'CONFIGURED',jsonb_build_object('schema','owner-beta-paper-configured-v1','account_name',account_name,'strategy_config_id',strategy_config_id,'contract',contract,'initial_cash','10000000.0000','currency','KRW'),public.daily_0060_digest(jsonb_build_object('schema','owner-beta-paper-configured-v1','account_name',account_name,'strategy_config_id',strategy_config_id,'contract',contract,'initial_cash','10000000.0000','currency','KRW')::text),request_key);
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
  RETURN public.paper_0067_action('CONFIGURE','OK',0,acct,jsonb_build_object('account_id',acct,'event_id',ev,'state','OFF','initial_cash','10000000.0000','currency','KRW'));
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.record_owner_beta_paper_consent_v1(account_id uuid,expected_epoch bigint,consent_version text,contract_sha256 text,opt_in boolean,request_key text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE c public.owner_beta_paper_control; actor uuid; new_epoch bigint; ev uuid; existing public.owner_beta_paper_events; old_actor text:=current_setting('app.actor_user_id',true); kind text; payload jsonb;
BEGIN
  PERFORM public.paper_0067_principal('app');
  actor:=NULLIF(current_setting('app.actor_user_id',true),'')::uuid;
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' AND owner_user_id=actor AND owner_beta_paper_control.account_id=record_owner_beta_paper_consent_v1.account_id FOR UPDATE;
  IF NOT FOUND THEN RETURN public.paper_0067_action('CONSENT','NOT_FOUND',NULL,NULL); END IF;
  PERFORM pg_advisory_xact_lock(hashtextextended(c.account_id::text,381901));
  IF consent_version<>c.contract_id OR contract_sha256<>c.contract_sha256 OR request_key IS NULL OR length(request_key) NOT BETWEEN 1 AND 128 THEN RAISE EXCEPTION 'consent binding is malformed' USING ERRCODE='23514'; END IF;
  SELECT * INTO existing FROM public.owner_beta_paper_events e WHERE e.account_id=c.account_id AND e.request_key=record_owner_beta_paper_consent_v1.request_key FOR SHARE;
  IF FOUND THEN
    kind:=CASE WHEN opt_in THEN 'CONSENT_GRANTED' ELSE 'CONSENT_REVOKED' END;
    IF existing.kind<>kind OR existing.actor_user_id IS DISTINCT FROM actor OR existing.payload->>'schema'<>'owner-beta-paper-consent-v1'
       OR existing.payload->>'consent_version' IS DISTINCT FROM consent_version OR existing.payload->>'contract_sha256' IS DISTINCT FROM contract_sha256
       OR (existing.payload->>'opt_in')::boolean IS DISTINCT FROM opt_in OR (existing.payload->>'expected_epoch')::bigint IS DISTINCT FROM expected_epoch THEN RAISE EXCEPTION 'consent request key conflicts with prior event' USING ERRCODE='23505'; END IF;
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
    RETURN public.paper_0067_action('CONSENT','REPLAY',existing.control_epoch,c.account_id,jsonb_build_object('event_id',existing.id,'opt_in',opt_in));
  END IF;
  IF expected_epoch<>c.control_epoch THEN RAISE EXCEPTION 'consent control epoch is stale' USING ERRCODE='40001'; END IF;
  new_epoch:=c.control_epoch;
  IF NOT opt_in THEN IF new_epoch=9223372036854775807 THEN RAISE EXCEPTION 'control epoch exhausted' USING ERRCODE='22003'; END IF; new_epoch:=new_epoch+1; END IF;
  kind:=CASE WHEN opt_in THEN 'CONSENT_GRANTED' ELSE 'CONSENT_REVOKED' END;
  payload:=jsonb_build_object('schema','owner-beta-paper-consent-v1','consent_version',consent_version,'contract_sha256',contract_sha256,'opt_in',opt_in,'expected_epoch',expected_epoch,'resulting_epoch',new_epoch);
  ev:=gen_random_uuid();
  INSERT INTO public.owner_beta_paper_events(id,owner_user_id,account_id,control_epoch,kind,actor_user_id,payload,payload_sha256,request_key)
    VALUES(ev,actor,c.account_id,new_epoch,kind,actor,payload,public.daily_0060_digest(payload::text),request_key);
  UPDATE public.account_strategy_bindings b SET auto_apply_recommendations=opt_in WHERE b.account_id=c.account_id AND b.owner_user_id=c.owner_user_id AND b.strategy_config_id=c.strategy_config_id AND b.unbound_at IS NULL;
  IF NOT FOUND THEN RAISE EXCEPTION 'bound Paper strategy binding disappeared' USING ERRCODE='40001'; END IF;
  UPDATE public.owner_beta_paper_control SET control_epoch=new_epoch,consent_event_id=CASE WHEN opt_in THEN ev ELSE NULL END,
    state=CASE WHEN opt_in THEN state WHEN state='ENABLED' THEN 'DRAINING' ELSE state END,updated_at=clock_timestamp() WHERE id='etf11';
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
  RETURN public.paper_0067_action('CONSENT','OK',new_epoch,c.account_id,jsonb_build_object('event_id',ev,'opt_in',opt_in));
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.transition_owner_beta_paper_v1(expected_epoch bigint,transition text,evidence jsonb,request_key text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE old_actor text:=current_setting('app.actor_user_id',true); c public.owner_beta_paper_control; saved public.owner_beta_paper_events; ev uuid; next_epoch bigint; b jsonb; v_cohort jsonb; now_kst date:=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date; target public.pending_targets; session_row public.owner_beta_paper_sessions; recovery_target uuid; seq bigint; expected_kind text; daily public.owner_beta_daily_control; expected_session date; session_dates date[]; verified_session date; i integer;
BEGIN
  PERFORM public.paper_0067_principal('migration_owner');
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('TRANSITION','NOT_FOUND',NULL,NULL); END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  PERFORM pg_advisory_xact_lock(hashtextextended(c.account_id::text,381901));
  IF transition NOT IN ('ENABLE','DRAIN','OFF','RESTORED_OFF','RECOVERY') OR request_key IS NULL OR length(request_key) NOT BETWEEN 1 AND 128 OR jsonb_typeof(evidence)<>'object' OR octet_length(evidence::text)>262144 THEN RAISE EXCEPTION 'transition precondition failed' USING ERRCODE='22023'; END IF;
  SELECT * INTO saved FROM public.owner_beta_paper_events WHERE account_id=c.account_id AND owner_beta_paper_events.request_key=transition_owner_beta_paper_v1.request_key FOR SHARE;
  IF FOUND THEN
    expected_kind:=CASE transition WHEN 'ENABLE' THEN 'ENABLED' WHEN 'DRAIN' THEN 'DRAIN_STARTED' WHEN 'OFF' THEN 'OFF' WHEN 'RESTORED_OFF' THEN 'RESTORED_OFF' ELSE 'RECOVERY_AUTHORIZED' END;
    IF saved.kind<>expected_kind OR saved.payload IS DISTINCT FROM evidence THEN RAISE EXCEPTION 'transition request key conflicts with prior event' USING ERRCODE='23505'; END IF;
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('TRANSITION','REPLAY',c.control_epoch,c.account_id,jsonb_build_object('event_id',saved.id,'state',c.state,'pending_target_id',saved.pending_target_id));
  END IF;
  IF expected_epoch<>c.control_epoch THEN RAISE EXCEPTION 'transition control epoch is stale' USING ERRCODE='40001'; END IF;
  IF transition='ENABLE' THEN
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
         OR NOT EXISTS(SELECT 1 FROM public.owner_beta_recommendation_runs r JOIN public.owner_beta_daily_reservations dr ON dr.run_id=r.id JOIN public.owner_beta_paper_source_runs sr ON sr.run_id=r.id JOIN public.owner_beta_paper_source_slots ss ON ss.slot_id=sr.slot_id JOIN public.owner_beta_paper_source_admissions sa ON sa.slot_id=ss.slot_id JOIN public.owner_beta_daily_admissions a ON a.admission_sha256=sa.admission_sha256 WHERE r.id=(x->>'run_id')::uuid AND r.owner_user_id=c.owner_user_id AND r.status='SUCCEEDED' AND r.trigger_kind='DAILY' AND r.as_of=(x->>'session_date')::date AND dr.owner_user_id=c.owner_user_id AND dr.strategy_config_id=c.strategy_config_id AND dr.session_date=r.as_of AND dr.run_id=r.id AND dr.job_id=r.job_id AND dr.admission_sha256=sa.admission_sha256 AND dr.config_epoch=(v_cohort->>'config_epoch')::bigint AND dr.activation_epoch=(v_cohort->>'activation_epoch')::bigint AND dr.strategy_config_sha256=v_cohort->>'strategy_config_sha256' AND r.daily_pins->>'admission_sha256'=a.admission_sha256 AND a.owner_user_id=c.owner_user_id AND a.range_end=r.as_of AND ss.owner_user_id=c.owner_user_id AND ss.policy_sha256=v_cohort->>'policy_sha256' AND ss.activation_epoch=(v_cohort->>'activation_epoch')::bigint AND ss.session_date=r.as_of AND ss.cohort_sha256=v_cohort->>'cohort_sha256' AND ss.cohort IS NOT DISTINCT FROM v_cohort AND sa.admission_sha256=x->>'admission_sha256' AND sa.source_event_sha256=x->>'source_event_sha256' AND ss.root_event_sha256=x->>'root_event_sha256')) THEN RAISE EXCEPTION 'pre-session evidence list is invalid' USING ERRCODE='23514'; END IF;
    SELECT array_agg((x->>'session_date')::date ORDER BY (x->>'session_date')::date) INTO session_dates FROM jsonb_array_elements(evidence->'session_evidence') x;
    IF session_dates[3] IS DISTINCT FROM expected_session THEN RAISE EXCEPTION 'pre-session evidence does not end at current expected session' USING ERRCODE='23514'; END IF;
    FOR i IN 1..3 LOOP
      SELECT e.effective_session INTO verified_session FROM public.paper_0067_next_session(c.owner_user_id,daily.policy_sha256,CASE WHEN i=1 THEN session_dates[1]-1 ELSE session_dates[i-1] END) e;
      IF verified_session IS DISTINCT FROM session_dates[i] THEN RAISE EXCEPTION 'pre-session evidence is not three consecutive verified sessions' USING ERRCODE='23514'; END IF;
    END LOOP;
    IF NOT EXISTS(SELECT 1 FROM public.owner_beta_daily_control d WHERE d.owner_user_id=c.owner_user_id AND d.enabled AND v_cohort->>'cohort_sha256'=b->>'daily_cohort_sha256') THEN RAISE EXCEPTION 'Daily witness cohort not current' USING ERRCODE='23514'; END IF;
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
  INSERT INTO public.owner_beta_paper_events(id,owner_user_id,account_id,control_epoch,kind,payload,payload_sha256,request_key,pending_target_id)
   VALUES(ev,c.owner_user_id,c.account_id,next_epoch,CASE transition WHEN 'ENABLE' THEN 'ENABLED' WHEN 'DRAIN' THEN 'DRAIN_STARTED' WHEN 'OFF' THEN 'OFF' WHEN 'RESTORED_OFF' THEN 'RESTORED_OFF' ELSE 'RECOVERY_AUTHORIZED' END,evidence,public.daily_0060_digest(evidence::text),request_key,recovery_target);
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
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('TRANSITION','OK',next_epoch,c.account_id,jsonb_build_object('event_id',ev,'state',CASE transition WHEN 'ENABLE' THEN 'ENABLED' WHEN 'DRAIN' THEN 'DRAINING' WHEN 'OFF' THEN 'OFF' WHEN 'RESTORED_OFF' THEN 'RESTORED_OFF' ELSE c.state END));
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.read_owner_beta_paper_state_v1() RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET statement_timeout='5s' AS $$
DECLARE actor uuid:=NULLIF(current_setting('app.actor_user_id',true),'')::uuid; c public.owner_beta_paper_control; d jsonb;
BEGIN
  PERFORM public.paper_0067_principal('app');
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' AND owner_user_id=actor;
  IF NOT FOUND THEN RETURN public.paper_0067_action('READ_STATE','NOT_FOUND',NULL,NULL); END IF;
  IF c.state='ENABLED' THEN
    BEGIN
      PERFORM public.paper_0067_authority('SCAN',(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date,c.control_epoch,c.process_binding_sha256,true);
    EXCEPTION WHEN SQLSTATE '42501' OR SQLSTATE '23514' OR SQLSTATE '40001' THEN
      c.state:='BLOCKED';
    END;
  END IF;
  SELECT jsonb_build_object('control',jsonb_build_object('state',c.state,'control_epoch',c.control_epoch,'strategy_config_id',c.strategy_config_id,'consent_event_id',c.consent_event_id,'cohort_sha256',c.cohort_sha256,'process_binding_sha256',c.process_binding_sha256),
    'sessions',COALESCE(jsonb_agg(jsonb_build_object('pending_target_id',s.pending_target_id,'effective_session',s.effective_session,'state',s.state,'settlement_kind',s.settlement_kind,'reason_code',s.reason_code,'settlement_sha256',s.settlement_sha256,'valuation_sha256',s.valuation_sha256) ORDER BY s.effective_session),'[]'::jsonb))
    INTO d FROM (SELECT * FROM public.owner_beta_paper_sessions WHERE account_id=c.account_id ORDER BY effective_session DESC LIMIT 20) s;
  RETURN public.paper_0067_action('READ_STATE','OK',c.control_epoch,c.account_id,COALESCE(d,'{}'::jsonb));
END $$;

-- One authority path for all preview/economic consumers. It deliberately uses
-- the supplied protected-config byte digest only as a DB binding comparison;
-- Rust/installer still proves the filesystem and running executable.
CREATE FUNCTION public.paper_0067_authority(p_action text,p_session_date date,expected_epoch bigint,process_config_sha256 text,require_enabled boolean DEFAULT true)
RETURNS public.owner_beta_paper_control
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE c public.owner_beta_paper_control; d public.owner_beta_daily_control; sc public.user_strategy_configs; a public.accounts; b public.account_strategy_bindings; old_actor text:=current_setting('app.actor_user_id',true); today date:=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date;
BEGIN
  IF p_action NOT IN ('PREVIEW','APPLY','TARGET','SETTLE','VALUE','WAIT','SCAN') OR p_session_date IS NULL THEN RAISE EXCEPTION 'Paper action identity invalid' USING ERRCODE='22023'; END IF;
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN RAISE EXCEPTION 'Owner-beta Paper is unconfigured' USING ERRCODE='42501'; END IF;
  PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(c.account_id::text,381901));
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT * INTO a FROM public.accounts WHERE id=c.account_id AND owner_user_id=c.owner_user_id FOR UPDATE;
  IF NOT FOUND OR a.account_type<>'PAPER' OR a.status<>'ACTIVE' OR a.currency<>'KRW'
     OR a.initial_cash<>10000000.0000 OR a.cost_profile_id<>c.cost_profile_id OR a.cost_profile_version<>c.cost_profile_version THEN RAISE EXCEPTION 'bound Paper account changed' USING ERRCODE='23514'; END IF;
  IF expected_epoch IS DISTINCT FROM c.control_epoch OR (require_enabled AND c.state<>'ENABLED') THEN RAISE EXCEPTION 'Owner-beta Paper control epoch/state stale' USING ERRCODE='40001'; END IF;
  IF (require_enabled OR c.process_binding IS NOT NULL) AND (process_config_sha256 IS NULL OR process_config_sha256 IS DISTINCT FROM c.process_binding_sha256 OR c.process_binding IS NULL) THEN RAISE EXCEPTION 'Owner-beta Paper process binding unavailable' USING ERRCODE='42501'; END IF;
  IF require_enabled AND (NOT public.daily_0060_keys(c.process_binding,ARRAY['schema','owner_user_id','account_id','strategy_config_id','consent_event_id','consent_version','control_epoch','daily_cohort_sha256','financial_contract_sha256','code_commit','release_manifest_sha256','api_binary_sha256','worker_binary_sha256'])
     OR c.process_binding->>'schema'<>'owner-beta-paper-process-v1' OR c.process_binding->>'owner_user_id' IS DISTINCT FROM c.owner_user_id::text
     OR c.process_binding->>'account_id' IS DISTINCT FROM c.account_id::text OR c.process_binding->>'strategy_config_id' IS DISTINCT FROM c.strategy_config_id::text
     OR c.process_binding->>'consent_event_id' IS DISTINCT FROM c.consent_event_id::text OR c.process_binding->>'consent_version' IS DISTINCT FROM c.contract_id
     OR c.process_binding->>'control_epoch' !~ '^(0|[1-9][0-9]{0,18})$' OR (c.process_binding->>'control_epoch')::bigint IS DISTINCT FROM c.control_epoch
     OR c.process_binding->>'daily_cohort_sha256' IS DISTINCT FROM c.cohort_sha256 OR c.process_binding->>'financial_contract_sha256' IS DISTINCT FROM c.contract_sha256) THEN RAISE EXCEPTION 'current Paper process binding identity changed' USING ERRCODE='42501'; END IF;
  IF c.daily_cohort IS NOT NULL THEN
    SELECT * INTO d FROM public.owner_beta_daily_control WHERE id='etf11' AND owner_user_id=c.owner_user_id FOR SHARE;
    IF NOT FOUND OR NOT d.enabled THEN RAISE EXCEPTION 'Daily source authority unavailable' USING ERRCODE='42501'; END IF;
    PERFORM public.paper_0066_authority(c.daily_cohort,GREATEST(p_session_date,today));
  ELSIF require_enabled THEN RAISE EXCEPTION 'Daily witness cohort unavailable' USING ERRCODE='42501'; END IF;
  SELECT * INTO sc FROM public.user_strategy_configs WHERE id=c.strategy_config_id AND owner_user_id=c.owner_user_id FOR SHARE;
  IF NOT FOUND OR NOT sc.is_active OR NOT sc.daily_enabled OR (c.daily_cohort IS NOT NULL AND sc.daily_epoch IS DISTINCT FROM (c.daily_cohort->>'config_epoch')::bigint) THEN RAISE EXCEPTION 'Paper strategy configuration changed' USING ERRCODE='23514'; END IF;
  SELECT * INTO b FROM public.account_strategy_bindings WHERE account_id=c.account_id AND owner_user_id=c.owner_user_id AND strategy_config_id=c.strategy_config_id AND unbound_at IS NULL FOR SHARE;
  IF NOT FOUND THEN RAISE EXCEPTION 'Paper account strategy binding changed' USING ERRCODE='23514'; END IF;
  IF require_enabled AND (c.consent_event_id IS NULL OR NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_events e WHERE e.id=c.consent_event_id AND e.account_id=c.account_id AND e.owner_user_id=c.owner_user_id AND e.kind='CONSENT_GRANTED' AND e.actor_user_id=c.owner_user_id)) THEN RAISE EXCEPTION 'current Paper consent unavailable' USING ERRCODE='42501'; END IF;
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
  RETURN c;
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.paper_0067_next_session(p_owner uuid,p_policy text,p_after date)
RETURNS TABLE(effective_session date,calendar_evidence_sha256 text,calendar_link_json jsonb)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$
DECLARE candidate date; evidence text; link jsonb; civil_count integer; valid_count integer; day_rows jsonb; observed_at timestamptz:=clock_timestamp();
BEGIN
  SELECT min(d.session_date) INTO candidate FROM public.owner_beta_daily_calendar_days d
   JOIN public.owner_beta_daily_calendar_sources s USING(owner_user_id,calendar_evidence_sha256)
   WHERE d.owner_user_id=p_owner AND s.policy_sha256=p_policy AND d.session_date>p_after AND d.session_type='TRADING'
     AND s.source_retrieved_at<=observed_at AND s.projected_at<=observed_at;
  IF candidate IS NULL OR candidate-p_after NOT BETWEEN 1 AND 370 THEN RETURN; END IF;
  IF EXISTS(SELECT 1 FROM public.owner_beta_daily_calendar_days d JOIN public.owner_beta_daily_calendar_sources s USING(owner_user_id,calendar_evidence_sha256)
      WHERE d.owner_user_id=p_owner AND s.policy_sha256=p_policy AND d.session_date BETWEEN p_after+1 AND candidate
      AND (s.source_retrieved_at>observed_at OR s.projected_at>observed_at)) THEN RETURN; END IF;
  WITH civil AS (
    SELECT x::date AS session_date FROM generate_series(p_after+1,candidate,interval '1 day') AS g(x)
  ), policy_days AS (
    SELECT d.* FROM public.owner_beta_daily_calendar_days d
    JOIN public.owner_beta_daily_calendar_sources s USING(owner_user_id,calendar_evidence_sha256)
    WHERE d.owner_user_id=p_owner AND s.policy_sha256=p_policy AND d.session_date BETWEEN p_after+1 AND candidate
      AND s.source_retrieved_at<=observed_at AND s.projected_at<=observed_at
  ), selected AS (
    SELECT DISTINCT ON(pd.session_date) pd.* FROM policy_days pd
    ORDER BY pd.session_date,pd.calendar_evidence_sha256,pd.source_row_ordinal,pd.source_row_sha256
  ), per_day AS (
    SELECT c.session_date,count(d.session_date) AS row_count,
           count(DISTINCT d.session_type) AS type_count,
           min(d.session_type) AS session_type
    FROM civil c LEFT JOIN policy_days d USING(session_date)
    GROUP BY c.session_date
  )
  SELECT count(*),count(*) FILTER(WHERE p.row_count>0 AND p.type_count=1),
         jsonb_agg(jsonb_build_object('date',p.session_date,'kind',p.session_type,'calendar_evidence_sha256',s.calendar_evidence_sha256,'row_ordinal',s.source_row_ordinal,'row_sha256',s.source_row_sha256) ORDER BY p.session_date)
    INTO civil_count,valid_count,day_rows FROM per_day p LEFT JOIN selected s USING(session_date);
  IF civil_count IS DISTINCT FROM (candidate-p_after) OR valid_count IS DISTINCT FROM civil_count THEN RETURN; END IF;
  evidence:=day_rows->-1->>'calendar_evidence_sha256';
  link:=jsonb_build_object('schema','owner-beta-paper-calendar-link-v1','after',p_after,'effective_session',candidate,'calendar_evidence_sha256',evidence,'covered_days',day_rows);
  RETURN QUERY SELECT candidate,evidence,link;
END $$;

CREATE FUNCTION public.submit_owner_beta_paper_preview_v1(account_id uuid,run_id uuid,expected_epoch bigint,request_key text,process_config_sha256 text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE actor uuid:=NULLIF(current_setting('app.actor_user_id',true),'')::uuid; c public.owner_beta_paper_control; r public.owner_beta_recommendation_runs; res public.owner_beta_daily_reservations; source_run public.owner_beta_paper_source_runs; source_slot public.owner_beta_paper_source_slots; adm public.owner_beta_daily_admissions; e record; prior_lineage public.owner_beta_paper_lineage; lineage uuid; preview uuid; job uuid; arr jsonb; weight numeric; alloc_hash text; cash text; positions jsonb; account_version bigint; source_readback jsonb; old_actor text:=current_setting('app.actor_user_id',true); prior public.paper_rebalance_previews;
BEGIN
  PERFORM public.paper_0067_principal('app');
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' AND owner_user_id=actor AND owner_beta_paper_control.account_id=submit_owner_beta_paper_preview_v1.account_id FOR UPDATE;
  IF NOT FOUND THEN RETURN public.paper_0067_action('SUBMIT_PREVIEW','NOT_FOUND',NULL,NULL); END IF;
  c:=public.paper_0067_authority('PREVIEW',(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date,expected_epoch,process_config_sha256,true);
  IF actor IS DISTINCT FROM c.owner_user_id OR request_key IS NULL OR length(request_key) NOT BETWEEN 1 AND 128 THEN RAISE EXCEPTION 'preview request identity invalid' USING ERRCODE='42501'; END IF;
  SELECT * INTO prior FROM public.paper_rebalance_previews WHERE owner_user_id=c.owner_user_id AND beta_request_key=submit_owner_beta_paper_preview_v1.request_key FOR UPDATE;
  IF FOUND THEN RETURN public.paper_0067_action('SUBMIT_PREVIEW','REPLAY',c.control_epoch,c.account_id,jsonb_build_object('preview_id',prior.id,'job_id',prior.job_id,'effective_session',prior.proposed_effective_date)); END IF;
  SELECT * INTO STRICT r FROM public.owner_beta_recommendation_runs rr0 WHERE rr0.id=submit_owner_beta_paper_preview_v1.run_id AND rr0.owner_user_id=c.owner_user_id FOR SHARE;
  IF r.status<>'SUCCEEDED' OR r.trigger_kind<>'DAILY' OR r.as_of<>(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date OR r.strategy_config_id<>c.strategy_config_id OR r.daily_pins IS NULL OR r.factor_snapshot_sha256 IS NULL OR r.target_snapshot_sha256 IS NULL OR r.cash_weight IS NULL THEN RAISE EXCEPTION 'Daily signal is not current and terminal' USING ERRCODE='23514'; END IF;
  SELECT * INTO STRICT res FROM public.owner_beta_daily_reservations dr0 WHERE dr0.run_id=r.id AND dr0.owner_user_id=c.owner_user_id FOR SHARE;
  SELECT * INTO STRICT source_run FROM public.owner_beta_paper_source_runs sr0 WHERE sr0.run_id=r.id FOR SHARE;
  SELECT * INTO STRICT adm FROM public.owner_beta_daily_admissions WHERE admission_sha256=r.daily_pins->>'admission_sha256' AND owner_user_id=c.owner_user_id FOR SHARE;
  SELECT * INTO STRICT source_slot FROM public.owner_beta_paper_source_slots WHERE slot_id=source_run.slot_id FOR SHARE;
  source_readback:=public.paper_0066_read(source_slot);
  IF res.owner_user_id<>c.owner_user_id OR res.strategy_config_id<>c.strategy_config_id OR res.session_date<>r.as_of OR res.run_id<>r.id OR res.job_id<>r.job_id
     OR res.admission_sha256<>adm.admission_sha256 OR source_run.job_id<>r.job_id OR source_slot.owner_user_id<>c.owner_user_id
     OR source_slot.session_date<>r.as_of OR source_slot.policy_sha256<>(c.daily_cohort->>'policy_sha256')
     OR source_slot.activation_epoch<>(c.daily_cohort->>'activation_epoch')::bigint OR source_slot.cohort IS DISTINCT FROM c.daily_cohort
     OR source_readback->>'candidate_run_id'<>r.id::text OR source_readback->>'terminal'<>'SUCCEEDED'
     OR source_readback->>'candidate_admission_sha256'<>adm.admission_sha256
     OR source_readback->>'job_id'<>source_run.job_id::text OR source_readback->>'attempt_id'<>source_run.attempt_id::text
     OR r.daily_pins IS DISTINCT FROM jsonb_build_object('admission_sha256',adm.admission_sha256,'content_sha256',adm.content_sha256,'manifest_sha256',adm.manifest_sha256,'policy_sha256',adm.policy_sha256)
     OR r.daily_config_epoch<>res.config_epoch OR r.daily_activation_epoch<>res.activation_epoch
     OR res.config_epoch<>(c.daily_cohort->>'config_epoch')::bigint OR res.activation_epoch<>(c.daily_cohort->>'activation_epoch')::bigint
     OR res.strategy_config_sha256<>c.daily_cohort->>'strategy_config_sha256'
     OR NOT public.daily_0060_admission_usable((SELECT d FROM public.owner_beta_daily_control d WHERE id='etf11'),adm.admission_sha256,false) THEN RAISE EXCEPTION 'Daily admission or source lineage is no longer usable' USING ERRCODE='42501'; END IF;
  IF (SELECT count(*) FROM public.owner_beta_recommendation_items i WHERE i.recommendation_run_id=r.id AND i.owner_user_id=c.owner_user_id)<>11
     OR EXISTS(SELECT 1 FROM public.owner_beta_recommendation_items i WHERE i.recommendation_run_id=r.id AND i.owner_user_id=c.owner_user_id AND i.instrument_id NOT IN ('069500.KRX','102110.KRX','114260.KRX','132030.KRX','133690.KRX','143850.KRX','148070.KRX','153130.KRX','192090.KRX','195930.KRX','229200.KRX')) THEN RAISE EXCEPTION 'Daily recommendation must contain the exact ETF11 rows' USING ERRCODE='23514'; END IF;
  IF EXISTS(SELECT 1 FROM public.owner_beta_recommendation_items i WHERE i.recommendation_run_id=r.id AND COALESCE(i.target_weight,0)<>0 AND i.instrument_id<>'069500.KRX') OR EXISTS(SELECT 1 FROM public.owner_beta_recommendation_items i WHERE i.recommendation_run_id=r.id AND (i.target_weight IS NULL OR (i.excluded AND i.target_weight<>0))) OR r.cash_weight NOT IN (0,1) THEN RAISE EXCEPTION 'allocation outside adopted ETF11 signal' USING ERRCODE='23514'; END IF;
  SELECT COALESCE(max(target_weight),0) INTO weight FROM public.owner_beta_recommendation_items WHERE recommendation_run_id=r.id AND owner_user_id=c.owner_user_id AND instrument_id='069500.KRX' AND NOT excluded;
  IF weight NOT IN (0,1) OR (weight=0 AND r.cash_weight<>1) OR (weight=1 AND r.cash_weight<>0) THEN RAISE EXCEPTION 'allocation weights violate fixed strategy' USING ERRCODE='23514'; END IF;
  SELECT * INTO e FROM public.paper_0067_next_session(c.owner_user_id,(SELECT policy_sha256 FROM public.owner_beta_daily_control WHERE id='etf11'),r.as_of);
  IF e.effective_session IS NULL THEN RAISE EXCEPTION 'verified next trading calendar unavailable' USING ERRCODE='55000'; END IF;
  SELECT COALESCE(jsonb_agg(jsonb_build_object('instrument_id','069500.KRX','weight',to_char(weight,'FM0.000000')) ORDER BY '069500.KRX'),'[]'::jsonb) INTO arr WHERE weight>0;
  IF arr IS NULL THEN arr:='[]'::jsonb; END IF;
  alloc_hash:=public.daily_0060_digest(jsonb_build_object('schema','owner-beta-paper-signal-allocation-v1','instrument_id','069500.KRX','weight',to_char(weight,'FM0.000000'),'cash_weight',to_char(1-weight,'FM0.000000'))::text);
  SELECT * INTO prior_lineage FROM public.owner_beta_paper_lineage AS selected_lineage
   WHERE selected_lineage.account_id=c.account_id AND selected_lineage.recommendation_run_id=r.id AND selected_lineage.effective_session=e.effective_session
     AND selected_lineage.control_epoch=c.control_epoch AND selected_lineage.consent_event_id=c.consent_event_id
   FOR UPDATE;
  IF FOUND THEN
    IF prior_lineage.owner_user_id IS DISTINCT FROM c.owner_user_id
       OR prior_lineage.strategy_config_id IS DISTINCT FROM c.strategy_config_id
       OR prior_lineage.signal_date IS DISTINCT FROM r.as_of
       OR prior_lineage.signal_admission_sha256 IS DISTINCT FROM adm.admission_sha256
       OR prior_lineage.factor_snapshot_sha256 IS DISTINCT FROM r.factor_snapshot_sha256
       OR prior_lineage.target_snapshot_sha256 IS DISTINCT FROM r.target_snapshot_sha256
       OR prior_lineage.target_weights_json IS DISTINCT FROM arr
       OR prior_lineage.config_epoch IS DISTINCT FROM res.config_epoch
       OR prior_lineage.daily_activation_epoch IS DISTINCT FROM res.activation_epoch
       OR prior_lineage.activation_event_id IS DISTINCT FROM c.activation_event_id
       OR prior_lineage.cohort_sha256 IS DISTINCT FROM c.cohort_sha256
       OR prior_lineage.calendar_evidence_sha256 IS DISTINCT FROM e.calendar_evidence_sha256
    THEN RAISE EXCEPTION 'manual preview lineage conflicts with automatic target' USING ERRCODE='23505'; END IF;
    lineage:=prior_lineage.id;
  ELSE
    lineage:=gen_random_uuid();
    INSERT INTO public.owner_beta_paper_lineage(id,owner_user_id,account_id,strategy_config_id,recommendation_run_id,signal_date,effective_session,signal_admission_sha256,factor_snapshot_sha256,target_snapshot_sha256,target_weights_json,config_epoch,daily_activation_epoch,control_epoch,consent_event_id,activation_event_id,cohort_sha256,calendar_evidence_sha256,calendar_link_json)
     VALUES(lineage,c.owner_user_id,c.account_id,c.strategy_config_id,r.id,r.as_of,e.effective_session,adm.admission_sha256,r.factor_snapshot_sha256,r.target_snapshot_sha256,arr,res.config_epoch,res.activation_epoch,c.control_epoch,c.consent_event_id,c.activation_event_id,c.cohort_sha256,e.calendar_evidence_sha256,e.calendar_link_json);
  END IF;
  SELECT a.paper_state_version INTO account_version FROM public.accounts a WHERE a.id=c.account_id AND a.owner_user_id=c.owner_user_id;
  SELECT (SELECT l.balance::text FROM public.cash_ledger l WHERE l.account_id=c.account_id ORDER BY l.seq DESC LIMIT 1),
         COALESCE((SELECT jsonb_object_agg(p.instrument_id,p.quantity::text ORDER BY p.instrument_id) FROM public.positions p WHERE p.account_id=c.account_id AND p.quantity<>0),'{}'::jsonb)
    INTO cash,positions;
  IF cash IS NULL THEN RAISE EXCEPTION 'Paper cash ledger unavailable' USING ERRCODE='23514'; END IF;
  preview:=gen_random_uuid(); job:=gen_random_uuid();
  INSERT INTO public.jobs(id,owner_user_id,job_type,status,priority,idempotency_key,payload_json)
   VALUES(job,c.owner_user_id,'owner_beta_paper_preview','QUEUED',10,'owner-beta-paper-preview:'||c.account_id::text||':'||request_key,jsonb_build_object('schema','owner-beta-paper-preview-job-v1','preview_id',preview,'input_kind','OWNER_BETA_DAILY_V1','apply_mode','MANUAL'));
  INSERT INTO public.paper_rebalance_previews(id,owner_user_id,account_id,strategy_config_id,job_id,status,price_basis,price_date,preview_seoul_date,proposed_effective_date,dataset_manifest_sha256,target_portfolio_sha256,input_kind,beta_apply_mode,beta_lineage_id,beta_request_key,account_state_version,signal_allocation_sha256)
   VALUES(preview,c.owner_user_id,c.account_id,c.strategy_config_id,job,'PENDING','RECOMMENDATION_CLOSE',r.as_of,(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date,e.effective_session,NULL,substring(r.target_snapshot_sha256 from 8),'OWNER_BETA_DAILY_V1','MANUAL',lineage,request_key,account_version,alloc_hash);
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
  RETURN public.paper_0067_action('SUBMIT_PREVIEW','OK',c.control_epoch,c.account_id,jsonb_build_object('preview_id',preview,'job_id',job,'lineage_id',lineage,'effective_session',e.effective_session,'account_state_preimage',jsonb_build_object('schema_version',1,'paper_state_version',account_version,'cash',cash,'positions',positions)));
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.paper_0067_apply_target(lineage_id uuid,origin text,state_version bigint,state_sha256 text,state_preimage jsonb,signal_allocation_sha256 text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE l public.owner_beta_paper_lineage; c public.owner_beta_paper_control; d public.owner_beta_daily_control; existing public.pending_targets; target uuid; source text; processing text; s public.owner_beta_paper_sessions; old_actor text:=current_setting('app.actor_user_id',true);
BEGIN
  IF origin NOT IN ('MANUAL','AUTO') OR state_sha256 !~ '^[0-9a-f]{64}$' OR signal_allocation_sha256 !~ '^sha256:[0-9a-f]{64}$' OR state_version IS NULL OR jsonb_typeof(state_preimage)<>'object' THEN RAISE EXCEPTION 'beta target application input invalid' USING ERRCODE='22023'; END IF;
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN RAISE EXCEPTION 'Paper control unavailable' USING ERRCODE='42501'; END IF;
  PERFORM pg_advisory_xact_lock(hashtextextended(c.account_id::text,381901));
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT * INTO l FROM public.owner_beta_paper_lineage WHERE id=lineage_id AND account_id=c.account_id AND owner_user_id=c.owner_user_id FOR SHARE;
  IF NOT FOUND OR l.control_epoch<>c.control_epoch OR l.consent_event_id<>c.consent_event_id THEN RAISE EXCEPTION 'beta target lineage stale' USING ERRCODE='40001'; END IF;
  source:=CASE origin WHEN 'MANUAL' THEN 'OWNER_BETA_DAILY_MANUAL' ELSE 'OWNER_BETA_DAILY_AUTO' END;
  processing:=origin;
  SELECT * INTO existing FROM public.pending_targets WHERE account_id=c.account_id AND effective_date=l.effective_session FOR UPDATE;
  IF FOUND THEN
    IF existing.beta_lineage_id IS DISTINCT FROM l.id OR existing.strategy_config_id IS DISTINCT FROM l.strategy_config_id OR existing.computed_on IS DISTINCT FROM l.signal_date OR existing.targets_json IS DISTINCT FROM l.target_weights_json OR existing.source_kind NOT IN ('OWNER_BETA_DAILY_MANUAL','OWNER_BETA_DAILY_AUTO') THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN jsonb_build_object('outcome','CONFLICT','pending_target_id',NULL); END IF;
    SELECT * INTO s FROM public.owner_beta_paper_sessions WHERE pending_target_id=existing.id FOR UPDATE;
    IF NOT FOUND THEN RAISE EXCEPTION 'beta target missing session projection' USING ERRCODE='23514'; END IF;
    UPDATE public.owner_beta_paper_control SET last_signal_allocation_sha256=signal_allocation_sha256,last_signal_date=l.signal_date WHERE id='etf11' AND last_signal_allocation_sha256 IS DISTINCT FROM signal_allocation_sha256;
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
    RETURN jsonb_build_object('outcome','REPLAY','pending_target_id',existing.id,'effective_session',l.effective_session,'lineage_id',l.id,'session_state',s.state,'processing_origin',s.processing_origin);
  END IF;
  IF EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions earlier WHERE earlier.account_id=c.account_id AND earlier.effective_session<l.effective_session AND earlier.state<>'VALUED') THEN
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
    RETURN jsonb_build_object('outcome','WAITING_EARLIER_SESSION');
  END IF;
  SELECT * INTO d FROM public.owner_beta_daily_control WHERE id='etf11' AND owner_user_id=c.owner_user_id FOR SHARE;
  IF NOT FOUND THEN RAISE EXCEPTION 'beta target Daily control unavailable' USING ERRCODE='42501'; END IF;
  PERFORM public.daily_0060_source_lock(c.owner_user_id,false);
  IF NOT public.daily_0060_admission_usable(d,l.signal_admission_sha256,false) THEN
    RAISE EXCEPTION 'beta target signal admission unavailable' USING ERRCODE='42501';
  END IF;
  INSERT INTO public.pending_targets(account_id,owner_user_id,strategy_config_id,computed_on,effective_date,targets_json,status,source_kind,beta_lineage_id)
   VALUES(c.account_id,c.owner_user_id,l.strategy_config_id,l.signal_date,l.effective_session,l.target_weights_json,'PENDING',source,l.id) RETURNING id INTO target;
  INSERT INTO public.owner_beta_paper_sessions(pending_target_id,owner_user_id,account_id,effective_session,account_state_version,account_state_sha256,state,processing_origin,receipt_json)
   VALUES(target,c.owner_user_id,c.account_id,l.effective_session,state_version,state_sha256,'PENDING',processing,jsonb_build_object('lineage_id',l.id,'account_state_preimage',state_preimage,'target_snapshot_sha256',l.target_snapshot_sha256,'signal_allocation_sha256',signal_allocation_sha256));
  UPDATE public.owner_beta_paper_control SET last_signal_allocation_sha256=signal_allocation_sha256,last_signal_date=l.signal_date WHERE id='etf11' AND last_signal_allocation_sha256 IS DISTINCT FROM signal_allocation_sha256;
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true);
  RETURN jsonb_build_object('outcome','OK','pending_target_id',target,'effective_session',l.effective_session,'lineage_id',l.id,'session_state','PENDING');
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

-- Each worker scope saves the caller actor, selects the configured control
-- Owner with LOCAL set_config after the direct-login principal guard, and
-- restores it on every return/error. No function SET option or caller GUC
-- grants authority; tenant reads finish before restoring the caller actor.
CREATE FUNCTION public.paper_0067_apply_auto_preview(p_preview_id uuid,p_expected_epoch bigint,p_process_config_sha256 text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE p public.paper_rebalance_previews; c public.owner_beta_paper_control; l public.owner_beta_paper_lineage; b public.account_strategy_bindings; ver bigint; cash text; pos jsonb; preimage jsonb; applied jsonb; target uuid; today date:=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date; old_actor text:=current_setting('app.actor_user_id',true);
BEGIN
  PERFORM public.paper_0067_principal('worker');
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('AUTO_APPLY_PREVIEW','NOT_FOUND',NULL,NULL); END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT p0.* INTO p FROM public.paper_rebalance_previews p0 WHERE p0.id=p_preview_id AND p0.input_kind='OWNER_BETA_DAILY_V1' AND p0.beta_apply_mode='AUTO' AND p0.owner_user_id=c.owner_user_id AND p0.account_id=c.account_id;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('AUTO_APPLY_PREVIEW','NOT_FOUND',NULL,NULL); END IF;
  c:=public.paper_0067_authority('APPLY',p.price_date,p_expected_epoch,p_process_config_sha256,true);
  SELECT * INTO p FROM public.paper_rebalance_previews WHERE id=p_preview_id AND input_kind='OWNER_BETA_DAILY_V1' AND beta_apply_mode='AUTO' AND owner_user_id=c.owner_user_id AND account_id=c.account_id FOR UPDATE;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('AUTO_APPLY_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  IF p.status='APPLIED' THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('AUTO_APPLY_PREVIEW','REPLAY',c.control_epoch,c.account_id,jsonb_build_object('preview_id',p.id,'pending_target_id',p.pending_target_id,'effective_session',p.proposed_effective_date)); END IF;
  IF p.status<>'READY' OR p.preview_seoul_date<>today OR p.price_date<>today OR p.proposed_effective_date<=today
     OR p.result_json->>'schema'<>'owner-beta-paper-preview-result-v1'
     OR p.result_json->>'signal_allocation_sha256' IS DISTINCT FROM p.signal_allocation_sha256
  THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('AUTO_APPLY_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT * INTO b FROM public.account_strategy_bindings WHERE account_id=c.account_id AND owner_user_id=c.owner_user_id AND strategy_config_id=c.strategy_config_id AND unbound_at IS NULL FOR SHARE;
  IF NOT FOUND OR NOT b.auto_apply_recommendations THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('AUTO_APPLY_PREVIEW','CONSENT_REQUIRED',c.control_epoch,c.account_id); END IF;
  SELECT * INTO l FROM public.owner_beta_paper_lineage WHERE id=p.beta_lineage_id AND owner_user_id=c.owner_user_id AND account_id=c.account_id FOR SHARE;
  IF NOT FOUND OR l.control_epoch<>c.control_epoch OR l.consent_event_id<>c.consent_event_id OR l.effective_session<>p.proposed_effective_date THEN RAISE EXCEPTION 'automatic preview lineage changed' USING ERRCODE='40001'; END IF;
  IF p.result_json->>'target_snapshot_sha256' IS DISTINCT FROM l.target_snapshot_sha256 THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('AUTO_APPLY_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  SELECT a.paper_state_version,(SELECT cl.balance::text FROM public.cash_ledger cl WHERE cl.account_id=a.id AND cl.owner_user_id=a.owner_user_id ORDER BY cl.seq DESC LIMIT 1),COALESCE((SELECT jsonb_object_agg(x.instrument_id,x.quantity::text ORDER BY x.instrument_id) FROM public.positions x WHERE x.account_id=a.id AND x.owner_user_id=a.owner_user_id AND x.quantity<>0),'{}'::jsonb)
    INTO ver,cash,pos FROM public.accounts a WHERE a.id=c.account_id AND a.owner_user_id=c.owner_user_id FOR UPDATE;
  preimage:=jsonb_build_object('schema_version',1,'paper_state_version',ver,'cash',cash,'positions',pos);
  IF ver IS DISTINCT FROM p.account_state_version OR p.account_state_sha256 IS DISTINCT FROM public.paper_0067_account_state_sha256(ver,cash,pos) OR p.account_state_preimage IS DISTINCT FROM preimage THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('AUTO_APPLY_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  applied:=public.paper_0067_apply_target(l.id,'AUTO',ver,p.account_state_sha256,preimage,p.signal_allocation_sha256);
  IF applied->>'outcome'='WAITING_EARLIER_SESSION' THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('AUTO_APPLY_PREVIEW','WAITING_EARLIER_SESSION',c.control_epoch,c.account_id,jsonb_build_object('preview_id',p.id,'effective_session',l.effective_session)); END IF;
  IF applied->>'outcome'='CONFLICT' OR (applied->>'outcome'='REPLAY' AND applied->>'processing_origin'<>'AUTO') THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('AUTO_APPLY_PREVIEW','CONFLICT',c.control_epoch,c.account_id); END IF;
  target:=(applied->>'pending_target_id')::uuid;
  UPDATE public.paper_rebalance_previews SET status='APPLIED',pending_target_id=target,applied_at=clock_timestamp(),updated_at=clock_timestamp() WHERE id=p.id AND status='READY';
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('AUTO_APPLY_PREVIEW','OK',c.control_epoch,c.account_id,jsonb_build_object('preview_id',p.id,'pending_target_id',target,'effective_session',l.effective_session,'lineage_id',l.id,'processing_origin','AUTO'));
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.apply_owner_beta_paper_preview_auto_v1(preview_id uuid,expected_epoch bigint,process_config_sha256 text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
BEGIN
  PERFORM public.paper_0067_principal('worker');
  RETURN public.paper_0067_apply_auto_preview(preview_id,expected_epoch,process_config_sha256);
END $$;

CREATE FUNCTION public.snapshot_owner_beta_paper_preview_v1(preview_id uuid,job_id uuid,process_config_sha256 text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE old_actor text:=current_setting('app.actor_user_id',true); p public.paper_rebalance_previews; c public.owner_beta_paper_control; j public.jobs; attempt public.job_attempts; ver bigint; cash text; pos jsonb; l public.owner_beta_paper_lineage; r public.owner_beta_recommendation_runs; preimage jsonb; state_hash text; previous_target jsonb; job_claimed boolean;
BEGIN
  PERFORM public.paper_0067_principal('worker');
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('SNAPSHOT_PREVIEW','NOT_FOUND',NULL,NULL); END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT p0.* INTO p FROM public.paper_rebalance_previews p0 WHERE p0.id=snapshot_owner_beta_paper_preview_v1.preview_id AND p0.input_kind='OWNER_BETA_DAILY_V1' AND p0.owner_user_id=c.owner_user_id AND p0.account_id=c.account_id;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('SNAPSHOT_PREVIEW','NOT_FOUND',NULL,NULL); END IF;
  c:=public.paper_0067_authority('PREVIEW',p.price_date,c.control_epoch,process_config_sha256,true);
  -- Match the terminal projection's jobs -> preview order.
  SELECT * INTO j FROM public.jobs WHERE id=snapshot_owner_beta_paper_preview_v1.job_id AND owner_user_id=c.owner_user_id AND job_type='owner_beta_paper_preview' AND status='RUNNING' AND locked_at>clock_timestamp()-interval '60 seconds' FOR UPDATE;
  job_claimed:=FOUND;
  SELECT * INTO p FROM public.paper_rebalance_previews WHERE id=snapshot_owner_beta_paper_preview_v1.preview_id AND input_kind='OWNER_BETA_DAILY_V1' AND owner_user_id=c.owner_user_id AND account_id=c.account_id FOR UPDATE;
  IF NOT FOUND OR p.job_id<>snapshot_owner_beta_paper_preview_v1.job_id THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('SNAPSHOT_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  IF NOT job_claimed THEN RAISE EXCEPTION 'beta preview job claim unavailable' USING ERRCODE='42501'; END IF;
  SELECT * INTO attempt FROM public.job_attempts ja0 WHERE ja0.job_id=j.id AND ja0.attempt_no=j.attempt_count AND ja0.outcome='RUNNING' AND ja0.claimed_by=j.locked_by AND ja0.started_at IS NOT NULL FOR SHARE;
  IF NOT FOUND OR p.status NOT IN ('PENDING','RUNNING') OR p.account_id<>c.account_id OR p.beta_lineage_id IS NULL THEN RAISE EXCEPTION 'beta preview job claim unavailable' USING ERRCODE='42501'; END IF;
  SELECT * INTO l FROM public.owner_beta_paper_lineage WHERE id=p.beta_lineage_id AND owner_user_id=c.owner_user_id AND account_id=c.account_id FOR SHARE;
  IF NOT FOUND OR l.recommendation_run_id IS NULL OR l.control_epoch<>c.control_epoch OR l.consent_event_id<>c.consent_event_id THEN RAISE EXCEPTION 'beta preview lineage stale' USING ERRCODE='40001'; END IF;
  SELECT * INTO r FROM public.owner_beta_recommendation_runs WHERE id=l.recommendation_run_id AND owner_user_id=c.owner_user_id AND status='SUCCEEDED' FOR SHARE;
  IF NOT FOUND OR r.as_of<>l.signal_date OR r.strategy_config_id<>c.strategy_config_id OR r.daily_pins->>'admission_sha256'<>l.signal_admission_sha256 OR r.factor_snapshot_sha256<>l.factor_snapshot_sha256 OR r.target_snapshot_sha256<>l.target_snapshot_sha256 THEN RAISE EXCEPTION 'beta T recommendation pins changed' USING ERRCODE='40001'; END IF;
  SELECT t.targets_json INTO previous_target
    FROM public.owner_beta_paper_sessions s
    JOIN public.pending_targets t ON t.id=s.pending_target_id AND t.beta_lineage_id IS NOT NULL
   WHERE s.account_id=c.account_id AND s.owner_user_id=c.owner_user_id AND s.effective_session<l.effective_session
   ORDER BY s.effective_session DESC LIMIT 1;
  SELECT a.paper_state_version,(SELECT cl.balance::text FROM public.cash_ledger cl WHERE cl.account_id=a.id AND cl.owner_user_id=a.owner_user_id ORDER BY cl.seq DESC LIMIT 1),
    COALESCE((SELECT jsonb_object_agg(x.instrument_id,x.quantity::text ORDER BY x.instrument_id) FROM public.positions x WHERE x.account_id=a.id AND x.owner_user_id=a.owner_user_id AND x.quantity<>0),'{}'::jsonb)
    INTO ver,cash,pos FROM public.accounts a WHERE a.id=c.account_id AND a.owner_user_id=c.owner_user_id;
  IF cash IS NULL THEN RAISE EXCEPTION 'beta Paper cash is unavailable' USING ERRCODE='23514'; END IF;
  preimage:=jsonb_build_object('schema_version',1,'paper_state_version',ver,'cash',cash,'positions',pos);
  state_hash:=public.paper_0067_account_state_sha256(ver,cash,pos);
  UPDATE public.paper_rebalance_previews SET status='RUNNING',started_at=COALESCE(started_at,clock_timestamp()),updated_at=clock_timestamp(),account_state_version=ver,account_state_sha256=state_hash,account_state_preimage=preimage WHERE id=p.id;
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('SNAPSHOT_PREVIEW','OK',c.control_epoch,c.account_id,jsonb_build_object('preview_id',p.id,'job_id',j.id,'lineage_id',l.id,'apply_mode',p.beta_apply_mode,'owner_user_id',c.owner_user_id,'account_id',c.account_id,'strategy_config_id',c.strategy_config_id,'recommendation_run_id',r.id,'signal_date',l.signal_date,'effective_session',l.effective_session,'daily_pins',r.daily_pins,'signal_admission_sha256',l.signal_admission_sha256,'factor_snapshot_sha256',l.factor_snapshot_sha256,'target_snapshot_sha256',l.target_snapshot_sha256,'signal_allocation_sha256',p.signal_allocation_sha256,'target_weights',l.target_weights_json,'previous_target_weights',previous_target,'config_epoch',l.config_epoch,'daily_activation_epoch',l.daily_activation_epoch,'cohort_sha256',l.cohort_sha256,'current_cohort',c.daily_cohort,'consent_event_id',c.consent_event_id,'control_epoch',c.control_epoch,'calendar_evidence_sha256',l.calendar_evidence_sha256,'calendar_link',l.calendar_link_json,'account_state_version',ver,'account_state_sha256',state_hash,'account_state_preimage',preimage,'cost_profile_id',c.cost_profile_id,'cost_profile_version',c.cost_profile_version,'attempt_id',attempt.id,'attempt_no',attempt.attempt_no,'worker_id',attempt.claimed_by));
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.publish_owner_beta_paper_preview_v1(preview_id uuid,job_id uuid,publication jsonb,process_config_sha256 text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE old_actor text:=current_setting('app.actor_user_id',true); p public.paper_rebalance_previews; c public.owner_beta_paper_control; l public.owner_beta_paper_lineage; j public.jobs; attempt public.job_attempts; ver bigint; cash text; pos jsonb; preimage jsonb; now_kst date:=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date; auto_result jsonb; job_claimed boolean;
BEGIN
  PERFORM public.paper_0067_principal('worker');
  IF jsonb_typeof(publication)<>'object' OR octet_length(publication::text)>262144 OR NOT public.daily_0060_keys(publication,ARRAY['schema','preview_id','lineage_id','apply_mode','control_epoch','attempt_id','attempt_no','worker_id','process_config_sha256','account_state_version','account_state_sha256','account_state_preimage','cost_profile_id','cost_profile_version','target_snapshot_sha256','signal_allocation_sha256','preview_token','result']) OR publication->>'schema'<>'owner-beta-paper-preview-v1' OR publication->>'preview_id'<>publish_owner_beta_paper_preview_v1.preview_id::text OR jsonb_typeof(publication->'control_epoch')<>'number' OR jsonb_typeof(publication->'attempt_no')<>'number' OR jsonb_typeof(publication->'account_state_version')<>'number' OR jsonb_typeof(publication->'cost_profile_version')<>'number' OR publication->>'account_state_sha256'!~'^[0-9a-f]{64}$' OR publication->>'target_snapshot_sha256'!~'^sha256:[0-9a-f]{64}$' OR publication->>'signal_allocation_sha256'!~'^sha256:[0-9a-f]{64}$' OR publication->>'preview_token'!~'^[0-9a-f]{64}$' OR publication->>'process_config_sha256' IS DISTINCT FROM process_config_sha256 OR jsonb_typeof(publication->'result')<>'object' OR NOT public.daily_0060_keys(publication->'result',ARRAY['schema','target_snapshot_sha256','signal_allocation_sha256','plan_status','no_trade_reasons','equity','cash_before','available_cash','leftover_cash','buy_notional','sell_notional','explicit_fees','informational_slippage','decisions','orders','cost_profile_id','cost_profile_version','price_observations']) THEN RAISE EXCEPTION 'beta preview publication schema invalid' USING ERRCODE='22023'; END IF;
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('PUBLISH_PREVIEW','NOT_FOUND',NULL,NULL); END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT p0.* INTO p FROM public.paper_rebalance_previews p0 WHERE p0.id=publish_owner_beta_paper_preview_v1.preview_id AND p0.input_kind='OWNER_BETA_DAILY_V1' AND p0.owner_user_id=c.owner_user_id AND p0.account_id=c.account_id;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('PUBLISH_PREVIEW','NOT_FOUND',NULL,NULL); END IF;
  c:=public.paper_0067_authority('PREVIEW',p.price_date,c.control_epoch,process_config_sha256,true);
  -- Match the terminal projection's jobs -> preview order.
  SELECT * INTO j FROM public.jobs WHERE id=publish_owner_beta_paper_preview_v1.job_id AND owner_user_id=c.owner_user_id AND job_type='owner_beta_paper_preview' AND status='RUNNING' AND locked_at>clock_timestamp()-interval '60 seconds' FOR UPDATE;
  job_claimed:=FOUND;
  SELECT * INTO p FROM public.paper_rebalance_previews WHERE id=preview_id AND input_kind='OWNER_BETA_DAILY_V1' AND owner_user_id=c.owner_user_id AND account_id=c.account_id FOR UPDATE;
  IF NOT FOUND OR p.job_id<>publish_owner_beta_paper_preview_v1.job_id THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('PUBLISH_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  IF NOT job_claimed OR p.status<>'RUNNING' THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('PUBLISH_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  SELECT * INTO attempt FROM public.job_attempts ja0 WHERE ja0.id=(publication->>'attempt_id')::uuid AND ja0.job_id=j.id AND ja0.attempt_no=j.attempt_count AND ja0.attempt_no=(publication->>'attempt_no')::integer AND ja0.outcome='RUNNING' AND ja0.claimed_by=j.locked_by AND ja0.claimed_by=publication->>'worker_id' AND ja0.started_at IS NOT NULL FOR SHARE;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('PUBLISH_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  SELECT * INTO l FROM public.owner_beta_paper_lineage WHERE id=p.beta_lineage_id AND account_id=c.account_id AND owner_user_id=c.owner_user_id FOR SHARE;
  IF NOT FOUND OR publication->>'lineage_id'<>l.id::text OR publication->>'apply_mode' IS DISTINCT FROM p.beta_apply_mode OR (publication->>'control_epoch')::bigint<>c.control_epoch OR publication->>'target_snapshot_sha256'<>l.target_snapshot_sha256 OR publication->>'signal_allocation_sha256' IS DISTINCT FROM p.signal_allocation_sha256 OR (publication->>'cost_profile_version')::integer<>1 OR publication->>'cost_profile_id'<>'KRX_ETF_DEFAULT' OR publication->'result'->>'schema'<>'owner-beta-paper-preview-result-v1' OR publication->'result'->>'target_snapshot_sha256'<>l.target_snapshot_sha256 OR publication->'result'->>'signal_allocation_sha256'<>p.signal_allocation_sha256 OR publication->'result'->>'cost_profile_id'<>'KRX_ETF_DEFAULT' OR jsonb_typeof(publication->'result'->'cost_profile_version')<>'number' OR (publication->'result'->>'cost_profile_version')::integer<>1 OR publication->'result'->>'plan_status' NOT IN ('REBALANCE','NO_TRADE') OR jsonb_typeof(publication->'result'->'no_trade_reasons')<>'array' OR jsonb_typeof(publication->'result'->'decisions')<>'array' OR jsonb_typeof(publication->'result'->'orders')<>'array' OR jsonb_typeof(publication->'result'->'price_observations')<>'array' OR jsonb_array_length(publication->'result'->'decisions')>11 OR jsonb_array_length(publication->'result'->'orders')>11 OR jsonb_array_length(publication->'result'->'price_observations')>11 OR jsonb_typeof(publication->'result'->'equity')<>'string' OR jsonb_typeof(publication->'result'->'cash_before')<>'string' OR jsonb_typeof(publication->'result'->'available_cash')<>'string' OR jsonb_typeof(publication->'result'->'leftover_cash')<>'string' OR jsonb_typeof(publication->'result'->'buy_notional')<>'string' OR jsonb_typeof(publication->'result'->'sell_notional')<>'string' OR jsonb_typeof(publication->'result'->'explicit_fees')<>'string' OR jsonb_typeof(publication->'result'->'informational_slippage')<>'string' OR EXISTS(SELECT 1 FROM jsonb_array_elements(publication->'result'->'decisions') d0 WHERE jsonb_typeof(d0)<>'object' OR NOT public.daily_0060_keys(d0,ARRAY['instrument_id','current_quantity','current_value','current_weight','target_value','target_weight','delta_value','action','skip_reason'])) OR EXISTS(SELECT 1 FROM jsonb_array_elements(publication->'result'->'orders') o0 WHERE jsonb_typeof(o0)<>'object' OR NOT public.daily_0060_keys(o0,ARRAY['instrument_id','side','quantity','raw_price','estimated_execution_price','notional','commission','tax','informational_slippage'])) OR EXISTS(SELECT 1 FROM jsonb_array_elements(publication->'result'->'price_observations') q0 WHERE jsonb_typeof(q0)<>'object' OR NOT public.daily_0060_keys(q0,ARRAY['instrument_id','session_date','observed_at'])) THEN RAISE EXCEPTION 'beta preview identity or calculated result shape mismatch' USING ERRCODE='23514'; END IF;
  IF EXISTS(SELECT 1 FROM jsonb_array_elements(publication->'result'->'no_trade_reasons') n0 WHERE jsonb_typeof(n0)<>'string')
     OR EXISTS(SELECT 1 FROM jsonb_array_elements(publication->'result'->'decisions') d0 WHERE jsonb_typeof(d0->'instrument_id')<>'string' OR jsonb_typeof(d0->'current_quantity')<>'string' OR jsonb_typeof(d0->'current_value')<>'string' OR jsonb_typeof(d0->'current_weight')<>'string' OR jsonb_typeof(d0->'target_value')<>'string' OR jsonb_typeof(d0->'target_weight')<>'string' OR jsonb_typeof(d0->'delta_value')<>'string' OR jsonb_typeof(d0->'action')<>'string' OR jsonb_typeof(d0->'skip_reason') NOT IN ('string','null'))
     OR EXISTS(SELECT 1 FROM jsonb_array_elements(publication->'result'->'orders') o0 WHERE jsonb_typeof(o0->'instrument_id')<>'string' OR jsonb_typeof(o0->'side')<>'string' OR jsonb_typeof(o0->'quantity')<>'string' OR jsonb_typeof(o0->'raw_price')<>'string' OR jsonb_typeof(o0->'estimated_execution_price')<>'string' OR jsonb_typeof(o0->'notional')<>'string' OR jsonb_typeof(o0->'commission')<>'string' OR jsonb_typeof(o0->'tax')<>'string' OR jsonb_typeof(o0->'informational_slippage')<>'string')
     OR EXISTS(SELECT 1 FROM jsonb_array_elements(publication->'result'->'price_observations') q0 WHERE jsonb_typeof(q0->'instrument_id')<>'string' OR jsonb_typeof(q0->'session_date')<>'string' OR jsonb_typeof(q0->'observed_at')<>'string' OR q0->>'session_date'<>p.price_date::text OR (q0->>'observed_at')::timestamptz>clock_timestamp())
     OR (publication->'result'->>'plan_status'='NO_TRADE' AND (jsonb_array_length(publication->'result'->'orders')<>0 OR publication->'result'->>'buy_notional'<>'0.0000' OR publication->'result'->>'sell_notional'<>'0.0000' OR publication->'result'->>'explicit_fees'<>'0.0000' OR publication->'result'->>'informational_slippage'<>'0.0000'))
     OR (publication->'result'->>'plan_status'='REBALANCE' AND jsonb_array_length(publication->'result'->'orders')=0)
  THEN RAISE EXCEPTION 'beta preview calculated result values invalid' USING ERRCODE='23514'; END IF;
  SELECT a.paper_state_version,(SELECT cl.balance::text FROM public.cash_ledger cl WHERE cl.account_id=a.id AND cl.owner_user_id=a.owner_user_id ORDER BY cl.seq DESC LIMIT 1),COALESCE((SELECT jsonb_object_agg(x.instrument_id,x.quantity::text ORDER BY x.instrument_id) FROM public.positions x WHERE x.account_id=a.id AND x.owner_user_id=a.owner_user_id AND x.quantity<>0),'{}'::jsonb)
    INTO ver,cash,pos FROM public.accounts a WHERE a.id=c.account_id AND a.owner_user_id=c.owner_user_id FOR SHARE;
  preimage:=jsonb_build_object('schema_version',1,'paper_state_version',ver,'cash',cash,'positions',pos);
  IF ver<>(publication->>'account_state_version')::bigint OR p.account_state_version<>ver OR p.account_state_sha256 IS DISTINCT FROM publication->>'account_state_sha256' OR publication->>'account_state_sha256' IS DISTINCT FROM public.paper_0067_account_state_sha256(ver,cash,pos) OR publication->'account_state_preimage' IS DISTINCT FROM preimage OR now_kst<>p.price_date OR p.proposed_effective_date<=now_kst THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('PUBLISH_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  UPDATE public.paper_rebalance_previews SET status='READY',preview_seoul_date=now_kst,cost_profile_id=c.cost_profile_id,cost_profile_version=c.cost_profile_version,account_state_version=ver,account_state_sha256=publication->>'account_state_sha256',account_state_preimage=preimage,signal_allocation_sha256=publication->>'signal_allocation_sha256',preview_token=publication->>'preview_token',result_json=publication->'result',completed_at=clock_timestamp(),updated_at=clock_timestamp() WHERE id=p.id;
  IF p.beta_apply_mode='AUTO' THEN
    auto_result:=public.paper_0067_apply_auto_preview(p.id,c.control_epoch,process_config_sha256);
    PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('PUBLISH_PREVIEW',auto_result->>'outcome',c.control_epoch,c.account_id,COALESCE(auto_result->'data','{}'::jsonb)||jsonb_build_object('preview_id',p.id,'effective_session',p.proposed_effective_date,'attempt_id',attempt.id,'attempt_no',attempt.attempt_no));
  END IF;
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('PUBLISH_PREVIEW','OK',c.control_epoch,c.account_id,jsonb_build_object('preview_id',p.id,'apply_mode','MANUAL','effective_session',p.proposed_effective_date,'preview_token',publication->>'preview_token','account_state_version',ver,'attempt_id',attempt.id,'attempt_no',attempt.attempt_no));
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.apply_owner_beta_paper_preview_v1(preview_id uuid,preview_token text,expected_epoch bigint,process_config_sha256 text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE actor uuid:=NULLIF(current_setting('app.actor_user_id',true),'')::uuid; p public.paper_rebalance_previews; c public.owner_beta_paper_control; l public.owner_beta_paper_lineage; target uuid; now_kst date:=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date; cash text; pos jsonb; ver bigint; preimage jsonb; applied jsonb;
BEGIN
  PERFORM public.paper_0067_principal('app');
  SELECT * INTO p FROM public.paper_rebalance_previews WHERE id=preview_id AND input_kind='OWNER_BETA_DAILY_V1' AND beta_apply_mode='MANUAL' AND owner_user_id=actor;
  IF NOT FOUND THEN RETURN public.paper_0067_action('APPLY_PREVIEW','NOT_FOUND',NULL,NULL); END IF;
  c:=public.paper_0067_authority('APPLY',p.price_date,expected_epoch,process_config_sha256,true);
  SELECT * INTO p FROM public.paper_rebalance_previews WHERE id=preview_id AND input_kind='OWNER_BETA_DAILY_V1' AND beta_apply_mode='MANUAL' AND owner_user_id=c.owner_user_id AND account_id=c.account_id FOR UPDATE;
  IF NOT FOUND THEN RETURN public.paper_0067_action('APPLY_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  IF actor<>c.owner_user_id OR p.preview_token IS DISTINCT FROM preview_token THEN RETURN public.paper_0067_action('APPLY_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  IF p.status='APPLIED' THEN RETURN public.paper_0067_action('APPLY_PREVIEW','REPLAY',c.control_epoch,c.account_id,jsonb_build_object('pending_target_id',p.pending_target_id,'effective_session',p.proposed_effective_date)); END IF;
  IF p.status<>'READY' OR p.preview_seoul_date<>now_kst OR p.price_date<>now_kst OR p.proposed_effective_date<=now_kst OR p.account_state_version IS NULL OR p.account_state_sha256 IS NULL THEN RETURN public.paper_0067_action('APPLY_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  SELECT * INTO l FROM public.owner_beta_paper_lineage WHERE id=p.beta_lineage_id AND owner_user_id=c.owner_user_id AND account_id=c.account_id FOR SHARE;
  IF NOT FOUND OR l.control_epoch<>c.control_epoch OR l.consent_event_id<>c.consent_event_id OR l.target_snapshot_sha256 IS DISTINCT FROM p.result_json->>'target_snapshot_sha256' THEN RAISE EXCEPTION 'beta preview lineage changed' USING ERRCODE='40001'; END IF;
  SELECT a.paper_state_version,(SELECT cl.balance::text FROM public.cash_ledger cl WHERE cl.account_id=a.id AND cl.owner_user_id=a.owner_user_id ORDER BY cl.seq DESC LIMIT 1),COALESCE((SELECT jsonb_object_agg(x.instrument_id,x.quantity::text ORDER BY x.instrument_id) FROM public.positions x WHERE x.account_id=a.id AND x.owner_user_id=a.owner_user_id AND x.quantity<>0),'{}'::jsonb)
    INTO ver,cash,pos FROM public.accounts a WHERE a.id=c.account_id AND a.owner_user_id=c.owner_user_id FOR UPDATE;
  preimage:=jsonb_build_object('schema_version',1,'paper_state_version',ver,'cash',cash,'positions',pos);
  IF ver<>p.account_state_version OR public.paper_0067_account_state_sha256(ver,cash,pos) IS DISTINCT FROM p.account_state_sha256 OR p.account_state_preimage IS DISTINCT FROM preimage THEN RETURN public.paper_0067_action('APPLY_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  applied:=public.paper_0067_apply_target(l.id,'MANUAL',ver,p.account_state_sha256,preimage,p.signal_allocation_sha256);
  IF applied->>'outcome'='CONFLICT' THEN RETURN public.paper_0067_action('APPLY_PREVIEW','CONFLICT',c.control_epoch,c.account_id); END IF;
  IF applied->>'outcome'='WAITING_EARLIER_SESSION' THEN RETURN public.paper_0067_action('APPLY_PREVIEW','WAITING_EARLIER_SESSION',c.control_epoch,c.account_id); END IF;
  IF applied->>'outcome'='REPLAY' AND applied->>'processing_origin'<>'MANUAL' THEN RETURN public.paper_0067_action('APPLY_PREVIEW','CONFLICT',c.control_epoch,c.account_id); END IF;
  target:=(applied->>'pending_target_id')::uuid;
  UPDATE public.paper_rebalance_previews SET status='APPLIED',pending_target_id=target,applied_at=clock_timestamp(),updated_at=clock_timestamp() WHERE id=p.id AND status='READY';
  RETURN public.paper_0067_action('APPLY_PREVIEW','OK',c.control_epoch,c.account_id,jsonb_build_object('pending_target_id',target,'effective_session',l.effective_session,'lineage_id',l.id));
END $$;

CREATE FUNCTION public.queue_owner_beta_paper_target_v1(run_id uuid,expected_epoch bigint,process_config_sha256 text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE c public.owner_beta_paper_control; r public.owner_beta_recommendation_runs; res public.owner_beta_daily_reservations; sr public.owner_beta_paper_source_runs; sl public.owner_beta_paper_source_slots; sa public.owner_beta_paper_source_admissions; adm public.owner_beta_daily_admissions; binding public.account_strategy_bindings; e record; lineage uuid; preview uuid; job uuid; request_key text; arr jsonb; weight numeric; alloc_hash text; account_ver bigint; cash text; positions jsonb; source_readback jsonb; prior public.owner_beta_paper_lineage; prior_preview public.paper_rebalance_previews; old_actor text:=current_setting('app.actor_user_id',true); today date:=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date;
BEGIN
  PERFORM public.paper_0067_principal('worker');
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('QUEUE_TARGET','NOT_FOUND',NULL,NULL); END IF;
  c:=public.paper_0067_authority('TARGET',today,expected_epoch,process_config_sha256,true);
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT * INTO binding FROM public.account_strategy_bindings WHERE account_id=c.account_id AND owner_user_id=c.owner_user_id AND strategy_config_id=c.strategy_config_id AND unbound_at IS NULL FOR SHARE;
  IF NOT FOUND OR NOT binding.auto_apply_recommendations THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('QUEUE_TARGET','CONSENT_REQUIRED',c.control_epoch,c.account_id); END IF;
  SELECT * INTO STRICT r FROM public.owner_beta_recommendation_runs WHERE id=queue_owner_beta_paper_target_v1.run_id AND owner_user_id=c.owner_user_id FOR SHARE;
  IF r.status<>'SUCCEEDED' OR r.trigger_kind<>'DAILY' OR r.as_of<>today OR r.strategy_config_id<>c.strategy_config_id OR r.daily_pins IS NULL OR r.factor_snapshot_sha256 IS NULL OR r.target_snapshot_sha256 IS NULL OR r.cash_weight IS NULL THEN RAISE EXCEPTION 'automatic target run is not current and terminal' USING ERRCODE='23514'; END IF;
  SELECT * INTO STRICT res FROM public.owner_beta_daily_reservations dr0 WHERE dr0.run_id=r.id AND dr0.owner_user_id=c.owner_user_id FOR SHARE;
  SELECT * INTO STRICT sr FROM public.owner_beta_paper_source_runs sr0 WHERE sr0.run_id=r.id FOR SHARE;
  SELECT * INTO STRICT sa FROM public.owner_beta_paper_source_admissions WHERE slot_id=sr.slot_id FOR SHARE;
  SELECT * INTO STRICT adm FROM public.owner_beta_daily_admissions WHERE admission_sha256=r.daily_pins->>'admission_sha256' AND owner_user_id=c.owner_user_id FOR SHARE;
  SELECT * INTO STRICT sl FROM public.owner_beta_paper_source_slots WHERE slot_id=sr.slot_id FOR SHARE;
  source_readback:=public.paper_0066_read(sl);
  IF sa.admission_sha256<>adm.admission_sha256 OR res.owner_user_id<>c.owner_user_id OR res.strategy_config_id<>c.strategy_config_id OR res.session_date<>r.as_of OR res.run_id<>r.id OR res.job_id<>r.job_id
     OR res.admission_sha256<>adm.admission_sha256 OR sr.job_id<>r.job_id OR sl.owner_user_id<>c.owner_user_id OR sl.session_date<>r.as_of
     OR sl.policy_sha256<>(c.daily_cohort->>'policy_sha256') OR sl.activation_epoch<>(c.daily_cohort->>'activation_epoch')::bigint OR sl.cohort IS DISTINCT FROM c.daily_cohort
     OR source_readback->>'candidate_run_id'<>r.id::text OR source_readback->>'terminal'<>'SUCCEEDED' OR source_readback->>'candidate_admission_sha256'<>adm.admission_sha256
     OR source_readback->>'job_id'<>sr.job_id::text OR source_readback->>'attempt_id'<>sr.attempt_id::text
     OR r.daily_pins IS DISTINCT FROM jsonb_build_object('admission_sha256',adm.admission_sha256,'content_sha256',adm.content_sha256,'manifest_sha256',adm.manifest_sha256,'policy_sha256',adm.policy_sha256)
     OR r.daily_config_epoch<>res.config_epoch OR r.daily_activation_epoch<>res.activation_epoch
     OR res.config_epoch<>(c.daily_cohort->>'config_epoch')::bigint OR res.activation_epoch<>(c.daily_cohort->>'activation_epoch')::bigint
     OR res.strategy_config_sha256<>c.daily_cohort->>'strategy_config_sha256'
     OR NOT public.daily_0060_admission_usable((SELECT d FROM public.owner_beta_daily_control d WHERE d.id='etf11'),adm.admission_sha256,false) THEN RAISE EXCEPTION 'automatic target source admission is unavailable' USING ERRCODE='42501'; END IF;
  IF (SELECT count(*) FROM public.owner_beta_recommendation_items i WHERE i.recommendation_run_id=r.id AND i.owner_user_id=c.owner_user_id)<>11
     OR EXISTS(SELECT 1 FROM public.owner_beta_recommendation_items i WHERE i.recommendation_run_id=r.id AND i.owner_user_id=c.owner_user_id AND i.instrument_id NOT IN ('069500.KRX','102110.KRX','114260.KRX','132030.KRX','133690.KRX','143850.KRX','148070.KRX','153130.KRX','192090.KRX','195930.KRX','229200.KRX'))
     OR EXISTS(SELECT 1 FROM public.owner_beta_recommendation_items i WHERE i.recommendation_run_id=r.id AND i.owner_user_id=c.owner_user_id AND COALESCE(i.target_weight,0)<>0 AND i.instrument_id<>'069500.KRX')
     OR EXISTS(SELECT 1 FROM public.owner_beta_recommendation_items i WHERE i.recommendation_run_id=r.id AND i.owner_user_id=c.owner_user_id AND (i.target_weight IS NULL OR (i.excluded AND i.target_weight<>0))) OR r.cash_weight NOT IN (0,1) THEN RAISE EXCEPTION 'automatic allocation outside adopted ETF11 signal' USING ERRCODE='23514'; END IF;
  SELECT COALESCE(max(i.target_weight),0) INTO weight FROM public.owner_beta_recommendation_items i WHERE i.recommendation_run_id=r.id AND i.owner_user_id=c.owner_user_id AND i.instrument_id='069500.KRX' AND NOT i.excluded;
  IF weight NOT IN (0,1) OR (weight=0 AND r.cash_weight<>1) OR (weight=1 AND r.cash_weight<>0) THEN RAISE EXCEPTION 'automatic allocation violates fixed strategy' USING ERRCODE='23514'; END IF;
  SELECT * INTO e FROM public.paper_0067_next_session(c.owner_user_id,(SELECT policy_sha256 FROM public.owner_beta_daily_control WHERE id='etf11'),r.as_of);
  IF e.effective_session IS NULL THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('QUEUE_TARGET','WAITING_CALENDAR',c.control_epoch,c.account_id); END IF;
  SELECT COALESCE(jsonb_agg(jsonb_build_object('instrument_id','069500.KRX','weight',to_char(weight,'FM0.000000')) ORDER BY '069500.KRX'),'[]'::jsonb) INTO arr WHERE weight>0;
  IF arr IS NULL THEN arr:='[]'::jsonb; END IF;
  alloc_hash:=public.daily_0060_digest(jsonb_build_object('schema','owner-beta-paper-signal-allocation-v1','instrument_id','069500.KRX','weight',to_char(weight,'FM0.000000'),'cash_weight',to_char(1-weight,'FM0.000000'))::text);
  SELECT * INTO prior FROM public.owner_beta_paper_lineage WHERE account_id=c.account_id AND recommendation_run_id=r.id AND effective_session=e.effective_session AND control_epoch=c.control_epoch AND consent_event_id=c.consent_event_id FOR UPDATE;
  IF FOUND THEN
    IF prior.target_snapshot_sha256<>r.target_snapshot_sha256 OR prior.factor_snapshot_sha256<>r.factor_snapshot_sha256 OR prior.target_weights_json<>arr OR prior.signal_admission_sha256<>adm.admission_sha256 OR prior.calendar_evidence_sha256<>e.calendar_evidence_sha256 THEN RAISE EXCEPTION 'automatic target lineage conflicts with manual preview' USING ERRCODE='23505'; END IF;
    lineage:=prior.id;
  ELSE
    lineage:=gen_random_uuid();
    INSERT INTO public.owner_beta_paper_lineage(id,owner_user_id,account_id,strategy_config_id,recommendation_run_id,signal_date,effective_session,signal_admission_sha256,factor_snapshot_sha256,target_snapshot_sha256,target_weights_json,config_epoch,daily_activation_epoch,control_epoch,consent_event_id,activation_event_id,cohort_sha256,calendar_evidence_sha256,calendar_link_json)
      VALUES(lineage,c.owner_user_id,c.account_id,c.strategy_config_id,r.id,r.as_of,e.effective_session,adm.admission_sha256,r.factor_snapshot_sha256,r.target_snapshot_sha256,arr,res.config_epoch,res.activation_epoch,c.control_epoch,c.consent_event_id,c.activation_event_id,c.cohort_sha256,e.calendar_evidence_sha256,e.calendar_link_json);
  END IF;
  SELECT * INTO prior_preview FROM public.paper_rebalance_previews WHERE beta_lineage_id=lineage AND beta_apply_mode='AUTO' FOR UPDATE;
  IF FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('QUEUE_TARGET','REPLAY',c.control_epoch,c.account_id,jsonb_build_object('preview_id',prior_preview.id,'job_id',prior_preview.job_id,'lineage_id',lineage,'apply_mode','AUTO','preview_status',prior_preview.status,'effective_session',e.effective_session)); END IF;
  SELECT a.paper_state_version,(SELECT cl.balance::text FROM public.cash_ledger cl WHERE cl.account_id=a.id AND cl.owner_user_id=a.owner_user_id ORDER BY cl.seq DESC LIMIT 1),COALESCE((SELECT jsonb_object_agg(x.instrument_id,x.quantity::text ORDER BY x.instrument_id) FROM public.positions x WHERE x.account_id=a.id AND x.owner_user_id=a.owner_user_id AND x.quantity<>0),'{}'::jsonb)
    INTO account_ver,cash,positions FROM public.accounts a WHERE a.id=c.account_id AND a.owner_user_id=c.owner_user_id FOR UPDATE;
  IF cash IS NULL THEN RAISE EXCEPTION 'Paper cash ledger unavailable' USING ERRCODE='23514'; END IF;
  preview:=gen_random_uuid(); job:=gen_random_uuid(); request_key:='auto:'||r.id::text||':'||c.control_epoch::text;
  INSERT INTO public.jobs(id,owner_user_id,job_type,status,priority,idempotency_key,payload_json)
    VALUES(job,c.owner_user_id,'owner_beta_paper_preview','QUEUED',10,'owner-beta-paper-preview:'||c.account_id::text||':'||request_key,jsonb_build_object('schema','owner-beta-paper-preview-job-v1','preview_id',preview,'input_kind','OWNER_BETA_DAILY_V1','apply_mode','AUTO'));
  INSERT INTO public.paper_rebalance_previews(id,owner_user_id,account_id,strategy_config_id,job_id,status,price_basis,price_date,preview_seoul_date,proposed_effective_date,dataset_manifest_sha256,target_portfolio_sha256,input_kind,beta_apply_mode,beta_lineage_id,beta_request_key,account_state_version,signal_allocation_sha256)
    VALUES(preview,c.owner_user_id,c.account_id,c.strategy_config_id,job,'PENDING','RECOMMENDATION_CLOSE',r.as_of,today,e.effective_session,NULL,substring(r.target_snapshot_sha256 from 8),'OWNER_BETA_DAILY_V1','AUTO',lineage,NULL,account_ver,alloc_hash);
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('QUEUE_TARGET','QUEUED',c.control_epoch,c.account_id,jsonb_build_object('preview_id',preview,'job_id',job,'lineage_id',lineage,'apply_mode','AUTO','effective_session',e.effective_session,'signal_allocation_sha256',alloc_hash,'account_state_version',account_ver,'account_state_preimage',jsonb_build_object('schema_version',1,'paper_state_version',account_ver,'cash',cash,'positions',positions)));
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.scan_owner_beta_paper_targets_v1(limit_count integer,process_config_sha256 text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE c public.owner_beta_paper_control; today date:=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date; rows jsonb; auto_previews jsonb; old_actor text:=current_setting('app.actor_user_id',true);
BEGIN
  PERFORM public.paper_0067_principal('worker');
  IF limit_count NOT BETWEEN 1 AND 100 THEN RAISE EXCEPTION 'scan limit outside bound' USING ERRCODE='22023'; END IF;
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('SCAN_TARGETS','NOT_FOUND',NULL,NULL); END IF;
  c:=public.paper_0067_authority('SCAN',today,c.control_epoch,process_config_sha256,false);
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT COALESCE(jsonb_agg(q.row_json ORDER BY q.effective_session),'[]'::jsonb) INTO rows FROM (
    SELECT s.effective_session,jsonb_build_object('pending_target_id',s.pending_target_id,'effective_session',s.effective_session,'state',s.state,'processing_origin',s.processing_origin,'reason_code',s.reason_code,'lineage_id',l.id,'signal_date',l.signal_date,'signal_admission_sha256',l.signal_admission_sha256,'target_snapshot_sha256',l.target_snapshot_sha256,'execution_admission_sha256',s.execution_admission_sha256,'settlement_sha256',s.settlement_sha256,'valuation_sha256',s.valuation_sha256) AS row_json
    FROM public.owner_beta_paper_sessions s JOIN public.pending_targets t ON t.id=s.pending_target_id JOIN public.owner_beta_paper_lineage l ON l.id=t.beta_lineage_id
    WHERE s.account_id=c.account_id AND s.effective_session<=today
      AND (s.state IN ('PENDING','WAITING_PRICE','WAITING_ADMISSION','HELD','BLOCKED','RECOVERY_REQUIRED') OR s.state IN ('SETTLED_TRADE','SETTLED_NO_TRADE'))
      AND NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions earlier WHERE earlier.account_id=s.account_id AND earlier.effective_session<s.effective_session AND earlier.state<>'VALUED')
    ORDER BY s.effective_session LIMIT limit_count
  ) q;
  SELECT COALESCE(jsonb_agg(q.row_json ORDER BY q.effective_session),'[]'::jsonb) INTO auto_previews FROM (
    SELECT p.proposed_effective_date AS effective_session,jsonb_build_object('preview_id',p.id,'job_id',p.job_id,'lineage_id',p.beta_lineage_id,'apply_mode',p.beta_apply_mode,'preview_status',p.status,'signal_date',p.price_date,'effective_session',p.proposed_effective_date,'signal_allocation_sha256',p.signal_allocation_sha256,'target_snapshot_sha256',l.target_snapshot_sha256,'blocked_by_earlier_session',EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions s0 WHERE s0.account_id=c.account_id AND s0.effective_session<p.proposed_effective_date AND s0.state<>'VALUED')) AS row_json
    FROM public.paper_rebalance_previews p JOIN public.owner_beta_paper_lineage l ON l.id=p.beta_lineage_id
    WHERE p.owner_user_id=c.owner_user_id AND p.account_id=c.account_id AND p.input_kind='OWNER_BETA_DAILY_V1' AND p.beta_apply_mode='AUTO' AND p.status='READY'
    ORDER BY p.proposed_effective_date LIMIT limit_count
  ) q;
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('SCAN_TARGETS','OK',c.control_epoch,c.account_id,jsonb_build_object('today',today,'targets',rows,'auto_previews',auto_previews,'state',c.state));
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.snapshot_owner_beta_paper_target_v1(target_id uuid,expected_epoch bigint,process_config_sha256 text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE old_actor text:=current_setting('app.actor_user_id',true); c public.owner_beta_paper_control; t public.pending_targets; s public.owner_beta_paper_sessions; l public.owner_beta_paper_lineage; r public.owner_beta_recommendation_runs; ver bigint; cash text; pos jsonb; preimage jsonb; state_hash text; previous_target jsonb; today date:=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date; e_admission text; d public.owner_beta_daily_control;
BEGIN
  PERFORM public.paper_0067_principal('worker');
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('SNAPSHOT_TARGET','NOT_FOUND',NULL,NULL); END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT t0.* INTO t FROM public.pending_targets t0 WHERE t0.id=snapshot_owner_beta_paper_target_v1.target_id AND t0.beta_lineage_id IS NOT NULL AND t0.owner_user_id=c.owner_user_id AND t0.account_id=c.account_id;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('SNAPSHOT_TARGET','NOT_FOUND',c.control_epoch,c.account_id); END IF;
  c:=public.paper_0067_authority('TARGET',t.effective_date,expected_epoch,process_config_sha256,true);
  SELECT * INTO t FROM public.pending_targets WHERE id=target_id AND beta_lineage_id IS NOT NULL AND account_id=c.account_id AND owner_user_id=c.owner_user_id FOR UPDATE;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('SNAPSHOT_TARGET','NOT_FOUND',c.control_epoch,c.account_id); END IF;
  SELECT * INTO s FROM public.owner_beta_paper_sessions WHERE pending_target_id=t.id FOR UPDATE;
  IF NOT FOUND THEN RAISE EXCEPTION 'beta target has no session projection' USING ERRCODE='23514'; END IF;
  SELECT * INTO l FROM public.owner_beta_paper_lineage WHERE id=t.beta_lineage_id AND account_id=c.account_id AND owner_user_id=c.owner_user_id FOR SHARE;
  IF NOT FOUND OR l.control_epoch<>c.control_epoch OR l.consent_event_id<>c.consent_event_id THEN RAISE EXCEPTION 'target lineage stale' USING ERRCODE='40001'; END IF;
  SELECT * INTO r FROM public.owner_beta_recommendation_runs WHERE id=l.recommendation_run_id AND owner_user_id=c.owner_user_id AND status='SUCCEEDED' FOR SHARE;
  IF NOT FOUND OR r.as_of<>l.signal_date OR r.daily_pins->>'admission_sha256'<>l.signal_admission_sha256 OR r.factor_snapshot_sha256<>l.factor_snapshot_sha256 OR r.target_snapshot_sha256<>l.target_snapshot_sha256 THEN RAISE EXCEPTION 'target T recommendation pins changed' USING ERRCODE='40001'; END IF;
  SELECT a.paper_state_version,(SELECT cl.balance::text FROM public.cash_ledger cl WHERE cl.account_id=a.id AND cl.owner_user_id=a.owner_user_id ORDER BY cl.seq DESC LIMIT 1),COALESCE((SELECT jsonb_object_agg(x.instrument_id,x.quantity::text ORDER BY x.instrument_id) FROM public.positions x WHERE x.account_id=a.id AND x.owner_user_id=a.owner_user_id AND x.quantity<>0),'{}'::jsonb)
   INTO ver,cash,pos FROM public.accounts a WHERE a.id=c.account_id AND a.owner_user_id=c.owner_user_id FOR UPDATE;
  preimage:=jsonb_build_object('schema_version',1,'paper_state_version',ver,'cash',cash,'positions',pos);
  state_hash:=public.paper_0067_account_state_sha256(ver,cash,pos);
  SELECT prior_lineage.target_weights_json INTO previous_target
  FROM public.owner_beta_paper_sessions prior_session
  JOIN public.pending_targets prior_target ON prior_target.id=prior_session.pending_target_id
  JOIN public.owner_beta_paper_lineage prior_lineage ON prior_lineage.id=prior_target.beta_lineage_id
  WHERE prior_session.account_id=c.account_id
    AND prior_session.effective_session<l.effective_session
  ORDER BY prior_session.effective_session DESC
  LIMIT 1;
  PERFORM public.daily_0060_source_lock(c.owner_user_id,false);
  PERFORM public.daily_0060_lock_admissions(c.owner_user_id);
  SELECT * INTO d FROM public.owner_beta_daily_control WHERE id='etf11' AND owner_user_id=c.owner_user_id FOR SHARE;
  SELECT admission_sha256 INTO e_admission FROM public.owner_beta_daily_admissions WHERE owner_user_id=c.owner_user_id AND range_end=l.effective_session;
  IF e_admission IS NOT NULL AND NOT public.daily_0060_admission_usable(d,e_admission,false) THEN e_admission:=NULL; END IF;
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('SNAPSHOT_TARGET',CASE WHEN l.effective_session>today THEN 'WAITING_SESSION' WHEN e_admission IS NULL THEN 'WAITING_ADMISSION' ELSE 'OK' END,c.control_epoch,c.account_id,
   jsonb_build_object('pending_target_id',t.id,'lineage_id',l.id,'recommendation_run_id',l.recommendation_run_id,'owner_user_id',l.owner_user_id,'account_id',l.account_id,'strategy_config_id',l.strategy_config_id,'config_epoch',l.config_epoch,'daily_activation_epoch',l.daily_activation_epoch,'control_epoch',l.control_epoch,'daily_pins',r.daily_pins,'current_cohort',c.daily_cohort,'cohort_sha256',l.cohort_sha256,'consent_event_id',l.consent_event_id,'activation_event_id',l.activation_event_id,'process_binding_sha256',c.process_binding_sha256,'account_state_version',ver,'account_state_sha256',state_hash,'initial_account_state_sha256',s.account_state_sha256,'account_state_preimage',preimage,'target_weights',l.target_weights_json,'previous_target_weights',previous_target,'signal_date',l.signal_date,'effective_session',l.effective_session,'signal_admission_sha256',l.signal_admission_sha256,'execution_admission_sha256',e_admission,'factor_snapshot_sha256',l.factor_snapshot_sha256,'target_snapshot_sha256',l.target_snapshot_sha256,'calendar_evidence_sha256',l.calendar_evidence_sha256,'calendar_link',l.calendar_link_json,'target_status',t.status,'session_state',s.state,'settlement_sha256',s.settlement_sha256,'ledger_after_sha256',s.ledger_after_sha256,'ledger_after_version',s.ledger_after_version,'valuation_sha256',s.valuation_sha256,'receipt_json',s.receipt_json));
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.record_owner_beta_paper_wait_v1(target_id uuid,expected_epoch bigint,reason_code text,evidence_sha256 text,process_config_sha256 text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE old_actor text:=current_setting('app.actor_user_id',true); c public.owner_beta_paper_control; t public.pending_targets; s public.owner_beta_paper_sessions; v_state text; seq bigint; payload jsonb;
BEGIN
  PERFORM public.paper_0067_principal('worker');
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('RECORD_WAIT','NOT_FOUND',NULL,NULL); END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT t0.* INTO t FROM public.pending_targets t0 WHERE t0.id=record_owner_beta_paper_wait_v1.target_id AND t0.beta_lineage_id IS NOT NULL AND t0.owner_user_id=c.owner_user_id AND t0.account_id=c.account_id;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('RECORD_WAIT','NOT_FOUND',c.control_epoch,c.account_id); END IF;
  c:=public.paper_0067_authority('WAIT',t.effective_date,expected_epoch,process_config_sha256,false);
  SELECT * INTO t FROM public.pending_targets WHERE id=target_id AND beta_lineage_id IS NOT NULL AND account_id=c.account_id AND owner_user_id=c.owner_user_id FOR UPDATE;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('RECORD_WAIT','NOT_FOUND',c.control_epoch,c.account_id); END IF;
  SELECT * INTO s FROM public.owner_beta_paper_sessions WHERE pending_target_id=t.id FOR UPDATE;
  IF NOT FOUND OR s.account_id<>c.account_id OR s.settlement_sha256 IS NOT NULL OR evidence_sha256 !~ '^sha256:[0-9a-f]{64}$' OR reason_code NOT IN ('WAITING_PRICE','WAITING_ADMISSION','HELD','BLOCKED','RECOVERY_REQUIRED') THEN RAISE EXCEPTION 'wait evidence invalid or target terminal' USING ERRCODE='23514'; END IF;
  v_state:=reason_code;
  payload:=jsonb_build_object('schema','owner-beta-paper-wait-v1','reason_code',reason_code,'evidence_sha256',evidence_sha256,'effective_session',s.effective_session,'observed_at',clock_timestamp());
  SELECT COALESCE(max(event_seq),0)+1 INTO seq FROM public.owner_beta_paper_session_events WHERE pending_target_id=t.id;
  UPDATE public.owner_beta_paper_sessions SET state=v_state,reason_code=record_owner_beta_paper_wait_v1.reason_code,updated_at=clock_timestamp() WHERE pending_target_id=t.id;
  INSERT INTO public.owner_beta_paper_session_events(pending_target_id,event_seq,kind,reason_code,payload) VALUES(t.id,seq,CASE WHEN reason_code LIKE 'WAITING_%' THEN 'WAITING' ELSE reason_code END,reason_code,payload);
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('RECORD_WAIT','OK',c.control_epoch,c.account_id,jsonb_build_object('pending_target_id',t.id,'state',v_state,'event_seq',seq));
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.commit_owner_beta_paper_settlement_v1(target_id uuid,expected_epoch bigint,settlement jsonb,process_config_sha256 text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE c public.owner_beta_paper_control; t public.pending_targets; s public.owner_beta_paper_sessions; l public.owner_beta_paper_lineage; daily public.owner_beta_daily_control;
  today date; now_at timestamptz; now_epoch bigint; cash numeric(18,4); cash_start numeric(18,4); cash_after numeric(18,4); qty numeric(18,4); old_qty numeric(18,4); new_qty numeric(18,4); raw_open numeric(18,4); fill_price numeric(18,4); commission numeric(18,4); tax numeric(18,4); slip numeric(18,4); expected_cash numeric(18,4); seq bigint; ord jsonb; row_item jsonb; idx bigint; inst text; side text; order_ref text; fill_ref text; order_id uuid; fill_id uuid; ids jsonb:='[]'::jsonb; vpos jsonb; supplied_pos jsonb:='{}'::jsonb; pos_after jsonb; preimage jsonb; after_hash text; observed jsonb; original_at timestamptz; admitted_at timestamptz; observed_usable_at timestamptz; outcome text; reason text; event_seq bigint; outbox_id uuid; recovery boolean:=false; old_actor text:=current_setting('app.actor_user_id',true);
BEGIN
  PERFORM public.paper_0067_principal('worker');
  IF jsonb_typeof(settlement)<>'object' OR octet_length(settlement::text)>262144 OR NOT public.daily_0060_keys(settlement,ARRAY['schema','target_id','lineage_id','control_epoch','execution_admission_sha256','execution_input_sha256','account_state_version','account_state_sha256','ledger_before_sha256','outcome','reason_code','observed_input','orders','positions_after','cash_after','calculation_sha256']) OR settlement->>'schema'<>'owner-beta-paper-settlement-v1' OR jsonb_typeof(settlement->'target_id')<>'string' OR settlement->>'target_id'<>commit_owner_beta_paper_settlement_v1.target_id::text OR jsonb_typeof(settlement->'lineage_id')<>'string' OR jsonb_typeof(settlement->'control_epoch')<>'number' OR settlement->>'control_epoch' !~ '^(0|[1-9][0-9]{0,18})$' OR jsonb_typeof(settlement->'account_state_version')<>'number' OR settlement->>'account_state_version' !~ '^(0|[1-9][0-9]{0,18})$' OR jsonb_typeof(settlement->'account_state_sha256')<>'string' OR jsonb_typeof(settlement->'ledger_before_sha256')<>'string' OR jsonb_typeof(settlement->'execution_admission_sha256')<>'string' OR jsonb_typeof(settlement->'execution_input_sha256')<>'string' OR jsonb_typeof(settlement->'calculation_sha256')<>'string' OR jsonb_typeof(settlement->'cash_after')<>'string' THEN RAISE EXCEPTION 'settlement wire shape invalid' USING ERRCODE='22023'; END IF;
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('COMMIT_SETTLEMENT','NOT_FOUND',NULL,NULL); END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT * INTO t FROM public.pending_targets WHERE id=target_id AND account_id=c.account_id AND owner_user_id=c.owner_user_id AND beta_lineage_id IS NOT NULL;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('COMMIT_SETTLEMENT','NOT_FOUND',c.control_epoch,c.account_id); END IF;
  SELECT * INTO s FROM public.owner_beta_paper_sessions WHERE pending_target_id=t.id;
  IF NOT FOUND THEN RAISE EXCEPTION 'beta target has no session projection' USING ERRCODE='23514'; END IF;
  IF s.settlement_sha256 IS NOT NULL THEN
    IF s.receipt_json->'settlement' IS NOT DISTINCT FROM settlement THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('COMMIT_SETTLEMENT','REPLAY',c.control_epoch,c.account_id,jsonb_build_object('pending_target_id',t.id,'settlement_sha256',s.settlement_sha256,'processed_at',s.settled_at,'outbox_id',s.receipt_json->'outbox_id')); END IF;
    RAISE EXCEPTION 'settlement identity conflicts with committed receipt' USING ERRCODE='23505';
  END IF;
  c:=public.paper_0067_authority('SETTLE',s.effective_session,expected_epoch,process_config_sha256,true);
  today:=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date; now_at:=clock_timestamp();
  SELECT * INTO t FROM public.pending_targets WHERE id=target_id AND account_id=c.account_id AND owner_user_id=c.owner_user_id AND beta_lineage_id IS NOT NULL FOR UPDATE;
  SELECT * INTO s FROM public.owner_beta_paper_sessions WHERE pending_target_id=t.id FOR UPDATE;
  IF c.account_id<>s.account_id OR t.status<>'PENDING' OR s.state NOT IN ('PENDING','WAITING_PRICE','WAITING_ADMISSION','HELD','RECOVERY_REQUIRED') THEN RAISE EXCEPTION 'target is not eligible for settlement' USING ERRCODE='40001'; END IF;
  SELECT * INTO l FROM public.owner_beta_paper_lineage WHERE id=t.beta_lineage_id AND account_id=c.account_id AND owner_user_id=c.owner_user_id FOR SHARE;
  IF NOT FOUND OR settlement->>'lineage_id'<>l.id::text OR (settlement->>'control_epoch')::bigint<>c.control_epoch OR settlement->>'execution_admission_sha256' !~ '^sha256:[0-9a-f]{64}$' OR settlement->>'execution_input_sha256' !~ '^sha256:[0-9a-f]{64}$' OR settlement->>'ledger_before_sha256' !~ '^sha256:[0-9a-f]{64}$' OR settlement->>'calculation_sha256' !~ '^sha256:[0-9a-f]{64}$' OR settlement->>'account_state_sha256' !~ '^[0-9a-f]{64}$' OR (settlement->>'account_state_version')::bigint<>s.account_state_version OR settlement->>'account_state_sha256'<>s.account_state_sha256 OR settlement->>'ledger_before_sha256'<>'sha256:'||s.account_state_sha256 OR l.signal_date>=l.effective_session THEN RAISE EXCEPTION 'settlement lineage or account snapshot mismatch' USING ERRCODE='23514'; END IF;
  PERFORM public.daily_0060_source_lock(c.owner_user_id,false);
  SELECT * INTO daily FROM public.owner_beta_daily_control WHERE id='etf11' AND owner_user_id=c.owner_user_id FOR SHARE;
  IF NOT public.daily_0060_admission_usable(daily,l.signal_admission_sha256,false) OR NOT public.daily_0060_admission_usable(daily,settlement->>'execution_admission_sha256',false) THEN RAISE EXCEPTION 'settlement Daily admission unavailable' USING ERRCODE='42501'; END IF;
  IF NOT EXISTS(SELECT 1 FROM public.owner_beta_daily_admissions e JOIN public.owner_beta_daily_admissions t0 ON t0.admission_sha256=l.signal_admission_sha256 WHERE e.admission_sha256=settlement->>'execution_admission_sha256' AND e.owner_user_id=c.owner_user_id AND e.range_end=s.effective_session AND e.policy_sha256=t0.policy_sha256 AND e.range_start=t0.range_start AND e.range_end>=t0.range_end AND (WITH RECURSIVE ancestry(admission_sha256,parent_admission_sha256) AS (SELECT admission_sha256,parent_admission_sha256 FROM public.owner_beta_daily_admissions WHERE admission_sha256=e.admission_sha256 UNION ALL SELECT p.admission_sha256,p.parent_admission_sha256 FROM public.owner_beta_daily_admissions p JOIN ancestry a ON p.admission_sha256=a.parent_admission_sha256) SELECT bool_or(admission_sha256=l.signal_admission_sha256) FROM ancestry)) THEN RAISE EXCEPTION 'execution admission is not an exact descendant of signal admission' USING ERRCODE='23514'; END IF;
  IF NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_source_slots sl JOIN public.owner_beta_paper_source_admissions sa USING(slot_id) WHERE sl.owner_user_id=c.owner_user_id AND sl.session_date=s.effective_session AND sa.admission_sha256=settlement->>'execution_admission_sha256') THEN RAISE EXCEPTION 'execution source admission link unavailable' USING ERRCODE='23514'; END IF;
  IF EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions earlier WHERE earlier.account_id=c.account_id AND earlier.effective_session<s.effective_session AND earlier.state<>'VALUED') THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('COMMIT_SETTLEMENT','WAITING_EARLIER_SESSION',c.control_epoch,c.account_id,jsonb_build_object('effective_session',s.effective_session)); END IF;
  SELECT a.paper_state_version,(SELECT cl.balance FROM public.cash_ledger cl WHERE cl.account_id=a.id AND cl.owner_user_id=a.owner_user_id ORDER BY cl.seq DESC LIMIT 1),COALESCE((SELECT jsonb_object_agg(x.instrument_id,x.quantity::text ORDER BY x.instrument_id) FROM public.positions x WHERE x.account_id=a.id AND x.owner_user_id=a.owner_user_id AND x.quantity<>0),'{}'::jsonb)
    INTO now_epoch,cash,vpos FROM public.accounts a WHERE a.id=c.account_id AND a.owner_user_id=c.owner_user_id FOR UPDATE;
  preimage:=jsonb_build_object('schema_version',1,'paper_state_version',now_epoch,'cash',cash::text,'positions',vpos);
  IF now_epoch<>s.account_state_version OR preimage IS DISTINCT FROM s.receipt_json->'account_state_preimage' THEN RAISE EXCEPTION 'Paper account changed after target application' USING ERRCODE='40001'; END IF;
  IF s.effective_session<>today THEN
    recovery:=EXISTS(SELECT 1 FROM public.owner_beta_paper_events e WHERE e.account_id=c.account_id AND e.kind='RECOVERY_AUTHORIZED' AND e.payload->>'pending_target_id'=t.id::text AND e.payload->>'effective_session'=s.effective_session::text);
    IF NOT recovery OR s.effective_session>today THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('COMMIT_SETTLEMENT','WAITING_SESSION',c.control_epoch,c.account_id,jsonb_build_object('effective_session',s.effective_session,'today',today)); END IF;
  END IF;
  observed:=settlement->'observed_input';
  IF NOT public.daily_0060_keys(observed,ARRAY['original_retrieved_at','admitted_at','usable_at','precision']) OR jsonb_typeof(observed->'original_retrieved_at')<>'string' OR jsonb_typeof(observed->'admitted_at')<>'string' OR jsonb_typeof(observed->'usable_at')<>'string' OR jsonb_typeof(observed->'precision')<>'string' OR observed->>'precision'<>'APPLICATION_OBSERVED' THEN RAISE EXCEPTION 'settlement observation contract invalid' USING ERRCODE='23514'; END IF;
  original_at:=(observed->>'original_retrieved_at')::timestamptz; admitted_at:=(observed->>'admitted_at')::timestamptz; observed_usable_at:=(observed->>'usable_at')::timestamptz;
  IF observed_usable_at<original_at OR observed_usable_at<admitted_at OR original_at>now_at OR admitted_at>now_at OR observed_usable_at>now_at THEN RAISE EXCEPTION 'settlement observations are not ordered' USING ERRCODE='23514'; END IF;
  outcome:=settlement->>'outcome'; reason:=settlement->>'reason_code';
  IF outcome NOT IN ('TRADE','NO_TRADE') OR reason IS NULL OR reason !~ '^[A-Z][A-Z0-9_]{0,63}$' OR jsonb_typeof(settlement->'reason_code')<>'string' OR jsonb_typeof(settlement->'orders')<>'array' OR jsonb_array_length(settlement->'orders')>1 OR jsonb_typeof(settlement->'positions_after')<>'array' OR jsonb_array_length(settlement->'positions_after')>11 OR jsonb_typeof(settlement->'cash_after')<>'string' OR settlement->>'cash_after' !~ '^(0|[1-9][0-9]{0,13})\.[0-9]{4}$' OR EXISTS(SELECT 1 FROM jsonb_array_elements(settlement->'orders') o WHERE jsonb_typeof(o->'order_ref')<>'string' OR jsonb_typeof(o->'fill_ref')<>'string' OR jsonb_typeof(o->'instrument_id')<>'string' OR jsonb_typeof(o->'side')<>'string' OR jsonb_typeof(o->'quantity')<>'string' OR jsonb_typeof(o->'raw_open')<>'string' OR jsonb_typeof(o->'fill_price')<>'string' OR jsonb_typeof(o->'commission')<>'string' OR jsonb_typeof(o->'tax')<>'string' OR jsonb_typeof(o->'slippage')<>'string' OR jsonb_typeof(o->'cash_before')<>'string' OR jsonb_typeof(o->'cash_after')<>'string') OR EXISTS(SELECT 1 FROM jsonb_array_elements(settlement->'positions_after') p WHERE jsonb_typeof(p->'instrument_id')<>'string' OR jsonb_typeof(p->'quantity')<>'string') THEN RAISE EXCEPTION 'settlement values invalid' USING ERRCODE='23514'; END IF;
  cash_start:=cash; cash_after:=cash; seq:=(SELECT COALESCE(max(cl.seq),0) FROM public.cash_ledger cl WHERE cl.account_id=c.account_id AND cl.owner_user_id=c.owner_user_id); vpos:=COALESCE(vpos,'{}'::jsonb); old_qty:=COALESCE((vpos->>'069500.KRX')::numeric,0);
  IF outcome='NO_TRADE' AND jsonb_array_length(settlement->'orders')<>0 OR outcome='TRADE' AND jsonb_array_length(settlement->'orders')=0 THEN RAISE EXCEPTION 'trade/no-trade row shape mismatch' USING ERRCODE='23514'; END IF;
  FOR ord,idx IN SELECT value,ordinality FROM jsonb_array_elements(settlement->'orders') WITH ORDINALITY ORDER BY ordinality LOOP
    IF NOT public.daily_0060_keys(ord,ARRAY['order_ref','fill_ref','instrument_id','side','quantity','raw_open','fill_price','commission','tax','slippage','cash_before','cash_after']) OR ord->>'instrument_id'<>'069500.KRX' OR ord->>'side' NOT IN ('BUY','SELL') OR ord->>'quantity' !~ '^[1-9][0-9]{0,13}$' OR (ord->>'quantity')::numeric>99999999999999 OR ord->>'raw_open' !~ '^[1-9][0-9]{0,13}\.[0-9]{4}$' OR ord->>'fill_price' !~ '^[1-9][0-9]{0,13}\.[0-9]{4}$' OR ord->>'commission' !~ '^(0|[1-9][0-9]{0,13})\.[0-9]{4}$' OR ord->>'tax' !~ '^(0|[1-9][0-9]{0,13})\.[0-9]{4}$' OR ord->>'slippage' !~ '^(0|[1-9][0-9]{0,13})\.[0-9]{4}$' OR ord->>'cash_before' !~ '^(0|[1-9][0-9]{0,13})\.[0-9]{4}$' OR ord->>'cash_after' !~ '^(0|[1-9][0-9]{0,13})\.[0-9]{4}$' OR ord->>'order_ref' !~ '^[0-9a-f]{8}-[0-9a-f]{4}-5[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$' OR ord->>'fill_ref' !~ '^[0-9a-f]{8}-[0-9a-f]{4}-5[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$' OR ord->>'order_ref'='00000000-0000-0000-0000-000000000000' OR ord->>'fill_ref'='00000000-0000-0000-0000-000000000000' OR ord->>'order_ref'=ord->>'fill_ref' THEN RAISE EXCEPTION 'settlement order row invalid' USING ERRCODE='23514'; END IF;
    inst:=ord->>'instrument_id'; side:=ord->>'side'; qty:=(ord->>'quantity')::numeric; raw_open:=(ord->>'raw_open')::numeric; fill_price:=(ord->>'fill_price')::numeric; commission:=(ord->>'commission')::numeric; tax:=(ord->>'tax')::numeric; slip:=(ord->>'slippage')::numeric;
    order_ref:=ord->>'order_ref'; fill_ref:=ord->>'fill_ref';
    IF (side='SELL' AND tax<>0) OR ord->>'cash_before'<>to_char(cash_after,'FM99999999999990.0000') THEN RAISE EXCEPTION 'deterministic order/cash chain mismatch' USING ERRCODE='23514'; END IF;
    expected_cash:=CASE WHEN side='BUY' THEN cash_after-(qty*fill_price)-commission-tax ELSE cash_after+(qty*fill_price)-commission-tax END;
    IF expected_cash<0 OR expected_cash>99999999999999.9999 OR ord->>'cash_after'<>to_char(expected_cash,'FM99999999999990.0000') THEN RAISE EXCEPTION 'cash movement does not match fill economics' USING ERRCODE='23514'; END IF;
    old_qty:=COALESCE((vpos->>inst)::numeric,0); new_qty:=CASE WHEN side='BUY' THEN old_qty+qty ELSE old_qty-qty END; IF new_qty<0 OR new_qty<>trunc(new_qty) THEN RAISE EXCEPTION 'settlement position would be short or fractional' USING ERRCODE='23514'; END IF;
    vpos:=jsonb_set(vpos,ARRAY[inst],to_jsonb(new_qty::text),true); cash_after:=expected_cash;
    order_id:=gen_random_uuid(); fill_id:=gen_random_uuid();
    INSERT INTO public.orders(id,account_id,owner_user_id,order_ref,instrument_id,side,quantity,price,status,submitted_at,created_at,updated_at)
      VALUES(order_id,c.account_id,c.owner_user_id,order_ref,inst,side,qty,raw_open,'FILLED',now_at,now_at,now_at);
    INSERT INTO public.fills(id,account_id,owner_user_id,order_id,instrument_id,fill_ref,side,quantity,price,fees,ts,created_at)
      VALUES(fill_id,c.account_id,c.owner_user_id,order_id,inst,fill_ref,side,qty,fill_price,commission+tax,now_at,now_at);
    seq:=seq+1; INSERT INTO public.cash_ledger(account_id,owner_user_id,seq,event_type,amount,balance,currency,reference_id,ts,created_at)
      VALUES(c.account_id,c.owner_user_id,seq,side,(expected_cash-(ord->>'cash_before')::numeric),expected_cash,'KRW',fill_id,now_at,now_at);
    ids:=ids||jsonb_build_array(jsonb_build_object('order_id',order_id,'fill_id',fill_id,'order_ref',order_ref,'fill_ref',fill_ref));
  END LOOP;
  IF cash_after<>(settlement->>'cash_after')::numeric OR (outcome='NO_TRADE' AND cash_after<>cash_start) THEN RAISE EXCEPTION 'settlement final cash mismatch' USING ERRCODE='23514'; END IF;
  FOR row_item IN SELECT value FROM jsonb_array_elements(settlement->'positions_after') LOOP
    inst:=row_item->>'instrument_id';
    IF NOT public.daily_0060_keys(row_item,ARRAY['instrument_id','quantity']) OR inst<>'069500.KRX' OR row_item->>'quantity' !~ '^(0|[1-9][0-9]{0,13})$' OR supplied_pos ? inst THEN RAISE EXCEPTION 'settlement position row invalid or duplicate' USING ERRCODE='23514'; END IF;
    supplied_pos:=supplied_pos||jsonb_build_object(inst,(row_item->>'quantity')||'.0000');
  END LOOP;
  IF supplied_pos IS DISTINCT FROM vpos OR (outcome='NO_TRADE' AND vpos IS DISTINCT FROM COALESCE(s.receipt_json->'account_state_preimage'->'positions','{}'::jsonb)) THEN RAISE EXCEPTION 'settlement positions do not equal the fill plan' USING ERRCODE='23514'; END IF;
  IF outcome='TRADE' THEN
    FOR row_item IN SELECT jsonb_build_object('instrument_id',key,'quantity',value) FROM jsonb_each_text(supplied_pos) LOOP
      inst:=row_item->>'instrument_id'; qty:=(row_item->>'quantity')::numeric;
      INSERT INTO public.positions(account_id,owner_user_id,instrument_id,quantity,updated_at)
       VALUES(c.account_id,c.owner_user_id,inst,qty,now_at)
       ON CONFLICT(account_id,instrument_id) DO UPDATE SET quantity=EXCLUDED.quantity,updated_at=now_at;
    END LOOP;
  END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT a.paper_state_version,(SELECT cl.balance::text FROM public.cash_ledger cl WHERE cl.account_id=a.id AND cl.owner_user_id=a.owner_user_id ORDER BY cl.seq DESC LIMIT 1),COALESCE((SELECT jsonb_object_agg(x.instrument_id,x.quantity::text ORDER BY x.instrument_id) FROM public.positions x WHERE x.account_id=a.id AND x.owner_user_id=a.owner_user_id AND x.quantity<>0),'{}'::jsonb)
    INTO now_epoch,cash,pos_after FROM public.accounts a WHERE a.id=c.account_id AND a.owner_user_id=c.owner_user_id FOR UPDATE;
  preimage:=jsonb_build_object('schema_version',1,'paper_state_version',now_epoch,'cash',cash::text,'positions',pos_after);
  after_hash:='sha256:'||public.paper_0067_account_state_sha256(now_epoch,cash::text,pos_after);
  UPDATE public.pending_targets SET status=CASE outcome WHEN 'TRADE' THEN 'EXECUTED' ELSE 'SKIPPED' END,executed_at=now_at,
    non_execution_reason=CASE WHEN outcome='NO_TRADE' THEN jsonb_build_object('code',reason,'message','Owner-beta Paper decision produced no orders') ELSE NULL END
    WHERE id=t.id AND status='PENDING';
  IF NOT FOUND THEN RAISE EXCEPTION 'target terminal transition lost' USING ERRCODE='40001'; END IF;
  SELECT COALESCE(max(e.event_seq),0)+1 INTO event_seq FROM public.owner_beta_paper_session_events e WHERE e.pending_target_id=t.id;
  outbox_id:=public.paper_0067_generic_enqueue_outbox(t.id,'INFO','job','Owner-beta Paper session settled','Paper session result is available',jsonb_build_object('schema','owner-beta-paper-notice-v1','outcome',outcome,'reason_code',reason,'effective_session',s.effective_session));
  UPDATE public.owner_beta_paper_sessions SET state=CASE outcome WHEN 'TRADE' THEN 'SETTLED_TRADE' ELSE 'SETTLED_NO_TRADE' END,settlement_kind=outcome,reason_code=reason,
    execution_admission_sha256=settlement->>'execution_admission_sha256',execution_input_sha256=settlement->>'execution_input_sha256',ledger_before_sha256=settlement->>'ledger_before_sha256',ledger_after_sha256=after_hash,
    ledger_after_version=now_epoch,ledger_after_preimage=preimage,settlement_sha256=settlement->>'calculation_sha256',source_original_retrieved_at=original_at,input_admitted_at=admitted_at,usable_at=observed_usable_at,settled_at=now_at,
    receipt_json=jsonb_build_object('schema','owner-beta-paper-settlement-receipt-v1','settlement',settlement,'order_fill_ids',ids,'ledger_after_preimage',preimage,'ledger_after_sha256',after_hash,'ledger_after_version',now_epoch,'processed_at',now_at,'outbox_id',outbox_id),updated_at=now_at WHERE pending_target_id=t.id;
  INSERT INTO public.owner_beta_paper_session_events(pending_target_id,event_seq,kind,reason_code,payload,occurred_at) VALUES(t.id,event_seq,CASE outcome WHEN 'TRADE' THEN 'SETTLED_TRADE' ELSE 'SETTLED_NO_TRADE' END,reason,jsonb_build_object('schema','owner-beta-paper-settlement-event-v1','settlement_sha256',settlement->>'calculation_sha256','ledger_after_sha256',after_hash,'outbox_id',outbox_id),now_at);
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('COMMIT_SETTLEMENT','OK',c.control_epoch,c.account_id,jsonb_build_object('pending_target_id',t.id,'settlement_sha256',settlement->>'calculation_sha256','ledger_after_sha256',after_hash,'ledger_after_version',now_epoch,'processed_at',now_at,'outbox_id',outbox_id,'order_fill_ids',ids));
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.commit_owner_beta_paper_valuation_v1(target_id uuid,expected_epoch bigint,valuation jsonb,process_config_sha256 text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE c public.owner_beta_paper_control; t public.pending_targets; s public.owner_beta_paper_sessions; l public.owner_beta_paper_lineage; d public.owner_beta_daily_control; today date; now_at timestamptz; ver bigint; cash numeric(18,4); positions jsonb; actual_preimage jsonb; closes jsonb; row_item jsonb; inst text; qty numeric(18,4); close_price numeric(18,4); value numeric(18,4):=0; expected_cash numeric(18,4); expected_position_value numeric(18,4); expected_equity numeric(18,4); original_at timestamptz; admitted_at timestamptz; observed_usable_at timestamptz; observed jsonb; event_seq bigint; inserted public.daily_equity; old_actor text:=current_setting('app.actor_user_id',true);
BEGIN
  PERFORM public.paper_0067_principal('worker');
  IF jsonb_typeof(valuation)<>'object' OR octet_length(valuation::text)>262144 OR NOT public.daily_0060_keys(valuation,ARRAY['schema','target_id','control_epoch','valuation_admission_sha256','valuation_input_sha256','settlement_sha256','ledger_after_sha256','account_state_version','account_state_sha256','closes','equity','cash','positions_value','currency','observed_input','calculation_sha256']) OR valuation->>'schema'<>'owner-beta-paper-valuation-v1' OR jsonb_typeof(valuation->'target_id')<>'string' OR valuation->>'target_id'<>commit_owner_beta_paper_valuation_v1.target_id::text OR jsonb_typeof(valuation->'control_epoch')<>'number' OR valuation->>'control_epoch' !~ '^(0|[1-9][0-9]{0,18})$' OR jsonb_typeof(valuation->'account_state_version')<>'number' OR valuation->>'account_state_version' !~ '^(0|[1-9][0-9]{0,18})$' OR jsonb_typeof(valuation->'account_state_sha256')<>'string' OR jsonb_typeof(valuation->'valuation_admission_sha256')<>'string' OR jsonb_typeof(valuation->'valuation_input_sha256')<>'string' OR jsonb_typeof(valuation->'settlement_sha256')<>'string' OR jsonb_typeof(valuation->'ledger_after_sha256')<>'string' OR jsonb_typeof(valuation->'calculation_sha256')<>'string' THEN RAISE EXCEPTION 'valuation wire shape invalid' USING ERRCODE='22023'; END IF;
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('COMMIT_VALUATION','NOT_FOUND',NULL,NULL); END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT * INTO t FROM public.pending_targets WHERE id=target_id AND account_id=c.account_id AND owner_user_id=c.owner_user_id AND beta_lineage_id IS NOT NULL;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('COMMIT_VALUATION','NOT_FOUND',c.control_epoch,c.account_id); END IF;
  SELECT * INTO s FROM public.owner_beta_paper_sessions WHERE pending_target_id=t.id;
  IF NOT FOUND OR s.settlement_sha256 IS NULL THEN RAISE EXCEPTION 'valuation requires a settled beta session' USING ERRCODE='55000'; END IF;
  IF s.valuation_sha256 IS NOT NULL THEN
    IF s.receipt_json->'valuation' IS NOT DISTINCT FROM valuation THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('COMMIT_VALUATION','REPLAY',c.control_epoch,c.account_id,jsonb_build_object('pending_target_id',t.id,'valuation_sha256',s.valuation_sha256,'valued_at',s.valued_at)); END IF;
    RAISE EXCEPTION 'valuation identity conflicts with committed receipt' USING ERRCODE='23505';
  END IF;
  c:=public.paper_0067_authority('VALUE',s.effective_session,expected_epoch,process_config_sha256,true);
  PERFORM pg_advisory_xact_lock(hashtextextended(c.account_id::text,381901));
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT * INTO t FROM public.pending_targets WHERE id=target_id AND account_id=c.account_id AND owner_user_id=c.owner_user_id AND beta_lineage_id IS NOT NULL FOR UPDATE;
  SELECT * INTO s FROM public.owner_beta_paper_sessions WHERE pending_target_id=t.id FOR UPDATE;
  SELECT * INTO l FROM public.owner_beta_paper_lineage WHERE id=t.beta_lineage_id AND account_id=c.account_id AND owner_user_id=c.owner_user_id FOR SHARE;
  today:=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date; now_at:=clock_timestamp();
  IF s.state NOT IN ('SETTLED_TRADE','SETTLED_NO_TRADE') OR l.id IS NULL OR (valuation->>'control_epoch')::bigint<>c.control_epoch OR valuation->>'settlement_sha256'<>s.settlement_sha256 OR valuation->>'ledger_after_sha256'<>s.ledger_after_sha256 OR valuation->>'valuation_admission_sha256' !~ '^sha256:[0-9a-f]{64}$' OR valuation->>'valuation_input_sha256' !~ '^sha256:[0-9a-f]{64}$' OR valuation->>'calculation_sha256' !~ '^sha256:[0-9a-f]{64}$' OR valuation->>'account_state_sha256' !~ '^[0-9a-f]{64}$' THEN RAISE EXCEPTION 'valuation lineage or fingerprint mismatch' USING ERRCODE='23514'; END IF;
  IF s.effective_session<>today AND NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_events e WHERE e.account_id=c.account_id AND e.kind='RECOVERY_AUTHORIZED' AND e.payload->>'pending_target_id'=t.id::text AND e.payload->>'effective_session'=s.effective_session::text) THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('COMMIT_VALUATION','WAITING_SESSION',c.control_epoch,c.account_id,jsonb_build_object('effective_session',s.effective_session,'today',today)); END IF;
  IF (valuation->>'account_state_version')::bigint<>s.ledger_after_version OR jsonb_typeof(valuation->'closes')<>'array' OR jsonb_array_length(valuation->'closes')>11 OR jsonb_typeof(valuation->'currency')<>'string' OR valuation->>'currency'<>'KRW' OR jsonb_typeof(valuation->'cash')<>'string' OR valuation->>'cash' !~ '^(0|[1-9][0-9]{0,13})\.[0-9]{4}$' OR jsonb_typeof(valuation->'positions_value')<>'string' OR valuation->>'positions_value' !~ '^(0|[1-9][0-9]{0,13})\.[0-9]{4}$' OR jsonb_typeof(valuation->'equity')<>'string' OR valuation->>'equity' !~ '^(0|[1-9][0-9]{0,13})\.[0-9]{4}$' OR EXISTS(SELECT 1 FROM jsonb_array_elements(valuation->'closes') x WHERE jsonb_typeof(x->'instrument_id')<>'string' OR jsonb_typeof(x->'raw_close')<>'string') THEN RAISE EXCEPTION 'valuation amounts or close list invalid' USING ERRCODE='23514'; END IF;
  SELECT a.paper_state_version,(SELECT cl.balance FROM public.cash_ledger cl WHERE cl.account_id=a.id AND cl.owner_user_id=a.owner_user_id ORDER BY cl.seq DESC LIMIT 1),COALESCE((SELECT jsonb_object_agg(p.instrument_id,p.quantity::text ORDER BY p.instrument_id) FROM public.positions p WHERE p.account_id=a.id AND p.owner_user_id=a.owner_user_id AND p.quantity<>0),'{}'::jsonb)
    INTO ver,cash,positions FROM public.accounts a WHERE a.id=c.account_id AND a.owner_user_id=c.owner_user_id FOR UPDATE;
  actual_preimage:=jsonb_build_object('schema_version',1,'paper_state_version',ver,'cash',cash::text,'positions',positions);
  IF ver<>s.ledger_after_version OR actual_preimage IS DISTINCT FROM s.ledger_after_preimage OR public.paper_0067_account_state_sha256(ver,cash::text,positions) IS DISTINCT FROM valuation->>'account_state_sha256' OR 'sha256:'||public.paper_0067_account_state_sha256(ver,cash::text,positions) IS DISTINCT FROM s.ledger_after_sha256 OR ('sha256:'||s.account_state_sha256<>s.ledger_before_sha256 AND s.state='SETTLED_NO_TRADE') THEN RAISE EXCEPTION 'Paper account changed after settlement' USING ERRCODE='40001'; END IF;
  expected_cash:=(valuation->>'cash')::numeric; expected_position_value:=(valuation->>'positions_value')::numeric; expected_equity:=(valuation->>'equity')::numeric;
  IF expected_cash<>cash THEN RAISE EXCEPTION 'valuation cash does not match ledger' USING ERRCODE='23514'; END IF;
  closes:=valuation->'closes';
  IF jsonb_array_length(closes)<>(SELECT count(*) FROM pg_catalog.jsonb_object_keys(positions)) THEN RAISE EXCEPTION 'valuation requires one close for each held position' USING ERRCODE='23514'; END IF;
  FOR row_item IN SELECT item.value FROM jsonb_array_elements(closes) item LOOP
    IF NOT public.daily_0060_keys(row_item,ARRAY['instrument_id','raw_close']) OR row_item->>'instrument_id'<>'069500.KRX' OR row_item->>'raw_close' !~ '^[1-9][0-9]{0,13}\.[0-9]{4}$' THEN RAISE EXCEPTION 'valuation close row invalid' USING ERRCODE='23514'; END IF;
    inst:=row_item->>'instrument_id'; qty:=(positions->>inst)::numeric; close_price:=(row_item->>'raw_close')::numeric; value:=value+(qty*close_price);
  END LOOP;
  IF value<>expected_position_value OR cash+value<>expected_equity THEN RAISE EXCEPTION 'valuation totals do not equal close marks' USING ERRCODE='23514'; END IF;
  observed:=valuation->'observed_input';
  IF NOT public.daily_0060_keys(observed,ARRAY['original_retrieved_at','admitted_at','usable_at','precision']) OR jsonb_typeof(observed->'original_retrieved_at')<>'string' OR jsonb_typeof(observed->'admitted_at')<>'string' OR jsonb_typeof(observed->'usable_at')<>'string' OR jsonb_typeof(observed->'precision')<>'string' OR observed->>'precision'<>'APPLICATION_OBSERVED' THEN RAISE EXCEPTION 'valuation observation contract invalid' USING ERRCODE='23514'; END IF;
  original_at:=(observed->>'original_retrieved_at')::timestamptz; admitted_at:=(observed->>'admitted_at')::timestamptz; observed_usable_at:=(observed->>'usable_at')::timestamptz;
  IF observed_usable_at<original_at OR observed_usable_at<admitted_at OR original_at>now_at OR admitted_at>now_at OR observed_usable_at>now_at THEN RAISE EXCEPTION 'valuation observation order invalid' USING ERRCODE='23514'; END IF;
  PERFORM public.daily_0060_source_lock(c.owner_user_id,false);
  SELECT * INTO d FROM public.owner_beta_daily_control WHERE id='etf11' AND owner_user_id=c.owner_user_id FOR SHARE;
  IF NOT public.daily_0060_admission_usable(d,valuation->>'valuation_admission_sha256',false) OR NOT EXISTS(SELECT 1 FROM public.owner_beta_daily_admissions a WHERE a.admission_sha256=valuation->>'valuation_admission_sha256' AND a.owner_user_id=c.owner_user_id AND a.range_end=s.effective_session) THEN RAISE EXCEPTION 'valuation close admission unavailable' USING ERRCODE='42501'; END IF;
  INSERT INTO public.daily_equity(account_id,owner_user_id,trading_date,equity,cash,positions_value,currency,created_at)
    VALUES(c.account_id,c.owner_user_id,s.effective_session,expected_equity,expected_cash,expected_position_value,'KRW',now_at)
    ON CONFLICT(account_id,trading_date) DO NOTHING;
  SELECT * INTO inserted FROM public.daily_equity WHERE account_id=c.account_id AND trading_date=s.effective_session FOR UPDATE;
  IF NOT FOUND OR inserted.owner_user_id<>c.owner_user_id OR inserted.equity<>expected_equity OR inserted.cash<>expected_cash OR inserted.positions_value<>expected_position_value OR inserted.currency<>'KRW' THEN RAISE EXCEPTION 'daily equity replay conflicts with valuation' USING ERRCODE='23505'; END IF;
  UPDATE public.owner_beta_paper_sessions SET state='VALUED',valuation_admission_sha256=valuation->>'valuation_admission_sha256',valuation_input_sha256=valuation->>'valuation_input_sha256',valuation_sha256=valuation->>'calculation_sha256',valued_at=now_at,
    receipt_json=receipt_json||jsonb_build_object('valuation',valuation,'valuation_processed_at',now_at),updated_at=now_at WHERE pending_target_id=t.id;
  SELECT COALESCE(max(e.event_seq),0)+1 INTO event_seq FROM public.owner_beta_paper_session_events e WHERE e.pending_target_id=t.id;
  INSERT INTO public.owner_beta_paper_session_events(pending_target_id,event_seq,kind,payload,occurred_at) VALUES(t.id,event_seq,'VALUED',jsonb_build_object('schema','owner-beta-paper-valuation-event-v1','valuation_sha256',valuation->>'calculation_sha256','daily_equity_id',inserted.id),now_at);
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('COMMIT_VALUATION','OK',c.control_epoch,c.account_id,jsonb_build_object('pending_target_id',t.id,'valuation_sha256',valuation->>'calculation_sha256','daily_equity_id',inserted.id,'valued_at',now_at));
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

-- Rename the generic SECURITY DEFINER entrypoints and wrap them with a beta
-- account fence. Direct-table triggers alone cannot stop an older definer
-- from running as migration_owner.
ALTER FUNCTION public.lock_paper_rebalance_preview_submission(uuid,uuid,uuid,date) RENAME TO paper_0067_generic_lock_preview;
ALTER FUNCTION public.snapshot_paper_rebalance_preview(uuid,uuid,date) RENAME TO paper_0067_generic_snapshot_preview;
ALTER FUNCTION public.publish_paper_rebalance_preview(uuid,uuid,bigint,text,text,integer,date,text,jsonb,jsonb) RENAME TO paper_0067_generic_publish_preview;
ALTER FUNCTION public.fail_paper_rebalance_preview(uuid,uuid,jsonb) RENAME TO paper_0067_generic_fail_preview;
ALTER FUNCTION public.apply_paper_rebalance_preview(uuid,uuid,text,date) RENAME TO paper_0067_generic_apply_preview;
ALTER FUNCTION public.queue_scheduled_paper_targets(uuid,uuid,uuid,date,uuid,text,text,jsonb) RENAME TO paper_0067_generic_queue_targets;
ALTER FUNCTION public.preflight_paper_target(uuid,uuid) RENAME TO paper_0067_generic_preflight_target;
ALTER FUNCTION public.enqueue_paper_settlement_outbox(uuid,text,text,text,text,jsonb) RENAME TO paper_0067_generic_enqueue_outbox;
REVOKE ALL ON FUNCTION public.paper_0067_generic_lock_preview(uuid,uuid,uuid,date),public.paper_0067_generic_snapshot_preview(uuid,uuid,date),public.paper_0067_generic_publish_preview(uuid,uuid,bigint,text,text,integer,date,text,jsonb,jsonb),public.paper_0067_generic_fail_preview(uuid,uuid,jsonb),public.paper_0067_generic_apply_preview(uuid,uuid,text,date),public.paper_0067_generic_queue_targets(uuid,uuid,uuid,date,uuid,text,text,jsonb),public.paper_0067_generic_preflight_target(uuid,uuid),public.paper_0067_generic_enqueue_outbox(uuid,text,text,text,text,jsonb) FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter;

-- Generic worker fences must see beta previews even without an app actor.
-- Save/set/restore inside this private predicate so generic delegation
-- retains the caller's original actor and its existing tenant permissions.
CREATE FUNCTION public.paper_0067_worker_beta_preview(p_preview_id uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$
DECLARE old_actor text:=current_setting('app.actor_user_id',true); c public.owner_beta_paper_control; is_beta boolean;
BEGIN
  PERFORM public.paper_0067_principal('worker');
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11';
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN false; END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT EXISTS(SELECT 1 FROM public.paper_rebalance_previews p WHERE p.id=p_preview_id AND p.owner_user_id=c.owner_user_id AND p.account_id=c.account_id) INTO is_beta;
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN is_beta;
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

CREATE FUNCTION public.lock_paper_rebalance_preview_submission(p_owner_user_id uuid,p_account_id uuid,p_recommendation_run_id uuid,p_seoul_today date)
RETURNS TABLE(outcome text,target_portfolio_id uuid,strategy_config_id uuid,price_date date,dataset_version_id uuid,dataset_manifest_sha256 text,weights_json jsonb)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$ BEGIN
  IF public.paper_0067_is_beta_account(p_account_id) THEN RAISE EXCEPTION 'generic Paper preview cannot address a beta account' USING ERRCODE='42501'; END IF;
  RETURN QUERY SELECT * FROM public.paper_0067_generic_lock_preview(p_owner_user_id,p_account_id,p_recommendation_run_id,p_seoul_today);
END $$;

CREATE FUNCTION public.snapshot_paper_rebalance_preview(p_preview_id uuid,p_job_id uuid,p_seoul_today date)
RETURNS TABLE(owner_user_id uuid,account_id uuid,recommendation_run_id uuid,target_portfolio_id uuid,strategy_config_id uuid,price_date date,proposed_effective_date date,dataset_version_id uuid,dataset_id text,dataset_version text,curated_version integer,dataset_manifest_sha256 text,target_portfolio_sha256 text,cost_profile_id text,cost_profile_version integer,account_state_version bigint,cash_balance text,positions_json jsonb,weights_json jsonb)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$ BEGIN
  IF public.paper_0067_worker_beta_preview(p_preview_id) THEN RAISE EXCEPTION 'generic Paper preview cannot address a beta account' USING ERRCODE='42501'; END IF;
  RETURN QUERY SELECT * FROM public.paper_0067_generic_snapshot_preview(p_preview_id,p_job_id,p_seoul_today);
END $$;

CREATE FUNCTION public.publish_paper_rebalance_preview(p_preview_id uuid,p_job_id uuid,p_account_state_version bigint,p_account_state_sha256 text,p_cost_profile_id text,p_cost_profile_version integer,p_proposed_effective_date date,p_preview_token text,p_target_weights_json jsonb,p_result_json jsonb) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$ BEGIN
  IF public.paper_0067_worker_beta_preview(p_preview_id) THEN RAISE EXCEPTION 'generic Paper preview cannot address a beta account' USING ERRCODE='42501'; END IF;
  RETURN public.paper_0067_generic_publish_preview(p_preview_id,p_job_id,p_account_state_version,p_account_state_sha256,p_cost_profile_id,p_cost_profile_version,p_proposed_effective_date,p_preview_token,p_target_weights_json,p_result_json);
END $$;

CREATE FUNCTION public.fail_paper_rebalance_preview(p_preview_id uuid,p_job_id uuid,p_error_json jsonb) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$ BEGIN
  IF public.paper_0067_worker_beta_preview(p_preview_id) THEN RAISE EXCEPTION 'generic Paper preview cannot address a beta account' USING ERRCODE='42501'; END IF;
  RETURN public.paper_0067_generic_fail_preview(p_preview_id,p_job_id,p_error_json);
END $$;

CREATE FUNCTION public.apply_paper_rebalance_preview(p_owner_user_id uuid,p_preview_id uuid,p_preview_token text,p_seoul_today date)
RETURNS TABLE(outcome text,pending_target_id uuid,effective_date date,source_kind text)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$ BEGIN
  IF EXISTS(SELECT 1 FROM public.paper_rebalance_previews p WHERE p.id=p_preview_id AND p.owner_user_id=p_owner_user_id AND public.paper_0067_is_beta_account(p.account_id)) THEN RAISE EXCEPTION 'generic Paper apply cannot address a beta account' USING ERRCODE='42501'; END IF;
  RETURN QUERY SELECT * FROM public.paper_0067_generic_apply_preview(p_owner_user_id,p_preview_id,p_preview_token,p_seoul_today);
END $$;

CREATE FUNCTION public.queue_scheduled_paper_targets(p_recommendation_run_id uuid,p_owner_user_id uuid,p_strategy_config_id uuid,p_as_of date,p_dataset_version_id uuid,p_dataset_version text,p_dataset_manifest_sha256 text,p_targets_json jsonb)
RETURNS TABLE(targets integer,missing_next_session boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$ BEGIN
  IF EXISTS(SELECT 1 FROM public.owner_beta_paper_control c WHERE c.owner_user_id=p_owner_user_id AND c.strategy_config_id=p_strategy_config_id) THEN RAISE EXCEPTION 'generic scheduled Paper bridge cannot address a beta strategy' USING ERRCODE='42501'; END IF;
  RETURN QUERY SELECT * FROM public.paper_0067_generic_queue_targets(p_recommendation_run_id,p_owner_user_id,p_strategy_config_id,p_as_of,p_dataset_version_id,p_dataset_version,p_dataset_manifest_sha256,p_targets_json);
END $$;

CREATE FUNCTION public.preflight_paper_target(p_target_id uuid,p_owner_user_id uuid) RETURNS TABLE(authorized boolean,reason jsonb)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$ BEGIN
  IF EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions s WHERE s.pending_target_id=p_target_id AND s.owner_user_id=p_owner_user_id) THEN RAISE EXCEPTION 'generic Paper preflight cannot address a beta target' USING ERRCODE='42501'; END IF;
  RETURN QUERY SELECT * FROM public.paper_0067_generic_preflight_target(p_target_id,p_owner_user_id);
END $$;

CREATE FUNCTION public.enqueue_paper_settlement_outbox(p_pending_target_id uuid,p_severity text,p_kind text,p_title text,p_body text,p_parity_json jsonb) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$ DECLARE beta boolean; BEGIN
  SELECT EXISTS(SELECT 1 FROM public.owner_beta_paper_sessions s WHERE s.pending_target_id=p_pending_target_id) INTO beta;
  IF COALESCE(beta,false) AND current_user<>'migration_owner' THEN RAISE EXCEPTION 'beta outbox is internal to the atomic beta settlement' USING ERRCODE='42501'; END IF;
  IF COALESCE(beta,false) THEN RAISE EXCEPTION 'beta settlement must call the internal outbox function directly' USING ERRCODE='42501'; END IF;
  RETURN public.paper_0067_generic_enqueue_outbox(p_pending_target_id,p_severity,p_kind,p_title,p_body,p_parity_json);
END $$;

CREATE FUNCTION public.fail_owner_beta_paper_preview_v1(preview_id uuid,job_id uuid,error jsonb,process_config_sha256 text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE old_actor text:=current_setting('app.actor_user_id',true); p public.paper_rebalance_previews; c public.owner_beta_paper_control; j public.jobs; attempt public.job_attempts; job_claimed boolean;
BEGIN
  PERFORM public.paper_0067_principal('worker');
  SELECT * INTO c FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('FAIL_PREVIEW','NOT_FOUND',NULL,NULL); END IF;
  PERFORM set_config('app.actor_user_id',c.owner_user_id::text,true);
  SELECT p0.* INTO p FROM public.paper_rebalance_previews p0 WHERE p0.id=fail_owner_beta_paper_preview_v1.preview_id AND p0.input_kind='OWNER_BETA_DAILY_V1' AND p0.owner_user_id=c.owner_user_id AND p0.account_id=c.account_id;
  IF NOT FOUND THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('FAIL_PREVIEW','NOT_FOUND',NULL,NULL); END IF;
  c:=public.paper_0067_authority('PREVIEW',p.price_date,c.control_epoch,process_config_sha256,true);
  -- Match the terminal projection's jobs -> preview order.
  SELECT * INTO j FROM public.jobs WHERE id=fail_owner_beta_paper_preview_v1.job_id AND owner_user_id=c.owner_user_id AND job_type='owner_beta_paper_preview' AND status='RUNNING' AND locked_at>clock_timestamp()-interval '60 seconds' FOR UPDATE;
  job_claimed:=FOUND;
  SELECT * INTO p FROM public.paper_rebalance_previews WHERE id=fail_owner_beta_paper_preview_v1.preview_id AND input_kind='OWNER_BETA_DAILY_V1' AND owner_user_id=c.owner_user_id AND account_id=c.account_id FOR UPDATE;
  IF NOT FOUND OR p.job_id<>fail_owner_beta_paper_preview_v1.job_id THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('FAIL_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  IF jsonb_typeof(error)<>'object' OR octet_length(error::text)>4096 OR NOT public.daily_0060_keys(error,ARRAY['code','message','attempt_id','attempt_no','worker_id','process_config_sha256'])
     OR jsonb_typeof(error->'code')<>'string' OR error->>'code' !~ '^[A-Z][A-Z0-9_]{0,63}$' OR jsonb_typeof(error->'message')<>'string' OR length(error->>'message') NOT BETWEEN 1 AND 512
     OR jsonb_typeof(error->'attempt_id')<>'string' OR error->>'attempt_id' !~ '^[0-9a-fA-F-]{36}$' OR jsonb_typeof(error->'attempt_no')<>'number' OR error->>'attempt_no' !~ '^[1-9][0-9]{0,8}$'
     OR jsonb_typeof(error->'worker_id')<>'string' OR length(error->>'worker_id') NOT BETWEEN 1 AND 128 OR error->>'process_config_sha256' IS DISTINCT FROM process_config_sha256 THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('FAIL_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  IF NOT job_claimed THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('FAIL_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  SELECT * INTO attempt FROM public.job_attempts ja0 WHERE ja0.id=(error->>'attempt_id')::uuid AND ja0.job_id=j.id AND ja0.attempt_no=j.attempt_count AND ja0.attempt_no=(error->>'attempt_no')::integer AND ja0.outcome='RUNNING' AND ja0.claimed_by=j.locked_by AND ja0.claimed_by=error->>'worker_id' AND ja0.started_at IS NOT NULL FOR SHARE;
  IF NOT FOUND OR p.status NOT IN ('PENDING','RUNNING') THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('FAIL_PREVIEW','STALE',c.control_epoch,c.account_id); END IF;
  UPDATE public.paper_rebalance_previews SET status='FAILED',result_json=NULL,error_json=jsonb_build_object('code',error->>'code','message',error->>'message'),completed_at=clock_timestamp(),updated_at=clock_timestamp() WHERE id=p.id;
  PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RETURN public.paper_0067_action('FAIL_PREVIEW','OK',c.control_epoch,c.account_id,jsonb_build_object('preview_id',p.id,'job_id',j.id,'attempt_id',attempt.id,'attempt_no',attempt.attempt_no));
EXCEPTION WHEN OTHERS THEN PERFORM set_config('app.actor_user_id',COALESCE(old_actor,''),true); RAISE;
END $$;

-- The historical recommendation callback changes app.actor_user_id for every
-- non-recommendation job. Beta Paper uses its own definer projection and must
-- leave the caller actor untouched, as the existing price-only callback does.
CREATE OR REPLACE FUNCTION public.sync_recommendation_run_from_terminal_job()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF NEW.job_type IN ('owner_beta_price_only_simulation','owner_beta_paper_preview') THEN
        RETURN NEW;
    END IF;
    PERFORM pg_catalog.set_config('app.actor_user_id', NEW.owner_user_id::text, true);
    IF NEW.job_type = 'recommendation'
       AND NEW.status = 'FAILED'
       AND OLD.status IS DISTINCT FROM NEW.status
       AND NEW.error_code = 'attempts_exhausted' THEN
        UPDATE public.recommendation_runs
           SET status = 'FAILED',
               summary_json = '{"code":"RECOMMENDATION_ATTEMPTS_EXHAUSTED","message":"recommendation worker attempts were exhausted"}'::jsonb
         WHERE job_id = NEW.id
           AND owner_user_id = NEW.owner_user_id
           AND status = 'PENDING';
    ELSIF NEW.job_type = 'recommendation'
       AND NEW.status = 'CANCELED'
       AND OLD.status IS DISTINCT FROM NEW.status THEN
        UPDATE public.recommendation_runs
           SET status = 'FAILED',
               summary_json = '{"code":"RECOMMENDATION_CANCELED","message":"recommendation was canceled"}'::jsonb
         WHERE job_id = NEW.id
           AND owner_user_id = NEW.owner_user_id
           AND status = 'PENDING';
    END IF;
    RETURN NEW;
END;
$$;

-- Project terminal job outcomes into the narrow beta preview row. This is an
-- AFTER UPDATE callback, so the job and preview transition commit or roll back
-- together; it deliberately has no caller-facing EXECUTE grant.
CREATE FUNCTION public.paper_0067_project_terminal_preview() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp SET lock_timeout='5s' SET statement_timeout='15s' AS $$
DECLARE projection_code text; projection_message text;
BEGIN
  IF TG_TABLE_SCHEMA<>'public' OR TG_TABLE_NAME<>'jobs' OR TG_OP<>'UPDATE' THEN
    RAISE EXCEPTION 'beta preview terminal projection trigger context invalid' USING ERRCODE='42501';
  END IF;
  IF OLD.status IS NOT DISTINCT FROM NEW.status
     OR OLD.status IN ('SUCCEEDED','FAILED','CANCELED')
     OR NEW.job_type IS DISTINCT FROM 'owner_beta_paper_preview'
     OR NEW.status NOT IN ('FAILED','CANCELED')
     OR NEW.payload_json->>'schema' IS DISTINCT FROM 'owner-beta-paper-preview-job-v1'
     OR NEW.payload_json->>'input_kind' IS DISTINCT FROM 'OWNER_BETA_DAILY_V1'
     OR jsonb_typeof(NEW.payload_json->'preview_id') IS DISTINCT FROM 'string'
     OR (NEW.payload_json->>'apply_mode' IS DISTINCT FROM 'MANUAL' AND NEW.payload_json->>'apply_mode' IS DISTINCT FROM 'AUTO')
  THEN RETURN NEW; END IF;
  IF NEW.status='CANCELED' THEN
    projection_code:='OWNER_BETA_PAPER_PREVIEW_CANCELED';
    projection_message:='Owner-beta Paper preview job was canceled';
  ELSIF NEW.error_code='attempts_exhausted' THEN
    projection_code:='OWNER_BETA_PAPER_PREVIEW_ATTEMPTS_EXHAUSTED';
    projection_message:='Owner-beta Paper preview job attempts were exhausted';
  ELSE
    projection_code:='OWNER_BETA_PAPER_PREVIEW_FAILED';
    projection_message:='Owner-beta Paper preview job failed';
  END IF;
  UPDATE public.paper_rebalance_previews p
     SET status='FAILED',result_json=NULL,
         error_json=jsonb_build_object('code',projection_code,'message',projection_message),
         completed_at=clock_timestamp(),updated_at=clock_timestamp()
   WHERE p.id::text=(NEW.payload_json->>'preview_id')
     AND p.job_id=NEW.id AND p.owner_user_id=NEW.owner_user_id
     AND p.beta_apply_mode=(NEW.payload_json->>'apply_mode')
     AND p.input_kind='OWNER_BETA_DAILY_V1' AND p.status IN ('PENDING','RUNNING');
  RETURN NEW;
END $$;
CREATE TRIGGER paper_0067_project_terminal_preview
  AFTER UPDATE OF status ON public.jobs
  FOR EACH ROW EXECUTE FUNCTION public.paper_0067_project_terminal_preview();


DO $$ DECLARE f record; BEGIN
  FOR f IN SELECT * FROM (VALUES
    ('configure_owner_beta_paper_v1(uuid,text,jsonb,jsonb,text)','migration_owner'),
    ('record_owner_beta_paper_consent_v1(uuid,bigint,text,text,boolean,text)','app'),
    ('transition_owner_beta_paper_v1(bigint,text,jsonb,text)','migration_owner'),
    ('read_owner_beta_paper_state_v1()','app'),
    ('submit_owner_beta_paper_preview_v1(uuid,uuid,bigint,text,text)','app'),
    ('queue_owner_beta_paper_target_v1(uuid,bigint,text)','worker'),
    ('snapshot_owner_beta_paper_preview_v1(uuid,uuid,text)','worker'),
    ('publish_owner_beta_paper_preview_v1(uuid,uuid,jsonb,text)','worker'),
    ('fail_owner_beta_paper_preview_v1(uuid,uuid,jsonb,text)','worker'),
    ('apply_owner_beta_paper_preview_v1(uuid,text,bigint,text)','app'),
    ('apply_owner_beta_paper_preview_auto_v1(uuid,bigint,text)','worker'),
    ('scan_owner_beta_paper_targets_v1(integer,text)','worker'),
    ('snapshot_owner_beta_paper_target_v1(uuid,bigint,text)','worker'),
    ('record_owner_beta_paper_wait_v1(uuid,bigint,text,text,text)','worker'),
    ('commit_owner_beta_paper_settlement_v1(uuid,bigint,jsonb,text)','worker'),
    ('commit_owner_beta_paper_valuation_v1(uuid,bigint,jsonb,text)','worker'),
    ('lock_paper_rebalance_preview_submission(uuid,uuid,uuid,date)','app'),
    ('snapshot_paper_rebalance_preview(uuid,uuid,date)','worker'),
    ('publish_paper_rebalance_preview(uuid,uuid,bigint,text,text,integer,date,text,jsonb,jsonb)','worker'),
    ('fail_paper_rebalance_preview(uuid,uuid,jsonb)','worker'),
    ('apply_paper_rebalance_preview(uuid,uuid,text,date)','app'),
    ('queue_scheduled_paper_targets(uuid,uuid,uuid,date,uuid,text,text,jsonb)','worker'),
    ('preflight_paper_target(uuid,uuid)','worker'),
    ('enqueue_paper_settlement_outbox(uuid,text,text,text,text,jsonb)','app')
  ) AS x(sig,grantee) LOOP
    EXECUTE format('ALTER FUNCTION public.%s OWNER TO migration_owner',f.sig);
    EXECUTE format('ALTER FUNCTION public.%s SET search_path=pg_catalog,pg_temp',f.sig);
    EXECUTE format('REVOKE ALL ON FUNCTION public.%s FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter',f.sig);
    EXECUTE format('GRANT EXECUTE ON FUNCTION public.%s TO %I',f.sig,f.grantee);
  END LOOP;

  FOR f IN SELECT * FROM (VALUES
    ('paper_0067_action(text,text,bigint,uuid,jsonb)'),
    ('paper_0067_account_state_sha256(bigint,text,jsonb)'),
    ('paper_0067_principal(text)'),
    ('paper_0067_is_beta_account(uuid)'),
    ('paper_0067_guard_beta_write()'),
    ('paper_0067_immutable()'),
    ('paper_0067_authority(text,date,bigint,text,boolean)'),
    ('paper_0067_next_session(uuid,text,date)'),
    ('paper_0067_apply_target(uuid,text,bigint,text,jsonb,text)'),
    ('paper_0067_apply_auto_preview(uuid,bigint,text)'),
    ('paper_0067_project_terminal_preview()'),
    ('paper_0067_worker_beta_preview(uuid)'),
    ('paper_0067_generic_lock_preview(uuid,uuid,uuid,date)'),
    ('paper_0067_generic_snapshot_preview(uuid,uuid,date)'),
    ('paper_0067_generic_publish_preview(uuid,uuid,bigint,text,text,integer,date,text,jsonb,jsonb)'),
    ('paper_0067_generic_fail_preview(uuid,uuid,jsonb)'),
    ('paper_0067_generic_apply_preview(uuid,uuid,text,date)'),
    ('paper_0067_generic_queue_targets(uuid,uuid,uuid,date,uuid,text,text,jsonb)'),
    ('paper_0067_generic_preflight_target(uuid,uuid)'),
    ('paper_0067_generic_enqueue_outbox(uuid,text,text,text,text,jsonb)')
  ) AS x(sig) LOOP
    EXECUTE format('ALTER FUNCTION public.%s OWNER TO migration_owner',f.sig);
    EXECUTE format('ALTER FUNCTION public.%s SET search_path=pg_catalog,pg_temp',f.sig);
    EXECUTE format('REVOKE ALL ON FUNCTION public.%s FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter',f.sig);
  END LOOP;
END $$;

-- The invoker trigger calls this definer lookup while protecting existing
-- ledger tables. It reveals only whether a supplied account id is beta-bound.
GRANT EXECUTE ON FUNCTION public.paper_0067_is_beta_account(uuid) TO app,worker,admin,audit_writer,research_writer,daily_input_admitter;
