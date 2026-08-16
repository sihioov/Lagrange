#!/usr/bin/env bash
# Static and functional smoke test for the research-worker Compose service.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
compose_file="$root/deploy/compose/compose.yml"
dockerfile="$root/data-pipelines/collectors/Dockerfile"
dockerignore="$root/.dockerignore"
gitattributes="$root/.gitattributes"
schema_sql="$root/deploy/compose/research-schema-check.sql"
secret_example="$root/deploy/secrets/db_research_password.example"
read_only_fsync_probe="$root/scripts/qa/read-only-fsync.rs"
static_only="${LAGRANGE_RESEARCH_SMOKE_STATIC_ONLY:-0}"
self_test=0
# Compose production requires this immutable build input. Static/self-test
# fixtures use a deterministic placeholder only when the caller did not
# provide one; this does not relax the production Compose contract.
static_commit="${LAGRANGE_CODE_COMMIT:-0123456789abcdef0123456789abcdef01234567}"
export LAGRANGE_CODE_COMMIT="$static_commit"
export RESEARCH_APP_ENV=qa
export RESEARCH_FETCH_MODE=synthetic
# The research smoke resolves the complete production Compose model even
# though it starts only research services. Supply deterministic QA values for
# the independent backtest capacity/reconciler contract so interpolation
# remains fail-closed in production and reproducible here.
export BACKTEST_MIN_FREE_BYTES="${BACKTEST_MIN_FREE_BYTES:-1073741824}"
export BACKTEST_MAX_QUEUED_BACKTESTS="${BACKTEST_MAX_QUEUED_BACKTESTS:-1000}"
export BACKTEST_RECONCILE_GRACE_SECS="${BACKTEST_RECONCILE_GRACE_SECS:-900}"
export BACKTEST_RECONCILE_INTERVAL_SECS="${BACKTEST_RECONCILE_INTERVAL_SECS:-60}"

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
  cp "$read_only_fsync_probe" "$test_root/scripts/qa/read-only-fsync.rs"
  cp "$compose_file" "$test_root/deploy/compose/compose.yml"
  cp "$dockerfile" "$test_root/data-pipelines/collectors/Dockerfile"
  cp "$schema_sql" "$test_root/deploy/compose/research-schema-check.sql"
  if [ -f "$dockerignore" ]; then cp "$dockerignore" "$test_root/.dockerignore"; fi
  if [ -f "$gitattributes" ]; then cp "$gitattributes" "$test_root/.gitattributes"; fi
  cp "$root/deploy/secrets/.gitignore" "$root/deploy/secrets/README.md" "$secret_example" "$test_root/deploy/secrets/"
  git -C "$test_root" init -q
  git -C "$test_root" add -f -- deploy/secrets
  git -C "$test_root" config core.autocrlf true
  sed 's/\r$//; s/$/\r/' "$test_root/scripts/qa/research-worker-smoke.sh" >"$test_root/scripts/qa/research-worker-smoke.sh.crlf"
  mv "$test_root/scripts/qa/research-worker-smoke.sh.crlf" "$test_root/scripts/qa/research-worker-smoke.sh"
  git -C "$test_root" add -- .gitattributes scripts/qa/research-worker-smoke.sh
  git -C "$test_root" -c user.name=validator -c user.email=validator@example.invalid commit -q -m 'checkout fixture'
  rm "$test_root/scripts/qa/research-worker-smoke.sh"
  git -C "$test_root" checkout -q -- scripts/qa/research-worker-smoke.sh
  if grep -q $'\r' "$test_root/scripts/qa/research-worker-smoke.sh"; then fail '.gitattributes did not preserve LF under a simulated autocrlf checkout'; fi

  test_script="$test_root/scripts/qa/research-worker-smoke.sh"
  test_compose="$test_root/deploy/compose/compose.yml"
  test_dockerfile="$test_root/data-pipelines/collectors/Dockerfile"
  bash "$test_script" --static-only >/dev/null 2>&1 || fail 'self-test baseline fixture must pass'

  cp "$test_compose" "$test_compose.baseline"
  sed 's#${LAGRANGE_DATA_DIR:-../data}/raw:/data/raw#${LAGRANGE_DATA_DIR:-../data}/raw:/data/raw:ro#' "$test_compose.baseline" >"$test_compose"
  if bash "$test_script" --static-only >/dev/null 2>&1; then fail 'validator accepted a read-only Raw mount'; fi
  cp "$test_compose.baseline" "$test_compose"

  sed 's/File::open(&path)/OpenOptions::new().write(true).open(&path)/' "$read_only_fsync_probe" >"$test_root/scripts/qa/read-only-fsync.rs"
  if bash "$test_script" --static-only >/dev/null 2>&1; then fail 'validator accepted a write-opening Raw fsync probe'; fi
  cp "$read_only_fsync_probe" "$test_root/scripts/qa/read-only-fsync.rs"

  printf '%s\n' '!scripts/**' >>"$test_root/.dockerignore"
  if bash "$test_script" --static-only >/dev/null 2>&1; then fail 'validator accepted the QA fsync probe in the worker build context'; fi
  cp "$dockerignore" "$test_root/.dockerignore"

  sed 's#find /data/raw -xdev -type d#find -L /data/raw -type l#g' "$test_compose.baseline" >"$test_compose"
  if bash "$test_script" --static-only >/dev/null 2>&1; then fail 'validator accepted a symlink-following Raw init'; fi
  cp "$test_compose.baseline" "$test_compose"

  cp "$test_root/deploy/compose/research-schema-check.sql" "$test_root/deploy/compose/research-schema-check.sql.baseline"
  sed 's/has_sequence_privilege/has_sequence_permission/' "$test_root/deploy/compose/research-schema-check.sql.baseline" >"$test_root/deploy/compose/research-schema-check.sql"
  if bash "$test_script" --static-only >/dev/null 2>&1; then fail 'validator accepted a weakened schema gate'; fi
  cp "$test_root/deploy/compose/research-schema-check.sql.baseline" "$test_root/deploy/compose/research-schema-check.sql"

  printf '%s\n' 'scripts/qa/*.sh text eol=crlf' >"$test_root/.gitattributes"
  if bash "$test_script" --static-only >/dev/null 2>&1; then fail 'validator accepted CRLF shell checkout semantics'; fi
  cp "$gitattributes" "$test_root/.gitattributes"

  for mutation in DB_USER entrypoint healthcheck raw-init-caps schema-user schema-caps; do
    case "$mutation" in
      DB_USER) sed 's/^      DB_USER: research_writer$/      # DB_USER: research_writer/' "$test_compose.baseline" >"$test_compose" ;;
      entrypoint) sed 's|^    entrypoint: \["/usr/local/bin/research-worker"\]$|    # entrypoint: ["/usr/local/bin/research-worker"]|' "$test_compose.baseline" >"$test_compose" ;;
      healthcheck) sed 's|^      test: \["CMD", "/usr/local/bin/research-worker", "healthcheck"\]$|      # test: ["CMD", "/usr/local/bin/research-worker", "healthcheck"]|' "$test_compose.baseline" >"$test_compose" ;;
      raw-init-caps) sed '0,/^      - DAC_OVERRIDE$/s//      - SETUID/' "$test_compose.baseline" >"$test_compose" ;;
      schema-user) sed 's/^    user: "999:999"$/    user: "0:0"/' "$test_compose.baseline" >"$test_compose" ;;
      schema-caps) awk '/^      - ALL$/ { count++; if (count == 2) sub(/ALL/, "CHOWN") } { print }' "$test_compose.baseline" >"$test_compose" ;;
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
    # Static and functional smoke coverage is an explicit QA fixture run;
    # production Compose itself requires RESEARCH_APP_ENV from the operator.
    LAGRANGE_CODE_COMMIT="$static_commit" \
    RESEARCH_APP_ENV=qa \
    RESEARCH_FETCH_MODE=synthetic \
      docker compose -f "$compose_file" config --format json
  elif command -v powershell.exe >/dev/null 2>&1 && command -v wslpath >/dev/null 2>&1; then
    compose_windows="$(wslpath -w "$compose_file")"
    LAGRANGE_CODE_COMMIT="$static_commit" RESEARCH_APP_ENV=qa RESEARCH_FETCH_MODE=synthetic \
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
    "APP_ENV": "qa", "RESEARCH_FETCH_MODE": "synthetic",
    "RESEARCH_RUN_AT_KST": "16:30", "RESEARCH_MAX_PUBLICATION_AGE_SECS": "345600",
    "RESEARCH_RAW_ROOT": "/data", "DB_HOST": "postgres", "DB_PORT": "5432",
    "DB_NAME": "lagrange", "DB_USER": "research_writer",
    "DB_PASSWORD_FILE": "/run/secrets/db_research_password",
}
environment = worker.get("environment") or {}
for key, value in expected_env.items():
    require(environment.get(key) == value, f"research-worker environment is incorrect: {key}")
