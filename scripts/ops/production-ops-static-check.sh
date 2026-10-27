#!/usr/bin/env bash
# Static contract check for immutable release installation and backups. No
# Docker, provider, database, systemd, or production path is invoked.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ops=$root/scripts/ops
build=$ops/build-production-images.sh
systemd=$root/deploy/systemd
release=$ops/deploy-production-release.sh
compose_release=$ops/compose-release.sh
production_config=$ops/validate-production-config.sh
artifact_wrapper=$ops/kis-historical-price-beta-artifact.sh
manifest_lib=$ops/lib/release-image-manifest.sh
daily_lib=$ops/lib/owner-beta-daily-release.sh
layout_helper=$ops/lib/release-build-layout.sh
layout_config=$root/deploy/build/release-build-layout.json
backup=$ops/run-production-backup.sh
installer=$ops/install-production-backup.sh
release_runbook=$root/docs/runbooks/production-release-and-backup.md
ops_readme=$ops/README.md

die() { echo "production-ops-static: $*" >&2; exit 1; }

for script in "$release" "$build" "$compose_release" "$production_config" "$artifact_wrapper" \
  "$manifest_lib" "$daily_lib" "$layout_helper" "$backup" "$installer"; do
  [ -f "$script" ] || die "required script missing: $script"
  bash -n "$script" || die "shell syntax failure: $script"
done
[ -f "$layout_config" ] && [ ! -L "$layout_config" ] ||
  die 'G2 release-build layout config is missing or a symlink'
[ -f "$release_runbook" ] && [ ! -L "$release_runbook" ] ||
  die 'G2 production release runbook is missing or a symlink'
[ -f "$ops_readme" ] && [ ! -L "$ops_readme" ] ||
  die 'ops image-build documentation is missing or a symlink'
self_test=$ops/production-ops-self-test.sh
bash -n "$self_test" || die 'production ops self-test has shell syntax errors'
service_status_collector=$ops/collect-service-status.py
service_status_installer=$ops/install-service-status.py
service_status_policy=$ops/lib/service-status-policy.py
service_status_unit=$systemd/lagrange-service-status.service
service_status_timer=$systemd/lagrange-service-status.timer
service_status_overlay=$root/deploy/compose/compose.service-status.yml
for file in "$service_status_collector" "$service_status_installer" "$service_status_policy" \
  "$service_status_unit" "$service_status_timer" "$service_status_overlay"; do
  [ -f "$file" ] && [ ! -L "$file" ] || die "service-status file missing or symlinked: $file"
done
grep -Fq 'TOTAL_DEADLINE_SECONDS = 20.0' "$service_status_collector" ||
  die 'service-status collector deadline changed'
grep -Fq 'MAX_COMMAND_BYTES = 16 * 1024' "$service_status_collector" ||
  die 'service-status command output bound changed'
grep -Fq 'MAX_SNAPSHOT_BYTES = 256 * 1024' "$service_status_collector" ||
  die 'service-status snapshot bound changed'
grep -Fq 'OnUnitActiveSec=30s' "$service_status_timer" ||
  die 'service-status timer interval changed'
grep -Fq 'Type=oneshot' "$service_status_unit" ||
  die 'service-status collector must remain one-shot'
grep -Fq 'TimeoutStartSec=25s' "$service_status_unit" ||
  die 'service-status systemd timeout changed'
grep -Fq 'ReadWritePaths=/var/lib/lagrange/service-status /run/lagrange-service-status' "$service_status_unit" ||
  die 'service-status write sandbox changed'
grep -Fq 'user: "10001:10001"' "$service_status_overlay" ||
  die 'service-status overlay API identity changed'
grep -Fq 'read_only: true' "$service_status_overlay" ||
  die 'service-status overlay must mount the snapshot read-only'
grep -Fq 'create_host_path: false' "$service_status_overlay" ||
  die 'service-status overlay must not create host paths'
grep -Fq 'service-status-policy.py" startup' "$compose_release" ||
  die 'release startup must validate the installed service-status policy'
