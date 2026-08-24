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

Run a disposable PostgreSQL 18 migration upgrade and DB-gated Rust suites.
Upgrade credentials are generated under a private temporary directory; the
test lane uses the repository's synthetic compatibility fixture. Both
clusters are torn down on every exit by default.

Options:
  --evidence-dir PATH  Keep sanitized logs/evidence at PATH (default: a new
                       /tmp/lagrange-pg-validation-evidence.* directory).
  --main-port PORT     Loopback port for the upgrade cluster (default: 55432).
  --test-port PORT     Loopback port for the DB-gated test cluster
                       (default: 55433).
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
[ "$main_port" != "$test_port" ] || die_usage '--main-port and --test-port must differ'

min_free_bytes=${LAGRANGE_VALIDATION_MIN_FREE_BYTES:-134217728}
case "$min_free_bytes" in
  ''|*[!0-9]*) die_usage 'LAGRANGE_VALIDATION_MIN_FREE_BYTES must be numeric' ;;
esac

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

  self_migrations="$self_workdir/migrations"
  mkdir -p "$self_migrations"
  for limit in 38 39 40 41; do
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
  for required_file in \
    "$script_dir/preflight-baseline.sql" \
    "$script_dir/preflight-0039.sql" \
    "$script_dir/preflight-0040.sql" \
    "$script_dir/preflight-0041.sql" \
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
  echo 'PG_VALIDATION_SELF_TEST: PASS'
  exit 0
fi

for command_name in docker openssl mktemp awk sed grep find install date bash cat chmod cp rm tr wc cargo git; do
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
cleaned=0
docker_ready=0
stages_run=''
tests_run=''
service_logins=''
rollback_guard='NOT_RUN'
skip_guard='ENFORCED'

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
  local escaped_detail escaped_started escaped_finished
  escaped_detail=$(json_escape "$detail")
  escaped_started=$(json_escape "$started_at")
  escaped_finished=$(json_escape "$finished_at")
  {
    printf '{\n'
    printf '  "workflow": "postgres-integration-validation",\n'
    printf '  "verdict": "%s",\n' "$outcome"
    printf '  "detail": "%s",\n' "$escaped_detail"
    printf '  "started_at": "%s",\n' "$escaped_started"
    printf '  "finished_at": "%s",\n' "$escaped_finished"
    printf '  "code_commit": "%s",\n' "$validation_code_commit"
    printf '  "postgres_image": "postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a",\n'
    printf '  "upgrade": {"baseline": "0038", "sequential": ["0039", "0040", "0041"]},\n'
    printf '  "ports": {"upgrade": %s, "tests": %s},\n' "$main_port" "$test_port"
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
    printf '  "checks": {"direct_service_role_logins": "%s", "0039_down_undelivered_guard": "%s", "skip_markers": "%s"},\n' \
      "$(json_escape "$service_logins")" "$rollback_guard" "$skip_guard"
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
    printf '  "sanitized_evidence_dir": "%s"\n' "$(json_escape "$evidence_dir")"
    printf '}\n'
  } >"$evidence_dir/evidence.json"
  chmod 0600 -- "$evidence_dir/evidence.json"
  {
    printf 'verdict\t%s\n' "$outcome"
    printf 'detail\t%s\n' "$detail"
    printf 'code_commit\t%s\n' "$validation_code_commit"
    printf 'baseline\t0038\n'
    printf 'sequential\t0039,0040,0041\n'
    printf 'stages_run\t%s\n' "$stages_run"
    printf 'tests_run\t%s\n' "$tests_run"
    printf 'direct_service_role_logins\t%s\n' "$service_logins"
    printf '0039_down_undelivered_guard\t%s\n' "$rollback_guard"
    printf 'skip_markers\t%s\n' "$skip_guard"
    printf 'upgrade_port\t%s\n' "$main_port"
    printf 'test_port\t%s\n' "$test_port"
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
}

make_stages() {
  migration_root="$workdir/migrations"
  mkdir -p "$migration_root"
  for limit in 38 39 40 41; do
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
      MIGRATIONS_DIR: /validation/migrations/41
    volumes:
      - ${migration_root}:/validation/migrations:ro
    secrets:
      - db_migration_owner_password
    networks:
      - default
EOF
)
    else
      tool_block=$(cat <<EOF
  db-tool:
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
      MIGRATIONS_DIR: /validation/migrations/41
    volumes:
      - ${migration_root}:/validation/migrations:ro
    secrets:
      - db_migration_owner_password
    networks:
      - default
EOF
)
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
    if [ "$include_tool" -eq 1 ]; then
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
    qa-db /bin/bash /validation/bootstrap-roles.sh
}

