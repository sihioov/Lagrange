-- 0012: Robustness suites — bounded batches of derived runs sharing one
-- parent (plan Todo 29; design §9.5 RobustnessSuite). Every row here is
-- pure bookkeeping over lineage/job-queue: the suite planning invariants
-- (grid limits, one-axis-per-child, holdout, version pinning) are enforced
-- by result_model::robustness::suite BEFORE any row lands here. Suite
-- status is never stored redundantly — it is always derived by joining
-- `robustness_children` to `jobs`, so there is no second source of truth to
-- drift out of sync with the queue. Both tables are tenant tables and
-- follow the 0010 policy matrix exactly (app row-local, worker full-row,
-- admin read-only, FORCE RLS).

CREATE TABLE robustness_suites (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    parent_run_id uuid NOT NULL REFERENCES backtest_runs (id) ON DELETE CASCADE,
    created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX robustness_suites_owner_idx ON robustness_suites (owner_user_id);
CREATE INDEX robustness_suites_parent_idx ON robustness_suites (parent_run_id);

CREATE TABLE robustness_children (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    suite_id      uuid NOT NULL REFERENCES robustness_suites (id) ON DELETE CASCADE,
    owner_user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    run_id        uuid NOT NULL,        -- deterministic lineage run id (uuid5)
    job_id        uuid REFERENCES jobs (id),
    axis_code     text NOT NULL,        -- 'cost_stress' | 'execution_delay' | 'period_split' | ...
    axis_json     jsonb NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    -- Re-planning the identical suite must be a no-op, never a duplicate
    -- child row (the lineage run id is already crash-safe/deterministic;
    -- this constraint makes the DB agree).
    CONSTRAINT robustness_children_suite_run_key UNIQUE (suite_id, run_id)
);
CREATE INDEX robustness_children_suite_idx ON robustness_children (suite_id);
CREATE INDEX robustness_children_owner_idx ON robustness_children (owner_user_id);
CREATE INDEX robustness_children_job_idx ON robustness_children (job_id);

-- RLS: tenant tables, FORCE, row-local policies (same matrix as 0010/0011).
ALTER TABLE robustness_suites ENABLE ROW LEVEL SECURITY;
ALTER TABLE robustness_suites FORCE ROW LEVEL SECURITY;
ALTER TABLE robustness_children ENABLE ROW LEVEL SECURITY;
ALTER TABLE robustness_children FORCE ROW LEVEL SECURITY;

DO $rls$
DECLARE
  t text;
BEGIN
  FOREACH t IN ARRAY ARRAY['robustness_suites', 'robustness_children'] LOOP
    EXECUTE format(
      'CREATE POLICY tenant_all_app_%s ON %I FOR ALL TO app '
      || 'USING (owner_user_id = current_setting(''app.actor_user_id'', true)::uuid) '
      || 'WITH CHECK (owner_user_id = current_setting(''app.actor_user_id'', true)::uuid)',
      t, t);
    EXECUTE format(
      'CREATE POLICY tenant_all_owner_%s ON %I FOR ALL TO migration_owner '
      || 'USING (owner_user_id = current_setting(''app.actor_user_id'', true)::uuid) '
      || 'WITH CHECK (owner_user_id = current_setting(''app.actor_user_id'', true)::uuid)',
      t, t);
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

-- Grants: app full CRUD (actor-GUC'd) for suite creation/cancellation;
-- worker full-row access for the (future) robustness-child result writer;
-- admin read-only cross-user view.
GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE robustness_suites, robustness_children TO app;
GRANT SELECT, INSERT, UPDATE ON TABLE robustness_suites, robustness_children TO worker;
GRANT SELECT ON TABLE robustness_suites, robustness_children TO admin;
