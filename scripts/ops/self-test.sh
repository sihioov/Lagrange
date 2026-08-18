#!/usr/bin/env bash
# No-infrastructure self-test for the operator workflows.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ops="$root/scripts/ops"
out_dir=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-ops-self-test.XXXXXX")
trap 'rm -rf -- "$out_dir"' EXIT

for script in provision-linux.sh provision-db-secrets.sh validate-production-config.sh \
  compose-release.sh backfill-production.sh post-backfill-health.sh; do
  bash -n "$ops/$script"
done
bash "$root/deploy/secrets/runtime-static-check.sh" >/dev/null
bash "$root/deploy/secrets/provision-runtime-secrets.sh" --help >/dev/null
bash "$root/deploy/systemd/paper-runner-static-check.sh" >/dev/null
bash "$root/deploy/db/migrate-static-check.sh" >/dev/null
bash "$root/scripts/qa/research-worker-smoke.sh" --static-only >/dev/null
bash "$root/scripts/qa/recommendation-runner-smoke.sh" --static-only >/dev/null
bash "$ops/static-check.sh" >/dev/null

dry_run=$(LAGRANGE_CONFIG_ROOT="$out_dir/etc" \
  LAGRANGE_DEPLOY_ROOT="$out_dir/opt" \
  LAGRANGE_DATA_ROOT="$out_dir/data" \
  LAGRANGE_HOST_SECRET_ROOT="$out_dir/etc/secrets" \
  bash "$ops/provision-linux.sh" --dry-run)
grep -Fq 'DRY_RUN: no host changes made' <<<"$dry_run"

# The canonical host directories are root-owned and mode 0750, so preflight is
# intentionally root-only. Exercise that guard as an unprivileged user even
# when this self-test itself is launched by root.
preflight_guard_env=(
  "LAGRANGE_CONFIG_ROOT=$out_dir/etc"
  "LAGRANGE_DEPLOY_ROOT=$out_dir/opt"
  "LAGRANGE_DATA_ROOT=$out_dir/data"
  "LAGRANGE_HOST_SECRET_ROOT=$out_dir/etc/secrets"
)
if [ "$(id -u)" -eq 0 ] && command -v runuser >/dev/null 2>&1; then
  preflight_guard_cmd=(runuser -u nobody -- env "${preflight_guard_env[@]}" \
    bash "$ops/provision-linux.sh" --preflight)
elif [ "$(id -u)" -ne 0 ]; then
  preflight_guard_cmd=(env "${preflight_guard_env[@]}" \
    bash "$ops/provision-linux.sh" --preflight)
else
  preflight_guard_cmd=()
fi
if [ "${#preflight_guard_cmd[@]}" -gt 0 ]; then
  if "${preflight_guard_cmd[@]}" >"$out_dir/preflight-root.out" 2>&1; then
    echo 'self-test: non-root preflight unexpectedly passed' >&2
    exit 1
  fi
  grep -Fq -- 'provision-linux: --preflight must run as root' \
    "$out_dir/preflight-root.out" || {
    cat "$out_dir/preflight-root.out" >&2
    exit 1
  }
fi

mkdir -p "$out_dir/path-test/real"
ln -s "$out_dir/path-test/real" "$out_dir/path-test/link"
if LAGRANGE_CONFIG_ROOT="$out_dir/path-test/link/config" \
   LAGRANGE_DEPLOY_ROOT="$out_dir/path-test/deploy" \
   LAGRANGE_DATA_ROOT="$out_dir/path-test/data" \
   LAGRANGE_HOST_SECRET_ROOT="$out_dir/path-test/link/secrets" \
   bash "$ops/provision-linux.sh" --dry-run >"$out_dir/symlink.out" 2>&1; then
  echo 'self-test: provision accepted a symlinked ancestor' >&2
  exit 1
fi
grep -Fq 'must not traverse a symlink' "$out_dir/symlink.out"

