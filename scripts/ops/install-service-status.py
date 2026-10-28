#!/usr/bin/env python3
"""Plan, preflight, check, or explicitly install the service-status observer."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import stat
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Callable


SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
ROOT_DIR = os.path.dirname(os.path.dirname(SCRIPT_DIR))


def _load(name: str, path: str) -> Any:
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(name + "-unavailable")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


policy_lib = _load("service_status_policy", os.path.join(SCRIPT_DIR, "lib/service-status-policy.py"))
collector = _load("collect_service_status", os.path.join(SCRIPT_DIR, "collect-service-status.py"))


COMPOSE_KEYS = (
    "OWNER_INTRADAY_QUOTES_MODE", "OWNER_INTRADAY_QUOTE_TRANSPORT",
    "OWNER_INTRADAY_SESSION_WINDOWS_SOURCE", "KIS_READ_COORDINATION_MODE",
    "OWNER_EQUITY_V2_RUNTIME_MODE", "OWNER_BETA_ACCESS_MODE", "OWNER_BETA_PAPER_MODE",
)
ENV_KEY_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
INT_RE = re.compile(r"^(?:0|[1-9][0-9]{0,6})$")


class InstallError(Exception):
    def __init__(self, code: str):
        super().__init__(code)
        self.code = code


@dataclass(frozen=True)
class InstallPaths:
    install_root: str = policy_lib.INSTALL_ROOT
    release_root: str = ""
    env_file: str = ""
    systemd_root: str = policy_lib.SYSTEMD_ROOT
    private_root: str = policy_lib.PRIVATE_ROOT
    public_root: str = policy_lib.PUBLIC_ROOT
    policy_file: str = policy_lib.POLICY_PATH
    history_file: str = policy_lib.HISTORY_PATH
    lock_file: str = policy_lib.LOCK_PATH
    repo_root: str = ROOT_DIR
    test_owner: tuple[int, int] | None = None
    api_gid: int = 10001


@dataclass
class RunnerResult:
    returncode: int


@dataclass(frozen=True)
class ObserverActivation:
    present: bool
    enabled: bool
    active: bool


def _owner(paths: InstallPaths) -> tuple[int, int]:
    return paths.test_owner or (0, 0)


def _read_env_file(path: str, test_owner: tuple[int, int] | None = None) -> dict[str, str]:
    try:
        data = policy_lib.read_trusted_file(path, max_bytes=1024 * 1024, test_owner=test_owner)
        text = data.decode("utf-8", "strict")
    except (policy_lib.PolicyError, UnicodeDecodeError):
        raise InstallError("CONFIG_INVALID")
    values: dict[str, str] = {}
    for original in text.splitlines():
        line = original[:-1] if original.endswith("\r") else original
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            raise InstallError("CONFIG_INVALID")
        key, value = line.split("=", 1)
        if not ENV_KEY_RE.fullmatch(key) or key in values:
            raise InstallError("CONFIG_INVALID")
        if any(mark in value for mark in ("$", "\\", "\"", "'")) or re.search(r"\s#", value):
            raise InstallError("CONFIG_INVALID")
        values[key] = value
    return values


def _positive_int(value: str, low: int, high: int) -> int:
    if not isinstance(value, str) or not INT_RE.fullmatch(value):
        raise InstallError("CONFIG_INVALID")
    number = int(value)
    if not low <= number <= high:
        raise InstallError("CONFIG_INVALID")
    return number


def _lease_seconds(values: dict[str, str], compose_path: str, release_root: str, test_owner: tuple[int, int] | None = None) -> dict[str, int]:
    try:
        compose_data = policy_lib.read_trusted_file(compose_path, expected_mode=0o644, max_bytes=1024 * 1024, test_owner=test_owner)
        compose_text = compose_data.decode("utf-8", "strict")
    except (policy_lib.PolicyError, UnicodeDecodeError):
        raise InstallError("CONFIG_INVALID")

    def compose_default(key: str) -> int:
        pattern = rf'^\s*{re.escape(key)}:\s*\$\{{{re.escape(key)}:-([0-9]+)\}}\s*$'
        matches = re.findall(pattern, compose_text, re.MULTILINE)
        if len(matches) != 1:
            raise InstallError("CONFIG_INVALID")
        return int(matches[0])

    def trusted_source_default(path: str, pattern: str) -> int:
        try:
            data = policy_lib.read_trusted_file(
                os.path.join(release_root, path), expected_mode=0o644,
                max_bytes=1024 * 1024, test_owner=test_owner,
            ).decode("utf-8", "strict")
        except (policy_lib.PolicyError, UnicodeDecodeError):
            raise InstallError("CONFIG_INVALID")
        matches = re.findall(pattern, data, re.MULTILINE | re.DOTALL)
        if len(matches) != 1:
            raise InstallError("CONFIG_INVALID")
        return int(matches[0])

    candidate_ms = _positive_int(values.get("CANDIDATE_LEASE_MS", str(compose_default("CANDIDATE_LEASE_MS"))), 1000, 3_600_000)
    if candidate_ms % 1000:
        raise InstallError("CONFIG_INVALID")
    beta = _positive_int(values.get("PRICE_BETA_LEASE_SECS", str(compose_default("PRICE_BETA_LEASE_SECS"))), 1, 3600)
    lease_matches = re.findall(r'^\s*OWNER_EQUITY_V2_LEASE_SECS:\s*"([0-9]+)"\s*$', compose_text, re.MULTILINE)
    if len(lease_matches) != 1:
        raise InstallError("CONFIG_INVALID")
    v2 = _positive_int(lease_matches[0], 1, 3600)
    recommendation = trusted_source_default(
        "crates/job-queue/src/bin/recommendation-runner.rs",
        r"impl\s+Default\s+for\s+Args\s*\{.*?lease:\s*Duration::from_secs\(([0-9]+)\)",
    )
    backtest = trusted_source_default(
        "crates/job-queue/src/queue.rs",
        r"impl\s+Default\s+for\s+QueueConfig\s*\{.*?QueueConfig\s*\{\s*lease:\s*Duration::from_secs\(([0-9]+)\)",
    )
    return {
        "owner_equity_v2": v2,
        "owner_beta_price_recommendation": beta,
        "candidate_compute": candidate_ms // 1000,
        "recommendation": _positive_int(str(recommendation), 1, 3600),
        "backtest": _positive_int(str(backtest), 1, 3600),
    }


def _validate_effective_policy(values: dict[str, str], research_mode: str, *, maintenance_off: bool = False) -> None:
    # A compatible rollback release may disable quote production while keeping
    # Owner access, the configured transport and all observer bindings intact.
    # The v1 receipt attests those bindings, not live quote availability; the API
    # still requires independent window, grant and receipt evidence for that.
    if values.get("OWNER_INTRADAY_QUOTES_MODE") not in ("off", "owner_only"):
        raise InstallError("CONFIG_INVALID")
    if maintenance_off:
        # The v1 owner_access field governs observer visibility, not permission
        # to admit application work. Explicit OFF maintenance keeps that view
        # owner-only while all four application admission/input modes stay OFF.
        off = {"OWNER_INTRADAY_QUOTES_MODE": "off", "OWNER_BETA_ACCESS_MODE": "disabled",
               "OWNER_BETA_PRICE_INPUT_MODE": "disabled", "OWNER_BETA_DAILY_MODE": "disabled",
               "OWNER_BETA_EQUITY_SIGNALS_MODE": "disabled"}
        if any(values.get(key) != value for key, value in off.items()):
            raise InstallError("CONFIG_INVALID")
    expected = {
        "OWNER_INTRADAY_QUOTE_TRANSPORT": "market_ws",
        "OWNER_INTRADAY_SESSION_WINDOWS_SOURCE": "operational_v1",
        "KIS_READ_COORDINATION_MODE": "shared_required",
        "OWNER_EQUITY_V2_RUNTIME_MODE": "owner_only",
        "OWNER_BETA_ACCESS_MODE": "disabled" if maintenance_off else "owner_only",
        "OWNER_BETA_PAPER_MODE": "disabled",
    }
    for key, value in expected.items():
        if values.get(key) != value:
            raise InstallError("CONFIG_INVALID")
    if research_mode not in ("daily", "daemon"):
        raise InstallError("POLICY_INVALID")


def make_policy(
    *,
    release_commit: str,
    compose_project: str,
    research_mode: str,
    installed_at: datetime,
    leases: dict[str, int],
) -> dict[str, Any]:
    policy = {
        "schema_version": "service-status.policy.v1",
        "profile": "owner_daily" if research_mode == "daily" else "owner_daemon",
        "release_commit": release_commit,
        "compose_project": compose_project,
        "owner_access": "owner_only",
        "quote_transport": "market_ws",
        "window_source": "operational_v1",
        "research_mode": research_mode,
        "policy_installed_at": policy_lib.format_time(installed_at),
        "job_lease_seconds": leases,
    }
    try:
        return policy_lib.validate_policy(policy)
    except policy_lib.PolicyError:
        raise InstallError("POLICY_INVALID")


def _run_check_script(paths: InstallPaths, commit: str, runner: Callable[..., RunnerResult]) -> None:
    checker = os.path.join(paths.release_root, "scripts/ops/install-kis-daily.sh")
    try:
        service = policy_lib.read_trusted_file(
            os.path.join(paths.systemd_root, f"lagrange-kis-daily-{commit[:7]}.service"),
            expected_mode=0o644, max_bytes=64 * 1024, test_owner=paths.test_owner,
        ).decode("utf-8", "strict")
        flags = policy_lib.daily_runner_flags(
            service, os.path.join(paths.release_root, "scripts/ops/kis-daily-production.sh")
        )
    except (policy_lib.PolicyError, UnicodeDecodeError):
        raise InstallError("POLICY_MISMATCH") from None
    argv = (
        "/bin/bash", checker, "--check", "--release-root", paths.release_root,
        "--code-commit", commit, "--production-env-file", paths.env_file,
        "--systemd-dir", paths.systemd_root, *flags,
    )
    try:
        result = runner(argv, timeout=5.0, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except Exception:
        raise InstallError("POLICY_MISMATCH")
    if getattr(result, "returncode", 1) != 0:
        raise InstallError("POLICY_MISMATCH")


def _read_systemd_timer(paths: InstallPaths, timer_name: str, runner: Callable[..., RunnerResult]) -> dict[str, str]:
    argv = collector.systemctl_argv(collector.CollectorPaths(systemd_root=paths.systemd_root), timer_name)
    try:
        result = runner(argv, timeout=2.0, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    except Exception:
        raise InstallError("POLICY_MISMATCH")
    if getattr(result, "returncode", 1) != 0:
        raise InstallError("POLICY_MISMATCH")
    stdout = getattr(result, "stdout", b"")
    if not isinstance(stdout, bytes) or len(stdout) > collector.MAX_COMMAND_BYTES:
        raise InstallError("POLICY_MISMATCH")
    try:
        properties = (
            collector.SYSTEMD_TIMER_PROPERTIES
            if timer_name.endswith(".timer")
            else collector.SYSTEMD_SERVICE_PROPERTIES
        )
        fields = collector._parse_key_values(stdout, properties)
    except collector.CollectError:
        raise InstallError("POLICY_MISMATCH")
    return fields


def _validate_daily_unit_binding(
    paths: InstallPaths,
    commit: str,
    runner: Callable[..., RunnerResult],
) -> None:
    timer_name = f"lagrange-kis-daily-{commit[:7]}.timer"
    fields = _read_systemd_timer(paths, timer_name, runner)
    if fields.get("LoadState") == "not-found":
        raise InstallError("TIMER_MISSING")
    service_name = timer_name[:-6] + ".service"
    service_fields = _read_systemd_timer(paths, service_name, runner)
    try:
        load = collector._normalize_load(fields["LoadState"])
    except (KeyError, collector.CollectError):
        raise InstallError("POLICY_MISMATCH")
    fragment = os.path.join(paths.systemd_root, timer_name)
    service_fragment = os.path.join(paths.systemd_root, service_name)
    if (
        load != "loaded" or fields.get("FragmentPath") != fragment or fields.get("DropInPaths", "").strip()
        or fields.get("Unit") != service_name
        or service_fields.get("LoadState") != "loaded"
        or service_fields.get("FragmentPath") != service_fragment
        or service_fields.get("DropInPaths", "").strip()
    ):
        raise InstallError("POLICY_MISMATCH")


def _validate_layout(paths: InstallPaths, commit: str) -> str:
    try:
        return policy_lib.validate_current_release(paths.install_root, paths.release_root, commit, test_owner=paths.test_owner)
    except policy_lib.PolicyError:
        raise InstallError("POLICY_MISMATCH")


def _normalize_release_root(install_root: str, provided: str) -> tuple[str, str]:
    if not os.path.isabs(provided) or os.path.normpath(provided) != provided:
        raise InstallError("POLICY_MISMATCH")
    if provided == os.path.join(install_root, "current"):
        try:
            target = os.readlink(provided)
        except OSError:
            raise InstallError("POLICY_MISMATCH")
        match = re.fullmatch(r"releases/([0-9a-f]{40})", target)
        if not match:
            raise InstallError("POLICY_MISMATCH")
        return os.path.join(install_root, target), match.group(1)
    commit = os.path.basename(provided)
    if not policy_lib.COMMIT_RE.fullmatch(commit):
        raise InstallError("POLICY_MISMATCH")
    return provided, commit


def _unit_hashes(paths: InstallPaths, commit: str) -> tuple[str, str]:
    try:
        return policy_lib.validate_daily_unit_files(paths.systemd_root, paths.release_root, commit, test_owner=paths.test_owner)
    except policy_lib.PolicyError:
        raise InstallError("POLICY_MISMATCH")


def _trusted_existing(
    path: str, mode: int, max_bytes: int, test_owner: tuple[int, int] | None,
    parent_mode: int | None = None,
) -> bytes | None:
    try:
        return policy_lib.read_trusted_file(
            path, expected_mode=mode, max_bytes=max_bytes, test_owner=test_owner, parent_mode=parent_mode
        )
    except policy_lib.PolicyError as error:
        if error.code == "POLICY_MISSING":
            return None
        raise InstallError("POLICY_INVALID")


def _json_bytes(value: Any, maximum: int) -> bytes:
    raw = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True, allow_nan=False).encode("ascii") + b"\n"
    if len(raw) > maximum:
        raise InstallError("POLICY_INVALID")
    return raw


def _ensure_dir(path: str, *, owner: tuple[int, int], mode: int, test_owner: tuple[int, int] | None) -> None:
    """Create only fixed installer directories and verify every existing component."""
    if not os.path.isabs(path) or os.path.normpath(path) != path:
        raise InstallError("FS_INVALID")
    parts = path.split("/")[1:]
    current = ""
    for part in parts:
        current += "/" + part
        try:
            info = os.lstat(current)
        except FileNotFoundError:
            try:
                os.mkdir(current, mode if current == path else 0o755)
                info = os.lstat(current)
                if (info.st_uid, info.st_gid) != owner:
                    if not (test_owner is not None and owner == (os.geteuid(), os.getegid())):
                        os.chown(current, owner[0], owner[1])
                os.chmod(current, mode if current == path else 0o755)
                info = os.lstat(current)
            except OSError:
                raise InstallError("FS_INVALID")
        except OSError:
            raise InstallError("FS_INVALID")
        if test_owner is not None and current == "/tmp":
            continue
        expected = owner if current == path else (test_owner or (0, 0))
        if (
            not stat.S_ISDIR(info.st_mode) or stat.S_ISLNK(info.st_mode) or
            (info.st_uid, info.st_gid) != expected or info.st_mode & 0o022
        ):
            raise InstallError("FS_INVALID")
    info = os.lstat(path)
    expected = owner
    if (info.st_uid, info.st_gid) != expected or stat.S_IMODE(info.st_mode) != mode:
        raise InstallError("FS_INVALID")


def _atomic(paths: InstallPaths, path: str, data: bytes, uid: int, gid: int, mode: int, max_bytes: int, parent_mode: int) -> None:
    try:
        policy_lib.atomic_write(
            path, data, expected_uid=uid, expected_gid=gid, mode=mode, max_bytes=max_bytes,
            parent_mode=parent_mode, test_owner=paths.test_owner,
        )
    except policy_lib.PolicyError:
        raise InstallError("FS_INVALID")


def _commands_for_apply(
    paths: InstallPaths,
    runner: Callable[..., RunnerResult],
    *,
    before_activation: Callable[[], None],
) -> None:
    commands = (
        ("/usr/bin/systemctl", "daemon-reload"),
        ("/usr/bin/systemctl", "enable", "--now", "lagrange-service-status.timer"),
    )
    for argv in commands:
        if argv[1] == "enable":
            before_activation()
        try:
            result = runner(argv, timeout=10.0, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except Exception:
            raise InstallError("APPLY_FAILED")
        if getattr(result, "returncode", 1) != 0:
            raise InstallError("APPLY_FAILED")


def _capture_observer_activation(
    paths: InstallPaths,
    runner: Callable[..., RunnerResult],
    *,
    service_present: bool,
    timer_present: bool,
) -> dict[str, ObserverActivation]:
    states: dict[str, ObserverActivation] = {}
    for unit, present in (
        ("lagrange-service-status.timer", timer_present),
        ("lagrange-service-status.service", service_present),
    ):
        fields = _read_systemd_timer(paths, unit, runner)
        try:
            load = collector._normalize_load(fields.get("LoadState", ""))
            active = collector._normalize_active(fields.get("ActiveState", ""))
        except collector.CollectError:
            raise InstallError("ACTIVATION_STATE_UNAVAILABLE")
        enabled_raw = fields.get("UnitFileState", "")
        if active not in {"active", "inactive"}:
            raise InstallError("ACTIVATION_STATE_UNAVAILABLE")
        if load == "not-found":
            if active != "inactive" or enabled_raw not in {"", "not-found", "disabled", "n/a"}:
                raise InstallError("ACTIVATION_STATE_UNAVAILABLE")
            if not present:
                states[unit] = ObserverActivation(present=False, enabled=False, active=False)
                continue
            states[unit] = ObserverActivation(present=True, enabled=False, active=False)
            continue
        if load != "loaded" or not present or enabled_raw not in {"enabled", "disabled"}:
            raise InstallError("ACTIVATION_STATE_UNAVAILABLE")
        states[unit] = ObserverActivation(
            present=True, enabled=(enabled_raw == "enabled"), active=(active == "active")
        )
    return states


def _systemd_mutation(runner: Callable[..., RunnerResult], *args: str) -> bool:
    argv = ("/usr/bin/systemctl", *args)
    try:
        result = runner(argv, timeout=10.0, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except Exception:
        return False
    return getattr(result, "returncode", 1) == 0


def _rollback_apply(
    paths: InstallPaths,
    runner: Callable[..., RunnerResult],
    *,
    previous_files: tuple[tuple[str, bytes | None, int, int, int, int], ...],
    previous_activation: dict[str, ObserverActivation],
    activation_attempted: bool,
) -> bool:
    complete = True
    timer = "lagrange-service-status.timer"
    service = "lagrange-service-status.service"
    if activation_attempted:
        complete = _systemd_mutation(runner, "disable", "--now", timer) and complete
        complete = _systemd_mutation(runner, "stop", service) and complete

    for path, old, mode, uid, gid, parent_mode in previous_files:
        try:
            if old is None:
                try:
                    os.unlink(path)
                except FileNotFoundError:
                    pass
            else:
                _atomic(paths, path, old, uid, gid, mode, 64 * 1024, parent_mode)
        except (OSError, InstallError):
            complete = False

    complete = _systemd_mutation(runner, "daemon-reload") and complete
    for unit in (timer, service):
        state = previous_activation[unit]
        if not state.present:
            continue
        enable_command = "enable" if state.enabled else "disable"
        active_command = "start" if state.active else "stop"
        complete = _systemd_mutation(runner, enable_command, unit) and complete
        complete = _systemd_mutation(runner, active_command, unit) and complete
    return complete


def _create_lock(paths: InstallPaths) -> None:
    flags = os.O_RDWR | os.O_CREAT | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    created = False
    try:
        fd = os.open(paths.lock_file, flags | os.O_EXCL, 0o600)
        created = True
    except FileExistsError:
        try:
            fd = os.open(paths.lock_file, flags & ~os.O_CREAT)
        except OSError:
            raise InstallError("FS_INVALID")
    except OSError:
        raise InstallError("FS_INVALID")
    try:
        owner = _owner(paths)
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
            raise InstallError("FS_INVALID")
        if created:
            if (info.st_uid, info.st_gid) != owner:
                if not (paths.test_owner is not None and owner == (os.geteuid(), os.getegid())):
                    os.fchown(fd, *owner)
            os.fchmod(fd, 0o600)
        elif (info.st_uid, info.st_gid) != owner or stat.S_IMODE(info.st_mode) != 0o600:
            raise InstallError("FS_INVALID")
        check = os.fstat(fd)
        if (check.st_uid, check.st_gid) != owner or stat.S_IMODE(check.st_mode) != 0o600:
            raise InstallError("FS_INVALID")
    finally:
        os.close(fd)


def _verify_installed(paths: InstallPaths, policy: dict[str, Any], manifest_hash: str) -> dict[str, Any]:
    try:
        raw = policy_lib.read_trusted_file(paths.policy_file, test_owner=paths.test_owner, parent_mode=0o700)
        saved = policy_lib.validate_policy(policy_lib.strict_json_loads(raw))
        if saved != policy:
            raise InstallError("POLICY_MISMATCH")
        history_raw = policy_lib.read_trusted_file(paths.history_file, test_owner=paths.test_owner, parent_mode=0o700)
        history = collector.validate_history(policy_lib.strict_json_loads(history_raw, collector.MAX_COMMAND_BYTES), policy, manifest_hash)
    except (policy_lib.PolicyError, collector.CollectError):
        raise InstallError("POLICY_MISMATCH")
    try:
        service_hash, timer_hash = policy_lib.validate_daily_unit_files(
            paths.systemd_root, paths.release_root, policy["release_commit"], test_owner=paths.test_owner
        )
    except policy_lib.PolicyError:
        raise InstallError("POLICY_MISMATCH")
    if history["binding"]["daily_service_sha256"] != service_hash or history["binding"]["daily_timer_sha256"] != timer_hash:
        raise InstallError("POLICY_MISMATCH")
    for filename in ("lagrange-service-status.service", "lagrange-service-status.timer"):
        source = os.path.join(paths.release_root, "deploy/systemd", filename)
        target = os.path.join(paths.systemd_root, filename)
        try:
            expected = policy_lib.read_trusted_file(
                source, expected_mode=0o644, max_bytes=64 * 1024, test_owner=paths.test_owner
            )
            actual = policy_lib.read_trusted_file(target, expected_mode=0o644, max_bytes=64 * 1024, test_owner=paths.test_owner)
        except policy_lib.PolicyError:
            raise InstallError("POLICY_MISMATCH")
        if actual != expected:
            raise InstallError("POLICY_MISMATCH")
    return history


def install_service_status(
    paths: InstallPaths,
    *,
    research_mode: str,
    compose_project: str,
    now: datetime | None = None,
    runner: Callable[..., RunnerResult] = subprocess.run,
    apply: bool = False,
    maintenance_off: bool = False,
) -> dict[str, Any]:
    """Run the complete read-only preflight, then optionally apply through runner."""
    if paths.test_owner is None and os.geteuid() != 0:
        raise InstallError("DENIED")
    if not policy_lib.PROJECT_RE.fullmatch(compose_project):
        raise InstallError("POLICY_INVALID")
    release_root, commit = _normalize_release_root(paths.install_root, paths.release_root)
    actual_paths = InstallPaths(**{**paths.__dict__, "release_root": release_root})
    if not actual_paths.env_file:
        actual_paths = InstallPaths(**{**actual_paths.__dict__, "env_file": os.path.join(release_root, "deploy/compose/.env")})
    manifest_hash = _validate_layout(actual_paths, commit)
    values = _read_env_file(actual_paths.env_file, actual_paths.test_owner)
    if values.get("LAGRANGE_CODE_COMMIT") != commit:
        raise InstallError("POLICY_MISMATCH")
    _validate_effective_policy(values, research_mode, maintenance_off=maintenance_off)
    compose_path = os.path.join(actual_paths.release_root, "deploy/compose/compose.yml")
    leases = _lease_seconds(values, compose_path, actual_paths.release_root, actual_paths.test_owner)
    _run_check_script(actual_paths, commit, runner)
    _validate_daily_unit_binding(actual_paths, commit, runner)
    service_hash, timer_hash = _unit_hashes(actual_paths, commit)
    installed_at = (now or datetime.now(timezone.utc)).astimezone(timezone.utc)
    policy = make_policy(
        release_commit=commit, compose_project=compose_project, research_mode=research_mode,
        installed_at=installed_at, leases=leases,
    )
    history = collector.empty_history(policy, manifest_hash, service_hash, timer_hash)
    if not apply:
        return {"policy": policy, "history": history, "manifest_sha256": manifest_hash}

    owner = _owner(actual_paths)
    api_owner = (owner[0], actual_paths.api_gid) if actual_paths.test_owner is None else actual_paths.test_owner
    _ensure_dir(os.path.dirname(actual_paths.private_root), owner=owner, mode=0o755, test_owner=actual_paths.test_owner)
    _ensure_dir(actual_paths.private_root, owner=owner, mode=0o700, test_owner=actual_paths.test_owner)
    _ensure_dir(actual_paths.public_root, owner=api_owner, mode=0o750, test_owner=actual_paths.test_owner)
    _ensure_dir(actual_paths.systemd_root, owner=owner, mode=0o755, test_owner=actual_paths.test_owner)
    old_policy = _trusted_existing(actual_paths.policy_file, 0o600, policy_lib.MAX_POLICY_BYTES, actual_paths.test_owner, 0o700)
    old_history = _trusted_existing(actual_paths.history_file, 0o600, collector.MAX_COMMAND_BYTES, actual_paths.test_owner, 0o700)
    status_service_path = os.path.join(actual_paths.systemd_root, "lagrange-service-status.service")
    status_timer_path = os.path.join(actual_paths.systemd_root, "lagrange-service-status.timer")
    old_service = _trusted_existing(status_service_path, 0o644, 64 * 1024, actual_paths.test_owner)
    old_timer = _trusted_existing(status_timer_path, 0o644, 64 * 1024, actual_paths.test_owner)
    previous_activation = _capture_observer_activation(
        actual_paths, runner, service_present=(old_service is not None), timer_present=(old_timer is not None)
    )
    activation_attempted = False

    def mark_activation_attempted() -> None:
        nonlocal activation_attempted
        activation_attempted = True

    created: list[str] = []
    try:
        for filename in ("lagrange-service-status.service", "lagrange-service-status.timer"):
            source = os.path.join(actual_paths.release_root, "deploy/systemd", filename)
            try:
                data = policy_lib.read_trusted_file(
                    source, expected_mode=0o644, max_bytes=64 * 1024, test_owner=actual_paths.test_owner
                )
            except policy_lib.PolicyError:
                raise InstallError("FS_INVALID")
            target = os.path.join(actual_paths.systemd_root, filename)
            _atomic(actual_paths, target, data, owner[0], owner[1], 0o644, 64 * 1024, 0o755)
            created.append(target)
        _atomic(actual_paths, actual_paths.policy_file, _json_bytes(policy, policy_lib.MAX_POLICY_BYTES), owner[0], owner[1], 0o600, policy_lib.MAX_POLICY_BYTES, 0o700)
        _atomic(actual_paths, actual_paths.history_file, _json_bytes(history, collector.MAX_COMMAND_BYTES), owner[0], owner[1], 0o600, collector.MAX_COMMAND_BYTES, 0o700)
        _commands_for_apply(actual_paths, runner, before_activation=mark_activation_attempted)
    except Exception as original:
        previous_files = (
            (actual_paths.policy_file, old_policy, 0o600, owner[0], owner[1], 0o700),
            (actual_paths.history_file, old_history, 0o600, owner[0], owner[1], 0o700),
            (status_service_path, old_service, 0o644, owner[0], owner[1], 0o755),
            (status_timer_path, old_timer, 0o644, owner[0], owner[1], 0o755),
        )
        if not _rollback_apply(
            actual_paths, runner, previous_files=previous_files,
            previous_activation=previous_activation, activation_attempted=activation_attempted,
        ):
            raise InstallError("ROLLBACK_INCOMPLETE") from original
        raise
    return {"policy": policy, "history": history, "manifest_sha256": manifest_hash}


def _default_runner(argv: tuple[str, ...], **kwargs: Any) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(argv, timeout=kwargs.get("timeout"), stdout=kwargs.get("stdout"), stderr=kwargs.get("stderr"), check=False, close_fds=True, env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LC_ALL": "C", "TZ": "UTC"})


def _cli() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--plan", action="store_true")
    modes.add_argument("--preflight", action="store_true")
    modes.add_argument("--check", action="store_true")
    modes.add_argument("--apply", action="store_true")
    parser.add_argument("--maintenance-off", action="store_true",
        help="Bind owner-only observer visibility for an explicitly all-OFF maintenance release; never enables application collection or admission")
    parser.add_argument("--install-root", default=policy_lib.INSTALL_ROOT)
    parser.add_argument("--release-root", default=os.path.join(policy_lib.INSTALL_ROOT, "current"))
    parser.add_argument("--compose-project", required=True)
    parser.add_argument("--research-mode", choices=("daily", "daemon"), required=True)
    parser.add_argument("--systemd-dir", default=policy_lib.SYSTEMD_ROOT)
    parser.add_argument("--env-file", default="")
    args = parser.parse_args()
    apply = bool(args.apply)
    if args.plan or (not args.preflight and not args.check and not args.apply):
        print("SERVICE_STATUS_PLAN mode=" + args.research_mode + " compose_project=" + args.compose_project)
        print("  apply installs the root-owned receipt, safe snapshot directory, fixed units, and enables only the status timer")
        print("PLAN_ONLY: no host file write, systemd mutation, Docker or provider call")
        return 0
    try:
        paths = InstallPaths(
            install_root=args.install_root,
            release_root=args.release_root,
            env_file=args.env_file,
            systemd_root=args.systemd_dir,
        )
        resolved, _ = _normalize_release_root(paths.install_root, paths.release_root)
        paths = InstallPaths(**{**paths.__dict__, "release_root": resolved})
        result = install_service_status(
            paths, research_mode=args.research_mode, compose_project=args.compose_project,
            runner=_default_runner, apply=apply, maintenance_off=args.maintenance_off,
        )
        if args.check:
            try:
                saved = policy_lib.validate_policy(policy_lib.strict_json_loads(
                    policy_lib.read_trusted_file(paths.policy_file, parent_mode=0o700)
                ))
            except policy_lib.PolicyError:
                raise InstallError("POLICY_MISMATCH")
            requested = dict(result["policy"])
            requested["policy_installed_at"] = saved["policy_installed_at"]
            if requested != saved:
                raise InstallError("POLICY_MISMATCH")
            _verify_installed(paths, saved, result["manifest_sha256"])
            print("SERVICE_STATUS_CHECK: PASS")
        elif apply:
            print("SERVICE_STATUS_APPLY: PASS")
        else:
            print("SERVICE_STATUS_PREFLIGHT: PASS (read-only)")
        return 0
    except InstallError as error:
        print("SERVICE_STATUS_ERROR " + error.code, file=sys.stderr)
        return 1
    except Exception:
        print("SERVICE_STATUS_ERROR invalid", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(_cli())
