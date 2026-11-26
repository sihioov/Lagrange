#!/usr/bin/env bash
# Build the exact V3 product images on a fresh GitHub-hosted Ubuntu runner.
# This adapter keeps the production producer, byte-scanner and batch order,
# replacing only production-host service gates with a closed hosted-runner gate.
set -euo pipefail
umask 077

script_path=$(/usr/bin/readlink -f -- "${BASH_SOURCE[0]}")
script_dir=$(CDPATH= cd -- "${script_path%/*}" && pwd -P)
root=$(CDPATH= cd -- "$script_dir/../.." && pwd -P)

usage() {
  printf '%s\n' \
    'Usage: sudo scripts/ops/build-hosted-release.sh COMMIT OUTPUT_DIR' \
    '' \
    'Builds the exact thirteen-image product release from a clean checkout on the' \
    'fixed GitHub-hosted ubuntu-24.04 x86_64 release-delivery job. OUTPUT_DIR must' \
    'be a new or empty root-owned 0700 directory below a trusted root-owned parent,' \
    'outside the checkout. It receives release.manifest and d8/{artifact.json,COMPLETE}.' \
    'Private compiler state, temporary image archives, and build logs stay outside' \
    'OUTPUT_DIR. The Docker build environment contains no provider or release-store' \
    'credentials and starts no container or service.'
}

die() { echo "build-hosted-release: $*" >&2; exit 1; }

clean_env=0
if [ "$#" -eq 3 ] && [ "$3" = __HOSTED_RELEASE_CLEAN_ENV_V1__ ]; then
  clean_env=1
else
  [ "$#" -eq 2 ] || { usage >&2; exit 2; }
fi
commit=$1
output_dir=$2
[[ $(/usr/bin/id -u) -eq 0 ]] || die 'must run through sudo as root'
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] && [ "$commit" != 0000000000000000000000000000000000000000 ] ||
  die 'COMMIT must be exact lowercase 40-hex'

# Sudo normally drops GitHub's job context. The caller passes only these
# non-secret identity fields; this clean re-exec discards every other inherited
# variable before Docker, Compose, the resource gate, or Cargo can observe it.
if [ "$clean_env" -ne 1 ]; then
  env_args=(
    PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
    HOME=/root USER=root LOGNAME=root SHELL=/bin/bash LC_ALL=C LANG=C
    CARGO_BUILD_JOBS=2 COMPOSE_PARALLEL_LIMIT=1
    "SUDO_USER=${SUDO_USER:-}" "SUDO_UID=${SUDO_UID:-}"
  )
  for name in GITHUB_ACTIONS GITHUB_REPOSITORY GITHUB_REF GITHUB_EVENT_NAME \
    GITHUB_WORKFLOW_REF GITHUB_JOB GITHUB_SHA GITHUB_WORKSPACE \
    GITHUB_RUN_ID GITHUB_RUN_ATTEMPT RUNNER_OS RUNNER_ARCH RUNNER_ENVIRONMENT; do
    env_args+=("$name=${!name:-}")
  done
  exec /usr/bin/env -i "${env_args[@]}" /bin/bash "$script_path" "$commit" "$output_dir" \
    __HOSTED_RELEASE_CLEAN_ENV_V1__
fi

source "$script_dir/lib/release-image-manifest.sh"
source "$script_dir/lib/release-build-layout.sh"

safe_absolute_path() {
  local path=$1 label=$2
  release_image_manifest_require_absolute_path "$path" "$label" ||
    die "$RELEASE_IMAGE_MANIFEST_ERROR"
  case "$path" in
    /|/etc|/opt|/usr|/usr/local|/var|/var/lib|/tmp|/run)
      die "$label is too broad"
      ;;
  esac
}