# DB source credentials are generated only by the explicit root apply mode.
# Exercise the non-root plan/guard in every environment, and exercise the
# complete file contract when this no-infrastructure test itself has root.
db_secret_source="$out_dir/db-secret-source"
db_secret_plan=$(LAGRANGE_SECRET_SOURCE_DIR="$db_secret_source" \
  bash "$ops/provision-db-secrets.sh" --dry-run)
grep -Fq 'DB_SECRET_PROVISION mode=dry-run' <<<"$db_secret_plan"
grep -Fq 'DRY_RUN: no files created' <<<"$db_secret_plan"
[ ! -e "$db_secret_source" ]

if [ "$(id -u)" -eq 0 ] && command -v runuser >/dev/null 2>&1; then
  db_apply_guard_cmd=(runuser -u nobody -- env \
    "LAGRANGE_SECRET_SOURCE_DIR=$out_dir/db-secret-guard" \
    bash "$ops/provision-db-secrets.sh" --apply)
elif [ "$(id -u)" -ne 0 ]; then
  db_apply_guard_cmd=(env \
    "LAGRANGE_SECRET_SOURCE_DIR=$out_dir/db-secret-guard" \
    bash "$ops/provision-db-secrets.sh" --apply)
else
  db_apply_guard_cmd=()
fi
if [ "${#db_apply_guard_cmd[@]}" -gt 0 ]; then
  if "${db_apply_guard_cmd[@]}" >"$out_dir/db-secret-root.out" 2>&1; then
    echo 'self-test: non-root DB secret apply unexpectedly passed' >&2
    exit 1
  fi
  grep -Fq -- 'provision-db-secrets: --apply must run as root' \
    "$out_dir/db-secret-root.out" || {
    cat "$out_dir/db-secret-root.out" >&2
    exit 1
  }
fi

