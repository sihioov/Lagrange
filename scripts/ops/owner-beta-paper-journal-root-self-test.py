#!/usr/bin/env python3
"""Finite real-root filesystem verification for the Paper journal.

This runner is intended only for the existing disposable hosted policy job.
It has no public path, fixture, UID, or command selector and never opens a
production install root.
"""
from __future__ import annotations

import errno
import fcntl
import hashlib
import importlib.util
import json
import os
import select
import signal
import stat
import struct
import sys
import time
import uuid
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
JOURNAL_PATH = ROOT / "scripts/ops/lib/owner-beta-paper-journal.py"
TRACEBACK_PATH_ALLOWLIST = {
    str(Path(__file__).resolve()): "scripts/ops/owner-beta-paper-journal-root-self-test.py",
    str(JOURNAL_PATH.resolve()): "scripts/ops/lib/owner-beta-paper-journal.py",
}
MAX_TRACEBACK_LINE = 4096
EXPECTED_JOURNAL_SHA256 = "4e8d9bf5684d784c189ba758e5b7665e70c8b980aef3300a2159bdfece0ffe85"
ANCHOR_PARENT = "/var/lib"
ANCHOR_PREFIX = "lagrange-paper-journal-"
CHILD_TIMEOUT = 5.0
OVERALL_TIMEOUT = 180.0
EXPECTED_CASES = (
    "BOOTSTRAP_PERSISTENCE",
    "NO_REBOOTSTRAP_WITH_HISTORY",
    "IMMUTABLE_IDEMPOTENCE_CONFLICT",
    "BOUND_CALLER_ACTION_PUBLICATION",
    "LOCK_INHERITED_LIFETIME",
    "LINK_OWNER_MODE_AND_ANCESTOR_REJECTION",
    "NAMED_INODE_REPLACEMENT",
    "POSIX_ACL_REJECTION",
    "STAGE_CHAIN_GAP_FORK_MUTATION",
    "FLOOR_MONOTONICITY_OVERFLOW",
    "NOREPLACE_FSYNC_FAILURE_CLEANUP",
    "REAL_UID_DROP_GATE",
)


def load_journal():
    spec = importlib.util.spec_from_file_location("paper_journal_root_tested", JOURNAL_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("JOURNAL_IMPORT_UNAVAILABLE")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


journal = load_journal()


def canonical(value):
    return json.dumps(value, ensure_ascii=True, sort_keys=True,
                      separators=(",", ":"), allow_nan=False).encode("ascii")


def digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def require_file_observation(path, expected, mode):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        info = os.fstat(fd)
        require(stat.S_ISREG(info.st_mode), "OBSERVED_OUTPUT_NOT_REGULAR")
        require((info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode), info.st_nlink)
                == (0, 0, mode, 1), "OBSERVED_OUTPUT_METADATA_MISMATCH")
        for attribute in ("system.posix_acl_access", "system.posix_acl_default"):
            try:
                os.getxattr(fd, attribute)
            except OSError as error:
                if error.errno in (errno.ENODATA, errno.ENOTSUP, errno.EOPNOTSUPP):
                    continue
                raise CaseFailure("OBSERVED_OUTPUT_ACL_CHECK_UNAVAILABLE") from None
            raise CaseFailure("OBSERVED_OUTPUT_ACL_PRESENT")
        content = bytearray()
        while True:
            chunk = os.read(fd, 65536)
            if not chunk:
                break
            content.extend(chunk)
        require(bytes(content) == expected, "OBSERVED_OUTPUT_BYTES_MISMATCH")
    finally:
        os.close(fd)


def fixture_component(value):
    if not isinstance(value, str) or len(value) != 36:
        raise ValueError("FIXTURE_NAME_INVALID")
    parsed = uuid.UUID(value)
    if str(parsed) != value:
        raise ValueError("FIXTURE_NAME_INVALID")
    return value


def anchor_basename(nonce):
    return ANCHOR_PREFIX + fixture_component(nonce)


def directory_identity(info):
    return (info.st_dev, info.st_ino, info.st_uid, info.st_gid, info.st_mode)


def assert_root_directory(fd, *, exact_mode=None):
    info = os.fstat(fd)
    if (not stat.S_ISDIR(info.st_mode) or info.st_uid != 0 or info.st_gid != 0
            or info.st_mode & 0o022
            or (exact_mode is not None and stat.S_IMODE(info.st_mode) != exact_mode)):
        raise RuntimeError("FIXTURE_DIRECTORY_UNSAFE")
    for attribute in ("system.posix_acl_access", "system.posix_acl_default"):
        try:
            os.getxattr(fd, attribute)
        except OSError as error:
            if error.errno in (errno.ENODATA, errno.ENOTSUP, errno.EOPNOTSUPP):
                continue
            raise RuntimeError("FIXTURE_ACL_CHECK_UNAVAILABLE") from None
        raise RuntimeError("FIXTURE_DIRECTORY_ACL_PRESENT")
    return info


def create_anchor(parent_fd, nonce):
    """Create a unique owned anchor with mkdirat; never adopt an existing name."""
    name = anchor_basename(nonce)
    os.mkdir(name, 0o700, dir_fd=parent_fd)
    try:
        fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                     dir_fd=parent_fd)
    except BaseException:
        try:
            entry = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
            if (stat.S_ISDIR(entry.st_mode) and entry.st_uid == 0 and entry.st_gid == 0
                    and stat.S_IMODE(entry.st_mode) == 0o700):
                os.rmdir(name, dir_fd=parent_fd)
        except OSError:
            pass
        raise
    identity = directory_identity(os.fstat(fd))
    try:
        assert_root_directory(fd, exact_mode=0o700)
        named = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
        if directory_identity(named) != identity:
            raise RuntimeError("FIXTURE_ANCHOR_CHANGED")
        return name, fd, identity
    except BaseException:
        try:
            named = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
            if directory_identity(named) == identity and not os.listdir(fd):
                os.close(fd)
                fd = -1
                os.rmdir(name, dir_fd=parent_fd)
        finally:
            if fd >= 0:
                os.close(fd)
        raise


