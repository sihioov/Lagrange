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
import selectors
import socket
import stat
import struct
import subprocess
import sys
import threading
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


class ProductionPhases:
    """Observe exact real Python frames without replacing a production callable.

    A callback may read actual effects or damage an owned fixture resource. It
    cannot change a function's arguments, result, frame locals or authority.
    Exceptions are real cancellation/failure at that phase, never success.
    """
    def __init__(self, functions, callback, deadline_ns):
        self.codes = {id(function.__code__): (function.__code__, function.__globals__, name)
                      for name, function in functions.items()}
        require(len(self.codes) == len(functions), "FAULT_PHASE_DUPLICATE")
        self.callback, self.deadline = callback, deadline_ns
        self.thread = None

    def observe(self, frame, event, result):
        binding = self.codes.get(id(frame.f_code))
        if binding is None or binding[0] is not frame.f_code or binding[1] is not frame.f_globals or event not in {
                "call", "return", "c_call", "c_return", "c_exception"}:
            return
        require(threading.get_ident() == self.thread and time.monotonic_ns() < self.deadline,
                "FAULT_PHASE_DEADLINE")
        self.callback(binding[2], event, frame, result)

    def __enter__(self):
        require(sys.getprofile() is None and self.thread is None, "FAULT_PHASE_ALREADY_OWNED")
        self.thread = threading.get_ident()
        sys.setprofile(self.observe)
        return self

    def __exit__(self, *_error):
        current = sys.getprofile()
        # CPython clears the profile function when a callback raises. That is
        # cancellation, not evidence that a phase or resource completed.
        require(current is None or current == self.observe, "FAULT_PHASE_CHANGED")
        sys.setprofile(None)
        self.thread = None


class RetainedMemberFault:
    """One actual byte/inode/name fault, restored only by its original owner."""
    def __init__(self, original, kind, *, member=None):
        require(kind in {"ciphertext", "missing-member", "changed-member"}, "FAULT_MEMBER_KIND")
        if member is None:
            original.revalidate(None)
            self.path, descriptor, size = pathlib.Path(original.path), original.fd, original.size
        else:
            require(member == "postgres.dump.enc", "FAULT_MEMBER_KIND")
            original.revalidate()
            descriptor, _identity, size, _digest = original.files[member]
            self.path = pathlib.Path(original.path) / member
        self.parent_identity = directory_identity(self.path.parent)
        self.identity = (os.fstat(descriptor).st_dev, os.fstat(descriptor).st_ino)
        self.saved = self.path.with_name(".fixture-held-" + uuid.uuid4().hex)
        self.kind, self.byte, self.closed = kind, None, False
        if kind == "ciphertext":
            require(size > 0, "FAULT_EMPTY_CIPHERTEXT")
            fd = os.open(self.path, os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC)
            try:
                require((os.fstat(fd).st_dev, os.fstat(fd).st_ino) == self.identity, "FAULT_MEMBER_CHANGED")
                self.byte = os.pread(fd, 1, size - 1)
                self.offset = size - 1
                require(len(self.byte) == 1, "FAULT_EMPTY_CIPHERTEXT")
                os.pwrite(fd, bytes([self.byte[0] ^ 1]), self.offset)
                os.fsync(fd)
            finally:
                os.close(fd)
        else:
            self.path.rename(self.saved)
            if kind == "changed-member":
                info = os.stat(self.saved, follow_symlinks=False)
                fd = os.open(self.path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, stat.S_IMODE(info.st_mode))
                try:
                    os.fchown(fd, info.st_uid, info.st_gid)
                    offset = 0
                    while data := os.pread(descriptor, 1024 * 1024, offset):
                        view = memoryview(data)
                        while view:
                            view = view[os.write(fd, view):]
                        offset += len(data)
                    os.fsync(fd)
                    require(os.fstat(fd).st_ino != self.identity[1], "FAULT_MEMBER_CHANGED")
                finally:
                    os.close(fd)
                self.replacement_identity = (self.path.lstat().st_dev, self.path.lstat().st_ino)

    def close(self):
        if self.closed:
            return
        require(directory_identity(self.path.parent) == self.parent_identity, "FAULT_MEMBER_PARENT_CHANGED")
        if self.kind == "ciphertext":
            fd = os.open(self.path, os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC)
            try:
                require((os.fstat(fd).st_dev, os.fstat(fd).st_ino) == self.identity, "FAULT_MEMBER_CHANGED")
                os.pwrite(fd, self.byte, self.offset)
                os.fsync(fd)
            finally:
                os.close(fd)
        else:
            require((self.saved.lstat().st_dev, self.saved.lstat().st_ino) == self.identity, "FAULT_MEMBER_CHANGED")
            if self.kind == "changed-member":
                require((self.path.lstat().st_dev, self.path.lstat().st_ino) == self.replacement_identity,
                        "FAULT_MEMBER_CHANGED")
                self.path.unlink()
            require(not self.path.exists() and not self.path.is_symlink(), "FAULT_MEMBER_CHANGED")
            self.saved.rename(self.path)
        self.closed = True


