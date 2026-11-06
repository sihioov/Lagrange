#!/usr/bin/env python3
"""Host-only fault observer for one owned disposable native helper.

No tracing runs at import time. This is not a production drain facility.
"""
from __future__ import annotations

import contextlib
import ctypes
import errno
import hashlib
import json
import os
import pathlib
import platform
import re
import signal
import stat
import struct
import time
import uuid


class FaultError(Exception):
    pass


def require(value, code):
    if not value:
        raise FaultError(code)


def start_ticks(pid):
    text = pathlib.Path("/proc/%d/stat" % pid).read_text()
    return int(text[text.rfind(")") + 2:].split()[19])


def directory_identity(path):
    info = os.stat(path, follow_symlinks=False)
    require(stat.S_ISDIR(info.st_mode), "FAULT_OUTPUT_DIRECTORY")
    return info.st_dev, info.st_ino


def syscall_record(data):
    """Decode Linux PTRACE_GET_SYSCALL_INFO, never infer entry/exit parity."""
    require(type(data) is bytes and len(data) >= 33, "FAULT_SYSCALL_INFO")
    op, arch = data[0], struct.unpack_from("<I", data, 4)[0]
    require(arch == 0xC000003E, "FAULT_ABI")
    if op == 1:
        require(len(data) >= 80, "FAULT_SYSCALL_INFO")
        return ("entry", struct.unpack_from("<Q", data, 24)[0],
                struct.unpack_from("<6Q", data, 32))
    if op == 2:
        return "exit", struct.unpack_from("<q", data, 24)[0], bool(data[32])
    raise FaultError("FAULT_SYSCALL_PHASE")


def _unique(items):
    value = {}
    for key, item in items:
        require(key not in value, "FAULT_RESULT_WIRE")
        value[key] = item
    return value


def result_identity(data, leaf):
    require(type(data) is bytes and 0 < len(data) <= 65536, "FAULT_RESULT_SIZE")
    value = json.loads(data.decode("ascii"), object_pairs_hook=_unique)
    require(json.dumps(value, sort_keys=True, separators=(",", ":")).encode() == data,
            "FAULT_RESULT_WIRE")
    require(type(value) is dict and set(value) ==
            {"schema", "action", "outcome", "control_epoch", "account_id", "data"}
            and value["schema"] == "owner-beta-paper-action-v1"
            and value["action"] == "BEGIN_OPERATION" and value["outcome"] == "OK"
            and type(value["control_epoch"]) is int and 0 < value["control_epoch"] < 2**63,
            "FAULT_NOT_BEGIN_RESULT")
    inner = value["data"]
    require(type(inner) is dict and set(inner) ==
            {"generation", "operation_event_id", "intent_sha256", "fenced_epoch"}
            and type(inner["fenced_epoch"]) is int and inner["fenced_epoch"] == value["control_epoch"]
            and type(inner["intent_sha256"]) is str
            and re.fullmatch("sha256:[0-9a-f]{64}", inner["intent_sha256"]), "FAULT_NOT_BEGIN_RESULT")
    for identifier in (value["account_id"], inner["generation"], inner["operation_event_id"]):
        try:
            parsed = uuid.UUID(identifier)
        except (ValueError, TypeError, AttributeError):
            raise FaultError("FAULT_NOT_BEGIN_RESULT") from None
        require(parsed.int != 0 and str(parsed) == identifier, "FAULT_NOT_BEGIN_RESULT")
    digest = hashlib.sha256(data).hexdigest()
    require(leaf == digest + ".json", "FAULT_RESULT_DIGEST")
    return value, "sha256:" + digest


class StoppedResult:
    """Owned stopped-thread token; valid only until its observer resumes."""
    def __init__(self, owner, tid, fd, leaf, identity, data, value, digest):
        self._owner, self.tid, self.fd = owner, tid, fd
        self.leaf, self.identity, self.bytes = leaf, identity, data
        self.value, self.digest = value, digest

    def revalidate(self):
        require(self._owner.token is self and self.tid in self._owner.stopped,
                "FAULT_TOKEN_CLOSED")
        self._owner._identity()
        info = os.fstat(self.fd)
        named = os.stat("/proc/%d/root/run/owner-beta-paper-ops/output/%s" %
                        (self._owner.pid, self.leaf), follow_symlinks=False)
        require((info.st_dev, info.st_ino) == self.identity == (named.st_dev, named.st_ino)
                and info.st_nlink == 1 and stat.S_IMODE(info.st_mode) == 0o400
                and os.pread(self.fd, len(self.bytes) + 1, 0) == self.bytes, "FAULT_RESULT_CHANGED")


