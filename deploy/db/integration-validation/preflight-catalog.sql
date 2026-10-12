-- Assertions for the commit-pinned baseline strategy catalog and its audit.
-- Run as migration_owner with psql variables derived from baseline-v1.json.
\set ON_ERROR_STOP on

SELECT pg_catalog.set_config(
    'lagrange.validation.catalog_json', :'catalog_json', false
) AS validation_setting \gset
SELECT pg_catalog.set_config(
    'lagrange.validation.catalog_sha256', :'catalog_sha256', false
) AS validation_setting \gset
SELECT pg_catalog.set_config(
    'lagrange.validation.expected_lane', :'expected_lane', false
) AS validation_setting \gset

\if :probe_unexpected_strategy
BEGIN;
INSERT INTO public.strategies (id, display_name, state)
VALUES ('postgres_validation_unexpected', 'Unexpected validation row', 'Draft');
\set ON_ERROR_STOP off
\endif

\if :probe_missing_baseline
BEGIN;
DELETE FROM public.strategies WHERE id = 'buy_and_hold';
\set ON_ERROR_STOP off
\endif

\if :probe_mutated_baseline
BEGIN;
UPDATE public.strategies
   SET display_name = display_name || ' synthetic baseline mutation'
 WHERE id = 'buy_and_hold';
\set ON_ERROR_STOP off
\endif

\if :probe_full_paper
BEGIN;
INSERT INTO public.strategies (id, display_name, state)
VALUES ('postgres_validation_paper', 'PostgreSQL validation Paper', 'Paper');
\set ON_ERROR_STOP off
\endif

DO $catalog_preflight$
DECLARE
    v_manifest jsonb := pg_catalog.current_setting(
        'lagrange.validation.catalog_json'
    )::jsonb;
    v_sha256 text := pg_catalog.current_setting(
        'lagrange.validation.catalog_sha256'
    );
    v_manifest_ids text[];
    v_expected_lane text := pg_catalog.current_setting(
        'lagrange.validation.expected_lane'
    );
    v_expected_strategy_count bigint;
    v_bad_rows bigint;
    v_table_count bigint;
    v_wrong_owner bigint;
    v_function oid;
    v_function_owner text;
    v_function_config text[];
    v_rls boolean;
    v_force boolean;
    v_event_key text;
    v_event_count bigint;
    v_event public.auth_audit_outbox%ROWTYPE;
