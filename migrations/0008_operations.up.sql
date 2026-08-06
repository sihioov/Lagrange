-- 0008: Operations — audit_logs, worker_heartbeats, notifications,
-- system_flags. Design §7.2 (Operations), §14.3, §15; requirements §10.3
-- NFR-SEC-007. audit_logs is append-only: only audit_writer may INSERT and
-- no serving role may UPDATE/DELETE/TRUNCATE (grants land in 0009).
-- notifications is a tenant table (owner_user_id); the rest are system-owned.

-- Append-only audit trail: actor/time/target/before-after/reason plus a
-- correlation id for request tracing. Rows are immutable by privilege
-- design, never by convention.
CREATE TABLE audit_logs (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    action         text NOT NULL,           -- e.g. 'invite.created', 'job.canceled'
    actor_role     text NOT NULL DEFAULT 'system',
    actor_user_id  uuid REFERENCES users (id),
    target_type    text,
    target_id      text,
    before_json    jsonb,
    after_json     jsonb,
    reason         text,
    correlation_id text,
    created_at     timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX audit_logs_created_at_idx ON audit_logs (created_at);
CREATE INDEX audit_logs_action_idx ON audit_logs (action);

CREATE TABLE worker_heartbeats (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    worker_id    text NOT NULL,
    worker_type  text NOT NULL,             -- 'research' | 'backtest' | 'paper' | 'report'
    heartbeat_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX worker_heartbeats_worker_idx ON worker_heartbeats (worker_id, heartbeat_at);

CREATE TABLE notifications (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind          text NOT NULL,
    title         text NOT NULL,
    body          text NOT NULL DEFAULT '',
    read_at       timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX notifications_owner_idx ON notifications (owner_user_id, read_at);

CREATE TABLE system_flags (
    id         text PRIMARY KEY,            -- e.g. 'maintenance_mode'
    value_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    updated_by text NOT NULL DEFAULT 'system',
    updated_at timestamptz NOT NULL DEFAULT now()
);