def remove_owned_contents(directory_fd):
    """Remove only direct children reached without following symlinks."""
    assert_root_directory(directory_fd, exact_mode=0o700)
    for name in os.listdir(directory_fd):
        if name in (".", "..") or "/" in name or "\x00" in name:
            raise RuntimeError("CLEANUP_NAME_INVALID")
        try:
            before = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
        except FileNotFoundError:
            raise RuntimeError("CLEANUP_ENTRY_CHANGED") from None
        if stat.S_ISDIR(before.st_mode):
            child_fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                               dir_fd=directory_fd)
            try:
                child_before = directory_identity(assert_root_directory(child_fd, exact_mode=0o700))
                if child_before != directory_identity(before):
                    raise RuntimeError("CLEANUP_ENTRY_CHANGED")
                remove_owned_contents(child_fd)
                child_after = os.fstat(child_fd)
                named_after = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
                if (directory_identity(child_after) != child_before
                        or directory_identity(named_after) != child_before):
                    raise RuntimeError("CLEANUP_ENTRY_CHANGED")
            finally:
                os.close(child_fd)
            os.rmdir(name, dir_fd=directory_fd)
        else:
            now = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
            if (now.st_dev, now.st_ino, now.st_uid, now.st_gid, now.st_mode, now.st_nlink) != (
                    before.st_dev, before.st_ino, before.st_uid, before.st_gid,
                    before.st_mode, before.st_nlink):
                raise RuntimeError("CLEANUP_ENTRY_CHANGED")
            if not (stat.S_ISREG(now.st_mode) or stat.S_ISLNK(now.st_mode)):
                raise RuntimeError("CLEANUP_ENTRY_TYPE")
            os.unlink(name, dir_fd=directory_fd)
    os.fsync(directory_fd)


def cleanup_anchor(parent_fd, name, anchor_fd, expected_identity):
    """Delete a fixture only while its path still names our retained inode."""
    if OWNED_CHILDREN:
        raise RuntimeError("CLEANUP_CHILDREN_ACTIVE")
    assert_root_directory(anchor_fd, exact_mode=0o700)
    retained = directory_identity(os.fstat(anchor_fd))
    named = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    if retained != expected_identity or directory_identity(named) != expected_identity:
        raise RuntimeError("CLEANUP_ANCHOR_IDENTITY")
    remove_owned_contents(anchor_fd)
    named = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    if directory_identity(named) != expected_identity:
        raise RuntimeError("CLEANUP_ANCHOR_IDENTITY")
    os.rmdir(name, dir_fd=parent_fd)
    os.fsync(parent_fd)


OWNED_CHILDREN = {}
STOP_SIGNAL = None
CLEANING_UP = False
CHILD_REAP_TIMEOUT = 2.0
CHILD_POLL_INTERVAL = 0.02


def _track_child(pid, read_fd):
    if pid <= 0 or pid in OWNED_CHILDREN:
        raise RuntimeError("CHILD_TRACKING_INVALID")
    OWNED_CHILDREN[pid] = {"read_fd": read_fd, "reaped": False, "status": None}


def _close_child_pipe(pid):
    child = OWNED_CHILDREN.get(pid)
    if child is None or child["read_fd"] is None:
        return
    fd = child["read_fd"]
    child["read_fd"] = None
    try:
        os.close(fd)
    except OSError:
        pass


def _finish_reaped_child(pid):
    child = OWNED_CHILDREN.get(pid)
    if child is not None and child["reaped"]:
        _close_child_pipe(pid)
        OWNED_CHILDREN.pop(pid, None)


def _poll_owned_child(pid, deadline):
    """Use WNOHANG only for a registered child; retain it until waitpid reaps it."""
    while True:
        child = OWNED_CHILDREN.get(pid)
        if child is None:
            return True, None
        if child["reaped"]:
            return True, child["status"]
        try:
            result, status = os.waitpid(pid, os.WNOHANG)
        except InterruptedError:
            continue
        except ChildProcessError:
            # It is gone from our wait set, so never signal this numeric PID again.
            child["reaped"] = True
            child["status"] = None
            return True, None
        if result == pid:
            child["reaped"] = True
            child["status"] = status
            return True, status
        if time.monotonic() >= deadline:
            return False, None
        remaining = max(0.0, min(CHILD_POLL_INTERVAL, deadline - time.monotonic()))
        try:
            select.select([], [], [], remaining)
        except InterruptedError:
            continue


def _signal_owned_child(pid, signum):
    child = OWNED_CHILDREN.get(pid)
    if child is None or child["reaped"]:
        return
    try:
        os.kill(pid, signum)
    except ProcessLookupError:
        pass


def _terminate_and_reap(pid, timeout=CHILD_REAP_TIMEOUT):
    child = OWNED_CHILDREN.get(pid)
    if child is None:
        return True
    if child["reaped"]:
        _finish_reaped_child(pid)
        return True
    try:
        _signal_owned_child(pid, signal.SIGKILL)
        reaped, _status = _poll_owned_child(pid, time.monotonic() + timeout)
        if reaped:
            _finish_reaped_child(pid)
            return True
    except BaseException:
        pass
    # Close our pipe even if an abnormal kernel state prevents reap. Keep the PID
    # registered so fixture cleanup is forbidden and a later reap can retry.
    _close_child_pipe(pid)
    return False


def reap_owned_children(timeout=CHILD_REAP_TIMEOUT):
    """Kill and reap only registered children, retaining any PID not yet reaped."""
    deadline = time.monotonic() + timeout
    all_reaped = True
    for pid in tuple(OWNED_CHILDREN):
        remaining = max(0.0, deadline - time.monotonic())
        try:
            reaped = _terminate_and_reap(pid, remaining)
        except BaseException:
            _close_child_pipe(pid)
            reaped = False
        if not reaped:
            all_reaped = False
    return all_reaped and not OWNED_CHILDREN


