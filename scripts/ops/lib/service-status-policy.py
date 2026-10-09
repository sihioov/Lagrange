#!/usr/bin/env python3
"""Closed policy receipt and immutable-release checks for service status."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import re
import stat
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable


INSTALL_ROOT = "/opt/lagrange"
PRIVATE_ROOT = "/var/lib/lagrange/service-status"
PUBLIC_ROOT = PRIVATE_ROOT + "/public"
POLICY_PATH = PRIVATE_ROOT + "/policy.json"
HISTORY_PATH = PRIVATE_ROOT + "/history.json"
LOCK_PATH = "/run/lagrange-service-status/collector.lock"
SYSTEMD_ROOT = "/etc/systemd/system"
MAX_POLICY_BYTES = 16 * 1024
MANIFEST_FORMAT = "LAGRANGE_RELEASE_MANIFEST_V2"
MANIFEST_SERVICES = (
    "db-role-bootstrap", "db-migrate", "api-server", "web", "research-worker",
    "recommendation-runner", "candidate-runner", "owner-beta-runner",
    "owner-equity-v2-runner", "nt-backtest-worker-1", "nt-backtest-worker-2",
    "paper-scheduler",
)
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
PROJECT_RE = re.compile(r"^[a-z0-9][a-z0-9_-]{0,62}$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")


class PolicyError(Exception):
    """An error carrying only a stable, non-sensitive code."""

    def __init__(self, code: str):
        super().__init__(code)
        self.code = code


def fail(code: str) -> None:
    raise PolicyError(code)


def _unique_pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            fail("POLICY_INVALID")
        result[key] = value
    return result


def _reject_constant(_: str) -> None:
    fail("POLICY_INVALID")


def strict_json_loads(data: bytes, maximum: int = MAX_POLICY_BYTES) -> Any:
    if len(data) > maximum:
        fail("POLICY_INVALID")
    try:
        text = data.decode("utf-8", "strict")
        return json.loads(text, object_pairs_hook=_unique_pairs, parse_constant=_reject_constant)
    except PolicyError:
        raise
    except (UnicodeDecodeError, json.JSONDecodeError, ValueError, RecursionError):
        fail("POLICY_INVALID")


def timestamp(value: Any) -> datetime:
    if not isinstance(value, str) or not re.fullmatch(
        r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,6})?Z", value
    ):
        fail("POLICY_INVALID")
    try:
        return datetime.fromisoformat(value[:-1] + "+00:00").astimezone(timezone.utc)
    except ValueError:
        fail("POLICY_INVALID")


def format_time(value: datetime) -> str:
    value = value.astimezone(timezone.utc)
    if value.microsecond:
        fraction = f"{value.microsecond:06d}".rstrip("0")
        return value.strftime("%Y-%m-%dT%H:%M:%S") + "." + fraction + "Z"
    return value.isoformat(timespec="seconds").replace("+00:00", "Z")


def validate_policy(value: Any) -> dict[str, Any]:
    expected = {
        "schema_version", "profile", "release_commit", "compose_project", "owner_access",
        "quote_transport", "window_source", "research_mode", "policy_installed_at",
        "job_lease_seconds",
    }
    if not isinstance(value, dict) or set(value) != expected:
        fail("POLICY_INVALID")
    mode = value["research_mode"]
    if mode not in ("daily", "daemon"):
        fail("POLICY_INVALID")
    if value["schema_version"] != "service-status.policy.v1":
        fail("POLICY_INVALID")
    if value["profile"] != ("owner_daily" if mode == "daily" else "owner_daemon"):
        fail("POLICY_INVALID")
    if not isinstance(value["release_commit"], str) or not COMMIT_RE.fullmatch(value["release_commit"]):
        fail("POLICY_INVALID")
    if not isinstance(value["compose_project"], str) or not PROJECT_RE.fullmatch(value["compose_project"]):
        fail("POLICY_INVALID")
    if value["owner_access"] != "owner_only" or value["quote_transport"] != "market_ws" or value["window_source"] != "operational_v1":
        fail("POLICY_INVALID")
    timestamp(value["policy_installed_at"])
    leases = value["job_lease_seconds"]
    lease_keys = {
        "owner_equity_v2", "owner_beta_price_recommendation", "candidate_compute",
        "recommendation", "backtest",
    }
    if not isinstance(leases, dict) or set(leases) != lease_keys:
        fail("POLICY_INVALID")
    for lease in leases.values():
        if isinstance(lease, bool) or not isinstance(lease, int) or not 1 <= lease <= 3600:
            fail("POLICY_INVALID")
    return value


def _path_chain_safe(path: str, test_owner: tuple[int, int] | None = None) -> None:
    if not os.path.isabs(path) or os.path.normpath(path) != path or path.endswith("/"):
        fail("POLICY_INVALID")
    owner = test_owner or (0, 0)
    current = "/"
    for part in Path(path).parts[1:-1]:
        current = os.path.join(current, part)
        try:
            info = os.lstat(current)
        except OSError:
            fail("POLICY_INVALID")
        if not stat.S_ISDIR(info.st_mode) or stat.S_ISLNK(info.st_mode):
            fail("POLICY_INVALID")
        if test_owner is not None and current == "/tmp":
            continue
        if (info.st_uid, info.st_gid) != owner or info.st_mode & 0o022:
            fail("POLICY_INVALID")


def _open_parent_directory(
    path: str,
    *,
    test_owner: tuple[int, int] | None = None,
    required_mode: int | None = None,
    expected_parent_owner: tuple[int, int] | None = None,
) -> int:
    """Walk an absolute parent with directory FDs and O_NOFOLLOW at each step."""
    parent = os.path.dirname(path)
    if not os.path.isabs(path) or os.path.normpath(path) != path or path.endswith("/"):
        fail("POLICY_INVALID")
    parts = Path(parent).parts[1:]
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0)
    try:
        fd = os.open("/", flags)
    except OSError:
        fail("POLICY_INVALID")
    owner = test_owner or (0, 0)
    current = ""
    try:
        for part in parts:
            current += "/" + part
            try:
                child = os.open(part, flags, dir_fd=fd)
            except OSError:
                fail("POLICY_INVALID")
            info = os.fstat(child)
            if not stat.S_ISDIR(info.st_mode):
                os.close(child)
                fail("POLICY_INVALID")
            current_owner = test_owner or (expected_parent_owner if current == parent else None) or (0, 0)
            if not (test_owner is not None and current == "/tmp") and (
                (info.st_uid, info.st_gid) != current_owner or info.st_mode & 0o022
            ):
                os.close(child)
                fail("POLICY_INVALID")
            os.close(fd)
            fd = child
        final = os.fstat(fd)
        if required_mode is not None and stat.S_IMODE(final.st_mode) != required_mode:
            fail("POLICY_INVALID")
        return fd
    except Exception:
        os.close(fd)
        raise


def open_trusted_directory(
    path: str,
    *,
    expected_uid: int,
    expected_gid: int,
    expected_mode: int,
    test_owner: tuple[int, int] | None = None,
) -> int:
    """Open a fixed directory through no-follow directory FDs at every level."""
    if not os.path.isabs(path) or os.path.normpath(path) != path or path == "/":
        fail("POLICY_INVALID")
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0)
    try:
        fd = os.open("/", flags)
    except OSError:
        fail("POLICY_INVALID")
    current = ""
    seam_owner = test_owner or (expected_uid, expected_gid)
    try:
        for part in Path(path).parts[1:]:
            current += "/" + part
            try:
                child = os.open(part, flags, dir_fd=fd)
            except OSError:
                fail("POLICY_INVALID")
            info = os.fstat(child)
            if not stat.S_ISDIR(info.st_mode):
                os.close(child)
                fail("POLICY_INVALID")
            if current == path:
                if (info.st_uid, info.st_gid) != seam_owner or stat.S_IMODE(info.st_mode) != expected_mode:
                    os.close(child)
                    fail("POLICY_INVALID")
            elif not (test_owner is not None and current == "/tmp") and (
                (info.st_uid, info.st_gid) != (test_owner or (0, 0)) or info.st_mode & 0o022
            ):
                os.close(child)
                fail("POLICY_INVALID")
            os.close(fd)
            fd = child
        return fd
    except Exception:
        os.close(fd)
        raise


def optional_trusted_file_exists(
    path: str,
    *,
    parent_mode: int,
    test_owner: tuple[int, int] | None = None,
) -> bool:
    """Distinguish an absent receipt from an unsafe symlinked path."""
    if not os.path.isabs(path) or os.path.normpath(path) != path or path.endswith("/"):
        fail("POLICY_INVALID")
    parent = os.path.dirname(path)
    parts = Path(parent).parts[1:]
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0)
    try:
        fd = os.open("/", flags)
    except OSError:
        fail("POLICY_INVALID")
    current = ""
    try:
        for part in parts:
            current += "/" + part
            try:
                child = os.open(part, flags, dir_fd=fd)
            except FileNotFoundError:
                return False
            except OSError:
                fail("POLICY_INVALID")
            info = os.fstat(child)
            if not stat.S_ISDIR(info.st_mode):
                os.close(child)
                fail("POLICY_INVALID")
            if not (test_owner is not None and current == "/tmp") and (
                (info.st_uid, info.st_gid) != (test_owner or (0, 0)) or info.st_mode & 0o022
            ):
                os.close(child)
                fail("POLICY_INVALID")
            os.close(fd)
            fd = child
        if stat.S_IMODE(os.fstat(fd).st_mode) != parent_mode:
            fail("POLICY_INVALID")
        try:
            info = os.stat(os.path.basename(path), dir_fd=fd, follow_symlinks=False)
        except FileNotFoundError:
            return False
        except OSError:
            fail("POLICY_INVALID")
        if stat.S_ISLNK(info.st_mode):
            fail("POLICY_INVALID")
        return True
    finally:
        os.close(fd)


def read_trusted_file(
    path: str,
    *,
    expected_uid: int = 0,
    expected_gid: int = 0,
    expected_mode: int = 0o600,
    max_bytes: int = MAX_POLICY_BYTES,
    test_owner: tuple[int, int] | None = None,
    parent_mode: int | None = None,
) -> bytes:
    """Read one private regular file with no-follow and link-count checks."""
    owner = test_owner or (expected_uid, expected_gid)
    if not os.path.isabs(path) or os.path.normpath(path) != path or path.endswith("/"):
        fail("POLICY_INVALID")
    name = os.path.basename(path)
    dirfd = _open_parent_directory(path, test_owner=test_owner, required_mode=parent_mode)
    flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(name, flags, dir_fd=dirfd)
    except FileNotFoundError:
        os.close(dirfd)
        fail("POLICY_MISSING")
    except OSError:
        os.close(dirfd)
        fail("POLICY_INVALID")
    try:
        before = os.fstat(fd)
        if (
            not stat.S_ISREG(before.st_mode) or before.st_nlink != 1 or
            (before.st_uid, before.st_gid) != owner or stat.S_IMODE(before.st_mode) != expected_mode or
            before.st_size < 0 or before.st_size > max_bytes
        ):
            fail("POLICY_INVALID")
        chunks: list[bytes] = []
        remaining = max_bytes + 1
        while remaining:
            part = os.read(fd, min(remaining, 8192))
            if not part:
                break
            chunks.append(part)
            remaining -= len(part)
        after = os.fstat(fd)
        if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) != (
            after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns
        ):
            fail("POLICY_INVALID")
        data = b"".join(chunks)
        if len(data) > max_bytes or len(data) != before.st_size:
            fail("POLICY_INVALID")
        try:
            named = os.stat(name, dir_fd=dirfd, follow_symlinks=False)
        except OSError:
            fail("POLICY_INVALID")
        if (named.st_dev, named.st_ino) != (before.st_dev, before.st_ino):
            fail("POLICY_INVALID")
        return data
    finally:
        os.close(fd)
        os.close(dirfd)


def atomic_write(
    path: str,
    data: bytes,
    *,
    expected_uid: int,
    expected_gid: int,
    mode: int,
    max_bytes: int,
    parent_mode: int = 0o700,
    test_owner: tuple[int, int] | None = None,
    write_hook: Callable[[int, bytes], int] | None = None,
) -> None:
    """Atomically replace a named file using its already-open parent directory."""
    if len(data) > max_bytes:
        fail("POLICY_INVALID")
    parent, name = os.path.split(path)
    if not name or name in (".", ".."):
        fail("POLICY_INVALID")
    owner = test_owner or (expected_uid, expected_gid)
    full_path = os.path.join(parent, name)
    try:
        dirfd = _open_parent_directory(
            full_path, test_owner=test_owner, required_mode=parent_mode,
            expected_parent_owner=(expected_uid, expected_gid),
        )
    except PolicyError:
        fail("POLICY_INVALID")
    temp_name = "." + name + ".tmp." + str(os.getpid()) + "." + hashlib.sha256(os.urandom(32)).hexdigest()[:16]
    fd = -1
    try:
        dstat = os.fstat(dirfd)
        if not stat.S_ISDIR(dstat.st_mode) or (dstat.st_uid, dstat.st_gid) != owner or stat.S_IMODE(dstat.st_mode) != parent_mode:
            fail("POLICY_INVALID")
        try:
            target = os.stat(name, dir_fd=dirfd, follow_symlinks=False)
        except FileNotFoundError:
            target = None
        if target is not None and (
            not stat.S_ISREG(target.st_mode) or target.st_nlink != 1 or
            (target.st_uid, target.st_gid) != owner or stat.S_IMODE(target.st_mode) != mode or
            target.st_size > max_bytes
        ):
            fail("POLICY_INVALID")
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
        fd = os.open(temp_name, flags, 0o600, dir_fd=dirfd)
        if os.fstat(fd).st_dev != dstat.st_dev:
            fail("POLICY_INVALID")
        created = os.fstat(fd)
        if (created.st_uid, created.st_gid) != owner:
            if not (test_owner is not None and owner == (os.geteuid(), os.getegid())):
                os.fchown(fd, owner[0], owner[1])
        os.fchmod(fd, mode)
        view = memoryview(data)
        while view:
            written = write_hook(fd, bytes(view)) if write_hook else os.write(fd, view)
            if written <= 0 or written > len(view):
                fail("POLICY_INVALID")
            view = view[written:]
        os.fsync(fd)
        check = os.fstat(fd)
        if (
            not stat.S_ISREG(check.st_mode) or check.st_nlink != 1 or
            (check.st_uid, check.st_gid) != owner or stat.S_IMODE(check.st_mode) != mode or check.st_size != len(data)
        ):
            fail("POLICY_INVALID")
        os.replace(temp_name, name, src_dir_fd=dirfd, dst_dir_fd=dirfd)
        os.fsync(dirfd)
    except PolicyError:
        raise
    except OSError:
        fail("POLICY_INVALID")
    finally:
        if fd >= 0:
            os.close(fd)
        try:
            os.unlink(temp_name, dir_fd=dirfd)
        except FileNotFoundError:
            pass
        os.close(dirfd)


def validate_manifest(path: str, commit: str, *, test_owner: tuple[int, int] | None = None) -> str:
    if not COMMIT_RE.fullmatch(commit) or commit == "0" * 40:
        fail("POLICY_MISMATCH")
    data = read_trusted_file(path, expected_mode=0o600, max_bytes=64 * 1024, test_owner=test_owner)
    try:
        text = data.decode("ascii", "strict")
    except UnicodeDecodeError:
        fail("POLICY_MISMATCH")
    if not text.endswith("\n") or "\r" in text or "\x00" in text:
        fail("POLICY_MISMATCH")
    lines = text[:-1].split("\n")
    if len(lines) != len(MANIFEST_SERVICES) + 2 or lines[0] != MANIFEST_FORMAT or lines[1] != "commit|" + commit:
        fail("POLICY_MISMATCH")
    for index, service in enumerate(MANIFEST_SERVICES, start=2):
        fields = lines[index].split("|")
        if len(fields) != 5 or fields[0] != "image" or fields[1] != service:
            fail("POLICY_MISMATCH")
        expected_ref = f"lagrange-station-{service}:{commit}"
        if fields[2] != expected_ref or not re.fullmatch(r"sha256:[0-9a-f]{64}", fields[3]) or fields[4] != commit:
            fail("POLICY_MISMATCH")
    return hashlib.sha256(data).hexdigest()


def validate_current_release(install_root: str, release_root: str, commit: str, *, test_owner: tuple[int, int] | None = None) -> str:
    """Check current -> exact immutable release and its canonical image manifest."""
    if not COMMIT_RE.fullmatch(commit) or commit == "0" * 40:
        fail("POLICY_MISMATCH")
    expected = os.path.join(install_root, "releases", commit)
    if release_root != expected or os.path.basename(release_root) != commit:
        fail("POLICY_MISMATCH")
    for path in (install_root, os.path.join(install_root, "releases"), release_root):
        try:
            info = os.lstat(path)
        except OSError:
            fail("POLICY_MISMATCH")
        if (
            not stat.S_ISDIR(info.st_mode) or stat.S_ISLNK(info.st_mode) or
            (info.st_uid, info.st_gid) != (test_owner or (0, 0)) or info.st_mode & 0o022
        ):
            fail("POLICY_MISMATCH")
    link = os.path.join(install_root, "current")
    try:
        info = os.lstat(link)
        target = os.readlink(link)
    except OSError:
        fail("POLICY_MISMATCH")
    if (
        not stat.S_ISLNK(info.st_mode) or
        (info.st_uid, info.st_gid) != (test_owner or (0, 0)) or
        target != "releases/" + commit
    ):
        fail("POLICY_MISMATCH")
    manifest_hash = validate_manifest(os.path.join(release_root, ".lagrange-release-manifest"), commit, test_owner=test_owner)
    return manifest_hash


def validate_daily_unit_files(
    systemd_root: str,
    release_root: str,
    commit: str,
    *,
    test_owner: tuple[int, int] | None = None,
) -> tuple[str, str]:
    """Return full service/timer hashes only after exact commit and target checks."""
    if not COMMIT_RE.fullmatch(commit):
        fail("POLICY_MISMATCH")
    short = commit[:7]
    service_name = f"lagrange-kis-daily-{short}.service"
    timer_name = f"lagrange-kis-daily-{short}.timer"
    service_path = os.path.join(systemd_root, service_name)
    timer_path = os.path.join(systemd_root, timer_name)
    service = read_trusted_file(service_path, expected_mode=0o644, max_bytes=64 * 1024, test_owner=test_owner)
    timer = read_trusted_file(timer_path, expected_mode=0o644, max_bytes=64 * 1024, test_owner=test_owner)
    try:
        service_text = service.decode("utf-8", "strict")
        timer_text = timer.decode("utf-8", "strict")
    except UnicodeDecodeError:
        fail("POLICY_MISMATCH")
    wrapper = os.path.join(release_root, "scripts/ops/kis-daily-production.sh")
    required_service = (
        f"Environment=LAGRANGE_CODE_COMMIT={commit}",
        f"ExecStart={wrapper} --execute",
        "ConditionFileIsExecutable=" + wrapper,
    )
    if any(service_text.count(line) != 1 for line in required_service):
        fail("POLICY_MISMATCH")
    if timer_text.count(f"Unit={service_name}") != 1:
        fail("POLICY_MISMATCH")
    # The existing installer renders the rest of the unit from the immutable
    # release plus trusted installation inputs. Its --check mode is read-only.
    checker = os.path.join(release_root, "scripts/ops/install-kis-daily.sh")
    if not os.path.isfile(checker) or os.path.islink(checker) or not os.access(checker, os.X_OK):
        fail("POLICY_MISMATCH")
    return hashlib.sha256(service).hexdigest(), hashlib.sha256(timer).hexdigest()


def validate_startup_binding(
    policy_path: str,
    install_root: str,
    release_root: str,
    commit: str,
    *,
    test_owner: tuple[int, int] | None = None,
) -> str:
    """Return absent/daily/daemon, rejecting a present stale or unsafe receipt."""
    try:
        exists = optional_trusted_file_exists(policy_path, parent_mode=0o700, test_owner=test_owner)
    except PolicyError:
        fail("POLICY_MISMATCH")
    if not exists:
        return "absent"
    value = strict_json_loads(read_trusted_file(policy_path, test_owner=test_owner, parent_mode=0o700))
    policy = validate_policy(value)
    if policy["release_commit"] != commit:
        fail("POLICY_MISMATCH")
    validate_current_release(install_root, release_root, commit, test_owner=test_owner)
    return policy["research_mode"]


def validate_startup_policy(
    policy_path: str,
    history_path: str,
    install_root: str,
    release_root: str,
    systemd_root: str,
    commit: str,
    *,
    test_owner: tuple[int, int] | None = None,
    runner: Callable[..., Any] | None = None,
) -> tuple[str, str | None]:
    """Validate the receipt, immutable binding, daily units, and timer before release startup."""
    mode = validate_startup_binding(policy_path, install_root, release_root, commit, test_owner=test_owner)
    if mode == "absent":
        return mode, None
    try:
        policy = validate_policy(strict_json_loads(read_trusted_file(policy_path, test_owner=test_owner, parent_mode=0o700)))
        manifest_hash = validate_current_release(install_root, release_root, commit, test_owner=test_owner)
        history_data = read_trusted_file(history_path, test_owner=test_owner, parent_mode=0o700)
        history_value = strict_json_loads(history_data, 64 * 1024)
        collector_path = os.path.join(os.path.dirname(os.path.dirname(__file__)), "collect-service-status.py")
        spec = importlib.util.spec_from_file_location("service_status_startup_collector", collector_path)
        if spec is None or spec.loader is None:
            fail("POLICY_MISMATCH")
        collector = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = collector
        spec.loader.exec_module(collector)
        history = collector.validate_history(history_value, policy, manifest_hash)
        service_hash, timer_hash = validate_daily_unit_files(
            systemd_root, release_root, commit, test_owner=test_owner
        )
        binding = history["binding"]
        if binding["daily_service_sha256"] != service_hash or binding["daily_timer_sha256"] != timer_hash:
            fail("POLICY_MISMATCH")
        if runner is None:
            def runner(argv: tuple[str, ...], *, deadline: float) -> Any:
                return collector.run_fixed_command(argv, deadline=deadline)
        paths = collector.CollectorPaths(systemd_root=systemd_root, test_owner=test_owner)
        timer_name = f"lagrange-kis-daily-{commit[:7]}.timer"
        service_name = f"lagrange-kis-daily-{commit[:7]}.service"
        outputs: dict[str, dict[str, str]] = {}
        for unit in (timer_name, service_name):
            result = collector.call_runner(
                runner, collector.systemctl_argv(paths, unit), time.monotonic() + 2.0
            )
            if result.result != "ok":
                fail("POLICY_MISMATCH")
            properties = (
                collector.SYSTEMD_TIMER_PROPERTIES
                if unit.endswith(".timer")
                else collector.SYSTEMD_SERVICE_PROPERTIES
            )
            outputs[unit] = collector._parse_key_values(result.stdout, properties)
        timer = outputs[timer_name]
        service = outputs[service_name]
        timer_path = os.path.join(systemd_root, timer_name)
        service_path = os.path.join(systemd_root, service_name)
        if (
            timer.get("LoadState") != "loaded" or timer.get("FragmentPath") != timer_path or
            timer.get("DropInPaths", "").strip() or timer.get("Unit") != service_name or
            service.get("LoadState") != "loaded" or service.get("FragmentPath") != service_path or
            service.get("DropInPaths", "").strip()
        ):
            fail("POLICY_MISMATCH")
        return mode, policy["compose_project"]
    except PolicyError:
        raise
    except Exception:
        fail("POLICY_MISMATCH")


def validate_daemon_timer_gate(
    systemd_root: str,
    commit: str,
    *,
    test_owner: tuple[int, int] | None = None,
    runner: Callable[..., Any] | None = None,
) -> None:
    """Reject a full daemon release start while its exact daily timer is active/enabled."""
    if not COMMIT_RE.fullmatch(commit):
        fail("POLICY_MISMATCH")
    collector_path = os.path.join(os.path.dirname(__file__), "..", "collect-service-status.py")
    spec = importlib.util.spec_from_file_location("service_status_daemon_gate_collector", collector_path)
    if spec is None or spec.loader is None:
        fail("POLICY_MISMATCH")
    collector = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = collector
    spec.loader.exec_module(collector)
    if runner is None:
        def runner(argv: tuple[str, ...], *, deadline: float) -> Any:
            return collector.run_fixed_command(argv, deadline=deadline)
    timer_name = f"lagrange-kis-daily-{commit[:7]}.timer"
    service_name = timer_name[:-6] + ".service"
    paths = collector.CollectorPaths(systemd_root=systemd_root, test_owner=test_owner)
    result = collector.call_runner(
        runner, collector.systemctl_argv(paths, timer_name), time.monotonic() + 2.0
    )
    if result.result != "ok":
        fail("POLICY_MISMATCH")
    try:
        timer = collector._parse_key_values(result.stdout, collector.SYSTEMD_TIMER_PROPERTIES)
        if (
            timer.get("LoadState") != "loaded" or
            timer.get("FragmentPath") != os.path.join(systemd_root, timer_name) or
            timer.get("DropInPaths", "").strip() or timer.get("Unit") != service_name
        ):
            fail("POLICY_MISMATCH")
        active = collector._normalize_active(timer.get("ActiveState", ""))
        enabled = collector._normalize_enabled(timer.get("UnitFileState", ""))
    except (KeyError, collector.CollectError):
        fail("POLICY_MISMATCH")
    if active != "inactive" or enabled != "disabled":
        fail("DAEMON_MODE_CONFLICT")


def _main() -> int:
    parser = argparse.ArgumentParser(add_help=True)
    sub = parser.add_subparsers(dest="command", required=True)
    startup = sub.add_parser("startup")
    startup.add_argument("--policy-path", default=POLICY_PATH)
    startup.add_argument("--history-path", default=HISTORY_PATH)
    startup.add_argument("--install-root", default=INSTALL_ROOT)
    startup.add_argument("--systemd-root", default=SYSTEMD_ROOT)
    startup.add_argument("--release-root", required=True)
    startup.add_argument("--commit", required=True)
    daemon_gate = sub.add_parser("daemon-gate")
    daemon_gate.add_argument("--systemd-root", default=SYSTEMD_ROOT)
    daemon_gate.add_argument("--commit", required=True)
    args = parser.parse_args()
    try:
        if os.geteuid() != 0:
            fail("POLICY_MISMATCH")
        if args.command == "daemon-gate":
            validate_daemon_timer_gate(args.systemd_root, args.commit)
            print("DAEMON_MODE_GATE: PASS")
            return 0
        mode, project = validate_startup_policy(
            args.policy_path, args.history_path, args.install_root, args.release_root,
            args.systemd_root, args.commit,
        )
    except PolicyError as error:
        print(error.code, file=sys.stderr)
        return 1
    print(mode if project is None else mode + "|" + project)
    return 0


if __name__ == "__main__":
    raise SystemExit(_main())
