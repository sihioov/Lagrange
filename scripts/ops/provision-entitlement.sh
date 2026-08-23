#!/usr/bin/env bash
# Register redacted KIS rights metadata as PENDING, then perform a separate
# explicit PENDING -> ACTIVE transition. The source document is hashed only;
# its bytes are never stored, printed, or sent to KIS.
set -euo pipefail

script_dir=$(cd "$(dirname "$0")" && pwd)
source "$script_dir/lib/db.sh"

action=register
mode=plan
metadata_file=
document_file=
managed_by=
entitlement_id=
activation_date=
confirmation=
env_file=${LAGRANGE_ENV_FILE:-}

usage() {
  cat <<'USAGE'
Usage:
  provision-entitlement.sh register [--plan|--check|--apply]
    --metadata-file PATH --document-file PATH --managed-by OWNER_UUID
    [--env-file PATH]
  provision-entitlement.sh activate [--plan|--check|--apply]
    --entitlement-id DB_UUID --managed-by OWNER_UUID --activation-date YYYY-MM-DD
    [--env-file PATH]

Default: register --plan. Registration creates only PENDING. ACTIVE is a
separate explicit operation. No KIS, account, order, or provider call occurs.
Compose DB input: use --env-file PATH when the deployment env is not the
default; PostgreSQL is reached only through the db-migrate Compose service.
USAGE
}

die() { echo "entitlement: $*" >&2; exit 1; }
blocked() { echo "BLOCKED_EXTERNAL: entitlement: $*" >&2; exit 2; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    register|activate) action=$1; shift ;;
    --plan) mode=plan; shift ;;
    --check) mode=check; shift ;;
    --apply) mode=apply; shift ;;
    --metadata-file) [ "$#" -ge 2 ] || die '--metadata-file needs a path'; metadata_file=$2; shift 2 ;;
    --document-file) [ "$#" -ge 2 ] || die '--document-file needs a path'; document_file=$2; shift 2 ;;
    --managed-by) [ "$#" -ge 2 ] || die '--managed-by needs a UUID'; managed_by=$2; shift 2 ;;
    --entitlement-id) [ "$#" -ge 2 ] || die '--entitlement-id needs a UUID'; entitlement_id=$2; shift 2 ;;
    --activation-date) [ "$#" -ge 2 ] || die '--activation-date needs a date'; activation_date=$2; shift 2 ;;
    --env-file) [ "$#" -ge 2 ] || die '--env-file needs a path'; env_file=$2; shift 2 ;;
    --confirm) [ "$#" -ge 2 ] || die '--confirm needs a value'; confirmation=$2; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

[ -z "$env_file" ] || export LAGRANGE_ENV_FILE="$env_file"

[ "$action" = register ] || [ "$action" = activate ] || die 'action must be register or activate'
[ "$mode" = plan ] || [ "$mode" = check ] || [ "$mode" = apply ] || die 'invalid mode'
[ "$mode" != apply ] || [ "$(id -u)" -eq 0 ] || die '--apply must run as root'
command -v jq >/dev/null 2>&1 || die 'jq is required'
command -v sha256sum >/dev/null 2>&1 || die 'sha256sum is required'

valid_uuid() {
  [[ "$1" =~ ^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-5][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}$ ]]
}
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
secure_file() {
  local path=$1 label=$2 mode_value
  [ -n "$path" ] || blocked "$label was not supplied"
  [ -f "$path" ] || blocked "$label is missing"
  [ ! -L "$path" ] || die "$label must not be a symlink"
  [ -s "$path" ] || die "$label is empty"
  mode_value=$(stat -c '%a' -- "$path") || die "cannot stat $label"
  case "$mode_value" in 400|600) ;; *) die "$label must be mode 0400 or 0600" ;; esac
}
jget() { jq -er "$1" "$metadata_file" 2>/dev/null || die 'rights metadata is malformed'; }

