#!/usr/bin/env bash
set -euo pipefail

# Keep the real workspace and temporary fixture on the same installed toolchain.
# Rustup proxies must fail locally if it is absent, rather than installing it.
# https://rust-lang.github.io/rustup/environment-variables.html
export RUSTUP_TOOLCHAIN=1.97.1 RUSTUP_AUTO_INSTALL=0

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
helper=$repo_root/scripts/ops/lib/release-build-layout.sh

if [ "$#" -ne 0 ]; then
  printf '%s\n' 'build-layout-graph-self-test: this focused test accepts no arguments' >&2
  exit 2
fi

run_dir=$(mktemp -d /tmp/wp10-cargo-graph-fix-XXXXXX)
report=$run_dir/report.md
cases_dir=$run_dir/cases
fixture=$run_dir/fixture
real_metadata=$run_dir/real-metadata.json
fixture_metadata=$run_dir/fixture-metadata.json
fixture_layout=$run_dir/fixture-layout.json
real_metadata_stderr=$run_dir/real-metadata.stderr
fixture_metadata_stderr=$run_dir/fixture-metadata.stderr
fixture_lockfile_stderr=$run_dir/fixture-lockfile.stderr

mkdir -m 0700 -- "$cases_dir" "$fixture"

pass_count=0
fail_count=0
result_lines=()
cargo_path=not-found
cargo_version=not-run
real_metadata_sha256=not-created
fixture_metadata_sha256=not-created
fixture_lock_sha256=not-created
real_package_count=not-measured
real_member_count=not-measured
fixture_package_count=not-measured
fixture_member_count=not-measured
failure_reason=

