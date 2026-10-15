#!/usr/bin/env python3
"""Owned Linux supervisor for the synthetic candidate Web E2E lanes."""

from __future__ import annotations

import argparse
import contextlib
import dataclasses
import hashlib
import http.client
import ipaddress
import json
import os
import re
import signal
import socket
import stat
import subprocess
import sys
import time
import uuid
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence


ROOT = Path(__file__).resolve().parents[2]
WEB_DIR = ROOT / "apps/web"
API_SCRIPT = WEB_DIR / "tests/e2e/support/synthetic-api.mjs"
NEXT_CLI = ROOT / "node_modules/next/dist/bin/next"
PLAYWRIGHT_CLI = ROOT / "node_modules/@playwright/test/cli.js"
LOCK_PATH = ROOT / "target/.candidate-web-e2e.lock"
READY_PATH = "/api/v1/__test/candidate-e2e-ready"
READY_ENV_TOKEN = "CANDIDATE_E2E_READINESS_TOKEN"
READY_ENV_LANE = "CANDIDATE_E2E_READINESS_LANE"
ACTUAL_FILE = "tests/e2e/service-status.spec.ts"
ACTUAL_TITLE = (
    "Owner service-status browser acceptance › actual DB-backed API router rereads the mounted snapshot "
    "and updates policy after atomic replacement @actual"
)
EXPECTED_ACTUAL = ("chromium", ACTUAL_FILE, ACTUAL_TITLE)
BASE_API_PORT = 38182
BASE_WEB_PORT = 33001
STREAM_WEB_PORT = 33043
API_PORT_RANGE = (38180, 38279)
WEB_PORT_RANGE = (33000, 33099)
POLL_SECONDS = 0.25
HTTP_TIMEOUT_SECONDS = 2.0
API_START_TIMEOUT_SECONDS = 20.0
NEXT_START_TIMEOUT_SECONDS = 120.0
INVENTORY_TIMEOUT_SECONDS = 90.0
LANE_TIMEOUT_SECONDS = 1800.0
TERM_GRACE_SECONDS = 5.0
REAP_GRACE_SECONDS = 5.0
LOG_META_LIMIT = 4096
CAPTURE_DRAIN_SECONDS = 0.02
CAPTURE_DRAIN_BYTES = 262144

OFF_FILES = (
    "tests/e2e/backtests.spec.ts",
    "tests/e2e/candidates.spec.ts",
    "tests/e2e/live-kill-switch.spec.ts",
    "tests/e2e/no-member-live.spec.ts",
    "tests/e2e/owner-beta-recommendations.spec.ts",
    "tests/e2e/paper.spec.ts",
    "tests/e2e/phase1/phase1-multi-user.spec.ts",
    "tests/e2e/recommendations.spec.ts",
    "tests/e2e/session-recovery.spec.ts",
    "tests/e2e/typography.spec.ts",
    "tests/e2e/wp3-owner-browser-order.spec.ts",
    "tests/e2e/stock-beta.spec.ts",
    "tests/e2e/service-status.spec.ts",
)
STREAM_FILE = "tests/e2e/service-status-market-stream.spec.ts"
INTRADAY_FILE = "tests/e2e/stock-beta-intraday.spec.ts"
STOCK_BETA_FILE = "tests/e2e/stock-beta.spec.ts"
OWNER_EMPTY_TITLE = "provider-free Stock Beta V2 › renders an empty owner capacity with no signal rows"
STREAM_TITLE = "Owner status polling adds no market lease or SSE subscription in owner_only+market_ws"
INTRADAY_DESCRIBE = "provider-free Stock Beta intraday integration"
INTRADAY_TITLE_MAP = {
    "updates a READY membership quote without an analysis snapshot or page reload":
        "updates a READY membership quote without an analysis snapshot or page reload",
    "renders the signed current quote beside an unchanged EOD chart on dashboard and detail":
        "renders the signed current quote beside an unchanged EOD chart on dashboard and detail",
    "switches identity and discards a late A response after B is selected":
        "selects B without replacing the board lease and rejects stale delivery",
    "adds 005930 to READY, disables it, and re-registers the same instrument with a new identity":
        "adds 005930 to READY, disables it, and re-registers the same instrument with a new identity",
    "keeps multi-tab consumers and identity demand isolated with typed capacity state":
        "isolates multi-tab board leases and fails closed at typed capacity",
    "stops on hidden/offline state, reconnects with a new demand, and stops on logout notification":
        "clears hidden/offline scopes, bounds reconnect and latches logout",
    "advances one instrument's price, signed change, version, and receipt time":
        "advances one instrument's price, signed change, version, and receipt time",
    "retains the last good quote across bounded zero and invalid updates":
        "retains typed invalid-trade status and purges malformed delivery",
    "renders closed, halted, stale, unknown-calendar, and typed failure states":
        "renders closed, halted, stale, unknown-calendar, and typed failure states",
    "surfaces retained production failure and paused responses, then recovers normally":
        "retains nonlive failure observations and restores valid stream delivery",
    "keeps quote catalog placement independent across dashboard and detail":
        "keeps quote catalog placement independent across dashboard and detail",
    "repeats cache-only app GETs 1, 10, and 100 times without demand mutations":
        "repeats stream GETs 1, 10, and 100 times without lease mutations",
}
FULL_FILE_COUNTS = dict(zip(OFF_FILES, (3, 3, 8, 6, 11, 9, 4, 3, 4, 1, 1, 39, 38)))
FULL_FILE_COUNTS.update({INTRADAY_FILE: 12, STREAM_FILE: 1})
DOTENV_NAMES = (
    ".env",
    ".env.local",
    ".env.development",
    ".env.development.local",
)


class RunnerError(RuntimeError):
    def __init__(self, code: str, *, exit_code: int = 1, lane: str | None = None):
        super().__init__(code)
        self.code = code
        self.exit_code = normalize_exit_code(exit_code)
        self.lane = lane


def normalize_exit_code(code: int) -> int:
    return code if 1 <= code <= 255 else 1


def child_exit_code(returncode: int) -> int:
    if returncode < 0:
        return min(255, 128 + -returncode)
    return normalize_exit_code(returncode)


def json_bytes(value: Any) -> bytes:
    return (json.dumps(value, ensure_ascii=True, sort_keys=True, separators=(",", ":")) + "\n").encode()


def write_private(path: Path, payload: bytes, *, exclusive: bool = True) -> None:
    flags = os.O_WRONLY | os.O_CREAT | os.O_CLOEXEC
    flags |= os.O_EXCL if exclusive else os.O_TRUNC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    fd = os.open(path, flags, 0o600)
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "wb", closefd=False) as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(fd)
    finally:
        os.close(fd)


def make_private_dir(path: Path) -> None:
    path.mkdir(mode=0o700, parents=False, exist_ok=False)
    os.chmod(path, 0o700)


def safe_real_directory(value: str | None) -> Path | None:
    if value is None:
        return None
    candidate = Path(value)
    try:
        metadata = candidate.lstat()
    except OSError as exc:
        raise RunnerError("BROWSER_CACHE_UNAVAILABLE") from exc
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        raise RunnerError("BROWSER_CACHE_NOT_REAL_DIRECTORY")
    resolved = candidate.resolve(strict=True)
    if not resolved.is_dir():
        raise RunnerError("BROWSER_CACHE_NOT_REAL_DIRECTORY")
    return resolved


def parse_port_override(name: str, raw: str | None, default: int, bounds: tuple[int, int]) -> int:
    if raw is None:
        return default
    if not re.fullmatch(r"(?:0|[1-9][0-9]*)", raw):
        raise RunnerError(f"INVALID_{name}")
    value = int(raw)
    if not bounds[0] <= value <= bounds[1]:
        raise RunnerError(f"INVALID_{name}")
    return value


def verify_dotenv_absent(web_dir: Path) -> None:
    found: list[str] = []
    for name in DOTENV_NAMES:
        try:
            metadata = (web_dir / name).lstat()
        except FileNotFoundError:
            continue
        except OSError as exc:
            raise RunnerError("DOTENV_METADATA_UNAVAILABLE") from exc
        # Metadata only: do not open, resolve, or inspect file contents.
        found.append(name if not stat.S_ISLNK(metadata.st_mode) else f"{name}:symlink")
    if found:
        raise RunnerError("DOTENV_PRESENT")


def current_time_ns() -> int:
    return time.time_ns()


@dataclasses.dataclass(frozen=True)
class ProcessInfo:
    pid: int
    ppid: int
    pgid: int
    sid: int
    start_ticks: int
    uid: int
    state: str


@dataclasses.dataclass(frozen=True)
class ProcessRef:
    pid: int
    start_ticks: int
    uid: int
    origin_root: tuple[int, int] | None
    pgid: int
    sid: int


@dataclasses.dataclass
class ManagedProcess:
    label: str
    argv: tuple[str, ...]
    scope: str
    popen: subprocess.Popen[bytes]
    root_info: ProcessInfo | None
    capture: "LogCapture"
    returncode: int | None = None
    pidfd: int | None = None

    @property
    def root_key(self) -> tuple[int, int] | None:
        if self.root_info is None:
            return None
        return (self.root_info.pid, self.root_info.start_ticks)


def proc_info(pid: int) -> ProcessInfo | None:
    base = Path("/proc") / str(pid)
    try:
        raw = (base / "stat").read_text()
        close = raw.rfind(")")
        if close < 0:
            return None
        parsed_pid = int(raw[: raw.find(" ")])
        fields = raw[close + 2 :].split()
        if len(fields) <= 19:
            return None
        state = fields[0]
        ppid, pgid, sid = map(int, fields[1:4])
        start_ticks = int(fields[19])
        status = (base / "status").read_text()
        uid_match = re.search(r"^Uid:\s+(\d+)", status, re.MULTILINE)
        if uid_match is None:
            return None
        return ProcessInfo(parsed_pid, ppid, pgid, sid, start_ticks, int(uid_match.group(1)), state)
    except (OSError, ValueError, IndexError):
        return None


def proc_snapshot() -> dict[int, ProcessInfo]:
    result: dict[int, ProcessInfo] = {}
    try:
        entries = os.scandir("/proc")
    except OSError as exc:
        raise RunnerError("PROC_UNAVAILABLE") from exc
    with entries:
        for entry in entries:
            if not entry.name.isdecimal():
                continue
            info = proc_info(int(entry.name))
            if info is not None:
                result[info.pid] = info
    return result


def proc_network_address(encoded: str, family: int) -> str:
    raw = bytes.fromhex(encoded)
    if family == socket.AF_INET:
        return str(ipaddress.IPv4Address(raw[::-1]))
    if len(raw) != 16:
        raise ValueError("invalid IPv6 address")
    ordered = b"".join(raw[index : index + 4][::-1] for index in range(0, 16, 4))
    return str(ipaddress.IPv6Address(ordered))


@dataclasses.dataclass(frozen=True)
class Listener:
    family: int
    address: str
    port: int
    inode: int


