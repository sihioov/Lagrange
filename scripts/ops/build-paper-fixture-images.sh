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
COMMIT_RE = re.compile(r"[0-9a-f]{40}\Z")
STATE_RE = re.compile(r"lagrange-paper-fixture-build-[a-z0-9][a-z0-9-]{7,63}\Z")
NAME_RE = re.compile(r"[a-z0-9][a-z0-9_.-]{0,127}\Z")
HELPER_OPS = frozenset({
    "PLAN", "MAKE_CONTEXT", "PRODUCE_NATIVE", "PRODUCE_BIN",
    "BUNDLE_CREATE", "BUNDLE_VERIFY", "WRITE_OVERRIDE", "PREPARE_NONE",
    "COMPOSE_BUILD", "VERIFY_IMAGE", "PUBLISH_TEMP", "QUIT",
})
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
        for name, cleanup in reversed(self.items):
            try:
                cleanup()
            except Exception:
                errors.append(name)
        self.items.clear()
        if errors:
            raise CleanupFailure("cleanup-failed:" + ",".join(errors))


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


class HelperServer:
    def __init__(self, source: pathlib.Path, commit: str, state: pathlib.Path,
                 env_file: pathlib.Path, namespace: str, builder: str,
                 env: dict[str, str], deadline_ns: int, cleanup_deadline_ns: int):
        self.deadline_ns = deadline_ns
        self.cleanup_deadline_ns = cleanup_deadline_ns
        errfd = os.open(state / "logs/helper-server.stderr.log",
                        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
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

    def request(self, op: str, args: tuple[str, ...] = (), *, monitor=None,
                label=None, cutoff_ns: int | None = None) -> str:
        if op not in HELPER_OPS or len(args) > 4 or self.proc.stdin is None:
            raise BuildFailure("helper-request-invalid")
        line = " ".join([op, *(b64_field(arg) for arg in args)]).rstrip() + "\n"
        try:
            self.proc.stdin.write(line.encode("ascii"))
            self.proc.stdin.flush()
        except (BrokenPipeError, OSError) as exc:
            raise BuildFailure("helper-server-input-failed") from exc
        response = self.readline(cutoff_ns or self.deadline_ns, monitor, label or op.lower())
        try:
            kind, encoded = response.decode("ascii").split(" ", 1)
            result = base64.b64decode(encoded, validate=True).decode("utf-8")
        except (UnicodeError, ValueError) as exc:
            raise BuildFailure("helper-response-invalid") from exc
        if kind == "ERR":
            raise BuildFailure("helper-action-failed:" + result)
        if kind != "OK":
            raise BuildFailure("helper-response-kind-invalid")
        return result

    def close(self):
        if self.proc.poll() is None:
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


def read_regular_bytes(path: pathlib.Path, label: str, root: pathlib.Path) -> tuple[bytes, int]:
    assert_safe_ancestors(path, root)
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    except OSError as exc:
        raise BuildFailure(label + "-unavailable") from exc
    try:
        info = os.fstat(fd)
        if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid()
                or info.st_size > 16 * 1024 * 1024):
            raise BuildFailure(label + "-file-identity-invalid")
        with os.fdopen(fd, "rb") as handle:
            fd = -1
            data = handle.read()
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
                 guard: Callable[[str], None], commit: str, state: pathlib.Path):
    if tuple(item.service for item in plan) != SERVICES:
        raise BuildFailure("execution-plan-not-exact-v3")
    images = {}

    def call(label, fn, *args):
        guard("before:" + label)
        result = fn(*args)
        guard("after:" + label)
        return result

    for index, item in enumerate(plan):
        service = item.service
        if item.recipe == "NONE":
            if service not in NONE_SERVICES or item.bins:
                raise BuildFailure("none-service-outside-fixed-set")
            if call(service + ":prepare-none", backend.prepare_none,
                    service, commit, state) != "NONE":
                raise BuildFailure("none-prepare-result-invalid")
            override = "NONE"
        else:
            context = call(service + ":context-pending", backend.make_context,
                           item.recipe, state)
            call(service + ":native", backend.produce_native, context, state)
            context = call(service + ":context-measured", backend.make_context,
                           item.recipe, state)
            for bin_name in item.bins:
                call(service + ":producer:" + bin_name, backend.produce_bin,
                     item.recipe, bin_name, context, state)
            bundle = call(service + ":bundle-create", backend.bundle_create,
                          item.recipe, state)
            call(service + ":bundle-verify", backend.bundle_verify, bundle, item.recipe)
            override_path = state / "overrides" / f"{service}-{index:02d}.json"
            call(service + ":override", backend.write_override,
                 service, bundle, override_path)
            override = str(override_path)
        call(service + ":compose-build", backend.compose_build, service, override)
        image = call(service + ":image-verify", backend.verify_image,
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
                  state: pathlib.Path, candidate: pathlib.Path):
    images = execute_plan(plan, backend, guard, commit, state)
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
        return self.call("PRODUCE_BIN", recipe, bin_name, context)
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


def failure_diagnostic(error):
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
    return {"kind": kind, "stage": stage, "reason": reason,
            "line": frames[-1][1] + 3 if frames else 0,
            "caller_line": frames[-2][1] + 3 if len(frames) > 1 else 0,
            "errno": number if type(number) is int and 0 < number < 4096 else 0}


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
                              builder, env, deadline, cleanup_deadline)
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
            args.commit, state, candidate,
        )
        monitor.check("before-producer-evidence", helper.proc.pid)
        receipt_evidence = producer_receipt_evidence(state, plan, args.commit)
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
    finally:
        if not cleanup.closed:
            try:
                cleanup.close()
            except Exception:
                if published is None:
                    raise


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (BuildFailure, OSError, subprocess.SubprocessError) as exc:
        report_failure(exc)
        raise SystemExit(1)
PY
