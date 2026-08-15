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
  research-schema-check research-worker recommendation-runner \
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

grep -Fq '[ ! -L "$input" ]' "$provision" \
  || die 'provisioner must reject symlinked source secrets'
grep -Fq "\\r'" "$provision" \
  || die 'provisioner must reject CR-containing credential secrets'

echo 'SECRETS_RUNTIME_STATIC: PASS'
