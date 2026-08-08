"""One live node per account, enforced by the filesystem (plan Todo 41).

Design §6.12: "Live 노드는 계좌당 하나의 프로세스로 운영한다."

Two processes trading one account double every order either places, and
neither can see the other's in-flight state. The database already refuses a
second `broker_nodes` row for one connection (migration 0016's partial unique
index), which stops the API from *recording* two nodes — but it cannot stop
someone running `python -m live_node` twice on a host. This module closes that
gap at the only place that can: the process itself.

The lock is an O_EXCL file holding the owner's pid. `O_EXCL` is atomic in the
kernel, so two processes racing to start cannot both win — a check-then-create
would let both pass the check before either created anything.

A stale lock (the owner is gone) is reclaimed, because otherwise a crash would
require manual intervention before Live could restart, and an operator deleting
lock files under pressure is how the exclusivity gets lost for real. Reclaiming
is safe precisely because it is verified against the OS, not assumed from the
file's age.
"""
from __future__ import annotations

import errno
import os
from dataclasses import dataclass
from pathlib import Path


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
        if not account_id:
            raise ValueError("an account lock needs an account")
        self._path = Path(lock_dir) / f"live-node-{account_id}.lock"
        self._account_id = account_id
        self._held = False

    @property
    def path(self) -> Path:
        return self._path

    def read_holder(self) -> LockInfo | None:
        """Who holds the lock, or None if nobody does."""
        try:
            raw = self._path.read_text(encoding="utf-8").strip()
        except FileNotFoundError:
            return None
        pid_text, _, account = raw.partition("\n")
        try:
            pid = int(pid_text)
        except ValueError:
            # Unreadable. NOT the same as unheld: a corrupt lock is a fault
            # someone should look at, and clearing it is indistinguishable
            # from stealing a running node's claim.
            return LockInfo(pid=None, account_id=account or self._account_id)
        return LockInfo(pid=pid, account_id=account or self._account_id)

    def acquire(self) -> LockInfo:
        """Claims the account, or raises `NodeAlreadyRunning`."""
        self._path.parent.mkdir(parents=True, exist_ok=True)
        payload = f"{os.getpid()}\n{self._account_id}"
        try:
            fd = os.open(self._path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        except FileExistsError:
            holder = self.read_holder()
            # Refuse if someone holds it, AND refuse if we cannot tell. Only a
            # lock we can read and have confirmed dead may be reclaimed.
            if holder is not None and (
                not holder.is_readable or _pid_alive(holder.pid)
            ):
                raise NodeAlreadyRunning(holder) from None
            # The holder is gone. Reclaim, then re-acquire: a crash must not
            # need a human to delete a file before Live can come back, because
            # that is how "just remove the lock" becomes routine and the
            # exclusivity stops meaning anything.
            self._path.unlink(missing_ok=True)
            try:
                fd = os.open(self._path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            except FileExistsError:
                # Someone else reclaimed it in the same instant. They won.
                # `pid=None` if the winner's file is not readable yet -- the
                # same "we cannot tell" value used above, never a numeric
                # sentinel that a later comparison would mistake for a pid.
                holder = self.read_holder()
                raise NodeAlreadyRunning(
                    holder or LockInfo(pid=None, account_id=self._account_id)
                ) from None
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(payload)
        self._held = True
        return LockInfo(pid=os.getpid(), account_id=self._account_id)

    def release(self) -> None:
        """Gives the account up. Only removes a lock this process holds, so a
        late release cannot delete a successor's claim."""
        if not self._held:
            return
        holder = self.read_holder()
        if holder is not None and holder.pid == os.getpid():
            self._path.unlink(missing_ok=True)
        self._held = False

    def __enter__(self) -> LockInfo:
        return self.acquire()

    def __exit__(self, *_exc: object) -> None:
        self.release()
