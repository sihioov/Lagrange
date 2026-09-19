#!/usr/bin/env bash
# Static contract check for the G2 common-artifact production image path. It
# never calls Docker, a provider, a database, or a runtime service.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ops=$root/scripts/ops
build=$ops/build-production-images.sh
compose_release=$ops/compose-release.sh
manifest_lib=$ops/lib/release-image-manifest.sh
layout_helper=$ops/lib/release-build-layout.sh
layout_config=$root/deploy/build/release-build-layout.json
artifact_dockerfile=$root/deploy/build/Dockerfile.rust-artifacts
compose_file=$root/deploy/compose/compose.yml

die() { echo "production-image-build-static: $*" >&2; exit 1; }

require_file() {
  local path=$1 label=$2
  [ -f "$path" ] && [ ! -L "$path" ] || die "$label is missing or a symlink: $path"
}

require_literal() {
  local path=$1 literal=$2 label=$3
  grep -Fq -- "$literal" "$path" || die "$label: $literal"
}

require_function() {
  local path=$1 name=$2
  grep -Eq "^[[:space:]]*(function[[:space:]]+)?${name}[[:space:]]*(\(\))?[[:space:]]*\{" "$path" ||
    die "required helper API is missing: $name"
}

require_schema_field() {
  local path=$1 field=$2 label=$3
  grep -Eq "['\"]${field}['\"]" "$path" || die "$label: $field"
}

require_any_regex() {
  local pattern=$1 label=$2
  shift 2
  local path
  for path in "$@"; do
    grep -Eq -- "$pattern" "$path" && return 0
  done
  die "$label"
}

reject_source_pattern() {
  local path=$1 pattern=$2 label=$3
  if grep -Eiq "$pattern" "$path"; then
    die "$label"
  fi
}

for path in "$build" "$compose_release" "$manifest_lib" "$layout_helper"; do
  require_file "$path" 'required production helper'
  bash -n "$path" || die "shell syntax failure: $path"
done
require_file "$layout_config" 'frozen layout config'
require_file "$artifact_dockerfile" 'artifact producer Dockerfile'

self_test=$ops/build-production-images-self-test.sh
bash -n "$self_test" || die 'image build self-test has shell syntax errors'
require_literal "$self_test" 'TEST_ENVIRONMENT_ERROR: image-build root fixture requires user namespaces or fakeroot' \
  'root fixture must fail rather than skip when unavailable'
if grep -Fq 'PASS (apply fixture skipped' "$self_test" || grep -Fq 'fakeroot "$@"' "$self_test"; then
  die 'image build root fixture must not pass-on-skip or nest fakeroot'
fi

# The accepted smoke tool is not part of the G2 implementation change. Keep
# its executable, isolated-fixture, cache-namespace and complete scenario
# contract visible here while the benchmark grows whole-release coverage.
smoke=$root/scripts/qa/build-cache-smoke.sh
benchmark=$root/scripts/qa/build-cache-benchmark.sh
for cache_tool in "$smoke" "$benchmark"; do
  require_file "$cache_tool" 'build-cache verification tool'
  [ -x "$cache_tool" ] || die "build-cache verification tool must be executable: $cache_tool"
  bash -n "$cache_tool" || die "build-cache verification tool has shell syntax errors: $cache_tool"
  require_literal "$cache_tool" 'mode=plan' 'build-cache tool must default to plan'
  require_literal "$cache_tool" '--apply' 'build-cache tool apply mode is missing'
  require_literal "$cache_tool" '--self-test' 'build-cache tool self-test mode is missing'
done
for literal in \
  'BUILDKIT_CACHE_MOUNT_NS' \
  '--build-arg "BUILDKIT_CACHE_MOUNT_NS=$namespace"' \
  '--output-dir' \
  '--no-cache' \
  'docker run --rm' \
  'compile-layer-cached' \
  'SMOKE_CARGO_PHASE bin=cache-bin-a' \
  'SMOKE_CARGO_PHASE bin=cache-bin-b'
do
  require_literal "$smoke" "$literal" 'accepted cache-smoke contract is missing'
done
for smoke_case in cache-absent repeat-same-source different-bin changed-local-source \
  changed-workspace-library changed-build-script-source changed-external-dependency-lockfile \
  changed-embedded-data changed-build-setting commit-only-change older-mtimes-branch-reversion \
  source-deletion-failure restore-after-source-deletion cache-repopulate
