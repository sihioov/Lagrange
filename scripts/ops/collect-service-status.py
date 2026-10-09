#!/usr/bin/env python3
"""One-shot, fixed-field host observer for the Owner service-status API."""

from __future__ import annotations

import fcntl
import importlib.util
import json
import os
import re
import selectors
import signal
import stat
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import Any, Callable


SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location(
    "service_status_policy", os.path.join(SCRIPT_DIR, "lib/service-status-policy.py")
)
if _spec is None or _spec.loader is None:
    raise RuntimeError("service-status-policy-unavailable")
policy_lib = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(policy_lib)


MAX_COMMAND_BYTES = 16 * 1024
MAX_SNAPSHOT_BYTES = 256 * 1024
COMMAND_TIMEOUT_SECONDS = 2.0
TOTAL_DEADLINE_SECONDS = 20.0
PUBLIC_FILE = "snapshot.json"
STATE_ENUM = {
    "running", "created", "restarting", "paused", "exited", "dead", "removing",
    "active", "activating", "inactive", "failed", "deactivating", "missing", "unknown",
}
HEALTH_ENUM = {"healthy", "unhealthy", "starting", "none", "unknown"}
RESULT_ENUM = {"ok", "missing", "timeout", "command_unavailable", "denied", "invalid", "deadline"}
SYSTEMD_COMMON_PROPERTIES = (
    "LoadState", "ActiveState", "SubState", "UnitFileState", "Result", "FragmentPath", "DropInPaths",
)
SYSTEMD_SERVICE_PROPERTIES = SYSTEMD_COMMON_PROPERTIES + (
    "ExecMainCode", "ExecMainStatus", "ExecMainStartTimestamp", "ExecMainExitTimestamp",
)
SYSTEMD_TIMER_PROPERTIES = SYSTEMD_COMMON_PROPERTIES + (
    "LastTriggerUSec", "NextElapseUSecRealtime", "Unit",
)
# systemctl is called with this fixed union; each result is parsed against its
# concrete unit kind below because systemd omits inapplicable properties.
SYSTEMD_PROPERTIES = SYSTEMD_COMMON_PROPERTIES + (
    "ExecMainCode", "ExecMainStatus", "ExecMainStartTimestamp", "ExecMainExitTimestamp",
    "LastTriggerUSec", "NextElapseUSecRealtime", "Unit",
)
DOCKER_IDS = (
    "postgres", "reverse-proxy", "web", "api-server", "owner-equity-v2-runner",
    "owner-beta-runner", "candidate-runner", "recommendation-runner",
    "nt-backtest-worker-1", "nt-backtest-worker-2", "research-worker",
    "paper-scheduler", "metrics", "live-node-owner",
)
SYSTEMD_HOST_IDS = ("docker", "tailscaled")
SCHEDULES = {
    "production-backup": ("lagrange-production-backup.timer", "lagrange-production-backup.service"),
    "production-backup-verify": ("lagrange-production-backup-verify.timer", "lagrange-production-backup-verify.service"),
    "tls-renewal": ("lagrange-tailscale-tls-renewal.timer", "lagrange-tailscale-tls-renewal.service"),
}
HOST_IDS = (
    "postgres", "reverse-proxy", "web", "api-server", "owner-equity-v2-runner",
    "owner-beta-runner", "candidate-runner", "recommendation-runner",
    "nt-backtest-worker-1", "nt-backtest-worker-2", "research-worker", "kis-eod",
    "production-backup", "production-backup-verify", "tls-renewal", "docker",
    "tailscaled", "paper-scheduler", "metrics", "live-node-owner",
)
SYSTEMD_RESULTS_FAILURE = {
    "resources", "timeout", "exit-code", "signal", "core-dump", "watchdog",
    "start-limit-hit", "exec-condition", "protocol", "dependency", "condition",
    "assert", "oom-kill", "job-canceled", "start-limit-hit",
}
_LABEL_RE = re.compile(r"^[a-z0-9][a-z0-9_.-]{0,63}$")
_CONTAINER_ID_RE = re.compile(r"^[0-9a-f]{12,64}$")


@dataclass(frozen=True)
class CollectorPaths:
    install_root: str = policy_lib.INSTALL_ROOT
    release_root: str = policy_lib.INSTALL_ROOT + "/current"
    private_root: str = policy_lib.PRIVATE_ROOT
    public_root: str = policy_lib.PUBLIC_ROOT
    policy_file: str = policy_lib.POLICY_PATH
    history_file: str = policy_lib.HISTORY_PATH
    lock_file: str = policy_lib.LOCK_PATH
    systemd_root: str = policy_lib.SYSTEMD_ROOT
    proc_stat: str = "/proc/stat"
    docker_bin: str = "/usr/bin/docker"
    systemctl_bin: str = "/usr/bin/systemctl"
    api_gid: int = 10001
    test_owner: tuple[int, int] | None = None


class CollectError(Exception):
    def __init__(self, code: str):
        super().__init__(code)
        self.code = code


@dataclass
class CommandResult:
    result: str
    stdout: bytes = b""
    returncode: int | None = None


def utc_now() -> datetime:
    return datetime.now(timezone.utc)


def canonical_time(value: datetime) -> str:
    return policy_lib.format_time(value)


def parse_timestamp(value: Any) -> datetime | None:
    if value is None or value == "" or value in ("n/a", "0", "0001-01-01T00:00:00Z"):
        return None
    if not isinstance(value, str):
        raise CollectError("invalid")
    iso_match = re.fullmatch(
        r"([0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2})(?:\.([0-9]{1,9}))?Z",
        value,
    )
    if iso_match:
        base, fraction = iso_match.groups()
        if base == "0001-01-01T00:00:00" and (fraction is None or not fraction.strip("0")):
            return None
        normalized = base
        if fraction is not None:
            normalized += "." + fraction[:6].ljust(6, "0")
        try:
            parsed = datetime.fromisoformat(normalized + "+00:00").astimezone(timezone.utc)
            if parsed == datetime.min.replace(tzinfo=timezone.utc):
                return None
            return parsed
        except ValueError:
            raise CollectError("invalid")
    # `systemctl show` uses the host-local zone with LC_ALL=C. Production is
    # configured for Seoul; admit only this fixed zone or UTC.
    match = re.fullmatch(r"[A-Z][a-z]{2} (\d{4}-\d\d-\d\d) (\d\d:\d\d:\d\d) (KST|UTC|GMT)", value)
    if not match:
        raise CollectError("invalid")
    zone = timezone(timedelta(hours=9)) if match.group(3) == "KST" else timezone.utc
    try:
        return datetime.strptime(match.group(1) + " " + match.group(2), "%Y-%m-%d %H:%M:%S").replace(tzinfo=zone).astimezone(timezone.utc)
    except ValueError:
        raise CollectError("invalid")