worker_secrets = {item.get("source") for item in worker.get("secrets", [])}
require({"research_db_research_password", "research_krx_api_key"}.issubset(worker_secrets), "research-worker secrets are incomplete")
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
require(raw_init.get("cap_drop") == ["ALL"] and sorted(raw_init.get("cap_add", [])) == ["CHOWN", "DAC_OVERRIDE", "FOWNER"], "research-raw-init capability contract is incorrect")
require("no-new-privileges:true" in raw_init.get("security_opt", []), "research-raw-init no-new-privileges contract is missing")
require(len(init_raw) == 1 and not init_raw[0].get("read_only", False) and init_raw[0].get("source") == raw[0].get("source"), "research-raw-init Raw mount is incorrect")
init_command = " ".join(raw_init.get("command", []))
require("find /data/raw -xdev -type d" in init_command and "find /data/raw -xdev -type f" in init_command, "research-raw-init must recurse without crossing filesystems")
require("manifest.jsonl" in init_command and "commit.lock" in init_command, "research-raw-init mutable-file contract is missing")
require("chown 10001:10001" in init_command and all(mode in init_command for mode in ("chmod 0750", "chmod 0640", "chmod 0440")), "research-raw-init ownership/mode contract is missing")
require(not re.search(r"(^|\s)-L(\s|$)", init_command) and "-type l" not in init_command, "research-raw-init must never follow or mutate symlinks")

