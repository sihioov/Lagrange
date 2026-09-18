#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "$0")" && pwd)
repo_root=$(cd "$script_dir/../.." && pwd)
entitlement="$repo_root/scripts/ops/provision-entitlement.sh"
entitlement_amend="$repo_root/scripts/ops/lib/entitlement-amend.sh"
dataset="$repo_root/scripts/ops/register-dataset-version.sh"
db_helper="$repo_root/scripts/ops/lib/db.sh"
fail() { echo "OPERATOR_ATTESTATION_SELF_TEST: FAIL: $*" >&2; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT

[ -x "$entitlement" ] || fail "entitlement helper is not executable"
[ -x "$entitlement_amend" ] || fail "entitlement amendment helper is not executable"
[ -x "$dataset" ] || fail "dataset helper is not executable"
for shell_script in "$entitlement" "$entitlement_amend" "$0"; do
  bash -n "$shell_script" || fail "shell syntax failed: $(basename "$shell_script")"
done
grep -Fq 'provision-entitlement.sh amend [--plan|--check|--apply]' "$entitlement" || fail 'amend mode is not exposed by the entitlement helper'
grep -Fq 'BEGIN READ ONLY;' "$entitlement_amend" || fail 'amend check is not read-only'
grep -Fq "SET LOCAL lock_timeout = '5s';" "$entitlement_amend" || fail 'amend lock timeout is not pinned'
grep -Fq "SET LOCAL statement_timeout = '15s';" "$entitlement_amend" || fail 'amend statement timeout is not pinned'
grep -Fq "I_UNDERSTAND_AMEND_ACTIVE_ENTITLEMENT" "$entitlement_amend" || fail 'amend confirmation guard is missing'
grep -Fq "entitlement.approved_document.amended" "$entitlement_amend" || fail 'amend audit action is missing'
grep -Fq 'run --rm --no-deps --entrypoint /bin/sh db-migrate' "$db_helper" || fail 'DB helper must use private db-migrate Compose image'
! grep -Fq '127.0.0.1' "$db_helper" || fail 'DB helper must not use a host PostgreSQL address'
grep -Fq 'export PGPASSWORD="$(cat "$DB_PASSWORD_FILE")"' "$db_helper" || fail 'container secret handoff is missing'

# Psql substitutions are permitted only in set_config statements outside
# PL/pgSQL.  A substitution inside a dollar-quoted body is not portable.
for sql_script in "$entitlement" "$dataset"; do
  if sed -n '/DO \$body\$/,/\$body\$/p' "$sql_script" | grep -Eq ":'[A-Za-z_][A-Za-z0-9_]*"; then
    fail "psql substitution remains inside PL/pgSQL: $sql_script"
  fi
done

doc="$tmp/rights.pdf"
printf '%s' 'operator-controlled rights fixture; never a real entitlement' >"$doc"
chmod 0600 "$doc"
# A rights window may legitimately end on 9999-12-31 -- that is the sentinel in
# configs/data-rights/kis.entitlement.json. The previous date(1) round-trip
# rejected exactly that value on UTC and negative-offset hosts, so pin the
# calendar rule here and run it under a timezone on each side of UTC. The
# fixture above now carries the same sentinel, so the plan path exercises it too.
for helper in "$entitlement" "$dataset"; do
  for tz in UTC Asia/Seoul America/Sao_Paulo; do
    for probe in '2020-01-31:0' '9999-12-31:0' '2020-02-29:0' '0001-01-01:0' \
                 '2020-02-30:1' '2021-02-29:1' '2020-13-01:1' '2020-00-10:1'; do
      probe_date=${probe%:*}
      probe_want=${probe#*:}
      if TZ=$tz bash -c "source <(sed -n '/^valid_date()/,/^}/p' \"\$1\"); valid_date \"\$2\"" \
        _ "$helper" "$probe_date"; then
        probe_got=0
      else
        probe_got=1
      fi
      [ "$probe_got" = "$probe_want" ] ||
        fail "valid_date($probe_date) in $(basename "$helper") under TZ=$tz returned $probe_got, wanted $probe_want"
    done
  done
done

# psql prints one line per result row for the whole file, and the advisory lock
# guarding each attestation transaction returns void -- an empty leading line.
# The previous NR==1 parser read that line, so --apply reported "database
# returned no entitlement row" after successfully committing: an operator saw a
# failure and a written row. Pin the parser against that exact shape.
for helper in "$entitlement" "$dataset"; do
  parse() { bash -c "source <(sed -n '/^db_row_field()/,/^}/p' \"\$1\"); shift; db_row_field \"\$@\"" _ "$helper" "$@"; }
  two_col=$(printf '\nrow-id\tPENDING\n')
  [ "$(printf '%s\n' "$two_col" | parse 1)" = row-id ] ||
    fail "db_row_field in $(basename "$helper") did not skip the empty advisory-lock line"
  [ "$(printf '%s\n' "$two_col" | parse 2)" = PENDING ] ||
    fail "db_row_field in $(basename "$helper") returned the wrong column"
  four_col=$(printf '\nrow-id\tREADY\tdeadbeef\t/curated\n')
  [ "$(printf '%s\n' "$four_col" | parse 3 4)" = deadbeef ] ||
    fail "db_row_field in $(basename "$helper") mishandled a four-column readback"
  [ -z "$(printf '\n\n' | parse 1)" ] ||
    fail "db_row_field in $(basename "$helper") invented a row from empty output"
done

doc_hash=$(sha256sum -- "$doc" | awk '{print $1}')
metadata="$tmp/kis-entitlement.json"
jq -n --arg hash "$doc_hash" '{
  schema_version: 1,
  provider: "kis",
  entitlement_id: "ent_self_test",
  contract_document: {
    document_hash: {algorithm: "SHA-256", hex: $hash},
    document_reference: "operator-attestation://self-test/kis-readonly"
  },
  covered_datasets: ["krx_eod_bars"],
  covered_uses: ["dataset", "recommendation", "backtest", "paper_view"],
  covered_users: ["usr_self_test"],
  effective_from: "2026-01-01",
  effective_until: "9999-12-31",
  lifecycle: "PENDING"
}' >"$metadata"
chmod 0600 "$metadata"
plan_output=$("$entitlement" register --plan --metadata-file "$metadata" --document-file "$doc" --managed-by 00000000-0000-4000-8000-000000000001 2>&1) || fail "entitlement plan failed: $plan_output"
grep -Fq 'status=PENDING' <<<"$plan_output" || fail 'entitlement plan did not remain PENDING'
! grep -Fq 'operator-controlled rights fixture' <<<"$plan_output" || fail 'entitlement plan leaked document content'

# The amendment path accepts only protected, byte-exact copies of the two
# approved Git pairs.  These copies are disposable test inputs; the checked-in
# source files are never chmodded or edited.
amend_fixture="$tmp/entitlement-amend"
mkdir -p "$amend_fixture"
git show cdaffe6b:configs/data-rights/kis.entitlement.json >"$amend_fixture/original-metadata.json"
git show cdaffe6b:docs/decisions/0005-kis-personal-use-entitlement.md >"$amend_fixture/original-document.md"
git show 91a4ff68:configs/data-rights/kis.entitlement.json >"$amend_fixture/current-metadata.json"
git show 91a4ff68:docs/decisions/0005-kis-personal-use-entitlement.md >"$amend_fixture/current-document.md"
chmod 0600 "$amend_fixture"/*
amend_args=(
  amend --plan
  --current-metadata-file "$amend_fixture/current-metadata.json"
  --current-document-file "$amend_fixture/current-document.md"
  --original-metadata-file "$amend_fixture/original-metadata.json"
  --original-document-file "$amend_fixture/original-document.md"
  --target-id 00000000-0000-4000-8000-000000000011
  --expected-old-manager 00000000-0000-4000-8000-000000000012
  --current-owner 00000000-0000-4000-8000-000000000013
  --amendment-revision 91a4ff68
  --executing-release-revision 57f34879eb93ac0f8723d64b701dfb3ee19302e9
)
amend_plan=$(
  "$entitlement" "${amend_args[@]}" 2>"$tmp/amend-plan.err"
) || fail 'valid amendment plan failed'
grep -Fq 'ENTITLEMENT_AMEND_PLAN: PASS metadata_pairs=2 documents=2 database_queries=0 mutations=0' <<<"$amend_plan" ||
  fail 'amendment plan did not report typed counts'
! grep -Fq 'repo://docs/decisions/0005-kis-personal-use-entitlement.md' <<<"$amend_plan$(<"$tmp/amend-plan.err")" ||
  fail 'amendment plan leaked the protected reference'
amend_plan_all="$amend_plan$(<"$tmp/amend-plan.err")"
! grep -Eq '[0-9a-f]{64}|00000000-0000-4000-8000-00000000001[123]' <<<"$amend_plan_all" ||
  fail 'amendment plan leaked a protected digest or UUID'

cp "$amend_fixture/current-metadata.json" "$amend_fixture/forged-metadata.json"
jq '.effective_from = "2016-08-30"' "$amend_fixture/forged-metadata.json" >"$tmp/forged-metadata.json"
chmod 0600 "$tmp/forged-metadata.json"
if "$entitlement" amend --plan \
    --current-metadata-file "$tmp/forged-metadata.json" \
    --current-document-file "$amend_fixture/current-document.md" \
    --original-metadata-file "$amend_fixture/original-metadata.json" \
    --original-document-file "$amend_fixture/original-document.md" \
    --target-id 00000000-0000-4000-8000-000000000011 \
    --expected-old-manager 00000000-0000-4000-8000-000000000012 \
    --current-owner 00000000-0000-4000-8000-000000000013 \
    --amendment-revision 91a4ff68 \
    --executing-release-revision 57f34879eb93ac0f8723d64b701dfb3ee19302e9 \
    >"$tmp/forged.out" 2>"$tmp/forged.err"; then
  fail 'forged amendment metadata unexpectedly passed'
fi
! grep -Eq '[0-9a-f]{64}|repo://|00000000-0000-4000-8000-00000000001[123]' "$tmp/forged.out" "$tmp/forged.err" ||
  fail 'forged amendment failure leaked protected values'

ln -s "$amend_fixture/current-metadata.json" "$amend_fixture/current-metadata-link.json"
if "$entitlement" amend --plan \
    --current-metadata-file "$amend_fixture/current-metadata-link.json" \
    --current-document-file "$amend_fixture/current-document.md" \
    --original-metadata-file "$amend_fixture/original-metadata.json" \
    --original-document-file "$amend_fixture/original-document.md" \
    --target-id 00000000-0000-4000-8000-000000000011 \
    --expected-old-manager 00000000-0000-4000-8000-000000000012 \
    --current-owner 00000000-0000-4000-8000-000000000013 \
    --amendment-revision 91a4ff68 \
    --executing-release-revision 57f34879eb93ac0f8723d64b701dfb3ee19302e9 \
    >"$tmp/link.out" 2>"$tmp/link.err"; then
  fail 'symlinked amendment input unexpectedly passed'
fi

chmod 0644 "$amend_fixture/current-document.md"
if "$entitlement" "${amend_args[@]}" >"$tmp/mode.out" 2>"$tmp/mode.err"; then
  fail '0644 amendment input unexpectedly passed'
fi
chmod 0600 "$amend_fixture/current-document.md"

if [ "$(id -u)" -eq 0 ]; then
  if "$entitlement" amend --apply \
      --current-metadata-file "$amend_fixture/current-metadata.json" \
      --current-document-file "$amend_fixture/current-document.md" \
      --original-metadata-file "$amend_fixture/original-metadata.json" \
      --original-document-file "$amend_fixture/original-document.md" \
      --target-id 00000000-0000-4000-8000-000000000011 \
      --expected-old-manager 00000000-0000-4000-8000-000000000012 \
      --current-owner 00000000-0000-4000-8000-000000000013 \
      --amendment-revision 91a4ff68 \
      --executing-release-revision 57f34879eb93ac0f8723d64b701dfb3ee19302e9 \
      --confirm WRONG >"$tmp/amend-confirm.out" 2>&1; then
    fail 'amendment apply accepted incorrect confirmation'
  fi
  grep -Fq 'I_UNDERSTAND_AMEND_ACTIVE_ENTITLEMENT' "$tmp/amend-confirm.out" ||
    fail 'amendment confirmation guard is missing'
else
  if "$entitlement" amend --apply \
      --current-metadata-file "$amend_fixture/current-metadata.json" \
      --current-document-file "$amend_fixture/current-document.md" \
      --original-metadata-file "$amend_fixture/original-metadata.json" \
      --original-document-file "$amend_fixture/original-document.md" \
      --target-id 00000000-0000-4000-8000-000000000011 \
      --expected-old-manager 00000000-0000-4000-8000-000000000012 \
      --current-owner 00000000-0000-4000-8000-000000000013 \
      --amendment-revision 91a4ff68 \
      --executing-release-revision 57f34879eb93ac0f8723d64b701dfb3ee19302e9 \
      --confirm I_UNDERSTAND_AMEND_ACTIVE_ENTITLEMENT >"$tmp/amend-root.out" 2>&1; then
    fail 'non-root amendment apply unexpectedly passed'
  fi
  grep -Fq -- '--apply must run as root' "$tmp/amend-root.out" ||
    fail 'amendment root fence is missing'
fi

if [ "$(id -u)" -eq 0 ]; then
  compose_env_fixture="$tmp/compose.env"
  printf '%s\n' 'POSTGRES_DB=lagrange' >"$compose_env_fixture"
  if LAGRANGE_ENV_FILE="$compose_env_fixture" "$entitlement" activate --apply --entitlement-id 00000000-0000-4000-8000-000000000002 --managed-by 00000000-0000-4000-8000-000000000001 --activation-date 2026-08-18 --confirm WRONG >"$tmp/activation.out" 2>&1; then
    fail 'activation accepted incorrect confirmation'
  fi
  grep -Fq 'I_UNDERSTAND_ACTIVATE_ENTITLEMENT' "$tmp/activation.out" || fail 'activation confirmation guard is missing'
fi

curated="$tmp/data/curated"
manifest="$curated/datasets/krx_eod_bars/version=1/manifest.json"
mkdir -p "$(dirname "$manifest")"
symbols=(069500 102110 229200 143850 133690 195930 192090 148070 114260 153130 132030)
artifacts='[]'
for symbol in "${symbols[@]}"; do
  for name_schema in 'bars.parquet|bars-v1' 'adjusted_bars.parquet|adjusted-bars-v1' 'total_return_bars.parquet|total-return-bars-v1'; do
    IFS='|' read -r file_name schema <<<"$name_schema"
    relative="bars/market=kr/symbol=$symbol.KRX/year=2024/version=1/$file_name"
    path="$curated/$relative"
    mkdir -p "$(dirname "$path")"
    printf 'PAR1PAR1' >"$path"
    hash=$(sha256sum -- "$path" | awk '{print $1}')
    size=$(stat -c '%s' -- "$path")
    artifacts=$(jq -c --arg path "$relative" --arg hash "sha256:$hash" --arg schema "$schema" --argjson size "$size" '. + [{path: $path, sha256: $hash, size_bytes: $size, schema: $schema}]' <<<"$artifacts")
  done
done
source_batches='[{"batch_id":"00000000-0000-4000-8000-000000000003","bars_file":"bars.json","bars_hash":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","actions_file":"corporate-actions.json","actions_hash":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}]'
canonical=$(jq -cn --arg dataset_id krx_eod_bars --arg capability TOTAL_RETURN_CAPABLE --arg created_at 2026-08-18T00:00:00Z --argjson source_batches "$source_batches" --argjson artifacts "$artifacts" '{dataset_id:$dataset_id,version:1,capability:$capability,created_at:$created_at,source_batches:$source_batches,artifacts:$artifacts,bar_count:11,action_count:0}')
manifest_hash=$(printf '%s' "$canonical" | sha256sum | awk '{print $1}')
jq -cn --argjson base "$canonical" --arg hash "sha256:$manifest_hash" '$base + {content_hash:$hash}' >"$manifest"

dataset_plan=$("$dataset" --plan --manifest-file "$manifest" --dataset-id krx_eod_bars --dataset-version kis-20260818.1 --storage-path /data/curated --entitlement-id 00000000-0000-4000-8000-000000000004 --as-of-date 2026-08-18 --entitlement-reference operator-attestation://self-test/kis-readonly --curated-root "$curated" 2>&1) || fail "dataset plan failed: $dataset_plan"
grep -Fq 'artifacts=33 parquet_files=33' <<<"$dataset_plan" || fail 'dataset plan did not attest exact ETF artifacts'

if [ "$(id -u)" -eq 0 ]; then
  env_fixture="$tmp/release.env"
  printf '%s\n' 'RESEARCH_ENTITLEMENT_REFERENCE=operator-attestation://self-test/kis-readonly' >"$env_fixture"
  chmod 0600 "$env_fixture"
  common_dataset_args=(
    --apply --manifest-file "$manifest" --dataset-id krx_eod_bars
    --dataset-version kis-20260818.1 --storage-path /data/curated
    --entitlement-id 00000000-0000-4000-8000-000000000004
    --as-of-date 2026-08-18
    --entitlement-reference operator-attestation://self-test/kis-readonly
    --curated-root "$curated" --write-env-file "$env_fixture"
  )
  if "$dataset" "${common_dataset_args[@]}" \
      --confirm I_UNDERSTAND_REGISTER_READY_DATASET >"$tmp/write-confirm.out" 2>&1; then
    fail 'dataset env write accepted a missing independent write confirmation'
  fi
  grep -Fq -- '--confirm-write I_UNDERSTAND_WRITE_RELEASE_PINS' "$tmp/write-confirm.out" ||
    fail 'dataset env write does not require its independent confirmation'

  if "$dataset" "${common_dataset_args[@]}" --confirm WRONG \
      --confirm-write I_UNDERSTAND_WRITE_RELEASE_PINS >"$tmp/register-confirm.out" 2>&1; then
    fail 'dataset apply accepted an incorrect READY registration confirmation'
  fi
  grep -Fq -- '--confirm I_UNDERSTAND_REGISTER_READY_DATASET' "$tmp/register-confirm.out" ||
    fail 'dataset apply registration confirmation was not kept independent'
fi

rm -f -- "$curated/bars/market=kr/symbol=069500.KRX/year=2024/version=1/bars.parquet"
if "$dataset" --plan --manifest-file "$manifest" --dataset-id krx_eod_bars --dataset-version kis-20260818.1 --storage-path /data/curated --entitlement-id 00000000-0000-4000-8000-000000000004 --as-of-date 2026-08-18 --entitlement-reference operator-attestation://self-test/kis-readonly --curated-root "$curated" >"$tmp/dataset-missing.out" 2>&1; then
  fail 'dataset plan accepted a missing exact artifact'
fi
grep -Eq 'missing|differs|BLOCKED_EXTERNAL' "$tmp/dataset-missing.out" || fail 'missing artifact failure was not explained'

echo 'OPERATOR_ATTESTATION_SELF_TEST: PASS (local-only; no DB/Docker/KIS/network)'