def read_listener_table(path: Path, family: int) -> list[Listener]:
    try:
        lines = path.read_text().splitlines()[1:]
    except OSError as exc:
        raise RunnerError("PROC_SOCKET_TABLE_UNAVAILABLE") from exc
    listeners: list[Listener] = []
    for line in lines:
        fields = line.split()
        if len(fields) < 10 or fields[3] != "0A":
            continue
        try:
            address_hex, port_hex = fields[1].split(":", 1)
            listeners.append(
                Listener(family, proc_network_address(address_hex, family), int(port_hex, 16), int(fields[9]))
            )
        except (ValueError, IndexError) as exc:
            raise RunnerError("PROC_SOCKET_TABLE_INVALID") from exc
    return listeners


def all_listeners() -> list[Listener]:
    return read_listener_table(Path("/proc/net/tcp"), socket.AF_INET) + read_listener_table(
        Path("/proc/net/tcp6"), socket.AF_INET6
    )


def socket_owners(inodes: set[int], snapshot: Mapping[int, ProcessInfo]) -> dict[int, set[int]]:
    owners: dict[int, set[int]] = {inode: set() for inode in inodes}
    if not inodes:
        return owners
    try:
        processes = os.scandir("/proc")
    except OSError as exc:
        raise RunnerError("PROC_FD_SCAN_UNAVAILABLE") from exc
    with processes:
        for process in processes:
            if not process.name.isdecimal() or int(process.name) not in snapshot:
                continue
            fd_dir = Path("/proc") / process.name / "fd"
            try:
                fds = os.scandir(fd_dir)
            except OSError:
                continue
            with fds:
                for fd in fds:
                    try:
                        target = os.readlink(fd.path)
                    except OSError:
                        continue
                    match = re.fullmatch(r"socket:\[(\d+)\]", target)
                    if match is not None:
                        inode = int(match.group(1))
                        if inode in owners:
                            owners[inode].add(int(process.name))
    return owners


def enable_child_subreaper() -> None:
    if not sys.platform.startswith("linux"):
        raise RunnerError("LINUX_REQUIRED")
    try:
        import ctypes

        libc = ctypes.CDLL(None, use_errno=True)
        if libc.prctl(36, 1, 0, 0, 0) != 0:  # PR_SET_CHILD_SUBREAPER
            raise OSError(ctypes.get_errno(), "prctl")
    except (AttributeError, OSError) as exc:
        raise RunnerError("SUBREAPER_UNAVAILABLE") from exc


class PortReservations:
    """Binds both loopback families without listening, before any child starts."""

    def __init__(self) -> None:
        self._sockets: dict[int, list[socket.socket]] = {}

    @staticmethod
    def _bind_family(port: int, family: int, *, reuse_address: bool = False) -> socket.socket:
        sock = socket.socket(family, socket.SOCK_STREAM)
        try:
            if reuse_address:
                sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            if family == socket.AF_INET6:
                sock.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
                sock.bind(("::1", port))
            else:
                sock.bind(("127.0.0.1", port))
            return sock
        except OSError:
            sock.close()
            raise

    def reserve(self, port: int, *, after_owned_shutdown: bool = False) -> None:
        if port in self._sockets:
            return
        if after_owned_shutdown and any(item.port == port for item in all_listeners()):
            raise RunnerError("PORT_BUSY")
        sockets: list[socket.socket] = []
        try:
            sockets.append(self._bind_family(port, socket.AF_INET, reuse_address=after_owned_shutdown))
            sockets.append(self._bind_family(port, socket.AF_INET6, reuse_address=after_owned_shutdown))
            if after_owned_shutdown and any(item.port == port for item in all_listeners()):
                raise OSError("listener appeared while restoring reservation")
        except OSError as exc:
            for sock in sockets:
                sock.close()
            raise RunnerError("PORT_BUSY") from exc
        self._sockets[port] = sockets

    def reserve_after_owned_shutdown(self, port: int) -> None:
        # SO_REUSEADDR lets this reservation bind while this task's accepted TCP
        # connections are in TIME_WAIT. The listener table is checked before and
        # after bind; the later owner check remains mandatory before any HTTP.
        self.reserve(port, after_owned_shutdown=True)

    def reserve_all(self, ports: Iterable[int]) -> None:
        try:
            for port in dict.fromkeys(ports):
                self.reserve(port)
        except RunnerError:
            self.close_all()
            raise

    def release(self, port: int) -> None:
        for sock in self._sockets.pop(port, []):
            sock.close()

    def close_all(self) -> None:
        for port in tuple(self._sockets):
            self.release(port)

    def held(self, port: int) -> bool:
        return port in self._sockets


class LogCapture:
    """Finite nonblocking pipe reads; no reader thread or buffered-reader lock."""

    def __init__(self, stream: Any) -> None:
        self.stream = stream
        self.digest = hashlib.sha256()
        self.byte_count = 0
        self.complete = False
        self.closed = False
        self.reason = "pipe_open"
        os.set_blocking(stream.fileno(), False)

    def drain(self) -> None:
        if self.closed or self.complete:
            return
        deadline = time.monotonic() + CAPTURE_DRAIN_SECONDS
        remaining = CAPTURE_DRAIN_BYTES
        while remaining > 0 and time.monotonic() < deadline:
            try:
                chunk = os.read(self.stream.fileno(), min(65536, remaining))
            except BlockingIOError:
                return
            except (OSError, ValueError):
                self.reason = "read_failed"
                return
            if not chunk:
                self.complete = True
                self.reason = "eof"
                return
            self.byte_count += len(chunk)
            self.digest.update(chunk)
            remaining -= len(chunk)

    def finish(self) -> dict[str, Any]:
        if not self.closed:
            self.drain()
            # Nobody else reads this stream. Closing cannot contend with a
            # blocked BufferedReader.read, even when a live writer keeps it open.
            self.stream.close()
            self.closed = True
        return {
            "output_bytes": self.byte_count,
            "output_sha256": self.digest.hexdigest(),
            "capture_complete": self.complete,
            "capture_reason": self.reason,
            "output_persisted": False,
        }


class SignalController:
    def __init__(self) -> None:
        self.requested: int | None = None
        self.previous: dict[int, Any] = {}

    def _handle(self, signum: int, _frame: Any) -> None:
        if self.requested is None:
            self.requested = signum

    def __enter__(self) -> "SignalController":
        for signum in (signal.SIGINT, signal.SIGTERM):
            self.previous[signum] = signal.getsignal(signum)
            signal.signal(signum, self._handle)
        return self

    def __exit__(self, _kind: Any, _value: Any, _traceback: Any) -> None:
        for signum, handler in self.previous.items():
            signal.signal(signum, handler)

    def check(self) -> None:
        if self.requested is not None:
            raise RunnerError("INTERRUPTED", exit_code=128 + self.requested)


