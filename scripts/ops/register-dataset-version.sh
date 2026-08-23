#!/usr/bin/env bash
# Fail-closed curated dataset attestation and immutable READY registration.
set -euo pipefail

script_dir=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
source "$script_dir/lib/db.sh"
source "$script_dir/lib/dotenv.sh"

mode=plan
manifest_file=
curated_root=
dataset_id=
dataset_version=
storage_path=
entitlement_id=
as_of_date=
entitlement_reference=
env_file=
pin_file=
write_env_file=
confirmation=

usage() {
  cat <<'USAGE'
Usage:
  register-dataset-version.sh [--plan|--check|--apply]
    --manifest-file PATH --dataset-id ID --dataset-version VERSION
    --storage-path ABSOLUTE_RUNTIME_ROOT --entitlement-id DB_UUID
    --as-of-date YYYY-MM-DD [--curated-root PATH] [--env-file PATH]
    [--pin-file PATH] [--write-env-file PATH]

Plan is local-only. Check is DB read-only. Apply is root-only and requires
I_UNDERSTAND_REGISTER_READY_DATASET. It creates no KIS/order/account traffic.
USAGE
}

die() { echo "dataset-attestation: $*" >&2; exit 1; }
blocked() { echo "BLOCKED_EXTERNAL: dataset-attestation: $*" >&2; exit 2; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --plan) mode=plan; shift ;;
    --check) mode=check; shift ;;
    --apply) mode=apply; shift ;;
    --manifest-file) [ "$#" -ge 2 ] || die '--manifest-file needs a path'; manifest_file=$2; shift 2 ;;
    --curated-root) [ "$#" -ge 2 ] || die '--curated-root needs a path'; curated_root=$2; shift 2 ;;
    --dataset-id) [ "$#" -ge 2 ] || die '--dataset-id needs a value'; dataset_id=$2; shift 2 ;;
    --dataset-version) [ "$#" -ge 2 ] || die '--dataset-version needs a value'; dataset_version=$2; shift 2 ;;
    --storage-path) [ "$#" -ge 2 ] || die '--storage-path needs a path'; storage_path=$2; shift 2 ;;
    --entitlement-id) [ "$#" -ge 2 ] || die '--entitlement-id needs a UUID'; entitlement_id=$2; shift 2 ;;
    --as-of-date) [ "$#" -ge 2 ] || die '--as-of-date needs a date'; as_of_date=$2; shift 2 ;;
    --entitlement-reference) [ "$#" -ge 2 ] || die '--entitlement-reference needs a value'; entitlement_reference=$2; shift 2 ;;
    --env-file) [ "$#" -ge 2 ] || die '--env-file needs a path'; env_file=$2; shift 2 ;;
    --pin-file) [ "$#" -ge 2 ] || die '--pin-file needs a path'; pin_file=$2; shift 2 ;;
    --write-env-file) [ "$#" -ge 2 ] || die '--write-env-file needs a path'; write_env_file=$2; shift 2 ;;
    --confirm) [ "$#" -ge 2 ] || die '--confirm needs a value'; confirmation=$2; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

[ "$mode" = plan ] || [ "$mode" = check ] || [ "$mode" = apply ] || die 'invalid mode'
[ "$mode" != apply ] || [ "$(id -u)" -eq 0 ] || die '--apply must run as root'
[ -n "$manifest_file" ] || blocked '--manifest-file is required'
[ -n "$dataset_id" ] || die '--dataset-id is required'
[ -n "$dataset_version" ] || die '--dataset-version is required'
[ -n "$storage_path" ] || die '--storage-path is required'
[ -n "$entitlement_id" ] || die '--entitlement-id is required'
[ -n "$as_of_date" ] || die '--as-of-date is required'
command -v jq >/dev/null 2>&1 || die 'jq is required'
command -v sha256sum >/dev/null 2>&1 || die 'sha256sum is required'

