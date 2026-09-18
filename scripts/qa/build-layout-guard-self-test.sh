#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
helper=$repo_root/scripts/ops/lib/release-build-layout.sh
layout=$repo_root/deploy/build/release-build-layout.json

if [ "$#" -ne 0 ]; then
  printf '%s\n' 'build-layout-guard-self-test: this focused test accepts no arguments' >&2
  exit 2
fi

run_dir=$(mktemp -d /tmp/wp10-c8-guard-fix-XXXXXX)
cases_dir=$run_dir/cases
report=$run_dir/report.md
mkdir -m 0700 -- "$cases_dir"

pass_count=0
fail_count=0
result_lines=()
failure_reason=
namespace=c8-guard-self-test
layout_sha=not-measured
helper_sha=not-measured
test_sha=not-measured

write_report() {
  local exit_status=$1
  local entry label expected actual status detail
  {
    printf '%s\n' '# C8 release-build-layout guard self-test'
    printf '%s\n' ''
    printf '%s\n' "- run_dir: $run_dir"
    printf '%s\n' "- script_exit_status: $exit_status"
    printf '%s\n' "- checked_in_layout_sha256: $layout_sha"
    printf '%s\n' "- production_helper_sha256: $helper_sha"
    printf '%s\n' "- test_script_sha256: $test_sha"
    printf '%s\n' "- docker_or_rust_compile: not-run"
    printf '%s\n' "- checks_passed: $pass_count"
    printf '%s\n' "- checks_failed: $fail_count"
    if [ -n "$failure_reason" ]; then
      printf '%s\n' "- failure_reason: $failure_reason"
    fi
    printf '%s\n' ''
    printf '%s\n' '| Check | Expected | Actual | Status | Evidence |'
    printf '%s\n' '|---|---|---|---|---|'
    for entry in "${result_lines[@]}"; do
      IFS=$'\t' read -r label expected actual status detail <<<"$entry"
      printf '| %s | %s | %s | %s | %s |\n' "$label" "$expected" "$actual" "$status" "$detail"
    done
  } >"$report"
}

on_exit() {
  local exit_status=$?
  if [ -f "$0" ]; then
    test_sha=$(sha256sum -- "$0" | awk '{print $1}')
  fi
  write_report "$exit_status"
  if [ "$exit_status" -eq 0 ]; then
    printf 'C8_BUILD_LAYOUT_GUARD_SELF_TEST: PASS\nreport: %s\nchecks: %s\n' \
      "$report" "$pass_count"
  else
    printf 'C8_BUILD_LAYOUT_GUARD_SELF_TEST: FAIL\nreport: %s\nchecks_passed: %s\nchecks_failed: %s\n' \
      "$report" "$pass_count" "$fail_count" >&2
  fi
  exit "$exit_status"
}
trap on_exit EXIT

fail_test() {
  failure_reason=$1
  printf 'build-layout-guard-self-test: %s\nreport: %s\n' "$failure_reason" "$report" >&2
  exit 1
}

record_result() {
  local label=$1 expected=$2 actual=$3 status=$4 detail=$5
  result_lines+=("$label"$'\t'"$expected"$'\t'"$actual"$'\t'"$status"$'\t'"$detail")
  if [ "$status" = PASS ]; then
    pass_count=$((pass_count + 1))
  else
    fail_count=$((fail_count + 1))
  fi
}

invoke_guard() {
  local variant=$1
  local guard=$2
  local request=$3
  local selected_layout=$4
  export RBL_TEST_GUARD_PATH=$guard
  export RUST_ARTIFACT_CACHE_NAMESPACE=$namespace
  case "$variant" in
    fixed) rbl_test_builder_guard "$request" "$selected_layout" ;;
    old) rbl_old_builder_guard "$request" "$selected_layout" ;;
    *) return 2 ;;
  esac
}

