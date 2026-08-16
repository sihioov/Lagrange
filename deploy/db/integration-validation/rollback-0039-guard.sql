-- This is intentionally expected to fail. It executes the tracked 0039 down
-- migration with one synthetic undelivered obligation present. The shell
-- harness asserts SQLSTATE 55000 and then verifies that the table survived.
\set ON_ERROR_STOP on
BEGIN;
INSERT INTO public.auth_audit_outbox (
    event_key, action, actor_user_id, target_type, target_id, reason
)
VALUES (
    'validation:0039-rollback-guard',
    'validation.rollback_guard',
    NULL,
    NULL,
    NULL,
    'synthetic undelivered row for rollback guard'
);
COMMIT;
BEGIN;
\i /validation/migrations/41/0039_auth_audit_outbox.down.sql
