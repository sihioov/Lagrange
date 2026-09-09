#!/usr/bin/env bash
# Provider-free mutation self-test for the WP6-A artifact/schema checker.
# Every mutation is made below a throw-away mktemp directory; no operational
# file, provider, credential, clock, Docker, database, or network is touched.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
static=$root/scripts/ops/stock-beta-intraday-static-check.sh
artifact_rel=configs/market-hours/krx-intraday-session-windows-v1.json
schema_rel=configs/market-hours/krx-intraday-session-windows-v1.schema.json
static_rel=scripts/ops/stock-beta-intraday-static-check.sh
tmp=$(mktemp -d "${TMPDIR:-/tmp}/stock-beta-intraday-self-test.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

die() {
  echo "STOCK_BETA_INTRADAY_SELF_TEST: $*" >&2
  exit 1
}

[ -f "$static" ] || die 'STATIC_CHECK_MISSING'
[ ! -L "$static" ] || die 'STATIC_CHECK_SYMLINK'
command -v python3 >/dev/null 2>&1 || die 'PYTHON3_MISSING'
bash -n "$static" || die 'STATIC_CHECK_SYNTAX_INVALID'

copy_fixture() {
  local fixture=$1
  mkdir -p "$fixture/configs/market-hours" "$fixture/scripts/ops"
  cp -- "$root/$artifact_rel" "$fixture/$artifact_rel"
  cp -- "$root/$schema_rel" "$fixture/$schema_rel"
  cp -- "$root/$static_rel" "$fixture/$static_rel"
}

rewrite_json() {
  local path=$1
  local mutation=$2
  python3 - "$path" "$mutation" <<'PY'
import json
import pathlib
import sys


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate fixture key")
        result[key] = value
    return result


path = pathlib.Path(sys.argv[1])
mutation = sys.argv[2]
document = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique_object)

if mutation == "nonempty":
    document["entries"].append(
        {
            "date": "2026-09-09",
            "disposition": "REGULAR",
            "open_local": "09:00:00",
            "close_local": "15:30:00",
            "evidence_url": "https://global.krx.co.kr/fixture",
            "evidence_retrieved_at": "2026-09-09T00:00:00Z",
            "evidence_sha256": "sha256:" + ("0" * 64),
        }
    )
elif mutation == "extra":
    document["fixture_extra"] = True
elif mutation == "missing":
    del document["timezone"]
elif mutation == "bool":
    document["schema_version"] = True
elif mutation == "wrong_type":
    document["entries"] = {}
elif mutation == "schema_open":
    document["additionalProperties"] = True
elif mutation == "schema_missing_closed":
    document["$defs"]["entry"]["allOf"] = [
        clause
        for clause in document["$defs"]["entry"]["allOf"]
        if clause["if"]["properties"]["disposition"]["const"] != "CLOSED"
    ]
elif mutation == "schema_missing_date_format":
    del document["$defs"]["entry"]["properties"]["date"]["format"]
elif mutation == "schema_bool_const":
    document["properties"]["schema_version"]["const"] = True
else:
    raise ValueError("unknown fixture mutation")

path.write_text(json.dumps(document, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
PY
}

duplicate_artifact_key() {
  local path=$1
  python3 - "$path" <<'PY'
import pathlib
import sys


path = pathlib.Path(sys.argv[1])
raw = path.read_text(encoding="utf-8")
needle = '  "exchange": "KRX",\n'
if raw.count(needle) != 1:
    raise SystemExit("fixture duplicate target is not unique")
path.write_text(raw.replace(needle, needle + needle, 1), encoding="utf-8")
PY
}

pass_count=0
expected_failure_count=0
actual_nonzero_count=0

expect_pass() {
  local name=$1
  local fixture=$2
  local output status
  if output=$(bash "$fixture/$static_rel" 2>&1); then
    status=0
  else
    status=$?
  fi
  [ "$status" -eq 0 ] || {
    printf '%s\n' "$output" >&2
    die "CASE_FAILED name=$name exit=$status expected=0"
  }
  grep -Fq 'STOCK_BETA_INTRADAY_STATIC: PASS' <<<"$output" ||
    die "CASE_PASS_MARKER_MISSING name=$name"
  pass_count=$((pass_count + 1))
  printf 'STOCK_BETA_INTRADAY_SELF_TEST: case=%s exit=%s result=PASS\n' "$name" "$status"
}

expect_failure() {
  local name=$1
  local fixture=$2
  local output status
  if output=$(bash "$fixture/$static_rel" 2>&1); then
    status=0
  else
    status=$?
  fi
  [ "$status" -ne 0 ] || {
    printf '%s\n' "$output" >&2
    die "CASE_FAILED name=$name exit=$status expected=nonzero"
  }
  grep -Fq 'STOCK_BETA_INTRADAY_STATIC:' <<<"$output" ||
    die "CASE_UNTYPED_FAILURE name=$name"
  if grep -Fq 'Traceback' <<<"$output"; then
    die "CASE_UNSAFE_FAILURE_OUTPUT name=$name"
  fi
  expected_failure_count=$((expected_failure_count + 1))
  actual_nonzero_count=$((actual_nonzero_count + 1))
  printf 'STOCK_BETA_INTRADAY_SELF_TEST: case=%s exit=%s result=EXPECTED_FAILURE\n' \
    "$name" "$status"
}

baseline=$tmp/baseline
copy_fixture "$baseline"
expect_pass baseline "$baseline"

fixture=$tmp/nonempty
copy_fixture "$fixture"
rewrite_json "$fixture/$artifact_rel" nonempty
expect_failure nonempty-artifact "$fixture"

fixture=$tmp/extra
copy_fixture "$fixture"
rewrite_json "$fixture/$artifact_rel" extra
expect_failure extra-artifact-key "$fixture"

fixture=$tmp/missing
copy_fixture "$fixture"
rewrite_json "$fixture/$artifact_rel" missing
expect_failure missing-artifact-key "$fixture"

fixture=$tmp/bool
copy_fixture "$fixture"
rewrite_json "$fixture/$artifact_rel" bool
expect_failure boolean-schema-version "$fixture"

fixture=$tmp/wrong-type
copy_fixture "$fixture"
rewrite_json "$fixture/$artifact_rel" wrong_type
expect_failure wrong-artifact-type "$fixture"

fixture=$tmp/duplicate
copy_fixture "$fixture"
duplicate_artifact_key "$fixture/$artifact_rel"
expect_failure duplicate-artifact-key "$fixture"

fixture=$tmp/schema-open
copy_fixture "$fixture"
rewrite_json "$fixture/$schema_rel" schema_open
expect_failure schema-open-top-level "$fixture"

fixture=$tmp/schema-missing-conditional
copy_fixture "$fixture"
rewrite_json "$fixture/$schema_rel" schema_missing_closed
expect_failure schema-missing-closed-constraint "$fixture"

fixture=$tmp/schema-missing-format
copy_fixture "$fixture"
rewrite_json "$fixture/$schema_rel" schema_missing_date_format
expect_failure schema-missing-date-format "$fixture"

fixture=$tmp/schema-bool
copy_fixture "$fixture"
rewrite_json "$fixture/$schema_rel" schema_bool_const
expect_failure schema-boolean-const "$fixture"

[ "$pass_count" -eq 1 ] || die "COUNT_MISMATCH passes=$pass_count expected=1"
[ "$expected_failure_count" -eq 10 ] ||
  die "COUNT_MISMATCH expected_failures=$expected_failure_count expected=10"
[ "$actual_nonzero_count" -eq 10 ] ||
  die "COUNT_MISMATCH actual_nonzero=$actual_nonzero_count expected=10"

printf 'STOCK_BETA_INTRADAY_SELF_TEST: PASS cases=%s expected_failures=%s actual_nonzero=%s\n' \
  "$((pass_count + expected_failure_count))" "$expected_failure_count" "$actual_nonzero_count"
