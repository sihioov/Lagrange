#!/usr/bin/env bash
# Narrow, audited amendment of the one approved KIS entitlement document.
#
# This file is sourced by provision-entitlement.sh.  It deliberately keeps
# the approved pair facts here, rather than consulting Git at runtime: an
# installed release is allowed to have no .git directory.  The metadata-file
# digests and the document digests are separate domains and are both checked.

_amend_helper_source=${BASH_SOURCE[0]}
_amend_helper_dir=$(cd -P "$(dirname -- "$_amend_helper_source")" && pwd -P)
AMEND_ENTITLEMENT_HELPER_SOURCE_PATH="$_amend_helper_dir/$(basename -- "$_amend_helper_source")"
AMEND_ENTITLEMENT_ENTRYPOINT_SOURCE=${BASH_SOURCE[1]:-}
source "$_amend_helper_dir/entitlement-amend-installed-release.sh"
unset _amend_helper_source _amend_helper_dir

AMEND_ORIGINAL_REVISION='cdaffe6b20bfb6cabce09f8f87b1ad707ed725f9'
AMEND_APPROVED_REVISION='91a4ff68bb38c2517fe83fca406baf4063023e9a'
AMEND_APPROVED_REVISION_SHORT='91a4ff68'

# These are SHA-256 values of the exact Git bytes, not credentials.
AMEND_ORIGINAL_METADATA_SHA256='dd690d397b9e8846bb8177f23f78643f46866f58ac3b602f095de263b315abb2'
AMEND_APPROVED_METADATA_SHA256='56bc018f748e2a1cfa78c4b94c18adccb2e0afd6a2d66fea4ecd3654db56b36e'
AMEND_ORIGINAL_DOCUMENT_SHA256='b888942df6b03d7920c404610ce21f1f3bc54e44c5647519a28e6ae6c68dad36'
AMEND_APPROVED_DOCUMENT_SHA256='5904a9c5ee00af734c762e877227761ab23391acf65220715f90d08b61947ea9'

# Git blob IDs document the provenance of the pinned bytes.  Runtime
# validation uses the SHA-256 byte pins above, so a release does not need Git.
AMEND_ORIGINAL_METADATA_BLOB='a83766aa1a4af3b96a188a08ded21e99af21792f'
AMEND_APPROVED_METADATA_BLOB='3044affe3ae7f6f2272030ec14b49d30a0b58210'
AMEND_ORIGINAL_DOCUMENT_BLOB='fb7fab7e32c685d408845157f5b5ae16392525f0'
AMEND_APPROVED_DOCUMENT_BLOB='d4a05657682968b7d6a159189d6e7256d951a7b5'

AMEND_PROVIDER='kis'
AMEND_EXTERNAL_ID='ent_kis_personal_owner_20260821'
AMEND_DOCUMENT_REFERENCE='repo://docs/decisions/0005-kis-personal-use-entitlement.md'
AMEND_ORIGINAL_FROM='2020-01-31'
AMEND_APPROVED_FROM='2016-08-29'
AMEND_EFFECTIVE_UNTIL='9999-12-31'
AMEND_LIFECYCLE='ACTIVE'

AMEND_DATASETS='["krx_eod_bars","krx_instruments","krx_calendar","krx_market_status","krx_investor_flows","krx_fundamentals","krx_kospi200_membership","krx_kosdaq150_membership","krx_sector_classification"]'
AMEND_USES='["dataset","factor","recommendation","candidate","backtest","report","benchmark","paper_view","payload","download"]'
AMEND_USERS='["usr_owner"]'
AMEND_AUDIT_ACTION='entitlement.approved_document.amended'
AMEND_AUDIT_TARGET_TYPE='data_entitlement'
AMEND_AUDIT_REASON='operator attestation; approved document amendment; document body excluded'

amend_error() {
  echo "ENTITLEMENT_AMEND: FAIL status=$1" >&2
  exit 1
}

amend_blocked() {
  echo "ENTITLEMENT_AMEND: BLOCKED status=$1" >&2
  exit 2
}

amend_confirmation_error() {
  echo 'ENTITLEMENT_AMEND: FAIL status=apply_confirmation required=I_UNDERSTAND_AMEND_ACTIVE_ENTITLEMENT' >&2
  exit 1
}

amend_valid_uuid() {
  [[ "$1" =~ ^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-5][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}$ ]]
}

amend_lower() {
  printf '%s' "$1" | tr '[:upper:]' '[:lower:]'
}

amend_no_symlink_components() {
  local path=$1 absolute component rest prefix
  case "$path" in
    /*) absolute=$path ;;
    *) absolute=$PWD/$path ;;
  esac
  prefix=/
  rest=${absolute#/}
  while [ -n "$rest" ]; do
    component=${rest%%/*}
    if [ "$rest" = "$component" ]; then
      rest=
    else
      rest=${rest#*/}
    fi
    case "$component" in
      ''|.) ;;
      ..) prefix=$(dirname -- "$prefix") ;;
      *)
        prefix=${prefix%/}/$component
        [ ! -L "$prefix" ] || return 1
        ;;
    esac
  done
}

amend_secure_path() {
  local path=$1 mode_value
  [ -n "$path" ] || amend_blocked input_missing
  amend_no_symlink_components "$path" || amend_error input_symlink
  [ -f "$path" ] || amend_blocked input_missing
  [ ! -L "$path" ] || amend_error input_symlink
  mode_value=$(stat -c '%a' -- "$path" 2>/dev/null) || amend_error input_stat
  case "$mode_value" in
    400|600) ;;
    *) amend_error input_mode ;;
  esac
  [ -s "$path" ] || amend_error input_empty
}

amend_hash_file() {
  sha256sum -- "$1" 2>/dev/null | awk '{print $1}'
}