if [ "$action" = register ]; then
  [ -n "$metadata_file" ] || blocked '--metadata-file is required'
  [ -n "$document_file" ] || blocked '--document-file is required'
  secure_file "$metadata_file" 'metadata file'
  secure_file "$document_file" 'document file'
  [ "$metadata_file" != "$document_file" ] || die 'metadata and document must be separate files'
  jq -e '
    type == "object" and .schema_version == 1 and .provider == "kis"
    and (.entitlement_id | type == "string" and test("^ent_[A-Za-z0-9_-]+$"))
    and (.contract_document.document_hash.algorithm == "SHA-256")
    and (.contract_document.document_hash.hex | type == "string" and test("^[0-9a-f]{64}$"))
    and (.contract_document.document_reference | type == "string" and length > 0
         and test("^[A-Za-z0-9_./:-]+$"))
    and (.covered_datasets | type == "array" and length > 0
         and (map(type == "string" and test("^krx_[a-z0-9_]+$")) | all))
    and (.covered_uses | type == "array" and length > 0
         and (map(. as $u | ["dataset","factor","recommendation","candidate","backtest","report","benchmark","paper_view","payload","download"] | index($u) != null) | all))
    and (.covered_users | type == "array"
         and (map(type == "string" and test("^usr_[A-Za-z0-9_-]+$")) | all))
    and (.effective_from | type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}$"))
    and (.effective_until | type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}$"))
    and .lifecycle == "PENDING"
  ' "$metadata_file" >/dev/null 2>&1 || die 'metadata must be a KIS schema-v1 PENDING record'

  entitlement_external_id=$(jget '.entitlement_id')
  contract_hash=$(jget '.contract_document.document_hash.hex')
  contract_reference=$(jget '.contract_document.document_reference')
  covered_datasets=$(jq -c '.covered_datasets' "$metadata_file")
  covered_uses=$(jq -c '.covered_uses' "$metadata_file")
  effective_from=$(jget '.effective_from')
  effective_until=$(jget '.effective_until')
  valid_date "$effective_from" || die 'effective_from is not a real date'
  valid_date "$effective_until" || die 'effective_until is not a real date'
  if [[ "$effective_until" < "$effective_from" ]]; then
    die 'effective_until precedes effective_from'
  fi
  jq -e '.covered_datasets | index("krx_eod_bars") != null' "$metadata_file" >/dev/null ||
    die 'metadata must cover krx_eod_bars'
  jq -e '.covered_uses | index("dataset") != null' "$metadata_file" >/dev/null ||
    die 'metadata must cover dataset use'
  [[ "$contract_reference" != *REPLACE_WITH* && "$contract_reference" != *PLACEHOLDER* && "$contract_reference" != *placeholder* ]] ||
    blocked 'contract reference is a placeholder'
  [[ "$contract_hash" != 0000000000000000000000000000000000000000000000000000000000000000 ]] ||
    blocked 'contract hash is the zero placeholder'
  document_hash=$(sha256sum -- "$document_file" | awk '{print $1}')
  [ "$document_hash" = "$contract_hash" ] ||
    die 'document SHA-256 does not match metadata'
  [ -n "$managed_by" ] || die '--managed-by is required'
  valid_uuid "$managed_by" || die '--managed-by must be an owner UUID'
  printf 'ENTITLEMENT_PLAN: KIS redacted rights metadata\n'
  printf '  external_id=%s status=PENDING managed_by=%s\n' "$entitlement_external_id" "$managed_by"
  printf '  document_sha256=%s reference=%s\n' "$contract_hash" "$contract_reference"
  printf '  effective=%s..%s datasets=%s uses=%s\n' "$effective_from" "$effective_until" \
    "$(jq -r 'join(",")' <<<"$covered_datasets")" "$(jq -r 'join(",")' <<<"$covered_uses")"
else
  [ -n "$entitlement_id" ] || die '--entitlement-id is required'
  valid_uuid "$entitlement_id" || die '--entitlement-id must be a database UUID'
  [ -n "$managed_by" ] || die '--managed-by is required'
  valid_uuid "$managed_by" || die '--managed-by must be an owner UUID'
  [ -n "$activation_date" ] || die '--activation-date is required'
  valid_date "$activation_date" || die '--activation-date must be a real date'
  printf 'ENTITLEMENT_ACTIVATION_PLAN: explicit PENDING to ACTIVE transition\n'
  printf '  entitlement_id=%s managed_by=%s activation_date=%s\n' "$entitlement_id" "$managed_by" "$activation_date"