schema = services["research-schema-check"]
postgres = "postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a"
schema_secrets = {item.get("source") for item in schema.get("secrets", [])}
schema_volumes = [volume for volume in schema.get("volumes", []) if volume.get("target") == "/opt/lagrange/research-schema-check.sql"]
require(schema.get("image") == postgres and schema.get("read_only") is True and schema.get("restart") == "no", "research-schema-check runtime contract is incorrect")
require(schema.get("user") == "999:999" and "ALL" in schema.get("cap_drop", []) and "no-new-privileges:true" in schema.get("security_opt", []), "research-schema-check user/capability contract is incorrect")
require((schema.get("depends_on", {}).get("postgres") or {}).get("condition") == "service_healthy" and schema_secrets == {"schema_check_postgres_password"}, "research-schema-check dependency/secret is incorrect")
require(len(schema_volumes) == 1 and schema_volumes[0].get("read_only") is True, "research-schema-check SQL mount is incorrect")
schema_command = "\n".join(schema.get("command", []))
require("/opt/lagrange/research-schema-check.sql" in schema_command, "research-schema-check command does not execute tracked SQL")

for identity in ("api_db_app_password", "recommendation_db_worker_password", "api_db_audit_password", "research_db_research_password"):
    require(identity in model.get("secrets", {}), f"Compose secret identity is missing: {identity}")
resolved = json.dumps(model)
require(not re.search(r"\blagrange_(app|worker)\b", resolved), "legacy Compose DB role spelling remains")
PY
)
validate_compose

[ -f "$dockerfile" ] || fail "missing worker Dockerfile: $dockerfile"
[ -f "$dockerignore" ] || fail "missing Docker build-context policy: $dockerignore"
[ -f "$schema_sql" ] || fail "missing tracked schema gate: $schema_sql"
[ -f "$gitattributes" ] || fail "missing .gitattributes"
[ -f "$read_only_fsync_probe" ] || fail "missing read-only fsync probe: $read_only_fsync_probe"
[ -f "$secret_example" ] || fail "missing research DB secret example: $secret_example"
probe_text="$(<"$read_only_fsync_probe")"
contains "$probe_text" 'File::open(&path)' 'read-only fsync probe'
contains "$probe_text" 'file.sync_all()' 'read-only fsync probe'
if printf '%s\n' "$probe_text" | grep -Eq 'OpenOptions|\.write[[:space:]]*\('; then fail 'read-only fsync probe must not request write access'; fi
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

for pattern in '**' '!Cargo.toml' '!Cargo.lock' '!rust-toolchain.toml' '!crates/**' '!data-pipelines/collectors/**' '!apps/api-server/auth/**' '!tests/integration/migration-contract/**' '!tests/fixtures/kr-etf/contract/**' '**/target/**' '**/.git/**' '**/.worktrees/**' '**/.env.*' '**/credentials/**' '**/secrets/**' '**/raw/**' '**/*.pem' '**/*.key' '**/*.p12' '**/*.pfx'; do
  grep -Fxq -- "$pattern" "$dockerignore" || fail "Docker build-context policy is missing: $pattern"
