#!/usr/bin/env bash
# Actual PostgreSQL coverage for the narrow entitlement amendment.
#
# This test is self-contained: it creates a temporary Compose project with no
# published ports, named volumes, external networks, or production fallback.
# The amendment itself still reaches PostgreSQL only through scripts/ops/lib/db.sh.
set -euo pipefail

script_dir=$(cd "$(dirname "$0")" && pwd)
repo_root=$(cd "$script_dir/../.." && pwd)
entitlement="$repo_root/scripts/ops/provision-entitlement.sh"
db_helper="$repo_root/scripts/ops/lib/db.sh"
amend_helper="$repo_root/scripts/ops/lib/entitlement-amend.sh"

fail() { echo 'ENTITLEMENT_AMEND_DB_TEST: FAIL status=assertion' >&2; exit 1; }
blocked() { echo "ENTITLEMENT_AMEND_DB_TEST: BLOCKED status=$1" >&2; exit 2; }

[ -x "$entitlement" ] || fail
[ -x "$db_helper" ] || fail
[ -x "$amend_helper" ] || fail
command -v docker >/dev/null 2>&1 || blocked docker_missing
command -v jq >/dev/null 2>&1 || blocked jq_missing
command -v sha256sum >/dev/null 2>&1 || blocked sha256sum_missing
[ -z "${LAGRANGE_COMPOSE_FILE:-}" ] || blocked inherited_compose_target
[ -z "${LAGRANGE_ENV_FILE:-}" ] || blocked inherited_env_target

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
init_sql="$tmp/init.sql"
postgres_password_file="$tmp/postgres.password"
migration_password_file="$tmp/migration.password"
project=$(basename "$tmp" | tr '[:upper:]' '[:lower:]' | tr '.' '-')
cleanup() {
  if [ -f "$compose_file" ]; then
    docker compose -p "$project" --env-file "$env_file" -f "$compose_file" down --volumes --remove-orphans >/dev/null 2>&1 || true
  fi
  rm -rf -- "$tmp"
}
trap cleanup EXIT

printf '%s' 'disposable-postgres-admin-password' >"$postgres_password_file"
printf '%s' 'disposable-postgres-migration-password' >"$migration_password_file"
chmod 0600 "$postgres_password_file" "$migration_password_file"
printf '%s\n' 'POSTGRES_DB=entitlement_amend_test' >"$env_file"
chmod 0600 "$env_file"

