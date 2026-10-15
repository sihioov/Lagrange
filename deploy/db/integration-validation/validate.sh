#!/usr/bin/env bash
# Disposable PostgreSQL integration-validation workflow.
set -euo pipefail
umask 077

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../../.." && pwd)
qa_compose="$root/deploy/qa/qa-db.compose.yml"
role_script="$root/deploy/db/bootstrap-roles.sh"

usage() {
  cat <<'USAGE'
Usage: deploy/db/integration-validation/validate.sh [options]

Run disposable PostgreSQL 18 staged-upgrade, full-release, and DB-gated Rust lanes.
Upgrade credentials are generated under a private temporary directory; the
test lane uses the repository's synthetic compatibility fixture. Both
clusters are torn down on every exit by default.

Options:
  --evidence-dir PATH  Keep sanitized logs/evidence at PATH (default: a new
                       /tmp/lagrange-pg-validation-evidence.* directory).
  --main-port PORT     Loopback port for the upgrade cluster (default: 55432).
  --test-port PORT     Loopback port for the DB-gated test cluster
                       (default: 55433).
  --full-port PORT     Loopback port for the empty full-release cluster
                       (default: 55434).
  --self-test          Exercise stage construction, secret safety, SQL
                       markers, and command wiring without Docker/PostgreSQL.
  --help               Show this help.

Environment:
  LAGRANGE_DB_VALIDATION_IMAGE  Optional prebuilt deploy/db image. If unset,
                                the pinned deploy/db/Dockerfile is built.
  LAGRANGE_VALIDATION_MIN_FREE_BYTES
                                Minimum free bytes in the disposable data
                                tmpfs (default: 134217728).

Exit status: 0 APPROVED, 1 DENIED, 2 BLOCKED_EXTERNAL.
USAGE
}

die_usage() {
  echo "postgres-integration-validation: $*" >&2
  usage >&2
  exit 2
}

evidence_dir=''
main_port=55432
test_port=55433
full_port=55434
self_test=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --evidence-dir)
      [ "$#" -ge 2 ] || die_usage '--evidence-dir requires a path'
      evidence_dir=$2
      shift 2
      ;;
    --main-port)
      [ "$#" -ge 2 ] || die_usage '--main-port requires a number'
      main_port=$2
      shift 2
      ;;
    --test-port)
      [ "$#" -ge 2 ] || die_usage '--test-port requires a number'
      test_port=$2
      shift 2
      ;;
    --full-port)
      [ "$#" -ge 2 ] || die_usage '--full-port requires a number'
      full_port=$2
      shift 2
      ;;
    --self-test)
      self_test=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      die_usage "unknown option: $1"
      ;;
  esac
done

validate_port() {
  local label=$1 value=$2
  case "$value" in
    ''|*[!0-9]*) die_usage "$label must be numeric" ;;
  esac
  (( value >= 1024 && value <= 65535 )) || die_usage "$label must be between 1024 and 65535"
}

validate_port --main-port "$main_port"
validate_port --test-port "$test_port"
validate_port --full-port "$full_port"
[ "$main_port" != "$test_port" ] || die_usage '--main-port and --test-port must differ'
[ "$main_port" != "$full_port" ] || die_usage '--main-port and --full-port must differ'
[ "$test_port" != "$full_port" ] || die_usage '--test-port and --full-port must differ'

min_free_bytes=${LAGRANGE_VALIDATION_MIN_FREE_BYTES:-134217728}
case "$min_free_bytes" in
  ''|*[!0-9]*) die_usage 'LAGRANGE_VALIDATION_MIN_FREE_BYTES must be numeric' ;;
esac

bootstrap_gate_redacted_marker='BOOTSTRAP_OUTPUT_REDACTED'

bootstrap_gate_write_marker() {
  local destination=$1
  if ! rm -f -- "$destination" 2>/dev/null; then
    return 1
  fi
  if ! printf '%s\n' "$bootstrap_gate_redacted_marker" >"$destination" 2>/dev/null; then
    rm -f -- "$destination" 2>/dev/null || true
    return 1
  fi
  if ! chmod 0600 -- "$destination" 2>/dev/null; then
    rm -f -- "$destination" 2>/dev/null || true
    return 1
  fi
}

bootstrap_record_gate() {
  [ "$#" -eq 4 ] || return 21
  local credential_directory=$1 raw_output=$2 destination=$3 bootstrap_status=$4
  local name credential_path line_count grep_status
  local -a secret_names=(
    postgres_password db_migration_owner_password db_app_password db_worker_password
    db_audit_password db_research_password db_admin_password db_daily_input_admitter_password
  )

  if [[ ! "$bootstrap_status" =~ ^[0-9]+$ ]]; then
    bootstrap_gate_write_marker "$destination" || true
    return 21
  fi
  if ! rm -f -- "$destination" 2>/dev/null; then
    return 21
  fi
  if [[ ! -f "$raw_output" || -L "$raw_output" || ! -r "$raw_output" || ! -s "$raw_output" ]]; then
    bootstrap_gate_write_marker "$destination" || true
    return 21
  fi

  for name in "${secret_names[@]}"; do
    credential_path="$credential_directory/$name"
    if [[ ! -f "$credential_path" || -L "$credential_path" || ! -r "$credential_path" || ! -s "$credential_path" ]]; then
      bootstrap_gate_write_marker "$destination" || true
      return 21
    fi
    if ! line_count=$(wc -l <"$credential_path" 2>/dev/null) || [[ ! "$line_count" =~ ^[[:space:]]*0[[:space:]]*$ ]]; then
      bootstrap_gate_write_marker "$destination" || true
      return 21
    fi
    if grep -Fq -f "$credential_path" "$raw_output" >/dev/null 2>&1; then
      if ! bootstrap_gate_write_marker "$destination"; then
        return 21
      fi
      return 20
    else
      grep_status=$?
      if [ "$grep_status" -ne 1 ]; then
        bootstrap_gate_write_marker "$destination" || true
        return 21
      fi
    fi
  done

  if ! cat -- "$raw_output" >"$destination" 2>/dev/null; then
    rm -f -- "$destination" 2>/dev/null || true
    bootstrap_gate_write_marker "$destination" || true
    return 21
  fi
  if ! chmod 0600 -- "$destination" 2>/dev/null; then
    rm -f -- "$destination" 2>/dev/null || true
    bootstrap_gate_write_marker "$destination" || true
    return 21
  fi

  if [ "$bootstrap_status" = 0 ]; then
    return 0
  fi
  return 10
}

