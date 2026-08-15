"""One live node per account, enforced by the filesystem (plan Todo 41).

Design §6.12: "Live 노드는 계좌당 하나의 프로세스로 운영한다."

Two processes trading one account double every order either places, and
neither can see the other's in-flight state. The database already refuses a
second `broker_nodes` row for one connection (migration 0016's partial unique
index), which stops the API from *recording* two nodes — but it cannot stop
someone running `python -m live_node` twice on a host. This module closes that
gap at the only place that can: the process itself.

The lock is an O_EXCL file holding the owner's pid and a private claim token.
`O_EXCL` is atomic in the
kernel, so two processes racing to start cannot both win — a check-then-create
would let both pass the check before either created anything.  A stale lock is
reclaimed while holding a second, stable per-account guard file locked with
the operating system's file-lock primitive.  The guard covers the stale
check, reclaim, and replacement create as one critical section; without it,
two starters could both observe the same dead pid and one could unlink the
other's newly-created lock.

A stale lock (the owner is gone) is reclaimed, because otherwise a crash would
require manual intervention before Live could restart, and an operator deleting
lock files under pressure is how the exclusivity gets lost for real. Reclaiming
is safe precisely because it is verified against the OS, not assumed from the
file's age.
"""
from __future__ import annotations

import errno
import os
import re
import secrets
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Iterator


ACCOUNT_ID_RE = re.compile(r"^[A-Za-z0-9_.:-]+$")


def _has_symlink_component(path: Path) -> bool:
    """Return true when an existing path component is a symlink.

    Lock paths are security boundaries.  Following a symlink supplied by an
    operator (or planted by another local user) can put the lock outside the
    configured directory and make the one-node-per-account claim meaningless.
    Missing components are allowed because ``acquire`` creates them.
    """

    candidate = path
    while True:
        try:
            if candidate.is_symlink():
                return True
        except OSError:
            # An unreadable path cannot be proved safe.  Fail closed just as
            # we do for an unreadable lock file.
            return True
        if candidate.parent == candidate:
            return False
        candidate = candidate.parent


@dataclass(frozen=True)
class LockInfo:
    """Who holds a lock.

    `pid` is `None` when the lock file exists but cannot be parsed. That is a
    distinct case from "nobody holds it", and conflating the two is how a
    corrupt lock silently becomes a stolen one — so it is a distinct value
    rather than a sentinel like -1 that arithmetic and comparisons would
    quietly treat as a real pid.
    """

    pid: int | None
    account_id: str

    @property
    def is_readable(self) -> bool:
        return self.pid is not None


class NodeAlreadyRunning(RuntimeError):
    """Another live process owns this account."""

    def __init__(self, holder: LockInfo) -> None:
        who = f"pid {holder.pid}" if holder.is_readable else "an unreadable lock file"
        super().__init__(
            f"account {holder.account_id} is already served by {who}; "
            "one live node per account"
        )
        self.holder = holder


def _lock_guard_fd(fd: int) -> None:
    """Take an exclusive lock on one byte of the stable account guard.

    ``flock`` is released by the kernel if a process crashes, which is the
    important property here: the PID lock remains available for stale
    recovery while the guard itself never needs a second stale-recovery
    protocol.  ``msvcrt.locking`` is the standard-library equivalent on
    Windows.  The guard file gets a byte before locking because Windows does
    not lock a byte range beyond EOF.
    """

    if os.name == "nt":
        import msvcrt

        os.lseek(fd, 0, os.SEEK_SET)
        if os.fstat(fd).st_size == 0:
            os.write(fd, b"\0")
            os.lseek(fd, 0, os.SEEK_SET)
        msvcrt.locking(fd, msvcrt.LK_LOCK, 1)
        return

    import fcntl

    fcntl.flock(fd, fcntl.LOCK_EX)


def _unlock_guard_fd(fd: int) -> None:
    """Release a guard acquired by :func:`_lock_guard_fd`."""

    if os.name == "nt":
        import msvcrt

        os.lseek(fd, 0, os.SEEK_SET)
        msvcrt.locking(fd, msvcrt.LK_UNLCK, 1)
        return

    import fcntl

    fcntl.flock(fd, fcntl.LOCK_UN)


def _pid_alive_posix(pid: int) -> bool:
    """POSIX liveness: signal 0 checks reachability without sending anything."""
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        # It exists and belongs to someone else. Existence is what we asked.
        return True
    except OSError as exc:  # pragma: no cover - defensive
        return exc.errno != errno.ESRCH
    return True