def parse_btime(proc_stat_path: str = "/proc/stat") -> str:
    """Read only the single btime line, rejecting duplicate/malformed records."""
    try:
        with open(proc_stat_path, "rb", buffering=0) as handle:
            found = None
            while True:
                line = handle.readline(4096)
                if not line:
                    break
                if len(line) == 4096 and not line.endswith(b"\n"):
                    raise CollectError("invalid")
                if line.startswith(b"btime "):
                    if found is not None or not re.fullmatch(rb"btime [1-9][0-9]{0,11}\n?", line):
                        raise CollectError("invalid")
                    found = int(line.split()[1])
            if found is None:
                raise CollectError("invalid")
            return canonical_time(datetime.fromtimestamp(found, timezone.utc))
    except CollectError:
        raise
    except OSError:
        raise CollectError("invalid")


def _kill_process(proc: subprocess.Popen[bytes]) -> None:
    try:
        proc.kill()
    except OSError:
        pass
    try:
        proc.wait(timeout=0.2)
    except (OSError, subprocess.TimeoutExpired):
        pass


def run_fixed_command(
    argv: tuple[str, ...],
    *,
    deadline: float,
    timeout: float = COMMAND_TIMEOUT_SECONDS,
    max_output: int = MAX_COMMAND_BYTES,
) -> CommandResult:
    """Run fixed argv with bounded stdout and discard stderr completely."""
    if not argv or argv[0] not in ("/usr/bin/docker", "/usr/bin/systemctl"):
        return CommandResult("invalid")
    remaining = min(timeout, deadline - time.monotonic())
    if remaining <= 0:
        return CommandResult("deadline")
    try:
        proc = subprocess.Popen(
            argv,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            close_fds=True,
            env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LC_ALL": "C", "TZ": "UTC"},
        )
    except FileNotFoundError:
        return CommandResult("command_unavailable")
    except PermissionError:
        return CommandResult("denied")
    except OSError:
        return CommandResult("invalid")
    assert proc.stdout is not None
    selector = selectors.DefaultSelector()
    selector.register(proc.stdout, selectors.EVENT_READ)
    output = bytearray()
    end = time.monotonic() + remaining
    try:
        while True:
            remaining = end - time.monotonic()
            if remaining <= 0:
                _kill_process(proc)
                return CommandResult("timeout")
            events = selector.select(remaining)
            if not events:
                if proc.poll() is not None:
                    break
                _kill_process(proc)
                return CommandResult("timeout")
            for key, _ in events:
                try:
                    chunk = os.read(key.fd, min(4096, max_output + 1 - len(output)))
                except OSError:
                    _kill_process(proc)
                    return CommandResult("invalid")
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                output.extend(chunk)
                if len(output) > max_output:
                    _kill_process(proc)
                    return CommandResult("invalid")
            if not selector.get_map():
                break
        try:
            rc = proc.wait(timeout=max(0.01, end - time.monotonic()))
        except subprocess.TimeoutExpired:
            _kill_process(proc)
            return CommandResult("timeout")
        return CommandResult("ok" if rc == 0 else "invalid", bytes(output), rc)
    finally:
        selector.close()
        proc.stdout.close()


def call_runner(runner: Callable[..., CommandResult], argv: tuple[str, ...], deadline: float) -> CommandResult:
    try:
        result = runner(argv, deadline=deadline)
    except FileNotFoundError:
        return CommandResult("command_unavailable")
    except PermissionError:
        return CommandResult("denied")
    except TimeoutError:
        return CommandResult("timeout")
    except Exception:
        return CommandResult("invalid")
    if not isinstance(result, CommandResult) or result.result not in RESULT_ENUM or len(result.stdout) > MAX_COMMAND_BYTES:
        return CommandResult("invalid")
    return result


def failed_observation(service_id: str, result: str) -> dict[str, Any]:
    if result not in RESULT_ENUM:
        result = "invalid"
    return {"id": service_id, "result": result, "observed_at": None, "process": None, "timer": None}


def _process(state: str, health: str = "none", started: datetime | None = None, finished: datetime | None = None, exit_code: int | None = None) -> dict[str, Any]:
    if state not in STATE_ENUM or health not in HEALTH_ENUM:
        raise CollectError("invalid")
    return {
        "state": state,
        "health": health,
        "started_at": canonical_time(started) if started else None,
        "finished_at": canonical_time(finished) if finished else None,
        "exit_code": exit_code,
    }


def map_docker_fields(service_id: str, observed_at: str, fields: dict[str, Any]) -> dict[str, Any]:
    """Reduce the five fixed Docker State fields; never copy labels or text through."""
    required = {"State.Status", "State.Health.Status", "State.StartedAt", "State.FinishedAt", "State.ExitCode"}
    if not isinstance(fields, dict) or set(fields) != required:
        raise CollectError("invalid")
    state = fields["State.Status"]
    if state not in {"running", "created", "restarting", "paused", "exited", "dead", "removing"}:
        raise CollectError("invalid")
    health = fields["State.Health.Status"]
    if health not in HEALTH_ENUM:
        raise CollectError("invalid")
    started = parse_timestamp(fields["State.StartedAt"])
    finished = parse_timestamp(fields["State.FinishedAt"])
    exit_code = fields["State.ExitCode"]
    if state == "running":
        finished = None
        exit_code = None
    elif exit_code is not None:
        if isinstance(exit_code, bool) or not isinstance(exit_code, int) or not 0 <= exit_code <= 255:
            raise CollectError("invalid")
    process = _process(state, health, started, finished, exit_code)
    return {"id": service_id, "result": "ok", "observed_at": observed_at, "process": process, "timer": None}


def missing_docker_observation(service_id: str, observed_at: str) -> dict[str, Any]:
    return {"id": service_id, "result": "ok", "observed_at": observed_at,
            "process": _process("missing"), "timer": None}


def _parse_key_values(raw: bytes, allowed: tuple[str, ...]) -> dict[str, str]:
    try:
        text = raw.decode("utf-8", "strict")
    except UnicodeDecodeError:
        raise CollectError("invalid")
    result: dict[str, str] = {}
    for line in text.splitlines():
        if not line:
            continue
        if "=" not in line:
            raise CollectError("invalid")
        key, value = line.split("=", 1)
        if key not in allowed or key in result or len(value) > 4096:
            raise CollectError("invalid")
        result[key] = value
    if set(allowed) != set(result):
        raise CollectError("invalid")
    return result


def docker_ps_argv(paths: CollectorPaths, project: str, service: str) -> tuple[str, ...]:
    if not policy_lib.PROJECT_RE.fullmatch(project) or service not in DOCKER_IDS:
        raise CollectError("invalid")
    fmt = '{{.ID}}\t{{.Label "com.docker.compose.project"}}\t{{.Label "com.docker.compose.service"}}\t{{.Label "com.docker.compose.oneoff"}}'
    return (
        paths.docker_bin, "ps", "--all", "--no-trunc",
        "--filter", "label=com.docker.compose.project=" + project,
        "--filter", "label=com.docker.compose.service=" + service,
        "--format", fmt,
    )


