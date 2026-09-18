#!/usr/bin/env bash
# Full-migration, actual-role PostgreSQL coverage for the one approved
# entitlement amendment. This never uses a permissive surrogate schema: every
# tracked up migration runs as the same NOBYPASSRLS migration_owner used by
# db_psql, and the disposable database is destroyed on exit.
set -euo pipefail

script_dir=$(cd "$(dirname "$0")" && pwd)
repo_root=$(cd "$script_dir/../.." && pwd)
db_helper="$repo_root/scripts/ops/lib/db.sh"
amend_helper="$repo_root/scripts/ops/lib/entitlement-amend.sh"
bootstrap_roles="$repo_root/deploy/db/bootstrap-roles.sh"
migrations_dir="$repo_root/migrations"

phase=preflight
fail() { echo "ENTITLEMENT_AMEND_DB_TEST: FAIL status=assertion phase=${phase:-unknown}" >&2; exit 1; }
blocked() { echo "ENTITLEMENT_AMEND_DB_TEST: BLOCKED status=$1" >&2; exit 2; }

[ -x "$db_helper" ] || fail
[ -x "$amend_helper" ] || fail
[ -x "$bootstrap_roles" ] || fail
[ -d "$migrations_dir" ] || fail
command -v docker >/dev/null 2>&1 || blocked docker_missing
command -v flock >/dev/null 2>&1 || blocked flock_missing
command -v jq >/dev/null 2>&1 || blocked jq_missing
command -v sha256sum >/dev/null 2>&1 || blocked sha256sum_missing
[ -z "${LAGRANGE_COMPOSE_FILE:-}" ] || blocked inherited_compose_target
[ -z "${LAGRANGE_ENV_FILE:-}" ] || blocked inherited_env_target
[ -z "${COMPOSE_PROJECT_NAME:-}" ] || blocked inherited_compose_project

# Cluster-global role names are shared with other disposable role tests.
exec 9>/tmp/lagrange-kis-live-cargo.lock
flock -w 60 9 || blocked role_test_lock_busy

available_kib=$(awk '$1 == "MemAvailable:" {print $2; exit}' /proc/meminfo)
[ -n "$available_kib" ] && [ "$available_kib" -ge 1048576 ] || blocked low_memory

postgres_image='postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a'
if ! docker image inspect "$postgres_image" >/dev/null 2>&1; then
  if docker info >/dev/null 2>&1; then
    blocked pinned_postgres_image_missing
  fi
  blocked docker_unavailable
fi

tmp=$(mktemp -d /tmp/lagrange-entitlement-amend-db-test.XXXXXX)
compose_file="$tmp/compose.yml"
env_file="$tmp/compose.env"
postgres_password_file="$tmp/postgres.password"
migration_password_file="$tmp/migration.password"
app_password_file="$tmp/app.password"
worker_password_file="$tmp/worker.password"
audit_password_file="$tmp/audit.password"
research_password_file="$tmp/research.password"
admin_password_file="$tmp/admin.password"
project=$(basename "$tmp" | tr '[:upper:]' '[:lower:]' | tr '.' '-')
# db_psql deliberately uses the repository helper unchanged.  Pin its Compose
# project identity to this disposable project too, so `compose run` cannot join
# a same-named private test network under a different derived project label.
export COMPOSE_PROJECT_NAME="$project"

cleanup() {
  local status=$?
  if [ -f "$compose_file" ]; then
    docker compose -p "$project" --env-file "$env_file" -f "$compose_file" \
      down --volumes --remove-orphans >/dev/null 2>&1 || true
  fi
  rm -rf -- "$tmp"
  if [ "$status" -ne 0 ]; then
    echo "ENTITLEMENT_AMEND_DB_TEST: FAIL status=unexpected_exit phase=${phase:-unknown}" >&2
  fi
  return "$status"
}
trap cleanup EXIT

printf '%s' 'disposable-postgres-admin-password' >"$postgres_password_file"
printf '%s' 'disposable-postgres-migration-password' >"$migration_password_file"
printf '%s' 'disposable-postgres-app-password' >"$app_password_file"
printf '%s' 'disposable-postgres-worker-password' >"$worker_password_file"
printf '%s' 'disposable-postgres-audit-password' >"$audit_password_file"
printf '%s' 'disposable-postgres-research-password' >"$research_password_file"
printf '%s' 'disposable-postgres-admin-role-password' >"$admin_password_file"
chmod 0600 "$postgres_password_file" "$migration_password_file" "$app_password_file" \
  "$worker_password_file" "$audit_password_file" "$research_password_file" "$admin_password_file"
printf '%s\n' 'POSTGRES_DB=entitlement_amend_test' >"$env_file"
chmod 0600 "$env_file"