def _pid_alive_windows(pid: int) -> bool:
    """Windows liveness, via `OpenProcess` rather than `os.kill`.

    `os.kill(pid, 0)` is the obvious thing and it is WRONG here. Measured on
    this platform: for a pid that does not exist it raises
    ``OSError: [WinError 87] The parameter is incorrect`` — not
    `ProcessLookupError`, and with an errno that is not `ESRCH`. Any
    reasonable-looking `except OSError: return True` fallback therefore
    reports a long-dead process as alive, which in this module means a stale
    lock is never reclaimed and Live cannot restart after a crash without
    someone deleting a file by hand.

    `OpenProcess` answers the question directly. A NULL handle with
    ERROR_ACCESS_DENIED means the process exists but belongs to someone else
    — existence is what we asked, so that is alive. Any other NULL means it
    is gone. A live handle is checked with `GetExitCodeProcess`, because a
    handle can outlive the process it names.
    """
    import ctypes
    from ctypes import wintypes

    PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
    ERROR_ACCESS_DENIED = 5
    STILL_ACTIVE = 259

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.OpenProcess.argtypes = (wintypes.DWORD, wintypes.BOOL, wintypes.DWORD)

    handle = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not handle:
        return ctypes.get_last_error() == ERROR_ACCESS_DENIED
    try:
        code = wintypes.DWORD()
        if not kernel32.GetExitCodeProcess(handle, ctypes.byref(code)):
            # We hold a handle but cannot read the exit code. Refusing to
            # reclaim is the safe reading: a lock wrongly kept costs a manual
            # step, a lock wrongly stolen costs a second live node.
            return True
        return code.value == STILL_ACTIVE
    finally:
        kernel32.CloseHandle(handle)


def _pid_alive(pid: int) -> bool:
    """Whether a process with this pid exists."""
    if pid <= 0:
        return False
    if os.name == "nt":
        return _pid_alive_windows(pid)
    return _pid_alive_posix(pid)


