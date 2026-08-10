#!/usr/bin/env bash
# Static and functional smoke test for the research-worker Compose service.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
compose_file="$root/deploy/compose/compose.yml"
dockerfile="$root/data-pipelines/collectors/Dockerfile"
secret_example="$root/deploy/secrets/db_research_password.example"
static_only="${LAGRANGE_RESEARCH_SMOKE_STATIC_ONLY:-0}"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --static-only) static_only=1; shift ;;
    *) echo "USAGE: $0 [--static-only]" >&2; exit 2 ;;
  esac
done

fail() { echo "RESEARCH_WORKER_SMOKE: $*" >&2; exit 1; }
contains() { printf '%s' "$1" | grep -Fq -- "$2" || fail "$3 missing required value: $2"; }

[ -f "$compose_file" ] || fail "missing Compose file: $compose_file"
compose_text="$(<"$compose_file")"
worker="$(awk '
  /^  research-worker:[[:space:]]*$/ { found=1; next }
  found && /^  [A-Za-z0-9][A-Za-z0-9-]*:[[:space:]]*$/ { exit }
  found && /^secrets:[[:space:]]*$/ { exit }
  found { print }
' "$compose_file")"
[ -n "$worker" ] || fail 'research-worker service is missing from Compose'

while IFS= read -r required; do
  contains "$worker" "$required" 'research-worker service'
done <<'REQUIRED'
build:
context: ../..
dockerfile: data-pipelines/collectors/Dockerfile
entrypoint: ["/usr/local/bin/research-worker"]
APP_ENV: ${APP_ENV:-development}
RESEARCH_FETCH_MODE: ${RESEARCH_FETCH_MODE:-synthetic}
RESEARCH_RUN_AT_KST: ${RESEARCH_RUN_AT_KST:-16:30}
RESEARCH_MAX_PUBLICATION_AGE_SECS: ${RESEARCH_MAX_PUBLICATION_AGE_SECS:-345600}
RESEARCH_RAW_ROOT: /data/raw
DB_HOST: postgres
DB_PORT: "5432"
DB_NAME: ${POSTGRES_DB:-lagrange}
DB_USER: research_writer
DB_PASSWORD_FILE: /run/secrets/db_research_password
- db_research_password
${LAGRANGE_DATA_DIR:-../data}/raw:/data/raw
test: ["CMD", "/usr/local/bin/research-worker", "healthcheck"]
REQUIRED

printf '%s' "$worker" | grep -Eqi 'time\.sleep|(^|[^[:alnum:]_])sleep([^[:alnum:]_]|$)|python[[:space:]]+-c' && fail 'research-worker service still contains a sleep/Python placeholder'
while IFS= read -r mount; do
  case "$mount" in *:ro) ;; *) fail "research-worker non-Raw data mount is not read-only: $mount" ;; esac
done < <(printf '%s\n' "$worker" | grep -E ':/data/(curated|nautilus_catalog|artifacts)(/[^[:space:]:]*)?([[:space:]]|$)' || true)

contains "$compose_text" '  db_research_password:' 'Compose secrets'
contains "$compose_text" 'file: ${LAGRANGE_DB_RESEARCH_PASSWORD_SECRET_SOURCE:-../secrets/db_research_password}' 'research DB secret'
printf '%s' "$compose_text" | grep -Eq '\blagrange_(app|worker)\b' && fail 'legacy Compose DB role spelling remains (lagrange_app or lagrange_worker)'
for identity in db_app_password: db_worker_password: db_audit_password:; do
  contains "$compose_text" "$identity" 'existing Compose secret identities'
done