valid_uuid() { [[ "$1" =~ ^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-5][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}$ ]]; }
valid_date() {
  # Validate the calendar arithmetically rather than round-tripping through
  # date(1).  A rights window legitimately ends on 9999-12-31 -- that is the
  # open-ended sentinel in this repository's own
  # configs/data-rights/kis.entitlement.json -- and GNU date cannot represent
  # that day once a timezone offset is applied: `date -u -d 9999-12-31` fails
  # outright, and dropping -u only moves the failure to UTC and negative-offset
  # hosts (it passes only where the local offset is positive).  So the old
  # round-trip rejected the exact value the approved record carries, on exactly
  # the hosts CI and most operators run.
  local value=$1 year month day last_day
  [[ "$value" =~ ^([0-9]{4})-([0-9]{2})-([0-9]{2})$ ]] || return 1
  year=$((10#${BASH_REMATCH[1]}))
  month=$((10#${BASH_REMATCH[2]}))
  day=$((10#${BASH_REMATCH[3]}))
  [ "$year" -ge 1 ] || return 1
  [ "$month" -ge 1 ] && [ "$month" -le 12 ] || return 1
  case "$month" in
    1 | 3 | 5 | 7 | 8 | 10 | 12) last_day=31 ;;
    4 | 6 | 9 | 11) last_day=30 ;;
    2)
      if [ $((year % 4)) -eq 0 ] && { [ $((year % 100)) -ne 0 ] || [ $((year % 400)) -eq 0 ]; }; then
        last_day=29
      else
        last_day=28
      fi
      ;;
    *) return 1 ;;
  esac
  [ "$day" -ge 1 ] && [ "$day" -le "$last_day" ]
}
regular_file() {
  [ -f "$1" ] || blocked "$2 is missing"
  [ ! -L "$1" ] || die "$2 must not be a symlink"
  [ -s "$1" ] || die "$2 is empty"
}
safe_absolute() {
  [[ "$1" = /* ]] || die "$2 must be absolute"
  [[ "$1" != *$'\n'* && "$1" != *$'\r'* ]] || die "$2 contains a line ending"
  [[ "$1" != */../* && "$1" != */.. && "$1" != ../* ]] || die "$2 must not contain .."
}
jget() { jq -er "$1" "$manifest_file" 2>/dev/null || die 'manifest JSON is malformed'; }

regular_file "$manifest_file" manifest
valid_uuid "$entitlement_id" || die '--entitlement-id must be a UUID'
valid_date "$as_of_date" || die '--as-of-date must be a real date'
[[ "$dataset_id" =~ ^[a-z0-9][a-z0-9_.-]{0,95}$ ]] || die 'unsafe dataset id'
[[ "$dataset_version" =~ ^[A-Za-z0-9][A-Za-z0-9_.-]{0,95}$ ]] || die 'unsafe dataset version'
safe_absolute "$storage_path" storage_path

if [ -n "$env_file" ]; then
  [ -f "$env_file" ] || blocked "env file is missing: $env_file"
  [ ! -L "$env_file" ] || die 'env file must not be a symlink'
  dotenv_load "$env_file" || die 'env file is malformed'
  env_reference=$(dotenv_get RESEARCH_ENTITLEMENT_REFERENCE)
  if [ -n "$entitlement_reference" ] && [ -n "$env_reference" ] &&
     [ "$entitlement_reference" != "$env_reference" ]; then
    die 'entitlement reference conflicts with env file'
  fi
  [ -n "$entitlement_reference" ] || entitlement_reference=$env_reference
fi
[ -n "$entitlement_reference" ] || blocked 'entitlement reference is required'
[[ "$entitlement_reference" != *$'\\n'* && "$entitlement_reference" != *$'\\r'* ]] ||
  die 'entitlement reference contains a line ending'

manifest_dataset_id=$(jget '.dataset_id')
curated_version=$(jget '.version')
manifest_capability=$(jget '.capability')
manifest_hash_value=$(jget '.content_hash')
[ "$manifest_dataset_id" = "$dataset_id" ] || die 'manifest dataset_id mismatch'
[[ "$curated_version" =~ ^[1-9][0-9]*$ ]] || die 'manifest version must be positive'
[[ "$manifest_hash_value" =~ ^sha256:[0-9a-f]{64}$ ]] || die 'manifest hash must be lowercase sha256'
manifest_hash=${manifest_hash_value#sha256:}
canonical=$(jq -c '{dataset_id,version,capability,created_at,source_batches,artifacts,bar_count,action_count}' "$manifest_file") ||
  die 'manifest canonicalization failed'
computed_hash=$(printf '%s' "$canonical" | sha256sum | awk '{print $1}')
[ "$computed_hash" = "$manifest_hash" ] || die 'manifest self-hash does not match content_hash'
[[ "$manifest_capability" = PRICE_RETURN_ONLY || "$manifest_capability" = TOTAL_RETURN_CAPABLE ]] ||
  die 'manifest capability is invalid'
jq -e '
  (.source_batches | type == "array" and length > 0)
  and (.artifacts | type == "array" and length > 0)
  and (.bar_count | type == "number" and . >= 1)
  and (all(.source_batches[]?;
    (.batch_id | type == "string" and test("^[0-9a-fA-F-]{36}$"))
    and (.bars_file | type == "string" and test("^[^/[:cntrl:]]+$") and (contains("..") | not))
    and (.actions_file | type == "string" and test("^[^/[:cntrl:]]+$") and (contains("..") | not))
    and (.bars_hash | type == "string" and test("^sha256:[0-9a-f]{64}$"))
    and (.actions_hash | type == "string" and test("^sha256:[0-9a-f]{64}$"))
  ))
  and (all(.artifacts[]?;
    (.path | type == "string"
      and test("^(bars|corporate_actions)/[^[:cntrl:]]+$")
      and (startswith("/") | not)
      and (contains("..") | not)
      and (contains("\\") | not)
      and (split("/") | all(. != "" and . != "." and . != "..")))
    and (.sha256 | type == "string" and test("^sha256:[0-9a-f]{64}$"))
    and (.size_bytes | type == "number" and . >= 0 and floor == .)
    and (.schema | type == "string" and (
      . == "bars-v1"
      or . == "adjusted-bars-v1"
      or . == "total-return-bars-v1"
      or . == "corporate-actions-v2"
    ))
    and (
      (.schema == "bars-v1" and (.path | endswith("/bars.parquet")))
      or (.schema == "adjusted-bars-v1" and (.path | endswith("/adjusted_bars.parquet")))
      or (.schema == "total-return-bars-v1" and (.path | endswith("/total_return_bars.parquet")))
      or (.schema == "corporate-actions-v2" and (.path | endswith("/corporate_actions.parquet")))
    )
  ))
' "$manifest_file" >/dev/null 2>&1 || die 'manifest source refs are invalid'
source_refs=$(jq -c '[.source_batches[] | {batch_id,bars_file,bars_hash,actions_file,actions_hash}]' "$manifest_file")
artifact_refs=$(jq -c '[.artifacts[] | {path,sha256,size_bytes,schema}]' "$manifest_file")
action_count=$(jget '.action_count')
[[ "$action_count" =~ ^[0-9]+$ ]] || die 'manifest action_count must be a non-negative integer'
artifact_action_count=$(jq '[.artifacts[] | select(.schema == "corporate-actions-v2")] | length' "$manifest_file")
if [ "$action_count" -eq 0 ] && [ "$artifact_action_count" -ne 0 ]; then
  die 'manifest has corporate-action artifacts but action_count is zero'
fi
if [ "$action_count" -gt 0 ] && [ "$artifact_action_count" -eq 0 ]; then
  blocked 'manifest has actions but no corporate-action artifact inventory'
fi

if [ -z "$curated_root" ]; then
  manifest_dir=$(cd "$(dirname "$manifest_file")" && pwd)
  curated_root=$(cd "$manifest_dir/../../.." && pwd)
else
  safe_absolute "$curated_root" curated_root
  curated_root=$(cd "$curated_root" 2>/dev/null && pwd) || die 'curated root is unavailable'
fi
version_dir=$curated_root/datasets/$manifest_dataset_id/version=$curated_version
expected_manifest=$version_dir/manifest.json
actual_manifest=$(cd "$(dirname "$manifest_file")" && pwd)/$(basename "$manifest_file")
[ "$actual_manifest" = "$expected_manifest" ] || die 'manifest is not at canonical curated path'
[ -d "$version_dir" ] || blocked 'curated version directory is missing'
[ ! -L "$curated_root" ] && [ ! -L "$version_dir" ] || die 'curated path must not be a symlink'
if find "$version_dir" -type l -print -quit | grep -q .; then
  die 'curated version contains a symlink'
fi
curated_root=$(cd "$curated_root" && pwd -P) || die 'curated root is unavailable'
expected_paths=$(jq -r '.artifacts[].path' "$manifest_file" | LC_ALL=C sort)
[ -n "$expected_paths" ] || blocked 'manifest has no curated output artifacts'

# Verify only the exact files named by the manifest.  A broad recursive
# version glob is deliberately not sufficient: it could count an unrelated
# instrument or a stale generation and still produce a READY row.
while IFS=$'\t' read -r artifact_path artifact_hash artifact_size artifact_schema; do
  [ -n "$artifact_path" ] || die 'manifest contains an empty artifact path'
  [[ "$artifact_path" != /* && "$artifact_path" != *$'\n'* && "$artifact_path" != *$'\r'* ]] ||
    die 'artifact path is not a safe relative path'
  [[ "$artifact_path" != *'..'* && "$artifact_path" != *'\\'* ]] ||
    die 'artifact path contains an unsafe component'
  artifact_file=$curated_root/$artifact_path
  artifact_real=$(realpath -e -- "$artifact_file") || blocked "curated artifact is missing: $artifact_path"
  [ "$artifact_real" = "$artifact_file" ] || die "curated artifact path contains a symlink: $artifact_path"
  [ -f "$artifact_file" ] && [ ! -L "$artifact_file" ] || die "curated artifact is not a regular file: $artifact_path"
  actual_size=$(stat -c '%s' -- "$artifact_file") || die "cannot stat curated artifact: $artifact_path"
  [ "$actual_size" = "$artifact_size" ] || die "curated artifact size mismatch: $artifact_path"
  actual_hash=$(sha256sum -- "$artifact_file" | awk '{print $1}')
  [ "$actual_hash" = "${artifact_hash#sha256:}" ] || die "curated artifact hash mismatch: $artifact_path"
  header=$(head -c 4 -- "$artifact_file" | od -An -tx1 | tr -d '[:space:]')
  footer=$(tail -c 4 -- "$artifact_file" | od -An -tx1 | tr -d '[:space:]')
  [ "$header" = 50415231 ] && [ "$footer" = 50415231 ] ||
    die "curated artifact is not a complete Parquet file: $artifact_path"
  case "$artifact_schema" in
    bars-v1) expected_name=bars.parquet ;;
    adjusted-bars-v1) expected_name=adjusted_bars.parquet ;;
    total-return-bars-v1) expected_name=total_return_bars.parquet ;;
    corporate-actions-v2) expected_name=corporate_actions.parquet ;;
    *) die "unsupported curated artifact schema: $artifact_path" ;;
  esac
  [ "${artifact_path##*/}" = "$expected_name" ] || die "artifact/schema filename mismatch: $artifact_path"
  if [ "$dataset_id" = krx_eod_bars ]; then
    case "$artifact_path" in
      bars/market=kr/symbol=*.KRX/year=*/version=$curated_version/*.parquet|\
      corporate_actions/market=kr/symbol=*.KRX/year=*/version=$curated_version/corporate_actions.parquet) ;;
      *) die "KR ETF artifact is outside the exact KR partition layout: $artifact_path" ;;
    esac
  fi
done < <(jq -r '.artifacts[] | [.path,.sha256,.size_bytes,.schema] | @tsv' "$manifest_file")

actual_paths=$(
  {
    find "$curated_root/bars" -type f -path "*/version=$curated_version/*.parquet" \
      -printf 'bars/%P\n' 2>/dev/null || :
    find "$curated_root/corporate_actions" -type f -path "*/version=$curated_version/*.parquet" \
      -printf 'corporate_actions/%P\n' 2>/dev/null || :
  } | LC_ALL=C sort
)
[ "$actual_paths" = "$expected_paths" ] ||
  die 'curated output files differ from the exact manifest artifact set'

# The launch dataset is the fixed 11-instrument ETF universe.  Require every
# symbol and its three price-return partitions for every year in this version;
# an unrelated file elsewhere in curated_root must never satisfy this gate.
if [ "$dataset_id" = krx_eod_bars ]; then
  expected_symbols=(069500 102110 229200 143850 133690 195930 192090 148070 114260 153130 132030)
  bars_market_root=$curated_root/bars/market=kr
  [ -d "$bars_market_root" ] || blocked 'KR ETF curated bars market partition is missing'
  for symbol in "${expected_symbols[@]}"; do
    symbol_root=$bars_market_root/symbol=${symbol}.KRX
    version_dirs=$(find "$symbol_root" -mindepth 2 -maxdepth 2 -type d \
      -name "version=$curated_version" -print 2>/dev/null | sort)
    [ -n "$version_dirs" ] || blocked "KR ETF symbol/version partition is missing: $symbol"
    while IFS= read -r version_dir_path; do
      for file_name in bars.parquet adjusted_bars.parquet total_return_bars.parquet; do
        rel=${version_dir_path#$curated_root/}/$file_name
        grep -Fxq "$rel" <<<"$expected_paths" || die "manifest misses required ETF artifact: $rel"
      done
    done <<<"$version_dirs"
  done
  while IFS= read -r symbol_root; do
    symbol_name=${symbol_root##*/symbol=}
    case "$symbol_name" in
      069500.KRX|102110.KRX|229200.KRX|143850.KRX|133690.KRX|195930.KRX|192090.KRX|148070.KRX|114260.KRX|153130.KRX|132030.KRX) ;;
      *)
        if find "$symbol_root" -mindepth 2 -maxdepth 2 -type d -name "version=$curated_version" -print -quit | grep -q .; then
          die "unexpected ETF symbol in curated generation: $symbol_name"
        fi
        ;;
    esac
  done < <(find "$bars_market_root" -mindepth 1 -maxdepth 1 -type d -name 'symbol=*.KRX' -print | sort)
fi

artifact_count=$(jq '.artifacts | length' "$manifest_file")
parquet_count=$(grep -c '\.parquet$' <<<"$actual_paths")

printf 'DATASET_ATTESTATION_PLAN: local manifest/files verified\n'
printf '  dataset_id=%s db_version=%s curated_version=%s\n' "$dataset_id" "$dataset_version" "$curated_version"
printf '  manifest_sha256=%s artifacts=%s parquet_files=%s entitlement_reference=%s\n' \
  "$manifest_hash" "$artifact_count" "$parquet_count" "$entitlement_reference"
[ "$mode" = plan ] && {
  echo 'PLAN_ONLY: no database, KIS, account/order, network, or file write made'
  exit 0
}
if [ -n "$write_env_file" ]; then
  [ "$mode" = apply ] || die '--write-env-file requires --apply'
  [ "$confirmation" = I_UNDERSTAND_WRITE_RELEASE_PINS ] ||
    die '--write-env-file requires --confirm I_UNDERSTAND_WRITE_RELEASE_PINS'
  [ -f "$write_env_file" ] && [ ! -L "$write_env_file" ] || die 'env output must be an existing regular file'
  [ "$(stat -c '%a' "$write_env_file")" = 600 ] || die 'env output must be mode 0600'
fi
[ "$mode" = apply ] && [ "$confirmation" = I_UNDERSTAND_REGISTER_READY_DATASET ] ||
  [ "$mode" = check ] || die '--apply requires --confirm I_UNDERSTAND_REGISTER_READY_DATASET'

db_init
sql_file=$(mktemp /tmp/lagrange-dataset-attestation.XXXXXX.sql)
chmod 0600 "$sql_file"
trap 'rm -f -- "$sql_file"' EXIT
if [ "$mode" = check ]; then
cat >"$sql_file" <<'SQL'
BEGIN;
SET LOCAL statement_timeout = '30s';
SELECT pg_catalog.set_config('operator.dataset.entitlement_id', :'entitlement_id', true) AS _set_entitlement_id \gset
SELECT pg_catalog.set_config('operator.dataset.as_of_date', :'as_of_date', true) AS _set_as_of_date \gset
SELECT pg_catalog.set_config('operator.dataset.entitlement_reference', :'entitlement_reference', true) AS _set_entitlement_reference \gset
SELECT pg_catalog.set_config('operator.dataset.dataset_id', :'dataset_id', true) AS _set_dataset_id \gset
SELECT pg_catalog.set_config('operator.dataset.dataset_version', :'dataset_version', true) AS _set_dataset_version \gset
SELECT pg_catalog.set_config('operator.dataset.manifest_hash', :'manifest_hash', true) AS _set_manifest_hash \gset
SELECT pg_catalog.set_config('operator.dataset.storage_path', :'storage_path', true) AS _set_storage_path \gset
SELECT pg_catalog.set_config('operator.dataset.source_refs', :'source_refs', true) AS _set_source_refs \gset
DO $body$
DECLARE v_ref jsonb; v_count integer;
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM public.data_entitlements
     WHERE id = current_setting('operator.dataset.entitlement_id')::uuid AND status = 'ACTIVE'
       AND effective_from <= current_setting('operator.dataset.as_of_date')::date
       AND effective_until >= current_setting('operator.dataset.as_of_date')::date
       AND contract_reference = current_setting('operator.dataset.entitlement_reference')
       AND covered_datasets @> pg_catalog.jsonb_build_array(current_setting('operator.dataset.dataset_id'))
       AND covered_uses @> '["recommendation","backtest","paper_view"]'::jsonb
  ) THEN RAISE EXCEPTION 'active entitlement coverage failed' USING ERRCODE = '42501'; END IF;
  FOR v_ref IN SELECT value FROM pg_catalog.jsonb_array_elements(current_setting('operator.dataset.source_refs')::jsonb) LOOP
    SELECT count(*) INTO v_count FROM public.data_batches b
     WHERE b.provider = 'KRX' AND b.market = 'KR' AND b.fetch_mode = 'credentialed'
       AND b.source_batch_id = (v_ref->>'batch_id')::uuid
       AND ((b.source_file_name = v_ref->>'bars_file' AND b.content_sha256 = substring(v_ref->>'bars_hash' from 8))
         OR (b.source_file_name = v_ref->>'actions_file' AND b.content_sha256 = substring(v_ref->>'actions_hash' from 8)));
    IF v_count <> 2 THEN RAISE EXCEPTION 'normalized DB lineage is incomplete' USING ERRCODE = '55000'; END IF;
  END LOOP;
END
$body$;
SELECT id::text, status, manifest_sha256, storage_path
  FROM public.dataset_versions
 WHERE dataset_id = current_setting('operator.dataset.dataset_id')
   AND version = current_setting('operator.dataset.dataset_version')
   AND status = 'READY' AND manifest_sha256 = current_setting('operator.dataset.manifest_hash')
   AND storage_path = current_setting('operator.dataset.storage_path');
COMMIT;
SQL
else
cat >"$sql_file" <<'SQL'
BEGIN;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
SELECT pg_catalog.set_config('operator.dataset.entitlement_id', :'entitlement_id', true) AS _set_entitlement_id \gset
SELECT pg_catalog.set_config('operator.dataset.as_of_date', :'as_of_date', true) AS _set_as_of_date \gset
SELECT pg_catalog.set_config('operator.dataset.entitlement_reference', :'entitlement_reference', true) AS _set_entitlement_reference \gset
SELECT pg_catalog.set_config('operator.dataset.dataset_id', :'dataset_id', true) AS _set_dataset_id \gset
SELECT pg_catalog.set_config('operator.dataset.dataset_version', :'dataset_version', true) AS _set_dataset_version \gset
SELECT pg_catalog.set_config('operator.dataset.manifest_hash', :'manifest_hash', true) AS _set_manifest_hash \gset
SELECT pg_catalog.set_config('operator.dataset.storage_path', :'storage_path', true) AS _set_storage_path \gset
SELECT pg_catalog.set_config('operator.dataset.source_refs', :'source_refs', true) AS _set_source_refs \gset
SELECT pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(
  current_setting('operator.dataset.dataset_id') || ':' || current_setting('operator.dataset.dataset_version'), 0));