done
if grep -Eq '^!scripts(/|$)' "$dockerignore"; then fail 'QA fsync probe must remain outside the worker build context'; fi
grep -Eq '^scripts/qa/\*\.sh[[:space:]]+text[[:space:]]+eol=lf[[:space:]]*$' "$gitattributes" || fail 'scripts/qa shell scripts must be forced to LF by .gitattributes'
schema_text="$(<"$schema_sql")"
for token in _sqlx_migrations 'version IN (22, 23, 24, 25, 33, 34, 35)' convalidated \
  pg_get_constraintdef format_type attnotnull attidentity pg_get_expr storage_path EXCEPT \
  data_batches_source_file_uq trading_calendar_versions_source_lookup_idx \
  indisunique indisvalid indisready indislive relrowsecurity research_writer \
  rolcanlogin rolsuper rolbypassrls rolcreatedb rolcreaterole pg_auth_members \
  pg_policy polcmd polpermissive trading_calendar_versions_append_only \
  tgenabled tgtype prosecdef pg_get_functiondef regexp_replace actual_function expected_function \
  role_table_grants has_schema_privilege \
  has_table_privilege has_sequence_privilege lock_recommendation_source_pins MAINTAIN; do
  contains "$schema_text" "$token" 'research-schema-check SQL'
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
  case "$name" in
    README.md|.gitignore|*.example|provision-runtime-secrets.sh|runtime-static-check.sh) ;;
    *) fail "real secret-like file is tracked: $path" ;;
  esac
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
runtime_secret_root="$temp_root/runtime-secrets"
postgres_secret="$runtime_secret_root/postgres/postgres_password"
bootstrap_secret_root="$runtime_secret_root/db-role-bootstrap"
migrate_secret_root="$runtime_secret_root/db-migrate"
schema_postgres_secret="$runtime_secret_root/research-schema-check/postgres_password"
research_secret="$runtime_secret_root/research-worker/db_research_password"
krx_secret="$runtime_secret_root/research-worker/krx_api_key"
created=0
rc() { dkr compose -p "$project" -f "$(hostpath "$compose_file")" "$@"; }
context_audit_tag="${project}-context-audit"
cleanup() {
  if [ "$created" -eq 1 ]; then
    rc run --rm --no-deps --entrypoint /bin/sh --user 0:0 research-raw-init \
      -ec 'find /data/raw -mindepth 1 -delete' >/dev/null 2>&1 || true
    rc down -v --remove-orphans --rmi local >/dev/null 2>&1 || true
  fi
  dkr image rm -f "$context_audit_tag" >/dev/null 2>&1 || true
  rm -rf -- "$temp_root"
}
trap cleanup EXIT

raw_init_ownership_probe() (
  alpine='alpine@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d'
  rust_image='rust:1.97.1-alpine@sha256:3c38f3f82c2f3d73da3b38e18d279393a04cb43ddded0e35088a8c3324d40900'
  probe_id="${project}-raw-init"
  raw_volume="${probe_id}-raw"
  outside_volume="${probe_id}-outside"
  binary_volume="${probe_id}-binary"
  trap 'dkr volume rm -f "$raw_volume" "$outside_volume" "$binary_volume" >/dev/null 2>&1 || true' EXIT
  dkr volume create "$raw_volume" >/dev/null || fail 'Raw init probe volume creation failed'
  dkr volume create "$outside_volume" >/dev/null || fail 'Raw init outside volume creation failed'
  dkr volume create "$binary_volume" >/dev/null || fail 'Raw init fsync-probe volume creation failed'
  dkr run --rm --network none --user 0:0 -v "${raw_volume}:/data/raw" -v "${outside_volume}:/outside" "$alpine" /bin/sh -ec '
    mkdir -p /data/raw/manifests/provider=krx/market=kr /data/raw/provider=krx/market=kr/date=2020-01-31/batch=fixture
    printf "{}\n" > /data/raw/manifests/provider=krx/market=kr/manifest.jsonl
    : > /data/raw/manifests/provider=krx/market=kr/commit.lock
    printf evidence > /data/raw/provider=krx/market=kr/date=2020-01-31/batch=fixture/eod.json
    chown -R 12345:12345 /data/raw
    find /data/raw -type d -exec chmod 0700 {} +
    chmod 0600 /data/raw/manifests/provider=krx/market=kr/manifest.jsonl /data/raw/manifests/provider=krx/market=kr/commit.lock /data/raw/provider=krx/market=kr/date=2020-01-31/batch=fixture/eod.json
    printf outside > /outside/sentinel
    chown 12345:12345 /outside/sentinel
    chmod 0600 /outside/sentinel
    ln -s /outside /data/raw/outside-link
  ' >/dev/null || fail 'Raw init ownership fixture setup failed'
  init_command="$(rc config --format json | python3 -c 'import json,sys; value=json.load(sys.stdin)["services"]["research-raw-init"]["command"]; print("\n".join(value) if isinstance(value,list) else value)')" || fail 'Raw init command extraction failed'
  dkr run --rm --network none --user 0:0 --cap-drop ALL --cap-add CHOWN --cap-add FOWNER --cap-add DAC_OVERRIDE --security-opt no-new-privileges:true -v "${raw_volume}:/data/raw" -v "${outside_volume}:/outside" "$alpine" /bin/sh -ec "$init_command" || fail 'recursive Raw init probe failed'
  dkr run --rm --network none --user 0:0 --cap-drop ALL --security-opt no-new-privileges:true -v "$(hostpath "$root"):/source:ro" -v "${binary_volume}:/probe" "$rust_image" rustc -O -o /probe/read-only-fsync /source/scripts/qa/read-only-fsync.rs || fail 'read-only fsync probe compilation failed'
  dkr run --rm --network none --user 10001:10001 --cap-drop ALL --security-opt no-new-privileges:true -v "${raw_volume}:/data/raw" -v "${outside_volume}:/outside" -v "${binary_volume}:/probe:ro" "$alpine" /bin/sh -ec '
    evidence=/data/raw/provider=krx/market=kr/date=2020-01-31/batch=fixture/eod.json
    manifest=/data/raw/manifests/provider=krx/market=kr/manifest.jsonl
    lock=/data/raw/manifests/provider=krx/market=kr/commit.lock
    test "$(stat -c "%u:%g:%a" "$evidence")" = 10001:10001:440
    test "$(stat -c "%u:%g:%a" "$manifest")" = 10001:10001:640
    test "$(stat -c "%u:%g:%a" "$lock")" = 10001:10001:640
    /probe/read-only-fsync "$evidence"
    printf recovered >> /data/raw/manifests/provider=krx/market=kr/manifest.jsonl
    printf lock >> /data/raw/manifests/provider=krx/market=kr/commit.lock
  ' || fail 'UID 10001 cannot use existing Raw files'
  dkr run --rm --network none --user 0:0 -v "${outside_volume}:/outside" "$alpine" /bin/sh -ec '
    test "$(cat /outside/sentinel)" = outside
    test "$(stat -c "%u:%g:%a" /outside/sentinel)" = 12345:12345:600
  ' || fail 'Raw init changed the outside symlink target'
)

