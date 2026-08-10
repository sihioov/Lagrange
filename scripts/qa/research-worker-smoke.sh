#!/usr/bin/env bash
# Static and functional smoke test for the research-worker Compose service.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
compose_file="$root/deploy/compose/compose.yml"
dockerfile="$root/data-pipelines/collectors/Dockerfile"
dockerignore="$root/.dockerignore"
secret_example="$root/deploy/secrets/db_research_password.example"
static_only="${LAGRANGE_RESEARCH_SMOKE_STATIC_ONLY:-0}"
self_test=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --static-only) static_only=1; shift ;;
    --self-test) self_test=1; shift ;;
    *) echo "USAGE: $0 [--static-only] [--self-test]" >&2; exit 2 ;;
  esac
done

fail() { echo "RESEARCH_WORKER_SMOKE: $*" >&2; exit 1; }
contains() { printf '%s' "$1" | grep -Fq -- "$2" || fail "$3 missing required value: $2"; }

validator_self_tests() (
  test_root="$(mktemp -d "$root/.research-validator.XXXXXX")"
  trap 'rm -rf -- "$test_root"' EXIT
  mkdir -p "$test_root/scripts/qa" "$test_root/deploy/compose" "$test_root/deploy/secrets" "$test_root/data-pipelines/collectors"
  cp "$BASH_SOURCE" "$test_root/scripts/qa/research-worker-smoke.sh"
  cp "$compose_file" "$test_root/deploy/compose/compose.yml"
  cp "$dockerfile" "$test_root/data-pipelines/collectors/Dockerfile"
  if [ -f "$dockerignore" ]; then cp "$dockerignore" "$test_root/.dockerignore"; fi
  cp "$root/deploy/secrets/.gitignore" "$root/deploy/secrets/README.md" "$secret_example" "$test_root/deploy/secrets/"
  git -C "$test_root" init -q
  git -C "$test_root" add -f -- deploy/secrets

  test_script="$test_root/scripts/qa/research-worker-smoke.sh"
  test_compose="$test_root/deploy/compose/compose.yml"
  test_dockerfile="$test_root/data-pipelines/collectors/Dockerfile"
  bash "$test_script" --static-only >/dev/null 2>&1 || fail 'self-test baseline fixture must pass'

  cp "$test_compose" "$test_compose.baseline"
  sed 's#${LAGRANGE_DATA_DIR:-../data}/raw:/data/raw#${LAGRANGE_DATA_DIR:-../data}/raw:/data/raw:ro#' "$test_compose.baseline" >"$test_compose"
  if bash "$test_script" --static-only >/dev/null 2>&1; then fail 'validator accepted a read-only Raw mount'; fi
  cp "$test_compose.baseline" "$test_compose"

  for mutation in DB_USER entrypoint healthcheck; do
    case "$mutation" in
      DB_USER) sed 's/^      DB_USER: research_writer$/      # DB_USER: research_writer/' "$test_compose.baseline" >"$test_compose" ;;
      entrypoint) sed 's|^    entrypoint: \["/usr/local/bin/research-worker"\]$|    # entrypoint: ["/usr/local/bin/research-worker"]|' "$test_compose.baseline" >"$test_compose" ;;
      healthcheck) sed 's|^      test: \["CMD", "/usr/local/bin/research-worker", "healthcheck"\]$|      # test: ["CMD", "/usr/local/bin/research-worker", "healthcheck"]|' "$test_compose.baseline" >"$test_compose" ;;
    esac
    if bash "$test_script" --static-only >/dev/null 2>&1; then fail "validator accepted commented-out $mutation"; fi
  done
  awk '
    { print }
    /\$\{LAGRANGE_DATA_DIR:-\.\.\/data\}\/raw:\/data\/raw$/ {
      print "      - type: bind"
      print "        source: ${LAGRANGE_DATA_DIR:-../data}/curated"
      print "        target: /data/curated"
      print "        read_only: false"
    }
  ' "$test_compose.baseline" >"$test_compose"
  if bash "$test_script" --static-only >/dev/null 2>&1; then fail 'validator accepted a writable long-syntax curated mount'; fi
  cp "$test_compose.baseline" "$test_compose"

  cp "$test_dockerfile" "$test_dockerfile.baseline"
  awk '!changed && /^FROM / { sub(/^FROM /, "from "); changed=1 } { print }' "$test_dockerfile.baseline" >"$test_dockerfile"
  bash "$test_script" --static-only >/dev/null 2>&1 || fail 'validator rejected a lowercase digest-pinned FROM'
  awk '!changed && /^from[[:space:]]+rust:1\.97\.1-alpine@sha256:[0-9a-f]{64}/ { sub(/@sha256:[0-9a-f]{64}/, ""); changed=1 } { print }' "$test_dockerfile" >"$test_dockerfile.unpinned"
  mv "$test_dockerfile.unpinned" "$test_dockerfile"
  if bash "$test_script" --static-only >/dev/null 2>&1; then fail 'validator accepted a lowercase unpinned FROM'; fi
  echo 'RESEARCH_WORKER_SMOKE: validator self-test PASS'
)

