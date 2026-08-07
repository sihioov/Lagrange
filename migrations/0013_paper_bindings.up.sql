-- 0013: Paper account funding, cost profile, and strategy binding history
-- (plan Todo 30; design §10.1 PaperAccount, §10.2 processing rules).
--
-- accounts gains its funding/cost identity (initial_cash, cost_profile);
-- current cash/positions/equity stay derived from cash_ledger/positions/
-- daily_equity (0006/0007's manifest pattern: never cache a second source
-- of truth). initial_cash is nullable at the schema level -- "a PAPER
-- account must be positively funded" is an application-layer rule enforced
-- before the row is ever written, NOT a DB CHECK, because tenancy_rls.rs's
-- existing RLS-denial tests insert PAPER rows without it and must keep
-- observing SQLSTATE 42501, not a constraint violation.
--
-- account_strategy_bindings is the immutable one-binding-per-account
-- history: FR-PAPER-004 lets a Member switch strategies on the SAME account
-- (closing the old binding, opening a new one) without ever mixing
-- execution history between strategy versions. A partial unique index
-- enforces "at most one ACTIVE binding" without ever deleting a row; the
-- only mutation ever allowed on an existing row is closing it out
-- (unbound_at NULL -> a timestamp), enforced by application code, never by
-- rewriting a binding's strategy identity.

ALTER TABLE accounts
    ADD COLUMN initial_cash        numeric(18, 4),
    ADD COLUMN cost_profile_id     text NOT NULL DEFAULT 'KRX_ETF_DEFAULT',
    ADD COLUMN cost_profile_version int NOT NULL DEFAULT 1;

ALTER TABLE accounts
    ADD CONSTRAINT accounts_cost_profile_id_check
        CHECK (cost_profile_id IN ('KRX_ETF_DEFAULT', 'CUSTOM'));

CREATE TABLE account_strategy_bindings (
    id                 uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id         uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    owner_user_id      uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    strategy_config_id uuid NOT NULL REFERENCES user_strategy_configs (id),
    strategy_id        text NOT NULL REFERENCES strategies (id),
    strategy_version   text NOT NULL,
    bound_at           timestamptz NOT NULL DEFAULT now(),
    unbound_at         timestamptz,
    CONSTRAINT account_strategy_bindings_unbound_after_bound
        CHECK (unbound_at IS NULL OR unbound_at > bound_at)
);
CREATE INDEX account_strategy_bindings_account_idx ON account_strategy_bindings (account_id);
CREATE INDEX account_strategy_bindings_owner_idx ON account_strategy_bindings (owner_user_id);
-- At most one ACTIVE (unbound_at IS NULL) binding per account -- the
-- structural enforcement of "one binding per account" (never two
-- strategies combined in one account).
CREATE UNIQUE INDEX account_strategy_bindings_one_active_per_account
    ON account_strategy_bindings (account_id) WHERE unbound_at IS NULL;

-- RLS: tenant table, FORCE, row-local policies (0010/0011/0012 matrix).
ALTER TABLE account_strategy_bindings ENABLE ROW LEVEL SECURITY;
ALTER TABLE account_strategy_bindings FORCE ROW LEVEL SECURITY;

CREATE POLICY tenant_all_app_account_strategy_bindings ON account_strategy_bindings FOR ALL TO app
    USING (owner_user_id = current_setting('app.actor_user_id', true)::uuid)
    WITH CHECK (owner_user_id = current_setting('app.actor_user_id', true)::uuid);
CREATE POLICY tenant_all_owner_account_strategy_bindings ON account_strategy_bindings FOR ALL TO migration_owner
    USING (owner_user_id = current_setting('app.actor_user_id', true)::uuid)
    WITH CHECK (owner_user_id = current_setting('app.actor_user_id', true)::uuid);
CREATE POLICY tenant_all_worker_account_strategy_bindings ON account_strategy_bindings FOR ALL TO worker
    USING (true) WITH CHECK (true);
CREATE POLICY tenant_select_admin_account_strategy_bindings ON account_strategy_bindings FOR SELECT TO admin
    USING (true);

GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE account_strategy_bindings TO app;
GRANT SELECT, INSERT, UPDATE ON TABLE account_strategy_bindings TO worker;
GRANT SELECT ON TABLE account_strategy_bindings TO admin;
