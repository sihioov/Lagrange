-- Fail-closed contract for the research metadata publication service.
-- Run only as the deployment administrator; this file never mutates schema.

DO $schema$
DECLARE
  research_role oid;
  actual_definition text;
BEGIN
  IF to_regclass('public._sqlx_migrations') IS NULL
     OR (SELECT max(version) FROM _sqlx_migrations) <> 25
     OR (SELECT count(*) FROM _sqlx_migrations
         WHERE version BETWEEN 22 AND 25 AND success) <> 4 THEN
    RAISE EXCEPTION 'successful SQLx migrations 22-25 are required and 25 must be latest';
  END IF;

  IF to_regclass('public.data_batches') IS NULL
     OR to_regclass('public.trading_calendar_versions') IS NULL
     OR to_regclass('public.trading_calendars') IS NULL THEN
    RAISE EXCEPTION 'research publication tables are missing';
  END IF;

  IF (SELECT count(*)
      FROM pg_constraint constraint_row
      JOIN pg_class table_row ON table_row.oid = constraint_row.conrelid
      JOIN pg_namespace table_schema ON table_schema.oid = table_row.relnamespace
      JOIN (VALUES
        ('data_batches', 'data_batches_pkey', 'p'),
        ('data_batches', 'data_batches_content_sha256_check', 'c'),
        ('data_batches', 'data_batches_bytes_positive_check', 'c'),
        ('data_batches', 'data_batches_fetch_mode_check', 'c'),
        ('data_batches', 'data_batches_provenance_all_or_none_check', 'c'),
        ('trading_calendars', 'trading_calendars_pkey', 'p'),
        ('trading_calendars', 'trading_calendars_exchange_date_key', 'u'),
        ('trading_calendars', 'trading_calendars_content_sha256_check', 'c'),
        ('trading_calendars', 'trading_calendars_provenance_all_or_none_check', 'c'),
        ('trading_calendar_versions', 'trading_calendar_versions_pkey', 'p'),
        ('trading_calendar_versions', 'trading_calendar_versions_session_type_check', 'c'),
        ('trading_calendar_versions', 'trading_calendar_versions_timezone_check', 'c'),
        ('trading_calendar_versions', 'trading_calendar_versions_content_sha256_check', 'c'),
        ('trading_calendar_versions', 'trading_calendar_versions_exchange_date_source_version_key', 'u')
      ) expected(table_name, constraint_name, constraint_type)
        ON expected.table_name = table_row.relname
       AND expected.constraint_name = constraint_row.conname
       AND expected.constraint_type = constraint_row.contype::text
      WHERE table_schema.nspname = 'public' AND constraint_row.convalidated) <> 14
     OR EXISTS (
       SELECT 1 FROM pg_constraint
       WHERE conrelid IN (
         'public.data_batches'::regclass,
         'public.trading_calendar_versions'::regclass,
         'public.trading_calendars'::regclass
       ) AND NOT convalidated
     ) THEN
    RAISE EXCEPTION 'research publication constraints are missing or unvalidated';
  END IF;

  SELECT pg_get_indexdef(indexrelid) INTO actual_definition
  FROM pg_index
  WHERE indexrelid = to_regclass('public.data_batches_source_file_uq')
    AND indrelid = 'public.data_batches'::regclass
    AND indisunique AND indisvalid AND indisready AND indislive;
  IF actual_definition IS DISTINCT FROM
     'CREATE UNIQUE INDEX data_batches_source_file_uq ON public.data_batches USING btree (provider, market, source_batch_id, source_file_name) WHERE (source_batch_id IS NOT NULL)' THEN
    RAISE EXCEPTION 'data_batches_source_file_uq is missing, invalid, or drifted';
  END IF;

  SELECT pg_get_indexdef(indexrelid) INTO actual_definition
  FROM pg_index
  WHERE indexrelid = to_regclass('public.trading_calendar_versions_source_lookup_idx')
    AND indrelid = 'public.trading_calendar_versions'::regclass
    AND NOT indisunique AND indisvalid AND indisready AND indislive;
  IF actual_definition IS DISTINCT FROM
     'CREATE INDEX trading_calendar_versions_source_lookup_idx ON public.trading_calendar_versions USING btree (exchange, source_version) INCLUDE (source, timezone, content_sha256)' THEN
    RAISE EXCEPTION 'trading_calendar_versions_source_lookup_idx is missing, invalid, or drifted';
  END IF;

  IF EXISTS (
    SELECT 1 FROM pg_class
    WHERE oid IN (
      'public.data_batches'::regclass,
      'public.trading_calendar_versions'::regclass,
      'public.trading_calendars'::regclass
    ) AND NOT relrowsecurity
  ) THEN
    RAISE EXCEPTION 'research publication RLS is not enabled';
  END IF;

  SELECT oid INTO research_role FROM pg_roles
  WHERE rolname = 'research_writer'
    AND rolcanlogin
    AND NOT rolsuper
    AND NOT rolbypassrls
    AND NOT rolcreatedb
    AND NOT rolcreaterole
    AND NOT rolreplication;
  IF research_role IS NULL THEN
    RAISE EXCEPTION 'research_writer role attributes are unsafe';
  END IF;
  IF EXISTS (
    SELECT 1 FROM pg_auth_members
    WHERE member = research_role OR roleid = research_role
  ) THEN
    RAISE EXCEPTION 'research_writer has unexpected role memberships';
  END IF;

  IF (SELECT count(*) FROM pg_policy WHERE research_role = ANY(polroles)) <> 7
     OR (SELECT count(*)
         FROM pg_policy p
         JOIN pg_class c ON c.oid = p.polrelid
         JOIN (VALUES
           ('data_batches', 'data_batches_select_research_writer', 'r', true, false),
           ('data_batches', 'data_batches_insert_research_writer', 'a', false, true),
           ('trading_calendars', 'trading_calendars_select_research_writer', 'r', true, false),
           ('trading_calendars', 'trading_calendars_insert_research_writer', 'a', false, true),
           ('trading_calendars', 'trading_calendars_update_research_writer', 'w', true, true),
           ('trading_calendar_versions', 'trading_calendar_versions_select_research_writer', 'r', true, false),
           ('trading_calendar_versions', 'trading_calendar_versions_insert_research_writer', 'a', false, true)
         ) expected(table_name, policy_name, command, needs_qual, needs_check)
           ON expected.table_name = c.relname
          AND expected.policy_name = p.polname
          AND expected.command = p.polcmd::text
         WHERE p.polpermissive
           AND p.polroles = ARRAY[research_role]::oid[]
           AND (NOT expected.needs_qual OR pg_get_expr(p.polqual, p.polrelid) = 'true')
           AND (expected.needs_qual OR p.polqual IS NULL)
           AND (NOT expected.needs_check OR pg_get_expr(p.polwithcheck, p.polrelid) = 'true')
           AND (expected.needs_check OR p.polwithcheck IS NULL)) <> 7 THEN
    RAISE EXCEPTION 'required research_writer RLS policies are missing or drifted';
  END IF;

  IF NOT EXISTS (
    SELECT 1
    FROM pg_trigger t
    JOIN pg_proc f ON f.oid = t.tgfoid
    JOIN pg_language l ON l.oid = f.prolang
    WHERE t.tgrelid = 'public.trading_calendar_versions'::regclass
      AND t.tgname = 'trading_calendar_versions_append_only'
      AND NOT t.tgisinternal
      AND t.tgenabled = 'O'
      AND t.tgtype = 27
      AND f.proname = 'trading_calendar_versions_reject_mutation'
      AND f.prorettype = 'trigger'::regtype
      AND NOT f.prosecdef
      AND l.lanname = 'plpgsql'
      AND f.prosrc LIKE '%trading_calendar_versions is append-only%'
  ) THEN
    RAISE EXCEPTION 'append-only trigger/function is missing, disabled, or drifted';
  END IF;

  IF EXISTS (
    WITH actual AS (
      SELECT table_name, privilege_type
      FROM information_schema.role_table_grants
      WHERE grantee = 'research_writer' AND table_schema = 'public'
    ), expected(table_name, privilege_type) AS (VALUES
      ('data_batches', 'SELECT'),
      ('data_batches', 'INSERT'),
      ('trading_calendar_versions', 'SELECT'),
      ('trading_calendar_versions', 'INSERT'),
      ('trading_calendars', 'SELECT'),
      ('trading_calendars', 'INSERT'),
      ('trading_calendars', 'UPDATE')
    )
    (SELECT * FROM actual EXCEPT SELECT * FROM expected)
    UNION ALL
    (SELECT * FROM expected EXCEPT SELECT * FROM actual)
  ) THEN
    RAISE EXCEPTION 'research_writer direct table grants are not exact';
  END IF;

  IF NOT has_schema_privilege('research_writer', 'public', 'USAGE')
     OR has_schema_privilege('research_writer', 'public', 'CREATE')
     OR has_table_privilege('research_writer', 'public.data_batches', 'UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
     OR has_table_privilege('research_writer', 'public.trading_calendar_versions', 'UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
     OR has_table_privilege('research_writer', 'public.trading_calendars', 'DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
     OR EXISTS (
       SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
       WHERE n.nspname = 'public' AND c.relkind IN ('r','p','v','m','f')
         AND c.relname NOT IN ('data_batches','trading_calendar_versions','trading_calendars')
         AND has_table_privilege(
           'research_writer', c.oid,
           'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN'
         )
     ) THEN
    RAISE EXCEPTION 'research_writer has forbidden tenant/order/audit/job or metadata privileges';
  END IF;

  IF EXISTS (
    WITH public_sequences AS MATERIALIZED (
      SELECT c.oid
      FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE n.nspname = 'public' AND c.relkind = 'S'
    )
    SELECT 1 FROM public_sequences s
    WHERE has_sequence_privilege('research_writer', s.oid, 'USAGE,SELECT,UPDATE')
  ) THEN
    RAISE EXCEPTION 'research_writer must not have identity-sequence usage';
  END IF;
END
$schema$;
