#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "$0")" && pwd)
repo_root=$(cd "$script_dir/../.." && pwd)
entitlement="$repo_root/scripts/ops/provision-entitlement.sh"
dataset="$repo_root/scripts/ops/register-dataset-version.sh"
db_helper="$repo_root/scripts/ops/lib/db.sh"
fail() { echo "OPERATOR_ATTESTATION_SELF_TEST: FAIL: $*" >&2; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT

[ -x "$entitlement" ] || fail "entitlement helper is not executable"
[ -x "$dataset" ] || fail "dataset helper is not executable"
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
  effective_until: "2026-12-31",
  lifecycle: "PENDING"
}' >"$metadata"
chmod 0600 "$metadata"
plan_output=$("$entitlement" register --plan --metadata-file "$metadata" --document-file "$doc" --managed-by 00000000-0000-4000-8000-000000000001 2>&1) || fail "entitlement plan failed: $plan_output"
grep -Fq 'status=PENDING' <<<"$plan_output" || fail 'entitlement plan did not remain PENDING'
! grep -Fq 'operator-controlled rights fixture' <<<"$plan_output" || fail 'entitlement plan leaked document content'

if [ "$(id -u)" -eq 0 ]; then
  if "$entitlement" activate --apply --entitlement-id 00000000-0000-4000-8000-000000000002 --managed-by 00000000-0000-4000-8000-000000000001 --activation-date 2026-08-18 --confirm WRONG >"$tmp/activation.out" 2>&1; then
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

rm -f -- "$curated/bars/market=kr/symbol=069500.KRX/year=2024/version=1/bars.parquet"
if "$dataset" --plan --manifest-file "$manifest" --dataset-id krx_eod_bars --dataset-version kis-20260818.1 --storage-path /data/curated --entitlement-id 00000000-0000-4000-8000-000000000004 --as-of-date 2026-08-18 --entitlement-reference operator-attestation://self-test/kis-readonly --curated-root "$curated" >"$tmp/dataset-missing.out" 2>&1; then
  fail 'dataset plan accepted a missing exact artifact'
fi
grep -Eq 'missing|differs|BLOCKED_EXTERNAL' "$tmp/dataset-missing.out" || fail 'missing artifact failure was not explained'

echo 'OPERATOR_ATTESTATION_SELF_TEST: PASS (local-only; no DB/Docker/KIS/network)'