do
  require_literal "$smoke" "$smoke_case" 'accepted cache-smoke scenario is missing'
done
for literal in \
  '--baseline-commit' '--candidate-commit' '--output-dir' \
  'git clone --quiet --no-local' 'CARGO_BUILD_JOBS=2' \
  '--repetitions' '--order' 'release_build_layout_lock' \
  'LAGRANGE_BENCHMARK_COLD_NONCE' 'strict V2' \
  'complete fixed window' \
  'd1baf9da9b13fcb61649b1c26de56aed87a83418' \
  'f4eb4f83abb7c3f43ede072d0077a1a74edd0ee3' \
  'side_product_kind' 'publish_source_manifest_and_revalidate' \
  'publish_common_manifest_and_revalidate' \
  'source-manifests.tsv' 'release-totals.tsv' 'release-comparison.tsv' \
  'BENCHMARK_SYSTEMD_SERVICE->RELEASE_BUILD_SYSTEMD_UNIT' \
  'write_benchmark_compose_env' \
  'df9d4d1ceb45d0ddb79b98b1fc12c5a2925424c46b79b5d9959f0a1640b27bf6' \
  'inputs/image-only-compose.env' \
  'inactive-research-entitlement-sentinel-only' \
  'BENCHMARK_DRAINED_READERS_ATTESTATION' \
  'production-drained-readers-attestation-unsupported' \
  '${TMPDIR:-/tmp}/lagrange-build-cache-benchmark-self-test.XXXXXXXXXX' \
  'ls-files --error-unmatch deploy/compose/.env'
do
  require_literal "$benchmark" "$literal" 'whole-release benchmark contract is missing'
done
require_literal "$benchmark" 'compose --env-file "$compose_env"' \
  'common-C benchmark must pass its explicit private image-only Compose env'
if grep -Fq '$checkout/deploy/compose/.env' "$benchmark"; then
  die 'benchmark must not read or require an operational Compose env from a clean checkout'
fi
if grep -Eiq '(^|[[:space:]])(curl|wget|psql)([[:space:]]|$)|KIS_APP_(KEY|SECRET)' "$smoke" "$benchmark"; then
  die 'build-cache verification tools contain a forbidden provider/credential channel'
fi

# The immutable config is deliberately private in representation, but its JSON
# syntax/duplicate-key behavior and frozen common declaration are public G2
# requirements. Do not turn this into a second config schema in the checker.
python3 - "$layout_config" <<'PY' || die 'layout config is not strict JSON with a common declaration'
import json, sys
def pairs(items):
    value = {}
    for key, item in items:
        if key in value:
            raise ValueError("duplicate-json-key")
        value[key] = item
    return value
def reject_constant(value):
    raise ValueError("non-finite-json-number")
try:
    with open(sys.argv[1], "r", encoding="utf-8", newline="") as handle:
        value = json.load(handle, object_pairs_hook=pairs, parse_constant=reject_constant)
except (OSError, ValueError, json.JSONDecodeError) as exc:
    raise SystemExit(str(exc))
if not isinstance(value, dict):
    raise SystemExit("config-not-object")
def strings(item):
    if isinstance(item, str):
        yield item
    elif isinstance(item, list):
        for child in item:
            yield from strings(child)
    elif isinstance(item, dict):
        for key, child in item.items():
            yield key
            yield from strings(child)
if "common" not in set(strings(value)):
    raise SystemExit("common-declaration-missing")
PY

# The config must still describe every frozen D7 binary and every canonical
# service binding. Searching the config instead of Dockerfile RUN spelling
# keeps this check about the selected product contract rather than B's shape.
for token in \
  db-role-bootstrap db-migrate api-server web research-worker \
  recommendation-runner candidate-runner owner-beta-runner owner-equity-v2-runner \
  nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler \
  backtest-runner paper-runner \
  kis-historical-price-beta-artifact kis-historical-price-v3-artifact \
  kis-historical-price-beta-approval-check kis-action-range-raw \
  kis-stock-price-beta-raw kis-stock-price-beta-materialize \
  kis-historical-price-v3-input-check owner-equity-v2-check owner-equity-v2-materialize
