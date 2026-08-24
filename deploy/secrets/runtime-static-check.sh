#!/usr/bin/env bash
# Static contract check for service-specific native-Linux secret delivery.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
compose="$root/deploy/compose/compose.yml"
provision="$root/deploy/secrets/provision-runtime-secrets.sh"
secret_gitignore="$root/deploy/secrets/.gitignore"
env_example="$root/deploy/compose/.env.example"
secrets_readme="$root/deploy/secrets/README.md"
db_dockerfile="$root/deploy/db/Dockerfile"
paper_wrapper="$root/deploy/runtime/paper-runner-entrypoint"

die() {
  echo "secrets-runtime-static-check: $*" >&2
  exit 1
}

[ -x "$provision" ] || die "provisioner must be executable"
[ "$(stat -c '%a' "$provision")" = 755 ] || die "provisioner must have mode 0755"
[ ! -L "$provision" ] || die "provisioner must not be a symlink"
grep -Fxq '!provision-runtime-secrets.sh' "$secret_gitignore" \
  || die 'provisioner must be visible to Git while real secrets stay ignored'
if git check-ignore -q "$provision"; then
  die 'provisioner is unexpectedly ignored by Git'
fi
bash -n "$provision" || die "provisioner has shell syntax errors"
grep -Fq -- '--scope infrastructure|serving-prereqs|backfill|range-raw|range-raw-recovery|release' "$provision" \
  || die 'provisioner scope contract is absent'
grep -Fq 'scope=$scope' "$provision" \
  || die 'provisioner must report the selected scope'
grep -Fq 'serving-prereqs)' "$provision" \
  || die 'provisioner serving-prereqs scope branch is missing'
grep -Fq 'preflight_source' "$provision" \
  || die 'provisioner source preflight is missing'
grep -Fq 'for spec in "${copy_specs[@]}"' "$provision" \
  || die 'provisioner must preflight and copy the selected inventory'
grep -Fq 'before creating the runtime' "$provision" \
  || die 'provisioner must preflight before writes'
[ -f "$db_dockerfile" ] || die 'missing database one-shot Dockerfile'
[ -f "$paper_wrapper" ] || die 'missing Paper runtime wrapper'
bash -n "$paper_wrapper" || die 'Paper runtime wrapper has shell syntax errors'
grep -Fq 'validate_health_state' "$paper_wrapper" \
  || die 'Paper runtime wrapper must validate loop progress in healthcheck'
grep -Fq 'cycle_deadline_at' "$paper_wrapper" \
  || die 'Paper runtime wrapper must honor bounded cycle health state'
grep -Fxq 'USER 999:999' "$db_dockerfile" \
  || die 'database one-shot image must remain non-root UID/GID 999:999'
grep -Fq 'LAGRANGE_RUNTIME_SECRET_DIR' "$compose" \
  || die 'Compose must use the runtime secret directory'
if awk '/^[[:space:]]+file:/ && $0 !~ /LAGRANGE_RUNTIME_SECRET_DIR/ {bad=1} END {exit bad}' "$compose"; then
  :
else
  die 'Compose must not mount operator source files directly'
fi

for uid in 101 999 10001; do
  grep -Fq "uid: \"$uid\"" "$compose" \
    || die "Compose is missing an explicit UID contract for $uid"
  grep -Fq "gid: \"$uid\"" "$compose" \
    || die "Compose is missing an explicit GID contract for $uid"
done
grep -Fq 'mode: 0440' "$compose" || die 'Compose is missing 0440 secret mounts'
grep -Fq 'mode: 0400' "$compose" || die 'database one-shots must use 0400 mounts'
grep -Fq 'provision-runtime-secrets.sh' "$compose" \
  || die 'Compose documentation must reference the provisioner'
grep -Fq 'deploy/secrets/provision-runtime-secrets.sh' "$env_example" \
  || die 'Compose env documentation must reference the provisioner'
grep -Fq 'deploy/secrets/provision-runtime-secrets.sh' "$secrets_readme" \
  || die 'secret documentation must reference the provisioner'
for service in reverse-proxy api-server db-role-bootstrap db-migrate postgres \
  research-schema-check research-worker recommendation-runner candidate-runner \
  owner-beta-runner research-range-raw nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler; do
  grep -Fq "/$service/" "$compose" \
    || die "Compose is missing service-specific runtime path for $service"
done