cat >"$compose_file" <<YAML
services:
  postgres:
    image: $postgres_image
    init: true
    environment:
      POSTGRES_USER: postgres
      POSTGRES_DB: entitlement_amend_test
      POSTGRES_PASSWORD_FILE: /run/secrets/postgres_password
    secrets:
      - source: postgres_password
        target: postgres_password
      - source: migration_password
        target: db_migration_owner_password
      - source: app_password
        target: db_app_password
      - source: worker_password
        target: db_worker_password
      - source: audit_password
        target: db_audit_password
      - source: research_password
        target: db_research_password
      - source: admin_password
        target: db_admin_password
    volumes:
      - type: bind
        source: $bootstrap_roles
        target: /fixture/bootstrap-roles.sh
        read_only: true
    tmpfs:
      - /var/lib/postgresql
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres -d entitlement_amend_test"]
      interval: 1s
      timeout: 2s
      retries: 30
    networks: [amend-test]
  db-migrate:
    image: $postgres_image
    entrypoint: ["/bin/sh"]
    environment:
      DB_HOST: postgres
      DB_PORT: "5432"
      DB_NAME: entitlement_amend_test
      DB_USER: migration_owner
      DB_PASSWORD_FILE: /run/secrets/db_migration_owner_password
    secrets:
      - source: migration_password
        target: db_migration_owner_password
    networks: [amend-test]
secrets:
  postgres_password:
    file: $postgres_password_file
  migration_password:
    file: $migration_password_file
  app_password:
    file: $app_password_file
  worker_password:
    file: $worker_password_file
  audit_password:
    file: $audit_password_file
  research_password:
    file: $research_password_file
  admin_password:
    file: $admin_password_file
networks:
  amend-test:
    name: ${project}_private
    internal: true
YAML
chmod 0600 "$compose_file"

compose() {
  docker compose -p "$project" --env-file "$env_file" -f "$compose_file" "$@"
}

if ! compose up -d postgres >"$tmp/compose-up.out" 2>"$tmp/compose-up.err"; then
  blocked postgres_start
fi
ready=0
for _ in $(seq 1 30); do
  if compose exec -T postgres pg_isready -U postgres -d entitlement_amend_test \
      >"$tmp/ready.out" 2>"$tmp/ready.err"; then
    ready=1
    break
  fi
  sleep 1
done
[ "$ready" -eq 1 ] || blocked postgres_not_ready

if ! compose exec -T postgres sh -ec \
    'DB_HOST=127.0.0.1 DB_PORT=5432 DB_NAME=entitlement_amend_test DB_ADMIN_USER=postgres exec /bin/bash /fixture/bootstrap-roles.sh' \
    >"$tmp/bootstrap.out" 2>"$tmp/bootstrap.err"; then
  blocked role_bootstrap_failed
fi

admin_sql() {
  local sql=$1
  compose exec -T postgres sh -ec \
    'export PGPASSWORD="$(cat /run/secrets/postgres_password)"; exec psql -X --no-password -v ON_ERROR_STOP=1 -qAt -F "|" -h 127.0.0.1 -U postgres -d entitlement_amend_test' \
    >"$tmp/admin.out" 2>"$tmp/admin.err" <<<"$sql" || fail
}

admin_query() {
  admin_sql "$1"
  sed -n '1,$p' "$tmp/admin.out"
}

migration_psql() {
  compose exec -T postgres sh -ec \
    'export PGPASSWORD="$(cat /run/secrets/db_migration_owner_password)"; exec psql -X --no-password -v ON_ERROR_STOP=1 -qAt -F "|" -h 127.0.0.1 -U migration_owner -d entitlement_amend_test "$@"' \
    migration-owner-psql "$@"
}

role_sql() {
  local role=$1 secret_path=$2 sql=$3
  compose exec -T postgres sh -ec \
    'role=$1; secret_path=$2; export PGPASSWORD="$(cat "$secret_path")"; exec psql -X --no-password -v ON_ERROR_STOP=1 -qAt -F "|" -h 127.0.0.1 -U "$role" -d entitlement_amend_test' \
    role-psql "$role" "$secret_path" >"$tmp/role.out" 2>"$tmp/role.err" <<<"$sql"
}

run_migration_file() {
  local migration_file=$1
  if head -n 1 "$migration_file" | grep -Fx -- '-- no-transaction' >/dev/null; then
    migration_psql <"$migration_file" >"$tmp/migration.out" 2>"$tmp/migration.err"
  else
    {
      printf 'BEGIN;\n'
      sed -n '1,$p' "$migration_file"
      printf '\nCOMMIT;\n'
    } | migration_psql >"$tmp/migration.out" 2>"$tmp/migration.err"
  fi
}

