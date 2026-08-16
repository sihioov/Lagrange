-- Final assertions after sequential 0039, 0040, and 0041.
\set ON_ERROR_STOP on

DO $$
DECLARE
    v_applied bigint;
    v_terminal bigint;
    v_active bigint;
    v_archive bigint;
    v_covered bigint;
    v_wrong_owner bigint;
BEGIN
    SELECT count(*) INTO v_applied FROM public._sqlx_migrations;
    IF v_applied <> 41 THEN
        RAISE EXCEPTION 'migration version count is %, expected 41', v_applied;
    END IF;

    SELECT count(*) INTO v_terminal
      FROM public.pending_targets
     WHERE status <> 'PENDING';
    SELECT count(*) INTO v_active FROM public.paper_settlement_outbox;
    SELECT count(*) INTO v_archive FROM public.paper_settlement_outbox_archive;
    SELECT count(*) INTO v_covered
      FROM (
        SELECT pending_target_id FROM public.paper_settlement_outbox
        UNION
        SELECT pending_target_id FROM public.paper_settlement_outbox_archive
      ) obligations
      JOIN public.pending_targets AS target
        ON target.id = obligations.pending_target_id
       AND target.status <> 'PENDING';
    IF v_terminal <> 1 THEN
        RAISE EXCEPTION 'terminal Paper target count is %, expected 1', v_terminal;
    END IF;
    IF v_covered <> v_terminal THEN
        RAISE EXCEPTION 'terminal Paper target obligation coverage is %, expected %', v_covered, v_terminal;
    END IF;
    IF v_active + v_archive < v_terminal THEN
        RAISE EXCEPTION 'Paper outbox/backfill rows are %, expected at least %', v_active + v_archive, v_terminal;
    END IF;
    IF NOT EXISTS (
        SELECT 1
          FROM public.paper_settlement_outbox
         WHERE pending_target_id = '00000000-0000-4000-8000-000000000394'
           AND owner_user_id = '00000000-0000-4000-8000-000000000039'
    ) THEN
        RAISE EXCEPTION '0041 did not backfill the representative terminal Paper target';
    END IF;

    SELECT count(*) INTO v_wrong_owner
      FROM pg_catalog.pg_class AS c
      JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
     WHERE n.nspname = 'public'
       AND c.relkind IN ('r', 'p')
       AND pg_catalog.pg_get_userbyid(c.relowner) <> 'migration_owner';
    IF v_wrong_owner <> 0 THEN
        RAISE EXCEPTION 'public table ownership drift after 0041: % tables', v_wrong_owner;
    END IF;

    IF to_regclass('public.paper_settlement_outbox') IS NULL
       OR to_regclass('public.paper_settlement_outbox_archive') IS NULL
       OR NOT EXISTS (
           SELECT 1
             FROM pg_catalog.pg_trigger AS trigger_row
             JOIN pg_catalog.pg_class AS table_row
               ON table_row.oid = trigger_row.tgrelid
             JOIN pg_catalog.pg_namespace AS namespace_row
               ON namespace_row.oid = table_row.relnamespace
            WHERE namespace_row.nspname = 'public'
              AND trigger_row.tgname = 'pending_targets_require_settlement_outbox'
              AND NOT trigger_row.tgisinternal
       )
       OR to_regprocedure('public.paper_settlement_outbox_stats(bigint)') IS NULL
       OR to_regprocedure('public.prune_paper_settlement_outbox(bigint,integer)') IS NULL THEN
        RAISE EXCEPTION '0041 Paper settlement outbox objects are incomplete';
    END IF;
    IF NOT EXISTS (
        SELECT 1
          FROM pg_catalog.pg_constraint AS constraint_row
          JOIN pg_catalog.pg_class AS table_row
            ON table_row.oid = constraint_row.conrelid
          JOIN pg_catalog.pg_namespace AS namespace_row
            ON namespace_row.oid = table_row.relnamespace
         WHERE namespace_row.nspname = 'public'
           AND table_row.relname = 'pending_targets'
           AND constraint_row.conname = 'pending_targets_recommendation_exact_lineage_fk'
           AND constraint_row.contype = 'f'
           AND constraint_row.convalidated
    ) THEN
        RAISE EXCEPTION '0041 exact recommendation lineage FK is missing or not validated';
    END IF;
END
$$;

DO $$
DECLARE
    v_bad_functions bigint;
    v_active_rls boolean;
    v_active_force boolean;
    v_archive_rls boolean;
    v_archive_force boolean;