do
  require_literal "$layout_config" "$token" "frozen layout config omitted required service/bin"
done

# Preserve the old public builder fences: plan/preflight/apply shape, root
# ownership, clean HEAD, V2 manifest no-clobber, IDs/revisions, and complete
# canonical final service order. The compiler implementation underneath has
# changed; these release-facing guarantees have not.
for literal in \
  'mode=plan' '--preflight' '--apply' '--manifest-file' \
  '--apply requires --manifest-file' 'id -u' \
  'status --porcelain=v1 --untracked-files=all' \
  '?? docs/kis_openapi_entiredocs_20260818_030007.xlsx' \
  "rev-parse --verify 'HEAD^{commit}'" \
  'does not match the build root HEAD' \
  'manifest-file already exists; refusing to overwrite it' \
  'validate_manifest_output' 'docker image inspect' \
  'built image_id is not an exact local Docker image ID' \
  'built image revision label does not match source commit' \
  'final image_id differs from byte-verified image' \
  'release_image_manifest_write' \
  'release_image_manifest_load' 'COMPOSE_PARALLEL_LIMIT=1'
do
  require_literal "$build" "$literal" 'public builder safety contract is missing'
done
for literal in \
  'IMAGE_BUILD_FAKE_TAG_SWAP_SERVICE' \
  'changed same-revision final image tag unexpectedly passed' \
  'tag swap did not preserve twelve valid saved roots and exact per-service final identity binding' \
  'expect_host_path_reject' \
  'dot dotdot relative redundant-separator symlink-ancestor' \
  'host-path-not-canonical'
do
  require_literal "$self_test" "$literal" 'focused image verification regression is missing'
done
for literal in \
  'RELEASE_BUILD_DRAINED_READERS_ATTESTATION' \
  'lagrange-station-research-worker-1' \
  'lagrange-station-owner-equity-v2-runner-1' \
  'IMAGE_BUILD_FAKE_DRAINED_DRIFT' \
  'drained-run-start-drift'
do
  require_literal "$self_test" "$literal" 'drained-reader official fake apply regression is missing'
done
require_literal "$build" 'source "$script_dir/lib/release-build-layout.sh"' \
  'official builder must source the G2 layout helper'
require_literal "$build" 'for service in "${local_image_services[@]}"; do' \
  'official builder must retain the canonical sequential service loop'
require_literal "$build" 'build --pull=false "$service"' \
  'official builder must issue one-service Compose builds'
require_any_regex 'git[[:space:]]+-c[[:space:]]+"?safe\.directory=' \
  'official build provenance must use command-local Git safe.directory' "$build" "$layout_helper"
for literal in \
  'config --quiet' \
  'RESEARCH_APP_ENV=prebuild-disabled' \
  'RESEARCH_ENTITLEMENT_REFERENCE=prebuild-disabled' \
  'BACKTEST_MIN_FREE_BYTES=0' \
  'never edits the env file' \
  'network caveat'
do
  require_literal "$build" "$literal" 'public builder safety contract is missing'
done
if grep -Fq -- '--repo-root' "$build"; then
  die 'public alternate repo-root override must not exist'
fi

require_literal "$manifest_lib" "RELEASE_IMAGE_MANIFEST_FORMAT='LAGRANGE_RELEASE_MANIFEST_V2'" \
  'strict V2 manifest header is missing or changed'
for literal in \
  'canonical whitelist/order' 'manifest record count is not canonical' \
  'trailing separator' 'manifest configured image reference disagrees with Compose' \
  'local Docker image ID'
do
  require_literal "$manifest_lib" "$literal" 'strict V2 manifest contract is missing'
done
if grep -Eq '\.RepoDigests|RepoDigest[s]?[[:space:]:=]' "$build"; then
  die 'build manifest must not claim Docker RepoDigest provenance'
fi

# Compose remains the unchanged declarative source for the twelve image tags
# and source-commit build arguments. Preserve these provenance bindings without
# coupling the static check to the internal artifact-stage Dockerfile layout.
service_block() {
  local service=$1
  awk -v service="$service" '
    $0 == "  " service ":" { inside=1; next }
    inside && /^  [A-Za-z0-9_-]+:/ { exit }
    inside { print }
  ' "$compose_file"
}