cat >"$init_sql" <<'SQL'
CREATE ROLE migration_owner LOGIN PASSWORD 'disposable-postgres-migration-password';
CREATE TABLE public.users (
    id uuid PRIMARY KEY,
    issuer text NOT NULL,
    subject text NOT NULL,
    email text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (issuer, subject),
    UNIQUE (email)
);
CREATE TABLE public.user_roles (
    user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE CASCADE,
    role_id text NOT NULL,
    PRIMARY KEY (user_id, role_id)
);
CREATE TABLE public.data_entitlements (
    id uuid PRIMARY KEY,
    contract_document_sha256 text NOT NULL,
    contract_reference text NOT NULL,
    status text NOT NULL,
    covered_datasets jsonb NOT NULL,
    covered_uses jsonb NOT NULL,
    effective_from date NOT NULL,
    effective_until date NOT NULL,
    managed_by uuid NOT NULL REFERENCES public.users(id),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE public.audit_logs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    action text NOT NULL,
    actor_role text NOT NULL DEFAULT 'system',
    actor_user_id uuid REFERENCES public.users(id),
    target_type text,
    target_id text,
    before_json jsonb,
    after_json jsonb,
    reason text,
    correlation_id text,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
ALTER TABLE public.users OWNER TO migration_owner;
ALTER TABLE public.user_roles OWNER TO migration_owner;
ALTER TABLE public.data_entitlements OWNER TO migration_owner;
ALTER TABLE public.audit_logs OWNER TO migration_owner;
GRANT USAGE ON SCHEMA public TO migration_owner;
SQL
# The disposable image runs its init script as the postgres UID; this fixture
# contains only generated test credentials and is removed with the temp tree.
chmod 0444 "$init_sql"

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
    volumes:
      - type: bind
        source: $init_sql
        target: /docker-entrypoint-initdb.d/001-init.sql
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
  if compose exec -T postgres pg_isready -U postgres -d entitlement_amend_test >"$tmp/ready.out" 2>"$tmp/ready.err"; then
    ready=1
    break
  fi
  sleep 1
done
[ "$ready" -eq 1 ] || blocked postgres_not_ready

export LAGRANGE_COMPOSE_FILE="$compose_file"
export LAGRANGE_ENV_FILE="$env_file"

admin_sql() {
  local sql=$1
  compose exec -T postgres sh -ec \
    'export PGPASSWORD="$(cat /run/secrets/postgres_password)"; exec psql -X --no-password -v ON_ERROR_STOP=1 -At -F "\t" -U postgres -d entitlement_amend_test' \
    >"$tmp/admin.out" 2>"$tmp/admin.err" <<<"$sql" || fail
}

admin_query() {
  admin_sql "$1"
  cat "$tmp/admin.out"
}

source "$amend_helper"
source "$db_helper"
db_init

target_id='00000000-0000-4000-8000-000000000101'
old_manager='00000000-0000-4000-8000-000000000102'
current_owner='00000000-0000-4000-8000-000000000103'
other_owner='00000000-0000-4000-8000-000000000104'
other_entitlement='00000000-0000-4000-8000-000000000105'
duplicate_entitlement='00000000-0000-4000-8000-000000000106'
reference="$AMEND_DOCUMENT_REFERENCE"

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
    "$AMEND_APPROVED_REVISION" 57f34879eb93ac0f8723d64b701dfb3ee19302e9)
  amend_after_json=$(amend_json_for_audit "$tmp/current-metadata.json" \
    "$amend_new_metadata_hash" "$amend_new_document_hash" "$target_id" "$current_owner" \
    "$AMEND_APPROVED_REVISION" 57f34879eb93ac0f8723d64b701dfb3ee19302e9)
  correlation_digest=$(printf '%s\0' "$AMEND_AUDIT_ACTION" "$target_id" "$reference" \
    "$amend_old_metadata_hash" "$amend_new_metadata_hash" "$amend_old_document_hash" \
    "$amend_new_document_hash" "$amend_old_from" "$amend_new_from" "$old_manager" \
    "$current_owner" "$AMEND_APPROVED_REVISION" \
    57f34879eb93ac0f8723d64b701dfb3ee19302e9 | sha256sum | awk '{print $1}')
  amend_correlation_id="entitlement.approved_document.amended:v1:$correlation_digest"
}
prepare_sql_globals

run_amend() {
  local output=$1 error=$2 current_override=${3:-$current_owner} old_override=${4:-$old_manager}
  if [ "$(id -u)" -ne 0 ]; then
    # Production --apply remains root-only.  This branch invokes the exact
    # generated transaction through db.sh only for the disposable DB owned by
    # this test, while the root fence is exercised below and in the local
    # operator self-test.
    amend_current_owner=$current_override
    amend_old_manager=$old_override
    amend_write_apply_sql "$output.sql"
    chmod 0600 "$output.sql"
    local result state rows audits
    if ! result=$(amend_run_db apply "$output.sql" "$output" "$error" 2>>"$error"); then
      return 1
    fi
    state=$(printf '%s' "$result" | amend_db_row 1)
    rows=$(printf '%s' "$result" | amend_db_row 2)
    audits=$(printf '%s' "$result" | amend_db_row 3)
    [ -n "$state" ] && [ -n "$rows" ] && [ -n "$audits" ] || return 1
    case "$state" in
      APPLIED|ALREADY_APPLIED)
        printf 'ENTITLEMENT_AMEND_APPLY: PASS state=%s rows=%s audits=%s\n' "$state" "$rows" "$audits" >"$output"
        : >"$error"
        return 0
        ;;
      *) return 1 ;;
    esac
  fi
  if ! LAGRANGE_COMPOSE_FILE="$compose_file" LAGRANGE_ENV_FILE="$env_file" \
      "$entitlement" amend --apply \
      --current-metadata-file "$tmp/current-metadata.json" \
      --current-document-file "$tmp/current-document.md" \
      --original-metadata-file "$tmp/original-metadata.json" \
      --original-document-file "$tmp/original-document.md" \
      --target-id "$target_id" \
      --expected-old-manager "$old_override" \
      --current-owner "$current_override" \
      --amendment-revision "$AMEND_APPROVED_REVISION" \
      --executing-release-revision 57f34879eb93ac0f8723d64b701dfb3ee19302e9 \
      --confirm I_UNDERSTAND_AMEND_ACTIVE_ENTITLEMENT >"$output" 2>"$error"; then
    return 1
  fi
}