BEGIN
    IF current_user IS DISTINCT FROM 'migration_owner'
       OR session_user IS DISTINCT FROM 'migration_owner' THEN
        RAISE EXCEPTION 'catalog preflight did not connect as migration_owner';
    END IF;
    IF v_sha256 !~ '^[0-9a-f]{64}$'
       OR pg_catalog.jsonb_typeof(v_manifest) IS DISTINCT FROM 'object'
       OR v_manifest->>'schema_id' IS DISTINCT FROM 'lagrange-baseline-strategy-catalog'
       OR (v_manifest->>'schema_version')::integer IS DISTINCT FROM 1
       OR (v_manifest->>'released_at_epoch')::bigint IS DISTINCT FROM 1787670000
       OR pg_catalog.jsonb_typeof(v_manifest->'strategies') IS DISTINCT FROM 'array'
       OR pg_catalog.jsonb_array_length(v_manifest->'strategies') IS DISTINCT FROM 5 THEN
        RAISE EXCEPTION 'baseline catalog safe input envelope is invalid';
    END IF;

    SELECT pg_catalog.array_agg(entry->>'strategy_id' ORDER BY entry->>'strategy_id')
      INTO v_manifest_ids
      FROM pg_catalog.jsonb_array_elements(v_manifest->'strategies') AS records(entry);
    IF v_manifest_ids IS DISTINCT FROM ARRAY[
        'buy_and_hold',
        'dual_momentum',
        'inverse_volatility',
        'relative_momentum',
        'trend_following'
    ]::text[] THEN
        RAISE EXCEPTION 'baseline catalog does not contain the five exact expected IDs';
    END IF;

    IF v_expected_lane NOT IN ('staged', 'full') THEN
        RAISE EXCEPTION 'catalog preflight lane must be staged or full';
    END IF;
    v_expected_strategy_count := CASE v_expected_lane
        WHEN 'staged' THEN 6
        ELSE 5
    END;

    IF v_expected_lane = 'full' THEN
        SELECT pg_catalog.count(*) INTO v_bad_rows
          FROM public.strategies AS strategy
         WHERE strategy.id = 'postgres_validation_paper'
            OR strategy.state = 'Paper';
        IF v_bad_rows <> 0 THEN
            RAISE EXCEPTION USING
                ERRCODE = '23514',
                MESSAGE = 'catalog_preflight: full_lane_paper_fixture_present';
        END IF;
    END IF;

    SELECT pg_catalog.count(*) INTO v_bad_rows
      FROM public.strategies AS strategy
     WHERE NOT (strategy.id = ANY(v_manifest_ids))
       AND NOT (
           v_expected_lane = 'staged'
           AND strategy.id = 'postgres_validation_paper'
       );
    IF v_bad_rows <> 0 THEN
        RAISE EXCEPTION USING
            ERRCODE = '23514',
            MESSAGE = 'catalog_preflight: unexpected_strategy_id';
    END IF;

    SELECT pg_catalog.count(*) INTO v_bad_rows
      FROM pg_catalog.jsonb_array_elements(v_manifest->'strategies') AS records(entry)
      LEFT JOIN public.strategies AS strategy
        ON strategy.id = records.entry->>'strategy_id'
     WHERE strategy.id IS NULL;
    IF v_bad_rows <> 0 THEN
        RAISE EXCEPTION USING
            ERRCODE = '23514',
            MESSAGE = 'catalog_preflight: missing_baseline_strategy';
    END IF;

    SELECT pg_catalog.count(*) INTO v_table_count
      FROM public.strategies;
    IF v_table_count <> v_expected_strategy_count THEN
        RAISE EXCEPTION USING
            ERRCODE = '23514',
            MESSAGE = 'catalog_preflight: wrong_strategy_count';
    END IF;

    SELECT pg_catalog.count(*) INTO v_bad_rows
      FROM pg_catalog.jsonb_array_elements(v_manifest->'strategies') AS records(entry)
      LEFT JOIN public.strategies AS strategy
        ON strategy.id = records.entry->>'strategy_id'
     WHERE strategy.id IS NULL
        OR strategy.display_name IS DISTINCT FROM records.entry->>'display_name'
        OR strategy.description IS DISTINCT FROM records.entry->>'description'
        OR strategy.risk_description IS DISTINCT FROM records.entry->>'risk_description'
        OR strategy.state IS DISTINCT FROM 'Draft';
    IF v_bad_rows <> 0 THEN
        RAISE EXCEPTION USING
            ERRCODE = '23514',
            MESSAGE = 'catalog_preflight: baseline_strategy_content_mismatch';
    END IF;

    IF v_expected_lane = 'staged' THEN
        SELECT pg_catalog.count(*) INTO v_bad_rows
          FROM public.strategies AS strategy
         WHERE strategy.id = 'postgres_validation_paper'
           AND strategy.display_name = 'PostgreSQL validation Paper'
           AND strategy.state = 'Paper';
        IF v_bad_rows <> 1 THEN
            RAISE EXCEPTION USING
                ERRCODE = '23514',
                MESSAGE = 'catalog_preflight: staged_paper_fixture_mismatch';
        END IF;
    ELSE
        SELECT pg_catalog.count(*) INTO v_bad_rows
          FROM public.strategies AS strategy
         WHERE strategy.id = 'postgres_validation_paper'
            OR strategy.state = 'Paper';
        IF v_bad_rows <> 0 THEN
            RAISE EXCEPTION USING
                ERRCODE = '23514',
                MESSAGE = 'catalog_preflight: full_lane_paper_fixture_present';
        END IF;
    END IF;

    SELECT pg_catalog.count(*) INTO v_table_count
      FROM public.strategy_versions;
    IF v_table_count <> 5 THEN
        RAISE EXCEPTION 'catalog version row count is %, expected 5', v_table_count;
    END IF;
    SELECT pg_catalog.count(*) INTO v_bad_rows
      FROM pg_catalog.jsonb_array_elements(v_manifest->'strategies') AS records(entry)
      LEFT JOIN public.strategy_versions AS version_row
        ON version_row.strategy_id = records.entry->>'strategy_id'
       AND version_row.version = records.entry->>'version'
     WHERE version_row.strategy_id IS NULL
        OR records.entry->>'version' IS DISTINCT FROM '1.0.0'
        OR version_row.required_factors IS DISTINCT FROM records.entry->'required_factors'
        OR version_row.min_lookback IS DISTINCT FROM (records.entry->>'min_lookback')::integer
        OR version_row.supported_market IS DISTINCT FROM records.entry->>'supported_market'
        OR version_row.cadence IS DISTINCT FROM records.entry->>'cadence';
    IF v_bad_rows <> 0 THEN
        RAISE EXCEPTION 'baseline strategy version mismatch: % rows', v_bad_rows;
    END IF;

    SELECT pg_catalog.count(*) INTO v_table_count
      FROM public.strategy_parameter_schemas;
    IF v_table_count <> 5 THEN
        RAISE EXCEPTION 'catalog parameter schema row count is %, expected 5', v_table_count;
    END IF;
    SELECT pg_catalog.count(*) INTO v_bad_rows
      FROM pg_catalog.jsonb_array_elements(v_manifest->'strategies') AS records(entry)
      LEFT JOIN public.strategy_parameter_schemas AS schema_row
        ON schema_row.strategy_id = records.entry->>'strategy_id'
       AND schema_row.version = records.entry->>'version'
     WHERE schema_row.strategy_id IS NULL
        OR schema_row.schema_json IS DISTINCT FROM records.entry->'parameter_schema';
    IF v_bad_rows <> 0 THEN
        RAISE EXCEPTION 'baseline parameter schema mismatch: % rows', v_bad_rows;
    END IF;

    SELECT pg_catalog.count(*) INTO v_wrong_owner
      FROM pg_catalog.pg_class AS relation
      JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace
     WHERE namespace.nspname = 'public'
       AND relation.relkind IN ('r', 'p')
       AND pg_catalog.pg_get_userbyid(relation.relowner) <> 'migration_owner';
    IF v_wrong_owner <> 0 THEN
        RAISE EXCEPTION 'public table ownership drift: % tables', v_wrong_owner;
    END IF;

    v_function := pg_catalog.to_regprocedure(
        'public.enqueue_auth_audit(text,text,uuid,text,text,text,bigint)'
    );
    IF v_function IS NULL THEN
        RAISE EXCEPTION 'seven-argument enqueue_auth_audit identity is missing';
    END IF;
    SELECT pg_catalog.pg_get_userbyid(proc.proowner), proc.proconfig
      INTO v_function_owner, v_function_config
      FROM pg_catalog.pg_proc AS proc
     WHERE proc.oid = v_function;
    IF v_function_owner IS DISTINCT FROM 'migration_owner'
       OR NOT ('search_path=pg_catalog, pg_temp' = ANY(
           coalesce(v_function_config, ARRAY[]::text[])
       )) THEN
        RAISE EXCEPTION 'enqueue_auth_audit owner or safe search_path drifted';
    END IF;
    IF NOT pg_catalog.has_function_privilege('app', v_function, 'EXECUTE')
       OR NOT pg_catalog.has_function_privilege('audit_writer', v_function, 'EXECUTE')
       OR pg_catalog.has_function_privilege('worker', v_function, 'EXECUTE')
       OR pg_catalog.has_function_privilege('admin', v_function, 'EXECUTE')
       OR pg_catalog.has_function_privilege('research_writer', v_function, 'EXECUTE') THEN
        RAISE EXCEPTION 'enqueue_auth_audit EXECUTE ACL drifted';
    END IF;

    SELECT relation.relrowsecurity, relation.relforcerowsecurity
      INTO v_rls, v_force
      FROM pg_catalog.pg_class AS relation
      JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace
     WHERE namespace.nspname = 'public'
       AND relation.relname = 'auth_audit_outbox';
    IF v_rls IS DISTINCT FROM true OR v_force IS DISTINCT FROM true THEN
        RAISE EXCEPTION 'auth_audit_outbox RLS is not enabled and forced';
    END IF;

    v_event_key := 'strategy-catalog:' || v_sha256 || ':installed';
    SELECT pg_catalog.count(*) INTO v_event_count
      FROM public.auth_audit_outbox
     WHERE event_key LIKE 'strategy-catalog:%:installed';
    IF v_event_count <> 1 THEN
        RAISE EXCEPTION 'catalog install audit row count is %, expected 1', v_event_count;
    END IF;
    SELECT * INTO v_event
      FROM public.auth_audit_outbox
     WHERE event_key = v_event_key;
    IF NOT FOUND
       OR v_event.action IS DISTINCT FROM 'strategy_catalog.installed'
       OR v_event.actor_role IS DISTINCT FROM 'system'
       OR v_event.actor_user_id IS NOT NULL
       OR v_event.target_type IS DISTINCT FROM 'strategy_catalog'
       OR v_event.target_id IS DISTINCT FROM v_sha256
       OR v_event.reason IS DISTINCT FROM 'IMMUTABLE_RELEASE_BASELINE'
       OR v_event.created_at IS DISTINCT FROM pg_catalog.to_timestamp(
           (v_manifest->>'released_at_epoch')::double precision
       )
       OR v_event.delivered_at IS NOT NULL THEN
        RAISE EXCEPTION 'catalog install audit payload does not match the immutable baseline';
    END IF;
