-- 0010: Row-Level Security (plan Todo 23; design §7.3 multi-tenancy, §14.3
-- data protection; requirements FR-AUTH-003, SC-01, NFR-SEC-007).
--
-- Privilege model (extends 0009; roles are created by the harness bootstrap):
--   migration_owner : table owner, FORCE RLS, NO policies of its own on the
--                     serving surface - a connection as the table owner
--                     WITHOUT an explicit actor context sees zero tenant rows
--                     and is denied writes (42501). With an actor context
--                     (app.actor_user_id GUC) it behaves like that user only
--                     (the migration-contract suite depends on this).
--   app             : serving role; every statement runs under an actor GUC.
--                     SELECT/UPDATE/DELETE are row-local (owner = actor);
--                     INSERT's WITH CHECK requires the row's owner to equal
--                     the actor (crafted owner ids and GUC-less writes are
--                     denied with 42501, fail-closed everywhere).
--   worker          : trusted engine role; full-row access on the tenant
--                     tables it serves (claims, manifests) - it is never a
--                     per-user role.
--   admin           : dedicated read-only admin role (no BYPASSRLS): SELECT
--                     on every tenant/shared/audit table via USING (true)
--                     policies, gated at the repository by the Owner role and
--                     audited. No DML policies exist for it.
--   audit_writer    : the only INSERTer of audit_logs; no UPDATE/DELETE/
--                     TRUNCATE anywhere (append-only, NFR-SEC-007).
--
-- The actor GUC is `app.actor_user_id` (uuid, set via SET LOCAL by the
-- repository layer; unset => NULL => reads invisible, writes denied).
-- ---------------------------------------------------------------------------
-- Tenant tables: RLS enabled AND FORCED with row-local policies.
-- Ownership column mapping (schema 0001-0008; web_sessions/invitations use
-- user_id, everything else owner_user_id).
-- ---------------------------------------------------------------------------

DO $rls$
DECLARE
  t text;
  c text;
BEGIN
  FOREACH t IN ARRAY ARRAY[
      'user_strategy_configs','recommendation_runs','recommendation_items',
      'target_portfolios','jobs','backtest_runs','backtest_metrics',
      'backtest_warnings','result_artifacts','accounts','cash_ledger',
      'positions','orders','fills','daily_equity','broker_connections',
      'reconciliation_runs','risk_events','notifications'
  ] LOOP
    c := 'owner_user_id';
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format(
      'CREATE POLICY tenant_all_app_%s ON %I FOR ALL TO app '
      || 'USING (%I = current_setting(''app.actor_user_id'', true)::uuid) '
      || 'WITH CHECK (%I = current_setting(''app.actor_user_id'', true)::uuid)',
      t, t, c, c);
    EXECUTE format(
      'CREATE POLICY tenant_all_owner_%s ON %I FOR ALL TO migration_owner '
      || 'USING (%I = current_setting(''app.actor_user_id'', true)::uuid) '
      || 'WITH CHECK (%I = current_setting(''app.actor_user_id'', true)::uuid)',
      t, t, c, c);
    EXECUTE format(
      'CREATE POLICY tenant_all_worker_%s ON %I FOR ALL TO worker '
      || 'USING (true) WITH CHECK (true)',
      t, t);
    EXECUTE format(
      'CREATE POLICY tenant_select_admin_%s ON %I FOR SELECT TO admin '
      || 'USING (true)',
      t, t);
  END LOOP;

  FOREACH t IN ARRAY ARRAY['web_sessions', 'invitations'] LOOP
    c := 'user_id';
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format(
      'CREATE POLICY tenant_all_app_%s ON %I FOR ALL TO app '
      || 'USING (%I = current_setting(''app.actor_user_id'', true)::uuid) '
      || 'WITH CHECK (%I = current_setting(''app.actor_user_id'', true)::uuid)',
      t, t, c, c);
    EXECUTE format(
      'CREATE POLICY tenant_all_owner_%s ON %I FOR ALL TO migration_owner '
      || 'USING (%I = current_setting(''app.actor_user_id'', true)::uuid) '
      || 'WITH CHECK (%I = current_setting(''app.actor_user_id'', true)::uuid)',
      t, t, c, c);
    EXECUTE format(
      'CREATE POLICY tenant_all_worker_%s ON %I FOR ALL TO worker '
      || 'USING (true) WITH CHECK (true)',
      t, t);
    EXECUTE format(
      'CREATE POLICY tenant_select_admin_%s ON %I FOR SELECT TO admin '
      || 'USING (true)',
      t, t);
  END LOOP;
