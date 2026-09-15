#!/usr/bin/env bash
# Fixture-scoped helpers for the frozen WP-3 layout prototype.  This file is
# deliberately small in surface area: it is not a general cache framework.
set -euo pipefail

helper_die() { printf '%s\n' "build-layout-helper: $*" >&2; return 1; }

sha256_file() {
  local path=${1:?path}
  [ -f "$path" ] && [ ! -L "$path" ] || { helper_die "not a regular file: $path"; return 1; }
  sha256sum -- "$path" | awk '{print $1}'
}

# Hashes record type, mode, relative name, bytes, and empty directories. A
# source/artifact tree may not contain symlinks or special files.
tree_hash() {
  local root=${1:?root}
  [ -d "$root" ] && [ ! -L "$root" ] || { helper_die "not a directory: $root"; return 1; }
  python3 - "$root" <<'PY'
import hashlib, os, stat, sys
root = os.path.abspath(sys.argv[1])
records = []
for current, dirs, files in os.walk(root, topdown=True, followlinks=False):
    dirs.sort(); files.sort()
    kept = []
    for name in dirs:
        path = os.path.join(current, name); info = os.lstat(path)
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
            raise SystemExit("invalid-directory-entry")
        kept.append(name)
        records.append(f"d\t{stat.S_IMODE(info.st_mode):04o}\t{os.path.relpath(path, root)}\t-\n".encode())
    dirs[:] = kept
    for name in files:
        path = os.path.join(current, name); info = os.lstat(path)
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
            raise SystemExit("invalid-file-entry")
        digest = hashlib.sha256()
        with open(path, "rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        records.append(f"f\t{stat.S_IMODE(info.st_mode):04o}\t{os.path.relpath(path, root)}\t{digest.hexdigest()}\n".encode())
records.sort()
print(hashlib.sha256(b"".join(records)).hexdigest())
PY
}

source_input_hash() {
  local root=${1:?source root}
  # The source snapshot identity deliberately includes the Dockerfiles,
  # Compose recipe, helper, and layout contract.  Compile/runtime identities
  # remain separate, but resume may not silently accept changed recipes.
  tree_hash "$root"
}

identity_validate() {
  local identity=${1:?identity JSON} platform=${2:?platform}
  python3 - "$identity" "$platform" <<'PY'
import json, os, re, stat, sys
path, expected_platform = sys.argv[1:]
info = os.lstat(path)
if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
    raise SystemExit("identity-file-invalid")
def no_duplicates(pairs):
    value = {}
    for key, item in pairs:
        if key in value: raise ValueError("duplicate-json-key")
        value[key] = item
    return value
try:
    with open(path, encoding="utf-8", newline="") as handle:
        value = json.load(handle, object_pairs_hook=no_duplicates)
except (OSError, ValueError, json.JSONDecodeError):
    raise SystemExit("identity-json-invalid")
required = {"cargo_version", "format", "native_packages", "rustc_vv", "target_platform"}
if not isinstance(value, dict) or set(value) != required:
    raise SystemExit("identity-keys-invalid")
if value["format"] != "lagrange-build-layout-native-v1" or value["target_platform"] != expected_platform:
    raise SystemExit("identity-contract-mismatch")
if not isinstance(value["rustc_vv"], str) or not value["rustc_vv"].endswith("\n"):
    raise SystemExit("identity-rustc-invalid")
hosts = re.findall(r"(?m)^host: ([A-Za-z0-9_.-]+)$", value["rustc_vv"])
if len(hosts) != 1:
    raise SystemExit("identity-host-invalid")
# Preserve the complete one-line `cargo -V` identity, including release hash/date.
# Stable, beta/nightly and SemVer build metadata are accepted, never free prose.
version = r"cargo (?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
version += r"(?: \([0-9a-f]{7,40} [0-9]{4}-[0-9]{2}-[0-9]{2}\))?"
if not isinstance(value["cargo_version"], str) or not re.fullmatch(version, value["cargo_version"]):
    raise SystemExit("identity-cargo-invalid")
if value["cargo_version"].endswith(")"):
    import datetime
    try: datetime.date.fromisoformat(value["cargo_version"][-11:-1])
    except ValueError: raise SystemExit("identity-cargo-date-invalid")
packages = value["native_packages"]
if (not isinstance(packages, list) or not packages or
        not all(isinstance(item, str) and item and "\n" not in item for item in packages) or
        packages != sorted(set(packages))):
    raise SystemExit("identity-native-packages-invalid")
print(hosts[0])
PY
}

compile_input_hash() {
  local root=${1:?source root}
  [ -d "$root" ] && [ ! -L "$root" ] || { helper_die "not a source directory: $root"; return 1; }
  python3 - "$root" <<'PY'
import hashlib, os, stat, sys
root = os.path.abspath(sys.argv[1])
paths = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "fixture-lib", "fixture-app"]
records = []
def add(path, rel):
    info = os.lstat(path)
    if stat.S_ISLNK(info.st_mode):
        raise SystemExit("compile-input-symlink")
    if stat.S_ISREG(info.st_mode):
        digest = hashlib.sha256()
        with open(path, "rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        records.append(f"f\t{stat.S_IMODE(info.st_mode):04o}\t{rel}\t{digest.hexdigest()}\n".encode())
    elif stat.S_ISDIR(info.st_mode):
        records.append(f"d\t{stat.S_IMODE(info.st_mode):04o}\t{rel}\t-\n".encode())
        for name in sorted(os.listdir(path)):
            add(os.path.join(path, name), f"{rel}/{name}")
    else:
        raise SystemExit("compile-input-special-file")
for item in paths:
    path = os.path.join(root, item)
    if not os.path.exists(path):
        raise SystemExit("compile-input-missing")
    add(path, item)
records.sort()
print(hashlib.sha256(b"".join(records)).hexdigest())
PY
}

compatibility_key() {
  local root=${1:?source root} identity=${2:?identity file} recipe=${3:?recipe sha} guard=${4:?guard sha}
  local platform=${5:?platform} config=${6:?effective config}
  python3 - "$root" "$identity" "$recipe" "$guard" "$platform" "$config" <<'PY'
import hashlib, json, os, stat, sys
root, identity, recipe, guard, platform, config = sys.argv[1:]
if not os.path.isfile(identity) or os.path.islink(identity):
    raise SystemExit("identity-missing")
members = ["Cargo.toml", "fixture-lib/Cargo.toml", "fixture-app/Cargo.toml"]
raw = {}
for rel in ["Cargo.lock", "rust-toolchain.toml", *members]:
    path = os.path.join(root, rel)
    info = os.lstat(path)
    if not stat.S_ISREG(info.st_mode) or stat.S_ISLNK(info.st_mode):
        raise SystemExit("compatibility-input-invalid")
    raw[rel] = hashlib.sha256(open(path, "rb").read()).hexdigest()
native = hashlib.sha256(open(identity, "rb").read()).hexdigest()
value = {
    "schema": "lagrange-build-layout-k-v2",
    "builder_native_inventory_sha256": native,
    "builder_recipe_sha256": recipe,
    "cargo_config": config,
    "compiler_env": {"CARGO_BUILD_JOBS": "2", "CARGO_TARGET_DIR": "/cargo-target", "RUSTFLAGS": "<unset>"},
    "guard_sha256": guard,
    "manifest_and_lock_raw_sha256": raw,
    "paths": {"target": "/cargo-target", "workdir": "/build"},
    "platform": platform,
    "profile": "release",
}
print(hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest())
PY
}

guard_inputs() {
  local root=${1:?source root} expected_compile=${2:?compile hash} expected_lib=${3:?lib hash} expected_app=${4:?app hash}
  local required
  for required in Cargo.toml Cargo.lock rust-toolchain.toml fixture-lib/Cargo.toml fixture-lib/src/lib.rs fixture-app/Cargo.toml fixture-app/build.rs fixture-app/data/embedded.txt fixture-app/src/bin/cache-bin-a.rs fixture-app/src/bin/cache-bin-b.rs; do
    [ -f "$root/$required" ] && [ ! -L "$root/$required" ] || { helper_die "declared fixture input is missing: $required"; return 1; }
  done
  [ "$(compile_input_hash "$root")" = "$expected_compile" ] || { helper_die 'compile input hash mismatch'; return 1; }
  [ "$(tree_hash "$root/fixture-lib")" = "$expected_lib" ] || { helper_die 'library input hash mismatch'; return 1; }
  [ "$(tree_hash "$root/fixture-app")" = "$expected_app" ] || { helper_die 'application input hash mismatch'; return 1; }
  return 0
}

artifact_write() {
  local dir=${1:?dir} commit=${2:?commit} package=${3:?package} bin=${4:?bin} platform=${5:?platform}
  local host=${6:?host} profile=${7:?profile} features=${8:?features} env_json=${9:?env json}
  local cache_key=${10:?cache key} input_hash=${11:?input hash} recipe_hash=${12:?recipe hash}
  local binary=${13:?binary} success=${14:?success}
  [ -d "$dir" ] && [ ! -L "$dir" ] || { helper_die 'artifact directory is invalid'; return 1; }
  mkdir -p -- "$dir/bin"
  [ -f "$binary" ] && [ ! -L "$binary" ] || { helper_die 'binary is missing'; return 1; }
  [ -f "$dir/runtime.txt" ] && [ ! -L "$dir/runtime.txt" ] || { helper_die 'runtime payload is missing'; return 1; }
  [ -f "$dir/cargo.jsonl" ] && [ ! -L "$dir/cargo.jsonl" ] || { helper_die 'Cargo JSON evidence is missing'; return 1; }
  [ -f "$dir/cargo-summary.json" ] && [ ! -L "$dir/cargo-summary.json" ] || { helper_die 'Cargo summary is missing'; return 1; }
  [ -f "$dir/timing.json" ] && [ ! -L "$dir/timing.json" ] || { helper_die 'timing evidence is missing'; return 1; }
  local destination="$dir/bin/$bin"
  if [ "$(readlink -f -- "$binary")" != "$(readlink -f -- "$destination" 2>/dev/null || printf '%s' "$destination")" ]; then
    cp -- "$binary" "$destination"
  fi
  chmod 0755 -- "$destination"
  PROBE_ARTIFACT_DIR="$dir" PROBE_SOURCE_COMMIT="$commit" PROBE_PACKAGE="$package" PROBE_BIN="$bin" \
  PROBE_PLATFORM="$platform" PROBE_HOST="$host" PROBE_PROFILE="$profile" PROBE_FEATURES="$features" \
  PROBE_ENV_JSON="$env_json" PROBE_CACHE_KEY="$cache_key" PROBE_INPUT_HASH="$input_hash" \
  PROBE_RECIPE_HASH="$recipe_hash" PROBE_SUCCESS="$success" python3 - <<'PY'
import hashlib, json, os, re
directory = os.environ["PROBE_ARTIFACT_DIR"]
binary = os.path.join(directory, "bin", os.environ["PROBE_BIN"])
commit = os.environ["PROBE_SOURCE_COMMIT"]
if not re.fullmatch(r"[0-9a-f]{40}", commit): raise SystemExit("invalid-source-commit")
try:
    features = json.loads(os.environ["PROBE_FEATURES"])
    env = json.loads(os.environ["PROBE_ENV_JSON"])
except (ValueError, json.JSONDecodeError):
    raise SystemExit("invalid-artifact-json-argument")
if not isinstance(features, list) or not all(isinstance(item, str) for item in features) or not isinstance(env, dict):
    raise SystemExit("invalid-artifact-argument-types")
with open(binary, "rb") as handle: binary_hash = hashlib.sha256(handle.read()).hexdigest()
record = {
  "binary_mode": "0755", "binary_sha256": binary_hash, "bin": os.environ["PROBE_BIN"],
  "cache_key": os.environ["PROBE_CACHE_KEY"], "cargo_success": os.environ["PROBE_SUCCESS"] == "true",
  "compile_env": env, "features": features, "format": "lagrange-rust-artifact-v1",
  "host_triple": os.environ["PROBE_HOST"], "input_sha256": os.environ["PROBE_INPUT_HASH"],
  "package": os.environ["PROBE_PACKAGE"], "platform": os.environ["PROBE_PLATFORM"],
  "profile": os.environ["PROBE_PROFILE"], "recipe_sha256": os.environ["PROBE_RECIPE_HASH"],
  "source_commit": commit,
}
if not record["cargo_success"]: raise SystemExit("artifact-cargo-success-false")
path = os.path.join(directory, "artifact.json")
with open(path, "w", encoding="utf-8", newline="\n") as handle:
    handle.write(json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n")
with open(path, "rb") as handle: digest = hashlib.sha256(handle.read()).hexdigest()
with open(os.path.join(directory, "COMPLETE"), "w", encoding="ascii", newline="\n") as handle:
    handle.write(digest + "\n")
PY
}

artifact_validate() {
  local dir=${1:?dir} commit=${2:?commit} package=${3:?package} bin=${4:?bin} platform=${5:?platform}
  local host=${6:?host} profile=${7:?profile} features=${8:?features} env_json=${9:?env json}
  local cache_key=${10:?cache key} input_hash=${11:?input hash} recipe_hash=${12:?recipe hash}
  PROBE_ARTIFACT_DIR="$dir" PROBE_EXPECTED_COMMIT="$commit" PROBE_EXPECTED_PACKAGE="$package" \
  PROBE_EXPECTED_BIN="$bin" PROBE_EXPECTED_PLATFORM="$platform" PROBE_EXPECTED_HOST="$host" \
  PROBE_EXPECTED_PROFILE="$profile" PROBE_EXPECTED_FEATURES="$features" PROBE_EXPECTED_ENV="$env_json" \
  PROBE_EXPECTED_CACHE="$cache_key" PROBE_EXPECTED_INPUT="$input_hash" PROBE_EXPECTED_RECIPE="$recipe_hash" python3 - <<'PY'
import hashlib, json, os, re, stat
directory = os.path.abspath(os.environ["PROBE_ARTIFACT_DIR"])
expected_bin = os.environ["PROBE_EXPECTED_BIN"]
if not re.fullmatch(r"[A-Za-z0-9._-]+", expected_bin): raise SystemExit("artifact-bin-path-invalid")
def regular(path):
    try: info = os.lstat(path)
    except OSError: return False
    return stat.S_ISREG(info.st_mode) and not stat.S_ISLNK(info.st_mode)
def directory_ok(path):
    try: info = os.lstat(path)
    except OSError: return False
    return stat.S_ISDIR(info.st_mode) and not stat.S_ISLNK(info.st_mode)
if not directory_ok(directory): raise SystemExit("artifact-directory-invalid")
allowed_root = {"artifact.json", "COMPLETE", "bin", "cargo.jsonl", "cargo-summary.json", "runtime.txt", "timing.json"}
if set(os.listdir(directory)) - allowed_root: raise SystemExit("artifact-unexpected-entry")
for name in ("artifact.json", "COMPLETE", "cargo.jsonl", "cargo-summary.json", "runtime.txt", "timing.json"):
    if not regular(os.path.join(directory, name)): raise SystemExit("artifact-missing-or-symlink-" + name)
bin_dir = os.path.join(directory, "bin")
if not directory_ok(bin_dir) or set(os.listdir(bin_dir)) != {expected_bin}: raise SystemExit("artifact-bin-layout-invalid")
binary_path = os.path.join(bin_dir, expected_bin)
if not regular(binary_path): raise SystemExit("artifact-binary-invalid")
def no_duplicates(pairs):
    result = {}
    for key, value in pairs:
        if key in result: raise ValueError("duplicate-json-key")
        result[key] = value
    return result
def reject_constant(value): raise ValueError("non-finite-json-number")
try:
    with open(os.path.join(directory, "artifact.json"), "r", encoding="utf-8", newline="") as handle:
        record = json.load(handle, object_pairs_hook=no_duplicates, parse_constant=reject_constant)
    expected_features = json.loads(os.environ["PROBE_EXPECTED_FEATURES"], object_pairs_hook=no_duplicates, parse_constant=reject_constant)
    expected_env = json.loads(os.environ["PROBE_EXPECTED_ENV"], object_pairs_hook=no_duplicates, parse_constant=reject_constant)
except (OSError, ValueError, json.JSONDecodeError):
    raise SystemExit("artifact-json-invalid")
required = {"binary_mode", "binary_sha256", "bin", "cache_key", "cargo_success", "compile_env", "features", "format", "host_triple", "input_sha256", "package", "platform", "profile", "recipe_sha256", "source_commit"}
if not isinstance(record, dict) or set(record) != required: raise SystemExit("artifact-json-keys-invalid")
expected = {
 "format": "lagrange-rust-artifact-v1", "source_commit": os.environ["PROBE_EXPECTED_COMMIT"],
 "package": os.environ["PROBE_EXPECTED_PACKAGE"], "bin": expected_bin,
 "platform": os.environ["PROBE_EXPECTED_PLATFORM"], "host_triple": os.environ["PROBE_EXPECTED_HOST"],
 "profile": os.environ["PROBE_EXPECTED_PROFILE"], "features": expected_features,
 "compile_env": expected_env, "cache_key": os.environ["PROBE_EXPECTED_CACHE"],
 "input_sha256": os.environ["PROBE_EXPECTED_INPUT"], "recipe_sha256": os.environ["PROBE_EXPECTED_RECIPE"],
 "cargo_success": True, "binary_mode": "0755",
}
for key, value in expected.items():
    if record.get(key) != value: raise SystemExit("artifact-field-mismatch-" + key)
if not isinstance(record["source_commit"], str) or not re.fullmatch(r"[0-9a-f]{40}", record["source_commit"]): raise SystemExit("artifact-commit-invalid")
if not isinstance(record["binary_sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", record["binary_sha256"]): raise SystemExit("artifact-binary-hash-invalid")
if not isinstance(record["features"], list) or not all(isinstance(item, str) for item in record["features"]): raise SystemExit("artifact-features-invalid")
if not isinstance(record["compile_env"], dict) or not all(isinstance(k, str) and isinstance(v, str) for k,v in record["compile_env"].items()): raise SystemExit("artifact-env-invalid")
with open(binary_path, "rb") as handle: binary_hash = hashlib.sha256(handle.read()).hexdigest()
if binary_hash != record["binary_sha256"]: raise SystemExit("artifact-binary-hash-mismatch")
if stat.S_IMODE(os.stat(binary_path).st_mode) != 0o755: raise SystemExit("artifact-binary-mode-mismatch")
with open(os.path.join(directory, "artifact.json"), "rb") as handle: record_hash = hashlib.sha256(handle.read()).hexdigest()
with open(os.path.join(directory, "COMPLETE"), "rb") as handle: complete = handle.read()
if complete != (record_hash + "\n").encode("ascii"): raise SystemExit("artifact-complete-mismatch")
try:
    with open(os.path.join(directory, "cargo-summary.json"), encoding="utf-8") as handle: cargo = json.load(handle, object_pairs_hook=no_duplicates, parse_constant=reject_constant)
    with open(os.path.join(directory, "timing.json"), encoding="utf-8") as handle: timing = json.load(handle, object_pairs_hook=no_duplicates, parse_constant=reject_constant)
except (OSError, ValueError, json.JSONDecodeError): raise SystemExit("artifact-evidence-json-invalid")
if not isinstance(cargo, dict) or cargo.get("cargo_success") is not True: raise SystemExit("artifact-cargo-evidence-invalid")
if not isinstance(timing, dict) or not isinstance(timing.get("cargo_ms"), int) or timing["cargo_ms"] < 0: raise SystemExit("artifact-timing-invalid")
PY
}

artifact_publish() (
  local source=${1:?private artifact} root=${2:?publish root}
  umask 077
  [ -d "$source" ] && [ ! -L "$source" ] || { helper_die 'private artifact is missing'; return 1; }
  mkdir -p -- "$root"
  [ -d "$root" ] && [ ! -L "$root" ] || { helper_die 'publish root is invalid'; return 1; }
  local digest destination lock lock_fd temporary move_status
  # The directory name identifies every immutable published byte, including
  # Cargo and timing evidence.  Two compilations of the same binary/receipt
  # therefore coexist when their evidence differs.
  digest=$(tree_hash "$source") || return 1
  destination="$root/$digest"
  lock="$root/.publish-lock-$digest"
  [ ! -L "$lock" ] || { helper_die 'publish lock symlink'; return 1; }
  exec {lock_fd}<>"$lock" || return 1
  # Keep one inode; never unlink another publisher's lock. The subshell owns
  # the inherited open description and releases it on every return/exit path.
  python3 - "$lock_fd" "$lock" <<'PY'
import fcntl,os,stat,sys,time
fd=int(sys.argv[1]);info=os.fstat(fd);path=os.lstat(sys.argv[2])
if (not stat.S_ISREG(info.st_mode) or not stat.S_ISREG(path.st_mode) or
        (info.st_dev,info.st_ino)!=(path.st_dev,path.st_ino) or
        info.st_uid!=os.geteuid() or info.st_nlink!=1 or stat.S_IMODE(info.st_mode)!=0o600):
    raise SystemExit("publish-lock-identity-invalid")
deadline=time.monotonic()+5
while True:
    try:fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB);break
    except BlockingIOError:
        if time.monotonic()>=deadline:raise SystemExit("publish-lock-timeout")
        time.sleep(0.05)
PY
  [ "$?" -eq 0 ] || return 1
  if [ -e "$destination" ]; then
    if [ -d "$destination" ] && [ ! -L "$destination" ] && [ "$(tree_hash "$source")" = "$(tree_hash "$destination")" ]; then
      printf '%s\n' "$destination"
      return 0
    fi
    helper_die 'publish no-clobber mismatch'
    return 1
  fi
  temporary="$root/.partial-$digest-$$-$RANDOM"
  if ! mkdir -- "$temporary"; then helper_die 'publish private directory creation failed'; return 1; fi
  if ! cp -a -- "$source/." "$temporary/"; then rm -rf -- "$temporary"; helper_die 'publish copy failed'; return 1; fi
  if python3 - "$temporary" "$destination" <<'PY'
import os, sys
try:
    os.rename(sys.argv[1], sys.argv[2])
except OSError:
    raise SystemExit(17)
PY
  then move_status=0
  else move_status=$?
  fi
  if [ "$move_status" -ne 0 ]; then
    if [ "$move_status" -eq 17 ] && [ -d "$destination" ] && [ ! -L "$destination" ] && [ "$(tree_hash "$source")" = "$(tree_hash "$destination")" ]; then
      rm -rf -- "$temporary"
    else
      rm -rf -- "$temporary"
      printf '%s\n' "build-layout-helper: publish rename failed status=$move_status" >&2
      return "$move_status"
    fi
  fi
  [ "$(tree_hash "$source")" = "$digest" ] && [ "$(tree_hash "$destination")" = "$digest" ] || { helper_die 'publish bytes changed'; return 1; }
  printf '%s\n' "$destination"
)

injected_export_failure() {
  local dir=${1:?partial directory}
  [ -d "$dir" ] && [ ! -L "$dir" ] && [ -f "$dir/INCOMPLETE" ] && [ ! -L "$dir/INCOMPLETE" ] || { helper_die 'export failure marker is missing'; return 1; }
  cmp -s -- "$dir/INCOMPLETE" <(printf 'fixture-export-failure\n') || { helper_die 'export failure marker is invalid'; return 1; }
  [ ! -e "$dir/artifact.json" ] && [ ! -L "$dir/artifact.json" ] && [ ! -e "$dir/COMPLETE" ] && [ ! -L "$dir/COMPLETE" ] || { helper_die 'failed export became complete'; return 1; }
  printf '%s\n' 'build-layout-helper: injected export failure status=73' >&2
  return 73
}

ledger_plan() {
  local path=${1:?ledger} key=${2:?cache key} lib_hash=${3:?lib hash} app_hash=${4:?app hash}
  PROBE_LEDGER="$path" PROBE_KEY="$key" PROBE_LIB="$lib_hash" PROBE_APP="$app_hash" python3 - <<'PY'
import json, os
path = os.environ["PROBE_LEDGER"]
if not os.path.isfile(path) or os.path.islink(path):
    print("rebuild=all reason=missing"); raise SystemExit(0)
def no_duplicates(pairs):
    result = {}
    for key, value in pairs:
        if key in result: raise ValueError("duplicate-json-key")
        result[key] = value
    return result
try:
    with open(path, encoding="utf-8") as handle: value = json.load(handle, object_pairs_hook=no_duplicates)
except (OSError, ValueError, json.JSONDecodeError):
    print("rebuild=all reason=malformed"); raise SystemExit(0)
required = {"compatibility_key", "format", "packages", "pending"}
if not isinstance(value, dict) or set(value) != required or value.get("format") != "lagrange-build-layout-ledger-v2" or value.get("pending") is not False:
    print("rebuild=all reason=unknown-or-pending"); raise SystemExit(0)
packages = value.get("packages")
if not isinstance(packages, dict) or set(packages) != {"build-cache-fixture-lib", "build-cache-fixture-app"} or not all(isinstance(v, str) for v in packages.values()):
    print("rebuild=all reason=packages"); raise SystemExit(0)
if value.get("compatibility_key") != os.environ["PROBE_KEY"]:
    print("rebuild=all reason=compatibility-key"); raise SystemExit(0)
if packages["build-cache-fixture-lib"] != os.environ["PROBE_LIB"]:
    print("rebuild=packages packages=build-cache-fixture-lib,build-cache-fixture-app reason=input-change")
elif packages["build-cache-fixture-app"] != os.environ["PROBE_APP"]:
    print("rebuild=packages packages=build-cache-fixture-app reason=input-change")
else:
    print("rebuild=none reason=unchanged")
PY
}

ledger_write() {
  local path=${1:?ledger} key=${2:?cache key} lib_hash=${3:?lib hash} app_hash=${4:?app hash} pending=${5:?pending}
  [[ "$pending" = true || "$pending" = false ]] || { helper_die 'ledger pending is invalid'; return 1; }
  PROBE_LEDGER="$path" PROBE_KEY="$key" PROBE_LIB="$lib_hash" PROBE_APP="$app_hash" PROBE_PENDING="$pending" python3 - <<'PY'
import json, os
path = os.environ["PROBE_LEDGER"]
parent = os.path.dirname(path)
if not os.path.isdir(parent) or os.path.islink(parent): raise SystemExit("ledger-parent-invalid")
value = {"compatibility_key": os.environ["PROBE_KEY"], "format": "lagrange-build-layout-ledger-v2", "packages": {"build-cache-fixture-app": os.environ["PROBE_APP"], "build-cache-fixture-lib": os.environ["PROBE_LIB"]}, "pending": os.environ["PROBE_PENDING"] == "true"}
temporary = path + ".tmp"
with open(temporary, "w", encoding="utf-8", newline="\n") as handle:
    handle.write(json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n")
os.replace(temporary, path)
PY
}

# This operates only inside the fixture-owned target cache mount. A pending
# ledger is written before calling it; if interrupted, the old ledger is either
# still pending or absent, both of which force regeneration next time.
ledger_reset_target() {
  local target=${1:?target}
  python3 - "$target" <<'PY'
import os, shutil, stat, sys
target = os.path.abspath(sys.argv[1])
if target in ("/", "/tmp") or not os.path.isabs(target): raise SystemExit("unsafe-target-reset")
if os.path.lexists(target):
    info = os.lstat(target)
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode): raise SystemExit("target-reset-invalid")
else:
    os.makedirs(target, mode=0o700)
for name in os.listdir(target):
    path = os.path.join(target, name)
    info = os.lstat(path)
    if stat.S_ISLNK(info.st_mode) or stat.S_ISREG(info.st_mode): os.unlink(path)
    elif stat.S_ISDIR(info.st_mode): shutil.rmtree(path)
    else: raise SystemExit("target-reset-special-file")
with open(os.path.join(target, ".build-layout-owned-v1"), "w", encoding="ascii") as handle: handle.write("owned\n")
PY
}

parse_cargo_json() {
  local log=${1:?log} bin=${2:?binary} output=${3:?summary} host=${4:?host triple}
  PROBE_LOG="$log" PROBE_BIN="$bin" PROBE_SUMMARY="$output" PROBE_HOST="$host" python3 - <<'PY'
import hashlib, json, os, re
log, bin_name, output = os.environ["PROBE_LOG"], os.environ["PROBE_BIN"], os.environ["PROBE_SUMMARY"]
def no_duplicates(pairs):
    result = {}
    for key, value in pairs:
        if key in result: raise ValueError("duplicate-json-key")
        result[key] = value
    return result
events = []
verbose_build_script_runs = []
verbose_lines = []
with open(log, encoding="utf-8", errors="strict") as handle:
    for number, raw in enumerate(handle, 1):
        text = re.sub(r"^#[0-9]+ [0-9]+(?:\.[0-9]+)? ", "", raw).lstrip()
        if re.match(r"(?:Fresh|Compiling) (?:itoa|build-cache-fixture-(?:app|lib)) v[0-9]", text) or re.match(r"Running `", text):
            verbose_lines.append(hashlib.sha256(raw.encode()).hexdigest())
        if ("Running" in raw and re.search(r"(?:^|[/\\])build-script-build(?:[ `\"']|$)", raw)
                and "--crate-name build_script_build" not in raw):
            verbose_build_script_runs.append(hashlib.sha256(raw.encode()).hexdigest())
        line = raw.rstrip("\n")
        candidate = line.lstrip()
        brace = candidate.find("{")
        if brace >= 0:
            candidate = candidate[brace:]
            if candidate.startswith("{"):
                try: value = json.loads(candidate, object_pairs_hook=no_duplicates, parse_constant=lambda x: (_ for _ in ()).throw(ValueError(x)))
                except (ValueError, json.JSONDecodeError):
                    if '"reason"' in candidate or candidate.endswith("}"):
                        raise SystemExit(f"malformed-cargo-json-line-{number}")
                    continue
                if not isinstance(value, dict) or not isinstance(value.get("reason"), str):
                    raise SystemExit(f"invalid-cargo-event-line-{number}")
                events.append(value)
artifacts = []
for event in events:
    if event.get("reason") != "compiler-artifact": continue
    target = event.get("target")
    profile = event.get("profile")
    if (not isinstance(target, dict) or not isinstance(target.get("name"), str) or
            not isinstance(target.get("kind"), list) or not all(isinstance(x, str) for x in target["kind"])):
        raise SystemExit("malformed-compiler-artifact")
    if (not isinstance(event.get("package_id"), str) or not isinstance(event.get("fresh"), bool) or
            not isinstance(event.get("features"), list) or not all(isinstance(x, str) for x in event["features"]) or
            not isinstance(profile, dict)):
        raise SystemExit("malformed-compiler-artifact-fields")
    artifacts.append({"executable": event.get("executable"), "features": event["features"], "fresh": event["fresh"], "package_id": event["package_id"], "profile": profile, "target": {"crate_types": target.get("crate_types"), "edition": target.get("edition"), "kind": target["kind"], "name": target["name"], "src_path": target.get("src_path")}})
finished = [event for event in events if event.get("reason") == "build-finished"]
if len(finished) != 1 or finished[0].get("success") is not True:
    raise SystemExit("missing-or-unsuccessful-cargo-finish")
app_ids = {"path+file:///build/fixture-app#0.1.0", "path+file:///build/fixture-app#build-cache-fixture-app@0.1.0"}
lib_ids = {"path+file:///build/fixture-lib#0.1.0", "path+file:///build/fixture-lib#build-cache-fixture-lib@0.1.0"}
itoa_id = "registry+https://github.com/rust-lang/crates.io-index#itoa@1.0.18"
matching = [item for item in artifacts if item["package_id"] in app_ids and item["target"]["name"] == bin_name and item["target"]["kind"] == ["bin"] and isinstance(item.get("executable"), str)]
if len(matching) != 1: raise SystemExit("missing-or-ambiguous-cargo-executable")
if matching[0]["executable"] != "/cargo-target/release/" + bin_name: raise SystemExit("cargo-executable-path-invalid")
if matching[0]["target"].get("src_path") != "/build/fixture-app/src/bin/" + bin_name + ".rs": raise SystemExit("app-source-path-invalid")
itoa = [item for item in artifacts if item["package_id"] == itoa_id and item["target"]["name"] == "itoa" and item["target"]["kind"] == ["lib"]]
lib = [item for item in artifacts if item["package_id"] in lib_ids and item["target"]["name"] == "build_cache_fixture_lib" and item["target"]["kind"] == ["lib"]]
build = [item for item in artifacts if item["package_id"] in app_ids and item["target"]["kind"] == ["custom-build"] and item["target"].get("src_path") == "/build/fixture-app/build.rs"]
if len(itoa) != 1 or len(lib) != 1 or len(build) != 1: raise SystemExit("missing-or-ambiguous-fixture-unit-evidence")
if not isinstance(itoa[0]["target"].get("src_path"), str) or not re.search(r"/itoa-1\.0\.18/src/lib\.rs$", itoa[0]["target"]["src_path"]): raise SystemExit("itoa-source-or-version-evidence-missing")
if lib[0]["target"].get("src_path") != "/build/fixture-lib/src/lib.rs": raise SystemExit("lib-source-path-invalid")
build_scripts = [event for event in events if event.get("reason") == "build-script-executed"]
if len(build_scripts) != 1 or build_scripts[0].get("package_id") not in app_ids: raise SystemExit("missing-or-ambiguous-build-script-event")
if not verbose_lines: raise SystemExit("missing-cargo-verbose-evidence")
value = {"artifact_events": artifacts, "build_script_event_records": build_scripts, "build_script_events": len(build_scripts), "cargo_success": True, "event_count": len(events), "host_triple": os.environ["PROBE_HOST"], "log_sha256": hashlib.sha256(open(log,"rb").read()).hexdigest(), "observed_bin": bin_name, "matching_executable": matching[0]["executable"], "units": {"app": matching[0], "build_script_compile": build[0], "itoa": itoa[0], "lib": lib[0]}, "verbose_build_script_run_count": len(verbose_build_script_runs), "verbose_build_script_run_line_sha256": verbose_build_script_runs, "verbose_line_sha256": verbose_lines}
with open(output, "w", encoding="utf-8", newline="\n") as handle:
    handle.write(json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n")
PY
}

cargo_assert() {
  local summary=${1:?summary} bin=${2:?bin} requested=${3:?requested features} resolved=${4:?resolved features}
  local host=${5:?host} app_fresh=${6:?app fresh} lib_fresh=${7:?lib fresh} itoa_fresh=${8:?itoa fresh}
  PROBE_SUMMARY="$summary" PROBE_BIN="$bin" PROBE_REQUESTED="$requested" PROBE_RESOLVED="$resolved" PROBE_HOST="$host" \
  PROBE_APP_FRESH="$app_fresh" PROBE_LIB_FRESH="$lib_fresh" PROBE_ITOA_FRESH="$itoa_fresh" python3 - <<'PY'
import json, os, re
def no_duplicates(pairs):
    result = {}
    for key, value in pairs:
        if key in result: raise ValueError("duplicate-json-key")
        result[key] = value
    return result
try:
    value = json.load(open(os.environ["PROBE_SUMMARY"], encoding="utf-8"), object_pairs_hook=no_duplicates)
    requested = json.loads(os.environ["PROBE_REQUESTED"])
    resolved = json.loads(os.environ["PROBE_RESOLVED"])
except (OSError, ValueError, json.JSONDecodeError): raise SystemExit("cargo-summary-invalid")
if value.get("cargo_success") is not True: raise SystemExit("cargo-not-successful")
units = value.get("units")
if not isinstance(units, dict) or set(units) != {"app", "build_script_compile", "itoa", "lib"}: raise SystemExit("cargo-units-invalid")
if requested not in ([], ["wide"]): raise SystemExit("cargo-requested-features-invalid")
if resolved != (["default"] if requested == [] else ["default", "wide"]): raise SystemExit("cargo-resolved-feature-oracle-invalid")
if value.get("host_triple") != os.environ["PROBE_HOST"]: raise SystemExit("cargo-host-triple-mismatch")
checks = (("app", os.environ["PROBE_APP_FRESH"] == "true"), ("lib", os.environ["PROBE_LIB_FRESH"] == "true"), ("itoa", os.environ["PROBE_ITOA_FRESH"] == "true"))
for name, expected in checks:
    unit = units[name]
    if not isinstance(unit, dict) or unit.get("fresh") is not expected: raise SystemExit("unexpected-recompile-set-" + name)
if units["app"].get("target", {}).get("name") != os.environ["PROBE_BIN"] or units["app"].get("features") != resolved: raise SystemExit("app-unit-contract-mismatch")
if units["lib"].get("features") != resolved or units["build_script_compile"].get("features") != resolved: raise SystemExit("local-feature-contract-mismatch")
if units["itoa"].get("features") != []: raise SystemExit("itoa-feature-contract-mismatch")
for name in ("app", "lib", "itoa"):
    profile = units[name].get("profile")
    if (not isinstance(profile, dict) or profile.get("opt_level") != "3" or profile.get("test") is not False or
            profile.get("debug_assertions") is not False or profile.get("overflow_checks") is not False):
        raise SystemExit("release-profile-contract-mismatch-" + name)
build_profile = units["build_script_compile"].get("profile")
if not isinstance(build_profile, dict) or build_profile.get("opt_level") != "0" or build_profile.get("test") is not False:
    raise SystemExit("build-script-profile-contract-mismatch")
if not isinstance(units["build_script_compile"].get("fresh"),bool): raise SystemExit("build-script-compile-evidence-invalid")
count = value.get("verbose_build_script_run_count")
if type(count) is not int or count < 0: raise SystemExit("build-script-run-count-invalid")
hashes = value.get("verbose_build_script_run_line_sha256")
if not isinstance(hashes, list) or len(hashes)!=count or any(not isinstance(x,str) or not re.fullmatch(r"[0-9a-f]{64}",x) for x in hashes): raise SystemExit("build-script-run-hash-invalid")
verbose = value.get("verbose_line_sha256")
if not isinstance(verbose,list) or not verbose or any(not isinstance(x,str) or not re.fullmatch(r"[0-9a-f]{64}",x) for x in verbose): raise SystemExit("missing-cargo-verbose-evidence")
if not all(item in verbose for item in hashes): raise SystemExit("build-script-run-verbose-mismatch")
events = value.get("build_script_event_records")
if not isinstance(events,list) or not events or type(value.get("build_script_events")) is not int or len(events)!=value["build_script_events"]: raise SystemExit("missing-build-script-json-evidence")
PY
}

build_log_mode() {
  local log=${1:?build log}
  python3 - "$log" <<'PY'
import re, sys
lines = open(sys.argv[1], encoding="utf-8", errors="strict").read().splitlines()
executed = [line for line in lines if re.search(r"(?:^| )BUILD_LAYOUT_CARGO_MS=[0-9]+$", line)]
vertices = {}
cached = set()
for line in lines:
    match = re.match(r"^(#[0-9]+) .*RUN --mount=type=cache,target=/usr/local/cargo/registry", line)
    if match: vertices[match.group(1)] = True
    match = re.match(r"^(#[0-9]+) CACHED$", line)
    if match: cached.add(match.group(1))
hits = set(vertices).intersection(cached)
if len(executed) == 1 and not hits:
    print("executed")
elif not executed and len(hits) == 1:
    print("cache-hit")
else:
    raise SystemExit("compile-vertex-evidence-ambiguous")
PY
}

cargo_elapsed_ms() {
  local log=${1:?build log}
  python3 - "$log" <<'PY'
import re, sys
values=[]
for line in open(sys.argv[1], encoding="utf-8", errors="strict"):
    match=re.search(r"(?:^| )BUILD_LAYOUT_CARGO_MS=([0-9]+)$",line.rstrip("\n"))
    if match: values.append(int(match.group(1)))
if len(values)!=1: raise SystemExit("cargo-timing-evidence-ambiguous")
print(values[0])
PY
}

gate_validate_list() {
  local value=${1-} kind=${2:?kind} item count=0
  local -a items=()
  local -A seen=()
  [ "$kind" = unit ] || [ "$kind" = container ] || { helper_die 'invalid health list kind'; return 1; }
  [ -n "$value" ] || { helper_die "$kind health list is empty"; return 1; }
  [[ "$value" != *[[:space:]]* ]] || { helper_die "$kind health list contains whitespace"; return 1; }
  [[ "$value" != ,* && "$value" != *, && "$value" != *,,* ]] || { helper_die "$kind health list contains an empty item"; return 1; }
  IFS=',' read -r -a items <<<"$value"
  for item in "${items[@]}"; do
    [ -n "$item" ] || { helper_die "$kind health list contains an empty item"; return 1; }
    [[ "$item" != *[[:space:]/\*\?]* ]] || { helper_die "$kind health item has forbidden characters"; return 1; }
    if [ "$kind" = unit ]; then [[ "$item" =~ ^[A-Za-z0-9_.:@-]+\.service$ ]] || { helper_die 'unit name is not literal'; return 1; }; fi
    if [ "$kind" = container ]; then [[ "$item" =~ ^[A-Za-z0-9_.:@-]+$ ]] || { helper_die 'container name is not literal'; return 1; }; fi
    [ -z "${seen[$item]+x}" ] || { helper_die "$kind health list contains a duplicate"; return 1; }
    seen[$item]=1; count=$((count + 1))
  done
  [ "$count" -ge 1 ] && [ "$count" -le 16 ] || { helper_die "$kind health list count is outside 1..16"; return 1; }
  if [ "$kind" = container ] && [ -z "${seen[lagrange-station-research-worker-1]+x}" ]; then
    helper_die 'research-worker container is not listed'; return 1
  fi
  return 0
}

gate_test_allowed() {
  [ "${BUILD_LAYOUT_SELF_TEST_ACTIVE-}" = 1 ] && [ "${LAYOUT_GATE_TEST_SEAM-}" = build-layout-self-test ]
}

# Internal to the image-only layout controller; no helper CLI or health bypass.
# The binding includes exact bytes, so even whitespace changes require a new run.
research_exception_check() {
  local expected=${1-} snapshot=${2-} action=${3:-verify}
  python3 - "${BUILD_LAYOUT_RESEARCH_EXCEPTION-}" "$expected" "$snapshot" "$action" "${BUILD_LAYOUT_RESEARCH_EXCEPTION+x}" <<'PY'
import base64,datetime,hashlib,json,os,re,stat,sys,time
path,expected,snapshot,action,supplied=sys.argv[1:]
def fail(reason):raise SystemExit("research-exception-"+reason)
def pairs(items):
    value={}
    for key,item in items:
        if key in value:fail("duplicate-key")
        value[key]=item
    return value
def parent_fd(path):
    if (not path.startswith("/") or path.startswith("//") or os.path.normpath(path)!=path or
            any(ord(c)<32 or ord(c)==127 for c in path) or path=="/"):fail("path-invalid")
    fd=os.open("/",os.O_RDONLY|os.O_DIRECTORY)
    try:
        for part in path.split("/")[1:-1]:
            nxt=os.open(part,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd)
            os.close(fd);fd=nxt
        return fd
    except BaseException:
        os.close(fd);raise
def read_private(path):
    parent=parent_fd(path)
    try:
        fd=os.open(os.path.basename(path),os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=parent)
        with os.fdopen(fd,"rb") as handle:
            info=os.fstat(handle.fileno())
            if not stat.S_ISREG(info.st_mode):fail("not-regular")
            if info.st_uid!=os.geteuid():fail("owner-invalid")
            if stat.S_IMODE(info.st_mode)!=0o600:fail("mode-invalid")
            raw=handle.read()
            after=os.fstat(handle.fileno())
            if (info.st_size,info.st_mtime_ns,info.st_ctime_ns)!=(after.st_size,after.st_mtime_ns,after.st_ctime_ns):fail("changed-during-read")
            return raw
    finally:os.close(parent)
try:
    value=None;raw=None
    if supplied and not path:fail("path-invalid")
    if path:
        raw=read_private(path)
        fields=json.loads(raw.decode("utf-8"),object_pairs_hook=pairs)
        fixed={"format":"lagrange-build-research-exception-v1","scope":"image-build-only",
               "container_name":"lagrange-station-research-worker-1","known_error_code":"PRICE_CURATION_FAILED"}
        keys=set(fixed)|{"container_id","image_id","observed_at_utc","expires_at_utc","initial_restart_count","known_exit_code"}
        if type(fields) is not dict or set(fields)!=keys:fail("keys-invalid")
        if any(type(fields[k]) is not str or fields[k]!=v for k,v in fixed.items()):fail("contract-invalid")
        for key,pattern in (("container_id",r"[0-9a-f]{64}"),("image_id",r"sha256:[0-9a-f]{64}")):
            if type(fields[key]) is not str or not re.fullmatch(pattern,fields[key]):fail("identity-invalid")
        if type(fields["initial_restart_count"]) is not int or fields["initial_restart_count"]<0:fail("restart-type-invalid")
        if type(fields["known_exit_code"]) is not int or fields["known_exit_code"]!=2:fail("exit-type-invalid")
        def timestamp(key):
            text=fields[key]
            if type(text) is not str or not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z",text):fail("time-format-invalid")
            return int(datetime.datetime.strptime(text,"%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=datetime.timezone.utc).timestamp())
        observed=timestamp("observed_at_utc");expires=timestamp("expires_at_utc");now=time.time_ns()
        if not observed*10**9<=now<expires*10**9 or expires>observed+86400:fail("validity-invalid")
        value={"path":path,"sha256":hashlib.sha256(raw).hexdigest(),"bytes_base64":base64.b64encode(raw).decode("ascii"),"fields":fields}
    if expected and value!=json.loads(expected,object_pairs_hook=pairs):fail("binding-changed")
    if snapshot:
        if value is None:
            if os.path.lexists(snapshot):fail("unexpected-snapshot")
        elif action=="create":
            parent=parent_fd(snapshot)
            try:
                fd=os.open(os.path.basename(snapshot),os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600,dir_fd=parent)
                with os.fdopen(fd,"wb") as handle:
                    os.fchmod(handle.fileno(),0o600);handle.write(raw);handle.flush();os.fsync(handle.fileno())
            finally:os.close(parent)
        elif action=="verify":
            if read_private(snapshot)!=raw:fail("snapshot-changed")
        else:fail("action-invalid")
    print(json.dumps(value,sort_keys=True,separators=(",",":")))
except (OSError,ValueError,UnicodeError,OverflowError):fail("input-invalid")
PY
}

research_exception_validate_bound() {
  research_exception_check "${LAYOUT_RESEARCH_BINDING:-null}" "${LAYOUT_RESEARCH_SNAPSHOT-}" >/dev/null
}

research_exception_resume_check() {
  python3 - "$1" "$2" "$3" "${LAYOUT_RESEARCH_BINDING:-null}" <<'PY'
import datetime,json,os,stat,sys
state_path,gates_path,run_path,binding=sys.argv[1:];bound=json.loads(binding)
def read(path):
    info=os.lstat(path)
    if not stat.S_ISREG(info.st_mode) or os.path.realpath(path)!=path or info.st_uid!=os.geteuid() or stat.S_IMODE(info.st_mode)!=0o600:raise ValueError()
    return open(path,encoding="utf-8").read()
try:
    state=json.loads(read(state_path));records=[json.loads(line) for line in read(gates_path).splitlines()]
    gate=json.loads(read(run_path))["gate"];name=bound["fields"]["container_name"]
    expected={k:bound[k] for k in ("path","sha256","fields")}
    passed=[]
    for record in records:
        if record["evidence"].get("research_exception")!=expected:raise ValueError()
        if record["status"]=="PASS":
            if record["reason"]!="image-build-only-known-incident":raise ValueError()
            passed.append(record)
    if not passed:raise ValueError()
    def selected(record,kind):return {k:v["selected"] for k,v in record["evidence"][kind].items()}
    first=selected(passed[0],"containers");units=selected(passed[0],"units")
    if set(first)!=set(gate["containers"]) or set(units)!=set(gate["units"]):raise ValueError()
    if state["containers"]!=first or state["units"]!=units:raise ValueError()
    if state["journal_since_us"]!=gate["journal_since_us"] or state["journal_since"]!=gate["journal_since"]:raise ValueError()
    prior=bound["fields"]["initial_restart_count"];prior_time=0
    def stamp(text):return int(datetime.datetime.strptime(text,"%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=datetime.timezone.utc).timestamp())*10**9
    observed=stamp(bound["fields"]["observed_at_utc"]);expires=stamp(bound["fields"]["expires_at_utc"])
    for record in passed:
        containers=selected(record,"containers");sample=containers[name];ts=sample["monitored_at_unix_ns"];count=int(sample["restart_count"])
        if set(containers)!=set(first) or any(v!=first[k] for k,v in containers.items() if k!=name) or selected(record,"units")!=units:raise ValueError()
        if record["journal"]["boot_id"]!=state["boot_id"] or record["journal"]["since_us"]!=gate["journal_since_us"] or record["journal"]["since"]!=gate["journal_since"]:raise ValueError()
        if sample["id"]!=bound["fields"]["container_id"] or sample["image_id"]!=bound["fields"]["image_id"]:raise ValueError()
        limit=bound["fields"]["initial_restart_count"]+(ts-observed+30*10**9-1)//(30*10**9)+2
        if not observed<=ts<expires or ts<prior_time or not prior<=count<=limit or sample["restart_limit"]!=limit:raise ValueError()
        prior=count;prior_time=ts
    if state["research_exception"]!={"binding_sha256":bound["sha256"],"first_observation":first[name],"latest_observation":sample}:raise ValueError()
except (OSError,ValueError,KeyError,TypeError,IndexError):raise SystemExit("research-exception-resume-state-invalid")
PY
}

gate_proc_root() {
  if gate_test_allowed && [ -n "${LAYOUT_GATE_PROC_ROOT-}" ]; then printf '%s\n' "$LAYOUT_GATE_PROC_ROOT"; else printf '%s\n' /proc; fi
}

gate_command() {
  local name=$1
  case "$name" in
    systemctl) if gate_test_allowed && [ -n "${LAYOUT_GATE_SYSTEMCTL_BIN-}" ]; then printf '%s\n' "$LAYOUT_GATE_SYSTEMCTL_BIN"; else printf '%s\n' systemctl; fi ;;
    journalctl) if gate_test_allowed && [ -n "${LAYOUT_GATE_JOURNALCTL_BIN-}" ]; then printf '%s\n' "$LAYOUT_GATE_JOURNALCTL_BIN"; else printf '%s\n' journalctl; fi ;;
    docker) if gate_test_allowed && [ -n "${LAYOUT_GATE_DOCKER_BIN-}" ]; then printf '%s\n' "$LAYOUT_GATE_DOCKER_BIN"; else printf '%s\n' docker; fi ;;
    ps) if gate_test_allowed && [ -n "${LAYOUT_GATE_PS_BIN-}" ]; then printf '%s\n' "$LAYOUT_GATE_PS_BIN"; else printf '%s\n' ps; fi ;;
    *) helper_die 'unknown gate command'; return 1 ;;
  esac
}

gate_record() {
  local case_id=$1 phase=$2 previous=$3 status=$4 reason=$5 evidence=${6:-'{}'} path=${LAYOUT_GATE_RECORD_FILE:?gate record}
  mkdir -p -- "$(dirname -- "$path")"
  PROBE_RESEARCH="${LAYOUT_RESEARCH_BINDING:-null}" PROBE_RECORD="$path" PROBE_CASE="$case_id" PROBE_PHASE="$phase" PROBE_PREVIOUS="$previous" PROBE_STATUS="$status" PROBE_REASON="$reason" \
  PROBE_MEM="${LAYOUT_GATE_MEM:-unknown}" PROBE_SWAP="${LAYOUT_GATE_SWAP:-unknown}" \
  PROBE_JSTATUS="${LAYOUT_GATE_JOURNAL_STATUS:-unknown}" PROBE_JCOUNT="${LAYOUT_GATE_JOURNAL_COUNT:-unknown}" PROBE_JOOM="${LAYOUT_GATE_JOURNAL_OOM:-unknown}" \
  PROBE_JPROBE_EXIT="${LAYOUT_GATE_PROBE_EXIT:-unknown}" PROBE_JPROBE_OUT="${LAYOUT_GATE_PROBE_STDOUT_SHA:-unknown}" PROBE_JPROBE_ERR="${LAYOUT_GATE_PROBE_STDERR_SHA:-unknown}" \
  PROBE_JRANGE_EXIT="${LAYOUT_GATE_RANGE_EXIT:-unknown}" PROBE_JRANGE_OUT="${LAYOUT_GATE_RANGE_STDOUT_SHA:-unknown}" PROBE_JRANGE_ERR="${LAYOUT_GATE_RANGE_STDERR_SHA:-unknown}" \
  PROBE_JBOOT="${LAYOUT_JOURNAL_BOOT_ID:-unknown}" PROBE_JSINCE_US="${LAYOUT_JOURNAL_SINCE_US:-unknown}" PROBE_JSINCE="${LAYOUT_JOURNAL_SINCE:-unknown}" PROBE_JUNTIL="${LAYOUT_GATE_JOURNAL_UNTIL:-unknown}" PROBE_EVIDENCE="$evidence" python3 - <<'PY' >>"$path"
import json, os, time
evidence = json.loads(os.environ["PROBE_EVIDENCE"])
if not isinstance(evidence, dict): raise SystemExit("gate-evidence-invalid")
bound=json.loads(os.environ["PROBE_RESEARCH"])
if bound is not None:
    evidence["research_exception"]={k:bound[k] for k in ("path","sha256","fields")}
def integer_or_text(name):
    value=os.environ[name]
    return int(value) if value.isdigit() else value
value = {"case": os.environ["PROBE_CASE"], "evidence": evidence, "journal": {"boot_id":os.environ["PROBE_JBOOT"],"count": integer_or_text("PROBE_JCOUNT"),"oom_count":integer_or_text("PROBE_JOOM"),"probe":{"exit":integer_or_text("PROBE_JPROBE_EXIT"),"stderr_sha256":os.environ["PROBE_JPROBE_ERR"],"stdout_sha256":os.environ["PROBE_JPROBE_OUT"]},"range":{"exit":integer_or_text("PROBE_JRANGE_EXIT"),"stderr_sha256":os.environ["PROBE_JRANGE_ERR"],"stdout_sha256":os.environ["PROBE_JRANGE_OUT"]},"since": os.environ["PROBE_JSINCE"],"since_us":integer_or_text("PROBE_JSINCE_US"),"status": os.environ["PROBE_JSTATUS"],"until": os.environ["PROBE_JUNTIL"]}, "mem_available_kib": integer_or_text("PROBE_MEM"), "phase": os.environ["PROBE_PHASE"], "previous_exit": int(os.environ["PROBE_PREVIOUS"]), "reason": os.environ["PROBE_REASON"], "status": os.environ["PROBE_STATUS"], "swap_free_kib": integer_or_text("PROBE_SWAP"), "time_unix": time.time()}
print(json.dumps(value, sort_keys=True, separators=(",", ":")))
PY
}

gate_fail() {
  local evidence=${5-}
  [ -n "$evidence" ] || evidence='{}'
  LAYOUT_LAST_GATE_REASON=$4
  gate_record "$1" "$2" "$3" FAIL "$4" "$evidence"
  return 1
}

gate_parse_probe() {
  local output=${1:?output} boot=${2:?boot}
  python3 - "$output" "$boot" <<'PY'
import json, re, sys
path, expected = sys.argv[1:]
def norm(value):
    if not isinstance(value, str): raise ValueError("boot-id-type")
    value = value.replace("-", "").lower()
    if not re.fullmatch(r"[0-9a-f]{32}", value): raise ValueError("boot-id")
    return value
lines = [line for line in open(path, encoding="utf-8") if line.strip()]
if len(lines) != 1: raise SystemExit("probe-count")
value = json.loads(lines[0])
required = ("__REALTIME_TIMESTAMP", "__CURSOR", "_BOOT_ID", "_TRANSPORT", "MESSAGE")
if not isinstance(value, dict) or any(key not in value for key in required): raise SystemExit("probe-fields")
if value["_TRANSPORT"] != "kernel" or not isinstance(value["MESSAGE"], str) or not isinstance(value["__CURSOR"], str) or not value["__CURSOR"]: raise SystemExit("probe-contract")
if not isinstance(value["__REALTIME_TIMESTAMP"], str) or not re.fullmatch(r"[0-9]+", value["__REALTIME_TIMESTAMP"]): raise SystemExit("probe-timestamp")
if norm(value["_BOOT_ID"]) != norm(expected): raise SystemExit("probe-boot")
print(norm(value["_BOOT_ID"]))
PY
}

gate_parse_range() {
  local output=${1:?output} boot=${2:?boot} since_us=${3:?since} until_us=${4:?until}
  python3 - "$output" "$boot" "$since_us" "$until_us" <<'PY'
import json, re, sys
path, expected, since, until = sys.argv[1:]
since, until = int(since), int(until)
def norm(value):
    if not isinstance(value, str): raise ValueError("boot-id-type")
    value = value.replace("-", "").lower()
    if not re.fullmatch(r"[0-9a-f]{32}", value): raise ValueError("boot-id")
    return value
expected = norm(expected); count = 0; oom = 0
for raw in open(path, encoding="utf-8"):
    if not raw.strip(): continue
    value = json.loads(raw)
    required = ("__REALTIME_TIMESTAMP", "__CURSOR", "_BOOT_ID", "_TRANSPORT", "MESSAGE")
    if not isinstance(value, dict) or any(key not in value for key in required): raise SystemExit("range-fields")
    if value["_TRANSPORT"] != "kernel" or not isinstance(value["MESSAGE"], str) or not isinstance(value["__CURSOR"], str): raise SystemExit("range-contract")
    if not isinstance(value["__REALTIME_TIMESTAMP"], str) or not re.fullmatch(r"[0-9]+", value["__REALTIME_TIMESTAMP"]): raise SystemExit("range-timestamp")
    timestamp = int(value["__REALTIME_TIMESTAMP"])
    if timestamp < since or timestamp > until: raise SystemExit("range-outside-window")
    if norm(value["_BOOT_ID"]) != expected: raise SystemExit("range-boot")
    if re.search(r"out of memory|oom[-_ ]?kill|killed process", value["MESSAGE"], re.I): oom += 1
    count += 1
if count >= 1000: raise SystemExit("range-truncated")
print(f"{count} {oom}")
PY
}

gate_parse_unit() {
  local output=${1:?systemctl output}
  python3 - "$output" <<'PY'
import re, sys
lines = [line.rstrip("\n") for line in open(sys.argv[1], encoding="utf-8") if line.rstrip("\n")]
required = {"LoadState", "ActiveState", "SubState", "ExecMainStatus", "MainPID", "NRestarts"}
value = {}
for line in lines:
    if "=" not in line: raise SystemExit("unit-line")
    key, item = line.split("=", 1)
    if key in value or key not in required: raise SystemExit("unit-keys")
    value[key] = item
if set(value) != required: raise SystemExit("unit-missing")
if (value["LoadState"], value["ActiveState"], value["SubState"], value["ExecMainStatus"]) != ("loaded", "active", "running", "0"): raise SystemExit("unit-state")
if not re.fullmatch(r"[1-9][0-9]*", value["MainPID"]): raise SystemExit("unit-pid")
if not re.fullmatch(r"[0-9]+", value["NRestarts"]): raise SystemExit("unit-restarts")
import json
print(json.dumps({"active_state":value["ActiveState"],"exec_main_status":value["ExecMainStatus"],"load_state":value["LoadState"],"main_pid":value["MainPID"],"n_restarts":value["NRestarts"],"sub_state":value["SubState"]},sort_keys=True,separators=(",",":")))
PY
}

gate_parse_container() {
  local output=${1:?container output}
  python3 - "$output" <<'PY'
import re, sys
data = open(sys.argv[1], encoding="utf-8").read()
if not data.endswith("\n") or data.count("\n") != 1: raise SystemExit("container-line")
values = data[:-1].split("\t")
if len(values) != 7: raise SystemExit("container-columns")
ident, running, restarting, oom, health, restarts, project = values
if not re.fullmatch(r"[0-9a-f]{64}", ident): raise SystemExit("container-id")
if (running, restarting, oom, health, project) != ("true", "false", "false", "healthy", "lagrange-station"): raise SystemExit("container-state")
if not re.fullmatch(r"[0-9]+", restarts): raise SystemExit("container-restarts")
import json
print(json.dumps({"health_status":health,"id":ident,"oom_killed":oom,"project":project,"restart_count":restarts,"restarting":restarting,"running":running},sort_keys=True,separators=(",",":")))
PY
}

gate_parse_research_container() {
  local output=${1:?container output} name=${2:?container name}
  python3 - "$output" "$name" "${LAYOUT_RESEARCH_BINDING:-null}" <<'PY'
import datetime,json,re,sys,time
path,name,binding=sys.argv[1:];bound=json.loads(binding)
if bound is None or name!="lagrange-station-research-worker-1":raise SystemExit("research-container-unbound")
grant=bound["fields"];data=open(path,encoding="utf-8").read();values=data.rstrip("\n").split("\t")
# Keep safely typed counts even when another field rejects the observation.
def field(index,pattern):
    return values[index] if len(values)>index and re.fullmatch(pattern,values[index]) else "unknown"
ident=field(0,r"[0-9a-f]{64}");running=field(1,r"true|false");restarting=field(2,r"true|false")
oom=field(3,r"true|false");health=field(4,r"healthy|unhealthy|starting");restarts=field(5,r"[0-9]+")
project=field(6,r"lagrange-station");image=field(7,r"sha256:[0-9a-f]{64}");exit_code=field(8,r"[0-9]+")
now=time.time_ns()
def stamp(text):return int(datetime.datetime.strptime(text,"%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=datetime.timezone.utc).timestamp())*10**9
observed=stamp(grant["observed_at_utc"]);expires=stamp(grant["expires_at_utc"])
limit=grant["initial_restart_count"]+(now-observed+30*10**9-1)//(30*10**9)+2
reason=None
if not data.endswith("\n") or data.count("\n")!=1 or len(values)!=9:reason="research-container-columns"
elif ident!=grant["container_id"] or image!=grant["image_id"]:reason="research-container-identity"
elif running!="true" or oom!="false" or project!="lagrange-station" or health=="unknown":reason="research-container-state"
elif restarting not in ("true","false") or exit_code not in (("2",) if restarting=="true" else ("0","2")):reason="research-container-exit"
elif not observed<=now<expires:reason="research-container-expired"
elif restarts=="unknown" or not grant["initial_restart_count"]<=int(restarts)<=limit:reason="research-container-restart-bound"
value={"health_status":health,"id":ident,"image_id":image,"exit_code":exit_code,"oom_killed":oom,"project":project,
       "restart_count":restarts,"restarting":restarting,"running":running,"monitored_at_unix_ns":now,
       "monitored_at_utc":datetime.datetime.fromtimestamp(now//10**9,datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),"restart_limit":limit}
if reason:value["validation_error"]=reason
print(json.dumps(value,sort_keys=True,separators=(",",":")))
if reason:raise SystemExit(reason)
PY
}

gate_state_check_and_write() {
  local state=${1:?state} boot=${2:?boot} since_us=${3:?since microseconds} since=${4:?since text} units=${5:?units json} containers=${6:?containers json}
  PROBE_RESEARCH="${LAYOUT_RESEARCH_BINDING:-null}" PROBE_GATE_STATE="$state" PROBE_GATE_BOOT="$boot" PROBE_GATE_SINCE_US="$since_us" PROBE_GATE_SINCE="$since" PROBE_GATE_UNITS="$units" PROBE_GATE_CONTAINERS="$containers" python3 - <<'PY'
import json, os, tempfile
state_path = os.environ["PROBE_GATE_STATE"]
new = {"boot_id": os.environ["PROBE_GATE_BOOT"], "containers": json.loads(os.environ["PROBE_GATE_CONTAINERS"]), "journal_since": os.environ["PROBE_GATE_SINCE"], "journal_since_us": int(os.environ["PROBE_GATE_SINCE_US"]), "units": json.loads(os.environ["PROBE_GATE_UNITS"])}
if not isinstance(new["boot_id"],str) or not isinstance(new["containers"],dict) or not isinstance(new["units"],dict) or not isinstance(new["journal_since"],str) or new["journal_since_us"] < 0: raise SystemExit("gate-state-new-invalid")
bound=json.loads(os.environ["PROBE_RESEARCH"]);name="lagrange-station-research-worker-1"
if bound is not None:
    new["research_exception"]={"binding_sha256":bound["sha256"],"first_observation":new["containers"][name],"latest_observation":new["containers"][name]}
write=not os.path.exists(state_path)
if os.path.exists(state_path):
    old = json.load(open(state_path, encoding="utf-8"))
    if old.get("boot_id") != new["boot_id"]: raise SystemExit("boot-id-changed")
    if old.get("journal_since") != new["journal_since"] or old.get("journal_since_us") != new["journal_since_us"]: raise SystemExit("journal-since-changed")
    if ({k:(v.get("main_pid"),v.get("n_restarts")) for k,v in old.get("units",{}).items()} != {k:(v.get("main_pid"),v.get("n_restarts")) for k,v in new["units"].items()}): raise SystemExit("unit-identity-or-restart-changed")
    def strict(containers):
        return {k:(v.get("id"),v.get("restart_count")) for k,v in containers.items() if bound is None or k!=name}
    if set(old.get("containers",{}))!=set(new["containers"]) or strict(old["containers"])!=strict(new["containers"]):raise SystemExit("container-identity-or-restart-changed")
    if bound is None:
        if "research_exception" in old:raise SystemExit("research-exception-removed")
    else:
        prior=old.get("research_exception",{});latest=prior.get("latest_observation",{});current=new["containers"][name]
        if prior.get("binding_sha256")!=bound["sha256"] or prior.get("first_observation")!=old["containers"][name]:raise SystemExit("research-exception-state-changed")
        for sample in (old["containers"][name],latest,current):
            if sample.get("id")!=bound["fields"]["container_id"] or sample.get("image_id")!=bound["fields"]["image_id"]:raise SystemExit("research-identity-changed")
        if int(current["restart_count"])<int(latest["restart_count"]) or current["monitored_at_unix_ns"]<latest["monitored_at_unix_ns"]:raise SystemExit("research-restart-or-time-decreased")
        # Preserve initial identities/journal origin; advance only the monitored sample.
        old["research_exception"]["latest_observation"]=current;new=old;write=True
if write:
    parent = os.path.dirname(state_path); os.makedirs(parent, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".gate-state-", dir=parent)
    with os.fdopen(fd, "w", encoding="utf-8") as handle: json.dump(new, handle, sort_keys=True, separators=(",", ":")); handle.write("\n")
    os.replace(temporary, state_path)
PY
}

gate_fake() {
  local case_id=$1 phase=$2 previous=$3
  [ "$previous" -eq 0 ] || { gate_fail "$case_id" "$phase" "$previous" previous-step-failed '{}'; return 1; }
  LAYOUT_GATE_MEM=8388608 LAYOUT_GATE_SWAP=1048576 LAYOUT_GATE_JOURNAL_STATUS=PASS LAYOUT_GATE_JOURNAL_COUNT=0 LAYOUT_GATE_JOURNAL_OOM=0
  LAYOUT_GATE_PROBE_EXIT=0 LAYOUT_GATE_PROBE_STDOUT_SHA=fake-probe-stdout LAYOUT_GATE_PROBE_STDERR_SHA=fake-probe-stderr
  LAYOUT_GATE_RANGE_EXIT=0 LAYOUT_GATE_RANGE_STDOUT_SHA=fake-range-stdout LAYOUT_GATE_RANGE_STDERR_SHA=fake-range-stderr LAYOUT_GATE_JOURNAL_UNTIL=1970-01-01T00:00:00Z
  : "${LAYOUT_JOURNAL_SINCE_US:=0}" "${LAYOUT_JOURNAL_SINCE:=1970-01-01 00:00:00 UTC}" "${LAYOUT_JOURNAL_BOOT_ID:=00000000000000000000000000000000}"
  case ${LAYOUT_FAKE_GATE_RESULT:-pass} in
    pass) gate_record "$case_id" "$phase" "$previous" PASS fake-pass '{}' ;;
    memory) gate_fail "$case_id" "$phase" "$previous" memory-threshold '{}' ;;
    journal) LAYOUT_GATE_JOURNAL_STATUS=FAIL; gate_fail "$case_id" "$phase" "$previous" kernel-journal-unestablished '{}' ;;
    *) gate_fail "$case_id" "$phase" "$previous" fake-unknown-result '{}' ;;
  esac
}

gate_evidence_json() {
  local units=${1:?units evidence} containers=${2:?containers evidence} ps_exit=${3:-unknown} ps_out=${4:-unknown} ps_err=${5:-unknown}
  python3 - "$units" "$containers" "$ps_exit" "$ps_out" "$ps_err" <<'PY'
import json, os, sys
units_path, containers_path, ps_exit, ps_out, ps_err = sys.argv[1:]
def read(path):
    value={}
    if not os.path.exists(path): return value
    for raw in open(path, encoding="utf-8"):
        name,status,out_hash,err_hash,selected=raw.rstrip("\n").split("\t",4)
        value[name]={"command_exit":int(status),"stderr_sha256":err_hash,"stdout_sha256":out_hash,"selected":json.loads(selected)}
    return value
process_exit=int(ps_exit) if ps_exit.isdigit() else ps_exit
print(json.dumps({"containers":read(containers_path),"process_scan":{"exit":process_exit,"stderr_sha256":ps_err,"stdout_sha256":ps_out},"units":read(units_path)},sort_keys=True,separators=(",",":")))
PY
}

gate_real() {
  local case_id=$1 phase=$2 previous=$3
  LAYOUT_GATE_MEM=unknown LAYOUT_GATE_SWAP=unknown LAYOUT_GATE_JOURNAL_STATUS=unknown LAYOUT_GATE_JOURNAL_COUNT=unknown LAYOUT_GATE_JOURNAL_OOM=unknown
  LAYOUT_GATE_PROBE_EXIT=unknown LAYOUT_GATE_PROBE_STDOUT_SHA=unknown LAYOUT_GATE_PROBE_STDERR_SHA=unknown
  LAYOUT_GATE_RANGE_EXIT=unknown LAYOUT_GATE_RANGE_STDOUT_SHA=unknown LAYOUT_GATE_RANGE_STDERR_SHA=unknown LAYOUT_GATE_JOURNAL_UNTIL=unknown
  [ "$previous" -eq 0 ] || { gate_fail "$case_id" "$phase" "$previous" previous-step-failed '{}'; return 1; }
  gate_validate_list "${BUILD_LAYOUT_HEALTH_UNITS-}" unit || { gate_fail "$case_id" "$phase" "$previous" invalid-health-units '{}'; return 1; }
  gate_validate_list "${BUILD_LAYOUT_HEALTH_CONTAINERS-}" container || { gate_fail "$case_id" "$phase" "$previous" invalid-health-containers '{}'; return 1; }
  research_exception_validate_bound || { gate_fail "$case_id" "$phase" "$previous" research-exception-invalid '{}'; return 1; }
  local proc_root mem swap
  proc_root=$(gate_proc_root) || return 1
  if ! mem=$(awk '$1 == "MemAvailable:" {print $2; found=1} END {if (!found) exit 1}' "$proc_root/meminfo"); then gate_fail "$case_id" "$phase" "$previous" meminfo-unreadable '{}'; return 1; fi
  if ! swap=$(awk '$1 == "SwapFree:" {print $2; found=1} END {if (!found) exit 1}' "$proc_root/meminfo"); then gate_fail "$case_id" "$phase" "$previous" swapinfo-unreadable '{}'; return 1; fi
  [[ "$mem" =~ ^[0-9]+$ ]] || { gate_fail "$case_id" "$phase" "$previous" meminfo-invalid '{}'; return 1; }
  [[ "$swap" =~ ^[0-9]+$ ]] || { gate_fail "$case_id" "$phase" "$previous" swapinfo-invalid '{}'; return 1; }
  LAYOUT_GATE_MEM=$mem; LAYOUT_GATE_SWAP=$swap
  [ "$mem" -ge 2097152 ] || { gate_fail "$case_id" "$phase" "$previous" memory-threshold '{}'; return 1; }
  [ "$swap" -ge 524288 ] || { gate_fail "$case_id" "$phase" "$previous" swap-threshold '{}'; return 1; }
  local private_root temp boot journal_bin probe_status range_status probe_out_hash probe_err_hash range_out_hash range_err_hash parsed_boot since_us until_us until_text range_result count oom
  private_root=${LAYOUT_GATE_PRIVATE_DIR:-"${TMPDIR:-/tmp}"}
  mkdir -p -- "$private_root" || { gate_fail "$case_id" "$phase" "$previous" gate-temp-unavailable '{}'; return 1; }
  temp=$(mktemp -d "$private_root/build-layout-gate.XXXXXX") || { gate_fail "$case_id" "$phase" "$previous" gate-temp-unavailable '{}'; return 1; }
  chmod 0700 -- "$temp" || { rm -rf -- "$temp"; gate_fail "$case_id" "$phase" "$previous" gate-temp-permission '{}'; return 1; }
  local units_lines="$temp/units.tsv" containers_lines="$temp/containers.tsv" ps_status=0 ps_out_hash ps_err_hash evidence
  : >"$units_lines"; : >"$containers_lines"
  local ps_bin
  ps_bin=$(gate_command ps) || { rm -rf -- "$temp"; return 1; }
  "$ps_bin" -eo comm= >"$temp/ps.out" 2>"$temp/ps.err" || ps_status=$?
  ps_out_hash=$(sha256_file "$temp/ps.out") || { rm -rf -- "$temp"; return 1; }
  ps_err_hash=$(sha256_file "$temp/ps.err") || { rm -rf -- "$temp"; return 1; }
  if [ "$ps_status" -ne 0 ] || [ -s "$temp/ps.err" ]; then
    evidence=$(gate_evidence_json "$units_lines" "$containers_lines" "$ps_status" "$ps_out_hash" "$ps_err_hash"); rm -rf -- "$temp"
    gate_fail "$case_id" "$phase" "$previous" proc-unreadable "$evidence"; return 1
  fi
  if awk '$1 == "cargo" || $1 == "rustc" || $1 == "rustdoc" {found=1} END {exit found ? 0 : 1}' "$temp/ps.out"; then
    evidence=$(gate_evidence_json "$units_lines" "$containers_lines" "$ps_status" "$ps_out_hash" "$ps_err_hash"); rm -rf -- "$temp"
    gate_fail "$case_id" "$phase" "$previous" compiler-process-active "$evidence"; return 1
  fi
  if ! boot=$(cat "$proc_root/sys/kernel/random/boot_id"); then rm -rf -- "$temp"; gate_fail "$case_id" "$phase" "$previous" boot-id-unreadable '{}'; return 1; fi
  journal_bin=$(gate_command journalctl) || { rm -rf -- "$temp"; return 1; }
  probe_status=0
  LC_ALL=C timeout 10s "$journal_bin" -k -b --no-pager -o json -n 1 >"$temp/probe.out" 2>"$temp/probe.err" || probe_status=$?
  probe_out_hash=$(sha256_file "$temp/probe.out") || { rm -rf -- "$temp"; return 1; }
  probe_err_hash=$(sha256_file "$temp/probe.err") || { rm -rf -- "$temp"; return 1; }
  LAYOUT_GATE_PROBE_EXIT=$probe_status LAYOUT_GATE_PROBE_STDOUT_SHA=$probe_out_hash LAYOUT_GATE_PROBE_STDERR_SHA=$probe_err_hash
  if [ "$probe_status" -ne 0 ] || [ -s "$temp/probe.err" ] || ! parsed_boot=$(gate_parse_probe "$temp/probe.out" "$boot"); then
    LAYOUT_GATE_JOURNAL_STATUS=FAIL LAYOUT_GATE_JOURNAL_COUNT=unknown
    evidence=$(gate_evidence_json "$units_lines" "$containers_lines" "$ps_status" "$ps_out_hash" "$ps_err_hash"); rm -rf -- "$temp"
    gate_fail "$case_id" "$phase" "$previous" kernel-journal-unestablished "$evidence"; return 1
  fi
  until_us=$(( $(date -u +%s) * 1000000 ))
  if [ -z "${LAYOUT_JOURNAL_SINCE_US:-}" ]; then
    since_us=$((until_us - 1800000000))
    LAYOUT_JOURNAL_SINCE_US=$since_us
    LAYOUT_JOURNAL_BOOT_ID=$parsed_boot
    LAYOUT_JOURNAL_SINCE=$(date -u -d "@$((since_us / 1000000))" '+%Y-%m-%d %H:%M:%S UTC')
  else
    since_us=$LAYOUT_JOURNAL_SINCE_US
    if [ -z "${LAYOUT_JOURNAL_BOOT_ID:-}" ]; then LAYOUT_JOURNAL_BOOT_ID=$parsed_boot
    elif [ "$LAYOUT_JOURNAL_BOOT_ID" != "$parsed_boot" ]; then evidence=$(gate_evidence_json "$units_lines" "$containers_lines" "$ps_status" "$ps_out_hash" "$ps_err_hash"); rm -rf -- "$temp"; gate_fail "$case_id" "$phase" "$previous" boot-id-changed "$evidence"; return 1
    fi
  fi
  until_text=$(date -u -d "@$((until_us / 1000000))" '+%Y-%m-%d %H:%M:%S UTC') || { rm -rf -- "$temp"; gate_fail "$case_id" "$phase" "$previous" time-unreadable '{}'; return 1; }
  range_status=0
  LC_ALL=C timeout 10s "$journal_bin" -k -b --no-pager -o json --since "$LAYOUT_JOURNAL_SINCE" --until "$until_text" -n 1000 >"$temp/range.out" 2>"$temp/range.err" || range_status=$?
  range_out_hash=$(sha256_file "$temp/range.out") || { rm -rf -- "$temp"; return 1; }
  range_err_hash=$(sha256_file "$temp/range.err") || { rm -rf -- "$temp"; return 1; }
  LAYOUT_GATE_RANGE_EXIT=$range_status LAYOUT_GATE_RANGE_STDOUT_SHA=$range_out_hash LAYOUT_GATE_RANGE_STDERR_SHA=$range_err_hash LAYOUT_GATE_JOURNAL_UNTIL=$until_text
  if [ "$range_status" -ne 0 ] || [ -s "$temp/range.err" ] || ! range_result=$(gate_parse_range "$temp/range.out" "$boot" "$since_us" "$until_us"); then
    LAYOUT_GATE_JOURNAL_STATUS=FAIL LAYOUT_GATE_JOURNAL_COUNT=unknown
    evidence=$(gate_evidence_json "$units_lines" "$containers_lines" "$ps_status" "$ps_out_hash" "$ps_err_hash"); rm -rf -- "$temp"
    gate_fail "$case_id" "$phase" "$previous" journal-range-invalid-or-oom "$evidence"; return 1
  fi
  read -r count oom <<<"$range_result"
  LAYOUT_GATE_JOURNAL_COUNT=$count LAYOUT_GATE_JOURNAL_OOM=$oom
  if [ "$oom" -ne 0 ]; then
    LAYOUT_GATE_JOURNAL_STATUS=FAIL
    evidence=$(gate_evidence_json "$units_lines" "$containers_lines" "$ps_status" "$ps_out_hash" "$ps_err_hash"); rm -rf -- "$temp"
    gate_fail "$case_id" "$phase" "$previous" kernel-oom-observed "$evidence"; return 1
  fi
  LAYOUT_GATE_JOURNAL_STATUS=PASS
  local unit unit_bin unit_status unit_value unit_out_hash unit_err_hash container docker_bin container_status container_value container_out_hash container_err_hash
  unit_bin=$(gate_command systemctl) || { rm -rf -- "$temp"; return 1; }
  IFS=',' read -r -a gate_units <<<"$BUILD_LAYOUT_HEALTH_UNITS"
  for unit in "${gate_units[@]}"; do
    unit_status=0
    LC_ALL=C timeout 10s "$unit_bin" show "$unit" --property=LoadState,ActiveState,SubState,ExecMainStatus,MainPID,NRestarts >"$temp/unit.out" 2>"$temp/unit.err" || unit_status=$?
    unit_out_hash=$(sha256_file "$temp/unit.out") || { rm -rf -- "$temp"; return 1; }; unit_err_hash=$(sha256_file "$temp/unit.err") || { rm -rf -- "$temp"; return 1; }
    if [ "$unit_status" -ne 0 ] || [ -s "$temp/unit.err" ] || ! unit_value=$(gate_parse_unit "$temp/unit.out"); then
      printf '%s\t%s\t%s\t%s\tnull\n' "$unit" "$unit_status" "$unit_out_hash" "$unit_err_hash" >>"$units_lines"
      evidence=$(gate_evidence_json "$units_lines" "$containers_lines" "$ps_status" "$ps_out_hash" "$ps_err_hash"); rm -rf -- "$temp"; gate_fail "$case_id" "$phase" "$previous" systemd-health-invalid "$evidence"; return 1
    fi
    printf '%s\t%s\t%s\t%s\t%s\n' "$unit" "$unit_status" "$unit_out_hash" "$unit_err_hash" "$unit_value" >>"$units_lines"
  done
  docker_bin=$(gate_command docker) || { rm -rf -- "$temp"; return 1; }
  local container_format='{{.Id}}{{printf "\t"}}{{.State.Running}}{{printf "\t"}}{{.State.Restarting}}{{printf "\t"}}{{.State.OOMKilled}}{{printf "\t"}}{{if .State.Health}}{{.State.Health.Status}}{{else}}absent{{end}}{{printf "\t"}}{{.RestartCount}}{{printf "\t"}}{{if index .Config.Labels "com.docker.compose.project"}}{{index .Config.Labels "com.docker.compose.project"}}{{else}}absent{{end}}'
  local research_format="$container_format"'{{printf "\t"}}{{.Image}}{{printf "\t"}}{{.State.ExitCode}}' selected_format parser parser_status
  IFS=',' read -r -a gate_containers <<<"$BUILD_LAYOUT_HEALTH_CONTAINERS"
  for container in "${gate_containers[@]}"; do
    container_status=0
    selected_format=$container_format; parser=gate_parse_container
    if [ "${LAYOUT_RESEARCH_BINDING:-null}" != null ] && [ "$container" = lagrange-station-research-worker-1 ]; then selected_format=$research_format; parser=gate_parse_research_container; fi
    timeout 10s "$docker_bin" inspect --type container --format "$selected_format" "$container" >"$temp/container.out" 2>"$temp/container.err" || container_status=$?
    container_out_hash=$(sha256_file "$temp/container.out") || { rm -rf -- "$temp"; return 1; }; container_err_hash=$(sha256_file "$temp/container.err") || { rm -rf -- "$temp"; return 1; }
    parser_status=0; container_value=null
    if [ "$container_status" -eq 0 ] && [ ! -s "$temp/container.err" ]; then
      container_value=$("$parser" "$temp/container.out" "$container") || parser_status=$?
    fi
    if [ "$container_status" -ne 0 ] || [ -s "$temp/container.err" ] || [ "$parser_status" -ne 0 ]; then
      printf '%s\t%s\t%s\t%s\t%s\n' "$container" "$container_status" "$container_out_hash" "$container_err_hash" "${container_value:-null}" >>"$containers_lines"
      evidence=$(gate_evidence_json "$units_lines" "$containers_lines" "$ps_status" "$ps_out_hash" "$ps_err_hash"); rm -rf -- "$temp"; gate_fail "$case_id" "$phase" "$previous" container-health-invalid "$evidence"; return 1
    fi
    printf '%s\t%s\t%s\t%s\t%s\n' "$container" "$container_status" "$container_out_hash" "$container_err_hash" "$container_value" >>"$containers_lines"
  done
  local units_json containers_json evidence state_file
  units_json=$(python3 - "$units_lines" <<'PY'
import json, sys
value={}
for line in open(sys.argv[1], encoding="utf-8"):
    name,status,out_hash,err_hash,selected=line.rstrip("\n").split("\t",4)
    value[name]=json.loads(selected)
print(json.dumps(value, sort_keys=True, separators=(",",":")))
PY
) || { rm -rf -- "$temp"; return 1; }
  containers_json=$(python3 - "$containers_lines" <<'PY'
import json, sys
value={}
for line in open(sys.argv[1], encoding="utf-8"):
    name,status,out_hash,err_hash,selected=line.rstrip("\n").split("\t",4)
    value[name]=json.loads(selected)
print(json.dumps(value, sort_keys=True, separators=(",",":")))
PY
) || { rm -rf -- "$temp"; return 1; }
  state_file=${LAYOUT_GATE_STATE_FILE:?gate state file}
  if ! research_exception_validate_bound; then evidence=$(gate_evidence_json "$units_lines" "$containers_lines" "$ps_status" "$ps_out_hash" "$ps_err_hash"); rm -rf -- "$temp"; gate_fail "$case_id" "$phase" "$previous" research-exception-invalid "$evidence"; return 1; fi
  if ! gate_state_check_and_write "$state_file" "$parsed_boot" "$LAYOUT_JOURNAL_SINCE_US" "$LAYOUT_JOURNAL_SINCE" "$units_json" "$containers_json"; then evidence=$(gate_evidence_json "$units_lines" "$containers_lines" "$ps_status" "$ps_out_hash" "$ps_err_hash"); rm -rf -- "$temp"; gate_fail "$case_id" "$phase" "$previous" gate-identity-changed "$evidence"; return 1; fi
  evidence=$(gate_evidence_json "$units_lines" "$containers_lines" "$ps_status" "$ps_out_hash" "$ps_err_hash") || { rm -rf -- "$temp"; return 1; }
  rm -rf -- "$temp"
  local reason=healthy
  [ "${LAYOUT_RESEARCH_BINDING:-null}" = null ] || reason=image-build-only-known-incident
  gate_record "$case_id" "$phase" "$previous" PASS "$reason" "$evidence"
}

probe_gate() {
  local case_id=${1:?case} phase=${2:?phase} previous=${3:?previous exit}
  if gate_test_allowed && [ "${LAYOUT_GATE_MODE:-real}" = fake ]; then
    gate_fake "$case_id" "$phase" "$previous"
  else
    gate_real "$case_id" "$phase" "$previous"
  fi
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  command=${1:-}
  case "$command" in
    tree-hash) shift; tree_hash "${1:?root}" ;;
    source-input-hash) shift; source_input_hash "${1:?root}" ;;
    identity-validate) shift; identity_validate "$@" ;;
    compile-input-hash) shift; compile_input_hash "${1:?root}" ;;
    compatibility-key) shift; compatibility_key "$@" ;;
    guard-inputs) shift; guard_inputs "$@" ;;
    artifact-write) shift; artifact_write "$@" ;;
    artifact-verify) shift; artifact_validate "$@" ;;
    artifact-publish) shift; artifact_publish "$@" ;;
    injected-export-failure) shift; injected_export_failure "$@" ;;
    ledger-plan) shift; ledger_plan "$@" ;;
    ledger-pending) shift; ledger_write "$@" true ;;
    ledger-complete) shift; ledger_write "$@" false ;;
    ledger-reset-target) shift; ledger_reset_target "$@" ;;
    cargo-summary) shift; parse_cargo_json "$@" ;;
    cargo-assert) shift; cargo_assert "$@" ;;
    build-log-mode) shift; build_log_mode "$@" ;;
    cargo-elapsed-ms) shift; cargo_elapsed_ms "$@" ;;
    *) helper_die 'unknown command'; exit 2 ;;
  esac
fi