service_block() {
  local service=$1
  awk -v service="$service" '
    $0 == "  " service ":" { in_service=1; print; next }
    in_service && $0 ~ /^  [^[:space:]][^:]*:/ { exit }
    in_service { print }
  ' "$compose"
}

# Docker file-backed secrets are source files on native Linux, so verify both
# sides of the ownership contract. The image, provisioner, and long syntax
# mounts must agree on the non-root UID/GID and the stricter one-shot mode.
for service in db-role-bootstrap db-migrate; do
  grep -Eq "^[[:space:]]+add_copy[[:space:]]+$service[[:space:]].*[[:space:]]999[[:space:]]+999[[:space:]]+0400[[:space:]]+yes$" "$provision" \
    || die "provisioner must install $service secrets as 999:999 mode 0400"
  expected_count=1
  [ "$service" = db-role-bootstrap ] && expected_count=7
  provision_count=$(grep -Ec "^[[:space:]]+add_copy[[:space:]]+$service[[:space:]]" "$provision" || true)
  [ "$provision_count" -eq "$expected_count" ] \
    || die "$service provisioner entry count changed (expected $expected_count)"
  block=$(service_block "$service")
  [ -n "$block" ] || die "missing Compose service block: $service"
  if grep -Eq 'uid: "0"|gid: "0"' <<<"$block"; then
    die "$service must not mount root-owned secrets"
  fi
  uid_count=$(grep -Ec '^[[:space:]]+uid: "999"$' <<<"$block" || true)
  gid_count=$(grep -Ec '^[[:space:]]+gid: "999"$' <<<"$block" || true)
  mode_count=$(grep -Ec '^[[:space:]]+mode: 0400$' <<<"$block" || true)
  [ "$uid_count" -eq "$expected_count" ] \
    || die "$service must mount $expected_count secrets with uid 999"
  [ "$gid_count" -eq "$expected_count" ] \
    || die "$service must mount $expected_count secrets with gid 999"
  [ "$mode_count" -eq "$expected_count" ] \
    || die "$service must mount $expected_count secrets with mode 0400"
done

# Every selected runtime inventory entry is declared once in the provisioner
# copy-spec functions and must remain aligned with validator runtime_specs. The
# serving-prereqs list intentionally includes all non-KIS serving copies but no
# KIS source; backup_encryption_key is source-only and is preflighted without a
# runtime destination.
for expected in \
  'db-role-bootstrap postgres_password postgres_password 999 999 0400 yes' \
  'db-role-bootstrap db_migration_owner_password db_migration_owner_password 999 999 0400 yes' \
  'db-role-bootstrap db_app_password db_app_password 999 999 0400 yes' \
  'db-role-bootstrap db_worker_password db_worker_password 999 999 0400 yes' \
  'db-role-bootstrap db_audit_password db_audit_password 999 999 0400 yes' \
  'db-role-bootstrap db_research_password db_research_password 999 999 0400 yes' \
  'db-role-bootstrap db_admin_password db_admin_password 999 999 0400 yes' \
  'db-migrate db_migration_owner_password db_migration_owner_password 999 999 0400 yes' \
  'postgres postgres_password postgres_password 999 999 0440 yes' \
  'research-schema-check postgres_password postgres_password 999 999 0440 yes' \
  'reverse-proxy lagrange_tls_cert tls/lagrange.crt 101 101 0440 no' \
  'reverse-proxy lagrange_tls_key tls/lagrange.key 101 101 0440 no' \
  'api-server db_app_password db_app_password 10001 10001 0440 yes' \
  'api-server db_admin_password db_admin_password 10001 10001 0440 yes' \
  'api-server db_audit_password db_audit_password 10001 10001 0440 yes' \
  'api-server cursor_secret cursor_secret 10001 10001 0440 yes' \
  'api-server session_secret session_secret 10001 10001 0440 yes' \
  'api-server csrf_secret csrf_secret 10001 10001 0440 yes' \
  'api-server auth0_client_secret auth0_client_secret 10001 10001 0440 yes' \
  'research-worker db_research_password db_research_password 10001 10001 0440 yes' \
  'recommendation-runner db_worker_password db_worker_password 10001 10001 0440 yes' \
  'candidate-runner db_worker_password db_worker_password 10001 10001 0440 yes' \
  'owner-beta-runner db_worker_password db_worker_password 10001 10001 0440 yes' \
  'nt-backtest-worker-1 db_worker_password db_worker_password 10001 10001 0440 yes' \
  'nt-backtest-worker-2 db_worker_password db_worker_password 10001 10001 0440 yes' \
  'paper-scheduler db_app_password db_app_password 10001 10001 0440 yes' \
  'paper-scheduler db_worker_password db_worker_password 10001 10001 0440 yes' \
  'paper-scheduler db_admin_password db_admin_password 10001 10001 0440 yes' \
  'paper-scheduler db_audit_password db_audit_password 10001 10001 0440 yes' \
  'research-worker kis_app_key kis_app_key 10001 10001 0440 yes' \
  'research-worker kis_app_secret kis_app_secret 10001 10001 0440 yes' \
  'research-range-raw kis_app_key kis_app_key 10001 10001 0440 yes' \
  'research-range-raw kis_app_secret kis_app_secret 10001 10001 0440 yes'; do
  read -r service target source uid gid mode single_line <<<"$expected"
  grep -Eq "^[[:space:]]+add_copy[[:space:]]+$service[[:space:]]+$target[[:space:]]+$source[[:space:]]+$uid[[:space:]]+$gid[[:space:]]+$mode[[:space:]]+$single_line$" "$provision" \
    || die "infrastructure provisioner inventory missing: $expected"
