#!/usr/bin/env bash
# Fail-closed validation for the owner-only, read-only KIS Compose release.
# This script reads names and metadata only; it never prints secret contents and
# never accepts KIS account/order credentials for this release.
#
# `--scope infrastructure` is the KIS-free/non-KIS database/filesystem
# contract: it gates only PostgreSQL, role bootstrap, migrations, Raw ownership,
# and the research schema check. It deliberately does not require KIS
# credentials, serving-only Auth0/TLS values, or the curated dataset pin that
# is produced by the later backfill.
# `--scope backfill` is the deliberately smaller first-phase contract: it gates
# the KIS worker/bootstrap DB path without requiring serving-only Auth0/TLS
# values or the curated dataset pin that is produced by that very backfill.
# `--scope range-raw` is the DB-free Stage5 capture contract: it gates only the
# production credentialed daily-range worker, its Raw root, entitlement, KIS
# source files, and isolated runtime copies. It deliberately does not require
# PostgreSQL, Curated, Auth0/TLS, or recommendation inputs.
# `--scope range-raw-recovery` is the provider-free Stage5 recovery contract:
# it gates only the production Raw root, entitlement, and code identity. It
# intentionally requires no KIS source/runtime files, database values, or
# network-capable service inputs.
# `--scope serving-prereqs` is the copy/readiness contract for all non-KIS
# serving inputs. It requires Auth0/TLS, API/worker source secrets, and the
# non-KIS runtime copies, but deliberately does not require KIS credentials,
# RESEARCH_* settings, entitlement, or recommendation dataset pins. It never
# starts a service; Compose remains on the three execution scopes below.
# `--scope release` is the full serving contract and remains the default.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
source "$script_dir/lib/dotenv.sh"
env_file=${LAGRANGE_ENV_FILE:-$root/deploy/compose/.env}
source_dir=$root/deploy/secrets
runtime_dir=
commit=
scope=release
missing=()
invalid=()
warnings=()

usage() {
  cat <<'EOF'
Usage: scripts/ops/validate-production-config.sh
       [--scope infrastructure|serving-prereqs|backfill|range-raw|range-raw-recovery|release]
       [--env-file PATH]

Exit 0: production configuration and required secret files are ready.
Exit 2: BLOCKED_EXTERNAL (operator values, credentials, runtime copies, or
         immutable dataset pins are not available yet).
Exit 1: invalid configuration or unsafe file shape.

The KIS account reference is intentionally optional. Live/order credentials are
not required and are rejected from this read-only release profile.

Validation is root-only because production source secrets and service-specific
runtime copies are intentionally protected by root ownership and restrictive
modes. `--help` remains available to unprivileged callers. Preserve the commit
when invoking through sudo:
  export LAGRANGE_CODE_COMMIT="$(git rev-parse HEAD)"
  sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" scripts/ops/validate-production-config.sh --scope infrastructure --env-file deploy/compose/.env

infrastructure scope requires only the production DB/bootstrap inputs and
runtime copies for PostgreSQL, role bootstrap, migrations, Raw ownership, and
the research schema check. It does not require KIS credentials, Auth0/TLS
serving inputs, or recommendation dataset five-pin values.
backfill scope additionally requires the KIS worker inputs, but does not
require Auth0/TLS serving inputs or recommendation dataset five-pin values.
range-raw scope requires only the DB-free production credentialed KIS Raw
inputs and its isolated runtime copies; it does not require database, Curated,
Auth0/TLS, or recommendation values.
range-raw-recovery scope requires only the production Raw root, entitlement,
and exact code commit. It uses a network-disabled, secret-free recovery
service and does not require KIS source/runtime files or any database values.
serving-prereqs scope checks Auth0/TLS and every non-KIS runtime copy needed by
the serving Compose inventory. It does not require KIS app credentials,
RESEARCH_* or entitlement values, or recommendation dataset five-pin values;
it is read-only and does not start Compose services.
release scope requires every serving value and the approved five-pin dataset.
EOF
}

die() { echo "production-config: $*" >&2; exit 1; }
while [ "$#" -gt 0 ]; do
  case "$1" in
    --scope)
      [ "$#" -ge 2 ] || die '--scope needs infrastructure, serving-prereqs, backfill, range-raw, range-raw-recovery, or release'
      scope=$2
      shift 2
      ;;
    --env-file)
      [ "$#" -ge 2 ] || die '--env-file needs a path'
      env_file=$2
      shift 2
      ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