END
$rls$;

-- ---------------------------------------------------------------------------
-- Shared system-owned tables: RLS enabled (not forced - the data pipelines
-- write them as migration_owner), read-only SELECT policies for the serving
-- roles and admin; no DML policies exist, so mutations fail with 42501.
-- users completes the 0001/0009 documented intent (read-only to serving
-- roles; the 0009 grant list omitted it).
-- ---------------------------------------------------------------------------

DO $rls$
DECLARE
  t text;
BEGIN
  FOREACH t IN ARRAY ARRAY[
      'users','roles','user_roles','strategies','strategy_versions',
      'strategy_parameter_schemas','strategy_promotions','instruments',
      'instrument_aliases','trading_calendars','data_batches',
      'dataset_versions','data_quality_issues','corporate_actions',
      'universe_snapshots','factor_definitions','factor_snapshot_manifests',
      'data_entitlements','system_flags'
  ] LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format(
      'CREATE POLICY shared_select_%s ON %I FOR SELECT TO app, worker, admin '
      || 'USING (true)',
      t, t);
  END LOOP;
END
$rls$;

-- ---------------------------------------------------------------------------
-- audit_logs: append-only by privilege AND policy (NFR-SEC-007). FORCE RLS so
-- even the table owner cannot UPDATE/DELETE/INSERT without a policy; only
-- audit_writer holds an INSERT policy, app/admin hold SELECT.
-- ---------------------------------------------------------------------------

ALTER TABLE audit_logs ENABLE ROW LEVEL SECURITY;
ALTER TABLE audit_logs FORCE ROW LEVEL SECURITY;

CREATE POLICY audit_insert_audit_writer ON audit_logs
  FOR INSERT TO audit_writer WITH CHECK (true);
CREATE POLICY audit_select_app ON audit_logs
  FOR SELECT TO app USING (true);
CREATE POLICY audit_select_admin ON audit_logs
  FOR SELECT TO admin USING (true);

-- ---------------------------------------------------------------------------
-- admin role grants: read-only cross-user views (queue / audit / shared
-- metadata). No INSERT/UPDATE/DELETE anywhere.
-- ---------------------------------------------------------------------------

GRANT SELECT ON TABLE
    user_strategy_configs, recommendation_runs, recommendation_items,
    target_portfolios, jobs, backtest_runs, backtest_metrics,
    backtest_warnings, result_artifacts, accounts, cash_ledger, positions,
    orders, fills, daily_equity, broker_connections, reconciliation_runs,
    risk_events, notifications, web_sessions, invitations
    TO admin;
GRANT SELECT ON TABLE
    users, roles, user_roles, strategies, strategy_versions,
    strategy_parameter_schemas, strategy_promotions, instruments,
    instrument_aliases, trading_calendars, data_batches, dataset_versions,
    data_quality_issues, corporate_actions, universe_snapshots,
    factor_definitions, factor_snapshot_manifests, data_entitlements,
    system_flags
    TO admin;
GRANT SELECT ON TABLE audit_logs TO admin;

-- Completes the 0001 comment "read-only to serving roles (grants land in
-- 0009)": users was omitted from 0009's shared SELECT grant.
GRANT SELECT ON TABLE users TO app, worker;