fi

[ "$mode" = plan ] && {
  echo 'PLAN_ONLY: no database, KIS, account/order, network, or file write made'
  exit 0
}

db_init
sql_file=$(mktemp /tmp/lagrange-entitlement.XXXXXX.sql)
chmod 0600 "$sql_file"
trap 'rm -f -- "$sql_file"' EXIT

if [ "$action" = register ]; then
  if [ "$mode" = check ]; then
    result=$(db_psql -qAt -F $'\t' \
      -v contract_hash="$contract_hash" -v contract_reference="$contract_reference" \
      -v covered_datasets="$covered_datasets" -v covered_uses="$covered_uses" \
      -v effective_from="$effective_from" -v effective_until="$effective_until" \
      -v managed_by="$managed_by" <<'SQL'
SELECT id::text, status
  FROM public.data_entitlements
 WHERE contract_document_sha256 = :'contract_hash'
   AND contract_reference = :'contract_reference'
   AND status = 'PENDING'
   AND covered_datasets = :'covered_datasets'::jsonb
   AND covered_uses = :'covered_uses'::jsonb
   AND effective_from = :'effective_from'::date
   AND effective_until = :'effective_until'::date
   AND managed_by = :'managed_by'::uuid;
SQL
    ) || blocked 'pending entitlement row is absent or conflicts'
    row_id=$(printf '%s\n' "$result" | awk -F '\t' 'NR==1 {print $1}')
    row_status=$(printf '%s\n' "$result" | awk -F '\t' 'NR==1 {print $2}')
    [ -n "$row_id" ] && [ "$row_status" = PENDING ] || blocked 'pending entitlement row is absent or conflicts'
    printf 'ENTITLEMENT_CHECK: PASS db_row=%s status=%s\n' "$row_id" "$row_status"
    exit 0
  fi
  if [ "$mode" = apply ]; then
    [ "$confirmation" = I_UNDERSTAND_REGISTER_PENDING_ENTITLEMENT ] ||
      die '--apply requires --confirm I_UNDERSTAND_REGISTER_PENDING_ENTITLEMENT'
  fi
  cat >"$sql_file" <<'SQL'
BEGIN;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '15s';
SELECT pg_catalog.set_config('operator.entitlement.contract_hash', :'contract_hash', true) AS _set_contract_hash \gset
SELECT pg_catalog.set_config('operator.entitlement.contract_reference', :'contract_reference', true) AS _set_contract_reference \gset
SELECT pg_catalog.set_config('operator.entitlement.covered_datasets', :'covered_datasets', true) AS _set_covered_datasets \gset
SELECT pg_catalog.set_config('operator.entitlement.covered_uses', :'covered_uses', true) AS _set_covered_uses \gset
SELECT pg_catalog.set_config('operator.entitlement.effective_from', :'effective_from', true) AS _set_effective_from \gset
SELECT pg_catalog.set_config('operator.entitlement.effective_until', :'effective_until', true) AS _set_effective_until \gset
SELECT pg_catalog.set_config('operator.entitlement.managed_by', :'managed_by', true) AS _set_managed_by \gset
SELECT pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(current_setting('operator.entitlement.contract_reference'), 0));
DO $body$
DECLARE v_existing public.data_entitlements%ROWTYPE;
BEGIN
  SELECT * INTO v_existing FROM public.data_entitlements
   WHERE contract_reference = current_setting('operator.entitlement.contract_reference') FOR UPDATE;
  IF FOUND THEN
    IF v_existing.contract_document_sha256 IS DISTINCT FROM current_setting('operator.entitlement.contract_hash')
       OR v_existing.covered_datasets IS DISTINCT FROM current_setting('operator.entitlement.covered_datasets')::jsonb
       OR v_existing.covered_uses IS DISTINCT FROM current_setting('operator.entitlement.covered_uses')::jsonb
       OR v_existing.effective_from IS DISTINCT FROM current_setting('operator.entitlement.effective_from')::date
       OR v_existing.effective_until IS DISTINCT FROM current_setting('operator.entitlement.effective_until')::date
       OR v_existing.managed_by IS DISTINCT FROM current_setting('operator.entitlement.managed_by')::uuid
    THEN
      RAISE EXCEPTION 'entitlement reference has conflicting metadata' USING ERRCODE = '23505';
    END IF;
    IF v_existing.status <> 'PENDING' THEN
      RAISE EXCEPTION 'exact entitlement exists but is not PENDING' USING ERRCODE = '55000';
    END IF;
  END IF;