def parse_docker_ps(raw: bytes, project: str, service: str) -> list[str]:
    try:
        text = raw.decode("ascii", "strict")
    except UnicodeDecodeError:
        raise CollectError("invalid")
    result: list[str] = []
    seen: set[str] = set()
    for line in text.splitlines():
        if len(line) > 512:
            raise CollectError("invalid")
        fields = line.split("\t")
        if len(fields) != 4:
            raise CollectError("invalid")
        container_id, found_project, found_service, oneoff = fields
        if not _CONTAINER_ID_RE.fullmatch(container_id):
            raise CollectError("invalid")
        if found_project != project or found_service != service:
            # The daemon's filter is advisory. Recheck exact labels locally.
            continue
        if oneoff.lower() == "true":
            continue
        if oneoff not in ("", "False", "false", "0"):
            raise CollectError("invalid")
        if container_id in seen:
            raise CollectError("invalid")
        seen.add(container_id)
        result.append(container_id)
        if len(result) > 1:
            raise CollectError("invalid")
    return result


def docker_inspect_argv(paths: CollectorPaths, container_id: str) -> tuple[str, ...]:
    if not _CONTAINER_ID_RE.fullmatch(container_id):
        raise CollectError("invalid")
    fmt = (
        '{{.State.Status}}\t{{if .State.Health}}{{if .State.Health.Status}}{{.State.Health.Status}}{{else}}unknown{{end}}'
        '{{else}}none{{end}}\t{{.State.StartedAt}}\t{{.State.FinishedAt}}\t{{.State.ExitCode}}\t'
        '{{index .Config.Labels "com.docker.compose.project"}}\t'
        '{{index .Config.Labels "com.docker.compose.service"}}\t'
        '{{index .Config.Labels "com.docker.compose.oneoff"}}'
    )
    return (paths.docker_bin, "inspect", "--type", "container", "--format", fmt, container_id)


def parse_docker_inspect(raw: bytes, service_id: str, project: str) -> dict[str, Any]:
    try:
        text = raw.decode("ascii", "strict").rstrip("\n")
    except UnicodeDecodeError:
        raise CollectError("invalid")
    fields = text.split("\t")
    if len(fields) != 8:
        raise CollectError("invalid")
    state, health, started, finished, exit_raw, found_project, found_service, oneoff = fields
    if found_project != project or found_service != service_id or oneoff.lower() == "true":
        raise CollectError("invalid")
    if oneoff not in ("", "False", "false", "0"):
        raise CollectError("invalid")
    exit_code = None
    if state != "running":
        if not re.fullmatch(r"(?:0|[1-9][0-9]{0,2})", exit_raw):
            raise CollectError("invalid")
        exit_code = int(exit_raw)
        if exit_code > 255:
            raise CollectError("invalid")
    fields_dict = {
        "State.Status": state,
        "State.Health.Status": health,
        "State.StartedAt": started,
        "State.FinishedAt": finished,
        "State.ExitCode": exit_code,
    }
    return fields_dict


def _normalize_load(value: str) -> str:
    mapping = {"loaded": "loaded", "not-found": "not-found", "masked": "masked", "bad-setting": "error", "error": "error"}
    if value not in mapping:
        raise CollectError("invalid")
    return mapping[value]


def _normalize_active(value: str) -> str:
    if value not in {"active", "inactive", "activating", "failed", "deactivating"}:
        raise CollectError("invalid")
    return value


def _normalize_enabled(value: str) -> str:
    if value in {"enabled", "enabled-runtime"}:
        return "enabled"
    if value in {"masked", "masked-runtime"}:
        return "masked"
    if value in {"static", "indirect"}:
        return "static"
    if value in {"disabled", "generated", "transient", "linked", "linked-runtime", "alias", "", "n/a"}:
        return "disabled"
    raise CollectError("invalid")


def _systemd_result(value: str) -> str:
    if value == "success":
        return "success"
    if value in SYSTEMD_RESULTS_FAILURE:
        return "failure"
    if value in ("", "none", "n/a"):
        return "none"
    raise CollectError("invalid")


def _number(value: str, *, max_value: int = 255) -> int | None:
    if isinstance(value, int) and not isinstance(value, bool):
        if 0 <= value <= max_value:
            return value
        raise CollectError("invalid")
    if value in ("", "0", "n/a"):
        return None if value != "0" else 0
    if not re.fullmatch(r"[1-9][0-9]{0,4}", value):
        raise CollectError("invalid")
    number = int(value)
    if number > max_value:
        raise CollectError("invalid")
    return number


def _binding_for_unit(
    unit_name: str,
    fragment_path: str,
    dropins: str,
    expected_path: str,
    expected_sha256: str | None = None,
    systemd_root: str = policy_lib.SYSTEMD_ROOT,
    test_owner: tuple[int, int] | None = None,
) -> str:
    if unit_name != os.path.basename(expected_path) or fragment_path != expected_path or dropins.strip():
        return "mismatch"
    if expected_sha256 is None:
        try:
            policy_lib.read_trusted_file(expected_path, expected_mode=0o644, max_bytes=64 * 1024, test_owner=test_owner)
        except policy_lib.PolicyError:
            return "mismatch"
        return "matched"
    try:
        content = policy_lib.read_trusted_file(
            expected_path, expected_mode=0o644, max_bytes=64 * 1024, test_owner=test_owner
        )
    except policy_lib.PolicyError:
        return "mismatch"
    import hashlib
    return "matched" if hashlib.sha256(content).hexdigest() == expected_sha256 else "mismatch"


def _clear_preboot_systemd_times(
    service_fields: dict[str, str],
    timer_fields: dict[str, str] | None,
    booted_at: str,
) -> None:
    booted = policy_lib.timestamp(booted_at)
    start = parse_timestamp(service_fields.get("ExecMainStartTimestamp", ""))
    end = parse_timestamp(service_fields.get("ExecMainExitTimestamp", ""))
    if (start is not None and start < booted) or (end is not None and end < booted):
        service_fields["ExecMainStartTimestamp"] = ""
        service_fields["ExecMainExitTimestamp"] = ""
        service_fields["ExecMainCode"] = "0"
        service_fields["ExecMainStatus"] = ""
        service_fields["Result"] = "none"
    if timer_fields is not None:
        trigger = parse_timestamp(timer_fields.get("LastTriggerUSec", ""))
        if trigger is not None and trigger < booted:
            timer_fields["LastTriggerUSec"] = ""