amend_metadata_matches() {
  local metadata_file=$1 expected_document_hash=$2 expected_from=$3
  jq -e \
    --arg provider "$AMEND_PROVIDER" \
    --arg external_id "$AMEND_EXTERNAL_ID" \
    --arg reference "$AMEND_DOCUMENT_REFERENCE" \
    --arg document_hash "$expected_document_hash" \
    --arg from "$expected_from" \
    --arg until "$AMEND_EFFECTIVE_UNTIL" \
    --arg lifecycle "$AMEND_LIFECYCLE" \
    --argjson datasets "$AMEND_DATASETS" \
    --argjson uses "$AMEND_USES" \
    --argjson users "$AMEND_USERS" \
    'type == "object"
     and .schema_version == 1
     and .provider == $provider
     and .entitlement_id == $external_id
     and .contract_document == {
       document_hash: {algorithm: "SHA-256", hex: $document_hash},
       document_reference: $reference
     }
     and .covered_datasets == $datasets
     and .covered_uses == $uses
     and .covered_users == $users
     and .effective_from == $from
     and .effective_until == $until
     and .lifecycle == $lifecycle
     and ((keys | sort) == [
       "contract_document", "covered_datasets", "covered_users", "covered_uses",
       "effective_from", "effective_until", "entitlement_id", "lifecycle",
       "provider", "schema_version"
     ])' "$metadata_file" >/dev/null 2>&1
}

amend_validate_pair() {
  local metadata_file=$1 document_file=$2 expected_metadata_hash=$3
  local expected_document_hash=$4 expected_from=$5 metadata_hash document_hash
  amend_secure_path "$metadata_file"
  amend_secure_path "$document_file"
  [ "$metadata_file" != "$document_file" ] || amend_error input_pair
  metadata_hash=$(amend_hash_file "$metadata_file") || amend_error input_hash
  document_hash=$(amend_hash_file "$document_file") || amend_error input_hash
  [ "$metadata_hash" = "$expected_metadata_hash" ] || amend_error metadata_pin
  [ "$document_hash" = "$expected_document_hash" ] || amend_error document_pin
  amend_metadata_matches "$metadata_file" "$document_hash" "$expected_from" ||
    amend_error metadata_contract
  [ "$document_hash" = "$(jq -er '.contract_document.document_hash.hex' "$metadata_file" 2>/dev/null)" ] ||
    amend_error metadata_document_domain
  printf '%s\t%s\n' "$metadata_hash" "$document_hash"
}

amend_json_for_audit() {
  local metadata_file=$1 metadata_hash=$2 document_hash=$3 database_id=$4 managed_by=$5
  local amendment_revision=$6 executing_revision=$7
  jq -c \
    --arg metadata_file_hash "$metadata_hash" \
    --arg document_file_hash "$document_hash" \
    --arg database_id "$database_id" \
    --arg managed_by "$managed_by" \
    --arg original_revision "$AMEND_ORIGINAL_REVISION" \
    --arg amendment_revision "$amendment_revision" \
    --arg executing_revision "$executing_revision" \
    ' {
        metadata_file_sha256: $metadata_file_hash,
        document_file_sha256: $document_file_hash,
        original_git_revision: $original_revision,
        amendment_git_revision: $amendment_revision,
        executing_release_revision: $executing_revision,
        approval: {
          schema_version: .schema_version,
          provider: .provider,
          entitlement_id: .entitlement_id,
          contract_document: .contract_document,
          covered_datasets: .covered_datasets,
          covered_uses: .covered_uses,
          covered_users: .covered_users,
          effective_from: .effective_from,
          effective_until: .effective_until,
          lifecycle: .lifecycle
        },
        database: {
          id: $database_id,
          contract_document_sha256: .contract_document.document_hash.hex,
          contract_reference: .contract_document.document_reference,
          covered_datasets: .covered_datasets,
          covered_uses: .covered_uses,
          effective_from: .effective_from,
          effective_until: .effective_until,
          status: .lifecycle,
          managed_by: $managed_by
        }
      }' "$metadata_file" 2>/dev/null
}

# Replay identity deliberately excludes the release that first performed the
# amendment.  The full event retains that immutable executor field; SQL below
# validates it is one canonical commit shared by before/after rather than
# replacing it with the release currently attempting the replay.
amend_json_semantic_for_replay() {
  printf '%s' "$1" | jq -c 'del(.executing_release_revision)' 2>/dev/null
}

amend_revision_is_approved() {
  [ "$1" = "$AMEND_APPROVED_REVISION" ] || [ "$1" = "$AMEND_APPROVED_REVISION_SHORT" ]
}

amend_normalize_revision() {
  if [ "$1" = "$AMEND_APPROVED_REVISION_SHORT" ]; then
    printf '%s' "$AMEND_APPROVED_REVISION"
  else
    printf '%s' "$1"
  fi
}

amend_write_check_sql() {
  cat >"$1" <<'SQL'
BEGIN READ ONLY;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '15s';
SELECT pg_catalog.set_config('operator.entitlement.amend.target_id', :'target_id', true) AS _set_target_id \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.reference', :'reference', true) AS _set_reference \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.current_owner', :'current_owner', true) AS _set_current_owner \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.old_hash', :'old_hash', true) AS _set_old_hash \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.new_hash', :'new_hash', true) AS _set_new_hash \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.old_datasets', :'old_datasets', true) AS _set_old_datasets \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.new_datasets', :'new_datasets', true) AS _set_new_datasets \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.old_uses', :'old_uses', true) AS _set_old_uses \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.new_uses', :'new_uses', true) AS _set_new_uses \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.old_from', :'old_from', true) AS _set_old_from \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.new_from', :'new_from', true) AS _set_new_from \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.until', :'until_date', true) AS _set_until \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.old_manager', :'old_manager', true) AS _set_old_manager \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.new_manager', :'new_manager', true) AS _set_new_manager \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.status', :'status', true) AS _set_status \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.action', :'audit_action', true) AS _set_action \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.target_type', :'audit_target_type', true) AS _set_target_type \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.reason', :'audit_reason', true) AS _set_reason \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.correlation_id', :'correlation_id', true) AS _set_correlation_id \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.before_json', :'before_json', true) AS _set_before_json \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.after_json', :'after_json', true) AS _set_after_json \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.before_semantic_json', :'before_semantic_json', true) AS _set_before_semantic_json \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.after_semantic_json', :'after_semantic_json', true) AS _set_after_semantic_json \gset