END
$body$;
INSERT INTO public.data_entitlements (
  contract_document_sha256, contract_reference, status, covered_datasets,
  covered_uses, effective_from, effective_until, managed_by
)
SELECT current_setting('operator.entitlement.contract_hash'),
       current_setting('operator.entitlement.contract_reference'), 'PENDING',
       current_setting('operator.entitlement.covered_datasets')::jsonb,
       current_setting('operator.entitlement.covered_uses')::jsonb,
       current_setting('operator.entitlement.effective_from')::date,
       current_setting('operator.entitlement.effective_until')::date,
       current_setting('operator.entitlement.managed_by')::uuid
WHERE NOT EXISTS (
  SELECT 1 FROM public.data_entitlements
   WHERE contract_reference = current_setting('operator.entitlement.contract_reference')
);
INSERT INTO public.audit_logs (
  action, actor_role, actor_user_id, target_type, target_id, after_json, reason
)
SELECT 'entitlement.pending.registered', 'operator', current_setting('operator.entitlement.managed_by')::uuid,
       'data_entitlement', e.id::text,
       jsonb_build_object('status', e.status, 'contract_document_sha256', e.contract_document_sha256,
         'contract_reference', e.contract_reference, 'covered_datasets', e.covered_datasets,
         'covered_uses', e.covered_uses, 'effective_from', e.effective_from,
         'effective_until', e.effective_until, 'managed_by', e.managed_by),
       'operator attestation; document body excluded'
  FROM public.data_entitlements e
 WHERE e.contract_reference = current_setting('operator.entitlement.contract_reference')
   AND NOT EXISTS (
     SELECT 1 FROM public.audit_logs a
      WHERE a.action = 'entitlement.pending.registered'
        AND a.target_type = 'data_entitlement' AND a.target_id = e.id::text
        AND a.reason = 'operator attestation; document body excluded'
   );
SELECT id::text, status FROM public.data_entitlements
 WHERE contract_reference = current_setting('operator.entitlement.contract_reference');
COMMIT;
SQL
  result=$(db_psql -qAt -F $'\t' -v contract_hash="$contract_hash" -v contract_reference="$contract_reference" \
    -v covered_datasets="$covered_datasets" -v covered_uses="$covered_uses" \
    -v effective_from="$effective_from" -v effective_until="$effective_until" \
    -v managed_by="$managed_by" <"$sql_file") || {
      [ "$mode" = check ] && blocked 'pending entitlement row is absent or conflicts' || exit 1
    }
  row_id=$(printf '%s\n' "$result" | awk -F '\t' 'NR==1 {print $1}')
  row_status=$(printf '%s\n' "$result" | awk -F '\t' 'NR==1 {print $2}')
  [ -n "$row_id" ] && [ -n "$row_status" ] || blocked 'database returned no entitlement row'
  printf 'ENTITLEMENT_%s: PASS db_row=%s status=%s\n' \
    "$( [ "$mode" = check ] && printf CHECK || printf APPLY )" "$row_id" "$row_status"