def _clear_preboot_observation(observation: dict[str, Any], booted_at: str) -> dict[str, Any]:
    booted = policy_lib.timestamp(booted_at)
    process = observation.get("process")
    if process is not None:
        timestamps = [process.get(key) for key in ("started_at", "finished_at")]
        if any(value is not None and policy_lib.timestamp(value) < booted for value in timestamps):
            process["started_at"] = None
            process["finished_at"] = None
            process["exit_code"] = None
    timer = observation.get("timer")
    if timer is not None:
        trigger = timer.get("last_trigger_at")
        if trigger is not None and policy_lib.timestamp(trigger) < booted:
            timer["last_trigger_at"] = None
        started = timer.get("last_started_at")
        finished = timer.get("last_finished_at")
        if any(value is not None and policy_lib.timestamp(value) < booted for value in (started, finished)):
            timer["last_started_at"] = None
            timer["last_finished_at"] = None
            timer["exit_status"] = None
            timer["last_result"] = "unknown"
    return observation


def _timer_last_result(
    system_result: str,
    exec_code: int | None,
    exit_status: int | None,
    started: datetime | None,
    finished: datetime | None,
    history: dict[str, Any] | None,
) -> str:
    # A systemd condition skip has no positive process interval even when
    # systemd reports Result=success and status zero.
    if started is None or finished is None or finished < started or exec_code != 1:
        if system_result in {"success", "failure"} or exit_status in (74, 75):
            return "unknown"
        if history is not None and history.get("last_result") in {"success", "failure", "unknown"}:
            return history["last_result"]
        return "none"
    if exit_status in (74, 75):
        return "unknown"
    if system_result == "success" and exit_status == 0:
        return "success"
    return "failure"


def verified_completion(
    *,
    start: datetime | None,
    end: datetime | None,
    exec_code: int | None,
    exit_status: int | None,
    result: str,
    expected_unit: str,
    observed_unit: str,
    installed_at: datetime,
) -> bool:
    return bool(
        start is not None and end is not None and start > datetime.fromtimestamp(0, timezone.utc)
        and start >= installed_at and end >= start and end >= installed_at and exec_code == 1 and exit_status == 0
        and result == "success" and observed_unit == expected_unit
    )


def empty_history(policy: dict[str, Any], manifest_sha256: str, service_hash: str | None, timer_hash: str | None) -> dict[str, Any]:
    jobs = {}
    for service_id in ("kis-eod", "production-backup", "production-backup-verify", "tls-renewal"):
        jobs[service_id] = {
            "last_started_at": None,
            "last_finished_at": None,
            "last_result": "unknown",
            "exit_status": None,
            "last_success_at": None,
        }
    return {
        "schema_version": "service-status.history.v1",
        "installed_at": policy["policy_installed_at"],
        "binding": {
            "release_commit": policy["release_commit"],
            "compose_project": policy["compose_project"],
            "research_mode": policy["research_mode"],
            "manifest_sha256": manifest_sha256,
            "daily_service_sha256": service_hash,
            "daily_timer_sha256": timer_hash,
        },
        "jobs": jobs,
    }