class FaultObserver:
    SEIZE, INTERRUPT, DETACH, SYSCALL = 0x4206, 0x4207, 17, 24
    GETEVENTMSG, GETINFO = 0x4201, 0x420E
    OPTIONS = 1 | 8 | 64  # TRACESYSGOOD, TRACECLONE, TRACEEXIT
    WALL = 0x40000000

    def __init__(self, cid, kernel_pid, ticks, output_identity, deadline_ns):
        require(type(cid) is str and re.fullmatch("[0-9a-f]{64}", cid), "FAULT_CID")
        require(type(kernel_pid) is int and kernel_pid > 1 and kernel_pid != os.getpid(),
                "FAULT_PID")
        require(type(ticks) is int and type(deadline_ns) is int, "FAULT_CONTEXT")
        self.cid, self.pid, self.ticks = cid, kernel_pid, ticks
        self.output_identity, self.deadline = tuple(output_identity), deadline_ns
        self.tids, self.stopped, self.pending = set(), set(), {}
        self.auto_pending = set()
        self.token, self.closed = None, False
        self.retained_fds = []
        self.libc = ctypes.CDLL(None, use_errno=True)
        self.libc.ptrace.restype = ctypes.c_long

    def _bound(self):
        require(time.monotonic_ns() < self.deadline, "FAULT_DEADLINE")

    def _identity(self):
        self._bound()
        require(start_ticks(self.pid) == self.ticks, "FAULT_PID_REUSED")
        cgroup = pathlib.Path("/proc/%d/cgroup" % self.pid).read_text()
        require(self.cid in cgroup, "FAULT_FOREIGN_PROCESS")
        executable = os.readlink("/proc/%d/exe" % self.pid)
        require(executable.endswith("/owner-beta-paper-ops-native-bin")
                and not executable.endswith(" (deleted)"), "FAULT_HELPER_EXE")
        path = "/proc/%d/root/run/owner-beta-paper-ops/output" % self.pid
        require(directory_identity(path) == self.output_identity, "FAULT_OUTPUT_IDENTITY")

    def _ptrace(self, request, tid, address=0, data=0):
        ctypes.set_errno(0)
        result = self.libc.ptrace(ctypes.c_uint(request), ctypes.c_uint(tid),
                                  ctypes.c_void_p(address), ctypes.c_void_p(data))
        if result == -1 and ctypes.get_errno():
            raise FaultError("FAULT_PTRACE_%d" % ctypes.get_errno())
        return result

    def _wait(self):
        while True:
            self._bound()
            for tid in tuple(self.tids):
                try:
                    found, status = os.waitpid(tid, os.WNOHANG | self.WALL)
                except ChildProcessError:
                    raise FaultError("FAULT_TRACE_LOST") from None
                if not found:
                    continue
                if os.WIFEXITED(status) or os.WIFSIGNALED(status):
                    self.tids.discard(tid)
                    self.stopped.discard(tid)
                    self.pending.pop(tid, None)
                    continue
                require(os.WIFSTOPPED(status), "FAULT_WAIT_STATE")
                self.stopped.add(tid)
                self.auto_pending.discard(tid)
                return tid, status
            require(self.tids, "FAULT_HELPER_EXITED")
            time.sleep(0.001)

    def _clone(self, tid):
        number = ctypes.c_ulonglong()
        self._ptrace(self.GETEVENTMSG, tid, data=ctypes.addressof(number))
        child = number.value
        require(child > 1, "FAULT_CLONE")
        self.tids.add(child)
        self.auto_pending.add(child)

    def _resume_tid(self, tid, delivered_signal=0):
        self._ptrace(self.SYSCALL, tid, data=delivered_signal)
        self.stopped.discard(tid)

    def _reap_exited(self):
        for tid in tuple(self.tids):
            try:
                found, status = os.waitpid(tid, os.WNOHANG | self.WALL)
            except ChildProcessError:
                require(not pathlib.Path("/proc/%d" % tid).exists(), "FAULT_TRACE_LOST")
                found, status = tid, 0
            if found:
                if os.WIFEXITED(status) or os.WIFSIGNALED(status):
                    self.tids.discard(tid)
                    self.stopped.discard(tid)
                    self.pending.pop(tid, None)
                else:
                    require(os.WIFSTOPPED(status), "FAULT_WAIT_STATE")
                    self.stopped.add(tid)
                    self.auto_pending.discard(tid)
                    if status >> 16 == 3:
                        self._clone(tid)

    def _stop_all(self):
        self._reap_exited()
        for tid in self.tids - self.stopped - self.auto_pending:
            self._ptrace(self.INTERRUPT, tid)
        while self.tids - self.stopped:
            tid, status = self._wait()
            if status >> 16 == 3:
                self._clone(tid)

    def __enter__(self):
        require(os.geteuid() == 0 and platform.machine() == "x86_64", "FAULT_ROOT_X86_REQUIRED")
        self._identity()
        try:
            while True:
                current = {int(p.name) for p in pathlib.Path("/proc/%d/task" % self.pid).iterdir()}
                new = current - self.tids
                for tid in new:
                    try:
                        self._ptrace(self.SEIZE, tid, data=self.OPTIONS)
                    except FaultError as error:
                        # TRACECLONE may have attached this new task before its
                        # parent event is consumed. Accept only our own tracer.
                        status = pathlib.Path("/proc/%d/status" % tid).read_text()
                        tracer = re.search(r"^TracerPid:\s+([0-9]+)$", status, re.M)
                        require(str(error) == "FAULT_PTRACE_1" and tracer is not None
                                and int(tracer[1]) == os.getpid(), "FAULT_FOREIGN_TRACER")
                    self.tids.add(tid)
                self._stop_all()
                again = {int(p.name) for p in pathlib.Path("/proc/%d/task" % self.pid).iterdir()}
                if again <= self.tids:
                    break
            self._identity()
            return self
        except BaseException:
            self.close()
            raise

    def _candidate(self, tid, number):
        path = "/proc/%d/fd/%d" % (tid, number)
        target = os.readlink(path)
        prefix = "/run/owner-beta-paper-ops/output/"
        if not target.startswith(prefix):
            return None
        leaf = target[len(prefix):]
        if not re.fullmatch("[0-9a-f]{64}\\.json", leaf):
            return None
        info = os.stat(path)
        if not stat.S_ISREG(info.st_mode):
            return None
        self._identity()
        require(info.st_uid == 0 and info.st_gid == 0 and info.st_nlink == 1
                and stat.S_IMODE(info.st_mode) == 0o400, "FAULT_RESULT_METADATA")
        return number, leaf, (info.st_dev, info.st_ino)

    def wait_for_result_fsync(self):
        require(not self.closed and self.token is None, "FAULT_OBSERVER_STATE")
        for tid in tuple(self.stopped):
            self._resume_tid(tid)
        while True:
            tid, status = self._wait()
            event = status >> 16
            if event:
                if event == 3:
                    self._clone(tid)
                self._resume_tid(tid)
                continue
            signum = os.WSTOPSIG(status)
            if signum != signal.SIGTRAP | 0x80:
                self._resume_tid(tid, 0 if signum in (signal.SIGTRAP, signal.SIGSTOP) else signum)
                continue
            buffer = ctypes.create_string_buffer(128)
            length = self._ptrace(self.GETINFO, tid, 128, ctypes.addressof(buffer))
            phase, number, args = syscall_record(buffer.raw[:length])
            if phase == "entry":
                self.pending.pop(tid, None)
                if number == 74:  # x86_64 fsync
                    candidate = self._candidate(tid, args[0])
                    if candidate is not None:
                        self.pending[tid] = candidate
            elif tid in self.pending:
                candidate = self.pending.pop(tid)
                require(number == 0 and not args, "FAULT_FSYNC_FAILED")
                self._stop_all()
                fd_number, leaf, identity = candidate
                fd = os.open("/proc/%d/fd/%d" % (tid, fd_number), os.O_RDONLY | os.O_CLOEXEC)
                try:
                    info = os.fstat(fd)
                    require((info.st_dev, info.st_ino) == identity, "FAULT_FD_REPLACED")
                    data = os.pread(fd, 65537, 0)
                    value, digest = result_identity(data, leaf)
                    self.token = StoppedResult(self, tid, fd, leaf, identity, data, value, digest)
                    return self.token
                except BaseException:
                    os.close(fd)
                    raise
            self._resume_tid(tid)

    def resume(self):
        # SYSCALL would immediately stop again at the next syscall. Detach all
        # stopped owned threads to let the real helper finish without a race.
        require(self.token is not None and self.stopped == self.tids, "FAULT_RESUME_STATE")
        self.retained_fds.append(self.token.fd)
        self.token = None
        for tid in tuple(self.stopped):
            self._ptrace(self.DETACH, tid)
            self.tids.discard(tid)
            self.stopped.discard(tid)

    def release_results(self):
        """Fixture supervisor calls only after helper/backend disposition proof."""
        require(self.closed, "FAULT_TRACE_STILL_OWNED")
        while self.retained_fds:
            os.close(self.retained_fds.pop())

    def close(self):
        if self.closed:
            return
        # The caller reserves 120s outside the action deadline for disposal.
        # Detach gets its own bounded ten-second slice, never a new action.
        self.deadline = max(self.deadline, time.monotonic_ns() + 10_000_000_000)
        # Never kill a tracee or infer its DB disposition from detach.
        if self.token is not None:
            self.retained_fds.append(self.token.fd)
            self.token = None
        errors = []
        if self.tids:
            try:
                self._stop_all()
            except (FaultError, OSError) as error:
                errors.append(error)
            for tid in tuple(self.stopped):
                try:
                    self._ptrace(self.DETACH, tid)
                    self.tids.discard(tid)
                    self.stopped.discard(tid)
                except FaultError as error:
                    if not pathlib.Path("/proc/%d" % tid).exists():
                        self.tids.discard(tid)
                        self.stopped.discard(tid)
                    else:
                        errors.append(error)
        self.closed = not self.tids
        require(self.closed and not errors, "FAULT_DETACH_UNPROVEN")

    def __exit__(self, *_exc):
        self.close()


def arm(cid, kernel_pid, start_ticks, action_output_dir_identity, deadline_ns):
    return FaultObserver(cid, kernel_pid, start_ticks, action_output_dir_identity, deadline_ns)
