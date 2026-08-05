-- Lagrange Station database bootstrap (Todo 3).
--
-- Executed by the migration-contract harness as the cluster SUPERUSER on each
-- fresh disposable database, before `sqlx migrate run`. Idempotent: roles are
-- cluster-level and created only once; schema grants are per-database.
-- (GRANT CONNECT ON DATABASE is issued per-database by the harness.)
--
-- Role split (design §7.3, §14.3; plan Todo 3):
--   migration_owner : runs migrations (DDL) only; owns every table.
--   app             : serving API role; NO table ownership, NO schema CREATE,
--                     NO BYPASSRLS, no audit writes (SELECT only there).
--   worker          : job/backtest workers; claims/updates jobs and attempts.
--   audit_writer    : the ONLY writer of append-only audit_logs.
--
-- Serving roles must never hold BYPASSRLS (plan: "app roles must not own
-- tables or have BYPASSRLS") - asserted by the harness.

DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'migration_owner') THEN
    CREATE ROLE migration_owner LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
  END IF;
END $role$;

DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'app') THEN
    CREATE ROLE app LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
  END IF;
END $role$;

DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'worker') THEN
    CREATE ROLE worker LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
  END IF;
END $role$;

DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'audit_writer') THEN
    CREATE ROLE audit_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
  END IF;
END $role$;

-- migration_owner performs DDL via migrations; serving roles receive USAGE but
-- never CREATE, so they cannot create objects at schema level.
GRANT USAGE ON SCHEMA public TO migration_owner, app, worker, audit_writer;
GRANT CREATE ON SCHEMA public TO migration_owner;
