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
grep -Fq -- '--scope infrastructure|backfill|release' "$provision" \
  || die 'provisioner scope contract is absent'
grep -Fq 'scope=$scope' "$provision" \
  || die 'provisioner must report the selected scope'
grep -Fq 'if [ "$scope" = release ]; then' "$provision" \
  || die 'provisioner must fence serving-only copies to release scope'
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
  nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler; do
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
  grep -Eq "^copy_secret[[:space:]]+$service[[:space:]].*[[:space:]]999[[:space:]]+999[[:space:]]+0400[[:space:]]+yes$" "$provision" \
    || die "provisioner must install $service secrets as 999:999 mode 0400"
  expected_count=1
  [ "$service" = db-role-bootstrap ] && expected_count=7
  provision_count=$(grep -Ec "^copy_secret[[:space:]]+$service[[:space:]]" "$provision" || true)
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

# Infrastructure scope must be able to complete the DB/raw/schema gates before
# KIS credentials or serving approval exist. Keep its exact runtime inventory
# aligned with the validator: seven bootstrap copies, one migration copy, and
# one PostgreSQL plus one schema-check copy. No research-worker copy belongs to
# this scope.
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
  'research-schema-check postgres_password postgres_password 999 999 0440 yes'; do
  read -r service target source uid gid mode single_line <<<"$expected"
  awk -v s="$service" -v t="$target" -v src="$source" -v u="$uid" \
    -v g="$gid" -v m="$mode" -v one="$single_line" \
    '$1 == "copy_secret" && $2 == s && $3 == t && $4 == src &&
     $5 == u && $6 == g && $7 == m && $8 == one { found=1 }
     END { exit !found }' "$provision" \
    || die "infrastructure provisioner inventory missing: $expected"
done

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
grep -Fq "\\r'" "$provision" \
  || die 'provisioner must reject CR-containing credential secrets'

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

echo 'SECRETS_RUNTIME_STATIC: PASS'
