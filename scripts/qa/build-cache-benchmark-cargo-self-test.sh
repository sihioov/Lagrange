#!/usr/bin/env bash
# Focused, Docker/Cargo-free regression test for the common Cargo JSON parser.
# The production function is extracted at run time; this file deliberately
# contains fixtures and assertions, not a second copy of that validator.
set -euo pipefail
umask 077

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
production_script=$script_dir/build-cache-benchmark.sh

usage() {
  cat <<'EOF'
Usage: scripts/qa/build-cache-benchmark-cargo-self-test.sh [--help]

Extract and exercise record_common_cargo_units from the production benchmark
script with synthetic Cargo JSONL.  This focused test never invokes Docker,
Cargo, a build, a provider, a service, or a runtime action.
EOF
}

case "$#" in
  0) ;;
  1)
    case "$1" in
      -h|--help) usage; exit 0 ;;
      *)
        printf '%s\n' 'build-cache-benchmark-cargo-self-test: unknown option' >&2
        usage >&2
        exit 2
        ;;
    esac
    ;;
  *)
    printf '%s\n' 'build-cache-benchmark-cargo-self-test: accepts no arguments' >&2
    usage >&2
    exit 2
    ;;
esac

[ -f "$production_script" ] && [ ! -L "$production_script" ] || {
  printf 'build-cache-benchmark-cargo-self-test: production script is missing: %s\n' \
    "$production_script" >&2
  exit 1
}

run_dir=$(mktemp -d /tmp/wp13-common-cargo-XXXXXX)
chmod 0700 -- "$run_dir"
cases_dir=$run_dir/cases
mkdir -m 0700 -- "$cases_dir"
report=$run_dir/report.tsv
function_source=$run_dir/record_common_cargo_units.sh
function_sha256=not-extracted
pass_count=0
fail_count=0
failure_reason=
result_lines=()

write_report() {
  local exit_status=$1 entry label expected actual status detail
  {
    printf 'format\twp13-common-cargo-self-test-v1\n'
    printf 'run_dir\t%s\n' "$run_dir"
    printf 'script_exit_status\t%s\n' "$exit_status"
    printf 'production_script\t%s\n' "$production_script"
    printf 'extracted_function_sha256\t%s\n' "$function_sha256"
    printf 'docker_cargo_build_or_runtime\tnot-run\n'
    printf 'checks_passed\t%s\n' "$pass_count"
    printf 'checks_failed\t%s\n' "$fail_count"
    if [ -n "$failure_reason" ]; then
      printf 'failure_reason\t%s\n' "$failure_reason"
    fi
    printf 'check\texpected\tactual\tstatus\tdetail\n'
    for entry in "${result_lines[@]}"; do
      IFS=$'\t' read -r label expected actual status detail <<<"$entry"
      printf '%s\t%s\t%s\t%s\t%s\n' \
        "$label" "$expected" "$actual" "$status" "$detail"
    done
  } >"$report"
  chmod 0600 -- "$report"
}

on_exit() {
  local exit_status=$?
  write_report "$exit_status"
  if [ "$exit_status" -eq 0 ]; then
    printf 'WP13_COMMON_CARGO_SELF_TEST: PASS\nreport: %s\nchecks: %s\n' \
      "$report" "$pass_count"
  else
    printf 'WP13_COMMON_CARGO_SELF_TEST: FAIL\nreport: %s\nchecks_passed: %s\nchecks_failed: %s\n' \
      "$report" "$pass_count" "$fail_count" >&2
  fi
  exit "$exit_status"
}
trap on_exit EXIT

fail_test() {
  failure_reason=$1
  fail_count=$((fail_count + 1))
  printf 'build-cache-benchmark-cargo-self-test: %s\nreport: %s\n' \
    "$failure_reason" "$report" >&2
  exit 1
}

record_result() {
  local label=$1 expected=$2 actual=$3 status=$4 detail=${5:-}
  result_lines+=("$label"$'\t'"$expected"$'\t'"$actual"$'\t'"$status"$'\t'"$detail")
  if [ "$status" = PASS ]; then
    pass_count=$((pass_count + 1))
  else
    fail_count=$((fail_count + 1))
  fi
}