done
grep -Fq 'add_extra_source backup_encryption_key yes' "$provision" \
  || die 'serving-prereqs/release backup source preflight is missing'
serving_branch=$(awk '
  /^  serving-prereqs\)/ { in_scope=1; next }
  in_scope && /^  backfill\)/ { exit }
  in_scope { print }
' "$provision")
if grep -Eq 'kis_app_key|kis_app_secret' <<<"$serving_branch"; then
  die 'serving-prereqs must not add KIS runtime copies'
fi
recovery_branch=$(awk '
  /^  range-raw-recovery\)/ { in_scope=1; next }
  in_scope && /^  release\)/ { exit }
  in_scope { print }
' "$provision")
grep -Fq 'range-raw-recovery)' "$provision" \
  || die 'range-raw-recovery provisioner scope is missing'
if grep -Eq 'add_copy|kis_app_key|kis_app_secret' <<<"$recovery_branch"; then
  die 'range-raw-recovery must not install runtime or KIS secret copies'
fi

grep -Fq 'reject_dotdot()' "$provision" \
  || die 'provisioner must reject .. path aliases'
grep -Fq 'check_path "$source_dir" source-directory' "$provision" \
  || die 'provisioner must fence source directory ancestors'
grep -Fq 'check_path "$runtime_dir" runtime-directory' "$provision" \
  || die 'provisioner must fence runtime directory ancestors'
grep -Fq 'check_path "$input"' "$provision" \
  || die 'provisioner must fence source-file ancestors'
grep -Fq 'check_path "$output_dir"' "$provision" \
  || die 'provisioner must fence runtime service-directory ancestors'
grep -Fq 'check_path "$output"' "$provision" \
  || die 'provisioner must fence runtime secret-file ancestors'
grep -Fq 'must not traverse a symlink' "$provision" \
  || die 'provisioner must reject symlinked ancestors'
grep -Fq "stat -c '%a'" "$provision" \
  || die 'provisioner must validate source file modes before writes'
grep -Fq 'crypto_placeholder_pattern' "$provision" \
  || die 'provisioner crypto placeholder contract is missing'
grep -Fq "grep -Eq '^[0-9a-f]{64}$'" "$provision" \
  || die 'provisioner crypto lowercase-hex contract is missing'
grep -Fq 'crypto source secrets must be distinct' "$provision" \
  || die 'provisioner crypto distinctness contract is missing'
grep -Fq "\\r'" "$provision" \
  || die 'provisioner must reject CR-containing credential secrets'
grep -Fq 'numeric_chown()' "$provision" \
  || die 'provisioner must use an explicit numeric ownership helper'
grep -Fq 'chown --no-dereference -- "$uid:$gid"' "$provision" \
  || die 'provisioner must assign Docker-only UIDs with numeric chown'
if grep -Eq '^[[:space:]]*install([[:space:]]|$)' "$provision"; then
  die 'provisioner must not use install -o/-g for Docker-only numeric UIDs'
fi
grep -Fq 'mktemp -- "$output_dir/.lagrange-secret.' "$provision" \
  || die 'provisioner must stage runtime files before replacement'
