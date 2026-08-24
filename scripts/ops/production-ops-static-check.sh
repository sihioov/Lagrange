#!/usr/bin/env bash
# Static contract check for immutable release installation and backups. No
# Docker, provider, database, systemd, or production path is invoked.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ops=$root/scripts/ops
systemd=$root/deploy/systemd
release=$ops/deploy-production-release.sh
compose_release=$ops/compose-release.sh
production_config=$ops/validate-production-config.sh
artifact_wrapper=$ops/kis-historical-price-beta-artifact.sh
manifest_lib=$ops/lib/release-image-manifest.sh
backup=$ops/run-production-backup.sh
installer=$ops/install-production-backup.sh

die() { echo "production-ops-static: $*" >&2; exit 1; }

for script in "$release" "$compose_release" "$production_config" "$artifact_wrapper" \
  "$manifest_lib" "$backup" "$installer"; do
  [ -f "$script" ] || die "required script missing: $script"
  bash -n "$script" || die "shell syntax failure: $script"
done
self_test=$ops/production-ops-self-test.sh
bash -n "$self_test" || die 'production ops self-test has shell syntax errors'
grep -Fq 'TEST_ENVIRONMENT_ERROR: production-ops root fixture requires user namespaces or fakeroot' \
  "$self_test" || die 'root fixture must fail rather than skip when unavailable'
if grep -Fq 'PASS (root fixture skipped' "$self_test" || grep -Fq 'fakeroot "$@"' "$self_test"; then
  die 'production ops root fixture must not pass-on-skip or nest fakeroot'
fi

compose_file=$root/deploy/compose/compose.yml
grep -Fq 'OWNER_BETA_ACCESS_MODE: ${OWNER_BETA_ACCESS_MODE:-disabled}' "$compose_file" ||
  die 'Compose owner-beta access policy injection missing'
grep -Fq 'OWNER_BETA_PRICE_INPUT_MODE: ${OWNER_BETA_PRICE_INPUT_MODE:-disabled}' "$compose_file" ||
  die 'Compose owner-beta sealed price-input policy injection missing'
grep -Fq 'OWNER_BETA_PAPER_MODE: ${OWNER_BETA_PAPER_MODE:-disabled}' "$compose_file" ||
  die 'Compose owner-beta Paper policy injection missing'
[ "$(grep -Ec '^[[:space:]]+OWNER_BETA_[A-Z0-9_]+:' "$compose_file")" -eq 3 ] ||
  die 'Compose must expose exactly the three approved owner-beta policy modes'