class Supervisor:
    def __init__(self, *, evidence_dir: Path, signals: SignalController | None = None) -> None:
        self.evidence_dir = evidence_dir
        self.signals = signals or SignalController()
        self.roots: list[ManagedProcess] = []
        self.owned: dict[int, ProcessRef] = {}
        self.process_records: dict[tuple[int, int], dict[str, Any]] = {}
        self.active_root: tuple[int, int] | None = None
        self.cleanup_errors: list[str] = []
        self.raw_reports: set[Path] = set()
        self.uid = os.geteuid()
        self.caller_pid = os.getpid()
        self.caller_sid = os.getsid(0)
        self.preexisting_children = frozenset(
            (info.pid, info.start_ticks, info.uid)
            for info in proc_snapshot().values()
            if info.ppid == self.caller_pid
        )

    def _identity(self, pid: int) -> ProcessInfo | None:
        return proc_info(pid)

    def _excluded_identity(self, info: ProcessInfo) -> bool:
        # spawn() creates a new session. The caller's session and child
        # generations that existed before this supervisor carry no authority.
        return (
            info.pid == self.caller_pid
            or info.sid == self.caller_sid
            or (info.pid, info.start_ticks, info.uid) in self.preexisting_children
        )

    def spawn(
        self,
        *,
        label: str,
        scope: str,
        argv: Sequence[str],
        cwd: Path,
        env: Mapping[str, str],
    ) -> ManagedProcess:
        if not argv or not all(isinstance(part, str) for part in argv):
            raise RunnerError("INVALID_COMMAND", lane=scope)
        try:
            proc = subprocess.Popen(
                list(argv),
                cwd=cwd,
                env=dict(env),
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                close_fds=True,
                start_new_session=True,
                text=False,
                bufsize=0,
            )
        except OSError as exc:
            raise RunnerError("CHILD_START_FAILED", lane=scope) from exc
        # An unreaped Popen child cannot have its PID reused. Pin its kernel
        # identity before any poll/reap, including the failed /proc-read path.
        try:
            pidfd = os.pidfd_open(proc.pid)
        except (AttributeError, OSError) as exc:
            # Popen still owns this unreaped child; send_signal polls before
            # signaling and is used only to unwind this failed prerequisite.
            proc.kill()
            proc.wait(timeout=REAP_GRACE_SECONDS)
            if proc.stdout is not None:
                proc.stdout.close()
            raise RunnerError("PIDFD_UNAVAILABLE", lane=scope) from exc
        capture = LogCapture(proc.stdout)
        info = self._identity(proc.pid)
        if info is not None and (
            info.uid != self.uid or info.ppid != os.getpid()
            or info.pgid != proc.pid or info.sid != proc.pid
        ):
            info = None
        managed = ManagedProcess(label, tuple(argv), scope, proc, info, capture, pidfd=pidfd)
        self.roots.append(managed)
        if info is not None:
            root_key = (info.pid, info.start_ticks)
            ref = ProcessRef(info.pid, info.start_ticks, info.uid, root_key, info.pgid, info.sid)
            self.owned[info.pid] = ref
            self._record_process(info, root_key, label, tuple(argv))
        else:
            # Never recover an unidentified launch by looking at this PID later.
            self.process_records[(proc.pid, -1)] = {
                "pid": proc.pid, "start_ticks": None, "identity_verified": False,
                "label": label, "argv": list(argv), "kernel_pidfd_pinned": True,
            }
            raise RunnerError("CHILD_IDENTITY_UNVERIFIED", lane=scope)
        return managed

    def _record_process(
        self,
        info: ProcessInfo,
        origin_root: tuple[int, int] | None,
        label: str | None = None,
        argv: tuple[str, ...] | None = None,
    ) -> None:
        key = (info.pid, info.start_ticks)
        record = self.process_records.setdefault(
            key,
            {
                "pid": info.pid,
                "start_ticks": info.start_ticks,
                "uid": info.uid,
                "pgid": info.pgid,
                "sid": info.sid,
                "origin_root": list(origin_root) if origin_root is not None else None,
                "identity_verified": True,
            },
        )
        if label is not None:
            record["label"] = label
        if argv is not None:
            record["argv"] = list(argv)

    def refresh_owned(self) -> dict[int, ProcessInfo]:
        for root in self.roots:
            root.capture.drain()
        snapshot = proc_snapshot()
        # Rebuild session authority from live, previously registered generations.
        # A stale root SID alone carries no authority after its last member exits.
        anchors: dict[tuple[int, int], tuple[int, int] | None] = {}
        for pid, ref in tuple(self.owned.items()):
            info = snapshot.get(pid)
            if info is None or not self._same_generation(ref, info) or self._excluded_identity(info):
                continue
            self.owned[pid] = dataclasses.replace(ref, pgid=info.pgid, sid=info.sid)
            anchors[(info.sid, info.uid)] = ref.origin_root
        root_pids = {root.popen.pid for root in self.roots}
        for info in snapshot.values():
            anchor = (info.sid, info.uid)
            if anchor in anchors and info.pid not in root_pids:
                self._register_descendant(info, anchors[anchor])

        # Walk parent links only through an unchanged registered parent. UID
        # changes or an already-known PID with a new generation fail closed.
        changed = True
        while changed:
            changed = False
            for info in snapshot.values():
                if info.pid in self.owned:
                    continue
                parent = self.owned.get(info.ppid)
                parent_info = snapshot.get(info.ppid)
                if (
                    parent is None or parent_info is None
                    or not self._same_generation(parent, parent_info)
                    or self._excluded_identity(parent_info)
                ):
                    continue
                if info.pid not in root_pids and self._register_descendant(info, parent.origin_root):
                    changed = True

        # A newly adopted child may have detached from its root's session.
        # Exclude caller-session helpers and pre-existing child generations:
        # an embedding pytest process can already own a resource tracker.
        # Unknown launch PIDs and old PID generations are never recovered here.
        for info in snapshot.values():
            if info.ppid == os.getpid() and info.pid not in root_pids:
                self._register_descendant(info, None)
        return snapshot

    def _register_descendant(self, info: ProcessInfo, root_key: tuple[int, int] | None) -> bool:
        if info.uid != self.uid or info.pid in self.owned or self._excluded_identity(info):
            return False
        self.owned[info.pid] = ProcessRef(
            info.pid, info.start_ticks, info.uid, root_key, info.pgid, info.sid
        )
        self._record_process(info, root_key)
        return True

    def _same_generation(self, ref: ProcessRef, info: ProcessInfo) -> bool:
        return info.pid == ref.pid and info.start_ticks == ref.start_ticks and info.uid == ref.uid == self.uid

    def is_current(self, ref: ProcessRef) -> bool:
        info = self._identity(ref.pid)
        return info is not None and self._same_generation(ref, info)

    def safe_signal(self, ref: ProcessRef, signum: int) -> str:
        current = self._identity(ref.pid)
        if current is None:
            return "gone"
        if current.start_ticks != ref.start_ticks:
            return "generation_mismatch"
        if current.uid != ref.uid or ref.uid != self.uid:
            return "uid_mismatch"
        if self._excluded_identity(current):
            return "ownership_unverified"
        registered = self.owned.get(ref.pid)
        if registered is None or (
            registered.start_ticks, registered.uid, registered.origin_root
        ) != (ref.start_ticks, ref.uid, ref.origin_root):
            return "ownership_unverified"
        try:
            os.kill(ref.pid, signum)
        except ProcessLookupError:
            return "gone"
        except PermissionError:
            return "permission_denied"
        return "signaled"

    def _current_owned(self, scope_root: tuple[int, int] | None = None) -> list[ProcessRef]:
        self.refresh_owned()
        result: list[ProcessRef] = []
        for ref in self.owned.values():
            if scope_root is not None and ref.origin_root != scope_root:
                continue
            if self.is_current(ref):
                result.append(ref)
        return result

    def _signal_session(self, root: ManagedProcess, signum: int) -> set[int]:
        info = root.root_info
        if info is None or self._excluded_identity(info):
            return set()
        snapshot = self.refresh_owned()
        members = [item for item in snapshot.values() if item.pgid == info.pgid and item.sid == info.sid]
        if not members:
            return set()
        for member in members:
            ref = self.owned.get(member.pid)
            if (
                ref is None or not self._same_generation(ref, member)
                or ref.origin_root != root.root_key or self._excluded_identity(member)
            ):
                return set()
        # Verify once more immediately before signaling the registered group.
        for member in members:
            current = self._identity(member.pid)
            if (
                current is None or not self._same_generation(self.owned[member.pid], current)
                or self._excluded_identity(current)
            ):
                return set()
            if current.pgid != info.pgid or current.sid != info.sid:
                return set()
        try:
            os.killpg(info.pgid, signum)
            return {member.pid for member in members}
        except ProcessLookupError:
            return set()
        except PermissionError:
            return set()

    def _reap_adopted(self, snapshot: Mapping[int, ProcessInfo]) -> None:
        root_pids = {root.popen.pid for root in self.roots}
        for pid, ref in tuple(self.owned.items()):
            info = snapshot.get(pid)
            if pid in root_pids or info is None or info.ppid != os.getpid():
                continue
            if not self._same_generation(ref, info):
                continue
            try:
                os.waitpid(pid, os.WNOHANG)
            except (ChildProcessError, ProcessLookupError):
                continue
            except OSError:
                self.cleanup_errors.append("REAP_FAILED")

    def wait(self, process: ManagedProcess, timeout: float, *, poll_interval: float = 0.05) -> int:
        deadline = time.monotonic() + timeout
        self.active_root = process.root_key
        try:
            while True:
                self.signals.check()
                returncode = process.popen.poll()
                snapshot = self.refresh_owned()
                if process.root_key is not None:
                    self.active_root = process.root_key
                self._reap_adopted(snapshot)
                if returncode is not None:
                    process.returncode = returncode
                    return returncode
                if time.monotonic() >= deadline:
                    raise RunnerError("COMMAND_TIMEOUT", lane=process.scope)
                time.sleep(poll_interval)
        finally:
            self.active_root = None

    def wait_listener(
        self,
        process: ManagedProcess,
        port: int,
        *,
        timeout: float,
        expected_address: str = "127.0.0.1",
        allowed_owned_ports: set[int] | None = None,
    ) -> None:
        deadline = time.monotonic() + timeout
        self.active_root = process.root_key
        try:
            while True:
                self.signals.check()
                returncode = process.popen.poll()
                if returncode is not None:
                    process.returncode = returncode
                    raise RunnerError("SERVER_EXITED", exit_code=child_exit_code(returncode), lane=process.scope)
                snapshot = self.refresh_owned()
                if process.root_key is not None:
                    self.active_root = process.root_key
                self._verify_root_generation(process, snapshot)
                listeners = all_listeners()
                on_port = [listener for listener in listeners if listener.port == port]
                if on_port:
                    inodes = {listener.inode for listener in on_port}
                    owners = socket_owners(inodes, snapshot)
                    owned_pids = {
                        pid
                        for pid, ref in self.owned.items()
                        if (info := snapshot.get(pid)) is not None and self._same_generation(ref, info)
                    }
                    if any(not owners.get(inode) for inode in inodes):
                        raise RunnerError("LISTENER_OWNER_UNVERIFIED", lane=process.scope)
                    if any(not owner_pids.issubset(owned_pids) for owner_pids in owners.values()):
                        raise RunnerError("FOREIGN_LISTENER", lane=process.scope)
                    if len(on_port) != 1 or len(inodes) != 1:
                        raise RunnerError("COMPETING_LISTENER", lane=process.scope)
                    listener = on_port[0]
                    if listener.family != socket.AF_INET or listener.address != expected_address:
                        raise RunnerError("LISTENER_ADDRESS_MISMATCH", lane=process.scope)
                    if not any(
                        self.owned[pid].origin_root == process.root_key
                        for pid in owners[inodes.pop()]
                    ):
                        raise RunnerError("LISTENER_NOT_OWNED", lane=process.scope)
                    self._assert_owned_listener_ports(allowed_owned_ports or {port})
                    return
                if time.monotonic() >= deadline:
                    raise RunnerError("LISTENER_START_TIMEOUT", lane=process.scope)
                time.sleep(POLL_SECONDS)
        finally:
            self.active_root = None

    def _assert_owned_listener_ports(self, allowed: set[int]) -> None:
        snapshot = self.refresh_owned()
        live_pids = {
            pid
            for pid, ref in self.owned.items()
            if (info := snapshot.get(pid)) is not None and self._same_generation(ref, info)
        }
        if not live_pids:
            return
        entries = all_listeners()
        inodes = {entry.inode for entry in entries}
        owners = socket_owners(inodes, snapshot)
        for inode, pids in owners.items():
            if pids & live_pids and not pids.issubset(live_pids):
                raise RunnerError("LISTENER_OWNER_AMBIGUOUS")
        own_inodes = {inode for inode, pids in owners.items() if pids and pids.issubset(live_pids)}
        unexpected = {entry.port for entry in entries if entry.inode in own_inodes} - allowed
        if unexpected:
            raise RunnerError("UNEXPECTED_OWNED_PORT")

    def verify_listener(self, process: ManagedProcess, port: int) -> None:
        snapshot = self.refresh_owned()
        if process.popen.poll() is not None:
            raise RunnerError("SERVER_EXITED", exit_code=child_exit_code(process.popen.returncode or 1), lane=process.scope)
        self._verify_root_generation(process, snapshot)
        listeners = [item for item in all_listeners() if item.port == port]
        if len(listeners) != 1:
            raise RunnerError("LISTENER_CHANGED", lane=process.scope)
        inodes = {item.inode for item in listeners}
        owners = socket_owners(inodes, snapshot)
        if len(inodes) != 1 or any(not owners.get(inode) for inode in inodes):
            raise RunnerError("LISTENER_OWNER_UNVERIFIED", lane=process.scope)
        if any(not self._owned_generation(pid, snapshot) for pids in owners.values() for pid in pids):
            raise RunnerError("FOREIGN_LISTENER", lane=process.scope)
        if any(item.family != socket.AF_INET or item.address != "127.0.0.1" for item in listeners):
            raise RunnerError("LISTENER_ADDRESS_MISMATCH", lane=process.scope)
        if not any(
            self.owned.get(pid) is not None and self.owned[pid].origin_root == process.root_key
            for pids in owners.values()
            for pid in pids
        ):
            raise RunnerError("LISTENER_NOT_OWNED", lane=process.scope)

    def _verify_root_generation(self, process: ManagedProcess, snapshot: Mapping[int, ProcessInfo]) -> None:
        if process.root_info is None:
            raise RunnerError("CHILD_IDENTITY_UNVERIFIED", lane=process.scope)
        ref = self.owned.get(process.root_info.pid)
        current = snapshot.get(process.root_info.pid)
        if ref is None or current is None or not self._same_generation(ref, current) or ref.origin_root != process.root_key:
            raise RunnerError("ROOT_GENERATION_MISMATCH", lane=process.scope)

    def _owned_generation(self, pid: int, snapshot: Mapping[int, ProcessInfo]) -> bool:
        ref = self.owned.get(pid)
        info = snapshot.get(pid)
        return ref is not None and info is not None and self._same_generation(ref, info)

    def request(self, process: ManagedProcess, port: int, path: str) -> tuple[int, Mapping[str, str], bytes]:
        self.signals.check()
        self.verify_listener(process, port)
        self.signals.check()
        connection = http.client.HTTPConnection("127.0.0.1", port, timeout=HTTP_TIMEOUT_SECONDS)
        try:
            connection.request("GET", path, headers={"Connection": "close", "Accept": "application/json, text/plain"})
            response = connection.getresponse()
            body = response.read(4097)
            status = response.status
            headers = {key.lower(): value for key, value in response.getheaders()}
        except (OSError, http.client.HTTPException) as exc:
            self.signals.check()
            raise RunnerError("READINESS_HTTP_FAILED", lane=process.scope) from exc
        finally:
            connection.close()
        self.verify_listener(process, port)
        self.signals.check()
        if len(body) > 4096:
            raise RunnerError("READINESS_BODY_TOO_LARGE", lane=process.scope)
        return status, headers, body

    def check_api_identity(
        self,
        process: ManagedProcess,
        *,
        port: int,
        token: str,
        lane: str,
    ) -> None:
        status, headers, body = self.request(process, port, READY_PATH)
        expected = {
            "schema_version": 1,
            "fixture": "candidate-synthetic-api",
            "run_id": token,
            "lane": lane,
            "pid": process.popen.pid,
        }
        try:
            decoded = json.loads(body)
        except (json.JSONDecodeError, UnicodeDecodeError) as exc:
            raise RunnerError("API_IDENTITY_INVALID", lane=lane) from exc
        if status != 200 or headers.get("cache-control") != "no-store" or decoded != expected:
            raise RunnerError("API_IDENTITY_MISMATCH", lane=lane)

    def check_web_identity(
        self,
        process: ManagedProcess,
        *,
        port: int,
        token: str,
        lane: str,
        api_pid: int,
    ) -> None:
        status, headers, body = self.request(process, port, READY_PATH)
        expected = {
            "schema_version": 1,
            "fixture": "candidate-synthetic-api",
            "run_id": token,
            "lane": lane,
            "pid": api_pid,
        }
        try:
            decoded = json.loads(body)
        except (json.JSONDecodeError, UnicodeDecodeError) as exc:
            raise RunnerError("WEB_RELAY_IDENTITY_INVALID", lane=lane) from exc
        if status != 200 or headers.get("cache-control") != "no-store" or decoded != expected:
            raise RunnerError("WEB_RELAY_IDENTITY_MISMATCH", lane=lane)

    def check_health(self, process: ManagedProcess, port: int, lane: str) -> None:
        status, headers, body = self.request(process, port, "/healthz")
        if status != 200 or headers.get("cache-control") != "no-store" or body != b"ok\n":
            raise RunnerError("WEB_HEALTH_MISMATCH", lane=lane)

    def _signal_ref(self, ref: ProcessRef, signum: int) -> None:
        result = self.safe_signal(ref, signum)
        if result == "permission_denied":
            self.cleanup_errors.append("PROCESS_SIGNAL_DENIED")

    def _terminate_owned_phase(self, signum: int, timeout: float) -> bool:
        # One fixed deadline per phase, including generations first discovered
        # during root shutdown. Kernel-adopted children need no origin_root to
        # receive the same guarded termination as registered descendants.
        deadline = time.monotonic() + timeout
        signaled: set[tuple[int, int]] = set()
        sessions_attempted: set[tuple[int, int]] = set()
        unidentified_attempted: set[int] = set()
        quiet_snapshot = False
        while True:
            # Reap roots before discovery. Confirm a quiet snapshot again within
            # this same deadline, so root exit/adoption during /proc enumeration
            # cannot make one transient empty observation authorize raw removal.
            roots_alive = [root for root in self.roots if root.popen.poll() is None]
            snapshot = self.refresh_owned()
            self._reap_adopted(snapshot)
            alive = [
                ref
                for ref in self.owned.values()
                if self._owned_generation(ref.pid, snapshot)
            ]
            if not alive and not roots_alive:
                if quiet_snapshot:
                    return True
                quiet_snapshot = True
            else:
                quiet_snapshot = False

            for root in reversed(self.roots):
                if root.root_info is None:
                    # Pinning happened before any poll/reap. Do not recover an
                    # unidentified launch by reading its numeric PID later.
                    if root not in roots_alive or root.popen.pid in unidentified_attempted:
                        continue
                    unidentified_attempted.add(root.popen.pid)
                    if root.pidfd is None:
                        self.cleanup_errors.append("ROOT_IDENTITY_UNVERIFIED")
                        continue
                    try:
                        signal.pidfd_send_signal(root.pidfd, signum)
                    except ProcessLookupError:
                        pass
                    except OSError:
                        self.cleanup_errors.append("PROCESS_SIGNAL_DENIED")
                elif root.root_key not in sessions_attempted:
                    sessions_attempted.add(root.root_key)
                    for pid in self._signal_session(root, signum):
                        ref = self.owned[pid]
                        signaled.add((ref.pid, ref.start_ticks))

            for ref in alive:
                key = (ref.pid, ref.start_ticks)
                if key not in signaled:
                    self._signal_ref(ref, signum)
                    signaled.add(key)
            if time.monotonic() >= deadline:
                return False
            time.sleep(min(0.05, max(0.0, deadline - time.monotonic())))

    def register_raw_report(self, path: Path) -> None:
        parents = {self.evidence_dir / name for name in ("inventory", "off", "market-stream")}
        if path.parent not in parents or not path.name.endswith("-raw.json"):
            raise RunnerError("RAW_REPORT_PATH_INVALID")
        # Register before spawn: cancellation, startup failure or an exited root
        # with a live writer must still reach mandatory post-reap deletion.
        self.raw_reports.add(path)

    def _remove_raw_reports(self, *, writers_gone: bool) -> None:
        if not writers_gone:
            if self.raw_reports:
                self.cleanup_errors.append("RAW_REPORT_WRITERS_REMAIN")
            return
        for path in sorted(self.raw_reports):
            try:
                path.unlink(missing_ok=True)
            except OSError:
                self.cleanup_errors.append("RAW_REPORT_REMOVAL_FAILED")
            else:
                self.raw_reports.remove(path)

    def cleanup_all(self) -> bool:
        before = len(self.cleanup_errors)
        if not self._terminate_owned_phase(signal.SIGTERM, TERM_GRACE_SECONDS):
            self._terminate_owned_phase(signal.SIGKILL, REAP_GRACE_SECONDS)
        snapshot = self.refresh_owned()
        self._reap_adopted(snapshot)
        snapshot = self.refresh_owned()
        owned_alive = any(self._owned_generation(ref.pid, snapshot) for ref in self.owned.values())
        roots_alive = any(root.popen.poll() is None for root in self.roots)
        if owned_alive:
            self.cleanup_errors.append("OWNED_PROCESS_REMAINS")
        if roots_alive:
            self.cleanup_errors.append("ROOT_PROCESS_REMAINS")
        self._remove_raw_reports(writers_gone=not owned_alive and not roots_alive)
        for root in self.roots:
            code = root.popen.poll()
            root.returncode = code
            capture = root.capture.finish()
            if code is not None and root.pidfd is not None:
                os.close(root.pidfd)
                root.pidfd = None
            if root.root_info is not None:
                log_path = self.evidence_dir / "logs" / f"{root.label}-{root.popen.pid}-{root.root_info.start_ticks}.json"
                metadata = {
                    "label": root.label,
                    "pid": root.popen.pid,
                    "start_ticks": root.root_info.start_ticks,
                    "uid": root.root_info.uid,
                    "pgid": root.root_info.pgid,
                    "sid": root.root_info.sid,
                    "exit_code": code,
                    **capture,
                }
                if not log_path.exists():
                    write_private(log_path, json_bytes(metadata))
            else:
                log_path = self.evidence_dir / "logs" / f"{root.label}-{root.popen.pid}-unverified.json"
                if not log_path.exists():
                    write_private(log_path, json_bytes({"label": root.label, "pid": root.popen.pid, "identity_verified": False, "exit_code": code, **capture}))
        return len(self.cleanup_errors) == before


