#!/usr/bin/env bash
# Shared PostgreSQL runner for operator attestation.  Host PostgreSQL ports
# are intentionally not published by Compose, so psql runs inside the already
# built db-migrate image on the private backend network.  The migration-owner
# password is mounted by Compose as a Docker secret; it never enters argv,
# host environment, SQL, or operator output.

db_die() {
  echo "operator-db: $*" >&2
  exit 1
}

db_init() {
  command -v docker >/dev/null 2>&1 || db_die 'docker is required for --check/--apply'
  compose_file=${LAGRANGE_COMPOSE_FILE:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)/deploy/compose/compose.yml}
  compose_env_file=${LAGRANGE_ENV_FILE:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)/deploy/compose/.env}
  [ -f "$compose_file" ] || db_die "Compose file is missing: $compose_file"
  [ ! -L "$compose_file" ] || db_die 'Compose file must not be a symlink'
  [ -f "$compose_env_file" ] || db_die "Compose env file is missing: $compose_env_file"
  [ ! -L "$compose_env_file" ] || db_die 'Compose env file must not be a symlink'
  if [ -z "${POSTGRES_DB:-}" ]; then
    POSTGRES_DB=$(awk -F= '$1 == "POSTGRES_DB" {print substr($0, index($0, "=") + 1); exit}' "$compose_env_file")
  fi
  : "${POSTGRES_DB:=lagrange}"
  compose_file=$(cd "$(dirname "$compose_file")" && pwd)/$(basename "$compose_file")
  compose_env_file=$(cd "$(dirname "$compose_env_file")" && pwd)/$(basename "$compose_env_file")
  export compose_file compose_env_file POSTGRES_DB
}

db_psql() {
  # The service mounts db-migrate's migration_owner secret and supplies its
  # own DB_* service settings.  Read the secret only inside the short-lived
  # container; --no-deps keeps this helper from starting/restarting a service.
  # PGPASSWORD exists only in that container process environment so libpq can
  # authenticate; it never enters the host argv/environment or SQL text.
  docker compose --env-file "$compose_env_file" -f "$compose_file" \
    run --rm --no-deps --entrypoint /bin/sh db-migrate \
    -ec 'export PGPASSWORD="$(cat "$DB_PASSWORD_FILE")"; exec psql \
      -X --no-password -v ON_ERROR_STOP=1 -P pager=off \
      -h postgres -p 5432 -U migration_owner -d "$POSTGRES_DB" "$@"' \
    operator-attestation-psql "$@"
}