# G2 helper public API. Its implementation-local receipt and ledger shapes are
# intentionally not duplicated here; focused fake tests exercise them through
# these interfaces and the frozen config.
for function in \
  release_build_layout_lock \
  release_build_layout_init \
  release_build_layout_plan \
  release_build_layout_gate \
  release_build_layout_prepare \
  release_build_layout_verify_bundle \
  release_build_layout_write_override \
  release_build_layout_verify_image \
  release_build_layout_archive_scan
do
  require_function "$layout_helper" "$function"
done
for literal in \
  'RELEASE_BUILD_SYSTEMD_UNIT' 'RELEASE_BUILD_SYSTEMD_MANAGER' \
  'RELEASE_BUILD_HEALTH_UNITS' 'RELEASE_BUILD_HEALTH_CONTAINERS' \
  'RELEASE_BUILD_RESEARCH_EXCEPTION' 'RELEASE_BUILD_DRAINED_READERS_ATTESTATION' \
  'lagrange-build-drained-readers-attestation-v1' 'image-build-only-drained-readers' \
  'drained_readers_attestation' 'State.StartedAt' 'State.FinishedAt' \
  'lagrange-image-files-v1' 'lagrange-image-files-result-v1' \
  'RUST_ARTIFACT_HELPER_SHA256' 'RUST_ARTIFACT_BUNDLE_SHA256' \
  'linux/amd64' 'whiteout'
do
  if ! grep -Fq -- "$literal" "$layout_helper" && ! grep -Fq -- "$literal" "$artifact_dockerfile" &&
     ! grep -Fq -- "$literal" "$build"; then
    die "G2 helper/artifact contract is missing: $literal"
  fi
done
require_literal "$layout_helper" 'os.path.normpath(value)!=value' \
  'archive API must reject noncanonical dot/dotdot/redundant caller text before normalization'
require_literal "$layout_helper" 'os.path.realpath(value)!=value' \
  'archive API must reject caller paths with a symlinked ancestor'
require_any_regex 'EM_X86_64|prefix\[18:20\].*62|struct\.[A-Za-z_]+\([^)]*62' \
  'archive parser must check the linux/amd64 ELF machine, not magic alone' \
  "$layout_helper" "$artifact_dockerfile" "$build"
require_any_regex 'opaque|\.wh\.\.wh\.\.opq' \
  'archive parser must apply opaque-directory markers' \
  "$layout_helper" "$artifact_dockerfile" "$build"
require_any_regex '1024[[:space:]]*\*[[:space:]]*1024' \
  'archive parser must retain one-MiB bounded chunk handling' \
  "$layout_helper" "$artifact_dockerfile" "$build"

# Every source fallback retains its own direct-build cache route, while the
# common producer uses one artifact Dockerfile. Inspect all cache mounts rather
# than accepting a single incidental `sharing=locked` token: registry, Git,
# and target must each be locked everywhere. The producer target must bind the
# namespace plus K/platform/release/guard identity (never the source commit),
# and the seven fallback recipes must retain one common target-cache identity.
d7_dockerfiles=(
  crates/api-server/Dockerfile
  crates/job-queue/Dockerfile
  crates/job-queue/Dockerfile.owner-beta-runner
  crates/job-queue/Dockerfile.owner-equity-v2-runner
  crates/job-queue/Dockerfile.backtest-runner
  data-pipelines/collectors/Dockerfile
  deploy/runtime/Dockerfile.paper-runner
)
for relative in "${d7_dockerfiles[@]}"; do
  require_file "$root/$relative" 'D7 Dockerfile'
done
if ! python3 - "$artifact_dockerfile" "${d7_dockerfiles[@]/#/$root/}" <<'PY'
import re
import sys

artifact, *fallbacks = sys.argv[1:]
targets = ("/usr/local/cargo/registry", "/usr/local/cargo/git", "/cargo-target")

def logical_instructions(path):
    try:
        lines = open(path, encoding="utf-8", newline="").read().splitlines()
    except OSError:
        raise SystemExit("cache-source-unreadable")
    result, current = [], ""
    for raw in lines:
        line = raw.rstrip()
        current = (current + " " + line.lstrip()).strip() if current else line
        if current.endswith("\\"):
            current = current[:-1].rstrip()
            continue
        result.append(current)
        current = ""
    if current:
        raise SystemExit("unterminated-docker-instruction")
    return result