def check_response_identity(status: int, headers: Mapping[str, str], body: bytes, expected: Mapping[str, Any], lane: str, code: str) -> None:
    try:
        decoded = json.loads(body)
    except (json.JSONDecodeError, UnicodeDecodeError) as exc:
        raise RunnerError(code, lane=lane) from exc
    if status != 200 or headers.get("cache-control") != "no-store" or decoded != dict(expected):
        raise RunnerError(code, lane=lane)


@dataclasses.dataclass(frozen=True, order=True)
class CaseKey:
    project: str
    file: str
    line: int
    title: str
    test_id: str

    def display(self) -> str:
        return f"{self.file}:{self.line}: {self.title}"


@dataclasses.dataclass
class Case:
    key: CaseKey
    tags: tuple[str, ...]
    status: str
    expected_status: str
    results: tuple[Mapping[str, Any], ...]
    annotations: tuple[Mapping[str, Any], ...]


def normalize_test_file(value: Any) -> str:
    if not isinstance(value, str):
        raise RunnerError("INVENTORY_FILE_INVALID")
    file = value.replace("\\", "/")
    while file.startswith("./"):
        file = file[2:]
    if file.startswith("apps/web/"):
        file = file[len("apps/web/") :]
    if file.startswith("/"):
        try:
            file = "tests/e2e/" + str(Path(file).relative_to(WEB_DIR / "tests/e2e"))
        except ValueError as exc:
            raise RunnerError("INVENTORY_FILE_OUTSIDE_E2E") from exc
    if "/../" in f"/{file}/" or file.startswith("../"):
        raise RunnerError("INVENTORY_FILE_OUTSIDE_E2E")
    if not file.startswith("tests/e2e/"):
        # Playwright's shipped JSON reporter uses paths relative to testDir.
        file = "tests/e2e/" + file
    return file