grep -Fq 'mv -T -- "$staged" "$output"' "$provision" \
  || die 'provisioner must atomically rename staged runtime files'

# Exercise the path fence without requiring real root or any secret copy. A
# fake id(1) satisfies the early root guard; every fixture exits before mkdir/
# install, so this remains a no-root static test of symlink and `..` rejection.
fixture=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-secret-path-test.XXXXXX")
trap 'rm -rf -- "$fixture"' EXIT
fake_bin="$fixture/bin"
mkdir -p "$fake_bin"
printf '%s\n' '#!/usr/bin/env bash' 'if [ "$1" = -u ]; then echo 0; else exec /usr/bin/id "$@"; fi' >"$fake_bin/id"
chmod 0755 "$fake_bin/id"
mkdir -p "$fixture/source-real" "$fixture/runtime-real" "$fixture/source" "$fixture/runtime"
ln -s "$fixture/source-real" "$fixture/source-link"
ln -s "$fixture/runtime-real" "$fixture/runtime-link"

expect_path_rejection() {
  local label=$1 output=$2
  shift 2
  if PATH="$fake_bin:$PATH" "$@" >"$output" 2>&1; then
    die "provisioner accepted unsafe $label path"
  fi
  grep -Eq "must not (traverse a symlink|contain '\.\.')" "$output" \
    || die "provisioner did not report unsafe $label path"
}

expect_path_rejection source-ancestor "$fixture/source-ancestor.out" \
  env LAGRANGE_SECRET_SOURCE_DIR="$fixture/source-link/child" \
      LAGRANGE_RUNTIME_SECRET_DIR="$fixture/runtime" \
      bash "$provision" --scope backfill
expect_path_rejection runtime-ancestor "$fixture/runtime-ancestor.out" \
  env LAGRANGE_SECRET_SOURCE_DIR="$fixture/source" \
      LAGRANGE_RUNTIME_SECRET_DIR="$fixture/runtime-link/child" \
      bash "$provision" --scope backfill
expect_path_rejection dotdot-alias "$fixture/dotdot.out" \
  env LAGRANGE_SECRET_SOURCE_DIR="$fixture/source/../source" \
      LAGRANGE_RUNTIME_SECRET_DIR="$fixture/runtime" \
      bash "$provision" --scope backfill

printf 'fixture-secret' >"$fixture/source/postgres_password.real"
ln -s "$fixture/source/postgres_password.real" "$fixture/source/postgres_password"
expect_path_rejection input-ancestor "$fixture/input.out" \
  env LAGRANGE_SECRET_SOURCE_DIR="$fixture/source" \
      LAGRANGE_RUNTIME_SECRET_DIR="$fixture/runtime" \
      bash "$provision" --scope backfill

rm -f "$fixture/source/postgres_password"
printf 'fixture-secret' >"$fixture/source/postgres_password"
chmod 0600 "$fixture/source/postgres_password"
ln -s "$fixture/runtime-real" "$fixture/runtime/db-role-bootstrap"
expect_path_rejection output-directory "$fixture/output-dir.out" \
  env LAGRANGE_SECRET_SOURCE_DIR="$fixture/source" \
      LAGRANGE_RUNTIME_SECRET_DIR="$fixture/runtime" \
      bash "$provision" --scope backfill

rm -f "$fixture/runtime/db-role-bootstrap"
mkdir -p "$fixture/runtime/db-role-bootstrap"
ln -s "$fixture/runtime-real/postgres_password" \
  "$fixture/runtime/db-role-bootstrap/postgres_password"
expect_path_rejection output-file "$fixture/output-file.out" \
  env LAGRANGE_SECRET_SOURCE_DIR="$fixture/source" \
      LAGRANGE_RUNTIME_SECRET_DIR="$fixture/runtime" \
      bash "$provision" --scope backfill

# A serving-prereqs crypto source with 63 hex characters must fail before any
# runtime directory is created. The fake id(1) keeps this focused test rootless.
crypto_source="$fixture/crypto-source"
crypto_runtime="$fixture/crypto-runtime"
malformed_cursor=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde
mkdir -p "$crypto_source/tls"
for name in postgres_password db_migration_owner_password db_app_password \
  db_worker_password db_audit_password db_research_password db_admin_password \
  session_secret csrf_secret cursor_secret auth0_client_secret backup_encryption_key; do
  value=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
  [ "$name" = cursor_secret ] && value=$malformed_cursor
  printf '%s' "$value" >"$crypto_source/$name"
  chmod 0600 "$crypto_source/$name"