if [ "$self_test" -eq 1 ]; then
  validator_self_tests
  exit 0
fi

[ -f "$compose_file" ] || fail "missing Compose file: $compose_file"
command -v python3 >/dev/null 2>&1 || fail 'python3 is required for semantic Compose validation'

compose_config_json() {
  if command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
    docker compose -f "$compose_file" config --format json
  elif command -v powershell.exe >/dev/null 2>&1 && command -v wslpath >/dev/null 2>&1; then
    compose_windows="$(wslpath -w "$compose_file")"
    powershell.exe -NoProfile -NonInteractive -Command "& docker compose -f '$compose_windows' config --format json"
  else
    fail 'Docker Compose CLI is required for semantic static validation'
  fi
}

validate_compose() (
  compose_json="$(mktemp "${TMPDIR:-/tmp}/lagrange-compose.XXXXXX.json")"
  trap 'rm -f -- "$compose_json"' EXIT
  compose_config_json >"$compose_json" || fail 'docker compose config failed during static validation'
  python3 - "$compose_json" "$root" <<'PY'
import json
import pathlib
import re
import sys

def require(condition, message):
    if not condition:
        raise SystemExit(f"RESEARCH_WORKER_SMOKE: {message}")

with open(sys.argv[1], encoding="utf-8") as stream:
    model = json.load(stream)
def normalized_path(value):
    text = str(value).replace("\\", "/")
    if re.match(r"^[A-Za-z]:/", text):
        text = f"/mnt/{text[0].lower()}/{text[3:]}"
    return pathlib.Path(text).resolve()

root = normalized_path(sys.argv[2])
services = model.get("services", {})
for name in ("research-worker", "research-raw-init", "research-schema-check"):
    require(name in services, f"Compose service is missing: {name}")

worker = services["research-worker"]
build = worker.get("build") or {}
require(normalized_path(build.get("context", "")) == root, "research-worker build context does not resolve to the repository root")
require(build.get("dockerfile") == "data-pipelines/collectors/Dockerfile", "research-worker Dockerfile is incorrect")
require(worker.get("entrypoint") == ["/usr/local/bin/research-worker"], "research-worker entrypoint is incorrect")
expected_env = {
    "APP_ENV": "development", "RESEARCH_FETCH_MODE": "synthetic",
    "RESEARCH_RUN_AT_KST": "16:30", "RESEARCH_MAX_PUBLICATION_AGE_SECS": "345600",
    "RESEARCH_RAW_ROOT": "/data/raw", "DB_HOST": "postgres", "DB_PORT": "5432",
    "DB_NAME": "lagrange", "DB_USER": "research_writer",
    "DB_PASSWORD_FILE": "/run/secrets/db_research_password",
}
environment = worker.get("environment") or {}
for key, value in expected_env.items():
    require(environment.get(key) == value, f"research-worker environment is incorrect: {key}")
worker_secrets = {item.get("source") for item in worker.get("secrets", [])}
require({"db_research_password", "krx_api_key"}.issubset(worker_secrets), "research-worker secrets are incomplete")
require((worker.get("healthcheck") or {}).get("test") == ["CMD", "/usr/local/bin/research-worker", "healthcheck"], "research-worker healthcheck is incorrect")
for dependency, condition in {
    "postgres": "service_healthy",
    "research-raw-init": "service_completed_successfully",
    "research-schema-check": "service_completed_successfully",
}.items():
    require((worker.get("depends_on", {}).get(dependency) or {}).get("condition") == condition, f"research-worker dependency is incorrect: {dependency}")

volumes = worker.get("volumes", [])
raw = [volume for volume in volumes if volume.get("target") == "/data/raw"]
require(len(raw) == 1 and raw[0].get("type") == "bind" and not raw[0].get("read_only", False), "research-worker must have exactly one read/write bind targeting /data/raw")
for volume in volumes:
    target = volume.get("target", "")
    require(not (target.startswith("/data/") and target != "/data/raw" and not volume.get("read_only", False)), f"research-worker data mount is writable: {target}")

raw_init = services["research-raw-init"]
alpine = "alpine@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d"
init_raw = [volume for volume in raw_init.get("volumes", []) if volume.get("target") == "/data/raw"]
require(raw_init.get("image") == alpine and raw_init.get("user") == "0:0" and raw_init.get("read_only") is True, "research-raw-init identity/root filesystem is incorrect")
require(raw_init.get("network_mode") == "none" and raw_init.get("restart") == "no" and "secrets" not in raw_init and "networks" not in raw_init, "research-raw-init isolation is incorrect")
require(len(init_raw) == 1 and not init_raw[0].get("read_only", False) and init_raw[0].get("source") == raw[0].get("source"), "research-raw-init Raw mount is incorrect")
init_command = " ".join(raw_init.get("command", []))
require("chown 10001:10001 /data/raw" in init_command and "chmod 0750 /data/raw" in init_command, "research-raw-init command is incorrect")

schema = services["research-schema-check"]
postgres = "postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a"
schema_secrets = {item.get("source") for item in schema.get("secrets", [])}
require(schema.get("image") == postgres and schema.get("read_only") is True and schema.get("restart") == "no", "research-schema-check runtime contract is incorrect")
require((schema.get("depends_on", {}).get("postgres") or {}).get("condition") == "service_healthy" and "postgres_password" in schema_secrets, "research-schema-check dependency/secret is incorrect")
schema_command = "\n".join(schema.get("command", []))
for required in ("data_batches", "trading_calendar_versions", "trading_calendars", "data_batches_source_file_uq", "trading_calendar_versions_source_lookup_idx", "research_writer"):
    require(required in schema_command, f"research-schema-check command is missing: {required}")

for identity in ("db_app_password", "db_worker_password", "db_audit_password", "db_research_password"):
    require(identity in model.get("secrets", {}), f"Compose secret identity is missing: {identity}")
resolved = json.dumps(model)
require(not re.search(r"\blagrange_(app|worker)\b", resolved), "legacy Compose DB role spelling remains")
PY
)
validate_compose