WITH target_rows AS (
    SELECT e.*
      FROM public.data_entitlements AS e
     WHERE e.contract_reference = current_setting('operator.entitlement.amend.reference')
), target_stats AS (
    SELECT count(*)::bigint AS reference_count,
           count(*) FILTER (
               WHERE id = current_setting('operator.entitlement.amend.target_id')::uuid
           )::bigint AS target_count,
           COALESCE(bool_or(
               id = current_setting('operator.entitlement.amend.target_id')::uuid
               AND contract_document_sha256 = current_setting('operator.entitlement.amend.new_hash')
               AND contract_reference = current_setting('operator.entitlement.amend.reference')
               AND status = current_setting('operator.entitlement.amend.status')
               AND covered_datasets IS NOT DISTINCT FROM
                   current_setting('operator.entitlement.amend.new_datasets')::jsonb
               AND covered_uses IS NOT DISTINCT FROM
                   current_setting('operator.entitlement.amend.new_uses')::jsonb
               AND effective_from = current_setting('operator.entitlement.amend.new_from')::date
               AND effective_until = current_setting('operator.entitlement.amend.until')::date
               AND managed_by = current_setting('operator.entitlement.amend.new_manager')::uuid
           ), false) AS post_match,
           COALESCE(bool_or(
               id = current_setting('operator.entitlement.amend.target_id')::uuid
               AND contract_document_sha256 = current_setting('operator.entitlement.amend.old_hash')
               AND contract_reference = current_setting('operator.entitlement.amend.reference')
               AND status = current_setting('operator.entitlement.amend.status')
               AND covered_datasets IS NOT DISTINCT FROM
                   current_setting('operator.entitlement.amend.old_datasets')::jsonb
               AND covered_uses IS NOT DISTINCT FROM
                   current_setting('operator.entitlement.amend.old_uses')::jsonb
               AND effective_from = current_setting('operator.entitlement.amend.old_from')::date
               AND effective_until = current_setting('operator.entitlement.amend.until')::date
               AND managed_by = current_setting('operator.entitlement.amend.old_manager')::uuid
           ), false) AS old_match
      FROM target_rows
), owners AS (
    SELECT ur.user_id
      FROM public.user_roles AS ur
      JOIN public.users AS u ON u.id = ur.user_id
     WHERE ur.role_id = 'owner'
), owner_stats AS (
    SELECT count(*)::bigint AS owner_count, min(user_id::text)::uuid AS owner_id
      FROM owners
), audit_stats AS (
    SELECT count(*) FILTER (
               WHERE a.action = current_setting('operator.entitlement.amend.action')
                 AND a.actor_role = 'operator'
                 AND a.actor_user_id = current_setting('operator.entitlement.amend.new_manager')::uuid
                 AND a.target_type = current_setting('operator.entitlement.amend.target_type')
                 AND a.target_id = current_setting('operator.entitlement.amend.target_id')
                 AND jsonb_typeof(a.before_json) = 'object'
                 AND jsonb_typeof(a.after_json) = 'object'
                 AND a.before_json ? 'executing_release_revision'
                 AND a.after_json ? 'executing_release_revision'
                 AND (a.before_json ->> 'executing_release_revision') ~ '^[0-9a-f]{40}$'
                 AND (a.after_json ->> 'executing_release_revision') ~ '^[0-9a-f]{40}$'
                 AND (a.before_json ->> 'executing_release_revision') =
                     (a.after_json ->> 'executing_release_revision')
                 AND (a.before_json - 'executing_release_revision') IS NOT DISTINCT FROM
                     current_setting('operator.entitlement.amend.before_semantic_json')::jsonb
                 AND (a.after_json - 'executing_release_revision') IS NOT DISTINCT FROM
                     current_setting('operator.entitlement.amend.after_semantic_json')::jsonb
                 AND a.reason = current_setting('operator.entitlement.amend.reason')
                 AND a.correlation_id = current_setting('operator.entitlement.amend.correlation_id')
           )::bigint AS exact_audit_count,
           count(*) FILTER (
               WHERE a.action = current_setting('operator.entitlement.amend.action')
                 AND a.target_type = current_setting('operator.entitlement.amend.target_type')
                 AND a.target_id = current_setting('operator.entitlement.amend.target_id')
                 AND a.correlation_id = current_setting('operator.entitlement.amend.correlation_id')
           )::bigint AS correlation_count,
           count(*) FILTER (
               WHERE a.action = current_setting('operator.entitlement.amend.action')
                 AND a.target_type = current_setting('operator.entitlement.amend.target_type')
                 AND a.target_id = current_setting('operator.entitlement.amend.target_id')
           )::bigint AS target_audit_count
      FROM public.audit_logs AS a
)
SELECT CASE
         WHEN owner_count <> 1
              OR owner_id IS DISTINCT FROM current_setting('operator.entitlement.amend.current_owner')::uuid
           THEN 'INVALID_OWNER'
         WHEN reference_count <> 1 OR target_count <> 1
           THEN 'INVALID_TARGET'
         WHEN correlation_count > 1 OR target_audit_count > 1
           THEN 'INVALID_AUDIT'
         WHEN post_match AND exact_audit_count = 1 AND target_audit_count = 1
           THEN 'ALREADY_APPLIED'
         WHEN post_match
           THEN 'INVALID_POSTSTATE_AUDIT'
         WHEN target_audit_count > 0
           THEN 'INVALID_AUDIT'
         WHEN old_match AND exact_audit_count = 0 AND correlation_count = 0
              AND target_audit_count = 0
           THEN 'ELIGIBLE'
         ELSE 'INVALID_CAS'
       END AS state,
       reference_count,
       target_count,
       exact_audit_count,
       correlation_count
  FROM target_stats CROSS JOIN owner_stats CROSS JOIN audit_stats;