END
$catalog_preflight$;

\if :probe_unexpected_strategy
ROLLBACK;
\set ON_ERROR_STOP on
\echo CATALOG_NEGATIVE_PROBE_ROLLED_BACK:unexpected_strategy
\endif

\if :probe_missing_baseline
ROLLBACK;
\set ON_ERROR_STOP on
\echo CATALOG_NEGATIVE_PROBE_ROLLED_BACK:missing_baseline
\endif

\if :probe_mutated_baseline
ROLLBACK;
\set ON_ERROR_STOP on
\echo CATALOG_NEGATIVE_PROBE_ROLLED_BACK:mutated_baseline
\endif

\if :probe_full_paper
ROLLBACK;
\set ON_ERROR_STOP on
\echo CATALOG_NEGATIVE_PROBE_ROLLED_BACK:full_paper
\endif

SELECT 'catalog_role=' || current_user || ':' || session_user;
SELECT 'catalog_public_table_ownership=' ||
       (pg_catalog.count(*) FILTER (
           WHERE pg_catalog.pg_get_userbyid(relation.relowner) = 'migration_owner'
       ))::text || '/' || pg_catalog.count(*)::text
  FROM pg_catalog.pg_class AS relation
  JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace
 WHERE namespace.nspname = 'public'
   AND relation.relkind IN ('r', 'p');