done
printf '%s\n' fixture-certificate >"$crypto_source/tls/lagrange.crt"
printf '%s\n' fixture-private-key >"$crypto_source/tls/lagrange.key"
chmod 0600 "$crypto_source/tls/lagrange.crt" "$crypto_source/tls/lagrange.key"
if PATH="$fake_bin:$PATH" env \
   LAGRANGE_SECRET_SOURCE_DIR="$crypto_source" \
   LAGRANGE_RUNTIME_SECRET_DIR="$crypto_runtime" \
   bash "$provision" --scope serving-prereqs \
   >"$fixture/crypto-shape.out" 2>&1; then
  die 'provisioner accepted malformed cursor crypto source'
fi
grep -Fq 'crypto source cursor_secret must contain exactly 64 lowercase hex characters' \
  "$fixture/crypto-shape.out" \
  || die 'provisioner did not report malformed cursor shape'
[ ! -e "$crypto_runtime" ] || die 'malformed crypto source left partial runtime writes'
if grep -Fq "$malformed_cursor" "$fixture/crypto-shape.out"; then
  die 'provisioner leaked a crypto fixture value'
fi

# Exercise the numeric ownership path with fakeroot.  UID/GID 10001 is
# deliberately a Docker-only identity on production hosts, so no NSS user is
# required for this test.  fakeroot lets the root guard and stat assertions run
# without mutating host ownership; the provisioner still receives numeric
# chown arguments and the resulting metadata is checked for every UID class.
if command -v fakeroot >/dev/null 2>&1; then
  numeric_source="$fixture/numeric-source"
  numeric_runtime="$fixture/numeric-runtime"
  mkdir -p "$numeric_source/tls"
  for name in postgres_password db_migration_owner_password db_app_password \
    db_worker_password db_audit_password db_research_password db_admin_password \
    auth0_client_secret; do
    printf 'fixture-%s' "$name" >"$numeric_source/$name"
    chmod 0600 "$numeric_source/$name"
  done
  printf '%064d' 1 >"$numeric_source/session_secret"
  printf '%064d' 2 >"$numeric_source/csrf_secret"
  printf '%064d' 3 >"$numeric_source/cursor_secret"
  printf '%064d' 4 >"$numeric_source/backup_encryption_key"
  chmod 0600 "$numeric_source/session_secret" "$numeric_source/csrf_secret" \
    "$numeric_source/cursor_secret" "$numeric_source/backup_encryption_key"
  printf '%s\n' fixture-certificate >"$numeric_source/tls/lagrange.crt"
  printf '%s\n' fixture-private-key >"$numeric_source/tls/lagrange.key"
  chmod 0600 "$numeric_source/tls/lagrange.crt" "$numeric_source/tls/lagrange.key"
  if ! PATH="$fake_bin:$PATH" fakeroot bash -c '
    set -e
    source_dir=$2
    runtime_dir=$3
    LAGRANGE_SECRET_SOURCE_DIR="$source_dir" \
      LAGRANGE_RUNTIME_SECRET_DIR="$runtime_dir" \
      bash "$1" --scope serving-prereqs >/dev/null
    LAGRANGE_SECRET_SOURCE_DIR="$source_dir" \
      LAGRANGE_RUNTIME_SECRET_DIR="$runtime_dir" \
      bash "$1" --scope serving-prereqs >/dev/null
    [ "$(stat -c "%u:%g:%a" "$runtime_dir/api-server/session_secret")" = 10001:10001:440 ]
    [ "$(stat -c "%u:%g:%a" "$runtime_dir/reverse-proxy/lagrange_tls_cert")" = 101:101:440 ]
    [ "$(stat -c "%u:%g:%a" "$runtime_dir/db-role-bootstrap/postgres_password")" = 999:999:400 ]
    [ "$(stat -c "%u:%g:%a" "$runtime_dir/api-server")" = 10001:10001:750 ]
  ' _ "$provision" "$numeric_source" "$numeric_runtime" \
    >"$fixture/numeric-ownership.out" 2>&1; then
    cat "$fixture/numeric-ownership.out" >&2
    die 'numeric ownership fixture did not converge to expected metadata'
  fi
else
  echo 'secrets-runtime-static-check: fakeroot unavailable; numeric stat fixture skipped' >&2
fi

echo 'SECRETS_RUNTIME_STATIC: PASS'