def wait_owned_child(pid, read_fd, timeout=CHILD_TIMEOUT, *, after_payload=None):
    """Read and validate a fixed child result, then reap it under one deadline."""
    if pid not in OWNED_CHILDREN:
        _track_child(pid, read_fd)
    elif OWNED_CHILDREN[pid]["read_fd"] != read_fd:
        raise RuntimeError("CHILD_PIPE_IDENTITY")
    deadline = time.monotonic() + timeout
    payload = bytearray()
    eof = False
    callback_called = False
    invalid_size = False
    result = "CHILD_IO_ERROR"
    try:
        while True:
            child = OWNED_CHILDREN.get(pid)
            if child is None:
                return "CHILD_TRACKING_LOST"
            if STOP_SIGNAL is not None:
                result = "CHILD_CANCELLED"
                return result if _terminate_and_reap(pid) else "CHILD_REAP_TIMEOUT"
            if not eof:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    result = "CHILD_TIMEOUT"
                    return result if _terminate_and_reap(pid) else "CHILD_REAP_TIMEOUT"
                fd = child["read_fd"]
                if fd is None:
                    result = "CHILD_PIPE_CLOSED"
                    return result if _terminate_and_reap(pid) else "CHILD_REAP_TIMEOUT"
                try:
                    ready, _, _ = select.select([fd], [], [], min(remaining, CHILD_POLL_INTERVAL))
                except InterruptedError:
                    continue
                if ready:
                    chunk = os.read(fd, 129)
                    if not chunk:
                        eof = True
                    else:
                        payload.extend(chunk)
                        if len(payload) > 128:
                            invalid_size = True
                        if not callback_called and b"\n" in payload:
                            callback_called = True
                            if after_payload is not None:
                                after_payload()
            reaped, status = _poll_owned_child(pid, deadline)
            if eof and reaped:
                if status is None or not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
                    result = "CHILD_EXIT_INVALID"
                elif (invalid_size or not payload or not payload.endswith(b"\n")
                      or payload.count(b"\n") != 1):
                    result = "CHILD_RESULT_INVALID"
                else:
                    try:
                        decoded = bytes(payload[:-1]).decode("ascii", "strict")
                    except UnicodeError:
                        result = "CHILD_RESULT_INVALID"
                    else:
                        if not decoded or any(ch not in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_:-"
                                              for ch in decoded):
                            result = "CHILD_RESULT_INVALID"
                        else:
                            result = decoded
                _finish_reaped_child(pid)
                return result
            if time.monotonic() >= deadline:
                result = "CHILD_TIMEOUT"
                return result if _terminate_and_reap(pid) else "CHILD_REAP_TIMEOUT"
    except ControlledStop:
        return "CHILD_CANCELLED" if _terminate_and_reap(pid) else "CHILD_REAP_TIMEOUT"
    except BaseException:
        reaped = _terminate_and_reap(pid)
        return "CHILD_INTERRUPTED" if reaped else "CHILD_REAP_TIMEOUT"
    finally:
        child = OWNED_CHILDREN.get(pid)
        if child is not None:
            _close_child_pipe(pid)
            if child["reaped"]:
                OWNED_CHILDREN.pop(pid, None)


def _write_child_result(fd, value):
    os.write(fd, (value + "\n").encode("ascii"))


