#!/bin/sh
# Common L1 production build layout controller.
#
# The file is POSIX-sh executable because the verified-artifacts stage runs
# these exact bytes with the Python standard library. Sourcing it from the
# Bash image-build controller exposes only the frozen host API.

RBL_FORMAT=lagrange-build-layout-v1
RBL_ARTIFACT_FORMAT=lagrange-rust-artifact-v1
RBL_BUNDLE_FORMAT=lagrange-rust-artifact-bundle-v1
RBL_ARCHIVE_REQUEST_FORMAT=lagrange-image-files-v1
RBL_ARCHIVE_RESULT_FORMAT=lagrange-image-files-result-v1
RBL_GUARD_VERSION=common-1
RBL_PLATFORM=linux/amd64
RBL_LOCK_PREFIX=/tmp/lagrange-production-image-build
RBL_RUNTIME_INVENTORY_FORMAT=lagrange-runtime-payload-inventory-v1
RBL_DOCKERIGNORE_SHA256=0b69fbfaf5417fdd0be8ef3bb4b6cf7eaebffb16a1883809e9fe21713adf863e

rbl_die() {
  printf '%s\n' "release-build-layout: $*" >&2
  return 1
}

rbl_require_command() {
  command -v "$1" >/dev/null 2>&1 || {
    rbl_die "required command is unavailable: $1"
    return 1
  }
}

rbl_commit_ok() {
  python3 - "$1" <<'PY'
import re, sys
value = sys.argv[1] if len(sys.argv) == 2 else ""
if not re.fullmatch(r"[0-9a-f]{40}", value) or set(value) == {"0"}:
    raise SystemExit(1)
PY
}

rbl_namespace_ok() {
  rbl_namespace_value=$1
  [ -n "$rbl_namespace_value" ] || return 1
  rbl_namespace_length=$(printf '%s' "$rbl_namespace_value" | wc -c)
  [ "$rbl_namespace_length" -le 48 ] || return 1
  case "$rbl_namespace_value" in
    [a-z0-9]*)
      case "$rbl_namespace_value" in *[!a-z0-9._-]*) return 1 ;; esac
      ;;
    *) return 1 ;;
  esac
}

rbl_abs_path_ok() {
  rbl_path=$1
  case "$rbl_path" in
    /*) : ;;
    *) return 1 ;;
  esac
  case "$rbl_path" in
    *'//'*) return 1 ;;
    */../*|*/..|*/./*|*/.) return 1 ;;
    */) return 1 ;;
    /|/etc|/opt|/usr|/usr/local|/var|/var/lib|/tmp|/run) return 1 ;;
  esac
  python3 - "$rbl_path" <<'PY'
import os, sys
path = sys.argv[1]
if any(ord(ch) < 32 or ord(ch) == 127 for ch in path):
    raise SystemExit(1)
if os.path.normpath(path) != path or os.path.realpath(path) != path:
    raise SystemExit(1)
PY
}

rbl_root() {
  if [ -n "${RELEASE_BUILD_LAYOUT_SOURCE_ROOT:-}" ]; then
    printf '%s\n' "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT"
    return 0
  fi
  git rev-parse --show-toplevel 2>/dev/null
}

rbl_config_path() {
  rbl_root_path=$(rbl_root) || return 1
  printf '%s\n' "$rbl_root_path/deploy/build/release-build-layout.json"
}

rbl_helper_path() {
  rbl_root_path=$(rbl_root) || return 1
  printf '%s\n' "$rbl_root_path/scripts/ops/lib/release-build-layout.sh"
}

rbl_sha256_file() {
  rbl_file=$1
  [ -f "$rbl_file" ] && [ ! -L "$rbl_file" ] || {
    rbl_die "not a regular file: $rbl_file"
    return 1
  }
  sha256sum -- "$rbl_file" | awk '{print $1}'
}

rbl_validate_layout() {
  rbl_layout=$1
  [ -f "$rbl_layout" ] && [ ! -L "$rbl_layout" ] || {
    rbl_die "layout config is missing or symlinked"
    return 1
  }
  python3 - "$rbl_layout" <<'PY'
import json, sys
def pairs(items):
    value = {}
    for key, item in items:
        if key in value: raise ValueError("duplicate-json-key")
        value[key] = item
    return value
def reject(value): raise ValueError("non-finite-json-number")
value = json.load(open(sys.argv[1], encoding="utf-8", newline=""),
                  object_pairs_hook=pairs, parse_constant=reject)
if not isinstance(value, dict): raise SystemExit("layout-not-object")
required = {"format","schema_version","layout","guard_version","platform","builder",
            "source_inventory","packages","runtime_payloads","recipes","services","batches","gates","archive"}
if set(value) != required: raise SystemExit("layout-keys-invalid")
if (value["format"], value["schema_version"], value["layout"],
    value["guard_version"], value["platform"]) != (
        "lagrange-build-layout-v1", 1, "common", "common-1", "linux/amd64"):
    raise SystemExit("layout-selection-invalid")
builder = value["builder"]
if not isinstance(builder, dict) or builder.get("profile") != "release" or builder.get("jobs") != "2":
    raise SystemExit("builder-contract-invalid")
if builder.get("target_dir") != "/cargo-target" or builder.get("workdir") != "/build":
    raise SystemExit("builder-path-contract-invalid")
if builder.get("features") != [] or builder.get("rustflags") is not None:
    raise SystemExit("builder-feature-contract-invalid")
if (not isinstance(builder.get("native_packages"), list) or
        builder["native_packages"] != sorted(set(builder["native_packages"]))):
    raise SystemExit("native-package-contract-invalid")
inventory = value["source_inventory"]
for field in ("root_files", "package_dirs", "external_compile_inputs"):
    if (not isinstance(inventory.get(field), list) or
            not inventory[field] or any(not isinstance(x, str) for x in inventory[field])):
        raise SystemExit("source-inventory-invalid")
if len(inventory["package_dirs"]) != 16 or len(set(inventory["package_dirs"])) != 16:
    raise SystemExit("workspace-member-inventory-invalid")
packages = value["packages"]
expected_packages = {
    "api-server","api-server-auth","auth","collectors","data-go-client","domain",
    "factor-engine","job-queue","kis-client","market-data","migration-contract",
    "opendart-client","portfolio-model","result-model","risk-gateway","selector"
}
if not isinstance(packages, dict) or set(packages) != expected_packages:
    raise SystemExit("package-inventory-keys-invalid")
if {record.get("path") for record in packages.values() if isinstance(record, dict)} != set(inventory["package_dirs"]):
    raise SystemExit("package-inventory-paths-invalid")
all_external = set()
for name, record in packages.items():
    if not isinstance(record, dict) or set(record) != {"path","external_compile_inputs","local_dependencies"}:
        raise SystemExit("package-record-keys-invalid")
    if (not isinstance(record["path"], str) or record["path"] not in inventory["package_dirs"] or
            not isinstance(record["external_compile_inputs"], list) or
            not isinstance(record["local_dependencies"], list) or
            len(record["external_compile_inputs"]) != len(set(record["external_compile_inputs"])) or
            record["local_dependencies"] != sorted(set(record["local_dependencies"])) or
            any(not isinstance(item, str) for item in record["external_compile_inputs"]) or
            any(item not in expected_packages or item == name for item in record["local_dependencies"])):
        raise SystemExit("package-record-invalid")
    all_external.update(record["external_compile_inputs"])
if all_external != set(inventory["external_compile_inputs"]):
    raise SystemExit("package-external-inventory-invalid")
# The pinned package graph is intentionally acyclic.  This is a declaration
# check; the builder later compares it with Cargo metadata using Cargo's own
# parser instead of a host TOML parser.
remaining = {name:set(record["local_dependencies"]) for name,record in packages.items()}
seen = set()
while remaining:
    ready = sorted(name for name,deps in remaining.items() if deps <= seen)
    if not ready: raise SystemExit("package-graph-cycle")
    for name in ready:
        seen.add(name); del remaining[name]
recipes = value["recipes"]
if set(recipes) != {"D1","D2","D3","D4","D5","D6","D7"}:
    raise SystemExit("recipe-set-invalid")
expected_recipe_payloads = {"D2":"backtest","D5":"backtest","D6":"collectors","D7":"paper"}
bin_count = 0
for recipe_id in sorted(recipes):
    recipe = recipes[recipe_id]
    required_recipe = {"dockerfile","package","bins","compile_commit","external_inputs",
                       "clean_packages","runtime_binaries","checkpoints"}
    optional_recipe = {"runtime_payload","backtest_commit_literal"}
    if (set(recipe) - (required_recipe | optional_recipe) or
            not required_recipe.issubset(recipe)):
        raise SystemExit("recipe-keys-invalid")
    if recipe["compile_commit"] not in ("unset", "present") or not recipe["bins"]:
        raise SystemExit("recipe-compile-contract-invalid")
    if len(recipe["bins"]) != len(set(recipe["bins"])):
        raise SystemExit("recipe-bin-duplicates")
    if len(recipe["runtime_binaries"]) != len(recipe["bins"]):
        raise SystemExit("runtime-bin-contract-invalid")
    if recipe["package"] not in packages or recipe["external_inputs"] != packages[recipe["package"]]["external_compile_inputs"]:
        raise SystemExit("recipe-package-input-contract-invalid")
    if recipe.get("runtime_payload") != expected_recipe_payloads.get(recipe_id):
        raise SystemExit("recipe-runtime-payload-contract-invalid")
    bin_count += len(recipe["bins"])
if bin_count != 17: raise SystemExit("production-bin-count-invalid")
services = value["services"]
expected_services = {
    "db-role-bootstrap","db-migrate","api-server","web","research-worker",
    "recommendation-runner","candidate-runner","owner-beta-runner",
    "owner-equity-v2-runner","nt-backtest-worker-1","nt-backtest-worker-2","paper-scheduler"
}
if set(services) != expected_services: raise SystemExit("service-set-invalid")
for service, record in services.items():
    if set(record) != {"kind", "recipe"} or record["kind"] not in ("database","web","rust"):
        raise SystemExit("service-record-invalid")
    if record["kind"] == "rust" and record["recipe"] not in recipes:
        raise SystemExit("service-recipe-invalid")
    if record["kind"] != "rust" and record["recipe"] is not None:
        raise SystemExit("nonproducer-recipe-invalid")
if [len(batch["services"]) for batch in value["batches"]] != [3,3,3,3]:
    raise SystemExit("batch-shape-invalid")
if (value["archive"].get("request_format") != "lagrange-image-files-v1" or
        value["archive"].get("result_format") != "lagrange-image-files-result-v1" or
        value["archive"].get("chunk_bytes") != 1048576):
    raise SystemExit("archive-format-invalid")
PY
}

rbl_clean_worktree() {
  rbl_root_path=$1
  rbl_status=$(git -c "safe.directory=$rbl_root_path" -C "$rbl_root_path" \
    status --porcelain=v1 --untracked-files=all 2>/dev/null) ||
    {
      rbl_die 'cannot inspect source worktree status'
      return 1
    }
  while IFS= read -r rbl_status_line; do
    [ -z "$rbl_status_line" ] && continue
    [ "$rbl_status_line" = '?? docs/kis_openapi_entiredocs_20260818_030007.xlsx' ] || {
      rbl_die 'source worktree is not clean (tracked or unapproved untracked changes present)'
      return 1
    }
  done <<EOF
$rbl_status
EOF
}

# Produce the one authoritative host-side runtime payload view. Git's index
# selects files without descending into ignored/untracked directories; every
# selected worktree ancestor, type, mode and byte hash is then measured. The
# matcher below intentionally implements only the frozen final security and
# generated-output deny rules, and the exact .dockerignore hash forces review
# before that policy may change.
rbl_runtime_payload_inventory() {
  rbl_runtime_source=$1
  rbl_runtime_layout=$2
  RBL_RUNTIME_SOURCE=$rbl_runtime_source RBL_RUNTIME_LAYOUT=$rbl_runtime_layout \
  RBL_RUNTIME_FORMAT=$RBL_RUNTIME_INVENTORY_FORMAT \
  RBL_DOCKERIGNORE_HASH=$RBL_DOCKERIGNORE_SHA256 python3 - <<'PY'
import hashlib, json, os, posixpath, re, stat, subprocess

root=os.path.abspath(os.environ["RBL_RUNTIME_SOURCE"])
layout_path=os.path.abspath(os.environ["RBL_RUNTIME_LAYOUT"])
expected_ignore=os.environ["RBL_DOCKERIGNORE_HASH"]

def pairs(items):
    value={}
    for key,item in items:
        if key in value: raise ValueError("duplicate-json-key")
        value[key]=item
    return value
def reject(value): raise ValueError("non-finite-json-number")
def sha(path):
    digest=hashlib.sha256()
    with open(path,"rb") as handle:
        for chunk in iter(lambda:handle.read(1024*1024),b""): digest.update(chunk)
    return digest.hexdigest()
def relative(value):
    if (not isinstance(value,str) or not value or value.startswith("/") or "\\" in value or
            any(ord(char)<32 or ord(char)==127 for char in value) or
            posixpath.normpath(value)!=value or value=="." or value.startswith("../") or
            "/../" in value or value.endswith("/..") or "//" in value):
        raise SystemExit("runtime-payload-path-invalid")
    return value
def image_path(value):
    if not isinstance(value,str) or not value.startswith("/") or value.endswith("/") or "\\" in value:
        raise SystemExit("runtime-payload-image-path-invalid")
    relative(value[1:])
    return value
def docker_excluded(value):
    parts=value.split("/")
    exact={".git",".worktrees","target",".venv","node_modules",".next",
           "__pycache__",".pytest_cache","credentials","secrets","raw"}
    for name in parts:
        if name in exact or name==".env" or name.startswith(".env.") or name.endswith(".env"):
            return True
        if re.search(r"\.py[cod]$",name) or name.endswith((".pem",".key",".p12",".pfx")):
            return True
    return False
def lstat(path,reason):
    try: return os.lstat(path)
    except FileNotFoundError: raise SystemExit(reason)
def require_ancestors(rel,terminal):
    parts=rel.split("/")
    for index in range(1,len(parts)+1):
        current="/".join(parts[:index])
        info=lstat(os.path.join(root,current),"runtime-payload-entry-missing")
        if index<len(parts):
            if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
                raise SystemExit("runtime-payload-ancestor-invalid")
        elif terminal=="file":
            if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
                raise SystemExit("runtime-payload-entry-invalid")
        elif stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
            raise SystemExit("runtime-payload-directory-invalid")

root_info=lstat(root,"runtime-payload-root-missing")
if (stat.S_ISLNK(root_info.st_mode) or not stat.S_ISDIR(root_info.st_mode) or
        os.path.realpath(root)!=root):
    raise SystemExit("runtime-payload-root-invalid")
layout_info=lstat(layout_path,"runtime-payload-layout-missing")
if stat.S_ISLNK(layout_info.st_mode) or not stat.S_ISREG(layout_info.st_mode):
    raise SystemExit("runtime-payload-layout-invalid")
ignore_path=os.path.join(root,".dockerignore")
ignore_info=lstat(ignore_path,"runtime-payload-dockerignore-missing")
if stat.S_ISLNK(ignore_info.st_mode) or not stat.S_ISREG(ignore_info.st_mode):
    raise SystemExit("runtime-payload-dockerignore-invalid")
ignore_hash=sha(ignore_path)
if ignore_hash!=expected_ignore: raise SystemExit("runtime-payload-dockerignore-policy-drift")

layout=json.load(open(layout_path,encoding="utf-8",newline=""),
                 object_pairs_hook=pairs,parse_constant=reject)
groups=layout.get("runtime_payloads")
if not isinstance(groups,dict) or set(groups)!={"backtest","collectors","database","paper"}:
    raise SystemExit("runtime-payload-groups-invalid")

result={}
for group in sorted(groups):
    values=groups[group]
    if not isinstance(values,list) or not values:
        raise SystemExit("runtime-payload-group-invalid")
    observed=[]
    seen_sources=set()
    for item in values:
        if not isinstance(item,dict) or set(item)!={"source","image"}:
            raise SystemExit("runtime-payload-record-invalid")
        source=relative(item["source"]); image=image_path(item["image"])
        if source in seen_sources: raise SystemExit("runtime-payload-source-duplicate")
        seen_sources.add(source)
        if docker_excluded(source): raise SystemExit("runtime-payload-docker-excluded")
        source_path=os.path.join(root,source)
        source_info=lstat(source_path,"runtime-payload-source-missing")
        if stat.S_ISLNK(source_info.st_mode) or not (stat.S_ISREG(source_info.st_mode) or stat.S_ISDIR(source_info.st_mode)):
            raise SystemExit("runtime-payload-source-invalid")
        raw=subprocess.run(
            ["git","-c","safe.directory="+root,"-C",root,"ls-files","--stage","-z","--",source],
            check=True,stdout=subprocess.PIPE).stdout
        tracked={}
        for row in raw.split(b"\0"):
            if not row: continue
            try:
                metadata,path_bytes=row.split(b"\t",1)
                mode,_object_id,stage=metadata.decode("ascii").split()
                path=path_bytes.decode("utf-8")
            except (ValueError,UnicodeDecodeError):
                raise SystemExit("runtime-payload-git-record-invalid")
            path=relative(path)
            if path in tracked or stage!="0" or mode not in ("100644","100755"):
                raise SystemExit("runtime-payload-tracked-type-invalid")
            if docker_excluded(path): raise SystemExit("runtime-payload-docker-excluded")
            tracked[path]=mode
        if not tracked: raise SystemExit("runtime-payload-untracked-or-empty")
        if stat.S_ISREG(source_info.st_mode):
            if set(tracked)!={source}: raise SystemExit("runtime-payload-file-selection-invalid")
        elif any(not path.startswith(source+"/") for path in tracked):
            raise SystemExit("runtime-payload-directory-selection-invalid")

        directories=set()
        if stat.S_ISDIR(source_info.st_mode): directories.add(source)
        for path in tracked:
            require_ancestors(path,"file")
            parent=posixpath.dirname(path)
            while parent and (parent==source or parent.startswith(source+"/")):
                directories.add(parent)
                parent=posixpath.dirname(parent)
        entries=[]
        for path in sorted(directories):
            require_ancestors(path,"directory")
            info=os.lstat(os.path.join(root,path))
            entries.append({"kind":"directory","mode":format(stat.S_IMODE(info.st_mode),"04o"),
                            "path":path,"sha256":None})
        for path in sorted(tracked):
            info=os.lstat(os.path.join(root,path))
            entries.append({"kind":"file","mode":format(stat.S_IMODE(info.st_mode),"04o"),
                            "path":path,"sha256":sha(os.path.join(root,path))})
        # Match the strict in-image walk: a directory and its descendants
        # precede the next sibling, including names such as config-old.
        entries.sort(key=lambda entry:entry["path"].split("/"))
        rows=[]
        for entry in entries:
            marker="d" if entry["kind"]=="directory" else "f"
            digest="-" if entry["sha256"] is None else entry["sha256"]
            rows.append(f'{marker}\t{entry["mode"]}\t{entry["path"]}\t{digest}\n'.encode())
        observed.append({"entries":entries,"image":image,"source":source,
                         "tree_sha256":hashlib.sha256(b"".join(rows)).hexdigest()})
    result[group]=observed
value={"dockerignore_sha256":ignore_hash,"format":os.environ["RBL_RUNTIME_FORMAT"],
       "payload_groups":result}
print(json.dumps(value,sort_keys=True,separators=(",",":")))
PY
}

rbl_runtime_payload_inventory_current() {
  rbl_runtime_current=$(rbl_runtime_payload_inventory \
    "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT" \
    "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT/deploy/build/release-build-layout.json") || return 1
  rbl_runtime_current_hash=$(printf '%s\n' "$rbl_runtime_current" | sha256sum | awk '{print $1}') || return 1
  [ "$rbl_runtime_current_hash" = "$RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256" ] || {
    rbl_die 'runtime payload inventory changed after initialization'
    return 1
  }
  printf '%s\n' "$rbl_runtime_current"
}

# Copy only entries selected by rbl_runtime_payload_inventory. No directory is
# enumerated here; each selected source is revalidated against its recorded
# type/mode/hash immediately before and after copying.
rbl_runtime_payload_copy() {
  rbl_runtime_copy_source=$1
  rbl_runtime_copy_destination=$2
  rbl_runtime_copy_group=$3
  rbl_runtime_copy_inventory=$4
  RBL_RUNTIME_COPY_SOURCE=$rbl_runtime_copy_source \
  RBL_RUNTIME_COPY_DESTINATION=$rbl_runtime_copy_destination \
  RBL_RUNTIME_COPY_GROUP=$rbl_runtime_copy_group \
  RBL_RUNTIME_COPY_INVENTORY=$rbl_runtime_copy_inventory \
  RBL_RUNTIME_FORMAT=$RBL_RUNTIME_INVENTORY_FORMAT \
  RBL_DOCKERIGNORE_HASH=$RBL_DOCKERIGNORE_SHA256 python3 - <<'PY'
import hashlib,json,os,shutil,stat

source=os.path.abspath(os.environ["RBL_RUNTIME_COPY_SOURCE"])
destination=os.path.abspath(os.environ["RBL_RUNTIME_COPY_DESTINATION"])
group=os.environ["RBL_RUNTIME_COPY_GROUP"]
inventory=json.loads(os.environ["RBL_RUNTIME_COPY_INVENTORY"])
if (inventory.get("format")!=os.environ["RBL_RUNTIME_FORMAT"] or
        inventory.get("dockerignore_sha256")!=os.environ["RBL_DOCKERIGNORE_HASH"]):
    raise SystemExit("runtime-copy-inventory-invalid")
groups=inventory.get("payload_groups")
if not isinstance(groups,dict): raise SystemExit("runtime-copy-groups-invalid")
selected_payloads=[] if not group else groups.get(group)
if not isinstance(selected_payloads,list): raise SystemExit("runtime-copy-group-invalid")
if not os.path.isdir(source) or os.path.islink(source) or not os.path.isdir(destination) or os.path.islink(destination):
    raise SystemExit("runtime-copy-root-invalid")
def sha(path):
    digest=hashlib.sha256()
    with open(path,"rb") as handle:
        for chunk in iter(lambda:handle.read(1024*1024),b""): digest.update(chunk)
    return digest.hexdigest()
def under(root,path):
    return os.path.commonpath((root,os.path.abspath(path)))==root
def selected_path(root,rel):
    path=os.path.join(root,rel)
    if not under(root,path): raise SystemExit("runtime-copy-path-escape")
    return path
def verify(entry):
    path=selected_path(source,entry["path"])
    try: info=os.lstat(path)
    except FileNotFoundError: raise SystemExit("runtime-copy-source-missing")
    mode=format(stat.S_IMODE(info.st_mode),"04o")
    if mode!=entry["mode"]: raise SystemExit("runtime-copy-source-mode-changed")
    if entry["kind"]=="directory":
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode) or entry["sha256"] is not None:
            raise SystemExit("runtime-copy-source-directory-invalid")
    elif entry["kind"]=="file":
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode) or sha(path)!=entry["sha256"]:
            raise SystemExit("runtime-copy-source-file-invalid")
    else: raise SystemExit("runtime-copy-entry-kind-invalid")
    return path,info
payloads=[]
for payload in selected_payloads:
    if not isinstance(payload,dict) or set(payload)!={"entries","image","source","tree_sha256"}:
        raise SystemExit("runtime-copy-payload-invalid")
    entries=payload["entries"]
    if not isinstance(entries,list) or not entries: raise SystemExit("runtime-copy-entries-invalid")
    directories=[entry for entry in entries if entry.get("kind")=="directory"]
    files=[entry for entry in entries if entry.get("kind")=="file"]
    for entry in sorted(directories,key=lambda value:(value["path"].count("/"),value["path"])):
        src,info=verify(entry); dst=selected_path(destination,entry["path"])
        if os.path.lexists(dst):
            current=os.lstat(dst)
            if stat.S_ISLNK(current.st_mode) or not stat.S_ISDIR(current.st_mode):
                raise SystemExit("runtime-copy-destination-conflict")
        else: os.makedirs(dst,exist_ok=False)
        os.chmod(dst,stat.S_IMODE(info.st_mode))
    for entry in sorted(files,key=lambda value:value["path"]):
        src,before=verify(entry); dst=selected_path(destination,entry["path"])
        os.makedirs(os.path.dirname(dst),exist_ok=True)
        if os.path.lexists(dst): raise SystemExit("runtime-copy-destination-conflict")
        shutil.copyfile(src,dst)
        os.chmod(dst,stat.S_IMODE(before.st_mode))
        os.utime(dst,ns=(before.st_atime_ns,before.st_mtime_ns))
        _src,after=verify(entry)
        if ((before.st_dev,before.st_ino,before.st_size,before.st_mtime_ns,before.st_ctime_ns)!=
                (after.st_dev,after.st_ino,after.st_size,after.st_mtime_ns,after.st_ctime_ns) or
                sha(dst)!=entry["sha256"] or
                format(stat.S_IMODE(os.lstat(dst).st_mode),"04o")!=entry["mode"]):
            raise SystemExit("runtime-copy-source-changed")
    for entry in sorted(directories,key=lambda value:(-value["path"].count("/"),value["path"])):
        src,info=verify(entry); dst=selected_path(destination,entry["path"])
        os.chmod(dst,stat.S_IMODE(info.st_mode)); os.utime(dst,ns=(info.st_atime_ns,info.st_mtime_ns))
    payloads.append({"source":payload["source"],"image":payload["image"],
                     "tree_sha256":payload["tree_sha256"]})
print(json.dumps(payloads,sort_keys=True,separators=(",",":")))
PY
}

# Compare an actual bundle payload tree with the selected source inventory.
# Expected paths come from Git, but actual payload roots are walked in full so
# no unexpected regular file, directory, symlink or special entry is filtered.
rbl_runtime_payload_verify_copy() {
  rbl_runtime_verify_bundle=$1
  rbl_runtime_verify_group=$2
  rbl_runtime_verify_inventory=$3
  RBL_RUNTIME_VERIFY_BUNDLE=$rbl_runtime_verify_bundle \
  RBL_RUNTIME_VERIFY_GROUP=$rbl_runtime_verify_group \
  RBL_RUNTIME_VERIFY_INVENTORY=$rbl_runtime_verify_inventory \
  RBL_RUNTIME_FORMAT=$RBL_RUNTIME_INVENTORY_FORMAT \
  RBL_DOCKERIGNORE_HASH=$RBL_DOCKERIGNORE_SHA256 python3 - <<'PY'
import hashlib,json,os,stat

bundle=os.path.abspath(os.environ["RBL_RUNTIME_VERIFY_BUNDLE"])
group=os.environ["RBL_RUNTIME_VERIFY_GROUP"]
inventory=json.loads(os.environ["RBL_RUNTIME_VERIFY_INVENTORY"])
if (inventory.get("format")!=os.environ["RBL_RUNTIME_FORMAT"] or
        inventory.get("dockerignore_sha256")!=os.environ["RBL_DOCKERIGNORE_HASH"]):
    raise SystemExit("runtime-verify-inventory-invalid")
groups=inventory.get("payload_groups")
if not isinstance(groups,dict): raise SystemExit("runtime-verify-groups-invalid")
payloads=[] if not group else groups.get(group)
if not isinstance(payloads,list): raise SystemExit("runtime-verify-group-invalid")
if not os.path.isdir(bundle) or os.path.islink(bundle): raise SystemExit("runtime-verify-bundle-invalid")
def sha(path):
    digest=hashlib.sha256()
    with open(path,"rb") as handle:
        for chunk in iter(lambda:handle.read(1024*1024),b""): digest.update(chunk)
    return digest.hexdigest()
def path(rel):
    candidate=os.path.abspath(os.path.join(bundle,rel))
    if os.path.commonpath((bundle,candidate))!=bundle: raise SystemExit("runtime-verify-path-escape")
    return candidate
expected={}
payload_roots=set()
for payload in payloads:
    if not isinstance(payload,dict) or set(payload)!={"entries","image","source","tree_sha256"}:
        raise SystemExit("runtime-verify-payload-invalid")
    payload_roots.add(payload["source"].split("/",1)[0])
    for entry in payload["entries"]:
        old=expected.get(entry["path"])
        if old is not None and old!=entry: raise SystemExit("runtime-verify-entry-conflict")
        expected[entry["path"]]=entry
        parent=os.path.dirname(entry["path"])
        while parent and parent!=".":
            expected.setdefault(parent,{"kind":"ancestor","mode":None,"path":parent,"sha256":None})
            parent=os.path.dirname(parent)
for root in sorted(payload_roots):
    root_path=path(root)
    try: root_info=os.lstat(root_path)
    except FileNotFoundError: raise SystemExit("runtime-verify-payload-missing")
    if stat.S_ISLNK(root_info.st_mode) or not stat.S_ISDIR(root_info.st_mode):
        raise SystemExit("runtime-verify-root-invalid")
    for current,dirs,files in os.walk(root_path,topdown=True,followlinks=False):
        dirs.sort(); files.sort()
        current_rel=os.path.relpath(current,bundle).replace(os.sep,"/")
        if current_rel not in expected: raise SystemExit("runtime-verify-extra-entry")
        for name in dirs:
            candidate=os.path.join(current,name); rel=os.path.relpath(candidate,bundle).replace(os.sep,"/")
            info=os.lstat(candidate)
            if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
                raise SystemExit("runtime-verify-directory-invalid")
            if rel not in expected: raise SystemExit("runtime-verify-extra-entry")
        for name in files:
            candidate=os.path.join(current,name); rel=os.path.relpath(candidate,bundle).replace(os.sep,"/")
            info=os.lstat(candidate)
            if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
                raise SystemExit("runtime-verify-file-invalid")
            if rel not in expected: raise SystemExit("runtime-verify-extra-entry")
for rel,entry in sorted(expected.items()):
    candidate=path(rel)
    try: info=os.lstat(candidate)
    except FileNotFoundError: raise SystemExit("runtime-verify-entry-missing")
    if entry["kind"] in ("directory","ancestor"):
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
            raise SystemExit("runtime-verify-directory-invalid")
        if entry["kind"]=="directory" and format(stat.S_IMODE(info.st_mode),"04o")!=entry["mode"]:
            raise SystemExit("runtime-verify-mode-mismatch")
    elif entry["kind"]=="file":
        if (stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode) or
                format(stat.S_IMODE(info.st_mode),"04o")!=entry["mode"] or sha(candidate)!=entry["sha256"]):
            raise SystemExit("runtime-verify-file-mismatch")
    else: raise SystemExit("runtime-verify-kind-invalid")