# Linux UAPI: unix_diag.h, sock_diag.h and netlink.h. No kernel access on import.
def unix_diag_request(sequence):
    require(type(sequence) is int and 0 < sequence < 2**32, "FAULT_DIAG_SEQUENCE")
    body = struct.pack("=BBHIIIII", 1, 0, 0, 0xffffffff, 0, 4, 0xffffffff, 0xffffffff)
    return struct.pack("=IHHII", 16 + len(body), 20, 0x301, sequence, 0) + body


def unix_diag_packet(data, sequence, port):
    """Strict multipart decoder; return inode/peer pairs and a terminal marker.

    A peer inode may belong to a different netns (the networkless PG clone).
    Only the helper's local socket is looked up in the retained helper netns.
    No socket path, PG protocol, query or credentials cross this boundary.
    """
    require(type(data) is bytes and 0 < len(data) <= 65536, "FAULT_DIAG_PACKET")
    offset, rows, done = 0, {}, False
    while offset < len(data):
        require(not done and len(data) - offset >= 16, "FAULT_DIAG_LENGTH")
        size, kind, flags, seq, destination = struct.unpack_from("=IHHII", data, offset)
        aligned = (size + 3) & ~3
        require(size >= 16 and offset + aligned <= len(data) and seq == sequence
                and destination == port and flags == 2
                and not any(data[offset + size:offset + aligned]), "FAULT_DIAG_HEADER")
        body = data[offset + 16:offset + size]
        if kind == 3:  # NLMSG_DONE, including the dump's success status
            require(body == struct.pack("=i", 0), "FAULT_DIAG_DONE")
            done = True
        else:
            require(kind == 20 and len(body) >= 16, "FAULT_DIAG_TYPE")
            family, socktype, state, pad, inode, _c0, _c1 = struct.unpack_from("=BBBBIII", body)
            require(family == 1 and pad == 0 and inode > 0 and inode not in rows,
                    "FAULT_DIAG_SOCKET")
            attrs, cursor = {}, 16
            while cursor < len(body):
                require(len(body) - cursor >= 4, "FAULT_DIAG_ATTRIBUTE")
                length, tag = struct.unpack_from("=HH", body, cursor)
                end = cursor + ((length + 3) & ~3)
                require(length >= 4 and end <= len(body) and tag <= 7 and tag not in attrs
                        and not any(body[cursor + length:end]), "FAULT_DIAG_ATTRIBUTE")
                value = body[cursor + 4:cursor + length]
                fixed = {1: 8, 2: 4, 4: 8, 6: 1, 7: 4}
                require((tag not in fixed or len(value) == fixed[tag])
                        and (tag not in {3, 5} or len(value) % 4 == 0), "FAULT_DIAG_ATTRIBUTE")
                attrs[tag] = value
                cursor = end
            peer = struct.unpack("=I", attrs[2])[0] if 2 in attrs else 0
            # Only established SOCK_STREAM peers are eligible for correlation.
            rows[inode] = peer if socktype == 1 and state == 1 else 0
        offset += aligned
    return rows, done


def _unix_probe_child(ns_fd, expected_ns, inodes, deadline_ns):
    """One owned child changes ONLY its network namespace, through an inherited FD."""
    require((os.fstat(ns_fd).st_dev, os.fstat(ns_fd).st_ino) == expected_ns,
            "FAULT_DIAG_NAMESPACE")
    libc = ctypes.CDLL(None, use_errno=True)
    require(libc.setns(ctypes.c_int(ns_fd), ctypes.c_int(0x40000000)) == 0,
            "FAULT_DIAG_SETNS")
    require((os.stat('/proc/self/ns/net').st_dev, os.stat('/proc/self/ns/net').st_ino) == expected_ns,
            "FAULT_DIAG_NAMESPACE")
    os.close(ns_fd)
    sequence = 1
    with socket.socket(socket.AF_NETLINK, socket.SOCK_RAW, 4) as channel:
        channel.bind((0, 0))
        channel.sendto(unix_diag_request(sequence), (0, 0))
        port = channel.getsockname()[0]
        seen, total = {}, 0
        for _ in range(256):
            left = (deadline_ns - time.monotonic_ns()) / 1e9
            require(left > 0, "FAULT_DIAG_DEADLINE")
            channel.settimeout(left)
            packet, _ancillary, flags, sender = channel.recvmsg(65536)
            require(sender == (0, 0) and flags == 0, "FAULT_DIAG_SENDER")
            total += len(packet)
            require(total <= 4 * 1024**2, "FAULT_DIAG_LIMIT")
            rows, done = unix_diag_packet(packet, sequence, port)
            require(not (seen.keys() & rows.keys()), "FAULT_DIAG_DUPLICATE")
            seen.update(rows)
            require(len(seen) <= 16384, "FAULT_DIAG_LIMIT")
            if done:
                require(set(inodes) <= seen.keys(), "FAULT_DIAG_MISSING")
                result = {str(n): seen[n] for n in inodes if seen[n]}
                require(result and len(result) <= 64, "FAULT_DIAG_NO_PEER")
                return json.dumps(result, sort_keys=True, separators=(',', ':')).encode('ascii')
    raise FaultError("FAULT_DIAG_INCOMPLETE")