COMMIT;
SQL
}

amend_write_apply_sql() {
  cat >"$1" <<'SQL'
BEGIN;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '15s';
SELECT pg_catalog.set_config('operator.entitlement.amend.target_id', :'target_id', true) AS _set_target_id \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.reference', :'reference', true) AS _set_reference \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.current_owner', :'current_owner', true) AS _set_current_owner \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.old_hash', :'old_hash', true) AS _set_old_hash \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.new_hash', :'new_hash', true) AS _set_new_hash \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.old_datasets', :'old_datasets', true) AS _set_old_datasets \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.new_datasets', :'new_datasets', true) AS _set_new_datasets \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.old_uses', :'old_uses', true) AS _set_old_uses \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.new_uses', :'new_uses', true) AS _set_new_uses \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.old_from', :'old_from', true) AS _set_old_from \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.new_from', :'new_from', true) AS _set_new_from \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.until', :'until_date', true) AS _set_until \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.old_manager', :'old_manager', true) AS _set_old_manager \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.new_manager', :'new_manager', true) AS _set_new_manager \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.status', :'status', true) AS _set_status \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.action', :'audit_action', true) AS _set_action \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.target_type', :'audit_target_type', true) AS _set_target_type \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.reason', :'audit_reason', true) AS _set_reason \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.correlation_id', :'correlation_id', true) AS _set_correlation_id \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.before_json', :'before_json', true) AS _set_before_json \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.after_json', :'after_json', true) AS _set_after_json \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.before_semantic_json', :'before_semantic_json', true) AS _set_before_semantic_json \gset
SELECT pg_catalog.set_config('operator.entitlement.amend.after_semantic_json', :'after_semantic_json', true) AS _set_after_semantic_json \gset

SELECT pg_catalog.pg_advisory_xact_lock(
    pg_catalog.hashtextextended(current_setting('operator.entitlement.amend.reference'), 0)
);

DO $body$
DECLARE
    v_target public.data_entitlements%ROWTYPE;
    v_target_count bigint;
    v_owner_count bigint := 0;
    v_owner_id uuid;
    v_owner record;
    v_other_count_before bigint;
    v_other_fingerprint_before text;
    v_other_count_after bigint;
    v_other_fingerprint_after text;
    v_audit_exact_count bigint;
    v_audit_correlation_count bigint;
    v_audit_target_count bigint;
    v_changed_count bigint;
    v_post_count bigint;
    v_post_reference_count bigint;
    v_old_match boolean;
    v_post_match boolean;
