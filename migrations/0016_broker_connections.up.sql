-- 0016: complete the Owner-only Live boundary (plan Todo 37; design §6.12,
-- §13.4; requirements FR-LIVE-001..003, NFR-SEC-004).
--
-- `broker_connections` ALREADY EXISTS from 0007, which got the important part
-- right: `account_ref` and `secret_ref` are references, never values. This
-- migration extends that table rather than replacing it, and adds the two
-- things Phase 3 needs and 0007 did not model: a per-connection Live NODE with
-- a one-per-account guarantee, and the kill switch.
--
-- What is added to the existing table:
--   * `app_key_ref`  — KIS needs an app KEY as well as an app SECRET, and 0007
--                      modelled only one credential reference.
--   * `profile`      — 'mock' or 'live'. No default: a live connection is
--                      always an explicit choice, never arrived at by omission.
--   * `account_no_masked` / `account_product_code` — display and request
--                      metadata. The masked form is what the UI and every log
--                      may show; the real account number stays in the secret
--                      store behind `account_ref`.
--
-- The reference-shape CHECKs are the point: a row that COULD hold a secret is a
-- row that eventually leaks one through a backup, a log, or an admin read.

ALTER TABLE broker_connections
    ADD COLUMN label                text NOT NULL DEFAULT '',
    ADD COLUMN profile              text NOT NULL DEFAULT 'mock',
    ADD COLUMN app_key_ref          text NOT NULL DEFAULT 'env:KIS_APP_KEY',
    ADD COLUMN account_no_masked    text NOT NULL DEFAULT '****',
    ADD COLUMN account_product_code text NOT NULL DEFAULT '01';

-- The defaults above exist only so the ALTER can run against existing rows;
-- new rows must be explicit. Drop them so an INSERT cannot silently acquire a
-- profile or a credential reference it never stated.
ALTER TABLE broker_connections
    ALTER COLUMN label DROP DEFAULT,
    ALTER COLUMN profile DROP DEFAULT,
    ALTER COLUMN app_key_ref DROP DEFAULT,
    ALTER COLUMN account_no_masked DROP DEFAULT,
    ALTER COLUMN account_product_code DROP DEFAULT;

ALTER TABLE broker_connections
    ADD CONSTRAINT broker_connections_profile_check
        CHECK (profile IN ('mock', 'live')),
    -- A credential column holds a LOCATION. Anything that is not a recognised
    -- reference form is rejected at the schema level, so an operator cannot
    -- paste a raw secret into the field "just this once".
    ADD CONSTRAINT broker_connections_app_key_is_a_reference
        CHECK (app_key_ref ~ '^(env:[A-Za-z_][A-Za-z0-9_]*|file:/.+)$'),
    ADD CONSTRAINT broker_connections_secret_is_a_reference
        CHECK (secret_ref ~ '^(env:[A-Za-z_][A-Za-z0-9_]*|file:/.+)$'),
    -- The masked account must actually be masked, so an unmasked paste fails
    -- loudly instead of being stored and later displayed.
    ADD CONSTRAINT broker_connections_account_is_masked
        CHECK (account_no_masked LIKE '****%');

-- ---------------------------------------------------------------------------
-- Live nodes: one per connection, ever.
-- ---------------------------------------------------------------------------
CREATE TABLE broker_nodes (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    connection_id  uuid NOT NULL REFERENCES broker_connections (id) ON DELETE RESTRICT,
    owner_user_id  uuid NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    status         text NOT NULL DEFAULT 'STARTING',
    process_id     text,
    started_at     timestamptz NOT NULL DEFAULT now(),
    stopped_at     timestamptz,
    stop_reason    text,

    CONSTRAINT broker_nodes_status_check CHECK (status IN ('STARTING', 'RUNNING', 'STOPPED')),
    CONSTRAINT broker_nodes_stopped_has_time CHECK (
        (status = 'STOPPED' AND stopped_at IS NOT NULL)
        OR (status <> 'STOPPED' AND stopped_at IS NULL)
    )
);
-- Design §6.12: one Live node per account/process. Two nodes on one account
-- double every order it places, so this is a structural guarantee rather than
-- an application check that two concurrent start requests could race past.
CREATE UNIQUE INDEX broker_nodes_one_active_per_connection
    ON broker_nodes (connection_id) WHERE status <> 'STOPPED';
CREATE INDEX broker_nodes_owner_idx ON broker_nodes (owner_user_id);

-- ---------------------------------------------------------------------------
-- The kill switch: one row, engaged by default.
-- ---------------------------------------------------------------------------
CREATE TABLE live_kill_switch (
    id         boolean PRIMARY KEY DEFAULT true,
    engaged    boolean NOT NULL DEFAULT true,
    reason     text,
    changed_by uuid REFERENCES users (id),
    changed_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT live_kill_switch_singleton CHECK (id = true)
);
-- Engaged by default so a fresh install, a restored backup, and a half-applied
-- migration all land in the SAFE state. Live must be switched on deliberately,
-- never arrived at by omission.
INSERT INTO live_kill_switch (id, engaged, reason)
    VALUES (true, true, 'default: Live is disabled until an Owner explicitly enables it');

-- ---------------------------------------------------------------------------
-- RLS. These are Owner-only surfaces: the API layer requires Owner + fresh MFA
-- before any of this is reachable, so RLS here is a second fence rather than
-- the per-actor filter the tenant tables use.
-- ---------------------------------------------------------------------------
ALTER TABLE broker_nodes ENABLE ROW LEVEL SECURITY;
ALTER TABLE broker_nodes FORCE ROW LEVEL SECURITY;
ALTER TABLE live_kill_switch ENABLE ROW LEVEL SECURITY;
ALTER TABLE live_kill_switch FORCE ROW LEVEL SECURITY;

DO $rls$
DECLARE
  t text;
BEGIN
  FOREACH t IN ARRAY ARRAY['broker_nodes', 'live_kill_switch'] LOOP
    EXECUTE format(
      'CREATE POLICY owner_all_app_%s ON %I FOR ALL TO app USING (true) WITH CHECK (true)',
      t, t);
    EXECUTE format(
      'CREATE POLICY owner_all_migration_%s ON %I FOR ALL TO migration_owner USING (true) WITH CHECK (true)',
      t, t);
    EXECUTE format(
      'CREATE POLICY owner_select_admin_%s ON %I FOR SELECT TO admin USING (true)',
      t, t);
  END LOOP;
END
$rls$;

GRANT SELECT, INSERT, UPDATE ON TABLE broker_nodes, live_kill_switch TO app;
GRANT SELECT ON TABLE broker_nodes, live_kill_switch TO admin;
-- `worker` is deliberately granted nothing here: a backtest or Paper worker has
-- no business reading where broker credentials live, and the Live node runs on
-- its own Owner-scoped path.