[ -f "$dockerfile" ] || fail "missing worker Dockerfile: $dockerfile"
[ -f "$dockerignore" ] || fail "missing Docker build-context policy: $dockerignore"
[ -f "$secret_example" ] || fail "missing research DB secret example: $secret_example"
docker_text="$(<"$dockerfile")"
printf '%s\n' "$docker_text" | grep -Eiq '^FROM[[:space:]]+rust:1\.97\.1-alpine@sha256:3c38f3f82c2f3d73da3b38e18d279393a04cb43ddded0e35088a8c3324d40900[[:space:]]+AS[[:space:]]+builder[[:space:]]*$' || fail 'Dockerfile missing the approved digest-pinned Rust builder'
printf '%s\n' "$docker_text" | grep -Eiq '^FROM[[:space:]]+alpine:3\.21@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d[[:space:]]*$' || fail 'Dockerfile missing the approved digest-pinned Alpine runtime'
contains "$docker_text" 'cargo build --locked --release --package collectors --bin research-worker' 'Dockerfile'
contains "$docker_text" 'ENTRYPOINT ["/usr/local/bin/research-worker"]' 'Dockerfile'
from_count=0
while IFS= read -r line; do
  from_count=$((from_count + 1))
  printf '%s\n' "$line" | grep -Eiq '^FROM[[:space:]]+[^[:space:]]+@sha256:[0-9a-f]{64}([[:space:]]+AS[[:space:]]+[A-Za-z0-9._-]+)?$' || fail "Dockerfile FROM is not immutable: $line"
done < <(grep -i '^FROM[[:space:]]' "$dockerfile" || true)
[ "$from_count" -gt 0 ] || fail 'Dockerfile has no FROM instructions'

for pattern in '**' '!Cargo.toml' '!Cargo.lock' '!rust-toolchain.toml' '!crates/**' '!data-pipelines/collectors/**' '!apps/api-server/auth/**' '!tests/integration/migration-contract/**' '!tests/fixtures/kr-etf/contract/**' '**/target/**' '**/.git/**' '**/.worktrees/**' '**/.env.*' '**/credentials/**' '**/secrets/**' '**/raw/**'; do
  grep -Fxq -- "$pattern" "$dockerignore" || fail "Docker build-context policy is missing: $pattern"
done

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
context_audit_tag="${project}-context-audit"
cleanup() {
  if [ "$created" -eq 1 ]; then rc down -v --remove-orphans --rmi local >/dev/null 2>&1 || true; fi
  dkr image rm -f "$context_audit_tag" >/dev/null 2>&1 || true
  rm -rf -- "$temp_root"
}
trap cleanup EXIT