case "$scope" in
  infrastructure|serving-prereqs|backfill|range-raw|range-raw-recovery|release) ;;
  *) die "--scope must be infrastructure, serving-prereqs, backfill, range-raw, range-raw-recovery, or release" ;;
esac

# Production source secrets and runtime copies are root-owned by contract. Check
# the caller before touching the env file or any configured secret path so an
# unprivileged invocation reports the real permission prerequisite instead of
# misclassifying protected files as missing. Keep --help usable above.
if [ "$(id -u)" -ne 0 ]; then
  die "validation must run as root to inspect protected production paths; use sudo env LAGRANGE_CODE_COMMIT=\"\$LAGRANGE_CODE_COMMIT\" scripts/ops/validate-production-config.sh --scope $scope --env-file $env_file"
fi

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

if ! dotenv_load "$env_file"; then
  invalid+=("${DOTENV_ERRORS[@]}")
fi
if ! dotenv_validate_shell_overrides; then
  invalid+=("${DOTENV_SHELL_ERRORS[@]}")
fi

get() { dotenv_get "$1"; }

# Owner-beta admission is deployment policy, not a secret. Missing mode keys
# preserve the pre-beta release as disabled, while a present value must be
# exact. The root-side release gate derives any artifact identity only from the
# canonical registry embedded in its verified immutable image.
owner_beta_access_mode=$(get OWNER_BETA_ACCESS_MODE)
if ! dotenv_has OWNER_BETA_ACCESS_MODE; then
  owner_beta_access_mode=disabled
fi
owner_beta_price_input_mode=$(get OWNER_BETA_PRICE_INPUT_MODE)
if ! dotenv_has OWNER_BETA_PRICE_INPUT_MODE; then
  owner_beta_price_input_mode=disabled
fi
owner_beta_paper_mode=$(get OWNER_BETA_PAPER_MODE)
if ! dotenv_has OWNER_BETA_PAPER_MODE; then
  owner_beta_paper_mode=disabled
fi
for key in OWNER_BETA_ACCESS_MODE_FILE OWNER_BETA_PAPER_MODE_FILE; do
  dotenv_has "$key" && invalid+=("owner_beta_policy_file_forbidden")
done
if dotenv_has OWNER_BETA_PRICE_INPUT_MODE_FILE ||
   [[ -v OWNER_BETA_PRICE_INPUT_MODE_FILE ]]; then
  invalid+=("owner_beta_price_input_file_forbidden")
fi
# Compose lets a process environment override its protected env file. Keep the
# new mode protected without widening the shared dotenv key inventory: a shell
# override is acceptable only when it exactly repeats the parsed/default value.
if [[ -v OWNER_BETA_PRICE_INPUT_MODE ]] &&
   [ "${OWNER_BETA_PRICE_INPUT_MODE-}" != "$owner_beta_price_input_mode" ]; then
  invalid+=("owner_beta_price_input_shell_override_mismatch")
fi

case "$owner_beta_access_mode" in
  disabled)
    [ "$owner_beta_price_input_mode" = disabled ] ||
      invalid+=("owner_beta_price_input_requires_owner_only")
    [ "$owner_beta_paper_mode" = disabled ] ||
      invalid+=("owner_beta_paper_requires_owner_only")
    ;;
  owner_only) ;;
  *) invalid+=("owner_beta_access_mode_invalid") ;;
esac

case "$owner_beta_price_input_mode" in
  disabled|sealed_v1) ;;
  *) invalid+=("owner_beta_price_input_mode_invalid") ;;
esac

case "$owner_beta_paper_mode" in
  disabled) ;;
  enabled)
    # No trusted three-unattended-session evidence checker exists yet. Keep a
    # single static blocker instead of accepting an operator assertion.
    invalid+=("owner_beta_paper_evidence_unavailable")
    ;;
  *) invalid+=("owner_beta_paper_mode_invalid") ;;
esac

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
else
  runtime_dir="$source_dir/runtime"
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