def cache_mounts(path):
    found = []
    for instruction in logical_instructions(path):
        for token in re.findall(r"--mount=([^\s]+)", instruction):
            pairs = {}
            for item in token.split(","):
                if "=" not in item:
                    raise SystemExit("cache-mount-option-invalid")
                key, value = item.split("=", 1)
                if key in pairs:
                    raise SystemExit("cache-mount-duplicate-option")
                pairs[key] = value
            if pairs.get("type") == "cache":
                found.append(pairs)
    return found

def selected(path, mounts):
    matches = {target: [mount for mount in mounts if mount.get("target") == target]
               for target in targets}
    for target, values in matches.items():
        if not values:
            raise SystemExit(f"cache-mount-missing:{path}:{target}")
        for value in values:
            if value.get("sharing") != "locked":
                raise SystemExit(f"cache-mount-not-locked:{path}:{target}")
            cache_id = value.get("id")
            if not cache_id:
                raise SystemExit(f"cache-mount-id-missing:{path}:{target}")
            if "LAGRANGE_CODE_COMMIT" in cache_id:
                raise SystemExit(f"cache-mount-keys-on-source-commit:{path}:{target}")
    return matches

artifact_matches = selected(artifact, cache_mounts(artifact))
for target, values in artifact_matches.items():
    for value in values:
        cache_id = value["id"]
        if "RUST_ARTIFACT_CACHE_NAMESPACE" not in cache_id:
            raise SystemExit(f"producer-cache-namespace-missing:{target}")
        if target == "/cargo-target":
            required = ("RUST_ARTIFACT_CACHE_KEY", "linux-amd64-release-common-1")
            if any(part not in cache_id for part in required):
                raise SystemExit("producer-target-K-platform-release-guard-missing")

fallback_ids = {target: set() for target in targets}
for fallback in fallbacks:
    matches = selected(fallback, cache_mounts(fallback))
    for target, values in matches.items():
        fallback_ids[target].update(value["id"] for value in values)
for target, ids in fallback_ids.items():
    if len(ids) != 1:
        raise SystemExit(f"D7-cache-identity-not-common:{target}")
PY
then
  die 'cache mount locking/common-K contract is invalid'
fi

require_literal "$artifact_dockerfile" 'CARGO_BUILD_JOBS=2' \
  'common producer must retain the fixed Cargo jobs=2 setting'
if grep -Eiq '(^|[^[:alnum:]_])(docker[[:space:]]+system[[:space:]]+prune|docker[[:space:]]+builder[[:space:]]+prune|cargo[[:space:]]+clean[[:space:]]+--workspace)' \
  "$layout_helper" "$build"; then
  die 'G2 helper must not prune global state or clean every successful workspace artifact'
fi

# The exact archive request/result schema is frozen even though the parser is
# internal. Reject accidental weakening to a generic option bag or an unbounded
# in-memory image/layer read.
for literal in \
  format files nonempty_directories \
  path sha256 executable elf contains_hex \
  image_id manifest_digest config_digest \
  archive_sha256 request_sha256
do
  require_schema_field "$layout_helper" "$literal" 'archive API schema is incomplete'
done
reject_source_pattern "$layout_helper" 'tar[[:space:]].*(-x|--extract)|docker[[:space:]]+(run|compose[[:space:]].*(up|run|start|restart))' \
  'layout helper must not extract archives or invoke lifecycle commands'

# D7 source fallback remains available, but the verified named-context route
# has to bind the helper and bundle independently before using either. These
# checks intentionally avoid asserting the old source Dockerfile RUN layout.
for relative in "${d7_dockerfiles[@]}"; do
  dockerfile=$root/$relative
  require_file "$dockerfile" 'D7 Dockerfile'
  require_literal "$dockerfile" 'cargo build --locked --release' \
    "D7 source fallback no longer retains a locked release compile route: $relative"
  require_literal "$dockerfile" 'RUST_ARTIFACT_BUNDLE_SHA256' \
    "D7 verified artifact binding is missing: $relative"
  require_literal "$dockerfile" 'RUST_ARTIFACT_HELPER_SHA256' \
    "D7 helper byte binding is missing: $relative"
done