install -d -m 0700 "$raw_root/raw"
[ -d "$raw_root/raw" ] && [ -w "$raw_root/raw" ] || fail 'disposable Raw directory is not writable'
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
audit_root="$temp_root/context-audit"
audit_dockerfile="$temp_root/context-audit.Dockerfile"
install -d "$audit_root/target" "$audit_root/credentials" "$audit_root/secrets" "$audit_root/data/raw"
cp "$dockerignore" "$audit_root/.dockerignore"
printf '%s\n' '[workspace]' >"$audit_root/Cargo.toml"
printf '%s' 'sentinel-not-a-secret' >"$audit_root/.env"
printf '%s' 'must-not-enter-context' >"$audit_root/target/sentinel"
printf '%s' 'must-not-enter-context' >"$audit_root/credentials/sentinel"
printf '%s' 'must-not-enter-context' >"$audit_root/secrets/sentinel"
printf '%s' 'must-not-enter-context' >"$audit_root/data/raw/sentinel"
cat >"$audit_dockerfile" <<'DOCKERFILE'
FROM alpine:3.21@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d
COPY . /context
RUN test -f /context/Cargo.toml \
 && test ! -e /context/.env \
 && test ! -e /context/target/sentinel \
 && test ! -e /context/credentials/sentinel \
 && test ! -e /context/secrets/sentinel \
 && test ! -e /context/data/raw/sentinel
DOCKERFILE
dkr build --no-cache -q -t "$context_audit_tag" -f "$(hostpath "$audit_dockerfile")" "$(hostpath "$audit_root")" >/dev/null || fail 'Docker build-context sentinel audit failed'
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
rc run --rm --no-deps research-raw-init || fail 'research-raw-init failed'
rc run --rm --no-deps --entrypoint /bin/sh --user 10001:10001 research-worker -c 'probe="$RESEARCH_RAW_ROOT/.qa-write-probe"; : > "$probe"; rm -f "$probe"' || fail 'research-worker UID 10001 cannot write the Raw bind mount'
rc run --rm --no-deps research-schema-check || fail 'research-schema-check rejected the migrated database'
rc up -d research-worker >/dev/null || fail 'research-worker service failed to start'
rc run --rm --no-deps research-worker --once --date 2020-01-31 || fail 'first research-worker one-shot failed'

healthy=0
for _ in $(seq 1 30); do
  if dkr compose -p "$project" -f "$(hostpath "$compose_file")" exec -T research-worker /usr/local/bin/research-worker healthcheck >/dev/null 2>&1; then healthy=1; break; fi
  sleep 1
done
[ "$healthy" -eq 1 ] || fail 'research-worker did not become functionally healthy'

publication_evidence() {
  local value
  value="$(dkr compose -p "$project" -f "$(hostpath "$compose_file")" exec -T postgres psql -X -qAt -v ON_ERROR_STOP=1 -U lagrange -d lagrange <<'SQL'
WITH source AS (
  SELECT source_batch_id AS id FROM data_batches
  WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31'
  LIMIT 1
)
SELECT concat_ws('|',
  (SELECT count(DISTINCT source_batch_id) FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31' AND source_batch_id IS NOT NULL),
  (SELECT count(*) FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31'),
  (SELECT count(*) FROM trading_calendar_versions WHERE exchange = 'KRX'),
  (SELECT count(*) FROM trading_calendars WHERE exchange = 'KRX'),
  (SELECT string_agg(DISTINCT kind, ',' ORDER BY kind) FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31'),
  (SELECT string_agg(to_char(session_date, 'YYYY-MM-DD') || ':' || session_type, ',' ORDER BY session_date) FROM trading_calendar_versions WHERE exchange = 'KRX'),
  (SELECT string_agg(to_char(session_date, 'YYYY-MM-DD') || ':' || session_type, ',' ORDER BY session_date) FROM trading_calendars WHERE exchange = 'KRX'),
  (SELECT bool_and(v.source_batch_id = source.id) FROM trading_calendar_versions v CROSS JOIN source WHERE v.exchange = 'KRX'),
  (SELECT bool_and(c.source_batch_id = source.id) FROM trading_calendars c CROSS JOIN source WHERE c.exchange = 'KRX')
) FROM source;
SQL
)"
  expected='1|4|2|2|CALENDAR,CORPORATE_ACTIONS,EOD,REFERENCE|2020-01-30:TRADING,2020-01-31:TRADING|2020-01-30:TRADING,2020-01-31:TRADING|t|t'
  [ "$value" = "$expected" ] || fail "publication evidence mismatch: $value"
  printf '%s' "$value"
}

before="$(publication_evidence)"
rc run --rm --no-deps research-worker --once --date 2020-01-31 || fail 'second research-worker one-shot failed'
after="$(publication_evidence)"
[ "$before" = "$after" ] || fail "idempotency failed: counts changed from $before to $after"
echo "RESEARCH_WORKER_SMOKE: functional PASS ($after)"
