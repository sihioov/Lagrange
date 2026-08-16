-- Fail-closed connection/schema hazards for a disposable validation cluster.
\set ON_ERROR_STOP on

DO $$
DECLARE
    v_max_connections integer := current_setting('max_connections')::integer;
    v_backends integer;
BEGIN
    SELECT count(*) INTO v_backends FROM pg_stat_activity;
    IF v_max_connections < 64 THEN
        RAISE EXCEPTION 'max_connections is too small for the DB-gated suites: %', v_max_connections;
    END IF;
    IF v_backends >= (v_max_connections * 80 / 100) THEN
        RAISE EXCEPTION 'connection pool hazard: % of % slots are already active', v_backends, v_max_connections;
    END IF;
END
$$;

DO $$
BEGIN
    IF current_database() IS NULL OR current_database() <> 'postgres' THEN
        RAISE EXCEPTION 'validation must run against the disposable postgres database';
    END IF;
    IF current_setting('server_version_num')::integer < 180000 THEN
        RAISE EXCEPTION 'validation requires PostgreSQL 18 or newer, got %', current_setting('server_version');
    END IF;
END
$$;

SELECT 'database=' || current_database();
SELECT 'server_version=' || current_setting('server_version');
SELECT 'max_connections=' || current_setting('max_connections');
SELECT 'active_connections=' || count(*) FROM pg_stat_activity;
