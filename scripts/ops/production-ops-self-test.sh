#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ops=$root/scripts/ops
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-production-ops.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

bash "$ops/production-ops-static-check.sh" >/dev/null

plan=$(bash "$ops/deploy-production-release.sh" --dry-run \
  --commit 1111111111111111111111111111111111111111)
grep -Fq 'DRY_RUN: no Git archive' <<<"$plan"
backup_plan=$(bash "$ops/run-production-backup.sh" --plan)
grep -Fq 'PLAN_ONLY: no protected config/key read' <<<"$backup_plan"
install_plan=$(bash "$ops/install-production-backup.sh" --dry-run)
grep -Fq 'never starts them' <<<"$install_plan"

# Namespace root lets the exact owner/mode and root-only contracts run without
# changing the host. The fixture is a clean Git repository with its protected
# env deliberately outside Git.
release_fixture=$tmp/release
mkdir -p "$release_fixture/repo/scripts/ops" "$release_fixture/repo/deploy/compose" \
  "$release_fixture/repo/nt" "$release_fixture/repo/configs" \
  "$release_fixture/repo/migrations" "$release_fixture/install"
chmod 0700 "$release_fixture"
chmod 0755 "$release_fixture/install"
cp "$ops/deploy-production-release.sh" "$release_fixture/repo/scripts/ops/"
printf '%s\n' 'services: {}' >"$release_fixture/repo/deploy/compose/compose.yml"
printf '%s\n' fixture >"$release_fixture/repo/nt/fixture"
printf '%s\n' fixture >"$release_fixture/repo/configs/fixture"
printf '%s\n' fixture >"$release_fixture/repo/migrations/fixture"
git -C "$release_fixture/repo" init -q
git -C "$release_fixture/repo" config user.name test
git -C "$release_fixture/repo" config user.email test@example.invalid
git -C "$release_fixture/repo" add .
git -C "$release_fixture/repo" commit -qm first
commit_one=$(git -C "$release_fixture/repo" rev-parse HEAD)
mkdir -p "$release_fixture/repo/docs"
printf '%s' official-workbook-fixture >"$release_fixture/repo/docs/kis_openapi_entiredocs_20260818_030007.xlsx"
printf '%s\n' 'APP_ENV=production' >"$release_fixture/production.env"
chmod 0600 "$release_fixture/production.env"
unshare -Ur bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
  --apply --commit "$commit_one" --env-source "$release_fixture/production.env" \
  --install-root "$release_fixture/install"
[ "$(readlink "$release_fixture/install/current")" = "releases/$commit_one" ]
[ "$(stat -c %a "$release_fixture/install/releases/$commit_one/deploy/compose/.env")" = 600 ]
grep -Fxq 'APP_ENV=production' "$release_fixture/install/releases/$commit_one/deploy/compose/.env"
[ ! -e "$release_fixture/install/releases/$commit_one/docs/kis_openapi_entiredocs_20260818_030007.xlsx" ]

printf '%s\n' second >"$release_fixture/repo/second"
git -C "$release_fixture/repo" add second
git -C "$release_fixture/repo" commit -qm second
commit_two=$(git -C "$release_fixture/repo" rev-parse HEAD)
unshare -Ur bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
  --apply --commit "$commit_two" --env-source "$release_fixture/production.env" \
  --install-root "$release_fixture/install"
[ "$(readlink "$release_fixture/install/current")" = "releases/$commit_two" ]
unshare -Ur bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
  --rollback --commit "$commit_one" --env-source "$release_fixture/production.env" \
  --install-root "$release_fixture/install"
[ "$(readlink "$release_fixture/install/current")" = "releases/$commit_one" ]
touch "$release_fixture/repo/untracked-secret"
if unshare -Ur bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
  --apply --commit "$commit_two" --env-source "$release_fixture/production.env" \
  --install-root "$release_fixture/install" >"$tmp/dirty.out" 2>&1; then
  echo 'production-ops-self-test: dirty release source unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'repository must have no tracked changes or unapproved untracked files' "$tmp/dirty.out"