[ -f "$dockerfile" ] || fail "missing worker Dockerfile: $dockerfile"
[ -f "$secret_example" ] || fail "missing research DB secret example: $secret_example"
docker_text="$(<"$dockerfile")"
contains "$docker_text" 'FROM rust:1.97.1-alpine@sha256:3c38f3f82c2f3d73da3b38e18d279393a04cb43ddded0e35088a8c3324d40900 AS builder' 'Dockerfile'
contains "$docker_text" 'FROM alpine:3.21@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d' 'Dockerfile'
contains "$docker_text" 'cargo build --locked --release --package collectors --bin research-worker' 'Dockerfile'
contains "$docker_text" 'ENTRYPOINT ["/usr/local/bin/research-worker"]' 'Dockerfile'
from_count=0
while IFS= read -r line; do
  from_count=$((from_count + 1))
  printf '%s\n' "$line" | grep -Eq '^FROM[[:space:]]+[^[:space:]]+@sha256:[0-9a-f]{64}([[:space:]]+AS[[:space:]]+[A-Za-z0-9._-]+)?$' || fail "Dockerfile FROM is not immutable: $line"
done < <(grep '^FROM[[:space:]]' "$dockerfile" || true)
[ "$from_count" -gt 0 ] || fail 'Dockerfile has no FROM instructions'

git_ls_files() {
  if git -C "$root" rev-parse --git-dir >/dev/null 2>&1; then
    git -C "$root" ls-files -- deploy/secrets
    return
  fi
  [ -f "$root/.git" ] || return 1
  local git_dir
  git_dir="$(sed -n 's/^gitdir: //p' "$root/.git")"
  if command -v wslpath >/dev/null 2>&1; then
    git_dir="$(wslpath -u "$git_dir")"
  elif command -v cygpath >/dev/null 2>&1; then
    git_dir="$(cygpath -u "$git_dir")"
  fi
  git --git-dir="$git_dir" --work-tree="$root" ls-files -- deploy/secrets
}
tracked_secrets="$(git_ls_files)" || fail 'git ls-files failed while checking secrets'
while IFS= read -r path; do
  [ -n "$path" ] || continue
  name="${path##*/}"
  case "$name" in README.md|.gitignore|*.example) ;; *) fail "real secret-like file is tracked: $path" ;; esac
done <<<"$tracked_secrets"
echo 'RESEARCH_WORKER_SMOKE: static PASS'

if [ "$static_only" = 1 ]; then
  echo 'RESEARCH_WORKER_SMOKE: functional SKIPPED (explicit static-only request)'
  exit 0
fi

command -v docker >/dev/null 2>&1 || fail 'Docker is required for the functional phase; use --static-only only for an explicit static check'
docker info >/dev/null 2>&1 || fail 'Docker daemon is unavailable; use --static-only only for an explicit static check'
docker compose version >/dev/null 2>&1 || fail 'Docker Compose is unavailable'

hostpath() { if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
dkr() { MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' docker "$@"; }
project="lagrange-research-smoke-$$-$(date +%s)"
temp_root="$(mktemp -d "${TMPDIR:-/tmp}/${project}.XXXXXX")"
raw_root="$temp_root/data"
postgres_secret="$temp_root/postgres_password"
research_secret="$temp_root/db_research_password"
krx_secret="$temp_root/krx_api_key"
created=0
rc() { dkr compose -p "$project" -f "$(hostpath "$compose_file")" "$@"; }
cleanup() {
  if [ "$created" -eq 1 ]; then rc down -v --remove-orphans --rmi local >/dev/null 2>&1 || true; fi
  rm -rf -- "$temp_root"
}
trap cleanup EXIT

mkdir -p "$raw_root"
umask 077
if command -v openssl >/dev/null 2>&1; then
  openssl rand -base64 32 >"$postgres_secret"
  openssl rand -base64 32 >"$research_secret"
else
  head -c 32 /dev/urandom | base64 >"$postgres_secret"
  head -c 32 /dev/urandom | base64 >"$research_secret"
fi
printf '%s' 'unused-in-synthetic-smoke' >"$krx_secret"

export LAGRANGE_POSTGRES_PASSWORD_SECRET_SOURCE="$(hostpath "$postgres_secret")"
export LAGRANGE_DB_RESEARCH_PASSWORD_SECRET_SOURCE="$(hostpath "$research_secret")"
export LAGRANGE_KRX_API_KEY_SECRET_SOURCE="$(hostpath "$krx_secret")"
export LAGRANGE_DATA_DIR="$(hostpath "$raw_root")"
export LAGRANGE_PGDATA_VOLUME="${project}-pgdata"
export POSTGRES_USER=lagrange POSTGRES_DB=lagrange APP_ENV=qa RESEARCH_FETCH_MODE=synthetic
export RESEARCH_MAX_PUBLICATION_AGE_SECS=315576000
export RESEARCH_RUN_AT_KST="$(TZ=Asia/Seoul date -d '+12 hours' +%H:%M 2>/dev/null || TZ=Asia/Seoul date +%H:%M)"

created=1
rc up -d --wait postgres >/dev/null || fail 'PostgreSQL did not become healthy'

research_password="$(<"$research_secret")"
escaped_password="${research_password//\'/\'\'}"
dkr compose -p "$project" -f "$(hostpath "$compose_file")" exec -T postgres psql -X -q -v ON_ERROR_STOP=1 -U lagrange -d lagrange >/dev/null <<SQL
DO \$roles\$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'migration_owner') THEN CREATE ROLE migration_owner LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD '$escaped_password'; END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'app') THEN CREATE ROLE app LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD '$escaped_password'; END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'worker') THEN CREATE ROLE worker LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD '$escaped_password'; END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'audit_writer') THEN CREATE ROLE audit_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD '$escaped_password'; END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'research_writer') THEN CREATE ROLE research_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD '$escaped_password'; END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'admin') THEN CREATE ROLE admin LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD '$escaped_password'; END IF;
END
\$roles\$;
SQL
unset research_password escaped_password

