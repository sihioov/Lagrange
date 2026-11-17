-- Fixed read-only Owner-beta Paper history inventory (Owner Option A, 2026-11-17).
-- The installer runs this exact file as migration_owner through the existing
-- Daily control one-shot (lagrange-daily-control) of the current release's
-- db-migrate image. It never writes, never reads a Paper data row and prints
-- exactly one closed line:
--   PAPER_HISTORY_INVENTORY: <PRE_PAPER|PAPER_SCHEMA> max=<n> lineage=<hex>
-- Every other database state raises, and the caller treats it as UNKNOWN.
--
-- PRE_PAPER    all _sqlx_migrations rows succeeded, are contiguous from 1,
--              max <= 65, none of 66-69, and zero owner-beta Paper catalog
--              objects of any class below.
-- PAPER_SCHEMA 66-69 applied, the twelve owner-beta Paper tables and the P4
--              INSPECT ABI exist. Presence is then decided by the P4 INSPECT
--              inventory of the current Paper-capable release, not here.
--
-- lineage is SHA-256 over "<version>:<lowercase hex checksum>" joined by ","
-- in version order. The caller admits only lineages computed from its own
-- migrations/*.up.sql (sqlx 0.9 stores SHA-384 of the up-file text).
--
-- Generic pre-0066 Paper objects (paper_rebalance_previews, paper settlement
-- outbox, pending_targets and their functions) are NOT Paper history and are
-- deliberately outside every predicate. Each predicate class is one labelled
-- count; extend the matching class when 0066+ adds a new object kind.
\set ON_ERROR_STOP on
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL statement_timeout = '15s';
SET LOCAL lock_timeout = '5s';
SET LOCAL idle_in_transaction_session_timeout = '20s';
SET LOCAL search_path = pg_catalog, pg_temp;
DO $paper_inventory$
DECLARE
    max_version bigint;
    row_count bigint;
    paper_versions bigint;
    -- Class 1: relations of any kind (tables, views, indexes, sequences)
    -- named owner_beta_paper_* (0066-0068) or paper_preview_beta_* (0067).
    paper_relations bigint;
    -- Class 2: functions paper_0066_* .. paper_0069_* or *owner_beta_paper*.
    paper_functions bigint;
    -- Class 3: user triggers paper_0066_* .. paper_0069_* or owner_beta_paper_*.
    paper_triggers bigint;
    -- Class 4: row-level security policies paper_0066_* .. paper_0069_*.
    paper_policies bigint;
    -- Class 5: 0067 columns added to the pre-existing generic Paper tables.
    paper_columns bigint;
    -- Class 6: 0067 constraints added to five named pre-existing tables.
    paper_constraints bigint;
    paper_objects bigint;
    paper_tables bigint;
    lineage text;
BEGIN
    IF current_user <> 'migration_owner' OR session_user <> 'migration_owner'
       OR current_setting('transaction_read_only') <> 'on'
       OR current_setting('transaction_isolation') <> 'repeatable read'
       OR to_regclass('public._sqlx_migrations') IS NULL THEN
        RAISE EXCEPTION 'PAPER_INVENTORY_VISIBILITY' USING ERRCODE = '42501';
    END IF;
    -- A failed or partially applied migration is never absence.
    IF EXISTS (SELECT 1 FROM public._sqlx_migrations m WHERE m.success IS DISTINCT FROM true) THEN
        RAISE EXCEPTION 'PAPER_INVENTORY_DIRTY' USING ERRCODE = '23514';
    END IF;
    SELECT max(m.version), count(*) INTO max_version, row_count FROM public._sqlx_migrations m;
    IF max_version IS NULL OR row_count <> max_version
       OR EXISTS (SELECT 1 FROM public._sqlx_migrations m WHERE m.version < 1 OR m.checksum IS NULL) THEN
        RAISE EXCEPTION 'PAPER_INVENTORY_LINEAGE' USING ERRCODE = '23514';
    END IF;
    SELECT count(*) INTO paper_versions FROM public._sqlx_migrations m
        WHERE m.version BETWEEN 66 AND 69;

    SELECT count(*) INTO paper_relations FROM pg_class c
        WHERE c.relname LIKE 'owner\_beta\_paper\_%'
           OR c.relname LIKE 'paper\_preview\_beta\_%';
    SELECT count(*) INTO paper_functions FROM pg_proc p
        WHERE p.proname ~ '^paper_006[6-9]_'
           OR p.proname LIKE '%owner\_beta\_paper%';
    SELECT count(*) INTO paper_triggers FROM pg_trigger t
        WHERE NOT t.tgisinternal
          AND t.tgname ~ '^(paper_006[6-9]_|owner_beta_paper_)';
    SELECT count(*) INTO paper_policies FROM pg_policy pol
        WHERE pol.polname ~ '^paper_006[6-9]_';
    SELECT count(*) INTO paper_columns
        FROM pg_attribute a
        JOIN pg_class c ON c.oid = a.attrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND a.attnum > 0 AND NOT a.attisdropped
          AND ((c.relname = 'pending_targets' AND a.attname = 'beta_lineage_id')
            OR (c.relname = 'paper_rebalance_previews' AND a.attname IN (
                'input_kind', 'beta_apply_mode', 'beta_lineage_id', 'beta_request_key',
                'account_state_preimage', 'signal_allocation_sha256')));
    SELECT count(*) INTO paper_constraints
        FROM pg_constraint k
        JOIN pg_class c ON c.oid = k.conrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public'
          AND c.relname IN ('accounts', 'user_strategy_configs', 'owner_beta_daily_admissions',
                            'pending_targets', 'paper_rebalance_previews')
          AND k.conname ~ '(_owner_beta_key$|_owner_account_beta_key$|_exact_beta_target_key$|_beta_lineage_fkey$|^paper_preview_beta_|^paper_preview_input_kind_check$)';
    paper_objects := paper_relations + paper_functions + paper_triggers + paper_policies
                     + paper_columns + paper_constraints;

    SELECT count(*) INTO paper_tables
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p')
          AND c.relname = ANY (ARRAY[
              'owner_beta_paper_source_slots', 'owner_beta_paper_source_admissions',
              'owner_beta_paper_source_originals', 'owner_beta_paper_source_runs',
              'owner_beta_paper_control', 'owner_beta_paper_events',
              'owner_beta_paper_lineage', 'owner_beta_paper_sessions',
              'owner_beta_paper_session_events', 'owner_beta_paper_operations',
              'owner_beta_paper_operation_observations', 'owner_beta_paper_recovery_grants']);

    IF max_version <= 65 AND paper_versions = 0 AND paper_objects = 0 THEN
        PERFORM set_config('lagrange.paper_inventory', 'PRE_PAPER', true);
    ELSIF max_version >= 69 AND paper_versions = 4 AND paper_tables = 12
          AND to_regprocedure('public.inspect_owner_beta_paper_operation_v1(uuid,uuid,integer)') IS NOT NULL THEN
        PERFORM set_config('lagrange.paper_inventory', 'PAPER_SCHEMA', true);
    ELSE
        -- 66/67 without 68, objects without migration rows, rows without objects.
        RAISE EXCEPTION 'PAPER_INVENTORY_PARTIAL' USING ERRCODE = '23514';
    END IF;
    SELECT encode(sha256(convert_to(
               string_agg(m.version::text || ':' || encode(m.checksum, 'hex'), ',' ORDER BY m.version),
               'UTF8')), 'hex')
        INTO lineage FROM public._sqlx_migrations m;
    IF lineage IS NULL OR lineage !~ '^[0-9a-f]{64}$' THEN
        RAISE EXCEPTION 'PAPER_INVENTORY_LINEAGE' USING ERRCODE = '23514';
    END IF;
    PERFORM set_config('lagrange.paper_inventory_max', max_version::text, true);
    PERFORM set_config('lagrange.paper_inventory_lineage', lineage, true);
END $paper_inventory$;
SELECT 'PAPER_HISTORY_INVENTORY: ' || current_setting('lagrange.paper_inventory')
    || ' max=' || current_setting('lagrange.paper_inventory_max')
    || ' lineage=' || current_setting('lagrange.paper_inventory_lineage');
ROLLBACK;