def parse_playwright_inventory(document: Mapping[str, Any]) -> list[Case]:
    errors = document.get("errors", [])
    if not isinstance(errors, list) or errors:
        raise RunnerError("PLAYWRIGHT_REPORT_ERRORS")
    config = document.get("config", {})
    if not isinstance(config, Mapping):
        raise RunnerError("INVENTORY_CONFIG_INVALID")
    if "rootDir" in config and Path(str(config["rootDir"])) != WEB_DIR / "tests/e2e":
        raise RunnerError("INVENTORY_ROOT_INVALID")
    suites = document.get("suites")
    if not isinstance(suites, list):
        raise RunnerError("INVENTORY_SUITES_INVALID")
    cases: list[Case] = []

    def visit(suite: Mapping[str, Any], parent_titles: tuple[str, ...], file_hint: str | None) -> None:
        own_file = suite.get("file")
        current_file = normalize_test_file(own_file) if isinstance(own_file, str) and own_file else file_hint
        title = suite.get("title")
        titles = parent_titles
        if isinstance(title, str) and title and current_file is not None:
            # The file-suite title is the file basename; only nested describe titles belong in test titles.
            if not (suite.get("line") == 0 and suite.get("column") == 0):
                titles = (*parent_titles, title)
        specs = suite.get("specs", [])
        if not isinstance(specs, list):
            raise RunnerError("INVENTORY_SPECS_INVALID")
        for spec in specs:
            if not isinstance(spec, Mapping):
                raise RunnerError("INVENTORY_SPEC_INVALID")
            file_value = spec.get("file") or current_file
            file = normalize_test_file(file_value)
            line = spec.get("line")
            title_value = spec.get("title")
            test_id = spec.get("id")
            tags_value = spec.get("tags", [])
            tests = spec.get("tests")
            if (
                not isinstance(line, int)
                or line < 1
                or not isinstance(title_value, str)
                or not isinstance(test_id, str)
                or not isinstance(tags_value, list)
                or not isinstance(tests, list)
                or len(tests) != 1
            ):
                raise RunnerError("INVENTORY_SPEC_FIELDS_INVALID")
            test = tests[0]
            if not isinstance(test, Mapping) or test.get("projectName") != "chromium":
                raise RunnerError("INVENTORY_PROJECT_INVALID")
            annotations_value = test.get("annotations", [])
            if not isinstance(annotations_value, list) or not all(isinstance(item, Mapping) for item in annotations_value):
                raise RunnerError("INVENTORY_ANNOTATIONS_INVALID")
            results_value = test.get("results", [])
            if not isinstance(results_value, list) or not all(
                isinstance(item, Mapping) and isinstance(item.get("retry", 0), int)
                and not isinstance(item.get("retry", 0), bool) and item.get("retry", 0) >= 0
                for item in results_value
            ):
                raise RunnerError("INVENTORY_RESULTS_INVALID")
            full_title = " › ".join((*titles, title_value))
            tags = tuple(sorted(str(tag).removeprefix("@").lower() for tag in tags_value))
            key = CaseKey("chromium", file, line, full_title, test_id)
            cases.append(
                Case(
                    key,
                    tags,
                    str(test.get("status", "")),
                    str(test.get("expectedStatus", "")),
                    tuple(results_value),
                    tuple(item for item in annotations_value if isinstance(item, Mapping)),
                )
            )
        children = suite.get("suites", [])
        if not isinstance(children, list):
            raise RunnerError("INVENTORY_CHILD_SUITES_INVALID")
        for child in children:
            if not isinstance(child, Mapping):
                raise RunnerError("INVENTORY_CHILD_SUITE_INVALID")
            visit(child, titles, current_file)

    for suite in suites:
        if not isinstance(suite, Mapping):
            raise RunnerError("INVENTORY_SUITE_INVALID")
        visit(suite, (), None)
    keys = [case.key for case in cases]
    identities = [(key.project, key.file, key.title) for key in keys]
    if len(keys) != len(set(keys)) or len(identities) != len(set(identities)):
        raise RunnerError("INVENTORY_DUPLICATE_KEY")
    return sorted(cases, key=lambda case: case.key)


def is_actual_case(case: Case) -> bool:
    return "actual" in case.tags or "@actual" in case.key.title


def validate_actual_inventory(cases: Sequence[Case]) -> Case:
    actual = [case for case in cases if is_actual_case(case)]
    if len(actual) != 1:
        raise RunnerError("ACTUAL_INVENTORY_CARDINALITY")
    case = actual[0]
    identity = (case.key.project, case.key.file, case.key.title)
    if identity != EXPECTED_ACTUAL or "actual" not in case.tags or "@actual" not in case.key.title:
        raise RunnerError("ACTUAL_INVENTORY_IDENTITY")
    return case


def compare_lane_inventories(
    full: Sequence[Case], lanes: Mapping[str, Sequence[Case]]
) -> dict[str, Any]:
    actual = validate_actual_inventory(full)
    required = {case.key: case for case in full if case.key != actual.key}
    seen: dict[CaseKey, list[str]] = {}
    duplicates: list[CaseKey] = []
    selected_actual: list[CaseKey] = []
    contract_errors: list[str] = []
    if len({(case.key.project, case.key.file, case.key.title) for case in full}) != len(full):
        contract_errors.append("FULL_DUPLICATE_IDENTITY")
    counts = {file: sum(case.key.file == file for case in full) for file in {case.key.file for case in full}}
    if counts != FULL_FILE_COUNTS:
        contract_errors.append("FULL_FILE_COUNTS_MISMATCH")
    expected_intraday_titles = {f"{INTRADAY_DESCRIBE} › {title}" for title in INTRADAY_TITLE_MAP.values()}
    if {case.key.title for case in full if case.key.file == INTRADAY_FILE} != expected_intraday_titles:
        contract_errors.append("INTRADAY_TITLE_MAP_MISMATCH")
    if [case.key.title for case in full if case.key.file == STREAM_FILE] != [STREAM_TITLE]:
        contract_errors.append("STREAM_TITLE_MISMATCH")
    owner = [case for case in full if case.key.file == STOCK_BETA_FILE and case.key.title == OWNER_EMPTY_TITLE]
    owner_key = owner[0].key if len(owner) == 1 else None
    if owner_key is None:
        contract_errors.append("OWNER_EMPTY_IDENTITY_MISMATCH")
    if set(lanes) != {"off", "market-stream"}:
        contract_errors.append("LANE_NAMES_MISMATCH")
    if any(
        case.expected_status != "passed" or any(annotation.get("type") in {"skip", "fixme"} for annotation in case.annotations)
        for case in (*full, *(case for cases in lanes.values() for case in cases))
    ):
        contract_errors.append("INVENTORY_SKIPPED_OR_IGNORED")
    expected_lanes = {
        "off": {key for key in required if key.file in OFF_FILES},
        "market-stream": {key for key in required if key.file in {INTRADAY_FILE, STREAM_FILE} or key == owner_key},
    }
    for lane, cases in lanes.items():
        if len(cases) != (129 if lane == "off" else 14):
            contract_errors.append(f"{lane.upper()}_COUNT_MISMATCH")
        if {case.key for case in cases} != expected_lanes.get(lane, set()):
            contract_errors.append(f"{lane.upper()}_SELECTION_MISMATCH")
        for case in cases:
            if is_actual_case(case):
                selected_actual.append(case.key)
            seen.setdefault(case.key, []).append(lane)
    for key, appearances in seen.items():
        if len(appearances) > 1 and not (key == owner_key and sorted(appearances) == ["market-stream", "off"]):
            duplicates.append(key)
    selected = set(seen)
    missing = sorted(set(required) - selected)
    extra = sorted(selected - set(required))
    return {
        "state": "complete" if not (missing or extra or duplicates or selected_actual or contract_errors) else "coverage_incomplete",
        "full_count": len(full),
        "required_synthetic_count": len(required),
        "selected_count": len(selected),
        "selected_execution_count": sum(len(cases) for cases in lanes.values()),
        "approved_overlap": [owner_key.display()] if owner_key is not None and sorted(seen.get(owner_key, [])) == ["market-stream", "off"] else [],
        "contract_errors": contract_errors,
        "title_map": [{"old": old, "new": new} for old, new in INTRADAY_TITLE_MAP.items()],
        "missing": [key.display() for key in missing],
        "extra": [key.display() for key in extra],
        "duplicates": [key.display() for key in sorted(set(duplicates))],
        "actual_case": {
            "state": "dedicated_not_executed_in_synthetic_CI",
            "title": ACTUAL_TITLE,
            "source": ACTUAL_FILE,
            "line": actual.key.line,
            "launcher": "scripts/qa/service-status-actual-api.sh",
        },
        "actual_selected": [key.display() for key in selected_actual],
    }


def validate_lane_results(expected: Sequence[Case], actual: Sequence[Case]) -> dict[str, int]:
    expected_keys = {case.key for case in expected}
    actual_keys = [case.key for case in actual]
    if len(actual_keys) != len(set(actual_keys)):
        raise RunnerError("RESULT_DUPLICATE_KEY")
    if set(actual_keys) != expected_keys:
        raise RunnerError("RESULT_INVENTORY_MISMATCH")
    skipped = 0
    flaky = 0
    retried = 0
    unexpected = 0
    if any(is_actual_case(case) for case in actual):
        raise RunnerError("ACTUAL_CASE_SELECTED")
    for case in actual:
        if case.status == "skipped" or case.expected_status == "skipped":
            skipped += 1
        if case.status == "flaky" or len(case.results) > 1:
            flaky += 1
        if any(result.get("retry", 0) != 0 for result in case.results):
            retried += 1
        if case.status != "expected" or case.expected_status != "passed":
            unexpected += 1
        if len(case.results) != 1 or case.results[0].get("status") != "passed":
            unexpected += 1
    if skipped:
        raise RunnerError("RESULT_SKIPPED")
    if flaky:
        raise RunnerError("RESULT_FLAKY")
    if retried:
        raise RunnerError("RESULT_RETRY")
    if unexpected:
        raise RunnerError("RESULT_UNEXPECTED")
    return {"selected": len(actual), "passed": len(actual), "skipped": 0, "flaky": 0, "unexpected": 0}


def summarize_lane_results(cases: Sequence[Case]) -> dict[str, int]:
    skipped = sum(case.status == "skipped" or case.expected_status == "skipped" for case in cases)
    flaky = sum(case.status == "flaky" or len(case.results) > 1 for case in cases)
    passed = sum(
        case.status == "expected"
        and case.expected_status == "passed"
        and len(case.results) == 1
        and case.results[0].get("status") == "passed"
        and case.results[0].get("retry", 0) == 0
        for case in cases
    )
    unexpected = sum(
        not (
            case.status == "expected"
            and case.expected_status == "passed"
            and len(case.results) == 1
            and case.results[0].get("status") == "passed"
            and case.results[0].get("retry", 0) == 0
        )
        for case in cases
    )
    return {
        "selected": len(cases),
        "passed": passed,
        "skipped": skipped,
        "flaky": flaky,
        "unexpected": unexpected,
        "retry_attempts": sum(max(len(case.results) - 1, max((int(result.get("retry", 0)) for result in case.results), default=0)) for case in cases),
    }


def safe_case(case: Case) -> dict[str, Any]:
    return {
        "key": {
            "project": case.key.project,
            "file": case.key.file,
            "line": case.key.line,
            "title": case.key.title,
            "test_id": case.key.test_id,
        },
        "tags": list(case.tags),
        "status": case.status,
        "expected_status": case.expected_status,
        "attempts": [{"status": str(result.get("status", "unknown")), "retry": result.get("retry", 0)} for result in case.results],
        "annotations": [
            {
                "type": str(annotation.get("type", ""))[:80],
                "description": str(annotation.get("description", ""))[:500],
            }
            for annotation in case.annotations
        ],
    }