# The diagram evidence cites the Paper source compile command by line. Verify
# its live target rather than pinning the entire Dockerfile's old instruction
# shape. If WP4 moves that source command, the diagram source/PNG must be
# updated by the coordinator after the code path stabilizes.
paper_diagram=$root/docs/diagrams/runtime_deployment.puml
paper_citation=$(grep -oE 'deploy/runtime/Dockerfile\.paper-runner:[0-9]+' "$paper_diagram" | head -n1 || true)
[ -n "$paper_citation" ] || die 'runtime diagram lacks Paper Dockerfile evidence'
paper_line=${paper_citation##*:}
paper_source_line=$(sed -n "${paper_line}p" "$root/deploy/runtime/Dockerfile.paper-runner")
grep -Fq 'cargo build --locked --release --package api-server --bin paper-runner' <<<"$paper_source_line" ||
  die 'Paper runtime diagram evidence no longer points to the source fallback compile command'

services=(
  db-role-bootstrap db-migrate api-server web research-worker
  recommendation-runner candidate-runner owner-beta-runner owner-equity-v2-runner
  nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler
)
declare -A service_dockerfile=(
  [db-role-bootstrap]=deploy/db/Dockerfile
  [db-migrate]=deploy/db/Dockerfile
  [api-server]=crates/api-server/Dockerfile
  [web]=apps/web/Dockerfile
  [research-worker]=data-pipelines/collectors/Dockerfile
  [recommendation-runner]=crates/job-queue/Dockerfile
  [candidate-runner]=crates/job-queue/Dockerfile
  [owner-beta-runner]=crates/job-queue/Dockerfile.owner-beta-runner
  [owner-equity-v2-runner]=crates/job-queue/Dockerfile.owner-equity-v2-runner
  [nt-backtest-worker-1]=crates/job-queue/Dockerfile.backtest-runner
  [nt-backtest-worker-2]=crates/job-queue/Dockerfile.backtest-runner
  [paper-scheduler]=deploy/runtime/Dockerfile.paper-runner
)
grep -Fq 'local_image_services=("${RELEASE_IMAGE_SERVICES[@]}")' "$build" ||
  die 'build helper must derive scope only from the canonical manifest whitelist'
for service in "${services[@]}"; do
  require_literal "$manifest_lib" "  $service" "manifest whitelist omitted service"
  block=$(service_block "$service")
  [ -n "$block" ] || die "Compose service block missing: $service"
  grep -Fq "image: lagrange-station-$service:\${LAGRANGE_CODE_COMMIT:?" <<<"$block" ||
    die "Compose image must retain an exact commit-specific tag: $service"
  grep -Fq 'LAGRANGE_CODE_COMMIT: ${LAGRANGE_CODE_COMMIT:?' <<<"$block" ||
    die "Compose build argument missing for local image: $service"
  dockerfile=$root/${service_dockerfile[$service]}
  require_file "$dockerfile" "production Dockerfile for $service"
  require_literal "$dockerfile" 'ARG LAGRANGE_CODE_COMMIT' \
    "source fallback build ARG is missing: $service"
  require_literal "$dockerfile" 'LABEL org.opencontainers.image.revision="$LAGRANGE_CODE_COMMIT"' \
    "source fallback OCI revision label is missing: $service"
  require_literal "$dockerfile" "grep -Eq '^[0-9a-f]{40}$'" \
    "source fallback exact revision validation is missing: $service"
done

# Preserve no-provider/no-lifecycle guarantees in both the public builder and
# helper. Compose configuration/build are the only Docker channels permitted.
for path in "$build" "$layout_helper"; do
  reject_source_pattern "$path" '(^|[[:space:]])(curl|wget|psql)([[:space:]]|$)|KIS_APP_(KEY|SECRET)|AUTH0_CLIENT_SECRET|(^|[^[:alnum:]_])(CANO|ACNT_PRDT_CD|KIS_ACCOUNT_REF)([^[:alnum:]_]|$)' \
    "image build path contains a forbidden provider/credential channel: $path"
  reject_source_pattern "$path" '(^|[[:space:]])docker[[:space:]]+compose[[:space:]].*(up|run|restart|start)([[:space:]]|$)' \
    "image build path contains a forbidden lifecycle command: $path"
done

echo 'PRODUCTION_IMAGE_BUILD_STATIC: PASS'