def validate_history(value: Any, policy: dict[str, Any], manifest_sha256: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != {"schema_version", "installed_at", "binding", "jobs"}:
        raise CollectError("invalid")
    if value["schema_version"] != "service-status.history.v1":
        raise CollectError("invalid")
    installed_at = policy_lib.timestamp(value["installed_at"])
    binding = value["binding"]
    if not isinstance(binding, dict) or set(binding) != {
        "release_commit", "compose_project", "research_mode", "manifest_sha256",
        "daily_service_sha256", "daily_timer_sha256",
    }:
        raise CollectError("invalid")
    expected = {
        "release_commit": policy["release_commit"],
        "compose_project": policy["compose_project"],
        "research_mode": policy["research_mode"],
        "manifest_sha256": manifest_sha256,
    }
    if any(binding.get(key) != val for key, val in expected.items()):
        raise CollectError("invalid")
    for key in ("daily_service_sha256", "daily_timer_sha256"):
        if binding[key] is not None and (not isinstance(binding[key], str) or not policy_lib.SHA256_RE.fullmatch(binding[key])):
            raise CollectError("invalid")
    jobs = value["jobs"]
    expected_jobs = {"kis-eod", "production-backup", "production-backup-verify", "tls-renewal"}
    if not isinstance(jobs, dict) or set(jobs) != expected_jobs:
        raise CollectError("invalid")
    for record in jobs.values():
        if not isinstance(record, dict) or set(record) != {
            "last_started_at", "last_finished_at", "last_result", "exit_status", "last_success_at"
        }:
            raise CollectError("invalid")
        if record["last_result"] not in {"success", "failure", "none", "unknown"}:
            raise CollectError("invalid")
        parsed_times: dict[str, datetime | None] = {}
        for key in ("last_started_at", "last_finished_at", "last_success_at"):
            if record[key] is not None:
                parsed = policy_lib.timestamp(record[key])
                if parsed < installed_at:
                    raise CollectError("invalid")
                parsed_times[key] = parsed
            else:
                parsed_times[key] = None
        if (parsed_times["last_started_at"] is None) != (parsed_times["last_finished_at"] is None):
            raise CollectError("invalid")
        if parsed_times["last_started_at"] is not None and parsed_times["last_finished_at"] < parsed_times["last_started_at"]:
            raise CollectError("invalid")
        if parsed_times["last_success_at"] is not None and (
            parsed_times["last_finished_at"] is None or parsed_times["last_success_at"] > parsed_times["last_finished_at"]
        ):
            raise CollectError("invalid")
        if record["last_result"] == "success" and (
            record["exit_status"] != 0 or parsed_times["last_success_at"] != parsed_times["last_finished_at"]
        ):
            raise CollectError("invalid")
        if record["exit_status"] is not None and parsed_times["last_finished_at"] is None:
            raise CollectError("invalid")
        if record["exit_status"] is not None and (
            isinstance(record["exit_status"], bool) or not isinstance(record["exit_status"], int)
            or not 0 <= record["exit_status"] <= 255
        ):
            raise CollectError("invalid")
    return value


def update_history_record(
    history: dict[str, Any],
    service_id: str,
    *,
    start: datetime | None,
    end: datetime | None,
    last_result: str,
    exit_status: int | None,
    successful: bool,
) -> None:
    if service_id not in history["jobs"]:
        raise CollectError("invalid")
    record = history["jobs"][service_id]
    record["last_started_at"] = canonical_time(start) if start else None
    record["last_finished_at"] = canonical_time(end) if end else None
    record["last_result"] = last_result
    record["exit_status"] = exit_status
    if successful and end is not None:
        record["last_success_at"] = canonical_time(end)


def _empty_process() -> dict[str, Any]:
    return _process("unknown", "unknown")


def map_systemd_fields(
    service_id: str,
    observed_at: str,
    *,
    timer_fields: dict[str, str] | None,
    service_fields: dict[str, str] | None,
    expected_timer_name: str | None = None,
    expected_service_name: str | None = None,
    expected_timer_sha256: str | None = None,
    expected_service_sha256: str | None = None,
    verified_unit_binding: str | None = None,
    systemd_root: str = policy_lib.SYSTEMD_ROOT,
    test_owner: tuple[int, int] | None = None,
    history_record: dict[str, Any] | None = None,
    installed_at: datetime | None = None,
) -> dict[str, Any]:
    """Map only the allowlisted systemd properties into the closed host DTO."""
    if service_fields is None:
        return failed_observation(service_id, "missing")
    if not set(service_fields).issubset(set(SYSTEMD_SERVICE_PROPERTIES)):
        raise CollectError("invalid")
    if timer_fields is not None and not set(timer_fields).issubset(set(SYSTEMD_TIMER_PROPERTIES)):
        raise CollectError("invalid")
    try:
        load = _normalize_load(service_fields.get("LoadState", ""))
        if load == "not-found" and timer_fields is None:
            return failed_observation(service_id, "missing")
        active = "missing" if load == "not-found" else _normalize_active(service_fields.get("ActiveState", ""))
        result = _systemd_result(service_fields.get("Result", ""))
        code = _number(service_fields.get("ExecMainCode", ""), max_value=3)
        exit_status = _number(service_fields.get("ExecMainStatus", ""), max_value=255)
        started = parse_timestamp(service_fields.get("ExecMainStartTimestamp", ""))
        finished = parse_timestamp(service_fields.get("ExecMainExitTimestamp", ""))
        if active == "inactive" and load == "loaded":
            process_state = "inactive"
        else:
            process_state = active
        process_exit = exit_status if code == 1 and finished is not None else None
        process = _process(process_state, "none", started, finished, process_exit)
    except CollectError:
        return failed_observation(service_id, "invalid")
    if timer_fields is None:
        return {"id": service_id, "result": "ok", "observed_at": observed_at, "process": process, "timer": None}
    try:
        timer_load = _normalize_load(timer_fields.get("LoadState", ""))
        raw_timer_active = _normalize_active(timer_fields.get("ActiveState", ""))
        timer_active = "unknown" if raw_timer_active == "deactivating" else raw_timer_active
        enabled = _normalize_enabled(timer_fields.get("UnitFileState", ""))
        timer_last_trigger = parse_timestamp(timer_fields.get("LastTriggerUSec", ""))
        next_run = parse_timestamp(timer_fields.get("NextElapseUSecRealtime", ""))
        expected_service_name = expected_service_name or timer_fields.get("Unit", "")
        timer_binding = "matched"
        service_binding = "matched"
        if expected_timer_name:
            timer_path = os.path.join(systemd_root, expected_timer_name)
            timer_binding = _binding_for_unit(
                timer_fields.get("Id", expected_timer_name), timer_fields.get("FragmentPath", ""),
                timer_fields.get("DropInPaths", ""), timer_path, expected_timer_sha256,
                systemd_root=systemd_root, test_owner=test_owner,
            )
        if expected_service_name:
            service_path = os.path.join(systemd_root, expected_service_name)
            service_binding = _binding_for_unit(
                service_fields.get("Id", expected_service_name), service_fields.get("FragmentPath", ""),
                service_fields.get("DropInPaths", ""), service_path, expected_service_sha256,
                systemd_root=systemd_root, test_owner=test_owner,
            )
        if timer_fields.get("Unit", "") != expected_service_name:
            timer_binding = "mismatch"
        unit_binding = "mismatch" if "mismatch" in (timer_binding, service_binding) else (
            "unknown" if "unknown" in (timer_binding, service_binding) else "matched"
        )
        if verified_unit_binding is not None:
            if verified_unit_binding not in {"matched", "mismatch", "unknown"}:
                raise CollectError("invalid")
            unit_binding = verified_unit_binding
        completion_result = _timer_last_result(result, code, exit_status, started, finished, history_record)
        last_success = history_record.get("last_success_at") if history_record else None
        if installed_at is not None and last_success is not None and policy_lib.timestamp(last_success) < installed_at:
            raise CollectError("invalid")
        timer = {
            "load": timer_load,
            "active": timer_active,
            "enabled": enabled,
            "last_trigger_at": canonical_time(timer_last_trigger) if timer_last_trigger else None,
            "next_run_at": canonical_time(next_run) if next_run else None,
            "unit_binding": unit_binding,
            "last_result": completion_result,
            "exit_status": exit_status if code == 1 and finished is not None else None,
            "last_started_at": canonical_time(started) if started else None,
            "last_finished_at": canonical_time(finished) if finished else None,
            "last_success_at": last_success,
        }
    except CollectError:
        return failed_observation(service_id, "invalid")
    return {"id": service_id, "result": "ok", "observed_at": observed_at, "process": process, "timer": timer}


def docker_observation(
    service_id: str,
    observed_at: str,
    project: str,
    paths: CollectorPaths,
    runner: Callable[..., CommandResult],
    deadline: float,
) -> dict[str, Any]:
    list_result = call_runner(runner, docker_ps_argv(paths, project, service_id), deadline)
    if list_result.result != "ok":
        return failed_observation(service_id, list_result.result)
    try:
        matches = parse_docker_ps(list_result.stdout, project, service_id)
    except CollectError:
        return failed_observation(service_id, "invalid")
    if not matches:
        return missing_docker_observation(service_id, observed_at)
    inspect_result = call_runner(runner, docker_inspect_argv(paths, matches[0]), deadline)
    if inspect_result.result != "ok":
        return failed_observation(service_id, inspect_result.result)
    try:
        fields = parse_docker_inspect(inspect_result.stdout, service_id, project)
        return map_docker_fields(service_id, observed_at, fields)
    except CollectError:
        return failed_observation(service_id, "invalid")


def systemctl_argv(paths: CollectorPaths, unit: str) -> tuple[str, ...]:
    scheduled_units = {
        name
        for pair in SCHEDULES.values()
        for name in pair
    }
    fixed_units = {
        "docker.service", "tailscaled.service",
        "lagrange-service-status.service", "lagrange-service-status.timer",
    } | scheduled_units
    daily_unit = re.fullmatch(r"lagrange-kis-daily-[0-9a-f]{7}\.(?:service|timer)", unit)
    if unit not in fixed_units and daily_unit is None:
        raise CollectError("invalid")
    return (paths.systemctl_bin, "show", "--no-pager", "--property=" + ",".join(SYSTEMD_PROPERTIES), unit)


def systemd_observation(
    service_id: str,
    observed_at: str,
    paths: CollectorPaths,
    runner: Callable[..., CommandResult],
    deadline: float,
    *,
    policy: dict[str, Any],
    history: dict[str, Any] | None,
    booted_at: str,
) -> dict[str, Any]:
    if service_id in SYSTEMD_HOST_IDS:
        unit = "docker.service" if service_id == "docker" else "tailscaled.service"
        service_out = call_runner(runner, systemctl_argv(paths, unit), deadline)
        if service_out.result != "ok":
            return failed_observation(service_id, service_out.result)
        try:
            values = _parse_key_values(service_out.stdout, SYSTEMD_SERVICE_PROPERTIES)
            _clear_preboot_systemd_times(values, None, booted_at)
            return _clear_preboot_observation(
                map_systemd_fields(service_id, observed_at, timer_fields=None, service_fields=values), booted_at
            )
        except CollectError:
            return failed_observation(service_id, "invalid")
    if service_id == "kis-eod":
        short = policy["release_commit"][:7]
        timer_name = f"lagrange-kis-daily-{short}.timer"
        service_name = f"lagrange-kis-daily-{short}.service"
    elif service_id in SCHEDULES:
        timer_name, service_name = SCHEDULES[service_id]
    else:
        return failed_observation(service_id, "invalid")
    timer_out = call_runner(runner, systemctl_argv(paths, timer_name), deadline)
    if timer_out.result != "ok":
        return failed_observation(service_id, timer_out.result)
    service_out = call_runner(runner, systemctl_argv(paths, service_name), deadline)
    if service_out.result != "ok":
        return failed_observation(service_id, service_out.result)
    try:
        timer_values = _parse_key_values(timer_out.stdout, SYSTEMD_TIMER_PROPERTIES)
        service_values = _parse_key_values(service_out.stdout, SYSTEMD_SERVICE_PROPERTIES)
        _clear_preboot_systemd_times(service_values, timer_values, booted_at)
    except CollectError:
        return failed_observation(service_id, "invalid")
    record = history["jobs"].get(service_id) if history else None
    binding = history["binding"] if history else {}
    if service_id == "kis-eod":
        service_hash = binding.get("daily_service_sha256")
        timer_hash = binding.get("daily_timer_sha256")
    else:
        service_hash = timer_hash = None
    observed = map_systemd_fields(
        service_id, observed_at, timer_fields=timer_values, service_fields=service_values,
        expected_timer_name=timer_name, expected_service_name=service_name,
        expected_timer_sha256=timer_hash, expected_service_sha256=service_hash,
        systemd_root=paths.systemd_root, test_owner=paths.test_owner,
        history_record=record,
        installed_at=policy_lib.timestamp(history["installed_at"]) if history else None,
    )
    if service_id == "kis-eod" and observed["timer"] is not None:
        if history is None or not service_hash or not timer_hash:
            observed["timer"]["unit_binding"] = "unknown"
        else:
            start = parse_timestamp(service_values.get("ExecMainStartTimestamp", ""))
            end = parse_timestamp(service_values.get("ExecMainExitTimestamp", ""))
            result = _systemd_result(service_values.get("Result", ""))
            code = _number(service_values.get("ExecMainCode", ""), max_value=3)
            exit_status = _number(service_values.get("ExecMainStatus", ""), max_value=255)
            expected_service = os.path.join(paths.systemd_root, service_name)
            completed = verified_completion(
                start=start, end=end, exec_code=code, exit_status=exit_status, result=result,
                expected_unit=expected_service, observed_unit=service_values.get("FragmentPath", ""),
                installed_at=policy_lib.timestamp(history["installed_at"]),
            )
            if completed:
                update_history_record(
                    history, service_id, start=start, end=end, last_result="success",
                    exit_status=exit_status, successful=True,
                )
            elif (
                start is not None and end is not None and end >= start and
                end >= policy_lib.timestamp(history["installed_at"])
            ):
                last_result = observed["timer"]["last_result"]
                update_history_record(
                    history, service_id, start=start, end=end, last_result=last_result,
                    exit_status=exit_status, successful=False,
                )
    elif service_id in SCHEDULES and history is not None and observed["timer"] is not None:
        start = parse_timestamp(service_values.get("ExecMainStartTimestamp", ""))
        end = parse_timestamp(service_values.get("ExecMainExitTimestamp", ""))
        result = _systemd_result(service_values.get("Result", ""))
        code = _number(service_values.get("ExecMainCode", ""), max_value=3)
        exit_status = _number(service_values.get("ExecMainStatus", ""), max_value=255)
        expected_service = os.path.join(paths.systemd_root, service_name)
        completed = verified_completion(
            start=start, end=end, exec_code=code, exit_status=exit_status, result=result,
            expected_unit=expected_service, observed_unit=service_values.get("FragmentPath", ""),
            installed_at=policy_lib.timestamp(history["installed_at"]),
        )
        if (
            start is not None and end is not None and end >= start and
            end >= policy_lib.timestamp(history["installed_at"])
        ):
            update_history_record(
                history, service_id, start=start, end=end,
                last_result=observed["timer"]["last_result"], exit_status=exit_status,
                successful=completed,
            )
    return _clear_preboot_observation(observed, booted_at)


def validate_snapshot(snapshot: Any, *, now: datetime | None = None) -> dict[str, Any]:
    if not isinstance(snapshot, dict) or set(snapshot) != {
        "schema_version", "generated_at", "collection_started_at", "booted_at", "policy", "observations"
    }:
        raise CollectError("invalid")
    if snapshot["schema_version"] != "service-status.host.v1":
        raise CollectError("invalid")
    try:
        generated = policy_lib.timestamp(snapshot["generated_at"])
        started = policy_lib.timestamp(snapshot["collection_started_at"])
        booted = policy_lib.timestamp(snapshot["booted_at"])
        policy = policy_lib.validate_policy(snapshot["policy"])
    except policy_lib.PolicyError:
        raise CollectError("invalid")
    if generated < started or booted > generated:
        raise CollectError("invalid")
    if now is not None and generated > now.astimezone(timezone.utc):
        raise CollectError("invalid")
    if policy_lib.timestamp(policy["policy_installed_at"]) > generated:
        raise CollectError("invalid")
    observations = snapshot["observations"]
    if not isinstance(observations, list) or len(observations) != len(HOST_IDS):
        raise CollectError("invalid")
    found: set[str] = set()
    for obs in observations:
        if not isinstance(obs, dict) or set(obs) != {"id", "result", "observed_at", "process", "timer"}:
            raise CollectError("invalid")
        service_id = obs["id"]
        if service_id not in HOST_IDS or service_id in found or obs["result"] not in RESULT_ENUM:
            raise CollectError("invalid")
        found.add(service_id)
        if obs["observed_at"] is not None:
            observed = policy_lib.timestamp(obs["observed_at"])
            if observed < started or observed > generated:
                raise CollectError("invalid")
        process = obs["process"]
        timer = obs["timer"]
        if obs["result"] == "ok" and process is None:
            raise CollectError("invalid")
        if process is not None:
            if not isinstance(process, dict) or set(process) != {"state", "health", "started_at", "finished_at", "exit_code"}:
                raise CollectError("invalid")
            if process["state"] not in STATE_ENUM or process["health"] not in HEALTH_ENUM:
                raise CollectError("invalid")
            process_times: dict[str, datetime | None] = {}
            for key in ("started_at", "finished_at"):
                if process[key] is not None:
                    value = policy_lib.timestamp(process[key])
                    if value > generated:
                        raise CollectError("invalid")
                    process_times[key] = value
                else:
                    process_times[key] = None
            if (
                process_times["started_at"] is not None and process_times["finished_at"] is not None and
                process_times["finished_at"] < process_times["started_at"]
            ):
                raise CollectError("invalid")
            code = process["exit_code"]
            if code is not None and (isinstance(code, bool) or not isinstance(code, int) or not 0 <= code <= 255):
                raise CollectError("invalid")
        if timer is not None:
            if not isinstance(timer, dict) or set(timer) != {
                "load", "active", "enabled", "last_trigger_at", "next_run_at", "unit_binding",
                "last_result", "exit_status", "last_started_at", "last_finished_at", "last_success_at",
            }:
                raise CollectError("invalid")
            if timer["load"] not in {"loaded", "not-found", "masked", "error"}:
                raise CollectError("invalid")
            if timer["active"] not in {"active", "inactive", "activating", "failed", "unknown"}:
                raise CollectError("invalid")
            if timer["enabled"] not in {"enabled", "disabled", "masked", "static", "unknown"}:
                raise CollectError("invalid")
            if timer["unit_binding"] not in {"matched", "mismatch", "unknown"}:
                raise CollectError("invalid")
            if timer["last_result"] not in {"success", "failure", "none", "unknown"}:
                raise CollectError("invalid")
            timer_times: dict[str, datetime | None] = {}
            for key in ("last_trigger_at", "next_run_at", "last_started_at", "last_finished_at", "last_success_at"):
                if timer[key] is not None:
                    value = policy_lib.timestamp(timer[key])
                    if key != "next_run_at" and value > generated:
                        raise CollectError("invalid")
                    timer_times[key] = value
                else:
                    timer_times[key] = None
            if (
                timer_times["last_started_at"] is not None and timer_times["last_finished_at"] is not None and
                timer_times["last_finished_at"] < timer_times["last_started_at"]
            ):
                raise CollectError("invalid")
            if (
                timer_times["last_success_at"] is not None and timer_times["last_finished_at"] is not None and
                timer_times["last_success_at"] > timer_times["last_finished_at"]
            ):
                raise CollectError("invalid")
            code = timer["exit_status"]
            if code is not None and (isinstance(code, bool) or not isinstance(code, int) or not 0 <= code <= 255):
                raise CollectError("invalid")
    if found != set(HOST_IDS):
        raise CollectError("invalid")
    return snapshot


def read_public_snapshot(
    public_root: str,
    *,
    expected_uid: int = 0,
    expected_gid: int = 10001,
    now: datetime | None = None,
    test_owner: tuple[int, int] | None = None,
) -> tuple[dict[str, Any], tuple[int, int]]:
    """Open the directory and snapshot by name on each read; verify inode stability."""
    owner = test_owner or (expected_uid, expected_gid)
    try:
        dfd = policy_lib.open_trusted_directory(
            public_root, expected_uid=expected_uid, expected_gid=expected_gid,
            expected_mode=0o750, test_owner=test_owner,
        )
    except policy_lib.PolicyError:
        raise CollectError("invalid")
    except OSError:
        raise CollectError("invalid")
    try:
        dstat = os.fstat(dfd)
        if not stat.S_ISDIR(dstat.st_mode) or (dstat.st_uid, dstat.st_gid) != owner or stat.S_IMODE(dstat.st_mode) != 0o750:
            raise CollectError("invalid")
        try:
            fd = os.open(PUBLIC_FILE, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0), dir_fd=dfd)
        except FileNotFoundError:
            raise CollectError("missing")
        except OSError:
            raise CollectError("invalid")
        try:
            before = os.fstat(fd)
            if (
                not stat.S_ISREG(before.st_mode) or before.st_nlink != 1 or
                (before.st_uid, before.st_gid) != owner or stat.S_IMODE(before.st_mode) != 0o640 or
                before.st_size < 1 or before.st_size > MAX_SNAPSHOT_BYTES
            ):
                raise CollectError("invalid")
            chunks: list[bytes] = []
            remaining = MAX_SNAPSHOT_BYTES + 1
            while remaining:
                chunk = os.read(fd, min(remaining, 16384))
                if not chunk:
                    break
                chunks.append(chunk)
                remaining -= len(chunk)
            after = os.fstat(fd)
            if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) != (
                after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns
            ):
                raise CollectError("invalid")
            data = b"".join(chunks)
            if len(data) != before.st_size or len(data) > MAX_SNAPSHOT_BYTES:
                raise CollectError("invalid")
            try:
                named = os.stat(PUBLIC_FILE, dir_fd=dfd, follow_symlinks=False)
            except OSError:
                raise CollectError("invalid")
            if (named.st_dev, named.st_ino) != (before.st_dev, before.st_ino):
                raise CollectError("invalid")
            try:
                snapshot = policy_lib.strict_json_loads(data, MAX_SNAPSHOT_BYTES)
            except policy_lib.PolicyError:
                raise CollectError("invalid")
            return validate_snapshot(snapshot, now=now), (before.st_dev, before.st_ino)
        finally:
            os.close(fd)
    finally:
        os.close(dfd)