run_amend_expected_failure() {
  local output=$1 error=$2 current_override=${3:-$current_owner} old_override=${4:-$old_manager}
  if run_amend "$output" "$error" "$current_override" "$old_override"; then
    fail
  fi
  ! grep -Eq '[0-9a-f]{64}|repo://|00000000-0000-4000-8000-00000000010[1-5]' "$output" "$error" || fail
}

reset_db() {
  admin_sql "TRUNCATE public.audit_logs, public.data_entitlements, public.user_roles, public.users;
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

assert_query() {
  local expected=$1 sql=$2 actual
  actual=$(admin_query "$sql")
  [ "$actual" = "$expected" ] || fail
}

assert_target_old() {
  assert_query 1 "SELECT count(*) FROM public.data_entitlements WHERE id = '$target_id' AND contract_document_sha256 = '$AMEND_ORIGINAL_DOCUMENT_SHA256' AND effective_from = '$AMEND_ORIGINAL_FROM' AND managed_by = '$old_manager' AND status = 'ACTIVE';"
}

assert_target_new() {
  assert_query 1 "SELECT count(*) FROM public.data_entitlements WHERE id = '$target_id' AND contract_document_sha256 = '$AMEND_APPROVED_DOCUMENT_SHA256' AND effective_from = '$AMEND_APPROVED_FROM' AND managed_by = '$current_owner' AND status = 'ACTIVE';"
}

assert_audit_count() {
  assert_query "$1" "SELECT count(*) FROM public.audit_logs WHERE action = '$AMEND_AUDIT_ACTION' AND target_type = 'data_entitlement' AND target_id = '$target_id';"
}

# Valid update, genuinely read-only check, and complete replay.
reset_db
before_updated_at=$(admin_query "SELECT updated_at::text FROM public.data_entitlements WHERE id = '$target_id';")
check_output="$tmp/check.out"
check_error="$tmp/check.err"
if ! LAGRANGE_COMPOSE_FILE="$compose_file" LAGRANGE_ENV_FILE="$env_file" \
    "$entitlement" amend --check \
    --current-metadata-file "$tmp/current-metadata.json" --current-document-file "$tmp/current-document.md" \
    --original-metadata-file "$tmp/original-metadata.json" --original-document-file "$tmp/original-document.md" \
    --target-id "$target_id" --expected-old-manager "$old_manager" --current-owner "$current_owner" \
    --amendment-revision "$AMEND_APPROVED_REVISION" \
    --executing-release-revision 57f34879eb93ac0f8723d64b701dfb3ee19302e9 \
    >"$check_output" 2>"$check_error"; then
  fail
fi
grep -Fq 'ENTITLEMENT_AMEND_CHECK: PASS state=ELIGIBLE rows=1 audits=0 mutations=0' "$check_output" || fail
after_check_updated_at=$(admin_query "SELECT updated_at::text FROM public.data_entitlements WHERE id = '$target_id';")
[ "$before_updated_at" = "$after_check_updated_at" ] || fail
assert_target_old
assert_audit_count 0

if ! run_amend "$tmp/valid.out" "$tmp/valid.err"; then
  fail
fi
grep -Fq 'ENTITLEMENT_AMEND_APPLY: PASS state=APPLIED rows=1 audits=1' "$tmp/valid.out" || fail
assert_target_new
assert_audit_count 1
assert_query 1 "SELECT count(*) FROM public.data_entitlements WHERE id = '$other_entitlement' AND contract_document_sha256 = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';"
assert_query 1 "SELECT count(*) FROM public.audit_logs WHERE action = '$AMEND_AUDIT_ACTION' AND before_json->>'original_git_revision' = '$AMEND_ORIGINAL_REVISION' AND after_json->>'amendment_git_revision' = '$AMEND_APPROVED_REVISION' AND after_json->>'executing_release_revision' = '57f34879eb93ac0f8723d64b701dfb3ee19302e9';"
run_amend "$tmp/replay.out" "$tmp/replay.err" || fail
grep -Fq 'ENTITLEMENT_AMEND_APPLY: PASS state=ALREADY_APPLIED rows=1 audits=1' "$tmp/replay.out" || fail
assert_audit_count 1

# Two concurrent applies serialize on the same reference lock.
reset_db
run_amend "$tmp/concurrent-a.out" "$tmp/concurrent-a.err" & pid_a=$!
run_amend "$tmp/concurrent-b.out" "$tmp/concurrent-b.err" & pid_b=$!
status_a=0
status_b=0
wait "$pid_a" || status_a=$?
wait "$pid_b" || status_b=$?
[ "$status_a" -eq 0 ] && [ "$status_b" -eq 0 ] || fail
grep -Eq 'state=(APPLIED|ALREADY_APPLIED)' "$tmp/concurrent-a.out" || fail
grep -Eq 'state=(APPLIED|ALREADY_APPLIED)' "$tmp/concurrent-b.out" || fail
assert_target_new
assert_audit_count 1

# Every old CAS field is independently drifted and must fail closed.
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
  run_amend_expected_failure "$tmp/cas-$case_name.out" "$tmp/cas-$case_name.err"
  assert_audit_count 0
done

# Duplicate reference, missing target, missing/multiple Owner evidence, and a
# caller-supplied wrong current Owner are distinct fences.
reset_db
admin_sql "INSERT INTO public.data_entitlements (id, contract_document_sha256, contract_reference, status, covered_datasets, covered_uses, effective_from, effective_until, managed_by) VALUES ('$duplicate_entitlement', '$AMEND_ORIGINAL_DOCUMENT_SHA256', '$reference', 'ACTIVE', '$AMEND_DATASETS'::jsonb, '$AMEND_USES'::jsonb, '$AMEND_ORIGINAL_FROM', '$AMEND_EFFECTIVE_UNTIL', '$old_manager');"
run_amend_expected_failure "$tmp/duplicate.out" "$tmp/duplicate.err"
assert_audit_count 0

reset_db
admin_sql "DELETE FROM public.data_entitlements WHERE id = '$target_id';"
run_amend_expected_failure "$tmp/missing-target.out" "$tmp/missing-target.err"
assert_audit_count 0

reset_db
admin_sql "INSERT INTO public.user_roles (user_id, role_id) VALUES ('$other_owner', 'owner');"
run_amend_expected_failure "$tmp/multiple-owner.out" "$tmp/multiple-owner.err"
assert_audit_count 0

reset_db
admin_sql "DELETE FROM public.user_roles WHERE user_id = '$current_owner' AND role_id = 'owner';"
run_amend_expected_failure "$tmp/missing-owner.out" "$tmp/missing-owner.err"
assert_audit_count 0

reset_db
run_amend_expected_failure "$tmp/wrong-owner.out" "$tmp/wrong-owner.err" "$other_owner"
assert_audit_count 0

reset_db
run_amend_expected_failure "$tmp/wrong-old-manager.out" "$tmp/wrong-old-manager.err" "$current_owner" "$other_owner"
assert_audit_count 0

# A post-state without its exact audit is not blessed on replay.
reset_db
run_amend "$tmp/missing-audit-first.out" "$tmp/missing-audit-first.err" || fail
admin_sql "DELETE FROM public.audit_logs WHERE action = '$AMEND_AUDIT_ACTION' AND target_id = '$target_id';"
run_amend_expected_failure "$tmp/missing-poststate-audit.out" "$tmp/missing-poststate-audit.err"
assert_target_new
assert_audit_count 0

# An audit insertion failure rolls the row update back.
reset_db
admin_sql "CREATE OR REPLACE FUNCTION public.fail_amend_audit() RETURNS trigger LANGUAGE plpgsql AS \$\$ BEGIN RAISE EXCEPTION 'disposable audit failure'; END; \$\$; CREATE TRIGGER fail_amend_audit BEFORE INSERT ON public.audit_logs FOR EACH ROW EXECUTE FUNCTION public.fail_amend_audit();"
run_amend_expected_failure "$tmp/audit-failure.out" "$tmp/audit-failure.err"
assert_target_old
assert_audit_count 0
admin_sql "DROP TRIGGER fail_amend_audit ON public.audit_logs; DROP FUNCTION public.fail_amend_audit();"

# Metadata/document domains and forbidden rights-scope fields are rejected
# before any database call.
for mutation in \
  '.provider = "other"' \
  '.contract_document.document_hash.hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"' \
  '.contract_document.document_reference = "fixture://forbidden"' \
  '.covered_datasets = ["krx_eod_bars"]' \
  '.covered_uses = ["dataset"]' \
  '.covered_users = ["usr_other"]' \
  '.effective_from = "2016-08-30"' \
  '.effective_until = "2025-01-01"' \
  '.entitlement_id = "ent_forged"' \
  '.lifecycle = "PENDING"'; do
  forged="$tmp/forged-$(printf '%s' "$mutation" | sha256sum | awk '{print substr($1,1,8)}').json"
  jq "$mutation" "$tmp/current-metadata.json" >"$forged"
  chmod 0600 "$forged"
  if "$entitlement" amend --plan \
      --current-metadata-file "$forged" --current-document-file "$tmp/current-document.md" \
      --original-metadata-file "$tmp/original-metadata.json" --original-document-file "$tmp/original-document.md" \
      --target-id "$target_id" --expected-old-manager "$old_manager" --current-owner "$current_owner" \
      --amendment-revision "$AMEND_APPROVED_REVISION" \
      --executing-release-revision 57f34879eb93ac0f8723d64b701dfb3ee19302e9 \
      >"$tmp/scope.out" 2>"$tmp/scope.err"; then
    fail
  fi
done
cp "$tmp/current-document.md" "$tmp/forged-document.md"
printf '%s' 'forged' >>"$tmp/forged-document.md"
chmod 0600 "$tmp/forged-document.md"
if "$entitlement" amend --plan \
    --current-metadata-file "$tmp/current-metadata.json" --current-document-file "$tmp/forged-document.md" \
    --original-metadata-file "$tmp/original-metadata.json" --original-document-file "$tmp/original-document.md" \
    --target-id "$target_id" --expected-old-manager "$old_manager" --current-owner "$current_owner" \
    --amendment-revision "$AMEND_APPROVED_REVISION" \
    --executing-release-revision 57f34879eb93ac0f8723d64b701dfb3ee19302e9 \
    >"$tmp/forged-document.out" 2>"$tmp/forged-document.err"; then
  fail
fi

# Confirmation fence; no DB work is permitted before it.
if [ "$(id -u)" -eq 0 ]; then
  if "$entitlement" amend --apply \
      --current-metadata-file "$tmp/current-metadata.json" --current-document-file "$tmp/current-document.md" \
      --original-metadata-file "$tmp/original-metadata.json" --original-document-file "$tmp/original-document.md" \
      --target-id "$target_id" --expected-old-manager "$old_manager" --current-owner "$current_owner" \
      --amendment-revision "$AMEND_APPROVED_REVISION" \
      --executing-release-revision 57f34879eb93ac0f8723d64b701dfb3ee19302e9 \
      --confirm WRONG >"$tmp/confirm.out" 2>"$tmp/confirm.err"; then
    fail
  fi
  grep -Fq 'I_UNDERSTAND_AMEND_ACTIVE_ENTITLEMENT' "$tmp/confirm.out" "$tmp/confirm.err" || fail
fi

echo 'ENTITLEMENT_AMEND_DB_TEST: PASS (actual disposable PostgreSQL; production target untouched)'