else
  if [ "$mode" = check ]; then
    result=$(db_psql -qAt -F $'\t' -v entitlement_id="$entitlement_id" \
      -v managed_by="$managed_by" -v activation_date="$activation_date" <<'SQL'
SELECT id::text, status
  FROM public.data_entitlements
 WHERE id = :'entitlement_id'::uuid
   AND managed_by = :'managed_by'::uuid
   AND :'activation_date'::date BETWEEN effective_from AND effective_until
   AND status IN ('PENDING', 'ACTIVE');
SQL
    ) || blocked 'entitlement is not an eligible PENDING row'
    row_id=$(printf '%s\n' "$result" | awk -F '\t' 'NR==1 {print $1}')
    row_status=$(printf '%s\n' "$result" | awk -F '\t' 'NR==1 {print $2}')
    [ -n "$row_id" ] && [ "$row_id" = "$entitlement_id" ] || blocked 'entitlement is not an eligible PENDING row'
    printf 'ENTITLEMENT_CHECK: PASS db_row=%s status=%s\n' "$row_id" "$row_status"
    exit 0
  fi
  if [ "$mode" = apply ]; then
    [ "$confirmation" = I_UNDERSTAND_ACTIVATE_ENTITLEMENT ] ||
      die '--apply requires --confirm I_UNDERSTAND_ACTIVATE_ENTITLEMENT'
  fi
  cat >"$sql_file" <<'SQL'
BEGIN;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '15s';
SELECT pg_catalog.set_config('operator.entitlement.id', :'entitlement_id', true) AS _set_entitlement_id \gset
SELECT pg_catalog.set_config('operator.entitlement.managed_by', :'managed_by', true) AS _set_managed_by \gset
SELECT pg_catalog.set_config('operator.entitlement.activation_date', :'activation_date', true) AS _set_activation_date \gset
SELECT pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(current_setting('operator.entitlement.id'), 0));
DO $body$
DECLARE v_status text; v_managed_by uuid; v_from date; v_until date;
BEGIN
  SELECT status, managed_by, effective_from, effective_until
    INTO v_status, v_managed_by, v_from, v_until
    FROM public.data_entitlements
   WHERE id = current_setting('operator.entitlement.id')::uuid FOR UPDATE;
  IF NOT FOUND THEN RAISE EXCEPTION 'entitlement row does not exist' USING ERRCODE = '02000'; END IF;
  IF v_managed_by IS DISTINCT FROM current_setting('operator.entitlement.managed_by')::uuid THEN
    RAISE EXCEPTION 'entitlement owner UUID does not match' USING ERRCODE = '42501';
  END IF;
  IF current_setting('operator.entitlement.activation_date')::date < v_from
     OR current_setting('operator.entitlement.activation_date')::date > v_until THEN
    RAISE EXCEPTION 'activation date is outside the effective window' USING ERRCODE = '22007';
  END IF;
  IF v_status NOT IN ('PENDING', 'ACTIVE') THEN
    RAISE EXCEPTION 'only PENDING entitlements may be activated' USING ERRCODE = '55000';
  END IF;
  IF v_status = 'PENDING' THEN
    UPDATE public.data_entitlements SET status = 'ACTIVE', updated_at = pg_catalog.clock_timestamp()
     WHERE id = current_setting('operator.entitlement.id')::uuid;
    INSERT INTO public.audit_logs (
      action, actor_role, actor_user_id, target_type, target_id,
      before_json, after_json, reason
    ) VALUES (
      'entitlement.activated', 'operator', current_setting('operator.entitlement.managed_by')::uuid,
      'data_entitlement', current_setting('operator.entitlement.id'), jsonb_build_object('status','PENDING'),
      jsonb_build_object('status','ACTIVE','activation_date',current_setting('operator.entitlement.activation_date')::date),
      'explicit operator activation; document body excluded'
    );
  END IF;
END
$body$;
SELECT id::text, status FROM public.data_entitlements
 WHERE id = current_setting('operator.entitlement.id')::uuid;
COMMIT;
SQL
  result=$(db_psql -qAt -F $'\t' -v entitlement_id="$entitlement_id" \
    -v managed_by="$managed_by" -v activation_date="$activation_date" <"$sql_file") || {
      [ "$mode" = check ] && blocked 'entitlement is not an eligible PENDING row' || exit 1
    }
  row_id=$(printf '%s\n' "$result" | awk -F '\t' 'NR==1 {print $1}')
  row_status=$(printf '%s\n' "$result" | awk -F '\t' 'NR==1 {print $2}')
  [ -n "$row_id" ] && [ -n "$row_status" ] || blocked 'database returned no entitlement row'
  printf 'ENTITLEMENT_%s: PASS db_row=%s status=%s\n' \
    "$( [ "$mode" = check ] && printf CHECK || printf APPLY )" "$row_id" "$row_status"
fi