write_report() {
  local exit_status=$1
  local entry label expected actual status detail
  {
    printf '%s\n' '# WP10 C7 Cargo metadata graph self-test'
    printf '%s\n' ''
    printf '%s\n' "- run_dir: $run_dir"
    printf '%s\n' "- script_exit_status: $exit_status"
    printf '%s\n' "- cargo_path: $cargo_path"
    printf '%s\n' "- cargo_version: $cargo_version"
    printf '%s\n' "- real_metadata_sha256: $real_metadata_sha256"
    printf '%s\n' "- real_workspace_packages: $real_package_count"
    printf '%s\n' "- real_workspace_members: $real_member_count"
    printf '%s\n' "- fixture_metadata_sha256: $fixture_metadata_sha256"
    printf '%s\n' "- fixture_lock_sha256: $fixture_lock_sha256"
    printf '%s\n' "- fixture_workspace_packages: $fixture_package_count"
    printf '%s\n' "- fixture_workspace_members: $fixture_member_count"
    printf '%s\n' "- checks_passed: $pass_count"
    printf '%s\n' "- checks_failed: $fail_count"
    if [ -n "$failure_reason" ]; then
      printf '%s\n' "- failure_reason: $failure_reason"
    fi
    printf '%s\n' ''
    printf '%s\n' 'The Cargo metadata commands used for both workspaces were exactly:'
    printf '%s\n' ''
    printf '%s\n' '    cargo metadata --locked --offline --no-deps --format-version 1'
    printf '%s\n' ''
    printf '%s\n' '## Checks'
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
  write_report "$exit_status"
  if [ "$exit_status" -eq 0 ]; then
    printf 'C7_BUILD_LAYOUT_GRAPH_SELF_TEST: PASS\nreport: %s\nchecks: %s\n' \
      "$report" "$pass_count"
  else
    printf 'C7_BUILD_LAYOUT_GRAPH_SELF_TEST: FAIL\nreport: %s\nchecks_passed: %s\nchecks_failed: %s\n' \
      "$report" "$pass_count" "$fail_count" >&2
  fi
  exit "$exit_status"
}
trap on_exit EXIT

fail_test() {
  failure_reason=$1
  printf 'build-layout-graph-self-test: %s\nreport: %s\n' "$failure_reason" "$report" >&2
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

run_metadata() {
  local label=$1 workdir=$2 output=$3 error=$4 status
  if (cd -- "$workdir" && cargo metadata --locked --offline --no-deps --format-version 1 >"$output" 2>"$error"); then
    status=0
  else
    status=$?
  fi
  if [ "$status" -eq 0 ]; then
    record_result "$label" 'exit=0' "exit=$status" PASS \
      'cargo metadata --locked --offline --no-deps --format-version 1'
  else
    record_result "$label" 'exit=0' "exit=$status" FAIL \
      'stderr retained in run directory'
    fail_test "$label failed; Cargo stderr is retained in $error"
  fi
}

run_lockfile_generation() {
  local status
  if (cd -- "$fixture" && cargo generate-lockfile --offline >"$run_dir/fixture-lockfile.stdout" 2>"$fixture_lockfile_stderr"); then
    status=0
  else
    status=$?
  fi
  if [ "$status" -eq 0 ] && [ -f "$fixture/Cargo.lock" ] && [ ! -L "$fixture/Cargo.lock" ]; then
    record_result 'fixture-lockfile-generation' 'exit=0 and Cargo.lock exists' \
      "exit=$status and Cargo.lock exists" PASS \
      'cargo generate-lockfile --offline'
  else
    record_result 'fixture-lockfile-generation' 'exit=0 and Cargo.lock exists' \
      "exit=$status or Cargo.lock missing" FAIL \
      'stderr retained in run directory'
    fail_test 'fixture lockfile generation failed; Cargo stderr is retained in run directory'
  fi
}

check_metadata_shape() {
  local label=$1 metadata=$2 expected=$3 counts
  if counts=$(python3 - "$metadata" 2>"$run_dir/$label-shape.stderr" <<'PY'
import json,sys
value=json.load(open(sys.argv[1],encoding="utf-8"))
if not isinstance(value,dict) or not isinstance(value.get("packages"),list) or not isinstance(value.get("workspace_members"),list):
    raise SystemExit("metadata-shape-invalid")
print("%d %d" % (len(value["packages"]),len(value["workspace_members"])))
PY
  ); then
    :
  else
    record_result "$label" "$expected" 'shape parse failed' FAIL \
      'shape stderr retained in run directory'
    fail_test "$label could not parse genuine Cargo output"
  fi
  if [ "$counts" = "$expected" ]; then
    record_result "$label" "$expected" "$counts" PASS 'JSON shape/count check'
  else
    record_result "$label" "$expected" "$counts" FAIL 'JSON shape/count check'
    fail_test "$label count mismatch"
  fi
}

run_helper_case() {
  local label=$1 metadata=$2 layout=$3 build_root=$4 expected_error=$5
  local case_dir=$cases_dir/$label
  local stdout=$case_dir/stdout
  local stderr=$case_dir/stderr
  local status actual_error actual_stdout expected actual
  mkdir -p -m 0700 -- "$case_dir"
  if rbl_validate_cargo_metadata_graph "$metadata" "$layout" "$build_root" >"$stdout" 2>"$stderr"; then
    status=0
  else
    status=$?
  fi
  actual_error=$(<"$stderr")
  actual_stdout=$(<"$stdout")
  if [ "$expected_error" = success ]; then
    expected='exit=0 and no output'
    actual="exit=$status"
    if [ "$status" -eq 0 ] && [ -z "$actual_error" ] && [ -z "$actual_stdout" ]; then
      record_result "$label" "$expected" "$actual" PASS \
        'shared rbl_validate_cargo_metadata_graph helper'
      return
    fi
  else
    expected="exit=1 and $expected_error"
    actual="exit=$status and ${actual_error:-<no-stderr>}"
    if [ "$status" -eq 1 ] && [ "$actual_error" = "$expected_error" ] && [ -z "$actual_stdout" ]; then
      record_result "$label" "$expected" "$actual" PASS \
        'shared rbl_validate_cargo_metadata_graph helper'
      return
    fi
  fi
  record_result "$label" "$expected" "$actual" FAIL \
    'stdout/stderr retained in run directory'
  fail_test "$label produced an unexpected validator result"
}

make_fixture() {
  python3 - "$fixture" <<'PY'
from pathlib import Path
import sys

root=Path(sys.argv[1])
members=[
    "root",
    "normal-lib",
    "build-lib",
    "optional-lib",
    "target-lib",
    "dev-lib",
    "dev-cycle-a",
    "dev-cycle-b",
]
base_manifests={}
for name in members[1:]:
    base_manifests[f"{name}/Cargo.toml"]=f"""[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
"""
base_manifests["root/Cargo.toml"]="""[package]
name = "fixture-root"
version = "0.1.0"
edition = "2024"

[dependencies]
normal-lib = { path = "../normal-lib" }
optional-lib = { path = "../optional-lib", optional = true }

[target.'cfg(target_os = "never")'.dependencies]
target-lib = { path = "../target-lib" }

[build-dependencies]
build-lib = { path = "../build-lib" }

[dev-dependencies]
dev-lib = { path = "../dev-lib" }
"""
base_manifests["dev-cycle-a/Cargo.toml"] += """
[dev-dependencies]
dev-cycle-b = { path = "../dev-cycle-b" }
"""
base_manifests["dev-cycle-b/Cargo.toml"] += """
[dev-dependencies]
dev-cycle-a = { path = "../dev-cycle-a" }
"""
base_manifests["Cargo.toml"]="""[workspace]
resolver = "3"
members = [
    "root",
    "normal-lib",
    "build-lib",
    "optional-lib",
    "target-lib",
    "dev-lib",
    "dev-cycle-a",
    "dev-cycle-b",
]
"""
for relative,content in base_manifests.items():
    path=root/relative
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_text(content,encoding="utf-8")
for name in members[1:]:
    path=root/name/"src/lib.rs"
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_text(f"pub const FIXTURE_PACKAGE: &str = \"{name}\";\n",encoding="utf-8")
path=root/"root/src/lib.rs"
path.parent.mkdir(parents=True,exist_ok=True)
path.write_text('pub const FIXTURE_PACKAGE: &str = "fixture-root";\n',encoding="utf-8")
PY
}

write_fixture_layout() {
  python3 - "$fixture_layout" <<'PY'
import json,sys

packages={}
paths={
    "fixture-root":"root",
    "normal-lib":"normal-lib",
    "build-lib":"build-lib",
    "optional-lib":"optional-lib",
    "target-lib":"target-lib",
    "dev-lib":"dev-lib",
    "dev-cycle-a":"dev-cycle-a",
    "dev-cycle-b":"dev-cycle-b",
}
for name,path in paths.items():
    packages[name]={"path":path,"local_dependencies":[]}
packages["fixture-root"]["local_dependencies"]=[
    "build-lib","normal-lib","optional-lib","target-lib"
]
with open(sys.argv[1],"w",encoding="utf-8",newline="\n") as handle:
    json.dump({"packages":packages},handle,sort_keys=True,separators=(",",":"))
    handle.write("\n")
PY
}

check_fixture_declarations() {
  local declarations
  if declarations=$(python3 - "$fixture_metadata" 2>"$run_dir/fixture-declarations.stderr" <<'PY'
import json,sys
value=json.load(open(sys.argv[1],encoding="utf-8"))
packages={package["name"]:package for package in value["packages"]}
root={item["name"]:item for item in packages["fixture-root"]["dependencies"]}
if root["normal-lib"].get("kind") is not None:
    raise SystemExit("normal-kind-invalid")
if root["build-lib"].get("kind") != "build":
    raise SystemExit("build-kind-invalid")
if root["optional-lib"].get("kind") is not None or root["optional-lib"].get("optional") is not True:
    raise SystemExit("optional-declaration-invalid")
if root["target-lib"].get("kind") is not None or not root["target-lib"].get("target"):
    raise SystemExit("target-declaration-invalid")
if root["dev-lib"].get("kind") != "dev":
    raise SystemExit("dev-declaration-invalid")
cycle_a={item["name"]:item for item in packages["dev-cycle-a"]["dependencies"]}
cycle_b={item["name"]:item for item in packages["dev-cycle-b"]["dependencies"]}
if cycle_a["dev-cycle-b"].get("kind") != "dev" or cycle_b["dev-cycle-a"].get("kind") != "dev":
    raise SystemExit("dev-cycle-invalid")
print("normal=null build=build optional=normal+optional target=normal+target dev=dev cycle=dev<->dev")
PY
  ); then
    record_result 'fixture-declarations' \
      'normal/build/optional/target accepted; dev and dev-cycle ignored' \
      "$declarations" PASS 'genuine fixture Cargo metadata'
  else
    record_result 'fixture-declarations' \
      'normal/build/optional/target accepted; dev and dev-cycle ignored' \
      'declaration inspection failed' FAIL \
      'stderr retained in run directory'
    fail_test 'fixture declarations were not represented by genuine Cargo metadata'
  fi
}

make_layout_case() {
  local label=$1
  local mutation=$2
  local path=$cases_dir/$label/layout.json
  mkdir -m 0700 -- "$cases_dir/$label"
  cp -- "$fixture_layout" "$path"
  python3 - "$path" "$mutation" <<'PY'
import json,sys
path,mutation=sys.argv[1:3]
with open(path,encoding="utf-8") as handle:
    value=json.load(handle)
packages=value["packages"]
if mutation == "missing-normal":
    packages["fixture-root"]["local_dependencies"].remove("normal-lib")
elif mutation == "missing-build":
    packages["fixture-root"]["local_dependencies"].remove("build-lib")
elif mutation == "inserted-dev":
    packages["fixture-root"]["local_dependencies"].append("dev-lib")
elif mutation == "undeclared":
    packages["fixture-root"]["local_dependencies"].append("undeclared-local")
elif mutation == "member-set":
    del packages["normal-lib"]
elif mutation == "member-path":
    packages["normal-lib"]["path"]="wrong-path"
else:
    raise SystemExit("unknown-layout-mutation")
if "fixture-root" in packages:
    packages["fixture-root"]["local_dependencies"].sort()
with open(path,"w",encoding="utf-8",newline="\n") as handle:
    json.dump(value,handle,sort_keys=True,separators=(",",":"))
    handle.write("\n")
PY
  printf '%s\n' "$path"
}

make_metadata_case() {
  local label=$1 mutation=$2
  local path=$cases_dir/$label/metadata.json
  mkdir -m 0700 -- "$cases_dir/$label"
  cp -- "$real_metadata" "$path"
  python3 - "$path" "$mutation" <<'PY'
import json,sys
path,mutation=sys.argv[1:3]
with open(path,encoding="utf-8") as handle:
    value=json.load(handle)
changed=False
for package in value["packages"]:
    for dependency in package.get("dependencies",[]):
        if "kind" not in dependency:
            continue
        if mutation == "missing":
            del dependency["kind"]
        elif mutation == "unknown":
            dependency["kind"]="runtime"
        elif mutation == "invalid":
            dependency["kind"]=[]
        else:
            raise SystemExit("unknown-kind-mutation")
        changed=True
        break
    if changed:
        break
if not changed:
    raise SystemExit("no-dependency-kind-found")
with open(path,"w",encoding="utf-8",newline="\n") as handle:
    json.dump(value,handle,sort_keys=True,separators=(",",":"))
    handle.write("\n")
PY
  printf '%s\n' "$path"
}

if ! cargo_path=$(command -v cargo 2>/dev/null); then
  fail_test 'required installed Cargo 1.97.1 is unavailable; no rustup or network fallback is permitted'
fi
if ! cargo_version=$(cargo --version 2>"$run_dir/cargo-version.stderr"); then
  fail_test 'installed Cargo could not report its version; no rustup or network fallback is permitted'
fi
case "$cargo_version" in
  'cargo 1.97.1 ('*)
    record_result 'cargo-version' 'cargo 1.97.1' "$cargo_version" PASS \
      'installed Cargo; no installation/network fallback'
    ;;
  *)
    record_result 'cargo-version' 'cargo 1.97.1' "$cargo_version" FAIL \
      'installed Cargo version check'
    fail_test 'required installed Cargo 1.97.1 is unavailable; found a different Cargo'
    ;;