check_runner_identity() {
  local helper=$script_dir/lib/hosted-release-build-policy.py
  [ -f "$helper" ] && [ ! -L "$helper" ] || die 'hosted runner policy is missing or symlinked'
  RELEASE_COMMIT=$commit RELEASE_SOURCE_ROOT=$root RELEASE_POLICY=$helper \
  GITHUB_ACTIONS=${GITHUB_ACTIONS:-} GITHUB_REPOSITORY=${GITHUB_REPOSITORY:-} \
  GITHUB_REF=${GITHUB_REF:-} GITHUB_EVENT_NAME=${GITHUB_EVENT_NAME:-} \
  GITHUB_WORKFLOW_REF=${GITHUB_WORKFLOW_REF:-} GITHUB_JOB=${GITHUB_JOB:-} \
  GITHUB_SHA=${GITHUB_SHA:-} GITHUB_WORKSPACE=${GITHUB_WORKSPACE:-} \
  GITHUB_RUN_ID=${GITHUB_RUN_ID:-} GITHUB_RUN_ATTEMPT=${GITHUB_RUN_ATTEMPT:-} \
  RUNNER_OS=${RUNNER_OS:-} RUNNER_ARCH=${RUNNER_ARCH:-} \
  RUNNER_ENVIRONMENT=${RUNNER_ENVIRONMENT:-} SUDO_USER=${SUDO_USER:-} SUDO_UID=${SUDO_UID:-} \
  python3 - <<'PY' || die 'host must be the fixed GitHub-hosted release-delivery runner'
import importlib.util
import os
import pwd
import subprocess
import sys

path=os.environ["RELEASE_POLICY"]
spec=importlib.util.spec_from_file_location("lagrange_hosted_release_policy",path)
if spec is None or spec.loader is None:
    raise SystemExit(1)
policy=importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)
source=os.environ["RELEASE_SOURCE_ROOT"]
remote=subprocess.run(
    ["git","-c","safe.directory="+source,"-C",source,"remote","get-url","origin"],
    stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,timeout=10,check=False)
if remote.returncode:
    raise SystemExit(1)
fields={name:os.environ.get(name,"") for name in (
    "GITHUB_ACTIONS","GITHUB_REPOSITORY","GITHUB_REF","GITHUB_EVENT_NAME",
    "GITHUB_WORKFLOW_REF","GITHUB_JOB","GITHUB_SHA","GITHUB_WORKSPACE",
    "GITHUB_RUN_ID","GITHUB_RUN_ATTEMPT","RUNNER_OS","RUNNER_ARCH",
    "RUNNER_ENVIRONMENT","SUDO_USER","SUDO_UID")}
with open("/etc/os-release",encoding="ascii") as handle:
    os_release=policy.parse_os_release(handle.read())
runner_uid=pwd.getpwnam("runner").pw_uid
source_info=os.stat(source,follow_symlinks=False)
policy.validate_runner_identity(
    fields=fields,os_release=os_release,machine=os.uname().machine,
    commit=os.environ["RELEASE_COMMIT"],source_root=source,
    source_owner_uid=source_info.st_uid,runner_uid=runner_uid,
    remote_url=remote.stdout.decode("utf-8").strip())
PY
}

check_source_checkout() {
  local head status origin status_line
  command -v git >/dev/null 2>&1 || die 'git is required'
  head=$(git -c "safe.directory=$root" -C "$root" rev-parse --verify 'HEAD^{commit}' 2>/dev/null) ||
    die 'checkout is not a Git worktree with a commit'
  [ "$head" = "$commit" ] || die 'COMMIT does not match checkout HEAD'
  origin=$(git -c "safe.directory=$root" -C "$root" remote get-url origin 2>/dev/null) ||
    die 'checkout origin is unavailable'
  case "$origin" in
    https://github.com/sihioov/Lagrange|https://github.com/sihioov/Lagrange.git) ;;
    *) die 'checkout origin does not match the fixed source repository' ;;
  esac
  status=$(git -c "safe.directory=$root" -C "$root" status --porcelain=v1 --untracked-files=all 2>/dev/null) ||
    die 'cannot inspect checkout status'
  while IFS= read -r status_line; do
    [ -z "$status_line" ] && continue
    [ "$status_line" = '?? docs/kis_openapi_entiredocs_20260818_030007.xlsx' ] ||
      die 'checkout is not clean'
  done <<<"$status"
}