def _safe_lock(path: str, paths: CollectorPaths) -> int:
    owner = paths.test_owner or (0, 0)
    parent = os.path.dirname(path)
    try:
        dirfd = policy_lib.open_trusted_directory(
            parent, expected_uid=owner[0], expected_gid=owner[1], expected_mode=0o700,
            test_owner=paths.test_owner,
        )
    except (OSError, policy_lib.PolicyError):
        raise CollectError("invalid")
    os.close(dirfd)
    flags = os.O_RDWR | os.O_CREAT | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(path, flags, 0o600)
    except PermissionError:
        raise CollectError("denied")
    except OSError:
        raise CollectError("invalid")
    info = os.fstat(fd)
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or (info.st_uid, info.st_gid) != owner or stat.S_IMODE(info.st_mode) != 0o600:
        os.close(fd)
        raise CollectError("invalid")
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        os.close(fd)
        raise CollectError("locked")
    return fd


def _read_policy(paths: CollectorPaths) -> dict[str, Any]:
    try:
        data = policy_lib.read_trusted_file(
            paths.policy_file, test_owner=paths.test_owner, parent_mode=0o700
        )
        policy = policy_lib.validate_policy(policy_lib.strict_json_loads(data))
        root = paths.release_root
        try:
            real_root = os.path.realpath(root)
        except OSError:
            raise CollectError("invalid")
        expected_root = os.path.join(paths.install_root, "releases", policy["release_commit"])
        if real_root != expected_root:
            raise CollectError("invalid")
        manifest_hash = policy_lib.validate_current_release(
            paths.install_root, expected_root, policy["release_commit"], test_owner=paths.test_owner
        )
        return {"policy": policy, "manifest_sha256": manifest_hash}
    except policy_lib.PolicyError as error:
        raise CollectError("missing" if error.code == "POLICY_MISSING" else "invalid")


