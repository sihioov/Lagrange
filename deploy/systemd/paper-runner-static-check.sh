#!/usr/bin/env bash
# Static contract check for the host Paper systemd deployment.
#
# This deliberately does not start systemd, PostgreSQL, or the Paper binary.
# It catches the unsafe regression where a direct role URL is put back into
# the EnvironmentFile template or the unit stops invoking the secret adapter.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
unit="$root/deploy/systemd/paper-runner.service"
env_example="$root/deploy/systemd/paper-runner.env.example"
wrapper="$root/deploy/runtime/paper-runner-entrypoint"
health_test="$root/deploy/runtime/paper-runner-healthcheck-test.sh"

die() {
  echo "paper-runner-systemd-check: $*" >&2
  exit 1
}

[ -f "$unit" ] || die "missing unit: $unit"
[ -f "$env_example" ] || die "missing environment template: $env_example"
[ -f "$wrapper" ] || die "missing secret-file wrapper: $wrapper"
[ -x "$health_test" ] || die "missing executable healthcheck contract test: $health_test"

grep -Fq 'ConditionPathExists=/opt/lagrange/bin/paper-runner' "$unit" \
  || die 'unit must require the wrapper path'
grep -Fq 'ConditionPathExists=/usr/local/bin/paper-runner-bin' "$unit" \
  || die 'unit must require the Rust binary path'
grep -Fq 'ConditionFileIsExecutable=/opt/lagrange/bin/paper-runner' "$unit" \
  || die 'unit must require an executable wrapper'
grep -Fq 'ConditionFileIsExecutable=/usr/local/bin/paper-runner-bin' "$unit" \
  || die 'unit must require an executable Rust binary'
grep -Fq 'ExecStartPre=/opt/lagrange/bin/paper-runner healthcheck --startup' "$unit" \
  || die 'unit must gate startup on the wrapper healthcheck'
grep -Fq 'ExecStart=/opt/lagrange/bin/paper-runner' "$unit" \
  || die 'unit must execute the wrapper'
grep -Fq 'Type=notify' "$unit" \
  || die 'unit must use systemd readiness/watchdog notifications'
grep -Fq 'NotifyAccess=main' "$unit" \
  || die 'unit must allow the main daemon to notify systemd'
grep -Fq 'WatchdogSec=30s' "$unit" \
  || die 'unit must configure a runtime watchdog'
grep -Fq 'validate_curated_dataset' "$wrapper" \
  || die 'wrapper must validate the curated manifest and bars layout'
grep -Fq -- "-path '*/version=2/manifest.json'" "$wrapper" \
  || die 'wrapper must require a version=2 dataset manifest'
grep -Fq -- "-path '*/version=2/bars.parquet'" "$wrapper" \
  || die 'wrapper must require non-empty version=2 bars'

if awk '!/^[[:space:]]*#/ && /^[A-Z_]*DATABASE_URL=/' "$env_example" \
  | grep -q .; then
  die 'environment template contains a direct database URL assignment'
fi

for key in \
  APP_ENV PAPER_DB_HOST PAPER_DB_PORT PAPER_DB_NAME \
  PAPER_APP_DB_PASSWORD_FILE PAPER_WORKER_DB_PASSWORD_FILE \
  PAPER_ADMIN_DB_PASSWORD_FILE PAPER_AUDIT_DB_PASSWORD_FILE \
  LAGRANGE_DATASET_ROOT LAGRANGE_REPO_ROOT PAPER_HEALTH_STATE_PATH \
  PAPER_HEALTH_MAX_AGE_SECS PAPER_OPERATION_TIMEOUT_MS \
  PAPER_CYCLE_TIMEOUT_MS PAPER_SHUTDOWN_GRACE_MS; do
  grep -Eq "^${key}=" "$env_example" \
    || die "environment template is missing ${key}"
done

grep -Eq '^APP_ENV=production$' "$env_example" \
  || die 'systemd environment must select production mode'
grep -Fq 'PAPER_HEALTH_STATE_PATH=/run/lagrange-paper-runner/health.json' "$env_example" \
  || die 'systemd environment must configure the non-secret progress state path'
grep -Fq 'RuntimeDirectory=lagrange-paper-runner' "$unit" \
  || die 'unit must provision the progress state directory'
grep -Fq 'validate_health_state' "$wrapper" \
  || die 'runtime healthcheck must validate loop progress'
grep -Fq 'paper_settlement_outbox_stats' "$wrapper" \
  || die 'runtime healthcheck must gate on Paper settlement backlog readiness'
for marker in cycle_in_progress cycle_deadline_at 'loop progress is stale' \
  'cycle deadline exceeded' 'PAPER_HEALTH_MAX_AGE_SECS'; do
  grep -Fq "$marker" "$wrapper" \
    || die "runtime healthcheck is missing progress marker: $marker"
done
for marker in 'READY=1' 'WATCHDOG=1' 'progress_is_live' 'WatchdogSec=30s'; do
  grep -Fq "$marker" "$root/crates/api-server/src/bin/paper-runner.rs" "$unit" "$root/deploy/systemd/README.md" \
    || die "systemd runtime supervision is missing marker: $marker"
done
for marker in 'outbox_backlog' 'outbox_oldest_age_secs' 'outbox_exhausted' 'outbox_ready'; do
  grep -Fq "$marker" "$root/crates/api-server/src/bin/paper-runner.rs" \
    || die "Paper runner telemetry is missing marker: $marker"
done
grep -Fq 'systemd abstract notify socket' "$root/crates/api-server/src/bin/paper-runner.rs" \
  || die 'Paper runner must support Linux abstract NOTIFY_SOCKET addresses'
grep -Fq 'systemd_notify_sends_ready_and_watchdog_to_pathname_socket' \
  "$root/crates/api-server/src/bin/paper-runner.rs" \
  || die 'Paper runner must test pathname READY/WATCHDOG notifications'
grep -Fq 'systemd_notify_sends_ready_and_watchdog_to_abstract_socket' \
  "$root/crates/api-server/src/bin/paper-runner.rs" \
  || die 'Paper runner must test abstract READY/WATCHDOG notifications'
grep -Fq 'deploy/runtime/paper-runner-entrypoint' "$root/deploy/systemd/README.md" \
  || die 'README must document the wrapper installation'

"$health_test" >/dev/null || die 'runtime healthcheck contract test failed'

echo 'PAPER_RUNNER_SYSTEMD_STATIC: PASS'