check_output_parent() {
  safe_absolute_path "$output_dir" output-dir
  local parent
  parent=$(dirname -- "$output_dir")
  [ -d "$parent" ] && [ ! -L "$parent" ] || die 'output parent must exist and not be a symlink'
  release_image_manifest_trusted_directory "$parent" output-parent || die "$RELEASE_IMAGE_MANIFEST_ERROR"
  python3 - "$root" "$output_dir" "$parent" <<'PY' || die 'output path must be canonical and outside the checkout'
import os,sys
root,path,parent=map(os.path.abspath,sys.argv[1:])
if os.path.realpath(parent)!=parent or os.path.realpath(path)!=path or os.path.dirname(path)!=parent:
    raise SystemExit(1)
if os.path.commonpath((root,path))==root or os.path.commonpath((path,root))==path:
    raise SystemExit(1)
PY
  if [ -e "$output_dir" ] || [ -L "$output_dir" ]; then
    [ -d "$output_dir" ] && [ ! -L "$output_dir" ] || die 'OUTPUT_DIR is not a regular directory'
    [ "$(stat -c '%u:%g:%a' -- "$output_dir")" = 0:0:700 ] ||
      die 'OUTPUT_DIR must be root:root mode 0700'
    [ -z "$(find "$output_dir" -mindepth 1 -maxdepth 1 -print -quit)" ] ||
      die 'OUTPUT_DIR must be empty'
  else
    mkdir -m 0700 -- "$output_dir" || die 'cannot create OUTPUT_DIR'
  fi
  [ "$(stat -c '%u:%g:%a' -- "$output_dir")" = 0:0:700 ] ||
    die 'OUTPUT_DIR must be root:root mode 0700'
}

check_runner_identity
check_source_checkout
check_output_parent

output_ready=0
cleanup_output() {
  local status=$?
  trap - EXIT
  if [ "$output_ready" -ne 1 ]; then
    rm -f -- "$output_dir/release.manifest" "$output_dir/.release.manifest.partial"
    rm -rf -- "$output_dir/d8" "$output_dir/.d8-partial"
  fi
  exit "$status"
}
trap cleanup_output EXIT

state_parent=$(dirname -- "$output_dir")/.lagrange-hosted-build-state
if [ -e "$state_parent" ] || [ -L "$state_parent" ]; then
  [ -d "$state_parent" ] && [ ! -L "$state_parent" ] || die 'private state parent is unsafe'
  [ "$(stat -c '%u:%a' -- "$state_parent")" = 0:700 ] || die 'private state parent is not root-private'
else
  mkdir -m 0700 -- "$state_parent" || die 'cannot create private state parent'
fi
state_root=$state_parent/$commit
release_build_layout_init "$root" "$commit" "$state_root" product hosted ||
  die 'hosted release build layout initialization failed'

docker_config=$state_root/docker-config
if [ -e "$docker_config" ] || [ -L "$docker_config" ]; then
  [ -d "$docker_config" ] && [ ! -L "$docker_config" ] &&
    [ "$(stat -c '%u:%a' -- "$docker_config")" = 0:700 ] ||
    die 'private Docker config is unsafe'
else
  mkdir -m 0700 -- "$docker_config" || die 'cannot create private Docker config'
fi

docker_clean() {
  env -i PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    HOME=/root USER=root LOGNAME=root LC_ALL=C DOCKER_CONFIG="$docker_config" \
    docker "$@"
}

