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
  #
  # The inner script is single-quoted and therefore expands INSIDE the
  # container, where the only database settings that exist are the service's
  # `DB_*` ones (compose.yml db-migrate: `DB_NAME: ${POSTGRES_DB:-lagrange}`).
  # `POSTGRES_DB` is a HOST variable consumed by compose interpolation and is
  # never passed through, so reading it here yielded an empty `-d`, at which
  # point libpq falls back to the username and psql died with
  # `database "migration_owner" does not exist`.  Use the service settings the
  # comment above already promises.
  #
  # Compose interpolates the WHOLE file to run one service, so both required
  # variables must be satisfiable here or the call dies before any container
  # exists.  RANGE_RAW_BATCH_ID belongs to the Stage5 range services and
  # LAGRANGE_CODE_COMMIT to other services' build args; `db-migrate` consumes
  # neither, and LAGRANGE_CODE_COMMIT is NOT a key in the production env file
  # (only a comment), so without a value here every caller that does not inject
  # it separately fails.  kis-daily-production.sh survives because its systemd
  # unit sets it; provision-entitlement.sh and register-dataset-version.sh do
  # not set it at all, and reported the resulting interpolation abort as
  # "pending entitlement row is absent or conflicts" — a data message for a
  # config failure, which is the exact confusion this helper already had once.
  # Defaults, so a real value from the caller still wins.
  RANGE_RAW_BATCH_ID=${RANGE_RAW_BATCH_ID:-compose-config-disabled} \
  LAGRANGE_CODE_COMMIT=${LAGRANGE_CODE_COMMIT:-compose-config-disabled} \
  docker compose --env-file "$compose_env_file" -f "$compose_file" \
    run --rm --no-deps --entrypoint /bin/sh db-migrate \
    -ec 'export PGPASSWORD="$(cat "$DB_PASSWORD_FILE")"; exec psql \
      -X --no-password -v ON_ERROR_STOP=1 -P pager=off \
      -h "$DB_HOST" -p "$DB_PORT" -U "$DB_USER" -d "$DB_NAME" "$@"' \
    operator-attestation-psql "$@"
}