run_psql() {
  local compose_function=$1 sql_file=$2
  "$compose_function" exec -T -u 0 qa-db /bin/bash -ec \
    'export PGPASSWORD="$(cat /validation/secrets/postgres_password)"; exec psql -X -v ON_ERROR_STOP=1 -P pager=off -At -h 127.0.0.1 -p 5432 -U postgres -d postgres' \
    <"$sql_file"
}

run_role_psql() {
  local compose_function=$1 role=$2 sql_file=$3
  local secret_name
  case "$role" in
    migration_owner) secret_name=db_migration_owner_password ;;
    app) secret_name=db_app_password ;;
    worker) secret_name=db_worker_password ;;
    audit_writer) secret_name=db_audit_password ;;
    research_writer) secret_name=db_research_password ;;
    admin) secret_name=db_admin_password ;;
    *) return 1 ;;
  esac
  "$compose_function" exec -T -u 0 qa-db /bin/bash -ec \
    "export PGPASSWORD=\"\$(cat /validation/secrets/${secret_name})\"; exec psql -X -v ON_ERROR_STOP=1 -v VERBOSITY=verbose -P pager=off -At -h 127.0.0.1 -p 5432 -U ${role} -d postgres" \
    <"$sql_file"
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
    *) return 1 ;;
  esac
  "$compose_function" exec -T -u 0 qa-db /bin/bash -ec \
    "export PGPASSWORD=\"\$(cat /validation/secrets/${secret_name})\"; exec psql -X -v ON_ERROR_STOP=1 -P pager=off -At -h 127.0.0.1 -p 5432 -U ${role} -d postgres" \
    <"$script_dir/service-login.sql" >"$raw" 2>&1
  expected="$role=$role"
  observed=$(tr -d '\r' <"$raw" | tail -n 1)
  [ "$observed" = "$expected" ]
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

run_migration_stage() {
  local version=$1 raw="$workdir/migrate-$1.log"
  if ! main_compose run --rm --no-deps -e "MIGRATIONS_DIR=/validation/migrations/$version" db-tool >"$raw" 2>&1; then
    record_log "$raw" "migrate-$version.log"
    fail "migration stage $version failed"
  fi
  record_log "$raw" "migrate-$version.log"
  stage_count_sql="$workdir/check-version-$version.sql"
  printf 'SELECT count(*) FROM _sqlx_migrations;\n' >"$stage_count_sql"
  count_raw="$workdir/version-$version.log"
  if ! run_psql main_compose "$stage_count_sql" >"$count_raw" 2>&1; then
    record_log "$count_raw" "version-$version.log"
    fail "could not verify migration stage $version"
  fi
  record_log "$count_raw" "version-$version.log"
  observed=$(tr -d '\r' <"$count_raw" | tail -n 1)
  [ "$observed" = "$version" ] || fail "migration stage $version recorded $observed applied migrations; expected $version"
  stages_run=$(append_csv "$stages_run" "$version")
}

run_migration_rerun_0041() {
  raw="$workdir/migrate-0041-rerun.log"
  if ! main_compose run --rm --no-deps -e 'MIGRATIONS_DIR=/validation/migrations/41' db-tool >"$raw" 2>&1; then
    record_log "$raw" migrate-0041-rerun.log
    fail '0041 rerun was not a no-op'
  fi
  record_log "$raw" migrate-0041-rerun.log
  count_sql="$workdir/check-version-0041-rerun.sql"
  printf 'SELECT count(*) FROM _sqlx_migrations;\n' >"$count_sql"
  if ! run_psql main_compose "$count_sql" >"$workdir/version-0041-rerun.log" 2>&1; then
    record_log "$workdir/version-0041-rerun.log" version-0041-rerun.log
    fail 'could not verify 0041 no-op rerun'
  fi
  record_log "$workdir/version-0041-rerun.log" version-0041-rerun.log
  observed=$(tr -d '\r' <"$workdir/version-0041-rerun.log" | tail -n 1)
  [ "$observed" = 41 ] || fail "0041 rerun changed applied migration count to $observed"
  stages_run=$(append_csv "$stages_run" '0041-rerun')
}

