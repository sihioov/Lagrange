#!/usr/bin/env bash
set -euo pipefail
exec /usr/bin/env -i PATH=/usr/bin:/bin:/usr/local/bin LANG=C.UTF-8 python3 - "$@" <<'PY'
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import pathlib
import re
import secrets
import select
import signal
import stat
import subprocess
import sys
import time
import types
from dataclasses import dataclass
from typing import Callable

BUILD_RESERVE_NS = 120_000_000_000
SERVICES = (
    "db-role-bootstrap", "db-migrate", "api-server", "web", "research-worker",
    "recommendation-runner", "candidate-runner", "owner-beta-runner",
    "owner-equity-v2-runner", "nt-backtest-worker-1", "nt-backtest-worker-2",
    "paper-scheduler", "owner-beta-simulation-runner",
)
NONE_SERVICES = frozenset({"db-role-bootstrap", "db-migrate", "web"})
SOURCE_EVIDENCE_FILES = (
    "scripts/ops/build-paper-fixture-images.sh",
    "scripts/ops/test_paper_fixture_images.py",
    "scripts/ops/lib/release-build-layout.sh",
    "scripts/ops/lib/release-image-manifest.sh",
    "scripts/ops/lib/build-resource-policy.py",
    "scripts/ops/build-production-images.sh",
    "deploy/build/release-build-layout.json",
    "deploy/compose/compose.yml",
    "crates/api-server/Dockerfile",
    "deploy/runtime/Dockerfile.paper-runner",
)
RECEIPT_BINS = {
    "D1": frozenset({"api-server"}),
    "D7": frozenset({"paper-runner", "owner-beta-paper-ops-native"}),
    "D8": frozenset({"owner-beta-simulation-runner"}),
}
OBSERVATION_BINS = {
    "D1": ("api-server",), "D2": ("recommendation-runner", "candidate-runner"),
    "D3": ("owner-beta-runner",), "D4": ("owner-equity-v2-runner", "kis-market-stream-state"),
    "D5": ("backtest-runner",), "D6": ("research-worker", "kis-historical-price-beta-artifact",
        "kis-historical-price-v3-artifact", "kis-historical-price-beta-approval-check", "kis-action-range-raw",
        "kis-stock-price-beta-raw", "kis-stock-price-beta-materialize", "kis-historical-price-v3-input-check",
        "owner-equity-v2-check", "owner-equity-v2-materialize"),
    "D7": ("paper-runner", "owner-beta-paper-ops-native"), "D8": ("owner-beta-simulation-runner",),
}
COMMIT_RE = re.compile(r"[0-9a-f]{40}\Z")
STATE_RE = re.compile(r"lagrange-paper-fixture-build-[a-z0-9][a-z0-9-]{7,63}\Z")
NAME_RE = re.compile(r"[a-z0-9][a-z0-9_.-]{0,127}\Z")
HELPER_OPS = frozenset({
    "PLAN", "MAKE_CONTEXT", "PRODUCE_NATIVE", "PRODUCE_BIN",
    "BUNDLE_CREATE", "BUNDLE_VERIFY", "WRITE_OVERRIDE", "PREPARE_NONE",
    "COMPOSE_BUILD", "VERIFY_IMAGE", "PUBLISH_TEMP", "QUIT",
})
PRODUCER_PHASES = frozenset({
    "NATIVE_IDENTITY", "CURRENT_K", "REQUEST_K", "STALE_K",
    "PRIVATE_OUTPUT", "RECIPE", "ATTEMPT", "OUTPUT_PRODUCER",
    "OUTPUT_NATIVE", "VERIFY_PRODUCER", "NO_CLOBBER", "MOVE",
    "NATIVE_IDENTITY_COMPARE", "NATIVE_IDENTITY_INSTALL", "OUTPUT_PATH",
})
PRODUCER_PHASE_RE = re.compile(r"RBL_PRODUCE_BIN_PHASE\|([A-Z_]+)\|([1-9][0-9]{0,2})\Z")
CAPTURE_ATTEMPT_PHASES = (
    "SHELL_VALIDATE", "PYTHON_SETUP", "HISTORY_VALIDATE", "PRIOR_ARCHIVE",
    "ACTIVE_LOG_OPEN", "CHILD_PENDING", "CHILD_RETURNED", "CHILD_SPAWN_ERROR",
    "LOG_SYNC", "COMPLETED_ARCHIVE", "COMPLETED_RECEIPT", "COMPLETE",
)
CAPTURE_ATTEMPT_PHASE_SET = frozenset(CAPTURE_ATTEMPT_PHASES)
CAPTURE_ATTEMPT_RE = re.compile(
    r"RBL_CAPTURE_ATTEMPT\|([A-Z_]+)\|(UNKNOWN|SPAWN_ERROR|-?(?:0|[1-9][0-9]{0,2}))\Z"
)
ATTEMPT_PROGRESS_PREFIX = "PAPER_FIXTURE_IMAGE_PROGRESS "
ATTEMPT_PROGRESS_FORMAT = "lagrange-paper-fixture-attempt-progress-v1"
ATTEMPT_PROGRESS_OPS = frozenset({
    "PREPARE_NONE", "MAKE_CONTEXT", "PRODUCE_NATIVE", "PRODUCE_BIN",
    "BUNDLE_CREATE", "BUNDLE_VERIFY", "WRITE_OVERRIDE", "COMPOSE_BUILD",
    "VERIFY_IMAGE",
})
ATTEMPT_CAPTURE_NEXT = {
    None: {"SHELL_VALIDATE"},
    "SHELL_VALIDATE": {"PYTHON_SETUP"},
    "PYTHON_SETUP": {"HISTORY_VALIDATE"},
    "HISTORY_VALIDATE": {"PRIOR_ARCHIVE"},
    "PRIOR_ARCHIVE": {"ACTIVE_LOG_OPEN"},
    "ACTIVE_LOG_OPEN": {"CHILD_PENDING"},
    "CHILD_PENDING": {"CHILD_RETURNED", "CHILD_SPAWN_ERROR"},
    "CHILD_RETURNED": {"LOG_SYNC"},
    "CHILD_SPAWN_ERROR": {"LOG_SYNC"},
    "LOG_SYNC": {"COMPLETED_ARCHIVE"},
    "COMPLETED_ARCHIVE": {"COMPLETED_RECEIPT"},
    "COMPLETED_RECEIPT": {"COMPLETE"},
}
CHILD_FAILURE_CODES = frozenset("""
archive-format-invalid batch-shape-invalid builder-binary-invalid builder-cargo-build-failed builder-commit-env-mismatch builder-compile-env-value-invalid builder-contract-invalid builder-empty-clean-package builder-executable-missing builder-executable-not-executable builder-feature-contract-invalid builder-features-changed-before-cargo builder-h-binding-invalid builder-helper-hash-binding-invalid builder-layout-hash-binding-invalid builder-package-cycle builder-package-hash-invalid builder-package-set-invalid builder-path-contract-invalid builder-pinned-cargo-metadata-failed builder-recipe-binding-invalid builder-request-format-invalid builder-request-hash-mismatch builder-request-target-mismatch builder-request-tools-missing builder-resolution-input-invalid builder-source-package-binding-invalid builder-target-guard-action-invalid builder-target-guard-empty-clean-closure builder-target-platform-invalid builder-transitive-p-invalid cargo-executable-mismatch cargo-json-invalid cargo-metadata-d4-target-invalid cargo-metadata-d8-target-invalid cargo-metadata-d8-target-source-invalid cargo-metadata-dependency-invalid cargo-metadata-dependency-kind-invalid cargo-metadata-initializer-alias-invalid cargo-metadata-local-graph-invalid cargo-metadata-member-path-invalid cargo-metadata-member-set-invalid cargo-metadata-package-invalid cargo-metadata-path-invalid cargo-metadata-runner-required-features-invalid cargo-metadata-shape-invalid cargo-metadata-targets-invalid cargo-target-event-missing cargo-verbose-evidence-missing consumer-batch-gates-invalid duplicate-json-key feature-artifact-arguments-invalid feature-artifact-binary-hash-invalid feature-artifact-binding-invalid feature-artifact-schema-invalid feature-bundle-arguments-invalid feature-bundle-binding-invalid feature-bundle-bins-invalid feature-bundle-schema-invalid feature-consumer-artifact-mismatch feature-contract-mode-invalid feature-d4-bins-invalid feature-d4-runtime-binaries-invalid feature-d7-recipe-invalid feature-d8-recipe-invalid feature-global-builder-invalid feature-layout-arguments-invalid feature-list-invalid feature-official-hash-invalid feature-recipe-invalid feature-recipe-set-invalid feature-recipe-unknown feature-recipes-invalid feature-request-arguments-invalid feature-request-compile-env-invalid feature-request-h-binding-invalid feature-request-hash-invalid feature-request-platform-invalid feature-request-recipe-mismatch feature-request-schema-invalid feature-request-selection-mismatch feature-resolution-input-mismatch feature-source-commit-invalid layout-keys-invalid layout-not-object layout-selection-invalid native-apk-inventory-invalid native-apk-inventory-not-normalized native-apk-package-duplicate native-apk-package-schema-invalid native-apk-package-status-invalid native-apk-packages-invalid native-apk-provides-duplicate native-apk-provides-invalid native-cargo-version-invalid native-compiler-env-invalid native-host-binding-invalid native-host-triple-invalid native-identity-binding-invalid native-identity-permission-invalid native-identity-schema-invalid native-package-contract-invalid native-package-inventory-missing native-package-set-invalid native-platform-invalid native-rustc-output-invalid native-tool-identity-invalid non-finite non-finite-json-number noncanonical-json nonproducer-recipe-invalid package-external-inventory-invalid package-graph-cycle package-inventory-keys-invalid package-inventory-paths-invalid package-record-invalid package-record-keys-invalid production-bin-count-invalid recipe-bin-duplicates recipe-compile-contract-invalid recipe-d7-closed-contract-invalid recipe-d8-closed-contract-invalid recipe-keys-invalid recipe-package-input-contract-invalid recipe-runtime-payload-contract-invalid recipe-set-invalid requested-executable-not-elf64-x86-64 runtime-bin-contract-invalid runtime-payload-groups-invalid runtime-payload-simulation-contract-invalid service-d8-contract-invalid service-recipe-invalid service-record-invalid service-set-invalid source-inventory-invalid target-guard-binding-changed target-guard-not-pending target-guard-not-regular target-guard-package-graph-invalid target-guard-package-hash-invalid target-guard-package-set-invalid workspace-member-inventory-invalid
""".split())
CAPTURE_CHILD_FAILURE_RE = re.compile(r"RBL_CAPTURE_CHILD_FAILURE\|([a-z0-9-]+)\Z")


def cargo_failure_wire(value, recipe, bin_name):
    if type(recipe) is not str or type(bin_name) is not str or type(value) is not str or len(value)>512:
        return None
    parts=value.split("|")
    if len(parts)!=7 or parts[:2]!=[recipe,bin_name]:
        return None
    if bin_name not in {"D1":("api-server",),
                        "D7":("paper-runner","owner-beta-paper-ops-native"),
                        "D8":("owner-beta-simulation-runner",)}.get(recipe,()):
        return None
    status,code,path,line,column=parts[2:]
    if not re.fullmatch(r"[1-9][0-9]{0,2}",status) or int(status)>255:
        return None
    if [code,path,line,column]==["UNKNOWN","UNKNOWN","0","0"]:
        return value
    if (not re.fullmatch(r"E[0-9]{4}",code) or
            not re.fullmatch(r"(?:crates/(?:api-server|auth|data-go-client|domain|factor-engine|job-queue|kis-client|market-data|opendart-client|portfolio-model|r5-close-custody|result-model|risk-gateway|selector)|apps/api-server/auth|data-pipelines/collectors|tests/integration/migration-contract)/(?:[a-z0-9_/-]+/)?[a-z0-9_-]+\.rs",path) or
            "//" in path or len(path)>240 or
            not re.fullmatch(r"[1-9][0-9]{0,6}",line) or
            not re.fullmatch(r"[1-9][0-9]{0,5}",column)):
        return None
    return value

def capture_completed_status_matches(shell_status, child_status):
    return ((type(child_status) is int and child_status != 0 and
             child_status % 256 == shell_status) or
            (type(child_status) is str and child_status == "SPAWN_ERROR" and
             shell_status == 127))


IMAGE_VERIFICATION_CODES = frozenset("""
archive-changed-during-read archive-duplicate-entry archive-member-size-mismatch
archive-member-unreadable archive-special-entry binding-invalid
config-platform-invalid config-revision-invalid descriptor-binding-invalid
descriptor-blob-missing descriptor-digest-invalid descriptor-invalid
digest-blob-binding-invalid duplicate-directory-path duplicate-file-path
duplicate-json-key file-directory-path-conflict file-literal-invalid
file-requirement-schema-invalid file-requirement-type-invalid file-sha256-invalid
host-path-not-canonical image-id-invalid image-path-invalid
image-root-not-bound index-cycle-or-depth index-schema-invalid
input-not-regular layer-descriptor-invalid layer-duplicate-entry
layer-file-size-mismatch layer-file-unreadable layer-hardlink-body-invalid
layer-hardlink-self-target layer-hardlink-target-missing layer-hardlink-target-not-regular
layer-hardlink-target-parent-not-directory layer-parent-not-directory layer-root-entry-not-directory
layer-special-entry manifest-schema-invalid multiple-runnable-manifests
non-finite-json-number product-bundle-missing product-d8-receipt-binding-invalid
product-d8-receipt-file-invalid product-d8-receipt-missing product-d8-service-contract-invalid
product-directory-set-invalid product-file-hash-invalid product-file-mode-invalid
product-file-set-invalid product-image-mode-contract-drift product-image-path-invalid
product-receipt-binding-invalid product-request-binding-changed product-request-existing-raced
product-request-existing-unsafe product-request-parent-invalid product-result-binding-invalid
product-result-sha-invalid product-runtime-directory-payload-invalid product-runtime-entry-outside-payload
product-runtime-file-payload-invalid product-runtime-inventory-invalid product-runtime-payload-declaration-mismatch
product-runtime-payload-entries-invalid product-runtime-payload-group-invalid product-runtime-payload-invalid
product-runtime-payload-root-missing product-selected-path-conflict product-service-kind-invalid
product-service-unknown product-simulation-payload-identity-invalid product-simulation-source-mode-invalid
product-source-directory-invalid product-source-entry-missing product-source-escape
product-source-file-invalid product-source-kind-invalid product-source-mode-changed
product-source-path-invalid regular-file-parent-path-conflict request-schema-invalid
result-already-exists result-parent-invalid runnable-manifest-missing
selected-directory-empty selected-directory-not-real selected-file-hash-mismatch
selected-file-literal-missing selected-file-not-elf64-x86-64 selected-file-not-executable
selected-parent-missing selected-parent-not-directory selected-path-link
selected-path-missing selected-path-not-regular tar-path-invalid
whiteout-target-invalid
""".split())
HASH_RE = re.compile(r"[0-9a-f]{64}\Z")
INTERPOLATION_RE = re.compile(r"(?<!\$)\$\{([A-Za-z_][A-Za-z0-9_]*)")


class BuildFailure(RuntimeError):
    pass


class DeadlineExceeded(BuildFailure):
    pass


class CleanupFailure(BuildFailure):
    pass


@dataclass(frozen=True)
class ServicePlan:
    service: str
    recipe: str
    bins: tuple[str, ...]