# Compose parses the complete file even for infrastructure's service subset,
# so the exact build commit remains a preflight input. Worker/serving settings
# below begin only at backfill and are intentionally absent from infrastructure.
required_keys=(LAGRANGE_DATA_DIR)
if [ "$scope" != range-raw ] && [ "$scope" != range-raw-recovery ]; then
  required_keys+=(LAGRANGE_RUNTIME_SECRET_DIR)
fi
if [ "$scope" != range-raw ] && [ "$scope" != range-raw-recovery ]; then
  required_keys+=(POSTGRES_USER POSTGRES_DB)
fi
if [ "$scope" = backfill ] || [ "$scope" = range-raw ] || [ "$scope" = range-raw-recovery ] || [ "$scope" = release ]; then
  required_keys+=(RESEARCH_APP_ENV RESEARCH_FETCH_MODE RESEARCH_ENTITLEMENT_REFERENCE)
fi
if [ "$scope" = serving-prereqs ] || [ "$scope" = release ]; then
  required_keys+=(
    LAGRANGE_ARTIFACTS_DIR
    AUTH0_DOMAIN AUTH0_CLIENT_ID AUTH0_REDIRECT_URI
  )
fi
if [ "$scope" = release ]; then
  required_keys+=(
    RECOMMENDATION_DATASET_VERSION_ID RECOMMENDATION_DATASET_ID
    RECOMMENDATION_DATASET_VERSION RECOMMENDATION_CURATED_VERSION
    RECOMMENDATION_DATASET_MANIFEST_SHA256
  )
fi
for key in "${required_keys[@]}"; do
  require_value "$key"
done