compose() {
  local override=${COMPOSE_BUILD_OVERRIDE_FILE:-}
  local -a compose_args=(--env-file "$root/deploy/compose/.env.example" --file "$root/deploy/compose/compose.yml")
  if [ -n "$override" ]; then compose_args+=(--file "$override"); fi
  env -i PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    HOME=/root USER=root LOGNAME=root LC_ALL=C DOCKER_CONFIG="$docker_config" \
    COMPOSE_PARALLEL_LIMIT=1 CARGO_BUILD_JOBS=2 COMPOSE_PROFILES= \
    LAGRANGE_CODE_COMMIT="$commit" RESEARCH_APP_ENV=prebuild-disabled \
    RESEARCH_FETCH_MODE=synthetic RESEARCH_ENTITLEMENT_REFERENCE=prebuild-disabled \
    RESEARCH_ENTITLEMENT_SHA256=0000000000000000000000000000000000000000000000000000000000000000 \
    RESEARCH_CANDIDATE_ENABLED=false BACKTEST_MIN_FREE_BYTES=0 \
    BACKTEST_MAX_QUEUED_BACKTESTS=0 BACKTEST_RECONCILE_GRACE_SECS=0 \
    BACKTEST_RECONCILE_INTERVAL_SECS=0 RANGE_RAW_BATCH_ID=compose-config-disabled \
    LIVE_NODE_MODE=disabled LIVE_NODE_DRY_RUN=1 \
    docker compose "${compose_args[@]}" "$@"
}

command -v docker >/dev/null 2>&1 || die 'Docker CLI is unavailable'
docker_clean version >/dev/null 2>&1 || die 'Docker daemon is unavailable'
docker_clean compose version >/dev/null 2>&1 || die 'Docker Compose v2 is unavailable'
docker_clean buildx version >/dev/null 2>&1 || die 'Docker Buildx is unavailable'
docker_platform=$(docker_clean info --format '{{.OSType}}/{{.Architecture}}' 2>/dev/null) ||
  die 'Docker daemon platform is unavailable'
case "$docker_platform" in linux/amd64|linux/x86_64) ;; *) die 'Docker daemon must be linux/amd64' ;; esac
docker_root=$(docker_clean info --format '{{.DockerRootDir}}' 2>/dev/null) ||
  die 'Docker data root is unavailable'
[ -n "$docker_root" ] && [ -d "$docker_root" ] || die 'Docker data root is invalid'
compose config --quiet || die 'synthetic Compose configuration is invalid'

report_disk_capacity() {
  local boundary=$1
  HOSTED_BUILD_STATE_ROOT=$state_root HOSTED_DOCKER_ROOT=$docker_root \
  HOSTED_DISK_BOUNDARY=$boundary python3 - <<'PY' || die 'disk capacity observation failed'
import os,re
boundary=os.environ["HOSTED_DISK_BOUNDARY"]
if boundary not in ("run-start","final") and not re.fullmatch(r"batch:B[1-5]",boundary):
    raise SystemExit(1)
for name,path in (("build-state",os.environ["HOSTED_BUILD_STATE_ROOT"]),
                  ("docker-store",os.environ["HOSTED_DOCKER_ROOT"])):
    if not os.path.isabs(path) or not os.path.isdir(path):
        raise SystemExit(1)
    values=os.statvfs(path)
    available=values.f_bavail*values.f_frsize
    total=values.f_blocks*values.f_frsize
    print(f"HOSTED_RELEASE_DISK boundary={boundary} volume={name} available_bytes={available} total_bytes={total}")
PY
}

report_disk_capacity run-start
release_build_layout_gate run-start 0 || die 'initial hosted release gate failed'

local_image_services=("${RELEASE_IMAGE_SERVICES[@]}")