BEGIN
    SELECT count(*)::bigint
      INTO v_target_count
      FROM public.data_entitlements AS e
     WHERE e.contract_reference = current_setting('operator.entitlement.amend.reference');
    IF v_target_count <> 1 THEN
        RAISE EXCEPTION 'amendment target reference row count invalid';
    END IF;

    SELECT e.*
      INTO v_target
      FROM public.data_entitlements AS e
     WHERE e.id = current_setting('operator.entitlement.amend.target_id')::uuid
     FOR UPDATE;
    IF NOT FOUND
       OR v_target.contract_reference IS DISTINCT FROM current_setting('operator.entitlement.amend.reference') THEN
        RAISE EXCEPTION 'amendment target row invalid';
    END IF;

    SELECT count(*)::bigint
      INTO v_target_count
      FROM public.data_entitlements AS e
     WHERE e.contract_reference = current_setting('operator.entitlement.amend.reference');
    IF v_target_count <> 1 THEN
        RAISE EXCEPTION 'amendment target reference row count changed';
    END IF;

    LOCK TABLE public.user_roles IN SHARE MODE;
    LOCK TABLE public.users IN SHARE MODE;
    FOR v_owner IN
        SELECT ur.user_id
          FROM public.user_roles AS ur
          JOIN public.users AS u ON u.id = ur.user_id
         WHERE ur.role_id = 'owner'
         ORDER BY ur.user_id
         FOR UPDATE OF ur, u
    LOOP
        v_owner_count := v_owner_count + 1;
        v_owner_id := v_owner.user_id;
    END LOOP;
    IF v_owner_count <> 1
       OR v_owner_id IS DISTINCT FROM current_setting('operator.entitlement.amend.current_owner')::uuid THEN
        RAISE EXCEPTION 'amendment current owner evidence invalid';
    END IF;

    PERFORM 1
      FROM public.data_entitlements AS e
     WHERE e.id <> v_target.id
     FOR SHARE;
    SELECT count(*)::bigint,
           pg_catalog.md5(COALESCE(
               pg_catalog.string_agg(pg_catalog.to_jsonb(e)::text, E'\\x1f' ORDER BY e.id),
               ''
           ))
      INTO v_other_count_before, v_other_fingerprint_before
      FROM public.data_entitlements AS e
     WHERE e.id <> v_target.id;

    SELECT count(*) FILTER (
               WHERE a.action = current_setting('operator.entitlement.amend.action')
                 AND a.actor_role = 'operator'
                 AND a.actor_user_id = current_setting('operator.entitlement.amend.new_manager')::uuid
                 AND a.target_type = current_setting('operator.entitlement.amend.target_type')
                 AND a.target_id = current_setting('operator.entitlement.amend.target_id')
                 AND jsonb_typeof(a.before_json) = 'object'
                 AND jsonb_typeof(a.after_json) = 'object'
                 AND a.before_json ? 'executing_release_revision'
                 AND a.after_json ? 'executing_release_revision'
                 AND (a.before_json ->> 'executing_release_revision') ~ '^[0-9a-f]{40}$'
                 AND (a.after_json ->> 'executing_release_revision') ~ '^[0-9a-f]{40}$'
                 AND (a.before_json ->> 'executing_release_revision') =
                     (a.after_json ->> 'executing_release_revision')
                 AND (a.before_json - 'executing_release_revision') IS NOT DISTINCT FROM
                     current_setting('operator.entitlement.amend.before_semantic_json')::jsonb
                 AND (a.after_json - 'executing_release_revision') IS NOT DISTINCT FROM
                     current_setting('operator.entitlement.amend.after_semantic_json')::jsonb
                 AND a.reason = current_setting('operator.entitlement.amend.reason')
                 AND a.correlation_id = current_setting('operator.entitlement.amend.correlation_id')
           )::bigint,
           count(*) FILTER (
               WHERE a.action = current_setting('operator.entitlement.amend.action')
                 AND a.target_type = current_setting('operator.entitlement.amend.target_type')
                 AND a.target_id = current_setting('operator.entitlement.amend.target_id')
                 AND a.correlation_id = current_setting('operator.entitlement.amend.correlation_id')
           )::bigint,
           count(*) FILTER (
               WHERE a.action = current_setting('operator.entitlement.amend.action')
                 AND a.target_type = current_setting('operator.entitlement.amend.target_type')
                 AND a.target_id = current_setting('operator.entitlement.amend.target_id')
           )::bigint
      INTO v_audit_exact_count, v_audit_correlation_count, v_audit_target_count
      FROM public.audit_logs AS a;

    v_post_match := v_target.contract_document_sha256 = current_setting('operator.entitlement.amend.new_hash')
        AND v_target.contract_reference = current_setting('operator.entitlement.amend.reference')
        AND v_target.status = current_setting('operator.entitlement.amend.status')
        AND v_target.covered_datasets IS NOT DISTINCT FROM
            current_setting('operator.entitlement.amend.new_datasets')::jsonb
        AND v_target.covered_uses IS NOT DISTINCT FROM
            current_setting('operator.entitlement.amend.new_uses')::jsonb
        AND v_target.effective_from = current_setting('operator.entitlement.amend.new_from')::date
        AND v_target.effective_until = current_setting('operator.entitlement.amend.until')::date
        AND v_target.managed_by = current_setting('operator.entitlement.amend.new_manager')::uuid;
    v_old_match := v_target.contract_document_sha256 = current_setting('operator.entitlement.amend.old_hash')
        AND v_target.contract_reference = current_setting('operator.entitlement.amend.reference')
        AND v_target.status = current_setting('operator.entitlement.amend.status')
        AND v_target.covered_datasets IS NOT DISTINCT FROM
            current_setting('operator.entitlement.amend.old_datasets')::jsonb
        AND v_target.covered_uses IS NOT DISTINCT FROM
            current_setting('operator.entitlement.amend.old_uses')::jsonb
        AND v_target.effective_from = current_setting('operator.entitlement.amend.old_from')::date
        AND v_target.effective_until = current_setting('operator.entitlement.amend.until')::date
        AND v_target.managed_by = current_setting('operator.entitlement.amend.old_manager')::uuid;

    IF v_post_match THEN
        IF v_audit_exact_count <> 1 OR v_audit_correlation_count <> 1
           OR v_audit_target_count <> 1 THEN
            RAISE EXCEPTION 'amendment post-state audit is missing or ambiguous';
        END IF;
        SELECT count(*)::bigint,
               pg_catalog.md5(COALESCE(
                   pg_catalog.string_agg(pg_catalog.to_jsonb(e)::text, E'\\x1f' ORDER BY e.id),
                   ''
               ))
          INTO v_other_count_after, v_other_fingerprint_after
          FROM public.data_entitlements AS e
         WHERE e.id <> v_target.id;
        IF v_other_count_after <> v_other_count_before
           OR v_other_fingerprint_after IS DISTINCT FROM v_other_fingerprint_before THEN
            RAISE EXCEPTION 'amendment non-target state changed';
        END IF;
        PERFORM pg_catalog.set_config('operator.entitlement.amend.result', 'ALREADY_APPLIED', true);
        RETURN;
    END IF;

    IF NOT v_old_match OR v_audit_exact_count <> 0 OR v_audit_correlation_count <> 0
       OR v_audit_target_count <> 0 THEN
        RAISE EXCEPTION 'amendment compare-and-swap precondition failed';
    END IF;

    UPDATE public.data_entitlements
       SET contract_document_sha256 = current_setting('operator.entitlement.amend.new_hash'),
           effective_from = current_setting('operator.entitlement.amend.new_from')::date,
           managed_by = current_setting('operator.entitlement.amend.new_manager')::uuid,
           updated_at = pg_catalog.clock_timestamp()
     WHERE id = v_target.id
       AND contract_document_sha256 = current_setting('operator.entitlement.amend.old_hash')
       AND contract_reference = current_setting('operator.entitlement.amend.reference')
       AND status = current_setting('operator.entitlement.amend.status')
       AND covered_datasets IS NOT DISTINCT FROM
           current_setting('operator.entitlement.amend.old_datasets')::jsonb
       AND covered_uses IS NOT DISTINCT FROM
           current_setting('operator.entitlement.amend.old_uses')::jsonb
       AND effective_from = current_setting('operator.entitlement.amend.old_from')::date
       AND effective_until = current_setting('operator.entitlement.amend.until')::date
       AND managed_by = current_setting('operator.entitlement.amend.old_manager')::uuid;
    GET DIAGNOSTICS v_changed_count = ROW_COUNT;
    IF v_changed_count <> 1 THEN
        RAISE EXCEPTION 'amendment compare-and-swap changed unexpected rows';
    END IF;

    INSERT INTO public.audit_logs (
        action, actor_role, actor_user_id, target_type, target_id,
        before_json, after_json, reason, correlation_id
    ) VALUES (
        current_setting('operator.entitlement.amend.action'),
        'operator',
        current_setting('operator.entitlement.amend.new_manager')::uuid,
        current_setting('operator.entitlement.amend.target_type'),
        current_setting('operator.entitlement.amend.target_id'),
        current_setting('operator.entitlement.amend.before_json')::jsonb,
        current_setting('operator.entitlement.amend.after_json')::jsonb,
        current_setting('operator.entitlement.amend.reason'),
        current_setting('operator.entitlement.amend.correlation_id')
    );

    SELECT count(*)::bigint
      INTO v_post_reference_count
      FROM public.data_entitlements AS e
     WHERE e.contract_reference = current_setting('operator.entitlement.amend.reference');
    SELECT count(*)::bigint
      INTO v_post_count
      FROM public.data_entitlements AS e
     WHERE e.id = v_target.id
       AND e.contract_document_sha256 = current_setting('operator.entitlement.amend.new_hash')
       AND e.contract_reference = current_setting('operator.entitlement.amend.reference')
       AND e.status = current_setting('operator.entitlement.amend.status')
       AND e.covered_datasets IS NOT DISTINCT FROM
           current_setting('operator.entitlement.amend.new_datasets')::jsonb
       AND e.covered_uses IS NOT DISTINCT FROM
           current_setting('operator.entitlement.amend.new_uses')::jsonb
       AND e.effective_from = current_setting('operator.entitlement.amend.new_from')::date
       AND e.effective_until = current_setting('operator.entitlement.amend.until')::date
       AND e.managed_by = current_setting('operator.entitlement.amend.new_manager')::uuid
       AND e.created_at IS NOT DISTINCT FROM v_target.created_at;
    IF v_post_reference_count <> 1 OR v_post_count <> 1 THEN
        RAISE EXCEPTION 'amendment post-state is not exact';
    END IF;

    SELECT count(*) FILTER (
               WHERE a.action = current_setting('operator.entitlement.amend.action')
                 AND a.actor_role = 'operator'
                 AND a.actor_user_id = current_setting('operator.entitlement.amend.new_manager')::uuid
                 AND a.target_type = current_setting('operator.entitlement.amend.target_type')
                 AND a.target_id = current_setting('operator.entitlement.amend.target_id')
                 AND jsonb_typeof(a.before_json) = 'object'
                 AND jsonb_typeof(a.after_json) = 'object'
                 AND a.before_json ? 'executing_release_revision'
                 AND a.after_json ? 'executing_release_revision'
                 AND (a.before_json ->> 'executing_release_revision') ~ '^[0-9a-f]{40}$'
                 AND (a.after_json ->> 'executing_release_revision') ~ '^[0-9a-f]{40}$'
                 AND (a.before_json ->> 'executing_release_revision') =
                     (a.after_json ->> 'executing_release_revision')
                 AND (a.before_json - 'executing_release_revision') IS NOT DISTINCT FROM
                     current_setting('operator.entitlement.amend.before_semantic_json')::jsonb
                 AND (a.after_json - 'executing_release_revision') IS NOT DISTINCT FROM
                     current_setting('operator.entitlement.amend.after_semantic_json')::jsonb
                 AND a.reason = current_setting('operator.entitlement.amend.reason')
                 AND a.correlation_id = current_setting('operator.entitlement.amend.correlation_id')
           )::bigint,
           count(*) FILTER (
               WHERE a.action = current_setting('operator.entitlement.amend.action')
                 AND a.target_type = current_setting('operator.entitlement.amend.target_type')
                 AND a.target_id = current_setting('operator.entitlement.amend.target_id')
                 AND a.correlation_id = current_setting('operator.entitlement.amend.correlation_id')
           )::bigint,
           count(*) FILTER (
               WHERE a.action = current_setting('operator.entitlement.amend.action')
                 AND a.target_type = current_setting('operator.entitlement.amend.target_type')
                 AND a.target_id = current_setting('operator.entitlement.amend.target_id')
           )::bigint
      INTO v_audit_exact_count, v_audit_correlation_count, v_audit_target_count
      FROM public.audit_logs AS a;
    IF v_audit_exact_count <> 1 OR v_audit_correlation_count <> 1
       OR v_audit_target_count <> 1 THEN
        RAISE EXCEPTION 'amendment audit state is not exact';
    END IF;

    SELECT count(*)::bigint,
           pg_catalog.md5(COALESCE(
               pg_catalog.string_agg(pg_catalog.to_jsonb(e)::text, E'\\x1f' ORDER BY e.id),
               ''
           ))
      INTO v_other_count_after, v_other_fingerprint_after
      FROM public.data_entitlements AS e
     WHERE e.id <> v_target.id;
    IF v_other_count_after <> v_other_count_before
       OR v_other_fingerprint_after IS DISTINCT FROM v_other_fingerprint_before THEN
        RAISE EXCEPTION 'amendment non-target state changed';
    END IF;
    PERFORM pg_catalog.set_config('operator.entitlement.amend.result', 'APPLIED', true);