while IFS= read -r migration; do
  dkr compose -p "$project" -f "$(hostpath "$compose_file")" exec -T postgres psql -X -q -v ON_ERROR_STOP=1 -U lagrange -d lagrange <"$migration" >/dev/null || fail "migration failed: ${migration##*/}"
done < <(find "$root/migrations" -maxdepth 1 -type f -name '*.up.sql' | sort)

rc build research-worker || fail 'research-worker image build failed'
rc up -d --no-deps research-worker >/dev/null || fail 'research-worker service failed to start'
rc run --rm --no-deps research-worker --once --date 2020-01-31 || fail 'first research-worker one-shot failed'

healthy=0
for _ in $(seq 1 30); do
  if dkr compose -p "$project" -f "$(hostpath "$compose_file")" exec -T research-worker /usr/local/bin/research-worker healthcheck >/dev/null 2>&1; then healthy=1; break; fi
  sleep 1
done
[ "$healthy" -eq 1 ] || fail 'research-worker did not become functionally healthy'

publication_counts() {
  local value
  value="$(dkr compose -p "$project" -f "$(hostpath "$compose_file")" exec -T postgres psql -X -qAt -v ON_ERROR_STOP=1 -U lagrange -d lagrange <<'SQL'
SELECT concat_ws('|',
  (SELECT count(DISTINCT source_batch_id) FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31' AND source_batch_id IS NOT NULL),
  (SELECT count(*) FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31'),
  (SELECT count(*) FROM trading_calendar_versions WHERE exchange = 'KRX' AND source_batch_id IN (SELECT source_batch_id FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31')),
  (SELECT count(*) FROM trading_calendars WHERE exchange = 'KRX' AND source_batch_id IN (SELECT source_batch_id FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31'))
);
SQL
)"
  [[ "$value" =~ ^[0-9]+\|[0-9]+\|[0-9]+\|[0-9]+$ ]] || fail "unexpected publication count result: $value"
  IFS='|' read -r source_batches data_batches history current <<<"$value"
  [ "$source_batches" -gt 0 ] && [ "$data_batches" -gt 0 ] && [ "$history" -gt 0 ] && [ "$current" -gt 0 ] || fail "publication evidence is incomplete: $value"
  printf '%s' "$value"
}

before="$(publication_counts)"
rc run --rm --no-deps research-worker --once --date 2020-01-31 || fail 'second research-worker one-shot failed'
after="$(publication_counts)"
[ "$before" = "$after" ] || fail "idempotency failed: counts changed from $before to $after"
echo "RESEARCH_WORKER_SMOKE: functional PASS ($after)"
