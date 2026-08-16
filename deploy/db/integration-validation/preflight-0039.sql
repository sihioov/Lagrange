-- Assertions immediately after 0039 and before 0040.
\set ON_ERROR_STOP on

DO $$
DECLARE
    v_applied bigint;
    v_wrong_owner bigint;
BEGIN
    SELECT count(*) INTO v_applied FROM public._sqlx_migrations;
    IF v_applied <> 39 THEN
        RAISE EXCEPTION 'migration version count is %, expected 39', v_applied;
    END IF;
    IF to_regclass('public.auth_audit_outbox') IS NULL
       OR to_regprocedure('public.enqueue_auth_audit(text,text,uuid,text,text,text,bigint)') IS NULL
       OR to_regprocedure('public.deliver_auth_audit_batch(integer)') IS NULL
       OR to_regprocedure('public.auth_audit_outbox_stats()') IS NULL THEN
        RAISE EXCEPTION '0039 auth-audit outbox objects are incomplete';
    END IF;
    SELECT count(*) INTO v_wrong_owner
      FROM pg_catalog.pg_class AS c
      JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
     WHERE n.nspname = 'public'
       AND c.relname = 'auth_audit_outbox'
       AND pg_catalog.pg_get_userbyid(c.relowner) <> 'migration_owner';
    IF v_wrong_owner <> 0 THEN
        RAISE EXCEPTION '0039 auth_audit_outbox owner is not migration_owner';
    END IF;
END
$$;

DO $$
DECLARE
    v_bad_functions bigint;
BEGIN
    SELECT count(*) INTO v_bad_functions
      FROM pg_catalog.pg_proc AS p
      JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
     WHERE n.nspname = 'public'
       AND p.proname IN (
           'enqueue_auth_audit',
           'deliver_auth_audit_batch',
           'auth_audit_outbox_stats',
           'prune_auth_audit_outbox'
       )
       AND (
           pg_catalog.pg_get_userbyid(p.proowner) <> 'migration_owner'
           OR NOT EXISTS (
               SELECT 1
                 FROM pg_catalog.unnest(
                     coalesce(p.proconfig, ARRAY[]::text[])
                 ) AS setting
                WHERE setting = 'search_path=pg_catalog, pg_temp'
           )
       );
    IF v_bad_functions <> 0 THEN
        RAISE EXCEPTION '0039 function owner/search_path drift detected: %', v_bad_functions;
    END IF;
    IF NOT has_function_privilege(
           'app', 'public.enqueue_auth_audit(text,text,uuid,text,text,text,bigint)', 'EXECUTE'
       )
       OR NOT has_function_privilege(
           'audit_writer', 'public.deliver_auth_audit_batch(integer)', 'EXECUTE'
       )
       OR NOT has_function_privilege(
           'audit_writer', 'public.auth_audit_outbox_stats()', 'EXECUTE'
       )
       OR NOT has_function_privilege(
           'audit_writer', 'public.prune_auth_audit_outbox(bigint,integer)', 'EXECUTE'
       )
    THEN
        RAISE EXCEPTION '0039 auth-audit function EXECUTE ACL is incomplete';
    END IF;
    IF has_table_privilege('app', 'public.auth_audit_outbox', 'SELECT')
       OR has_table_privilege('audit_writer', 'public.auth_audit_outbox', 'SELECT')
    THEN
        RAISE EXCEPTION 'serving roles have direct auth-audit outbox table visibility';
    END IF;
END
$$;

DO $$
DECLARE
    v_rls boolean;
    v_force boolean;
BEGIN
    SELECT c.relrowsecurity, c.relforcerowsecurity
      INTO v_rls, v_force
      FROM pg_catalog.pg_class AS c
      JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
     WHERE n.nspname = 'public' AND c.relname = 'auth_audit_outbox';
    IF v_rls IS DISTINCT FROM true OR v_force IS DISTINCT FROM true THEN
        RAISE EXCEPTION '0039 auth_audit_outbox RLS is not enabled and forced';
    END IF;
END
$$;

-- Confirm the legacy target is still terminal but has no Paper obligation yet;
-- 0041, not 0039, owns the backfill.
DO $$
BEGIN
    IF (SELECT count(*) FROM public.pending_targets WHERE status <> 'PENDING') <> 1 THEN
        RAISE EXCEPTION 'terminal Paper target changed during 0039';
    END IF;
    IF (SELECT count(*) FROM public.auth_audit_outbox) <> 0 THEN
        RAISE EXCEPTION 'auth audit outbox unexpectedly contains fixture rows';
    END IF;
END
$$;

SELECT 'preflight=0039';
SELECT 'sqlx_migrations=' || count(*) FROM public._sqlx_migrations;
SELECT 'auth_audit_outbox=' || count(*) FROM public.auth_audit_outbox;
