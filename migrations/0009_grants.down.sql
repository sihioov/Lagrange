-- Revoke every privilege granted to the serving roles (0009 up). Object
-- ownership (migration_owner) is untouched; REVOKE only strips grants.
REVOKE ALL PRIVILEGES ON ALL TABLES IN SCHEMA public FROM app, worker, audit_writer;