def _load_history(paths: CollectorPaths, policy: dict[str, Any], manifest_sha256: str) -> dict[str, Any]:
    try:
        data = policy_lib.read_trusted_file(
            paths.history_file, test_owner=paths.test_owner, parent_mode=0o700
        )
        value = policy_lib.strict_json_loads(data, MAX_COMMAND_BYTES)
        return validate_history(value, policy, manifest_sha256)
    except policy_lib.PolicyError as error:
        if error.code == "POLICY_MISSING":
            release_root = os.path.join(paths.install_root, "releases", policy["release_commit"])
            try:
                service_hash, timer_hash = policy_lib.validate_daily_unit_files(
                    paths.systemd_root, release_root, policy["release_commit"], test_owner=paths.test_owner
                )
            except policy_lib.PolicyError:
                raise CollectError("invalid")
            return empty_history(policy, manifest_sha256, service_hash, timer_hash)
        raise CollectError("invalid")
    except CollectError:
        raise CollectError("invalid")


def _write_history(paths: CollectorPaths, history: dict[str, Any]) -> None:
    data = json.dumps(history, sort_keys=True, separators=(",", ":"), ensure_ascii=True, allow_nan=False).encode("ascii") + b"\n"
    try:
        policy_lib.atomic_write(
            paths.history_file, data, expected_uid=0, expected_gid=0, mode=0o600,
            max_bytes=MAX_COMMAND_BYTES, test_owner=paths.test_owner,
        )
    except policy_lib.PolicyError:
        raise CollectError("invalid")