if [ "$self_test" -eq 1 ]; then
  self_workdir=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-pg-validation-self.XXXXXX")
  case "$self_workdir" in
    /tmp/lagrange-pg-validation-self.*) ;;
    *) echo 'postgres-integration-validation: unsafe self-test temp path' >&2; exit 1 ;;
  esac
  self_cleanup() { rm -rf -- "$self_workdir"; }
  trap self_cleanup EXIT HUP INT TERM

  command -v openssl >/dev/null 2>&1 || {
    echo 'PG_VALIDATION_SELF_TEST: openssl is required' >&2
    exit 1
  }
  command -v bash >/dev/null 2>&1 || {
    echo 'PG_VALIDATION_SELF_TEST: bash is required' >&2
    exit 1
  }
  for command_name in cat chmod cmp cp find grep install mkdir mv rm sha256sum sort tail wc; do
    command -v "$command_name" >/dev/null 2>&1 || {
      echo "PG_VALIDATION_SELF_TEST: $command_name is required" >&2
      exit 1
    }
  done
  bash -n "$BASH_SOURCE"

  secret="$self_workdir/secret"
  openssl rand -hex 32 | tr -d '\r\n' >"$secret"
  [ -s "$secret" ] || { echo 'PG_VALIDATION_SELF_TEST: empty generated secret' >&2; exit 1; }
  [ "$(wc -l <"$secret")" -eq 0 ] || {
    echo 'PG_VALIDATION_SELF_TEST: generated secret has a newline' >&2
    exit 1
  }
  if LC_ALL=C grep -Fq $'\r' "$secret"; then
    echo 'PG_VALIDATION_SELF_TEST: generated secret has CR' >&2
    exit 1
  fi
  [[ "$(cat -- "$secret")" =~ ^[0-9a-f]{64}$ ]] || {
    echo 'PG_VALIDATION_SELF_TEST: generated secret is not exactly 64 lowercase hex characters' >&2
    exit 1
  }

  self_migrations="$self_workdir/migrations"
  mkdir -p "$self_migrations"
  for limit in 38 39 40 41 59 60; do
    stage="$self_migrations/$limit"
    mkdir -p "$stage"
    while IFS= read -r migration; do
      name=${migration##*/}
      if [[ "$name" =~ ^([0-9]{4})_ ]]; then
        version=$((10#${BASH_REMATCH[1]}))
        if (( version <= limit )); then
          install -m 0644 -- "$migration" "$stage/$name"
        fi
      fi
    done < <(find "$root/migrations" -maxdepth 1 -type f -name '*.sql' -print | sort)
    expected=$((limit * 2))
    actual=$(find "$stage" -maxdepth 1 -type f -name '*.sql' | wc -l)
    [ "$actual" -eq "$expected" ] || {
      echo "PG_VALIDATION_SELF_TEST: stage $limit has $actual files, expected $expected" >&2
      exit 1
    }
  done

  for required in \
    '0039_auth_audit_outbox.up.sql' \
    '0040_identity_provisioning.up.sql' \
    '0041_paper_settlement_outbox.up.sql'; do
    [ -f "$self_migrations/41/$required" ] \
      || { echo "PG_VALIDATION_SELF_TEST: missing $required" >&2; exit 1; }
  done
  latest_migration_file=$(find "$root/migrations" -maxdepth 1 -type f -name '*.up.sql' -print | sort | tail -n 1)
  [[ "${latest_migration_file##*/}" =~ ^([0-9]{4})_ ]] || {
    echo 'PG_VALIDATION_SELF_TEST: latest migration filename is invalid' >&2
    exit 1
  }
  self_latest_migration=$((10#${BASH_REMATCH[1]}))
  self_full_expected=$((self_latest_migration * 2))
  self_full_actual=$(find "$root/migrations" -maxdepth 1 -type f -name '*.sql' | wc -l)
  [ "$self_full_actual" -eq "$self_full_expected" ] || {
    echo "PG_VALIDATION_SELF_TEST: full source has $self_full_actual files, expected $self_full_expected for latest migration $self_latest_migration" >&2
    exit 1
  }
  for required_file in \
    "$script_dir/preflight-baseline.sql" \
    "$script_dir/preflight-0039.sql" \
    "$script_dir/preflight-catalog.sql" \
    "$script_dir/preflight-0040.sql" \
    "$script_dir/preflight-0041.sql" \
    "$script_dir/preflight-0060.sql" \
    "$script_dir/actual-role-0060.sql" \
    "$script_dir/rollback-0060-guard.sql" \
    "$script_dir/hazards.sql" \
    "$script_dir/rollback-0039-guard.sql" \
    "$script_dir/rollback-0039-postflight.sql" \
    "$script_dir/migration-safety-audit.sh" \
    "$script_dir/EVIDENCE_TEMPLATE.md"; do
    [ -f "$required_file" ] || {
      echo "PG_VALIDATION_SELF_TEST: missing required file $required_file" >&2
      exit 1
    }
  done
  grep -Fq 'SKIP:' "$BASH_SOURCE" || {
    echo 'PG_VALIDATION_SELF_TEST: no explicit SKIP guard marker' >&2
    exit 1
  }
  grep -Fq 'down -v --remove-orphans' "$BASH_SOURCE" || {
    echo 'PG_VALIDATION_SELF_TEST: teardown marker missing' >&2
    exit 1
  }
  grep -Fq 'printf '\''%s\n'\'' "$tool_block"' "$BASH_SOURCE" || {
    echo 'PG_VALIDATION_SELF_TEST: Compose service separator marker missing' >&2
    exit 1
  }
  grep -Fq 'LAGRANGE_CODE_COMMIT: ${validation_code_commit}' "$BASH_SOURCE" || {
    echo 'PG_VALIDATION_SELF_TEST: db-tool revision build argument is missing' >&2
    exit 1
  }
  grep -Fq 'run_schema_only_stage38' "$BASH_SOURCE" || {
    echo 'PG_VALIDATION_SELF_TEST: schema-only stage38 path is missing' >&2
    exit 1
  }
  grep -Fq -- '--entrypoint /bin/bash' "$BASH_SOURCE" || {
    echo 'PG_VALIDATION_SELF_TEST: stage38 entrypoint override is missing' >&2
    exit 1
  }
  grep -Fq '[[ "$password" =~ ^[0-9a-f]{64}$ ]]' "$BASH_SOURCE" || {
    echo 'PG_VALIDATION_SELF_TEST: stage38 exact generated-password check is missing' >&2
    exit 1
  }
  grep -Fq 'run_full_release_lane' "$BASH_SOURCE" || {
    echo 'PG_VALIDATION_SELF_TEST: separate full-release lane is missing' >&2
    exit 1
  }
  grep -Fq 'run_catalog_negative_probe' "$BASH_SOURCE" || {
    echo 'PG_VALIDATION_SELF_TEST: catalog lane-boundary negative probes are missing' >&2
    exit 1
  }
  grep -Fq 'run_terminal_paper_check' "$BASH_SOURCE" || {
    echo 'PG_VALIDATION_SELF_TEST: terminal Paper RLS-aware preflight is missing' >&2
    exit 1
  }
  grep -Fq 'full_cleanup=PASS_NO_OWNED_CONTAINERS' "$BASH_SOURCE" || {
    echo 'PG_VALIDATION_SELF_TEST: full-release teardown verification is missing' >&2
    exit 1
  }
  grep -Fq 'run_migration_rerun_0039' "$BASH_SOURCE" || {
    echo 'PG_VALIDATION_SELF_TEST: staged catalog rerun check is missing' >&2
    exit 1
  }

  bootstrap_gate_fixture="$self_workdir/bootstrap-gate"
  bootstrap_gate_credentials="$bootstrap_gate_fixture/credentials"
  bootstrap_gate_destination="$bootstrap_gate_fixture/evidence/bootstrap.log"
  bootstrap_gate_raw="$bootstrap_gate_fixture/bootstrap.raw"
  bootstrap_gate_diagnostic="$bootstrap_gate_fixture/gate.diagnostic"
  bootstrap_gate_expected_marker="$bootstrap_gate_fixture/redacted.expected"
  mkdir -p "$bootstrap_gate_credentials" "${bootstrap_gate_destination%/*}"
  bootstrap_gate_names=(
    postgres_password db_migration_owner_password db_app_password db_worker_password
    db_audit_password db_research_password db_admin_password db_daily_input_admitter_password
  )
  bootstrap_gate_sentinels=()
  for bootstrap_gate_index in "${!bootstrap_gate_names[@]}"; do
    printf -v bootstrap_gate_sentinel 'synthetic-r2-bootstrap-sentinel-%02d' "$bootstrap_gate_index"
    bootstrap_gate_sentinels+=("$bootstrap_gate_sentinel")
    printf '%s' "$bootstrap_gate_sentinel" >"$bootstrap_gate_credentials/${bootstrap_gate_names[$bootstrap_gate_index]}"
  done
  printf '%s\n' "$bootstrap_gate_redacted_marker" >"$bootstrap_gate_expected_marker"

  bootstrap_gate_expect_case() {
    local expected_status=$1 credential_directory=$2 raw_path=$3 destination=$4 bootstrap_exit=$5 marker_expected=$6
    local observed_status name credential_path grep_status
    if bootstrap_record_gate "$credential_directory" "$raw_path" "$destination" "$bootstrap_exit" \
      >"$bootstrap_gate_diagnostic" 2>&1; then
      observed_status=0
    else
      observed_status=$?
    fi
    [ "$observed_status" -eq "$expected_status" ] || return 1
    [ ! -s "$bootstrap_gate_diagnostic" ] || return 1
    if [ "$marker_expected" = yes ]; then
      cmp -s -- "$bootstrap_gate_expected_marker" "$destination" || return 1
    else
      cmp -s -- "$raw_path" "$destination" || return 1
    fi
    for name in "${bootstrap_gate_names[@]}"; do
      credential_path="$credential_directory/$name"
      [[ -f "$credential_path" && -s "$credential_path" ]] || continue
      if grep -Fq -f "$credential_path" "$bootstrap_gate_diagnostic" "$destination" 2>/dev/null; then
        return 1
      else
        grep_status=$?
        [ "$grep_status" -eq 1 ] || return 1
      fi
    done
  }

  for bootstrap_gate_index in "${!bootstrap_gate_names[@]}"; do
    bootstrap_gate_sentinel=${bootstrap_gate_sentinels[$bootstrap_gate_index]}
    for bootstrap_gate_form in url env sql; do
      case "$bootstrap_gate_form" in
        url)
          printf 'CONTEXT: postgres://synthetic@fixture/db?password=%s\n' "$bootstrap_gate_sentinel" >"$bootstrap_gate_raw"
          ;;
        env)
          printf 'DB_DAILY_INPUT_ADMITTER_PASSWORD=%s\n' "$bootstrap_gate_sentinel" >"$bootstrap_gate_raw"
          ;;
        sql)
          printf "ERROR: ALTER ROLE daily_input_admitter PASSWORD '%s';\n" "$bootstrap_gate_sentinel" >"$bootstrap_gate_raw"
          ;;
      esac
      for bootstrap_gate_exit in 0 7; do
        if ! bootstrap_gate_expect_case 20 "$bootstrap_gate_credentials" "$bootstrap_gate_raw" \
          "$bootstrap_gate_destination" "$bootstrap_gate_exit" yes; then
          echo 'PG_VALIDATION_SELF_TEST: bootstrap gate disclosure case failed' >&2
          exit 1
        fi
      done
    done
  done

  printf '%s\n' 'role bootstrap completed without credentials' >"$bootstrap_gate_raw"
  if ! bootstrap_gate_expect_case 0 "$bootstrap_gate_credentials" "$bootstrap_gate_raw" \
    "$bootstrap_gate_destination" 0 no; then
    echo 'PG_VALIDATION_SELF_TEST: clean bootstrap success gate case failed' >&2
    exit 1
  fi
  printf '%s\n' 'role bootstrap failed safely with exit 7' >"$bootstrap_gate_raw"
  if ! bootstrap_gate_expect_case 10 "$bootstrap_gate_credentials" "$bootstrap_gate_raw" \
    "$bootstrap_gate_destination" 7 no; then
    echo 'PG_VALIDATION_SELF_TEST: clean bootstrap failure gate case failed' >&2
    exit 1
  fi

  bootstrap_gate_missing_credentials="$bootstrap_gate_fixture/missing-credentials"
  bootstrap_gate_unreadable_credentials="$bootstrap_gate_fixture/unreadable-credentials"
  bootstrap_gate_empty_credentials="$bootstrap_gate_fixture/empty-credentials"
  mkdir -p "$bootstrap_gate_missing_credentials" "$bootstrap_gate_unreadable_credentials" "$bootstrap_gate_empty_credentials"
  for bootstrap_gate_name in "${bootstrap_gate_names[@]:0:7}"; do
    cp -- "$bootstrap_gate_credentials/$bootstrap_gate_name" "$bootstrap_gate_missing_credentials/$bootstrap_gate_name"
    cp -- "$bootstrap_gate_credentials/$bootstrap_gate_name" "$bootstrap_gate_unreadable_credentials/$bootstrap_gate_name"
    cp -- "$bootstrap_gate_credentials/$bootstrap_gate_name" "$bootstrap_gate_empty_credentials/$bootstrap_gate_name"
  done
  mkdir "$bootstrap_gate_unreadable_credentials/db_daily_input_admitter_password"
  : >"$bootstrap_gate_empty_credentials/db_daily_input_admitter_password"
  printf '%s\n' 'synthetic bootstrap output for scan-error cases' >"$bootstrap_gate_raw"
  if ! bootstrap_gate_expect_case 21 "$bootstrap_gate_missing_credentials" "$bootstrap_gate_raw" \
    "$bootstrap_gate_destination" 0 yes; then
    echo 'PG_VALIDATION_SELF_TEST: missing credential scan input was not rejected safely' >&2
    exit 1
  fi
  if ! bootstrap_gate_expect_case 21 "$bootstrap_gate_unreadable_credentials" "$bootstrap_gate_raw" \
    "$bootstrap_gate_destination" 7 yes; then
    echo 'PG_VALIDATION_SELF_TEST: unreadable credential scan input was not rejected safely' >&2
    exit 1
  fi
  if ! bootstrap_gate_expect_case 21 "$bootstrap_gate_empty_credentials" "$bootstrap_gate_raw" \
    "$bootstrap_gate_destination" 0 yes; then
    echo 'PG_VALIDATION_SELF_TEST: empty credential scan input was not rejected safely' >&2
    exit 1
  fi
  if ! bootstrap_gate_expect_case 21 "$bootstrap_gate_credentials" "$bootstrap_gate_fixture/missing.raw" \
    "$bootstrap_gate_destination" 0 yes; then
    echo 'PG_VALIDATION_SELF_TEST: missing bootstrap log was not rejected safely' >&2
    exit 1
  fi
  : >"$bootstrap_gate_fixture/empty.raw"
  if ! bootstrap_gate_expect_case 21 "$bootstrap_gate_credentials" "$bootstrap_gate_fixture/empty.raw" \
    "$bootstrap_gate_destination" 0 yes; then
    echo 'PG_VALIDATION_SELF_TEST: empty bootstrap log was not rejected safely' >&2
    exit 1
  fi
  mkdir "$bootstrap_gate_fixture/unreadable.raw"
  if ! bootstrap_gate_expect_case 21 "$bootstrap_gate_credentials" "$bootstrap_gate_fixture/unreadable.raw" \
    "$bootstrap_gate_destination" 7 yes; then
    echo 'PG_VALIDATION_SELF_TEST: unreadable bootstrap log was not rejected safely' >&2
    exit 1
  fi

  echo 'PG_VALIDATION_SELF_TEST: PASS'
  exit 0
fi

for command_name in docker openssl mktemp awk sed grep find install date bash cat chmod cp rm tr wc cargo git sha256sum sort tail; do
  command -v "$command_name" >/dev/null 2>&1 || {
    echo "postgres-integration-validation: required command is missing: $command_name" >&2
    exit 2
  }
done
[ -f "$qa_compose" ] || { echo "postgres-integration-validation: missing $qa_compose" >&2; exit 1; }
[ -x "$role_script" ] || { echo "postgres-integration-validation: missing $role_script" >&2; exit 1; }

validation_code_commit=$(git -C "$root" rev-parse --verify HEAD 2>/dev/null || true)
if [[ ! "$validation_code_commit" =~ ^[0-9a-f]{40}$ ]] \
  || [ "$validation_code_commit" = 0000000000000000000000000000000000000000 ]; then
  echo 'postgres-integration-validation: repository HEAD is not an exact nonzero 40-hex commit' >&2
  exit 2
fi

if [ -n "$evidence_dir" ]; then
  [ ! -L "$evidence_dir" ] || {
    echo 'postgres-integration-validation: evidence directory must not be a symlink' >&2
    exit 1
  }
  mkdir -p -- "$evidence_dir"
else
  evidence_dir=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-pg-validation-evidence.XXXXXX")
fi
chmod 0700 -- "$evidence_dir"

workdir=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-pg-validation.XXXXXX")
case "$workdir" in
  /tmp/lagrange-pg-validation.*) ;;
  *) echo 'postgres-integration-validation: unsafe temp path' >&2; exit 1 ;;
esac
chmod 0700 -- "$workdir"

