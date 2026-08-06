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
--
-- PASSWORDS: the harness builds role connection URLs by rewriting the
-- superuser DATABASE_URL in place (`conn_url` in migration_contract.rs keeps
-- the password part), so on scram-sha-256 clusters (pg_hba
-- `host ... scram-sha-256`) every role must be created WITH the same
-- password as the superuser URL. The disposable WSL PG18 cluster runs scram
-- with password `lagrange`; production deployments inject real passwords via
-- the compose secret flow instead. Roles are CLUSTER-WIDE and created only
-- once (the DO blocks are idempotent), so this file must carry the final
-- password before the first harness run on a given cluster.

DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'migration_owner') THEN
    CREATE ROLE migration_owner LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;

DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'app') THEN
    CREATE ROLE app LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;

DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'worker') THEN
    CREATE ROLE worker LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;

DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'audit_writer') THEN
    CREATE ROLE audit_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;

-- admin (Todo 23): dedicated read-only admin role for the explicit, audited
-- Owner admin pathway (0010 grants SELECT on tenant/shared/audit tables).
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'admin') THEN
    CREATE ROLE admin LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;

-- migration_owner performs DDL via migrations; serving roles receive USAGE but
-- never CREATE, so they cannot create objects at schema level.
GRANT USAGE ON SCHEMA public TO migration_owner, app, worker, audit_writer, admin;
GRANT CREATE ON SCHEMA public TO migration_owner;