def _write_snapshot(paths: CollectorPaths, snapshot: dict[str, Any]) -> None:
    data = json.dumps(snapshot, sort_keys=True, separators=(",", ":"), ensure_ascii=True, allow_nan=False).encode("ascii") + b"\n"
    if len(data) > MAX_SNAPSHOT_BYTES:
        raise CollectError("invalid")
    try:
        policy_lib.atomic_write(
            os.path.join(paths.public_root, PUBLIC_FILE), data, expected_uid=0, expected_gid=paths.api_gid,
            mode=0o640, max_bytes=MAX_SNAPSHOT_BYTES, parent_mode=0o750, test_owner=paths.test_owner,
        )
    except policy_lib.PolicyError:
        raise CollectError("invalid")


def _stale_observations(service_id: str, result: str = "deadline") -> dict[str, Any]:
    return failed_observation(service_id, result)


def collect_snapshot(
    paths: CollectorPaths,
    *,
    runner: Callable[..., CommandResult] = run_fixed_command,
    clock: Callable[[], datetime] = utc_now,
    monotonic: Callable[[], float] = time.monotonic,
    total_deadline_seconds: float = TOTAL_DEADLINE_SECONDS,
    supplied_policy: dict[str, Any] | None = None,
    supplied_history: dict[str, Any] | None = None,
    manifest_sha256: str | None = None,
) -> tuple[dict[str, Any], dict[str, Any] | None]:
    started_clock = clock().astimezone(timezone.utc)
    started = canonical_time(started_clock)
    deadline = monotonic() + total_deadline_seconds
    if supplied_policy is None:
        loaded = _read_policy(paths)
        policy = loaded["policy"]
        manifest_sha256 = loaded["manifest_sha256"]
    else:
        policy = policy_lib.validate_policy(supplied_policy)
        manifest_sha256 = manifest_sha256 or "0" * 64
    history = supplied_history if supplied_history is not None else (
        _load_history(paths, policy, manifest_sha256 or "0" * 64) if supplied_policy is None else None
    )
    if history is not None:
        try:
            history = validate_history(history, policy, manifest_sha256 or "0" * 64)
        except CollectError:
            raise CollectError("invalid")
    booted_at = parse_btime(paths.proc_stat)
    observations: dict[str, dict[str, Any]] = {}
    for service_id in HOST_IDS:
        if monotonic() >= deadline:
            observations[service_id] = _stale_observations(service_id, "deadline")
            continue
        sampled = canonical_time(clock().astimezone(timezone.utc))
        if service_id in DOCKER_IDS:
            observations[service_id] = _clear_preboot_observation(
                docker_observation(service_id, sampled, policy["compose_project"], paths, runner, deadline), booted_at
            )
        else:
            observations[service_id] = systemd_observation(
                service_id, sampled, paths, runner, deadline, policy=policy, history=history, booted_at=booted_at
            )
    generated = canonical_time(clock().astimezone(timezone.utc))
    # If the absolute deadline elapsed while a command was in flight, every
    # row not yet visited was already filled above. Collection time itself is
    # represented honestly and is never used to infer a completed job.
    snapshot = {
        "schema_version": "service-status.host.v1",
        "generated_at": generated,
        "collection_started_at": started,
        "booted_at": booted_at,
        "policy": policy,
        "observations": [observations[service_id] for service_id in HOST_IDS],
    }
    try:
        validate_snapshot(snapshot, now=clock().astimezone(timezone.utc))
    except CollectError:
        raise
    return snapshot, history


def _main() -> int:
    paths = CollectorPaths()
    if os.geteuid() != 0:
        print("SERVICE_STATUS_ERROR denied", file=sys.stderr)
        return 74
    try:
        lock_fd = _safe_lock(paths.lock_file, paths)
    except CollectError as error:
        print("SERVICE_STATUS_ERROR " + error.code, file=sys.stderr)
        return 75 if error.code == "locked" else 74
    try:
        snapshot, history = collect_snapshot(paths)
        if history is not None:
            _write_history(paths, history)
        _write_snapshot(paths, snapshot)
    except CollectError as error:
        print("SERVICE_STATUS_ERROR " + error.code, file=sys.stderr)
        return 75 if error.code == "deadline" else 74
    except Exception:
        print("SERVICE_STATUS_ERROR invalid", file=sys.stderr)
        return 74
    finally:
        try:
            fcntl.flock(lock_fd, fcntl.LOCK_UN)
        finally:
            os.close(lock_fd)
    return 0


if __name__ == "__main__":
    raise SystemExit(_main())
