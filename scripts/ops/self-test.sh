#!/usr/bin/env bash
# No-infrastructure self-test for the operator workflows.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ops="$root/scripts/ops"
out_dir=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-ops-self-test.XXXXXX")
trap 'rm -rf -- "$out_dir"' EXIT

for script in provision-linux.sh provision-db-secrets.sh provision-auth0-secret.sh \
  provision-crypto-secrets.sh provision-kis-credentials.sh validate-production-config.sh compose-release.sh \
  backfill-production.sh post-backfill-health.sh renew-tailscale-tls.sh \
  install-tailscale-tls-renewal.sh tailscale-tls-self-test.sh \
  build-production-images.sh build-production-images-static-check.sh \
  build-production-images-self-test.sh deploy-production-release.sh \
  run-production-backup.sh install-production-backup.sh \
  production-ops-static-check.sh production-ops-self-test.sh; do
  bash -n "$ops/$script"
done
bash "$root/deploy/secrets/runtime-static-check.sh" >/dev/null
bash "$root/deploy/secrets/provision-runtime-secrets.sh" --help >/dev/null
bash "$root/deploy/systemd/paper-runner-static-check.sh" >/dev/null
bash "$root/deploy/db/migrate-static-check.sh" >/dev/null
bash "$root/scripts/qa/research-worker-smoke.sh" --static-only >/dev/null
bash "$root/scripts/qa/recommendation-runner-smoke.sh" --static-only >/dev/null
bash "$ops/static-check.sh" >/dev/null
bash "$ops/tailscale-tls-self-test.sh" >/dev/null
bash "$ops/build-production-images-self-test.sh" >/dev/null
python3 - "$ops/lib/backfill-progress.py" <<'PY'
import pathlib
import sys
compile(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"), sys.argv[1], "exec")
PY

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

# Auth0 client-secret provisioning is intentionally interactive and is never
# exercised with a real credential here.  Exercise its default plan, root-only
# guards, protected-path fence, and read-only shape reporting with deterministic
# fixture text; assert that the fixture value never appears in output.
auth0_secret_plan=$(bash "$ops/provision-auth0-secret.sh" --dry-run \
  --source-dir "$out_dir/auth0-plan-source" 2>"$out_dir/auth0-plan.err")
grep -Fq 'AUTH0_SECRET_PROVISION mode=dry-run' <<<"$auth0_secret_plan"
grep -Fq 'DRY_RUN: no files created' <<<"$auth0_secret_plan"
[ ! -e "$out_dir/auth0-plan-source" ]

auth0_source="$out_dir/auth0-source"
mkdir -p "$auth0_source"
chmod 0750 "$auth0_source"
if [ "$(id -u)" -eq 0 ] && command -v runuser >/dev/null 2>&1; then
  auth0_apply_guard_cmd=(runuser -u nobody -- \
    bash "$ops/provision-auth0-secret.sh" --apply --source-dir "$auth0_source")
  auth0_check_guard_cmd=(runuser -u nobody -- \
    bash "$ops/provision-auth0-secret.sh" --check --source-dir "$auth0_source")
elif [ "$(id -u)" -ne 0 ]; then
  auth0_apply_guard_cmd=(bash "$ops/provision-auth0-secret.sh" --apply --source-dir "$auth0_source")
  auth0_check_guard_cmd=(bash "$ops/provision-auth0-secret.sh" --check --source-dir "$auth0_source")
else
  auth0_apply_guard_cmd=()
  auth0_check_guard_cmd=()
fi
if [ "${#auth0_apply_guard_cmd[@]}" -gt 0 ]; then
  if "${auth0_apply_guard_cmd[@]}" >"$out_dir/auth0-apply-root.out" 2>&1; then
    echo 'self-test: non-root Auth0 secret apply unexpectedly passed' >&2
    exit 1
  fi
  grep -Fq -- 'provision-auth0-secret: --apply must run as root' \
    "$out_dir/auth0-apply-root.out" || {
    cat "$out_dir/auth0-apply-root.out" >&2
    exit 1
  }
  if "${auth0_check_guard_cmd[@]}" >"$out_dir/auth0-check-root.out" 2>&1; then
    echo 'self-test: non-root Auth0 secret check unexpectedly passed' >&2
    exit 1
  fi
  grep -Fq -- 'provision-auth0-secret: --check must run as root' \
    "$out_dir/auth0-check-root.out" || {
    cat "$out_dir/auth0-check-root.out" >&2
    exit 1
  }
fi

auth0_path_real="$out_dir/auth0-path-real"
mkdir -p "$auth0_path_real"
ln -s "$auth0_path_real" "$out_dir/auth0-path-link"
if bash "$ops/provision-auth0-secret.sh" --dry-run \
   --source-dir "$out_dir/auth0-path-link/secrets" >"$out_dir/auth0-symlink.out" 2>&1; then
  echo 'self-test: Auth0 secret provision accepted a symlinked ancestor' >&2
  exit 1
fi
grep -Fq 'must not traverse a symlink' "$out_dir/auth0-symlink.out"

if [ "$(id -u)" -eq 0 ]; then
  auth0_fixture='safe-test-value-0123456789'
  printf '%s' "$auth0_fixture" >"$auth0_source/auth0_client_secret"
  chown root:root -- "$auth0_source/auth0_client_secret"
  chmod 0600 -- "$auth0_source/auth0_client_secret"
  if bash "$ops/provision-auth0-secret.sh" --check \
     --source-dir "$auth0_source" \
     >"$out_dir/auth0-check-valid.out" 2>&1; then
    grep -Fxq 'AUTH0_SECRET_CHECK: PASS' "$out_dir/auth0-check-valid.out"
  else
    cat "$out_dir/auth0-check-valid.out" >&2
    exit 1
  fi
  if grep -Fq -- "$auth0_fixture" "$out_dir/auth0-check-valid.out"; then
    echo 'self-test: Auth0 fixture value leaked in valid check output' >&2
    exit 1
  fi

  printf '%s' 'your-client-secret' >"$auth0_source/auth0_client_secret"
  if bash "$ops/provision-auth0-secret.sh" --check \
     --source-dir "$auth0_source" \
     >"$out_dir/auth0-check-placeholder.out" 2>&1; then
    echo 'self-test: Auth0 placeholder unexpectedly passed --check' >&2
    exit 1
  fi
  grep -Fq 'looks like a placeholder' "$out_dir/auth0-check-placeholder.out"
  if grep -Fq -- 'your-client-secret' "$out_dir/auth0-check-placeholder.out"; then
    echo 'self-test: Auth0 placeholder leaked in check output' >&2
    exit 1
  fi

  printf '%s\n' "$auth0_fixture" >"$auth0_source/auth0_client_secret"
  if bash "$ops/provision-auth0-secret.sh" --check \
     --source-dir "$auth0_source" \
     >"$out_dir/auth0-check-newline.out" 2>&1; then
    echo 'self-test: newline-terminated Auth0 secret unexpectedly passed --check' >&2
    exit 1
  fi
  grep -Fq 'must be one non-empty line' "$out_dir/auth0-check-newline.out"
  if grep -Fq -- "$auth0_fixture" "$out_dir/auth0-check-newline.out"; then
    echo 'self-test: Auth0 fixture value leaked in newline check output' >&2
    exit 1
  fi

  printf '%s' 'existing-auth0-fixture' >"$auth0_source/auth0_client_secret"
  if bash "$ops/provision-auth0-secret.sh" --apply \
     --source-dir "$auth0_source" \
     >"$out_dir/auth0-existing-apply.out" 2>&1; then
    echo 'self-test: existing Auth0 target unexpectedly accepted --apply' >&2
    exit 1
  fi
  grep -Fq 'refusing to overwrite existing Auth0 client secret' \
    "$out_dir/auth0-existing-apply.out"
  grep -Fxq 'existing-auth0-fixture' "$auth0_source/auth0_client_secret"
  if grep -Fq -- 'existing-auth0-fixture' "$out_dir/auth0-existing-apply.out"; then
    echo 'self-test: existing Auth0 fixture value leaked in apply output' >&2
    exit 1
  fi

  auth0_import_source="$out_dir/auth0-import-source"
  auth0_import_target="$out_dir/auth0-import-target"
  mkdir -p "$auth0_import_source" "$auth0_import_target"
  chmod 0750 "$auth0_import_source" "$auth0_import_target"
  auth0_import_fixture='legacy-test-value-987654321'
  printf '%s' "$auth0_import_fixture" >"$auth0_import_source/legacy-secret"
  chown root:root -- "$auth0_import_source/legacy-secret"
  chmod 0600 -- "$auth0_import_source/legacy-secret"
  auth0_import_output=$(bash "$ops/provision-auth0-secret.sh" \
    --import-file "$auth0_import_source/legacy-secret" \
    --source-dir "$auth0_import_target" 2>&1)
  grep -Fq 'AUTH0_SECRET_PROVISION mode=import' <<<"$auth0_import_output"
  [ "$(stat -c '%u:%g:%a' -- "$auth0_import_target/auth0_client_secret")" = '0:0:600' ]
  cmp -s "$auth0_import_source/legacy-secret" \
    "$auth0_import_target/auth0_client_secret"
  if grep -Fq -- "$auth0_import_fixture" <<<"$auth0_import_output"; then
    echo 'self-test: imported Auth0 fixture value leaked in import output' >&2
    exit 1
  fi
  if bash "$ops/provision-auth0-secret.sh" \
     --import-file "$auth0_import_source/legacy-secret" \
     --source-dir "$auth0_import_target" \
     >"$out_dir/auth0-import-existing.out" 2>&1; then
    echo 'self-test: existing Auth0 target unexpectedly accepted --import-file' >&2
    exit 1
  fi
  grep -Fq 'refusing to overwrite existing Auth0 client secret' \
    "$out_dir/auth0-import-existing.out"
  if grep -Fq -- "$auth0_import_fixture" "$out_dir/auth0-import-existing.out"; then
    echo 'self-test: imported Auth0 fixture value leaked on target refusal' >&2
    exit 1
  fi
fi

# KIS app-key/app-secret provisioning is interactive and is never exercised
# with a real credential here. Exercise its plan, root-only guards, path fence,
# read-only shape checks, pairwise distinctness, no-overwrite gate, and output
# non-disclosure with deterministic fixture text.
kis_credential_plan=$(bash "$ops/provision-kis-credentials.sh" --dry-run \
  --source-dir "$out_dir/kis-plan-source")
grep -Fq 'KIS_CREDENTIAL_PROVISION mode=dry-run' <<<"$kis_credential_plan"
grep -Fq 'DRY_RUN: no files created' <<<"$kis_credential_plan"
grep -Fq 'source directory is absent or protected from current user' <<<"$kis_credential_plan"
[ ! -e "$out_dir/kis-plan-source" ]

kis_source="$out_dir/kis-source"
mkdir -p "$kis_source"
chmod 0750 "$kis_source"
if [ "$(id -u)" -eq 0 ] && command -v runuser >/dev/null 2>&1; then
  kis_apply_guard_cmd=(runuser -u nobody -- \
    bash "$ops/provision-kis-credentials.sh" --apply --source-dir "$kis_source")
  kis_check_guard_cmd=(runuser -u nobody -- \
    bash "$ops/provision-kis-credentials.sh" --check --source-dir "$kis_source")
elif [ "$(id -u)" -ne 0 ]; then
  kis_apply_guard_cmd=(bash "$ops/provision-kis-credentials.sh" --apply --source-dir "$kis_source")
  kis_check_guard_cmd=(bash "$ops/provision-kis-credentials.sh" --check --source-dir "$kis_source")
else
  kis_apply_guard_cmd=()
  kis_check_guard_cmd=()
fi
if [ "${#kis_apply_guard_cmd[@]}" -gt 0 ]; then
  if "${kis_apply_guard_cmd[@]}" >"$out_dir/kis-apply-root.out" 2>&1; then
    echo 'self-test: non-root KIS credential apply unexpectedly passed' >&2
    exit 1
  fi
  grep -Fq -- 'provision-kis-credentials: --apply must run as root' \
    "$out_dir/kis-apply-root.out" || {
    cat "$out_dir/kis-apply-root.out" >&2
    exit 1
  }
  if "${kis_check_guard_cmd[@]}" >"$out_dir/kis-check-root.out" 2>&1; then
    echo 'self-test: non-root KIS credential check unexpectedly passed' >&2
    exit 1
  fi
  grep -Fq -- 'provision-kis-credentials: --check must run as root' \
    "$out_dir/kis-check-root.out" || {
    cat "$out_dir/kis-check-root.out" >&2
    exit 1
  }
fi

kis_path_real="$out_dir/kis-path-real"
mkdir -p "$kis_path_real"
ln -s "$kis_path_real" "$out_dir/kis-path-link"
if bash "$ops/provision-kis-credentials.sh" --dry-run \
   --source-dir "$out_dir/kis-path-link/secrets" >"$out_dir/kis-symlink.out" 2>&1; then
  echo 'self-test: KIS credential provision accepted a symlinked ancestor' >&2
  exit 1
fi
grep -Fq 'must not traverse a symlink' "$out_dir/kis-symlink.out"

kis_key_fixture='alpha-value-0123456789'
kis_secret_fixture='beta-value-9876543210'
if [ "$(id -u)" -eq 0 ]; then
  if command -v script >/dev/null 2>&1; then
    kis_apply_source="$out_dir/kis-apply-source"
    mkdir -p "$kis_apply_source"
    chmod 0750 "$kis_apply_source"
    kis_apply_output=$(printf '%s\n' \
      "$kis_key_fixture" "$kis_key_fixture" \
      "$kis_secret_fixture" "$kis_secret_fixture" | \
      script -qefE never -c \
      "bash '$ops/provision-kis-credentials.sh' --apply --source-dir '$kis_apply_source'" \
      /dev/null 2>&1)
    grep -Fq 'KIS_CREDENTIAL_PROVISION mode=apply' <<<"$kis_apply_output"
    for name in kis_app_key kis_app_secret; do
      kis_file="$kis_apply_source/$name"
      [ "$(stat -c '%u:%g:%a' -- "$kis_file")" = '0:0:600' ]
      [ "$(wc -l <"$kis_file")" -eq 0 ]
      [ "$(wc -c <"$kis_file")" -gt 0 ]
    done
    if grep -Eq "$kis_key_fixture|$kis_secret_fixture" <<<"$kis_apply_output"; then
      echo 'self-test: KIS fixture value leaked in interactive apply output' >&2
      exit 1
    fi
  fi
  printf '%s' "$kis_key_fixture" >"$kis_source/kis_app_key"
  printf '%s' "$kis_secret_fixture" >"$kis_source/kis_app_secret"
  chown root:root -- "$kis_source/kis_app_key" "$kis_source/kis_app_secret"
  chmod 0600 -- "$kis_source/kis_app_key" "$kis_source/kis_app_secret"
  if bash "$ops/provision-kis-credentials.sh" --check \
     --source-dir "$kis_source" >"$out_dir/kis-check-valid.out" 2>&1; then
    grep -Fxq 'KIS_CREDENTIAL_CHECK: PASS' "$out_dir/kis-check-valid.out"
  else
    cat "$out_dir/kis-check-valid.out" >&2
    exit 1
  fi
  if grep -Eq "$kis_key_fixture|$kis_secret_fixture" "$out_dir/kis-check-valid.out"; then
    echo 'self-test: KIS fixture value leaked in valid check output' >&2
    exit 1
  fi

  printf '%s\n' "$kis_key_fixture" >"$kis_source/kis_app_key"
  if bash "$ops/provision-kis-credentials.sh" --check \
     --source-dir "$kis_source" >"$out_dir/kis-check-newline.out" 2>&1; then
    echo 'self-test: newline-terminated KIS credential unexpectedly passed --check' >&2
    exit 1
  fi
  grep -Fq 'one non-empty printable whitespace-free line' "$out_dir/kis-check-newline.out"
  if grep -Fq -- "$kis_key_fixture" "$out_dir/kis-check-newline.out"; then
    echo 'self-test: KIS fixture value leaked in newline check output' >&2
    exit 1
  fi

  printf '%s' 'replace-me' >"$kis_source/kis_app_key"
  printf '%s' "$kis_secret_fixture" >"$kis_source/kis_app_secret"
  if bash "$ops/provision-kis-credentials.sh" --check \
     --source-dir "$kis_source" >"$out_dir/kis-check-placeholder.out" 2>&1; then
    echo 'self-test: KIS placeholder unexpectedly passed --check' >&2
    exit 1
  fi
  grep -Fq 'no placeholder' "$out_dir/kis-check-placeholder.out"
  if grep -Fq 'replace-me' "$out_dir/kis-check-placeholder.out"; then
    echo 'self-test: KIS placeholder leaked in check output' >&2
    exit 1
  fi

  printf '%s' "$kis_key_fixture" >"$kis_source/kis_app_key"
  printf '%s' "$kis_key_fixture" >"$kis_source/kis_app_secret"
  if bash "$ops/provision-kis-credentials.sh" --check \
     --source-dir "$kis_source" >"$out_dir/kis-check-duplicate.out" 2>&1; then
    echo 'self-test: duplicate KIS credentials unexpectedly passed --check' >&2
    exit 1
  fi
  grep -Fq 'values must differ' "$out_dir/kis-check-duplicate.out"
  if grep -Fq -- "$kis_key_fixture" "$out_dir/kis-check-duplicate.out"; then
    echo 'self-test: duplicate KIS fixture leaked in check output' >&2
    exit 1
  fi

  printf '%s' 'existing-target-sentinel' >"$kis_source/kis_app_key"
  printf '%s' "$kis_secret_fixture" >"$kis_source/kis_app_secret"
  if bash "$ops/provision-kis-credentials.sh" --apply \
     --source-dir "$kis_source" >"$out_dir/kis-existing-apply.out" 2>&1; then
    echo 'self-test: existing KIS target unexpectedly accepted --apply' >&2
    exit 1
  fi
  grep -Fq 'refusing to overwrite an existing KIS credential' \
    "$out_dir/kis-existing-apply.out"
  grep -Fxq 'existing-target-sentinel' "$kis_source/kis_app_key"
  [ "$(find "$kis_source" -maxdepth 1 -name '.lagrange-kis-credentials.*' -print | wc -l)" -eq 0 ]
  if grep -Fq 'existing-target-sentinel' "$out_dir/kis-existing-apply.out"; then
    echo 'self-test: existing KIS fixture value leaked in apply output' >&2
    exit 1
  fi
fi

# The four non-KIS cryptographic source secrets use a fixed 32-byte contract
# represented as exactly 64 lowercase hex bytes. Exercise plans, root guards,
# path fences, generation/check shape, pairwise distinctness, placeholder
# rejection, no output leakage, and the no-overwrite gate in a temp tree.
crypto_secret_plan=$(bash "$ops/provision-crypto-secrets.sh" --dry-run \
  --source-dir "$out_dir/crypto-plan-source")
grep -Fq 'CRYPTO_SECRET_PROVISION mode=dry-run' <<<"$crypto_secret_plan"
grep -Fq 'DRY_RUN: no files created' <<<"$crypto_secret_plan"
[ ! -e "$out_dir/crypto-plan-source" ]

crypto_source="$out_dir/crypto-source"
mkdir -p "$crypto_source"
chmod 0750 "$crypto_source"
if [ "$(id -u)" -eq 0 ] && command -v runuser >/dev/null 2>&1; then
  crypto_apply_guard_cmd=(runuser -u nobody -- \
    bash "$ops/provision-crypto-secrets.sh" --apply --source-dir "$crypto_source")
  crypto_check_guard_cmd=(runuser -u nobody -- \
    bash "$ops/provision-crypto-secrets.sh" --check --source-dir "$crypto_source")
elif [ "$(id -u)" -ne 0 ]; then
  crypto_apply_guard_cmd=(bash "$ops/provision-crypto-secrets.sh" --apply --source-dir "$crypto_source")
  crypto_check_guard_cmd=(bash "$ops/provision-crypto-secrets.sh" --check --source-dir "$crypto_source")
else
  crypto_apply_guard_cmd=()
  crypto_check_guard_cmd=()
fi
if [ "${#crypto_apply_guard_cmd[@]}" -gt 0 ]; then
  if "${crypto_apply_guard_cmd[@]}" >"$out_dir/crypto-apply-root.out" 2>&1; then
    echo 'self-test: non-root crypto secret apply unexpectedly passed' >&2
    exit 1
  fi
  grep -Fq -- 'provision-crypto-secrets: --apply must run as root' \
    "$out_dir/crypto-apply-root.out" || {
    cat "$out_dir/crypto-apply-root.out" >&2
    exit 1
  }
  if "${crypto_check_guard_cmd[@]}" >"$out_dir/crypto-check-root.out" 2>&1; then
    echo 'self-test: non-root crypto secret check unexpectedly passed' >&2
    exit 1
  fi
  grep -Fq -- 'provision-crypto-secrets: --check must run as root' \
    "$out_dir/crypto-check-root.out" || {
    cat "$out_dir/crypto-check-root.out" >&2
    exit 1
  }
fi

crypto_path_real="$out_dir/crypto-path-real"
mkdir -p "$crypto_path_real"
ln -s "$crypto_path_real" "$out_dir/crypto-path-link"
if bash "$ops/provision-crypto-secrets.sh" --dry-run \
   --source-dir "$out_dir/crypto-path-link/secrets" >"$out_dir/crypto-symlink.out" 2>&1; then
  echo 'self-test: crypto secret provision accepted a symlinked ancestor' >&2
  exit 1
fi
grep -Fq 'must not traverse a symlink' "$out_dir/crypto-symlink.out"

if [ "$(id -u)" -eq 0 ]; then
  crypto_apply_source="$out_dir/crypto-apply-source"
  mkdir -p "$crypto_apply_source"
  chmod 0750 "$crypto_apply_source"
  crypto_apply_output=$(bash "$ops/provision-crypto-secrets.sh" \
    --apply --source-dir "$crypto_apply_source")
  grep -Fq 'CRYPTO_SECRET_PROVISION mode=apply' <<<"$crypto_apply_output"
  crypto_secret_names=(session_secret csrf_secret cursor_secret backup_encryption_key)
  for name in "${crypto_secret_names[@]}"; do
    crypto_file="$crypto_apply_source/$name"
    [ -f "$crypto_file" ] && [ ! -L "$crypto_file" ]
    [ "$(stat -c '%u:%g:%a' -- "$crypto_file")" = '0:0:600' ]
    [ "$(wc -c <"$crypto_file")" -eq 64 ]
    LC_ALL=C grep -Eq '^[0-9a-f]{64}$' -- "$crypto_file"
    crypto_value=$(<"$crypto_file")
    if grep -Fq -- "$crypto_value" <<<"$crypto_apply_output"; then
      echo "self-test: crypto secret value leaked in apply output: $name" >&2
      exit 1
    fi
  done
  [ "$(find "$crypto_apply_source" -maxdepth 1 -type f -printf '%f\n' | wc -l)" -eq 4 ]
  for ((i = 0; i < ${#crypto_secret_names[@]}; i++)); do
    for ((j = i + 1; j < ${#crypto_secret_names[@]}; j++)); do
      if cmp -s "$crypto_apply_source/${crypto_secret_names[i]}" \
         "$crypto_apply_source/${crypto_secret_names[j]}"; then
        echo 'self-test: generated crypto source values are not distinct' >&2
        exit 1
      fi
    done
  done
  crypto_check_output=$(bash "$ops/provision-crypto-secrets.sh" \
    --check --source-dir "$crypto_apply_source")
  grep -Fxq 'CRYPTO_SECRET_CHECK: PASS' <<<"$crypto_check_output"
  for name in "${crypto_secret_names[@]}"; do
    crypto_value=$(<"$crypto_apply_source/$name")
    if grep -Fq -- "$crypto_value" <<<"$crypto_check_output"; then
      echo "self-test: crypto secret value leaked in check output: $name" >&2
      exit 1
    fi
  done

  crypto_placeholder_fixture='example-test-secret'
  printf '%s' "$crypto_placeholder_fixture" >"$crypto_apply_source/session_secret"
  if bash "$ops/provision-crypto-secrets.sh" --check \
     --source-dir "$crypto_apply_source" >"$out_dir/crypto-placeholder.out" 2>&1; then
    echo 'self-test: crypto placeholder unexpectedly passed --check' >&2
    exit 1
  fi
  grep -Fq 'must contain exactly 64 lowercase hex characters' \
    "$out_dir/crypto-placeholder.out"
  if grep -Fq -- "$crypto_placeholder_fixture" "$out_dir/crypto-placeholder.out"; then
    echo 'self-test: crypto placeholder value leaked in check output' >&2
    exit 1
  fi

  crypto_existing_source="$out_dir/crypto-existing-source"
  mkdir -p "$crypto_existing_source"
  chmod 0750 "$crypto_existing_source"
  printf '%s' sentinel >"$crypto_existing_source/session_secret"
  if bash "$ops/provision-crypto-secrets.sh" --apply \
     --source-dir "$crypto_existing_source" >"$out_dir/crypto-existing.out" 2>&1; then
    echo 'self-test: existing crypto target unexpectedly accepted --apply' >&2
    exit 1
  fi
  grep -Fq 'refusing to overwrite existing crypto secret' "$out_dir/crypto-existing.out"
  grep -Fxq sentinel "$crypto_existing_source/session_secret"
  [ "$(find "$crypto_existing_source" -maxdepth 1 -type f -printf '%f\n' | wc -l)" -eq 1 ]
fi

# DB source credentials are generated only by the explicit root apply mode.
# Exercise the non-root plan/check/apply guards in every environment, and
# exercise the complete file contract when this no-infrastructure test itself
# has root.
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

if [ "$(id -u)" -eq 0 ] && command -v runuser >/dev/null 2>&1; then
  db_check_guard_cmd=(runuser -u nobody -- env \
    "LAGRANGE_SECRET_SOURCE_DIR=$out_dir/db-secret-check-guard" \
    bash "$ops/provision-db-secrets.sh" --check)
elif [ "$(id -u)" -ne 0 ]; then
  db_check_guard_cmd=(env \
    "LAGRANGE_SECRET_SOURCE_DIR=$out_dir/db-secret-check-guard" \
    bash "$ops/provision-db-secrets.sh" --check)
else
  db_check_guard_cmd=()
fi
if [ "${#db_check_guard_cmd[@]}" -gt 0 ]; then
  if "${db_check_guard_cmd[@]}" >"$out_dir/db-secret-check-root.out" 2>&1; then
    echo 'self-test: non-root DB secret check unexpectedly passed' >&2
    exit 1
  fi
  grep -Fq -- 'provision-db-secrets: --check must run as root' \
    "$out_dir/db-secret-check-root.out" || {
    cat "$out_dir/db-secret-check-root.out" >&2
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

  db_check_output=$(LAGRANGE_SECRET_SOURCE_DIR="$db_apply_source" \
    bash "$ops/provision-db-secrets.sh" --check)
  grep -Fxq 'DB_SECRET_CHECK: PASS' <<<"$db_check_output"
  [ "$(find "$db_apply_source" -maxdepth 1 -type f -printf '%f\n' | wc -l)" -eq 7 ]

  # Existing operators may have generated the same 32 bytes as strict
  # standard Base64 (`openssl rand -base64 32`). Verify that accepted format,
  # plus malformed and short Base64 rejection, without exposing fixture values.
  db_check_base64_source="$out_dir/db-secret-check-base64"
  mkdir -p "$db_check_base64_source"
  chmod 0750 "$db_check_base64_source"
  for ((i = 0; i < ${#db_secret_names[@]}; i++)); do
    name=${db_secret_names[i]}
    printf '%032d' "$((i + 1))" | base64 | tr -d '\r\n' >"$db_check_base64_source/$name"
    chown root:root -- "$db_check_base64_source/$name"
    chmod 0600 -- "$db_check_base64_source/$name"
  done
  db_base64_output=$(LAGRANGE_SECRET_SOURCE_DIR="$db_check_base64_source" \
    bash "$ops/provision-db-secrets.sh" --check)
  grep -Fxq 'DB_SECRET_CHECK: PASS' <<<"$db_base64_output"
  db_base64_value=$(<"$db_check_base64_source/db_worker_password")
  printf '%s' "${db_base64_value:0:43}" >"$db_check_base64_source/db_app_password"
  chmod 0600 -- "$db_check_base64_source/db_app_password"
  if LAGRANGE_SECRET_SOURCE_DIR="$db_check_base64_source" \
     bash "$ops/provision-db-secrets.sh" --check >"$out_dir/db-secret-check-base64-short.out" 2>&1; then
    echo 'self-test: short Base64 DB secret unexpectedly passed --check' >&2
    exit 1
  fi
  grep -Fq 'DB_SECRET_CHECK: FAIL db_app_password:' \
    "$out_dir/db-secret-check-base64-short.out"
  if grep -Fq -- "$db_base64_value" "$out_dir/db-secret-check-base64-short.out"; then
    echo 'self-test: Base64 value leaked in short-format check output' >&2
    exit 1
  fi
  install -o root -g root -m 0600 -- \
    "$db_check_base64_source/db_worker_password" "$db_check_base64_source/db_app_password"
  printf '%s!%s' "${db_base64_value:0:42}" "${db_base64_value:43:1}" \
    >"$db_check_base64_source/db_app_password"
  chmod 0600 -- "$db_check_base64_source/db_app_password"
  if LAGRANGE_SECRET_SOURCE_DIR="$db_check_base64_source" \
     bash "$ops/provision-db-secrets.sh" --check >"$out_dir/db-secret-check-base64-malformed.out" 2>&1; then
    echo 'self-test: malformed Base64 DB secret unexpectedly passed --check' >&2
    exit 1
  fi
  grep -Fq 'DB_SECRET_CHECK: FAIL db_app_password:' \
    "$out_dir/db-secret-check-base64-malformed.out"

  # The explicit normalizer atomically repairs only a complete set containing
  # one LF terminator per 64-hex value; mixed sets are refused without writes.
  db_normalize_source="$out_dir/db-secret-normalize"
  mkdir -p "$db_normalize_source"
  chmod 0750 "$db_normalize_source"
  for name in "${db_secret_names[@]}"; do
    install -o root -g root -m 0600 -- \
      "$db_apply_source/$name" "$db_normalize_source/$name"
    printf '\n' >>"$db_normalize_source/$name"
  done
  if LAGRANGE_SECRET_SOURCE_DIR="$db_normalize_source" \
     bash "$ops/provision-db-secrets.sh" --check >"$out_dir/db-secret-check-newline.out" 2>&1; then
    echo 'self-test: newline-terminated DB secret set unexpectedly passed --check' >&2
    exit 1
  fi
  normalize_output=$(LAGRANGE_SECRET_SOURCE_DIR="$db_normalize_source" \
    bash "$ops/provision-db-secrets.sh" --strip-trailing-newline)
  grep -Fxq 'DB_SECRET_NORMALIZE: PASS' <<<"$normalize_output"
  for name in "${db_secret_names[@]}"; do
    [ "$(wc -c <"$db_normalize_source/$name")" -eq 64 ]
    cmp -s "$db_apply_source/$name" "$db_normalize_source/$name"
  done

  db_normalize_mixed_source="$out_dir/db-secret-normalize-mixed"
  mkdir -p "$db_normalize_mixed_source"
  chmod 0750 "$db_normalize_mixed_source"
  for name in "${db_secret_names[@]}"; do
    install -o root -g root -m 0600 -- \
      "$db_apply_source/$name" "$db_normalize_mixed_source/$name"
  done
  printf '\n' >>"$db_normalize_mixed_source/db_app_password"
  if LAGRANGE_SECRET_SOURCE_DIR="$db_normalize_mixed_source" \
     bash "$ops/provision-db-secrets.sh" --strip-trailing-newline >"$out_dir/db-secret-normalize-mixed.out" 2>&1; then
    echo 'self-test: mixed DB secret set unexpectedly passed normalization' >&2
    exit 1
  fi
  grep -Fq 'db_app_password' "$out_dir/db-secret-normalize-mixed.out"
  [ "$(wc -c <"$db_normalize_mixed_source/db_app_password")" -eq 65 ]

  # A complete, otherwise valid set with one missing target must fail without
  # creating or repairing anything, while naming the actionable filename.
  db_check_partial_source="$out_dir/db-secret-check-partial"
  mkdir -p "$db_check_partial_source"
  chmod 0750 "$db_check_partial_source"
  for name in "${db_secret_names[@]}"; do
    [ "$name" = db_admin_password ] && continue
    install -o root -g root -m 0600 -- \
      "$db_apply_source/$name" "$db_check_partial_source/$name"
  done
  if LAGRANGE_SECRET_SOURCE_DIR="$db_check_partial_source" \
     bash "$ops/provision-db-secrets.sh" --check >"$out_dir/db-secret-check-partial.out" 2>&1; then
    echo 'self-test: partial DB secret set unexpectedly passed --check' >&2
    exit 1
  fi
  grep -Fq 'DB_SECRET_CHECK: FAIL db_admin_password: missing file' \
    "$out_dir/db-secret-check-partial.out"
  [ "$(find "$db_check_partial_source" -maxdepth 1 -type f -printf '%f\n' | wc -l)" -eq 6 ]

  # Exercise the pairwise cmp gate as well as the missing-target gate.
  db_check_duplicate_source="$out_dir/db-secret-check-duplicate"
  mkdir -p "$db_check_duplicate_source"
  chmod 0750 "$db_check_duplicate_source"
  for name in "${db_secret_names[@]}"; do
    install -o root -g root -m 0600 -- \
      "$db_apply_source/$name" "$db_check_duplicate_source/$name"
  done
  install -o root -g root -m 0600 -- \
    "$db_apply_source/db_app_password" "$db_check_duplicate_source/db_admin_password"
  if LAGRANGE_SECRET_SOURCE_DIR="$db_check_duplicate_source" \
     bash "$ops/provision-db-secrets.sh" --check >"$out_dir/db-secret-check-duplicate.out" 2>&1; then
    echo 'self-test: duplicate DB secret set unexpectedly passed --check' >&2
    exit 1
  fi
  grep -Fq 'DB_SECRET_CHECK: FAIL db_app_password,db_admin_password: values are not distinct' \
    "$out_dir/db-secret-check-duplicate.out"

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

# Per-date progress is committed as each body-free worker event arrives. A
# later failure must preserve earlier PUBLISHED records for an idempotent rerun.
progress_state="$out_dir/progress.tsv"
progress_identity=$(printf 'a%.0s' {1..64})
printf 'LAGRANGE_BACKFILL_STATE_V3\t%s\n' "$progress_identity" >"$progress_state"
printf '%s\tRUNNING\t%s\n' 2026-01-01 "$progress_identity" >>"$progress_state"
printf '%s\tRUNNING\t%s\n' 2026-01-02 "$progress_identity" >>"$progress_state"
if printf '%s\n' \
  '{"status":"event","event":"published","phase":"canonical_publication","batch_id":"00000000-0000-4000-8000-000000000001","target_date":"2026-01-01"}' | \
  python3 "$ops/lib/backfill-progress.py" "$progress_state" "$progress_identity" \
    2026-01-01 2026-01-02 >"$out_dir/progress.out" 2>&1; then
  echo 'self-test: incomplete backfill progress stream unexpectedly passed' >&2
  exit 1
fi
grep -Fq $'2026-01-01\tPUBLISHED\t'"$progress_identity" "$progress_state"
if grep -Fq $'2026-01-02\tPUBLISHED\t'"$progress_identity" "$progress_state"; then
  echo 'self-test: incomplete date was marked PUBLISHED' >&2
  exit 1
fi
printf '%s\n' \
  '{"status":"event","event":"published","phase":"canonical_publication","batch_id":"00000000-0000-4000-8000-000000000001","target_date":"2026-01-01"}' \
  '{"status":"event","event":"published","phase":"canonical_publication","batch_id":"00000000-0000-4000-8000-000000000002","target_date":"2026-01-02"}' | \
  python3 "$ops/lib/backfill-progress.py" "$progress_state" "$progress_identity" \
    2026-01-01 2026-01-02 >"$out_dir/progress-rerun.out"
grep -Fq $'2026-01-02\tPUBLISHED\t'"$progress_identity" "$progress_state"

# The remaining validator fixtures intentionally use production-shaped secret
# ownership/modes and therefore require root. Keep the non-root self-test
# useful by asserting the explicit guard, then leave those protected fixtures
# to a root invocation instead of accepting an insecure test bypass.
if [ "$(id -u)" -ne 0 ]; then
  if bash "$ops/validate-production-config.sh" --scope infrastructure \
     >"$out_dir/config-root.out" 2>&1; then
    echo 'self-test: non-root production validation unexpectedly passed' >&2
    exit 1
  fi
  grep -Fq 'validation must run as root to inspect protected production paths' \
    "$out_dir/config-root.out" || {
    cat "$out_dir/config-root.out" >&2
    exit 1
  }
  echo 'OPS_SELF_TEST: validator fixture checks skipped for non-root caller (production validation is root-only)'
  echo 'OPS_SELF_TEST: PASS'
  exit 0
fi

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

if LAGRANGE_ENV_FILE="$out_dir/.env" \
   LAGRANGE_CODE_COMMIT=0000000000000000000000000000000000000000 \
   bash "$ops/validate-production-config.sh" --scope serving-prereqs >"$out_dir/serving-prereqs-config.out" 2>&1; then
  echo 'self-test: serving-prereqs scope unexpectedly passed incomplete fixtures' >&2
  exit 1
else
  grep -Eq '^(INVALID_CONFIG|BLOCKED_EXTERNAL):' "$out_dir/serving-prereqs-config.out"
  if grep -Eq 'kis_app_key|kis_app_secret|RESEARCH_|RECOMMENDATION_DATASET_|RECOMMENDATION_CURATED' \
     "$out_dir/serving-prereqs-config.out"; then
    echo 'self-test: serving-prereqs scope requested deferred KIS/research/dataset values' >&2
    cat "$out_dir/serving-prereqs-config.out" >&2
    exit 1
  fi
fi

# The validator must reject malformed crypto source shapes and pairwise
# duplicates without printing either fixture value. Runtime copies are omitted
# deliberately; the source-level INVALID_CONFIG is the assertion under test.
crypto_source="$out_dir/crypto-source"
crypto_env="$out_dir/crypto.env"
mkdir -p "$crypto_source/tls"
for name in postgres_password db_migration_owner_password db_app_password \
  db_worker_password db_audit_password db_research_password db_admin_password; do
  printf 'db-fixture-%s' "$name" >"$crypto_source/$name"
  chmod 0600 "$crypto_source/$name"
done
for name in session_secret csrf_secret cursor_secret backup_encryption_key; do
  case "$name" in
    session_secret) value=1111111111111111111111111111111111111111111111111111111111111111 ;;
    csrf_secret) value=2222222222222222222222222222222222222222222222222222222222222222 ;;
    cursor_secret) value=3333333333333333333333333333333333333333333333333333333333333333 ;;
    backup_encryption_key) value=4444444444444444444444444444444444444444444444444444444444444444 ;;
  esac
  printf '%s' "$value" >"$crypto_source/$name"
  chmod 0600 "$crypto_source/$name"
done
printf '%s' fixture-auth0-client-secret >"$crypto_source/auth0_client_secret"
printf '%s\n' fixture-certificate >"$crypto_source/tls/lagrange.crt"
printf '%s\n' fixture-private-key >"$crypto_source/tls/lagrange.key"
chmod 0600 "$crypto_source/auth0_client_secret" "$crypto_source/tls/lagrange.crt" "$crypto_source/tls/lagrange.key"
cp "$out_dir/.env" "$crypto_env"
chmod 0600 "$crypto_env"
sed -i -e "s|^LAGRANGE_SECRET_SOURCE_DIR=.*|LAGRANGE_SECRET_SOURCE_DIR=$crypto_source|" -e 's|^AUTH0_DOMAIN=.*|AUTH0_DOMAIN=tenant.auth0.com|' -e 's|^AUTH0_CLIENT_ID=.*|AUTH0_CLIENT_ID=client-id-fixture|' -e 's|^AUTH0_REDIRECT_URI=.*|AUTH0_REDIRECT_URI=https://l1nnx-sh.taild74a33.ts.net/auth/callback|' "$crypto_env"
printf '%s' 333333333333333333333333333333333333333333333333333333333333333 >"$crypto_source/cursor_secret"
if LAGRANGE_ENV_FILE="$crypto_env" LAGRANGE_CODE_COMMIT=0000000000000000000000000000000000000000 bash "$ops/validate-production-config.sh" --scope serving-prereqs >"$out_dir/crypto-shape.out" 2>&1; then
  echo 'self-test: malformed crypto source unexpectedly passed validator' >&2
  exit 1
fi
grep -Fq 'crypto secret cursor_secret must contain exactly 64 lowercase hex characters' "$out_dir/crypto-shape.out"
if grep -Eq '3333333333|fixture-auth0-client-secret' "$out_dir/crypto-shape.out"; then
  echo 'self-test: malformed crypto fixture leaked in validator output' >&2
  exit 1
fi
printf '%s' 1111111111111111111111111111111111111111111111111111111111111111 >"$crypto_source/cursor_secret"
if LAGRANGE_ENV_FILE="$crypto_env" LAGRANGE_CODE_COMMIT=0000000000000000000000000000000000000000 bash "$ops/validate-production-config.sh" --scope serving-prereqs >"$out_dir/crypto-duplicate.out" 2>&1; then
  echo 'self-test: duplicate crypto sources unexpectedly passed validator' >&2
  exit 1
fi
grep -Fq 'crypto source secrets must be distinct: session_secret conflicts with cursor_secret' "$out_dir/crypto-duplicate.out"
if grep -Eq '1111111111|fixture-auth0-client-secret' "$out_dir/crypto-duplicate.out"; then
  echo 'self-test: duplicate crypto fixture leaked in validator output' >&2
  exit 1
fi

# The serving-prereqs provisioner must validate its complete source inventory
# before creating or replacing any runtime copy. Use shape-valid fixtures with
# one deliberately missing source and assert that the runtime tree is untouched.
serving_source="$out_dir/serving-source"
serving_runtime="$out_dir/serving-runtime"
mkdir -p "$serving_source/tls"
for name in postgres_password db_migration_owner_password db_app_password \
  db_worker_password db_audit_password db_research_password db_admin_password \
  cursor_secret session_secret csrf_secret; do
  printf 'fixture-%s' "$name" >"$serving_source/$name"
  chmod 0600 "$serving_source/$name"
done
printf '%s\n' 'fixture-certificate' >"$serving_source/tls/lagrange.crt"
printf '%s\n' 'fixture-private-key' >"$serving_source/tls/lagrange.key"
chmod 0600 "$serving_source/tls/lagrange.crt" "$serving_source/tls/lagrange.key"
if LAGRANGE_SECRET_SOURCE_DIR="$serving_source" \
   LAGRANGE_RUNTIME_SECRET_DIR="$serving_runtime" \
   bash "$root/deploy/secrets/provision-runtime-secrets.sh" \
   --scope serving-prereqs >"$out_dir/serving-prereqs-provision.out" 2>&1; then
  echo 'self-test: serving-prereqs provision unexpectedly accepted a missing source' >&2
  exit 1
fi
grep -Fq 'auth0_client_secret' "$out_dir/serving-prereqs-provision.out"
[ ! -e "$serving_runtime" ] || {
  echo 'self-test: serving-prereqs provision left partial runtime writes' >&2
  find "$serving_runtime" -print >&2
  exit 1
}

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
grep -Fq 'one bounded worker/provider reuses one in-memory token' <<<"$plan"
grep -Fq 'no bearer token is persisted' <<<"$plan"
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