esac

unset RELEASE_BUILD_LAYOUT_EXECUTION
# shellcheck source=../ops/lib/release-build-layout.sh
source "$helper"

run_metadata 'real-workspace-metadata' "$repo_root" "$real_metadata" "$real_metadata_stderr"
if real_metadata_sha256=$(sha256sum -- "$real_metadata" | awk '{print $1}'); then
  :
else
  fail_test 'could not hash genuine root Cargo metadata'
fi
if real_counts=$(python3 - "$real_metadata" <<'PY'
import json,sys
value=json.load(open(sys.argv[1],encoding="utf-8"))
print("%d %d" % (len(value["packages"]),len(value["workspace_members"])))
PY
); then
  read -r real_package_count real_member_count <<<"$real_counts"
else
  fail_test 'could not count genuine root Cargo metadata'
fi
check_metadata_shape 'real-workspace-shape' "$real_metadata" '16 16'
run_helper_case 'real-production-graph' "$real_metadata" \
  "$repo_root/deploy/build/release-build-layout.json" "$repo_root" success

make_fixture
write_fixture_layout
run_lockfile_generation
fixture_lock_sha256=$(sha256sum -- "$fixture/Cargo.lock" | awk '{print $1}')
run_metadata 'fixture-workspace-metadata' "$fixture" "$fixture_metadata" "$fixture_metadata_stderr"
fixture_metadata_sha256=$(sha256sum -- "$fixture_metadata" | awk '{print $1}')
if fixture_counts=$(python3 - "$fixture_metadata" <<'PY'
import json,sys
value=json.load(open(sys.argv[1],encoding="utf-8"))
print("%d %d" % (len(value["packages"]),len(value["workspace_members"])))
PY
); then
  read -r fixture_package_count fixture_member_count <<<"$fixture_counts"
