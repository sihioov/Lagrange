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

db_url_file=${DATABASE_URL_FILE:-${MIGRATION_DATABASE_URL_FILE:-}}
if [ -n "${DATABASE_URL_FILE:-}" ] && [ -n "${MIGRATION_DATABASE_URL_FILE:-}" ]; then
  die 'DATABASE_URL_FILE and MIGRATION_DATABASE_URL_FILE are mutually exclusive'
fi
if [ -n "${DATABASE_URL:-}" ] && [ -n "$db_url_file" ]; then
  die 'DATABASE_URL and DATABASE_URL_FILE are mutually exclusive'
fi

if [ -n "$db_url_file" ]; then
  database_url=$(read_secret DATABASE_URL "$db_url_file")
  case "$database_url" in postgres://*|postgresql://*) ;; *) die 'DATABASE_URL_FILE must contain a PostgreSQL URL' ;; esac
elif [ -n "${DATABASE_URL:-}" ]; then
  die 'DATABASE_URL plaintext is forbidden; use DATABASE_URL_FILE'
else
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

# SQLx migration files explicitly mark the concurrent-index steps as
# no-transaction. A finite lock timeout is mandatory so deploys fail and can
# be retried instead of holding an unbounded DDL lock.
export DATABASE_URL="$database_url"
export PGOPTIONS='-c lock_timeout=5s -c statement_timeout=60s'
exec sqlx migrate run --source "$MIGRATIONS_DIR"