def unix_socket_inodes(pid):
    found = set()
    entries = list(pathlib.Path('/proc/%d/fd' % pid).iterdir())
    require(len(entries) <= 4096, "FAULT_DIAG_FD_LIMIT")
    for entry in entries:
        # Vanishing descriptors invalidate this observation; don't fill gaps.
        match = re.fullmatch(r'socket:\[([0-9]+)\]', os.readlink(entry))
        if match:
            found.add(int(match[1]))
    require(0 < len(found) <= 64 and all(0 < n < 2**32 for n in found), "FAULT_DIAG_INODES")
    return found


def namespace_identity(pid, kind):
    require(kind in {'net', 'pid'}, "FAULT_DIAG_NAMESPACE")
    info = os.stat('/proc/%d/ns/%s' % (pid, kind))
    return info.st_dev, info.st_ino


def unix_peers(pid, ticks, cid, deadline_ns):
    """Retain namespace FD, spawn/reap one bounded host probe; parent never setns."""
    def identity():
        require(time.monotonic_ns() < deadline_ns and start_ticks(pid) == ticks
                and cid in pathlib.Path('/proc/%d/cgroup' % pid).read_text(), "FAULT_DIAG_GENERATION")
        return namespace_identity(pid, 'net'), namespace_identity(pid, 'pid'), unix_socket_inodes(pid)
    before = identity()
    fd = os.open('/proc/%d/ns/net' % pid, os.O_RDONLY | os.O_CLOEXEC)
    child = None
    try:
        require((os.fstat(fd).st_dev, os.fstat(fd).st_ino) == before[0], "FAULT_DIAG_NAMESPACE")
        child = subprocess.Popen([sys.executable, '-I', str(pathlib.Path(__file__).resolve()),
            '--unix-peer-probe', str(fd), str(before[0][0]), str(before[0][1]), str(deadline_ns),
            *map(str, sorted(before[2]))], pass_fds=(fd,), close_fds=True,
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            env={'PATH': '/usr/bin:/bin', 'LANG': 'C', 'LC_ALL': 'C'})
        # A child cannot produce unbounded output before communicate's limit.
        output = bytearray()
        with selectors.DefaultSelector() as selector:
            selector.register(child.stdout, selectors.EVENT_READ)
            while True:
                left = (deadline_ns - time.monotonic_ns()) / 1e9
                require(left > 0, "FAULT_DIAG_DEADLINE")
                if not selector.select(min(left, 0.05)):
                    continue
                chunk = os.read(child.stdout.fileno(), 4096)
                if not chunk:
                    break
                output.extend(chunk)
                require(len(output) <= 4096, "FAULT_DIAG_LIMIT")
        child.wait(timeout=max(0.001, (deadline_ns-time.monotonic_ns())/1e9))
        require(child.returncode == 0 and identity() == before, "FAULT_DIAG_CHANGED")
        value = json.loads(output, object_pairs_hook=_unique)
        require(type(value) is dict and value and len(value) <= 64
                and all(re.fullmatch('[1-9][0-9]*', k) and int(k) in before[2]
                        and type(v) is int and 0 < v < 2**32 for k, v in value.items()), "FAULT_DIAG_RESULT")
        return {int(k): v for k, v in value.items()}
    finally:
        if child is not None:
            if child.poll() is None:
                child.kill()
            # Reap the owned probe even on timeout; never detach a namespace child.
            child.wait(timeout=1)
            child.stdout.close()
        os.close(fd)


def unique_unix_backend(peers, candidates):
    """Pure ownership join: candidate=(SQL identity, host identity, socket inodes)."""
    require(type(peers) is dict and len(peers) == 1 and all(
                type(k) is int and type(v) is int and 0 < k < 2**32 and 0 < v < 2**32
                for k, v in peers.items()),
            "FAULT_DIAG_AMBIGUOUS")
    matches = [(local, peer, row, process) for local, peer in peers.items()
               for row, process, sockets in candidates if peer in sockets]
    require(len(matches) == 1, "FAULT_DIAG_AMBIGUOUS")
    return matches[0]


if __name__ == '__main__':
    # Private probe protocol: finite integer-only inputs/outputs, no diagnostics.
    try:
        require(7 <= len(sys.argv) <= 70 and sys.argv[1] == '--unix-peer-probe', 'FAULT_DIAG_ARGS')
        numbers = [int(n) for n in sys.argv[2:]]
        fd, device, inode, end, *sockets = numbers
        require(sockets and len(set(sockets)) == len(sockets)
                and all(0 < n < 2**32 for n in sockets), 'FAULT_DIAG_ARGS')
        data = _unix_probe_child(fd, (device, inode), sockets, end)
        os.write(1, data)
    except BaseException:
        sys.exit(1)
