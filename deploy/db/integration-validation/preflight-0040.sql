-- Assertions after 0039 + 0040 and before 0041.
\set ON_ERROR_STOP on

DO $$
DECLARE
    v_duplicates bigint;
    v_applied bigint;
    v_wrong_owner bigint;
    v_index_valid boolean;
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
        RAISE EXCEPTION 'normalized pending-invite duplicates found after 0040: %', v_duplicates;
    END IF;

    SELECT count(*) INTO v_applied FROM public._sqlx_migrations;
    IF v_applied <> 40 THEN
        RAISE EXCEPTION 'migration version count is %, expected 40', v_applied;
    END IF;

    SELECT i.indisvalid INTO v_index_valid
      FROM pg_catalog.pg_class AS c
      JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
      JOIN pg_catalog.pg_index AS i ON i.indexrelid = c.oid
     WHERE n.nspname = 'public'
       AND c.relname = 'invitations_pending_email_uq';
    IF v_index_valid IS DISTINCT FROM true THEN
        RAISE EXCEPTION '0040 normalized pending-invite unique index is missing or invalid';
    END IF;

    SELECT count(*) INTO v_wrong_owner
      FROM pg_catalog.pg_class AS c
      JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
     WHERE n.nspname = 'public'
       AND c.relkind IN ('r', 'p')
       AND pg_catalog.pg_get_userbyid(c.relowner) <> 'migration_owner';
    IF v_wrong_owner <> 0 THEN
        RAISE EXCEPTION 'public table ownership drift after 0040: % tables', v_wrong_owner;
    END IF;

    IF to_regclass('public.auth_audit_outbox') IS NULL
       OR (
           to_regprocedure('public.create_invitation(uuid,text,text,text,bigint)') IS NULL
           AND to_regprocedure('public.create_invitation(uuid,text,text,text,bigint,uuid)') IS NULL
       )
       OR (
           to_regprocedure('public.claim_invitation(uuid,uuid,text,text)') IS NULL
           AND to_regprocedure('public.claim_invitation(uuid,uuid,text,text,text)') IS NULL
       ) THEN
        RAISE EXCEPTION '0039/0040 identity objects are incomplete';
    END IF;

    IF NOT has_table_privilege('app', 'public.invitations', 'SELECT')
       OR has_table_privilege('app', 'public.invitations', 'INSERT')
       OR has_table_privilege('app', 'public.invitations', 'UPDATE')
       OR has_table_privilege('app', 'public.invitations', 'DELETE') THEN
        RAISE EXCEPTION
            '0040 invitation ACL drift: app must retain SELECT and lose direct DML';
    END IF;
END
$$;

DO $$
DECLARE
    v_bad_functions bigint;
    v_bad_capabilities bigint;
    v_rls boolean;
    v_force boolean;
BEGIN
    SELECT count(*) INTO v_bad_functions
      FROM pg_catalog.pg_proc AS p
      JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
     WHERE n.nspname = 'public'
       AND p.proname IN (
           'expire_pending_invitations',
           'create_invitation',
           'claim_invitation',
           'bind_redeemed_identity',
           'authenticate_identity_actor',
           'consume_identity_actor_capability'
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
        RAISE EXCEPTION '0040 identity function owner/search_path drift detected: %', v_bad_functions;
    END IF;
    IF NOT EXISTS (
        SELECT 1
          FROM pg_catalog.pg_proc AS p
          JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
         WHERE n.nspname = 'public'
           AND p.proname = 'create_invitation'
           AND has_function_privilege('app', p.oid, 'EXECUTE')
    ) THEN
        RAISE EXCEPTION '0040 app cannot execute create_invitation';
    END IF;
    IF NOT EXISTS (
        SELECT 1
          FROM pg_catalog.pg_proc AS p
          JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
         WHERE n.nspname = 'public'
           AND p.proname = 'claim_invitation'
           AND has_function_privilege('app', p.oid, 'EXECUTE')
    ) THEN
        RAISE EXCEPTION '0040 app cannot execute claim_invitation';
    END IF;

    SELECT c.relrowsecurity, c.relforcerowsecurity
      INTO v_rls, v_force
      FROM pg_catalog.pg_class AS c
      JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
     WHERE n.nspname = 'public' AND c.relname = 'invitations';
    IF v_rls IS DISTINCT FROM true OR v_force IS DISTINCT FROM true THEN
        RAISE EXCEPTION '0040 invitations RLS is not enabled and forced';
    END IF;

    SELECT count(*) INTO v_bad_capabilities
      FROM pg_catalog.pg_class AS c
      JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
     WHERE n.nspname = 'public'
       AND c.relname = 'identity_actor_capabilities'
       AND (
           pg_catalog.pg_get_userbyid(c.relowner) <> 'migration_owner'
           OR c.relrowsecurity IS DISTINCT FROM true
           OR c.relforcerowsecurity IS DISTINCT FROM true
       );
    IF v_bad_capabilities <> 0 THEN
        RAISE EXCEPTION '0040 identity capability table ownership/RLS drift detected';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM pg_catalog.pg_proc AS p
          JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
         WHERE n.nspname = 'public'
           AND p.proname = 'authenticate_identity_actor'
           AND NOT has_function_privilege('app', p.oid, 'EXECUTE')
    ) THEN
        RAISE EXCEPTION '0040 app cannot execute authenticate_identity_actor';
    END IF;
END
$$;

SELECT 'preflight=0040';
SELECT 'sqlx_migrations=' || count(*) FROM public._sqlx_migrations;
SELECT 'normalized_pending_invite_duplicates=' || count(*)
  FROM (
    SELECT lower(btrim(email))
      FROM public.invitations
     WHERE status = 'PENDING'
     GROUP BY lower(btrim(email))
    HAVING count(*) > 1
  ) duplicates;
SELECT 'pending_invite_unique_index=' ||
       CASE WHEN to_regclass('public.invitations_pending_email_uq') IS NULL THEN 'missing' ELSE 'present' END;
SELECT 'app_invitation_acl_select=' || has_table_privilege('app', 'public.invitations', 'SELECT')
       || ',dml=' || (
           has_table_privilege('app', 'public.invitations', 'INSERT')
           OR has_table_privilege('app', 'public.invitations', 'UPDATE')
           OR has_table_privilege('app', 'public.invitations', 'DELETE')
       );