class CleanupStack:
    def __init__(self) -> None:
        self.items: list[tuple[str, Callable[[], None]]] = []
        self.closed = False

    def push(self, name: str, cleanup: Callable[[], None]) -> None:
        if self.closed:
            raise BuildFailure("cleanup-stack-closed")
        self.items.append((name, cleanup))

    def close(self) -> None:
        if self.closed:
            return
        self.closed = True
        errors = []
        failures = []
        for name, cleanup in reversed(self.items):
            try:
                cleanup()
            except Exception as error:
                errors.append(name)
                failures.append((name, error))
        self.items.clear()
        if errors:
            failure = CleanupFailure("cleanup-failed:" + ",".join(errors))
            failure._cleanup_failures = tuple(failures)
            raise failure


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Build authentic V3 fixture images.")
    parser.add_argument("--source-root", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--state-root", required=True)
    parser.add_argument("--manifest-file", required=True)
    parser.add_argument("--deadline-monotonic-ns", required=True, type=int)
    return parser.parse_args(argv)


def canonical_abs(raw: str, label: str) -> pathlib.Path:
    if (not raw or not os.path.isabs(raw) or raw != os.path.normpath(raw)
            or raw.endswith("/") or "\n" in raw or "\r" in raw):
        raise BuildFailure(label + "-path-invalid")
    current = pathlib.Path("/")
    for component in pathlib.Path(raw).parts[1:]:
        current /= component
        try:
            if stat.S_ISLNK(os.lstat(current).st_mode):
                raise BuildFailure(label + "-path-symlink")
        except FileNotFoundError:
            pass
    return pathlib.Path(raw)


def validate_path_arguments(source_raw: str, commit: str, state_raw: str,
                            manifest_raw: str, deadline_ns: int):
    if not COMMIT_RE.fullmatch(commit) or commit == "0" * 40:
        raise BuildFailure("commit-invalid")
    source = canonical_abs(source_raw, "source-root")
    if str(pathlib.Path(os.path.realpath(source))) != source_raw or not source.is_dir():
        raise BuildFailure("source-root-not-canonical-directory")
    state = canonical_abs(state_raw, "state-root")
    if state.parent != pathlib.Path("/var/tmp") or not STATE_RE.fullmatch(state.name):
        raise BuildFailure("state-root-location-invalid")
    if os.path.commonpath((str(source), str(state))) in (str(source), str(state)):
        raise BuildFailure("state-root-overlaps-source")
    manifest = canonical_abs(manifest_raw, "manifest-file")
    if manifest.parent != state:
        raise BuildFailure("manifest-must-be-direct-state-child")
    if os.path.lexists(state) or os.path.lexists(manifest):
        raise BuildFailure("state-or-manifest-already-exists")
    if deadline_ns <= time.monotonic_ns() + BUILD_RESERVE_NS:
        raise DeadlineExceeded("build-budget-below-cleanup-reserve")
    return source, state, manifest


def assert_clean_source(source: pathlib.Path, commit: str) -> None:
    clean_env = {"PATH": "/usr/bin:/bin:/usr/local/bin", "HOME": "/nonexistent", "LC_ALL": "C"}
    head = subprocess.run(
        ["git", "-c", f"safe.directory={source}", "-C", str(source),
         "rev-parse", "--verify", "HEAD^{commit}"],
        check=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
        timeout=15, env=clean_env,
    ).stdout.strip()
    if head != commit:
        raise BuildFailure("source-head-mismatch")
    status = subprocess.run(
        ["git", "-c", f"safe.directory={source}", "-C", str(source),
         "status", "--porcelain=v1", "--untracked-files=all"],
        check=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
        timeout=15, env=clean_env,
    ).stdout
    if status:
        raise BuildFailure("source-worktree-not-clean")


def disabled_env_values(compose: str, commit: str, state: pathlib.Path, nonce: str):
    names = sorted(set(INTERPOLATION_RE.findall(compose)))
    if not names:
        raise BuildFailure("compose-interpolation-set-empty")
    path_values = {
        "LAGRANGE_DATA_DIR": state / "empty-data",
        "LAGRANGE_ARTIFACTS_DIR": state / "empty-artifacts",
        "LAGRANGE_RUNTIME_SECRET_DIR": state / "empty-runtime-secrets",
    }
    fixed = {
        "LAGRANGE_CODE_COMMIT": commit,
        "LAGRANGE_PGDATA_VOLUME": "paper-fixture-data-" + nonce,
        "POSTGRES_DB": "paper_fixture",
        "POSTGRES_USER": "paper_fixture",
        "API_INTERNAL_URL": "http://127.0.0.1:1/prebuild-disabled",
        "AUTH0_DOMAIN": "prebuild-disabled.invalid",
        "AUTH0_REDIRECT_URI": "http://127.0.0.1/disabled",
        "LIVE_NODE_ACCOUNT_ID": "",
        "LIVE_NODE_MODE": "disabled",
        "LIVE_NODE_DRY_RUN": "1",
        "LIVE_NODE_DATA_FRESH": "0",
        "RESEARCH_APP_ENV": "prebuild-disabled",
        "RESEARCH_ENTITLEMENT_REFERENCE": "prebuild-disabled",
        # Compose requires a nonempty value even for inactive profiles. This
        # deliberately invalid digest carries no entitlement or runtime proof.
        "RESEARCH_ENTITLEMENT_SHA256": "prebuild-disabled",
        "RESEARCH_CANDIDATE_ENABLED": "0",
        "RESEARCH_FETCH_MODE": "disabled",
        "RANGE_RAW_BATCH_ID": "compose-config-disabled",
        "OWNER_BETA_PAPER_MODE": "disabled",
        "OWNER_BETA_ACCESS_MODE": "disabled",
        "OWNER_BETA_EQUITY_SIGNALS_MODE": "disabled",
        "OWNER_BETA_PRICE_INPUT_MODE": "disabled",
        "RUST_LOG": "warn",
    }
    numeric = re.compile(
        r"(?:_MS|_SECS|_SECONDS|_POLL|_TIMEOUT|_INTERVAL|_GRACE|_LEASE|"
        r"_BACKOFF|_HEARTBEAT|_SWEEP|_CONNECTIONS|_BYTES|_MAX)$"
    )
    values = {}
    for name in names:
        if re.search(r"(?:KIS|DART|TOKEN|PASSWORD|APP_KEY)", name):
            raise BuildFailure("compose-references-prohibited-credential-input")
        if "SECRET" in name and name != "LAGRANGE_RUNTIME_SECRET_DIR":
            raise BuildFailure("compose-references-prohibited-secret-input")
        if "ACCOUNT" in name and name != "LIVE_NODE_ACCOUNT_ID":
            raise BuildFailure("compose-references-prohibited-account-input")
        if name in path_values:
            values[name] = str(path_values[name])
        elif name in fixed:
            values[name] = fixed[name]
        elif name.endswith(("_SHA256", "_HASH")):
            values[name] = ""
        elif name.endswith("_MODE"):
            values[name] = "disabled"
        elif name in {"LIVE_NODE_KILL_SWITCH", "LIVE_NODE_GATE_FILE",
                      "LIVE_NODE_RISK_GATE", "LIVE_NODE_RECONCILIATION_GATE"}:
            values[name] = str(state / "disabled-nonexistent-gate")
        elif numeric.search(name) or "_MAX_" in name or name == "MAX_JOBS_PER_OWNER":
            values[name] = "0"
        else:
            values[name] = "prebuild-disabled"
    if values.get("LIVE_NODE_ACCOUNT_ID") != "":
        raise BuildFailure("live-account-interpolation-not-disabled")
    return values


def create_private_state(source: pathlib.Path, state: pathlib.Path,
                         manifest: pathlib.Path, commit: str):
    os.umask(0o077)
    try:
        os.mkdir(state, 0o700)
    except FileExistsError as exc:
        raise BuildFailure("state-root-already-exists") from exc
    info = os.lstat(state)
    if (not stat.S_ISDIR(info.st_mode) or stat.S_ISLNK(info.st_mode)
            or info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o700):
        raise BuildFailure("state-root-identity-invalid")
    nonce = state.name.removeprefix("lagrange-paper-fixture-build-")
    for name in ("home", "docker-config", "tmp", "empty-data", "empty-artifacts",
                 "empty-runtime-secrets", "logs"):
        os.mkdir(state / name, 0o700)
    compose_file = source / "deploy/compose/compose.yml"
    info = os.lstat(compose_file)
    if not stat.S_ISREG(info.st_mode) or stat.S_ISLNK(info.st_mode):
        raise BuildFailure("compose-file-not-regular")
    values = disabled_env_values(compose_file.read_text(encoding="utf-8"), commit, state, nonce)
    env_file = state / ".env"
    data = "".join(f"{key}={values[key]}\n" for key in sorted(values)).encode("ascii")
    fd = os.open(env_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as handle:
        handle.write(data)
        handle.flush()
        os.fsync(handle.fileno())
    if stat.S_IMODE(os.lstat(env_file).st_mode) != 0o600:
        raise BuildFailure("private-env-mode-invalid")
    if manifest.parent != state or os.path.lexists(manifest):
        raise BuildFailure("manifest-path-invalid-or-present")
    return env_file, nonce


def parse_proc_stat(path: pathlib.Path):
    try:
        fields = path.read_text(encoding="ascii").rsplit(")", 1)[1].split()
        return fields[0], int(fields[1]), int(fields[2])
    except FileNotFoundError:
        return None
    except PermissionError as exc:
        raise BuildFailure("process-inventory-unobservable") from exc
    except (OSError, ValueError, IndexError) as exc:
        raise BuildFailure("process-inventory-invalid") from exc


def group_members(pgid: int):
    result = []
    for item in pathlib.Path("/proc").iterdir():
        if item.name.isdigit():
            parsed = parse_proc_stat(item / "stat")
            if parsed is not None and parsed[2] == pgid:
                result.append(int(item.name))
    return sorted(result)


def compiler_pids():
    result = []
    for item in pathlib.Path("/proc").iterdir():
        if not item.name.isdigit():
            continue
        try:
            name = (item / "comm").read_text(encoding="ascii").strip()
        except FileNotFoundError:
            continue
        except PermissionError as exc:
            raise BuildFailure("compiler-inventory-unobservable") from exc
        except OSError as exc:
            raise BuildFailure("compiler-inventory-unobservable") from exc
        if name in {"cargo", "rustc", "rustdoc"}:
            result.append((int(item.name), name))
    return sorted(result)


def oom_kill_count():
    try:
        lines = pathlib.Path("/proc/vmstat").read_text(encoding="ascii").splitlines()
    except OSError as exc:
        raise BuildFailure("kernel-oom-counter-unavailable") from exc
    for line in lines:
        parts = line.split()
        if len(parts) == 2 and parts[0] == "oom_kill" and parts[1].isdigit():
            return int(parts[1])
    raise BuildFailure("kernel-oom-counter-unavailable")


class ResourceMonitor:
    def __init__(self, source: pathlib.Path, final_deadline_ns: int):
        path = source / "scripts/ops/lib/build-resource-policy.py"
        if path.is_symlink() or not path.is_file():
            raise BuildFailure("resource-policy-missing-or-symlinked")
        self.policy = types.ModuleType("paper_fixture_resource_policy")
        code = compile(path.read_text(encoding="utf-8"), str(path), "exec", dont_inherit=True)
        exec(code, self.policy.__dict__)
        if not callable(getattr(self.policy, "read_observation", None)) or not callable(
            getattr(self.policy, "failure_reason", None)
        ):
            raise BuildFailure("resource-policy-api-invalid")
        self.final_deadline_ns = final_deadline_ns
        self.build_deadline_ns = final_deadline_ns - BUILD_RESERVE_NS
        self.last_live_check_ns = 0
        self.oom_baseline = oom_kill_count()
        self.checks: list[dict[str, object]] = []

    def check(self, label: str, server_pid: int | None = None,
              require_idle_group: bool = True, *, in_cleanup: bool = False):
        now = time.monotonic_ns()
        cutoff = self.final_deadline_ns if in_cleanup else self.build_deadline_ns
        if now >= cutoff:
            raise DeadlineExceeded("build-deadline-reserve-reached")
        try:
            observation = self.policy.read_observation()
            reason = self.policy.failure_reason(observation)
        except Exception as exc:
            raise BuildFailure("resource-observation-unavailable-or-invalid") from exc
        if reason is not None:
            raise BuildFailure("resource-policy-stop:" + reason)
        oom = oom_kill_count()
        if oom != self.oom_baseline:
            raise BuildFailure("kernel-oom-counter-changed")
        compilers = compiler_pids()
        if require_idle_group and compilers:
            raise BuildFailure("compiler-process-present-at-boundary")
        if server_pid is not None and require_idle_group and group_members(server_pid) != [server_pid]:
            raise BuildFailure("owned-helper-has-unreaped-children")
        self.checks.append({
            "boundary": label, "monotonic_ns": now,
            "MemAvailable": observation["MemAvailable"], "SwapFree": observation["SwapFree"],
            "memory_psi_some_avg10": observation["memory_psi_some_avg10"],
            "memory_psi_full_avg10": observation["memory_psi_full_avg10"],
            "oom_kill": oom,
            "compiler_processes": [{"pid": pid, "name": name} for pid, name in compilers],
            "compiler_absence_required": require_idle_group,
            "helper_children_absence_required": require_idle_group and server_pid is not None,
        })


def stop_process_group(proc: subprocess.Popen, grace: float = 5.0):
    pgid = proc.pid
    if proc.poll() is None:
        try:
            os.killpg(pgid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    try:
        proc.wait(timeout=grace)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(pgid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        proc.wait(timeout=grace)
    end = time.monotonic() + grace
    while group_members(pgid) and time.monotonic() < end:
        try:
            os.killpg(pgid, signal.SIGKILL)
        except ProcessLookupError:
            break
        time.sleep(0.02)
    if group_members(pgid):
        raise CleanupFailure("owned-process-group-remains")


def run_owned(argv: list[str], label: str, env: dict[str, str],
              state: pathlib.Path, deadline_ns: int, output=False):
    if not argv or any("\x00" in value for value in argv):
        raise BuildFailure("command-argv-invalid")
    log = state / "logs" / (label + ".log")
    fd = os.open(log, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        proc = subprocess.Popen(
            argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE if output else fd,
            stderr=fd, env=env, cwd=state, start_new_session=True,
        )
    finally:
        os.close(fd)
    if output:
        try:
            data, _ = proc.communicate(timeout=max(0.01, (deadline_ns - time.monotonic_ns()) / 1e9))
        except subprocess.TimeoutExpired as exc:
            stop_process_group(proc)
            raise DeadlineExceeded("command-deadline:" + label) from exc
        if time.monotonic_ns() >= deadline_ns:
            raise DeadlineExceeded("command-finished-after-deadline:" + label)
        if group_members(proc.pid):
            stop_process_group(proc)
            raise BuildFailure("command-left-owned-processes:" + label)
        if proc.returncode:
            raise BuildFailure(f"command-failed:{label}:{proc.returncode}")
        try:
            return data.decode("utf-8", "strict").strip()
        except UnicodeError as exc:
            raise BuildFailure("command-output-invalid:" + label) from exc
    while proc.poll() is None:
        if time.monotonic_ns() >= deadline_ns:
            stop_process_group(proc)
            raise DeadlineExceeded("command-deadline:" + label)
        time.sleep(0.1)
    code = proc.wait()
    if time.monotonic_ns() >= deadline_ns:
        raise DeadlineExceeded("command-finished-after-deadline:" + label)
    if group_members(proc.pid):
        stop_process_group(proc)
        raise BuildFailure("command-left-owned-processes:" + label)
    if code:
        raise BuildFailure(f"command-failed:{label}:{code}")
    return log.read_text(encoding="utf-8", errors="replace").strip()


BASH_SERVER = r"""#!/usr/bin/env bash
set -Eeuo pipefail
umask 077
source "$PAPER_SOURCE_ROOT/scripts/ops/lib/release-image-manifest.sh"
source "$PAPER_SOURCE_ROOT/scripts/ops/lib/release-build-layout.sh"
release_build_layout_init "$PAPER_SOURCE_ROOT" "$PAPER_COMMIT" "$PAPER_STATE_ROOT" "$PAPER_NAMESPACE"
printf 'READY\n'
decode_arg() { if [ "$1" = - ]; then printf ''; else printf '%s' "$1" | base64 -d; fi; }
reply_ok() { local encoded; encoded=$(printf '%s' "$1" | base64 -w0); printf 'OK %s\n' "$encoded"; }
reply_error() { local encoded; encoded=$(printf '%s' "$1" | base64 -w0); printf 'ERR %s\n' "$encoded"; }
while IFS= read -r request; do
  IFS=' ' read -r op a1 a2 a3 a4 <<<"$request"
  a1=$(decode_arg "$a1")
  a2=$(decode_arg "$a2")
  a3=$(decode_arg "$a3")
  a4=$(decode_arg "$a4")
  result=
  case "$op" in
    PLAN)
      layout=$PAPER_SOURCE_ROOT/deploy/build/release-build-layout.json
      while IFS= read -r service; do
        [ -n "$service" ] || continue
        recipe=$(rbl_recipe_for_service "$service" "$layout") || {
          reply_error plan-recipe-invalid
          continue 2
        }
        bins='[]'
        if [ "$recipe" != NONE ]; then
          bins=$(rbl_recipe_json "$recipe" "$layout" |
            python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin)["bins"],separators=(",",":")))') || {
              reply_error plan-bins-invalid
              continue 2
            }
        fi
        printf '%s\t%s\t%s\n' "$service" "$recipe" "$bins"
      done < <(printf '%s\n' "${RELEASE_IMAGE_SERVICES_V3[@]}") >"$PAPER_STATE_ROOT/logs/plan.tsv"
      result=$(cat "$PAPER_STATE_ROOT/logs/plan.tsv")
      reply_ok "$result"
      ;;
    MAKE_CONTEXT)
      if result=$(rbl_make_context "$a1" "$PAPER_STATE_ROOT"); then reply_ok "$result"; else reply_error helper-failed; fi
      ;;
    PRODUCE_NATIVE)
      if rbl_produce_native "$a1" "$PAPER_STATE_ROOT" >/dev/null; then reply_ok ok; else reply_error helper-failed; fi
      ;;
    PRODUCE_BIN)
      if result=$(rbl_produce_bin "$a1" "$a2" "$a3" "$PAPER_STATE_ROOT"); then reply_ok "$result"; else reply_error helper-failed; fi
      ;;
    BUNDLE_CREATE)
      if result=$(rbl_bundle_create "$a1" "$PAPER_STATE_ROOT"); then reply_ok "$result"; else reply_error helper-failed; fi
      ;;
    BUNDLE_VERIFY)
      if rbl_bundle_verify "$a1" "$a2" >/dev/null; then reply_ok ok; else reply_error helper-failed; fi
      ;;
    WRITE_OVERRIDE)
      if release_build_layout_write_override "$a1" "$a2" "$a3"; then reply_ok ok; else reply_error helper-failed; fi
      ;;
    PREPARE_NONE)
      if result=$(release_build_layout_prepare "$a1" "$PAPER_COMMIT" "$PAPER_STATE_ROOT") &&
          [ "$result" = NONE ] &&
          [ "$a1" = db-role-bootstrap -o "$a1" = db-migrate -o "$a1" = web ]; then
        reply_ok NONE
      else
        reply_error none-service-invalid
      fi
      ;;
    COMPOSE_BUILD)
      service=$a1
      override=$a2
      log=$PAPER_STATE_ROOT/logs/compose-$service.log
      [ ! -e "$log" ] && [ ! -L "$log" ] || {
        reply_error compose-log-exists
        continue
      }
      if [ "$override" = NONE ]; then
        if COMPOSE_PARALLEL_LIMIT=1 BUILDX_BUILDER=$PAPER_BUILDER COMPOSE_PROFILES= \
            LAGRANGE_CODE_COMMIT=$PAPER_COMMIT RESEARCH_APP_ENV=prebuild-disabled \
            RESEARCH_ENTITLEMENT_REFERENCE=prebuild-disabled BACKTEST_MIN_FREE_BYTES=0 \
            BACKTEST_MAX_QUEUED_BACKTESTS=0 BACKTEST_RECONCILE_GRACE_SECS=0 \
            BACKTEST_RECONCILE_INTERVAL_SECS=0 RANGE_RAW_BATCH_ID=compose-config-disabled \
            LIVE_NODE_MODE=disabled LIVE_NODE_DRY_RUN=1 CARGO_BUILD_JOBS=2 \
            docker compose --env-file "$PAPER_ENV_FILE" --file "$PAPER_COMPOSE_FILE" \
            build --pull=false "$service" >"$log" 2>&1; then
          chmod 0600 "$log"; reply_ok ok
        else
          chmod 0600 "$log" 2>/dev/null || true; reply_error compose-build-failed
        fi
      else
        if COMPOSE_PARALLEL_LIMIT=1 BUILDX_BUILDER=$PAPER_BUILDER COMPOSE_PROFILES= \
            LAGRANGE_CODE_COMMIT=$PAPER_COMMIT RESEARCH_APP_ENV=prebuild-disabled \
            RESEARCH_ENTITLEMENT_REFERENCE=prebuild-disabled BACKTEST_MIN_FREE_BYTES=0 \
            BACKTEST_MAX_QUEUED_BACKTESTS=0 BACKTEST_RECONCILE_GRACE_SECS=0 \
            BACKTEST_RECONCILE_INTERVAL_SECS=0 RANGE_RAW_BATCH_ID=compose-config-disabled \
            LIVE_NODE_MODE=disabled LIVE_NODE_DRY_RUN=1 CARGO_BUILD_JOBS=2 \
            docker compose --env-file "$PAPER_ENV_FILE" --file "$PAPER_COMPOSE_FILE" \
            --file "$override" build --pull=false "$service" >"$log" 2>&1; then
          chmod 0600 "$log"; reply_ok ok
        else
          chmod 0600 "$log" 2>/dev/null || true; reply_error compose-build-failed
        fi
      fi
      ;;
    VERIFY_IMAGE)
      service=$a1
      image_ref=$(release_image_manifest_ref_for "$service" "$PAPER_COMMIT") || {
        reply_error image-reference-invalid
        continue
      }
      inspected=$(docker image inspect \
        --format '{{.Id}}|{{index .Config.Labels "org.opencontainers.image.revision"}}' \
        "$image_ref") || {
          reply_error image-inspect-failed
          continue
        }
      case "$inspected" in *'|'*) ;; *) reply_error image-inspect-shape-invalid; continue ;; esac
      image_id=${inspected%%|*}
      revision=${inspected#*|}
      release_image_manifest_is_image_id "$image_id" &&
        release_image_manifest_is_commit "$revision" &&
        [ "$revision" = "$PAPER_COMMIT" ] || {
          reply_error image-identity-invalid
          continue
        }
      release_build_layout_verify_image "$service" "$PAPER_COMMIT" "$image_id" "$PAPER_STATE_ROOT" || {
        reply_error saved-image-verification-failed
        continue
      }
      RELEASE_IMAGE_MANIFEST_REFS["$service"]=$image_ref
      RELEASE_IMAGE_MANIFEST_IDS["$service"]=$image_id
      RELEASE_IMAGE_MANIFEST_REVISIONS["$service"]=$revision
      reply_ok "$image_ref|$image_id|$revision"
      ;;
    PUBLISH_TEMP)
      manifest_temp=$a1
      for service in $PAPER_SERVICES; do
        expected_ref=${RELEASE_IMAGE_MANIFEST_REFS[$service]:-}
        expected_id=${RELEASE_IMAGE_MANIFEST_IDS[$service]:-}
        expected_revision=${RELEASE_IMAGE_MANIFEST_REVISIONS[$service]:-}
        [ -n "$expected_ref" ] && [ -n "$expected_id" ] && [ -n "$expected_revision" ] || {
          reply_error final-image-map-incomplete
          continue 2
        }
        inspected=$(docker image inspect \
          --format '{{.Id}}|{{index .Config.Labels "org.opencontainers.image.revision"}}' \
          "$expected_ref") || {
            reply_error final-image-reinspect-failed
            continue 2
          }
        case "$inspected" in *'|'*) ;; *) reply_error final-image-reinspect-shape-invalid; continue 2 ;; esac
        [ "${inspected%%|*}" = "$expected_id" ] &&
          [ "${inspected#*|}" = "$expected_revision" ] &&
          [ "$expected_revision" = "$PAPER_COMMIT" ] || {
            reply_error final-image-identity-drift
            continue 2
          }
      done
      [ ! -e "$manifest_temp" ] && [ ! -L "$manifest_temp" ] || {
        reply_error manifest-temp-already-exists
        continue
      }
      release_image_manifest_write "$manifest_temp" "$PAPER_COMMIT" || {
        reply_error manifest-write-failed
        continue
      }
      chmod 0600 "$manifest_temp"
      release_image_manifest_load "$manifest_temp" "$PAPER_COMMIT" || {
        reply_error manifest-validation-failed
        continue
      }
      [ "$RELEASE_IMAGE_MANIFEST_LOADED_FORMAT" = "$RELEASE_IMAGE_MANIFEST_V3_FORMAT" ] || {
        reply_error manifest-not-v3
        continue
      }
      reply_ok "$manifest_temp"
      ;;
    QUIT)
      reply_ok bye
      break
      ;;
    *)
      reply_error action-invalid
      ;;
  esac
done
"""


def b64_field(value: str) -> str:
    return "-" if value == "" else base64.b64encode(value.encode()).decode("ascii")


class AttemptProgress:
    """Best-effort, bounded observations; never participates in build decisions."""

    def __init__(self, cutoff_ns: int, commit: str):
        self.cutoff_ns = cutoff_ns
        self.commit = commit
        self.call_index = 0
        self.current = None

    def _emit(self, record: dict[str, object]) -> None:
        try:
            encoded = json.dumps(record, sort_keys=True, separators=(",", ":"))
            sys.stdout.write(ATTEMPT_PROGRESS_PREFIX + encoded + "\n")
            sys.stdout.flush()
        except Exception:
            pass

    @staticmethod
    def _now() -> int | str:
        try:
            value = time.monotonic_ns()
            return value if type(value) is int and value >= 0 else "UNKNOWN"
        except Exception:
            return "UNKNOWN"

    def start(self, service, recipe, operation, bin_name=None, bin_index=0,
              allowed_bins=()) -> object | None:
        try:
            if (self.call_index >= 256 or type(service) is not str or service not in SERVICES
                    or type(recipe) is not str
                    or not (recipe == "NONE" or re.fullmatch(r"D[1-8]", recipe))
                    or type(operation) is not str or operation not in ATTEMPT_PROGRESS_OPS):
                return None
            if (service in NONE_SERVICES) != (recipe == "NONE"):
                return None
            if operation == "PRODUCE_BIN":
                if (type(bin_name) is not str or bin_name not in allowed_bins
                        or type(bin_index) is not int or not 1 <= bin_index <= len(allowed_bins)
                        or allowed_bins[bin_index - 1] != bin_name):
                    return None
                observed_bin, observed_bin_index = bin_name, bin_index
            else:
                observed_bin, observed_bin_index = "UNKNOWN", "UNKNOWN"
            self.call_index += 1
            current = {
                "call_index": self.call_index, "service": service, "recipe": recipe,
                "bin": observed_bin, "bin_index": observed_bin_index,
                "op": operation, "start_monotonic_ns": self._now(),
            }
            self.current = current
            self._emit({
                "format": ATTEMPT_PROGRESS_FORMAT, "event": "STARTED",
                **current, "end_monotonic_ns": "UNKNOWN",
                "build_cutoff_monotonic_ns": self.cutoff_ns,
                "capture_phase": "UNKNOWN", "capture_child_status": "UNKNOWN",
            })
            return current
        except Exception:
            return None

    def returned(self, current) -> None:
        try:
            if current is not self.current:
                return
            self.current = None
            self._emit({
                "format": ATTEMPT_PROGRESS_FORMAT, "event": "RETURNED", **current,
                "end_monotonic_ns": self._now(),
                "build_cutoff_monotonic_ns": self.cutoff_ns,
                "capture_phase": "UNKNOWN", "capture_child_status": "UNKNOWN",
            })
        except Exception:
            self.current = None

    def failed(self, current) -> None:
        try:
            if current is self.current:
                self.current = None
        except Exception:
            pass

    def response_pending(self, operation, capture) -> None:
        try:
            current = self.current
            if (type(current) is not dict or type(operation) is not str
                    or operation not in HELPER_OPS or current.get("op") != operation):
                return
            phase, child_status = capture
            if (type(phase) is not str or phase not in CAPTURE_ATTEMPT_PHASE_SET
                    or not (type(child_status) is int and -255 <= child_status <= 255
                            or type(child_status) is str and child_status in {
                                "UNKNOWN", "SPAWN_ERROR"})):
                phase, child_status = "UNKNOWN", "UNKNOWN"
            self._emit({
                "format": ATTEMPT_PROGRESS_FORMAT, "event": "RESPONSE_PENDING",
                **current, "end_monotonic_ns": self._now(),
                "build_cutoff_monotonic_ns": self.cutoff_ns,
                "capture_phase": phase, "capture_child_status": child_status,
                "original_evidence": "UNKNOWN",
            })
        except Exception:
            pass

    def response_returned(self, operation) -> None:
        try:
            current = self.current
            if (type(current) is dict and type(operation) is str
                    and current.get("op") == operation):
                self.returned(current)
        except Exception:
            pass

    def originals_verified(self, rows) -> None:
        try:
            expected = {(recipe, bin_name) for recipe, bins in RECEIPT_BINS.items()
                        for bin_name in bins}
            if type(rows) is not list or len(rows) != len(expected):
                return
            evidence = []
            seen = set()
            for row in rows:
                if type(row) is not dict:
                    return
                recipe, bin_name = row.get("recipe"), row.get("bin")
                pair = (recipe, bin_name)
                if (type(recipe) is not str or type(bin_name) is not str or pair not in expected
                        or pair in seen or type(row.get("source_commit")) is not str
                        or row.get("source_commit") != self.commit):
                    return
                seen.add(pair)
                hashes = (
                    row.get("artifact_json_sha256"), row.get("complete_sha256"),
                    row.get("receipt_pair_sha256"), row.get("compile_env_sha256"),
                    row.get("elf_sha256"),
                )
                size = row.get("elf_bytes")
                if (any(type(value) is not str or not HASH_RE.fullmatch(value)
                        for value in hashes) or type(size) is not int
                        or not 0 <= size <= 0x7FFFFFFFFFFFFFFF):
                    return
                evidence.append({
                    "recipe": recipe, "bin": bin_name,
                    "artifact_json_sha256": hashes[0], "complete_sha256": hashes[1],
                    "receipt_pair_sha256": hashes[2], "compile_env_sha256": hashes[3],
                    "elf_sha256": hashes[4], "elf_bytes": size,
                })
            if seen != expected:
                return
            evidence.sort(key=lambda value: (value["recipe"], value["bin"]))
            self._emit({
                "format": ATTEMPT_PROGRESS_FORMAT, "event": "ORIGINALS_VERIFIED",
                "build_cutoff_monotonic_ns": self.cutoff_ns, "evidence": evidence,
            })
        except Exception:
            pass


class HelperServer:
    def __init__(self, source: pathlib.Path, commit: str, state: pathlib.Path,
                 env_file: pathlib.Path, namespace: str, builder: str,
                 env: dict[str, str], deadline_ns: int, cleanup_deadline_ns: int,
                 progress=None):
        self.deadline_ns = deadline_ns
        self.cleanup_deadline_ns = cleanup_deadline_ns
        self.progress = progress
        self.source, self.state, self.commit = source, state, commit
        self._request_pending = False
        errfd = os.open(state / "logs/helper-server.stderr.log",
                        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        self._stderr_path = state / "logs/helper-server.stderr.log"
        error_info = os.fstat(errfd)
        self._stderr_identity = (error_info.st_dev, error_info.st_ino)
        child_env = dict(env)
        child_env.update({
            "PAPER_SOURCE_ROOT": str(source), "PAPER_COMMIT": commit,
            "PAPER_STATE_ROOT": str(state), "PAPER_ENV_FILE": str(env_file),
            "PAPER_COMPOSE_FILE": str(source / "deploy/compose/compose.yml"),
            "PAPER_NAMESPACE": namespace, "PAPER_BUILDER": builder,
            "PAPER_SERVICES": " ".join(SERVICES),
        })
        try:
            self.proc = subprocess.Popen(
                ["bash", "--noprofile", "--norc", "-c", BASH_SERVER],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errfd,
                env=child_env, cwd=state, start_new_session=True,
            )
        finally:
            os.close(errfd)
        self.buffer = bytearray()
        try:
            if self.readline(deadline_ns, None, "startup") != b"READY":
                raise BuildFailure("helper-server-startup-failed")
        except Exception:
            stop_process_group(self.proc, 3.0)
            raise

    def readline(self, cutoff_ns: int, monitor, label: str) -> bytes:
        assert self.proc.stdout is not None
        fd = self.proc.stdout.fileno()
        while True:
            index = self.buffer.find(b"\n")
            if index >= 0:
                result = bytes(self.buffer[:index])
                del self.buffer[:index + 1]
                return result
            if self.proc.poll() is not None:
                raise BuildFailure("helper-server-exited")
            now = time.monotonic_ns()
            if now >= cutoff_ns:
                raise DeadlineExceeded("helper-deadline:" + label)
            ready, _, _ = select.select([fd], [], [], min(0.5, (cutoff_ns - now) / 1e9))
            if not ready:
                if monitor is not None and now - monitor.last_live_check_ns >= 5_000_000_000:
                    monitor.check("during:" + label, self.proc.pid, require_idle_group=False)
                    monitor.last_live_check_ns = now
                continue
            chunk = os.read(fd, 65536)
            if not chunk:
                raise BuildFailure("helper-server-closed-output")
            self.buffer.extend(chunk)

    def capture_attempt_prefix(self, offset, pending_expectation=None):
        """Best-effort current-request capture phases; distinct from terminal receipt readers."""
        unknown = ("UNKNOWN", "UNKNOWN")
        self._pending_capture_marker = None
        fd = -1
        try:
            if type(offset) is not int or offset < 0:
                return unknown
            fd = os.open(self._stderr_path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            before = os.fstat(fd)
            before_identity = (before.st_dev, before.st_ino)
            named_before = os.stat(self._stderr_path, follow_symlinks=False)
            if (not stat.S_ISREG(before.st_mode) or before.st_uid != os.geteuid()
                    or stat.S_IMODE(before.st_mode) != 0o600 or before.st_nlink != 1
                    or before_identity != self._stderr_identity
                    or before_identity != (named_before.st_dev, named_before.st_ino)):
                return unknown
            size = before.st_size - offset
            if not 0 < size <= 65536:
                return unknown
            data = os.pread(fd, size, offset)
            after = os.fstat(fd)
            named_after = os.stat(self._stderr_path, follow_symlinks=False)
            identity_fields = lambda value: (
                value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns,
                value.st_ctime_ns, value.st_uid, stat.S_IMODE(value.st_mode), value.st_nlink,
            )
            if (identity_fields(named_before) != identity_fields(before) or
                    len(data) != size or not data.endswith(b"\n") or b"\r" in data
                    or identity_fields(after) != identity_fields(before)
                    or identity_fields(named_after) != identity_fields(before)):
                return unknown
            lines = data[:-1].split(b"\n")
            if not lines or any(not line for line in lines):
                return unknown
            records = []
            capture_positions = []
            pending_rows = []
            for line_index, raw_line in enumerate(lines):
                if raw_line.startswith(b"RBL_CAPTURE_PENDING"):
                    if (not raw_line.startswith(b"RBL_CAPTURE_PENDING|") or len(raw_line) > 8192):
                        return unknown
                    pending_rows.append((line_index, raw_line))
                    continue
                line = raw_line.decode("ascii")
                match = CAPTURE_ATTEMPT_RE.fullmatch(line)
                if match is None:
                    return unknown
                phase, raw_status = match.groups()
                if phase not in CAPTURE_ATTEMPT_PHASE_SET or raw_status == "-0":
                    return unknown
                if raw_status in {"UNKNOWN", "SPAWN_ERROR"}:
                    child_status = raw_status
                else:
                    child_status = int(raw_status)
                    if not -255 <= child_status <= 255:
                        return unknown
                records.append((phase, child_status))
                capture_positions.append(line_index)
            previous_phase = None
            observed_child_status = "UNKNOWN"
            for phase, child_status in records:
                if phase not in ATTEMPT_CAPTURE_NEXT.get(previous_phase, set()):
                    return unknown
                previous_phase = phase
                if phase in CAPTURE_ATTEMPT_PHASES[:6]:
                    if child_status != "UNKNOWN":
                        return unknown
                elif phase == "CHILD_RETURNED":
                    if type(child_status) is not int:
                        return unknown
                    observed_child_status = child_status
                elif phase == "CHILD_SPAWN_ERROR":
                    if child_status != "SPAWN_ERROR":
                        return unknown
                    observed_child_status = "SPAWN_ERROR"
                elif child_status != observed_child_status or observed_child_status == "UNKNOWN":
                    return unknown
            if pending_rows:
                if len(pending_rows) != 1 or pending_expectation is None:
                    return unknown
                pending_index, pending_line = pending_rows[0]
                try:
                    active_index = capture_positions[
                        next(i for i, (phase, _status) in enumerate(records)
                             if phase == "ACTIVE_LOG_OPEN")
                    ]
                    child_index = capture_positions[
                        next(i for i, (phase, _status) in enumerate(records)
                             if phase == "CHILD_PENDING")
                    ]
                    value = observation_json(
                        pending_line[len(b"RBL_CAPTURE_PENDING|"):] + b"\n"
                    )
                    identity = value.get("log_identity")
                    if (not active_index < pending_index < child_index or
                            set(value) != {"format", "request_sha256", "recipe", "bin", "attempt_label", "log_identity"} or
                            value.get("format") != "lagrange-paper-fixture-pending-open-v1" or
                            value.get("request_sha256") != pending_expectation["request_sha256"] or
                            value.get("recipe") != pending_expectation["recipe"] or
                            value.get("bin") != pending_expectation["bin"] or
                            value.get("attempt_label") != "producer-" + pending_expectation["recipe"] + "-" + pending_expectation["bin"] or
                            set(identity) != {"dev", "ino", "uid", "mode", "nlink"} or
                            any(type(identity[key]) is not int or identity[key] < 0
                                for key in ("dev", "ino", "uid", "mode", "nlink")) or
                            identity["uid"] != os.geteuid() or identity["mode"] != 0o600 or
                            identity["nlink"] != 1 or identity["ino"] <= 0):
                        return unknown
                    self._pending_capture_marker = value
                except Exception:
                    return unknown
            return records[-1][0], observed_child_status
        except Exception:
            return unknown
        finally:
            if fd >= 0:
                try:
                    os.close(fd)
                except Exception:
                    pass

    def verification_stderr(self, offset=None):
        """Read only this call's bounded, original private stderr; emit fixed codes."""
        try:
            fd = os.open(self._stderr_path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            try:
                before = os.fstat(fd)
                identity = (before.st_dev, before.st_ino)
                named = os.stat(self._stderr_path, follow_symlinks=False)
                if (not stat.S_ISREG(before.st_mode) or before.st_nlink != 1 or
                        stat.S_IMODE(before.st_mode) != 0o600 or before.st_uid != os.geteuid() or
                        identity != self._stderr_identity or
                        identity != (named.st_dev, named.st_ino)):
                    return None, "UNKNOWN"
                if offset is None:
                    return before.st_size, "UNKNOWN"
                size = before.st_size - offset
                if type(offset) is not int or offset < 0 or not 0 < size <= 65536:
                    return None, "UNKNOWN"
                data = os.pread(fd, size, offset)
                after = os.fstat(fd)
                named = os.stat(self._stderr_path, follow_symlinks=False)
                if (len(data) != size or not data.endswith(b"\n") or
                        (after.st_size, after.st_mtime_ns, after.st_ctime_ns) !=
                        (before.st_size, before.st_mtime_ns, before.st_ctime_ns) or
                        identity != (named.st_dev, named.st_ino)):
                    return None, "UNKNOWN"
                value = data.splitlines()[-1].decode("ascii")
                return None, value if value in IMAGE_VERIFICATION_CODES else "UNKNOWN"
            finally:
                os.close(fd)
        except (OSError, AttributeError, TypeError, ValueError, UnicodeError):
            return None, "UNKNOWN"

    def producer_phase_stderr(self, offset=None, *, include_attempt=False, cargo_witness=None, target=()):
        """Read only the current PRODUCE_BIN phase marker from the original private stderr."""
        unknown = ("UNKNOWN", "UNKNOWN")
        unknown_result = (None, "UNKNOWN", "UNKNOWN", "UNKNOWN", "UNKNOWN") if include_attempt else (None, unknown)
        try:
            fd = os.open(self._stderr_path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            try:
                before = os.fstat(fd)
                identity = (before.st_dev, before.st_ino)
                named = os.stat(self._stderr_path, follow_symlinks=False)
                if (not stat.S_ISREG(before.st_mode) or before.st_nlink != 1 or
                        stat.S_IMODE(before.st_mode) != 0o600 or before.st_uid != os.geteuid() or
                        identity != self._stderr_identity or
                        identity != (named.st_dev, named.st_ino)):
                    return unknown_result
                if offset is None:
                    after = os.fstat(fd)
                    named = os.stat(self._stderr_path, follow_symlinks=False)
                    if ((after.st_dev, after.st_ino) != identity or
                            (after.st_size, after.st_mtime_ns, after.st_ctime_ns) !=
                            (before.st_size, before.st_mtime_ns, before.st_ctime_ns) or
                            after.st_nlink != 1 or stat.S_IMODE(after.st_mode) != 0o600 or
                            after.st_uid != os.geteuid() or
                            identity != (named.st_dev, named.st_ino)):
                        return unknown_result
                    return (before.st_size, "UNKNOWN", "UNKNOWN", "UNKNOWN", "UNKNOWN") if include_attempt else (before.st_size, unknown)
                if type(offset) is not int or offset < 0 or offset >= before.st_size:
                    return unknown_result
                size = before.st_size - offset
                if not 0 < size <= 65536:
                    return unknown_result
                data = os.pread(fd, size, offset)
                after = os.fstat(fd)
                named = os.stat(self._stderr_path, follow_symlinks=False)
                if (len(data) != size or not data.endswith(b"\n") or b"\r" in data or
                        (after.st_dev, after.st_ino) != identity or
                        (after.st_size, after.st_mtime_ns, after.st_ctime_ns) !=
                        (before.st_size, before.st_mtime_ns, before.st_ctime_ns) or
                        after.st_nlink != 1 or stat.S_IMODE(after.st_mode) != 0o600 or
                        after.st_uid != os.geteuid() or
                        identity != (named.st_dev, named.st_ino)):
                    return unknown_result
                lines = data[:-1].split(b"\n")
                producer_lines = [index for index, line in enumerate(lines)
                                  if line.startswith(b"RBL_PRODUCE_BIN_PHASE|")]
                if producer_lines != [len(lines) - 1]:
                    return unknown_result
                line = lines[-1].decode("ascii")
                match = PRODUCER_PHASE_RE.fullmatch(line)
                if match is None or match.group(1) not in PRODUCER_PHASES:
                    return unknown_result
                status = int(match.group(2))
                if not 1 <= status <= 255:
                    return unknown_result
                producer = (match.group(1), status)
                if not include_attempt or match.group(1) != "ATTEMPT":
                    return (match.group(1), status, "UNKNOWN", "UNKNOWN", "UNKNOWN") if include_attempt else producer

                capture_records = []
                capture_positions = []
                complete_line_index = None
                child_marker_indices = [index for index, item in enumerate(lines[:-1])
                                        if item.startswith(b"RBL_CAPTURE_CHILD_FAILURE")]
                for line_index, capture_line in enumerate(lines[:-1]):
                    if not capture_line.startswith(b"RBL_CAPTURE_ATTEMPT|"):
                        continue
                    try:
                        capture_text = capture_line.decode("ascii")
                    except UnicodeError:
                        return (match.group(1), status, "UNKNOWN", "UNKNOWN", "UNKNOWN")
                    capture_match = CAPTURE_ATTEMPT_RE.fullmatch(capture_text)
                    if capture_match is None:
                        return (match.group(1), status, "UNKNOWN", "UNKNOWN", "UNKNOWN")
                    capture_phase, raw_status = capture_match.groups()
                    if capture_phase not in CAPTURE_ATTEMPT_PHASE_SET or raw_status == "-0":
                        return (match.group(1), status, "UNKNOWN", "UNKNOWN", "UNKNOWN")
                    if raw_status in {"UNKNOWN", "SPAWN_ERROR"}:
                        child_status = raw_status
                    else:
                        child_status = int(raw_status)
                        if not -255 <= child_status <= 255:
                            return (match.group(1), status, "UNKNOWN", "UNKNOWN", "UNKNOWN")
                    capture_records.append((capture_phase, child_status))
                    capture_positions.append(line_index)
                    if capture_phase == "COMPLETE":
                        complete_line_index = line_index
                if not capture_records:
                    return (match.group(1), status, "UNKNOWN", "UNKNOWN", "UNKNOWN")
                previous_phase = None
                observed_child_status = "UNKNOWN"
                for capture_phase, child_status in capture_records:
                    allowed_next = {
                        None: {"SHELL_VALIDATE"},
                        "SHELL_VALIDATE": {"PYTHON_SETUP"},
                        "PYTHON_SETUP": {"HISTORY_VALIDATE"},
                        "HISTORY_VALIDATE": {"PRIOR_ARCHIVE"},
                        "PRIOR_ARCHIVE": {"ACTIVE_LOG_OPEN"},
                        "ACTIVE_LOG_OPEN": {"CHILD_PENDING"},
                        "CHILD_PENDING": {"CHILD_RETURNED", "CHILD_SPAWN_ERROR"},
                        "CHILD_RETURNED": {"LOG_SYNC"},
                        "CHILD_SPAWN_ERROR": {"LOG_SYNC"},
                        "LOG_SYNC": {"COMPLETED_ARCHIVE"},
                        "COMPLETED_ARCHIVE": {"COMPLETED_RECEIPT"},
                        "COMPLETED_RECEIPT": {"COMPLETE"},
                    }
                    if capture_phase not in allowed_next.get(previous_phase, set()):
                        return (match.group(1), status, "UNKNOWN", "UNKNOWN", "UNKNOWN")
                    previous_phase = capture_phase
                    if capture_phase in CAPTURE_ATTEMPT_PHASES[:6]:
                        if child_status != "UNKNOWN":
                            return (match.group(1), status, "UNKNOWN", "UNKNOWN", "UNKNOWN")
                    elif capture_phase == "CHILD_RETURNED":
                        if type(child_status) is not int:
                            return (match.group(1), status, "UNKNOWN", "UNKNOWN", "UNKNOWN")
                        observed_child_status = child_status
                    elif capture_phase == "CHILD_SPAWN_ERROR":
                        if child_status != "SPAWN_ERROR":
                            return (match.group(1), status, "UNKNOWN", "UNKNOWN", "UNKNOWN")
                        observed_child_status = "SPAWN_ERROR"
                    elif child_status != observed_child_status or observed_child_status == "UNKNOWN":
                        return (match.group(1), status, "UNKNOWN", "UNKNOWN", "UNKNOWN")
                pending_rows = [(index, line) for index, line in enumerate(lines[:-1])
                                if line.startswith(b"RBL_CAPTURE_PENDING")]
                if pending_rows:
                    if len(pending_rows) != 1:
                        return (match.group(1), status, "UNKNOWN", "UNKNOWN", "UNKNOWN")
                    pending_index, pending_line = pending_rows[0]
                    try:
                        if (not pending_line.startswith(b"RBL_CAPTURE_PENDING|") or
                                len(pending_line) > 8192 or len(target) != 2):
                            raise ValueError()
                        pending = observation_json(
                            pending_line[len(b"RBL_CAPTURE_PENDING|"):] + b"\n"
                        )
                        pending_identity = pending.get("log_identity")
                        guard_rows = [line for line in lines[:-1]
                                      if line.startswith(b"RBL_CAPTURE_GUARD")]
                        if len(guard_rows) > 1:
                            raise ValueError()
                        if guard_rows:
                            guard_line = guard_rows[0]
                            if (not guard_line.startswith(b"RBL_CAPTURE_GUARD|") or
                                    len(guard_line) > 8192):
                                raise ValueError()
                            completed_guard = observation_json(
                                guard_line[len(b"RBL_CAPTURE_GUARD|"):] + b"\n"
                            )
                            if (set(completed_guard) != {"request_sha256", "binding_sha256", "recipe", "bin",
                                                          "mode", "changed", "clean"} or
                                    completed_guard.get("request_sha256") != pending.get("request_sha256") or
                                    completed_guard.get("recipe") != target[0] or
                                    completed_guard.get("bin") != target[1] or
                                    completed_guard.get("mode") not in ("reset", "reuse", "clean")):
                                raise ValueError()
                        active_index = next(i for i, row in enumerate(capture_records)
                                             if row[0] == "ACTIVE_LOG_OPEN")
                        child_index = next(i for i, row in enumerate(capture_records)
                                           if row[0] == "CHILD_PENDING")
                        if (not capture_positions[active_index] < pending_index < capture_positions[child_index] or
                                set(pending) != {"format", "request_sha256", "recipe", "bin",
                                                 "attempt_label", "log_identity"} or
                                pending.get("format") != "lagrange-paper-fixture-pending-open-v1" or
                                not HASH_RE.fullmatch(pending.get("request_sha256", "")) or
                                pending.get("recipe") != target[0] or pending.get("bin") != target[1] or
                                pending.get("attempt_label") != "producer-" + target[0] + "-" + target[1] or
                                set(pending_identity) != {"dev", "ino", "uid", "mode", "nlink"} or
                                any(type(pending_identity[key]) is not int or pending_identity[key] < 0
                                    for key in ("dev", "ino", "uid", "mode", "nlink")) or
                                pending_identity["uid"] != os.geteuid() or
                                pending_identity["mode"] != 0o600 or pending_identity["nlink"] != 1 or
                                pending_identity["ino"] <= 0):
                            raise ValueError()
                    except Exception:
                        return (match.group(1), status, "UNKNOWN", "UNKNOWN", "UNKNOWN")
                if (capture_records[-1][0] == "COMPLETE" and
                        not capture_completed_status_matches(status, capture_records[-1][1])):
                    return (match.group(1), status, "UNKNOWN", "UNKNOWN", "UNKNOWN")
                child_failure_code = "UNKNOWN"
                final_phase, final_child_status = capture_records[-1]
                if (final_phase == "COMPLETE" and type(final_child_status) is int and
                        final_child_status != 0 and
                        capture_completed_status_matches(status, final_child_status) and
                        len(child_marker_indices) == 1 and complete_line_index is not None and
                        complete_line_index < child_marker_indices[0] < len(lines) - 1):
                    try:
                        child_marker = lines[child_marker_indices[0]].decode("ascii")
                    except UnicodeError:
                        child_marker = ""
                    child_match = CAPTURE_CHILD_FAILURE_RE.fullmatch(child_marker)
                    if child_match is not None and child_match.group(1) in CHILD_FAILURE_CODES:
                        child_failure_code = child_match.group(1)
                    elif child_marker == "RBL_CAPTURE_CHILD_FAILURE|UNKNOWN":
                        child_failure_code = "UNKNOWN"
                    if child_failure_code == "builder-cargo-build-failed":
                        cargo_indices = [i for i, item in enumerate(lines[:-1])
                                         if item.startswith(b"RBL_CAPTURE_CARGO_FAILURE")]
                        if (cargo_indices == [child_marker_indices[0] + 1] and
                                len(target) == 2 and type(cargo_witness) is dict):
                            try:
                                marker = lines[cargo_indices[0]].decode("ascii")
                            except UnicodeError:
                                marker = ""
                            prefix = "RBL_CAPTURE_CARGO_FAILURE|"
                            wire = marker[len(prefix):] if marker.startswith(prefix) else None
                            if cargo_failure_wire(wire, *target) is not None:
                                cargo_witness["wire"] = wire
                return (match.group(1), status, final_phase, final_child_status,
                        child_failure_code)
            finally:
                os.close(fd)
        except (OSError, AttributeError, TypeError, ValueError, UnicodeError):
            return unknown_result

    def request(self, op: str, args: tuple[str, ...] = (), *, monitor=None,
                label=None, cutoff_ns: int | None = None) -> str:
        if op not in HELPER_OPS or len(args) > 4 or self.proc.stdin is None:
            raise BuildFailure("helper-request-invalid")
        verification = op == "VERIFY_IMAGE" and len(args) == 1 and args[0] in SERVICES
        error_offset = self.verification_stderr()[0] if verification else None
        producer_offset = self.producer_phase_stderr()[0] if op == "PRODUCE_BIN" else None
        line = " ".join([op, *(b64_field(arg) for arg in args)]).rstrip() + "\n"
        self._request_pending = True
        try:
            self.proc.stdin.write(line.encode("ascii"))
            self.proc.stdin.flush()
        except (BrokenPipeError, OSError) as exc:
            raise BuildFailure("helper-server-input-failed") from exc
        try:
            response = self.readline(cutoff_ns or self.deadline_ns, monitor, label or op.lower())
        except DeadlineExceeded as error:
            if (type(op) is str and op in HELPER_OPS and error.args and
                    type(error.args[0]) is str and error.args[0].startswith("helper-deadline:")):
                error.helper_op = op
                progress = getattr(self, "progress", None)
                capture = ("UNKNOWN", "UNKNOWN")
                if op == "PRODUCE_BIN":
                    try:
                        expectation = getattr(self, "_pending_observation_expectation", None)
                        capture = self.capture_attempt_prefix(
                            producer_offset, pending_expectation=expectation,
                        )
                        if getattr(self, "_pending_capture_marker", None) is not None:
                            pending_producer_observation(self, expectation)
                    except Exception:
                        capture = ("UNKNOWN", "UNKNOWN")
                if progress is not None:
                    try:
                        progress.response_pending(op, capture)
                    except Exception:
                        pass
            raise
        try:
            kind, encoded = response.decode("ascii").split(" ", 1)
            result = base64.b64decode(encoded, validate=True).decode("utf-8")
        except (UnicodeError, ValueError) as exc:
            raise BuildFailure("helper-response-invalid") from exc
        if kind not in ("OK", "ERR"):
            raise BuildFailure("helper-response-kind-invalid")
        self._request_pending = False
        progress = getattr(self, "progress", None)
        if progress is not None:
            try:
                progress.response_returned(op)
            except Exception:
                pass
        if kind == "ERR":
            error = BuildFailure("helper-action-failed:" + result)
            if result == "helper-failed":
                error.helper_op = op
                if op == "PRODUCE_BIN":
                    if producer_offset is None:
                        error.producer_phase, error.producer_status = "UNKNOWN", "UNKNOWN"
                        error.attempt_phase, error.attempt_child_status = "UNKNOWN", "UNKNOWN"
                        error.attempt_failure_code = "UNKNOWN"
                    else:
                        cargo_witness = {}
                        (error.producer_phase, error.producer_status,
                         error.attempt_phase, error.attempt_child_status,
                         error.attempt_failure_code) = self.producer_phase_stderr(
                            producer_offset, include_attempt=True, cargo_witness=cargo_witness,
                            target=args[:2]
                        )
                        error.cargo_failure_wire = cargo_witness.get("wire")
                        error.cargo_failure_target = args[:2]
            if result == "saved-image-verification-failed" and verification:
                error.image_service = args[0]
                error.image_validator_reason = (self.verification_stderr(error_offset)[1]
                                                if error_offset is not None else "UNKNOWN")
            raise error
        return result

    def close(self):
        if self.proc.poll() is None:
            if getattr(self, "_request_pending", None) is False:
                try:
                    self.request("QUIT", cutoff_ns=self.cleanup_deadline_ns)
                except Exception:
                    pass
        stop_process_group(self.proc, 3.0)


def parse_plan(raw: str) -> tuple[ServicePlan, ...]:
    result = []
    for line in raw.splitlines():
        fields = line.split("\t")
        if len(fields) != 3:
            raise BuildFailure("helper-plan-row-invalid")
        service, recipe, encoded_bins = fields
        try:
            bins = json.loads(encoded_bins)
        except json.JSONDecodeError as exc:
            raise BuildFailure("helper-plan-bins-invalid") from exc
        if not isinstance(bins, list) or any(
            not isinstance(item, str) or not NAME_RE.fullmatch(item) for item in bins
        ):
            raise BuildFailure("helper-plan-bins-invalid")
        result.append(ServicePlan(service, recipe, tuple(bins)))
    if tuple(item.service for item in result) != SERVICES:
        raise BuildFailure("helper-plan-not-exact-v3")
    for item in result:
        if item.recipe == "NONE":
            if item.service not in NONE_SERVICES or item.bins:
                raise BuildFailure("helper-plan-none-service-invalid")
        elif not re.fullmatch(r"D[1-8]", item.recipe) or not item.bins:
            raise BuildFailure("helper-plan-recipe-invalid")
    return tuple(result)


def expected_plan_from_layout(source: pathlib.Path) -> tuple[ServicePlan, ...]:
    layout_path = source / "deploy/build/release-build-layout.json"
    try:
        layout = json.loads(layout_path.read_text(encoding="utf-8"))
        services = layout["services"]
        recipes = layout["recipes"]
    except (OSError, UnicodeError, json.JSONDecodeError, KeyError, TypeError) as exc:
        raise BuildFailure("checked-in-layout-invalid") from exc
    if not isinstance(services, dict) or not isinstance(recipes, dict):
        raise BuildFailure("checked-in-layout-invalid")
    result = []
    for service in SERVICES:
        record = services.get(service)
        if not isinstance(record, dict):
            raise BuildFailure("checked-in-service-record-invalid")
        recipe = record.get("recipe") or "NONE"
        if recipe == "NONE":
            if service not in NONE_SERVICES:
                raise BuildFailure("checked-in-none-service-invalid")
            bins = ()
        else:
            recipe_record = recipes.get(recipe)
            if not isinstance(recipe, str) or not re.fullmatch(r"D[1-8]", recipe):
                raise BuildFailure("checked-in-recipe-invalid")
            if not isinstance(recipe_record, dict):
                raise BuildFailure("checked-in-recipe-record-invalid")
            raw_bins = recipe_record.get("bins")
            if not isinstance(raw_bins, list) or not raw_bins or any(
                not isinstance(item, str) or not NAME_RE.fullmatch(item) for item in raw_bins
            ):
                raise BuildFailure("checked-in-recipe-bins-invalid")
            bins = tuple(raw_bins)
        result.append(ServicePlan(service, recipe, bins))
    return tuple(result)


def assert_plan_matches_layout(plan: tuple[ServicePlan, ...], source: pathlib.Path) -> None:
    if plan != expected_plan_from_layout(source):
        raise BuildFailure("helper-plan-differs-from-checked-in-layout")


def assert_safe_ancestors(path: pathlib.Path, root: pathlib.Path) -> None:
    try:
        relative = path.relative_to(root)
    except ValueError as exc:
        raise BuildFailure("evidence-path-outside-root") from exc
    current = root
    for component in relative.parts[:-1]:
        current /= component
        try:
            info = os.lstat(current)
        except OSError as exc:
            raise BuildFailure("evidence-directory-unavailable") from exc
        if (stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode)
                or info.st_uid != os.geteuid()):
            raise BuildFailure("evidence-directory-identity-invalid")


def observation_snapshot(info):
    return [info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns,
            info.st_uid, stat.S_IMODE(info.st_mode), info.st_nlink]


def read_regular_bytes(path: pathlib.Path, label: str, root: pathlib.Path, *,
                       limit=16 * 1024 * 1024, original=None, snapshots=None) -> tuple[bytes, int]:
    assert_safe_ancestors(path, root)
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    except OSError as exc:
        raise BuildFailure(label + "-unavailable") from exc
    try:
        info = os.fstat(fd)
        if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid()
                or info.st_size > limit or (original is not None and
                    (info.st_nlink != 1 or observation_snapshot(info) != original))):
            raise BuildFailure(label + "-file-identity-invalid")
        data = os.pread(fd, limit + 1, 0)
        named = os.stat(path, follow_symlinks=False)
        if (len(data) != info.st_size or observation_snapshot(os.fstat(fd)) != observation_snapshot(info)
                or observation_snapshot(named) != observation_snapshot(info)):
            raise BuildFailure(label + "-file-changed")
        if snapshots is not None:
            snapshots.append(observation_snapshot(info))
        return data, stat.S_IMODE(info.st_mode)
    finally:
        if fd >= 0:
            os.close(fd)


def hash_regular_file(path: pathlib.Path, label: str, root: pathlib.Path):
    assert_safe_ancestors(path, root)
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    except OSError as exc:
        raise BuildFailure(label + "-unavailable") from exc
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid():
            raise BuildFailure(label + "-file-identity-invalid")
        with os.fdopen(fd, "rb") as handle:
            fd = -1
            prefix = handle.read(4)
            handle.seek(0)
            digest = hashlib.file_digest(handle, "sha256").hexdigest()
        return {"sha256": digest, "bytes": info.st_size,
                "mode": f"{stat.S_IMODE(info.st_mode):04o}"}, prefix
    finally:
        if fd >= 0:
            os.close(fd)


def source_evidence_hashes(source: pathlib.Path) -> dict[str, dict[str, object]]:
    result = {}
    for relative in SOURCE_EVIDENCE_FILES:
        raw, mode = read_regular_bytes(source / relative, "source-input", source)
        result[relative] = {
            "sha256": hashlib.sha256(raw).hexdigest(),
            "bytes": len(raw), "mode": f"{mode:04o}",
        }
    return result


def observation_json(raw):
    def pairs(items):
        value = {}
        for key, item in items:
            if key in value:
                raise ValueError("duplicate observation key")
            value[key] = item
        return value
    value = json.loads(raw, object_pairs_hook=pairs,
                       parse_constant=lambda _value: (_ for _ in ()).throw(ValueError()))
    if raw != (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode():
        raise ValueError("noncanonical observation")
    return value


def observation_digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def validate_producer_guard(lines, prefix, recipe, bin_name, request, layout, request_digest):
    """Reuse the completed guard binding and reverse-closure contract on a prefix."""
    prefix_text = prefix.decode("ascii") if isinstance(prefix, bytes) else prefix
    candidates = []
    for raw_line in lines:
        line = raw_line.decode("utf-8", errors="strict") if isinstance(raw_line, bytes) else raw_line
        match = re.match(r"^#[0-9]+ [0-9]+(?:\.[0-9]+)? ", line)
        if match:
            line = line[match.end():]
        if line.startswith(prefix_text[:-1]):
            if not line.startswith(prefix_text) or len(line.encode("utf-8")) > 8192:
                raise ValueError("guard-marker-shape")
            candidates.append(line[len(prefix_text):].rstrip("\n"))
    if not candidates:
        return None
    if len(candidates) != 1:
        raise ValueError("guard-marker-duplicate")
    raw = (candidates[0] + "\n").encode("utf-8")
    guard = observation_json(raw)
    if (set(guard) != {"request_sha256", "binding_sha256", "recipe", "bin", "mode", "changed", "clean"}
            or guard["request_sha256"] != request_digest or guard["recipe"] != recipe
            or guard["bin"] != bin_name or guard["mode"] not in ("reset", "reuse", "clean")):
        raise ValueError("guard-marker-binding")
    binding = {"format": "lagrange-build-target-guard-v2", "k_sha256": request["k_sha256"],
               "cache_key": request["cache_key"], "cache_namespace": request["cache_namespace"],
               "platform": "linux/amd64", "profile": "release", "target_dir": "/cargo-target",
               "guard_version": "common-1"}
    if guard["binding_sha256"] != observation_digest(binding):
        raise ValueError("guard-binding-digest")
    for key in ("changed", "clean"):
        names = guard[key]
        if (type(names) is not list or len(names) > 32 or any(type(name) is not str or
                name not in layout["packages"] for name in names) or names != sorted(set(names))):
            raise ValueError("guard-package-list")
    closure = set(guard["changed"])
    for _ in layout["packages"]:
        closure.update(name for name, record in layout["packages"].items()
                       if closure.intersection(record["local_dependencies"]))
    if ((guard["mode"] == "clean" and (not closure or guard["clean"] != sorted(closure))) or
            (guard["mode"] != "clean" and (guard["changed"] or guard["clean"]))):
        raise ValueError("guard-reverse-closure")
    result = {key: guard[key] for key in ("mode", "changed", "clean", "binding_sha256")}
    result.update({"changed_count": len(guard["changed"]), "clean_count": len(guard["clean"]),
                   "selection_sha256": observation_digest(guard)})
    return result


def pending_producer_observation(helper, expectation):
    """Emit only a request-bound, stable current-log prefix; failures stay UNKNOWN."""
    try:
        if type(expectation) is not dict:
            return None
        recipe, bin_name = expectation.get("recipe"), expectation.get("bin")
        if (type(recipe) is not str or type(bin_name) is not str or
                bin_name not in OBSERVATION_BINS.get(recipe, ())):
            return None
        state = pathlib.Path(expectation["state"])
        context = pathlib.Path(expectation["context"])
        if (context.parent != state / "contexts" or
                not re.fullmatch(r"context-[0-9a-f]{64}", context.name)):
            return None

        def private_dir(path):
            info = os.lstat(path)
            if (stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode) or
                    info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o700):
                raise ValueError("private-directory-invalid")

        for directory in (state, state / "contexts", context,
                          context / ".release-build", state / "logs"):
            private_dir(directory)
        request_path = context / ".release-build/request.json"
        original_request = expectation.get("request_original")
        if (type(original_request) is not list or len(original_request) != 8 or
                any(type(value) is not int or value < 0 for value in original_request)):
            raise ValueError("pending-request-original")
        request_raw, _ = read_regular_bytes(
            request_path, "pending-request", state, limit=65536,
            original=original_request,
        )
        if hashlib.sha256(request_raw).hexdigest() != expectation.get("request_sha256"):
            raise ValueError("pending-request-binding")
        request = observation_json(request_raw)
        layout_raw, _ = read_regular_bytes(
            helper.source / "deploy/build/release-build-layout.json",
            "pending-layout", helper.source, limit=65536,
        )
        def unique_layout_pairs(items):
            result = {}
            for key, value in items:
                if key in result:
                    raise ValueError("pending-layout-duplicate-key")
                result[key] = value
            return result
        layout = json.loads(layout_raw.decode("utf-8"), object_pairs_hook=unique_layout_pairs)
        selected = layout["recipes"][recipe]
        if (request["recipe"] != recipe or request["package"] != selected["package"] or
                request["bins"] != selected["bins"] or bin_name not in selected["bins"] or
                request["source_commit"] != helper.commit or
                not COMMIT_RE.fullmatch(request["source_commit"]) or
                request["features"] != selected.get("features", []) or
                request["layout_sha256"] != hashlib.sha256(layout_raw).hexdigest() or
                request["cache_key"] != request["k_sha256"] or
                set(request["package_hashes"]) != set(layout["packages"])):
            raise ValueError("pending-request-contract")
        for key in ("input_sha256", "k_sha256", "h_sha256"):
            if not HASH_RE.fullmatch(request[key]):
                raise ValueError("pending-request-hash")
        request_digest = hashlib.sha256(request_raw).hexdigest()
        pending = getattr(helper, "_pending_capture_marker", None)
        if (type(pending) is not dict or pending.get("request_sha256") != request_digest or
                pending.get("recipe") != recipe or pending.get("bin") != bin_name or
                pending.get("attempt_label") != "producer-" + recipe + "-" + bin_name):
            raise ValueError("pending-open-marker")
        identity = pending["log_identity"]
        log_path = state / "logs" / ("producer-" + recipe + "-" + bin_name + ".log")
        fd = os.open(log_path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            before = os.fstat(fd)
            named_before = os.stat(log_path, follow_symlinks=False)
            stable = lambda info: (info.st_dev, info.st_ino, info.st_mode, info.st_uid, info.st_nlink,
                                   info.st_size, info.st_mtime_ns, info.st_ctime_ns)
            if (not stat.S_ISREG(before.st_mode) or before.st_uid != os.geteuid() or
                    stat.S_IMODE(before.st_mode) != 0o600 or before.st_nlink != 1 or
                    (before.st_dev, before.st_ino, before.st_uid, stat.S_IMODE(before.st_mode), before.st_nlink) !=
                    (identity["dev"], identity["ino"], identity["uid"], identity["mode"], identity["nlink"]) or
                    stable(named_before) != stable(before) or
                    before.st_size > 1048576):
                raise ValueError("pending-log-identity")
            data = os.read(fd, 1048577)
            after = os.fstat(fd)
            named_after = os.stat(log_path, follow_symlinks=False)
            if (len(data) != before.st_size or len(data) > 1048576 or
                    (data and (not data.endswith(b"\n") or b"\r" in data)) or
                    stable(after) != stable(before) or stable(named_after) != stable(before)):
                raise ValueError("pending-log-changed")
        finally:
            os.close(fd)
        lines = data.splitlines(keepends=True)
        guard = validate_producer_guard(lines, b"RBL_TARGET_GUARD|", recipe, bin_name,
                                        request, layout, request_digest)
        cargo_markers = []
        for raw_line in lines:
            line = raw_line.decode("utf-8", errors="strict")
            match = re.match(r"^#[0-9]+ [0-9]+(?:\.[0-9]+)? ", line)
            if match:
                line = line[match.end():]
            if line.startswith("RBL_CARGO_PROGRESS"):
                if not line.startswith("RBL_CARGO_PROGRESS|") or len(line.encode("utf-8")) > 8192:
                    raise ValueError("cargo-marker-shape")
                value = observation_json(line[len("RBL_CARGO_PROGRESS|"):].rstrip("\n").encode("utf-8") + b"\n")
                if (set(value) != {"format", "request_sha256", "recipe", "bin", "marker", "status"} or
                        value["format"] != "lagrange-cargo-call-observation-v1" or
                        value["request_sha256"] != request_digest or value["recipe"] != recipe or
                        value["bin"] != bin_name or value["marker"] not in ("CARGO_CALL_SITE", "CARGO_RETURNED")):
                    raise ValueError("cargo-marker-binding")
                cargo_markers.append(value)
        if not cargo_markers:
            cargo_marker, cargo_status = "UNKNOWN", "UNKNOWN"
        else:
            names = [value["marker"] for value in cargo_markers]
            if names not in (["CARGO_CALL_SITE"], ["CARGO_CALL_SITE", "CARGO_RETURNED"]):
                raise ValueError("cargo-marker-order")
            if cargo_markers[0]["status"] != "UNKNOWN":
                raise ValueError("cargo-callsite-status")
            if len(cargo_markers) == 1:
                cargo_marker, cargo_status = "CARGO_CALL_SITE", "UNKNOWN"
            else:
                raw_status = cargo_markers[1]["status"]
                if (type(raw_status) is not str or not re.fullmatch(r"(?:0|[1-9][0-9]{0,2})", raw_status)
                        or int(raw_status) > 255):
                    raise ValueError("cargo-return-status")
                cargo_marker, cargo_status = "CARGO_RETURNED", int(raw_status)
        result = {
            "format": "lagrange-paper-fixture-pending-observation-v1",
            "recipe": recipe, "bin": bin_name, "evidence_state": "PENDING_PREFIX",
            "request_sha256": request_digest,
            "guard": guard if guard is not None else "UNKNOWN",
            "cargo_marker": cargo_marker, "cargo_status": cargo_status,
            "read_bytes": len(data), "read_sha256": hashlib.sha256(data).hexdigest(),
        }
        print("PAPER_FIXTURE_PENDING_OBSERVATION " +
              json.dumps(result, sort_keys=True, separators=(",", ":")), flush=True)
        return result
    except Exception:
        return None


def producer_observation(helper, recipe, bin_name, context, state, started, offset, request_original,
                         returned):
    """Best-effort projection; neither a receipt nor authority to proceed."""
    if type(recipe) is not str or type(bin_name) is not str or bin_name not in OBSERVATION_BINS.get(recipe, ()):
        return
    result = {"recipe": recipe, "bin": bin_name, "wrapper_ms": "UNKNOWN",
              "cargo": "UNKNOWN", "guard": "UNKNOWN"}
    try:
        if type(started) is int:
            elapsed = (time.monotonic_ns() - started) // 1_000_000
            if 0 <= elapsed <= 5_400_000:
                result["wrapper_ms"] = elapsed
        source = helper.source
        layout_raw, _ = read_regular_bytes(source / "deploy/build/release-build-layout.json",
                                           "observation-layout", source, limit=65536)
        layout = json.loads(layout_raw)
        selected = layout["recipes"][recipe]
        if bin_name not in selected["bins"] or len(selected["bins"]) > 16:
            return
        context = pathlib.Path(context)
        if (context.parent != state / "contexts" or
                not re.fullmatch(r"context-[0-9a-f]{64}", context.name) or request_original is None):
            raise ValueError()
        request_raw, _ = read_regular_bytes(context / ".release-build/request.json", "observation-request",
                                            state, limit=65536, original=request_original)
        request = observation_json(request_raw)
        if (request["recipe"] != recipe or request["package"] != selected["package"] or
                request["bins"] != selected["bins"] or request["source_commit"] != helper.commit or
                not COMMIT_RE.fullmatch(request["source_commit"]) or
                request["features"] != selected.get("features", []) or
                request["layout_sha256"] != hashlib.sha256(layout_raw).hexdigest() or
                request["cache_key"] != request["k_sha256"] or
                set(request["package_hashes"]) != set(layout["packages"])):
            raise ValueError()
        for key in ("input_sha256", "k_sha256", "h_sha256"):
            if not HASH_RE.fullmatch(request[key]):
                raise ValueError()
        request_digest = hashlib.sha256(request_raw).hexdigest()
        log_snapshots = []
        raw, mode = read_regular_bytes(helper._stderr_path, "observation-stderr", state,
                                       limit=65536, snapshots=log_snapshots)
        if (mode != 0o600 or log_snapshots[0][:2] != list(helper._stderr_identity) or
                log_snapshots[0][-1] != 1 or type(offset) is not int or not 0 <= offset <= len(raw)):
            raise ValueError()
        lines = raw[offset:].splitlines(keepends=True)

        def wire(prefix):
            found = [line for line in lines if line.startswith(prefix)]
            if len(found) != 1 or len(found[0]) > 8192:
                raise ValueError()
            return observation_json(found[0][len(prefix):])

        try:
            result["guard"] = validate_producer_guard(
                lines, b"RBL_CAPTURE_GUARD|", recipe, bin_name, request, layout, request_digest,
            ) or "UNKNOWN"
        except Exception:
            pass
        if not returned:
            raise ValueError()
        originals = wire(b"RBL_PRODUCER_ORIGINALS|")
        names = {"artifact.json", "COMPLETE", "cargo.jsonl", "cargo.stderr", "cargo-summary.json",
                 "timing.json", "binary", "request"}
        if (set(originals) != {"request_sha256", "files"} or originals["request_sha256"] != request_digest
                or set(originals["files"]) != names or originals["files"]["request"] != request_original):
            raise ValueError()
        producer = state / "producers" / recipe / bin_name
        values = {}
        for name in sorted(names - {"request"}):
            original = originals["files"][name]
            if (type(original) is not list or len(original) != 8 or
                    any(type(v) is not int or not 0 <= v < 2**64 for v in original) or original[-1] != 1):
                raise ValueError()
            path = producer / "bin" / bin_name if name == "binary" else producer / name
            assert_safe_ancestors(path, state)
            if observation_snapshot(os.lstat(path)) != original:
                raise ValueError()
            if name in {"artifact.json", "COMPLETE", "cargo-summary.json", "timing.json"}:
                values[name] = read_regular_bytes(path, "observation-original", state, limit=65536,
                                                  original=original)[0]
        artifact = observation_json(values["artifact.json"])
        summary = observation_json(values["cargo-summary.json"])
        timing = observation_json(values["timing.json"])
        if (values["COMPLETE"] != (hashlib.sha256(values["artifact.json"]).hexdigest() + "\n").encode() or
                artifact["bin"] != bin_name or artifact["cargo_success"] is not True or
                any(artifact[key] != request[key] for key in ("source_commit", "package", "cache_key",
                    "input_sha256", "recipe_sha256", "h_sha256", "compile_env", "features")) or
                set(summary) != {"format", "cargo_success", "observed_bin", "package", "fresh_count",
                                 "compiling_count", "executable", "cargo_ms"} or
                summary["format"] != "lagrange-cargo-summary-v1" or summary["cargo_success"] is not True or
                summary["observed_bin"] != bin_name or summary["package"] != request["package"] or
                summary["executable"] != "/cargo-target/release/" + bin_name or
                set(timing) != {"format", "cargo_ms"} or timing["format"] != "lagrange-cargo-timing-v1" or
                type(timing["cargo_ms"]) is not int or
                timing["cargo_ms"] != summary["cargo_ms"]):
            raise ValueError()
        for key in ("fresh_count", "compiling_count", "cargo_ms"):
            if type(summary[key]) is not int or not 0 <= summary[key] <= (5_400_000 if key == "cargo_ms" else 100000):
                raise ValueError()
        result.update({"source_commit": request["source_commit"], "input_sha256": request["input_sha256"],
                       "k_sha256": request["k_sha256"], "h_sha256": request["h_sha256"],
                       "compile_env_sha256": observation_digest(request["compile_env"]),
                       "features_sha256": observation_digest(request["features"])})
        result["cargo"] = {key: summary[key] for key in ("fresh_count", "compiling_count", "cargo_ms")}
    except Exception:
        pass
    # Names were selected from the fixed source plan by the caller. No input prose.
    print("PAPER_FIXTURE_PRODUCER_OBSERVATION " + json.dumps(result, sort_keys=True, separators=(",", ":")),
          flush=True)


def producer_receipt_evidence(state: pathlib.Path,
                              plan: tuple[ServicePlan, ...], commit: str):
    expected_pairs = {
        (recipe, bin_name)
        for recipe, bins in RECEIPT_BINS.items()
        for bin_name in bins
    }
    result = []
    for item in plan:
        if item.recipe not in RECEIPT_BINS:
            continue
        if frozenset(item.bins) != RECEIPT_BINS[item.recipe]:
            raise BuildFailure("receipt-evidence-bin-set-invalid")
        for bin_name in item.bins:
            producer = state / "bundles" / item.recipe / ".release-build" / "producer" / bin_name
            artifact_raw, _ = read_regular_bytes(
                producer / "artifact.json", "producer-artifact", state,
            )
            complete_raw, _ = read_regular_bytes(
                producer / "COMPLETE", "producer-complete", state,
            )
            try:
                artifact = json.loads(artifact_raw.decode("utf-8"))
            except (UnicodeError, json.JSONDecodeError) as exc:
                raise BuildFailure("producer-artifact-invalid") from exc
            if (not isinstance(artifact, dict) or artifact.get("bin") != bin_name
                    or artifact.get("source_commit") != commit
                    or artifact.get("cargo_success") is not True
                    or artifact.get("binary_mode") != "0755"
                    or not isinstance(artifact.get("compile_env"), dict)
                    or artifact["compile_env"].get("LAGRANGE_CODE_COMMIT") != commit
                    or not isinstance(artifact.get("binary_sha256"), str)
                    or not HASH_RE.fullmatch(artifact["binary_sha256"])):
                raise BuildFailure("producer-artifact-binding-invalid")
            artifact_digest = hashlib.sha256(artifact_raw).hexdigest()
            if complete_raw != (artifact_digest + "\n").encode("ascii"):
                raise BuildFailure("producer-complete-binding-invalid")
            binary_info, elf_prefix = hash_regular_file(
                state / "bundles" / item.recipe / "target" / "release" / bin_name,
                "producer-elf", state,
            )
            if (elf_prefix != b"\x7fELF" or binary_info["mode"] != "0755"
                    or binary_info["sha256"] != artifact["binary_sha256"]):
                raise BuildFailure("producer-elf-binding-invalid")
            pair_material = (
                len(artifact_raw).to_bytes(8, "big") + artifact_raw
                + len(complete_raw).to_bytes(8, "big") + complete_raw
            )
            result.append({
                "recipe": item.recipe, "bin": bin_name,
                "artifact_json_sha256": artifact_digest,
                "complete_sha256": hashlib.sha256(complete_raw).hexdigest(),
                "receipt_pair_sha256": hashlib.sha256(pair_material).hexdigest(),
                "compile_env_sha256": hashlib.sha256(json.dumps(
                    artifact["compile_env"], sort_keys=True, separators=(",", ":")
                ).encode("utf-8")).hexdigest(),
                "source_commit": commit,
                "elf_sha256": binary_info["sha256"],
                "elf_bytes": binary_info["bytes"],
            })
    actual_pairs = {(item["recipe"], item["bin"]) for item in result}
    if actual_pairs != expected_pairs:
        raise BuildFailure("producer-receipt-evidence-incomplete")
    return result


def execute_plan(plan: tuple[ServicePlan, ...], backend: object,
                 guard: Callable[[str], None], commit: str, state: pathlib.Path,
                 progress: AttemptProgress | None = None):
    if tuple(item.service for item in plan) != SERVICES:
        raise BuildFailure("execution-plan-not-exact-v3")
    images = {}

    def call(operation, label, fn, *args, bin_name=None, bin_index=0):
        guard("before:" + label)
        observation = None
        if progress is not None:
            try:
                observation = progress.start(
                    item.service, item.recipe, operation, bin_name, bin_index, item.bins,
                )
            except Exception:
                observation = None
        try:
            result = fn(*args)
        except BuildFailure as error:
            helper_reason = (error.args[0] if error.args and type(error.args[0]) is str else "")
            response_deadline = (
                type(error) is DeadlineExceeded and helper_reason.startswith("helper-deadline:") and
                type(getattr(error, "helper_op", None)) is str and
                error.helper_op in HELPER_OPS
            )
            if error.args == ("helper-action-failed:helper-failed",) or response_deadline:
                error.build_service = item.service
                error.build_recipe = item.recipe
            if progress is not None:
                try:
                    progress.failed(observation)
                except Exception:
                    pass
            raise
        except Exception:
            if progress is not None:
                try:
                    progress.failed(observation)
                except Exception:
                    pass
            raise
        if progress is not None:
            try:
                progress.returned(observation)
            except Exception:
                pass
        guard("after:" + label)
        return result

    for index, item in enumerate(plan):
        service = item.service
        if item.recipe == "NONE":
            if service not in NONE_SERVICES or item.bins:
                raise BuildFailure("none-service-outside-fixed-set")
            if call("PREPARE_NONE", service + ":prepare-none", backend.prepare_none,
                    service, commit, state) != "NONE":
                raise BuildFailure("none-prepare-result-invalid")
            override = "NONE"
        else:
            context = call("MAKE_CONTEXT", service + ":context-pending", backend.make_context,
                           item.recipe, state)
            call("PRODUCE_NATIVE", service + ":native", backend.produce_native, context, state)
            context = call("MAKE_CONTEXT", service + ":context-measured", backend.make_context,
                           item.recipe, state)
            for bin_index, bin_name in enumerate(item.bins, 1):
                call("PRODUCE_BIN", service + ":producer:" + bin_name, backend.produce_bin,
                     item.recipe, bin_name, context, state,
                     bin_name=bin_name, bin_index=bin_index)
            bundle = call("BUNDLE_CREATE", service + ":bundle-create", backend.bundle_create,
                          item.recipe, state)
            call("BUNDLE_VERIFY", service + ":bundle-verify", backend.bundle_verify,
                 bundle, item.recipe)
            override_path = state / "overrides" / f"{service}-{index:02d}.json"
            call("WRITE_OVERRIDE", service + ":override", backend.write_override,
                 service, bundle, override_path)
            override = str(override_path)
        call("COMPOSE_BUILD", service + ":compose-build", backend.compose_build,
             service, override)
        image = call("VERIFY_IMAGE", service + ":image-verify", backend.verify_image,
                     service, commit, state)
        image_id = image.get("image_id") if isinstance(image, dict) else None
        image_ref = image.get("image_ref") if isinstance(image, dict) else None
        if (
            not isinstance(image, dict) or image.get("service") != service
            or image.get("revision") != commit
            or not isinstance(image_id, str) or not image_id.startswith("sha256:")
            or not HASH_RE.fullmatch(image_id.removeprefix("sha256:"))
            or image_ref != f"lagrange-station-{service}:{commit}"
        ):
            raise BuildFailure("verified-image-observation-invalid")
        images[service] = image
    if tuple(images) != SERVICES:
        raise BuildFailure("verified-image-set-incomplete")
    return images


def execute_build(plan: tuple[ServicePlan, ...], backend: object,
                  guard: Callable[[str], None], commit: str,
                  state: pathlib.Path, candidate: pathlib.Path,
                  progress: AttemptProgress | None = None):
    images = execute_plan(plan, backend, guard, commit, state, progress)
    guard("before:final-image-reinspection-and-v3-validation")
    backend.publish_temp(images, candidate)
    guard("after:final-image-reinspection-and-v3-validation")
    return images


def process_environment(state: pathlib.Path, commit: str, builder: str,
                        env_file: pathlib.Path) -> dict[str, str]:
    return {
        "PATH": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "HOME": str(state / "home"),
        "DOCKER_CONFIG": str(state / "docker-config"), "TMPDIR": str(state / "tmp"),
        "LANG": "C.UTF-8", "LC_ALL": "C",
        "CARGO_BUILD_JOBS": "2", "COMPOSE_PARALLEL_LIMIT": "1",
        "COMPOSE_PROFILES": "", "BUILDX_BUILDER": builder,
        "LAGRANGE_CODE_COMMIT": commit, "PAPER_ENV_FILE": str(env_file),
    }


def read_fd_bytes(fd: int) -> bytes:
    os.lseek(fd, 0, os.SEEK_SET)
    chunks = []
    while True:
        chunk = os.read(fd, 1024 * 1024)
        if not chunk:
            return b"".join(chunks)
        chunks.append(chunk)


def validate_manifest_bytes(data: bytes, commit: str) -> str:
    try:
        lines = data.decode("ascii").splitlines()
    except UnicodeError as exc:
        raise BuildFailure("manifest-candidate-not-ascii") from exc
    if (not data.endswith(b"\n") or b"\r" in data or b"\x00" in data
            or len(lines) != 15 or lines[0] != "LAGRANGE_RELEASE_MANIFEST_V3"
            or lines[1] != "commit|" + commit):
        raise BuildFailure("manifest-candidate-not-v3")
    return hashlib.sha256(data).hexdigest()


def candidate_bytes(path: pathlib.Path, state: pathlib.Path, commit: str):
    info = os.lstat(path)
    if (not stat.S_ISREG(info.st_mode) or stat.S_ISLNK(info.st_mode)
            or info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o600
            or info.st_nlink != 1 or path.parent != state):
        raise BuildFailure("manifest-candidate-identity-invalid")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        opened = os.fstat(fd)
        if ((opened.st_dev, opened.st_ino) != (info.st_dev, info.st_ino)
                or not stat.S_ISREG(opened.st_mode) or opened.st_uid != os.geteuid()
                or stat.S_IMODE(opened.st_mode) != 0o600 or opened.st_nlink != 1):
            raise BuildFailure("manifest-candidate-identity-invalid")
        data = read_fd_bytes(fd)
    finally:
        os.close(fd)
    return data, validate_manifest_bytes(data, commit)


def publish_no_replace(candidate: pathlib.Path, manifest: pathlib.Path,
                       state: pathlib.Path, commit: str) -> str:
    if candidate.parent != state or manifest.parent != state or candidate.name == manifest.name:
        raise BuildFailure("manifest-publication-path-invalid")
    data, digest = candidate_bytes(candidate, state, commit)
    dirfd = os.open(state, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    candidate_fd = -1
    manifest_fd = -1
    linked_identity = None
    try:
        directory_info = os.fstat(dirfd)
        if (not stat.S_ISDIR(directory_info.st_mode) or directory_info.st_uid != os.geteuid()
                or stat.S_IMODE(directory_info.st_mode) != 0o700):
            raise BuildFailure("manifest-state-directory-invalid")
        try:
            os.stat(manifest.name, dir_fd=dirfd, follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            raise BuildFailure("manifest-target-already-exists")

        candidate_fd = os.open(candidate.name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=dirfd)
        source_info = os.fstat(candidate_fd)
        source_identity = (source_info.st_dev, source_info.st_ino)
        source_data = read_fd_bytes(candidate_fd)
        candidate_name_info = os.stat(candidate.name, dir_fd=dirfd, follow_symlinks=False)
        if ((candidate_name_info.st_dev, candidate_name_info.st_ino) != source_identity
                or not stat.S_ISREG(source_info.st_mode) or source_info.st_uid != os.geteuid()
                or stat.S_IMODE(source_info.st_mode) != 0o600 or source_info.st_nlink != 1
                or source_data != data or validate_manifest_bytes(source_data, commit) != digest):
            raise BuildFailure("manifest-candidate-changed-before-publish")

        try:
            os.link(candidate.name, manifest.name, src_dir_fd=dirfd, dst_dir_fd=dirfd,
                    follow_symlinks=False)
        except FileExistsError as exc:
            raise BuildFailure("manifest-target-no-replace-failed") from exc
        linked_identity = source_identity
        os.fsync(dirfd)

        source_info = os.fstat(candidate_fd)
        manifest_fd = os.open(manifest.name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=dirfd)
        target_info = os.fstat(manifest_fd)
        target_name_info = os.stat(manifest.name, dir_fd=dirfd, follow_symlinks=False)
        candidate_name_info = os.stat(candidate.name, dir_fd=dirfd, follow_symlinks=False)
        if ((target_info.st_dev, target_info.st_ino) != source_identity
                or (target_name_info.st_dev, target_name_info.st_ino) != source_identity
                or (candidate_name_info.st_dev, candidate_name_info.st_ino) != source_identity
                or target_info.st_nlink != 2 or source_info.st_nlink != 2
                or stat.S_IMODE(target_info.st_mode) != 0o600
                or target_info.st_uid != os.geteuid() or read_fd_bytes(manifest_fd) != data):
            raise BuildFailure("published-manifest-identity-invalid")

        os.unlink(candidate.name, dir_fd=dirfd)
        os.fsync(dirfd)
        try:
            os.stat(candidate.name, dir_fd=dirfd, follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            raise BuildFailure("manifest-candidate-retirement-failed")
        target_info = os.fstat(manifest_fd)
        source_info = os.fstat(candidate_fd)
        target_name_info = os.stat(manifest.name, dir_fd=dirfd, follow_symlinks=False)
        if ((target_info.st_dev, target_info.st_ino) != source_identity
                or (target_name_info.st_dev, target_name_info.st_ino) != source_identity
                or (source_info.st_dev, source_info.st_ino) != source_identity
                or target_info.st_nlink != 1 or source_info.st_nlink != 1
                or stat.S_IMODE(target_info.st_mode) != 0o600
                or target_info.st_uid != os.geteuid() or read_fd_bytes(manifest_fd) != data
                or hashlib.sha256(read_fd_bytes(manifest_fd)).hexdigest() != digest):
            raise BuildFailure("published-manifest-final-identity-invalid")
        return digest
    except BaseException as publication_error:
        if linked_identity is not None:
            try:
                target_info = os.stat(manifest.name, dir_fd=dirfd, follow_symlinks=False)
                if (target_info.st_dev, target_info.st_ino) == linked_identity:
                    os.unlink(manifest.name, dir_fd=dirfd)
                    os.fsync(dirfd)
                else:
                    raise CleanupFailure("publication-rollback-target-identity-changed")
            except FileNotFoundError:
                pass
            except BaseException as cleanup_error:
                raise CleanupFailure("publication-rollback-failed") from cleanup_error
        raise publication_error
    finally:
        for fd in (manifest_fd, candidate_fd, dirfd):
            if fd >= 0:
                try:
                    os.close(fd)
                except OSError:
                    pass


def remove_unaccepted_checks(path: pathlib.Path, state: pathlib.Path,
                             expected_identity: tuple[int, int]) -> None:
    if path.parent != state:
        raise CleanupFailure("unaccepted-checks-path-invalid")
    dirfd = os.open(state, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        try:
            info = os.stat(path.name, dir_fd=dirfd, follow_symlinks=False)
        except FileNotFoundError:
            return
        if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid()
                or stat.S_IMODE(info.st_mode) != 0o600 or info.st_nlink != 1
                or (info.st_dev, info.st_ino) != expected_identity):
            raise CleanupFailure("unaccepted-checks-identity-changed")
        os.unlink(path.name, dir_fd=dirfd)
        os.fsync(dirfd)
        try:
            os.stat(path.name, dir_fd=dirfd, follow_symlinks=False)
        except FileNotFoundError:
            return
        raise CleanupFailure("unaccepted-checks-remains")
    finally:
        os.close(dirfd)


def publish_with_checks_cleanup(candidate: pathlib.Path, manifest: pathlib.Path,
                                state: pathlib.Path, commit: str,
                                checks_path: pathlib.Path,
                                checks_identity: tuple[int, int]) -> str:
    try:
        return publish_no_replace(candidate, manifest, state, commit)
    except BaseException:
        try:
            remove_unaccepted_checks(checks_path, state, checks_identity)
        except BaseException as cleanup_error:
            raise CleanupFailure("unaccepted-checks-cleanup-failed") from cleanup_error
        raise


def cleanup_builder(builder: str, env: dict[str, str], state: pathlib.Path,
                    deadline_ns: int) -> None:
    before = run_owned(
        ["docker", "buildx", "ls", "--format", "{{.Name}}"],
        "buildx-ls-before-remove", env, state, deadline_ns, output=True,
    )
    names = {line.strip().removesuffix("*") for line in before.splitlines()}
    if builder in names:
        run_owned(["docker", "buildx", "rm", "--force", builder],
                  "buildx-remove", env, state, deadline_ns)
    after = run_owned(
        ["docker", "buildx", "ls", "--format", "{{.Name}}"],
        "buildx-ls-after-remove", env, state, deadline_ns, output=True,
    )
    if builder in {line.strip().removesuffix("*") for line in after.splitlines()}:
        raise CleanupFailure("ephemeral-buildx-builder-remains")


class RealBackend:
    def __init__(self, helper: HelperServer, monitor: ResourceMonitor):
        self.helper = helper
        self.monitor = monitor

    def call(self, op: str, *args: str):
        result = self.helper.request(op, tuple(args), monitor=self.monitor, label=op.lower())
        self.monitor.check("after:" + op.lower(), self.helper.proc.pid)
        return result

    def make_context(self, recipe, state): return self.call("MAKE_CONTEXT", recipe)
    def produce_native(self, context, state): return self.call("PRODUCE_NATIVE", context)
    def produce_bin(self, recipe, bin_name, context, state):
        started = time.monotonic_ns()
        offset = None
        snapshots = []
        request_raw = None
        try:
            offset = self.helper.producer_phase_stderr()[0]
            path = pathlib.Path(context)
            if path.parent != state / "contexts" or not re.fullmatch(r"context-[0-9a-f]{64}", path.name):
                raise ValueError()
            request_raw, _ = read_regular_bytes(path / ".release-build/request.json", "observation-request",
                                                state, limit=65536, snapshots=snapshots)
        except Exception:
            pass
        expectation = None
        if request_raw is not None and snapshots:
            expectation = {
                "recipe": recipe, "bin": bin_name, "context": str(context), "state": state,
                "request_original": snapshots[0],
                "request_sha256": hashlib.sha256(request_raw).hexdigest(),
            }
        self.helper._pending_observation_expectation = expectation
        returned = False
        try:
            result = self.call("PRODUCE_BIN", recipe, bin_name, context)
            returned = result == str(state / "producers" / recipe / bin_name)
            return result
        finally:
            self.helper._pending_observation_expectation = None
            try:
                producer_observation(self.helper, recipe, bin_name, context, state, started, offset,
                                     snapshots[0] if snapshots else None, returned)
            except Exception:
                pass
    def bundle_create(self, recipe, state): return self.call("BUNDLE_CREATE", recipe)
    def bundle_verify(self, bundle, recipe): return self.call("BUNDLE_VERIFY", bundle, recipe)
    def write_override(self, service, bundle, path):
        return self.call("WRITE_OVERRIDE", service, bundle, str(path))
    def prepare_none(self, service, commit, state): return self.call("PREPARE_NONE", service)
    def compose_build(self, service, override):
        return self.call("COMPOSE_BUILD", service, override)
    def verify_image(self, service, commit, state):
        fields = self.call("VERIFY_IMAGE", service).split("|")
        if len(fields) != 3:
            raise BuildFailure("image-verification-response-invalid")
        return {"service": service, "image_ref": fields[0],
                "image_id": fields[1], "revision": fields[2]}
    def publish_temp(self, images, path):
        if tuple(images) != SERVICES:
            raise BuildFailure("publish-image-set-invalid")
        if self.call("PUBLISH_TEMP", str(path)) != str(path):
            raise BuildFailure("publish-candidate-path-mismatch")


# Only source-owned finite failure vocabulary may cross the CLI boundary.
CLOSED_FAILURE_REASONS = frozenset({
    'build-budget-below-cleanup-reserve',
    'build-deadline-reserve-reached',
    'checked-in-layout-invalid',
    'checked-in-none-service-invalid',
    'checked-in-recipe-bins-invalid',
    'checked-in-recipe-invalid',
    'checked-in-recipe-record-invalid',
    'checked-in-service-record-invalid',
    'checks-file-identity-invalid',
    'cleanup-stack-closed',
    'command-argv-invalid',
    'commit-invalid',
    'compiler-inventory-unobservable',
    'compiler-process-present-at-boundary',
    'compose-file-not-regular',
    'compose-interpolation-set-empty',
    'compose-references-prohibited-account-input',
    'compose-references-prohibited-credential-input',
    'compose-references-prohibited-secret-input',
    'docker-daemon-identity-changed',
    'docker-daemon-identity-unavailable',
    'ephemeral-buildx-builder-remains',
    'ephemeral-buildx-driver-invalid',
    'ephemeral-buildx-name-collision',
    'evidence-directory-identity-invalid',
    'evidence-directory-unavailable',
    'evidence-path-outside-root',
    'execution-plan-not-exact-v3',
    'helper-plan-bins-invalid',
    'helper-plan-differs-from-checked-in-layout',
    'helper-plan-none-service-invalid',
    'helper-plan-not-exact-v3',
    'helper-plan-recipe-invalid',
    'helper-plan-row-invalid',
    'helper-request-invalid',
    'helper-response-invalid',
    'helper-response-kind-invalid',
    'helper-server-closed-output',
    'helper-server-exited',
    'helper-server-input-failed',
    'helper-server-startup-failed',
    'image-verification-response-invalid',
    'kernel-oom-counter-changed',
    'kernel-oom-counter-unavailable',
    'live-account-interpolation-not-disabled',
    'manifest-candidate-changed-before-publish',
    'manifest-candidate-identity-invalid',
    'manifest-candidate-not-ascii',
    'manifest-candidate-not-v3',
    'manifest-candidate-retirement-failed',
    'manifest-file-path-invalid',
    'manifest-file-path-symlink',
    'manifest-must-be-direct-state-child',
    'manifest-path-invalid-or-present',
    'manifest-publication-path-invalid',
    'manifest-state-directory-invalid',
    'manifest-target-already-exists',
    'manifest-target-no-replace-failed',
    'none-prepare-result-invalid',
    'none-service-outside-fixed-set',
    'owned-helper-has-unreaped-children',
    'owned-process-group-remains',
    'private-env-mode-invalid',
    'process-inventory-invalid',
    'process-inventory-unobservable',
    'producer-artifact-binding-invalid',
    'producer-artifact-file-identity-invalid',
    'producer-artifact-invalid',
    'producer-artifact-unavailable',
    'producer-complete-binding-invalid',
    'producer-complete-file-identity-invalid',
    'producer-complete-unavailable',
    'producer-elf-binding-invalid',
    'producer-elf-file-identity-invalid',
    'producer-elf-unavailable',
    'producer-receipt-evidence-incomplete',
    'publication-rollback-failed',
    'publication-rollback-target-identity-changed',
    'publish-candidate-path-mismatch',
    'publish-image-set-invalid',
    'published-manifest-final-identity-invalid',
    'published-manifest-identity-invalid',
    'receipt-evidence-bin-set-invalid',
    'resource-observation-unavailable-or-invalid',
    'resource-policy-api-invalid',
    'resource-policy-missing-or-symlinked',
    'source-head-mismatch',
    'source-input-file-identity-invalid',
    'source-input-unavailable',
    'source-root-not-canonical-directory',
    'source-root-path-invalid',
    'source-root-path-symlink',
    'source-worktree-not-clean',
    'state-or-manifest-already-exists',
    'state-root-already-exists',
    'state-root-identity-invalid',
    'state-root-location-invalid',
    'state-root-overlaps-source',
    'state-root-path-invalid',
    'state-root-path-symlink',
    'unaccepted-checks-cleanup-failed',
    'unaccepted-checks-identity-changed',
    'unaccepted-checks-path-invalid',
    'unaccepted-checks-remains',
    'verified-image-observation-invalid',
    'verified-image-set-incomplete',
})
CLOSED_HELPER_REASONS = frozenset({
    'action-invalid',
    'compose-build-failed',
    'compose-log-exists',
    'final-image-identity-drift',
    'final-image-map-incomplete',
    'final-image-reinspect-failed',
    'final-image-reinspect-shape-invalid',
    'helper-failed',
    'image-identity-invalid',
    'image-inspect-failed',
    'image-inspect-shape-invalid',
    'image-reference-invalid',
    'manifest-not-v3',
    'manifest-temp-already-exists',
    'manifest-validation-failed',
    'manifest-write-failed',
    'none-service-invalid',
    'plan-bins-invalid',
    'plan-recipe-invalid',
    'saved-image-verification-failed',
})
CLOSED_FAILURE_PREFIXES = frozenset((
    "resource-policy-stop", "command-deadline", "command-finished-after-deadline",
    "command-left-owned-processes", "command-failed", "command-output-invalid",
    "helper-deadline", "helper-action-failed", "cleanup-failed",
))
CLEANUP_ACTIONS = ("helper-server", "buildx-builder")
FAILURE_STAGES = {
    "main": "MAIN", "validate_path_arguments": "INPUTS", "canonical_abs": "INPUTS",
    "assert_clean_source": "SOURCE", "source_evidence_hashes": "SOURCE",
    "create_private_state": "STATE", "disabled_env_values": "STATE",
    "ResourceMonitor.__init__": "RESOURCE", "ResourceMonitor.check": "RESOURCE",
    "run_owned": "COMMAND", "HelperServer.__init__": "HELPER",
    "HelperServer.readline": "HELPER", "HelperServer.request": "HELPER",
    "execute_plan": "BUILD", "execute_build": "BUILD",
    "producer_receipt_evidence": "PRODUCER", "hash_regular_file": "EVIDENCE",
    "read_regular_bytes": "EVIDENCE", "assert_safe_ancestors": "EVIDENCE",
    "publish_no_replace": "PUBLICATION", "publish_with_checks_cleanup": "PUBLICATION",
    "candidate_bytes": "PUBLICATION", "candidate_data_valid": "PUBLICATION",
    "CleanupStack.close": "CLEANUP", "cleanup_builder": "CLEANUP",
    "stop_process_group": "CLEANUP", "HelperServer.close": "CLEANUP",
}


def cleanup_failure_projection(error):
    cleanup_diagnostic = failure_diagnostic(error, _include_cleanup=False)
    if cleanup_diagnostic["reason"] != "cleanup-failed":
        return None
    base_keys = ("kind", "stage", "reason", "line", "caller_line", "errno")
    projection = {key: cleanup_diagnostic[key] for key in base_keys}
    attributes = getattr(error, "__dict__", {})
    failures = attributes.get("_cleanup_failures") if type(attributes) is dict else None
    actions = []
    valid = type(failures) is tuple and 1 <= len(failures) <= 2
    previous_index = -1
    if valid:
        for pair in failures:
            if (type(pair) is not tuple or len(pair) != 2 or
                    type(pair[0]) is not str or pair[0] not in CLEANUP_ACTIONS or
                    not isinstance(pair[1], Exception)):
                valid = False
                break
            index = CLEANUP_ACTIONS.index(pair[0])
            if index <= previous_index:
                valid = False
                break
            previous_index = index
            try:
                callback = failure_diagnostic(pair[1], _include_cleanup=False)
            except Exception:
                valid = False
                break
            actions.append({"action": pair[0], **{
                key: callback[key] for key in base_keys
            }})
    projection["actions"] = actions if valid else "UNKNOWN"
    return projection


def failure_diagnostic(error, *, _include_cleanup=True):
    """Closed codes and source positions only; never arbitrary exception prose."""
    if isinstance(error, DeadlineExceeded):
        kind = "DeadlineExceeded"
    elif isinstance(error, CleanupFailure):
        kind = "CleanupFailure"
    elif isinstance(error, BuildFailure):
        kind = "BuildFailure"
    elif isinstance(error, OSError):
        kind = "OSError"
    elif isinstance(error, subprocess.SubprocessError):
        kind = "SubprocessError"
    else:
        kind = "UNKNOWN"
    value = error.args[0] if error.args and type(error.args[0]) is str else ""
    reason = value if value in CLOSED_FAILURE_REASONS else "UNKNOWN"
    if reason == "UNKNOWN":
        prefix, separator, detail = value.partition(":")
        if separator and prefix in CLOSED_FAILURE_PREFIXES:
            reason = prefix
            if prefix == "helper-action-failed" and detail in CLOSED_HELPER_REASONS:
                reason += ":" + detail
            elif prefix == "resource-policy-stop" and detail in (
                    "mem-available-below-floor", "memory-pressure-high"):
                reason += ":" + detail
    # Embedded Python starts at physical shell-script line4 (fixed entrypoint).
    frames, trace = [], error.__traceback__
    while trace is not None:
        if trace.tb_frame.f_globals is globals():
            frames.append((trace.tb_frame.f_code.co_qualname, trace.tb_lineno))
        trace = trace.tb_next
    stage = FAILURE_STAGES.get(frames[-1][0], "INTERNAL") if frames else "UNKNOWN"
    number = getattr(error, "errno", None)
    diagnostic = {"kind": kind, "stage": stage, "reason": reason,
                  "line": frames[-1][1] + 3 if frames else 0,
                  "caller_line": frames[-2][1] + 3 if len(frames) > 1 else 0,
                  "errno": number if type(number) is int and 0 < number < 4096 else 0}
    operation = getattr(error, "helper_op", None)
    helper_context = (reason == "helper-action-failed:helper-failed" or
                      (type(error) is DeadlineExceeded and reason == "helper-deadline"))
    if (helper_context and
            type(operation) is str and operation in HELPER_OPS):
        diagnostic["helper_op"] = operation
        service, recipe = getattr(error, "build_service", None), getattr(error, "build_recipe", None)
        if (type(service) is str and service in SERVICES and
                type(recipe) is str and re.fullmatch(r"D[1-8]", recipe)):
            diagnostic["build_service"], diagnostic["build_recipe"] = service, recipe
    if (reason == "helper-action-failed:helper-failed" and
            type(operation) is str and operation == "PRODUCE_BIN"):
        phase = getattr(error, "producer_phase", None)
        status = getattr(error, "producer_status", None)
        if (type(phase) is str and phase in PRODUCER_PHASES and
                type(status) is int and 1 <= status <= 255):
            diagnostic["producer_phase"], diagnostic["producer_status"] = phase, status
        else:
            diagnostic["producer_phase"] = "UNKNOWN"
            diagnostic["producer_status"] = "UNKNOWN"
        if (diagnostic["producer_phase"] == "ATTEMPT" and
                type(status) is int and 1 <= status <= 255):
            attempt_phase = getattr(error, "attempt_phase", None)
            attempt_status = getattr(error, "attempt_child_status", None)
            prechild = {"SHELL_VALIDATE", "PYTHON_SETUP", "HISTORY_VALIDATE", "PRIOR_ARCHIVE",
                        "ACTIVE_LOG_OPEN", "CHILD_PENDING"}
            valid_phase = type(attempt_phase) is str and attempt_phase in CAPTURE_ATTEMPT_PHASE_SET
            valid_status = valid_phase and (
                type(attempt_status) is str and
                ((attempt_status == "UNKNOWN" and attempt_phase in prechild) or
                 (attempt_status == "SPAWN_ERROR" and attempt_phase in {
                     "CHILD_SPAWN_ERROR", "LOG_SYNC", "COMPLETED_ARCHIVE",
                     "COMPLETED_RECEIPT", "COMPLETE"})) or
                type(attempt_status) is int and -255 <= attempt_status <= 255 and
                attempt_phase in {"CHILD_RETURNED", "LOG_SYNC", "COMPLETED_ARCHIVE",
                                  "COMPLETED_RECEIPT", "COMPLETE"})
            if valid_status and attempt_phase == "COMPLETE":
                valid_status = capture_completed_status_matches(status, attempt_status)
            if valid_status:
                diagnostic["attempt_phase"] = attempt_phase
                diagnostic["attempt_child_status"] = attempt_status
                if (attempt_phase == "COMPLETE" and type(attempt_status) is int and
                        attempt_status != 0 and status == attempt_status % 256):
                    code = getattr(error, "attempt_failure_code", None)
                    diagnostic["attempt_failure_code"] = (
                        code if type(code) is str and
                        (code == "UNKNOWN" or code in CHILD_FAILURE_CODES) else "UNKNOWN"
                    )
                    if diagnostic["attempt_failure_code"] == "builder-cargo-build-failed":
                        diagnostic["cargo_failure"] = "UNKNOWN"
                        target = getattr(error, "cargo_failure_target", ())
                        wire = getattr(error, "cargo_failure_wire", None)
                        if (type(target) is tuple and len(target) == 2 and
                                target[0] == diagnostic.get("build_recipe") and
                                cargo_failure_wire(wire, *target) is not None):
                            recipe, bin_name, cargo_status, code, path, row, column = wire.split("|")
                            diagnostic["cargo_failure"] = {
                                "status": int(cargo_status), "bin": bin_name,
                                "diagnostic": ("UNKNOWN" if code == "UNKNOWN" else {
                                    "code": code, "path": path, "line": int(row), "column": int(column),
                                }),
                            }
            else:
                diagnostic["attempt_phase"] = "UNKNOWN"
                diagnostic["attempt_child_status"] = "UNKNOWN"
    service = getattr(error, "image_service", None)
    if (reason == "helper-action-failed:saved-image-verification-failed" and
            type(service) is str and service in SERVICES):
        code = getattr(error, "image_validator_reason", None)
        diagnostic["image_service"] = service
        diagnostic["image_validator_reason"] = (code if type(code) is str and
                                                 code in IMAGE_VERIFICATION_CODES else "UNKNOWN")
    if _include_cleanup is True:
        cleanup_error = None
        if isinstance(error, Exception):
            attributes = getattr(error, "__dict__", {})
            attached = attributes.get("_cleanup_failure") if type(attributes) is dict else None
            if type(attached) is CleanupFailure:
                cleanup_error = attached
        if cleanup_error is None and type(error) is CleanupFailure:
            cleanup_error = error
        if cleanup_error is not None:
            cleanup_projection = cleanup_failure_projection(cleanup_error)
            if cleanup_projection is not None:
                diagnostic["cleanup_failure"] = cleanup_projection
    return diagnostic


def report_failure(error):
    diagnostic = failure_diagnostic(error)
    print("paper-fixture-image-build: " + diagnostic["kind"], file=sys.stderr)
    print("PAPER_FIXTURE_IMAGE_FAILURE " + json.dumps(diagnostic, sort_keys=True,
          separators=(",", ":")), file=sys.stderr)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    source, state, manifest = validate_path_arguments(
        args.source_root, args.commit, args.state_root, args.manifest_file,
        args.deadline_monotonic_ns,
    )
    assert_clean_source(source, args.commit)
    source_hashes = source_evidence_hashes(source)
    env_file, nonce = create_private_state(source, state, manifest, args.commit)
    monitor = ResourceMonitor(source, args.deadline_monotonic_ns)
    deadline = monitor.build_deadline_ns
    builder = "paper-fixture-" + secrets.token_hex(16)
    namespace = "fixture-" + nonce[:24]
    cleanup_deadline = args.deadline_monotonic_ns - 15_000_000_000
    env = process_environment(state, args.commit, builder, env_file)
    cleanup = CleanupStack()
    builder_removed = False
    published = None
    body_error = None
    progress = AttemptProgress(deadline, args.commit)

    def cleanup_ephemeral_builder():
        nonlocal builder_removed
        if builder_removed:
            return
        cleanup_builder(builder, env, state, cleanup_deadline)
        builder_removed = True

    try:
        monitor.check("before-preflight")
        run_owned(["docker", "compose", "version"], "compose-version", env, state, deadline)
        monitor.check("after-compose-version")
        run_owned(["docker", "buildx", "version"], "buildx-version", env, state, deadline)
        monitor.check("after-buildx-version")
        daemon_id = run_owned(
            ["docker", "info", "--format", "{{.ID}}"], "daemon-identity-before",
            env, state, deadline, output=True,
        )
        if not daemon_id or "\n" in daemon_id or "\r" in daemon_id:
            raise BuildFailure("docker-daemon-identity-unavailable")
        monitor.check("after-daemon-identity")
        run_owned(
            ["docker", "compose", "--env-file", str(env_file), "--file",
             str(source / "deploy/compose/compose.yml"), "config", "--quiet"],
            "compose-config", env, state, deadline,
        )
        monitor.check("after-preflight")

        existing_builders = run_owned(
            ["docker", "buildx", "ls", "--format", "{{.Name}}"],
            "buildx-ls-before-create", env, state, deadline, output=True,
        )
        if builder in {line.strip().removesuffix("*") for line in existing_builders.splitlines()}:
            raise BuildFailure("ephemeral-buildx-name-collision")
        # The 128-bit name was absent immediately before creation; register
        # exact-name cleanup before a timeout can hide a partial create.
        cleanup.push("buildx-builder", cleanup_ephemeral_builder)
        run_owned(["docker", "buildx", "create", "--name", builder,
                   "--driver", "docker-container"],
                  "buildx-create", env, state, deadline)
        monitor.check("after-buildx-create")
        builder_rows = run_owned(
            ["docker", "buildx", "ls", "--format", "{{.Name}}\t{{.DriverEndpoint}}"],
            "buildx-driver", env, state, deadline, output=True,
        )
        drivers = [
            value for name, _, value in (row.partition("\t") for row in builder_rows.splitlines())
            if name == builder
        ]
        if drivers != ["docker-container"]:
            raise BuildFailure("ephemeral-buildx-driver-invalid")
        run_owned(["docker", "buildx", "inspect", "--bootstrap", builder],
                  "buildx-bootstrap", env, state, deadline)
        monitor.check("after-buildx-bootstrap")

        helper = HelperServer(source, args.commit, state, env_file, namespace,
                              builder, env, deadline, cleanup_deadline, progress=progress)
        cleanup.push("helper-server", helper.close)
        monitor.check("after-layout-init", helper.proc.pid)
        plan = parse_plan(helper.request("PLAN", monitor=monitor, label="fixed-v3-plan"))
        assert_plan_matches_layout(plan, source)
        monitor.check("after-plan", helper.proc.pid)
        backend = RealBackend(helper, monitor)
        candidate = state / (".manifest-candidate-" + secrets.token_hex(8))
        images = execute_build(
            plan, backend,
            lambda label: monitor.check(label, helper.proc.pid),
            args.commit, state, candidate, progress=progress,
        )
        monitor.check("before-producer-evidence", helper.proc.pid)
        receipt_evidence = producer_receipt_evidence(state, plan, args.commit)
        try:
            progress.originals_verified(receipt_evidence)
        except Exception:
            pass
        monitor.check("after-producer-evidence", helper.proc.pid)
        daemon_after = run_owned(
            ["docker", "info", "--format", "{{.ID}}"], "daemon-identity-after",
            env, state, deadline, output=True,
        )
        if daemon_after != daemon_id:
            raise BuildFailure("docker-daemon-identity-changed")
        monitor.check("after-all-thirteen-images", helper.proc.pid)
        monitor.check("after-manifest-candidate", helper.proc.pid)
        cleanup.close()
        monitor.check("after-owned-cleanup", in_cleanup=True)
        _, candidate_digest = candidate_bytes(candidate, state, args.commit)
        monitor.check("before-manifest-publish", in_cleanup=True)
        checks = {
            "format": "lagrange-paper-fixture-image-checks-v1",
            "source_commit": args.commit,
            "daemon_id_sha256": hashlib.sha256(daemon_id.encode()).hexdigest(),
            "services": [
                {"service": service, "image_id": images[service]["image_id"],
                 "image_ref": images[service]["image_ref"],
                 "revision": images[service]["revision"]}
                for service in SERVICES
            ],
            "resource_observations": monitor.checks,
            "source_inputs": source_hashes,
            "producer_receipt_evidence": receipt_evidence,
            "manifest_candidate_sha256": candidate_digest,
            "cleanup": {
                "helper_server": "reaped",
                "ephemeral_buildx_builder": (
                    "removed-and-verified-absent" if builder_removed else "not-created"
                ),
            },
            "runtime_authority": "none",
        }
        path = state / "checks.json"
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "wb") as handle:
            handle.write((json.dumps(checks, sort_keys=True, separators=(",", ":")) + "\n").encode())
            handle.flush()
            os.fsync(handle.fileno())
        checks_info = os.stat(path, follow_symlinks=False)
        if (not stat.S_ISREG(checks_info.st_mode) or checks_info.st_uid != os.geteuid()
                or stat.S_IMODE(checks_info.st_mode) != 0o600 or checks_info.st_nlink != 1):
            raise BuildFailure("checks-file-identity-invalid")
        published = publish_with_checks_cleanup(
            candidate, manifest, state, args.commit, path,
            (checks_info.st_dev, checks_info.st_ino),
        )
        print("PAPER_FIXTURE_IMAGE_BUILD=PASS")
        print(f"manifest={manifest}")
        print(f"manifest_sha256={published}")
        return 0
    except Exception as error:
        body_error = error
        raise
    finally:
        if not cleanup.closed:
            try:
                cleanup.close()
            except Exception as cleanup_error:
                if published is None:
                    if body_error is not None:
                        body_error._cleanup_failure = cleanup_error
                    else:
                        raise


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (BuildFailure, OSError, subprocess.SubprocessError) as exc:
        report_failure(exc)
        raise SystemExit(1)
PY
