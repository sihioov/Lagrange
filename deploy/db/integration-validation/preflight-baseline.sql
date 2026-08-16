-- Assertions immediately before 0039. The database is a fresh 0038 schema
-- with one normalized pending invite and one terminal Paper target.
\set ON_ERROR_STOP on

DO $$
DECLARE
    v_duplicates bigint;
    v_pending bigint;
    v_terminal bigint;
    v_applied bigint;
    v_wrong_owner bigint;
BEGIN
    SELECT count(*) INTO v_duplicates
      FROM (
        SELECT lower(btrim(email))
        FROM public.invitations
        WHERE status = 'PENDING'
        GROUP BY lower(btrim(email))
        HAVING count(*) > 1
      ) duplicates;
    IF v_duplicates <> 0 THEN
        RAISE EXCEPTION 'normalized pending-invite duplicates found before 0039: %', v_duplicates;
    END IF;

    SELECT count(*) INTO v_pending
      FROM public.invitations
     WHERE lower(btrim(email)) = 'pending-invite@example.test'
       AND status = 'PENDING';
    IF v_pending <> 1 THEN
        RAISE EXCEPTION 'representative pending invite count is %, expected 1', v_pending;
    END IF;

    SELECT count(*) INTO v_terminal
      FROM public.pending_targets
     WHERE status <> 'PENDING';
    IF v_terminal <> 1 THEN
        RAISE EXCEPTION 'terminal Paper target count is %, expected 1', v_terminal;
    END IF;

    SELECT count(*) INTO v_applied FROM public._sqlx_migrations;
    IF v_applied <> 38 THEN
        RAISE EXCEPTION 'migration version count is %, expected 38', v_applied;
    END IF;

    SELECT count(*) INTO v_wrong_owner
      FROM pg_catalog.pg_class AS c
      JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
     WHERE n.nspname = 'public'
       AND c.relkind IN ('r', 'p')
       AND pg_catalog.pg_get_userbyid(c.relowner) <> 'migration_owner';
    IF v_wrong_owner <> 0 THEN
        RAISE EXCEPTION 'public table ownership drift before 0039: % tables', v_wrong_owner;
    END IF;

    IF NOT has_schema_privilege('migration_owner', 'public', 'CREATE')
       OR has_schema_privilege('app', 'public', 'CREATE')
       OR has_schema_privilege('worker', 'public', 'CREATE')
       OR has_schema_privilege('audit_writer', 'public', 'CREATE')
       OR has_schema_privilege('research_writer', 'public', 'CREATE')
       OR has_schema_privilege('admin', 'public', 'CREATE') THEN
        RAISE EXCEPTION 'public schema CREATE privilege boundary is incorrect';
    END IF;
END
$$;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM pg_catalog.pg_roles
         WHERE rolname IN ('migration_owner', 'app', 'worker', 'audit_writer', 'research_writer', 'admin')
           AND (
               NOT rolcanlogin OR rolsuper OR rolcreatedb OR rolcreaterole
               OR rolreplication OR rolbypassrls
           )
    ) THEN
        RAISE EXCEPTION 'one or more serving roles are over-privileged or cannot log in';
    END IF;
END
$$;

DO $$
BEGIN
    IF to_regclass('public.auth_audit_outbox') IS NOT NULL
       OR to_regclass('public.paper_settlement_outbox') IS NOT NULL THEN
        RAISE EXCEPTION '0039/0041 objects exist before their upgrade stages';
    END IF;
END
$$;

SELECT 'preflight=baseline';
SELECT 'sqlx_migrations=' || count(*) FROM public._sqlx_migrations;
SELECT 'normalized_pending_invite_duplicates=' || count(*)
  FROM (
    SELECT lower(btrim(email))
      FROM public.invitations
     WHERE status = 'PENDING'
     GROUP BY lower(btrim(email))
    HAVING count(*) > 1
  ) duplicates;
SELECT 'terminal_paper_targets=' || count(*)
  FROM public.pending_targets
 WHERE status <> 'PENDING';
SELECT 'terminal_paper_outbox=' ||
       CASE WHEN to_regclass('public.paper_settlement_outbox') IS NULL THEN 0 ELSE 0 END;