assert_pending() {
  python3 - "$1" "$2" "$namespace" <<'PY'
import json,os,stat,sys
guard,request_path,namespace=sys.argv[1:4]
info=os.lstat(guard)
if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode) or stat.S_IMODE(info.st_mode)!=0o600:
    raise SystemExit("pending-ledger-file-invalid")
raw=open(guard,"rb").read()
value=json.loads(raw.decode("utf-8"))
request=json.load(open(request_path,encoding="utf-8"))
binding={
    "format":"lagrange-build-target-guard-v2",
    "k_sha256":request["k_sha256"],
    "cache_key":request["cache_key"],
    "cache_namespace":namespace,
    "platform":"linux/amd64",
    "profile":"release",
    "target_dir":"/cargo-target",
    "guard_version":"common-1",
}
expected={
    "format":"lagrange-build-target-guard-v2",
    "status":"pending",
    "binding":binding,
    "package_hashes":request["package_hashes"],
}
canonical=(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8")
if raw!=canonical or value!=expected:
    raise SystemExit("pending-ledger-content-invalid")
PY
}

run_success() {
  local label=$1
  local variant=$2
  local request=$3
  local selected_layout=$4
  local guard=$5
  local expected_mode=$6
  local case_dir=$cases_dir/$label
  local stdout=$case_dir/stdout
  local stderr=$case_dir/stderr
  local status mode
  mkdir -p -m 0700 -- "$case_dir"
  if invoke_guard "$variant" "$guard" "$request" "$selected_layout" >"$stdout" 2>"$stderr"; then
    status=0
  else
    status=$?
  fi
  mode=$(tr -d '\r\n' <"$stdout")
  if [ "$status" -eq 0 ] && [ "$mode" = "$expected_mode" ] && assert_pending "$guard" "$request"; then
    record_result "$label" "exit=0 mode=$expected_mode and canonical pending ledger" \
      "exit=$status mode=$mode" PASS 'shared extracted production guard'
  else
    record_result "$label" "exit=0 mode=$expected_mode and canonical pending ledger" \
      "exit=$status mode=$mode" FAIL 'case stdout/stderr and ledger retained'
    fail_test "$label produced an unexpected successful-guard result"
  fi
}

run_failure_no_guard() {
  local label=$1
  local variant=$2
  local request=$3
  local selected_layout=$4
  local guard=$5
  local expected_error=$6
  local case_dir=$cases_dir/$label
  local stdout=$case_dir/stdout
  local stderr=$case_dir/stderr
  local status
  mkdir -p -m 0700 -- "$case_dir"
  if invoke_guard "$variant" "$guard" "$request" "$selected_layout" >"$stdout" 2>"$stderr"; then
    status=0
  else
    status=$?
  fi
  if [ "$status" -ne 0 ] && [ ! -e "$guard" ] && [ ! -L "$guard" ] &&
     grep -Fxq "ValueError: $expected_error" "$stderr"; then
    record_result "$label" "$expected_error and no ledger write" "exit=$status $expected_error and no ledger" PASS \
      'parse failure precedes atomic ledger write'
  else
    record_result "$label" "$expected_error and no ledger write" "exit=$status, wrong error or ledger exists" FAIL \
      'case stdout/stderr and ledger retained'
    fail_test "$label did not fail before ledger write"
  fi
}

run_failure_unchanged() {
  local label=$1
  local variant=$2
  local request=$3
  local selected_layout=$4
  local guard=$5
  local case_dir=$cases_dir/$label
  local stdout=$case_dir/stdout
  local stderr=$case_dir/stderr
  local before after status
  mkdir -p -m 0700 -- "$case_dir"
  before=$(sha256sum -- "$guard" | awk '{print $1}')
  if invoke_guard "$variant" "$guard" "$request" "$selected_layout" >"$stdout" 2>"$stderr"; then
    status=0
  else
    status=$?
  fi
  after=$(sha256sum -- "$guard" | awk '{print $1}')
  if [ "$status" -ne 0 ] && [ "$before" = "$after" ]; then
    record_result "$label" 'nonzero and ledger bytes unchanged' "exit=$status unchanged" PASS \
      'parse failure precedes atomic ledger replacement'
  else
    record_result "$label" 'nonzero and ledger bytes unchanged' "exit=$status changed" FAIL \
      'case stdout/stderr and ledger retained'
    fail_test "$label changed the ledger on rejected input"
  fi
}