install -d -m 0700 \
  "$raw_root/raw" \
  "$runtime_secret_root/postgres" \
  "$bootstrap_secret_root" \
  "$migrate_secret_root" \
  "$runtime_secret_root/research-schema-check" \
  "$runtime_secret_root/research-worker"
[ -d "$raw_root/raw" ] && [ -w "$raw_root/raw" ] || fail 'disposable Raw directory is not writable'
umask 077
if command -v openssl >/dev/null 2>&1; then
  openssl rand -base64 32 >"$postgres_secret"
  openssl rand -base64 32 >"$research_secret"
else
  head -c 32 /dev/urandom | base64 >"$postgres_secret"
  head -c 32 /dev/urandom | base64 >"$research_secret"
fi
cp -- "$postgres_secret" "$schema_postgres_secret"
cp -- "$postgres_secret" "$bootstrap_secret_root/postgres_password"
for role_secret in db_migration_owner_password db_app_password db_worker_password \
  db_audit_password db_research_password db_admin_password; do
  cp -- "$research_secret" "$bootstrap_secret_root/$role_secret"
done
cp -- "$research_secret" "$migrate_secret_root/db_migration_owner_password"
printf '%s' 'unused-in-synthetic-smoke' >"$krx_secret"
find "$runtime_secret_root" -type f -exec chmod 0444 {} +

export LAGRANGE_RUNTIME_SECRET_DIR="$(hostpath "$runtime_secret_root")"
export LAGRANGE_DATA_DIR="$(hostpath "$raw_root")"
export LAGRANGE_PGDATA_VOLUME="${project}-pgdata"
export POSTGRES_USER=lagrange POSTGRES_DB=lagrange APP_ENV=qa RESEARCH_FETCH_MODE=synthetic
export RESEARCH_MAX_PUBLICATION_AGE_SECS=315576000
export RESEARCH_RUN_AT_KST="$(TZ=Asia/Seoul date -d '+12 hours' +%H:%M 2>/dev/null || TZ=Asia/Seoul date +%H:%M)"

created=1
raw_init_ownership_probe
audit_root="$temp_root/context-audit"
audit_dockerfile="$temp_root/context-audit.Dockerfile"
install -d "$audit_root/target" "$audit_root/credentials" "$audit_root/secrets" "$audit_root/data/raw" \
  "$audit_root/crates/sentinel" "$audit_root/data-pipelines/collectors/sentinel" \
  "$audit_root/apps/api-server/auth/sentinel" "$audit_root/tests/fixtures/kr-etf/contract/sentinel"
