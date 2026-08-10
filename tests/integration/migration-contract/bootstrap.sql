-- Per-database bootstrap for migration-contract scratch databases.
--
-- Cluster-global roles are created separately from role-bootstrap.sql while
-- holding a supervisor-database advisory lock. Keep this file limited to
-- grants on the newly created scratch database so test setup never mutates
-- the supervisor database's public schema.

GRANT USAGE ON SCHEMA public TO migration_owner, app, worker, audit_writer, research_writer, admin;
GRANT CREATE ON SCHEMA public TO migration_owner;
