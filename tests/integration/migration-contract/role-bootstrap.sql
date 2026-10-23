-- Cluster-global roles required before migrations run.
--
-- The migration-contract harness serializes this file with an advisory lock
-- in the supervisor database. Keep database-local schema and object grants in
-- bootstrap.sql; executing this file must not mutate the supervisor schema.

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

DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'research_writer') THEN
    CREATE ROLE research_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;

DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'admin') THEN
    CREATE ROLE admin LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;

DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'daily_input_admitter') THEN
    CREATE ROLE daily_input_admitter LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE
      NOREPLICATION NOBYPASSRLS PASSWORD 'lagrange';
  END IF;
END $role$;

DO $role_preflight$ BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM pg_roles
    WHERE rolname = 'daily_input_admitter'
      AND rolcanlogin
      AND NOT rolinherit
      AND NOT rolsuper
      AND NOT rolcreatedb
      AND NOT rolcreaterole
      AND NOT rolreplication
      AND NOT rolbypassrls
  ) THEN
    RAISE EXCEPTION 'daily_input_admitter role attributes mismatch';
  END IF;

  IF EXISTS (
    SELECT 1
    FROM pg_auth_members
    WHERE member = (SELECT oid FROM pg_roles WHERE rolname = 'daily_input_admitter')
       OR roleid = (SELECT oid FROM pg_roles WHERE rolname = 'daily_input_admitter')
  ) THEN
    RAISE EXCEPTION 'daily_input_admitter has unexpected role membership';
  END IF;

  IF EXISTS (
    WITH RECURSIVE membership_closure(start_role, reachable_role) AS (
      SELECT oid, oid
      FROM pg_roles
      WHERE rolname IN ('app', 'worker')
      UNION
      SELECT closure.start_role, membership.roleid
      FROM membership_closure AS closure
      JOIN pg_auth_members AS membership ON membership.member = closure.reachable_role
    )
    SELECT 1
    FROM membership_closure AS closure
    JOIN pg_roles AS start_role ON start_role.oid = closure.start_role
    JOIN pg_roles AS reachable_role ON reachable_role.oid = closure.reachable_role
    WHERE start_role.rolname IN ('app', 'worker')
      AND reachable_role.rolname IN ('migration_owner', 'daily_input_admitter')
  ) THEN
    RAISE EXCEPTION 'app or worker reaches a protected migration role';
  END IF;
END $role_preflight$;