SELECT 'catalog_function_identity=' || pg_catalog.pg_get_function_identity_arguments(proc.oid)
  FROM pg_catalog.pg_proc AS proc
 WHERE proc.oid = pg_catalog.to_regprocedure(
     'public.enqueue_auth_audit(text,text,uuid,text,text,text,bigint)'
 );
SELECT 'catalog_function_owner=' || pg_catalog.pg_get_userbyid(proc.proowner)
       || ';search_path=' || coalesce(pg_catalog.array_to_string(proc.proconfig, ','), 'none')
  FROM pg_catalog.pg_proc AS proc
 WHERE proc.oid = pg_catalog.to_regprocedure(
     'public.enqueue_auth_audit(text,text,uuid,text,text,text,bigint)'
 );
SELECT 'catalog_function_execute_acl=app:'
       || pg_catalog.has_function_privilege('app', proc.oid, 'EXECUTE')::text
       || ',audit_writer:'
       || pg_catalog.has_function_privilege('audit_writer', proc.oid, 'EXECUTE')::text
       || ',worker:'
       || pg_catalog.has_function_privilege('worker', proc.oid, 'EXECUTE')::text
       || ',admin:'
       || pg_catalog.has_function_privilege('admin', proc.oid, 'EXECUTE')::text
       || ',research_writer:'
       || pg_catalog.has_function_privilege('research_writer', proc.oid, 'EXECUTE')::text
  FROM pg_catalog.pg_proc AS proc
 WHERE proc.oid = pg_catalog.to_regprocedure(
     'public.enqueue_auth_audit(text,text,uuid,text,text,text,bigint)'
 );
SELECT 'catalog_auth_audit_outbox_rls=' || relation.relrowsecurity::text
       || ':' || relation.relforcerowsecurity::text
  FROM pg_catalog.pg_class AS relation
  JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace
 WHERE namespace.nspname = 'public'
   AND relation.relname = 'auth_audit_outbox';
SELECT 'catalog_fingerprint=' || pg_catalog.md5(coalesce(
    pg_catalog.string_agg(
        strategy.id || '|' || strategy.state || '|' || version_row.version || '|'
        || version_row.required_factors::text || '|' || version_row.min_lookback::text || '|'
        || version_row.supported_market || '|' || version_row.cadence || '|'
        || schema_row.schema_json::text,
        E'\n' ORDER BY strategy.id
    ),
    ''
))
  FROM public.strategies AS strategy
  JOIN public.strategy_versions AS version_row
    ON version_row.strategy_id = strategy.id
  JOIN public.strategy_parameter_schemas AS schema_row
    ON schema_row.strategy_id = strategy.id
   AND schema_row.version = version_row.version;
SELECT 'catalog_audit_id=' || id::text
  FROM public.auth_audit_outbox
 WHERE event_key = 'strategy-catalog:' || :'catalog_sha256' || ':installed';
SELECT 'catalog_audit_count=' || pg_catalog.count(*)
  FROM public.auth_audit_outbox
 WHERE event_key LIKE 'strategy-catalog:%:installed';
\if :conflict_probe
BEGIN;
UPDATE public.strategies
   SET description = description || ' synthetic catalog conflict'
 WHERE id = 'buy_and_hold';
\ir /validation/sync-baseline-strategy-catalog.sql
\endif