grep -Fq 'service-status-policy.py" daemon-gate' "$compose_release" ||
  die 'full daemon release startup must validate the daily-timer conflict gate'
grep -Fq 'service_status_mode" = daily' "$compose_release" ||
  die 'daily service-status policy must exclude the research daemon'
grep -Fq 'TEST_ENVIRONMENT_ERROR: production-ops root fixture requires user namespaces or fakeroot' \
  "$self_test" || die 'root fixture must fail rather than skip when unavailable'
if grep -Fq 'PASS (root fixture skipped' "$self_test" || grep -Fq 'fakeroot "$@"' "$self_test"; then
  die 'production ops root fixture must not pass-on-skip or nest fakeroot'
fi

compose_file=$root/deploy/compose/compose.yml
reverse_proxy_block=$(awk '
  /^  reverse-proxy:/ { in_proxy = 1 }
  in_proxy { print }
  in_proxy && /^  [[:alnum:]_-]+:/ && $0 !~ /^  reverse-proxy:/ { exit }
' "$compose_file")
grep -Fq -- '- /var/cache/nginx:uid=101,gid=101,mode=0750' <<<"$reverse_proxy_block" ||
  die 'reverse-proxy cache tmpfs must be writable only by the unprivileged nginx identity'
grep -Fq -- '- /var/run:uid=101,gid=101,mode=0750' <<<"$reverse_proxy_block" ||
  die 'reverse-proxy runtime tmpfs must be writable only by the unprivileged nginx identity'
api_block=$(awk '
  /^  api-server:/ { in_api = 1 }
  in_api { print }
  in_api && /^  [[:alnum:]_-]+:/ && $0 !~ /^  api-server:/ { exit }
' "$compose_file")
for key in RECOMMENDATION_DATASET_VERSION_ID RECOMMENDATION_DATASET_ID \
  RECOMMENDATION_DATASET_VERSION RECOMMENDATION_CURATED_VERSION \
  RECOMMENDATION_DATASET_MANIFEST_SHA256; do
  [ "$(grep -Ec "^[[:space:]]+$key:" <<<"$api_block")" -eq 1 ] ||
    die "api-server must receive exactly one $key release pin"
done
grep -Fq 'OWNER_BETA_ACCESS_MODE: ${OWNER_BETA_ACCESS_MODE:-disabled}' "$compose_file" ||
  die 'Compose owner-beta access policy injection missing'
grep -Fq 'OWNER_BETA_PRICE_INPUT_MODE: ${OWNER_BETA_PRICE_INPUT_MODE:-disabled}' "$compose_file" ||
  die 'Compose owner-beta sealed price-input policy injection missing'
grep -Fq 'OWNER_BETA_PAPER_MODE: ${OWNER_BETA_PAPER_MODE:-disabled}' "$compose_file" ||
  die 'Compose owner-beta Paper policy injection missing'
grep -Fq 'OWNER_BETA_EQUITY_SIGNALS_MODE: ${OWNER_BETA_EQUITY_SIGNALS_MODE:-disabled}' "$compose_file" ||
  die 'Compose fixed-equity owner-beta policy injection missing'
[ "$(grep -Ec '^[[:space:]]+OWNER_BETA_[A-Z0-9_]+:' "$compose_file")" -eq 5 ] ||
  die 'Compose must expose the four approved modes and the controlled daily mode only'
[ "$(grep -Ec '^      OWNER_BETA_DAILY_MODE: disabled$' <<<"$api_block")" -eq 1 ] ||
  die 'base api-server daily mode must be literal disabled'
daily_worker_block=$(awk '
  /^  owner-beta-runner:/ { in_worker = 1; next }
  in_worker && /^  [[:alnum:]_-]+:/ { exit }
  in_worker { print }
' "$compose_file")
[ "$(grep -Ec '^      PRICE_BETA_DAILY_MODE: disabled$' <<<"$daily_worker_block")" -eq 1 ] ||
  die 'base owner-beta-runner daily mode must be literal disabled'
[ "$(grep -Ec '^[[:space:]]+PRICE_BETA_DAILY_MODE:' "$compose_file")" -eq 1 ] ||
  die 'base daily worker mode must reach only owner-beta-runner'
for token in '--daily-prepare|--daily-enable|--daily-disable|--daily-rebind-first-session)' \
  'lib/owner-beta-daily-operations.py' '--daily-backup|--daily-backup-verify|--daily-reconcile)' \
  'lib/owner-beta-daily-backup.py'; do
  grep -Fq -- "$token" "$compose_release" ||
    die 'controlled daily operation entrypoint missing'
done
for guard_call in 'acquire_daily_release_lock' \
  'daily_release_guard guard-processes' 'daily_release_guard guard'; do
  grep -Eq "^[[:space:]]*$guard_call$" "$compose_release" ||
    die 'controlled daily full-release quiescence call missing'
done
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
grep -Fq '${LAGRANGE_DATA_DIR:-../data}/stock-price-beta-artifacts:/data/stock-price-beta-artifacts:ro' "$compose_file" ||
  die 'api-server fixed-equity artifact mount must remain read-only'
grep -Fq 'STOCK_PRICE_BETA_ARTIFACT_ROOT: /data/stock-price-beta-artifacts' "$compose_file" ||
  die 'api-server fixed-equity artifact root must be a fixed in-container path'
for token in owner_beta_equity_signals_mode_invalid \
  owner_beta_equity_signals_file_forbidden \
  owner_beta_equity_signals_requires_owner_only \
  owner_beta_equity_signals_shell_override_mismatch; do
  grep -Fq "$token" "$production_config" ||
    die "fixed-equity owner-beta production gate missing: $token"
done
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

grep -Fq 'COMPOSE_PARALLEL_LIMIT=1' "$build" ||
  die 'image build helper must force Compose parallelism to one'
grep -Fq 'COMPOSE_PARALLEL_LIMIT=1' "$compose_release" ||
  die 'infrastructure/backfill Compose builds must force parallelism to one'
grep -Fq 'validate_manifest_output' "$build" ||
  die 'image build helper must validate manifest output before and at publication'
grep -Fq 'release_build_layout_init' "$build" ||
  die 'image build helper must initialize the G2 common-artifact state before builds'
grep -Fq 'release_build_layout_verify_image' "$build" ||
  die 'image build helper must bind every saved image through G2 byte verification'
grep -Fq 'release_build_layout_lock' "$layout_helper" ||
  die 'G2 helper must retain the whole-run lock API'
grep -Fq 'release_build_layout_archive_scan' "$layout_helper" ||
  die 'G2 helper must retain the offline image-save parser API'

# Keep the operator-facing instructions aligned with the frozen G2 route. The
# check intentionally asserts contracts and checkpoint order, rather than a
# copy of the helper's private receipt representation.
for literal in \
  'scripts/ops/lib/release-build-layout.sh' \
  'release_build_layout_init <source-root> <commit> <state-root> <cache-namespace>' \
  '3 + 3 + 2 + 2' \
  'web`, `research-worker`, `recommendation-runner' \
  'candidate-runner`, `owner-beta-runner`, `owner-equity-v2-runner' \
  'strict saved-image byte/OCI checks' \
  'release-totals.tsv' \
  'private evidence, never official release manifests' \
  'reserved for WP6'
do
  grep -Fq -- "$literal" "$release_runbook" ||
    die "release runbook omitted frozen G2 operation contract: $literal"
done
for literal in \
  'current-EUID state directory' \
  '3+3+2+2' \
  '3+3+3+3' \
  'source fallbacks remain build-compatible' \
  '--manifest-file' \
  'release-totals.tsv' \
  'WP6 owns that production evidence'
do
  grep -Fq -- "$literal" "$ops_readme" ||
    die "ops README omitted frozen G2 operation contract: $literal"
done

# Parse the documented shell commands as logical continuation lines so a prose
# mention of --env-file cannot conceal its omission from plan, preflight, or
# the official background apply. The image-only input is external, exact, and
# must be prepared before the build; the operational env is installed only
# after the builder finishes successfully.
if ! python3 - "$ops_readme" "$release_runbook" <<'PY'
import pathlib
import sys

readme_path, runbook_path = map(pathlib.Path, sys.argv[1:])
readme = readme_path.read_text(encoding="utf-8")
runbook = runbook_path.read_text(encoding="utf-8")
sentinel = "RESEARCH_ENTITLEMENT_SHA256=" + "0" * 64
sentinel_hash = "df9d4d1ceb45d0ddb79b98b1fc12c5a2925424c46b79b5d9959f0a1640b27bf6"

def collapsed(text):
    return text.replace("\\\n", " ")

def require_one(document, marker, label):
    matches = [line for line in collapsed(document).splitlines() if marker in line]
    if len(matches) != 1:
        raise SystemExit(label + "-command-count")
    if matches[0].count('--env-file "$image_build_env"') != 1:
        raise SystemExit(label + "-env-binding")

readme_section = readme.split("## Prebuild production service images", 1)[1].split("\n## ", 1)[0]
for document, label in ((readme_section, "readme"), (runbook, "runbook")):
    if sentinel not in document or sentinel_hash not in document:
        raise SystemExit(label + "-sentinel-binding")
    if 'mktemp -d "${TMPDIR:-/tmp}/lagrange-image-only-env.XXXXXXXXXX"' not in document:
        raise SystemExit(label + "-private-external-parent")
    if 'chmod 0700 "$image_build_env_dir"' not in document or 'chmod 0600 "$image_build_env"' not in document:
        raise SystemExit(label + "-private-mode")
    if "printf 'image_build_env=%s sha256=%s" not in document:
        raise SystemExit(label + "-path-hash-record")

require_one(readme_section, "scripts/ops/build-production-images.sh --plan", "readme-plan")
require_one(readme_section, "scripts/ops/build-production-images.sh --preflight", "readme-preflight")
apply_marker = '/bin/bash "$release_source_root/scripts/ops/build-production-images.sh" --apply'
require_one(runbook, apply_marker, "runbook-background-apply")

runbook_flat = collapsed(runbook)
prepare_at = runbook_flat.index('image_build_env_dir=$(mktemp -d')
apply_at = runbook_flat.index(apply_marker)
operational_install = 'sudo install -o root -g root -m 0600 deploy/compose/.env'
install_at = runbook_flat.index(operational_install)
if not prepare_at < apply_at < install_at:
    raise SystemExit("runbook-image-build-operational-env-order")
PY
then
  die 'operator examples do not bind the private credential-free image-only Compose env'
fi
for service in db-role-bootstrap db-migrate research-worker; do
  grep -Fq "build --pull=false $service" "$compose_release" ||
    die "Compose build plan/call missing single-service build: $service"
done
if grep -Eq 'build --pull=false[[:space:]]+db-role-bootstrap[[:space:]]+db-migrate([[:space:]]|$)' \
   "$compose_release" ||
   grep -Eq 'build --pull=false[[:space:]]+db-role-bootstrap[[:space:]]+db-migrate[[:space:]]+research-worker([[:space:]]|$)' \
   "$compose_release"; then
  die 'infrastructure/backfill must not submit multiple services to one Compose build call'
fi
grep -Fq 'run_owner_beta_approval_gate' "$compose_release" ||
  die 'owner-beta pre-start approval gate missing'
approval_gate_line=$(grep -n '^run_owner_beta_approval_gate$' "$compose_release" | tail -n1 | cut -d: -f1)
first_release_up_line=$(grep -n '^compose up --no-build --wait postgres$' "$compose_release" | cut -d: -f1)
[ -n "$approval_gate_line" ] && [ -n "$first_release_up_line" ] &&
  [ "$approval_gate_line" -lt "$first_release_up_line" ] ||
  die 'owner-beta approval gate must precede the first release Compose up'
grep -Fq 'owner-beta-runner' "$compose_release" ||
  die 'owner-beta runner release integration missing'
grep -Fq 'elif [ "$owner_beta_access_mode" = owner_only ]; then' "$compose_release" ||
  die 'owner-beta runner must be selected only in owner-only mode'
grep -Fq 'candidate-runner owner-beta-runner' "$compose_release" ||
  die 'owner-beta runner startup order missing'
grep -Fq 'disabled leaves owner-beta-runner inactive' "$compose_release" ||
  die 'release plan must state disabled owner-beta behavior'
grep -Fq 'approved_registry_sha256=sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707' "$artifact_wrapper" ||
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
grep -Fq -- '--action activate' "$release" ||
  die 'release activation controller missing'
grep -Fq 'os.replace(temporary, "current", src_dir_fd=parent, dst_dir_fd=parent)' "$ops/lib/owner-beta-simulation-operations.py" ||
  die 'release current switch is not atomic'
for operation in '--action begin-release' '--action finish-release'; do
  grep -Fq -- "$operation" "$compose_release" || die 'simulation release handoff entrypoint missing'
done
for suite in test_owner_beta_simulation_operations.py test_owner_beta_simulation_backup.py; do
  [ "$(grep -Fc "python3 -B \"\$ops/$suite\"" "$self_test")" -eq 1 ] ||
    die 'simulation suite registration missing or duplicated'
done
grep -Fq 'current has a foreign link target' "$release" ||
  die 'foreign current-link fence missing'
grep -Fq 'deploy/compose/.env' "$release" || die 'protected Compose env install missing'
grep -Fq 'source "$script_dir/lib/dotenv.sh"' "$release" ||
  die 'release installer must use the non-evaluating dotenv parser'
grep -Fq 'dotenv_load "$path"' "$release" ||
  die 'release installer must parse the protected env before use'
grep -Fq 'dotenv_get LAGRANGE_CODE_COMMIT' "$release" ||
  die 'release installer must read the commit pin from the protected env'
if grep -Fq 'dotenv_effective_get' "$release"; then
  die 'release installer must not use a shell-overridable dotenv value'
fi
grep -Fq 'validate_release_env_commit "$env_source" env-source' "$release" ||
  die 'apply must validate the external env commit before staging'
grep -Fq 'validate_release_env_commit "$target/deploy/compose/.env" release-compose-env' "$release" ||
  die 'installed release env commit validation missing'
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
grep -Fq 'validate_release "$stage" "$release_commit"' "$release" ||
  die 'staged release env must be revalidated before publication'
grep -Fq 'validate_release "$release_dir" "$release_commit"' "$release" ||
  die 'installed manifest must be validated before atomic activation'
for token in '--daily-config-source' 'daily_release_parse_mode' \
  'validate_daily_release_config' 'reader-check' 'reader-guard' \
  'acquire_release_lock' 'flock --exclusive --nonblock'; do
  grep -Fq -- "$token" "$release" ||
    die 'immutable daily installation or rollback gate missing'
done
grep -Fq 'daily_release_parse_mode' "$production_config" ||
  die 'production config must share the closed daily mode grammar'
for token in 'OWNER_BETA_DAILY_MODE_FILE' 'LAGRANGE_DAILY_VERIFIED_CONFIG_SHA256' \
  'owner_beta_daily_requires_sealed_price' 'owner_beta_daily_requires_paper_disabled'; do
  grep -Fq "$token" "$daily_lib" ||
    die 'daily protected input boundary missing'
done
grep -Fq 'owner-beta-daily-producer|owner-beta-daily-admitter)' "$manifest_lib" ||
  die 'daily runtime alias canonical image resolver missing'
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

# This invokes only a repository static checker; it never contacts Docker or
# production state. Keep the build-DAG safety contract coupled to the release
# installer/backup checks that consume its immutable V2 output.
bash "$ops/build-production-images-static-check.sh" >/dev/null

echo 'PRODUCTION_OPS_STATIC: PASS'