END
$body$;
SELECT current_setting('operator.entitlement.amend.result'),
       (SELECT count(*)::bigint FROM public.data_entitlements
         WHERE contract_reference = current_setting('operator.entitlement.amend.reference')),
       (SELECT count(*)::bigint FROM public.audit_logs
         WHERE action = current_setting('operator.entitlement.amend.action')
           AND target_type = current_setting('operator.entitlement.amend.target_type')
           AND target_id = current_setting('operator.entitlement.amend.target_id')
           AND correlation_id = current_setting('operator.entitlement.amend.correlation_id'));
COMMIT;
SQL
}

amend_db_row() {
  local column=$1 columns=${2:-3}
  awk -F '\t' -v column="$column" -v columns="$columns" 'NF == columns { print $column; exit }'
}

amend_db_init_safely() {
  local output_file=$1
  if ! (db_init >"$output_file" 2>&1); then
    amend_blocked db_unavailable
  fi
  db_init
}

amend_run_db() {
  local mode=$1 sql_file=$2 output_file=$3 error_file=$4
  if ! db_psql -qAt -F $'\t' \
      -v target_id="$amend_target_id" \
      -v reference="$amend_reference" \
      -v current_owner="$amend_current_owner" \
      -v old_hash="$amend_old_document_hash" \
      -v new_hash="$amend_new_document_hash" \
      -v old_datasets="$amend_old_datasets" \
      -v new_datasets="$amend_new_datasets" \
      -v old_uses="$amend_old_uses" \
      -v new_uses="$amend_new_uses" \
      -v old_from="$amend_old_from" \
      -v new_from="$amend_new_from" \
      -v until_date="$amend_until" \
      -v old_manager="$amend_old_manager" \
      -v new_manager="$amend_current_owner" \
      -v status="$AMEND_LIFECYCLE" \
      -v audit_action="$AMEND_AUDIT_ACTION" \
      -v audit_target_type="$AMEND_AUDIT_TARGET_TYPE" \
      -v audit_reason="$AMEND_AUDIT_REASON" \
      -v correlation_id="$amend_correlation_id" \
      -v before_json="$amend_before_json" \
      -v after_json="$amend_after_json" \
      -v before_semantic_json="$amend_before_semantic_json" \
      -v after_semantic_json="$amend_after_semantic_json" \
      <"$sql_file" >"$output_file" 2>"$error_file"; then
    amend_error "${mode}_database"
  fi
  cat "$output_file"
}