DO $body$
DECLARE v_ref jsonb; v_count integer; v_existing public.dataset_versions%ROWTYPE;
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM public.data_entitlements
     WHERE id = current_setting('operator.dataset.entitlement_id')::uuid AND status = 'ACTIVE'
       AND effective_from <= current_setting('operator.dataset.as_of_date')::date
       AND effective_until >= current_setting('operator.dataset.as_of_date')::date
       AND contract_reference = current_setting('operator.dataset.entitlement_reference')
       AND covered_datasets @> pg_catalog.jsonb_build_array(current_setting('operator.dataset.dataset_id'))
       AND covered_uses @> '["recommendation","backtest","paper_view"]'::jsonb
  ) THEN RAISE EXCEPTION 'active entitlement coverage failed' USING ERRCODE = '42501'; END IF;
  FOR v_ref IN SELECT value FROM pg_catalog.jsonb_array_elements(current_setting('operator.dataset.source_refs')::jsonb) LOOP
    SELECT count(*) INTO v_count FROM public.data_batches b
     WHERE b.provider = 'KRX' AND b.market = 'KR' AND b.fetch_mode = 'credentialed'
       AND b.source_batch_id = (v_ref->>'batch_id')::uuid
       AND ((b.source_file_name = v_ref->>'bars_file' AND b.content_sha256 = substring(v_ref->>'bars_hash' from 8))
         OR (b.source_file_name = v_ref->>'actions_file' AND b.content_sha256 = substring(v_ref->>'actions_hash' from 8)));
    IF v_count <> 2 THEN RAISE EXCEPTION 'normalized DB lineage is incomplete' USING ERRCODE = '55000'; END IF;
  END LOOP;
  SELECT * INTO v_existing FROM public.dataset_versions
   WHERE dataset_id = current_setting('operator.dataset.dataset_id')
     AND version = current_setting('operator.dataset.dataset_version') FOR UPDATE;
  IF FOUND THEN
    IF v_existing.status <> 'READY'
       OR v_existing.manifest_sha256 IS DISTINCT FROM current_setting('operator.dataset.manifest_hash')
       OR v_existing.storage_path IS DISTINCT FROM current_setting('operator.dataset.storage_path')
    THEN RAISE EXCEPTION 'dataset version has conflicting metadata' USING ERRCODE = '23505'; END IF;
  ELSE
    INSERT INTO public.dataset_versions (dataset_id, version, status, manifest_sha256, storage_path)
    VALUES (current_setting('operator.dataset.dataset_id'),
            current_setting('operator.dataset.dataset_version'), 'READY',
            current_setting('operator.dataset.manifest_hash'),
            current_setting('operator.dataset.storage_path'));
  END IF;
