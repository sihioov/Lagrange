# Database bootstrap and migrations

The serving roles are deliberately not created by migration SQL. Run
`bootstrap-roles.sh` once as the PostgreSQL administrator, with Docker/secret
files mounted at the paths below, before starting any worker:

```sh
DB_ADMIN_PASSWORD_FILE=/run/secrets/postgres_password \
DB_MIGRATION_OWNER_PASSWORD_FILE=/run/secrets/db_migration_owner_password \
DB_APP_PASSWORD_FILE=/run/secrets/db_app_password \
DB_WORKER_PASSWORD_FILE=/run/secrets/db_worker_password \
DB_AUDIT_PASSWORD_FILE=/run/secrets/db_audit_password \
DB_RESEARCH_PASSWORD_FILE=/run/secrets/db_research_password \
DB_ADMIN_ROLE_PASSWORD_FILE=/run/secrets/db_admin_password \
  ./deploy/db/bootstrap-roles.sh
```

The script is idempotent, rejects symlinked/empty/multiline secret files, and
never accepts a plaintext password environment variable. It creates only the
six least-privilege login roles used by the migrations and grants
`migration_owner` the sole `public` schema `CREATE` privilege. The role SQL is
temporary mode `0600` and is removed on exit.

Paper can consume either full role-scoped URL files (`PAPER_*_DATABASE_URL_FILE`)
or the canonical component settings plus `PAPER_*_DB_PASSWORD_FILE`; the
Paper image builds the latter URLs in memory from mounted secret files.

`DB_MIGRATION_OWNER_PASSWORD_FILE` is a required, separate credential from
the PostgreSQL administrator file. The administrator secret is used only for
the bootstrap connection; migration SQL runs only as `migration_owner`.
`db_admin_password` is intentionally a separate input even though the role is
read-only in the migration contract.

## Migration runtime

`migrate.sh` is a one-shot wrapper around `sqlx migrate run`. It requires an
explicit migration-owner URL file (or complete DB component configuration and
`DB_PASSWORD_FILE`), sets finite lock/statement timeouts, and refuses direct
password values. It always enforces five-second lock and sixty-second statement
timeouts, regardless of inherited `PGOPTIONS`. It exits non-zero on any failed migration; workers must depend
on this one-shot completing successfully. The command is safe to rerun because
SQLx records checksums and migration state in `_sqlx_migrations`.

Both Compose one-shot database services use the pinned `deploy/db/Dockerfile`
image and run as non-root UID/GID `999:999`. Their service-specific secret
copies are therefore provisioned as `999:999` with mode `0400`; do not change
the image to root or reuse the PostgreSQL administrator file for migrations.
