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
  bootstrap-negative.sh \
  preflight-baseline.sql \
  preflight-0039.sql \
  preflight-catalog.sql \
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
bash -n "$dir/bootstrap-negative.sh" || die 'bootstrap-negative.sh has shell syntax errors'
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
  "run_db_test jobqueue-daily-admitter-pool '-p job-queue --test owner_beta_publish daily_input_admitter_pool_is_direct_bounded_and_unprivileged'" \
  "run_db_test api-daily-admitter-pool '-p api-server --test http_owner_beta_access http_daily_input_admitter_pool_is_direct_bounded_and_unprivileged'" \
  'preflight-0039.sql' \
  'run_terminal_paper_check' \
  'preflight-catalog.sql' \
  'rollback-0039-guard.sql' \
  'rollback-0039-postflight.sql' \
  'run_schema_only_stage38' \
  '--entrypoint /bin/bash' \
  'password_file=/run/secrets/db_migration_owner_password' \
  '[[ "$password" =~ ^[0-9a-f]{64}$ ]]' \
  'export PGOPTIONS="-c lock_timeout=5s -c statement_timeout=60s"' \
  'run_migration_rerun_0039' \
  'run_full_release_lane' \
  'run_bootstrap_negative_contract' \
  '/validation/bootstrap-negative.sh --disposable-ci' \
  'bootstrap_negative_cases' \
  'run_catalog_negative_probe' \
  'migration_checksums=' \
  'run_migration_rerun_0041' \
  'full_compose down -v --remove-orphans' \
  'full_cleanup=PASS_NO_OWNED_CONTAINERS' \
  'run_service_login' \
  'down -v --remove-orphans' \
  'printf '\''%s\n'\'' "$tool_block"'; do
  grep -Fq -- "$required_marker" "$workflow" || die "workflow marker missing: $required_marker"
done
for required_catalog_marker in \
  'baseline catalog does not contain the five exact expected IDs' \
  "v_expected_strategy_count := CASE v_expected_lane" \
  "MESSAGE = 'catalog_preflight: unexpected_strategy_id'" \
  "MESSAGE = 'catalog_preflight: missing_baseline_strategy'" \
  "MESSAGE = 'catalog_preflight: baseline_strategy_content_mismatch'" \
  "MESSAGE = 'catalog_preflight: staged_paper_fixture_mismatch'" \
  "MESSAGE = 'catalog_preflight: full_lane_paper_fixture_present'" \
  "VALUES ('postgres_validation_paper', 'PostgreSQL validation Paper', 'Paper')" \
  'CATALOG_NEGATIVE_PROBE_ROLLED_BACK:unexpected_strategy' \
  'CATALOG_NEGATIVE_PROBE_ROLLED_BACK:missing_baseline' \
  'CATALOG_NEGATIVE_PROBE_ROLLED_BACK:mutated_baseline' \
  'CATALOG_NEGATIVE_PROBE_ROLLED_BACK:full_paper' \
  "records.entry->>'version' IS DISTINCT FROM '1.0.0'" \
  "schema_row.schema_json IS DISTINCT FROM records.entry->'parameter_schema'" \
  'strategy-catalog:' \
  'catalog_fingerprint=' \
  'catalog_audit_id=' \
  'catalog_public_table_ownership=' \
  'catalog_function_identity=' \
  'catalog_function_execute_acl=' \
  'catalog_auth_audit_outbox_rls=' \
  'synthetic catalog conflict' \
  '/validation/sync-baseline-strategy-catalog.sql'; do
  grep -Fq "$required_catalog_marker" "$dir/preflight-catalog.sql" \
    || die "catalog preflight marker missing: $required_catalog_marker"
done
grep -Fq 'v_event_count <> 1' "$dir/preflight-0039.sql" \
  || die 'preflight-0039.sql does not require the exact catalog audit row count'
grep -Fq "v_event.action IS DISTINCT FROM 'strategy_catalog.installed'" "$dir/preflight-0039.sql" \
  || die 'preflight-0039.sql does not assert the catalog audit action'
grep -Fq "'terminal_paper_targets=' || count(*)" "$dir/preflight-0039.sql" \
  || die 'preflight-0039.sql does not retain the terminal Paper fixture assertion'
grep -Fq '\if :terminal_only' "$dir/preflight-0039.sql" \
  || die 'preflight-0039.sql does not isolate its RLS-safe terminal Paper pass'
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