install -d "$audit_root/scripts/qa"
cp "$dockerignore" "$audit_root/.dockerignore"
printf '%s\n' '[workspace]' >"$audit_root/Cargo.toml"
printf '%s' 'sentinel-not-a-secret' >"$audit_root/.env"
printf '%s' 'must-not-enter-context' >"$audit_root/target/sentinel"
printf '%s' 'must-not-enter-context' >"$audit_root/credentials/sentinel"
printf '%s' 'must-not-enter-context' >"$audit_root/secrets/sentinel"
printf '%s' 'must-not-enter-context' >"$audit_root/data/raw/sentinel"
printf '%s' 'must-not-enter-context' >"$audit_root/crates/sentinel/context.pem"
printf '%s' 'must-not-enter-context' >"$audit_root/data-pipelines/collectors/sentinel/context.key"
printf '%s' 'must-not-enter-context' >"$audit_root/apps/api-server/auth/sentinel/context.p12"
printf '%s' 'must-not-enter-context' >"$audit_root/tests/fixtures/kr-etf/contract/sentinel/context.pfx"
cp "$read_only_fsync_probe" "$audit_root/scripts/qa/read-only-fsync.rs"
cat >"$audit_dockerfile" <<'DOCKERFILE'
FROM alpine:3.21@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d
COPY . /context
RUN test -f /context/Cargo.toml \
 && test ! -e /context/.env \
 && test ! -e /context/target/sentinel \
 && test ! -e /context/credentials/sentinel \
 && test ! -e /context/secrets/sentinel \
 && test ! -e /context/data/raw/sentinel \
 && test ! -e /context/crates/sentinel/context.pem \
 && test ! -e /context/data-pipelines/collectors/sentinel/context.key \
 && test ! -e /context/apps/api-server/auth/sentinel/context.p12 \
 && test ! -e /context/tests/fixtures/kr-etf/contract/sentinel/context.pfx \
 && test ! -e /context/scripts/qa/read-only-fsync.rs
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

if rc run --rm --no-deps research-schema-check >/dev/null 2>&1; then
  fail 'research-schema-check accepted an unmigrated database'
fi
dkr compose -p "$project" -f "$(hostpath "$compose_file")" exec -T postgres psql -X -q -v ON_ERROR_STOP=1 -U lagrange -d lagrange >/dev/null <<'SQL'
CREATE TABLE _sqlx_migrations (
  version bigint PRIMARY KEY,
  description text NOT NULL,
  installed_on timestamptz NOT NULL DEFAULT now(),
  success boolean NOT NULL,
  checksum bytea NOT NULL,
  execution_time bigint NOT NULL
);
SQL

