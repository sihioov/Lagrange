-- Fail-closed contract for the research metadata publication service.
-- Run only as the deployment administrator; this file never mutates schema.

DO $schema$
DECLARE
  research_role oid;
  actual_definition text;
BEGIN
  IF to_regclass('public._sqlx_migrations') IS NULL
     OR (SELECT count(*) FROM _sqlx_migrations
         WHERE version IN (22, 23, 24, 25, 33, 34) AND success) <> 6 THEN
    RAISE EXCEPTION 'successful SQLx migrations 22-25 and 33-34 are required';
  END IF;

  IF to_regclass('public.data_batches') IS NULL
     OR to_regclass('public.trading_calendar_versions') IS NULL
     OR to_regclass('public.trading_calendars') IS NULL THEN
    RAISE EXCEPTION 'research publication tables are missing';
  END IF;

  IF EXISTS (
    WITH actual(table_name, constraint_name, constraint_type, validated, definition) AS (
      SELECT table_row.relname, constraint_row.conname,
             constraint_row.contype::text, constraint_row.convalidated,
             pg_get_constraintdef(constraint_row.oid, true)
      FROM pg_constraint constraint_row
      JOIN pg_class table_row ON table_row.oid = constraint_row.conrelid
      JOIN pg_namespace table_schema ON table_schema.oid = table_row.relnamespace
      WHERE table_schema.nspname = 'public'
        AND table_row.relname IN (
          'data_batches', 'trading_calendars', 'trading_calendar_versions'
        )
        AND constraint_row.contype IN ('p', 'u', 'c')
    ), expected(table_name, constraint_name, constraint_type, validated, definition) AS (VALUES
      ('data_batches', 'data_batches_pkey', 'p', true, 'PRIMARY KEY (id)'),
      ('data_batches', 'data_batches_content_sha256_check', 'c', true, 'CHECK (content_sha256 ~ ''^[0-9a-f]{64}$''::text)'),
      ('data_batches', 'data_batches_bytes_positive_check', 'c', true, 'CHECK (bytes_size >= 0)'),
      ('data_batches', 'data_batches_fetch_mode_check', 'c', true, 'CHECK (fetch_mode IS NULL OR (fetch_mode = ANY (ARRAY[''synthetic''::text, ''credentialed''::text])))'),
      ('data_batches', 'data_batches_provenance_all_or_none_check', 'c', true, 'CHECK (source_batch_id IS NULL AND source_file_name IS NULL AND fetch_mode IS NULL OR source_batch_id IS NOT NULL AND source_file_name IS NOT NULL AND fetch_mode IS NOT NULL)'),
      ('trading_calendars', 'trading_calendars_pkey', 'p', true, 'PRIMARY KEY (id)'),
      ('trading_calendars', 'trading_calendars_exchange_date_key', 'u', true, 'UNIQUE (exchange, session_date)'),
      ('trading_calendars', 'trading_calendars_content_sha256_check', 'c', true, 'CHECK (content_sha256 IS NULL OR content_sha256 ~ ''^[0-9a-f]{64}$''::text)'),
      ('trading_calendars', 'trading_calendars_provenance_all_or_none_check', 'c', true, 'CHECK (source_batch_id IS NULL AND content_sha256 IS NULL AND retrieved_at IS NULL OR source_batch_id IS NOT NULL AND content_sha256 IS NOT NULL AND retrieved_at IS NOT NULL)'),
      ('trading_calendar_versions', 'trading_calendar_versions_pkey', 'p', true, 'PRIMARY KEY (id)'),
      ('trading_calendar_versions', 'trading_calendar_versions_session_type_check', 'c', true, 'CHECK (session_type = ANY (ARRAY[''TRADING''::text, ''CLOSED''::text]))'),
      ('trading_calendar_versions', 'trading_calendar_versions_timezone_check', 'c', true, 'CHECK (timezone = ''Asia/Seoul''::text)'),
      ('trading_calendar_versions', 'trading_calendar_versions_content_sha256_check', 'c', true, 'CHECK (content_sha256 ~ ''^[0-9a-f]{64}$''::text)'),
      ('trading_calendar_versions', 'trading_calendar_versions_exchange_date_source_version_key', 'u', true, 'UNIQUE (exchange, session_date, source_version)')
    )
    (SELECT * FROM actual EXCEPT SELECT * FROM expected)
    UNION ALL
    (SELECT * FROM expected EXCEPT SELECT * FROM actual)
  ) THEN
    RAISE EXCEPTION 'research publication constraints are missing, unvalidated, or drifted';
  END IF;

  IF EXISTS (
    WITH actual(table_name, column_name, data_type, not_null, identity_kind, default_expression) AS (
      SELECT table_row.relname, column_row.attname,
             format_type(column_row.atttypid, column_row.atttypmod),
             column_row.attnotnull, column_row.attidentity::text,
             COALESCE(pg_get_expr(default_row.adbin, default_row.adrelid), '')
      FROM pg_attribute column_row
      JOIN pg_class table_row ON table_row.oid = column_row.attrelid
      JOIN pg_namespace table_schema ON table_schema.oid = table_row.relnamespace
      LEFT JOIN pg_attrdef default_row
        ON default_row.adrelid = column_row.attrelid
       AND default_row.adnum = column_row.attnum
      WHERE table_schema.nspname = 'public'
        AND table_row.relname IN (
          'data_batches', 'trading_calendars', 'trading_calendar_versions'
        )
        AND column_row.attnum > 0
        AND NOT column_row.attisdropped
    ), expected(table_name, column_name, data_type, not_null, identity_kind, default_expression) AS (VALUES
      ('data_batches', 'id', 'uuid', true, '', 'gen_random_uuid()'),
      ('data_batches', 'provider', 'text', true, '', ''),
      ('data_batches', 'market', 'text', true, '', ''),
      ('data_batches', 'batch_date', 'date', true, '', ''),
      ('data_batches', 'kind', 'text', true, '', ''),
      ('data_batches', 'storage_path', 'text', true, '', ''),
      ('data_batches', 'content_sha256', 'text', true, '', ''),
      ('data_batches', 'bytes_size', 'bigint', true, '', ''),
      ('data_batches', 'retrieved_at', 'timestamp with time zone', true, '', ''),
      ('data_batches', 'created_at', 'timestamp with time zone', true, '', 'now()'),
      ('data_batches', 'source_batch_id', 'uuid', false, '', ''),
      ('data_batches', 'source_file_name', 'text', false, '', ''),
      ('data_batches', 'fetch_mode', 'text', false, '', ''),
      ('trading_calendars', 'id', 'uuid', true, '', 'gen_random_uuid()'),
      ('trading_calendars', 'exchange', 'text', true, '', ''),
      ('trading_calendars', 'session_date', 'date', true, '', ''),
      ('trading_calendars', 'session_type', 'text', true, '', ''),
      ('trading_calendars', 'timezone', 'text', true, '', '''Asia/Seoul''::text'),
      ('trading_calendars', 'source', 'text', true, '', ''),
      ('trading_calendars', 'source_version', 'text', true, '', ''),
      ('trading_calendars', 'created_at', 'timestamp with time zone', true, '', 'now()'),
      ('trading_calendars', 'source_batch_id', 'uuid', false, '', ''),
      ('trading_calendars', 'content_sha256', 'text', false, '', ''),
      ('trading_calendars', 'retrieved_at', 'timestamp with time zone', false, '', ''),
      ('trading_calendar_versions', 'id', 'bigint', true, 'a', ''),
      ('trading_calendar_versions', 'exchange', 'text', true, '', ''),
      ('trading_calendar_versions', 'session_date', 'date', true, '', ''),
      ('trading_calendar_versions', 'session_type', 'text', true, '', ''),
      ('trading_calendar_versions', 'timezone', 'text', true, '', ''),
      ('trading_calendar_versions', 'source', 'text', true, '', ''),
      ('trading_calendar_versions', 'source_version', 'text', true, '', ''),
      ('trading_calendar_versions', 'source_batch_id', 'uuid', true, '', ''),
      ('trading_calendar_versions', 'content_sha256', 'text', true, '', ''),
      ('trading_calendar_versions', 'retrieved_at', 'timestamp with time zone', true, '', ''),
      ('trading_calendar_versions', 'created_at', 'timestamp with time zone', true, '', 'now()')
    )
    (SELECT * FROM actual EXCEPT SELECT * FROM expected)
    UNION ALL
    (SELECT * FROM expected EXCEPT SELECT * FROM actual)
  ) THEN
    RAISE EXCEPTION 'research publication column contract is missing or drifted';
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
  ) THEN
    RAISE EXCEPTION 'append-only trigger/function is missing, disabled, or drifted';
  END IF;

  IF EXISTS (
    WITH actual_function(definition) AS (
      SELECT btrim(regexp_replace(pg_get_functiondef(f.oid), E'\\s+', ' ', 'g'))
      FROM pg_proc f
      JOIN pg_namespace n ON n.oid = f.pronamespace
      WHERE n.nspname = 'public'
        AND f.proname = 'trading_calendar_versions_reject_mutation'
    ), expected_function(definition) AS (VALUES
      ('CREATE OR REPLACE FUNCTION public.trading_calendar_versions_reject_mutation() RETURNS trigger LANGUAGE plpgsql AS $function$ BEGIN RAISE EXCEPTION ''trading_calendar_versions is append-only: % is refused'', TG_OP USING ERRCODE = ''55000''; END $function$')
    )
    SELECT 1 FROM (
      (SELECT definition FROM actual_function EXCEPT SELECT definition FROM expected_function)
      UNION ALL
      (SELECT definition FROM expected_function EXCEPT SELECT definition FROM actual_function)
    ) function_drift
  ) THEN
    RAISE EXCEPTION 'append-only trigger function definition is missing or drifted';
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
