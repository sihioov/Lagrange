-- 0010 down: drop every policy, disable RLS, revoke the admin grants and the
-- users SELECT grant added in 0010 up. Object ownership stays with
-- migration_owner; REVOKE only strips privileges (0009 down pattern).

DO $rls$
DECLARE
  t text;
BEGIN
  FOREACH t IN ARRAY ARRAY[
      'user_strategy_configs','recommendation_runs','recommendation_items',
      'target_portfolios','jobs','backtest_runs','backtest_metrics',
      'backtest_warnings','result_artifacts','accounts','cash_ledger',
      'positions','orders','fills','daily_equity','broker_connections',
      'reconciliation_runs','risk_events','notifications',
      'web_sessions','invitations'
  ] LOOP
    EXECUTE format('DROP POLICY IF EXISTS tenant_all_app_%s ON %I', t, t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_all_owner_%s ON %I', t, t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_all_worker_%s ON %I', t, t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_select_admin_%s ON %I', t, t);
    EXECUTE format('ALTER TABLE %I DISABLE ROW LEVEL SECURITY', t);
  END LOOP;

  FOREACH t IN ARRAY ARRAY[
      'users','roles','user_roles','strategies','strategy_versions',
      'strategy_parameter_schemas','strategy_promotions','instruments',
      'instrument_aliases','trading_calendars','data_batches',
      'dataset_versions','data_quality_issues','corporate_actions',
      'universe_snapshots','factor_definitions','factor_snapshot_manifests',
      'data_entitlements','system_flags'
  ] LOOP
    EXECUTE format('DROP POLICY IF EXISTS shared_select_%s ON %I', t, t);
    EXECUTE format('ALTER TABLE %I DISABLE ROW LEVEL SECURITY', t);
  END LOOP;
END
$rls$;

DROP POLICY IF EXISTS audit_insert_audit_writer ON audit_logs;
DROP POLICY IF EXISTS audit_select_app ON audit_logs;
DROP POLICY IF EXISTS audit_select_admin ON audit_logs;
ALTER TABLE audit_logs DISABLE ROW LEVEL SECURITY;

REVOKE ALL PRIVILEGES ON TABLE
    user_strategy_configs, recommendation_runs, recommendation_items,
    target_portfolios, jobs, backtest_runs, backtest_metrics,
    backtest_warnings, result_artifacts, accounts, cash_ledger, positions,
    orders, fills, daily_equity, broker_connections, reconciliation_runs,
    risk_events, notifications, web_sessions, invitations,
    users, roles, user_roles, strategies, strategy_versions,
    strategy_parameter_schemas, strategy_promotions, instruments,
    instrument_aliases, trading_calendars, data_batches, dataset_versions,
    data_quality_issues, corporate_actions, universe_snapshots,
    factor_definitions, factor_snapshot_manifests, data_entitlements,
    system_flags, audit_logs
    FROM admin;
REVOKE SELECT ON TABLE users FROM app, worker;