else
  fail_test 'could not count genuine fixture Cargo metadata'
fi
check_metadata_shape 'fixture-workspace-shape' "$fixture_metadata" '8 8'
check_fixture_declarations
run_helper_case 'fixture-production-graph' "$fixture_metadata" "$fixture_layout" "$fixture" success

for mutation in missing-normal missing-build inserted-dev undeclared; do
  layout_case=$(make_layout_case "layout-$mutation" "$mutation")
  run_helper_case "layout-$mutation" "$fixture_metadata" "$layout_case" "$fixture" \
    cargo-metadata-local-graph-invalid
done

member_set_case=$(make_layout_case 'layout-member-set' member-set)
run_helper_case 'member-set-mismatch' "$fixture_metadata" "$member_set_case" "$fixture" \
  cargo-metadata-member-set-invalid
member_path_case=$(make_layout_case 'layout-member-path' member-path)
run_helper_case 'member-path-mismatch' "$fixture_metadata" "$member_path_case" "$fixture" \
  cargo-metadata-member-path-invalid

for mutation in missing unknown invalid; do
  metadata_case=$(make_metadata_case "metadata-kind-$mutation" "$mutation")
  run_helper_case "metadata-kind-$mutation" "$metadata_case" \
    "$repo_root/deploy/build/release-build-layout.json" "$repo_root" \
    cargo-metadata-dependency-kind-invalid
done