inspect_one_image() {
  local service=$1 image_ref inspected image_id revision
  image_ref=$(release_image_manifest_ref_for "$service" "$commit") || die "cannot derive image reference: $service"
  inspected=$(docker_clean image inspect --format '{{.Id}}|{{index .Config.Labels "org.opencontainers.image.revision"}}' \
    "$image_ref" 2>/dev/null) || die "cannot inspect built image: $service"
  case "$inspected" in *'|'*) ;; *) die "built image revision is missing: $service" ;; esac
  image_id=${inspected%%|*}
  revision=${inspected#*|}
  release_image_manifest_is_image_id "$image_id" || die "image ID is malformed: $service"
  release_image_manifest_is_commit "$revision" || die "image revision is malformed: $service"
  [ "$revision" = "$commit" ] || die "image revision differs from source commit: $service"
  release_build_layout_verify_image "$service" "$commit" "$image_id" "$state_root" ||
    die "strict product image byte verification failed: $service"
  discard_verified_archive "$image_id"
  RELEASE_IMAGE_MANIFEST_REFS["$service"]=$image_ref
  RELEASE_IMAGE_MANIFEST_IDS["$service"]=$image_id
  RELEASE_IMAGE_MANIFEST_REVISIONS["$service"]=$revision
}

discard_verified_archive() {
  local image_id=$1 archive metadata
  release_image_manifest_is_image_id "$image_id" || die 'cannot derive verified image archive path'
  archive=$state_root/images/archive-${image_id#sha256:}.tar
  [ -f "$archive" ] && [ ! -L "$archive" ] || die 'verified image archive is missing or unsafe'
  metadata=$(stat -c '%u:%a' -- "$archive") || die 'cannot inspect verified image archive'
  [ "$metadata" = 0:600 ] || die 'verified image archive is not root-private'
  rm -f -- "$archive" || die 'cannot remove redundant verified image archive'
  [ ! -e "$archive" ] && [ ! -L "$archive" ] || die 'verified image archive remains after cleanup'
}

build_service() {
  local service=$1 bundle override
  echo "HOSTED_RELEASE_SERVICE service=$service status=start"
  bundle=$(release_build_layout_prepare "$service" "$commit" "$state_root") ||
    die "official build artifact preparation failed: $service"
  if [ "$bundle" = NONE ]; then
    COMPOSE_BUILD_OVERRIDE_FILE= compose build --pull=false "$service" ||
      die "Compose image build failed: $service"
  else
    override=$state_root/overrides/$service-$$.json
    release_build_layout_write_override "$service" "$bundle" "$override" ||
      die "verified artifact override failed: $service"
    COMPOSE_BUILD_OVERRIDE_FILE=$override compose build --pull=false "$service" ||
      die "Compose verified artifact build failed: $service"
  fi
  inspect_one_image "$service"
  echo "HOSTED_RELEASE_SERVICE service=$service status=success"
}

run_batch() {
  local batch=$1 service
  shift
  for service in "$@"; do build_service "$service"; done
  release_build_layout_gate "batch:$batch" 0 || die "hosted batch gate failed: $batch"
  report_disk_capacity "batch:$batch"
}

run_batch B1 db-role-bootstrap db-migrate api-server
run_batch B2 web research-worker recommendation-runner
run_batch B3 candidate-runner owner-beta-runner owner-equity-v2-runner
run_batch B4 nt-backtest-worker-1 nt-backtest-worker-2
run_batch B5 paper-scheduler owner-beta-simulation-runner
release_build_layout_gate final-bytes 0 || die 'final hosted image gate failed'
report_disk_capacity final

# Re-derive each image reference and image ID after all builds. A retargeted tag
# cannot silently replace bytes already bound to a saved archive.
for service in "${local_image_services[@]}"; do
  inspected=$(docker_clean image inspect --format '{{.Id}}|{{index .Config.Labels "org.opencontainers.image.revision"}}' \
    "${RELEASE_IMAGE_MANIFEST_REFS[$service]}" 2>/dev/null) || die "final image inspection failed: $service"
  image_id=${inspected%%|*}
  revision=${inspected#*|}
  [ "$image_id" = "${RELEASE_IMAGE_MANIFEST_IDS[$service]}" ] &&
    [ "$revision" = "${RELEASE_IMAGE_MANIFEST_REVISIONS[$service]}" ] &&
    [ "$revision" = "$commit" ] || die "final image identity changed: $service"
done

export_d8_sidecars() {
  local bundle=$state_root/bundles/D8
  local producer=$bundle/.release-build/producer/owner-beta-simulation-runner
  local destination=$output_dir/.d8-partial
  [ -d "$producer" ] && [ ! -L "$producer" ] || die 'verified D8 producer is missing'
  release_build_layout_verify_bundle owner-beta-simulation-runner "$commit" "$bundle" >/dev/null ||
    die 'official D8 release bundle no longer verifies'
  mkdir -m 0700 -- "$destination" || die 'cannot create D8 staging directory'
  for name in artifact.json COMPLETE; do
    [ -f "$producer/$name" ] && [ ! -L "$producer/$name" ] || die "D8 sidecar is missing: $name"
    install -o 0 -g 0 -m 0644 -- "$producer/$name" "$destination/$name" ||
      die "cannot stage D8 sidecar: $name"
  done
  RELEASE_D8_DIR=$destination RELEASE_SOURCE_ROOT=$root RELEASE_COMMIT=$commit \
    python3 - <<'PY' || die 'installed D8 build-receipt validator rejected hosted output'
import os,runpy,stat
policy=runpy.run_path(os.path.join(os.environ["RELEASE_SOURCE_ROOT"],
    "scripts/ops/lib/owner-beta-simulation-release.py"))
validate=policy.get("validate_build_receipt")
if not callable(validate): raise SystemExit(1)
directory=os.environ["RELEASE_D8_DIR"]
for name in ("artifact.json","COMPLETE"):
    info=os.lstat(os.path.join(directory,name))
    if (stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode) or
            info.st_uid!=0 or stat.S_IMODE(info.st_mode)!=0o644):
        raise SystemExit(1)
artifact=open(os.path.join(directory,"artifact.json"),"rb").read()
complete=open(os.path.join(directory,"COMPLETE"),"rb").read()
validate(artifact,complete,os.environ["RELEASE_COMMIT"])
PY
  mv -- "$destination" "$output_dir/d8" || die 'cannot publish D8 sidecars'
}

write_manifest() {
  local temporary=$output_dir/.release.manifest.partial
  [ ! -e "$temporary" ] && [ ! -L "$temporary" ] || die 'stale manifest staging path'
  if ! release_image_manifest_write "$temporary" "$commit"; then
    rm -f -- "$temporary"
    die "strict V3 manifest write failed: $RELEASE_IMAGE_MANIFEST_ERROR"
  fi
  chmod 0600 -- "$temporary"
  release_image_manifest_load "$temporary" "$commit" || {
    rm -f -- "$temporary"
    die "strict V3 manifest validation failed: $RELEASE_IMAGE_MANIFEST_ERROR"
  }
  ln -- "$temporary" "$output_dir/release.manifest" || {
    rm -f -- "$temporary"
    die 'release manifest already exists'
  }
  rm -f -- "$temporary"
  [ "$(stat -c '%u:%g:%a' -- "$output_dir/release.manifest")" = 0:0:600 ] ||
    die 'release manifest is not root:root mode 0600'
  release_image_manifest_trusted_file "$output_dir/release.manifest" release-manifest ||
    die "$RELEASE_IMAGE_MANIFEST_ERROR"
}

export_d8_sidecars
write_manifest
[ "$(find "$output_dir" -mindepth 1 -maxdepth 1 -printf '%f\n' | LC_ALL=C sort)" = $'d8\nrelease.manifest' ] ||
  die 'OUTPUT_DIR contains an unexpected file'
output_ready=1
echo "HOSTED_RELEASE_BUILD: PASS commit=$commit platform=linux/amd64 images=13 d8=release"