def fork_probe(kind, install_path, lock_fd, timeout=CHILD_TIMEOUT):
    read_fd, write_fd = os.pipe2(os.O_CLOEXEC)
    try:
        pid = os.fork()
    except OSError:
        os.close(read_fd)
        os.close(write_fd)
        raise CaseFailure("OWNED_CHILD_UNAVAILABLE") from None
    if pid == 0:
        os.close(read_fd)
        result = "CHILD_INTERNAL_ERROR"
        try:
            if kind == "LOCK":
                candidate = os.open(os.path.join(install_path, journal.LOCK_NAME),
                                    os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC)
                try:
                    try:
                        fcntl.flock(candidate, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    except BlockingIOError:
                        result = "LOCK_HELD"
                    else:
                        result = "LOCK_ACQUIRED"
                finally:
                    os.close(candidate)
            elif kind == "UID_DROP":
                if not (hasattr(os, "setgid") and hasattr(os, "setuid")):
                    result = "UID_DROP_UNAVAILABLE"
                else:
                    os.setgroups([])
                    os.setgid(65534)
                    os.setuid(65534)
                    try:
                        with journal.open_journal(install_path, lock_fd, bootstrap=False):
                            result = "UNEXPECTED_JOURNAL_OPEN"
                    except journal.JournalError as error:
                        result = "ROOT_GATE_" + error.code
            else:
                result = "CHILD_KIND_INVALID"
        except OSError as error:
            result = "CHILD_OSERR_" + str(error.errno or 0)
        except BaseException:
            result = "CHILD_INTERNAL_ERROR"
        try:
            _write_child_result(write_fd, result)
        except BaseException:
            pass
        os.close(write_fd)
        os._exit(0)
    _track_child(pid, read_fd)
    os.close(write_fd)
    return wait_owned_child(pid, read_fd, timeout)


class Fixture:
    def __init__(self, anchor_fd, label):
        self.path = None
        self.fd = None
        self.lock_fd = None
        name = "install-" + label.lower().replace("_", "-")
        self.name = name
        try:
            os.mkdir(name, 0o700, dir_fd=anchor_fd)
            self.fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                              dir_fd=anchor_fd)
            assert_root_directory(self.fd, exact_mode=0o700)
            # The fixed /var/lib child is the only install path ever passed to the journal.
            self.path = os.path.realpath("/proc/self/fd/" + str(self.fd))
            anchor_path = os.path.realpath("/proc/self/fd/" + str(anchor_fd))
            if self.path != os.path.join(anchor_path, name):
                raise RuntimeError("FIXTURE_PATH_IDENTITY")
            self.lock_fd = os.open(journal.LOCK_NAME,
                                   os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                                   0o600, dir_fd=self.fd)
            os.fchmod(self.lock_fd, 0o600)
            os.fsync(self.lock_fd)
            if os.fstat(self.lock_fd).st_size != 0:
                raise RuntimeError("FIXTURE_LOCK_INVALID")
            fcntl.flock(self.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BaseException:
            self.close()
            raise

    def close(self):
        if self.lock_fd is not None:
            try:
                fcntl.flock(self.lock_fd, fcntl.LOCK_UN)
            finally:
                os.close(self.lock_fd)
                self.lock_fd = None
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None

    def open(self, bootstrap=True):
        return journal.open_journal(self.path, self.lock_fd, bootstrap=bootstrap)

    def open_once(self, bootstrap=True):
        with self.open(bootstrap=bootstrap):
            return None

    def intent(self, installation_id, *, generation=None, request_key=None, epoch=0):
        generation = generation or str(uuid.uuid4())
        return canonical({
            "schema": "owner-beta-paper-operation-intent-v1",
            "installation_id": installation_id,
            "generation": generation,
            "account_id": "33333333-3333-4333-8333-333333333333",
            "predecessor_generation": None,
            "request_key": request_key or generation + ":begin",
            "purpose": "STOP",
            "scope": "PAPER_STOP",
            "expected_epoch": epoch,
            "local_epoch_floor": epoch,
            "release_manifest_sha256": "sha256:" + "a" * 64,
            "artifact_sha256": None,
            "resume_target_id": None,
        })

    def stage(self, operation, installation_id, name, sequence, previous_hash):
        return canonical({
            "schema": "owner-beta-paper-operation-stage-v1",
            "installation_id": installation_id,
            "generation": operation.generation,
            "account_id": "33333333-3333-4333-8333-333333333333",
            "sequence": sequence,
            "previous_sha256": previous_hash,
            "intent_sha256": operation.intent_sha256,
            "stage": name,
            "boot_id": "44444444-4444-4444-8444-444444444444",
            "observed": {"realtime_ns": sequence + 1,
                         "monotonic_ns": sequence + 2,
                         "boottime_ns": sequence + 3},
            "control_epoch": 0,
            "artifact_sha256": None,
            "event_id": None,
            "error_code": "NONE",
        })


class CaseFailure(Exception):
    def __init__(self, code):
        self.code = code
        super().__init__(code)


def require(condition, code):
    if not condition:
        raise CaseFailure(code)


def expect_error(action, code):
    try:
        action()
    except journal.JournalError as error:
        if error.code == code:
            return
        raise CaseFailure("EXPECTED_" + code + "_GOT_" + error.code) from None
    except BaseException:
        raise CaseFailure("EXPECTED_" + code + "_GOT_OTHER") from None
    raise CaseFailure("EXPECTED_" + code + "_GOT_ACCEPTED")


def expect_rejected(action, journal_code=None, errno_code=None):
    try:
        action()
    except journal.JournalError as error:
        if journal_code is not None and error.code == journal_code:
            return
        raise CaseFailure("WRONG_TYPED_REJECTION") from None
    except OSError as error:
        if errno_code is not None and error.errno == errno_code:
            return
        raise CaseFailure("WRONG_OS_REJECTION") from None
    raise CaseFailure("UNSAFE_INPUT_ACCEPTED")


def expect_context_exit_error(context, code, body):
    """Require a complete negative check, then the same typed exit recheck."""
    before = descriptor_count()
    body_completed = False
    try:
        with context as value:
            body(value)
            body_completed = True
    except journal.JournalError as error:
        if not body_completed:
            raise CaseFailure("CONTEXT_REJECTED_BEFORE_BODY") from None
        if error.code != code:
            raise CaseFailure("CONTEXT_EXIT_EXPECTED_" + code + "_GOT_" + error.code) from None
    else:
        raise CaseFailure("CONTEXT_EXIT_EXPECTED_" + code + "_ACCEPTED")
    after = descriptor_count()
    if before is None or after is None:
        raise CaseFailure("CONTEXT_DESCRIPTOR_CHECK_UNAVAILABLE")
    require(after == before, "CONTEXT_DESCRIPTOR_LEAK")


def new_fixture(anchor_fd, label):
    return Fixture(anchor_fd, label)


def case_bootstrap(anchor_fd):
    fixture = new_fixture(anchor_fd, "bootstrap-persistence")
    try:
        with fixture.open() as store:
            identity = store.installation_id
            require(store.epoch_floor == 0, "FRESH_FLOOR_NOT_ZERO")
            require(store.advance_floor(7) == 7, "FLOOR_ADVANCE_FAILED")
            paper = os.path.join(fixture.path, journal.PAPER_DIR)
            require_file_observation(os.path.join(paper, "installation-id"),
                                     (identity + "\n").encode("ascii"), 0o600)
            require_file_observation(os.path.join(paper, "epoch-floor"), b"7\n", 0o600)
        with fixture.open(bootstrap=False) as reopened:
            require(reopened.installation_id == identity, "IDENTITY_NOT_DURABLE")
            require(reopened.epoch_floor == 7, "FLOOR_NOT_DURABLE")
        return "BOOTSTRAP_CLOSE_REOPEN_PASS"
    finally:
        fixture.close()


def case_no_rebootstrap(anchor_fd):
    fixture = new_fixture(anchor_fd, "no-rebootstrap-history")
    try:
        with fixture.open() as store:
            intent = fixture.intent(store.installation_id)
            operation = store.create(intent)
            operation.close()
        os.unlink(os.path.join(fixture.path, journal.PAPER_DIR, "installation-id"))
        with fixture.open() as _store:
            raise CaseFailure("HISTORY_REBOOTSTRAPPED")
    except journal.JournalError as error:
        require(error.code == "JOURNAL_IDENTITY_MISSING", "WRONG_REBOOTSTRAP_REJECTION")
        return "HISTORY_REBOOTSTRAP_REJECTED"
    finally:
        fixture.close()


def case_immutable(anchor_fd):
    fixture = new_fixture(anchor_fd, "immutable-idempotence")
    try:
        with fixture.open() as store:
            intent = fixture.intent(store.installation_id)
            generation = json.loads(intent)["generation"]
            first = store.create(intent)
            first.close()
            same = store.create(intent)
            same.close()
            changed = json.loads(intent)
            changed["request_key"] = generation + ":off"
            expect_error(lambda: store.create(canonical(changed)), "JOURNAL_CONFLICT")
            stored = Path(fixture.path, journal.PAPER_DIR, "operations", generation, "intent.json").read_bytes()
            require(stored == intent, "IMMUTABLE_BYTES_CHANGED")
            require_file_observation(os.path.join(fixture.path, journal.PAPER_DIR, "operations",
                                                  generation, "intent.json"), intent, 0o400)
        return "SAME_BYTES_REUSED_CHANGED_BYTES_REJECTED"
    finally:
        fixture.close()


def case_bound_caller_action(anchor_fd):
    """Real Root publication/FD checks only; no native dispatch or SQL is fabricated."""
    fixture = new_fixture(anchor_fd, "bound-caller-action")
    operation = action = None
    try:
        with fixture.open() as store:
            root_bytes = fixture.intent(store.installation_id)
            root = json.loads(root_bytes)
            caller_key = "root-caller"
            operation = store.bind_caller(caller_key, root_bytes)
            operation_fd = operation.borrow_directory_fd()
            assert_root_directory(operation_fd, exact_mode=0o700)
            original_generation = operation.generation
            original_intent_sha256 = operation.intent_sha256
            projection = canonical({key: value for key, value in root.items()
                                    if key not in ("generation", "request_key")})
            alternate = dict(root, generation=str(uuid.uuid4()))
            alternate["request_key"] = alternate["generation"] + ":begin"
            # A replay opens the existing operation through the public API. The
            # first operation must be closed so the journal's one-active-operation
            # rule remains authoritative rather than producing JOURNAL_BUSY.
            operation.close()
            expect_error(operation.borrow_directory_fd, "JOURNAL_CLOSED")
            try:
                os.fstat(operation_fd)
            except OSError as error:
                require(error.errno == errno.EBADF, "CLOSED_OPERATION_FD_WRONG_ERROR")
            else:
                raise CaseFailure("CLOSED_OPERATION_DIRECTORY_FD_LIVE")
            operation = None
            replay = store.bind_caller(caller_key, canonical(alternate))
            operation = replay
            try:
                require(replay.generation == original_generation, "CALLER_REPLAY_CHANGED_GENERATION")
                require(replay.intent_sha256 == original_intent_sha256 == digest(root_bytes),
                        "CALLER_REPLAY_CHANGED_ORIGINAL")
            finally:
                replay.close()
                operation = None
            changed = dict(root, release_manifest_sha256="sha256:" + "b" * 64)
            expect_error(lambda: store.bind_caller(caller_key, canonical(changed)), "JOURNAL_CONFLICT")
            operation = store.open(original_generation)
            require(operation.intent_sha256 == original_intent_sha256,
                    "CALLER_OPERATION_REOPEN_CHANGED_ORIGINAL")
            caller = {"schema": "owner-beta-paper-caller-binding-v1",
                      "installation_id": store.installation_id, "account_id": root["account_id"],
                      "caller_key": caller_key, "generation": operation.generation,
                      "root_intent_sha256": digest(root_bytes),
                      "command_canonical": projection.decode("ascii")}
            caller_name = digest(canonical({key: caller[key] for key in
                                             ("installation_id", "account_id", "caller_key")}))[7:] + ".json"
            require_file_observation(os.path.join(fixture.path, journal.PAPER_DIR, "callers", caller_name),
                                     canonical(caller), 0o400)
            stage = fixture.stage(operation, store.installation_id, "INTENT", 0, None)
            stage_hash = operation.append(stage)
            binding = canonical({"schema": "owner-beta-paper-action-binding-v1",
                                 "installation_id": store.installation_id,
                                 "generation": operation.generation, "root_intent_sha256": digest(root_bytes),
                                 "sequence": 0, "previous_binding_sha256": None, "verb": "begin",
                                 "request_key": operation.generation + ":begin", "expected_epoch": 0,
                                 "artifact_sha256": None, "stage_sha256": stage_hash})
            action = operation.prepare_action(binding)
            borrowed = action.borrow_output_fd()
            assert_root_directory(borrowed, exact_mode=0o700)
            os.mkdir("bound-inputs", 0o700, dir_fd=fixture.fd)
            inputs_fd = os.open("bound-inputs", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                                dir_fd=fixture.fd)
            try:
                action.stage_inputs(inputs_fd)
                require_file_observation(os.path.join(fixture.path, "bound-inputs", "request.json"),
                                         root_bytes, 0o400)
                require_file_observation(os.path.join(fixture.path, "bound-inputs", "action-binding.json"),
                                         binding, 0o400)
                require(set(os.listdir(inputs_fd)) == {"request.json", "action-binding.json"},
                        "BOUND_INPUTS_UNEXPECTED_ENTRY")
            finally:
                os.close(inputs_fd)
            action.close()
            action = None
            try:
                os.fstat(borrowed)
            except OSError as error:
                require(error.errno == errno.EBADF, "CLOSED_ACTION_FD_WRONG_ERROR")
            else:
                raise CaseFailure("CLOSED_ACTION_OUTPUT_FD_LIVE")
        return "ROOT_CALLER_REPLAY_CONFLICT_AND_ACTION_INPUTS_FD_PASS_NO_NATIVE_AUTHORITY"
    finally:
        if action is not None:
            action.close()
        if operation is not None:
            operation.close()
        fixture.close()


def case_lock_lifetime(anchor_fd):
    fixture = new_fixture(anchor_fd, "lock-inherited-lifetime")
    try:
        before = os.fstat(fixture.lock_fd)
        with fixture.open() as _store:
            require(fork_probe("LOCK", fixture.path, fixture.lock_fd) == "LOCK_HELD",
                    "INDEPENDENT_LOCK_ACQUIRED_DURING_CONTEXT")
        after = os.fstat(fixture.lock_fd)
        require((before.st_dev, before.st_ino) == (after.st_dev, after.st_ino), "BORROWED_FD_CLOSED")
        require(os.stat(journal.LOCK_NAME, dir_fd=fixture.fd, follow_symlinks=False).st_ino == after.st_ino,
                "LOCK_NAME_CHANGED")
        require(fork_probe("LOCK", fixture.path, fixture.lock_fd) == "LOCK_HELD",
                "CONTEXT_EXIT_UNLOCKED_BORROWED_LOCK")
        replacement_fixture = new_fixture(anchor_fd, "lock-named-replacement")
        try:
            def replace_lock_and_prove(store):
                lock_path = os.path.join(replacement_fixture.path, journal.LOCK_NAME)
                saved_path = os.path.join(replacement_fixture.path, "held-lock-inode")
                original = os.fstat(replacement_fixture.lock_fd)
                os.rename(lock_path, saved_path)
                new_fd = os.open(lock_path,
                                 os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                                 0o600)
                os.close(new_fd)
                expect_error(store.revalidate, "JOURNAL_LOCK")
                saved = os.stat(saved_path, follow_symlinks=False)
                named = os.stat(lock_path, follow_symlinks=False)
                require(saved.st_ino == original.st_ino and named.st_ino != original.st_ino,
                        "LOCK_REPLACEMENT_IDENTITY_NOT_PRESERVED")
                require(os.path.exists(saved_path) and os.path.exists(lock_path),
                        "LOCK_REPLACEMENT_PATH_REMOVED")
            expect_context_exit_error(replacement_fixture.open(), "JOURNAL_LOCK",
                                      replace_lock_and_prove)
        finally:
            replacement_fixture.close()
        return "INDEPENDENT_OPEN_BLOCKED_CONTEXT_BORROWED_AND_NAMED_REPLACEMENT_REJECTED"
    finally:
        fixture.close()


def case_links_owners_modes(anchor_fd):
    fixture = new_fixture(anchor_fd, "links-owner-mode")
    try:
        variants = ("symlink", "hardlink", "uid", "gid", "mode")
        for variant in variants:
            case_fixture = new_fixture(anchor_fd, "negative-" + variant)
            try:
                with case_fixture.open() as _store:
                    pass
                floor = os.path.join(case_fixture.path, journal.PAPER_DIR, "epoch-floor")
                external = os.path.join(case_fixture.path, "external")
                Path(external).write_bytes(b"0\n")
                os.chmod(external, 0o600)
                if variant in ("symlink", "hardlink"):
                    os.unlink(floor)
                    if variant == "symlink":
                        os.symlink(external, floor)
                    else:
                        os.link(external, floor)
                elif variant == "uid":
                    os.chown(floor, 65534, 0)
                elif variant == "gid":
                    os.chown(floor, 0, 65534)
                else:
                    os.chmod(floor, 0o640)
                expect_rejected(case_fixture.open_once,
                                journal_code=("JOURNAL_PATH" if variant == "symlink" else "JOURNAL_FILE"))
            finally:
                case_fixture.close()
        for variant in ("ancestor_uid", "ancestor_gid", "ancestor_mode"):
            case_fixture = new_fixture(anchor_fd, "negative-" + variant)
            try:
                if variant == "ancestor_uid":
                    os.fchown(case_fixture.fd, 65534, 0)
                elif variant == "ancestor_gid":
                    os.fchown(case_fixture.fd, 0, 65534)
                else:
                    os.fchmod(case_fixture.fd, 0o770)
                expect_rejected(case_fixture.open_once, journal_code="JOURNAL_DIRECTORY")
            finally:
                os.fchown(case_fixture.fd, 0, 0)
                os.fchmod(case_fixture.fd, 0o700)
                case_fixture.close()
        mode_fixture = new_fixture(anchor_fd, "negative-directory-mode")
        try:
            with mode_fixture.open() as _store:
                pass
            os.chmod(os.path.join(mode_fixture.path, journal.PAPER_DIR, "operations"), 0o750)
            expect_error(mode_fixture.open_once, "JOURNAL_DIRECTORY")
            os.chmod(os.path.join(mode_fixture.path, journal.PAPER_DIR, "operations"), 0o700)
        finally:
            os.chmod(os.path.join(mode_fixture.path, journal.PAPER_DIR, "operations"), 0o700)
            mode_fixture.close()
        # Replace an install-root path component with a symlink and require no-follow rejection.
        moved = fixture.path + "-held"
        os.rename(fixture.path, moved)
        os.symlink(moved, fixture.path)
        expect_rejected(lambda: fixture.open_once(bootstrap=False),
                        journal_code="JOURNAL_PATH")
        return "SYMLINK_HARDLINK_UID_GID_MODE_AND_ANCESTOR_REJECTED"
    finally:
        fixture.close()


def case_named_replacement(anchor_fd):
    fixture = new_fixture(anchor_fd, "named-inode-replacement")
    try:
        def replace_identity_and_prove(store):
            identity = os.path.join(fixture.path, journal.PAPER_DIR, "installation-id")
            replacement = os.path.join(fixture.path, "replacement")
            Path(replacement).write_bytes(Path(identity).read_bytes())
            os.chmod(replacement, 0o600)
            os.replace(replacement, identity)
            expect_error(store.revalidate, "JOURNAL_FILE")
        expect_context_exit_error(fixture.open(), "JOURNAL_FILE", replace_identity_and_prove)

        def replace_paper_directory_and_prove(store):
            paper = os.path.join(fixture.path, journal.PAPER_DIR)
            detached = os.path.join(fixture.path, ".owner-beta-paper-held")
            os.rename(paper, detached)
            os.mkdir(paper, 0o700)
            expect_error(store.revalidate, "JOURNAL_CHANGED")
        expect_context_exit_error(fixture.open(bootstrap=False), "JOURNAL_CHANGED",
                                  replace_paper_directory_and_prove)
        return "RETAINED_FILE_AND_ANCESTOR_NAMED_INODE_REPLACEMENT_REJECTED"
    finally:
        fixture.close()


def make_acl_bytes():
    # Version 2 ACL, with a zero-permission named user and zero mask.  It is
    # a real extended ACL while retaining the expected 0600 mode bits.
    entries = ((1, 6, 0xFFFFFFFF), (2, 0, 65534), (4, 0, 0xFFFFFFFF),
               (16, 0, 0xFFFFFFFF), (32, 0, 0xFFFFFFFF))
    return struct.pack("<I", 2) + b"".join(struct.pack("<HHI", *entry) for entry in entries)


def case_acl(anchor_fd):
    fixture = new_fixture(anchor_fd, "posix-acl")
    try:
        def install_acl_and_prove(store):
            floor_fd = os.open(os.path.join(fixture.path, journal.PAPER_DIR, "epoch-floor"),
                               os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
            try:
                os.setxattr(floor_fd, "system.posix_acl_access", make_acl_bytes())
                mode = stat.S_IMODE(os.fstat(floor_fd).st_mode)
                try:
                    actual = os.getxattr(floor_fd, "system.posix_acl_access")
                except OSError:
                    raise CaseFailure("ACL_XATTR_NOT_READABLE") from None
                require(actual == make_acl_bytes(), "ACL_BYTES_NOT_PERSISTED")
                require(mode == 0o600, "ACL_DID_NOT_PRESERVE_EXPECTED_MODE")
                expect_error(store.revalidate, "JOURNAL_ACL")
            except OSError as error:
                if error.errno in (errno.ENODATA, errno.ENOTSUP, errno.EOPNOTSUPP, errno.EPERM,
                                   errno.EACCES, errno.EINVAL):
                    raise CaseFailure("ACL_REQUIRED_PREREQUISITE_UNAVAILABLE") from None
                raise
            finally:
                os.close(floor_fd)
        expect_context_exit_error(fixture.open(), "JOURNAL_ACL", install_acl_and_prove)
        return "REAL_EXTENDED_ACL_REJECTED_WITH_EXPECTED_MODE"
    finally:
        fixture.close()


def case_stage_chain(anchor_fd):
    fixture = new_fixture(anchor_fd, "stage-chain")
    try:
        with fixture.open() as store:
            installation_id = store.installation_id
            intent = fixture.intent(installation_id)
            operation = store.create(intent)
            stage0 = fixture.stage(operation, installation_id, "INTENT", 0, None)
            hash0 = operation.append(stage0)
            stage1 = fixture.stage(operation, installation_id, "FENCED", 1, hash0)
            hash1 = operation.append(stage1)
            stage2 = fixture.stage(operation, installation_id, "STOPPED", 2, hash1)
            hash2 = operation.append(stage2)
            require(operation.append(stage2) == hash2, "STAGE_IDEMPOTENCE_FAILED")
            require(operation.revalidate() == "STOPPED", "CHAIN_HEAD_MISMATCH")
            stage_dir = os.path.join(fixture.path, journal.PAPER_DIR, "operations", operation.generation)
            for sequence, raw in enumerate((stage0, stage1, stage2)):
                require_file_observation(os.path.join(stage_dir, "%08d.json" % sequence), raw, 0o400)
                require_file_observation(os.path.join(stage_dir, digest(raw)[7:] + ".json"), raw, 0o400)
            operation.close()
            generation = json.loads(intent)["generation"]
            stage_dir = os.path.join(fixture.path, journal.PAPER_DIR, "operations", generation)
            os.unlink(os.path.join(stage_dir, "00000001.json"))
            expect_error(lambda: store.open(generation), "JOURNAL_INCOMPLETE")
            Path(os.path.join(stage_dir, "00000001.json")).write_bytes(stage1)
            os.chmod(os.path.join(stage_dir, "00000001.json"), 0o400)
            Path(os.path.join(stage_dir, "000000000.json")).write_bytes(stage0)
            os.chmod(os.path.join(stage_dir, "000000000.json"), 0o400)
            expect_error(lambda: store.open(generation), "JOURNAL_INCOMPLETE")
            os.unlink(os.path.join(stage_dir, "000000000.json"))
            reopened = store.open(generation)
            changed = fixture.intent(installation_id, generation=generation,
                                     request_key=generation + ":off")
            replacement = os.path.join(stage_dir, "intent-replacement")
            Path(replacement).write_bytes(changed)
            os.chmod(replacement, 0o400)
            os.replace(replacement, os.path.join(stage_dir, "intent.json"))
            expect_error(reopened.revalidate, "JOURNAL_FILE")
            reopened.close()
        return "VALID_CHAIN_ACCEPTED_GAP_ALIAS_AND_MUTATED_INTENT_REJECTED"
    finally:
        fixture.close()


def case_floor(anchor_fd):
    fixture = new_fixture(anchor_fd, "floor-monotonicity")
    try:
        with fixture.open() as store:
            require(store.advance_floor(3) == 3, "FLOOR_DID_NOT_ADVANCE")
            require(store.advance_floor(3) == 3, "FLOOR_EQUAL_NOT_IDEMPOTENT")
            expect_error(lambda: store.advance_floor(2), "JOURNAL_FLOOR_DECREMENT")
            expect_error(lambda: store.advance_floor(journal.MAX_I64 + 1), "JOURNAL_COUNTER")
        with fixture.open(bootstrap=False) as reopened:
            require(reopened.epoch_floor == 3, "FLOOR_NOT_DURABLE")
        return "FLOOR_MONOTONIC_DURABLE_AND_OVERFLOW_REJECTED"
    finally:
        fixture.close()


def _incomplete_after_injected_write(anchor_fd, label, patch_name):
    fixture = new_fixture(anchor_fd, label)
    try:
        with fixture.open() as store:
            installation_id = store.installation_id
            operation = store.create(fixture.intent(installation_id))
            stage = fixture.stage(operation, installation_id, "INTENT", 0, None)
            with mock.patch.object(journal.os, patch_name, side_effect=OSError(errno.EIO, "fixture")):
                try:
                    operation.append(stage)
                except OSError:
                    pass
                else:
                    raise CaseFailure("INJECTED_IO_FAILURE_NOT_OBSERVED")
            expect_error(operation.revalidate, "JOURNAL_INCOMPLETE")
            operation.close()
        return "INJECTED_" + patch_name.upper() + "_LEFT_FAIL_CLOSED_MARKER"
    finally:
        fixture.close()


def case_no_replace_fsync(anchor_fd):
    first = _incomplete_after_injected_write(anchor_fd, "injected-write-failure", "write")
    second = _incomplete_after_injected_write(anchor_fd, "injected-fsync-failure", "fsync")
    return first + ";" + second


def case_uid_drop(anchor_fd):
    fixture = new_fixture(anchor_fd, "uid-drop")
    try:
        with fixture.open() as _store:
            result = fork_probe("UID_DROP", fixture.path, fixture.lock_fd)
            require(result == "ROOT_GATE_JOURNAL_PLATFORM", "NONROOT_CHILD_NOT_DENIED")
        return "REAL_UID_65534_CHILD_DENIED_BY_JOURNAL_ROOT_GATE"
    finally:
        fixture.close()


CASES = (
    ("BOOTSTRAP_PERSISTENCE", case_bootstrap),
    ("NO_REBOOTSTRAP_WITH_HISTORY", case_no_rebootstrap),
    ("IMMUTABLE_IDEMPOTENCE_CONFLICT", case_immutable),
    ("BOUND_CALLER_ACTION_PUBLICATION", case_bound_caller_action),
    ("LOCK_INHERITED_LIFETIME", case_lock_lifetime),
    ("LINK_OWNER_MODE_AND_ANCESTOR_REJECTION", case_links_owners_modes),
    ("NAMED_INODE_REPLACEMENT", case_named_replacement),
    ("POSIX_ACL_REJECTION", case_acl),
    ("STAGE_CHAIN_GAP_FORK_MUTATION", case_stage_chain),
    ("FLOOR_MONOTONICITY_OVERFLOW", case_floor),
    ("NOREPLACE_FSYNC_FAILURE_CLEANUP", case_no_replace_fsync),
    ("REAL_UID_DROP_GATE", case_uid_drop),
)


def descriptor_count():
    try:
        return len(os.listdir("/proc/self/fd"))
    except OSError:
        return None


def result_summary(case_results, cleanup_status, descriptors):
    passed = (len(case_results) == len(CASES)
              and all(item.get("observed") == "PASS" for item in case_results)
              and cleanup_status == "PASS" and descriptors == "CLOSED")
    return {"status": "PASS" if passed else "FAIL",
            "code": "ALL_CASES_PASS" if passed else "CASE_FAILURE",
            "cases": case_results, "cleanup": cleanup_status,
            "descriptors": descriptors}


def traceback_origin(traceback):
    """Return only the deepest bounded frame from the fixed source allowlist."""
    origin = None
    while traceback is not None:
        filename = traceback.tb_frame.f_code.co_filename
        relative = None
        if isinstance(filename, str):
            try:
                relative = TRACEBACK_PATH_ALLOWLIST.get(str(Path(filename).resolve(strict=False)))
            except (OSError, RuntimeError, ValueError):
                relative = None
        line = traceback.tb_lineno
        if relative is not None and type(line) is int and 1 <= line <= MAX_TRACEBACK_LINE:
            origin = {"path": relative, "line": line}
        traceback = traceback.tb_next
    return origin


def unexpected_case_result(name, error):
    """Keep an unexpected case failed; expose only a closed source/line witness."""
    code = "INTERRUPTED" if STOP_SIGNAL is not None else (
        "UNEXPECTED_" + type(error).__name__.upper())
    result = {"case": name, "observed": "FAIL", "code": code}
    origin = traceback_origin(error.__traceback__)
    if origin is not None:
        result["origin"] = origin
    return result


class ControlledStop(Exception):
    pass


def _handle_signal(signum, _frame):
    global STOP_SIGNAL
    STOP_SIGNAL = signum
    for pid in tuple(OWNED_CHILDREN):
        _signal_owned_child(pid, signal.SIGTERM)
    if not OWNED_CHILDREN and not CLEANING_UP:
        raise ControlledStop()


def run_self_test():
    if sys.platform != "linux" or os.geteuid() != 0 or sys.maxsize <= 2**32:
        return {"status": "FAIL", "code": "ROOT_LINUX_REQUIRED", "cases": [],
                "cleanup": "NOT_STARTED"}
    try:
        actual_journal_hash = hashlib.sha256(JOURNAL_PATH.read_bytes()).hexdigest()
    except OSError:
        return {"status": "FAIL", "code": "JOURNAL_SOURCE_UNAVAILABLE",
                "cases": [{"case": "JOURNAL_SOURCE_PIN", "observed": "FAIL",
                           "code": "JOURNAL_SOURCE_UNAVAILABLE"}],
                "cleanup": "NOT_REQUIRED", "descriptors": "CLOSED"}
    if actual_journal_hash != EXPECTED_JOURNAL_SHA256:
        return {"status": "FAIL", "code": "JOURNAL_SOURCE_HASH_MISMATCH",
                "journal_sha256": actual_journal_hash,
                "cases": [{"case": "JOURNAL_SOURCE_PIN", "observed": "FAIL",
                           "code": "JOURNAL_SOURCE_HASH_MISMATCH"}],
                "cleanup": "NOT_REQUIRED", "descriptors": "CLOSED"}
    started = time.monotonic()
    before_fds = descriptor_count()
    parent_fd = anchor_fd = None
    anchor_name = None
    anchor_identity = None
    case_results = []
    cleanup_status = "NOT_STARTED"
    overall_deadline = started + OVERALL_TIMEOUT
    global CLEANING_UP
    try:
        parent_fd = os.open(ANCHOR_PARENT, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
        parent = assert_root_directory(parent_fd)
        if parent.st_uid != 0 or parent.st_gid != 0:
            raise RuntimeError("ANCHOR_PARENT_OWNER")
        nonce = str(uuid.uuid4())
        anchor_name, anchor_fd, anchor_identity = create_anchor(parent_fd, nonce)
        for name, run_case in CASES:
            if STOP_SIGNAL is not None:
                case_results.extend({"case": pending, "observed": "FAIL", "code": "INTERRUPTED"}
                                    for pending, _ in CASES[len(case_results):])
                break
            if time.monotonic() >= overall_deadline:
                case_results.append({"case": name, "observed": "FAIL", "code": "OVERALL_TIMEOUT"})
                break
            try:
                detail = run_case(anchor_fd)
                case_results.append({"case": name, "observed": "PASS", "detail": detail})
            except CaseFailure as error:
                case_results.append({"case": name, "observed": "FAIL", "code": error.code})
            except journal.JournalError as error:
                case_results.append({"case": name, "observed": "FAIL", "code": error.code})
            except BaseException as error:
                # Type names are bounded diagnostics; exception text is never emitted.
                case_results.append(unexpected_case_result(name, error))
                if STOP_SIGNAL is not None:
                    case_results.extend({"case": pending, "observed": "FAIL", "code": "INTERRUPTED"}
                                        for pending, _ in CASES[len(case_results):])
                    break
        if len(case_results) != len(CASES):
            missing = [name for name, _ in CASES[len(case_results):]]
            case_results.extend({"case": name, "observed": "FAIL", "code": "NOT_RUN"}
                                for name in missing)
    except BaseException as error:
        case_results.append({"case": "PREREQUISITE", "observed": "FAIL",
                             "code": "UNEXPECTED_" + type(error).__name__.upper()})
    finally:
        CLEANING_UP = True
        children_reaped = False
        try:
            children_reaped = reap_owned_children()
        except BaseException:
            children_reaped = False
        if anchor_fd is not None:
            try:
                if not children_reaped:
                    cleanup_status = "FAIL_CHILDREN_UNREAPED"
                elif anchor_name is not None and anchor_identity is not None:
                    cleanup_anchor(parent_fd, anchor_name, anchor_fd, anchor_identity)
                    cleanup_status = "PASS"
                else:
                    cleanup_status = "FAIL"
            except BaseException as error:
                cleanup_status = "FAIL_" + type(error).__name__.upper()
            finally:
                os.close(anchor_fd)
        if parent_fd is not None:
            os.close(parent_fd)
        CLEANING_UP = False
    after_fds = descriptor_count()
    descriptors = "UNAVAILABLE" if before_fds is None or after_fds is None else (
        "CLOSED" if before_fds == after_fds else "LEAKED")
    if descriptors == "LEAKED":
        case_results.append({"case": "DESCRIPTOR_LIVENESS", "observed": "FAIL",
                             "code": "DESCRIPTOR_COUNT_CHANGED"})
    if cleanup_status != "PASS":
        case_results.append({"case": "FIXTURE_CLEANUP", "observed": "FAIL",
                             "code": cleanup_status})
    result = result_summary(case_results, cleanup_status, descriptors)
    result["journal_sha256"] = actual_journal_hash
    return result


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if argv:
        result = {"status": "FAIL", "code": "PUBLIC_ARGUMENTS_FORBIDDEN",
                  "cases": [], "cleanup": "NOT_STARTED"}
        print(json.dumps(result, sort_keys=True, separators=(",", ":")))
        return 2
    if sys.platform != "linux" or os.geteuid() != 0 or sys.maxsize <= 2**32:
        result = {"status": "FAIL", "code": "ROOT_LINUX_REQUIRED",
                  "cases": [], "cleanup": "NOT_STARTED"}
        print(json.dumps(result, sort_keys=True, separators=(",", ":")))
        return 2
    global STOP_SIGNAL
    STOP_SIGNAL = None
    old_handlers = {sig: signal.signal(sig, _handle_signal) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        result = run_self_test()
    except ControlledStop:
        result = {"status": "FAIL", "code": "INTERRUPTED", "cases": [],
                  "cleanup": "NOT_STARTED"}
    finally:
        for sig, old_handler in old_handlers.items():
            signal.signal(sig, old_handler)
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0 if result["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