run_db_test() {
  local label=$1 target=$2
  raw="$workdir/test-$label.log"
  if ! (cd "$root" && DATABASE_URL="$test_database_url" CARGO_TERM_COLOR=never cargo test --locked $target -- --nocapture) >"$raw" 2>&1; then
    record_log "$raw" "test-$label.log"
    fail "DB-gated test failed: $label"
  fi
  record_log "$raw" "test-$label.log"
  if grep -Fq 'SKIP:' "$raw"; then
    fail "DB-gated test emitted SKIP despite DATABASE_URL: $label"
  fi
  tests_run=$(append_csv "$tests_run" "$label")
}

# Build private source files and role credentials before any container starts.
main_password_dir="$workdir/main-secrets"
test_password_dir="$workdir/test-secrets"
mkdir -p "$main_password_dir" "$test_password_dir"
chmod 0700 -- "$main_password_dir" "$test_password_dir"
for role in postgres_password db_migration_owner_password db_app_password db_worker_password db_audit_password db_research_password db_admin_password; do
  generate_secret "$main_password_dir/$role"
done
# The selected job-queue integration helper still constructs role URLs with
# the repository's existing disposable `lagrange` fixture instead of deriving
# them from DATABASE_URL. Keep that compatibility lane isolated from the
# production-like cluster; this value is never an operator or production
# credential and is written only to the private temporary secret directory.
printf '%s' 'lagrange' >"$test_password_dir/postgres_password"
chmod 0400 -- "$test_password_dir/postgres_password"
for role in db_migration_owner_password db_app_password db_worker_password db_audit_password db_research_password db_admin_password; do
  cp -- "$test_password_dir/postgres_password" "$test_password_dir/$role"
  chmod 0400 -- "$test_password_dir/$role"
done
make_stages
main_project="lagrange_pg_validation_${$}"
test_project="lagrange_pg_validation_test_${$}"
main_override="$workdir/main.compose.yml"
test_override="$workdir/test.compose.yml"
make_override "$main_override" "$main_password_dir" "$main_port" 1
make_override "$test_override" "$test_password_dir" "$test_port" 0

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
record_log "$compose_config_raw" compose-config.log

main_up_raw="$workdir/main-up.log"
main_started=1
if ! main_compose up -d --wait qa-db >"$main_up_raw" 2>&1; then
  record_log "$main_up_raw" main-up.log
  blocked 'upgrade PostgreSQL container did not become healthy'
fi
record_log "$main_up_raw" main-up.log

bootstrap_raw="$workdir/main-bootstrap.log"
if ! bootstrap_roles main_compose >"$bootstrap_raw" 2>&1; then
  record_log "$bootstrap_raw" main-bootstrap.log
  fail 'production-like role bootstrap failed'
fi
record_log "$bootstrap_raw" main-bootstrap.log

for service_role in migration_owner app worker audit_writer research_writer admin; do
  service_login_raw="$workdir/service-login-$service_role.log"
  if ! run_service_login main_compose "$service_role" "$service_login_raw"; then
    record_log "$service_login_raw" "service-login-$service_role.log"
    fail "direct service-role login failed: $service_role"
  fi
  record_log "$service_login_raw" "service-login-$service_role.log"
  service_logins=$(append_csv "$service_logins" "$service_role")
done

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

run_migration_stage 38
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
if ! run_psql main_compose "$script_dir/preflight-0039.sql" >"$workdir/preflight-0039.log" 2>&1; then
  record_log "$workdir/preflight-0039.log" preflight-0039.log
  fail '0039 auth-audit outbox preflight failed'
fi
record_log "$workdir/preflight-0039.log" preflight-0039.log

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

hazard_final_raw="$workdir/hazards-final-disk.log"
if ! run_disk_check main_compose "$hazard_final_raw"; then
  record_log "$hazard_final_raw" hazards-final-disk.log
  fail "upgrade cluster has less than ${min_free_bytes} free bytes after migration"
fi
record_log "$hazard_final_raw" hazards-final-disk.log

test_up_raw="$workdir/test-up.log"
test_started=1
if ! test_compose up -d --wait qa-db >"$test_up_raw" 2>&1; then
  record_log "$test_up_raw" test-up.log
  blocked 'DB-gated test PostgreSQL container did not become healthy'
fi
record_log "$test_up_raw" test-up.log

test_bootstrap_raw="$workdir/test-bootstrap.log"
if ! bootstrap_roles test_compose >"$test_bootstrap_raw" 2>&1; then
  record_log "$test_bootstrap_raw" test-bootstrap.log
  fail 'test-cluster role bootstrap failed'
fi
record_log "$test_bootstrap_raw" test-bootstrap.log

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

outcome=APPROVED
detail='0038 baseline, sequential 0039/0040/0041, preflights, and DB-gated tests passed without SKIP'
exit 0
