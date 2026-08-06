-- 0009: Role grants (design §7.3, §14.3; plan Todo 3). Roles themselves are
-- created by the harness bootstrap (tests/integration/migration-contract/
-- bootstrap.sql); migrations only assign privileges.
--
-- Privilege model:
--   migration_owner : runs migrations, OWNS every table (asserted by the
--                     migration-contract suite); never a serving role.
--   app             : serving API role — full CRUD on tenant tables, SELECT
--                     on system-owned shared tables, read-only audit. NO
--                     table ownership, NO schema CREATE, NO BYPASSRLS.
--   worker          : claims/advances jobs + attempts, stores normalized
--                     backtest results, reads ledgers read-only.
--   audit_writer    : the ONLY writer of append-only audit_logs (INSERT
--                     only; never UPDATE/DELETE/TRUNCATE).
--
-- Grants are conservative: anything not listed is denied by default.

-- Shared system-owned metadata: read-only for app and worker.
GRANT SELECT ON TABLE strategies, strategy_versions, strategy_parameter_schemas,
    strategy_promotions, instruments, instrument_aliases, trading_calendars,
    data_batches, dataset_versions, data_quality_issues, corporate_actions,
    universe_snapshots, factor_definitions, factor_snapshot_manifests,
    data_entitlements, roles, user_roles, system_flags
    TO app, worker;

-- app: full CRUD on tenant-owned tables.
GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE user_strategy_configs,
    recommendation_runs, recommendation_items, target_portfolios, jobs,
    backtest_runs, backtest_metrics, backtest_warnings, result_artifacts,
    accounts, cash_ledger, positions, orders, fills, daily_equity,
    broker_connections, reconciliation_runs, risk_events, notifications,
    web_sessions, invitations
    TO app;

-- app: audit visibility is read-only (append-only by design).
GRANT SELECT ON TABLE audit_logs TO app;

-- worker: claim/lock jobs, record attempts, store normalized results.
GRANT SELECT, UPDATE ON TABLE jobs TO worker;
GRANT SELECT, INSERT, UPDATE ON TABLE job_attempts TO worker;
GRANT SELECT, INSERT, UPDATE ON TABLE backtest_runs TO worker;
GRANT SELECT, INSERT ON TABLE backtest_metrics TO worker;
GRANT SELECT, INSERT ON TABLE backtest_warnings TO worker;
GRANT SELECT, INSERT, UPDATE ON TABLE result_artifacts TO worker;
GRANT SELECT ON TABLE accounts, cash_ledger, positions, orders, fills, daily_equity TO worker;
GRANT INSERT ON TABLE notifications TO worker;

-- audit_writer: append-only writer of audit_logs and nothing else.
GRANT INSERT ON TABLE audit_logs TO audit_writer;