@dataclasses.dataclass(frozen=True)
class RunnerSettings:
    root: Path
    api_port: int
    web_port: int
    browser_path: Path | None

    @property
    def stream_port(self) -> int:
        return STREAM_WEB_PORT


@dataclasses.dataclass
class LaneOutcome:
    lane: str
    state: str
    exit_code: int
    selected: int
    passed: int
    skipped: int
    flaky: int
    error: str | None
    cleanup_ok: bool
    reservation_ok: bool = True
    token_sha256: str | None = None
    cases: list[Case] = dataclasses.field(default_factory=list)
    retry_attempts: int = 0


def hosted_lane_evidence(outcome: LaneOutcome) -> dict[str, Any]:
    """Public CI evidence excludes process output, tokens, titles and annotations."""
    known_errors = {
        "COMMAND_FAILED", "LANE_TIMEOUT", "LANE_INTERNAL_FAILURE", "CLEANUP_FAILED",
        "PORT_BUSY_AFTER_CLEANUP", "LISTENER_REMAINS_AFTER_CLEANUP",
        "PLAYWRIGHT_JSON_INVALID", "RESULT_MISSING", "RESULT_UNEXPECTED",
        "RESULT_SKIPPED", "RESULT_FLAKY", "RESULT_RETRY", "RESULT_INVENTORY_MISMATCH",
        "OWNER_EMPTY_SELECTION_MISMATCH", "RESULT_DUPLICATE_KEY", "ACTUAL_CASE_SELECTED",
        "COMMAND_TIMEOUT", "INTERRUPTED", "CHILD_START_FAILED", "PIDFD_UNAVAILABLE",
        "CHILD_IDENTITY_UNVERIFIED", "SERVER_EXITED", "LISTENER_START_TIMEOUT",
        "LISTENER_OWNER_UNVERIFIED", "FOREIGN_LISTENER", "COMPETING_LISTENER",
        "LISTENER_ADDRESS_MISMATCH", "LISTENER_NOT_OWNED", "LISTENER_CHANGED",
        "LISTENER_OWNER_AMBIGUOUS", "UNEXPECTED_OWNED_PORT", "ROOT_GENERATION_MISMATCH",
        "READINESS_HTTP_FAILED", "READINESS_BODY_TOO_LARGE", "API_IDENTITY_INVALID",
        "API_IDENTITY_MISMATCH", "WEB_RELAY_IDENTITY_INVALID",
        "WEB_RELAY_IDENTITY_MISMATCH", "WEB_HEALTH_MISMATCH", "PLAYWRIGHT_REPORT_ERRORS",
    }
    statuses = {"expected", "unexpected", "flaky", "skipped"}
    attempt_statuses = {"passed", "failed", "timedOut", "interrupted", "skipped"}
    files = set(OFF_FILES) | {INTRADAY_FILE, STREAM_FILE}

    def count(value: Any) -> int | None:
        return value if type(value) is int and 0 <= value <= 1_000_000 else None

    def safe_enum(value: Any, allowed: set[str], fallback: str = "UNKNOWN") -> str:
        return value if isinstance(value, str) and value in allowed else fallback

    error = outcome.error if isinstance(outcome.error, str) and outcome.error in known_errors else (
        None if outcome.error is None else "OTHER_FAILURE"
    )
    state = safe_enum(outcome.state, {"passed", "running"} | {
        code.lower() for code in known_errors
    }, "other_failure")
    cases = []
    for case in outcome.cases[:1000]:
        cases.append({
            "project": "chromium" if case.key.project == "chromium" else "UNKNOWN",
            "file": safe_enum(case.key.file, files, "UNLISTED"),
            "line": count(case.key.line),
            "identity_sha256": hashlib.sha256(json_bytes({
                "project": case.key.project, "file": case.key.file, "line": case.key.line,
                "title": case.key.title, "test_id": case.key.test_id,
            })).hexdigest(),
            "status": safe_enum(case.status, statuses),
            "expected_status": safe_enum(case.expected_status, attempt_statuses),
            "attempt_count": len(case.results),
            "attempts": [
                {
                    "status": safe_enum(row.get("status"), attempt_statuses),
                    "retry": count(row.get("retry", 0)),
                }
                for row in case.results[:16]
            ],
        })
    return {
        "schema": "candidate-web-hosted-lane-v1",
        "lane": safe_enum(outcome.lane, {"off", "market-stream"}),
        "state": state, "error": error, "exit_code": count(outcome.exit_code),
        "selected": count(outcome.selected), "passed": count(outcome.passed),
        "skipped": count(outcome.skipped), "flaky": count(outcome.flaky),
        "retry_attempts": count(outcome.retry_attempts),
        "cleanup_ok": outcome.cleanup_ok is True,
        "reservation_ok": outcome.reservation_ok is True,
        "case_count": len(outcome.cases), "cases": cases,
        "case_rows_truncated": len(outcome.cases) > len(cases),
    }


def emit_hosted_lane_evidence(outcome: LaneOutcome) -> None:
    # The full private receipt remains local; only this explicit allowlist is
    # copied into hosted job logs, including on browser failure.
    print("CANDIDATE_WEB_LANE:" + json_bytes(hosted_lane_evidence(outcome)).decode("ascii").strip(), flush=True)