entitlement_amend_main() {
  local mode=plan current_metadata_file= current_document_file=
  local original_metadata_file= original_document_file= target_id=
  local old_manager= current_owner= amendment_revision= executing_revision=
  local env_file= confirmation= arg
  while [ "$#" -gt 0 ]; do
    arg=$1
    case "$arg" in
      --mode) [ "$#" -ge 2 ] || amend_error option_value; mode=$2; shift 2 ;;
      --current-metadata-file) [ "$#" -ge 2 ] || amend_error option_value; current_metadata_file=$2; shift 2 ;;
      --current-document-file) [ "$#" -ge 2 ] || amend_error option_value; current_document_file=$2; shift 2 ;;
      --original-metadata-file) [ "$#" -ge 2 ] || amend_error option_value; original_metadata_file=$2; shift 2 ;;
      --original-document-file) [ "$#" -ge 2 ] || amend_error option_value; original_document_file=$2; shift 2 ;;
      --target-id|--entitlement-id|--expected-entitlement-id) [ "$#" -ge 2 ] || amend_error option_value; target_id=$2; shift 2 ;;
      --expected-old-manager|--expected-old-manager-uuid) [ "$#" -ge 2 ] || amend_error option_value; old_manager=$2; shift 2 ;;
      --current-owner|--current-owner-uuid) [ "$#" -ge 2 ] || amend_error option_value; current_owner=$2; shift 2 ;;
      --amendment-revision|--approved-amendment-revision) [ "$#" -ge 2 ] || amend_error option_value; amendment_revision=$2; shift 2 ;;
      --executing-release-revision|--accepted-release-revision) [ "$#" -ge 2 ] || amend_error option_value; executing_revision=$2; shift 2 ;;
      --env-file) [ "$#" -ge 2 ] || amend_error option_value; env_file=$2; shift 2 ;;
      --confirm) [ "$#" -ge 2 ] || amend_error option_value; confirmation=$2; shift 2 ;;
      *) amend_error option_unknown ;;
    esac
  done

  case "$mode" in
    plan|check|apply) ;;
    *) amend_error mode ;;
  esac
  [ -n "$current_metadata_file" ] || amend_blocked input_missing
  [ -n "$current_document_file" ] || amend_blocked input_missing
  [ -n "$original_metadata_file" ] || amend_blocked input_missing
  [ -n "$original_document_file" ] || amend_blocked input_missing
  [ -n "$target_id" ] || amend_blocked identity_missing
  [ -n "$old_manager" ] || amend_blocked identity_missing
  [ -n "$current_owner" ] || amend_blocked identity_missing
  [ -n "$amendment_revision" ] || amend_blocked revision_missing
  [ -n "$executing_revision" ] || amend_blocked revision_missing
  amend_valid_uuid "$target_id" || amend_error identity_invalid
  amend_valid_uuid "$old_manager" || amend_error identity_invalid
  amend_valid_uuid "$current_owner" || amend_error identity_invalid
  amend_revision_is_approved "$amendment_revision" || amend_error revision_unapproved
  [[ "$executing_revision" =~ ^[0-9a-fA-F]{40}$ ]] || amend_error revision_invalid
  [ "$mode" != apply ] || [ "$(id -u)" -eq 0 ] || amend_error apply_root
  if [ "$mode" = apply ] && [ "$confirmation" != I_UNDERSTAND_AMEND_ACTIVE_ENTITLEMENT ]; then
    amend_confirmation_error
  fi

  target_id=$(amend_lower "$target_id")
  old_manager=$(amend_lower "$old_manager")
  current_owner=$(amend_lower "$current_owner")
  amendment_revision=$(amend_normalize_revision "$amendment_revision")
  executing_revision=$(amend_lower "$executing_revision")

  local original_pair current_pair release_context
  original_pair=$(amend_validate_pair "$original_metadata_file" "$original_document_file" \
    "$AMEND_ORIGINAL_METADATA_SHA256" "$AMEND_ORIGINAL_DOCUMENT_SHA256" "$AMEND_ORIGINAL_FROM") || exit $?
  current_pair=$(amend_validate_pair "$current_metadata_file" "$current_document_file" \
    "$AMEND_APPROVED_METADATA_SHA256" "$AMEND_APPROVED_DOCUMENT_SHA256" "$AMEND_APPROVED_FROM") || exit $?
  amend_old_metadata_hash=${original_pair%%$'\t'*}
  amend_old_document_hash=${original_pair#*$'\t'}
  amend_new_metadata_hash=${current_pair%%$'\t'*}
  amend_new_document_hash=${current_pair#*$'\t'}
  amend_target_id=$target_id
  amend_old_manager=$old_manager
  amend_current_owner=$current_owner
  amend_old_from=$AMEND_ORIGINAL_FROM
  amend_new_from=$AMEND_APPROVED_FROM
  amend_until=$AMEND_EFFECTIVE_UNTIL
  amend_reference=$AMEND_DOCUMENT_REFERENCE
  amend_old_datasets=$AMEND_DATASETS
  amend_new_datasets=$AMEND_DATASETS
  amend_old_uses=$AMEND_USES
  amend_new_uses=$AMEND_USES

  # Check/apply must obtain executor identity from a trusted installed release.
  # A plan remains offline even from a source checkout; it reports whether the
  # same evidence was available instead of treating its caller claim as proof.
  [ -z "$env_file" ] || export LAGRANGE_ENV_FILE="$env_file"
  if [ "$mode" = plan ]; then
    if entitlement_amend_verify_installed_release "$executing_revision" \
        "$AMEND_ENTITLEMENT_ENTRYPOINT_SOURCE"; then
      executing_revision=$ENTITLEMENT_AMEND_VERIFIED_RELEASE_REVISION
      release_context=verified
    else
      release_context=unavailable
    fi
  else
    if ! entitlement_amend_verify_installed_release "$executing_revision" \
        "$AMEND_ENTITLEMENT_ENTRYPOINT_SOURCE"; then
      amend_blocked "release_context_${ENTITLEMENT_AMEND_RELEASE_CONTEXT_ERROR:-unavailable}"
    fi
    executing_revision=$ENTITLEMENT_AMEND_VERIFIED_RELEASE_REVISION
    release_context=verified
  fi

  amend_before_json=$(amend_json_for_audit "$original_metadata_file" "$amend_old_metadata_hash" "$amend_old_document_hash" "$target_id" "$old_manager" "$amendment_revision" "$executing_revision") || amend_error audit_metadata
  amend_after_json=$(amend_json_for_audit "$current_metadata_file" "$amend_new_metadata_hash" "$amend_new_document_hash" "$target_id" "$current_owner" "$amendment_revision" "$executing_revision") || amend_error audit_metadata
  amend_before_semantic_json=$(amend_json_semantic_for_replay "$amend_before_json") || amend_error audit_metadata
  amend_after_semantic_json=$(amend_json_semantic_for_replay "$amend_after_json") || amend_error audit_metadata

  local correlation_input correlation_digest
  correlation_input=$(mktemp /tmp/lagrange-entitlement-amend-correlation.XXXXXX) || amend_error temp
  chmod 0600 "$correlation_input"
  printf '%s\0' "$AMEND_AUDIT_ACTION" "$target_id" "$amend_reference" \
    "$amend_old_metadata_hash" "$amend_new_metadata_hash" "$amend_old_document_hash" \
    "$amend_new_document_hash" "$amend_old_from" "$amend_new_from" "$old_manager" \
    "$current_owner" "$amendment_revision" >"$correlation_input"
  correlation_digest=$(sha256sum -- "$correlation_input" | awk '{print $1}') || amend_error temp
  rm -f -- "$correlation_input"
  amend_correlation_id="entitlement.approved_document.amended:v2:$correlation_digest"

  if [ "$mode" = plan ]; then
    printf 'ENTITLEMENT_AMEND_PLAN: PASS metadata_pairs=2 documents=2 database_queries=0 mutations=0 release_context=%s\n' "$release_context"
    return 0
  fi

  local db_init_output sql_file output_file error_file result state rows audits
  db_init_output=$(mktemp /tmp/lagrange-entitlement-amend-db-init.XXXXXX) || amend_error temp
  sql_file=$(mktemp /tmp/lagrange-entitlement-amend-sql.XXXXXX) || amend_error temp
  output_file=$(mktemp /tmp/lagrange-entitlement-amend-output.XXXXXX) || amend_error temp
  error_file=$(mktemp /tmp/lagrange-entitlement-amend-error.XXXXXX) || amend_error temp
  chmod 0600 "$db_init_output" "$sql_file" "$output_file" "$error_file"
  trap "rm -f -- '$db_init_output' '$sql_file' '$output_file' '$error_file'" EXIT
  amend_db_init_safely "$db_init_output"

  if [ "$mode" = check ]; then
    amend_write_check_sql "$sql_file"
    result=$(amend_run_db check "$sql_file" "$output_file" "$error_file")
    state=$(printf '%s' "$result" | amend_db_row 1 5)
    rows=$(printf '%s' "$result" | amend_db_row 2 5)
    audits=$(printf '%s' "$result" | amend_db_row 4 5)
    [ -n "$state" ] && [ -n "$rows" ] && [ -n "$audits" ] || amend_error check_result
    case "$state" in
      ELIGIBLE|ALREADY_APPLIED)
        printf 'ENTITLEMENT_AMEND_CHECK: PASS state=%s rows=%s audits=%s mutations=0\n' "$state" "$rows" "$audits"
        ;;
      *) amend_error check_precondition ;;
    esac
    return 0
  fi

  amend_write_apply_sql "$sql_file"
  result=$(amend_run_db apply "$sql_file" "$output_file" "$error_file")
  state=$(printf '%s' "$result" | amend_db_row 1)
  rows=$(printf '%s' "$result" | amend_db_row 2)
  audits=$(printf '%s' "$result" | amend_db_row 3)
  [ -n "$state" ] && [ -n "$rows" ] && [ -n "$audits" ] || amend_error apply_result
  case "$state" in
    APPLIED|ALREADY_APPLIED)
      printf 'ENTITLEMENT_AMEND_APPLY: PASS state=%s rows=%s audits=%s\n' "$state" "$rows" "$audits"
      ;;
    *) amend_error apply_result ;;
  esac
}
