-- 0014: Paper pending targets + the paper-runner's ledger write grants
-- (plan Todo 31; design §9.2 processing order, §10.2; requirements UC-04,
-- FR-PAPER-002/003, AT-07).
--
-- A PendingTarget is computed at close(T) and executes at the OPEN of
-- `effective_date` (T+1) and nowhere else. UNIQUE (account_id,
-- effective_date) is the structural "unique PendingTarget" the plan
-- requires: recomputing the same close can never queue a second target for
-- the same session, so a restarted scheduler is idempotent by schema, not
-- by convention.
--
-- `status` tracks only what the runner needs to resume: PENDING (queued at
-- close, not yet executed) -> EXECUTED (its session's orders/fills are in
-- the ledger) | SKIPPED (the session produced no orders, e.g. every
-- instrument was below the rebalance threshold). A target is never
-- deleted, so an entitlement pause or a missed session leaves an auditable
-- record rather than a hole.
--
-- targets_json holds the weight vector verbatim (canonical instrument ->
-- weight), matching portfolio_model::sizing::TargetAllocation's wire shape.
-- The orders/fills it produced are NOT duplicated here: they live in
-- orders/fills (0007) and are correlated by the deterministic uuid5 ids
-- portfolio_model::paper_flow mints, so there is no second source of truth.

CREATE TABLE pending_targets (
    id                 uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id         uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    owner_user_id      uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    strategy_config_id uuid NOT NULL REFERENCES user_strategy_configs (id),
    computed_on        date NOT NULL,          -- the close date T that produced it
    effective_date     date NOT NULL,          -- the session T+1 it executes at
    targets_json       jsonb NOT NULL,
    status             text NOT NULL DEFAULT 'PENDING',
    executed_at        timestamptz,
    created_at         timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pending_targets_status_check CHECK (status IN ('PENDING', 'EXECUTED', 'SKIPPED')),
    CONSTRAINT pending_targets_effective_after_computed CHECK (effective_date > computed_on),
    CONSTRAINT pending_targets_executed_when_terminal CHECK (
        (status = 'PENDING' AND executed_at IS NULL) OR (status <> 'PENDING')
    ),
    -- One target per account per session, forever.
    CONSTRAINT pending_targets_account_effective_key UNIQUE (account_id, effective_date)
);
CREATE INDEX pending_targets_account_idx ON pending_targets (account_id);
CREATE INDEX pending_targets_owner_idx ON pending_targets (owner_user_id);
-- The runner's claim scan: everything due at or before a session date.
CREATE INDEX pending_targets_due_idx ON pending_targets (effective_date) WHERE status = 'PENDING';

-- RLS: tenant table, FORCE, row-local policies (0010/0011/0012/0013 matrix).
ALTER TABLE pending_targets ENABLE ROW LEVEL SECURITY;
ALTER TABLE pending_targets FORCE ROW LEVEL SECURITY;

CREATE POLICY tenant_all_app_pending_targets ON pending_targets FOR ALL TO app
    USING (owner_user_id = current_setting('app.actor_user_id', true)::uuid)
    WITH CHECK (owner_user_id = current_setting('app.actor_user_id', true)::uuid);
CREATE POLICY tenant_all_owner_pending_targets ON pending_targets FOR ALL TO migration_owner
    USING (owner_user_id = current_setting('app.actor_user_id', true)::uuid)
    WITH CHECK (owner_user_id = current_setting('app.actor_user_id', true)::uuid);
CREATE POLICY tenant_all_worker_pending_targets ON pending_targets FOR ALL TO worker
    USING (true) WITH CHECK (true);
CREATE POLICY tenant_select_admin_pending_targets ON pending_targets FOR SELECT TO admin
    USING (true);

GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE pending_targets TO app;
GRANT SELECT, INSERT, UPDATE ON TABLE pending_targets TO worker;
GRANT SELECT ON TABLE pending_targets TO admin;

-- The paper-runner writes the ledger. 0009 granted `worker` SELECT-only on
-- these tables (it was provisioned for the backtest worker's reads); the
-- Paper session open now INSERTs orders/fills/cash_ledger rows and upserts
-- positions/daily_equity as the engine role. `app` keeps its own full CRUD
-- from 0009 and every write still passes RLS.
GRANT INSERT, UPDATE ON TABLE orders, fills, cash_ledger, positions, daily_equity TO worker;
GRANT UPDATE ON TABLE accounts TO worker;