# Every repository up migration executes under the production bootstrap role
# model. Normal files use SQLx-equivalent per-file transactions; only declared
# no-transaction migrations run outside one for concurrent indexes.
phase=full_migration
for migration_file in "$migrations_dir"/*.up.sql; do
  run_migration_file "$migration_file" || blocked full_migration_failed
done

export LAGRANGE_COMPOSE_FILE="$compose_file"
export LAGRANGE_ENV_FILE="$env_file"
source "$amend_helper"
source "$db_helper"
db_init

assert_query() {
  local expected=$1 sql=$2 actual
  actual=$(admin_query "$sql")
  [ "$actual" = "$expected" ] || fail
}

# These snapshots are taken from the complete repository schema before the
# amendment exercises run.  An amendment is data-only: it must not alter the
# pre-existing audit policy inventory or table ACLs.
audit_policy_inventory_sql="SELECT policyname || '|' || cmd || '|' || array_to_string(roles, ',') FROM pg_policies WHERE schemaname = 'public' AND tablename = 'audit_logs' ORDER BY policyname;"
audit_grant_inventory_sql="SELECT grantee || '|' || privilege_type || '|' || is_grantable FROM information_schema.role_table_grants WHERE table_schema = 'public' AND table_name = 'audit_logs' ORDER BY grantee, privilege_type, is_grantable;"

assert_audit_access_baseline() {
  local actual_policies actual_grants
  actual_policies=$(admin_query "$audit_policy_inventory_sql")
  actual_grants=$(admin_query "$audit_grant_inventory_sql")
  [ "$actual_policies" = "$audit_policy_baseline" ] || fail
  [ "$actual_grants" = "$audit_grant_baseline" ] || fail
  assert_query 'true|true' "SELECT relrowsecurity::text, relforcerowsecurity::text FROM pg_class WHERE oid = 'public.audit_logs'::regclass;"
  assert_query 0 "SELECT count(*) FROM pg_policies WHERE schemaname = 'public' AND tablename = 'audit_logs' AND cmd IN ('UPDATE', 'DELETE', 'ALL');"
}

target_id='00000000-0000-4000-8000-000000000101'
old_manager='00000000-0000-4000-8000-000000000102'
current_owner='00000000-0000-4000-8000-000000000103'
other_owner='00000000-0000-4000-8000-000000000104'
other_entitlement='00000000-0000-4000-8000-000000000105'
duplicate_entitlement='00000000-0000-4000-8000-000000000106'
reference="$AMEND_DOCUMENT_REFERENCE"
release_a='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
release_b='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'

assert_target_old() {
  assert_query 1 "SELECT count(*) FROM public.data_entitlements WHERE id = '$target_id' AND contract_document_sha256 = '$AMEND_ORIGINAL_DOCUMENT_SHA256' AND effective_from = '$AMEND_ORIGINAL_FROM' AND managed_by = '$old_manager' AND status = 'ACTIVE';"
}

assert_target_new() {
  assert_query 1 "SELECT count(*) FROM public.data_entitlements WHERE id = '$target_id' AND contract_document_sha256 = '$AMEND_APPROVED_DOCUMENT_SHA256' AND effective_from = '$AMEND_APPROVED_FROM' AND managed_by = '$current_owner' AND status = 'ACTIVE';"
}

assert_audit_count() {
  assert_query "$1" "SELECT count(*) FROM public.audit_logs WHERE action = '$AMEND_AUDIT_ACTION' AND target_type = 'data_entitlement' AND target_id = '$target_id';"
}

phase=role_invariants
phase=role_owner
role_owner_actual=$(admin_query "SELECT pg_get_userbyid(c.relowner), r.rolbypassrls::text FROM pg_class AS c JOIN pg_roles AS r ON r.rolname = 'migration_owner' WHERE c.oid = 'public.audit_logs'::regclass;")
[ "$role_owner_actual" = 'migration_owner|false' ] || {
  echo "ENTITLEMENT_AMEND_DB_TEST: invariant role_owner=$role_owner_actual" >&2
  fail
}
phase=rls_flags
assert_query 'true|true' "SELECT relrowsecurity::text, relforcerowsecurity::text FROM pg_class WHERE oid = 'public.audit_logs'::regclass;"
phase=audit_policy_baseline
audit_policy_baseline=$(admin_query "$audit_policy_inventory_sql")
expected_audit_policy_baseline=$'audit_insert_audit_writer|INSERT|audit_writer\naudit_select_admin|SELECT|admin\naudit_select_app|SELECT|app\nauth_audit_log_insert_migration_owner|INSERT|migration_owner\nauth_audit_log_select_migration_owner|SELECT|migration_owner'
[ "$audit_policy_baseline" = "$expected_audit_policy_baseline" ] || fail
phase=audit_grant_baseline
audit_grant_baseline=$(admin_query "$audit_grant_inventory_sql")
[ -n "$audit_grant_baseline" ] || fail
phase=role_privileges
assert_query 'false|false|false|false|false' "SELECT has_table_privilege('app', 'public.audit_logs', 'INSERT')::text, has_table_privilege('app', 'public.audit_logs', 'UPDATE')::text, has_table_privilege('app', 'public.data_entitlements', 'UPDATE')::text, has_table_privilege('research_writer', 'public.audit_logs', 'SELECT')::text, has_table_privilege('research_writer', 'public.audit_logs', 'INSERT')::text;"
assert_query 'true|false|false' "SELECT has_table_privilege('audit_writer', 'public.audit_logs', 'INSERT')::text, has_table_privilege('audit_writer', 'public.audit_logs', 'UPDATE')::text, has_table_privilege('audit_writer', 'public.audit_logs', 'DELETE')::text;"
assert_query 0 "SELECT count(*) FROM pg_policies WHERE schemaname = 'public' AND tablename = 'audit_logs' AND cmd IN ('UPDATE', 'DELETE', 'ALL');"

# This must be the same principal and bypass setting used by the actual
# db-migrate transport, rather than an administrator connection which would
# conceal FORCE-RLS behavior.
phase=migration_runner_identity
migration_runner_identity=$(db_psql -qAt -F '|' <<'SQL'
SELECT current_user,
       (SELECT rolbypassrls::text FROM pg_roles WHERE rolname = current_user),
       pg_has_role(current_user, 'audit_writer', 'member')::text;
SQL
)
[ "$migration_runner_identity" = 'migration_owner|false|false' ] || {
  echo "ENTITLEMENT_AMEND_DB_TEST: invariant migration_runner=$migration_runner_identity" >&2
  fail
}

stage_pairs() {
  git show cdaffe6b:configs/data-rights/kis.entitlement.json >"$tmp/original-metadata.json"
  git show cdaffe6b:docs/decisions/0005-kis-personal-use-entitlement.md >"$tmp/original-document.md"
  git show 91a4ff68:configs/data-rights/kis.entitlement.json >"$tmp/current-metadata.json"
  git show 91a4ff68:docs/decisions/0005-kis-personal-use-entitlement.md >"$tmp/current-document.md"
  chmod 0600 "$tmp/original-metadata.json" "$tmp/original-document.md" \
    "$tmp/current-metadata.json" "$tmp/current-document.md"
}
stage_pairs

prepare_sql_globals() {
  local executing_revision=$1 correlation_input correlation_digest
  amend_target_id=$target_id
  amend_reference=$reference
  amend_current_owner=$current_owner
  amend_old_manager=$old_manager
  amend_old_metadata_hash=$AMEND_ORIGINAL_METADATA_SHA256
  amend_new_metadata_hash=$AMEND_APPROVED_METADATA_SHA256
  amend_old_document_hash=$AMEND_ORIGINAL_DOCUMENT_SHA256
  amend_new_document_hash=$AMEND_APPROVED_DOCUMENT_SHA256
  amend_old_from=$AMEND_ORIGINAL_FROM
  amend_new_from=$AMEND_APPROVED_FROM
  amend_until=$AMEND_EFFECTIVE_UNTIL
  amend_old_datasets=$AMEND_DATASETS
  amend_new_datasets=$AMEND_DATASETS
  amend_old_uses=$AMEND_USES
  amend_new_uses=$AMEND_USES
  amend_before_json=$(amend_json_for_audit "$tmp/original-metadata.json" \
    "$amend_old_metadata_hash" "$amend_old_document_hash" "$target_id" "$old_manager" \
    "$AMEND_APPROVED_REVISION" "$executing_revision")
  amend_after_json=$(amend_json_for_audit "$tmp/current-metadata.json" \
    "$amend_new_metadata_hash" "$amend_new_document_hash" "$target_id" "$current_owner" \
    "$AMEND_APPROVED_REVISION" "$executing_revision")
  amend_before_semantic_json=$(amend_json_semantic_for_replay "$amend_before_json")
  amend_after_semantic_json=$(amend_json_semantic_for_replay "$amend_after_json")
  correlation_input=$(mktemp "$tmp/correlation.XXXXXX") || fail
  chmod 0600 "$correlation_input"
  printf '%s\0' "$AMEND_AUDIT_ACTION" "$target_id" "$reference" \
    "$amend_old_metadata_hash" "$amend_new_metadata_hash" "$amend_old_document_hash" \
    "$amend_new_document_hash" "$amend_old_from" "$amend_new_from" "$old_manager" \
    "$current_owner" "$AMEND_APPROVED_REVISION" >"$correlation_input"
  correlation_digest=$(sha256sum -- "$correlation_input" | awk '{print $1}')
  rm -f -- "$correlation_input"
  amend_correlation_id="entitlement.approved_document.amended:v2:$correlation_digest"
}

run_amend_sql() {
  local mode=$1 output=$2 error=$3 sql_file result state rows audits
  sql_file="$output.sql"
  case "$mode" in
    check) amend_write_check_sql "$sql_file" ;;
    apply) amend_write_apply_sql "$sql_file" ;;
    *) fail ;;
  esac
  chmod 0600 "$sql_file"
  if ! result=$(amend_run_db "$mode" "$sql_file" "$output" "$error" 2>>"$error"); then
    return 1
  fi
  case "$mode" in
    check)
      state=$(printf '%s' "$result" | amend_db_row 1 5)
      rows=$(printf '%s' "$result" | amend_db_row 2 5)
      audits=$(printf '%s' "$result" | amend_db_row 4 5)
      case "$state" in ELIGIBLE|ALREADY_APPLIED) ;; *) return 1 ;; esac
      printf 'ENTITLEMENT_AMEND_CHECK: PASS state=%s rows=%s audits=%s mutations=0\n' "$state" "$rows" "$audits" >"$output"
      ;;
    apply)
      state=$(printf '%s' "$result" | amend_db_row 1)
      rows=$(printf '%s' "$result" | amend_db_row 2)
      audits=$(printf '%s' "$result" | amend_db_row 3)
      case "$state" in APPLIED|ALREADY_APPLIED) ;; *) return 1 ;; esac
      printf 'ENTITLEMENT_AMEND_APPLY: PASS state=%s rows=%s audits=%s\n' "$state" "$rows" "$audits" >"$output"
      ;;
  esac
}

run_amend_expected_failure() {
  local mode=$1 output=$2 error=$3
  if run_amend_sql "$mode" "$output" "$error"; then
    phase="${phase}_unexpected_success"
    fail
  fi
  if grep -Eq '[0-9a-f]{64}|repo://|00000000-0000-4000-8000-00000000010[1-6]' "$output" "$error"; then
    phase="${phase}_redaction"
    fail
  fi
}

reset_db() {
  admin_sql "TRUNCATE public.audit_logs, public.auth_audit_outbox, public.data_entitlements, public.user_roles, public.users CASCADE;
INSERT INTO public.roles (id, description) VALUES ('owner', 'fixture owner') ON CONFLICT (id) DO NOTHING;
INSERT INTO public.users (id, issuer, subject, email) VALUES
 ('$old_manager', 'fixture', 'old-manager', 'old@example.invalid'),
 ('$current_owner', 'fixture', 'current-owner', 'current@example.invalid'),
 ('$other_owner', 'fixture', 'other-owner', 'other@example.invalid');
INSERT INTO public.user_roles (user_id, role_id) VALUES ('$current_owner', 'owner');
INSERT INTO public.data_entitlements (
 id, contract_document_sha256, contract_reference, status, covered_datasets,
 covered_uses, effective_from, effective_until, managed_by
) VALUES
 ('$target_id', '$AMEND_ORIGINAL_DOCUMENT_SHA256', '$reference', 'ACTIVE',
  '$AMEND_DATASETS'::jsonb, '$AMEND_USES'::jsonb, '$AMEND_ORIGINAL_FROM', '$AMEND_EFFECTIVE_UNTIL', '$old_manager'),
 ('$other_entitlement', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  'fixture://other-reference', 'ACTIVE', '[\"krx_eod_bars\"]'::jsonb,
  '[\"dataset\"]'::jsonb, '2020-01-01', '$AMEND_EFFECTIVE_UNTIL', '$old_manager');"
}

# Full-migration F1 correction: 0039 already grants migration_owner the SELECT
# and INSERT policies required by the db_psql path. This proves the approved
# amendment works with the actual existing schema, without a bypass or a new
# migration/policy/grant.
phase=f1_existing_0039_path
reset_db
prepare_sql_globals "$release_a"
run_amend_sql apply "$tmp/f1-existing-0039.out" "$tmp/f1-existing-0039.err" || fail
grep -Fq 'ENTITLEMENT_AMEND_APPLY: PASS state=APPLIED rows=1 audits=1' "$tmp/f1-existing-0039.out" || fail
assert_target_new
assert_audit_count 1
assert_audit_access_baseline

# Valid check/apply under the actual role, then release-B replay. The immutable
# stored executor remains A while semantic fields/correlation remain stable.
phase=apply_and_cross_release_replay
reset_db
prepare_sql_globals "$release_a"
run_amend_sql check "$tmp/check-a.out" "$tmp/check-a.err" || fail
grep -Fq 'ENTITLEMENT_AMEND_CHECK: PASS state=ELIGIBLE rows=1 audits=0 mutations=0' "$tmp/check-a.out" || fail
run_amend_sql apply "$tmp/apply-a.out" "$tmp/apply-a.err" || fail
grep -Fq 'ENTITLEMENT_AMEND_APPLY: PASS state=APPLIED rows=1 audits=1' "$tmp/apply-a.out" || fail
assert_target_new
assert_audit_count 1
assert_query 1 "SELECT count(*) FROM public.data_entitlements WHERE id = '$other_entitlement' AND contract_document_sha256 = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';"
assert_query "$release_a" "SELECT after_json->>'executing_release_revision' FROM public.audit_logs WHERE action = '$AMEND_AUDIT_ACTION' AND target_id = '$target_id';"
assert_query 1 "SELECT count(*) FROM public.audit_logs WHERE action = '$AMEND_AUDIT_ACTION' AND correlation_id ~ '^entitlement[.]approved_document[.]amended:v2:[0-9a-f]{64}$';"
prepare_sql_globals "$release_b"
run_amend_sql check "$tmp/check-b.out" "$tmp/check-b.err" || fail
grep -Fq 'ENTITLEMENT_AMEND_CHECK: PASS state=ALREADY_APPLIED rows=1 audits=1 mutations=0' "$tmp/check-b.out" || fail
run_amend_sql apply "$tmp/apply-b.out" "$tmp/apply-b.err" || fail
grep -Fq 'ENTITLEMENT_AMEND_APPLY: PASS state=ALREADY_APPLIED rows=1 audits=1' "$tmp/apply-b.out" || fail
assert_audit_count 1
assert_query "$release_a" "SELECT after_json->>'executing_release_revision' FROM public.audit_logs WHERE action = '$AMEND_AUDIT_ACTION' AND target_id = '$target_id';"

# Same-amendment concurrent applies serialize on the existing reference lock.
phase=concurrent_apply
reset_db
prepare_sql_globals "$release_a"
run_amend_sql apply "$tmp/concurrent-a.out" "$tmp/concurrent-a.err" & pid_a=$!
run_amend_sql apply "$tmp/concurrent-b.out" "$tmp/concurrent-b.err" & pid_b=$!
status_a=0
status_b=0
wait "$pid_a" || status_a=$?
wait "$pid_b" || status_b=$?
[ "$status_a" -eq 0 ] && [ "$status_b" -eq 0 ] || fail
grep -Eq 'state=(APPLIED|ALREADY_APPLIED)' "$tmp/concurrent-a.out" || fail
grep -Eq 'state=(APPLIED|ALREADY_APPLIED)' "$tmp/concurrent-b.out" || fail
assert_target_new
assert_audit_count 1
assert_query 1 "SELECT count(*) FROM public.data_entitlements WHERE id = '$other_entitlement' AND contract_document_sha256 = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';"

# Every exact-current-row CAS predicate remains required.
phase=cas_guards
cas_cases=(
  "hash|UPDATE public.data_entitlements SET contract_document_sha256 = 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' WHERE id = '$target_id';"
  "reference|UPDATE public.data_entitlements SET contract_reference = 'fixture://wrong-reference' WHERE id = '$target_id';"
  "status|UPDATE public.data_entitlements SET status = 'REVOKED' WHERE id = '$target_id';"
  "datasets|UPDATE public.data_entitlements SET covered_datasets = '[\"krx_eod_bars\"]'::jsonb WHERE id = '$target_id';"
  "uses|UPDATE public.data_entitlements SET covered_uses = '[\"dataset\"]'::jsonb WHERE id = '$target_id';"
  "from|UPDATE public.data_entitlements SET effective_from = '2020-02-01' WHERE id = '$target_id';"
  "until|UPDATE public.data_entitlements SET effective_until = '9999-12-30' WHERE id = '$target_id';"
  "manager|UPDATE public.data_entitlements SET managed_by = '$other_owner' WHERE id = '$target_id';"
)
for cas_case in "${cas_cases[@]}"; do
  case_name=${cas_case%%|*}
  mutation=${cas_case#*|}
  reset_db
  admin_sql "$mutation"
  prepare_sql_globals "$release_a"
  run_amend_expected_failure apply "$tmp/cas-$case_name.out" "$tmp/cas-$case_name.err"
  assert_audit_count 0
done

# Target and Owner evidence fail closed before any audit is appended.
phase=target_and_owner_guards
reset_db
admin_sql "INSERT INTO public.data_entitlements (id, contract_document_sha256, contract_reference, status, covered_datasets, covered_uses, effective_from, effective_until, managed_by) VALUES ('$duplicate_entitlement', '$AMEND_ORIGINAL_DOCUMENT_SHA256', '$reference', 'ACTIVE', '$AMEND_DATASETS'::jsonb, '$AMEND_USES'::jsonb, '$AMEND_ORIGINAL_FROM', '$AMEND_EFFECTIVE_UNTIL', '$old_manager');"
prepare_sql_globals "$release_a"
run_amend_expected_failure apply "$tmp/duplicate.out" "$tmp/duplicate.err"
assert_audit_count 0

phase=target_missing_guard
reset_db
admin_sql "DELETE FROM public.data_entitlements WHERE id = '$target_id';"
prepare_sql_globals "$release_a"
run_amend_expected_failure apply "$tmp/missing-target.out" "$tmp/missing-target.err"
assert_audit_count 0

phase=multiple_owner_guard
reset_db
admin_sql "INSERT INTO public.user_roles (user_id, role_id) VALUES ('$other_owner', 'owner');"
prepare_sql_globals "$release_a"
run_amend_expected_failure apply "$tmp/multiple-owner.out" "$tmp/multiple-owner.err"
assert_audit_count 0

phase=missing_owner_guard
reset_db
admin_sql "DELETE FROM public.user_roles WHERE user_id = '$current_owner' AND role_id = 'owner';"
prepare_sql_globals "$release_a"
run_amend_expected_failure apply "$tmp/missing-owner.out" "$tmp/missing-owner.err"
assert_audit_count 0

phase=wrong_owner_guard
reset_db
prepare_sql_globals "$release_a"
amend_current_owner=$other_owner
run_amend_expected_failure apply "$tmp/wrong-owner.out" "$tmp/wrong-owner.err"
assert_audit_count 0

phase=wrong_old_manager_guard
reset_db
prepare_sql_globals "$release_a"
amend_old_manager=$other_owner
run_amend_expected_failure apply "$tmp/wrong-old-manager.out" "$tmp/wrong-old-manager.err"
assert_audit_count 0

# Audit insertion failure rolls the exact row update back.
phase=audit_rollback
reset_db
prepare_sql_globals "$release_a"
admin_sql "CREATE OR REPLACE FUNCTION public.fail_amend_audit() RETURNS trigger LANGUAGE plpgsql AS \$\$ BEGIN RAISE EXCEPTION 'fixture audit failure'; END; \$\$; CREATE TRIGGER fail_amend_audit BEFORE INSERT ON public.audit_logs FOR EACH ROW EXECUTE FUNCTION public.fail_amend_audit();"
run_amend_expected_failure apply "$tmp/audit-failure.out" "$tmp/audit-failure.err"
assert_target_old
assert_audit_count 0
admin_sql "DROP TRIGGER fail_amend_audit ON public.audit_logs; DROP FUNCTION public.fail_amend_audit();"

# A post-state is never accepted merely because hashes match. Missing,
# duplicate/conflicting, semantic, and executor-provenance changes all fail.
phase=missing_audit_rejection
reset_db
prepare_sql_globals "$release_a"
run_amend_sql apply "$tmp/missing-audit-first.out" "$tmp/missing-audit-first.err" || fail
admin_sql "DELETE FROM public.audit_logs WHERE action = '$AMEND_AUDIT_ACTION' AND target_id = '$target_id';"
prepare_sql_globals "$release_b"
run_amend_expected_failure apply "$tmp/missing-poststate-audit.out" "$tmp/missing-poststate-audit.err"
assert_target_new
assert_audit_count 0

phase=duplicate_audit_rejection
reset_db
prepare_sql_globals "$release_a"
run_amend_sql apply "$tmp/duplicate-audit-first.out" "$tmp/duplicate-audit-first.err" || fail
admin_sql "INSERT INTO public.audit_logs (action, actor_role, actor_user_id, target_type, target_id, before_json, after_json, reason, correlation_id) SELECT action, actor_role, actor_user_id, target_type, target_id, before_json, after_json, reason, correlation_id FROM public.audit_logs WHERE action = '$AMEND_AUDIT_ACTION' AND target_id = '$target_id';"
prepare_sql_globals "$release_b"
run_amend_expected_failure apply "$tmp/duplicate-audit.out" "$tmp/duplicate-audit.err"
assert_audit_count 2

phase=conflicting_audit_rejection
reset_db
prepare_sql_globals "$release_a"
run_amend_sql apply "$tmp/conflicting-audit-first.out" "$tmp/conflicting-audit-first.err" || fail
admin_sql "INSERT INTO public.audit_logs (action, actor_role, actor_user_id, target_type, target_id, before_json, after_json, reason, correlation_id) SELECT action, actor_role, actor_user_id, target_type, target_id, before_json, after_json, reason, 'fixture-conflicting-correlation' FROM public.audit_logs WHERE action = '$AMEND_AUDIT_ACTION' AND target_id = '$target_id';"
prepare_sql_globals "$release_b"
run_amend_expected_failure apply "$tmp/conflicting-audit.out" "$tmp/conflicting-audit.err"
assert_audit_count 2

phase=semantic_audit_rejection
reset_db
prepare_sql_globals "$release_a"
run_amend_sql apply "$tmp/semantic-audit-first.out" "$tmp/semantic-audit-first.err" || fail
admin_sql "UPDATE public.audit_logs SET after_json = after_json || '{\"fixture_changed\":true}'::jsonb WHERE action = '$AMEND_AUDIT_ACTION' AND target_id = '$target_id';"
prepare_sql_globals "$release_b"
run_amend_expected_failure apply "$tmp/semantic-audit.out" "$tmp/semantic-audit.err"
assert_audit_count 1

phase=malformed_executor_audit_rejection
reset_db
prepare_sql_globals "$release_a"
run_amend_sql apply "$tmp/malformed-executor-first.out" "$tmp/malformed-executor-first.err" || fail
admin_sql "UPDATE public.audit_logs SET after_json = jsonb_set(after_json, '{executing_release_revision}', '\"not-a-commit\"'::jsonb) WHERE action = '$AMEND_AUDIT_ACTION' AND target_id = '$target_id';"
prepare_sql_globals "$release_b"
run_amend_expected_failure apply "$tmp/malformed-executor.out" "$tmp/malformed-executor.err"
assert_audit_count 1

# A syntactically valid but rewritten executor is also rejected unless the
# immutable before/after audit provenance agrees on the original identity.
phase=forged_executor_audit_rejection
reset_db
prepare_sql_globals "$release_a"
run_amend_sql apply "$tmp/forged-executor-first.out" "$tmp/forged-executor-first.err" || fail
admin_sql "UPDATE public.audit_logs SET before_json = jsonb_set(before_json, '{executing_release_revision}', '\"$release_b\"'::jsonb) WHERE action = '$AMEND_AUDIT_ACTION' AND target_id = '$target_id';"
prepare_sql_globals "$release_b"
run_amend_expected_failure apply "$tmp/forged-executor.out" "$tmp/forged-executor.err"
assert_audit_count 1

# Existing 0039 permits migration_owner audit SELECT/INSERT for its auth-outbox
# duties. The amendment must leave that baseline alone; append-only behavior is
# still enforced by FORCE RLS for UPDATE/DELETE, and serving roles gain nothing.
phase=audit_append_only_and_role_denials
reset_db
prepare_sql_globals "$release_a"
run_amend_sql apply "$tmp/permissions-first.out" "$tmp/permissions-first.err" || fail
assert_audit_access_baseline
if ! db_psql >"$tmp/audit-update.out" 2>"$tmp/audit-update.err" <<'SQL'
UPDATE public.audit_logs SET reason = 'forbidden' WHERE action = 'entitlement.approved_document.amended';
SQL
then
  fail
fi
if ! db_psql >"$tmp/audit-delete.out" 2>"$tmp/audit-delete.err" <<'SQL'
DELETE FROM public.audit_logs WHERE action = 'entitlement.approved_document.amended';
SQL
then
  fail
fi
assert_audit_count 1
if role_sql app /run/secrets/db_app_password "INSERT INTO public.audit_logs (action) VALUES ('fixture.app_insert');"; then
  fail
fi
if role_sql research_writer /run/secrets/db_research_password "SELECT count(*) FROM public.audit_logs;"; then
  fail
fi
if role_sql audit_writer /run/secrets/db_audit_password "UPDATE public.audit_logs SET reason = 'forbidden';"; then
  fail
fi
if role_sql audit_writer /run/secrets/db_audit_password "DELETE FROM public.audit_logs;"; then
  fail
fi
assert_audit_access_baseline

# 0039's existing delivery function still works through its intended
# audit_writer SECURITY DEFINER invocation. It remains available after the
# amendment, without changing any policy, role membership, or table ACL.
phase=auth_audit_delivery_preserved
reset_db
auth_event_id='00000000-0000-4000-8000-000000000107'
admin_sql "INSERT INTO public.auth_audit_outbox (
 id, event_key, action, actor_role, actor_user_id, target_type, target_id, reason, created_at
) VALUES (
 '$auth_event_id', 'fixture-auth-audit-delivery', 'auth.fixture_delivery', 'system', NULL,
 'fixture', 'fixture', 'fixture', clock_timestamp()
);"
if ! role_sql audit_writer /run/secrets/db_audit_password \
    "SELECT delivered_count, failed_count FROM public.deliver_auth_audit_batch(1);"; then
  fail
fi
grep -Fxq '1|0' "$tmp/role.out" || fail
assert_query 1 "SELECT count(*) FROM public.audit_logs WHERE id = '$auth_event_id' AND action = 'auth.fixture_delivery';"
assert_audit_access_baseline

echo 'ENTITLEMENT_AMEND_DB_TEST: PASS (full migrated disposable PostgreSQL; actual migration_owner RLS path; production target untouched)'