rm "$release_fixture/repo/untracked-secret"
printf '%s\n' tracked-dirty >>"$release_fixture/repo/second"
if unshare -Ur bash "$release_fixture/repo/scripts/ops/deploy-production-release.sh" \
  --apply --commit "$commit_two" --env-source "$release_fixture/production.env" \
  --install-root "$release_fixture/install" >"$tmp/tracked-dirty.out" 2>&1; then
  echo 'production-ops-self-test: tracked-dirty release source unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'repository must have no tracked changes or unapproved untracked files' "$tmp/tracked-dirty.out"

# Exercise encrypted set creation, isolated-restore invocation, and bounded
# prune with a fake Docker binary. No real daemon or database is contacted.
backup_fixture=$tmp/backup
mkdir -p "$backup_fixture"/{bin,data/raw,data/curated,backups,state,locks,compose}
chmod 0700 "$backup_fixture" "$backup_fixture/backups" "$backup_fixture/state" \
  "$backup_fixture/locks" "$backup_fixture/compose"
printf '%s' raw-fixture-secret >"$backup_fixture/data/raw/row"
printf '%s' curated-fixture-secret >"$backup_fixture/data/curated/row"
printf '%s\n' 'services: {}' >"$backup_fixture/compose/compose.yml"
printf '%s\n' 'APP_ENV=production' >"$backup_fixture/compose/.env"
printf '4%.0s' {1..64} >"$backup_fixture/key"
chmod 0600 "$backup_fixture/compose/.env" "$backup_fixture/key"
cat >"$backup_fixture/bin/docker" <<'SH'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >>"$FAKE_DOCKER_LOG"
if [ "${1:-}" = compose ]; then
  case " $* " in
    *' version '*) exit 0 ;;
    *' exec '*) printf '%s' FAKE_CUSTOM_DATABASE_DUMP; exit 0 ;;
  esac
fi
if [ "${1:-}" = run ]; then exit 0; fi
exit 1
SH
chmod 0755 "$backup_fixture/bin/docker"
write_backup_config() {
  local cap=$1
  cat >"$backup_fixture/backup.conf" <<EOF
BACKUP_ROOT=$backup_fixture/backups
DATA_ROOT=$backup_fixture/data
COMPOSE_FILE=$backup_fixture/compose/compose.yml
COMPOSE_ENV_FILE=$backup_fixture/compose/.env
COMPOSE_PROJECT=lagrange-test
LAGRANGE_CODE_COMMIT=2222222222222222222222222222222222222222
KEY_FILE=$backup_fixture/key
LOCK_FILE=$backup_fixture/locks/backup.lock
METRICS_FILE=$backup_fixture/state/backup.prom
MAX_TOTAL_BYTES=$cap
MIN_FREE_BYTES=1
RETENTION_DAYS=365
MIN_KEEP=1
POSTGRES_SERVICE=postgres
POSTGRES_IMAGE=postgres:18.4
EOF
  chmod 0600 "$backup_fixture/backup.conf"
}
write_backup_config 999999999
export FAKE_DOCKER_LOG=$backup_fixture/docker.log

printf '%s' 'fixture-backup-secret-must-never-leak' >"$backup_fixture/key"
if unshare -Ur bash "$ops/run-production-backup.sh" --check \
  --config-file "$backup_fixture/backup.conf" >"$backup_fixture/bad-key.out" 2>&1; then
  echo 'production-ops-self-test: malformed backup key unexpectedly passed' >&2
  exit 1
fi
if grep -Fq 'fixture-backup-secret-must-never-leak' "$backup_fixture/bad-key.out"; then
  echo 'production-ops-self-test: malformed key value leaked' >&2
  exit 1
fi
printf '4%.0s' {1..64} >"$backup_fixture/key"