BEGIN
    SELECT count(*) INTO v_bad_functions
      FROM pg_catalog.pg_proc AS p
      JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
     WHERE n.nspname = 'public'
       AND p.proname IN (
           'assert_paper_settlement_obligation',
           'preflight_paper_target',
           'enqueue_paper_settlement_outbox',
           'claim_paper_settlement_outbox',
           'mark_paper_settlement_outbox_delivered',
           'fail_paper_settlement_outbox',
           'paper_settlement_outbox_stats',
           'prune_paper_settlement_outbox'
       )
       AND (
           pg_catalog.pg_get_userbyid(p.proowner) <> 'migration_owner'
           OR NOT EXISTS (
               SELECT 1
                 FROM pg_catalog.unnest(
                     coalesce(p.proconfig, ARRAY[]::text[])
                 ) AS setting
                WHERE setting = 'search_path=pg_catalog, public'
           )
       );
    IF v_bad_functions <> 0 THEN
        RAISE EXCEPTION '0041 Paper function owner/search_path drift detected: %', v_bad_functions;
    END IF;

    IF NOT EXISTS (
        SELECT 1
          FROM pg_catalog.pg_proc AS p
          JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
         WHERE n.nspname = 'public'
           AND p.proname = 'enqueue_paper_settlement_outbox'
           AND has_function_privilege('app', p.oid, 'EXECUTE')
    ) OR NOT EXISTS (
        SELECT 1
          FROM pg_catalog.pg_proc AS p
          JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
         WHERE n.nspname = 'public'
           AND p.proname = 'claim_paper_settlement_outbox'
           AND has_function_privilege('worker', p.oid, 'EXECUTE')
    ) OR NOT EXISTS (
        SELECT 1
          FROM pg_catalog.pg_proc AS p
          JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
         WHERE n.nspname = 'public'
           AND p.proname = 'paper_settlement_outbox_stats'
           AND has_function_privilege('worker', p.oid, 'EXECUTE')
    ) OR NOT EXISTS (
        SELECT 1
          FROM pg_catalog.pg_proc AS p
          JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
         WHERE n.nspname = 'public'
           AND p.proname = 'prune_paper_settlement_outbox'
           AND has_function_privilege('worker', p.oid, 'EXECUTE')
    ) THEN
        RAISE EXCEPTION '0041 Paper function EXECUTE ACL is incomplete';
    END IF;

    SELECT c.relrowsecurity, c.relforcerowsecurity
      INTO v_active_rls, v_active_force
      FROM pg_catalog.pg_class AS c
      JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
     WHERE n.nspname = 'public' AND c.relname = 'paper_settlement_outbox';
    SELECT c.relrowsecurity, c.relforcerowsecurity
      INTO v_archive_rls, v_archive_force
      FROM pg_catalog.pg_class AS c
      JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
     WHERE n.nspname = 'public' AND c.relname = 'paper_settlement_outbox_archive';
    IF v_active_rls IS DISTINCT FROM true OR v_active_force IS DISTINCT FROM true
       OR v_archive_rls IS DISTINCT FROM true OR v_archive_force IS DISTINCT FROM true
    THEN
        RAISE EXCEPTION '0041 Paper outbox RLS is not enabled and forced';
    END IF;
    IF has_table_privilege('app', 'public.paper_settlement_outbox', 'INSERT')
       OR has_table_privilege('worker', 'public.paper_settlement_outbox', 'INSERT')
       OR has_table_privilege('admin', 'public.paper_settlement_outbox', 'INSERT')
    THEN
        RAISE EXCEPTION '0041 serving roles have direct Paper outbox INSERT privilege';
    END IF;
END
$$;

SELECT 'preflight=0041';
SELECT 'sqlx_migrations=' || count(*) FROM public._sqlx_migrations;
SELECT 'terminal_paper_targets=' || count(*)
  FROM public.pending_targets WHERE status <> 'PENDING';
SELECT 'paper_settlement_outbox=' || count(*) FROM public.paper_settlement_outbox;
SELECT 'paper_settlement_outbox_archive=' || count(*) FROM public.paper_settlement_outbox_archive;
SELECT 'paper_settlement_obligation_coverage=' || count(*)
  FROM (
    SELECT pending_target_id FROM public.paper_settlement_outbox
    UNION
    SELECT pending_target_id FROM public.paper_settlement_outbox_archive
  ) obligations;
