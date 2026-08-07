-- 0011: Notification subscriptions and delivery outcomes (plan Todo 27;
-- design §7.2 Operations, §15.3 alert grades; requirements FR-RPT-002
-- "사용자별 구독 설정과 전달 결과가 기록된다").
--
-- Extends 0008's `notifications` (the web rows) with per-user subscription
-- settings and durable delivery records (SUCCESS/FAILED + error detail), so
-- an outage is never silent. Both tables are tenant tables (owner_user_id)
-- and follow the 0010 policy matrix exactly: app full CRUD under the actor
-- GUC, worker INSERT for engine-originated rows, admin read-only SELECT.

CREATE TABLE notification_subscriptions (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind          text NOT NULL,          -- 'job' | 'recommendation' | 'backtest' | 'alert'
    channel       text NOT NULL,          -- 'web' | 'email' | 'admin'
    enabled       boolean NOT NULL DEFAULT true,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT notification_subscriptions_kind_check CHECK (
        kind IN ('job', 'recommendation', 'backtest', 'alert')
    ),
    CONSTRAINT notification_subscriptions_channel_check CHECK (
        channel IN ('web', 'email', 'admin')
    ),
    CONSTRAINT notification_subscriptions_owner_kind_channel UNIQUE (owner_user_id, kind, channel)
);
CREATE INDEX notification_subscriptions_owner_idx ON notification_subscriptions (owner_user_id);

CREATE TABLE notification_deliveries (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    notification_id uuid NOT NULL REFERENCES notifications (id) ON DELETE CASCADE,
    owner_user_id   uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    channel         text NOT NULL,        -- 'web' | 'email' | 'admin'
    status          text NOT NULL,        -- 'SUCCESS' | 'FAILED'
    error_detail    text,
    attempted_at    timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT notification_deliveries_channel_check CHECK (
        channel IN ('web', 'email', 'admin')
    ),
    CONSTRAINT notification_deliveries_status_check CHECK (status IN ('SUCCESS', 'FAILED'))
);
CREATE INDEX notification_deliveries_notification_idx ON notification_deliveries (notification_id);
CREATE INDEX notification_deliveries_owner_idx ON notification_deliveries (owner_user_id);

-- RLS: tenant tables, FORCE, row-local policies (same matrix as 0010).
ALTER TABLE notification_subscriptions ENABLE ROW LEVEL SECURITY;
ALTER TABLE notification_subscriptions FORCE ROW LEVEL SECURITY;
ALTER TABLE notification_deliveries ENABLE ROW LEVEL SECURITY;
ALTER TABLE notification_deliveries FORCE ROW LEVEL SECURITY;

DO $rls$
DECLARE
  t text;
BEGIN
  FOREACH t IN ARRAY ARRAY['notification_subscriptions', 'notification_deliveries'] LOOP
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

-- Grants: app full CRUD (actor-GUC'd), worker INSERT for engine-originated
-- deliveries, admin read-only cross-user view.
GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE notification_subscriptions, notification_deliveries TO app;
GRANT INSERT ON TABLE notification_deliveries TO worker;
GRANT SELECT ON TABLE notification_subscriptions, notification_deliveries TO admin;