run_symlink_failure() {
  local label=$1
  local request=$2
  local selected_layout=$3
  local guard=$4
  local target=$5
  local case_dir=$cases_dir/$label
  local stdout=$case_dir/stdout
  local stderr=$case_dir/stderr
  local status
  mkdir -p -m 0700 -- "$case_dir"
  if invoke_guard fixed "$guard" "$request" "$selected_layout" >"$stdout" 2>"$stderr"; then
    status=0
  else
    status=$?
  fi
  if [ "$status" -ne 0 ] && [ -L "$guard" ] && [ "$(cat -- "$target")" = symlink-target ]; then
    record_result "$label" 'nonzero and symlink/target preserved' \
      "exit=$status symlink-preserved" PASS 'nonregular ledger remains fatal'
  else
    record_result "$label" 'nonzero and symlink/target preserved' \
      "exit=$status symlink-or-target-changed" FAIL 'case stdout/stderr retained'
    fail_test "$label did not reject the symlink ledger"
  fi
}

extract_guard_functions() {
  python3 - "$helper" "$run_dir/fixed-guard.sh" "$run_dir/old-guard.sh" <<'PY'
import pathlib,re,sys
source,fixed_path,old_path=map(pathlib.Path,sys.argv[1:4])
text=source.read_text(encoding="utf-8")
match=re.search(r"(?ms)^rbl_builder_guard\(\) \{\n.*?^\}\n\nrbl_builder_finish_guard",text)
if match is None:
    raise SystemExit("guard-function-extraction-failed")
function=match.group(0).rsplit("\n\nrbl_builder_finish_guard",1)[0]+"\n"
needle="  rbl_guard=/cargo-target/.lagrange-build-layout-guard.json\n"
if function.count(needle)!=1:
    raise SystemExit("guard-path-assignment-not-unique")
fixed=function.replace("rbl_builder_guard() {","rbl_test_builder_guard() {",1)
fixed=fixed.replace(needle,"  rbl_guard=$RBL_TEST_GUARD_PATH\n",1)
old=function.replace("rbl_builder_guard() {","rbl_old_builder_guard() {",1)
old=old.replace(needle,"  rbl_guard=$RBL_TEST_GUARD_PATH\n",1)
replacements=[
    ('def load(path,canonical=True):','def load(path):'),
    ('    if canonical and raw != (json.dumps(value,sort_keys=True,separators=(",",":"))+"\\n").encode("utf-8"):',
     '    if raw != (json.dumps(value,sort_keys=True,separators=(",",":"))+"\\n").encode("utf-8"):'),
    ('layout=load(os.environ["RBL_LAYOUT"],canonical=False)',
     'layout=load(os.environ["RBL_LAYOUT"])'),
]
for before,after in replacements:
    if old.count(before)!=1:
        raise SystemExit("old-helper-substitution-not-unique")
    old=old.replace(before,after,1)
fixed_path.write_text("#!/bin/sh\n"+fixed,encoding="utf-8")
old_path.write_text("#!/bin/sh\n"+old,encoding="utf-8")
PY
}

write_request() {
  python3 - "$layout" "$1" <<'PY'
import json,sys
layout=json.load(open(sys.argv[1],encoding="utf-8"))
request={
    "cache_key":"22"*32,
    "k_sha256":"11"*32,
    "package_hashes":{name:"aa"*32 for name in sorted(layout["packages"])},
}
with open(sys.argv[2],"w",encoding="utf-8",newline="\n") as handle:
    json.dump(request,handle,sort_keys=True,separators=(",",":"))
    handle.write("\n")
PY
}