def digest(candidate,rel):
    info=os.lstat(candidate)
    if stat.S_ISLNK(info.st_mode): raise SystemExit("runtime-verify-symlink")
    if stat.S_ISDIR(info.st_mode):
        rows=[f"d\t{stat.S_IMODE(info.st_mode):04o}\t{rel}\t-\n".encode()]
        for child in sorted(os.listdir(candidate)):
            rows+=digest(os.path.join(candidate,child),rel+"/"+child)
        return rows
    if not stat.S_ISREG(info.st_mode): raise SystemExit("runtime-verify-special")
    return [f"f\t{stat.S_IMODE(info.st_mode):04o}\t{rel}\t{sha(candidate)}\n".encode()]
result=[]
for payload in payloads:
    actual=hashlib.sha256(b"".join(digest(path(payload["source"]),payload["source"]))).hexdigest()
    if actual!=payload["tree_sha256"]: raise SystemExit("runtime-verify-tree-mismatch")
    result.append({"source":payload["source"],"image":payload["image"],"tree_sha256":actual})
print(json.dumps(result,sort_keys=True,separators=(",",":")))
PY
}

rbl_hash_source() {
  rbl_source=$1
  rbl_layout=$2
  if [ "$#" -ge 3 ]; then rbl_use_git=$3; else rbl_use_git=0; fi
  python3 - "$rbl_source" "$rbl_layout" "$rbl_use_git" <<'PY'
import hashlib, json, os, stat, subprocess, sys
root, layout_path, use_git = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2]), sys.argv[3] == "1"
def pairs(items):
    value = {}
    for key, item in items:
        if key in value: raise ValueError("duplicate-json-key")
        value[key] = item
    return value
layout = json.load(open(layout_path, encoding="utf-8"), object_pairs_hook=pairs)
inventory = layout["source_inventory"]
specs = inventory["root_files"] + inventory["package_dirs"] + inventory["external_compile_inputs"]
selected = set()
def reject_rel(rel):
    if (not rel or rel.startswith("/") or os.path.normpath(rel) != rel or
            rel == "." or rel.startswith("../")):
        raise SystemExit("source-path-invalid")
    if (rel == ".git" or rel.startswith(".git/") or rel == ".env" or
            rel.startswith(".env/") or rel == ".cargo" or rel.startswith(".cargo/")):
        raise SystemExit("forbidden-source-entry")
def tracked(spec):
    if use_git:
        proc = subprocess.run(
            ["git", "-c", "safe.directory=" + root, "-C", root, "ls-files", "-z", "--", spec],
            check=True, stdout=subprocess.PIPE)
        paths = [item.decode("utf-8") for item in proc.stdout.split(b"\0") if item]
    else:
        base = os.path.join(root, spec)
        paths = []
        if os.path.isfile(base) and not os.path.islink(base):
            paths = [spec]
        elif os.path.isdir(base) and not os.path.islink(base):
            for current, dirs, files in os.walk(base, topdown=True, followlinks=False):
                dirs.sort(); files.sort()
                for name in files:
                    paths.append(os.path.relpath(os.path.join(current, name), root))
                for name in dirs:
                    if os.path.islink(os.path.join(current, name)):
                        raise SystemExit("source-directory-symlink")
        else:
            raise SystemExit("source-entry-missing")
    if not paths: raise SystemExit("source-entry-untracked-or-empty")
    return paths
for spec in specs:
    reject_rel(spec)
    for rel in tracked(spec):
        reject_rel(rel)
        selected.add(rel)
for rel in list(selected):
    parent = os.path.dirname(rel)
    while parent and parent != ".":
        selected.add(parent)
        parent = os.path.dirname(parent)
records = []
for rel in sorted(selected):
    path = os.path.join(root, rel)
    info = os.lstat(path)
    mode = stat.S_IMODE(info.st_mode)
    if stat.S_ISLNK(info.st_mode): raise SystemExit("source-symlink")
    if stat.S_ISDIR(info.st_mode):
        records.append(f"d\t{mode:04o}\t{rel}\t-\n".encode())
    elif stat.S_ISREG(info.st_mode):
        digest = hashlib.sha256()
        with open(path, "rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        records.append(f"f\t{mode:04o}\t{rel}\t{digest.hexdigest()}\n".encode())
    else:
        raise SystemExit("source-special-file")
print(hashlib.sha256(b"".join(records)).hexdigest())
PY
}

# P is deliberately per-package rather than a coarse source-tree hash.  It
# records the package tree and only that package's frozen external compile
# inputs, including type/mode/content and declared-path presence but never
# mtime.  The host walks Git-tracked inputs; the copied compiler context has
# no Git metadata and is remeasured from its exact validated filesystem tree.
rbl_package_hashes() {
  rbl_source=$1
  rbl_layout=$2
  if [ "$#" -ge 3 ]; then rbl_use_git=$3; else rbl_use_git=0; fi
  python3 - "$rbl_source" "$rbl_layout" "$rbl_use_git" <<'PY'
import hashlib, json, os, stat, subprocess, sys
root=os.path.abspath(sys.argv[1])
layout_path=os.path.abspath(sys.argv[2])
use_git=sys.argv[3] == "1"
def pairs(items):
    value={}
    for key,item in items:
        if key in value: raise ValueError("duplicate-json-key")
        value[key]=item
    return value
def reject(value): raise ValueError("non-finite-json-number")
layout=json.load(open(layout_path,encoding="utf-8",newline=""),object_pairs_hook=pairs,parse_constant=reject)
packages=layout.get("packages")
if not isinstance(packages,dict): raise SystemExit("package-layout-missing")
def relative(value):
    if (not isinstance(value,str) or not value or value.startswith("/") or "\\" in value or
            os.path.normpath(value)!=value or value=="." or value.startswith("../") or
            "/../" in value or value.endswith("/..") or value==".git" or value.startswith(".git/")):
        raise SystemExit("package-input-path-invalid")
    return value
def tracked(spec):
    spec=relative(spec)
    base=os.path.join(root,spec)
    if not os.path.lexists(base): raise SystemExit("package-input-missing")
    info=os.lstat(base)
    if stat.S_ISLNK(info.st_mode) or not (stat.S_ISREG(info.st_mode) or stat.S_ISDIR(info.st_mode)):
        raise SystemExit("package-input-special")
    if use_git:
        proc=subprocess.run(["git","-c","safe.directory="+root,"-C",root,"ls-files","-z","--",spec],
                            check=True,stdout=subprocess.PIPE)
        values=[item.decode("utf-8") for item in proc.stdout.split(b"\0") if item]
        if not values: raise SystemExit("package-input-untracked-or-empty")
        for value in values:
            relative(value)
        return values
    if stat.S_ISREG(info.st_mode): return [spec]
    values=[]
    for current,dirs,files in os.walk(base,topdown=True,followlinks=False):
        dirs.sort(); files.sort()
        for name in dirs:
            path=os.path.join(current,name)
            if os.path.islink(path): raise SystemExit("package-directory-symlink")
        for name in files:
            path=os.path.join(current,name)
            if os.path.islink(path): raise SystemExit("package-file-symlink")
            if not os.path.isfile(path): raise SystemExit("package-file-special")
            values.append(os.path.relpath(path,root))
    if not values: raise SystemExit("package-input-empty")
    return values
def record_path(rel,domain,records):
    rel=relative(rel)
    path=os.path.join(root,rel)
    info=os.lstat(path)
    mode=stat.S_IMODE(info.st_mode)
    if stat.S_ISLNK(info.st_mode): raise SystemExit("package-entry-symlink")
    if stat.S_ISDIR(info.st_mode):
        records.append(f"{domain}\td\t{mode:04o}\t{rel}\t-\n".encode())
    elif stat.S_ISREG(info.st_mode):
        digest=hashlib.sha256()
        with open(path,"rb") as handle:
            for chunk in iter(lambda:handle.read(1024*1024),b""): digest.update(chunk)
        records.append(f"{domain}\tf\t{mode:04o}\t{rel}\t{digest.hexdigest()}\n".encode())
    else: raise SystemExit("package-entry-special")
def selected_records(spec,domain):
    values=set(tracked(spec))
    values.add(relative(spec))
    for rel in list(values):
        parent=os.path.dirname(rel)
        limit=relative(spec)
        while parent and parent != ".":
            values.add(parent)
            if parent == limit: break
            parent=os.path.dirname(parent)
    records=[]
    for rel in sorted(values): record_path(rel,domain,records)
    return records
result={}
for name in sorted(packages):
    record=packages[name]
    if not isinstance(record,dict): raise SystemExit("package-record-invalid")
    rows=[]
    rows += selected_records(record["path"],"package")
    for external in record["external_compile_inputs"]:
        rows += selected_records(external,"external")
    # Several declared files share parent directories.  A parent mode is one
    # physical input record, not a duplicate requirement; layout bytes already
    # bind the declared input list through K/H.
    material=(f"lagrange-build-layout-p-v1\t{name}\n".encode()+b"".join(sorted(set(rows))))
    result[name]=hashlib.sha256(material).hexdigest()
print(json.dumps(result,sort_keys=True,separators=(",",":")))
PY
}

rbl_tree_hash() {
  rbl_tree=$1
  python3 - "$rbl_tree" <<'PY'
import hashlib, os, stat, sys
root = os.path.abspath(sys.argv[1])
if not os.path.isdir(root) or os.path.islink(root): raise SystemExit("tree-root-invalid")
records = []
for current, dirs, files in os.walk(root, topdown=True, followlinks=False):
    dirs.sort(); files.sort()
    kept = []
    for name in dirs:
        path = os.path.join(current, name); info = os.lstat(path)
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
            raise SystemExit("tree-directory-invalid")
        kept.append(name)
        records.append(f"d\t{stat.S_IMODE(info.st_mode):04o}\t{os.path.relpath(path, root)}\t-\n".encode())
    dirs[:] = kept
    for name in files:
        path = os.path.join(current, name); info = os.lstat(path)
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
            raise SystemExit("tree-file-invalid")
        digest = hashlib.sha256()
        with open(path, "rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        records.append(f"f\t{stat.S_IMODE(info.st_mode):04o}\t{os.path.relpath(path, root)}\t{digest.hexdigest()}\n".encode())
records.sort()
print(hashlib.sha256(b"".join(records)).hexdigest())
PY
}

rbl_recipe_json() {
  rbl_recipe=$1
  rbl_layout=$2
  python3 - "$rbl_layout" "$rbl_recipe" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
recipe = value["recipes"].get(sys.argv[2])
if recipe is None: raise SystemExit("unknown-recipe")
print(json.dumps(recipe, sort_keys=True, separators=(",", ":")))
PY
}

rbl_recipe_hash() {
  rbl_root_path=$1
  rbl_recipe=$2
  rbl_layout=$3
  rbl_dockerfile=$rbl_root_path/$(python3 - "$rbl_layout" "$rbl_recipe" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
print(value["recipes"][sys.argv[2]]["dockerfile"])
PY
)
  rbl_recipe_json_value=$(rbl_recipe_json "$rbl_recipe" "$rbl_layout") || return 1
  RBL_RECIPE_JSON=$rbl_recipe_json_value RBL_RECIPE_DOCKERFILE=$rbl_dockerfile python3 - <<'PY'
import hashlib, os
digest = hashlib.sha256()
with open(os.environ["RBL_RECIPE_DOCKERFILE"], "rb") as handle:
    for chunk in iter(lambda: handle.read(1024 * 1024), b""):
        digest.update(chunk)
digest.update(b"\n")
digest.update(os.environ["RBL_RECIPE_JSON"].encode("utf-8"))
print(digest.hexdigest())
PY
}

rbl_validate_native_identity() {
  rbl_identity_path=$1
  [ -f "$rbl_identity_path" ] && [ ! -L "$rbl_identity_path" ] ||
    {
      rbl_die 'native identity is not a regular file'
      return 1
    }
  RBL_NATIVE_IDENTITY=$rbl_identity_path python3 - <<'PY'
import json, os, re, stat
path=os.path.abspath(os.environ["RBL_NATIVE_IDENTITY"])
info=os.lstat(path)
if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o600:
    raise SystemExit("native-identity-permission-invalid")
def pairs(items):
    value={}
    for key,item in items:
        if key in value: raise ValueError("duplicate-json-key")
        value[key]=item
    return value
def reject(value): raise ValueError("non-finite-json-number")
value=json.load(open(path,encoding="utf-8",newline=""),object_pairs_hook=pairs,parse_constant=reject)
expected={"format","target_platform","host_triple","rustc_vv","cargo_version",
          "apk_info_vv","native_packages","compiler_env"}
if not isinstance(value,dict) or set(value) != expected:
    raise SystemExit("native-identity-schema-invalid")
if value["format"] != "lagrange-build-layout-native-v2" or value["target_platform"] != "linux/amd64":
    raise SystemExit("native-identity-binding-invalid")
rustc=value["rustc_vv"]
if not isinstance(rustc,str) or not rustc.endswith("\n"):
    raise SystemExit("native-rustc-output-invalid")
hosts=re.findall(r"^host:\s*(\S+)\s*$",rustc,re.MULTILINE)
if hosts != ["x86_64-unknown-linux-musl"]:
    raise SystemExit("native-host-triple-invalid")
if value["host_triple"] != hosts[0]:
    raise SystemExit("native-host-binding-invalid")
if not isinstance(value["cargo_version"],str) or not re.fullmatch(r"cargo \S+ \([^\n]+\)",value["cargo_version"]):
    raise SystemExit("native-cargo-version-invalid")
if not isinstance(value["apk_info_vv"],str) or not value["apk_info_vv"]:
    raise SystemExit("native-apk-inventory-invalid")
packages=value["native_packages"]
fixed=["build-base","musl-dev","openssl-dev","pkgconf","postgresql-dev"]
if packages != fixed:
    raise SystemExit("native-package-set-invalid")
for package in fixed:
    if not re.search(r"(?:^|[ \t\n])"+re.escape(package)+r"(?:[- \t\n]|$)",value["apk_info_vv"]):
        raise SystemExit("native-package-inventory-missing")
if value["compiler_env"] != {"CARGO_BUILD_JOBS":"2","CARGO_TARGET_DIR":"/cargo-target","RUSTFLAGS":"<unset>"}:
    raise SystemExit("native-compiler-env-invalid")
PY
}

rbl_k_hash() {
  rbl_root_path=$1
  rbl_recipe=$2
  rbl_layout=$3
  rbl_common_dockerfile=$rbl_root_path/deploy/build/Dockerfile.rust-artifacts
  rbl_native_identity=$RELEASE_BUILD_LAYOUT_STATE_ROOT/native-identity.json
  [ -f "$rbl_common_dockerfile" ] && [ ! -L "$rbl_common_dockerfile" ] || return 1
  if [ -f "$rbl_native_identity" ] && [ ! -L "$rbl_native_identity" ]; then
    rbl_validate_native_identity "$rbl_native_identity" || return 1
  fi
  RBL_ROOT=$rbl_root_path RBL_LAYOUT=$rbl_layout RBL_DOCKERFILE=$rbl_common_dockerfile \
  RBL_NATIVE_IDENTITY=$rbl_native_identity RBL_LAYOUT_HASH=$RELEASE_BUILD_LAYOUT_CONFIG_SHA256 \
  RBL_HELPER_HASH=$RELEASE_BUILD_LAYOUT_HELPER_SHA256 python3 - <<'PY'
import hashlib, json, os, stat
root=os.path.abspath(os.environ["RBL_ROOT"])
layout=json.load(open(os.environ["RBL_LAYOUT"],encoding="utf-8"))
dockerfile=os.path.abspath(os.environ["RBL_DOCKERFILE"])
records={}
for rel in ["Cargo.toml","Cargo.lock","rust-toolchain.toml"]:
    path=os.path.join(root,rel); info=os.lstat(path)
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode): raise SystemExit("k-input-missing")
    records[rel]={"mode":format(stat.S_IMODE(info.st_mode),"04o"),"sha256":hashlib.sha256(open(path,"rb").read()).hexdigest()}
for rel in layout["source_inventory"]["package_dirs"]:
    path=os.path.join(root,rel,"Cargo.toml")
    if os.path.isfile(path) and not os.path.islink(path):
        info=os.lstat(path)
        records[rel+"/Cargo.toml"]={"mode":format(stat.S_IMODE(info.st_mode),"04o"),"sha256":hashlib.sha256(open(path,"rb").read()).hexdigest()}
value={
  "format":"lagrange-build-layout-k-v2","platform":layout["platform"],
  "builder":{key:layout["builder"][key] for key in ("rust_image","host_triple","target_dir","workdir","profile","jobs","features","rustflags","native_packages","build_python")},
  "manifest_inputs":records,
  "common_artifact_dockerfile_sha256":hashlib.sha256(open(dockerfile,"rb").read()).hexdigest(),
  "helper_sha256":os.environ["RBL_HELPER_HASH"],"layout_schema_sha256":os.environ["RBL_LAYOUT_HASH"],
  "paths":{"workdir":"/build","target_dir":"/cargo-target"},
  "compiler_env":{"CARGO_BUILD_JOBS":"2","CARGO_TARGET_DIR":"/cargo-target","RUSTFLAGS":"<unset>"},"guard_version":"common-1"
}
identity=os.environ["RBL_NATIVE_IDENTITY"]
if os.path.isfile(identity) and not os.path.islink(identity):
    info=os.lstat(identity)
    if stat.S_IMODE(info.st_mode)!=0o600 or info.st_uid!=os.geteuid(): raise SystemExit("native-identity-permission-invalid")
    raw=open(identity,"rb").read()
    native=json.loads(raw.decode("utf-8"))
    if native.get("format")!="lagrange-build-layout-native-v2" or native.get("target_platform")!="linux/amd64" or native.get("host_triple")!="x86_64-unknown-linux-musl":
        raise SystemExit("native-identity-contract-invalid")
    if native.get("compiler_env") != value["compiler_env"]: raise SystemExit("native-identity-env-invalid")
    value["native_identity_sha256"]=hashlib.sha256(raw).hexdigest()
else:
    value["native_identity_sha256"]="P0-pending"
print(hashlib.sha256(json.dumps(value,sort_keys=True,separators=(",",":")).encode()).hexdigest())
PY
}

rbl_service_record() {
  rbl_service=$1
  rbl_layout=$2
  python3 - "$rbl_layout" "$rbl_service" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
record = value["services"].get(sys.argv[2])
if record is None: raise SystemExit("unknown-service")
print(json.dumps(record, sort_keys=True, separators=(",", ":")))
PY
}

rbl_recipe_for_service() {
  rbl_service_record "$1" "$2" | python3 -c 'import json,sys; print(json.load(sys.stdin)["recipe"] or "NONE")'
}

rbl_assert_root_commit() {
  rbl_root_path=$1
  rbl_commit=$2
  rbl_commit_ok "$rbl_commit" || {
    rbl_die 'commit is not exact lowercase 40-hex'
    return 1
  }
  rbl_head=$(git -c "safe.directory=$rbl_root_path" -C "$rbl_root_path" \
    rev-parse --verify 'HEAD^{commit}' 2>/dev/null) ||
    {
      rbl_die 'source root is not a Git worktree with a commit'
      return 1
    }
  [ "$rbl_head" = "$rbl_commit" ] || {
    rbl_die 'commit does not match source root HEAD'
    return 1
  }
  rbl_clean_worktree "$rbl_root_path" || return 1
}

release_build_layout_plan() {
  rbl_commit=$1
  rbl_root_path=$(rbl_root) || {
    rbl_die 'cannot resolve source root'
    return 1
  }
  rbl_assert_root_commit "$rbl_root_path" "$rbl_commit" || return 1
  rbl_layout=$rbl_root_path/deploy/build/release-build-layout.json
  rbl_validate_layout "$rbl_layout" || return 1
  rbl_runtime_inventory=$(rbl_runtime_payload_inventory "$rbl_root_path" "$rbl_layout") || return 1
  rbl_runtime_inventory_hash=$(printf '%s\n' "$rbl_runtime_inventory" | sha256sum | awk '{print $1}') || return 1
  rbl_source_hash=$(rbl_hash_source "$rbl_root_path" "$rbl_layout" 1) || return 1
  rbl_helper_hash=$(rbl_sha256_file "$(rbl_helper_path)") || return 1
  rbl_layout_hash=$(rbl_sha256_file "$rbl_layout") || return 1
  printf 'RELEASE_BUILD_LAYOUT_PLAN format=%s layout=common platform=%s\n' "$RBL_FORMAT" "$RBL_PLATFORM"
  printf '  source_root=%s commit=%s source_input_sha256=%s\n' "$rbl_root_path" "$rbl_commit" "$rbl_source_hash"
  printf '  helper_sha256=%s layout_sha256=%s cache_namespace=product\n' "$rbl_helper_hash" "$rbl_layout_hash"
  printf '  runtime_payload_inventory_sha256=%s dockerignore_sha256=%s\n' \
    "$rbl_runtime_inventory_hash" "$RBL_DOCKERIGNORE_SHA256"
  printf '%s\n' '  producer_order=D1:api-server,D6:collectors[3+3+2+2],D2:job-queue,D3:owner-beta,D4:owner-equity-v2,D5:backtest,D7:paper'
  printf '%s\n' '  consumer_batches=B1[db-role-bootstrap,db-migrate,api-server],B2[web,research-worker,recommendation-runner],B3[candidate-runner,owner-beta-runner,owner-equity-v2-runner],B4[nt-backtest-worker-1,nt-backtest-worker-2,paper-scheduler]'
  printf '%s\n' '  gate_order=prepare,native,producer-after-each-bin,collector-3+3+2+2,consumer-B1+B2+B3+B4,final-bytes'
  printf '%s\n' 'PLAN_ONLY: no Docker, state mutation, archive save, container lifecycle, provider call, or product binary execution'
}

rbl_private_dir() {
  rbl_dir=$1
  if [ -e "$rbl_dir" ] || [ -L "$rbl_dir" ]; then
    [ -d "$rbl_dir" ] && [ ! -L "$rbl_dir" ] || {
      rbl_die "private path is not a directory: $rbl_dir"
      return 1
    }
    [ "$(stat -c '%u:%a' -- "$rbl_dir")" = "$(id -u):700" ] ||
      {
        rbl_die "private directory ownership/mode is unsafe: $rbl_dir"
        return 1
      }
  else
    mkdir -m 0700 -- "$rbl_dir" || {
      rbl_die "cannot create private directory: $rbl_dir"
      return 1
    }
    [ "$(stat -c '%u:%a' -- "$rbl_dir")" = "$(id -u):700" ] ||
      {
        rbl_die "new private directory ownership/mode is unsafe: $rbl_dir"
        return 1
      }
  fi
}

release_build_layout_lock() {
  if [ "${RELEASE_BUILD_LAYOUT_LOCK_HELD:-0}" = 1 ]; then
    rbl_uid=$(id -u)
    rbl_expected_lock=$RBL_LOCK_PREFIX-$rbl_uid
    rbl_lock_dir=${RELEASE_BUILD_LAYOUT_LOCK_DIR:-}
    [ "$rbl_lock_dir" = "$rbl_expected_lock" ] ||
      {
        rbl_die 'inherited lock directory is not the canonical uid path'
        return 1
      }
    python3 - "$rbl_expected_lock" <<'PY'
import fcntl, os, stat, sys
path = sys.argv[1]
info = os.lstat(path)
fdinfo = os.fstat(9)
fd_path = os.readlink("/proc/self/fd/9")
if (not stat.S_ISDIR(info.st_mode) or stat.S_ISLNK(info.st_mode) or
        os.path.abspath(fd_path) != os.path.abspath(path) or
        (info.st_dev, info.st_ino) != (fdinfo.st_dev, fdinfo.st_ino) or
        info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o700):
    raise SystemExit("held-lock-identity-invalid")
try:
    fcntl.flock(9, fcntl.LOCK_EX | fcntl.LOCK_NB)
except BlockingIOError:
    raise SystemExit("held-lock-not-owned")
PY
    return $?
  fi
  rbl_uid=$(id -u)
  rbl_lock_dir=$RBL_LOCK_PREFIX-$rbl_uid
  rbl_new_lock=0
  if [ -e "$rbl_lock_dir" ] || [ -L "$rbl_lock_dir" ]; then
    [ -d "$rbl_lock_dir" ] && [ ! -L "$rbl_lock_dir" ] || {
      rbl_die 'whole-build lock path is not a directory'
      return 1
    }
    [ "$(stat -c '%u:%a' -- "$rbl_lock_dir")" = "$rbl_uid:700" ] ||
      {
        rbl_die 'whole-build lock ownership/mode is unsafe'
        return 1
      }
  else
    mkdir -m 0700 -- "$rbl_lock_dir" || {
      rbl_die 'cannot create whole-build lock directory'
      return 1
    }
    rbl_new_lock=1
  fi
  [ "$(stat -c '%u:%a' -- "$rbl_lock_dir")" = "$rbl_uid:700" ] ||
    {
      rbl_die 'whole-build lock ownership/mode is unsafe'
      return 1
    }
  exec 9<"$rbl_lock_dir" || {
    rbl_die 'cannot open whole-build lock directory'
    return 1
  }
  flock -n 9 || {
    rbl_die 'another build layout invocation owns the whole-build lock'
    return 1
  }
  RELEASE_BUILD_LAYOUT_LOCK_HELD=1
  RELEASE_BUILD_LAYOUT_LOCK_DIR=$rbl_lock_dir
  export RELEASE_BUILD_LAYOUT_LOCK_HELD RELEASE_BUILD_LAYOUT_LOCK_DIR
}

