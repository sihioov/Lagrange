#!/usr/bin/env bash
# One-shot SQLx migration runtime. This script intentionally does not create
# roles and never runs as a serving user.
set -euo pipefail

die() {
  echo "db-migrate: $*" >&2
  exit 1
}

reject_line_endings() {
  local label=$1 path=$2 line_count
  line_count=$(LC_ALL=C wc -l < "$path") || die "$label file is unreadable"
  [ "$line_count" -eq 0 ] || die "$label file must contain one line"
  if LC_ALL=C grep -Fq $'\r' "$path"; then
    die "$label file must contain one line"
  fi
}

read_secret() {
  local label=$1 path=$2 value
  [ -n "$path" ] || die "$label file path is required"
  [ -f "$path" ] || die "$label file is missing"
  [ ! -L "$path" ] || die "$label file must not be a symlink"
  [ -r "$path" ] || die "$label file is unreadable"
  reject_line_endings "$label" "$path"
  value=$(cat -- "$path") || die "$label file is unreadable"
  [ -n "$value" ] || die "$label file is empty"
  case "$value" in *$'\n'*|*$'\r'*) die "$label file must contain one line" ;; esac
  printf '%s' "$value"
}

url_encode() {
  # Encode URL userinfo/path components byte-for-byte. The migration image
  # intentionally has no Python/Node dependency, so use Bash's byte-oriented
  # printf conversion under the C locale. The resulting URL is kept only in
  # this process environment for SQLx; it is never written to logs or args.
  local input=$1 output='' char encoded
  local LC_ALL=C
  local index
  for ((index = 0; index < ${#input}; index++)); do
    char=${input:index:1}
    case "$char" in
      [A-Za-z0-9._~-]) output+=$char ;;
      *)
        printf -v encoded '%%%02X' "'$char"
        output+=$encoded
        ;;
    esac
  done
  printf '%s' "$output"
}

url_decode() {
  local input=$1 output='' prefix hex byte code
  local LC_ALL=C
  while [[ "$input" == *%* ]]; do
    prefix=${input%%\%*}
    output+=$prefix
    input=${input#*%}
    [ "${#input}" -ge 2 ] || die 'DATABASE_URL_FILE contains an invalid percent escape'
    hex=${input:0:2}
    case "$hex" in *[!0-9A-Fa-f]*) die 'DATABASE_URL_FILE contains an invalid percent escape' ;; esac
    code=$((16#$hex))
    ((code >= 32 && code != 127)) \
      || die 'DATABASE_URL_FILE contains a forbidden encoded control byte'
    printf -v byte '%b' "\\x$hex"
    output+=$byte
    input=${input:2}
  done
  printf '%s%s' "$output" "$input"
}

configure_psql_from_url() {
  # psql expands a URI supplied with -d, but not one inherited through
  # PGDATABASE. Passing the original URI on argv would expose its password.
  # Split the strict URL form into libpq environment variables instead.
  local url=$1 remainder authority encoded_database userinfo hostport
  local encoded_user encoded_password host suffix port
  case "$url" in
    postgres://*) remainder=${url#postgres://} ;;
    postgresql://*) remainder=${url#postgresql://} ;;
    *) die 'DATABASE_URL_FILE must contain a PostgreSQL URL' ;;
  esac
  case "$remainder" in *\?*|*\#*) die 'DATABASE_URL_FILE query and fragment components are unsupported' ;; esac
  case "$remainder" in */*) ;; *) die 'DATABASE_URL_FILE must include a database name' ;; esac
  authority=${remainder%%/*}
  encoded_database=${remainder#*/}
  [ -n "$encoded_database" ] || die 'DATABASE_URL_FILE must include a database name'
  case "$encoded_database" in */*) die 'DATABASE_URL_FILE database name must encode slash bytes' ;; esac
  case "$authority" in *@*) ;; *) die 'DATABASE_URL_FILE must include user credentials' ;; esac
  userinfo=${authority%@*}
  hostport=${authority##*@}
  case "$userinfo" in *:*) ;; *) die 'DATABASE_URL_FILE must include a password' ;; esac
  encoded_user=${userinfo%%:*}
  encoded_password=${userinfo#*:}
  [ -n "$encoded_user" ] || die 'DATABASE_URL_FILE must include a user'

  case "$hostport" in
    \[*\]*)
      host=${hostport%%]*}
      host=${host#\[}
      suffix=${hostport#*]}
      case "$suffix" in
        '') port=5432 ;;
        :*) port=${suffix#:} ;;
        *) die 'DATABASE_URL_FILE host is invalid' ;;
      esac
      ;;
    *:*)
      host=${hostport%:*}
      port=${hostport##*:}
      ;;
    *)
      host=$hostport
      port=5432
      ;;
  esac
  [ -n "$host" ] || die 'DATABASE_URL_FILE host is invalid'
  case "$port" in ''|*[!0-9]*) die 'DATABASE_URL_FILE port is invalid' ;; esac
  ((port > 0 && port <= 65535)) || die 'DATABASE_URL_FILE port is invalid'

  export PGHOST="$host"
  export PGPORT="$port"
  export PGUSER
  PGUSER=$(url_decode "$encoded_user")
  export PGPASSWORD
  PGPASSWORD=$(url_decode "$encoded_password")
  export PGDATABASE
  PGDATABASE=$(url_decode "$encoded_database")
}

db_url_file=${DATABASE_URL_FILE:-${MIGRATION_DATABASE_URL_FILE:-}}
if [ -n "${DATABASE_URL_FILE:-}" ] && [ -n "${MIGRATION_DATABASE_URL_FILE:-}" ]; then
  die 'DATABASE_URL_FILE and MIGRATION_DATABASE_URL_FILE are mutually exclusive'
fi
if [ -n "${DATABASE_URL:-}" ] && [ -n "$db_url_file" ]; then
  die 'DATABASE_URL and DATABASE_URL_FILE are mutually exclusive'
fi

if [ -n "$db_url_file" ]; then
  connection_mode=url
  database_url=$(read_secret DATABASE_URL "$db_url_file")
  case "$database_url" in postgres://*|postgresql://*) ;; *) die 'DATABASE_URL_FILE must contain a PostgreSQL URL' ;; esac
elif [ -n "${DATABASE_URL:-}" ]; then
  die 'DATABASE_URL plaintext is forbidden; use DATABASE_URL_FILE'
else
  connection_mode=components
  : "${DB_HOST:?DB_HOST is required in component mode}"
  : "${DB_PORT:?DB_PORT is required in component mode}"
  : "${DB_NAME:?DB_NAME is required in component mode}"
  : "${DB_USER:?DB_USER is required in component mode}"
  : "${DB_PASSWORD_FILE:?DB_PASSWORD_FILE is required in component mode}"
  password=$(read_secret DB_PASSWORD "$DB_PASSWORD_FILE")
  case "$DB_PORT" in *[!0-9]*|'') die 'DB_PORT must be numeric' ;; esac
  ((DB_PORT > 0 && DB_PORT <= 65535)) || die 'DB_PORT must be between 1 and 65535'
  [ -n "$DB_NAME" ] || die 'DB_NAME must not be empty'
  [ -n "$DB_USER" ] || die 'DB_USER must not be empty'
  case "$DB_HOST" in
    ''|*[!A-Za-z0-9_.:-]*) die 'DB_HOST must be a DNS name, IPv4 address, or IPv6 address' ;;
  esac
  database_host=$DB_HOST
  case "$database_host" in
    *:*)
      case "$database_host" in
        \[*\]) ;;
        *) database_host="[$database_host]" ;;
      esac
      ;;
  esac
  encoded_user=$(url_encode "$DB_USER")
  encoded_password=$(url_encode "$password")
  encoded_name=$(url_encode "$DB_NAME")
  database_url="postgresql://${encoded_user}:${encoded_password}@${database_host}:${DB_PORT}/${encoded_name}"
fi

: "${MIGRATIONS_DIR:=/opt/lagrange/migrations}"
[ -d "$MIGRATIONS_DIR" ] || die "migration directory is missing: $MIGRATIONS_DIR"
command -v sqlx >/dev/null 2>&1 || die 'sqlx executable is missing from the migration image'

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
if [ "$script_dir" = /usr/local/bin ]; then
  catalog_file=/opt/lagrange/configs/strategies/baseline-v1.json
  catalog_sql=/opt/lagrange/db/sync-baseline-strategy-catalog.sql
else
  # Repository-local execution is used only by the static regression harness.
  catalog_file="$script_dir/../../configs/strategies/baseline-v1.json"
  catalog_sql="$script_dir/sync-baseline-strategy-catalog.sql"
fi
for catalog_input in "$catalog_file" "$catalog_sql"; do
  [ -f "$catalog_input" ] || die 'strategy catalog runtime input is missing'
  [ ! -L "$catalog_input" ] || die 'strategy catalog runtime input must not be a symlink'
  [ -r "$catalog_input" ] || die 'strategy catalog runtime input is unreadable'
  [ -s "$catalog_input" ] || die 'strategy catalog runtime input is empty'
done
command -v psql >/dev/null 2>&1 || die 'psql executable is missing from the migration image'
command -v sha256sum >/dev/null 2>&1 || die 'sha256sum executable is missing from the migration image'

# Validate every non-secret catalog input and prepare the psql connection
# before applying migrations. A malformed URL or catalog must fail before any
# database state can change.
catalog_json=$(cat -- "$catalog_file") || die 'strategy catalog is unreadable'
catalog_sha256=$(sha256sum -- "$catalog_file") || die 'strategy catalog hash failed'
catalog_sha256=${catalog_sha256%% *}
case "$catalog_sha256" in *[!0-9a-f]*|'') die 'strategy catalog hash is invalid' ;; esac
[ "${#catalog_sha256}" -eq 64 ] || die 'strategy catalog hash is invalid'
if [ "$connection_mode" = components ]; then
  export PGHOST="$DB_HOST"
  export PGPORT="$DB_PORT"
  export PGUSER="$DB_USER"
  export PGPASSWORD="$password"
  export PGDATABASE="$DB_NAME"
else
  configure_psql_from_url "$database_url"
fi

# SQLx migration files explicitly mark the concurrent-index steps as
# no-transaction. A finite lock timeout is mandatory so deploys fail and can
# be retried instead of holding an unbounded DDL lock.
export DATABASE_URL="$database_url"
export PGOPTIONS='-c lock_timeout=5s -c statement_timeout=60s'
sqlx migrate run --source "$MIGRATIONS_DIR"

# The catalog is a non-secret, commit-pinned projection of selector's five
# baseline packages. It is installed only after every schema migration
# succeeds. The SQL refuses to mutate conflicting rows and is safe to rerun.
unset DATABASE_URL database_url password encoded_password encoded_user encoded_name
psql -X --no-password --quiet --tuples-only --no-align \
  --set=catalog_json="$catalog_json" \
  --set=catalog_sha256="$catalog_sha256" \
  --file="$catalog_sql"
unset PGPASSWORD
