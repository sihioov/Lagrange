-- Per-database bootstrap for migration-contract scratch databases.
--
-- Cluster-global roles are created separately from role-bootstrap.sql while
-- holding a supervisor-database advisory lock. Keep this file limited to
-- grants on the newly created scratch database so test setup never mutates
-- the supervisor database's public schema.

DO $database_privilege$ BEGIN
  EXECUTE format('GRANT CONNECT ON DATABASE %I TO daily_input_admitter', current_database());
END $database_privilege$;

REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO migration_owner, app, worker, audit_writer, research_writer, admin,
  daily_input_admitter;
GRANT CREATE ON SCHEMA public TO migration_owner;

DO $privilege_preflight$ BEGIN
  IF NOT has_database_privilege('daily_input_admitter', current_database(), 'CONNECT') THEN
    RAISE EXCEPTION 'daily_input_admitter lacks CONNECT on the scratch database';
  END IF;
  IF NOT has_schema_privilege('daily_input_admitter', 'public', 'USAGE') THEN
    RAISE EXCEPTION 'daily_input_admitter lacks USAGE on scratch public schema';
  END IF;
  IF has_database_privilege('daily_input_admitter', current_database(), 'CREATE') THEN
    RAISE EXCEPTION 'daily_input_admitter has CREATE on the scratch database';
  END IF;
  IF has_schema_privilege('daily_input_admitter', 'public', 'CREATE') THEN
    RAISE EXCEPTION 'daily_input_admitter has CREATE on scratch public schema';
  END IF;
  IF NOT has_schema_privilege('migration_owner', 'public', 'CREATE') THEN
    RAISE EXCEPTION 'migration_owner lacks CREATE on scratch public schema';
  END IF;
END $privilege_preflight$;