outcome=NOT_RUN
detail='workflow did not reach a terminal verdict'
started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
finished_at=''
main_started=0
test_started=0
full_started=0
cleaned=0
docker_ready=0
stages_run=''
tests_run=''
daily_harness='NOT_RUN'
db_test_results=()
daily_named_rows=()
daily_subcase_rows=()
bootstrap_negative_cases=''
service_logins=''
full_service_logins=''
rollback_guard='NOT_RUN'
catalog_install='NOT_RUN'
catalog_conflict='NOT_RUN'
catalog_negative_probes=''
catalog_rerun='NOT_RUN'
full_release_rerun='NOT_RUN'
full_cleanup='NOT_RUN'
full_migration_count_first=''
full_latest_migration_first=''
full_migration_checksums_first=''
full_migration_count_second=''
full_latest_migration_second=''
full_migration_checksums_second=''
skip_guard='ENFORCED'
catalog_sha256_line=$(sha256sum -- "$root/configs/strategies/baseline-v1.json")
catalog_sha256=${catalog_sha256_line%% *}
[[ "$catalog_sha256" =~ ^[0-9a-f]{64}$ ]] || {
  echo 'postgres-integration-validation: baseline catalog hash is invalid' >&2
  exit 2
}
latest_migration_file=$(find "$root/migrations" -maxdepth 1 -type f -name '*.up.sql' -print | sort | tail -n 1)
[[ "${latest_migration_file##*/}" =~ ^([0-9]{4})_ ]] || {
  echo 'postgres-integration-validation: latest migration filename is invalid' >&2
  exit 2
}
latest_migration_version=$((10#${BASH_REMATCH[1]}))

json_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g; s/\r//g; s/\n/\\n/g'
}

append_csv() {
  local current=$1 value=$2
  if [ -n "$current" ]; then
    printf '%s,%s' "$current" "$value"
  else
    printf '%s' "$value"
  fi
}

sanitize_log() {
  local source=$1 destination=$2
  # Credentials are generated as hex, so URL redaction cannot terminate on a
  # password metacharacter. This also removes accidental DATABASE_URL,
  # PGPASSWORD, or password-file values from tool diagnostics.
  sed -E \
    -e "s#(postgres(ql)?://)[^[:space:]\"'<>]+#\\1<redacted>#g" \
    -e "s#(DATABASE_URL=)[^[:space:]\"'<>]+#\\1<redacted>#g" \
    -e "s#(PGPASSWORD=)[^[:space:]\"'<>]+#\\1<redacted>#g" \
    "$source" >"$destination" || true
  chmod 0600 -- "$destination"
}

record_log() {
  local source=$1 name=$2
  sanitize_log "$source" "$evidence_dir/$name"
}

write_evidence() {
  finished_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  local escaped_detail escaped_started escaped_finished catalog_probe_evidence
  escaped_detail=$(json_escape "$detail")
  escaped_started=$(json_escape "$started_at")
  escaped_finished=$(json_escape "$finished_at")
  catalog_probe_evidence=${catalog_negative_probes:-NOT_RUN}
  {
    printf '{\n'
    printf '  "workflow": "postgres-integration-validation",\n'
    printf '  "verdict": "%s",\n' "$outcome"
    printf '  "detail": "%s",\n' "$escaped_detail"
    printf '  "started_at": "%s",\n' "$escaped_started"
    printf '  "finished_at": "%s",\n' "$escaped_finished"
    printf '  "code_commit": "%s",\n' "$validation_code_commit"
    printf '  "postgres_image": "postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a",\n'
    printf '  "upgrade": {"baseline": "0038_schema_only", "sequential": ["0039", "0040", "0041", "0059", "0060"], "catalog_sha256": "%s"},\n' "$catalog_sha256"
    printf '  "full_release": {"first_run": {"migration_count": "%s", "latest_version": "%s", "migration_checksums": "%s"}, "second_run": {"migration_count": "%s", "latest_version": "%s", "migration_checksums": "%s"}, "rerun": "%s"},\n' \
      "$(json_escape "$full_migration_count_first")" "$(json_escape "$full_latest_migration_first")" \
      "$(json_escape "$full_migration_checksums_first")" "$(json_escape "$full_migration_count_second")" \
      "$(json_escape "$full_latest_migration_second")" "$(json_escape "$full_migration_checksums_second")" "$full_release_rerun"
    printf '  "catalog_checks": {"initial_install": "%s", "conflict_probe": "%s", "negative_probes": "%s", "staged_rerun_preserved_identity": "%s"},\n' \
      "$catalog_install" "$catalog_conflict" "$catalog_probe_evidence" "$catalog_rerun"
    printf '  "full_release_cleanup": "%s",\n' "$full_cleanup"
    printf '  "ports": {"upgrade": %s, "tests": %s, "full_release": %s},\n' "$main_port" "$test_port" "$full_port"
    printf '  "credentials": "upgrade values generated in a private temporary directory; test lane uses the existing synthetic fixture; values omitted",\n'
    printf '  "stages_run": ['
    first=1
    IFS=',' read -r -a stage_items <<<"$stages_run"
    for stage_item in "${stage_items[@]}"; do
      [ -n "$stage_item" ] || continue
      [ "$first" -eq 1 ] || printf ', '
      printf '"%s"' "$stage_item"
      first=0
    done
    printf '],\n'
    printf '  "checks": {"direct_service_role_logins": "%s", "full_release_service_role_logins": "%s", "0039_down_undelivered_guard": "%s", "skip_markers": "%s"},\n' \
      "$(json_escape "$service_logins")" "$(json_escape "$full_service_logins")" "$rollback_guard" "$skip_guard"
    printf '  "daily_0060_harness": "%s",\n' "$daily_harness"
    printf '  "db_test_results": ['
    first=1
    for row in "${db_test_results[@]}"; do
      [ "$first" -eq 1 ] || printf ', '
      printf '%s' "$row"; first=0
    done
    printf '],\n'
    printf '  "daily_0060_named_rows": ['
    first=1
    for row in "${daily_named_rows[@]}"; do
      [ "$first" -eq 1 ] || printf ', '
      printf '"%s"' "$row"; first=0
    done
    printf '],\n'
    printf '  "daily_0060_subcase_rows": ['
    first=1
    for row in "${daily_subcase_rows[@]}"; do
      [ "$first" -eq 1 ] || printf ', '
      printf '"%s"' "$row"; first=0
    done
    printf '],\n'
    printf '  "tests_run": ['
    first=1
    IFS=',' read -r -a test_items <<<"$tests_run"
    for test_item in "${test_items[@]}"; do
      [ -n "$test_item" ] || continue
      [ "$first" -eq 1 ] || printf ', '
      printf '"%s"' "$(json_escape "$test_item")"
      first=0
    done
    printf '],\n'
    printf '  "bootstrap_negative_cases": ['
    first=1
    IFS=',' read -r -a bootstrap_items <<<"$bootstrap_negative_cases"
    for bootstrap_item in "${bootstrap_items[@]}"; do
      [ -n "$bootstrap_item" ] || continue
      [ "$first" -eq 1 ] || printf ', '
      printf '"%s"' "$bootstrap_item"
      first=0
    done
    printf '],\n'
    printf '  "sanitized_evidence_dir": "%s"\n' "$(json_escape "$evidence_dir")"
    printf '}\n'
  } >"$evidence_dir/evidence.json"
  chmod 0600 -- "$evidence_dir/evidence.json"
  {
    printf 'verdict\t%s\n' "$outcome"
    printf 'detail\t%s\n' "$detail"
    printf 'code_commit\t%s\n' "$validation_code_commit"
    printf 'baseline\t0038\n'
    printf 'sequential\t0039,0040,0041,0059,0060\n'
    printf 'stages_run\t%s\n' "$stages_run"
    printf 'tests_run\t%s\n' "$tests_run"
    printf 'bootstrap_negative_cases\t%s\n' "$bootstrap_negative_cases"
    printf 'direct_service_role_logins\t%s\n' "$service_logins"
    printf 'full_release_service_role_logins\t%s\n' "$full_service_logins"
    printf '0039_down_undelivered_guard\t%s\n' "$rollback_guard"
    printf 'catalog_sha256\t%s\n' "$catalog_sha256"
    printf 'catalog_install\t%s\n' "$catalog_install"
    printf 'catalog_conflict_probe\t%s\n' "$catalog_conflict"
    printf 'catalog_negative_probes\t%s\n' "$catalog_probe_evidence"
    printf 'catalog_rerun\t%s\n' "$catalog_rerun"
    printf 'full_release_cleanup\t%s\n' "$full_cleanup"
    printf 'full_release_rerun\t%s\n' "$full_release_rerun"
    printf 'full_release_first_migration_count\t%s\n' "$full_migration_count_first"
    printf 'full_release_first_latest_migration\t%s\n' "$full_latest_migration_first"
    printf 'full_release_first_migration_checksums\t%s\n' "$full_migration_checksums_first"
    printf 'full_release_second_migration_count\t%s\n' "$full_migration_count_second"
    printf 'full_release_second_latest_migration\t%s\n' "$full_latest_migration_second"
    printf 'full_release_second_migration_checksums\t%s\n' "$full_migration_checksums_second"
    printf 'skip_markers\t%s\n' "$skip_guard"
    printf 'upgrade_port\t%s\n' "$main_port"
    printf 'test_port\t%s\n' "$test_port"
    printf 'full_release_port\t%s\n' "$full_port"
  } >"$evidence_dir/evidence.tsv"
  chmod 0600 -- "$evidence_dir/evidence.tsv"
}

cleanup() {
  local status=$?
  [ "$cleaned" -eq 1 ] && exit "$status"
  cleaned=1
  set +e
  if [ "$docker_ready" -eq 1 ]; then
    if [ "$main_started" -eq 1 ]; then
      main_compose down -v --remove-orphans >"$workdir/cleanup-main.log" 2>&1
      record_log "$workdir/cleanup-main.log" cleanup-main.log
    fi
    if [ "$full_started" -eq 1 ]; then
      full_compose down -v --remove-orphans >"$workdir/cleanup-full.log" 2>&1
      record_log "$workdir/cleanup-full.log" cleanup-full.log
    fi
    if [ "$test_started" -eq 1 ]; then
      test_compose down -v --remove-orphans >"$workdir/cleanup-test.log" 2>&1
      record_log "$workdir/cleanup-test.log" cleanup-test.log
    fi
  fi
  if [ "$outcome" = NOT_RUN ]; then
    if [ "$status" -eq 2 ]; then
      outcome=BLOCKED_EXTERNAL
      detail='workflow exited before the database gate completed'
    else
      outcome=DENIED
      detail='workflow exited before the database gate completed'
    fi
  fi
  write_evidence
  rm -rf -- "$workdir"
  echo "postgres-integration-validation: $outcome"
  echo "postgres-integration-validation: sanitized evidence: $evidence_dir"
  exit "$status"
}
trap cleanup EXIT HUP INT TERM

fail() {
  detail=$*
  outcome=DENIED
  echo "postgres-integration-validation: DENIED: $detail" >&2
  exit 1
}

blocked() {
  detail=$*
  outcome=BLOCKED_EXTERNAL
  echo "postgres-integration-validation: BLOCKED_EXTERNAL: $detail" >&2
  exit 2
}

main_compose() {
  LAGRANGE_QA_DB_PORT="$main_port" docker compose -p "$main_project" -f "$qa_compose" -f "$main_override" "$@"
}

full_compose() {
  LAGRANGE_QA_DB_PORT="$full_port" docker compose -p "$full_project" -f "$qa_compose" -f "$full_override" "$@"
}

test_compose() {
  LAGRANGE_QA_DB_PORT="$test_port" docker compose -p "$test_project" -f "$qa_compose" -f "$test_override" "$@"
}

generate_secret() {
  local destination=$1
  openssl rand -hex 32 | tr -d '\r\n' >"$destination" || fail "could not generate temporary credential for ${destination##*/}"
  chmod 0400 -- "$destination"
  [ -s "$destination" ] || fail "generated credential is empty: ${destination##*/}"
  [ "$(wc -l <"$destination")" -eq 0 ] || fail "generated credential contains LF: ${destination##*/}"
  if LC_ALL=C grep -Fq $'\r' "$destination"; then
    fail "generated credential contains CR: ${destination##*/}"
  fi
  credential=$(cat -- "$destination")
  [[ "$credential" =~ ^[0-9a-f]{64}$ ]] || fail "generated credential has an invalid format: ${destination##*/}"
}

make_stages() {
  migration_root="$workdir/migrations"
  mkdir -p "$migration_root"
  for limit in 38 39 40 41 59 60; do
    stage="$migration_root/$limit"
    mkdir -p "$stage"
    while IFS= read -r migration; do
      name=${migration##*/}
      if [[ "$name" =~ ^([0-9]{4})_ ]]; then
        version=$((10#${BASH_REMATCH[1]}))
        if (( version <= limit )); then
          install -m 0644 -- "$migration" "$stage/$name"
        fi
      fi
    done < <(find "$root/migrations" -maxdepth 1 -type f -name '*.sql' -print | sort)
    expected=$((limit * 2))
    actual=$(find "$stage" -maxdepth 1 -type f -name '*.sql' | wc -l)
    [ "$actual" -eq "$expected" ] || fail "migration source stage $limit has $actual files; expected $expected"
  done
  chmod 0755 -- "$migration_root" "$migration_root"/*
}

make_override() {
  local destination=$1 password_dir=$2 port=$3 include_tool=$4
  local migration_mount=${5:-1} tool_migrations_dir=${6:-/validation/migrations/41}
  local tool_block=''
  if [ "$include_tool" -eq 1 ]; then
    if [ -n "${LAGRANGE_DB_VALIDATION_IMAGE:-}" ]; then
      tool_block=$(cat <<EOF
  db-tool:
    image: ${LAGRANGE_DB_VALIDATION_IMAGE}
    user: "0:0"
    entrypoint: ["/usr/local/bin/lagrange-migrate"]
    environment:
      DB_HOST: qa-db
      DB_PORT: "5432"
      DB_NAME: postgres
      DB_USER: migration_owner
      DB_PASSWORD_FILE: /run/secrets/db_migration_owner_password
      MIGRATIONS_DIR: ${tool_migrations_dir}
    secrets:
      - db_migration_owner_password
    networks:
      - default
EOF
)
    else
      tool_block=$(cat <<EOF
  db-tool:
    image: lagrange-db-validation:${validation_code_commit}
    build:
      context: ${root}
      dockerfile: deploy/db/Dockerfile
      args:
        LAGRANGE_CODE_COMMIT: ${validation_code_commit}
    user: "0:0"
    entrypoint: ["/usr/local/bin/lagrange-migrate"]
    environment:
      DB_HOST: qa-db
      DB_PORT: "5432"
      DB_NAME: postgres
      DB_USER: migration_owner
      DB_PASSWORD_FILE: /run/secrets/db_migration_owner_password
      MIGRATIONS_DIR: ${tool_migrations_dir}
    secrets:
      - db_migration_owner_password
    networks:
      - default
EOF
)
    fi
    if [ "$migration_mount" -eq 1 ]; then
      tool_block+=$'\n    volumes:\n'
      tool_block+="      - ${migration_root}:/validation/migrations:ro"
    fi
  fi
  {
    printf 'services:\n'
    printf '  qa-db:\n'
    printf '    environment:\n'
    printf '      POSTGRES_PASSWORD: null\n'
    printf '      POSTGRES_PASSWORD_FILE: /run/secrets/qa_password\n'
    printf '    ports:\n'
    printf '      - "127.0.0.1:%s:5432"\n' "$port"
    printf '    secrets:\n'
    printf '      - qa_password\n'
    printf '    volumes:\n'
    printf '      - %s:/validation/secrets:ro\n' "$password_dir"
    printf '      - %s:/validation/bootstrap-roles.sh:ro\n' "$role_script"
    printf '      - %s:/validation/bootstrap-negative.sh:ro\n' "$script_dir/bootstrap-negative.sh"
    printf '      - %s:/validation/baseline-v1.json:ro\n' "$root/configs/strategies/baseline-v1.json"
    printf '      - %s:/validation/sync-baseline-strategy-catalog.sql:ro\n' "$root/deploy/db/sync-baseline-strategy-catalog.sql"
    if [ "$include_tool" -eq 1 ] && [ "$migration_mount" -eq 1 ]; then
      printf '      - %s:/validation/migrations:ro\n' "$migration_root"
    fi
    if [ "$include_tool" -eq 1 ]; then
      printf '%s\n' "$tool_block"
    fi
    printf 'secrets:\n'
    printf '  qa_password:\n'
    printf '    file: %s/postgres_password\n' "$password_dir"
    if [ "$include_tool" -eq 1 ]; then
      printf '  db_migration_owner_password:\n'
      printf '    file: %s/db_migration_owner_password\n' "$password_dir"
    fi
  } >"$destination"
  chmod 0600 -- "$destination"
}

bootstrap_roles() {
  local compose_function=$1
  "$compose_function" exec -T -u 0 \
    -e DB_HOST=127.0.0.1 \
    -e DB_PORT=5432 \
    -e DB_NAME=postgres \
    -e DB_ADMIN_USER=postgres \
    -e DB_ADMIN_PASSWORD_FILE=/validation/secrets/postgres_password \
    -e DB_MIGRATION_OWNER_PASSWORD_FILE=/validation/secrets/db_migration_owner_password \
    -e DB_APP_PASSWORD_FILE=/validation/secrets/db_app_password \
    -e DB_WORKER_PASSWORD_FILE=/validation/secrets/db_worker_password \
    -e DB_AUDIT_PASSWORD_FILE=/validation/secrets/db_audit_password \
    -e DB_RESEARCH_PASSWORD_FILE=/validation/secrets/db_research_password \
    -e DB_ADMIN_ROLE_PASSWORD_FILE=/validation/secrets/db_admin_password \
    -e DB_DAILY_INPUT_ADMITTER_PASSWORD_FILE=/validation/secrets/db_daily_input_admitter_password \
    qa-db /bin/bash /validation/bootstrap-roles.sh
}

run_bootstrap_negative_contract() {
  local raw="$workdir/bootstrap-negative.log" row
  # Only this owned, fresh container receives private copies readable by the
  # non-root postgres OS user. Real 0000-file denial must not run as root.
  if ! main_compose exec -T -u 0 qa-db /bin/bash -ec '
    destination=/tmp/lagrange-bootstrap-contract-inputs
    test ! -e "$destination"
    install -d -o postgres -g postgres -m 0700 "$destination"
    for name in postgres_password db_migration_owner_password db_app_password \
      db_worker_password db_audit_password db_research_password db_admin_password \
      db_daily_input_admitter_password; do
      install -o postgres -g postgres -m 0400 "/validation/secrets/$name" "$destination/$name"
    done
  ' >"$raw" 2>&1; then
    fail 'could not prepare private disposable bootstrap-contract inputs'
  fi
  if ! main_compose exec -T -u postgres qa-db timeout 300s /bin/bash \
    /validation/bootstrap-negative.sh --disposable-ci >"$raw" 2>&1; then
    # Retain fixed case IDs only, even if an unexpected tool diagnostic occurs.
    grep -E '^BOOTSTRAP_CONTRACT_(CASE:[a-z0-9_]+:(PASS|FAIL)|FAILURE:[a-z0-9_]+:[a-z0-9_]+)$' "$raw" \
      >"$evidence_dir/bootstrap-negative.log" || true
    chmod 0600 "$evidence_dir/bootstrap-negative.log"
    fail 'actual bootstrap negative/rollback contract failed'
  fi
  grep -Fxq 'BOOTSTRAP_CONTRACT:PASS' "$raw" \
    || fail 'bootstrap contract omitted its final verdict'
  if ! awk '
    /^BOOTSTRAP_CONTRACT_CASE:[a-z0-9_]+:PASS$/ {
      if (seen[$0]++) exit 1
      count++
      next
    }
    /^BOOTSTRAP_CONTRACT:PASS$/ { verdict++; next }
    { exit 1 }
    END { if (count != 83 || verdict != 1) exit 1 }
  ' "$raw"; then
    fail 'bootstrap contract named outcomes are incomplete or malformed'
  fi
  while IFS= read -r row; do
    case "$row" in
      BOOTSTRAP_CONTRACT_CASE:*)
        row=${row#BOOTSTRAP_CONTRACT_CASE:}
        bootstrap_negative_cases=$(append_csv "$bootstrap_negative_cases" "$row")
        ;;
    esac
  done <"$raw"
  record_log "$raw" bootstrap-negative.log
}

run_psql() {
  local compose_function=$1 sql_file=$2
  local -a psql_args=("${@:3}")
  "$compose_function" exec -T -u 0 qa-db /bin/bash -ec \
    'export PGPASSWORD="$(cat /validation/secrets/postgres_password)"; exec psql -X -v ON_ERROR_STOP=1 -P pager=off -At -h 127.0.0.1 -p 5432 -U postgres -d postgres "$@"' \
    psql-wrapper "${psql_args[@]}" <"$sql_file"
}

run_role_psql() {
  local compose_function=$1 role=$2 sql_file=$3
  local secret_name
  local -a psql_args=("${@:4}")
  case "$role" in
    migration_owner) secret_name=db_migration_owner_password ;;
    app) secret_name=db_app_password ;;
    worker) secret_name=db_worker_password ;;
    audit_writer) secret_name=db_audit_password ;;
    research_writer) secret_name=db_research_password ;;
    admin) secret_name=db_admin_password ;;
    daily_input_admitter) secret_name=db_daily_input_admitter_password ;;
    *) return 1 ;;
  esac
  "$compose_function" exec -T -u 0 qa-db /bin/bash -ec \
    "export PGPASSWORD=\"\$(cat /validation/secrets/${secret_name})\"; exec psql -X -v ON_ERROR_STOP=1 -v VERBOSITY=verbose -P pager=off -At -h 127.0.0.1 -p 5432 -U ${role} -d postgres \"\$@\"" \
    psql-wrapper "${psql_args[@]}" <"$sql_file"
}

run_terminal_paper_check() {
  local compose_function=$1 label=$2 raw="$workdir/terminal-paper-$2.log"
  if ! run_psql "$compose_function" "$script_dir/preflight-0039.sql" \
      --set terminal_only=1 --set "catalog_sha256=$catalog_sha256" >"$raw" 2>&1; then
    record_log "$raw" "terminal-paper-$label.log"
    return 1
  fi
  record_log "$raw" "terminal-paper-$label.log"
  grep -Fqx 'terminal_paper_targets=1' "$raw"
}

run_catalog_preflight() {
  local compose_function=$1 conflict_probe=$2 raw=$3 expected_lane=$4 negative_probe=$5
  local probe_unexpected_strategy=0 probe_missing_baseline=0
  local probe_mutated_baseline=0 probe_full_paper=0
  case "$expected_lane" in
    staged|full) ;;
    *) fail 'catalog preflight received an invalid lane expectation' ;;
  esac
  case "$negative_probe" in
    none) ;;
    unexpected_strategy) probe_unexpected_strategy=1 ;;
    missing_baseline) probe_missing_baseline=1 ;;
    mutated_baseline) probe_mutated_baseline=1 ;;
    full_paper) probe_full_paper=1 ;;
    *) fail 'catalog preflight received an invalid negative-probe expectation' ;;
  esac
  case "$negative_probe:$expected_lane" in
    none:staged|none:full|unexpected_strategy:staged|missing_baseline:staged|mutated_baseline:staged|full_paper:full) ;;
    *) fail 'catalog negative probe does not match its disposable lane' ;;
  esac
  "$compose_function" exec -T -u 0 qa-db /bin/bash -ec \
    '
      set -euo pipefail
      export PGPASSWORD="$(cat /validation/secrets/db_migration_owner_password)"
      catalog_json="$(cat /validation/baseline-v1.json)"
      catalog_sha256=$(sha256sum /validation/baseline-v1.json)
      catalog_sha256=${catalog_sha256%% *}
      [[ "$catalog_sha256" =~ ^[0-9a-f]{64}$ ]]
      exec psql -X -v ON_ERROR_STOP=1 -v VERBOSITY=verbose -P pager=off -At \
        -v "catalog_json=$catalog_json" -v "catalog_sha256=$catalog_sha256" \
        -v "expected_lane=$2" -v "conflict_probe=$1" \
        -v "probe_unexpected_strategy=$3" -v "probe_missing_baseline=$4" \
        -v "probe_mutated_baseline=$5" -v "probe_full_paper=$6" \
        -h 127.0.0.1 -p 5432 -U migration_owner -d postgres
    ' validation "$conflict_probe" "$expected_lane" \
      "$probe_unexpected_strategy" "$probe_missing_baseline" \
      "$probe_mutated_baseline" "$probe_full_paper" \
    <"$script_dir/preflight-catalog.sql" >"$raw" 2>&1
}

run_catalog_negative_probe() {
  local compose_function=$1 lane=$2 probe=$3 expected_reason=$4 baseline_snapshot=$5
  local raw="$workdir/catalog-negative-$probe.log"
  local after_raw="$workdir/catalog-after-negative-$probe.log"
  local error_count after_snapshot
  if ! run_catalog_preflight "$compose_function" 0 "$raw" "$lane" "$probe"; then
    record_log "$raw" "catalog-negative-$probe.log"
    fail "catalog negative probe could not complete its disposable transaction: $probe"
  fi
  record_log "$raw" "catalog-negative-$probe.log"
  error_count=$(awk '/^ERROR:/ { count++ } END { print count + 0 }' "$raw")
  [ "$error_count" = 1 ] \
    || fail "catalog negative probe emitted $error_count SQL errors; expected exactly one: $probe"
  if ! grep -Eq "ERROR:[[:space:]]+23514:[[:space:]]+catalog_preflight: $expected_reason([[:space:]]|$)" "$raw"; then
    fail "catalog negative probe did not raise the expected SQLSTATE 23514 reason: $probe"
  fi
  grep -Fqx "CATALOG_NEGATIVE_PROBE_ROLLED_BACK:$probe" "$raw" \
    || fail "catalog negative probe did not explicitly roll back: $probe"
  if ! run_catalog_preflight "$compose_function" 0 "$after_raw" "$lane" none; then
    record_log "$after_raw" "catalog-after-negative-$probe.log"
    fail "catalog negative probe did not restore a valid post-rollback snapshot: $probe"
  fi
  record_log "$after_raw" "catalog-after-negative-$probe.log"
  after_snapshot=$(catalog_snapshot "$after_raw")
  [ "$after_snapshot" = "$baseline_snapshot" ] \
    || fail "catalog negative probe changed rows or audit identity/count: $probe"
  catalog_negative_probes=$(append_csv "$catalog_negative_probes" "$probe:SQLSTATE_23514_ROLLED_BACK")
}

run_full_ledger_report() {
  local compose_function=$1 label=$2
  local query_file="$workdir/full-ledger-$label.sql"
  local raw="$workdir/full-ledger-$label.log"
  local checksum_entry checksum_version checksum_value expected_version
  local -a checksum_items=()
  cat >"$query_file" <<'SQL'
SELECT 'migration_count=' || count(*) FROM public._sqlx_migrations;
SELECT 'latest_migration=' || coalesce(max(version)::text, 'none') FROM public._sqlx_migrations;
SELECT 'migration_checksums=' || coalesce(
    string_agg(version::text || ':' || encode(checksum, 'hex'), ',' ORDER BY version),
    ''
) FROM public._sqlx_migrations;
SQL
  if ! run_psql "$compose_function" "$query_file" >"$raw" 2>&1; then
    record_log "$raw" "full-ledger-$label.log"
    fail "could not inspect full-release migration ledger after $label run"
  fi
  record_log "$raw" "full-ledger-$label.log"
  full_count=$(sed -n 's/^migration_count=//p' "$raw")
  full_latest=$(sed -n 's/^latest_migration=//p' "$raw")
  full_checksums=$(sed -n 's/^migration_checksums=//p' "$raw")
  [[ "$full_count" =~ ^[0-9]+$ ]] || fail "full-release $label migration count is invalid"
  [[ "$full_latest" =~ ^[0-9]+$ ]] || fail "full-release $label latest migration is invalid"
  [[ -n "$full_checksums" ]] || fail "full-release $label migration checksums are missing"
  IFS=',' read -r -a checksum_items <<<"$full_checksums"
  [ "${#checksum_items[@]}" -eq "$full_count" ] \
    || fail "full-release $label checksum count does not match its migration count"
  expected_version=1
  for checksum_entry in "${checksum_items[@]}"; do
    [[ "$checksum_entry" =~ ^([0-9]+):([0-9a-f]+)$ ]] \
      || fail "full-release $label contains a malformed migration checksum"
    checksum_version=${BASH_REMATCH[1]}
    checksum_value=${BASH_REMATCH[2]}
    [ "$checksum_version" = "$expected_version" ] \
      || fail "full-release $label migration checksums are not a contiguous ordered ledger"
    (( ${#checksum_value} % 2 == 0 )) \
      || fail "full-release $label contains an odd-length migration checksum"
    expected_version=$((expected_version + 1))
  done
}

run_service_login() {
  local compose_function=$1 role=$2 raw=$3
  local secret_name
  case "$role" in
    migration_owner) secret_name=db_migration_owner_password ;;
    app) secret_name=db_app_password ;;
    worker) secret_name=db_worker_password ;;
    audit_writer) secret_name=db_audit_password ;;
    research_writer) secret_name=db_research_password ;;
    admin) secret_name=db_admin_password ;;
    daily_input_admitter) secret_name=db_daily_input_admitter_password ;;
    *) return 1 ;;
  esac
  "$compose_function" exec -T -u 0 qa-db /bin/bash -ec \
    "export PGPASSWORD=\"\$(cat /validation/secrets/${secret_name})\"; exec psql -X -v ON_ERROR_STOP=1 -P pager=off -At -h 127.0.0.1 -p 5432 -U ${role} -d postgres" \
    <"$script_dir/service-login.sql" >"$raw" 2>&1
  expected="$role=$role"
  observed=$(tr -d '\r' <"$raw" | tail -n 1)
  [ "$observed" = "$expected" ]
}

run_admitter_catalog_assertions() {
  local compose_function=$1 raw=$2 sql_file="$workdir/daily-input-admitter-catalog.sql"
  cat >"$sql_file" <<'SQL'
DO $daily_input_admitter_catalog$
BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM pg_roles
    WHERE rolname = 'daily_input_admitter'
      AND rolcanlogin
      AND NOT rolinherit
      AND NOT rolsuper
      AND NOT rolcreatedb
      AND NOT rolcreaterole
      AND NOT rolreplication
      AND NOT rolbypassrls
  ) THEN
    RAISE EXCEPTION 'daily_input_admitter role attributes mismatch';
  END IF;
  IF EXISTS (
    SELECT 1
    FROM pg_auth_members
    WHERE member = (SELECT oid FROM pg_roles WHERE rolname = 'daily_input_admitter')
       OR roleid = (SELECT oid FROM pg_roles WHERE rolname = 'daily_input_admitter')
  ) THEN
    RAISE EXCEPTION 'daily_input_admitter has unexpected role membership';
  END IF;
  IF EXISTS (
    WITH RECURSIVE membership_closure(start_role, reachable_role) AS (
      SELECT oid, oid
      FROM pg_roles
      WHERE rolname IN ('app', 'worker')
      UNION
      SELECT closure.start_role, membership.roleid
      FROM membership_closure AS closure
      JOIN pg_auth_members AS membership ON membership.member = closure.reachable_role
    )
    SELECT 1
    FROM membership_closure AS closure
    JOIN pg_roles AS start_role ON start_role.oid = closure.start_role
    JOIN pg_roles AS reachable_role ON reachable_role.oid = closure.reachable_role
    WHERE start_role.rolname IN ('app', 'worker')
      AND reachable_role.rolname IN ('migration_owner', 'daily_input_admitter')
  ) THEN
    RAISE EXCEPTION 'app or worker reaches a protected migration role';
  END IF;
  IF NOT has_database_privilege('daily_input_admitter', current_database(), 'CONNECT') THEN
    RAISE EXCEPTION 'daily_input_admitter lacks database CONNECT';
  END IF;
  IF NOT has_schema_privilege('daily_input_admitter', 'public', 'USAGE') THEN
    RAISE EXCEPTION 'daily_input_admitter lacks public schema USAGE';
  END IF;
  IF has_database_privilege('daily_input_admitter', current_database(), 'CREATE') THEN
    RAISE EXCEPTION 'daily_input_admitter has database CREATE';
  END IF;
  IF EXISTS (
    SELECT 1
    FROM pg_namespace AS ns
    WHERE has_schema_privilege('daily_input_admitter'::name, ns.oid, 'CREATE')
  ) THEN
    RAISE EXCEPTION 'daily_input_admitter has schema CREATE';
  END IF;
  IF NOT has_schema_privilege('migration_owner', 'public', 'CREATE') THEN
    RAISE EXCEPTION 'migration_owner lacks public schema CREATE';
  END IF;
END
$daily_input_admitter_catalog$;
SELECT 'daily_input_admitter_catalog=PASS';
SQL
  if ! run_psql "$compose_function" "$sql_file" >"$raw" 2>&1; then
    return 1
  fi
  grep -Fqx 'daily_input_admitter_catalog=PASS' "$raw"
}

run_admitter_role_guc_check() {
  local compose_function=$1 raw=$2 sql_file="$workdir/daily-input-admitter-role-guc.sql"
  cat >"$sql_file" <<'SQL'
SELECT current_user || '=' || session_user || '=' || current_setting('role');
SQL
  if ! run_role_psql "$compose_function" daily_input_admitter "$sql_file" >"$raw" 2>&1; then
    return 1
  fi
  grep -Fqx 'daily_input_admitter=daily_input_admitter=none' "$raw"
}

assert_bootstrap_log_redacts_credentials() {
  local password_dir=$1 raw=$2 label=$3 name
  local -a secret_names=(
    postgres_password db_migration_owner_password db_app_password db_worker_password
    db_audit_password db_research_password db_admin_password db_daily_input_admitter_password
  )
  for name in "${secret_names[@]}"; do
    if grep -Fq -f "$password_dir/$name" "$raw"; then
      fail "$label role bootstrap output exposed a credential value ($name)"
    fi
  done
}

bootstrap_gate_output_is_marker() {
  local output=$1 marker
  [[ -f "$output" && ! -L "$output" && -s "$output" ]] || return 1
  marker=$(cat -- "$output" 2>/dev/null) || return 1
  [ "$marker" = "$bootstrap_gate_redacted_marker" ]
}

record_bootstrap_result() {
  local password_dir=$1 raw=$2 evidence_name=$3 label=$4 bootstrap_status=$5
  local gated_output="$workdir/gated-$evidence_name" gate_status

  if bootstrap_record_gate "$password_dir" "$raw" "$gated_output" "$bootstrap_status"; then
    gate_status=0
  else
    gate_status=$?
  fi

  case "$gate_status" in
    0)
      assert_bootstrap_log_redacts_credentials "$password_dir" "$gated_output" "$label"
      record_log "$gated_output" "$evidence_name"
      ;;
    10)
      assert_bootstrap_log_redacts_credentials "$password_dir" "$gated_output" "$label"
      record_log "$gated_output" "$evidence_name"
      fail "$label role bootstrap failed (bootstrap exit=$bootstrap_status)"
      ;;
    20)
      if bootstrap_gate_output_is_marker "$gated_output"; then
        record_log "$gated_output" "$evidence_name"
      fi
      fail "$label role bootstrap output disclosed a credential; evidence redacted (bootstrap exit=$bootstrap_status)"
      ;;
    21)
      if bootstrap_gate_output_is_marker "$gated_output"; then
        record_log "$gated_output" "$evidence_name"
      fi
      fail "$label role bootstrap credential scan failed; evidence redacted (bootstrap exit=$bootstrap_status)"
      ;;
    *)
      fail "$label role bootstrap credential gate returned an unknown category (gate=$gate_status bootstrap exit=$bootstrap_status)"
      ;;
  esac
}

run_app_actor_sql() {
  local compose_function=$1 raw=$2
  "$compose_function" exec -T -u 0 qa-db /bin/bash -ec \
    'export PGPASSWORD="$(cat /validation/secrets/db_app_password)"; exec psql -X -v ON_ERROR_STOP=1 -P pager=off -At -h 127.0.0.1 -p 5432 -U app -d postgres' \
    <"$script_dir/identity-boundary.sql" >"$raw" 2>&1
}

run_disk_check() {
  local compose_function=$1 raw=$2
  "$compose_function" exec -T -u 0 qa-db /bin/bash -ec \
    'df -Pk /var/lib/postgresql | awk "NR == 2 { print \$4 * 1024 }"' >"$raw" 2>&1 || return 1
  free_bytes=$(tr -d '\r' <"$raw" | tail -n 1)
  case "$free_bytes" in
    ''|*[!0-9]*) return 1 ;;
  esac
  (( free_bytes >= min_free_bytes ))
}

verify_migration_count() {
  local compose_function=$1 expected=$2 label=$3
  local count_sql="$workdir/check-version-$label.sql"
  local count_raw="$workdir/version-$label.log"
  printf 'SELECT count(*) FROM public._sqlx_migrations;\n' >"$count_sql"
  if ! run_psql "$compose_function" "$count_sql" >"$count_raw" 2>&1; then
    record_log "$count_raw" "version-$label.log"
    fail "could not verify migration stage $label"
  fi
  record_log "$count_raw" "version-$label.log"
  observed=$(tr -d '\r' <"$count_raw" | tail -n 1)
  [ "$observed" = "$expected" ] || fail "migration stage $label recorded $observed applied migrations; expected $expected"
}

run_schema_only_stage38() {
  local raw="$workdir/migrate-38-schema-only.log"
  if ! main_compose run --rm --no-deps --entrypoint /bin/bash db-tool -ec '
    set -euo pipefail
    password_file=/run/secrets/db_migration_owner_password
    test -f "$password_file"
    password=$(cat -- "$password_file")
    [[ "$password" =~ ^[0-9a-f]{64}$ ]]
    export DATABASE_URL="postgres://migration_owner:${password}@qa-db:5432/postgres"
    export PGOPTIONS="-c lock_timeout=5s -c statement_timeout=60s"
    unset password
    exec sqlx migrate run --source /validation/migrations/38
  ' >"$raw" 2>&1; then
    record_log "$raw" migrate-38-schema-only.log
    fail 'schema-only migration stage 38 failed'
  fi
  record_log "$raw" migrate-38-schema-only.log
  verify_migration_count main_compose 38 38
  stages_run=$(append_csv "$stages_run" '0038-schema-only')
}

run_migration_stage() {
  local version=$1 raw="$workdir/migrate-$1.log"
  if ! main_compose run --rm --no-deps -e "MIGRATIONS_DIR=/validation/migrations/$version" db-tool >"$raw" 2>&1; then
    record_log "$raw" "migrate-$version.log"
    fail "migration stage $version failed"
  fi
  record_log "$raw" "migrate-$version.log"
  verify_migration_count main_compose "$version" "$version"
  stages_run=$(append_csv "$stages_run" "$version")
}

run_migration_rerun_0039() {
  local raw="$workdir/migrate-0039-rerun.log"
  if ! main_compose run --rm --no-deps -e 'MIGRATIONS_DIR=/validation/migrations/39' db-tool >"$raw" 2>&1; then
    record_log "$raw" migrate-0039-rerun.log
    fail '0039 catalog sync rerun was not a no-op'
  fi
  record_log "$raw" migrate-0039-rerun.log
  verify_migration_count main_compose 39 0039-rerun
  stages_run=$(append_csv "$stages_run" '0039-rerun')
}

run_migration_rerun_0041() {
  raw="$workdir/migrate-0041-rerun.log"
  if ! main_compose run --rm --no-deps -e 'MIGRATIONS_DIR=/validation/migrations/41' db-tool >"$raw" 2>&1; then
    record_log "$raw" migrate-0041-rerun.log
    fail '0041 rerun was not a no-op'
  fi
  record_log "$raw" migrate-0041-rerun.log
  verify_migration_count main_compose 41 0041-rerun
  stages_run=$(append_csv "$stages_run" '0041-rerun')
}

catalog_snapshot() {
  local raw=$1 snapshot
  snapshot=$(grep -E '^(catalog_fingerprint|catalog_audit_id|catalog_audit_count)=' "$raw") \
    || fail 'catalog preflight omitted its stable snapshot markers'
  [ "$(printf '%s\n' "$snapshot" | wc -l)" -eq 3 ] \
    || fail 'catalog preflight emitted an incomplete stable snapshot'
  printf '%s' "$snapshot"
}

run_full_release_lane() {
  local full_up_raw="$workdir/full-up.log"
  local full_bootstrap_raw="$workdir/full-bootstrap.log"
  local full_disk_raw="$workdir/full-disk-before.log"
  local full_migrate_raw="$workdir/full-migrate-first.log"
  local full_rerun_raw="$workdir/full-migrate-second.log"
  local catalog_first_raw="$workdir/full-catalog-first.log"
  local catalog_second_raw="$workdir/full-catalog-second.log"
  local catalog_first_snapshot catalog_second_snapshot full_bootstrap_status

  full_started=1
  if ! full_compose up -d --wait qa-db >"$full_up_raw" 2>&1; then
    record_log "$full_up_raw" full-up.log
    blocked 'empty full-release PostgreSQL container did not become healthy'
  fi
  record_log "$full_up_raw" full-up.log

  if bootstrap_roles full_compose >"$full_bootstrap_raw" 2>&1; then
    full_bootstrap_status=0
  else
    full_bootstrap_status=$?
  fi
  record_bootstrap_result "$full_password_dir" "$full_bootstrap_raw" full-bootstrap.log \
    full-release "$full_bootstrap_status"

  for service_role in migration_owner app worker audit_writer research_writer admin daily_input_admitter; do
    service_login_raw="$workdir/full-service-login-$service_role.log"
    if ! run_service_login full_compose "$service_role" "$service_login_raw"; then
      record_log "$service_login_raw" "full-service-login-$service_role.log"
      fail "full-release direct service-role login failed: $service_role"
    fi
    record_log "$service_login_raw" "full-service-login-$service_role.log"
    full_service_logins=$(append_csv "$full_service_logins" "$service_role")
  done
  if ! run_admitter_catalog_assertions full_compose "$workdir/full-admitter-catalog.log"; then
    record_log "$workdir/full-admitter-catalog.log" full-admitter-catalog.log
    fail 'full-release daily_input_admitter catalog/privilege assertions failed'
  fi
  record_log "$workdir/full-admitter-catalog.log" full-admitter-catalog.log
  if ! run_admitter_role_guc_check full_compose "$workdir/full-admitter-role-guc.log"; then
    record_log "$workdir/full-admitter-role-guc.log" full-admitter-role-guc.log
    fail 'full-release daily_input_admitter role GUC assertion failed'
  fi
  record_log "$workdir/full-admitter-role-guc.log" full-admitter-role-guc.log

  if ! run_disk_check full_compose "$full_disk_raw"; then
    record_log "$full_disk_raw" full-disk-before.log
    fail "full-release cluster has less than ${min_free_bytes} free bytes or disk inspection failed"
  fi
  record_log "$full_disk_raw" full-disk-before.log

  if ! full_compose run --rm --no-deps -e 'MIGRATIONS_DIR=/opt/lagrange/migrations' db-tool >"$full_migrate_raw" 2>&1; then
    record_log "$full_migrate_raw" full-migrate-first.log
    fail 'empty full-release migration run failed'
  fi
  record_log "$full_migrate_raw" full-migrate-first.log
  run_full_ledger_report full_compose first
  [ "$full_count:$full_latest" = "62:62" ] || fail '0062 full-release ledger must contain 62 rows'
  full_migration_count_first=$full_count
  full_latest_migration_first=$full_latest
  full_migration_checksums_first=$full_checksums
  [ "$full_migration_count_first" = "$latest_migration_version" ] \
    || fail "full-release first run applied $full_migration_count_first migrations; expected $latest_migration_version"
  [ "$full_latest_migration_first" = "$latest_migration_version" ] \
    || fail "full-release first run latest version is $full_latest_migration_first; expected $latest_migration_version"

  if ! run_catalog_preflight full_compose 0 "$catalog_first_raw" full none; then
    record_log "$catalog_first_raw" full-catalog-first.log
    fail 'full-release catalog preflight failed after the first migration run'
  fi
  record_log "$catalog_first_raw" full-catalog-first.log
  catalog_first_snapshot=$(catalog_snapshot "$catalog_first_raw")
  run_catalog_negative_probe full_compose full full_paper full_lane_paper_fixture_present \
    "$catalog_first_snapshot"

  if ! full_compose run --rm --no-deps -e 'MIGRATIONS_DIR=/opt/lagrange/migrations' db-tool >"$full_rerun_raw" 2>&1; then
    record_log "$full_rerun_raw" full-migrate-second.log
    fail 'empty full-release migration rerun failed'
  fi
  record_log "$full_rerun_raw" full-migrate-second.log
  run_full_ledger_report full_compose second
  full_migration_count_second=$full_count
  full_latest_migration_second=$full_latest
  full_migration_checksums_second=$full_checksums
  [ "$full_migration_count_second" = "$full_migration_count_first" ] \
    || fail 'full-release rerun changed the migration count'
  [ "$full_latest_migration_second" = "$full_latest_migration_first" ] \
    || fail 'full-release rerun changed the latest migration version'
  [ "$full_migration_checksums_second" = "$full_migration_checksums_first" ] \
    || fail 'full-release rerun changed migration ledger checksums'

  if ! run_catalog_preflight full_compose 0 "$catalog_second_raw" full none; then
    record_log "$catalog_second_raw" full-catalog-second.log
    fail 'full-release catalog preflight failed after the no-op rerun'
  fi
  record_log "$catalog_second_raw" full-catalog-second.log
  catalog_second_snapshot=$(catalog_snapshot "$catalog_second_raw")
  [ "$catalog_second_snapshot" = "$catalog_first_snapshot" ] \
    || fail 'full-release rerun changed catalog rows or catalog audit identity/count'
  full_release_rerun='NO_OP'

  full_disk_raw="$workdir/full-disk-after.log"
  if ! run_disk_check full_compose "$full_disk_raw"; then
    record_log "$full_disk_raw" full-disk-after.log
    fail "full-release cluster has less than ${min_free_bytes} free bytes after migrations"
  fi
  record_log "$full_disk_raw" full-disk-after.log
}

run_daily_0060_staged() {
  local raw role step status first_ledger second_ledger
  run_migration_stage 59
  raw="$workdir/preflight-0060-before.log"
  if ! run_role_psql main_compose migration_owner "$script_dir/preflight-0060.sql" --set seed=1 >"$raw" 2>&1; then
    record_log "$raw" preflight-0060-before.log; fail '0060 historical seed failed'
  fi
  record_log "$raw" preflight-0060-before.log
  run_migration_stage 60
  run_full_ledger_report main_compose staged-0060-first
  first_ledger=$full_checksums
  [ "$full_count:$full_latest" = '60:60' ] || fail '0060 staged ledger must contain 60 rows'
  for step in first rerun; do
    if [ "$step" = rerun ]; then
      run_migration_stage 60
      run_full_ledger_report main_compose staged-0060-second
      second_ledger=$full_checksums
      [ "$first_ledger" = "$second_ledger" ] || fail '0060 rerun changed migration checksums'
    fi
    raw="$workdir/preflight-0060-$step.log"
    if ! run_role_psql main_compose migration_owner "$script_dir/preflight-0060.sql" --set seed=0 >"$raw" 2>&1; then
      record_log "$raw" "preflight-0060-$step.log"; fail "0060 historical preservation failed: $step"
    fi
    record_log "$raw" "preflight-0060-$step.log"
  done
  for role in app worker research_writer admin daily_input_admitter; do
    raw="$workdir/actual-role-0060-$role.log"
    if ! run_role_psql main_compose "$role" "$script_dir/actual-role-0060.sql" >"$raw" 2>&1; then
      record_log "$raw" "actual-role-0060-$role.log"; fail "0060 actual role failed: $role"
    fi
    grep -Fqx "DAILY_0060_HARNESS:actual_$role:PASS" "$raw" || fail '0060 actual role result missing'
    printf 'DAILY_0060_HARNESS:actual_%s:PASS\n' "$role"
    record_log "$raw" "actual-role-0060-$role.log"
  done
  for step in unused seed_hidden attempt post; do
    raw="$workdir/rollback-0060-$step.log"
    if run_role_psql main_compose migration_owner "$script_dir/rollback-0060-guard.sql" \
       --set "unused=$([ "$step" = unused ] && printf 1 || printf 0)" \
       --set "seed_hidden=$([ "$step" = seed_hidden ] && printf 1 || printf 0)" \
       --set "attempt=$([ "$step" = attempt ] && printf 1 || printf 0)" >"$raw" 2>&1; then status=0; else status=$?; fi
    record_log "$raw" "rollback-0060-$step.log"
    if [ "$step" = attempt ]; then
      [ "$status" -ne 0 ] && grep -Eq 'ERROR:[[:space:]]+55000:' "$raw" || fail '0060 populated DOWN did not refuse with 55000'
    else
      [ "$status" -eq 0 ] || fail "0060 rollback assertion failed: $step"
    fi
    printf 'DAILY_0060_HARNESS:rollback_%s:PASS\n' "$step"
  done
  daily_harness=PASS
}

run_db_test() {
  local label=$1 target=$2 status skip=false row name expected actual
  local -a replay_rows=()
  local raw="$workdir/test-$label.log"
  if (cd "$root" && DATABASE_URL="$test_database_url" CARGO_TERM_COLOR=never cargo test --locked $target -- --nocapture) >"$raw" 2>&1; then status=0; else status=$?; fi
  if grep -Fq 'SKIP:' "$raw"; then skip=true; fi
  db_test_results+=("{\"label\":\"$label\",\"exit_code\":$status,\"skip_markers\":$skip}")
  printf 'PG_DB_TEST_EXIT:%s:%s:SKIP_%s\n' "$label" "$status" "$skip"
  if [ "$label" = migration-contract ]; then
    while IFS= read -r row; do
      if [[ "$row" =~ ^test[[:space:]](daily_0060_[a-z_]+)[[:space:]]\.\.\.[[:space:]](ok|FAILED)$ ]]; then
        name=${BASH_REMATCH[1]}; actual=${BASH_REMATCH[2]}
        daily_named_rows+=("$name:$actual")
        printf 'PG_DB_TEST_ROW:migration-contract:%s:%s\n' "$name" "$actual"
      elif [[ "$row" =~ ^DAILY_0060_SUBCASE:[a-z0-9_]+:PASS$ ]]; then
        daily_subcase_rows+=("${row#DAILY_0060_SUBCASE:}")
        printf '%s\n' "$row"
      fi
    done <"$raw"
  fi
  if [ "$label" = jobqueue-owner-beta-publish ] || [ "$label" = jobqueue-owner-beta-daily-schedule ] || [ "$label" = jobqueue-owner-beta-daily-publish ] || [ "$label" = jobqueue-owner-beta-daily-recovery ] || [ "$label" = jobqueue-owner-beta-daily-runner ]; then
    while IFS= read -r row; do
      if [[ "$row" =~ ^test[[:space:]]([a-z_]+)[[:space:]]\.\.\.[[:space:]](ok|FAILED)$ ]]; then
        name=${BASH_REMATCH[1]}; actual=${BASH_REMATCH[2]}
        replay_rows+=("$name:$actual")
        printf 'PG_DB_TEST_ROW:%s:%s:%s\n' "$label" "$name" "$actual"
      fi
    done <"$raw"
  fi
  record_log "$raw" "test-$label.log"
  [ "$status" -eq 0 ] || fail "DB-gated test failed: $label"
  [ "$skip" = false ] || fail "DB-gated test emitted SKIP despite DATABASE_URL: $label"
  if [ "$label" = jobqueue-owner-beta-publish ]; then
    for expected in published_owner_beta_target_persists_exact_results_and_seals_claim publication_config_lock_cannot_impersonate_another_owner canceled_before_owner_beta_publication_lock_has_no_outputs stale_swept_owner_beta_claim_cannot_publish_or_touch_run owner_beta_recovery_exhaustion_mirrors_failed_run owner_beta_recovery_mirrors_unclaimed_cancellation malformed_owner_beta_payload_is_terminal_without_inventing_a_run daily_input_admitter_pool_is_direct_bounded_and_unprivileged; do
      actual=0
      for row in "${replay_rows[@]}"; do [ "$row" != "$expected:ok" ] || actual=$((actual+1)); done
      [ "$actual" -eq 1 ] || fail "owner-beta unique publication test row missing: $expected"
    done
    [ "${#replay_rows[@]}" -eq 8 ] || fail 'owner-beta publication result inventory changed'
  fi
  if [ "$label" = jobqueue-owner-beta-daily-schedule ]; then
    for expected in \
      daily_mode_defaults_off_and_rejects_ambiguous_configuration \
      daily_disabled_mode_does_not_acquire_a_database_connection \
      daily_absent_or_disabled_control_does_not_reserve_work \
      daily_scheduler_reserves_canonical_snapshot_and_persistent_minute_gate \
      daily_scheduler_new_session_replays_permanent_reservation_without_duplicate_job \
      daily_scheduler_two_sessions_have_one_reservation_and_one_poll_receipt \
      daily_scheduler_stage_lock_precedes_the_persistent_poll_gate \
      daily_poll_roles_are_direct_and_cannot_be_interchanged \
      daily_admission_cycle_holds_lock_past_minute_and_commits_failure_receipt \
      daily_abandoned_admission_leaves_started_receipt_and_releases_session_lock \
      daily_schedule_missing_fresh_input_commits_checked_without_job_or_fallback \
      daily_schedule_capacity_includes_manual_jobs_without_reserving_on_deferral \
      daily_schedule_rechecks_optin_and_uses_the_current_config_epoch \
      daily_schedule_latest_failed_admission_cannot_fall_back_to_previous_verified_scan \
      daily_admitter_context_is_pending_scoped_and_default_off \
      daily_admitter_context_returns_exact_committed_tip \
      daily_admitter_context_rejects_completed_and_superseded_receipts \
      daily_admitter_readback_requires_direct_principal_and_no_table_grants \
      daily_admitter_readback_rechecks_owner_and_exact_rights \
      daily_admitter_readback_filters_foreign_owner_policy_and_stage \
      daily_admitter_receipt_recovery_distinguishes_started_and_completed_failure \
      daily_admitter_receipt_recovery_matches_verified_commit_without_mutation \
      daily_admitter_receipt_keeps_old_epoch_results_historical \
      daily_admitter_receipt_rejects_corrupt_verified_binding \
      daily_admitter_readback_retains_blocked_event_identity \
      daily_admitter_exact_input_absence_and_revocation_are_explicit \
      daily_admitter_readback_security_settings_are_fixed \
      daily_admitter_readback_down_preserves_existing_daily_evidence; do
      actual=0
      for row in "${replay_rows[@]}"; do [ "$row" != "$expected:ok" ] || actual=$((actual+1)); done
      [ "$actual" -eq 1 ] || fail "daily schedule unique test row missing: $expected"
    done
    [ "${#replay_rows[@]}" -eq 28 ] || fail 'daily schedule result inventory changed'
  fi
  if [ "$label" = jobqueue-owner-beta-daily-publish ]; then
    for expected in \
      daily_transient_failure_preserves_pending_row_and_permanent_reservation \
      daily_terminal_failure_releases_capacity_but_preserves_reservation \
      daily_expired_claim_cannot_settle_or_touch_the_pending_run \
      daily_failure_still_settles_after_control_is_disabled \
      daily_mismatched_payload_cannot_settle_the_valid_claim \
      daily_cancellation_wins_failure_and_clears_claim_without_outputs; do
      actual=0
      for row in "${replay_rows[@]}"; do [ "$row" != "$expected:ok" ] || actual=$((actual+1)); done
      [ "$actual" -eq 1 ] || fail "daily publish unique test row missing: $expected"
    done
    [ "${#replay_rows[@]}" -eq 6 ] || fail 'daily publish result inventory changed'
  fi
  if [ "$label" = jobqueue-owner-beta-daily-recovery ]; then
    for expected in \
      daily_recovery_does_not_steal_a_live_claim \
      daily_expired_recovery_requeues_the_same_job_and_preserves_all_sealed_rows \
      daily_recovery_exhausts_three_attempts_without_replacing_the_reservation \
      daily_racing_recoveries_orphan_and_requeue_only_once \
      daily_recovery_mirrors_unclaimed_audited_cancellation \
      daily_recovery_orphans_expired_canceled_attempt_and_releases_its_lease; do
      actual=0
      for row in "${replay_rows[@]}"; do [ "$row" != "$expected:ok" ] || actual=$((actual+1)); done
      [ "$actual" -eq 1 ] || fail "daily recovery unique test row missing: $expected"
    done
    [ "${#replay_rows[@]}" -eq 6 ] || fail 'daily recovery result inventory changed'
  fi
  if [ "$label" = migration-contract ]; then
    for expected in role_preflight up_preserves_histories repeat_is_noop catalog_authority direct_principal_denials config_owner_optin config_epoch control_epoch calendar_projection calendar_before_admission calendar_gap_cutoff admission_transport admission_parent_chain admission_conflict_replay poll_rate_and_crash receipt_authority event_and_freshness reservation_dedup capacity_deferral latest_only direct_daily_spoof valid_publish terminal_immutability claim_fences retry_and_recovery cancel_drain deferred_consistency revoke_publish_races cutoff_while_waiting down_unused down_all_tenants down_atomicity; do
      actual=0
      for row in "${daily_named_rows[@]}"; do [ "$row" != "daily_0060_$expected:ok" ] || actual=$((actual+1)); done
      [ "$actual" -eq 1 ] || fail "0060 unique test row missing: $expected"
    done
    for expected in role_super role_inherit role_createdb role_createrole role_bypass role_replication role_nologin role_member role_reverse role_schema role_database role_nousage role_noconnect role_protected_direct role_protected_transitive direct_app direct_worker direct_research_writer direct_admin direct_daily_input_admitter config_overflow calendar_copy_lineage blocked_replay_identity receipt_owner_user_id receipt_policy_sha256 down_daily_run down_admissions down_admission_events down_reservations down_calendar_sources down_calendar_days down_source_events down_publication_claims down_poll_receipts down_restored_off down_poll_state down_config_optin down_hidden_epoch latest_first_auto latest_disabled latest_expired_rights latest_unknown_calendar latest_newest_missing latest_failed_scan cancel_live cancel_expired cancel_queued retry_exhausted_live retry_exhausted_orphan wait_freshness wait_eligible_boundary matrix_queued_commit matrix_queued_immediate matrix_running_commit matrix_running_immediate matrix_succeeded_commit matrix_succeeded_immediate matrix_failed_commit matrix_failed_immediate matrix_canceled_commit matrix_canceled_immediate race_config_publish_first race_config_revoke_first race_off_on_publish_first race_off_on_revoke_first race_control_publish_first race_control_revoke_first race_rights_publish_first race_rights_revoke_first race_owner_publish_first race_owner_revoke_first race_source_publish_first race_source_revoke_first; do
      actual=0
      for row in "${daily_subcase_rows[@]}"; do [ "$row" != "$expected:PASS" ] || actual=$((actual+1)); done
      [ "$actual" -eq 1 ] || fail "0060 unique subcase row missing: $expected"
    done
  fi
  if [ "$label" = jobqueue-owner-beta-daily-runner ]; then
    for expected in \
      daily_preflight_current_claim_is_read_only_and_releases_locks \
      daily_preflight_denies_disabled_authority_and_settles_without_outputs \
      daily_preflight_rejects_edit_restore_epoch_without_resealing \
      daily_preflight_rejects_changed_payload_and_foreign_attempt \
      daily_preflight_finishes_audited_cancellation_exactly_once \
      daily_mixed_runner_disabled_mode_blocks_without_file_approval \
      daily_mixed_runner_current_thread_rejects_before_claim \
      daily_mixed_runner_foreign_policy_stops_before_base_or_admission_io \
      daily_mixed_runner_lease_mismatch_preserves_unclaimed_job \
      daily_preflight_rejects_app_admitter_and_elevated_pools \
      daily_supersession_preflight_cancels_and_replays_only_exact_identity \
      daily_supersession_failure_cleanup_cancels_instead_of_retrying \
      daily_supersession_cleanup_remains_possible_after_authority_revocation \
      daily_supersession_missing_calendar_does_not_invent_a_newer_session \
      daily_supersession_conflicting_calendar_does_not_cancel \
      daily_supersession_transaction_rollback_restores_every_output_and_lease \
      daily_supersession_samples_cutoff_after_waiting_for_control \
      daily_supersession_foreign_worker_attempt_and_payload_cannot_cancel \
      daily_supersession_expired_real_lease_cannot_settle \
      daily_supersession_rejects_other_real_principals_and_restores_actor \
      daily_supersession_additive_migration_down_up_preserves_populated_rows \
      daily_supersession_does_not_replace_an_audited_manual_cancel_reason \
      daily_supersession_observes_source_conflict_committed_while_waiting \
      daily_supersession_holds_source_fence_until_atomic_commit; do
      actual=0
      for row in "${replay_rows[@]}"; do [ "$row" != "$expected:ok" ] || actual=$((actual+1)); done
      [ "$actual" -eq 1 ] || fail "daily runner unique test row missing: $expected"
    done
    [ "${#replay_rows[@]}" -eq 24 ] || fail 'daily runner result inventory changed'
  fi
  tests_run=$(append_csv "$tests_run" "$label")
}

# Build private source files and role credentials before any container starts.
main_password_dir="$workdir/main-secrets"
test_password_dir="$workdir/test-secrets"
full_password_dir="$workdir/full-secrets"
mkdir -p "$main_password_dir" "$test_password_dir" "$full_password_dir"
chmod 0700 -- "$main_password_dir" "$test_password_dir" "$full_password_dir"
for role in postgres_password db_migration_owner_password db_app_password db_worker_password db_audit_password db_research_password db_admin_password db_daily_input_admitter_password; do
  generate_secret "$main_password_dir/$role"
  generate_secret "$full_password_dir/$role"
done
# The selected job-queue integration helper still constructs role URLs with
# the repository's existing disposable `lagrange` fixture instead of deriving
# them from DATABASE_URL. Keep that compatibility lane isolated from the
# production-like cluster; this value is never an operator or production
# credential and is written only to the private temporary secret directory.
printf '%s' 'lagrange' >"$test_password_dir/postgres_password"
chmod 0400 -- "$test_password_dir/postgres_password"
for role in db_migration_owner_password db_app_password db_worker_password db_audit_password db_research_password db_admin_password db_daily_input_admitter_password; do
  cp -- "$test_password_dir/postgres_password" "$test_password_dir/$role"
  chmod 0400 -- "$test_password_dir/$role"
done
make_stages
main_project="lagrange_pg_validation_${$}"
test_project="lagrange_pg_validation_test_${$}"
full_project="lagrange_pg_validation_full_${$}"
main_override="$workdir/main.compose.yml"
test_override="$workdir/test.compose.yml"
full_override="$workdir/full.compose.yml"
make_override "$main_override" "$main_password_dir" "$main_port" 1 1 /validation/migrations/41
make_override "$full_override" "$full_password_dir" "$full_port" 1 0 /opt/lagrange/migrations
make_override "$test_override" "$test_password_dir" "$test_port" 0 0

docker_info_raw="$workdir/docker-info.log"
if ! docker info >"$docker_info_raw" 2>&1; then
  record_log "$docker_info_raw" docker-info.log
  blocked 'Docker daemon is unavailable; run this command on a host with the Docker Engine started and accessible'
fi
docker_ready=1

compose_version_raw="$workdir/compose-version.log"
if ! docker compose version >"$compose_version_raw" 2>&1; then
  record_log "$compose_version_raw" compose-version.log
  blocked 'Docker Compose is unavailable; use the existing Docker Compose plugin on the validation host'
fi
record_log "$compose_version_raw" compose-version.log

compose_config_raw="$workdir/compose-config.log"
if ! main_compose config --quiet >"$compose_config_raw" 2>&1; then
  record_log "$compose_config_raw" compose-config.log
  fail 'generated upgrade Compose configuration is invalid'
fi
if ! test_compose config --quiet >>"$compose_config_raw" 2>&1; then
  record_log "$compose_config_raw" compose-config.log
  fail 'generated test Compose configuration is invalid'
fi
if ! full_compose config --quiet >>"$compose_config_raw" 2>&1; then
  record_log "$compose_config_raw" compose-config.log
  fail 'generated full-release Compose configuration is invalid'
fi
record_log "$compose_config_raw" compose-config.log

main_up_raw="$workdir/main-up.log"
main_started=1
if ! main_compose up -d --wait qa-db >"$main_up_raw" 2>&1; then
  record_log "$main_up_raw" main-up.log
  blocked 'upgrade PostgreSQL container did not become healthy'
fi
record_log "$main_up_raw" main-up.log

bootstrap_raw="$workdir/main-bootstrap.log"
if bootstrap_roles main_compose >"$bootstrap_raw" 2>&1; then
  bootstrap_status=0
else
  bootstrap_status=$?
fi
record_bootstrap_result "$main_password_dir" "$bootstrap_raw" main-bootstrap.log staged "$bootstrap_status"

run_bootstrap_negative_contract

for service_role in migration_owner app worker audit_writer research_writer admin daily_input_admitter; do
  service_login_raw="$workdir/service-login-$service_role.log"
  if ! run_service_login main_compose "$service_role" "$service_login_raw"; then
    record_log "$service_login_raw" "service-login-$service_role.log"
    fail "direct service-role login failed: $service_role"
  fi
  record_log "$service_login_raw" "service-login-$service_role.log"
  service_logins=$(append_csv "$service_logins" "$service_role")
done
if ! run_admitter_catalog_assertions main_compose "$workdir/admitter-catalog.log"; then
  record_log "$workdir/admitter-catalog.log" admitter-catalog.log
  fail 'staged daily_input_admitter catalog/privilege assertions failed'
fi
record_log "$workdir/admitter-catalog.log" admitter-catalog.log
if ! run_admitter_role_guc_check main_compose "$workdir/admitter-role-guc.log"; then
  record_log "$workdir/admitter-role-guc.log" admitter-role-guc.log
  fail 'staged daily_input_admitter role GUC assertion failed'
fi
record_log "$workdir/admitter-role-guc.log" admitter-role-guc.log

build_raw="$workdir/db-tool-build.log"
if [ -z "${LAGRANGE_DB_VALIDATION_IMAGE:-}" ]; then
  if ! main_compose build db-tool >"$build_raw" 2>&1; then
    record_log "$build_raw" db-tool-build.log
    blocked 'deploy/db image could not be built; no alternate migration tool is installed by this workflow'
  fi
  record_log "$build_raw" db-tool-build.log
fi

hazard_raw="$workdir/hazards-baseline.log"
if ! run_disk_check main_compose "$hazard_raw"; then
  record_log "$hazard_raw" hazards-baseline-disk.log
  fail "upgrade cluster has less than ${min_free_bytes} free bytes or disk inspection failed"
fi
record_log "$hazard_raw" hazards-baseline-disk.log
if ! run_psql main_compose "$script_dir/hazards.sql" >"$workdir/hazards-baseline-sql.log" 2>&1; then
  record_log "$workdir/hazards-baseline-sql.log" hazards-baseline-sql.log
  fail 'baseline connection/schema hazard preflight failed'
fi
record_log "$workdir/hazards-baseline-sql.log" hazards-baseline-sql.log

run_schema_only_stage38
seed_sql="$workdir/seed-pre-0039.sql"
cp -- "$script_dir/seed-pre-0039.sql" "$seed_sql"
if ! run_psql main_compose "$seed_sql" >"$workdir/seed-after-baseline.log" 2>&1; then
  record_log "$workdir/seed-after-baseline.log" seed-after-baseline.log
  fail 'baseline fixture could not be seeded after 0038'
fi
record_log "$workdir/seed-after-baseline.log" seed-after-baseline.log

if ! run_psql main_compose "$script_dir/preflight-baseline.sql" >"$workdir/preflight-baseline.log" 2>&1; then
  record_log "$workdir/preflight-baseline.log" preflight-baseline.log
  fail 'pre-0039 baseline preflight failed'
fi
record_log "$workdir/preflight-baseline.log" preflight-baseline.log

run_migration_stage 39
if ! run_role_psql main_compose migration_owner "$script_dir/preflight-0039.sql" \
    --set terminal_only=0 --set "catalog_sha256=$catalog_sha256" >"$workdir/preflight-0039.log" 2>&1; then
  record_log "$workdir/preflight-0039.log" preflight-0039.log
  fail '0039 auth-audit outbox preflight failed'
fi
record_log "$workdir/preflight-0039.log" preflight-0039.log
if ! run_terminal_paper_check main_compose first; then
  fail '0039 supervisor terminal-Paper fixture preflight failed'
fi

catalog_stage_first_raw="$workdir/catalog-preflight-first.log"
if ! run_catalog_preflight main_compose 0 "$catalog_stage_first_raw" staged none; then
  record_log "$catalog_stage_first_raw" catalog-preflight-first.log
  fail 'staged catalog preflight failed after the initial 0039 install'
fi
record_log "$catalog_stage_first_raw" catalog-preflight-first.log
catalog_stage_first_snapshot=$(catalog_snapshot "$catalog_stage_first_raw")
catalog_install=PASS
run_catalog_negative_probe main_compose staged unexpected_strategy unexpected_strategy_id \
  "$catalog_stage_first_snapshot"
run_catalog_negative_probe main_compose staged missing_baseline missing_baseline_strategy \
  "$catalog_stage_first_snapshot"
run_catalog_negative_probe main_compose staged mutated_baseline baseline_strategy_content_mismatch \
  "$catalog_stage_first_snapshot"

catalog_conflict_raw="$workdir/catalog-conflict-probe.log"
if run_catalog_preflight main_compose 1 "$catalog_conflict_raw" staged none; then
  record_log "$catalog_conflict_raw" catalog-conflict-probe.log
  fail 'catalog conflict probe accepted a synthetic conflicting row'
fi
record_log "$catalog_conflict_raw" catalog-conflict-probe.log
if ! grep -Eq 'ERROR:[[:space:]]+23505:' "$catalog_conflict_raw"; then
  fail 'catalog conflict probe failed without the expected SQLSTATE 23505'
fi
catalog_conflict='PASS_SQLSTATE_23505'
catalog_after_conflict_raw="$workdir/catalog-after-conflict.log"
if ! run_catalog_preflight main_compose 0 "$catalog_after_conflict_raw" staged none; then
  record_log "$catalog_after_conflict_raw" catalog-after-conflict.log
  fail 'catalog conflict probe did not roll back its synthetic change'
fi
record_log "$catalog_after_conflict_raw" catalog-after-conflict.log
catalog_after_conflict_snapshot=$(catalog_snapshot "$catalog_after_conflict_raw")
[ "$catalog_after_conflict_snapshot" = "$catalog_stage_first_snapshot" ] \
  || fail 'catalog conflict probe changed catalog rows or audit identity/count'

run_migration_rerun_0039
if ! run_role_psql main_compose migration_owner "$script_dir/preflight-0039.sql" \
    --set terminal_only=0 --set "catalog_sha256=$catalog_sha256" >"$workdir/preflight-0039-rerun.log" 2>&1; then
  record_log "$workdir/preflight-0039-rerun.log" preflight-0039-rerun.log
  fail '0039 audit preflight failed after the catalog sync rerun'
fi
record_log "$workdir/preflight-0039-rerun.log" preflight-0039-rerun.log
if ! run_terminal_paper_check main_compose rerun; then
  fail '0039 supervisor terminal-Paper fixture changed after the catalog-sync rerun'
fi
catalog_stage_rerun_raw="$workdir/catalog-preflight-rerun.log"
if ! run_catalog_preflight main_compose 0 "$catalog_stage_rerun_raw" staged none; then
  record_log "$catalog_stage_rerun_raw" catalog-preflight-rerun.log
  fail 'staged catalog preflight failed after the 0039 no-op rerun'
fi
record_log "$catalog_stage_rerun_raw" catalog-preflight-rerun.log
catalog_stage_rerun_snapshot=$(catalog_snapshot "$catalog_stage_rerun_raw")
[ "$catalog_stage_rerun_snapshot" = "$catalog_stage_first_snapshot" ] \
  || fail '0039 catalog rerun changed catalog rows or audit identity/count'
catalog_rerun=PASS

# Exercise the tracked 0039 down migration against a real undelivered row.
# The command is expected to fail with SQLSTATE 55000; any success or other
# error is a hard denial. The postflight removes only the synthetic row.
rollback_guard_raw="$workdir/rollback-0039-guard.log"
if run_role_psql main_compose migration_owner "$script_dir/rollback-0039-guard.sql" \
    >"$rollback_guard_raw" 2>&1; then
  record_log "$rollback_guard_raw" rollback-0039-guard.log
  fail '0039 down migration did not refuse an undelivered auth-audit row'
fi
record_log "$rollback_guard_raw" rollback-0039-guard.log
if ! grep -Eq 'ERROR:[[:space:]]+55000:' "$rollback_guard_raw"; then
  fail '0039 down migration failed without the expected SQLSTATE 55000 guard'
fi
if ! run_psql main_compose "$script_dir/rollback-0039-postflight.sql" \
    >"$workdir/rollback-0039-postflight.log" 2>&1; then
  record_log "$workdir/rollback-0039-postflight.log" rollback-0039-postflight.log
  fail '0039 rollback guard did not preserve the migration ledger/row'
fi
record_log "$workdir/rollback-0039-postflight.log" rollback-0039-postflight.log
rollback_guard='PASS'
if ! run_role_psql main_compose migration_owner "$script_dir/preflight-0039.sql" \
    --set terminal_only=0 --set "catalog_sha256=$catalog_sha256" >"$workdir/preflight-0039-after-guard.log" 2>&1; then
  record_log "$workdir/preflight-0039-after-guard.log" preflight-0039-after-guard.log
  fail '0039 catalog audit event did not survive cleanup of the down-guard fixture'
fi
record_log "$workdir/preflight-0039-after-guard.log" preflight-0039-after-guard.log
if ! run_terminal_paper_check main_compose after-down-guard; then
  fail '0039 supervisor terminal-Paper fixture changed after down-guard cleanup'
fi

run_migration_stage 40
if ! run_psql main_compose "$script_dir/preflight-0040.sql" >"$workdir/preflight-0040.log" 2>&1; then
  record_log "$workdir/preflight-0040.log" preflight-0040.log
  fail '0040 identity provisioning preflight failed'
fi
record_log "$workdir/preflight-0040.log" preflight-0040.log

if ! run_app_actor_sql main_compose "$workdir/identity-boundary.log"; then
  record_log "$workdir/identity-boundary.log" identity-boundary.log
  fail '0040 cross-owner identity provisioning boundary accepted Owner B input from Owner A'
fi
record_log "$workdir/identity-boundary.log" identity-boundary.log

run_migration_stage 41
if ! run_psql main_compose "$script_dir/preflight-0041.sql" >"$workdir/preflight-0041.log" 2>&1; then
  record_log "$workdir/preflight-0041.log" preflight-0041.log
  fail '0041 Paper settlement outbox preflight failed'
fi
record_log "$workdir/preflight-0041.log" preflight-0041.log
run_migration_rerun_0041
if ! run_psql main_compose "$script_dir/preflight-0041.sql" >"$workdir/preflight-0041-rerun.log" 2>&1; then
  record_log "$workdir/preflight-0041-rerun.log" preflight-0041-rerun.log
  fail '0041 no-op rerun changed final Paper outbox invariants'
fi
record_log "$workdir/preflight-0041-rerun.log" preflight-0041-rerun.log

run_daily_0060_staged

hazard_final_raw="$workdir/hazards-final-disk.log"
if ! run_disk_check main_compose "$hazard_final_raw"; then
  record_log "$hazard_final_raw" hazards-final-disk.log
  fail "upgrade cluster has less than ${min_free_bytes} free bytes after migration"
fi
record_log "$hazard_final_raw" hazards-final-disk.log

main_compose down -v --remove-orphans >"$workdir/cleanup-main-before-full.log" 2>&1
record_log "$workdir/cleanup-main-before-full.log" cleanup-main-before-full.log
main_started=0

run_full_release_lane

full_cleanup_raw="$workdir/cleanup-full-before-test.log"
if ! full_compose down -v --remove-orphans >"$full_cleanup_raw" 2>&1; then
  record_log "$full_cleanup_raw" cleanup-full-before-test.log
  fail 'full-release cluster teardown failed before the DB-gated test lane'
fi
record_log "$full_cleanup_raw" cleanup-full-before-test.log
full_ps_raw="$workdir/full-project-ps-after-cleanup.log"
if ! full_compose ps -q qa-db >"$full_ps_raw" 2>&1; then
  record_log "$full_ps_raw" full-project-ps-after-cleanup.log
  fail 'could not verify full-release containers were removed before the DB-gated test lane'
fi
record_log "$full_ps_raw" full-project-ps-after-cleanup.log
[ -z "$(tr -d '\r\n' <"$full_ps_raw")" ] \
  || fail 'full-release cluster still has an owned container before the DB-gated test lane'
full_started=0
full_cleanup=PASS_NO_OWNED_CONTAINERS

test_up_raw="$workdir/test-up.log"
test_started=1
if ! test_compose up -d --wait qa-db >"$test_up_raw" 2>&1; then
  record_log "$test_up_raw" test-up.log
  blocked 'DB-gated test PostgreSQL container did not become healthy'
fi
record_log "$test_up_raw" test-up.log

test_bootstrap_raw="$workdir/test-bootstrap.log"
if bootstrap_roles test_compose >"$test_bootstrap_raw" 2>&1; then
  test_bootstrap_status=0
else
  test_bootstrap_status=$?
fi
record_bootstrap_result "$test_password_dir" "$test_bootstrap_raw" test-bootstrap.log \
  test-cluster "$test_bootstrap_status"

test_database_url="postgres://postgres:$(<"$test_password_dir/postgres_password")@127.0.0.1:${test_port}/postgres"
# Every command receives DATABASE_URL. A SKIP marker is a hard failure.
run_db_test migration-contract '-p migration-contract --test migration_contract'
run_db_test auth-audit-readiness '-p api-server-auth audit'
run_db_test auth-router '-p api-server-auth router_qa'
run_db_test api-tenancy-rls '-p api-server --test tenancy_rls'
run_db_test api-paper-execution '-p api-server --test paper_execution_seam'
run_db_test api-paper-notifications '-p api-server --test paper_notifications'
run_db_test api-paper-scheduler '-p api-server --test paper_scheduler'
run_db_test api-paper-runner '-p api-server --test paper_runner'
run_db_test jobqueue-contract '-p job-queue --test queue_contract'
run_db_test jobqueue-paper-preview '-p job-queue --test paper_preview'
run_db_test jobqueue-daily-admitter-pool '-p job-queue --test owner_beta_publish daily_input_admitter_pool_is_direct_bounded_and_unprivileged'
run_db_test jobqueue-owner-beta-publish '-p job-queue --test owner_beta_publish'
run_db_test jobqueue-owner-beta-daily-schedule '-p job-queue --test owner_beta_daily_schedule'
run_db_test jobqueue-owner-beta-daily-publish '-p job-queue --test owner_beta_daily_publish'
run_db_test jobqueue-owner-beta-daily-recovery '-p job-queue --test owner_beta_daily_recovery'
run_db_test jobqueue-owner-beta-daily-runner '-p job-queue --test owner_beta_daily_runner'
run_db_test api-daily-admitter-pool '-p api-server --test http_owner_beta_access http_daily_input_admitter_pool_is_direct_bounded_and_unprivileged'

outcome=APPROVED
detail='schema-only 0038 upgrade, staged 0039/0040/0041 checks, empty full-release endpoint and no-op rerun, catalog audits, and DB-gated tests passed without SKIP'
exit 0
