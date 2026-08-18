#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ops=$root/scripts/ops
systemd=$root/deploy/systemd
die() { echo "production-ops-static: $*" >&2; exit 1; }

for script in deploy-production-release.sh run-production-backup.sh install-production-backup.sh; do
  bash -n "$ops/$script"
done

release=$ops/deploy-production-release.sh
backup=$ops/run-production-backup.sh
installer=$ops/install-production-backup.sh
grep -Fq 'status --porcelain=v1 --untracked-files=all' "$release" || die 'release clean-tree fence missing'
grep -Fq "?? docs/kis_openapi_entiredocs_20260818_030007.xlsx" "$release" \
  || die 'release workbook-only untracked allowlist missing'
grep -Fq 'git -c safe.directory="$repo_root"' "$release" || die 'release sudo safe.directory must be command-local'
grep -Fq 'archive --format=tar "$release_commit"' "$release" || die 'release must materialize exact Git object'
grep -Fq 'refusing to overwrite existing release directory' "$release" || die 'release overwrite refusal missing'
grep -Fq 'mv -Tf -- "$temporary" "$current_link"' "$release" || die 'release current switch is not atomic'
grep -Fq 'deploy/compose/.env' "$release" || die 'protected Compose env install missing'
grep -Fq 'mode 0600' "$release" || die 'protected Compose env metadata contract missing'
if grep -Eq 'rsync|cp[[:space:]].*repo_root|git[[:space:]]+checkout' "$release"; then
  die 'release must not copy mutable worktree content'
fi

grep -Fq 'aes-256-cbc -salt -pbkdf2 -iter 200000' "$backup" || die 'backup encryption contract missing'
grep -Fq 'postgres.dump.enc' "$backup" || die 'PostgreSQL encrypted class missing'
grep -Fq 'raw.tar.enc' "$backup" || die 'Raw encrypted class missing'
grep -Fq 'curated.tar.enc' "$backup" || die 'Curated encrypted class missing'
grep -Fq -- '--network none --read-only --user 999:999' "$backup" || die 'isolated restore verification contract missing'
grep -Fq 'pg_restore --no-owner --no-privileges -d verify' "$backup" || die 'actual PostgreSQL restore verification missing'
grep -Fq 'MIN_KEEP wins' "$backup" || die 'bounded retention floor missing'
grep -Fq 'MAX_TOTAL_BYTES cannot be satisfied' "$backup" || die 'disk cap failure missing'
grep -Fq 'rm -rf -- "$oldest_path"' "$backup" || die 'retention must use resolved exact set path'
if grep -Eq -- '--key([=[:space:]]|$)|BACKUP_KEY=' "$backup"; then
  die 'backup secret must not enter argv or environment'
fi

grep -Fq 'enables both timers, but never starts them' "$installer" || die 'installer no-start contract missing'
if grep -Eq 'enable[[:space:]]+--now|systemctl[[:space:]]+start' "$installer"; then
  die 'backup installer must not start timers/services'
fi
for name in lagrange-production-backup lagrange-production-backup-verify; do
  grep -Fq 'Persistent=true' "$systemd/$name.timer" || die "$name timer must be persistent"
  grep -Fq 'NoNewPrivileges=true' "$systemd/$name.service" || die "$name service hardening missing"
  grep -Fq 'RestrictAddressFamilies=AF_UNIX' "$systemd/$name.service" || die "$name must not have network address families"
done
grep -Fq 'OnCalendar=*-*-* 02:15:00 Asia/Seoul' "$systemd/lagrange-production-backup.timer" \
  || die 'daily backup schedule mismatch'
grep -Fq 'RandomizedDelaySec=30m' "$systemd/lagrange-production-backup.timer" \
  || die 'daily backup randomization missing'
grep -Fq 'OnCalendar=Sun *-*-* 04:15:00 Asia/Seoul' \
  "$systemd/lagrange-production-backup-verify.timer" || die 'weekly verify schedule mismatch'
grep -Fq -- '--run --config-file /etc/lagrange/production-backup.conf' \
  "$systemd/lagrange-production-backup.service" || die 'daily service command mismatch'
grep -Fq -- '--verify-latest --config-file /etc/lagrange/production-backup.conf' \
  "$systemd/lagrange-production-backup-verify.service" || die 'verify service command mismatch'

echo 'PRODUCTION_OPS_STATIC: PASS'