# Installer apply is also exercised inside namespace root. Its fake systemctl
# proves apply enables without --now/start and performs no backup/Docker call.
cat >"$backup_fixture/bin/systemctl" <<'SH'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >>"$FAKE_SYSTEMCTL_LOG"
SH
chmod 0755 "$backup_fixture/bin/systemctl"
mkdir -p "$backup_fixture/install-bin" "$backup_fixture/systemd" "$backup_fixture/etc"
chmod 0755 "$backup_fixture/install-bin" "$backup_fixture/systemd" "$backup_fixture/etc"
export FAKE_SYSTEMCTL_LOG=$backup_fixture/systemctl.log
unshare -Ur env PATH="$backup_fixture/bin:$PATH" FAKE_SYSTEMCTL_LOG="$FAKE_SYSTEMCTL_LOG" \
  bash "$ops/install-production-backup.sh" --apply \
  --config-source "$backup_fixture/backup.conf" \
  --install-bin "$backup_fixture/install-bin" --systemd-dir "$backup_fixture/systemd" \
  --config-target "$backup_fixture/etc/backup.conf" >"$backup_fixture/install.out"
grep -Fxq 'daemon-reload' "$FAKE_SYSTEMCTL_LOG"
grep -Fq 'enable lagrange-production-backup.timer lagrange-production-backup-verify.timer' \
  "$FAKE_SYSTEMCTL_LOG"
if grep -Eq -- '--now|(^| )start( |$)' "$FAKE_SYSTEMCTL_LOG"; then
  echo 'production-ops-self-test: installer unexpectedly started a unit' >&2
  exit 1
fi
[ ! -s "$FAKE_DOCKER_LOG" ]
if unshare -Ur env PATH="$backup_fixture/bin:$PATH" FAKE_SYSTEMCTL_LOG="$FAKE_SYSTEMCTL_LOG" \
  bash "$ops/install-production-backup.sh" --apply \
  --config-source "$backup_fixture/backup.conf" \
  --install-bin "$backup_fixture/install-bin" --systemd-dir "$backup_fixture/systemd" \
  --config-target "$backup_fixture/etc/backup.conf" >"$backup_fixture/install-existing.out" 2>&1; then
  echo 'production-ops-self-test: installer overwrote existing targets' >&2
  exit 1
fi
grep -Fq 'refusing to overwrite existing target' "$backup_fixture/install-existing.out"

unshare -Ur env PATH="$backup_fixture/bin:$PATH" FAKE_DOCKER_LOG="$FAKE_DOCKER_LOG" \
  bash "$ops/run-production-backup.sh" --run --config-file "$backup_fixture/backup.conf" \
  >"$backup_fixture/run-one.out"
first_set=$(find "$backup_fixture/backups" -maxdepth 1 -type d -name 'backup-*' -print -quit)
[ -n "$first_set" ]
[ -f "$first_set/COMPLETE" ] && [ -f "$first_set/VERIFIED" ]
if grep -aEq 'raw-fixture-secret|curated-fixture-secret|4444444444' "$first_set"/*.enc "$backup_fixture/run-one.out"; then
  echo 'production-ops-self-test: backup output leaked protected fixture content' >&2
  exit 1
fi
first_size=$(du -sb "$first_set" | awk '{print $1}')
write_backup_config $((first_size + 2048))
sleep 1
unshare -Ur env PATH="$backup_fixture/bin:$PATH" FAKE_DOCKER_LOG="$FAKE_DOCKER_LOG" \
  bash "$ops/run-production-backup.sh" --run --config-file "$backup_fixture/backup.conf" \
  >"$backup_fixture/run-two.out"
[ "$(find "$backup_fixture/backups" -maxdepth 1 -type d -name 'backup-*' | wc -l)" -eq 1 ]
grep -Fq 'PRODUCTION_BACKUP_PRUNED' "$backup_fixture/run-two.out"
grep -Fq -- '--network none --read-only --user 999:999' "$FAKE_DOCKER_LOG"
unshare -Ur env PATH="$backup_fixture/bin:$PATH" FAKE_DOCKER_LOG="$FAKE_DOCKER_LOG" \
  bash "$ops/run-production-backup.sh" --verify-latest \
  --config-file "$backup_fixture/backup.conf" >"$backup_fixture/verify.out"
grep -Fq 'isolated=true' "$backup_fixture/verify.out"

echo 'PRODUCTION_OPS_SELF_TEST: PASS'