class AccountLock:
    """An exclusive claim on one account for the life of this process."""

    def __init__(self, lock_dir: Path, account_id: str) -> None:
        if not account_id or not ACCOUNT_ID_RE.fullmatch(account_id):
            raise ValueError("account id contains unsupported characters")
        self._lock_dir = Path(lock_dir)
        if _has_symlink_component(self._lock_dir):
            raise ValueError("account lock directory must not contain symlinks")
        self._path = self._lock_dir / f"live-node-{account_id}.lock"
        self._guard_path = self._lock_dir / f"live-node-{account_id}.guard"
        if _has_symlink_component(self._guard_path):
            raise ValueError("account lock guard must not contain symlinks")
        self._account_id = account_id
        self._held = False
        self._owner_pid: int | None = None
        self._owner_identity: tuple[int, int] | None = None
        self._claim_token: str | None = None

    @property
    def path(self) -> Path:
        return self._path

    def read_holder(self) -> LockInfo | None:
        """Who holds the lock, or None if nobody does."""
        # Do not follow a lock-file symlink.  A symlink is an unreadable lock,
        # not permission to inspect or unlink an arbitrary target.
        if _has_symlink_component(self._path):
            return LockInfo(pid=None, account_id=self._account_id)
        try:
            flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
            fd = os.open(self._path, flags)
        except FileNotFoundError:
            return None
        except OSError:
            return LockInfo(pid=None, account_id=self._account_id)
        try:
            with os.fdopen(fd, "r", encoding="utf-8") as handle:
                raw = handle.read().strip()
        except (OSError, UnicodeError):
            return LockInfo(pid=None, account_id=self._account_id)
        lines = raw.splitlines()
        pid_text = lines[0] if lines else ""
        account = lines[1] if len(lines) > 1 else ""
        try:
            pid = int(pid_text)
        except ValueError:
            # Unreadable. NOT the same as unheld: a corrupt lock is a fault
            # someone should look at, and clearing it is indistinguishable
            # from stealing a running node's claim.
            return LockInfo(pid=None, account_id=account or self._account_id)
        return LockInfo(pid=pid, account_id=account or self._account_id)

    def _read_claim_token(self) -> str | None:
        """Read the private claim token, if this lock uses the new format."""

        if _has_symlink_component(self._path):
            return None
        try:
            flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
            fd = os.open(self._path, flags)
        except OSError:
            return None
        try:
            with os.fdopen(fd, "r", encoding="utf-8") as handle:
                lines = handle.read().strip().splitlines()
        except (OSError, UnicodeError):
            return None
        return lines[2] if len(lines) > 2 and lines[2] else None

    def _validate_paths(self) -> None:
        if _has_symlink_component(self._lock_dir):
            raise RuntimeError("account lock directory must not contain symlinks")
        if _has_symlink_component(self._guard_path):
            raise RuntimeError("account lock guard must not contain symlinks")

    @contextmanager
    def _guard(self) -> Iterator[None]:
        """Serialize all mutations of this account's PID lock.

        The guard file is intentionally never removed.  Its inode is stable,
        while ``flock``/``msvcrt`` ownership is released automatically when a
        process exits, including an unhandled crash.
        """

        self._validate_paths()
        self._lock_dir.mkdir(parents=True, exist_ok=True)
        self._validate_paths()
        flags = os.O_CREAT | os.O_RDWR | getattr(os, "O_NOFOLLOW", 0)
        fd = os.open(self._guard_path, flags, 0o600)
        locked = False
        try:
            _lock_guard_fd(fd)
            locked = True
            yield
        finally:
            try:
                if locked:
                    _unlock_guard_fd(fd)
            finally:
                os.close(fd)

    @staticmethod
    def _same_file(left: os.stat_result, right: os.stat_result) -> bool:
        return (left.st_dev, left.st_ino) == (right.st_dev, right.st_ino)

    def _unlink_if_same_file(self, expected: os.stat_result) -> bool:
        """Unlink only the stale inode we inspected, never a replacement."""

        try:
            current = os.stat(self._path, follow_symlinks=False)
        except FileNotFoundError:
            return False
        except OSError:
            return False
        if not self._same_file(current, expected):
            return False
        try:
            os.unlink(self._path)
        except FileNotFoundError:
            return False
        return True

    def _create(self, payload: str, claim_token: str) -> LockInfo:
        flags = os.O_CREAT | os.O_EXCL | os.O_WRONLY | getattr(os, "O_NOFOLLOW", 0)
        fd = os.open(self._path, flags, 0o600)
        identity: os.stat_result | None = None
        try:
            identity = os.fstat(fd)
            with os.fdopen(fd, "w", encoding="utf-8") as handle:
                handle.write(payload)
        except BaseException:
            # If writing our own claim fails, clean it up only while the path
            # still names the inode we created.  This avoids deleting a
            # successor if an unexpected external replacement occurs.
            try:
                current = os.stat(self._path, follow_symlinks=False)
                if identity is not None and self._same_file(current, identity):
                    os.unlink(self._path)
            except OSError:
                pass
            raise
        assert identity is not None
        self._held = True
        self._owner_pid = os.getpid()
        self._owner_identity = (identity.st_dev, identity.st_ino)
        self._claim_token = claim_token
        return LockInfo(pid=self._owner_pid, account_id=self._account_id)

    def acquire(self) -> LockInfo:
        """Claims the account, or raises `NodeAlreadyRunning`."""
        self._validate_paths()
        claim_token = secrets.token_hex(16)
        payload = f"{os.getpid()}\n{self._account_id}\n{claim_token}"
        with self._guard():
            try:
                return self._create(payload, claim_token)
            except FileExistsError:
                # Refuse if someone holds it, AND refuse if we cannot tell.
                # Only a lock we can read and have confirmed dead may be
                # reclaimed.
                try:
                    stale_identity = os.stat(self._path, follow_symlinks=False)
                except OSError:
                    stale_identity = None
                holder = self.read_holder()
                if holder is not None and (
                    not holder.is_readable or _pid_alive(holder.pid)
                ):
                    raise NodeAlreadyRunning(holder) from None
                if stale_identity is None or not self._unlink_if_same_file(stale_identity):
                    # The inspected inode disappeared or changed.  In
                    # particular, never unlink a winner created after our
                    # stale observation.
                    holder = self.read_holder()
                    raise NodeAlreadyRunning(
                        holder or LockInfo(pid=None, account_id=self._account_id)
                    ) from None
                return self._create(payload, claim_token)

    def release(self) -> None:
        """Gives the account up. Only removes a lock this process holds, so a
        late release cannot delete a successor's claim."""
        if not self._held:
            return
        try:
            # A forked child inherits ``_held`` but is not the owner.  It must
            # not release the parent's claim merely because it inherited the
            # Python object.
            if self._owner_pid != os.getpid():
                return
            with self._guard():
                holder = self.read_holder()
                claim_token = self._read_claim_token()
                if (
                    holder is not None
                    and holder.pid == self._owner_pid
                    and holder.account_id == self._account_id
                    and claim_token == self._claim_token
                ):
                    try:
                        identity = os.stat(self._path, follow_symlinks=False)
                    except OSError:
                        identity = None
                    if identity is not None and self._owner_identity == (
                        identity.st_dev,
                        identity.st_ino,
                    ):
                        self._unlink_if_same_file(identity)
        finally:
            self._held = False
            self._owner_pid = None
            self._owner_identity = None
            self._claim_token = None

    def __enter__(self) -> LockInfo:
        return self.acquire()

    def __exit__(self, *_exc: object) -> None:
        self.release()