if [ "$(id -u)" -eq 0 ]; then
  db_unsafe_source="$out_dir/db-secret-unsafe"
  mkdir -p "$db_unsafe_source"
  chmod 0770 "$db_unsafe_source"
  if LAGRANGE_SECRET_SOURCE_DIR="$db_unsafe_source" \
     bash "$ops/provision-db-secrets.sh" --apply >"$out_dir/db-secret-unsafe.out" 2>&1; then
    echo 'self-test: writable DB secret source directory was unexpectedly accepted' >&2
    exit 1
  fi
  grep -Fq 'source directory must not be group/other writable' \
    "$out_dir/db-secret-unsafe.out"
  [ "$(find "$db_unsafe_source" -maxdepth 1 -type f -printf '%f\n' | wc -l)" -eq 0 ]

  # 0750 is the production host-directory mode from provision-linux.sh.  It
  # must remain valid because group read/traverse is not group write access.
  db_apply_source="$out_dir/db-secret-apply"
  mkdir -p "$db_apply_source"
  chmod 0750 "$db_apply_source"
  [ "$(stat -c '%u:%a' -- "$db_apply_source")" = '0:750' ]
  db_apply_output=$(LAGRANGE_SECRET_SOURCE_DIR="$db_apply_source" \
    bash "$ops/provision-db-secrets.sh" --apply)
  grep -Fq 'APPLY: generated exactly seven distinct DB source secret files' \
    <<<"$db_apply_output"
  db_secret_names=(
    postgres_password
    db_migration_owner_password
    db_app_password
    db_worker_password
    db_audit_password
    db_research_password
    db_admin_password
  )
  for name in "${db_secret_names[@]}"; do
    db_file="$db_apply_source/$name"
    [ -f "$db_file" ] && [ ! -L "$db_file" ]
    [ "$(stat -c '%u:%g:%a' -- "$db_file")" = '0:0:600' ]
    [ "$(wc -c <"$db_file")" -eq 64 ]
    LC_ALL=C grep -Eq '^[0-9a-f]{64}$' -- "$db_file"
    value=$(<"$db_file")
    if grep -Fq -- "$value" <<<"$db_apply_output"; then
      echo "self-test: DB secret value leaked in apply output: $name" >&2
      exit 1
    fi
  done
  [ "$(find "$db_apply_source" -maxdepth 1 -type f -printf '%f\n' | wc -l)" -eq 7 ]
  for ((i = 0; i < ${#db_secret_names[@]}; i++)); do
    for ((j = i + 1; j < ${#db_secret_names[@]}; j++)); do
      if cmp -s "$db_apply_source/${db_secret_names[i]}" \
         "$db_apply_source/${db_secret_names[j]}"; then
        echo 'self-test: generated DB source values are not distinct' >&2
        exit 1
      fi
    done
  done

  db_existing_source="$out_dir/db-secret-existing"
  mkdir -p "$db_existing_source"
  printf '%s' sentinel >"$db_existing_source/db_app_password"
  if LAGRANGE_SECRET_SOURCE_DIR="$db_existing_source" \
     bash "$ops/provision-db-secrets.sh" --apply >"$out_dir/db-secret-existing.out" 2>&1; then
    echo 'self-test: existing DB secret target was unexpectedly overwritten' >&2
    exit 1
  fi
  grep -Fq 'refusing to overwrite existing DB source secret' \
    "$out_dir/db-secret-existing.out"
  [ "$(find "$db_existing_source" -maxdepth 1 -type f -printf '%f\n' | wc -l)" -eq 1 ]
  grep -Fxq sentinel "$db_existing_source/db_app_password"
fi

db_path_real="$out_dir/db-secret-path-real"
mkdir -p "$db_path_real"
ln -s "$db_path_real" "$out_dir/db-secret-path-link"
if LAGRANGE_SECRET_SOURCE_DIR="$out_dir/db-secret-path-link/secrets" \
   bash "$ops/provision-db-secrets.sh" --dry-run >"$out_dir/db-secret-symlink.out" 2>&1; then
  echo 'self-test: DB secret provision accepted a symlinked ancestor' >&2
  exit 1
fi
grep -Fq 'must not traverse a symlink' "$out_dir/db-secret-symlink.out"

if bash "$ops/backfill-production.sh" \
   --start 2026-02-30 --end 2026-03-01 --plan >"$out_dir/date.out" 2>&1; then
  echo 'self-test: backfill accepted an invalid calendar date' >&2
  exit 1
fi
grep -Fq 'invalid calendar date' "$out_dir/date.out"

cp "$root/deploy/compose/.env.example" "$out_dir/.env"
chmod 0600 "$out_dir/.env"
mkdir -p "$out_dir/source"
printf 'fixture-secret' >"$out_dir/source/postgres_password"
chmod 0644 "$out_dir/source/postgres_password"
sed -i \
  -e "s|^LAGRANGE_DATA_DIR=.*|LAGRANGE_DATA_DIR=$out_dir/data|" \
  -e "s|^LAGRANGE_ARTIFACTS_DIR=.*|LAGRANGE_ARTIFACTS_DIR=$out_dir/data/artifacts|" \
  -e "s|^LAGRANGE_SECRET_SOURCE_DIR=.*|LAGRANGE_SECRET_SOURCE_DIR=$out_dir/source|" \
  -e "s|^LAGRANGE_RUNTIME_SECRET_DIR=.*|LAGRANGE_RUNTIME_SECRET_DIR=$out_dir/runtime|" \
  "$out_dir/.env"
if LAGRANGE_ENV_FILE="$out_dir/.env" \
   LAGRANGE_CODE_COMMIT=0000000000000000000000000000000000000000 \
   bash "$ops/validate-production-config.sh" >"$out_dir/config.out" 2>&1; then
  echo 'self-test: template unexpectedly passed production validation' >&2
  exit 1
else
  grep -Fq 'secret postgres_password must be mode 0400 or 0600' "$out_dir/config.out" || {
    cat "$out_dir/config.out" >&2
    exit 1
  }
fi

if LAGRANGE_ENV_FILE="$out_dir/.env" \
   LAGRANGE_CODE_COMMIT=0000000000000000000000000000000000000000 \
   bash "$ops/validate-production-config.sh" --scope backfill >"$out_dir/backfill-config.out" 2>&1; then
  echo 'self-test: backfill scope unexpectedly passed incomplete fixtures' >&2
  exit 1
else
  grep -Eq '^(INVALID_CONFIG|BLOCKED_EXTERNAL):' "$out_dir/backfill-config.out"
  if grep -Eq 'RECOMMENDATION_DATASET_|AUTH0_DOMAIN|TLS file' "$out_dir/backfill-config.out"; then
    echo 'self-test: backfill scope requested serving-only values' >&2
    cat "$out_dir/backfill-config.out" >&2
    exit 1
  fi
fi

sed -E \
  '/^(RESEARCH_ENTITLEMENT_REFERENCE|RESEARCH_APP_ENV|RESEARCH_FETCH_MODE|RESEARCH_CANDIDATE_ENABLED|BACKTEST_MIN_FREE_BYTES|BACKTEST_MAX_QUEUED_BACKTESTS|BACKTEST_RECONCILE_GRACE_SECS|BACKTEST_RECONCILE_INTERVAL_SECS)=/d' \
  "$out_dir/.env" >"$out_dir/infrastructure-minimal.env"
chmod 0600 "$out_dir/infrastructure-minimal.env"
if LAGRANGE_ENV_FILE="$out_dir/infrastructure-minimal.env" \
   LAGRANGE_CODE_COMMIT=0000000000000000000000000000000000000000 \
   bash "$ops/validate-production-config.sh" --scope infrastructure >"$out_dir/infrastructure-config.out" 2>&1; then
  echo 'self-test: infrastructure scope unexpectedly passed incomplete fixtures' >&2
  exit 1
else
  grep -Eq '^(INVALID_CONFIG|BLOCKED_EXTERNAL):' "$out_dir/infrastructure-config.out"
  if grep -Eq 'kis_app_key|kis_app_secret|RECOMMENDATION_DATASET_|AUTH0_DOMAIN|TLS file|RESEARCH_ENTITLEMENT_REFERENCE|RESEARCH_APP_ENV|RESEARCH_FETCH_MODE|RESEARCH_CANDIDATE_ENABLED' \
     "$out_dir/infrastructure-config.out"; then
    echo 'self-test: infrastructure scope requested deferred credentials/serving values' >&2
    cat "$out_dir/infrastructure-config.out" >&2
    exit 1
  fi
fi

# DB role credentials must not be reused. Build a shape-valid fixture with one
# duplicate pair and verify the validator reports only the filenames, never
# the shared credential value.
mkdir -p "$out_dir/db-source-equality"
for name in postgres_password db_migration_owner_password db_app_password \
  db_worker_password db_audit_password db_research_password db_admin_password; do
  case "$name" in
    postgres_password|db_migration_owner_password) value=same-db-password ;;
    *) value="unique-$name" ;;
  esac
  printf '%s' "$value" >"$out_dir/db-source-equality/$name"
  chmod 0600 "$out_dir/db-source-equality/$name"
done
cp "$out_dir/.env" "$out_dir/db-source-equality.env"
sed -i \
  -e "s|^LAGRANGE_SECRET_SOURCE_DIR=.*|LAGRANGE_SECRET_SOURCE_DIR=$out_dir/db-source-equality|" \
  "$out_dir/db-source-equality.env"
if LAGRANGE_ENV_FILE="$out_dir/db-source-equality.env" \
   LAGRANGE_CODE_COMMIT=0000000000000000000000000000000000000000 \
   bash "$ops/validate-production-config.sh" --scope infrastructure \
   >"$out_dir/db-source-equality.out" 2>&1; then
  echo 'self-test: duplicate DB source secrets unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'INVALID_CONFIG: production configuration is unsafe or inconsistent' \
  "$out_dir/db-source-equality.out" || {
  cat "$out_dir/db-source-equality.out" >&2
  exit 1
}
grep -Fq 'postgres_password conflicts with db_migration_owner_password' \
  "$out_dir/db-source-equality.out" || {
  cat "$out_dir/db-source-equality.out" >&2
  exit 1
}
if grep -Fq 'same-db-password' "$out_dir/db-source-equality.out"; then
  echo 'self-test: duplicate DB secret value leaked in validator output' >&2
  exit 1
fi

# Compose expands inactive services too. Exercise the actual infrastructure
# compose() helper with a minimal env that omits every deferred research and
# backtest setting; a fake Docker client captures the process-local sentinels
# without contacting a daemon or starting a service.
mkdir -p "$out_dir/infra/scripts/ops/lib" "$out_dir/infra/bin"
cp "$ops/compose-release.sh" "$out_dir/infra/scripts/ops/compose-release.sh"
cp "$ops/lib/dotenv.sh" "$out_dir/infra/scripts/ops/lib-dotenv.tmp"
mv "$out_dir/infra/scripts/ops/lib-dotenv.tmp" "$out_dir/infra/scripts/ops/lib/dotenv.sh"
printf '%s\n' '#!/usr/bin/env bash' 'exit 0' >"$out_dir/infra/scripts/ops/validate-production-config.sh"
chmod 0755 "$out_dir/infra/scripts/ops/compose-release.sh" \
  "$out_dir/infra/scripts/ops/validate-production-config.sh"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'if [ "${1-}" = compose ]; then' \
  '  printf "%s\n" "RESEARCH_APP_ENV=${RESEARCH_APP_ENV-}" "RESEARCH_ENTITLEMENT_REFERENCE=${RESEARCH_ENTITLEMENT_REFERENCE-}" "BACKTEST_MIN_FREE_BYTES=${BACKTEST_MIN_FREE_BYTES-}" "BACKTEST_MAX_QUEUED_BACKTESTS=${BACKTEST_MAX_QUEUED_BACKTESTS-}" "BACKTEST_RECONCILE_GRACE_SECS=${BACKTEST_RECONCILE_GRACE_SECS-}" "BACKTEST_RECONCILE_INTERVAL_SECS=${BACKTEST_RECONCILE_INTERVAL_SECS-}" >"${CAPTURE_PATH:?}"' \
  'fi' \
  'exit 0' >"$out_dir/infra/bin/docker"
chmod 0755 "$out_dir/infra/bin/docker"
if PATH="$out_dir/infra/bin:$PATH" \
   CAPTURE_PATH="$out_dir/infrastructure-compose-env.out" \
   LAGRANGE_ENV_FILE="$out_dir/infrastructure-minimal.env" \
   LAGRANGE_COMPOSE_FILE="$root/deploy/compose/compose.yml" \
   LAGRANGE_CODE_COMMIT=0000000000000000000000000000000000000000 \
   bash "$out_dir/infra/scripts/ops/compose-release.sh" --scope infrastructure --plan \
   >"$out_dir/infrastructure-compose.out" 2>&1; then
  for expected in \
    'RESEARCH_APP_ENV=infrastructure-disabled' \
    'RESEARCH_ENTITLEMENT_REFERENCE=infrastructure-disabled' \
    'BACKTEST_MIN_FREE_BYTES=0' \
    'BACKTEST_MAX_QUEUED_BACKTESTS=0' \
    'BACKTEST_RECONCILE_GRACE_SECS=0' \
    'BACKTEST_RECONCILE_INTERVAL_SECS=0'; do
    grep -Fxq "$expected" "$out_dir/infrastructure-compose-env.out" || {
      echo "self-test: missing infrastructure Compose sentinel: $expected" >&2
      cat "$out_dir/infrastructure-compose-env.out" >&2
      exit 1
    }
  done
else
  echo 'self-test: infrastructure Compose sentinel helper failed' >&2
  cat "$out_dir/infrastructure-compose.out" >&2
  exit 1
fi

# Compose env-file interpolation must not turn an apparently empty profile
# into live when an unrelated shell variable is exported.
sed 's/^COMPOSE_PROFILES=.*/COMPOSE_PROFILES=${P:-}/' \
  "$out_dir/.env" >"$out_dir/interpolation.env"
chmod 0600 "$out_dir/interpolation.env"
if LAGRANGE_ENV_FILE="$out_dir/interpolation.env" \
   LAGRANGE_CODE_COMMIT=0000000000000000000000000000000000000000 P=live \
   bash "$ops/validate-production-config.sh" --scope backfill >"$out_dir/interpolation.out" 2>&1; then
  echo 'self-test: Compose env-file interpolation bypassed the literal contract' >&2
  exit 1
fi
grep -Fq 'dotenv value for COMPOSE_PROFILES uses Compose interpolation' \
  "$out_dir/interpolation.out" || {
  cat "$out_dir/interpolation.out" >&2
  exit 1
}

# A shell variable has higher precedence than Compose's --env-file. Every
# mutating/readiness path must reject a mismatched effective value before it
# reaches Docker, so a synthetic fetch mode cannot bypass the production gate.
mkdir -p "$out_dir/bin"
printf '%s\n' '#!/usr/bin/env bash' 'exit 0' >"$out_dir/bin/docker"
chmod 0755 "$out_dir/bin/docker"
for path in compose backfill health; do
  case "$path" in
    compose)
      command=(bash "$ops/compose-release.sh" --scope backfill --plan) ;;
    backfill)
      command=(bash "$ops/backfill-production.sh" --start 2026-01-01 --end 2026-01-01 --execute) ;;
    health)
      command=(bash "$ops/post-backfill-health.sh" --scope backfill --check) ;;
  esac
  if PATH="$out_dir/bin:$PATH" LAGRANGE_ENV_FILE="$out_dir/.env" \
     LAGRANGE_CODE_COMMIT=0000000000000000000000000000000000000000 \
     RESEARCH_FETCH_MODE=synthetic \
     BACKFILL_CONFIRM_EXTERNAL=I_UNDERSTAND_READ_ONLY_KIS_CALLS \
     "${command[@]}" >"$out_dir/$path-override.out" 2>&1; then
    echo "self-test: $path path accepted a mismatched shell fetch mode" >&2
    exit 1
  fi
  grep -Fq 'shell override for RESEARCH_FETCH_MODE does not exactly match env-file value' \
    "$out_dir/$path-override.out" || {
    cat "$out_dir/$path-override.out" >&2
    exit 1
  }
done

plan=$(LAGRANGE_ENV_FILE="$out_dir/.env" \
  bash "$ops/backfill-production.sh" --start 2026-01-01 --end 2026-01-03 --plan)
grep -Fq 'PLAN_ONLY: no KIS call' <<<"$plan"
grep -Fq "state: $out_dir/data/backfill/state.tsv" <<<"$plan"
if grep -Fq 'docker compose' <<<"$plan"; then
  echo 'self-test: backfill plan attempted an external command' >&2
  exit 1
fi
health_plan=$(bash "$ops/post-backfill-health.sh" --plan)
grep -Fq 'POST_BACKFILL_HEALTH_GATE: scope=backfill' <<<"$health_plan"
grep -Fq 'PLAN_ONLY: no Docker, DB, provider, or file operation made' <<<"$health_plan"
grep -Fq 'research-worker healthcheck' <<<"$health_plan"
echo 'OPS_SELF_TEST: PASS'