END
$body$;
INSERT INTO public.audit_logs (
  action, actor_role, actor_user_id, target_type, target_id, after_json, reason
)
SELECT 'dataset.version.registered', 'operator', e.managed_by, 'dataset_version', d.id::text,
       jsonb_build_object('dataset_id', d.dataset_id, 'version', d.version,
         'status', d.status, 'manifest_sha256', d.manifest_sha256,
         'storage_path', d.storage_path, 'entitlement_id', e.id,
         'source_batches', current_setting('operator.dataset.source_refs')::jsonb),
       'operator dataset attestation; source lineage is in manifest'
  FROM public.dataset_versions d
  JOIN public.data_entitlements e ON e.id = current_setting('operator.dataset.entitlement_id')::uuid
 WHERE d.dataset_id = current_setting('operator.dataset.dataset_id')
   AND d.version = current_setting('operator.dataset.dataset_version')
   AND NOT EXISTS (
     SELECT 1 FROM public.audit_logs a
      WHERE a.action = 'dataset.version.registered'
        AND a.target_type = 'dataset_version' AND a.target_id = d.id::text
        AND a.reason = 'operator dataset attestation; source lineage is in manifest'
   );
SELECT id::text, status, manifest_sha256, storage_path
  FROM public.dataset_versions
 WHERE dataset_id = current_setting('operator.dataset.dataset_id')
   AND version = current_setting('operator.dataset.dataset_version');