while IFS= read -r migration; do
  transaction_args=(-1)
  if grep -Eq '^-- no-transaction[[:space:]]*$' "$migration"; then transaction_args=(); fi
  dkr compose -p "$project" -f "$(hostpath "$compose_file")" exec -T -e 'PGOPTIONS=-c lock_timeout=5s' postgres \
    psql -X -q -v ON_ERROR_STOP=1 "${transaction_args[@]}" -U lagrange -d lagrange <"$migration" >/dev/null || fail "migration failed: ${migration##*/}"
  migration_name="${migration##*/}"
  migration_base="${migration_name%.up.sql}"
  version=$((10#${migration_base:0:4}))
  description="${migration_base:5}"
  dkr compose -p "$project" -f "$(hostpath "$compose_file")" exec -T postgres psql -X -q -v ON_ERROR_STOP=1 -U lagrange -d lagrange \
    -c "INSERT INTO _sqlx_migrations(version, description, success, checksum, execution_time) VALUES ($version, '$description', true, decode(repeat('00', 32), 'hex'), 0)" </dev/null >/dev/null || fail "migration ledger insert failed: $migration_name"
done < <(find "$root/migrations" -maxdepth 1 -type f -name '*.up.sql' | sort)

ledger_state="$(
  dkr compose -p "$project" -f "$(hostpath "$compose_file")" exec -T postgres \
    psql -X -qAt -v ON_ERROR_STOP=1 -U lagrange -d lagrange \
    -c "SELECT count(*) FILTER (WHERE version IN (22, 23, 24, 25, 33, 34, 35) AND success) FROM public._sqlx_migrations"
)" || fail 'migration ledger verification query failed'
if [ "$ledger_state" != "7" ]; then
  fail "migration ledger mismatch after applying migrations: $ledger_state"
fi

schema_gate_must_pass() {
  local schema_output
  if ! schema_output="$(rc run --rm --no-deps research-schema-check 2>&1)"; then
    printf '%s\n' "$schema_output" >&2
    fail "research-schema-check rejected $1"
  fi
}
schema_gate_must_fail() {
  if rc run --rm --no-deps research-schema-check >/dev/null 2>&1; then fail "research-schema-check accepted $1"; fi
}
psql_admin() {
  dkr compose -p "$project" -f "$(hostpath "$compose_file")" exec -T postgres psql -X -q -v ON_ERROR_STOP=1 -U lagrange -d lagrange -c "$1" >/dev/null || fail "schema mutation command failed: $2"
}

schema_gate_must_pass 'the migrated database'
psql_admin 'ALTER TABLE data_batches DROP CONSTRAINT data_batches_fetch_mode_check; ALTER TABLE data_batches ADD CONSTRAINT data_batches_fetch_mode_check CHECK (true)' 'weaken publication CHECK'
schema_gate_must_fail 'a same-name weakened publication CHECK'
psql_admin "ALTER TABLE data_batches DROP CONSTRAINT data_batches_fetch_mode_check; ALTER TABLE data_batches ADD CONSTRAINT data_batches_fetch_mode_check CHECK (fetch_mode IS NULL OR fetch_mode IN ('synthetic', 'credentialed'))" 'restore publication CHECK'
schema_gate_must_pass 'the restored publication CHECK'
psql_admin 'ALTER TABLE data_batches DROP COLUMN storage_path' 'drop publication storage_path'
schema_gate_must_fail 'a dropped publication storage_path column'
psql_admin 'ALTER TABLE data_batches ADD COLUMN storage_path text NOT NULL' 'restore publication storage_path'
schema_gate_must_pass 'the restored publication storage_path column'
psql_admin 'DROP INDEX CONCURRENTLY data_batches_source_file_uq' 'drop source index'
psql_admin 'CREATE INDEX data_batches_source_file_uq ON data_batches (provider)' 'create drifted source index'
schema_gate_must_fail 'a drifted same-name index'
psql_admin 'DROP INDEX CONCURRENTLY data_batches_source_file_uq' 'drop drifted source index'
psql_admin 'CREATE UNIQUE INDEX CONCURRENTLY data_batches_source_file_uq ON data_batches (provider, market, source_batch_id, source_file_name) WHERE source_batch_id IS NOT NULL' 'restore source index'
schema_gate_must_pass 'the restored source index'
psql_admin 'DROP POLICY data_batches_insert_research_writer ON data_batches' 'drop research policy'
schema_gate_must_fail 'a missing research_writer policy'
psql_admin 'CREATE POLICY data_batches_insert_research_writer ON data_batches FOR INSERT TO research_writer WITH CHECK (true)' 'restore research policy'
schema_gate_must_pass 'the restored research_writer policy'
psql_admin 'ALTER TABLE trading_calendar_versions DISABLE TRIGGER trading_calendar_versions_append_only' 'disable append-only trigger'
schema_gate_must_fail 'a disabled append-only trigger'
psql_admin 'ALTER TABLE trading_calendar_versions ENABLE TRIGGER trading_calendar_versions_append_only' 'enable append-only trigger'
schema_gate_must_pass 'the restored append-only trigger'
psql_admin "CREATE OR REPLACE FUNCTION public.trading_calendar_versions_reject_mutation() RETURNS trigger LANGUAGE plpgsql AS \$fn\$ BEGIN IF false THEN RAISE EXCEPTION 'trading_calendar_versions is append-only: % is refused', TG_OP USING ERRCODE = '55000'; END IF; RETURN NULL; END \$fn\$" 'replace append-only function with message-preserving no-op'
schema_gate_must_fail 'a same-name message-preserving no-op append-only function'
psql_admin "CREATE OR REPLACE FUNCTION public.trading_calendar_versions_reject_mutation() RETURNS trigger LANGUAGE plpgsql AS \$fn\$ BEGIN RAISE EXCEPTION 'trading_calendar_versions is append-only: % is refused', TG_OP USING ERRCODE = '55000'; END \$fn\$" 'restore exact append-only function'
schema_gate_must_pass 'the restored exact append-only function'
psql_admin 'GRANT DELETE ON orders TO research_writer' 'grant forbidden order privilege'
schema_gate_must_fail 'a forbidden order-table grant'
psql_admin 'REVOKE DELETE ON orders FROM research_writer' 'revoke forbidden order privilege'
schema_gate_must_pass 'the restored least-privilege role'

rc build research-worker || fail 'research-worker image build failed'
command -v cargo >/dev/null 2>&1 || fail 'cargo is required to prove the manual --root Raw contract'
manual_output="$(cargo run --quiet --locked -p collectors --bin collectors -- ingest-krx \
  --root "$raw_root" --date 2020-01-31 --mode synthetic \
  --bundle "$root/tests/fixtures/kr-etf/contract" --now 2020-01-31T08:00:00Z)" || fail 'manual collectors --root ingest failed'
direct_manifest="$raw_root/raw/manifests/provider=krx/market=kr/manifest.jsonl"
[ -f "$direct_manifest" ] || fail "direct host Raw manifest is missing: $direct_manifest"
[ ! -e "$raw_root/raw/raw" ] || fail 'Raw evidence was nested under <data>/raw/raw'
manual_manifest="$(printf '%s' "$manual_output" | python3 -c 'import json,sys; print(json.load(sys.stdin)["manifest"])')" || fail 'manual collectors output was not valid JSON'
[ "$(cd "$(dirname "$manual_manifest")" && pwd)/$(basename "$manual_manifest")" = "$(cd "$(dirname "$direct_manifest")" && pwd)/$(basename "$direct_manifest")" ] || fail "manual --root manifest mismatch: $manual_manifest"
rc run --rm --no-deps research-raw-init || fail 'research-raw-init failed'
rc run --rm --no-deps --entrypoint /bin/sh --user 10001:10001 research-worker -ec '
  manifest="$RESEARCH_RAW_ROOT/raw/manifests/provider=krx/market=kr/manifest.jsonl"
  test -s "$manifest"
  : > "$manifest"
  test ! -s "$manifest"
  probe="$RESEARCH_RAW_ROOT/raw/.qa-write-probe"
  : > "$probe"
  rm -f "$probe"
' || fail 'research-worker UID 10001 cannot prepare the startup orphan'
rc up -d research-worker >/dev/null || fail 'research-worker service failed to start'

healthy=0
for _ in $(seq 1 30); do
  if dkr compose -p "$project" -f "$(hostpath "$compose_file")" exec -T research-worker /usr/local/bin/research-worker healthcheck >/dev/null 2>&1; then healthy=1; break; fi
  sleep 1
done
[ "$healthy" -eq 1 ] || fail 'research-worker did not become functionally healthy'
manual_batch_id="$(printf '%s' "$manual_output" | python3 -c 'import json,sys; print(json.load(sys.stdin)["batch_id"])')" || fail 'manual collectors output omitted batch_id'
rc exec -T -e "EXPECTED_BATCH_ID=$manual_batch_id" research-worker /bin/sh -ec '
  manifest="$RESEARCH_RAW_ROOT/raw/manifests/provider=krx/market=kr/manifest.jsonl"
  test "$(grep -Fc "$EXPECTED_BATCH_ID" "$manifest")" -eq 1
' || fail 'startup orphan recovery did not restore the exact manifest row'

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
  (SELECT count(source_batch_id) FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31'),
  (SELECT bool_and(b.source_batch_id = source.id) FROM data_batches b CROSS JOIN source WHERE b.provider = 'KRX' AND b.market = 'KR' AND b.batch_date = DATE '2020-01-31'),
  (SELECT count(*) FROM trading_calendar_versions WHERE exchange = 'KRX'),
  (SELECT count(*) FROM trading_calendars WHERE exchange = 'KRX'),
  (SELECT string_agg(DISTINCT kind, ',' ORDER BY kind) FROM data_batches WHERE provider = 'KRX' AND market = 'KR' AND batch_date = DATE '2020-01-31'),
  (SELECT string_agg(to_char(session_date, 'YYYY-MM-DD') || ':' || session_type, ',' ORDER BY session_date) FROM trading_calendar_versions WHERE exchange = 'KRX'),
  (SELECT string_agg(to_char(session_date, 'YYYY-MM-DD') || ':' || session_type, ',' ORDER BY session_date) FROM trading_calendars WHERE exchange = 'KRX'),
  (SELECT bool_and(v.source_batch_id = source.id) FROM trading_calendar_versions v CROSS JOIN source WHERE v.exchange = 'KRX'),
  (SELECT bool_and(
      c.source_batch_id IS NOT NULL
      AND c.content_sha256 IS NOT NULL
      AND c.retrieved_at IS NOT NULL
      AND EXISTS (
        SELECT 1 FROM data_batches batch
        WHERE batch.source_batch_id = c.source_batch_id
      )
      AND EXISTS (
        SELECT 1 FROM trading_calendar_versions history
        WHERE history.exchange = c.exchange
          AND history.session_date = c.session_date
          AND history.session_type = c.session_type
          AND history.timezone = c.timezone
          AND history.source = c.source
          AND history.source_version = c.source_version
          AND history.content_sha256 = c.content_sha256
      )
    ) FROM trading_calendars c WHERE c.exchange = 'KRX')
) FROM source;
SQL
)"
  expected='1|4|4|t|2|2|CALENDAR,CORPORATE_ACTIONS,EOD,REFERENCE|2020-01-30:TRADING,2020-01-31:TRADING|2020-01-30:TRADING,2020-01-31:TRADING|t|t'
  [ "$value" = "$expected" ] || fail "publication evidence mismatch: $value"
  printf '%s' "$value"
}

before="$(publication_evidence)"
rc run --rm --no-deps research-worker --once --date 2020-01-31 || fail 'second research-worker one-shot failed'
after="$(publication_evidence)"
[ "$before" = "$after" ] || fail "idempotency failed: counts changed from $before to $after"
echo "RESEARCH_WORKER_SMOKE: functional PASS ($after)"
