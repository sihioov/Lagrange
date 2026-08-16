-- Representative legacy rows for the 0038 -> 0039/0040/0041 upgrade.
-- This file is fed to psql over stdin by the validator; it contains no
-- credentials and is safe to keep in the repository.
BEGIN;

INSERT INTO public.users (id, issuer, subject, email, display_name)
VALUES (
    '00000000-0000-4000-8000-000000000039',
    'https://validation.invalid',
    'postgres-integration-validation-owner',
    'validation-owner@example.test',
    'Validation Owner'
)
ON CONFLICT (id) DO NOTHING;

INSERT INTO public.roles (id, description)
VALUES ('owner', 'Owner'), ('member', 'Member')
ON CONFLICT (id) DO NOTHING;

INSERT INTO public.user_roles (user_id, role_id, granted_by)
VALUES (
    '00000000-0000-4000-8000-000000000039',
    'owner',
    '00000000-0000-4000-8000-000000000039'
)
ON CONFLICT (user_id, role_id) DO NOTHING;

-- A second Owner is used only by identity-boundary.sql to prove that the
-- SECURITY DEFINER functions cannot be used by Owner A with Owner B's UUID.
INSERT INTO public.users (id, issuer, subject, email, display_name)
VALUES (
    '00000000-0000-4000-8000-00000000003a',
    'https://validation.invalid',
    'postgres-integration-validation-owner-b',
    'validation-owner-b@example.test',
    'Validation Owner B'
)
ON CONFLICT (id) DO NOTHING;

INSERT INTO public.user_roles (user_id, role_id, granted_by)
VALUES (
    '00000000-0000-4000-8000-00000000003a',
    'owner',
    '00000000-0000-4000-8000-000000000039'
)
ON CONFLICT (user_id, role_id) DO NOTHING;

-- Synthetic active session used only when the finalized identity functions
-- require an authenticated capability before the owner-bound mutation.
INSERT INTO public.web_sessions (
    id, user_id, session_hash, csrf_hash, expires_at
)
VALUES (
    '00000000-0000-4000-8000-000000000396',
    '00000000-0000-4000-8000-000000000039',
    repeat('d', 64),
    repeat('e', 64),
    now() + interval '1 day'
)
ON CONFLICT (id) DO NOTHING;

INSERT INTO public.strategies (id, display_name, state)
VALUES ('postgres_validation_paper', 'PostgreSQL validation Paper', 'Paper')
ON CONFLICT (id) DO NOTHING;

INSERT INTO public.user_strategy_configs (
    id, owner_user_id, strategy_id, strategy_version, config_json
)
VALUES (
    '00000000-0000-4000-8000-000000000391',
    '00000000-0000-4000-8000-000000000039',
    'postgres_validation_paper',
    '1.0.0',
    '{}'::jsonb
)
ON CONFLICT (id) DO NOTHING;

INSERT INTO public.accounts (
    id, owner_user_id, account_type, name, currency, status, initial_cash
)
VALUES (
    '00000000-0000-4000-8000-000000000392',
    '00000000-0000-4000-8000-000000000039',
    'PAPER',
    'postgres-integration-validation',
    'KRW',
    'ACTIVE',
    1000000
)
ON CONFLICT (id) DO NOTHING;

-- The padded/case-varied address is the normalized duplicate key exercised by
-- 0040's fail-closed guard and partial unique index.
INSERT INTO public.invitations (
    id, user_id, email, invite_hash, status, expires_at
)
VALUES (
    '00000000-0000-4000-8000-000000000393',
    '00000000-0000-4000-8000-000000000039',
    '  Pending-Invite@Example.Test ',
    repeat('a', 64),
    'PENDING',
    now() + interval '7 days'
)
ON CONFLICT (id) DO NOTHING;

INSERT INTO public.invitations (
    id, user_id, email, invite_hash, status, expires_at
)
VALUES (
    '00000000-0000-4000-8000-000000000395',
    '00000000-0000-4000-8000-00000000003a',
    'owner-b-claim@example.test',
    repeat('b', 64),
    'PENDING',
    now() + interval '7 days'
)
ON CONFLICT (id) DO NOTHING;

-- A terminal legacy Paper target intentionally has no outbox before 0041.
-- 0041 must backfill exactly one durable obligation for this row.
INSERT INTO public.pending_targets (
    id, account_id, owner_user_id, strategy_config_id,
    computed_on, effective_date, targets_json, status, executed_at
)
VALUES (
    '00000000-0000-4000-8000-000000000394',
    '00000000-0000-4000-8000-000000000392',
    '00000000-0000-4000-8000-000000000039',
    '00000000-0000-4000-8000-000000000391',
    DATE '2026-08-10',
    DATE '2026-08-11',
    '[]'::jsonb,
    'EXECUTED',
    now()
)
ON CONFLICT (id) DO NOTHING;

COMMIT;
