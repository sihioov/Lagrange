#!/usr/bin/env bash
# Fail-closed validation for the owner-only, read-only KIS Compose release.
# This script reads names and metadata only; it never prints secret contents and
# never accepts KIS account/order credentials for this release.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
env_file=${LAGRANGE_ENV_FILE:-$root/deploy/compose/.env}
source_dir=${LAGRANGE_SECRET_SOURCE_DIR:-$root/deploy/secrets}
runtime_dir=${LAGRANGE_RUNTIME_SECRET_DIR:-$source_dir/runtime}
commit=${LAGRANGE_CODE_COMMIT:-}
missing=()
invalid=()
warnings=()
declare -A cfg=()

usage() {
  cat <<'EOF'
Usage: scripts/ops/validate-production-config.sh [--env-file PATH]

Exit 0: production configuration and required secret files are ready.
Exit 2: BLOCKED_EXTERNAL (operator values, credentials, runtime copies, or
         immutable dataset pins are not available yet).
Exit 1: invalid configuration or unsafe file shape.

The KIS account reference is intentionally optional. Live/order credentials are
not required and are rejected from this read-only release profile.
EOF
}

die() { echo "production-config: $*" >&2; exit 1; }
while [ "$#" -gt 0 ]; do
  case "$1" in
    --env-file)
      [ "$#" -ge 2 ] || die '--env-file needs a path'
      env_file=$2
      shift 2
      ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

if [ ! -f "$env_file" ]; then
  echo "BLOCKED_EXTERNAL: missing production env file: $env_file" >&2
  exit 2
fi
[ ! -L "$env_file" ] || die "env file must not be a symlink: $env_file"
env_mode=$(stat -c '%a' "$env_file") || die "cannot stat env file: $env_file"
case "$env_mode" in
  600) ;;
  *) invalid+=("env file must be mode 0600 (found $env_mode): $env_file") ;;
esac