COMMIT;
SQL
fi
result=$(db_psql -qAt -F $'\t' -v entitlement_id="$entitlement_id" -v as_of_date="$as_of_date" \
  -v entitlement_reference="$entitlement_reference" -v dataset_id="$dataset_id" \
  -v dataset_version="$dataset_version" -v manifest_hash="$manifest_hash" \
  -v storage_path="$storage_path" -v source_refs="$source_refs" <"$sql_file") ||
  blocked 'DB attestation failed'
row_id=$(printf '%s\n' "$result" | awk -F '\t' 'NR==1 {print $1}')
row_status=$(printf '%s\n' "$result" | awk -F '\t' 'NR==1 {print $2}')
row_hash=$(printf '%s\n' "$result" | awk -F '\t' 'NR==1 {print $3}')
row_storage=$(printf '%s\n' "$result" | awk -F '\t' 'NR==1 {print $4}')
[ "$row_status" = READY ] || die 'database returned a non-READY row'
[ "$row_hash" = "$manifest_hash" ] || die 'database manifest hash differs'
[ "$row_storage" = "$storage_path" ] || die 'database storage path differs'

pin_text=$(cat <<PIN
RECOMMENDATION_DATASET_VERSION_ID=$row_id
RECOMMENDATION_DATASET_ID=$dataset_id
RECOMMENDATION_DATASET_VERSION=$dataset_version
RECOMMENDATION_CURATED_VERSION=$curated_version
RECOMMENDATION_DATASET_MANIFEST_SHA256=$manifest_hash
RESEARCH_ENTITLEMENT_REFERENCE=$entitlement_reference
PIN
)
if [ "$mode" = apply ] && [ -n "$pin_file" ]; then
  [ ! -L "$pin_file" ] || die 'pin file must not be a symlink'
  pin_dir=$(dirname "$pin_file")
  [ -d "$pin_dir" ] || die 'pin file parent is missing'
  tmp_pin=$(mktemp "$pin_dir/.pins.XXXXXX")
  chmod 0600 "$tmp_pin"
  printf '%s' "$pin_text" >"$tmp_pin"
  if [ -e "$pin_file" ]; then
    cmp -s "$tmp_pin" "$pin_file" || { rm -f "$tmp_pin"; die 'existing pin file conflicts'; }
    rm -f "$tmp_pin"
  else
    mv "$tmp_pin" "$pin_file"
  fi