[ "$(awk '
  /^  api-server:/ { in_api = 1; next }
  in_api && /^  [[:alnum:]_-]+:/ { exit }
  in_api && /OWNER_BETA_PRICE_INPUT_MODE:/ { count++ }
  END { print count + 0 }
' "$compose_file")" -eq 1 ] ||
  die 'sealed price-input mode must reach only api-server'
if grep -Eq '^[[:space:]]+OWNER_BETA_[A-Z0-9_]*(CANDIDATE|SHA256|HASH|ROOT|PATH|REGISTRY)[A-Z0-9_]*:' \
   "$compose_file"; then
  die 'Compose must not expose owner-beta candidate, pin, path, root, or registry state'
fi
grep -Fq '${LAGRANGE_ARTIFACTS_DIR:-../data/artifacts}:/data/artifacts:ro' "$compose_file" ||
  die 'api-server sealed artifact mount must remain read-only'
grep -Fq 'owner_beta_price_input_mode_invalid' "$production_config" ||
  die 'sealed price-input mode must be exact'
grep -Fq 'disabled|sealed_v1)' "$production_config" ||
  die 'sealed price-input mode allowlist must be disabled or sealed_v1'
grep -Fq 'owner_beta_price_input_file_forbidden' "$production_config" ||
  die 'sealed price-input file channel must be rejected'
grep -Fq 'owner_beta_price_input_requires_owner_only' "$production_config" ||
  die 'sealed price input must require owner-only access'
grep -Fq 'owner_beta_price_input_shell_override_mismatch' "$production_config" ||
  die 'sealed price-input shell override must not bypass the protected env file'
grep -Fq 'owner_beta_paper_evidence_unavailable' "$production_config" ||
  die 'Paper must remain blocked without a future evidence checker'
grep -Fq 'run_owner_beta_approval_gate' "$compose_release" ||
  die 'owner-beta pre-start approval gate missing'
approval_gate_line=$(grep -n '^run_owner_beta_approval_gate$' "$compose_release" | tail -n1 | cut -d: -f1)
first_release_up_line=$(grep -n '^compose up --no-build --wait postgres$' "$compose_release" | cut -d: -f1)
[ -n "$approval_gate_line" ] && [ -n "$first_release_up_line" ] &&
  [ "$approval_gate_line" -lt "$first_release_up_line" ] ||
  die 'owner-beta approval gate must precede the first release Compose up'
grep -Fq 'approval_registry_sha256=sha256:[0-9a-f]{64}' "$artifact_wrapper" ||
  die 'artifact wrapper approval output must bind the embedded registry hash'

grep -Fq 'status --porcelain=v1 --untracked-files=all' "$release" ||
  die 'release clean-tree fence missing'
grep -Fq '?? docs/kis_openapi_entiredocs_20260818_030007.xlsx' "$release" ||
  die 'release workbook-only untracked allowlist missing'
grep -Fq 'git -c safe.directory="$repo_root"' "$release" ||
  die 'release sudo safe.directory must be command-local'
grep -Fq 'archive --format=tar "$release_commit"' "$release" ||
  die 'release must materialize exact Git object'
grep -Fq 'refusing to overwrite existing release directory' "$release" ||
  die 'release overwrite refusal missing'
grep -Fq 'mv -Tf -- "$temporary" "$current_link"' "$release" ||
  die 'release current switch is not atomic'
grep -Fq 'current has a foreign link target' "$release" ||
  die 'foreign current-link fence missing'
grep -Fq 'deploy/compose/.env' "$release" || die 'protected Compose env install missing'
grep -Fq 'root:root mode 0600' "$manifest_lib" || die 'protected file metadata contract missing'
grep -Fq -- '--release-manifest' "$release" || die 'release manifest option missing'
grep -Fq 'LAGRANGE_RELEASE_MANIFEST' "$release" || die 'release manifest environment contract missing'
grep -Fq 'LAGRANGE_RELEASE_MANIFEST_V2' "$manifest_lib" || die 'V2 manifest validation missing'
grep -Fq '.lagrange-release-manifest' "$release" || die 'installed release manifest marker missing'
grep -Fq -- '--apply requires --release-manifest' "$release" ||
  die 'manifest-less apply must be blocked'
grep -Fq 'legacy manifest-less release is blocked' "$release" ||
  die 'legacy manifest-less release block missing'
grep -Fq -- '--release-manifest is allowed only with --apply' "$release" ||
  die 'check/rollback external manifest rejection missing'
grep -Fq 'release_image_manifest_trusted_file "$path" release-manifest' "$release" ||
  die 'external manifest ownership/mode trust check missing'
grep -Fq 'release_image_manifest_trusted_directory' "$manifest_lib" ||
  die 'root-owned non-writable path trust check missing'
grep -Fq 'must be a regular non-symlink file' "$manifest_lib" ||
  die 'manifest regular-file/symlink fence missing'
grep -Fq 'install -o 0 -g 0 -m 0600 -- "$release_manifest"' "$release" ||
  die 'manifest must be copied once into root-owned staging'
grep -Fq 'validate_installed_manifest "$stage/.lagrange-release-manifest"' "$release" ||
  die 'staged manifest must be revalidated after its only copy'
grep -Fq 'validate_release "$release_dir" "$release_commit"' "$release" ||
  die 'installed manifest must be validated before atomic activation'
if grep -Eiq 'docker[[:space:]]' "$release"; then
  die 'installer must not perform Docker/image/container work'
fi
if grep -Eq 'rsync|cp[[:space:]].*repo_root|git[[:space:]]+checkout' "$release"; then
  die 'release must not copy mutable worktree content'
fi

grep -Fq 'aes-256-cbc -salt -pbkdf2 -iter 200000' "$backup" || die 'backup encryption contract missing'
grep -Fq 'postgres.dump.enc' "$backup" || die 'PostgreSQL encrypted class missing'
grep -Fq 'raw.tar.enc' "$backup" || die 'Raw encrypted class missing'
grep -Fq 'curated.tar.enc' "$backup" || die 'Curated encrypted class missing'
grep -Fq -- '--network none --read-only --user 999:999' "$backup" ||
  die 'isolated restore verification contract missing'
grep -Fq 'CREATE ROLE migration_owner NOLOGIN' "$backup" || die 'isolated restore role bootstrap missing'
grep -Fq 'CREATE ROLE research_writer NOLOGIN' "$backup" || die 'isolated restore role inventory incomplete'
grep -Fq 'pg_restore --exit-on-error --no-owner --no-privileges -d verify' "$backup" ||
  die 'actual PostgreSQL restore must fail closed on restore errors'
grep -Fq '_sqlx_migrations' "$backup" || die 'isolated restore schema verification missing'
grep -Fq 'dataset_versions' "$backup" || die 'isolated restore dataset schema verification missing'
grep -Fq 'MIN_KEEP wins' "$backup" || die 'bounded retention floor missing'
grep -Fq 'MAX_TOTAL_BYTES cannot be satisfied' "$backup" || die 'disk cap failure missing'
grep -Fq 'rm -rf -- "$oldest_path"' "$backup" || die 'retention must use resolved exact set path'
if grep -Eq -- '--key([=[:space:]]|$)|BACKUP_KEY=' "$backup"; then
  die 'backup secret must not enter argv or environment'
fi

grep -Fq 'enables both timers, but never starts them' "$installer" ||
  die 'installer no-start contract missing'
if grep -Eq 'enable[[:space:]]+--now|systemctl[[:space:]]+start' "$installer"; then
  die 'backup installer must not start timers/services'
fi
for name in lagrange-production-backup lagrange-production-backup-verify; do
  grep -Fq 'Persistent=true' "$systemd/$name.timer" || die "$name timer must be persistent"
  grep -Fq 'NoNewPrivileges=true' "$systemd/$name.service" || die "$name service hardening missing"
  grep -Fq 'RestrictAddressFamilies=AF_UNIX' "$systemd/$name.service" ||
    die "$name must not have network address families"
done
grep -Fq 'OnCalendar=*-*-* 02:15:00 Asia/Seoul' "$systemd/lagrange-production-backup.timer" ||
  die 'daily backup schedule mismatch'
grep -Fq 'RandomizedDelaySec=30m' "$systemd/lagrange-production-backup.timer" ||
  die 'daily backup randomization missing'
grep -Fq 'OnCalendar=Sun *-*-* 04:15:00 Asia/Seoul' \
  "$systemd/lagrange-production-backup-verify.timer" || die 'weekly verify schedule mismatch'
grep -Fq -- '--run --config-file /etc/lagrange/production-backup.conf' \
  "$systemd/lagrange-production-backup.service" || die 'daily service command mismatch'
grep -Fq -- '--verify-latest --config-file /etc/lagrange/production-backup.conf' \
  "$systemd/lagrange-production-backup-verify.service" || die 'verify service command mismatch'

echo 'PRODUCTION_OPS_STATIC: PASS'