write_complete_ledger() {
  python3 - "$1" "$2" "$namespace" "$3" "$4" <<'PY'
import json,sys
path,request_path,namespace,changed,style=sys.argv[1:6]
request=json.load(open(request_path,encoding="utf-8"))
hashes=dict(request["package_hashes"])
if changed:
    hashes[changed]="bb"*32
binding={
    "format":"lagrange-build-target-guard-v2",
    "k_sha256":request["k_sha256"],
    "cache_key":request["cache_key"],
    "cache_namespace":namespace,
    "platform":"linux/amd64",
    "profile":"release",
    "target_dir":"/cargo-target",
    "guard_version":"common-1",
}
value={
    "format":"lagrange-build-target-guard-v2",
    "status":"complete",
    "binding":binding,
    "package_hashes":hashes,
    "last_bin":"unit-dummy",
    "last_binary_sha256":"cc"*32,
}
with open(path,"w",encoding="utf-8",newline="\n") as handle:
    if style=="pretty":
        json.dump(value,handle,sort_keys=True,indent=2)
        handle.write("\n")
    else:
        handle.write(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n")
PY
}

write_pending_ledger() {
  python3 - "$1" "$2" "$namespace" <<'PY'
import json,sys
path,request_path,namespace=sys.argv[1:4]
request=json.load(open(request_path,encoding="utf-8"))
binding={
    "format":"lagrange-build-target-guard-v2",
    "k_sha256":request["k_sha256"],
    "cache_key":request["cache_key"],
    "cache_namespace":namespace,
    "platform":"linux/amd64",
    "profile":"release",
    "target_dir":"/cargo-target",
    "guard_version":"common-1",
}
value={"format":"lagrange-build-target-guard-v2","status":"pending",
       "binding":binding,"package_hashes":request["package_hashes"]}
with open(path,"w",encoding="utf-8",newline="\n") as handle:
    handle.write(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n")
PY
}

expected_domain_closure() {
  python3 - "$layout" <<'PY'
import json,sys
value=json.load(open(sys.argv[1],encoding="utf-8"))
packages=value["packages"]
reverse={name:set() for name in packages}
for name,record in packages.items():
    for dependency in record["local_dependencies"]:
        reverse[dependency].add(name)
result={"domain"}
pending=["domain"]
while pending:
    name=pending.pop()
    for dependent in sorted(reverse[name]):
        if dependent not in result:
            result.add(dependent)
            pending.append(dependent)
print("clean:"+",".join(sorted(result)))
PY
}

assert_pretty_layout() {
  python3 - "$layout" <<'PY'
import json,sys
raw=open(sys.argv[1],"rb").read()
value=json.loads(raw.decode("utf-8"))
canonical=(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8")
if raw==canonical:
    raise SystemExit("checked-in-layout-is-not-pretty")
if not isinstance(value.get("packages"),dict) or len(value["packages"])!=16:
    raise SystemExit("checked-in-layout-package-set-invalid")
PY
}

raw_layout_hash_case() {
  local case_dir=$cases_dir/raw-layout-hash
  local request=$case_dir/request.json
  local private_helper=$case_dir/release-build-layout.sh
  local bad_layout=$case_dir/release-build-layout.json
  local native=$case_dir/expected-native-identity.json
  local output=$case_dir/output
  local sentinel_dir=$case_dir/sentinel
  local marker=$case_dir/cargo-called
  local stdout=$case_dir/stdout
  local stderr=$case_dir/stderr
  local status
  mkdir -m 0700 -- "$case_dir" "$sentinel_dir"
  printf '%s\n' '{}' >"$request"
  printf '%s\n' '{}' >"$native"
  cp -- "$helper" "$private_helper"
  cp -- "$layout" "$bad_layout"
  printf '\n' >>"$bad_layout"
  printf '%s\n' '#!/bin/sh' "printf '%s' called > '$marker'" 'exit 91' >"$sentinel_dir/cargo"
  chmod 0700 -- "$sentinel_dir/cargo"
  mkdir -m 0700 -- "$output"
  if (
    export RUST_ARTIFACT_HELPER_SHA256
    export RUST_ARTIFACT_LAYOUT_SHA256
    export CARGO_PACKAGE=unused
    export CARGO_BIN=unused
    export RUST_ARTIFACT_INPUT_SHA256=unused
    export RUST_ARTIFACT_RECIPE_SHA256=unused
    export RUST_ARTIFACT_CACHE_KEY=unused
    RUST_ARTIFACT_HELPER_SHA256=$(sha256sum -- "$private_helper" | awk '{print $1}')
    RUST_ARTIFACT_LAYOUT_SHA256=$layout_sha
    PATH="$sentinel_dir:$PATH"
    export PATH
    rbl_builder_compile "$request" "$output"
  ) >"$stdout" 2>"$stderr"; then
    status=0
  else
    status=$?
  fi
  if [ "$status" -ne 0 ] &&
     grep -Fq 'builder layout hash does not match request binding' "$stderr" &&
     [ ! -e "$marker" ] && [ ! -L "$marker" ]; then
    record_result 'raw-layout-hash-before-compiler' \
      'nonzero hash mismatch and sentinel Cargo not called' \
      "exit=$status hash-mismatch sentinel-not-called" PASS \
      'real rbl_builder_compile with private whitespace-edited layout'
  else
    record_result 'raw-layout-hash-before-compiler' \
      'nonzero hash mismatch and sentinel Cargo not called' \
      "exit=$status or sentinel-called" FAIL \
      'case stdout/stderr retained'
    fail_test 'raw layout hash guard did not stop before compiler'
  fi
}

unset RELEASE_BUILD_LAYOUT_EXECUTION
source "$helper"

layout_sha=$(sha256sum -- "$layout" | awk '{print $1}')
helper_sha=$(sha256sum -- "$helper" | awk '{print $1}')
if assert_pretty_layout; then
  record_result 'checked-in-layout-provenance' 'pretty 16-package layout' \
    "pretty 16-package layout sha=$layout_sha" PASS \
    'real deploy/build/release-build-layout.json'
else
  record_result 'checked-in-layout-provenance' 'pretty 16-package layout' \
    'layout was canonical or package count differed' FAIL \
    'real checked-in layout'
  fail_test 'checked-in layout is not the required formatted 16-package fixture'
fi

extract_guard_functions
source "$run_dir/fixed-guard.sh"
source "$run_dir/old-guard.sh"

request=$run_dir/request.json
write_request "$request"

pre_guard=$cases_dir/pre-fix-old-helper/guard.json
run_failure_no_guard 'pre-fix-old-helper' old "$request" "$layout" "$pre_guard" noncanonical-json

post_guard=$cases_dir/post-fix-pretty-layout/guard.json
run_success 'post-fix-pretty-layout' fixed "$request" "$layout" "$post_guard" reset

pretty_request=$cases_dir/canonical-request-reformat/request.json
mkdir -m 0700 -- "$cases_dir/canonical-request-reformat"
python3 - "$request" "$pretty_request" <<'PY'
import json,sys
value=json.load(open(sys.argv[1],encoding="utf-8"))
with open(sys.argv[2],"w",encoding="utf-8",newline="\n") as handle:
    json.dump(value,handle,sort_keys=True,indent=2)
    handle.write("\n")
PY
run_failure_no_guard 'canonical-request-reformat' fixed "$pretty_request" "$layout" \
  "$cases_dir/canonical-request-reformat/guard.json" noncanonical-json

duplicate_request=$cases_dir/duplicate-request/request.json
mkdir -m 0700 -- "$cases_dir/duplicate-request"
printf '%s\n' '{"cache_key":"a","cache_key":"b"}' >"$duplicate_request"
run_failure_no_guard 'duplicate-request-keys' fixed "$duplicate_request" "$layout" \
  "$cases_dir/duplicate-request/guard.json" duplicate-json-key

duplicate_layout=$cases_dir/duplicate-layout/layout.json
mkdir -m 0700 -- "$cases_dir/duplicate-layout"
printf '%s\n' '{"packages":{},"packages":{}}' >"$duplicate_layout"
run_failure_no_guard 'duplicate-layout-keys' fixed "$request" "$duplicate_layout" \
  "$cases_dir/duplicate-layout/guard.json" duplicate-json-key

nan_request=$cases_dir/nan-request/request.json
mkdir -m 0700 -- "$cases_dir/nan-request"
printf '%s\n' '{"k_sha256":NaN}' >"$nan_request"
run_failure_no_guard 'nan-request' fixed "$nan_request" "$layout" \
  "$cases_dir/nan-request/guard.json" non-finite

nan_layout=$cases_dir/nan-layout/layout.json
mkdir -m 0700 -- "$cases_dir/nan-layout"
printf '%s\n' '{"packages":NaN}' >"$nan_layout"
run_failure_no_guard 'nan-layout' fixed "$request" "$nan_layout" \
  "$cases_dir/nan-layout/guard.json" non-finite

unchanged_guard=$cases_dir/complete-unchanged/guard.json
mkdir -m 0700 -- "$cases_dir/complete-unchanged"
write_complete_ledger "$unchanged_guard" "$request" '' canonical
run_success 'complete-unchanged-reuse' fixed "$request" "$layout" "$unchanged_guard" reuse

binding_guard=$cases_dir/complete-binding-mismatch/guard.json
mkdir -m 0700 -- "$cases_dir/complete-binding-mismatch"
write_complete_ledger "$binding_guard" "$request" '' canonical
python3 - "$binding_guard" <<'PY'
import json,sys
path=sys.argv[1]
value=json.load(open(path,encoding="utf-8"))
value["binding"]["cache_namespace"] += "-other"
with open(path,"w",encoding="utf-8",newline="\n") as handle:
    handle.write(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n")
PY
run_success 'complete-binding-mismatch-reset' fixed "$request" "$layout" "$binding_guard" reset

changed_guard=$cases_dir/complete-changed/guard.json
mkdir -m 0700 -- "$cases_dir/complete-changed"
write_complete_ledger "$changed_guard" "$request" domain canonical
run_success 'complete-changed-reverse-closure' fixed "$request" "$layout" "$changed_guard" \
  "$(expected_domain_closure)"

pending_guard=$cases_dir/pending-ledger/guard.json
mkdir -m 0700 -- "$cases_dir/pending-ledger"
write_pending_ledger "$pending_guard" "$request"
run_success 'pending-ledger-reset' fixed "$request" "$layout" "$pending_guard" reset

malformed_guard=$cases_dir/malformed-ledger/guard.json
mkdir -m 0700 -- "$cases_dir/malformed-ledger"
printf '%s\n' '{malformed' >"$malformed_guard"
run_success 'malformed-ledger-reset' fixed "$request" "$layout" "$malformed_guard" reset

noncanonical_guard=$cases_dir/noncanonical-ledger/guard.json
mkdir -m 0700 -- "$cases_dir/noncanonical-ledger"
write_complete_ledger "$noncanonical_guard" "$request" '' pretty
run_success 'noncanonical-ledger-reset' fixed "$request" "$layout" "$noncanonical_guard" reset

symlink_guard=$cases_dir/symlink-ledger/guard.json
symlink_target=$cases_dir/symlink-ledger/target
mkdir -m 0700 -- "$cases_dir/symlink-ledger"
printf '%s\n' symlink-target >"$symlink_target"
ln -s -- "$symlink_target" "$symlink_guard"
run_symlink_failure 'symlink-ledger-rejected' "$request" "$layout" "$symlink_guard" "$symlink_target"

raw_layout_hash_case