fi
if [ "$mode" = apply ] && [ -n "$write_env_file" ]; then
  env_tmp=$(mktemp "$(dirname "$write_env_file")/.env.attest.XXXXXX")
  awk -v id="$row_id" -v did="$dataset_id" -v dv="$dataset_version" \
      -v cv="$curated_version" -v mh="$manifest_hash" -v er="$entitlement_reference" '
    BEGIN {
      v["RECOMMENDATION_DATASET_VERSION_ID"]="RECOMMENDATION_DATASET_VERSION_ID=" id;
      v["RECOMMENDATION_DATASET_ID"]="RECOMMENDATION_DATASET_ID=" did;
      v["RECOMMENDATION_DATASET_VERSION"]="RECOMMENDATION_DATASET_VERSION=" dv;
      v["RECOMMENDATION_CURATED_VERSION"]="RECOMMENDATION_CURATED_VERSION=" cv;
      v["RECOMMENDATION_DATASET_MANIFEST_SHA256"]="RECOMMENDATION_DATASET_MANIFEST_SHA256=" mh;
      v["RESEARCH_ENTITLEMENT_REFERENCE"]="RESEARCH_ENTITLEMENT_REFERENCE=" er;
    }
    /^[A-Za-z_][A-Za-z0-9_]*=/ {
      key=$0; sub(/=.*/, "", key);
      if (key in v) { print v[key]; seen[key]=1; next; }
    }
    { print }
    END { for (key in v) if (!(key in seen)) print v[key]; }
  ' "$write_env_file" >"$env_tmp"
  chmod 0600 "$env_tmp"
  mv "$env_tmp" "$write_env_file"
fi
printf 'DATASET_%s: PASS db_row=%s status=%s\n' \
  "$( [ "$mode" = check ] && printf CHECK || printf APPLY )" "$row_id" "$row_status"
printf '%s\n' "$pin_text"
