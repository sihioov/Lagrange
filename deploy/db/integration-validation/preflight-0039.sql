-- Assertions immediately after 0039 and before 0040.
\set ON_ERROR_STOP on
SELECT pg_catalog.set_config(
    'lagrange.validation.catalog_sha256', :'catalog_sha256', false
);

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

-- migration_owner is subject to forced RLS on pending_targets, so the terminal
-- fixture check runs in the validator's separate supervisor-only pass below.
\if :terminal_only
DO $$
DECLARE
    v_terminal bigint;
BEGIN
    SELECT count(*) INTO v_terminal
      FROM public.pending_targets
     WHERE status <> 'PENDING';
    IF v_terminal <> 1 THEN
        RAISE EXCEPTION 'terminal Paper target changed during 0039';
    END IF;
END
$$;
SELECT 'preflight=0039-terminal-paper';
SELECT 'terminal_paper_targets=' || count(*)
  FROM public.pending_targets
 WHERE status <> 'PENDING';
\quit
\endif

-- The audit assertions below deliberately run as migration_owner. This proves
-- its post-0039 audit RLS policy and read access directly.
DO $$
DECLARE
    v_sha256 text := pg_catalog.current_setting(
        'lagrange.validation.catalog_sha256'
    );
    v_event public.auth_audit_outbox%ROWTYPE;
    v_event_count bigint;
BEGIN
    SELECT count(*) INTO v_event_count FROM public.auth_audit_outbox;
    IF v_event_count <> 1 THEN
        RAISE EXCEPTION 'auth audit outbox contains %, expected the single catalog install event', v_event_count;
    END IF;
    SELECT * INTO v_event
      FROM public.auth_audit_outbox
     WHERE event_key = 'strategy-catalog:' || v_sha256 || ':installed';
    IF NOT FOUND
       OR v_event.action IS DISTINCT FROM 'strategy_catalog.installed'
       OR v_event.actor_role IS DISTINCT FROM 'system'
       OR v_event.actor_user_id IS NOT NULL
       OR v_event.target_type IS DISTINCT FROM 'strategy_catalog'
       OR v_event.target_id IS DISTINCT FROM v_sha256
       OR v_event.reason IS DISTINCT FROM 'IMMUTABLE_RELEASE_BASELINE'
       OR v_event.created_at IS DISTINCT FROM pg_catalog.to_timestamp(1787670000::double precision)
       OR v_event.delivered_at IS NOT NULL THEN
        RAISE EXCEPTION 'catalog install audit payload does not match the immutable baseline';
    END IF;
END
$$;

SELECT 'preflight=0039';
SELECT 'sqlx_migrations=' || count(*) FROM public._sqlx_migrations;
SELECT 'auth_audit_outbox=' || count(*) FROM public.auth_audit_outbox;
SELECT 'catalog_audit_event=' || id
  FROM public.auth_audit_outbox
 WHERE event_key = 'strategy-catalog:' || current_setting('lagrange.validation.catalog_sha256') || ':installed';
