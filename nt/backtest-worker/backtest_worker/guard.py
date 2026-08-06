"""Runtime guard loaded by the worker child BEFORE any NT code runs.

Enforces the isolation contract the supervisor declares via the environment:
- `LAGRANGE_NETWORK_DISABLED=1` -> every non-loopback socket connect raises
  [`NetworkDisabledError`] (external network blocked, loopback allowed).
- `LAGRANGE_RO_MOUNTS=<paths>` -> any write attempt under a declared mount
  (open-write / os mutation / shutil) raises [`ReadOnlyMountError`].
- `LAGRANGE_STATUS_FILE` -> structured status written on start and on
  graceful termination (SIGTERM/SIGINT/SIGBREAK or the supervisor's control
  file), so the supervisor can distinguish TERMINATED from FAILED.
- `LAGRANGE_CONTROL_FILE` -> a watchdog thread polls it; when the supervisor
  drops a STOP marker the child terminates gracefully (deterministic even
  when console signals are unavailable).

The guard never panics: violations raise typed errors the simulate entrypoint
turns into a structured FAILED status.
"""
from __future__ import annotations

import json
import os
import threading
import time
from pathlib import Path

_LOOPBACK_HOSTS = ("127.0.0.1", "::1", "localhost")
import signal as _signal

_SIGNAL_NAMES = {
    getattr(_signal, name): name
    for name in ("SIGTERM", "SIGINT", "SIGBREAK")
    if hasattr(_signal, name)
}

_installed = False


class GuardError(RuntimeError):
    """Base class for runtime-guard violations."""


class NetworkDisabledError(GuardError):
    """A non-loopback network connection was attempted inside an isolated run."""


class ReadOnlyMountError(GuardError):
    """A write was attempted inside a read-only dataset/catalog mount."""


def install() -> None:
    global _installed
    if _installed:
        return
    _installed = True
    env = os.environ
    status_file = env.get("LAGRANGE_STATUS_FILE")
    control_file = env.get("LAGRANGE_CONTROL_FILE")
    if env.get("LAGRANGE_NETWORK_DISABLED") == "1":
        _patch_socket()
    mounts = env.get("LAGRANGE_RO_MOUNTS")
    if mounts:
        _patch_filesystem(tuple(p for p in mounts.split(os.pathsep) if p))
    _install_signal_handlers(status_file)
    if control_file:
        _watch_control_file(control_file, status_file)
    _write_status(status_file, state="RUNNING", pid=os.getpid())


def _write_status(path: str | None, **fields: object) -> None:
    if not path:
        return
    payload = {"state": fields.get("state"), "pid": os.getpid()}
    if "signal" in fields:
        payload["signal"] = fields["signal"]
    try:
        Path(path).write_text(json.dumps(payload, sort_keys=True) + "\n", encoding="utf-8")
    except OSError:
        pass


def _install_signal_handlers(status_file: str | None) -> None:
    import signal

    def handler(signum, _frame):
        _write_status(status_file, state="TERMINATED", signal=_SIGNAL_NAMES.get(signum, str(signum)))
        os._exit(143 if signum != signal.SIGINT else 130)

    for signum in (signal.SIGTERM, signal.SIGINT):
        try:
            signal.signal(signum, handler)
        except (ValueError, OSError):
            pass
    if hasattr(signal, "SIGBREAK"):
        try:
            signal.signal(signal.SIGBREAK, handler)
        except (ValueError, OSError):
            pass


def _watch_control_file(control_file: str, status_file: str | None) -> None:
    def poll() -> None:
        while True:
            try:
                if Path(control_file).exists():
                    _write_status(status_file, state="TERMINATED", signal="CONTROL_FILE")
                    os._exit(143)
            except OSError:
                pass
            time.sleep(0.2)

    threading.Thread(target=poll, daemon=True, name="lagrange-control-watchdog").start()


def _patch_socket() -> None:
    import socket as _socket

    _orig_connect = _socket.socket.connect
    _orig_connect_ex = _socket.socket.connect_ex
    _orig_create_connection = _socket.create_connection

    def _host_of(address) -> str:
        return address[0] if isinstance(address, tuple) else str(address)

    def _is_loopback(host: str) -> bool:
        if host in _LOOPBACK_HOSTS:
            return True
        try:
            import ipaddress

            return ipaddress.ip_address(host.split("%")[0]).is_loopback
        except ValueError:
            return False

    def _deny(host: str) -> None:
        raise NetworkDisabledError(
            f"network is disabled for the isolated worker (blocked {host!r})"
        )

    def connect(self, address, *args, **kwargs):
        host = _host_of(address)
        if not _is_loopback(host):
            _deny(host)
        return _orig_connect(self, address, *args, **kwargs)

    def connect_ex(self, address, *args, **kwargs):
        host = _host_of(address)
        if not _is_loopback(host):
            _deny(host)
        return _orig_connect_ex(self, address, *args, **kwargs)

    def create_connection(address, *args, **kwargs):
        host = _host_of(address)
        if not _is_loopback(host):
            _deny(host)
        return _orig_create_connection(address, *args, **kwargs)

    _socket.socket.connect = connect
    _socket.socket.connect_ex = connect_ex
    _socket.create_connection = create_connection


def _patch_filesystem(mounts: tuple[str, ...]) -> None:
    import builtins
    import shutil

    import os as _os

    normalized = tuple(_norm(p) for p in mounts)

    def _under(path) -> bool:
        target = _norm(path)
        return any(target == m or target.startswith(m + os.sep) or target.startswith(m + "/") for m in normalized)

    def _check(path) -> None:
        if isinstance(path, (str, os.PathLike)) and _under(os.fspath(path)):
            raise ReadOnlyMountError(f"path {path!r} is inside a read-only mount")

    orig_open = builtins.open

    def open(path, mode="r", *args, **kwargs):
        if isinstance(mode, str) and any(ch in mode for ch in "wax+"):
            _check(path)
        return orig_open(path, mode, *args, **kwargs)

    builtins.open = open

    for name in ("remove", "unlink", "rmdir", "mkdir", "makedirs", "rename", "renames"):
        original = getattr(_os, name, None)
        if original is None:
            continue

        def make_wrapper(original_fn):
            def wrapper(path, *args, **kwargs):
                _check(path)
                return original_fn(path, *args, **kwargs)

            return wrapper

        setattr(_os, name, make_wrapper(original))

    for name in ("rmtree", "copy", "copy2", "copyfile", "move", "copytree"):
        original = getattr(shutil, name, None)
        if original is None:
            continue

        def make_wrapper(original_fn):
            def wrapper(src, dst, *args, **kwargs):
                _check(src)
                _check(dst)
                return original_fn(src, dst, *args, **kwargs)

            return wrapper

        setattr(shutil, name, make_wrapper(original))


def _norm(path) -> str:
    return os.path.normcase(os.path.abspath(os.fspath(path)))