class Harness:
    def __init__(
        self,
        *,
        settings: RunnerSettings,
        evidence_dir: Path,
        supervisor: Supervisor,
        reservations: PortReservations,
        node: str,
        playwright_cli: Path,
        api_command_factory: Any | None = None,
        web_command_factory: Any | None = None,
        playwright_prefix: Sequence[str] | None = None,
    ) -> None:
        self.settings = settings
        self.evidence_dir = evidence_dir
        self.supervisor = supervisor
        self.reservations = reservations
        self.node = node
        self.playwright_cli = playwright_cli
        self.api_command_factory = api_command_factory
        self.web_command_factory = web_command_factory
        self.playwright_prefix = tuple(playwright_prefix) if playwright_prefix is not None else None
        self.command_records: list[dict[str, Any]] = []
        self.lane_records: dict[str, LaneOutcome] = {}
        self.port_names = {
            settings.api_port: "api",
            settings.web_port: "web",
            STREAM_WEB_PORT: "market-stream-web",
        }

    def minimal_env(self, *, lane: str | None = None, mode: str | None = None, token: str | None = None) -> dict[str, str]:
        task_tmp = self.evidence_dir / "tmp"
        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": os.environ.get("HOME", "/nonexistent"),
            "LANG": "C.UTF-8",
            "LC_ALL": "C.UTF-8",
            "TZ": "UTC",
            "TMPDIR": str(task_tmp),
            "NEXT_TELEMETRY_DISABLED": "1",
            "NEXT_PUBLIC_DISABLE_REACT_DEVTOOLS": "1",
            "NODE_ENV": "development",
        }
        if self.settings.browser_path is not None:
            env["PLAYWRIGHT_BROWSERS_PATH"] = str(self.settings.browser_path)
        if lane is not None:
            env["OWNER_INTRADAY_QUOTES_MODE"] = "off" if mode == "off" else "owner_only"
            env["OWNER_INTRADAY_QUOTE_TRANSPORT"] = "market_ws"
            env["CANDIDATE_E2E_LANE"] = lane
        if token is not None:
            env[READY_ENV_TOKEN] = token
            env[READY_ENV_LANE] = lane or ""
        return env

    def _record_command(
        self,
        process: ManagedProcess,
        argv: Sequence[str],
        timeout: float,
        phase: str,
        *,
        raise_on_failure: bool = True,
    ) -> int:
        try:
            returncode = self.supervisor.wait(process, timeout)
        except RunnerError as exc:
            self.command_records.append(
                {
                    "phase": phase,
                    "label": process.label,
                    "argv": list(argv),
                    "pid": process.popen.pid,
                    "exit_code": exc.exit_code,
                    "state": exc.code,
                }
            )
            raise
        code = child_exit_code(returncode) if returncode else 0
        self.command_records.append(
            {
                "phase": phase,
                "label": process.label,
                "argv": list(argv),
                "pid": process.popen.pid,
                "exit_code": code,
                "state": "exited",
            }
        )
        if returncode != 0 and raise_on_failure:
            raise RunnerError("COMMAND_FAILED", exit_code=code, lane=process.scope)
        return code

    def _playwright_json(
        self,
        *,
        phase: str,
        lane: str,
        files: Sequence[str],
        mode: str,
        list_only: bool,
        test_list: Path | None = None,
    ) -> list[Case]:
        lane_dir = self.evidence_dir / ("inventory" if phase == "full-inventory" else lane)
        artifacts = lane_dir / f"{phase}-artifacts"
        artifacts.mkdir(mode=0o700, exist_ok=False)
        result_path = lane_dir / f"{phase}-raw.json"
        self.supervisor.register_raw_report(result_path)
        args = [str(self.playwright_cli), "test"]
        if list_only:
            args.append("--list")
        args.extend(["--project", "chromium", "--workers", "1", "--retries", "0", "--forbid-only"])
        args.extend(["--reporter", "json" if list_only else "line,json"])
        args.extend(["--output", str(artifacts)])
        if lane == "off":
            args.extend(["--grep-invert", "@actual"])
        if test_list is not None:
            args.extend(["--test-list", str(test_list)])
        args.extend(files)
        env = self.minimal_env(lane=lane, mode=mode)
        env["PLAYWRIGHT_JSON_OUTPUT_FILE"] = str(result_path)
        env["SYNTHETIC_API_ORIGIN"] = f"http://127.0.0.1:{self.settings.api_port}"
        env["PLAYWRIGHT_BASE_URL"] = f"http://127.0.0.1:{self.settings.web_port if lane == 'off' else STREAM_WEB_PORT}"
        self.supervisor.signals.check()
        process = self.supervisor.spawn(
            label=f"playwright-{lane}-{phase}",
            scope=lane,
            argv=(self.node, *args),
            cwd=WEB_DIR,
            env=env,
        )
        self._record_command(
            process,
            (self.node, *args),
            INVENTORY_TIMEOUT_SECONDS if list_only else LANE_TIMEOUT_SECONDS,
            phase,
        )
        try:
            raw = json.loads(result_path.read_text())
        except (OSError, json.JSONDecodeError) as exc:
            raise RunnerError("PLAYWRIGHT_JSON_INVALID", lane=lane) from exc
        if not isinstance(raw, Mapping):
            raise RunnerError("PLAYWRIGHT_JSON_INVALID", lane=lane)
        cases = parse_playwright_inventory(raw)
        inventory_path = lane_dir / f"{phase}.json"
        write_private(inventory_path, json_bytes([safe_case(case) for case in cases]))
        return cases

    def collect_inventories(self) -> tuple[list[Case], list[Case], list[Case]]:
        full = self._playwright_json(
            phase="full-inventory",
            lane="inventory",
            files=("tests/e2e/",),
            mode="market-stream",
            list_only=True,
        )
        validate_actual_inventory(full)
        off = self._playwright_json(
            phase="off-inventory",
            lane="off",
            files=OFF_FILES,
            mode="off",
            list_only=True,
        )
        stream = self._playwright_json(
            phase="market-stream-inventory",
            lane="market-stream",
            files=(INTRADAY_FILE, STREAM_FILE),
            mode="market-stream",
            list_only=True,
        )
        stock = self._playwright_json(
            phase="owner-stock-inventory", lane="market-stream", files=(STOCK_BETA_FILE,),
            mode="market-stream", list_only=True,
        )
        full_stock = [case for case in full if case.key.file == STOCK_BETA_FILE]
        if {case.key for case in stock} != {case.key for case in full_stock}:
            raise RunnerError("OWNER_STOCK_INVENTORY_MISMATCH")
        owner = [case for case in stock if case.key.title == OWNER_EMPTY_TITLE]
        if len(owner) != 1:
            raise RunnerError("OWNER_EMPTY_IDENTITY_MISMATCH")
        owner_list = self._owner_test_list(owner[0])
        owner_exact = self._playwright_json(
            phase="owner-empty-inventory", lane="market-stream", files=(STOCK_BETA_FILE,),
            mode="market-stream", list_only=True, test_list=owner_list,
        )
        if [case.key for case in owner_exact] != [owner[0].key]:
            raise RunnerError("OWNER_EMPTY_SELECTION_MISMATCH")
        stream = [*stream, *owner_exact]
        coverage = compare_lane_inventories(full, {"off": off, "market-stream": stream})
        write_private(self.evidence_dir / "coverage.json", json_bytes(coverage))
        if coverage["state"] != "complete":
            raise RunnerError("FINAL_INVENTORY_CONTRACT_MISMATCH")
        return full, off, stream

    def _owner_test_list(self, case: Case) -> Path:
        if case.key.project != "chromium" or case.key.file != STOCK_BETA_FILE or case.key.title != OWNER_EMPTY_TITLE:
            raise RunnerError("OWNER_EMPTY_IDENTITY_MISMATCH")
        path = self.evidence_dir / "inventory" / "owner-empty.test-list"
        if not path.exists():
            # Playwright's --test-list file field is relative to its testDir.
            write_private(path, f"[chromium] › stock-beta.spec.ts › {case.key.title}\n".encode())
        return path

    def _read_lane_result(self, lane: str, result_path: Path) -> tuple[list[Case], dict[str, int]]:
        try:
            raw = json.loads(result_path.read_text())
        except (OSError, json.JSONDecodeError) as exc:
            raise RunnerError("PLAYWRIGHT_JSON_INVALID", lane=lane) from exc
        if not isinstance(raw, Mapping):
            raise RunnerError("PLAYWRIGHT_JSON_INVALID", lane=lane)
        cases = parse_playwright_inventory(raw)
        return cases, {"report_errors": int(bool(raw.get("errors")))}

    def _start_api(self, lane: str, token: str) -> ManagedProcess:
        port = self.settings.api_port
        self.reservations.release(port)
        env = self.minimal_env(lane=lane, mode=lane, token=token)
        env["SYNTHETIC_API_PORT"] = str(port)
        argv = (
            tuple(self.api_command_factory(lane, port, token))
            if self.api_command_factory is not None
            else (self.node, str(API_SCRIPT))
        )
        return self.supervisor.spawn(
            label=f"api-{lane}",
            scope=lane,
            argv=argv,
            cwd=WEB_DIR,
            env=env,
        )

    def _start_web(self, lane: str, token: str, web_port: int) -> ManagedProcess:
        self.reservations.release(web_port)
        mode = "off" if lane == "off" else "owner_only"
        env = self.minimal_env(lane=lane, mode=mode, token=token)
        env["API_INTERNAL_URL"] = f"http://127.0.0.1:{self.settings.api_port}"
        argv = (
            tuple(self.web_command_factory(lane, web_port, self.settings.api_port))
            if self.web_command_factory is not None
            else (
                self.node,
                str(NEXT_CLI),
                "dev",
                "--hostname",
                "127.0.0.1",
                "--port",
                str(web_port),
            )
        )
        return self.supervisor.spawn(
            label=f"next-{lane}",
            scope=lane,
            argv=argv,
            cwd=WEB_DIR,
            env=env,
        )

    def _run_browser_phase(
        self, lane: str, phase: str, files: Sequence[str], timeout: float,
        *, test_list: Path | None = None,
    ) -> tuple[list[Case], int]:
        lane_dir = self.evidence_dir / lane
        artifacts = lane_dir / f"{phase}-playwright-artifacts"
        artifacts.mkdir(mode=0o700, exist_ok=False)
        result_path = lane_dir / f"{phase}-raw.json"
        self.supervisor.register_raw_report(result_path)
        args = [str(self.playwright_cli), "test", "--project", "chromium", "--workers", "1", "--retries", "0", "--forbid-only", "--reporter", "line,json", "--output", str(artifacts)]
        if lane == "off":
            args.extend(["--grep-invert", "@actual"])
        if test_list is not None:
            args.extend(["--test-list", str(test_list)])
        args.extend(files)
        env = self.minimal_env(lane=lane, mode="off" if lane == "off" else "market-stream")
        env["CANDIDATE_E2E_PHASE"] = phase
        env["SYNTHETIC_API_ORIGIN"] = f"http://127.0.0.1:{self.settings.api_port}"
        env["PLAYWRIGHT_BASE_URL"] = f"http://127.0.0.1:{self.settings.web_port if lane == 'off' else STREAM_WEB_PORT}"
        env["PLAYWRIGHT_JSON_OUTPUT_FILE"] = str(result_path)
        argv = (*self.playwright_prefix, *args) if self.playwright_prefix is not None else (self.node, *args)
        self.supervisor.signals.check()
        process = self.supervisor.spawn(label=f"playwright-{lane}-{phase}", scope=lane, argv=argv, cwd=WEB_DIR, env=env)
        code = self._record_command(process, argv, timeout, phase, raise_on_failure=False)
        try:
            cases, _ = self._read_lane_result(lane, result_path)
        except RunnerError:
            if code:
                raise RunnerError("COMMAND_FAILED", exit_code=code, lane=lane)
            raise
        write_private(lane_dir / f"{phase}.json", json_bytes([safe_case(case) for case in cases]))
        return cases, code

    def _run_lane(self, lane: str, expected: Sequence[Case]) -> LaneOutcome:
        lane_deadline = time.monotonic() + LANE_TIMEOUT_SECONDS

        def remaining(maximum: float) -> float:
            seconds = min(maximum, lane_deadline - time.monotonic())
            if seconds <= 0:
                raise RunnerError("LANE_TIMEOUT", lane=lane)
            return seconds

        web_port = self.settings.web_port if lane == "off" else STREAM_WEB_PORT
        token = uuid.uuid4().hex
        token_hash = hashlib.sha256(token.encode()).hexdigest()
        result = LaneOutcome(lane, "running", 0, 0, 0, 0, 0, None, False, True, token_hash)
        result.selected = len(expected)
        released: list[int] = []
        error: RunnerError | None = None
        try:
            self.supervisor.signals.check()
            released.append(self.settings.api_port)
            api = self._start_api(lane, token)
            self.supervisor.wait_listener(
                api,
                self.settings.api_port,
                timeout=remaining(API_START_TIMEOUT_SECONDS),
                allowed_owned_ports={self.settings.api_port},
            )
            self.supervisor.check_api_identity(
                api, port=self.settings.api_port, token=token, lane=lane
            )
            self.supervisor.signals.check()
            released.append(web_port)
            web = self._start_web(lane, token, web_port)
            self.supervisor.wait_listener(
                web,
                web_port,
                timeout=remaining(NEXT_START_TIMEOUT_SECONDS),
                allowed_owned_ports={self.settings.api_port, web_port},
            )
            self.supervisor.check_web_identity(
                web,
                port=web_port,
                token=token,
                lane=lane,
                api_pid=api.popen.pid,
            )
            self.supervisor.check_health(web, web_port, lane)
            self.supervisor.signals.check()

            if lane == "off":
                phases = [("off-results", OFF_FILES, list(expected), None)]
            else:
                owner = [case for case in expected if case.key.file == STOCK_BETA_FILE and case.key.title == OWNER_EMPTY_TITLE]
                if len(owner) != 1:
                    raise RunnerError("OWNER_EMPTY_SELECTION_MISMATCH", lane=lane)
                main = [case for case in expected if case.key != owner[0].key]
                phases = [
                    ("market-stream-results", (INTRADAY_FILE, STREAM_FILE), main, None),
                    ("owner-empty-results", (STOCK_BETA_FILE,), owner, self._owner_test_list(owner[0])),
                ]
            for phase, files, phase_expected, test_list in phases:
                cases, browser_code = self._run_browser_phase(
                    lane, phase, files, remaining(LANE_TIMEOUT_SECONDS), test_list=test_list,
                )
                result.cases.extend(cases)
                result.selected = len(result.cases)
                counts = summarize_lane_results(result.cases)
                result.passed, result.skipped, result.flaky = counts["passed"], counts["skipped"], counts["flaky"]
                result.retry_attempts = counts["retry_attempts"]
                write_private(self.evidence_dir / lane / "results.json", json_bytes([safe_case(case) for case in result.cases]), exclusive=False)
                if browser_code:
                    raise RunnerError("COMMAND_FAILED", exit_code=browser_code, lane=lane)
                validate_lane_results(phase_expected, cases)
            validate_lane_results(expected, result.cases)
            result.state = "passed"
        except RunnerError as exc:
            error = exc
            result.state = exc.code.lower()
            result.exit_code = exc.exit_code
            result.error = exc.code
        except BaseException:
            error = RunnerError("LANE_INTERNAL_FAILURE", lane=lane)
            result.state = "lane_internal_failure"
            result.exit_code = 1
            result.error = error.code
        finally:
            result.cleanup_ok = self.supervisor.cleanup_all()
            for port in released:
                if not self._port_has_listener(port):
                    try:
                        self.reservations.reserve_after_owned_shutdown(port)
                    except RunnerError:
                        result.reservation_ok = False
                        if error is None:
                            error = RunnerError("PORT_BUSY_AFTER_CLEANUP", lane=lane)
                elif error is None:
                    result.reservation_ok = False
                    error = RunnerError("LISTENER_REMAINS_AFTER_CLEANUP", lane=lane)
                else:
                    result.reservation_ok = False
            if not result.cleanup_ok and error is None:
                error = RunnerError("CLEANUP_FAILED", lane=lane)
        if error is not None:
            result.state = error.code.lower()
            result.exit_code = error.exit_code
            result.error = error.code
        return result

    @staticmethod
    def _port_has_listener(port: int) -> bool:
        return any(listener.port == port for listener in all_listeners())

    def run(self) -> int:
        full: list[Case] = []
        off: list[Case] = []
        stream: list[Case] = []
        try:
            full, off, stream = self.collect_inventories()
        except RunnerError as exc:
            write_private(
                self.evidence_dir / "run-error.json",
                json_bytes({"state": exc.code, "exit_code": exc.exit_code}),
            )
            self.supervisor.cleanup_all()
            return exc.exit_code

        lane_expected = {"off": off, "market-stream": stream}
        first_failure: int | None = None
        for lane in ("off", "market-stream"):
            if lane == "market-stream" and any(
                not item.cleanup_ok or not item.reservation_ok for item in self.lane_records.values()
            ):
                break
            outcome = self._run_lane(lane, lane_expected[lane])
            self.lane_records[lane] = outcome
            emit_hosted_lane_evidence(outcome)
            write_private(
                self.evidence_dir / lane / "lane.json",
                json_bytes(
                    {
                        "lane": lane,
                        "state": outcome.state,
                        "exit_code": outcome.exit_code,
                        "selected": outcome.selected,
                        "passed": outcome.passed,
                        "skipped": outcome.skipped,
                        "flaky": outcome.flaky,
                        "retry_attempts": outcome.retry_attempts,
                        "error": outcome.error,
                        "cleanup_ok": outcome.cleanup_ok,
                        "reservation_ok": outcome.reservation_ok,
                        "token_sha256": outcome.token_sha256,
                    }
                ),
            )
            if outcome.exit_code and first_failure is None:
                first_failure = outcome.exit_code
            if outcome.exit_code in (130, 143):
                break

        coverage = compare_lane_inventories(full, {name: cases for name, cases in lane_expected.items()})
        write_private(self.evidence_dir / "coverage.json", json_bytes(coverage), exclusive=False)
        if coverage["state"] != "complete" and first_failure is None:
            first_failure = 1
        if first_failure is None:
            return 0
        return first_failure


