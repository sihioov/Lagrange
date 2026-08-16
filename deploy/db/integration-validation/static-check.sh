#!/usr/bin/env bash
# Static/self checks for the disposable PostgreSQL validation workflow.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
dir="$root/deploy/db/integration-validation"
workflow="$dir/validate.sh"

die() {
  echo "PG_VALIDATION_STATIC: FAIL: $*" >&2
  exit 1
}

[ -x "$workflow" ] || die 'validate.sh must be executable'
[ -x "$dir/migration-safety-audit.sh" ] || die 'migration-safety-audit.sh must be executable'
for required in \
  preflight-baseline.sql \
  preflight-0039.sql \
  preflight-0040.sql \
  preflight-0041.sql \
  hazards.sql \
  identity-boundary.sql \
  service-login.sql \
  seed-pre-0039.sql \
  rollback-0039-guard.sql \
  rollback-0039-postflight.sql \
  EVIDENCE_TEMPLATE.md; do
  [ -f "$dir/$required" ] || die "missing validation file: $required"
done

bash -n "$workflow" || die 'validate.sh has shell syntax errors'
bash -n "$dir/migration-safety-audit.sh" || die 'migration-safety-audit.sh has shell syntax errors'

help_output="$dir/.static-help.$$"
self_output="$dir/.static-self.$$"
trap 'rm -f -- "$help_output" "$self_output"' EXIT HUP INT TERM
if ! "$workflow" --help >"$help_output" 2>&1; then
  die '--help failed'
fi
grep -Fq -- '--self-test' "$help_output" || die '--help omits --self-test'
grep -Fq -- '--evidence-dir' "$help_output" || die '--help omits --evidence-dir'
if ! "$workflow" --self-test >"$self_output" 2>&1; then
  die '--self-test failed'
fi
grep -Fq 'PG_VALIDATION_SELF_TEST: PASS' "$self_output" || die 'self-test did not emit PASS'

for required_marker in \
  'DATABASE_URL="$test_database_url"' \
  "grep -Fq 'SKIP:'" \
  'preflight-0039.sql' \
  'rollback-0039-guard.sql' \
  'rollback-0039-postflight.sql' \
  'run_migration_rerun_0041' \
  'run_service_login' \
  'down -v --remove-orphans' \
  'printf '\''%s\n'\'' "$tool_block"'; do
  grep -Fq "$required_marker" "$workflow" || die "workflow marker missing: $required_marker"
done
grep -Fq 'app_invitation_acl_select=' "$dir/preflight-0040.sql" \
  || die 'preflight-0040.sql omits the app invitation ACL assertion'
grep -Fq 'current_user || '\''='\'' || session_user' "$dir/service-login.sql" \
  || die 'service-login.sql does not assert current_user=session_user'

# Keep the migration/product audit explicit: it may expose an existing defect
# while the harness still passes its no-daemon self-test. Include its output in
# Go/No-Go evidence and never suppress it.
audit_log="$dir/.static-migration-audit.$$"
if ! "$dir/migration-safety-audit.sh" >"$audit_log" 2>&1; then
  sed -E "s#(postgres(ql)?://)[^[:space:]\"'<>]+#\\1<redacted>#g" "$audit_log" >&2
  rm -f -- "$audit_log"
  die 'migration safety audit failed (see the emitted guard finding)'
fi
rm -f -- "$audit_log"

echo 'PG_VALIDATION_STATIC: PASS'