release_build_layout_init() {
  [ "${RELEASE_BUILD_LAYOUT_INITIALIZED:-0}" != 1 ] || {
    rbl_die 'layout state is already initialized in this shell'
    return 1
  }
  rbl_source_root=$1
  rbl_commit=$2
  rbl_state_root=$3
  rbl_namespace=$4
  rbl_abs_path_ok "$rbl_source_root" || {
    rbl_die 'source root is not canonical absolute'
    return 1
  }
  rbl_abs_path_ok "$rbl_state_root" || {
    rbl_die 'state root is not canonical absolute'
    return 1
  }
  rbl_assert_root_commit "$rbl_source_root" "$rbl_commit" || return 1
  rbl_namespace_ok "$rbl_namespace" || {
    rbl_die 'cache namespace is invalid'
    return 1
  }
  case "$rbl_state_root" in
    "$rbl_source_root"|"$rbl_source_root"/*)
      rbl_die 'state root must be outside source root'
      return 1
      ;;
  esac
  rbl_layout=$rbl_source_root/deploy/build/release-build-layout.json
  rbl_validate_layout "$rbl_layout" || return 1
  # Validate every runtime group before the whole-build lock, state mutation,
  # native setup or producer compilation. Later consumers independently
  # rederive this inventory and must match this initialized binding.
  rbl_runtime_inventory=$(rbl_runtime_payload_inventory "$rbl_source_root" "$rbl_layout") || return 1
  rbl_runtime_inventory_hash=$(printf '%s\n' "$rbl_runtime_inventory" | sha256sum | awk '{print $1}') || return 1
  # The descriptor is held before any state directory is created or mutated.
  release_build_layout_lock || return 1
  # The official CLI derives state as <manifest-parent>/.lagrange-build-state/<commit>.
  # Create and validate that one private container only after the whole-run
  # lock is held; benchmark-supplied state roots retain their existing parent
  # contract and are not broadened by this product-specific path.
  rbl_state_parent=$(dirname -- "$rbl_state_root")
  if [ "$(basename -- "$rbl_state_parent")" = .lagrange-build-state ]; then
    rbl_private_dir "$rbl_state_parent" || return 1
  fi
  rbl_private_dir "$rbl_state_root" || return 1
  for rbl_child in contexts producers bundles images verification gates logs overrides; do
    rbl_private_dir "$rbl_state_root/$rbl_child" || return 1
  done
  rbl_source_hash=$(rbl_hash_source "$rbl_source_root" "$rbl_layout" 1) || return 1
  rbl_helper_hash=$(rbl_sha256_file "$rbl_source_root/scripts/ops/lib/release-build-layout.sh") || return 1
  rbl_layout_hash=$(rbl_sha256_file "$rbl_layout") || return 1
  rbl_run_path=$rbl_state_root/run.json
  RBL_RUN_PATH=$rbl_run_path RBL_SOURCE_ROOT=$rbl_source_root RBL_COMMIT=$rbl_commit \
  RBL_STATE_ROOT=$rbl_state_root RBL_NAMESPACE=$rbl_namespace RBL_SOURCE_HASH=$rbl_source_hash \
  RBL_HELPER_HASH=$rbl_helper_hash RBL_LAYOUT_HASH=$rbl_layout_hash \
  RBL_RUNTIME_INVENTORY_HASH=$rbl_runtime_inventory_hash python3 - <<'PY' || return 1
import json, os, stat, time
path = os.environ["RBL_RUN_PATH"]
value = {
  "format":"lagrange-build-layout-run-v1",
  "source_root":os.environ["RBL_SOURCE_ROOT"],
  "source_commit":os.environ["RBL_COMMIT"],
  "state_root":os.environ["RBL_STATE_ROOT"],
  "cache_namespace":os.environ["RBL_NAMESPACE"],
  "source_input_sha256":os.environ["RBL_SOURCE_HASH"],
  "runtime_payload_inventory_sha256":os.environ["RBL_RUNTIME_INVENTORY_HASH"],
  "helper_sha256":os.environ["RBL_HELPER_HASH"],
  "layout_sha256":os.environ["RBL_LAYOUT_HASH"],
  "guard_version":"common-1",
  "platform":"linux/amd64",
  "created_at_unix_ns":time.time_ns(),
  "gate_inputs":{
    "systemd_unit":os.environ.get("RELEASE_BUILD_SYSTEMD_UNIT",""),
    "systemd_manager":os.environ.get("RELEASE_BUILD_SYSTEMD_MANAGER","system"),
    "health_units":os.environ.get("RELEASE_BUILD_HEALTH_UNITS",""),
    "health_containers":os.environ.get("RELEASE_BUILD_HEALTH_CONTAINERS",""),
    "research_exception":os.environ.get("RELEASE_BUILD_RESEARCH_EXCEPTION","")
  }
}
encoded = (json.dumps(value, sort_keys=True, separators=(",",":")) + "\n").encode()
if os.path.lexists(path):
    info = os.lstat(path)
    if (not stat.S_ISREG(info.st_mode) or stat.S_ISLNK(info.st_mode) or
            stat.S_IMODE(info.st_mode) != 0o600 or info.st_uid != os.geteuid()):
        raise SystemExit("run-record-unsafe")
    current = json.loads(open(path, "rb").read().decode())
    for key in ("format","source_root","source_commit","state_root","cache_namespace",
                "source_input_sha256","runtime_payload_inventory_sha256",
                "helper_sha256","layout_sha256","guard_version",
                "platform","gate_inputs"):
        if current.get(key) != value[key]:
            raise SystemExit("run-binding-changed")
else:
    fd = os.open(path, os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as handle:
        handle.write(encoded); handle.flush(); os.fsync(handle.fileno())
PY
  RELEASE_BUILD_LAYOUT_INITIALIZED=1
  RELEASE_BUILD_LAYOUT_SOURCE_ROOT=$rbl_source_root
  RELEASE_BUILD_LAYOUT_COMMIT=$rbl_commit
  RELEASE_BUILD_LAYOUT_STATE_ROOT=$rbl_state_root
  RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE=$rbl_namespace
  RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256=$rbl_source_hash
  RELEASE_BUILD_LAYOUT_HELPER_SHA256=$rbl_helper_hash
  RELEASE_BUILD_LAYOUT_CONFIG_SHA256=$rbl_layout_hash
  RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256=$rbl_runtime_inventory_hash
  export RELEASE_BUILD_LAYOUT_INITIALIZED RELEASE_BUILD_LAYOUT_SOURCE_ROOT RELEASE_BUILD_LAYOUT_COMMIT
  export RELEASE_BUILD_LAYOUT_STATE_ROOT RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE
  export RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256 RELEASE_BUILD_LAYOUT_HELPER_SHA256
  export RELEASE_BUILD_LAYOUT_CONFIG_SHA256 RELEASE_BUILD_LAYOUT_RUNTIME_INVENTORY_SHA256
}

rbl_make_context() {
  rbl_recipe=$1
  rbl_state_root=$2
  rbl_layout=$RELEASE_BUILD_LAYOUT_SOURCE_ROOT/deploy/build/release-build-layout.json
  rbl_context_base=$rbl_state_root/contexts
  rbl_recipe_hash_value=$(rbl_recipe_hash "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT" "$rbl_recipe" "$rbl_layout") || return 1
  rbl_k_hash_value=$(rbl_k_hash "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT" "$rbl_recipe" "$rbl_layout") || return 1
  rbl_package_hashes_value=$(rbl_package_hashes "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT" "$rbl_layout" 1) || return 1
  # K is the compatibility identity of the one common producer.  Namespace
  # qualifies the mounts independently; recipe identity remains in H/request.
  rbl_cache_key=$rbl_k_hash_value
  rbl_partial=$rbl_context_base/.partial-context-$rbl_recipe-$$
  rbl_private_dir "$rbl_partial" || return 1
  RBL_SOURCE_ROOT=$RELEASE_BUILD_LAYOUT_SOURCE_ROOT RBL_LAYOUT=$rbl_layout RBL_PARTIAL=$rbl_partial \
  RBL_NATIVE_IDENTITY=$rbl_state_root/native-identity.json \
  RBL_RECIPE=$rbl_recipe RBL_COMMIT=$RELEASE_BUILD_LAYOUT_COMMIT RBL_SOURCE_HASH=$RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256 \
  RBL_RECIPE_HASH=$rbl_recipe_hash_value RBL_CACHE_KEY=$rbl_cache_key \
  RBL_HELPER_HASH=$RELEASE_BUILD_LAYOUT_HELPER_SHA256 RBL_LAYOUT_HASH=$RELEASE_BUILD_LAYOUT_CONFIG_SHA256 \
  RBL_NAMESPACE=$RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE RBL_K_HASH=$rbl_k_hash_value \
  RBL_PACKAGE_HASHES=$rbl_package_hashes_value python3 - <<'PY' || return 1
import hashlib, json, os, shutil, stat, subprocess, sys
source = os.path.abspath(os.environ["RBL_SOURCE_ROOT"])
layout_path = os.path.abspath(os.environ["RBL_LAYOUT"])
partial = os.path.abspath(os.environ["RBL_PARTIAL"])
layout = json.load(open(layout_path, encoding="utf-8"))
inventory = layout["source_inventory"]
package_hashes = json.loads(os.environ["RBL_PACKAGE_HASHES"])
if set(package_hashes) != set(layout["packages"]) or any(
        not isinstance(value,str) or len(value)!=64 for value in package_hashes.values()):
    raise SystemExit("context-package-hashes-invalid")
def tracked(spec):
    out = subprocess.run(
        ["git","-c","safe.directory="+source,"-C",source,"ls-files","-z","--",spec],
        check=True, stdout=subprocess.PIPE).stdout
    values = [item.decode("utf-8") for item in out.split(b"\0") if item]
    if not values: raise SystemExit("context-input-untracked-or-empty")
    return values
selected = set()
for spec in inventory["root_files"] + inventory["package_dirs"] + inventory["external_compile_inputs"]:
    for rel in tracked(spec):
        selected.add(rel)
        parent = os.path.dirname(rel)
        while parent and parent != ".":
            selected.add(parent)
            parent = os.path.dirname(parent)
def copy_entry(rel):
    src = os.path.join(source, rel)
    dst = os.path.join(partial, rel)
    info = os.lstat(src)
    if stat.S_ISLNK(info.st_mode) or not (stat.S_ISREG(info.st_mode) or stat.S_ISDIR(info.st_mode)):
        raise SystemExit("context-special-or-symlink")
    if stat.S_ISDIR(info.st_mode):
        os.makedirs(dst, exist_ok=True)
        os.chmod(dst, stat.S_IMODE(info.st_mode))
        os.utime(dst, ns=(info.st_atime_ns, info.st_mtime_ns), follow_symlinks=False)
    else:
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        shutil.copyfile(src, dst, follow_symlinks=False)
        os.chmod(dst, stat.S_IMODE(info.st_mode))
        os.utime(dst, ns=(info.st_atime_ns, info.st_mtime_ns), follow_symlinks=False)
for rel in sorted(selected, key=lambda x:(x.count("/"), x)):
    copy_entry(rel)
release_dir = os.path.join(partial, ".release-build")
os.makedirs(release_dir, mode=0o700)
helper = os.path.join(source, "scripts/ops/lib/release-build-layout.sh")
config = layout_path
shutil.copyfile(helper, os.path.join(release_dir, "release-build-layout.sh"))
shutil.copyfile(config, os.path.join(release_dir, "release-build-layout.json"))
os.chmod(os.path.join(release_dir, "release-build-layout.sh"), 0o755)
os.chmod(os.path.join(release_dir, "release-build-layout.json"), 0o644)
# P0 may create the first context without a measured native identity.  A
# compiler context is assembled only after that identity exists, and carries
# the exact bytes that the derived stage must remeasure before Cargo.
native_identity=os.environ["RBL_NATIVE_IDENTITY"]
if os.path.lexists(native_identity):
    info=os.lstat(native_identity)
    if (stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode) or
            stat.S_IMODE(info.st_mode)!=0o600 or info.st_uid!=os.geteuid()):
        raise SystemExit("context-native-identity-invalid")
    shutil.copyfile(native_identity, os.path.join(release_dir, "expected-native-identity.json"))
    os.chmod(os.path.join(release_dir, "expected-native-identity.json"), 0o600)
recipe = layout["recipes"][os.environ["RBL_RECIPE"]]
payloads = layout["runtime_payloads"].get(recipe.get("runtime_payload",""), [])
def closure(name, visiting=None, result=None):
    if visiting is None: visiting=set()
    if result is None: result=set()
    if name in visiting: raise SystemExit("package-graph-cycle")
    if name in result: return result
    visiting.add(name)
    for dependency in layout["packages"][name]["local_dependencies"]:
        closure(dependency, visiting, result)
    visiting.remove(name); result.add(name)
    return result
transitive = [recipe["package"]] + sorted(closure(recipe["package"]) - {recipe["package"]})
resolution_inputs = {
  "argv":["cargo","build","--locked","--release","--package",recipe["package"],"--bin"],
  "default_features":True,
  "locked":True,
  "message_format":"json-render-diagnostics",
  "verbose":True
}
request = {
  "format":"lagrange-rust-artifact-request-v1",
  "recipe":os.environ["RBL_RECIPE"],
  "package":recipe["package"],
  "bins":recipe["bins"],
  "compile_commit":recipe["compile_commit"],
  "source_commit":os.environ["RBL_COMMIT"],
  "platform":"linux/amd64",
  "host_triple":"x86_64-unknown-linux-musl",
  "profile":"release",
  "features":[],
  "compile_env":{
    "CARGO_BUILD_JOBS":"2",
    "CARGO_TARGET_DIR":"/cargo-target",
    "RUSTFLAGS":"<unset>",
    "LAGRANGE_CODE_COMMIT":(os.environ["RBL_COMMIT"] if recipe["compile_commit"] == "present" else "<unset>")
  },
  "clean_packages":recipe["clean_packages"],
  "package_hashes":package_hashes,
  "transitive_package_hashes":[{"package":name,"p_sha256":package_hashes[name]} for name in transitive],
  "resolution_inputs":resolution_inputs,
  "input_sha256":os.environ["RBL_SOURCE_HASH"],
  "recipe_sha256":os.environ["RBL_RECIPE_HASH"],
  "k_sha256":os.environ["RBL_K_HASH"],
  "cache_key":os.environ["RBL_CACHE_KEY"],
  "helper_sha256":os.environ["RBL_HELPER_HASH"],
  "layout_sha256":os.environ["RBL_LAYOUT_HASH"],
  "guard_version":"common-1",
  "cache_namespace":os.environ["RBL_NAMESPACE"],
  "payloads":[{"source":item["source"],"image":item["image"]} for item in payloads]
}
h_material={
  "format":"lagrange-build-layout-h-v1",
  "k_sha256":request["k_sha256"],
  "package":request["package"],
  "bins":request["bins"],
  "recipe":request["recipe"],
  "recipe_sha256":request["recipe_sha256"],
  "transitive_package_hashes":request["transitive_package_hashes"],
  "resolution_inputs":request["resolution_inputs"],
  "compile_env":request["compile_env"],
  "features":request["features"],
  "profile":request["profile"],
  "platform":request["platform"],
  "host_triple":request["host_triple"]
}
request["h_sha256"]=hashlib.sha256(json.dumps(h_material,sort_keys=True,separators=(",",":")).encode("utf-8")).hexdigest()
with open(os.path.join(release_dir, "request.json"), "w", encoding="utf-8", newline="\n") as handle:
    handle.write(json.dumps(request, sort_keys=True, separators=(",",":")) + "\n")
os.chmod(os.path.join(release_dir, "request.json"), 0o600)
PY
  rbl_transport_hash=$(rbl_tree_hash "$rbl_partial") || return 1
  rbl_context=$rbl_context_base/context-$rbl_transport_hash
  if [ -e "$rbl_context" ] || [ -L "$rbl_context" ]; then
    [ -d "$rbl_context" ] && [ ! -L "$rbl_context" ] || {
      rbl_die 'content-addressed context is not a directory'
      return 1
    }
    [ "$(rbl_tree_hash "$rbl_context")" = "$rbl_transport_hash" ] || {
      rbl_die 'context digest collision or mutation'
      return 1
    }
    rm -rf -- "$rbl_partial"
  else
    mv -- "$rbl_partial" "$rbl_context" || return 1
  fi
  printf '%s\n' "$rbl_context"
}

rbl_request_value() {
  rbl_request=$1
  rbl_key=$2
  python3 - "$rbl_request" "$rbl_key" <<'PY'
import json, sys
value=json.load(open(sys.argv[1], encoding="utf-8"))
item=value.get(sys.argv[2])
if item is None: raise SystemExit("request-field-missing")
if isinstance(item, (dict,list)): print(json.dumps(item,sort_keys=True,separators=(",",":")))
else: print(item)
PY
}

rbl_verify_producer() {
  rbl_dir=$1
  rbl_recipe=$2
  rbl_bin=$3
  rbl_request=$4
  [ -d "$rbl_dir" ] && [ ! -L "$rbl_dir" ] || return 1
  RBL_PRODUCER=$rbl_dir RBL_RECIPE=$rbl_recipe RBL_BIN=$rbl_bin RBL_REQUEST=$rbl_request \
  python3 - <<'PY'
import hashlib, json, os, re, stat
directory=os.path.abspath(os.environ["RBL_PRODUCER"])
request=json.load(open(os.environ["RBL_REQUEST"],encoding="utf-8"))
bin_name=os.environ["RBL_BIN"]
allowed={"artifact.json","COMPLETE","bin","cargo.jsonl","cargo.stderr","cargo-summary.json","timing.json"}
def regular(path):
    try: info=os.lstat(path)
    except OSError: return False
    return stat.S_ISREG(info.st_mode) and not stat.S_ISLNK(info.st_mode)
def directory_ok(path):
    try: info=os.lstat(path)
    except OSError: return False
    return stat.S_ISDIR(info.st_mode) and not stat.S_ISLNK(info.st_mode)
if not directory_ok(directory) or set(os.listdir(directory)) != allowed:
    raise SystemExit("producer-layout-invalid")
for name in ("artifact.json","COMPLETE","cargo.jsonl","cargo.stderr","cargo-summary.json","timing.json"):
    if not regular(os.path.join(directory,name)): raise SystemExit("producer-evidence-missing")
bindir=os.path.join(directory,"bin")
if not directory_ok(bindir) or set(os.listdir(bindir)) != {bin_name}: raise SystemExit("producer-bin-layout-invalid")
binary=os.path.join(bindir,bin_name)
if not regular(binary) or stat.S_IMODE(os.stat(binary).st_mode) != 0o755: raise SystemExit("producer-binary-mode-invalid")
def pairs(items):
    value={}
    for key,item in items:
        if key in value: raise ValueError("duplicate-json-key")
        value[key]=item
    return value
def reject(v): raise ValueError("non-finite-json-number")
record=json.load(open(os.path.join(directory,"artifact.json"),encoding="utf-8",newline=""),
                 object_pairs_hook=pairs,parse_constant=reject)
artifact_raw=open(os.path.join(directory,"artifact.json"),"rb").read()
if artifact_raw != (json.dumps(record,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8"):
    raise SystemExit("artifact-json-not-canonical")
required={"binary_mode","binary_sha256","bin","cache_key","cargo_success","compile_env","features",
          "format","h_sha256","host_triple","input_sha256","package","platform","profile","recipe_sha256",
          "source_commit"}
if set(record) != required: raise SystemExit("artifact-json-keys-invalid")
expected={
  "format":"lagrange-rust-artifact-v1","source_commit":request["source_commit"],
  "package":request["package"],"bin":bin_name,"platform":"linux/amd64",
  "host_triple":"x86_64-unknown-linux-musl","profile":"release","features":[],
  "compile_env":request["compile_env"],"cache_key":request["cache_key"],
  "input_sha256":request["input_sha256"],"recipe_sha256":request["recipe_sha256"],
  "h_sha256":request["h_sha256"],"cargo_success":True,"binary_mode":"0755"
}
for key,value in expected.items():
    if record.get(key) != value: raise SystemExit("artifact-field-mismatch-"+key)
if not re.fullmatch(r"[0-9a-f]{64}",record["binary_sha256"]): raise SystemExit("binary-hash-invalid")
digest=hashlib.sha256()
with open(binary,"rb") as handle:
    for chunk in iter(lambda:handle.read(1024*1024),b""): digest.update(chunk)
if digest.hexdigest()!=record["binary_sha256"]: raise SystemExit("binary-hash-mismatch")
raw=open(os.path.join(directory,"artifact.json"),"rb").read()
if open(os.path.join(directory,"COMPLETE"),"rb").read() != (hashlib.sha256(raw).hexdigest()+"\n").encode():
    raise SystemExit("complete-mismatch")
summary_raw=open(os.path.join(directory,"cargo-summary.json"),"rb").read()
summary=json.loads(summary_raw.decode("utf-8"),object_pairs_hook=pairs,parse_constant=reject)
if summary_raw != (json.dumps(summary,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8"):
    raise SystemExit("cargo-summary-not-canonical")
if set(summary) != {"format","cargo_success","observed_bin","package","fresh_count","compiling_count","executable","cargo_ms"} or \
        summary.get("format") != "lagrange-cargo-summary-v1" or summary.get("cargo_success") is not True or \
        summary.get("observed_bin") != bin_name or summary.get("package") != request["package"] or \
        summary.get("executable") != "/cargo-target/release/"+bin_name or \
        type(summary.get("fresh_count")) is not int or summary["fresh_count"] < 0 or \
        type(summary.get("compiling_count")) is not int or summary["compiling_count"] < 0 or \
        type(summary.get("cargo_ms")) is not int or summary["cargo_ms"] < 0:
    raise SystemExit("cargo-summary-invalid")
timing_raw=open(os.path.join(directory,"timing.json"),"rb").read()
timing=json.loads(timing_raw.decode("utf-8"),object_pairs_hook=pairs,parse_constant=reject)
if timing_raw != (json.dumps(timing,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8"):
    raise SystemExit("timing-json-not-canonical")
if set(timing) != {"format","cargo_ms"} or timing.get("format") != "lagrange-cargo-timing-v1" or \
        type(timing.get("cargo_ms")) is not int or timing["cargo_ms"] < 0:
    raise SystemExit("timing-invalid")
seen_target=False
for raw in open(os.path.join(directory,"cargo.jsonl"),encoding="utf-8"):
    if not raw.strip(): continue
    try: event=json.loads(raw)
    except (ValueError,json.JSONDecodeError): raise SystemExit("cargo-json-invalid")
    if event.get("reason") == "compiler-artifact" and (event.get("target") or {}).get("name") == bin_name:
        if event.get("executable") != "/cargo-target/release/"+bin_name: raise SystemExit("cargo-json-executable-invalid")
        seen_target=True
if not seen_target or not __import__("re").search(r"(?:^|\s)(?:Fresh|Compiling)\s",open(os.path.join(directory,"cargo.stderr"),encoding="utf-8").read()):
    raise SystemExit("cargo-evidence-invalid")
PY
}

rbl_capture_attempt_log() {
  rbl_attempt_state=$1
  rbl_attempt_log=$2
  rbl_attempt_label=$3
  shift 3
  [ "$#" -gt 0 ] || {
    rbl_die 'attempt log capture needs a command'
    return 1
  }
  rbl_abs_path_ok "$rbl_attempt_state" || {
    rbl_die 'attempt log state root is not canonical'
    return 1
  }
  case "$rbl_attempt_label" in
    native-identity|producer-D[1-7]-[a-z0-9][a-z0-9_-]*) : ;;
    *)
      rbl_die 'attempt log label is not canonical'
      return 1
      ;;
  esac
  rbl_attempt_expected=$rbl_attempt_state/logs/$rbl_attempt_label.log
  [ "$rbl_attempt_log" = "$rbl_attempt_expected" ] || {
    rbl_die 'attempt log path does not match its label'
    return 1
  }
  rbl_private_dir "$rbl_attempt_state/logs" || return 1
  rbl_attempt_history=$rbl_attempt_state/logs/attempt-history
  rbl_private_dir "$rbl_attempt_history" || return 1
  rbl_attempt_history=$rbl_attempt_history/$rbl_attempt_label
  rbl_private_dir "$rbl_attempt_history" || return 1
  rbl_private_dir "$rbl_attempt_history/objects" || return 1
  rbl_private_dir "$rbl_attempt_history/receipts" || return 1
  RBL_ATTEMPT_LOG=$rbl_attempt_log RBL_ATTEMPT_HISTORY=$rbl_attempt_history \
  RBL_ATTEMPT_LABEL=$rbl_attempt_label python3 - "$@" <<'PY'
import hashlib, json, os, stat, subprocess, sys, tempfile, time

active=os.environ["RBL_ATTEMPT_LOG"]
history=os.environ["RBL_ATTEMPT_HISTORY"]
label=os.environ["RBL_ATTEMPT_LABEL"]
argv=sys.argv[1:]
objects=os.path.join(history,"objects")
receipts=os.path.join(history,"receipts")

def private_directory(path):
    if (not path.startswith("/") or path.startswith("//") or os.path.normpath(path)!=path or
            os.path.realpath(path)!=path):
        raise ValueError("attempt-history-path-invalid")
    info=os.lstat(path)
    if (stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode) or
            info.st_uid!=os.geteuid() or stat.S_IMODE(info.st_mode)!=0o700):
        raise ValueError("attempt-history-directory-unsafe")

def checked_regular_fd(path, flags):
    before=os.lstat(path)
    if (stat.S_ISLNK(before.st_mode) or not stat.S_ISREG(before.st_mode) or
            before.st_uid!=os.geteuid() or stat.S_IMODE(before.st_mode)!=0o600):
        raise ValueError("attempt-log-file-unsafe")
    fd=os.open(path,flags|getattr(os,"O_NOFOLLOW",0))
    opened=os.fstat(fd)
    if ((before.st_dev,before.st_ino)!=(opened.st_dev,opened.st_ino) or
            not stat.S_ISREG(opened.st_mode) or opened.st_uid!=os.geteuid() or
            stat.S_IMODE(opened.st_mode)!=0o600):
        os.close(fd)
        raise ValueError("attempt-log-file-raced")
    return fd,opened

def stable_digest(fd,expected):
    os.lseek(fd,0,os.SEEK_SET)
    digest=hashlib.sha256()
    size=0
    while True:
        chunk=os.read(fd,1024*1024)
        if not chunk:
            break
        digest.update(chunk)
        size+=len(chunk)
    after=os.fstat(fd)
    if ((expected.st_dev,expected.st_ino,expected.st_size,expected.st_mtime_ns,expected.st_ctime_ns)!=
            (after.st_dev,after.st_ino,after.st_size,after.st_mtime_ns,after.st_ctime_ns) or
            size!=expected.st_size):
        raise ValueError("attempt-log-file-changed")
    return digest.hexdigest(),size

def copy_object(fd,source_info):
    digest,size=stable_digest(fd,source_info)
    object_path=os.path.join(objects,digest+".bytes")
    created=False
    try:
        object_fd=os.open(object_path,os.O_RDWR|os.O_CREAT|os.O_EXCL|getattr(os,"O_NOFOLLOW",0),0o600)
    except FileExistsError:
        object_fd,object_info=checked_regular_fd(object_path,os.O_RDONLY)
        try:
            existing_digest,existing_size=stable_digest(object_fd,object_info)
        finally:
            os.close(object_fd)
        if existing_digest!=digest or existing_size!=size:
            raise ValueError("attempt-history-object-no-clobber")
    else:
        try:
            opened=os.fstat(object_fd)
            if (not stat.S_ISREG(opened.st_mode) or opened.st_uid!=os.geteuid() or
                    stat.S_IMODE(opened.st_mode)!=0o600):
                raise ValueError("attempt-history-object-unsafe")
            os.lseek(fd,0,os.SEEK_SET)
            remaining=size
            while remaining:
                chunk=os.read(fd,min(1024*1024,remaining))
                if not chunk:
                    raise ValueError("attempt-log-short-read")
                wrote=0
                while wrote<len(chunk):
                    count=os.write(object_fd,chunk[wrote:])
                    if count<=0:
                        raise OSError("attempt-history-short-write")
                    wrote+=count
                remaining-=len(chunk)
            os.fsync(object_fd)
            final_digest,final_size=stable_digest(object_fd,os.fstat(object_fd))
            if final_digest!=digest or final_size!=size:
                raise ValueError("attempt-history-object-mismatch")
            created=True
        except BaseException:
            try: os.unlink(object_path)
            except OSError: pass
            raise
        finally:
            os.close(object_fd)
    after=os.fstat(fd)
    if ((source_info.st_dev,source_info.st_ino,source_info.st_size,source_info.st_mtime_ns,source_info.st_ctime_ns)!=
            (after.st_dev,after.st_ino,after.st_size,after.st_mtime_ns,after.st_ctime_ns)):
        raise ValueError("attempt-log-file-changed")
    return digest,size,created

def receipt(phase,exit_code,digest,size):
    value={
        "active_log":os.path.basename(active),"attempt_label":label,
        "exit_code":exit_code,"format":"lagrange-build-log-attempt-v1",
        "phase":phase,"raw_bytes":size,"raw_path":"objects/"+digest+".bytes",
        "raw_sha256":digest,"time_unix_ns":time.time_ns()
    }
    data=(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8")
    fd,path=tempfile.mkstemp(prefix="attempt-",suffix=".json",dir=receipts)
    try:
        os.fchmod(fd,0o600)
        info=os.fstat(fd)
        if (not stat.S_ISREG(info.st_mode) or info.st_uid!=os.geteuid() or
                stat.S_IMODE(info.st_mode)!=0o600):
            raise ValueError("attempt-history-receipt-unsafe")
        offset=0
        while offset<len(data):
            wrote=os.write(fd,data[offset:])
            if wrote<=0:
                raise OSError("attempt-history-receipt-short-write")
            offset+=wrote
        os.fsync(fd)
    except BaseException:
        try: os.unlink(path)
        except OSError: pass
        raise
    finally:
        os.close(fd)

for directory in (history,objects,receipts):
    private_directory(directory)

if os.path.lexists(active):
    prior_fd,prior_info=checked_regular_fd(active,os.O_RDONLY)
    try:
        prior_digest,prior_size,prior_created=copy_object(prior_fd,prior_info)
    finally:
        os.close(prior_fd)
    if prior_created:
        receipt("pre-replace-unlinked",None,prior_digest,prior_size)

if os.path.lexists(active):
    active_fd,active_info=checked_regular_fd(active,os.O_WRONLY)
    try:
        os.ftruncate(active_fd,0)
        os.fsync(active_fd)
    except BaseException:
        os.close(active_fd)
        raise
else:
    active_fd=os.open(active,os.O_WRONLY|os.O_CREAT|os.O_EXCL|getattr(os,"O_NOFOLLOW",0),0o600)
    active_info=os.fstat(active_fd)
    if (not stat.S_ISREG(active_info.st_mode) or active_info.st_uid!=os.geteuid() or
            stat.S_IMODE(active_info.st_mode)!=0o600):
        os.close(active_fd)
        raise ValueError("attempt-log-new-file-unsafe")

try:
    try:
        result=subprocess.run(argv,stdin=subprocess.DEVNULL,stdout=active_fd,
                              stderr=subprocess.STDOUT,check=False)
        exit_code=result.returncode
    except OSError:
        exit_code=127
    os.fsync(active_fd)
finally:
    os.close(active_fd)

completed_fd,completed_info=checked_regular_fd(active,os.O_RDONLY)
try:
    completed_digest,completed_size,_=copy_object(completed_fd,completed_info)
finally:
    os.close(completed_fd)
receipt("completed",exit_code,completed_digest,completed_size)
raise SystemExit(exit_code)
PY
}

rbl_produce_native() {
  rbl_context=$1
  rbl_state_root=$2
  rbl_identity=$rbl_state_root/native-identity.json
  if [ -e "$rbl_identity" ] || [ -L "$rbl_identity" ]; then
    rbl_validate_native_identity "$rbl_identity" || return 1
    return 0
  fi
  rbl_partial=$rbl_state_root/contexts/.partial-native-$$
  [ ! -e "$rbl_partial" ] && [ ! -L "$rbl_partial" ] ||
    {
      rbl_die "stale native identity staging path: $rbl_partial"
      return 1
    }
  rbl_private_dir "$rbl_partial" || return 1
  rbl_log=$rbl_state_root/logs/native-identity.log
  if rbl_capture_attempt_log "$rbl_state_root" "$rbl_log" native-identity \
      docker buildx build --pull=false --progress=plain --platform "$RBL_PLATFORM" \
      --file "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT/deploy/build/Dockerfile.rust-artifacts" \
      --target native --output "type=local,dest=$rbl_partial" \
      --build-arg=RUST_ARTIFACT_CACHE_NAMESPACE="$RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE" \
      "$rbl_context"; then
    :
  else
    rbl_die "native identity export failed (see $rbl_log)" || return 1
  fi
  rbl_exported=$rbl_partial/.release-build/native-identity.json
  rbl_validate_native_identity "$rbl_exported" || return 1
  install -m 0600 -- "$rbl_exported" "$rbl_identity" || return 1
  rbl_validate_native_identity "$rbl_identity" || return 1
  rm -rf -- "$rbl_partial"
}

rbl_produce_bin() {
  rbl_recipe=$1
  rbl_bin=$2
  rbl_context=$3
  rbl_state_root=$4
  rbl_request=$rbl_context/.release-build/request.json
  rbl_output=$rbl_state_root/producers/$rbl_recipe/$rbl_bin
  rbl_validate_native_identity "$rbl_state_root/native-identity.json" || return 1
  rbl_current_k=$(rbl_k_hash "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT" "$rbl_recipe" \
    "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT/deploy/build/release-build-layout.json") || return 1
  rbl_request_k=$(rbl_request_value "$rbl_request" k_sha256) || return 1
  [ "$rbl_request_k" = "$rbl_current_k" ] || {
    rbl_die 'producer context has a pending or stale native compatibility key'
    return 1
  }
  if [ -d "$rbl_output" ] && rbl_verify_producer "$rbl_output" "$rbl_recipe" "$rbl_bin" "$rbl_request"; then
    printf '%s\n' "$rbl_output"
    return 0
  fi
  rbl_private_dir "$rbl_state_root/producers" || return 1
  rbl_private_dir "$rbl_state_root/producers/$rbl_recipe" || return 1
  rbl_partial=$rbl_state_root/producers/.partial-$rbl_recipe-$rbl_bin-$$
  rbl_private_dir "$rbl_partial" || return 1
  rbl_recipe_json_value=$(rbl_recipe_json "$rbl_recipe" "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT/deploy/build/release-build-layout.json") || return 1
  rbl_package=$(printf '%s' "$rbl_recipe_json_value" | python3 -c 'import json,sys; print(json.load(sys.stdin)["package"])')
  rbl_compile_commit=$(printf '%s' "$rbl_recipe_json_value" | python3 -c 'import json,sys; print(json.load(sys.stdin)["compile_commit"])')
  rbl_log=$RELEASE_BUILD_LAYOUT_STATE_ROOT/logs/producer-$rbl_recipe-$rbl_bin.log
  rbl_args="--pull=false --progress=plain --platform $RBL_PLATFORM --file $RELEASE_BUILD_LAYOUT_SOURCE_ROOT/deploy/build/Dockerfile.rust-artifacts --target artifacts --output type=local,dest=$rbl_partial"
  if [ "$rbl_compile_commit" = present ]; then
    rbl_commit_arg="--build-arg=LAGRANGE_CODE_COMMIT=$RELEASE_BUILD_LAYOUT_COMMIT"
  else
    rbl_commit_arg=
  fi
  # This is the sole producer invocation path. It is intentionally one bin per
  # call; D2/D6 never become an artificial multi-bin Cargo command.
  if rbl_capture_attempt_log "$rbl_state_root" "$rbl_log" "producer-$rbl_recipe-$rbl_bin" \
      docker buildx build $rbl_args \
      --build-arg=CARGO_PACKAGE=$rbl_package \
      --build-arg=CARGO_BIN=$rbl_bin \
      --build-arg=RUST_ARTIFACT_CACHE_NAMESPACE=$RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE \
      --build-arg=RUST_ARTIFACT_CACHE_KEY=$(rbl_request_value "$rbl_request" cache_key) \
      --build-arg=RUST_ARTIFACT_INPUT_SHA256=$(rbl_request_value "$rbl_request" input_sha256) \
      --build-arg=RUST_ARTIFACT_RECIPE_SHA256=$(rbl_request_value "$rbl_request" recipe_sha256) \
      --build-arg=RUST_ARTIFACT_HELPER_SHA256=$RELEASE_BUILD_LAYOUT_HELPER_SHA256 \
      --build-arg=RUST_ARTIFACT_LAYOUT_SHA256=$RELEASE_BUILD_LAYOUT_CONFIG_SHA256 \
      $rbl_commit_arg "$rbl_context"; then
    :
  else
    rbl_die "producer failed: $rbl_recipe/$rbl_bin (see $rbl_log)" || return 1
  fi
  rbl_produced=$rbl_partial/.release-build/producer/$rbl_bin
  rbl_native=$rbl_partial/.release-build/native-identity.json
  [ -d "$rbl_produced" ] || {
    rbl_die "producer output is missing: $rbl_recipe/$rbl_bin"
    return 1
  }
  [ -f "$rbl_native" ] || {
    rbl_die "native identity output is missing: $rbl_recipe/$rbl_bin"
    return 1
  }
  rbl_verify_producer "$rbl_produced" "$rbl_recipe" "$rbl_bin" "$rbl_request" || return 1
  if [ -e "$rbl_output" ] || [ -L "$rbl_output" ]; then
    [ -d "$rbl_output" ] && [ "$(rbl_tree_hash "$rbl_output")" = "$(rbl_tree_hash "$rbl_produced")" ] ||
      {
        rbl_die "producer no-clobber mismatch: $rbl_recipe/$rbl_bin"
        return 1
      }
  else
    mv -- "$rbl_produced" "$rbl_output" || return 1
  fi
  rbl_identity=$RELEASE_BUILD_LAYOUT_STATE_ROOT/native-identity.json
  if [ -e "$rbl_identity" ]; then
    cmp -s -- "$rbl_identity" "$rbl_native" || {
      rbl_die 'native compiler identity changed during one run'
      return 1
    }
  else
    install -m 0600 -- "$rbl_native" "$rbl_identity" || return 1
  fi
  printf '%s\n' "$rbl_output"
}

rbl_entry_hash() {
  rbl_entry_root=$1
  rbl_entry_rel=$2
  RBL_ENTRY_ROOT=$rbl_entry_root RBL_ENTRY_REL=$rbl_entry_rel python3 - <<'PY'
import hashlib, os, stat
root = os.path.abspath(os.environ["RBL_ENTRY_ROOT"])
rel = os.environ["RBL_ENTRY_REL"]
path = os.path.join(root, rel)
if os.path.normpath(rel) != rel or rel.startswith("/") or rel == ".." or rel.startswith("../"):
    raise SystemExit("entry-path-invalid")
def walk(path, name, records):
    info = os.lstat(path)
    if stat.S_ISLNK(info.st_mode) or not (stat.S_ISREG(info.st_mode) or stat.S_ISDIR(info.st_mode)):
        raise SystemExit("entry-special-or-symlink")
    if stat.S_ISDIR(info.st_mode):
        records.append(f"d\t{stat.S_IMODE(info.st_mode):04o}\t{name}\t-\n".encode())
        for child in sorted(os.listdir(path)):
            walk(os.path.join(path, child), name + "/" + child if name else child, records)
    else:
        digest = hashlib.sha256()
        with open(path, "rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        records.append(f"f\t{stat.S_IMODE(info.st_mode):04o}\t{name}\t{digest.hexdigest()}\n".encode())
records = []
walk(path, rel, records)
print(hashlib.sha256(b"".join(records)).hexdigest())
PY
}

rbl_bundle_tree_hash() {
  rbl_bundle=$1
  RBL_BUNDLE=$rbl_bundle python3 - <<'PY'
import hashlib, os, stat
root=os.path.abspath(os.environ["RBL_BUNDLE"])
records=[]
for current, dirs, files in os.walk(root, topdown=True, followlinks=False):
    dirs.sort(); files.sort()
    kept=[]
    for name in dirs:
        path=os.path.join(current,name); info=os.lstat(path)
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
            raise SystemExit("bundle-directory-invalid")
        kept.append(name)
        rel=os.path.relpath(path,root)
        records.append(f"d\t{stat.S_IMODE(info.st_mode):04o}\t{rel}\t-\n".encode())
    dirs[:]=kept
    for name in files:
        path=os.path.join(current,name)
        if os.path.relpath(path,root)==".release-build/complete":
            continue
        info=os.lstat(path)
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
            raise SystemExit("bundle-file-invalid")
        digest=hashlib.sha256()
        with open(path,"rb") as handle:
            for chunk in iter(lambda:handle.read(1024*1024),b""):
                digest.update(chunk)
        records.append(f"f\t{stat.S_IMODE(info.st_mode):04o}\t{os.path.relpath(path,root)}\t{digest.hexdigest()}\n".encode())
print(hashlib.sha256(b"".join(sorted(records))).hexdigest())
PY
}

rbl_copy_entry() {
  rbl_copy_source=$1
  rbl_copy_destination=$2
  RBL_COPY_SOURCE=$rbl_copy_source RBL_COPY_DESTINATION=$rbl_copy_destination python3 - <<'PY'
import os, shutil, stat
source=os.path.abspath(os.environ["RBL_COPY_SOURCE"])
destination=os.path.abspath(os.environ["RBL_COPY_DESTINATION"])
def copy_one(src,dst):
    info=os.lstat(src)
    if stat.S_ISLNK(info.st_mode) or not (stat.S_ISREG(info.st_mode) or stat.S_ISDIR(info.st_mode)):
        raise SystemExit("copy-source-special-or-symlink")
    if stat.S_ISDIR(info.st_mode):
        os.makedirs(dst, exist_ok=True)
        os.chmod(dst, stat.S_IMODE(info.st_mode))
        for name in sorted(os.listdir(src)):
            copy_one(os.path.join(src,name),os.path.join(dst,name))
        os.utime(dst,ns=(info.st_atime_ns,info.st_mtime_ns))
    else:
        os.makedirs(os.path.dirname(dst),exist_ok=True)
        shutil.copyfile(src,dst)
        os.chmod(dst,stat.S_IMODE(info.st_mode))
        os.utime(dst,ns=(info.st_atime_ns,info.st_mtime_ns))
copy_one(source,destination)
PY
}

rbl_bundle_create() {
  rbl_recipe=$1
  rbl_state_root=$2
  rbl_layout=$RELEASE_BUILD_LAYOUT_SOURCE_ROOT/deploy/build/release-build-layout.json
  # Shell functions have no local variables under POSIX sh.  Keep the target
  # separate from rbl_bundle_verify's public argument variable so verification
  # of the staging tree cannot redirect this final no-clobber move into itself.
  rbl_bundle_target=$rbl_state_root/bundles/$rbl_recipe
  if [ -e "$rbl_bundle_target" ] || [ -L "$rbl_bundle_target" ]; then
    [ -d "$rbl_bundle_target" ] && [ ! -L "$rbl_bundle_target" ] ||
      {
        rbl_die "bundle path is not a directory: $rbl_bundle_target"
        return 1
      }
    rbl_bundle_verify "$rbl_bundle_target" "$rbl_recipe" >/dev/null || return 1
    printf '%s\n' "$rbl_bundle_target"
    return 0
  fi
  rbl_context=$(rbl_make_context "$rbl_recipe" "$rbl_state_root") || return 1
  rbl_partial=$rbl_state_root/bundles/.partial-$rbl_recipe-$$
  [ ! -e "$rbl_partial" ] && [ ! -L "$rbl_partial" ] ||
    {
      rbl_die "stale private bundle staging path: $rbl_partial"
      return 1
    }
  rbl_private_dir "$rbl_partial" || return 1
  rbl_private_dir "$rbl_partial/.release-build" || return 1
  mkdir -- "$rbl_partial/target" || return 1
  mkdir -- "$rbl_partial/target/release" || return 1
  rbl_request=$rbl_context/.release-build/request.json
  for rbl_bin in $(rbl_recipe_json "$rbl_recipe" "$rbl_layout" |
      python3 -c 'import json,sys; print("\n".join(json.load(sys.stdin)["bins"]))'); do
    rbl_producer=$rbl_state_root/producers/$rbl_recipe/$rbl_bin
    rbl_verify_producer "$rbl_producer" "$rbl_recipe" "$rbl_bin" "$rbl_request" || return 1
    rbl_copy_entry "$rbl_producer" "$rbl_partial/.release-build/producer/$rbl_bin" || return 1
    install -m 0755 -- "$rbl_producer/bin/$rbl_bin" "$rbl_partial/target/release/$rbl_bin" || return 1
  done
  install -m 0644 -- "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT/deploy/build/release-build-layout.json" \
    "$rbl_partial/.release-build/release-build-layout.json" || return 1
  install -m 0755 -- "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT/scripts/ops/lib/release-build-layout.sh" \
    "$rbl_partial/.release-build/release-build-layout.sh" || return 1
  install -m 0600 -- "$rbl_request" "$rbl_partial/.release-build/request.json" || return 1
  install -m 0600 -- "$RELEASE_BUILD_LAYOUT_STATE_ROOT/native-identity.json" \
    "$rbl_partial/.release-build/native-identity.json" || return 1
  rbl_recipe_json_value=$(rbl_recipe_json "$rbl_recipe" "$rbl_layout") || return 1
  rbl_runtime_inventory_value=$(rbl_runtime_payload_inventory_current) || return 1
  rbl_runtime_payload_group=$(printf '%s\n' "$rbl_recipe_json_value" |
    python3 -c 'import json,sys; print(json.load(sys.stdin).get("runtime_payload",""))') || return 1
  rbl_payloads_json=$(rbl_runtime_payload_copy "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT" \
    "$rbl_partial" "$rbl_runtime_payload_group" "$rbl_runtime_inventory_value") || return 1
  RBL_PARTIAL=$rbl_partial RBL_RECIPE=$rbl_recipe RBL_REQUEST=$rbl_partial/.release-build/request.json \
  RBL_RECIPE_JSON=$rbl_recipe_json_value RBL_SOURCE_COMMIT=$RELEASE_BUILD_LAYOUT_COMMIT \
  RBL_SOURCE_HASH=$RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256 RBL_PAYLOADS_JSON=$rbl_payloads_json \
  RBL_RECIPE_HASH=$(rbl_recipe_hash "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT" "$rbl_recipe" "$rbl_layout") \
  RBL_HELPER_HASH=$RELEASE_BUILD_LAYOUT_HELPER_SHA256 RBL_LAYOUT_HASH=$RELEASE_BUILD_LAYOUT_CONFIG_SHA256 \
  RBL_CACHE_NAMESPACE=$RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE RBL_CACHE_KEY=$(rbl_request_value "$rbl_request" cache_key) \
  RBL_K_HASH=$(rbl_request_value "$rbl_request" k_sha256) \
  python3 - <<'PY' || return 1
import json, os
partial=os.path.abspath(os.environ["RBL_PARTIAL"])
request=json.load(open(os.environ["RBL_REQUEST"],encoding="utf-8"))
recipe=json.loads(os.environ["RBL_RECIPE_JSON"])
payloads=json.loads(os.environ["RBL_PAYLOADS_JSON"])
bundle={
  "format":"lagrange-rust-artifact-bundle-v1","recipe":os.environ["RBL_RECIPE"],
  "bins":recipe["bins"],"source_commit":os.environ["RBL_SOURCE_COMMIT"],
  "source_input_sha256":os.environ["RBL_SOURCE_HASH"],"recipe_sha256":os.environ["RBL_RECIPE_HASH"],
  "h_sha256":request["h_sha256"],
  "helper_sha256":os.environ["RBL_HELPER_HASH"],"layout_sha256":os.environ["RBL_LAYOUT_HASH"],
  "k_sha256":os.environ["RBL_K_HASH"],
  "cache_key":os.environ["RBL_CACHE_KEY"],"cache_namespace":os.environ["RBL_CACHE_NAMESPACE"],
  "guard_version":"common-1","platform":"linux/amd64","host_triple":"x86_64-unknown-linux-musl",
  "profile":"release","features":[],"compile_env":request["compile_env"],"payloads":payloads
}
path=os.path.join(partial,".release-build","bundle.json")
with open(path,"w",encoding="utf-8",newline="\n") as handle:
    handle.write(json.dumps(bundle,sort_keys=True,separators=(",",":"))+"\n")
os.chmod(path,0o600)
PY
  rbl_complete=$(rbl_bundle_tree_hash "$rbl_partial") || return 1
  printf '%s\n' "$rbl_complete" >"$rbl_partial/.release-build/complete"
  chmod 0600 "$rbl_partial/.release-build/complete"
  rbl_bundle_verify "$rbl_partial" "$rbl_recipe" >/dev/null || return 1
  mv -- "$rbl_partial" "$rbl_bundle_target" || return 1
  printf '%s\n' "$rbl_bundle_target"
}

rbl_bundle_verify() {
  rbl_bundle=$1
  rbl_recipe=$2
  rbl_layout=$RELEASE_BUILD_LAYOUT_SOURCE_ROOT/deploy/build/release-build-layout.json
  rbl_validate_layout "$rbl_layout" || return 1
  rbl_current_package_hashes=$(rbl_package_hashes "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT" "$rbl_layout" 1) || return 1
  rbl_current_recipe_hash=$(rbl_recipe_hash "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT" "$rbl_recipe" "$rbl_layout") || return 1
  [ -d "$rbl_bundle" ] && [ ! -L "$rbl_bundle" ] || return 1
  rbl_runtime_inventory_value=$(rbl_runtime_payload_inventory_current) || return 1
  rbl_runtime_payload_group=$(rbl_recipe_json "$rbl_recipe" "$rbl_layout" |
    python3 -c 'import json,sys; print(json.load(sys.stdin).get("runtime_payload",""))') || return 1
  rbl_verified_payloads=$(rbl_runtime_payload_verify_copy "$rbl_bundle" \
    "$rbl_runtime_payload_group" "$rbl_runtime_inventory_value") || return 1
  RBL_BUNDLE=$rbl_bundle RBL_RECIPE=$rbl_recipe RBL_LAYOUT=$rbl_layout \
  RBL_SOURCE_ROOT=$RELEASE_BUILD_LAYOUT_SOURCE_ROOT RBL_SOURCE_COMMIT=$RELEASE_BUILD_LAYOUT_COMMIT \
  RBL_SOURCE_HASH=$RELEASE_BUILD_LAYOUT_SOURCE_INPUT_SHA256 RBL_HELPER_HASH=$RELEASE_BUILD_LAYOUT_HELPER_SHA256 \
  RBL_LAYOUT_HASH=$RELEASE_BUILD_LAYOUT_CONFIG_SHA256 RBL_PACKAGE_HASHES=$rbl_current_package_hashes \
  RBL_RECIPE_HASH=$rbl_current_recipe_hash RBL_VERIFIED_PAYLOADS=$rbl_verified_payloads \
  python3 - <<'PY' || return 1
import hashlib, json, os, re, stat
bundle=os.path.abspath(os.environ["RBL_BUNDLE"])
layout=json.load(open(os.environ["RBL_LAYOUT"],encoding="utf-8"))
recipe=layout["recipes"].get(os.environ["RBL_RECIPE"])
if recipe is None: raise SystemExit("bundle-recipe-unknown")
def pairs(items):
    result={}
    for key,value in items:
        if key in result: raise ValueError("duplicate-json-key")
        result[key]=value
    return result
def reject(v): raise ValueError("non-finite-json-number")
def regular(path,mode=None):
    info=os.lstat(path)
    return stat.S_ISREG(info.st_mode) and not stat.S_ISLNK(info.st_mode) and (mode is None or stat.S_IMODE(info.st_mode)==mode)
def directory(path):
    info=os.lstat(path)
    return stat.S_ISDIR(info.st_mode) and not stat.S_ISLNK(info.st_mode)
release=os.path.join(bundle,".release-build")
if not directory(release) or not directory(os.path.join(bundle,"target")) or not directory(os.path.join(bundle,"target/release")):
    raise SystemExit("bundle-required-directory-missing")
allowed={"bundle.json","complete","native-identity.json","release-build-layout.json","release-build-layout.sh","request.json","producer"}
if set(os.listdir(release)) != allowed: raise SystemExit("bundle-release-entries-invalid")
for name,mode in (("bundle.json",0o600),("complete",0o600),("native-identity.json",0o600),("request.json",0o600),("release-build-layout.json",0o644),("release-build-layout.sh",0o755)):
    if not regular(os.path.join(release,name),mode): raise SystemExit("bundle-release-file-invalid")
request_raw=open(os.path.join(release,"request.json"),"rb").read()
bundle_raw=open(os.path.join(release,"bundle.json"),"rb").read()
request=json.loads(request_raw.decode("utf-8"),object_pairs_hook=pairs,parse_constant=reject)
record=json.loads(bundle_raw.decode("utf-8"),object_pairs_hook=pairs,parse_constant=reject)
if request_raw != (json.dumps(request,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8") or bundle_raw != (json.dumps(record,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8"):
    raise SystemExit("bundle-json-not-canonical")
required={"bins","cache_key","cache_namespace","compile_env","features","format","guard_version","h_sha256","helper_sha256","host_triple","k_sha256","layout_sha256","payloads","platform","profile","recipe","recipe_sha256","source_commit","source_input_sha256"}
if set(record) != required: raise SystemExit("bundle-json-keys-invalid")
if record["format"]!="lagrange-rust-artifact-bundle-v1" or record["recipe"]!=os.environ["RBL_RECIPE"] or record["source_commit"]!=os.environ["RBL_SOURCE_COMMIT"] or record["source_input_sha256"]!=os.environ["RBL_SOURCE_HASH"] or record["helper_sha256"]!=os.environ["RBL_HELPER_HASH"] or record["layout_sha256"]!=os.environ["RBL_LAYOUT_HASH"] or record["recipe_sha256"]!=os.environ["RBL_RECIPE_HASH"]:
    raise SystemExit("bundle-binding-invalid")
if record["k_sha256"]!=request.get("k_sha256") or record["cache_key"]!=request.get("cache_key"):
    raise SystemExit("bundle-cache-binding-invalid")
if record["h_sha256"]!=request.get("h_sha256"):
    raise SystemExit("bundle-h-binding-invalid")
if record["bins"]!=recipe["bins"] or record["platform"]!="linux/amd64" or record["host_triple"]!="x86_64-unknown-linux-musl" or record["profile"]!="release" or record["features"]!=[] or record["compile_env"]!=request["compile_env"]:
    raise SystemExit("bundle-compiler-binding-invalid")
request_keys={"bins","cache_key","cache_namespace","clean_packages","compile_commit","compile_env","features","format","guard_version","h_sha256","helper_sha256","host_triple","input_sha256","k_sha256","layout_sha256","package","package_hashes","payloads","platform","profile","recipe","recipe_sha256","resolution_inputs","source_commit","transitive_package_hashes"}
if set(request) != request_keys or request.get("format")!="lagrange-rust-artifact-request-v1" or request.get("recipe")!=os.environ["RBL_RECIPE"] or request.get("source_commit")!=os.environ["RBL_SOURCE_COMMIT"] or request.get("input_sha256")!=os.environ["RBL_SOURCE_HASH"] or request.get("recipe_sha256")!=os.environ["RBL_RECIPE_HASH"]:
    raise SystemExit("bundle-request-invalid")
expected_hashes=json.loads(os.environ["RBL_PACKAGE_HASHES"])
if request.get("package_hashes") != expected_hashes or set(expected_hashes) != set(layout["packages"]):
    raise SystemExit("bundle-package-hashes-invalid")
def closure(name,visiting=None,result=None):
    if visiting is None: visiting=set()
    if result is None: result=set()
    if name in visiting: raise SystemExit("bundle-package-cycle")
    if name in result: return result
    visiting.add(name)
    for dependency in layout["packages"][name]["local_dependencies"]: closure(dependency,visiting,result)
    visiting.remove(name); result.add(name)
    return result
if request["package"] != recipe["package"] or request["bins"] != recipe["bins"] or request["compile_commit"] != recipe["compile_commit"] or request["clean_packages"] != recipe["clean_packages"]:
    raise SystemExit("bundle-recipe-request-invalid")
transitive=[request["package"]]+sorted(closure(request["package"])-{request["package"]})
if request["transitive_package_hashes"] != [{"package":name,"p_sha256":expected_hashes[name]} for name in transitive]:
    raise SystemExit("bundle-transitive-p-invalid")
resolution={"argv":["cargo","build","--locked","--release","--package",request["package"],"--bin"],"default_features":True,"locked":True,"message_format":"json-render-diagnostics","verbose":True}
if request["resolution_inputs"] != resolution: raise SystemExit("bundle-resolution-input-invalid")
expected_env={"CARGO_BUILD_JOBS":"2","CARGO_TARGET_DIR":"/cargo-target","RUSTFLAGS":"<unset>","LAGRANGE_CODE_COMMIT":request["source_commit"] if request["compile_commit"]=="present" else "<unset>"}
if request["compile_env"] != expected_env: raise SystemExit("bundle-compile-env-invalid")
h_material={"format":"lagrange-build-layout-h-v1","k_sha256":request["k_sha256"],"package":request["package"],"bins":request["bins"],"recipe":request["recipe"],"recipe_sha256":request["recipe_sha256"],"transitive_package_hashes":request["transitive_package_hashes"],"resolution_inputs":request["resolution_inputs"],"compile_env":request["compile_env"],"features":request["features"],"profile":request["profile"],"platform":request["platform"],"host_triple":request["host_triple"]}
if not re.fullmatch(r"[0-9a-f]{64}",request["h_sha256"]) or request["h_sha256"] != hashlib.sha256(json.dumps(h_material,sort_keys=True,separators=(",",":")).encode()).hexdigest():
    raise SystemExit("bundle-h-invalid")
if set(os.listdir(os.path.join(bundle,"target"))) != {"release"} or set(os.listdir(os.path.join(bundle,"target","release"))) != set(recipe["bins"]):
    raise SystemExit("bundle-target-entries-invalid")
if not directory(os.path.join(release,"producer")) or set(os.listdir(os.path.join(release,"producer"))) != set(recipe["bins"]):
    raise SystemExit("bundle-producer-entries-invalid")
verified_payloads=json.loads(os.environ["RBL_VERIFIED_PAYLOADS"])
expected_payload_descriptors=[{"source":item["source"],"image":item["image"]}
                              for item in layout["runtime_payloads"].get(recipe.get("runtime_payload",""),[])]
if ([{"source":item["source"],"image":item["image"]} for item in verified_payloads]
        != expected_payload_descriptors or record.get("payloads")!=verified_payloads):
    raise SystemExit("bundle-payload-record-invalid")
allowed_roots={".release-build","target"}|{item["source"].split("/",1)[0] for item in verified_payloads}
if set(os.listdir(bundle)) != allowed_roots: raise SystemExit("bundle-root-entries-invalid")
for rel in ("release-build-layout.sh","release-build-layout.json"):
    digest=__import__("hashlib").sha256(open(os.path.join(release,rel),"rb").read()).hexdigest()
    if digest != (record["helper_sha256"] if rel.endswith(".sh") else record["layout_sha256"]):
        raise SystemExit("bundle-tool-hash-invalid")
complete=open(os.path.join(release,"complete"),"r",encoding="ascii").read().strip()
if not re.fullmatch(r"[0-9a-f]{64}",complete): raise SystemExit("bundle-complete-invalid")
PY
  rbl_validate_native_identity "$rbl_bundle/.release-build/native-identity.json" || return 1
  rbl_request=$rbl_bundle/.release-build/request.json
  for rbl_bin in $(rbl_recipe_json "$rbl_recipe" "$rbl_layout" |
      python3 -c 'import json,sys; print("\n".join(json.load(sys.stdin)["bins"]))'); do
    rbl_verify_producer "$rbl_bundle/.release-build/producer/$rbl_bin" "$rbl_recipe" "$rbl_bin" "$rbl_request" || return 1
    cmp -s -- "$rbl_bundle/.release-build/producer/$rbl_bin/bin/$rbl_bin" \
      "$rbl_bundle/target/release/$rbl_bin" || return 1
  done
  rbl_expected_complete=$(rbl_bundle_tree_hash "$rbl_bundle") || return 1
  [ "$(tr -d '\n' <"$rbl_bundle/.release-build/complete")" = "$rbl_expected_complete" ] ||
    {
      rbl_die 'bundle complete marker does not match bundle contents'
      return 1
    }
  rbl_expected_digest=$(rbl_tree_hash "$rbl_bundle") || return 1
  printf '%s\n' "$rbl_expected_digest"
}

release_build_layout_prepare() {
  [ "${RELEASE_BUILD_LAYOUT_INITIALIZED:-0}" = 1 ] || {
    rbl_die 'release-build-layout-init must run before prepare'
    return 1
  }
  rbl_service=$1
  rbl_commit=$2
  rbl_state_root=$3
  [ "$rbl_commit" = "$RELEASE_BUILD_LAYOUT_COMMIT" ] ||
    {
      rbl_die 'prepare commit does not match initialized source commit'
      return 1
    }
  [ "$rbl_state_root" = "$RELEASE_BUILD_LAYOUT_STATE_ROOT" ] ||
    {
      rbl_die 'prepare state root does not match initialized state'
      return 1
    }
  rbl_recipe=$(rbl_recipe_for_service "$rbl_service" "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT/deploy/build/release-build-layout.json") || return 1
  if [ "$rbl_recipe" = NONE ]; then
    case "$rbl_service" in
      db-role-bootstrap|db-migrate|web) printf '%s\n' NONE; return 0 ;;
      *) rbl_die "service unexpectedly has no producer recipe: $rbl_service"; return 1 ;;
    esac
  fi
  release_build_layout_gate "prepare:$rbl_service" 0 || return 1
  # Build the first transport with a pending K only to reach the native stage.
  # No producer is allowed to start until the measured P0 identity is bound and
  # the context/request have been regenerated with the actual K.
  rbl_context=$(rbl_make_context "$rbl_recipe" "$rbl_state_root") || return 1
  rbl_produce_native "$rbl_context" "$rbl_state_root" || return 1
  rbl_context=$(rbl_make_context "$rbl_recipe" "$rbl_state_root") || return 1
  release_build_layout_gate "native:$rbl_recipe" 0 || return 1
  rbl_index=0
  for rbl_bin in $(rbl_recipe_json "$rbl_recipe" "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT/deploy/build/release-build-layout.json" |
      python3 -c 'import json,sys; print("\n".join(json.load(sys.stdin)["bins"]))'); do
    rbl_index=$((rbl_index + 1))
    rbl_produce_bin "$rbl_recipe" "$rbl_bin" "$rbl_context" "$rbl_state_root" >/dev/null || return 1
    release_build_layout_gate "producer:$rbl_recipe:$rbl_bin" 0 || return 1
    case "$rbl_recipe:$rbl_index" in
      D6:3|D6:6|D6:8|D6:10)
        release_build_layout_gate "collector-checkpoint:$rbl_recipe:$rbl_index" 0 || return 1 ;;
    esac
  done
  rbl_bundle_create "$rbl_recipe" "$rbl_state_root" >/dev/null || return 1
  rbl_bundle_verify "$rbl_state_root/bundles/$rbl_recipe" "$rbl_recipe" >/dev/null || return 1
  printf '%s\n' "$rbl_state_root/bundles/$rbl_recipe"
}

release_build_layout_verify_bundle() {
  rbl_service=$1
  rbl_commit=$2
  rbl_bundle=$3
  [ "${RELEASE_BUILD_LAYOUT_INITIALIZED:-0}" = 1 ] || {
    rbl_die 'release-build-layout-init must run before verify-bundle'
    return 1
  }
  [ "$rbl_commit" = "$RELEASE_BUILD_LAYOUT_COMMIT" ] ||
    {
      rbl_die 'bundle commit does not match initialized source commit'
      return 1
    }
  rbl_recipe=$(rbl_recipe_for_service "$rbl_service" "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT/deploy/build/release-build-layout.json") || return 1
  [ "$rbl_recipe" != NONE ] || {
    rbl_die 'verify-bundle is only valid for a Rust service'
    return 1
  }
  rbl_bundle_verify "$rbl_bundle" "$rbl_recipe" || return 1
}

release_build_layout_write_override() {
  rbl_service=$1
  rbl_bundle=$2
  rbl_new_file=$3
  [ "${RELEASE_BUILD_LAYOUT_INITIALIZED:-0}" = 1 ] || {
    rbl_die 'release-build-layout-init must run before write-override'
    return 1
  }
  rbl_recipe=$(rbl_recipe_for_service "$rbl_service" "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT/deploy/build/release-build-layout.json") || return 1
  [ "$rbl_recipe" != NONE ] || {
    rbl_die 'write-override is only valid for a Rust service'
    return 1
  }
  rbl_bundle_verify "$rbl_bundle" "$rbl_recipe" >/dev/null || return 1
  [ ! -e "$rbl_new_file" ] && [ ! -L "$rbl_new_file" ] || {
    rbl_die 'override output already exists'
    return 1
  }
  rbl_parent=$(dirname -- "$rbl_new_file")
  [ -d "$rbl_parent" ] && [ ! -L "$rbl_parent" ] || {
    rbl_die 'override parent is not a directory'
    return 1
  }
  RBL_OVERRIDE=$rbl_new_file RBL_SERVICE=$rbl_service RBL_BUNDLE=$rbl_bundle \
  RBL_COMMIT=$RELEASE_BUILD_LAYOUT_COMMIT RBL_HELPER=$RELEASE_BUILD_LAYOUT_HELPER_SHA256 \
  RBL_BUNDLE_HASH=$(rbl_tree_hash "$rbl_bundle") python3 - <<'PY'
import json, os, re
service=os.environ["RBL_SERVICE"]
if not re.fullmatch(r"[a-z0-9][a-z0-9_.-]{0,127}",service): raise SystemExit("override-service-invalid")
values={
  "LAGRANGE_CODE_COMMIT":os.environ["RBL_COMMIT"],
  "RUST_ARTIFACT_SOURCE":"verified-artifacts",
  "RUST_ARTIFACT_HELPER_SHA256":os.environ["RBL_HELPER"],
  "RUST_ARTIFACT_BUNDLE_SHA256":os.environ["RBL_BUNDLE_HASH"],
}
value={"services":{service:{"build":{"args":values,"additional_contexts":{"release_artifacts":os.environ["RBL_BUNDLE"]}}}}}
data=(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8")
path=os.environ["RBL_OVERRIDE"]
fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
with os.fdopen(fd,"wb") as handle:
    handle.write(data); handle.flush(); os.fsync(handle.fileno())
PY
}

release_build_layout_gate() {
  [ "${RELEASE_BUILD_LAYOUT_INITIALIZED:-0}" = 1 ] || {
    rbl_die 'release-build-layout-init must run before gate'
    return 1
  }
  [ "$#" -eq 2 ] || {
    rbl_die 'gate needs exactly label and previous exit'
    return 1
  }
  rbl_gate_label=$1
  rbl_gate_previous=$2
  case "$rbl_gate_label" in
    ''|*[!A-Za-z0-9_.:-]*) rbl_die 'gate label is not canonical'; return 1 ;;
  esac
  case "$rbl_gate_previous" in
    ''|*[!0-9]*) rbl_die 'gate previous exit is not numeric'; return 1 ;;
  esac
  RBL_GATE_LABEL=$rbl_gate_label RBL_GATE_PREVIOUS=$rbl_gate_previous \
  RBL_GATE_STATE=$RELEASE_BUILD_LAYOUT_STATE_ROOT python3 - <<'PY'
import datetime, hashlib, json, os, posixpath, re, selectors, stat, subprocess, tempfile, time

state_root=os.path.abspath(os.environ["RBL_GATE_STATE"])
label=os.environ["RBL_GATE_LABEL"]
previous=int(os.environ["RBL_GATE_PREVIOUS"])
gate_dir=os.path.join(state_root,"gates")
records_path=os.path.join(gate_dir,"gates.jsonl")
state_path=os.path.join(gate_dir,"gate-state.json")

class GateFailure(Exception):
    pass

def pairs(items):
    value={}
    for key,item in items:
        if key in value: raise ValueError("duplicate-json-key")
        value[key]=item
    return value

def reject_constant(value):
    raise ValueError("non-finite-json-number")

def canonical(value):
    return (json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8")

def owned_directory(path):
    info=os.lstat(path)
    if (stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode) or
            info.st_uid!=os.geteuid() or stat.S_IMODE(info.st_mode)!=0o700):
        raise ValueError("private-directory-unsafe")

def owned_regular(path,limit):
    if (not path.startswith("/") or path.startswith("//") or os.path.normpath(path)!=path or
            os.path.realpath(path)!=path or "\x00" in path):
        raise ValueError("private-path-invalid")
    before=os.lstat(path)
    if (stat.S_ISLNK(before.st_mode) or not stat.S_ISREG(before.st_mode) or
            before.st_uid!=os.geteuid() or stat.S_IMODE(before.st_mode)!=0o600):
        raise ValueError("private-file-unsafe")
    flags=os.O_RDONLY|getattr(os,"O_NOFOLLOW",0)
    fd=os.open(path,flags)
    try:
        opened=os.fstat(fd)
        if ((opened.st_dev,opened.st_ino)!=(before.st_dev,before.st_ino) or
                not stat.S_ISREG(opened.st_mode) or opened.st_uid!=os.geteuid() or
                stat.S_IMODE(opened.st_mode)!=0o600):
            raise ValueError("private-file-raced")
        with os.fdopen(fd,"rb",closefd=False) as handle:
            raw=handle.read(limit+1)
        after=os.fstat(fd)
        if len(raw)>limit:
            raise ValueError("private-file-too-large")
        if (before.st_size,before.st_mtime_ns,before.st_ctime_ns)!=(after.st_size,after.st_mtime_ns,after.st_ctime_ns):
            raise ValueError("private-file-changed")
        return raw
    finally:
        os.close(fd)

def command_empty():
    return {
        "exit":None,"timeout":False,"limit":False,"stdout_eof":False,"stderr_eof":False,
        "child_reaped":False,"complete":False,"stop_requested":False,"failure":"not-run",
        "elapsed_ns":0,"stdout_bytes":0,"stderr_bytes":0,"line_count":0,
        "record_count":0,"max_line_bytes":0,"partial_final_line_bytes":0,
        "stdout":b"","stderr":b""
    }

def command_evidence(result):
    return {
        "exit":result["exit"],"timeout":result["timeout"],"limit":result["limit"],
        "stdout_eof":result["stdout_eof"],"stderr_eof":result["stderr_eof"],
        "child_reaped":result["child_reaped"],"complete":result["complete"],
        "stop_requested":result["stop_requested"],"failure":result["failure"],
        "elapsed_ns":result["elapsed_ns"],"stdout_bytes":result["stdout_bytes"],
        "stderr_bytes":result["stderr_bytes"],"line_count":result["line_count"],
        "record_count":result["record_count"],"max_line_bytes":result["max_line_bytes"],
        "partial_final_line_bytes":result["partial_final_line_bytes"],
        "stdout_sha256":hashlib.sha256(result["stdout"]).hexdigest(),
        "stderr_sha256":hashlib.sha256(result["stderr"]).hexdigest()
    }

empty_command=command_evidence(command_empty())
evidence={
    "mem_available_kib":"unknown","swap_free_kib":"unknown",
    "process_scan":empty_command,
    "build_unit":{"manager":"unknown","selected":{},"command":empty_command},
    "units":{},"containers":{},
    "journal":{
        "status":"unknown","boot_id":"unknown","since":"unknown","since_us":"unknown",
        "until":"unknown","count":"unknown","oom_count":"unknown",
        "probe":empty_command,"range":empty_command
    }
}

def append_record(value):
    data=canonical(value)
    flags=os.O_WRONLY|os.O_APPEND|os.O_CREAT|getattr(os,"O_NOFOLLOW",0)
    fd=os.open(records_path,flags,0o600)
    try:
        info=os.fstat(fd)
        if (not stat.S_ISREG(info.st_mode) or info.st_uid!=os.geteuid() or
                stat.S_IMODE(info.st_mode)!=0o600):
            raise ValueError("gate-record-unsafe")
        offset=0
        while offset<len(data):
            wrote=os.write(fd,data[offset:])
            if wrote<=0: raise OSError("gate-record-short-write")
            offset+=wrote
        os.fsync(fd)
    finally:
        os.close(fd)

def fail(reason):
    value={
        "format":"lagrange-build-gate-record-v2","label":label,
        "previous_exit":previous,"status":"FAIL","reason":reason,
        "time_unix_ns":time.time_ns(),"evidence":evidence
    }
    append_record(value)
    raise GateFailure()

def record_pass():
    value={
        "format":"lagrange-build-gate-record-v2","label":label,
        "previous_exit":previous,"status":"PASS",
        "reason":("image-build-only-known-incident" if "research_exception" in evidence else "healthy"),
        "time_unix_ns":time.time_ns(),"evidence":evidence
    }
    append_record(value)

def valid_list(raw,kind):
    if not raw or re.search(r"\s",raw) or raw.startswith(",") or raw.endswith(",") or ",," in raw:
        raise ValueError("invalid-"+kind+"-list")
    values=raw.split(",")
    if not 1<=len(values)<=16 or len(values)!=len(set(values)):
        raise ValueError("invalid-"+kind+"-list-count")
    pattern=(r"[A-Za-z0-9][A-Za-z0-9_.:@-]*\.service" if kind=="unit"
             else r"[A-Za-z0-9][A-Za-z0-9_.-]*")
    if any(not re.fullmatch(pattern,item) for item in values):
        raise ValueError("invalid-"+kind+"-name")
    if kind=="container" and "lagrange-station-research-worker-1" not in values:
        raise ValueError("research-worker-container-missing")
    return values

def normalized_boot(value):
    if not isinstance(value,str): raise ValueError("boot-id-type")
    value=value.replace("-","").lower()
    if not re.fullmatch(r"[0-9a-f]{32}",value): raise ValueError("boot-id-invalid")
    return value

def decimal(value,positive=False):
    if not isinstance(value,str) or not re.fullmatch(r"[0-9]+",value):
        raise ValueError("decimal-invalid")
    result=int(value)
    if positive and result<=0: raise ValueError("decimal-not-positive")
    return result

def timestamp(text):
    if not isinstance(text,str) or not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z",text):
        raise ValueError("timestamp-invalid")
    return int(datetime.datetime.strptime(text,"%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=datetime.timezone.utc).timestamp())

def utc_text(microseconds):
    # Query exactly the interval that parse_range validates, including the
    # fractional first and last seconds. Avoid a floating-point round trip.
    seconds, fraction = divmod(microseconds, 1000000)
    return datetime.datetime.fromtimestamp(seconds,datetime.timezone.utc).replace(
        microsecond=fraction).strftime("%Y-%m-%d %H:%M:%S.%f UTC")

def secure_exception(path,now_ns):
    if not path:
        return None
    raw=owned_regular(path,65536)
    value=json.loads(raw.decode("utf-8"),object_pairs_hook=pairs,parse_constant=reject_constant)
    if raw!=canonical(value):
        raise ValueError("research-exception-not-canonical")
    fixed={"format":"lagrange-build-research-exception-v1","scope":"image-build-only",
           "container_name":"lagrange-station-research-worker-1","known_error_code":"PRICE_CURATION_FAILED"}
    keys=set(fixed)|{"container_id","image_id","observed_at_utc","expires_at_utc","initial_restart_count","known_exit_code"}
    if not isinstance(value,dict) or set(value)!=keys or any(value.get(key)!=item for key,item in fixed.items()):
        raise ValueError("research-exception-contract")
    if (not isinstance(value["container_id"],str) or not re.fullmatch(r"[0-9a-f]{64}",value["container_id"]) or
            not isinstance(value["image_id"],str) or not re.fullmatch(r"sha256:[0-9a-f]{64}",value["image_id"])):
        raise ValueError("research-exception-identity")
    if type(value["initial_restart_count"]) is not int or value["initial_restart_count"]<0:
        raise ValueError("research-exception-count")
    if type(value["known_exit_code"]) is not int or value["known_exit_code"]!=2:
        raise ValueError("research-exception-exit")
    observed=timestamp(value["observed_at_utc"])
    expires=timestamp(value["expires_at_utc"])
    if not observed*1000000000<=now_ns<expires*1000000000 or expires>observed+86400:
        raise ValueError("research-exception-expired")
    binding={"path":path,"sha256":hashlib.sha256(raw).hexdigest(),"fields":value}
    return {"binding":binding,"observed_ns":observed*1000000000,"expires_ns":expires*1000000000}

def run_bounded(argv,stdout_limit=65536,stderr_limit=65536,deadline_seconds=10,
                line_limit=None,record_limit=None):
    result=command_empty()
    result["failure"]=None
    started=time.monotonic_ns()
    deadline=started+deadline_seconds*1000000000
    child=None
    selector=selectors.DefaultSelector()
    streams={}
    chunks={"stdout":bytearray(),"stderr":bytearray()}
    line_bytes=0
    line_nonblank=False

    def remaining():
        return max(0.0,(deadline-time.monotonic_ns())/1000000000)

    def set_failure(reason):
        if result["failure"] is None:
            result["failure"]=reason

    def consume_stdout(data):
        nonlocal line_bytes,line_nonblank
        if line_limit is None:
            return
        parts=data.split(b"\n")
        for index,part in enumerate(parts):
            ended=index<len(parts)-1
            line_bytes+=len(part)+int(ended)
            line_nonblank=line_nonblank or bool(part.strip())
            result["max_line_bytes"]=max(result["max_line_bytes"],line_bytes)
            result["partial_final_line_bytes"]=line_bytes
            if line_bytes>=line_limit:
                result["limit"]=True
                set_failure("line-byte-limit")
                return
            if ended:
                result["line_count"]+=1
                if line_nonblank:
                    result["record_count"]+=1
                    if record_limit is not None and result["record_count"]>=record_limit:
                        result["limit"]=True
                        set_failure("record-limit")
                        return
                line_bytes=0
                line_nonblank=False
                result["partial_final_line_bytes"]=0

    try:
        child=subprocess.Popen(argv,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE,
                               env={**os.environ,"LC_ALL":"C"})
        for name,stream in (("stdout",child.stdout),("stderr",child.stderr)):
            os.set_blocking(stream.fileno(),False)
            selector.register(stream,selectors.EVENT_READ,name)
            streams[name]=stream
        while selector.get_map() and result["failure"] is None:
            if time.monotonic_ns()>=deadline:
                result["timeout"]=True
                set_failure("deadline")
                break
            events=selector.select(remaining())
            if not events:
                if time.monotonic_ns()>=deadline:
                    result["timeout"]=True
                    set_failure("deadline")
                continue
            for key,_ in events:
                name=key.data
                allowance=(stdout_limit if name=="stdout" else stderr_limit)-len(chunks[name])
                if allowance<=0:
                    result["limit"]=True
                    set_failure(name+"-byte-limit")
                    break
                try:
                    data=os.read(key.fd,min(65536,allowance))
                except BlockingIOError:
                    continue
                except OSError:
                    set_failure("capture-io")
                    break
                if not data:
                    result[name+"_eof"]=True
                    selector.unregister(key.fileobj)
                    key.fileobj.close()
                    continue
                chunks[name].extend(data)
                if len(chunks[name])>= (stdout_limit if name=="stdout" else stderr_limit):
                    result["limit"]=True
                    set_failure(name+"-byte-limit")
                    break
                if name=="stdout":
                    consume_stdout(data)
                    if result["failure"] is not None:
                        break
        if result["failure"] is None:
            if not result["stdout_eof"] or not result["stderr_eof"]:
                set_failure("incomplete-stream")
            elif line_limit is not None and line_bytes:
                set_failure("partial-final-record")
            else:
                remain=remaining()
                if remain<=0:
                    result["timeout"]=True
                    set_failure("deadline")
                else:
                    try:
                        child.wait(timeout=remain)
                    except subprocess.TimeoutExpired:
                        result["timeout"]=True
                        set_failure("deadline")
    except OSError:
        set_failure("capture-spawn-or-io")
    finally:
        if child is not None and child.poll() is None:
            result["stop_requested"]=True
            try:
                child.kill()
                child.wait(timeout=remaining())
            except (OSError,subprocess.TimeoutExpired):
                pass
        for stream in streams.values():
            try: stream.close()
            except OSError: pass
        selector.close()
        if child is not None:
            result["exit"]=child.returncode
            result["child_reaped"]=child.returncode is not None
        if not result["child_reaped"]:
            set_failure("termination-unproven")
        result["elapsed_ns"]=time.monotonic_ns()-started
        if result["elapsed_ns"]>=deadline_seconds*1000000000:
            result["timeout"]=True
            set_failure("deadline")
        result["stdout"]=bytes(chunks["stdout"])
        result["stderr"]=bytes(chunks["stderr"])
        result["stdout_bytes"]=len(result["stdout"])
        result["stderr_bytes"]=len(result["stderr"])
        result["complete"]=(result["failure"] is None and result["stdout_eof"] and
                            result["stderr_eof"] and result["child_reaped"] and
                            not result["timeout"] and not result["limit"])
    return result

def command_ok(result):
    return result["complete"] and result["exit"]==0 and result["stderr_bytes"]==0

def systemd_show(manager,name,properties):
    command=["systemctl"]+(["--user"] if manager=="user" else [])+["show",name,"--property="+",".join(properties)]
    result=run_bounded(command)
    if not command_ok(result):
        raise ValueError("systemd-command-invalid")
    values={}
    try:
        lines=result["stdout"].decode("utf-8").splitlines()
    except UnicodeDecodeError:
        raise ValueError("systemd-output-encoding")
    for line in lines:
        if "=" not in line: raise ValueError("systemd-output-line")
        key,value=line.split("=",1)
        if key in values or key not in properties: raise ValueError("systemd-output-keys")
        values[key]=value
    if set(values)!=set(properties): raise ValueError("systemd-output-property-set")
    return values,command_evidence(result)

def in_build_cgroup(expected):
    if (not isinstance(expected,str) or expected=="/" or not expected.startswith("/") or
            expected.startswith("//") or posixpath.normpath(expected)!=expected or
            "/../" in expected or expected.endswith("/..")):
        raise ValueError("build-cgroup-invalid")
    found=False
    for raw in open("/proc/self/cgroup",encoding="ascii"):
        fields=raw.rstrip("\n").rsplit(":",1)
        if len(fields)!=2: raise ValueError("build-cgroup-row")
        observed=fields[1]
        if not observed.startswith("/") or posixpath.normpath(observed)!=observed:
            raise ValueError("build-cgroup-row")
        if observed==expected or observed.startswith(expected+"/"):
            found=True
    return found

def parse_probe(raw,boot):
    try:
        text=raw.decode("utf-8")
    except UnicodeDecodeError:
        raise ValueError("probe-encoding")
    if not text.endswith("\n") or text.count("\n")!=1:
        raise ValueError("probe-count")
    value=json.loads(text,object_pairs_hook=pairs,parse_constant=reject_constant)
    required={"__REALTIME_TIMESTAMP","__CURSOR","_BOOT_ID","_TRANSPORT","MESSAGE"}
    if not isinstance(value,dict) or not required.issubset(value):
        raise ValueError("probe-fields")
    if (not isinstance(value["__REALTIME_TIMESTAMP"],str) or
            not re.fullmatch(r"[0-9]+",value["__REALTIME_TIMESTAMP"]) or
            int(value["__REALTIME_TIMESTAMP"])<=0 or
            not isinstance(value["__CURSOR"],str) or not value["__CURSOR"] or
            normalized_boot(value["_BOOT_ID"])!=boot or value["_TRANSPORT"]!="kernel" or
            not isinstance(value["MESSAGE"],str)):
        raise ValueError("probe-contract")

def parse_range(raw,boot,since_us,until_us,capture_count):
    try:
        text=raw.decode("utf-8")
    except UnicodeDecodeError:
        raise ValueError("range-encoding")
    if text and not text.endswith("\n"):
        raise ValueError("range-partial-final-record")
    count=0
    oom=0
    pattern=re.compile(r"out of memory|oom[-_ ]?kill|killed process",re.I)
    for raw_line in text.splitlines(keepends=True):
        if not raw_line.endswith("\n") or not raw_line.strip():
            raise ValueError("range-jsonl-line")
        value=json.loads(raw_line,object_pairs_hook=pairs,parse_constant=reject_constant)
        required={"__REALTIME_TIMESTAMP","__CURSOR","_BOOT_ID","_TRANSPORT","MESSAGE"}
        if not isinstance(value,dict) or not required.issubset(value):
            raise ValueError("range-fields")
        stamp=value["__REALTIME_TIMESTAMP"]
        if (not isinstance(stamp,str) or not re.fullmatch(r"[0-9]+",stamp) or int(stamp)<=0 or
                not isinstance(value["__CURSOR"],str) or not value["__CURSOR"] or
                normalized_boot(value["_BOOT_ID"])!=boot or value["_TRANSPORT"]!="kernel" or
                not isinstance(value["MESSAGE"],str)):
            raise ValueError("range-contract")
        point=int(stamp)
        if point<since_us or point>until_us:
            raise ValueError("range-outside-window")
        count+=1
        if count>=100000:
            raise ValueError("range-record-limit")
        if pattern.search(value["MESSAGE"]):
            oom+=1
    if count!=capture_count:
        raise ValueError("range-capture-count-mismatch")
    return count,oom

def parse_container(raw,name,exception,now_ns):
    try:
        text=raw.decode("utf-8")
    except UnicodeDecodeError:
        raise ValueError("container-encoding")
    if not text.endswith("\n") or text.count("\n")!=1:
        raise ValueError("container-line")
    fields=text[:-1].split("\t")
    if len(fields)!=9:
        raise ValueError("container-columns")
    ident,running,restarting,oom,health,restarts,project,image_id,exit_code=fields
    if (not re.fullmatch(r"[0-9a-f]{64}",ident) or running not in ("true","false") or
            restarting not in ("true","false") or oom not in ("true","false") or
            not re.fullmatch(r"[0-9]+",restarts) or project!="lagrange-station" or
            not re.fullmatch(r"sha256:[0-9a-f]{64}",image_id) or not re.fullmatch(r"[0-9]+",exit_code)):
        raise ValueError("container-fields")
    restart_count=int(restarts)
    is_research=(exception is not None and name=="lagrange-station-research-worker-1")
    if is_research:
        grant=exception["binding"]["fields"]
        interval=30*1000000000
        elapsed=now_ns-exception["observed_ns"]
        if elapsed<0: raise ValueError("research-time")
        restart_limit=grant["initial_restart_count"]+(elapsed+interval-1)//interval+2
        if (ident!=grant["container_id"] or image_id!=grant["image_id"] or running!="true" or
                oom!="false" or health not in ("healthy","unhealthy","starting") or
                (restarting=="true" and exit_code!="2") or
                (restarting=="false" and exit_code not in ("0","2")) or
                not grant["initial_restart_count"]<=restart_count<=restart_limit):
            raise ValueError("research-container-invalid")
        return {
            "id":ident,"running":running,"restarting":restarting,"oom_killed":oom,
            "health_status":health,"restart_count":restart_count,"project":project,
            "image_id":image_id,"exit_code":int(exit_code),"monitored_at_unix_ns":now_ns,
            "monitored_at_utc":datetime.datetime.fromtimestamp(now_ns//1000000000,datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "restart_limit":restart_limit
        }
    if (running,restarting,oom,health,exit_code)!=("true","false","false","healthy","0"):
        raise ValueError("container-health-invalid")
    return {
        "id":ident,"running":running,"restarting":restarting,"oom_killed":oom,
        "health_status":health,"restart_count":restart_count,"project":project,
        "image_id":image_id,"exit_code":0
    }

def state_sample_valid(sample,exception):
    required={"id","running","restarting","oom_killed","health_status","restart_count","project",
              "image_id","exit_code","monitored_at_unix_ns","monitored_at_utc","restart_limit"}
    if not isinstance(sample,dict) or set(sample)!=required:
        raise ValueError("research-state-schema")
    grant=exception["binding"]["fields"]
    if sample["id"]!=grant["container_id"] or sample["image_id"]!=grant["image_id"]:
        raise ValueError("research-state-identity")
    if (sample["running"]!="true" or sample["oom_killed"]!="false" or
            sample["health_status"] not in ("healthy","unhealthy","starting") or
            sample["restarting"] not in ("true","false") or
            sample["project"]!="lagrange-station" or
            type(sample["restart_count"]) is not int or type(sample["restart_limit"]) is not int or
            type(sample["exit_code"]) is not int or type(sample["monitored_at_unix_ns"]) is not int or
            not isinstance(sample["monitored_at_utc"],str) or
            sample["monitored_at_unix_ns"]<exception["observed_ns"] or
            sample["monitored_at_unix_ns"]>=exception["expires_ns"]):
        raise ValueError("research-state-fields")
    expected_utc=datetime.datetime.fromtimestamp(
        sample["monitored_at_unix_ns"]//1000000000,datetime.timezone.utc
    ).strftime("%Y-%m-%dT%H:%M:%SZ")
    if sample["monitored_at_utc"]!=expected_utc:
        raise ValueError("research-state-time")
    if ((sample["restarting"]=="true" and sample["exit_code"]!=2) or
            (sample["restarting"]=="false" and sample["exit_code"] not in (0,2))):
        raise ValueError("research-state-exit")
    interval=30*1000000000
    limit=grant["initial_restart_count"]+((sample["monitored_at_unix_ns"]-exception["observed_ns"])+interval-1)//interval+2
    if sample["restart_limit"]!=limit or not grant["initial_restart_count"]<=sample["restart_count"]<=limit:
        raise ValueError("research-state-restart")

def load_state():
    if not os.path.lexists(state_path):
        return None
    raw=owned_regular(state_path,1024*1024)
    value=json.loads(raw.decode("utf-8"),object_pairs_hook=pairs,parse_constant=reject_constant)
    if raw!=canonical(value):
        raise ValueError("gate-state-not-canonical")
    return value

def write_state(value):
    data=canonical(value)
    fd,temporary=tempfile.mkstemp(prefix=".gate-state-",dir=gate_dir)
    try:
        os.fchmod(fd,0o600)
        with os.fdopen(fd,"wb") as handle:
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary,state_path)
    except BaseException:
        try: os.close(fd)
        except OSError: pass
        try: os.unlink(temporary)
        except OSError: pass
        raise

def next_state(old,boot,since_us,since,unit_selected,container_selected,exception):
    base={"format":"lagrange-build-gate-state-v2","boot_id":boot,"journal_since":since,
          "journal_since_us":since_us,"units":unit_selected,"containers":container_selected}
    research_name="lagrange-station-research-worker-1"
    if old is None:
        if exception is not None:
            base["research_exception"]={
                "binding":exception["binding"],"first_observation":container_selected[research_name],
                "latest_observation":container_selected[research_name]
            }
        return base
    has_exception=exception is not None
    old_has_exception="research_exception" in old
    if old_has_exception and not has_exception:
        raise ValueError("research-exception-removed")
    if has_exception and not old_has_exception:
        raise ValueError("research-exception-added")
    base_keys=set(base)
    if set(old)!=(base_keys|({"research_exception"} if has_exception else set())):
        raise ValueError("gate-state-schema")
    if (old.get("format")!="lagrange-build-gate-state-v2" or old["boot_id"]!=boot or
            old["journal_since_us"]!=since_us or old["journal_since"]!=since):
        raise ValueError("gate-origin-changed")
    if old["units"]!=unit_selected or set(old["containers"])!=set(container_selected):
        raise ValueError("gate-identity-changed")
    if not has_exception:
        if old["containers"]!=container_selected:
            raise ValueError("container-identity-changed")
        return old
    prior=old["research_exception"]
    if not isinstance(prior,dict) or set(prior)!={"binding","first_observation","latest_observation"}:
        raise ValueError("research-exception-state")
    if prior["binding"]!=exception["binding"]:
        raise ValueError("research-exception-binding-changed")
    first=prior["first_observation"]
    latest=prior["latest_observation"]
    current=container_selected[research_name]
    if old["containers"].get(research_name)!=first:
        raise ValueError("research-exception-origin-changed")
    for sample in (first,latest,current):
        state_sample_valid(sample,exception)
    for name,value in container_selected.items():
        if name!=research_name and old["containers"].get(name)!=value:
            raise ValueError("container-identity-changed")
    if (current["monitored_at_unix_ns"]<latest["monitored_at_unix_ns"] or
            current["restart_count"]<latest["restart_count"]):
        raise ValueError("research-restart-or-time-decreased")
    next_value=json.loads(json.dumps(old,sort_keys=True,separators=(",",":")))
    next_value["research_exception"]["latest_observation"]=current
    return next_value

try:
    owned_directory(gate_dir)
    if previous!=0:
        fail("previous-step-failed")
    unit=os.environ.get("RELEASE_BUILD_SYSTEMD_UNIT","")
    manager=os.environ.get("RELEASE_BUILD_SYSTEMD_MANAGER","system")
    if manager not in ("system","user") or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.:@-]*\.service",unit):
        fail("systemd-binding-invalid")
    units=valid_list(os.environ.get("RELEASE_BUILD_HEALTH_UNITS",""),"unit")
    containers=valid_list(os.environ.get("RELEASE_BUILD_HEALTH_CONTAINERS",""),"container")
    now_ns=time.time_ns()
    try:
        exception=secure_exception(os.environ.get("RELEASE_BUILD_RESEARCH_EXCEPTION",""),now_ns)
    except (OSError,ValueError,UnicodeError,json.JSONDecodeError,OverflowError):
        fail("research-exception-invalid")
    if exception is not None:
        evidence["research_exception"]=exception["binding"]
    meminfo={}
    for line in open("/proc/meminfo",encoding="ascii"):
        if ":" not in line: continue
        key,rest=line.split(":",1)
        matched=re.fullmatch(r"\s*([0-9]+)\s+kB\s*",rest)
        if matched: meminfo[key]=int(matched.group(1))
    mem,swap=meminfo.get("MemAvailable"),meminfo.get("SwapFree")
    if mem is None or swap is None:
        fail("meminfo-unreadable")
    evidence["mem_available_kib"]=mem
    evidence["swap_free_kib"]=swap
    if mem<2097152:
        fail("memory-threshold")
    if swap<524288:
        fail("swap-threshold")
    process=run_bounded(["ps","-eo","comm="])
    evidence["process_scan"]=command_evidence(process)
    if not command_ok(process):
        fail("process-scan-failed")
    try:
        process_names=[item.strip() for item in process["stdout"].decode("utf-8").splitlines() if item.strip()]
    except UnicodeDecodeError:
        fail("process-scan-failed")
    if any(item in ("cargo","rustc","rustdoc") for item in process_names):
        fail("compiler-process-active")
    build_properties=["LoadState","ActiveState","SubState","ExecMainStatus","MainPID","ControlGroup","Nice","IOSchedulingClass","IOSchedulingPriority"]
    build,build_command=systemd_show(manager,unit,build_properties)
    evidence["build_unit"]={"manager":manager,"selected":build,"command":build_command}
    if (build["LoadState"],build["ActiveState"],build["SubState"],build["ExecMainStatus"])!=("loaded","active","running","0"):
        fail("build-service-not-running")
    decimal(build["MainPID"],positive=True)
    nice=decimal(build["Nice"])
    io_priority=decimal(build["IOSchedulingPriority"])
    if not 1<=nice<=19:
        fail("build-service-priority-invalid")
    if build["IOSchedulingClass"] in ("idle","3"):
        pass
    elif build["IOSchedulingClass"] in ("best-effort","2") and 5<=io_priority<=7:
        pass
    else:
        fail("build-service-priority-invalid")
    if not in_build_cgroup(build["ControlGroup"]):
        fail("apply-not-running-inside-build-service")
    unit_selected={}
    for name in units:
        selected,command=systemd_show("system",name,["LoadState","ActiveState","SubState","ExecMainStatus","MainPID","NRestarts"])
        evidence["units"][name]={"selected":selected,"command":command,"manager":"system"}
        if ((selected["LoadState"],selected["ActiveState"],selected["SubState"],selected["ExecMainStatus"])!=
                ("loaded","active","running","0")):
            fail("systemd-health-invalid")
        decimal(selected["MainPID"],positive=True)
        decimal(selected["NRestarts"])
        unit_selected[name]=selected
    boot=normalized_boot(open("/proc/sys/kernel/random/boot_id",encoding="ascii").read().strip())
    try:
        old=load_state()
    except (OSError,ValueError,UnicodeError,json.JSONDecodeError):
        fail("gate-identity-changed")
    if old is not None:
        if not isinstance(old,dict):
            fail("gate-state-invalid")
        since_us=old.get("journal_since_us")
        since=old.get("journal_since")
        if type(since_us) is not int or since_us<0 or since!=utc_text(since_us):
            fail("gate-origin-changed")
        if old.get("boot_id")!=boot:
            fail("gate-origin-changed")
    else:
        since_us=now_ns//1000-1800*1000000
        since=utc_text(since_us)
    until_us=now_ns//1000
    until=utc_text(until_us)
    evidence["journal"].update({"boot_id":boot,"since":since,"since_us":since_us,"until":until})
    probe=run_bounded(["journalctl","-k","-b","--no-pager","-o","json","-n","1"])
    evidence["journal"]["probe"]=command_evidence(probe)
    if not command_ok(probe):
        evidence["journal"]["status"]="FAIL"
        fail("kernel-journal-unestablished")
    try:
        parse_probe(probe["stdout"],boot)
    except (ValueError,UnicodeError,json.JSONDecodeError):
        evidence["journal"]["status"]="FAIL"
        fail("kernel-journal-unestablished")
    scan=run_bounded(["journalctl","-k","-b","--no-pager","-o","json","--since",since,"--until",until,"--no-tail"],
                     stdout_limit=64*1024*1024,stderr_limit=64*1024,deadline_seconds=10,
                     line_limit=1024*1024,record_limit=100000)
    evidence["journal"]["range"]=command_evidence(scan)
    if not command_ok(scan):
        evidence["journal"]["status"]="FAIL"
        fail("journal-range-invalid")
    try:
        journal_count,oom_count=parse_range(scan["stdout"],boot,since_us,until_us,scan["record_count"])
    except (ValueError,UnicodeError,json.JSONDecodeError):
        evidence["journal"]["status"]="FAIL"
        fail("journal-range-invalid")
    evidence["journal"]["count"]=journal_count
    evidence["journal"]["oom_count"]=oom_count
    if oom_count:
        evidence["journal"]["status"]="FAIL"
        fail("kernel-oom-observed")
    evidence["journal"]["status"]="PASS"
    docker_format='{{.Id}}{{printf "\t"}}{{.State.Running}}{{printf "\t"}}{{.State.Restarting}}{{printf "\t"}}{{.State.OOMKilled}}{{printf "\t"}}{{if .State.Health}}{{.State.Health.Status}}{{else}}absent{{end}}{{printf "\t"}}{{.RestartCount}}{{printf "\t"}}{{if index .Config.Labels "com.docker.compose.project"}}{{index .Config.Labels "com.docker.compose.project"}}{{else}}absent{{end}}{{printf "\t"}}{{.Image}}{{printf "\t"}}{{.State.ExitCode}}'
    container_selected={}
    for name in containers:
        inspected=run_bounded(["docker","inspect","--type","container","--format",docker_format,name])
        evidence["containers"][name]={"command":command_evidence(inspected),"selected":None}
        if not command_ok(inspected):
            fail("container-inspect-failed")
        try:
            selected=parse_container(inspected["stdout"],name,exception,now_ns)
        except (ValueError,UnicodeError):
            fail("container-health-invalid")
        evidence["containers"][name]["selected"]=selected
        container_selected[name]=selected
    try:
        next_value=next_state(old,boot,since_us,since,unit_selected,container_selected,exception)
    except ValueError as error:
        state_reason={
            "research-exception-removed":"research-exception-removed",
            "research-exception-added":"research-exception-added",
            "research-exception-binding-changed":"research-exception-binding-changed",
            "research-restart-or-time-decreased":"research-restart-or-time-decreased",
        }.get(str(error),"gate-identity-changed")
        fail(state_reason)
    write_state(next_value)
    record_pass()
except GateFailure:
    raise SystemExit(1)
except (OSError,ValueError,KeyError,TypeError,UnicodeError,json.JSONDecodeError,OverflowError):
    try:
        fail("observation-invalid")
    except GateFailure:
        raise SystemExit(1)
PY
}

release_build_layout_archive_scan() {
  rbl_archive=$1
  rbl_image_id=$2
  rbl_platform=$3
  rbl_commit=$4
  rbl_request=$5
  rbl_result=$6
  RBL_ARCHIVE=$rbl_archive RBL_IMAGE_ID=$rbl_image_id RBL_PLATFORM=$rbl_platform \
  RBL_COMMIT=$rbl_commit RBL_REQUEST=$rbl_request RBL_RESULT=$rbl_result python3 - <<'PY'
import gzip, hashlib, io, json, os, posixpath, re, stat, tarfile, tempfile

def canonical_host_path(value):
    # Reject aliases before reading any input or creating a verification result.
    # Normalizing here would hide a violation of the public path contract.
    if (not os.path.isabs(value) or
            any(ord(ch)<32 or ord(ch)==127 for ch in value) or
            os.path.normpath(value)!=value or os.path.realpath(value)!=value):
        raise SystemExit("host-path-not-canonical")
    return value

archive=canonical_host_path(os.environ["RBL_ARCHIVE"])
request_path=canonical_host_path(os.environ["RBL_REQUEST"])
result_path=canonical_host_path(os.environ["RBL_RESULT"])
image_id=os.environ["RBL_IMAGE_ID"]
platform=os.environ["RBL_PLATFORM"]
commit=os.environ["RBL_COMMIT"]
if not re.fullmatch(r"sha256:[0-9a-f]{64}",image_id): raise SystemExit("image-id-invalid")
if platform!="linux/amd64" or not re.fullmatch(r"[0-9a-f]{40}",commit): raise SystemExit("binding-invalid")
for path in (archive,request_path):
    info=os.lstat(path)
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode): raise SystemExit("input-not-regular")
if os.path.lexists(result_path): raise SystemExit("result-already-exists")
parent=os.path.dirname(result_path)
if not os.path.isdir(parent) or os.path.islink(parent): raise SystemExit("result-parent-invalid")

def pairs(items):
    result={}
    for key,value in items:
        if key in result: raise ValueError("duplicate-json-key")
        result[key]=value
    return result
def reject_constant(value): raise ValueError("non-finite-json-number")
request_raw=open(request_path,"rb").read()
request=json.loads(request_raw.decode("utf-8"),object_pairs_hook=pairs,parse_constant=reject_constant)
if not isinstance(request,dict) or set(request)!={"format","files","nonempty_directories"} or request["format"]!="lagrange-image-files-v1" or not isinstance(request["files"],list) or not request["files"] or not isinstance(request["nonempty_directories"],list):
    raise SystemExit("request-schema-invalid")
def canonical_path(value):
    if not isinstance(value,str) or not value or value.startswith("/") or "\\" in value or value.endswith("/") or "\x00" in value or posixpath.normpath(value)!=value or value=="." or value.startswith("../") or "/../" in value or value.endswith("/..") or "/./" in value or value.endswith("/."):
        raise ValueError("image-path-invalid")
    parts=value.split("/")
    if any(not part or part in (".","..") for part in parts): raise ValueError("image-path-invalid")
    return value
files={}
for item in request["files"]:
    if not isinstance(item,dict) or set(item)!={"path","sha256","executable","elf","contains_hex"}: raise SystemExit("file-requirement-schema-invalid")
    path=canonical_path(item["path"])
    if path in files: raise SystemExit("duplicate-file-path")
    digest=item["sha256"]
    if digest is not None and (not isinstance(digest,str) or not re.fullmatch(r"[0-9a-f]{64}",digest)): raise SystemExit("file-sha256-invalid")
    if type(item["executable"]) is not bool or type(item["elf"]) is not bool or not isinstance(item["contains_hex"],list):
        raise SystemExit("file-requirement-type-invalid")
    patterns=[]
    for literal in item["contains_hex"]:
        if not isinstance(literal,str) or not literal or len(literal)%2 or not re.fullmatch(r"[0-9a-f]+",literal):
            raise SystemExit("file-literal-invalid")
        patterns.append(bytes.fromhex(literal))
    files[path]={"sha256":digest,"executable":item["executable"],"elf":item["elf"],"patterns":patterns}
directories=[]
for item in request["nonempty_directories"]:
    path=canonical_path(item)
    if path in directories: raise SystemExit("duplicate-directory-path")
    directories.append(path)
requested=list(files)+directories
for file_path in files:
    if file_path in directories:
        raise SystemExit("file-directory-path-conflict")
    for other_file in files:
        if other_file != file_path and other_file.startswith(file_path+"/"):
            raise SystemExit("regular-file-parent-path-conflict")
    for directory_path in directories:
        if directory_path.startswith(file_path+"/"):
            raise SystemExit("regular-file-parent-path-conflict")

def file_hash(path):
    digest=hashlib.sha256()
    with open(path,"rb") as handle:
        for chunk in iter(lambda:handle.read(1024*1024),b""): digest.update(chunk)
    return digest.hexdigest()
archive_info=os.stat(archive)
archive_sha=file_hash(archive)

def tar_name(raw,is_dir=False):
    if not isinstance(raw,str) or not raw or "\x00" in raw or "\\" in raw or raw.startswith("/") or raw.startswith("./") or "//" in raw:
        raise ValueError("tar-path-invalid")
    if is_dir:
        raw=raw.rstrip("/")
    if not raw or posixpath.normpath(raw)!=raw or raw=="." or raw.startswith("../") or "/../" in raw or raw.endswith("/..") or "/./" in raw or raw.endswith("/."):
        raise ValueError("tar-path-invalid")
    return raw

tmp=tempfile.TemporaryDirectory(prefix=".release-image-scan-",dir=parent)
blobs={}
try:
    with tarfile.open(archive,mode="r|*") as outer:
        seen=set()
        for member in outer:
            name=tar_name(member.name,member.isdir())
            if name in seen: raise SystemExit("archive-duplicate-entry")
            seen.add(name)
            if member.isdir(): continue
            if not member.isfile(): raise SystemExit("archive-special-entry")
            stream=outer.extractfile(member)
            if stream is None: raise SystemExit("archive-member-unreadable")
            fd,path=tempfile.mkstemp(prefix="blob-",dir=tmp.name)
            size=0
            with os.fdopen(fd,"wb") as handle:
                while True:
                    chunk=stream.read(1024*1024)
                    if not chunk: break
                    size+=len(chunk); handle.write(chunk)
            if size != member.size: raise SystemExit("archive-member-size-mismatch")
            blobs[name]={"path":path,"size":size,"sha256":file_hash(path)}
    def blob_path(digest):
        if not isinstance(digest,str) or not re.fullmatch(r"sha256:[0-9a-f]{64}",digest): raise ValueError("descriptor-digest-invalid")
        name="blobs/sha256/"+digest[7:]
        if name not in blobs: raise ValueError("descriptor-blob-missing")
        return blobs[name]
    def digest_blob(digest):
        item=blob_path(digest)
        # A digest-named member is not bound merely by its pathname.  This is
        # also the root binding for direct manifest/index archives, which have
        # no parent descriptor to perform this check.
        if item["sha256"]!=digest[7:]: raise ValueError("digest-blob-binding-invalid")
        return item
    def descriptor(value):
        if not isinstance(value,dict) or not isinstance(value.get("digest"),str) or type(value.get("size")) is not int or value["size"]<0:
            raise ValueError("descriptor-invalid")
        item=digest_blob(value["digest"])
        if item["size"]!=value["size"]: raise ValueError("descriptor-binding-invalid")
        return item
    def read_item(item):
        with open(item["path"],"rb") as handle: return handle.read()
    def read_blob(digest):
        return read_item(digest_blob(digest))
    def load_json_blob(digest):
        return json.loads(read_blob(digest).decode("utf-8"),object_pairs_hook=pairs,parse_constant=reject_constant)
    index_bytes=None
    index_object=None
    index_item=blobs.get("index.json")
    # Docker save's top-level index has no descriptor.  Treat it as a root
    # only after its actual bytes match the supplied image ID; otherwise it is
    # not an authority for the selected image.
    if index_item is not None and index_item["sha256"]==image_id[7:]:
        index_bytes=read_item(index_item)
        index_object=json.loads(index_bytes.decode("utf-8"),object_pairs_hook=pairs,parse_constant=reject_constant)
    selected=[]
    visited=set()
    def is_attestation(desc):
        annotations=desc.get("annotations") or {}
        return annotations.get("vnd.docker.reference.type")=="attestation-manifest"
    def walk_index(digest,obj,depth=0):
        if depth>8 or digest in visited: raise ValueError("index-cycle-or-depth")
        visited.add(digest)
        if not isinstance(obj,dict) or obj.get("schemaVersion")!=2 or not isinstance(obj.get("manifests"),list) or not obj["manifests"]:
            raise ValueError("index-schema-invalid")
        candidates=[]
        for desc in obj["manifests"]:
            item=descriptor(desc)
            if is_attestation(desc): continue
            media=str(desc.get("mediaType",""))
            if "image.index" in media:
                child=load_json_blob(desc["digest"]); child_candidates=walk_index(desc["digest"],child,depth+1)
                candidates.extend(child_candidates); continue
            if "image.manifest" not in media and "manifest" not in media: continue
            p=desc.get("platform")
            if p is not None and (not isinstance(p,dict) or p.get("os")!="linux" or p.get("architecture")!="amd64"):
                continue
            candidates.append(desc["digest"])
        if not candidates: raise ValueError("runnable-manifest-missing")
        return candidates
    def resolve_manifest(digest):
        value=load_json_blob(digest)
        if not isinstance(value,dict) or value.get("schemaVersion")!=2 or not isinstance(value.get("config"),dict) or not isinstance(value.get("layers"),list):
            raise ValueError("manifest-schema-invalid")
        config_desc=value["config"]; descriptor(config_desc)
        layers=[]
        for item in value["layers"]:
            if not isinstance(item,dict): raise ValueError("layer-descriptor-invalid")
            layers.append(descriptor(item))
        config=load_json_blob(config_desc["digest"])
        if not isinstance(config,dict) or config.get("os")!="linux" or config.get("architecture")!="amd64": raise ValueError("config-platform-invalid")
        labels=((config.get("config") or {}).get("Labels") or {})
        if not isinstance(labels,dict) or labels.get("org.opencontainers.image.revision")!=commit: raise ValueError("config-revision-invalid")
        return digest,config_desc["digest"],layers
    if index_object is not None and hashlib.sha256(index_bytes).hexdigest()==image_id[7:]:
        candidates=walk_index(image_id,index_object)
    elif "blobs/sha256/"+image_id[7:] in blobs:
        direct=load_json_blob(image_id)
        if isinstance(direct,dict) and "manifests" in direct:
            candidates=walk_index(image_id,direct)
        elif isinstance(direct,dict) and "config" in direct and "layers" in direct:
            candidates=[image_id]
        else:
            candidates=[]
    else:
        candidates=[]
    if len(set(candidates))!=1:
        if candidates: raise SystemExit("multiple-runnable-manifests")
        raise SystemExit("image-root-not-bound")
    manifest_digest,config_digest,layer_blobs=resolve_manifest(candidates[0])

    patterns={literal for item in files.values() for literal in item["patterns"]}
    entries={"":{"kind":"dir","mode":0o755}}
    def remove_tree(path,layer_entries):
        for key in list(entries):
            if key and (key==path or key.startswith(path+"/")):
                del entries[key]
                layer_entries.discard(key)
    def remove_lower_tree(path,lower_entries,layer_entries):
        # OCI whiteout semantics hide the lower filesystem.  Additions in the
        # same tar layer remain visible regardless of tar member ordering.
        for key in list(entries):
            if key and key in lower_entries and key not in layer_entries and (not path or key==path or key.startswith(path+"/")):
                del entries[key]
    def ensure_parent(path,layer_entries):
        parent=posixpath.dirname(path)
        if not parent: return
        parts=parent.split("/")
        current=""
        for part in parts:
            current=part if not current else current+"/"+part
            old=entries.get(current)
            if old is None: entries[current]={"kind":"dir","mode":0o755}
            elif old["kind"]!="dir": raise ValueError("layer-parent-not-directory")
            # An upper-layer child makes its directory chain part of that
            # layer for whiteout/opaque precedence, even if the directory was
            # inherited rather than emitted as a separate tar member.
            layer_entries.add(current)
    def hardlink_target(name,raw_target):
        target=tar_name(raw_target)
        if target==name: raise SystemExit("layer-hardlink-self-target")
        current=""
        for part in target.split("/")[:-1]:
            current=part if not current else current+"/"+part
            ancestor=entries.get(current)
            if ancestor is None: raise SystemExit("layer-hardlink-target-missing")
            if ancestor["kind"]!="dir": raise SystemExit("layer-hardlink-target-parent-not-directory")
        target_entry=entries.get(target)
        if target_entry is None: raise SystemExit("layer-hardlink-target-missing")
        if target_entry["kind"]!="file": raise SystemExit("layer-hardlink-target-not-regular")
        return target
    def apply_layer(item):
        lower_entries=set(entries)
        layer_entries=set()
        source=open(item["path"],"rb")
        stream=source
        if open(item["path"],"rb").read(2)==b"\x1f\x8b":
            source.seek(0); stream=gzip.GzipFile(fileobj=source)
        else: source.seek(0)
        layer_seen=set()
        with tarfile.open(fileobj=stream,mode="r|") as layer:
            for member in layer:
                raw=member.name
                # Python tarfile exposes the conventional root directory
                # header as exactly ".".  It is the sole layer-only exception
                # to tar_name: no non-directory dot entry or dot-prefixed
                # descendant is accepted, and outer archive paths stay strict.
                if raw==".":
                    if not member.isdir(): raise SystemExit("layer-root-entry-not-directory")
                    name=""
                else:
                    name=tar_name(raw,member.isdir())
                if name in layer_seen: raise SystemExit("layer-duplicate-entry")
                layer_seen.add(name)
                base=posixpath.basename(name); parent=posixpath.dirname(name)
                if base==".wh..wh..opq":
                    remove_lower_tree(parent,lower_entries,layer_entries)
                    continue
                if base.startswith(".wh."):
                    target=posixpath.join(parent,base[4:]) if parent else base[4:]
                    if target in ("",".",".."): raise SystemExit("whiteout-target-invalid")
                    remove_lower_tree(target,lower_entries,layer_entries); continue
                if name:
                    ensure_parent(name,layer_entries)
                if member.isdir():
                    # A later directory entry updates that directory's metadata
                    # but does not hide lower descendants.  Only replacement of
                    # a non-directory ancestor removes its former tree.
                    existing=entries.get(name)
                    if existing is not None and existing["kind"]!="dir":
                        remove_tree(name,layer_entries)
                    entries[name]={"kind":"dir","mode":member.mode & 0o7777}
                    layer_entries.add(name)
                    continue
                if member.islnk():
                    if member.size!=0: raise SystemExit("layer-hardlink-body-invalid")
                    target=hardlink_target(name,member.linkname)
                    remove_tree(name,layer_entries)
                    # A hardlink is retained only as non-regular overlay state.
                    # Its target bytes are never copied into this entry, so it
                    # cannot satisfy a selected file or make a selected
                    # directory nonempty.
                    entries[name]={"kind":"hardlink","mode":member.mode & 0o7777,"target":target}
                    layer_entries.add(name)
                    continue
                if member.issym():
                    remove_tree(name,layer_entries); entries[name]={"kind":"symlink","mode":member.mode & 0o7777,"target":member.linkname}; layer_entries.add(name); continue
                if not member.isfile(): raise SystemExit("layer-special-entry")
                stream_member=layer.extractfile(member)
                if stream_member is None: raise SystemExit("layer-file-unreadable")
                digest=hashlib.sha256(); total=0; prefix=b""; tail=b""; found=set()
                first=True
                while True:
                    chunk=stream_member.read(1024*1024)
                    if not chunk: break
                    total+=len(chunk); digest.update(chunk)
                    if first: prefix=chunk[:64]; first=False
                    joined=tail+chunk
                    for literal in patterns:
                        if literal in joined: found.add(literal)
                    keep=max((len(x) for x in patterns),default=1)-1
                    tail=joined[-keep:] if keep else b""
                if total!=member.size: raise SystemExit("layer-file-size-mismatch")
                remove_tree(name,layer_entries)
                entries[name]={"kind":"file","mode":member.mode & 0o7777,"size":total,"sha256":digest.hexdigest(),
                  "elf":(len(prefix)>=20 and prefix[:4]==b"\x7fELF" and prefix[4]==2 and prefix[5]==1 and int.from_bytes(prefix[18:20],"little")==62),
                  "contains":found}
                layer_entries.add(name)
        try: stream.close()
        except Exception: pass
        source.close()
    for item in layer_blobs: apply_layer(item)
    def selected_entry(path):
        # A selected image path is never permitted to resolve through a
        # symlink.  Base-image links elsewhere remain representable, but they
        # cannot satisfy a binary or payload requirement.
        current=""
        for part in path.split("/")[:-1]:
            current=part if not current else current+"/"+part
            parent=entries.get(current)
            if parent is None: raise SystemExit("selected-parent-missing")
            if parent["kind"]=="symlink": raise SystemExit("selected-path-link")
            if parent["kind"]!="dir": raise SystemExit("selected-parent-not-directory")
        entry=entries.get(path)
        if entry is None: raise SystemExit("selected-path-missing")
        if entry["kind"]=="symlink": raise SystemExit("selected-path-link")
        return entry
    output_files={}
    for path,requirement in files.items():
        entry=selected_entry(path)
        if entry["kind"]!="file": raise SystemExit("selected-path-not-regular")
        if requirement["sha256"] is not None and requirement["sha256"]!=entry["sha256"]: raise SystemExit("selected-file-hash-mismatch")
        if requirement["executable"] and not (entry["mode"] & 0o111): raise SystemExit("selected-file-not-executable")
        if requirement["elf"] and not entry["elf"]: raise SystemExit("selected-file-not-elf64-x86-64")
        if any(literal not in entry["contains"] for literal in requirement["patterns"]): raise SystemExit("selected-file-literal-missing")
        output_files[path]={"sha256":entry["sha256"],"mode":format(entry["mode"],"04o"),"size":entry["size"],"elf":entry["elf"]}
    for path in directories:
        entry=selected_entry(path)
        if entry["kind"]!="dir": raise SystemExit("selected-directory-not-real")
        prefix=path+"/"
        if not any(value["kind"]=="file" and (key==path or key.startswith(prefix)) for key,value in entries.items()):
            raise SystemExit("selected-directory-empty")
    after=os.stat(archive)
    if (after.st_size,after.st_mtime_ns,after.st_ctime_ns)!=(archive_info.st_size,archive_info.st_mtime_ns,archive_info.st_ctime_ns):
        raise SystemExit("archive-changed-during-read")
    result={
      "format":"lagrange-image-files-result-v1","image_id":image_id,
      "manifest_digest":manifest_digest,"config_digest":config_digest,"platform":platform,
      "source_commit":commit,"archive_sha256":archive_sha,"request_sha256":hashlib.sha256(request_raw).hexdigest(),
      "files":{key:output_files[key] for key in sorted(output_files)},
      "nonempty_directories":sorted(directories)
    }
    data=(json.dumps(result,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8")
    fd=os.open(result_path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
    with os.fdopen(fd,"wb") as handle:
        handle.write(data); handle.flush(); os.fsync(handle.fileno())
finally:
    tmp.cleanup()
PY
}

rbl_product_request() {
  rbl_service=$1
  rbl_commit=$2
  rbl_bundle=$3
  rbl_request_path=$4
  rbl_expected_path=$5
  rbl_runtime_inventory_value=$(rbl_runtime_payload_inventory_current) || return 1
  RBL_SERVICE=$rbl_service RBL_COMMIT=$rbl_commit RBL_BUNDLE=$rbl_bundle \
  RBL_SOURCE_ROOT=$RELEASE_BUILD_LAYOUT_SOURCE_ROOT \
  RBL_LAYOUT=$RELEASE_BUILD_LAYOUT_SOURCE_ROOT/deploy/build/release-build-layout.json \
  RBL_REQUEST_PATH=$rbl_request_path RBL_EXPECTED_PATH=$rbl_expected_path \
  RBL_RUNTIME_INVENTORY=$rbl_runtime_inventory_value \
  RBL_RUNTIME_FORMAT=$RBL_RUNTIME_INVENTORY_FORMAT \
  RBL_DOCKERIGNORE_HASH=$RBL_DOCKERIGNORE_SHA256 python3 - <<'PY'
import hashlib, json, os, posixpath, stat, struct

service=os.environ["RBL_SERVICE"]
commit=os.environ["RBL_COMMIT"]
source_root=os.path.abspath(os.environ["RBL_SOURCE_ROOT"])
layout=json.load(open(os.environ["RBL_LAYOUT"],encoding="utf-8"))
bundle=os.path.abspath(os.environ["RBL_BUNDLE"]) if os.environ["RBL_BUNDLE"] else None
runtime_inventory=json.loads(os.environ["RBL_RUNTIME_INVENTORY"])
if (runtime_inventory.get("format")!=os.environ["RBL_RUNTIME_FORMAT"] or
        runtime_inventory.get("dockerignore_sha256")!=os.environ["RBL_DOCKERIGNORE_HASH"]):
    raise SystemExit("product-runtime-inventory-invalid")

def canonical_image(value):
    if not isinstance(value,str) or not value.startswith("/") or value.endswith("/") or "\\" in value:
        raise SystemExit("product-image-path-invalid")
    rel=value[1:]
    if not rel or posixpath.normpath(rel)!=rel or rel=="." or rel.startswith("../") or "/../" in rel or "/./" in rel:
        raise SystemExit("product-image-path-invalid")
    if any(part in ("",".","..") for part in rel.split("/")): raise SystemExit("product-image-path-invalid")
    return rel

def source_rel(value):
    if not isinstance(value,str) or not value or value.startswith("/") or posixpath.normpath(value)!=value or value=="." or value.startswith("../") or "/../" in value:
        raise SystemExit("product-source-path-invalid")
    return value

def source_path(rel):
    rel=source_rel(rel)
    path=os.path.join(source_root,rel)
    if os.path.commonpath((source_root,os.path.abspath(path))) != source_root:
        raise SystemExit("product-source-escape")
    return path

def sha256_file(path):
    digest=hashlib.sha256()
    with open(path,"rb") as handle:
        for chunk in iter(lambda:handle.read(1024*1024),b""): digest.update(chunk)
    return digest.hexdigest()

def is_elf(path):
    with open(path,"rb") as handle: head=handle.read(20)
    return len(head)>=20 and head[:4]==b"\x7fELF" and head[4]==2 and head[5]==1 and struct.unpack("<H",head[18:20])[0]==62

files={}
expected={}
directories=set()

def add_file(image, digest, executable, elf, patterns, mode):
    image=canonical_image(image)
    item={"path":image,"sha256":digest,"executable":bool(executable),"elf":bool(elf),"contains_hex":sorted(patterns)}
    old=files.get(image)
    if old is not None and old != item: raise SystemExit("product-selected-path-conflict")
    files[image]=item
    expected[image]={"sha256":digest,"mode":(format(mode,"04o") if mode is not None else None)}

def selected_source(entry):
    path=source_path(entry["path"])
    try: info=os.lstat(path)
    except FileNotFoundError: raise SystemExit("product-source-entry-missing")
    if format(stat.S_IMODE(info.st_mode),"04o")!=entry["mode"]:
        raise SystemExit("product-source-mode-changed")
    if entry["kind"]=="directory":
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode) or entry["sha256"] is not None:
            raise SystemExit("product-source-directory-invalid")
    elif entry["kind"]=="file":
        if (stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode) or
                sha256_file(path)!=entry["sha256"]):
            raise SystemExit("product-source-file-invalid")
    else: raise SystemExit("product-source-kind-invalid")
    return path,info

def add_source_entry(payload):
    if not isinstance(payload,dict) or set(payload)!={"entries","image","source","tree_sha256"}:
        raise SystemExit("product-runtime-payload-invalid")
    source=source_rel(payload["source"])
    image=canonical_image(payload["image"])
    entries=payload["entries"]
    if not isinstance(entries,list) or not entries:
        raise SystemExit("product-runtime-payload-entries-invalid")
    root_entry=next((entry for entry in entries if entry.get("path")==source),None)
    if root_entry is None: raise SystemExit("product-runtime-payload-root-missing")
    if root_entry.get("kind")=="file":
        if len(entries)!=1: raise SystemExit("product-runtime-file-payload-invalid")
        path,info=selected_source(root_entry)
        add_file("/"+image,root_entry["sha256"],bool(stat.S_IMODE(info.st_mode)&0o111),
                 is_elf(path),[],stat.S_IMODE(info.st_mode))
        return
    if root_entry.get("kind")!="directory": raise SystemExit("product-runtime-directory-payload-invalid")
    directories.add(image)
    for entry in entries:
        path,info=selected_source(entry)
        if entry["kind"]!="file": continue
        relative=posixpath.relpath(entry["path"],source)
        if relative=="." or relative.startswith("../"):
            raise SystemExit("product-runtime-entry-outside-payload")
        add_file("/"+image+"/"+relative,entry["sha256"],bool(stat.S_IMODE(info.st_mode)&0o111),
                 is_elf(path),[],stat.S_IMODE(info.st_mode))

def add_payloads(name):
    values=runtime_inventory.get("payload_groups",{}).get(name)
    declared=layout["runtime_payloads"].get(name)
    if not isinstance(values,list) or not isinstance(declared,list):
        raise SystemExit("product-runtime-payload-group-invalid")
    if ([{"source":item.get("source"),"image":item.get("image")} for item in values]
            != declared):
        raise SystemExit("product-runtime-payload-declaration-mismatch")
    for item in values: add_source_entry(item)

record=layout["services"].get(service)
if record is None: raise SystemExit("product-service-unknown")
kind=record["kind"]
if kind=="rust":
    recipe_id=record["recipe"]
    recipe=layout["recipes"].get(recipe_id)
    if recipe is None or not bundle: raise SystemExit("product-bundle-missing")
    release=os.path.join(bundle,".release-build")
    for item in recipe["runtime_binaries"]:
        name=item["bin"]
        receipt_path=os.path.join(release,"producer",name,"artifact.json")
        receipt=json.load(open(receipt_path,encoding="utf-8"))
        if receipt.get("source_commit")!=commit or receipt.get("bin")!=name or receipt.get("binary_mode")!="0755":
            raise SystemExit("product-receipt-binding-invalid")
        patterns=[]
        if recipe_id=="D5" and recipe.get("backtest_commit_literal")==item["image"]:
            patterns=[commit.encode("ascii").hex()]
        add_file(item["image"],receipt.get("binary_sha256"),True,True,patterns,0o755)
    if recipe.get("runtime_payload"):
        add_payloads(recipe["runtime_payload"])
elif kind=="database":
    add_file("/usr/local/bin/sqlx",None,True,True,[],None)
    add_payloads("database")
elif kind=="web":
    # Next's standalone copy is rooted at /app and retains the workspace
    # server path used by Compose. Generated output is intentionally bound by
    # presence/nonempty checks rather than pretending it has a source hash.
    add_file("/app/apps/web/server.js",None,False,False,[],None)
    directories.update(("app/apps/web/.next","app/apps/web/.next/static"))
else:
    raise SystemExit("product-service-kind-invalid")

request={"format":"lagrange-image-files-v1",
         "files":[files[key] for key in sorted(files)],
         "nonempty_directories":sorted(directories)}
expected_value={"format":"lagrange-product-image-expectations-v1",
                "files":{key:expected[key] for key in sorted(expected)},
                "nonempty_directories":sorted(directories)}
def write_or_match(path,value):
    data=(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8")
    parent=os.path.dirname(path)
    if not os.path.isdir(parent) or os.path.islink(parent): raise SystemExit("product-request-parent-invalid")
    if os.path.lexists(path):
        info=os.lstat(path)
        if (not stat.S_ISREG(info.st_mode) or stat.S_ISLNK(info.st_mode) or
                stat.S_IMODE(info.st_mode)!=0o600 or info.st_uid!=os.geteuid()):
            raise SystemExit("product-request-existing-unsafe")
        fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW)
        try:
            opened=os.fstat(fd)
            if ((opened.st_dev,opened.st_ino)!=(info.st_dev,info.st_ino) or
                    not stat.S_ISREG(opened.st_mode) or stat.S_IMODE(opened.st_mode)!=0o600 or
                    opened.st_uid!=os.geteuid()):
                raise SystemExit("product-request-existing-raced")
            with os.fdopen(fd,"rb",closefd=False) as handle:
                current=handle.read()
            if current!=data: raise SystemExit("product-request-binding-changed")
        finally:
            os.close(fd)
        return
    fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
    with os.fdopen(fd,"wb") as handle:
        handle.write(data); handle.flush(); os.fsync(handle.fileno())
write_or_match(os.environ["RBL_REQUEST_PATH"],request)
write_or_match(os.environ["RBL_EXPECTED_PATH"],expected_value)
PY
}

release_build_layout_verify_image() {
  [ "${RELEASE_BUILD_LAYOUT_INITIALIZED:-0}" = 1 ] || {
    rbl_die 'release-build-layout-init must run before verify-image'
    return 1
  }
  rbl_service=$1
  rbl_commit=$2
  rbl_image_id=$3
  rbl_state_root=$4
  [ "$rbl_commit" = "$RELEASE_BUILD_LAYOUT_COMMIT" ] ||
    {
      rbl_die 'image commit does not match initialized source commit'
      return 1
    }
  [ "$rbl_state_root" = "$RELEASE_BUILD_LAYOUT_STATE_ROOT" ] ||
    {
      rbl_die 'image state root does not match initialized state'
      return 1
    }
  case "$rbl_image_id" in
    sha256:[0-9a-f][0-9a-f]* ) : ;;
    * ) rbl_die 'image ID is not a canonical sha256 identity'; return 1 ;;
  esac
  printf '%s' "$rbl_image_id" | grep -Eq '^sha256:[0-9a-f]{64}$' ||
    {
      rbl_die 'image ID is not a canonical sha256 identity'
      return 1
    }
  rbl_service_record "$rbl_service" "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT/deploy/build/release-build-layout.json" >/dev/null || return 1
  rbl_bundle=
  rbl_recipe=$(rbl_recipe_for_service "$rbl_service" "$RELEASE_BUILD_LAYOUT_SOURCE_ROOT/deploy/build/release-build-layout.json") || return 1
  if [ "$rbl_recipe" != NONE ]; then
    rbl_bundle=$rbl_state_root/bundles/$rbl_recipe
    rbl_bundle_verify "$rbl_bundle" "$rbl_recipe" >/dev/null || return 1
  fi
  rbl_request=$rbl_state_root/verification/$rbl_service-request.json
  rbl_expected=$rbl_state_root/verification/$rbl_service-expectations.json
  rbl_image_result_target=$rbl_state_root/verification/$rbl_service.json
  rbl_product_request "$rbl_service" "$rbl_commit" "$rbl_bundle" "$rbl_request" "$rbl_expected" || return 1
  rbl_archive=$rbl_state_root/images/archive-${rbl_image_id#sha256:}.tar
  if [ -e "$rbl_archive" ] || [ -L "$rbl_archive" ]; then
    [ -f "$rbl_archive" ] && [ ! -L "$rbl_archive" ] || {
      rbl_die 'cached image archive is not a regular file'
      return 1
    }
    [ "$(stat -c '%u:%a' -- "$rbl_archive")" = "$(id -u):600" ] ||
      {
        rbl_die 'cached image archive ownership/mode is unsafe'
        return 1
      }
  else
    rbl_archive_partial=$rbl_state_root/images/.partial-$rbl_service-$$.tar
    [ ! -e "$rbl_archive_partial" ] && [ ! -L "$rbl_archive_partial" ] ||
      {
        rbl_die 'stale image archive staging path'
        return 1
      }
    rbl_image_log=$rbl_state_root/logs/image-save-$rbl_service.log
    docker image save --output "$rbl_archive_partial" "$rbl_image_id" >"$rbl_image_log" 2>&1 ||
      {
        rbl_die "docker image save failed for $rbl_service (see $rbl_image_log)"
        return 1
      }
    chmod 0600 -- "$rbl_archive_partial"
    mv -- "$rbl_archive_partial" "$rbl_archive" || return 1
  fi
  if [ -e "$rbl_image_result_target" ] || [ -L "$rbl_image_result_target" ]; then
    [ -f "$rbl_image_result_target" ] && [ ! -L "$rbl_image_result_target" ] && \
      [ "$(stat -c '%u:%a' -- "$rbl_image_result_target")" = "$(id -u):600" ] || {
      rbl_die 'existing image verification record is unsafe'
      return 1
    }
    rbl_image_result_probe=$rbl_state_root/verification/.partial-$rbl_service-$$.json
    [ ! -e "$rbl_image_result_probe" ] && [ ! -L "$rbl_image_result_probe" ] || {
      rbl_die 'stale image verification staging path'
      return 1
    }
    release_build_layout_archive_scan "$rbl_archive" "$rbl_image_id" "$RBL_PLATFORM" \
      "$rbl_commit" "$rbl_request" "$rbl_image_result_probe" || return 1
    cmp -s -- "$rbl_image_result_probe" "$rbl_image_result_target" || {
      rbl_die 'existing image verification record does not bind current image bytes'
      return 1
    }
    rm -f -- "$rbl_image_result_probe" || return 1
  else
    release_build_layout_archive_scan "$rbl_archive" "$rbl_image_id" "$RBL_PLATFORM" \
      "$rbl_commit" "$rbl_request" "$rbl_image_result_target" || return 1
  fi
  rbl_result=$rbl_image_result_target
  RBL_RESULT=$rbl_result RBL_EXPECTED=$rbl_expected RBL_SERVICE=$rbl_service \
  RBL_COMMIT=$rbl_commit python3 - <<'PY'
import json, os, re, stat
result=json.load(open(os.environ["RBL_RESULT"],encoding="utf-8"))
expected=json.load(open(os.environ["RBL_EXPECTED"],encoding="utf-8"))
if result.get("format")!="lagrange-image-files-result-v1" or result.get("source_commit")!=os.environ["RBL_COMMIT"]:
    raise SystemExit("product-result-binding-invalid")
if set(result.get("files",{})) != set(expected["files"]): raise SystemExit("product-file-set-invalid")
if result.get("nonempty_directories") != expected["nonempty_directories"]: raise SystemExit("product-directory-set-invalid")
for path,need in expected["files"].items():
    got=result["files"][path]
    if need["sha256"] is not None and got["sha256"]!=need["sha256"]: raise SystemExit("product-file-hash-invalid")
    if need["mode"] is not None and got["mode"]!=need["mode"]: raise SystemExit("product-file-mode-invalid")
    if not re.fullmatch(r"[0-9a-f]{64}",got["sha256"]): raise SystemExit("product-result-sha-invalid")
PY
}

rbl_builder_native_identity() {
  rbl_builder_output=$1
  rbl_native_dir=$rbl_builder_output/.release-build
  rbl_target_platform=${TARGETPLATFORM:-}
  [ "$rbl_target_platform" = "$RBL_PLATFORM" ] || {
    rbl_die 'builder target platform is not linux/amd64'
    return 1
  }
  mkdir -p -- "$rbl_native_dir" || return 1
  rustc -vV >"$rbl_native_dir/.rustc-vv" || {
    rbl_die 'rustc identity export failed'
    return 1
  }
  cargo -V >"$rbl_native_dir/.cargo-version" || {
    rbl_die 'cargo identity export failed'
    return 1
  }
  apk info -vv >"$rbl_native_dir/.apk-info" || {
    rbl_die 'apk identity export failed'
    return 1
  }
  RBL_NATIVE_DIR=$rbl_native_dir RBL_TARGET_PLATFORM=$rbl_target_platform python3 - <<'PY' || return 1
import json, os, stat
directory=os.path.abspath(os.environ["RBL_NATIVE_DIR"])
rustc=open(os.path.join(directory,".rustc-vv"),encoding="utf-8").read()
cargo=open(os.path.join(directory,".cargo-version"),encoding="utf-8").read().strip()
apk=open(os.path.join(directory,".apk-info"),encoding="utf-8").read()
import re
hosts=re.findall(r"^host:\s*(\S+)\s*$",rustc,re.MULTILINE)
if not rustc.endswith("\n") or not cargo.startswith("cargo ") or hosts != ["x86_64-unknown-linux-musl"]:
    raise SystemExit("native-tool-identity-invalid")
if os.environ["RBL_TARGET_PLATFORM"] != "linux/amd64": raise SystemExit("native-platform-invalid")
value={
  "format":"lagrange-build-layout-native-v2","target_platform":"linux/amd64",
  "host_triple":hosts[0],"rustc_vv":rustc,"cargo_version":cargo,
  "apk_info_vv":apk,"native_packages":["build-base","musl-dev","openssl-dev","pkgconf","postgresql-dev"],
  "compiler_env":{"CARGO_BUILD_JOBS":"2","CARGO_TARGET_DIR":"/cargo-target","RUSTFLAGS":"<unset>"}
}
path=os.path.join(directory,"native-identity.json")
with open(path,"w",encoding="utf-8",newline="\n") as handle:
    handle.write(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n")
os.chmod(path,0o600)
for name in (".rustc-vv",".cargo-version",".apk-info"):
    os.unlink(os.path.join(directory,name))
PY
  rbl_validate_native_identity "$rbl_native_dir/native-identity.json" || return 1
}

rbl_builder_guard() {
  rbl_request=$1
  rbl_layout=$2
  rbl_guard=/cargo-target/.lagrange-build-layout-guard.json
  RBL_REQUEST=$rbl_request RBL_LAYOUT=$rbl_layout RBL_GUARD=$rbl_guard \
  RBL_NAMESPACE=$RUST_ARTIFACT_CACHE_NAMESPACE \
  python3 - <<'PY'
import json, os, re, stat, tempfile
def pairs(items):
    value={}
    for key,item in items:
        if key in value: raise ValueError("duplicate-json-key")
        value[key]=item
    return value
def load(path):
    raw=open(path,"rb").read()
    value=json.loads(raw.decode("utf-8"),object_pairs_hook=pairs,
                     parse_constant=lambda value: (_ for _ in ()).throw(ValueError("non-finite")))
    if raw != (json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8"):
        raise ValueError("noncanonical-json")
    return value
request=load(os.environ["RBL_REQUEST"])
layout=load(os.environ["RBL_LAYOUT"])
path=os.environ["RBL_GUARD"]
expected={
  "format":"lagrange-build-target-guard-v2","k_sha256":request["k_sha256"],
  "cache_key":request["cache_key"],"cache_namespace":os.environ["RBL_NAMESPACE"],
  "platform":"linux/amd64","profile":"release","target_dir":"/cargo-target",
  "guard_version":"common-1"
}
packages=layout.get("packages")
if not isinstance(packages,dict) or set(request.get("package_hashes",{})) != set(packages):
    raise SystemExit("target-guard-package-set-invalid")
if any(not isinstance(value,str) or not re.fullmatch(r"[0-9a-f]{64}",value)
       for value in request["package_hashes"].values()):
    raise SystemExit("target-guard-package-hash-invalid")
for name,record in packages.items():
    if not isinstance(record,dict) or not isinstance(record.get("local_dependencies"),list):
        raise SystemExit("target-guard-package-graph-invalid")
    if any(item not in packages or item == name for item in record["local_dependencies"]):
        raise SystemExit("target-guard-package-graph-invalid")
reverse={name:set() for name in packages}
for name,record in packages.items():
    for dependency in record["local_dependencies"]: reverse[dependency].add(name)
def closure(changed):
    result=set(changed); pending=list(changed)
    while pending:
        name=pending.pop()
        for dependent in sorted(reverse[name]):
            if dependent not in result:
                result.add(dependent); pending.append(dependent)
    return sorted(result)
valid=False
if os.path.lexists(path):
    info=os.lstat(path)
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode): raise SystemExit("target-guard-not-regular")
    try: current=load(path)
    except (OSError,ValueError): current=None
    expected_keys={"format","status","binding","package_hashes","last_bin","last_binary_sha256"}
    if (isinstance(current,dict) and set(current)==expected_keys and current.get("format")=="lagrange-build-target-guard-v2" and
            current.get("status")=="complete" and current.get("binding")==expected and
            isinstance(current.get("package_hashes"),dict) and
            set(current["package_hashes"])==set(packages) and
            all(isinstance(value,str) and re.fullmatch(r"[0-9a-f]{64}",value) for value in current["package_hashes"].values()) and
            isinstance(current.get("last_bin"),str) and
            isinstance(current.get("last_binary_sha256"),str) and re.fullmatch(r"[0-9a-f]{64}",current["last_binary_sha256"])):
        valid=True
if not valid:
    # A stale, malformed, pending, or unknown ledger means this K-qualified
    # target mount is recreated. Registry and Git mounts are never touched.
    mode="reset"
else:
    changed=sorted(name for name in packages if current["package_hashes"][name] != request["package_hashes"][name])
    mode=("clean:"+",".join(closure(changed))) if changed else "reuse"
value={"format":"lagrange-build-target-guard-v2","status":"pending","binding":expected,
       "package_hashes":request["package_hashes"]}
parent=os.path.dirname(path)
fd,tmp=tempfile.mkstemp(prefix=".guard-",dir=parent)
with os.fdopen(fd,"w",encoding="utf-8",newline="\n") as handle:
    handle.write(json.dumps(value,sort_keys=True,separators=(",",":"))+"\n"); handle.flush(); os.fsync(handle.fileno())
os.chmod(tmp,0o600); os.replace(tmp,path)
print(mode)
PY
}

rbl_builder_finish_guard() {
  rbl_request=$1
  rbl_binary=$2
  rbl_guard=/cargo-target/.lagrange-build-layout-guard.json
  RBL_REQUEST=$rbl_request RBL_GUARD=$rbl_guard RBL_BINARY=$rbl_binary python3 - <<'PY'
import hashlib, json, os, re, stat, tempfile
request=json.load(open(os.environ["RBL_REQUEST"],encoding="utf-8"))
path=os.environ["RBL_GUARD"]
current=json.load(open(path,encoding="utf-8"))
if (not isinstance(current,dict) or set(current)!={"format","status","binding","package_hashes"} or
        current.get("format")!="lagrange-build-target-guard-v2" or current.get("status")!="pending"):
    raise SystemExit("target-guard-not-pending")
if current.get("binding",{}).get("cache_key")!=request["cache_key"] or current.get("package_hashes")!=request.get("package_hashes"):
    raise SystemExit("target-guard-binding-changed")
binary=os.environ["RBL_BINARY"]
h=hashlib.sha256()
with open(binary,"rb") as handle:
    for chunk in iter(lambda:handle.read(1024*1024),b""): h.update(chunk)
current={"format":"lagrange-build-target-guard-v2","status":"complete","binding":current["binding"],"package_hashes":current["package_hashes"],"last_bin":os.path.basename(binary),"last_binary_sha256":h.hexdigest()}
fd,tmp=tempfile.mkstemp(prefix=".guard-",dir=os.path.dirname(path))
with os.fdopen(fd,"w",encoding="utf-8",newline="\n") as handle:
    handle.write(json.dumps(current,sort_keys=True,separators=(",",":"))+"\n"); handle.flush(); os.fsync(handle.fileno())
os.chmod(tmp,0o600); os.replace(tmp,path)
PY
}

rbl_builder_write_receipt() {
  rbl_request=$1
  rbl_producer=$2
  rbl_bin=$3
  rbl_cargo_json=$4
  rbl_cargo_stderr=$5
  rbl_cargo_ms=$6
  RBL_REQUEST=$rbl_request RBL_PRODUCER=$rbl_producer RBL_BIN=$rbl_bin \
  RBL_CARGO_JSON=$rbl_cargo_json RBL_CARGO_STDERR=$rbl_cargo_stderr RBL_CARGO_MS=$rbl_cargo_ms python3 - <<'PY'
import hashlib, json, os, re, stat
request=json.load(open(os.environ["RBL_REQUEST"],encoding="utf-8"))
producer=os.path.abspath(os.environ["RBL_PRODUCER"])
bin_name=os.environ["RBL_BIN"]
binary=os.path.join(producer,"bin",bin_name)
if not os.path.isfile(binary) or os.path.islink(binary) or stat.S_IMODE(os.stat(binary).st_mode)!=0o755: raise SystemExit("builder-binary-invalid")
h=hashlib.sha256()
with open(binary,"rb") as handle:
    for chunk in iter(lambda:handle.read(1024*1024),b""): h.update(chunk)
record={
  "binary_mode":"0755","binary_sha256":h.hexdigest(),"bin":bin_name,"cache_key":request["cache_key"],
  "cargo_success":True,"compile_env":request["compile_env"],"features":request["features"],
  "format":"lagrange-rust-artifact-v1","host_triple":request["host_triple"],
  "input_sha256":request["input_sha256"],"package":request["package"],"platform":request["platform"],
  "profile":request["profile"],"recipe_sha256":request["recipe_sha256"],"source_commit":request["source_commit"],
  "h_sha256":request["h_sha256"]
}
path=os.path.join(producer,"artifact.json")
with open(path,"w",encoding="utf-8",newline="\n") as handle:
    handle.write(json.dumps(record,sort_keys=True,separators=(",",":"))+"\n")
record_hash=hashlib.sha256(open(path,"rb").read()).hexdigest()
with open(os.path.join(producer,"COMPLETE"),"w",encoding="ascii",newline="\n") as handle: handle.write(record_hash+"\n")
summary={"format":"lagrange-cargo-summary-v1","cargo_success":True,"observed_bin":bin_name,"package":request["package"],"fresh_count":0,"compiling_count":0,"executable":"/cargo-target/release/"+bin_name}
observed_target=False
for raw in open(os.environ["RBL_CARGO_JSON"],encoding="utf-8"):
    if not raw.strip(): continue
    try: event=json.loads(raw)
    except (ValueError,json.JSONDecodeError): raise SystemExit("cargo-json-invalid")
    if event.get("reason")=="compiler-artifact":
        fresh=event.get("fresh")
        summary["fresh_count"] += int(fresh is True)
        summary["compiling_count"] += int(fresh is False)
        target=event.get("target") or {}
        if target.get("name")==bin_name:
            executable=event.get("executable")
            if executable != "/cargo-target/release/"+bin_name: raise SystemExit("cargo-executable-mismatch")
            observed_target=True
if not observed_target: raise SystemExit("cargo-target-event-missing")
stderr=open(os.environ["RBL_CARGO_STDERR"],encoding="utf-8").read()
if not re.search(r"(?:^|\s)(?:Fresh|Compiling)\s",stderr): raise SystemExit("cargo-verbose-evidence-missing")
summary["cargo_ms"]=int(os.environ["RBL_CARGO_MS"])
with open(os.path.join(producer,"cargo-summary.json"),"w",encoding="utf-8",newline="\n") as handle:
    handle.write(json.dumps(summary,sort_keys=True,separators=(",",":"))+"\n")
with open(os.path.join(producer,"timing.json"),"w",encoding="utf-8",newline="\n") as handle:
    json.dump({"format":"lagrange-cargo-timing-v1","cargo_ms":int(os.environ["RBL_CARGO_MS"])},handle,sort_keys=True,separators=(",",":")); handle.write("\n")
PY
}

rbl_builder_compile() {
  rbl_request=$1
  rbl_output=$2
  rbl_request_dir=$(dirname -- "$rbl_request")
  rbl_helper=$rbl_request_dir/release-build-layout.sh
  rbl_layout=$rbl_request_dir/release-build-layout.json
  rbl_expected_native=$rbl_request_dir/expected-native-identity.json
  [ -f "$rbl_request" ] && [ -f "$rbl_helper" ] && [ -f "$rbl_layout" ] && \
    [ -f "$rbl_expected_native" ] || {
      rbl_die 'builder request/tool files are missing'
      return 1
    }
  [ "$(rbl_sha256_file "$rbl_helper")" = "$RUST_ARTIFACT_HELPER_SHA256" ] ||
    {
      rbl_die 'builder helper hash does not match host-supplied binding'
      return 1
    }
  [ "$(rbl_sha256_file "$rbl_layout")" = "$RUST_ARTIFACT_LAYOUT_SHA256" ] ||
    {
      rbl_die 'builder layout hash does not match request binding'
      return 1
    }
  rbl_validate_layout "$rbl_layout" || return 1
  rbl_build_root=$(dirname -- "$rbl_request_dir")
  rbl_actual_source_hash=$(rbl_hash_source "$rbl_build_root" "$rbl_layout" 0) || return 1
  rbl_actual_package_hashes=$(rbl_package_hashes "$rbl_build_root" "$rbl_layout" 0) || return 1
  RBL_REQUEST=$rbl_request RBL_PACKAGE=$CARGO_PACKAGE RBL_BIN=$CARGO_BIN \
  RBL_COMMIT=${LAGRANGE_CODE_COMMIT:-} RBL_SOURCE_HASH=$rbl_actual_source_hash \
  RBL_PACKAGE_HASHES=$rbl_actual_package_hashes RBL_LAYOUT=$rbl_layout \
  RBL_INPUT=$RUST_ARTIFACT_INPUT_SHA256 RBL_RECIPE=$RUST_ARTIFACT_RECIPE_SHA256 \
  RBL_CACHE=$RUST_ARTIFACT_CACHE_KEY python3 - <<'PY' || return 1
import hashlib, json, os, re
def pairs(items):
    result={}
    for key,item in items:
        if key in result: raise ValueError("duplicate-json-key")
        result[key]=item
    return result
def load(path):
    return json.load(open(path,encoding="utf-8",newline=""),object_pairs_hook=pairs,
                     parse_constant=lambda value: (_ for _ in ()).throw(ValueError("non-finite")))
value=load(os.environ["RBL_REQUEST"])
layout=load(os.environ["RBL_LAYOUT"])
required={"bins","cache_key","cache_namespace","clean_packages","compile_commit","compile_env","features","format","guard_version","h_sha256","helper_sha256","host_triple","input_sha256","k_sha256","layout_sha256","package","package_hashes","payloads","platform","profile","recipe","recipe_sha256","resolution_inputs","source_commit","transitive_package_hashes"}
if set(value)!=required or value.get("format")!="lagrange-rust-artifact-request-v1": raise SystemExit("builder-request-format-invalid")
if value.get("package")!=os.environ["RBL_PACKAGE"] or os.environ["RBL_BIN"] not in value.get("bins",[]): raise SystemExit("builder-request-target-mismatch")
if value.get("input_sha256")!=os.environ["RBL_INPUT"] or value.get("recipe_sha256")!=os.environ["RBL_RECIPE"] or value.get("cache_key")!=os.environ["RBL_CACHE"]: raise SystemExit("builder-request-hash-mismatch")
if value.get("input_sha256")!=os.environ["RBL_SOURCE_HASH"] or value.get("package_hashes")!=json.loads(os.environ["RBL_PACKAGE_HASHES"]): raise SystemExit("builder-source-package-binding-invalid")
packages=layout.get("packages")
if not isinstance(packages,dict) or set(value["package_hashes"])!=set(packages): raise SystemExit("builder-package-set-invalid")
if any(not isinstance(item,str) or not re.fullmatch(r"[0-9a-f]{64}",item) for item in value["package_hashes"].values()): raise SystemExit("builder-package-hash-invalid")
recipe=layout.get("recipes",{}).get(value["recipe"])
if not isinstance(recipe,dict) or recipe.get("package")!=value["package"] or recipe.get("bins")!=value["bins"] or recipe.get("compile_commit")!=value["compile_commit"] or recipe.get("clean_packages")!=value["clean_packages"]: raise SystemExit("builder-recipe-binding-invalid")
def closure(name,visiting=None,result=None):
    if visiting is None: visiting=set()
    if result is None: result=set()
    if name in visiting: raise SystemExit("builder-package-cycle")
    if name in result: return result
    visiting.add(name)
    for dependency in packages[name]["local_dependencies"]: closure(dependency,visiting,result)
    visiting.remove(name); result.add(name)
    return result
transitive=[value["package"]]+sorted(closure(value["package"])-{value["package"]})
if value.get("transitive_package_hashes") != [{"package":name,"p_sha256":value["package_hashes"][name]} for name in transitive]: raise SystemExit("builder-transitive-p-invalid")
resolution={"argv":["cargo","build","--locked","--release","--package",value["package"],"--bin"],"default_features":True,"locked":True,"message_format":"json-render-diagnostics","verbose":True}
if value.get("resolution_inputs")!=resolution: raise SystemExit("builder-resolution-input-invalid")
policy=value.get("compile_env",{}).get("LAGRANGE_CODE_COMMIT")
actual=os.environ.get("RBL_COMMIT","")
if policy not in ("<unset>",actual): raise SystemExit("builder-commit-env-mismatch")
h_material={"format":"lagrange-build-layout-h-v1","k_sha256":value["k_sha256"],"package":value["package"],"bins":value["bins"],"recipe":value["recipe"],"recipe_sha256":value["recipe_sha256"],"transitive_package_hashes":value["transitive_package_hashes"],"resolution_inputs":value["resolution_inputs"],"compile_env":value["compile_env"],"features":value["features"],"profile":value["profile"],"platform":value["platform"],"host_triple":value["host_triple"]}
if value.get("h_sha256") != hashlib.sha256(json.dumps(h_material,sort_keys=True,separators=(",",":")).encode()).hexdigest(): raise SystemExit("builder-h-binding-invalid")
PY
  rbl_validate_native_identity "$rbl_expected_native" || return 1
  rbl_builder_native_identity "$rbl_output" || return 1
  cmp -s -- "$rbl_expected_native" "$rbl_output/.release-build/native-identity.json" || {
    rbl_die 'native identity changed after COPY under /build'
    return 1
  }
  rbl_metadata=/tmp/lagrange-build-layout-metadata-$$.json
  cargo metadata --locked --offline --no-deps --format-version 1 >"$rbl_metadata" ||
    {
      rbl_die 'pinned Cargo metadata validation failed'
      return 1
    }
  RBL_LAYOUT=$rbl_layout RBL_BUILD_ROOT=$rbl_build_root python3 - "$rbl_metadata" <<'PY' || return 1
import json,os,sys
value=json.load(open(sys.argv[1],encoding="utf-8"))
if not isinstance(value,dict) or not isinstance(value.get("packages"),list) or not isinstance(value.get("workspace_members"),list):
    raise SystemExit("cargo-metadata-shape-invalid")
layout=json.load(open(os.environ["RBL_LAYOUT"],encoding="utf-8"))
root=os.path.abspath(os.environ["RBL_BUILD_ROOT"])
expected=layout["packages"]
records={}
for package in value["packages"]:
    if not isinstance(package,dict) or not isinstance(package.get("name"),str) or not isinstance(package.get("manifest_path"),str):
        raise SystemExit("cargo-metadata-package-invalid")
    manifest=os.path.abspath(package["manifest_path"])
    if not manifest.startswith(root+os.sep) or not manifest.endswith("/Cargo.toml"):
        raise SystemExit("cargo-metadata-path-invalid")
    records[package["name"]]=package
if set(records)!=set(expected): raise SystemExit("cargo-metadata-member-set-invalid")
for name,record in expected.items():
    package=records[name]
    actual=os.path.relpath(os.path.dirname(package["manifest_path"]),root).replace(os.sep,"/")
    if actual!=record["path"]: raise SystemExit("cargo-metadata-member-path-invalid")
    local=[]
    for dependency in package.get("dependencies",[]):
        if not isinstance(dependency,dict): raise SystemExit("cargo-metadata-dependency-invalid")
        candidate=dependency.get("name")
        path=dependency.get("path")
        if candidate in expected and path is not None:
            local.append(candidate)
    if sorted(set(local)) != record["local_dependencies"]:
        raise SystemExit("cargo-metadata-local-graph-invalid")
PY
  rm -f -- "$rbl_metadata"
  rbl_clean_mode=$(rbl_builder_guard "$rbl_request" "$rbl_layout") || return 1
  case "$rbl_clean_mode" in
  reset)
    # This target mount is already K-qualified.  A malformed or interrupted
    # ledger recreates only its release artifacts; registry/Git mounts remain.
    cargo clean --release || return 1
    ;;
  clean:*)
    rbl_clean_packages=${rbl_clean_mode#clean:}
    [ -n "$rbl_clean_packages" ] || {
      rbl_die 'target guard produced an empty reverse clean closure'
      return 1
    }
    old_ifs=$IFS; IFS=,
    for rbl_package in $rbl_clean_packages; do
      [ -n "$rbl_package" ] || {
        rbl_die 'empty Cargo clean package'
        return 1
      }
      cargo clean --release --package "$rbl_package" || return 1
    done
    IFS=$old_ifs
    ;;
  reuse) : ;;
  *) rbl_die 'target guard returned an unknown clean action'; return 1 ;;
  esac
  mkdir -p -- "$rbl_output/.release-build/producer/$CARGO_BIN" "$rbl_output/.release-build"
  rbl_producer=$rbl_output/.release-build/producer/$CARGO_BIN
  mkdir -p -- "$rbl_producer/bin"
  rbl_start=$(python3 -c 'import time; print(time.time_ns())')
  rbl_cargo_status=0
  rbl_compile_policy=$(python3 - "$rbl_request" <<'PY'
import json,sys
value=json.load(open(sys.argv[1],encoding="utf-8"))
print(value["compile_env"]["LAGRANGE_CODE_COMMIT"])
PY
  )
  if [ "$rbl_compile_policy" != "<unset>" ]; then
    export LAGRANGE_CODE_COMMIT
  else
    unset LAGRANGE_CODE_COMMIT
  fi
  export CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/cargo-target
  unset RUSTFLAGS
  cargo build --locked --release --package "$CARGO_PACKAGE" --bin "$CARGO_BIN" \
    --message-format=json-render-diagnostics -vv >"$rbl_producer/cargo.jsonl" \
    2>"$rbl_producer/cargo.stderr" || rbl_cargo_status=$?
  rbl_end=$(python3 -c 'import time; print(time.time_ns())')
  rbl_cargo_ms=$(( (rbl_end - rbl_start) / 1000000 ))
  [ "$rbl_cargo_status" -eq 0 ] || {
    rbl_die "Cargo build failed for $CARGO_BIN"
    return 1
  }
  rbl_binary=/cargo-target/release/$CARGO_BIN
  [ -f "$rbl_binary" ] && [ ! -L "$rbl_binary" ] || {
    rbl_die 'requested Cargo executable is missing'
    return 1
  }
  [ -x "$rbl_binary" ] || {
    rbl_die 'requested Cargo executable is not executable'
    return 1
  }
  RBL_BINARY=$rbl_binary python3 - <<'PY' || return 1
import os,struct
path=os.environ["RBL_BINARY"]
with open(path,"rb") as handle: head=handle.read(20)
if len(head)<20 or head[:4]!=b"\x7fELF" or head[4]!=2 or head[5]!=1 or struct.unpack("<H",head[18:20])[0]!=62:
    raise SystemExit("requested-executable-not-elf64-x86-64")
PY
  install -m 0755 -- "$rbl_binary" "$rbl_producer/bin/$CARGO_BIN"
  rbl_builder_write_receipt "$rbl_request" "$rbl_producer" "$CARGO_BIN" \
    "$rbl_producer/cargo.jsonl" "$rbl_producer/cargo.stderr" "$rbl_cargo_ms" || return 1
  rbl_builder_finish_guard "$rbl_request" "$rbl_binary" || return 1
}

rbl_guard_build() {
  rbl_request=$1
  rbl_bundle=$2
  rbl_output=$3
  [ -f "$rbl_request" ] && [ -d "$rbl_bundle" ] && [ ! -L "$rbl_bundle" ] ||
    {
      rbl_die 'verified artifact bundle paths are invalid'
      return 1
    }
  rbl_layout=$rbl_bundle/.release-build/release-build-layout.json
  rbl_helper=$rbl_bundle/.release-build/release-build-layout.sh
  [ -f "$rbl_layout" ] && [ -f "$rbl_helper" ] ||
    {
      rbl_die 'verified artifact tools are missing'
      return 1
    }
  RBL_REQUEST=$rbl_request RBL_BUNDLE=$rbl_bundle RBL_LAYOUT=$rbl_layout \
  RBL_HELPER=$rbl_helper RBL_OUTPUT=$rbl_output python3 - <<'PY'
import hashlib, json, os, re, shutil, stat, struct

bundle=os.path.abspath(os.environ["RBL_BUNDLE"])
release=os.path.join(bundle,".release-build")
request_path=os.path.abspath(os.environ["RBL_REQUEST"])
layout_path=os.path.abspath(os.environ["RBL_LAYOUT"])
helper_path=os.path.abspath(os.environ["RBL_HELPER"])
output=os.path.abspath(os.environ["RBL_OUTPUT"])

def pairs(items):
    result={}
    for key,value in items:
        if key in result: raise ValueError("duplicate-json-key")
        result[key]=value
    return result
def reject(value): raise ValueError("non-finite-json-number")
def regular(path,mode=None):
    info=os.lstat(path)
    return stat.S_ISREG(info.st_mode) and not stat.S_ISLNK(info.st_mode) and (mode is None or stat.S_IMODE(info.st_mode)==mode)
def directory(path):
    info=os.lstat(path)
    return stat.S_ISDIR(info.st_mode) and not stat.S_ISLNK(info.st_mode)
def raw_json(path,canonical=True):
    raw=open(path,"rb").read()
    value=json.loads(raw.decode("utf-8"),object_pairs_hook=pairs,parse_constant=reject)
    if canonical and raw != (json.dumps(value,sort_keys=True,separators=(",",":"))+"\n").encode("utf-8"):
        raise SystemExit("guard-json-not-canonical")
    return value,raw
def sha(path):
    digest=hashlib.sha256()
    with open(path,"rb") as handle:
        for chunk in iter(lambda:handle.read(1024*1024),b""): digest.update(chunk)
    return digest.hexdigest()
def rel_path(value):
    if (not isinstance(value,str) or not value or value.startswith("/") or "\\" in value or
            "\x00" in value or os.path.normpath(value)!=value or value=="." or
            value.startswith("../") or "/../" in value or value.endswith("/..")):
        raise SystemExit("guard-path-invalid")
    return value
def elf(path):
    with open(path,"rb") as handle: head=handle.read(20)
    return len(head)>=20 and head[:4]==b"\x7fELF" and head[4]==2 and head[5]==1 and struct.unpack("<H",head[18:20])[0]==62
def tree_hash(root):
    records=[]
    for current,dirs,files in os.walk(root,topdown=True,followlinks=False):
        dirs.sort(); files.sort(); kept=[]
        for name in dirs:
            path=os.path.join(current,name); info=os.lstat(path)
            if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode): raise SystemExit("guard-tree-directory-invalid")
            kept.append(name)
            records.append(f"d\t{stat.S_IMODE(info.st_mode):04o}\t{os.path.relpath(path,root)}\t-\n".encode())
        dirs[:]=kept
        for name in files:
            path=os.path.join(current,name); rel=os.path.relpath(path,root); info=os.lstat(path)
            if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode): raise SystemExit("guard-tree-file-invalid")
            if rel==".release-build/complete": continue
            records.append(f"f\t{stat.S_IMODE(info.st_mode):04o}\t{rel}\t{sha(path)}\n".encode())
    return hashlib.sha256(b"".join(sorted(records))).hexdigest()
def entry_hash(path, rel):
    records=[]
    def walk(current,name):
        info=os.lstat(current)
        if stat.S_ISLNK(info.st_mode) or not (stat.S_ISREG(info.st_mode) or stat.S_ISDIR(info.st_mode)):
            raise SystemExit("guard-payload-entry-invalid")
        if stat.S_ISDIR(info.st_mode):
            records.append(f"d\t{stat.S_IMODE(info.st_mode):04o}\t{name}\t-\n".encode())
            for child in sorted(os.listdir(current)): walk(os.path.join(current,child),name+"/"+child)
        else:
            records.append(f"f\t{stat.S_IMODE(info.st_mode):04o}\t{name}\t{sha(current)}\n".encode())
    walk(path,rel)
    return hashlib.sha256(b"".join(records)).hexdigest()

if not directory(bundle) or not directory(release): raise SystemExit("guard-bundle-root-invalid")
if request_path != os.path.join(release,"request.json") or not os.path.samefile(request_path,os.path.join(release,"request.json")):
    raise SystemExit("guard-request-path-invalid")
if layout_path != os.path.join(release,"release-build-layout.json") or helper_path != os.path.join(release,"release-build-layout.sh"):
    raise SystemExit("guard-tool-path-invalid")
expected_release={"bundle.json","complete","native-identity.json","release-build-layout.json","release-build-layout.sh","request.json","producer"}
if set(os.listdir(release))!=expected_release: raise SystemExit("guard-release-entries-invalid")
for name,mode in (("bundle.json",0o600),("complete",0o600),("native-identity.json",0o600),("request.json",0o600),("release-build-layout.json",0o644),("release-build-layout.sh",0o755)):
    if not regular(os.path.join(release,name),mode): raise SystemExit("guard-release-entry-invalid")
if not directory(os.path.join(release,"producer")) or not directory(os.path.join(bundle,"target")) or not directory(os.path.join(bundle,"target","release")):
    raise SystemExit("guard-required-directory-invalid")

request,request_raw=raw_json(request_path)
layout,_=raw_json(layout_path,canonical=False)
bundle_record,_=raw_json(os.path.join(release,"bundle.json"))
request_keys={"bins","cache_key","cache_namespace","clean_packages","compile_commit","compile_env","features","format","guard_version","h_sha256","helper_sha256","host_triple","input_sha256","k_sha256","layout_sha256","package","package_hashes","payloads","platform","profile","recipe","recipe_sha256","resolution_inputs","source_commit","transitive_package_hashes"}
if set(request)!=request_keys or request.get("format")!="lagrange-rust-artifact-request-v1": raise SystemExit("guard-request-schema-invalid")
if layout.get("format")!="lagrange-build-layout-v1" or layout.get("layout")!="common" or layout.get("guard_version")!="common-1": raise SystemExit("guard-layout-invalid")
if request.get("guard_version")!="common-1" or request.get("platform")!="linux/amd64" or request.get("host_triple")!="x86_64-unknown-linux-musl" or request.get("profile")!="release" or request.get("features")!=[]:
    raise SystemExit("guard-compiler-binding-invalid")
if not re.fullmatch(r"[0-9a-f]{40}",request.get("source_commit","")): raise SystemExit("guard-source-commit-invalid")
for key in ("input_sha256","recipe_sha256","k_sha256","cache_key","helper_sha256","layout_sha256","h_sha256"):
    if not re.fullmatch(r"[0-9a-f]{64}",request.get(key,"")): raise SystemExit("guard-hash-invalid")
if request["helper_sha256"] != os.environ.get("RUST_ARTIFACT_HELPER_SHA256"):
    raise SystemExit("guard-helper-binding-invalid")
if sha(helper_path)!=request["helper_sha256"] or sha(layout_path)!=request["layout_sha256"]:
    raise SystemExit("guard-tool-hash-invalid")
recipe=layout.get("recipes",{}).get(request.get("recipe"))
if not isinstance(recipe,dict) or recipe.get("package")!=request["package"] or recipe.get("bins")!=request["bins"] or recipe.get("compile_commit")!=request["compile_commit"] or recipe.get("clean_packages")!=request["clean_packages"]:
    raise SystemExit("guard-recipe-invalid")
packages=layout.get("packages")
if not isinstance(packages,dict) or set(request.get("package_hashes",{})) != set(packages):
    raise SystemExit("guard-package-set-invalid")
if any(not isinstance(value,str) or not re.fullmatch(r"[0-9a-f]{64}",value) for value in request["package_hashes"].values()):
    raise SystemExit("guard-package-hash-invalid")
def closure(name,visiting=None,result=None):
    if visiting is None: visiting=set()
    if result is None: result=set()
    if name in visiting: raise SystemExit("guard-package-cycle")
    if name in result: return result
    visiting.add(name)
    record=packages.get(name)
    if not isinstance(record,dict) or not isinstance(record.get("local_dependencies"),list): raise SystemExit("guard-package-graph-invalid")
    for dependency in record["local_dependencies"]:
        if dependency not in packages: raise SystemExit("guard-package-graph-invalid")
        closure(dependency,visiting,result)
    visiting.remove(name); result.add(name)
    return result
transitive=[request["package"]]+sorted(closure(request["package"])-{request["package"]})
if request.get("transitive_package_hashes") != [{"package":name,"p_sha256":request["package_hashes"][name]} for name in transitive]:
    raise SystemExit("guard-transitive-p-invalid")
resolution={"argv":["cargo","build","--locked","--release","--package",request["package"],"--bin"],"default_features":True,"locked":True,"message_format":"json-render-diagnostics","verbose":True}
if request.get("resolution_inputs") != resolution: raise SystemExit("guard-resolution-input-invalid")
expected_env={"CARGO_BUILD_JOBS":"2","CARGO_TARGET_DIR":"/cargo-target","RUSTFLAGS":"<unset>","LAGRANGE_CODE_COMMIT":request["source_commit"] if request["compile_commit"]=="present" else "<unset>"}
if request.get("compile_env")!=expected_env: raise SystemExit("guard-compile-env-invalid")
expected_payloads=[{"source":item["source"],"image":item["image"]} for item in layout.get("runtime_payloads",{}).get(recipe.get("runtime_payload",""),[])]
if request.get("payloads")!=expected_payloads: raise SystemExit("guard-payload-set-invalid")
h_material={"format":"lagrange-build-layout-h-v1","k_sha256":request["k_sha256"],"package":request["package"],"bins":request["bins"],"recipe":request["recipe"],"recipe_sha256":request["recipe_sha256"],"transitive_package_hashes":request["transitive_package_hashes"],"resolution_inputs":request["resolution_inputs"],"compile_env":request["compile_env"],"features":request["features"],"profile":request["profile"],"platform":request["platform"],"host_triple":request["host_triple"]}
if request["h_sha256"]!=hashlib.sha256(json.dumps(h_material,sort_keys=True,separators=(",",":")).encode("utf-8")).hexdigest(): raise SystemExit("guard-hash-binding-invalid")

bundle_keys={"bins","cache_key","cache_namespace","compile_env","features","format","guard_version","h_sha256","helper_sha256","host_triple","k_sha256","layout_sha256","payloads","platform","profile","recipe","recipe_sha256","source_commit","source_input_sha256"}
if set(bundle_record)!=bundle_keys or bundle_record.get("format")!="lagrange-rust-artifact-bundle-v1": raise SystemExit("guard-bundle-schema-invalid")
for key in ("bins","cache_key","cache_namespace","compile_env","features","guard_version","h_sha256","helper_sha256","host_triple","k_sha256","layout_sha256","platform","profile","recipe","recipe_sha256","source_commit"):
    if bundle_record.get(key)!=request.get(key): raise SystemExit("guard-bundle-binding-invalid")
if bundle_record.get("source_input_sha256")!=request.get("input_sha256"): raise SystemExit("guard-bundle-source-invalid")

native,_=raw_json(os.path.join(release,"native-identity.json"))
native_keys={"apk_info_vv","cargo_version","compiler_env","format","host_triple","native_packages","rustc_vv","target_platform"}
if set(native)!=native_keys or native.get("format")!="lagrange-build-layout-native-v2" or native.get("target_platform")!="linux/amd64" or native.get("host_triple")!="x86_64-unknown-linux-musl": raise SystemExit("guard-native-schema-invalid")
if native.get("compiler_env")!={"CARGO_BUILD_JOBS":"2","CARGO_TARGET_DIR":"/cargo-target","RUSTFLAGS":"<unset>"}: raise SystemExit("guard-native-env-invalid")
if native.get("native_packages")!=["build-base","musl-dev","openssl-dev","pkgconf","postgresql-dev"] or not isinstance(native.get("apk_info_vv"),str) or not native["apk_info_vv"] or not isinstance(native.get("rustc_vv"),str) or not re.search(r"^host:\s*x86_64-unknown-linux-musl\s*$",native["rustc_vv"],re.MULTILINE) or not isinstance(native.get("cargo_version"),str) or not native["cargo_version"].startswith("cargo "):
    raise SystemExit("guard-native-identity-invalid")
complete=open(os.path.join(release,"complete"),"rb").read()
if complete!=(tree_hash(bundle)+"\n").encode("ascii"): raise SystemExit("guard-complete-invalid")

allowed_roots={".release-build","target"}
for payload in expected_payloads: allowed_roots.add(rel_path(payload["source"]).split("/",1)[0])
if set(os.listdir(bundle)) != allowed_roots: raise SystemExit("guard-bundle-root-entries-invalid")
if set(os.listdir(os.path.join(bundle,"target"))) != {"release"} or set(os.listdir(os.path.join(bundle,"target","release"))) != set(recipe["bins"]): raise SystemExit("guard-target-entries-invalid")
if set(os.listdir(os.path.join(release,"producer"))) != set(recipe["bins"]): raise SystemExit("guard-producer-parent-entries-invalid")
observed_payloads=[]
for payload in expected_payloads:
    rel=rel_path(payload["source"]); candidate=os.path.join(bundle,rel)
    if not os.path.lexists(candidate): raise SystemExit("guard-payload-missing")
    observed_payloads.append({"source":rel,"image":payload["image"],"tree_sha256":entry_hash(candidate,rel)})
if bundle_record.get("payloads") != observed_payloads: raise SystemExit("guard-payload-binding-invalid")
for name in recipe["bins"]:
    producer=os.path.join(release,"producer",name)
    if not directory(producer) or set(os.listdir(producer))!={"artifact.json","COMPLETE","bin","cargo.jsonl","cargo.stderr","cargo-summary.json","timing.json"}: raise SystemExit("guard-producer-entries-invalid")
    artifact,artifact_raw=raw_json(os.path.join(producer,"artifact.json"))
    artifact_keys={"binary_mode","binary_sha256","bin","cache_key","cargo_success","compile_env","features","format","h_sha256","host_triple","input_sha256","package","platform","profile","recipe_sha256","source_commit"}
    if set(artifact)!=artifact_keys or artifact.get("format")!="lagrange-rust-artifact-v1" or artifact.get("bin")!=name or artifact.get("binary_mode")!="0755" or artifact.get("cargo_success") is not True: raise SystemExit("guard-artifact-schema-invalid")
    for key,expected in (("cache_key",request["cache_key"]),("compile_env",request["compile_env"]),("features",[]),("h_sha256",request["h_sha256"]),("host_triple","x86_64-unknown-linux-musl"),("input_sha256",request["input_sha256"]),("package",request["package"]),("platform","linux/amd64"),("profile","release"),("recipe_sha256",request["recipe_sha256"]),("source_commit",request["source_commit"])):
        if artifact.get(key)!=expected: raise SystemExit("guard-artifact-binding-invalid")
    if not re.fullmatch(r"[0-9a-f]{64}",artifact.get("binary_sha256","")) or open(os.path.join(producer,"COMPLETE"),"rb").read()!=(hashlib.sha256(artifact_raw).hexdigest()+"\n").encode("ascii"): raise SystemExit("guard-artifact-complete-invalid")
    bindir=os.path.join(producer,"bin"); target=os.path.join(bundle,"target","release",name)
    if not directory(bindir) or set(os.listdir(bindir))!={name} or not regular(os.path.join(bindir,name),0o755) or not regular(target,0o755) or sha(target)!=artifact["binary_sha256"] or sha(os.path.join(bindir,name))!=artifact["binary_sha256"] or not elf(target): raise SystemExit("guard-binary-binding-invalid")
    summary,_=raw_json(os.path.join(producer,"cargo-summary.json"))
    if set(summary)!={"format","cargo_success","observed_bin","package","fresh_count","compiling_count","executable","cargo_ms"} or summary.get("format")!="lagrange-cargo-summary-v1" or summary.get("cargo_success") is not True or summary.get("observed_bin")!=name or summary.get("package")!=request["package"] or summary.get("executable")!="/cargo-target/release/"+name or type(summary.get("fresh_count")) is not int or summary["fresh_count"]<0 or type(summary.get("compiling_count")) is not int or summary["compiling_count"]<0 or type(summary.get("cargo_ms")) is not int or summary["cargo_ms"]<0: raise SystemExit("guard-cargo-summary-invalid")
    timing,_=raw_json(os.path.join(producer,"timing.json"))
    if set(timing)!={"format","cargo_ms"} or timing.get("format")!="lagrange-cargo-timing-v1" or type(timing.get("cargo_ms")) is not int or timing["cargo_ms"]<0 or not regular(os.path.join(producer,"cargo.jsonl")) or not regular(os.path.join(producer,"cargo.stderr")): raise SystemExit("guard-cargo-evidence-invalid")
    seen_target=False
    for raw in open(os.path.join(producer,"cargo.jsonl"),encoding="utf-8"):
        if not raw.strip(): continue
        try: event=json.loads(raw)
        except (ValueError,json.JSONDecodeError): raise SystemExit("guard-cargo-json-invalid")
        if event.get("reason")=="compiler-artifact" and (event.get("target") or {}).get("name")==name:
            if event.get("executable")!="/cargo-target/release/"+name: raise SystemExit("guard-cargo-executable-invalid")
            seen_target=True
    if not seen_target or not re.search(r"(?:^|\s)(?:Fresh|Compiling)\s",open(os.path.join(producer,"cargo.stderr"),encoding="utf-8").read()): raise SystemExit("guard-cargo-evidence-invalid")

if os.path.lexists(output):
    if not directory(output): raise SystemExit("guard-output-invalid")
else:
    os.makedirs(output,mode=0o755)
def copy_one(src,dst):
    info=os.lstat(src)
    if stat.S_ISLNK(info.st_mode) or not (stat.S_ISREG(info.st_mode) or stat.S_ISDIR(info.st_mode)): raise SystemExit("guard-payload-entry-invalid")
    if stat.S_ISDIR(info.st_mode):
        os.makedirs(dst,exist_ok=True); os.chmod(dst,stat.S_IMODE(info.st_mode))
        for child in sorted(os.listdir(src)): copy_one(os.path.join(src,child),os.path.join(dst,child))
        os.utime(dst,ns=(info.st_atime_ns,info.st_mtime_ns))
    else:
        os.makedirs(os.path.dirname(dst),exist_ok=True); shutil.copyfile(src,dst); os.chmod(dst,stat.S_IMODE(info.st_mode)); os.utime(dst,ns=(info.st_atime_ns,info.st_mtime_ns))
for item in recipe["runtime_binaries"]:
    copy_one(os.path.join(bundle,"target","release",item["bin"]),os.path.join(output,"target","release",item["bin"]))
for item in expected_payloads:
    source=rel_path(item["source"]); src=os.path.join(bundle,source)
    if not os.path.lexists(src): raise SystemExit("guard-payload-missing")
    copy_one(src,os.path.join(output,source))
PY
}

rbl_execution_value=$(printenv RELEASE_BUILD_LAYOUT_EXECUTION 2>/dev/null || true)
rbl_first_arg=
[ "$#" -gt 0 ] && rbl_first_arg=$1
if [ "$rbl_execution_value" = 1 ] || [ "$rbl_first_arg" = --builder ] || [ "$rbl_first_arg" = --guard-build ]; then
  case "$1" in
    --native)
      shift
      rbl_builder_native_identity "$1" || exit 1
      ;;
    --builder)
      shift
      rbl_builder_compile "$1" "$2" || exit 1
      ;;
    --guard-build)
      shift
      rbl_guard_request=$1
      rbl_guard_bundle=$2
      rbl_guard_output=$3
      # Product Dockerfiles perform the helper hash check before dispatch.  The
      # dispatch still revalidates the immutable bundle and copies only the
      # declared original paths.
      [ -n "$RUST_ARTIFACT_BUNDLE_SHA256" ] || {
        rbl_die 'bundle hash build argument is empty'
        exit 1
      }
      [ "$(rbl_tree_hash "$rbl_guard_bundle")" = "$RUST_ARTIFACT_BUNDLE_SHA256" ] ||
        {
          rbl_die 'artifact bundle hash does not match host binding'
          exit 1
        }
      rbl_guard_build "$rbl_guard_request" "$rbl_guard_bundle" "$rbl_guard_output" || exit 1
      ;;
    *) rbl_die 'unknown builder dispatch'; exit 1 ;;
  esac
fi