def create_evidence(root: Path) -> tuple[Path, Any]:
    target = root / "target"
    try:
        target.mkdir(mode=0o700)
    except FileExistsError:
        pass
    except OSError as exc:
        raise RunnerError("TASK_TARGET_UNAVAILABLE") from exc
    try:
        metadata = target.lstat()
    except OSError as exc:
        raise RunnerError("TASK_TARGET_UNAVAILABLE") from exc
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        raise RunnerError("TASK_TARGET_NOT_REAL_DIRECTORY")
    lock_path = target / ".candidate-web-e2e.lock"
    flags = os.O_RDWR | os.O_CREAT | os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        lock_fd = os.open(lock_path, flags, 0o600)
        os.fchmod(lock_fd, 0o600)
    except OSError as exc:
        raise RunnerError("CHECKOUT_LOCK_UNAVAILABLE") from exc
    import fcntl

    try:
        fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError as exc:
        os.close(lock_fd)
        raise RunnerError("CHECKOUT_LOCK_BUSY") from exc
    token = f"{time.time_ns()}-{uuid.uuid4().hex[:12]}"
    evidence = target / f"r1-web-e2e-{token}"
    try:
        make_private_dir(evidence)
        for child in ("logs", "tmp", "inventory", "off", "market-stream"):
            make_private_dir(evidence / child)
    except OSError as exc:
        os.close(lock_fd)
        raise RunnerError("EVIDENCE_CREATE_FAILED") from exc
    return evidence, lock_fd


def check_prerequisites(root: Path) -> tuple[str, Path | None]:
    if sys.version_info < (3, 12):
        raise RunnerError("PYTHON312_REQUIRED")
    if not sys.platform.startswith("linux"):
        raise RunnerError("LINUX_REQUIRED")
    if not hasattr(os, "pidfd_open") or not hasattr(signal, "pidfd_send_signal"):
        raise RunnerError("PIDFD_REQUIRED")
    if not Path("/proc/self/stat").is_file() or not Path("/proc/net/tcp").is_file() or not Path("/proc/net/tcp6").is_file():
        raise RunnerError("PROC_REQUIRED")
    node = shutil_which("node")
    if node is None:
        raise RunnerError("NODE_UNAVAILABLE")
    if not API_SCRIPT.is_file() or not NEXT_CLI.is_file() or not PLAYWRIGHT_CLI.is_file():
        raise RunnerError("LOCAL_WEB_CLI_UNAVAILABLE")
    verify_dotenv_absent(WEB_DIR)
    browser_path = safe_real_directory(os.environ.get("PLAYWRIGHT_BROWSERS_PATH"))
    return node, browser_path


def shutil_which(command: str) -> str | None:
    # Avoid importing or invoking npm/npx; resolve only through the explicitly chosen PATH.
    path = os.environ.get("PATH", "/usr/bin:/bin")
    for directory in path.split(os.pathsep):
        candidate = Path(directory or ".") / command
        try:
            metadata = candidate.stat()
        except OSError:
            continue
        if stat.S_ISREG(metadata.st_mode) and os.access(candidate, os.X_OK):
            return str(candidate.resolve())
    return None


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api-port", type=str, default=os.environ.get("CANDIDATE_E2E_API_PORT"))
    parser.add_argument("--web-port", type=str, default=os.environ.get("CANDIDATE_E2E_WEB_PORT"))
    return parser.parse_args(argv)


def repository_head(root: Path) -> str:
    """Resolve this checkout's commit for the receipt, without inherited Git overrides."""
    try:
        result = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "--verify", "HEAD"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LC_ALL": "C"},
            timeout=5,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise RunnerError("REPOSITORY_HEAD_UNAVAILABLE") from exc
    head = result.stdout.strip()
    if result.returncode != 0 or re.fullmatch(rb"(?:[0-9a-f]{40}|[0-9a-f]{64})", head) is None:
        raise RunnerError("REPOSITORY_HEAD_UNAVAILABLE")
    return head.decode("ascii")


def main(argv: Sequence[str] | None = None) -> int:
    os.umask(0o077)
    args = parse_args(argv)
    root = ROOT
    evidence_dir: Path | None = None
    lock_fd: int | None = None
    reservations = PortReservations()
    supervisor: Supervisor | None = None
    signal_controller = SignalController()
    result = 1
    state = "initializing"
    head: str | None = None
    try:
        evidence_dir, lock_fd = create_evidence(root)
        head = repository_head(root)
        for child in ("inventory", "off", "market-stream", "logs", "tmp"):
            directory = evidence_dir / child
            if not directory.is_dir() or directory.is_symlink():
                raise RunnerError("EVIDENCE_LAYOUT_INVALID")
        api_port = parse_port_override("CANDIDATE_E2E_API_PORT", args.api_port, BASE_API_PORT, API_PORT_RANGE)
        web_port = parse_port_override("CANDIDATE_E2E_WEB_PORT", args.web_port, BASE_WEB_PORT, WEB_PORT_RANGE)
        if len({api_port, web_port, STREAM_WEB_PORT}) != 3:
            raise RunnerError("PORT_ASSIGNMENT_COLLISION")
        node, browser_path = check_prerequisites(root)
        enable_child_subreaper()
        signal_controller.__enter__()
        supervisor = Supervisor(evidence_dir=evidence_dir, signals=signal_controller)
        settings = RunnerSettings(root, api_port, web_port, browser_path)
        reservations.reserve_all((api_port, web_port, STREAM_WEB_PORT))
        harness = Harness(
            settings=settings,
            evidence_dir=evidence_dir,
            supervisor=supervisor,
            reservations=reservations,
            node=node,
            playwright_cli=PLAYWRIGHT_CLI,
        )
        state = "running"
        result = harness.run()
        coverage_state = "unknown"
        coverage_path = evidence_dir / "coverage.json"
        if coverage_path.is_file():
            try:
                coverage_state = str(json.loads(coverage_path.read_text()).get("state", "unknown"))
            except (OSError, json.JSONDecodeError):
                coverage_state = "invalid"
        failed_lane = any(item.exit_code for item in harness.lane_records.values())
        state = "passed" if result == 0 else (
            "coverage_incomplete" if coverage_state == "coverage_incomplete" and not failed_lane else "failed"
        )
        command_records = harness.command_records
        lane_records = {
            lane: {
                "state": item.state,
                "exit_code": item.exit_code,
                "selected": item.selected,
                "passed": item.passed,
                "skipped": item.skipped,
                "flaky": item.flaky,
                "retry_attempts": item.retry_attempts,
                "error": item.error,
                "cleanup_ok": item.cleanup_ok,
                "token_sha256": item.token_sha256,
            }
            for lane, item in harness.lane_records.items()
        }
    except RunnerError as exc:
        state = exc.code.lower()
        result = exc.exit_code
        command_records = []
        lane_records = {}
    except BaseException:
        state = "runner_internal_failure"
        result = 1
        command_records = []
        lane_records = {}
    finally:
        cleanup_ok = True
        cleanup_errors: list[str] = []
        if supervisor is not None:
            cleanup_ok = supervisor.cleanup_all()
            cleanup_errors = list(supervisor.cleanup_errors)
            if not cleanup_ok and result == 0:
                result = 1
                state = "cleanup_failed"
        if signal_controller.requested is not None:
            result = normalize_exit_code(128 + signal_controller.requested)
            state = "interrupted"
        reservations.close_all()
        if evidence_dir is not None:
            receipt = {
                "schema_version": 1,
                "state": state,
                "exit_code": result,
                "head": head,
                "evidence_path": str(evidence_dir),
                "cleanup_ok": cleanup_ok,
                "cleanup_errors": cleanup_errors,
                "processes": sorted(
                    supervisor.process_records.values(), key=lambda item: (item["pid"], item["start_ticks"] if item["start_ticks"] is not None else -1)
                ) if supervisor is not None else [],
                "lanes": lane_records,
                "commands": command_records,
                "not_claimed": ["hosted_ci", "production", "provider", "dedicated_actual"],
            }
            try:
                write_private(evidence_dir / "run.json", json_bytes(receipt), exclusive=False)
            except OSError:
                if result == 0:
                    result = 1
                    state = "receipt_write_failed"
        if signal_controller.previous:
            signal_controller.__exit__(None, None, None)
        if lock_fd is not None:
            os.close(lock_fd)
        if evidence_dir is None:
            print(f"candidate-web-e2e: state={state} exit={result}", file=sys.stderr)
        else:
            coverage_state = "unknown"
            coverage_path = evidence_dir / "coverage.json"
            if coverage_path.is_file():
                try:
                    coverage_state = str(json.loads(coverage_path.read_text()).get("state", "unknown"))
                except (OSError, json.JSONDecodeError):
                    coverage_state = "invalid"
            print(
                f"candidate-web-e2e: state={state} exit={result} coverage={coverage_state} evidence={evidence_dir}"
            )
    return normalize_exit_code(result) if result else 0


if __name__ == "__main__":
    raise SystemExit(main())