# Parse a deliberately small dotenv grammar. Values are not evaluated or
# sourced, so a malicious env file cannot execute shell code in this checker.
while IFS= read -r line || [ -n "$line" ]; do
  line=${line%$'\r'}
  [ -z "$line" ] && continue
  case "$line" in
    \#*) continue ;;
  esac
  [[ "$line" == *=* ]] || { invalid+=("invalid dotenv line (missing '=')"); continue; }
  key=${line%%=*}
  value=${line#*=}
  [[ "$key" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || { invalid+=("invalid dotenv key"); continue; }
  [ -z "${cfg[$key]+set}" ] || { invalid+=("duplicate dotenv key: $key"); continue; }
  cfg[$key]=$value
done <"$env_file"

get() { printf '%s' "${cfg[$1]-}"; }
env_dir=$(cd "$(dirname "$env_file")" && pwd)
resolve_config_path() {
  case "$1" in
    /*) printf '%s' "$1" ;;
    *) printf '%s/%s' "$env_dir" "$1" ;;
  esac
}
if [ -n "$(get LAGRANGE_SECRET_SOURCE_DIR)" ]; then
  source_dir=$(resolve_config_path "$(get LAGRANGE_SECRET_SOURCE_DIR)")
fi
if [ -n "$(get LAGRANGE_RUNTIME_SECRET_DIR)" ]; then
  runtime_dir=$(resolve_config_path "$(get LAGRANGE_RUNTIME_SECRET_DIR)")
fi
require_value() {
  local key=$1 value
  value=$(get "$key")
  [ -n "$value" ] || missing+=("$key")
  case "$value" in
    *REPLACE_WITH*|your-tenant.auth0.com|your-client-id|app.lagrange.local|\<deployment-config\>|\<secure-source\>*)
      missing+=("$key contains a placeholder")
      ;;
  esac
}

for key in \
  LAGRANGE_DATA_DIR LAGRANGE_ARTIFACTS_DIR LAGRANGE_RUNTIME_SECRET_DIR \
  POSTGRES_USER POSTGRES_DB RESEARCH_APP_ENV RESEARCH_FETCH_MODE \
  RESEARCH_ENTITLEMENT_REFERENCE AUTH0_DOMAIN AUTH0_CLIENT_ID AUTH0_REDIRECT_URI \
  RECOMMENDATION_DATASET_VERSION_ID RECOMMENDATION_DATASET_ID \
  RECOMMENDATION_DATASET_VERSION RECOMMENDATION_CURATED_VERSION \
  RECOMMENDATION_DATASET_MANIFEST_SHA256; do
  require_value "$key"
done

data_dir=$(get LAGRANGE_DATA_DIR)
artifacts_dir=$(get LAGRANGE_ARTIFACTS_DIR)
[ -z "$data_dir" ] || [[ "$data_dir" = /* ]] || invalid+=("LAGRANGE_DATA_DIR must be absolute")
[ -z "$artifacts_dir" ] || [[ "$artifacts_dir" = /* ]] || invalid+=("LAGRANGE_ARTIFACTS_DIR must be absolute")
[ "$(get RESEARCH_APP_ENV)" = production ] || invalid+=("RESEARCH_APP_ENV must be production")
[ "$(get RESEARCH_FETCH_MODE)" = credentialed ] || invalid+=("RESEARCH_FETCH_MODE must be credentialed")
[ "$(get RESEARCH_CANDIDATE_ENABLED)" = false ] || invalid+=("RESEARCH_CANDIDATE_ENABLED must be false until the KIS candidate bridge is released")
case ",$(get COMPOSE_PROFILES)," in *,live,*) invalid+=("Compose live profile must remain disabled") ;; esac
[ -z "$(get LIVE_NODE_MODE)" ] || [ "$(get LIVE_NODE_MODE)" = disabled ] || invalid+=("LIVE_NODE_MODE must be disabled")
[ -z "$(get LIVE_NODE_DRY_RUN)" ] || [ "$(get LIVE_NODE_DRY_RUN)" = 1 ] || invalid+=("LIVE_NODE_DRY_RUN must remain 1")

if [ -n "$commit" ] && [[ ! "$commit" =~ ^[0-9a-fA-F]{40}$ ]]; then
  invalid+=("LAGRANGE_CODE_COMMIT must be the exact 40-hex commit")
elif [ -z "$commit" ]; then
  missing+=("LAGRANGE_CODE_COMMIT (export it for the build/preflight)")
fi
manifest_hash=$(get RECOMMENDATION_DATASET_MANIFEST_SHA256)
[ -z "$manifest_hash" ] || [[ "$manifest_hash" =~ ^[0-9a-f]{64}$ ]] || invalid+=("RECOMMENDATION_DATASET_MANIFEST_SHA256 must be lowercase 64-hex")

secret_files=(
  postgres_password db_migration_owner_password db_app_password db_worker_password
  db_audit_password db_research_password db_admin_password cursor_secret
  session_secret csrf_secret auth0_client_secret kis_app_key kis_app_secret
  backup_encryption_key
)
check_source_mode() {
  local path=$1 label=$2 mode
  mode=$(stat -c '%a' -- "$path") || die "cannot stat $label: $path"
  case "$mode" in
    400|600) ;;
    *) invalid+=("$label must be mode 0400 or 0600 (found $mode): $path") ;;
  esac
}
for name in "${secret_files[@]}"; do
  path="$source_dir/$name"
  if [ ! -f "$path" ] || [ -L "$path" ]; then
    missing+=("secret $name (run the approved secret provisioning procedure)")
  elif [ ! -s "$path" ]; then
    invalid+=("secret $name is empty")
  elif [ "$(wc -l <"$path")" -ne 0 ] || LC_ALL=C grep -Fq $'\r' "$path"; then
    invalid+=("secret $name must be a single line")
  else
    check_source_mode "$path" "secret $name"
    if grep -Eiq 'REPLACE_WITH|CHANGE_ME|YOUR_|example|placeholder' "$path"; then
      missing+=("secret $name still contains a template placeholder")
    fi
  fi
done

for name in lagrange.crt lagrange.key; do
  path="$source_dir/tls/$name"
  if [ ! -f "$path" ] || [ -L "$path" ]; then
    missing+=("TLS file tls/$name")
  elif [ ! -s "$path" ]; then
    invalid+=("TLS file tls/$name is empty")
  else
    check_source_mode "$path" "TLS file tls/$name"
    if grep -Eiq 'REPLACE_WITH|CHANGE_ME|YOUR_|example|placeholder' "$path"; then
      missing+=("TLS file tls/$name still contains a template placeholder")
    fi
  fi
done

# Runtime copies are separate native-Linux files because Compose file-backed
# secrets otherwise inherit the operator source ownership. Check the same
# owner/mode contract as provision-runtime-secrets.sh without printing values.
runtime_specs=(
  reverse-proxy/lagrange_tls_cert:101:101:440 reverse-proxy/lagrange_tls_key:101:101:440
  api-server/db_app_password:10001:10001:440 api-server/db_admin_password:10001:10001:440
  api-server/db_audit_password:10001:10001:440 api-server/cursor_secret:10001:10001:440
  api-server/session_secret:10001:10001:440 api-server/csrf_secret:10001:10001:440
  api-server/auth0_client_secret:10001:10001:440
  db-role-bootstrap/postgres_password:999:999:400
  db-role-bootstrap/db_migration_owner_password:999:999:400
  db-role-bootstrap/db_app_password:999:999:400 db-role-bootstrap/db_worker_password:999:999:400
  db-role-bootstrap/db_audit_password:999:999:400 db-role-bootstrap/db_research_password:999:999:400
  db-role-bootstrap/db_admin_password:999:999:400 db-migrate/db_migration_owner_password:999:999:400
  postgres/postgres_password:999:999:440 research-schema-check/postgres_password:999:999:440
  research-worker/db_research_password:10001:10001:440 research-worker/kis_app_key:10001:10001:440
  research-worker/kis_app_secret:10001:10001:440 recommendation-runner/db_worker_password:10001:10001:440
  candidate-runner/db_worker_password:10001:10001:440 nt-backtest-worker-1/db_worker_password:10001:10001:440
  nt-backtest-worker-2/db_worker_password:10001:10001:440 paper-scheduler/db_app_password:10001:10001:440
  paper-scheduler/db_worker_password:10001:10001:440 paper-scheduler/db_admin_password:10001:10001:440
  paper-scheduler/db_audit_password:10001:10001:440
)
for spec in "${runtime_specs[@]}"; do
  IFS=: read -r relative expected_uid expected_gid expected_mode <<<"$spec"
  path="$runtime_dir/$relative"
  if [ ! -f "$path" ] || [ -L "$path" ]; then
    missing+=("runtime secret $relative (run provision-runtime-secrets.sh)")
    continue
  fi
  [ -s "$path" ] || invalid+=("runtime secret $relative is empty")
  actual=$(stat -c '%u:%g:%a' -- "$path") || die "cannot stat runtime secret: $relative"
  [ "$actual" = "$expected_uid:$expected_gid:$expected_mode" ] ||
    invalid+=("runtime secret $relative has unsafe $actual; expected $expected_uid:$expected_gid:$expected_mode")
  grep -Eiq 'REPLACE_WITH|CHANGE_ME|YOUR_|example|placeholder' "$path" &&
    missing+=("runtime secret $relative still contains a template placeholder") || true
done

# The account/order reference is deliberately not part of the required list.
[ -n "$(get KIS_ACCOUNT_REF)" ] || warnings+=("KIS account/order credentials are intentionally not required for read-only release")

if [ "${#invalid[@]}" -gt 0 ]; then
  echo "INVALID_CONFIG: production configuration is unsafe or inconsistent" >&2
  printf '  - %s\n' "${invalid[@]}" >&2
  exit 1
fi
if [ "${#missing[@]}" -gt 0 ]; then
  echo "BLOCKED_EXTERNAL: production values/credentials/dataset pins are not provisioned" >&2
  printf '  - %s\n' "${missing[@]}" >&2
  exit 2
fi
printf 'PRODUCTION_CONFIG: PASS (KIS read-only; live/order profile disabled)\n'
printf '  dataset=%s version_id=%s manifest_sha256=%s\n' \
  "$(get RECOMMENDATION_DATASET_ID)" "$(get RECOMMENDATION_DATASET_VERSION_ID)" "${manifest_hash:0:12}..."
for warning in "${warnings[@]}"; do
  echo "  NOTE: $warning"
done