data_dir=$(get LAGRANGE_DATA_DIR)
artifacts_dir=$(get LAGRANGE_ARTIFACTS_DIR)
[ -z "$data_dir" ] || [[ "$data_dir" = /* ]] || invalid+=("LAGRANGE_DATA_DIR must be absolute")
if [ "$scope" = serving-prereqs ] || [ "$scope" = release ]; then
  [ -z "$artifacts_dir" ] || [[ "$artifacts_dir" = /* ]] || invalid+=("LAGRANGE_ARTIFACTS_DIR must be absolute")
fi
if [ "$scope" = backfill ] || [ "$scope" = range-raw ] || [ "$scope" = range-raw-recovery ] || [ "$scope" = release ]; then
  [ "$(get RESEARCH_APP_ENV)" = production ] || invalid+=("RESEARCH_APP_ENV must be production")
  [ "$(get RESEARCH_FETCH_MODE)" = credentialed ] || invalid+=("RESEARCH_FETCH_MODE must be credentialed")
  [ "$(get RESEARCH_CANDIDATE_ENABLED)" = false ] || invalid+=("RESEARCH_CANDIDATE_ENABLED must be false until the KIS candidate bridge is released")
fi
case ",$(get COMPOSE_PROFILES)," in *,live,*) invalid+=("Compose live profile must remain disabled") ;; esac
[ -z "$(get LIVE_NODE_MODE)" ] || [ "$(get LIVE_NODE_MODE)" = disabled ] || invalid+=("LIVE_NODE_MODE must be disabled")
[ -z "$(get LIVE_NODE_DRY_RUN)" ] || [ "$(get LIVE_NODE_DRY_RUN)" = 1 ] || invalid+=("LIVE_NODE_DRY_RUN must remain 1")

commit=$(dotenv_effective_get LAGRANGE_CODE_COMMIT)
if [ -n "$commit" ] && [[ ! "$commit" =~ ^[0-9a-fA-F]{40}$ ]]; then
  invalid+=("LAGRANGE_CODE_COMMIT must be the exact 40-hex commit")
elif [ -z "$commit" ]; then
  missing+=("LAGRANGE_CODE_COMMIT (export it for the build/preflight)")
fi
manifest_hash=$(get RECOMMENDATION_DATASET_MANIFEST_SHA256)
[ -z "$manifest_hash" ] || [[ "$manifest_hash" =~ ^[0-9a-f]{64}$ ]] || invalid+=("RECOMMENDATION_DATASET_MANIFEST_SHA256 must be lowercase 64-hex")

if [ "$scope" = infrastructure ]; then
  secret_files=(
    postgres_password db_migration_owner_password db_app_password db_worker_password
    db_audit_password db_research_password db_admin_password
  )
elif [ "$scope" = backfill ]; then
  secret_files=(
    postgres_password db_migration_owner_password db_app_password db_worker_password
    db_audit_password db_research_password db_admin_password kis_app_key kis_app_secret
  )
elif [ "$scope" = range-raw ]; then
  secret_files=(kis_app_key kis_app_secret)
elif [ "$scope" = range-raw-recovery ]; then
  secret_files=()
elif [ "$scope" = serving-prereqs ]; then
  secret_files=(
    postgres_password db_migration_owner_password db_app_password db_worker_password
    db_audit_password db_research_password db_admin_password cursor_secret
    session_secret csrf_secret auth0_client_secret backup_encryption_key
  )
else
  secret_files=(
    postgres_password db_migration_owner_password db_app_password db_worker_password
    db_audit_password db_research_password db_admin_password cursor_secret
    session_secret csrf_secret auth0_client_secret kis_app_key kis_app_secret
    backup_encryption_key
  )
fi
db_secret_names=(
  postgres_password db_migration_owner_password db_app_password db_worker_password
  db_audit_password db_research_password db_admin_password
)
crypto_secret_names=(session_secret csrf_secret cursor_secret backup_encryption_key)
declare -A db_secret_ready=()
declare -A crypto_secret_ready=()
check_source_mode() {
  local path=$1 label=$2 mode
  mode=$(stat -c '%a' -- "$path") || die "cannot stat $label: $path"
  case "$mode" in
    400|600) ;;
    *) invalid+=("$label must be mode 0400 or 0600 (found $mode): $path") ;;
  esac
}
crypto_placeholder_pattern='placeholder|example|todo|change[-_ ]*me|change[-_ ]*this|replace[-_ ]*(me|this|with)|your[-_ ]*(client[-_ ]*)?secret|secret[-_ ]*here|auth0[-_ ]*client[-_ ]*secret|<[^>]+>|\$\{[^}]+\}'
check_crypto_source_shape() {
  local path=$1 label=$2 byte_count
  byte_count=$(wc -c <"$path") || die "cannot inspect $label: $path"
  if [ "$byte_count" -ne 64 ] ||
     ! LC_ALL=C grep -Eq '^[0-9a-f]{64}$' -- "$path" ||
     LC_ALL=C grep -Eiq -- "$crypto_placeholder_pattern" "$path"; then
    invalid+=("$label must contain exactly 64 lowercase hex characters with no newline or placeholder")
    return 1
  fi
  return 0
}
for name in "${secret_files[@]}"; do
  path="$source_dir/$name"
  case "$name" in
    postgres_password|db_migration_owner_password|db_app_password|db_worker_password|db_audit_password|db_research_password|db_admin_password) is_db_secret=yes ;;
    *) is_db_secret=no ;;
  esac
  is_crypto_secret=no
  if [ "$scope" = serving-prereqs ] || [ "$scope" = release ]; then
    for crypto_name in "${crypto_secret_names[@]}"; do
      [ "$name" = "$crypto_name" ] && is_crypto_secret=yes
    done
  fi
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
    elif [ "$is_crypto_secret" = yes ]; then
      # The shape helper reports only filenames/reasons. Mark this value ready
      # only when its exact contract passed all three checks.
      if check_crypto_source_shape "$path" "crypto secret $name"; then
        crypto_secret_ready["$name"]=1
      fi
    elif [ "$is_db_secret" = yes ]; then
      # Equality checks run only after every DB source is regular,
      # non-empty, single-line, and free of template placeholders. They never
      # print or otherwise expose the credential bytes.
      db_secret_ready["$name"]=1
    fi
  fi
done

if [ "$scope" = serving-prereqs ] || [ "$scope" = release ]; then
  all_crypto_secrets_ready=yes
  for name in "${crypto_secret_names[@]}"; do
    if [ -z "${crypto_secret_ready[$name]+set}" ]; then
      all_crypto_secrets_ready=no
      break
    fi
  done
  if [ "$all_crypto_secrets_ready" = yes ]; then
    for ((left = 0; left < ${#crypto_secret_names[@]}; left++)); do
      for ((right = left + 1; right < ${#crypto_secret_names[@]}; right++)); do
        left_name=${crypto_secret_names[left]}
        right_name=${crypto_secret_names[right]}
        if cmp -s -- "$source_dir/$left_name" "$source_dir/$right_name"; then
          invalid+=("crypto source secrets must be distinct: $left_name conflicts with $right_name")
        fi
      done
    done
  fi
fi

# PostgreSQL role credentials are intentionally independent. Compare only
# source files that passed the shape gate above and report filenames, never
# secret values. A missing/malformed DB source is already reported as its own
# blocker and suppresses this secondary comparison.
all_db_secrets_ready=yes
for name in "${db_secret_names[@]}"; do
  if [ -z "${db_secret_ready[$name]+set}" ]; then
    all_db_secrets_ready=no
    break
  fi
done
if [ "$all_db_secrets_ready" = yes ]; then
  for ((left=0; left<${#db_secret_names[@]}; left++)); do
    for ((right=left+1; right<${#db_secret_names[@]}; right++)); do
      left_name=${db_secret_names[$left]}
      right_name=${db_secret_names[$right]}
      if ! cmp -s -- "$source_dir/$left_name" "$source_dir/$right_name"; then
        continue
      fi
      invalid+=("DB source secrets must be distinct: $left_name conflicts with $right_name")
    done
  done
fi

if [ "$scope" = serving-prereqs ] || [ "$scope" = release ]; then
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
fi

# Runtime copies are separate native-Linux files because Compose file-backed
# secrets otherwise inherit the operator source ownership. Check the same
# owner/mode contract as provision-runtime-secrets.sh without printing values.
if [ "$scope" = infrastructure ]; then
  runtime_specs=(
    db-role-bootstrap/postgres_password:999:999:400
    db-role-bootstrap/db_migration_owner_password:999:999:400
    db-role-bootstrap/db_app_password:999:999:400 db-role-bootstrap/db_worker_password:999:999:400
    db-role-bootstrap/db_audit_password:999:999:400 db-role-bootstrap/db_research_password:999:999:400
    db-role-bootstrap/db_admin_password:999:999:400 db-migrate/db_migration_owner_password:999:999:400
    postgres/postgres_password:999:999:440 research-schema-check/postgres_password:999:999:440
  )
elif [ "$scope" = serving-prereqs ]; then
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
    research-worker/db_research_password:10001:10001:440 recommendation-runner/db_worker_password:10001:10001:440
    candidate-runner/db_worker_password:10001:10001:440 nt-backtest-worker-1/db_worker_password:10001:10001:440
    nt-backtest-worker-2/db_worker_password:10001:10001:440 paper-scheduler/db_app_password:10001:10001:440
    paper-scheduler/db_worker_password:10001:10001:440 paper-scheduler/db_admin_password:10001:10001:440
    paper-scheduler/db_audit_password:10001:10001:440
  )
elif [ "$scope" = backfill ]; then
  runtime_specs=(
    db-role-bootstrap/postgres_password:999:999:400
    db-role-bootstrap/db_migration_owner_password:999:999:400
    db-role-bootstrap/db_app_password:999:999:400 db-role-bootstrap/db_worker_password:999:999:400
    db-role-bootstrap/db_audit_password:999:999:400 db-role-bootstrap/db_research_password:999:999:400
    db-role-bootstrap/db_admin_password:999:999:400 db-migrate/db_migration_owner_password:999:999:400
    postgres/postgres_password:999:999:440 research-schema-check/postgres_password:999:999:440
    research-worker/db_research_password:10001:10001:440 research-worker/kis_app_key:10001:10001:440
    research-worker/kis_app_secret:10001:10001:440
  )
elif [ "$scope" = range-raw ]; then
  runtime_specs=(
    research-range-raw/kis_app_key:10001:10001:440
    research-range-raw/kis_app_secret:10001:10001:440
  )
elif [ "$scope" = range-raw-recovery ]; then
  runtime_specs=()
else
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
fi
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
if [ "$scope" != infrastructure ] && [ -z "$(get KIS_ACCOUNT_REF)" ]; then
  warnings+=("KIS account/order credentials are intentionally not required for read-only release")
fi

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
printf 'PRODUCTION_CONFIG: PASS (scope=%s; KIS read-only; live/order profile disabled)\n' "$scope"
printf '  dataset=%s version_id=%s manifest_sha256=%s\n' \
  "$(get RECOMMENDATION_DATASET_ID)" "$(get RECOMMENDATION_DATASET_VERSION_ID)" "${manifest_hash:0:12}..."
for warning in "${warnings[@]}"; do
  echo "  NOTE: $warning"
done