if ! awk '
  /^record_common_cargo_units\(\) \{$/ { started=1 }
  started { print }
  started && $0 == "}" { ended=1; exit }
  END { if (!started || !ended) exit 1 }
' "$production_script" >"$function_source"; then
  fail_test 'could not extract the production record_common_cargo_units function'
fi
chmod 0600 -- "$function_source"
[ -s "$function_source" ] || fail_test 'extracted production function is empty'
grep -Fq -- 'record_common_cargo_units() {' "$function_source" ||
  fail_test 'extracted source does not contain the production function'
grep -Fq -- 'os.O_EXCL' "$function_source" ||
  fail_test 'extracted source does not contain the production atomic output gate'
function_sha256=$(sha256sum -- "$function_source" | awk '{print $1}')
if ! source "$function_source"; then
  fail_test 'extracted production function could not be loaded'
fi
declare -F record_common_cargo_units >/dev/null ||
  fail_test 'extracted production function was not defined'

write_fixture() {
  local case_dir=$1 variant=$2
  python3 - "$case_dir" "$variant" <<'PY'
import json
import os
import sys

case_dir, variant = sys.argv[1:3]
missing = object()


def artifact(name, kind, crate_types, executable):
    value = {
        "reason": "compiler-artifact",
        "package_id": "fixture " + name + " 0.1.0 (path+file:///build)",
        "target": {"name": name, "kind": kind, "crate_types": crate_types},
        "profile": {"opt_level": "3", "debug_assertions": False},
        "features": [],
        "fresh": False,
    }
    if executable is not missing:
        value["executable"] = executable
    return value


def finished(success=True):
    return {"reason": "build-finished", "success": success}


def encoded(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


library = artifact("fixture-library", ["lib"], ["lib"], None)
custom_build = artifact("fixture-build-script", ["custom-build"], ["bin"], None)
valid_binary = artifact(
    "selected-bin", ["bin"], ["bin"], "/cargo-target/release/selected-bin"
)

if variant == "library-null":
    events = [library, valid_binary, finished()]
elif variant == "custom-build-null":
    events = [custom_build, valid_binary, finished()]
elif variant == "binary-null":
    events = [artifact("selected-bin", ["bin"], ["bin"], None), finished()]
elif variant == "missing-executable":
    events = [artifact("selected-bin", ["bin"], ["bin"], missing), finished()]
elif variant == "executable-bool":
    events = [artifact("selected-bin", ["bin"], ["bin"], True), finished()]
elif variant == "executable-int":
    events = [artifact("selected-bin", ["bin"], ["bin"], 7), finished()]
elif variant == "executable-list":
    events = [artifact("selected-bin", ["bin"], ["bin"], []), finished()]
elif variant == "executable-dict":
    events = [artifact("selected-bin", ["bin"], ["bin"], {}), finished()]
elif variant == "wrong-target-path":
    events = [artifact("selected-bin", ["bin"], ["bin"], "/cargo-target/debug/selected-bin"), finished()]
elif variant == "missing-finish":
    events = [valid_binary]
elif variant == "false-finish":
    events = [valid_binary, finished(False)]
elif variant == "duplicate-finish":
    events = [valid_binary, finished(), finished()]
elif variant == "noncanonical-receipt":
    events = [library, valid_binary, finished()]
elif variant == "malformed-json":
    events = []
elif variant == "duplicate-key":
    events = []
elif variant == "nonfinite":
    events = []
else:
    raise SystemExit("unknown fixture variant: " + variant)

if variant == "noncanonical-receipt":
    receipt_raw = b'{"bin": "selected-bin"}\n'
else:
    receipt_raw = encoded({"bin": "selected-bin"})

if variant == "malformed-json":
    cargo_raw = b'{"reason":"compiler-artifact"\n'
elif variant == "duplicate-key":
    duplicate = dict(valid_binary)
    del duplicate["reason"]
    duplicate_raw = encoded(duplicate).decode()
    cargo_raw = (
        '{"reason":"compiler-artifact","reason":"compiler-artifact",'
        + duplicate_raw[1:]
    ).encode()
elif variant == "nonfinite":
    nonfinite = dict(valid_binary)
    nonfinite["profile"] = {"opt_level": float("nan")}
    cargo_raw = encoded(nonfinite)
else:
    cargo_raw = b"".join(encoded(event) for event in events)

with open(os.path.join(case_dir, "artifact.json"), "wb") as handle:
    handle.write(receipt_raw)
with open(os.path.join(case_dir, "cargo.jsonl"), "wb") as handle:
    handle.write(cargo_raw)
PY
}

assert_success_output() {
  python3 - "$1" "$2" "$3" "$4" "$5" "$6" <<'PY'
import hashlib
import json
import re
import stat
import sys

stdout_path, cargo_path, units_path, count_text, null_target, names_text = sys.argv[1:]
expected_count = int(count_text)
expected_names = names_text.split(",")
stdout = open(stdout_path, "rb").read()
cargo_raw = open(cargo_path, "rb").read()
units_raw = open(units_path, "rb").read()

if stdout.count(b"\n") != 1 or not stdout.endswith(b"\n"):
    raise SystemExit("success-output-line-count-invalid")
try:
    fields = stdout[:-1].decode("ascii").split("\t")
except UnicodeDecodeError:
    raise SystemExit("success-output-not-ascii")
if len(fields) != 3 or any(not re.fullmatch(r"[0-9a-f]{64}", value) for value in fields[:2]):
    raise SystemExit("success-output-fields-invalid")
if not re.fullmatch(r"[0-9]+", fields[2]):
    raise SystemExit("success-output-count-invalid")

if fields[0] != hashlib.sha256(units_raw).hexdigest():
    raise SystemExit("success-output-units-hash-mismatch")
if fields[1] != hashlib.sha256(cargo_raw).hexdigest():
    raise SystemExit("success-output-cargo-hash-mismatch")
if int(fields[2]) != expected_count:
    raise SystemExit("success-output-count-mismatch")

if stat.S_IMODE(__import__("os").stat(units_path).st_mode) != 0o600:
    raise SystemExit("success-output-mode-invalid")
value = json.loads(units_raw.decode("utf-8"))
canonical = (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()
if units_raw != canonical:
    raise SystemExit("success-output-not-canonical")
if value.get("format") != "lagrange-benchmark-cargo-units-v1":
    raise SystemExit("success-output-format-invalid")
if value.get("source") != "common-producer-receipt":
    raise SystemExit("success-output-source-invalid")
if value.get("raw_log_sha256") != fields[1]:
    raise SystemExit("success-output-raw-hash-mismatch")
units = value.get("units")
if not isinstance(units, list) or len(units) != expected_count:
    raise SystemExit("success-output-units-count-invalid")
names = [unit.get("target", {}).get("name") for unit in units]
if names != expected_names:
    raise SystemExit("success-output-unit-set-mismatch")
nullable = [unit for unit in units if unit.get("target", {}).get("name") == null_target]
if len(nullable) != 1 or "executable" not in nullable[0] or nullable[0]["executable"] is not None:
    raise SystemExit("success-output-null-executable-not-retained")
selected = [unit for unit in units if unit.get("target", {}).get("name") == "selected-bin"]
if len(selected) != 1 or selected[0].get("executable") != "/cargo-target/release/selected-bin":
    raise SystemExit("success-output-selected-target-invalid")
PY
}

run_success_case() {
  local name=$1 variant=$2 expected_count=$3 null_target=$4 expected_names=$5
  local case_dir=$cases_dir/$name
  local receipt=$case_dir/artifact.json cargo=$case_dir/cargo.jsonl units=$case_dir/units.json
  local stdout=$case_dir/stdout stderr=$case_dir/stderr assert_stdout=$case_dir/assert.stdout
  local assert_stderr=$case_dir/assert.stderr status
  mkdir -m 0700 -- "$case_dir"
  write_fixture "$case_dir" "$variant"
  if record_common_cargo_units "$receipt" "$cargo" "$units" >"$stdout" 2>"$stderr"; then
    status=0
  else
    status=$?
  fi
  [ "$status" -eq 0 ] || fail_test "$name unexpectedly failed; case evidence retained"
  [ ! -s "$stderr" ] || fail_test "$name wrote unexpected stderr; case evidence retained"
  [ -s "$stdout" ] || fail_test "$name wrote no success result; case evidence retained"
  if ! assert_success_output "$stdout" "$cargo" "$units" "$expected_count" \
    "$null_target" "$expected_names" >"$assert_stdout" 2>"$assert_stderr"; then
    fail_test "$name success hashes/counts/unit assertions failed; case evidence retained"
  fi
  [ ! -s "$assert_stdout" ] || fail_test "$name verifier wrote unexpected stdout"
  record_result "$name" 'exit=0; hashes/counts/unit set valid' "exit=$status; output verified" PASS \
    'production function extracted at runtime'
}

run_failure_case() {
  local name=$1 variant=$2 expected_fragment=$3
  local case_dir=$cases_dir/$name
  local receipt=$case_dir/artifact.json cargo=$case_dir/cargo.jsonl units=$case_dir/units.json
  local stdout=$case_dir/stdout stderr=$case_dir/stderr status
  mkdir -m 0700 -- "$case_dir"
  write_fixture "$case_dir" "$variant"
  if record_common_cargo_units "$receipt" "$cargo" "$units" >"$stdout" 2>"$stderr"; then
    status=0
  else
    status=$?
  fi
  [ "$status" -ne 0 ] || fail_test "$name was incorrectly accepted; case evidence retained"
  [ ! -s "$stdout" ] || fail_test "$name emitted success stdout on a negative path"
  [ ! -e "$units" ] && [ ! -L "$units" ] ||
    fail_test "$name published units on a negative path"
  grep -Fq -- "$expected_fragment" "$stderr" ||
    fail_test "$name did not report expected error/stage $expected_fragment"
  record_result "$name" "exit!=0; stderr contains $expected_fragment" \
    "exit=$status; stdout/artifact empty" PASS 'negative-path assertion'
}

run_preexisting_case() {
  local name=preexisting-destination variant=library-null
  local case_dir=$cases_dir/$name
  local receipt=$case_dir/artifact.json cargo=$case_dir/cargo.jsonl units=$case_dir/units.json
  local stdout=$case_dir/stdout stderr=$case_dir/stderr before after status
  mkdir -m 0700 -- "$case_dir"
  write_fixture "$case_dir" "$variant"
  printf '%s\n' 'preexisting-output-must-survive' >"$units"
  before=$(sha256sum -- "$units" | awk '{print $1}')
  if record_common_cargo_units "$receipt" "$cargo" "$units" >"$stdout" 2>"$stderr"; then
    status=0
  else
    status=$?
  fi
  after=$(sha256sum -- "$units" | awk '{print $1}')
  [ "$status" -ne 0 ] || fail_test 'preexisting destination was incorrectly overwritten'
  [ ! -s "$stdout" ] && [ ! -s "$stderr" ] ||
    fail_test 'preexisting destination path emitted unexpected output'
  [ "$before" = "$after" ] || fail_test 'preexisting destination bytes changed'
  record_result "$name" 'exit!=0; existing bytes unchanged' \
    "exit=$status; hash=$after" PASS 'no-overwrite gate'
}

run_success_case library-null-keeps-unit library-null 2 fixture-library fixture-library,selected-bin
run_success_case custom-build-null-keeps-unit custom-build-null 2 fixture-build-script fixture-build-script,selected-bin

run_failure_case requested-binary-null binary-null common-cargo-target-missing
run_failure_case missing-executable missing-executable common-cargo-unit-invalid
run_failure_case executable-bool executable-bool common-cargo-unit-invalid
run_failure_case executable-int executable-int common-cargo-unit-invalid
run_failure_case executable-list executable-list common-cargo-unit-invalid
run_failure_case executable-dict executable-dict common-cargo-unit-invalid
run_failure_case mismatched-target-path wrong-target-path common-cargo-target-missing
run_failure_case missing-build-finished missing-finish common-cargo-units-missing
run_failure_case false-build-finished false-finish common-cargo-build-failed
run_failure_case duplicate-build-finished duplicate-finish common-cargo-units-missing
run_failure_case malformed-json malformed-json JSONDecodeError
run_failure_case duplicate-key duplicate-key duplicate-key
run_failure_case nonfinite-json nonfinite constant
run_failure_case noncanonical-receipt noncanonical-receipt common-receipt-not-canonical
run_preexisting_case

printf '%s\n' 'focused cases covered: nullable library/custom-build, selected target, required executable key, rejected executable types, finish cardinality, JSON parse/duplicate/nonfinite, canonical receipt, and no-overwrite'
