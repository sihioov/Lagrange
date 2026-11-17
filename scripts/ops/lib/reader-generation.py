#!/usr/bin/env python3
"""Shared helpers for the checked-in reader drain and pre-upgrade backup tools.

This module never calls Docker, systemd or a database by itself.  It validates
arguments, keeps a root 0600 progress record in an explicit state directory,
loads installed release code by path after an ownership check, and describes
restart policies from a closed allowlist.  Exceptions carry fixed codes only;
no path, identifier, timestamp or provider text is ever placed in one.
"""
from __future__ import annotations

import argparse
import datetime
import fcntl
import importlib.util
import json
import os
import re
import signal
import stat
import subprocess
import sys

sys.dont_write_bytecode = True

COMMIT = re.compile(r"[0-9a-f]{40}")
CONTAINER = re.compile(r"[0-9a-f]{64}")
PROJECT = re.compile(r"[a-z0-9][a-z0-9_-]{0,62}")
CODE = re.compile(r"[A-Z][A-Z0-9_]{0,100}")
PROGRESS_LIMIT = 64 * 1024
CHILD_PATH = "/usr/sbin:/usr/bin:/sbin:/bin"
# Restart policies a tool may record and later restore.  Anything else
# (including on-failure with a retry count) fails closed.
RESTART_POLICIES = ("no", "always", "unless-stopped")
STOPPED_POLICY = {"Name": "no", "MaximumRetryCount": 0}


class ToolError(Exception):
    """Fixed upper-case code only; never attach input, output or a path."""

    def __init__(self, code, cause=None):
        super().__init__(code if CODE.fullmatch(code or "") else "TOOL_INTERNAL")
        self.code = str(self)
        self.cause = cause if isinstance(cause, str) and CODE.fullmatch(cause) else None


def require(condition, code):
    if not condition:
        raise ToolError(code)


def cause_of(exc):
    """A library's fixed diagnostic code, if it has exactly that shape."""
    if isinstance(exc, ToolError):
        return exc.cause
    text = str(exc) if exc.args else ""
    return text if CODE.fullmatch(text) else None


def is_commit(value):
    return isinstance(value, str) and bool(COMMIT.fullmatch(value)) and value != "0" * 40


def absolute(path):
    return (isinstance(path, str) and path.startswith("/") and path != "/"
            and all(part not in ("", ".", "..") for part in path[1:].split("/"))
            and not any(ord(c) < 32 or c == "\x7f" for c in path))


def is_date(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", value):
        return False
    try:
        datetime.date.fromisoformat(value)
    except ValueError:
        return False
    return True


class Parser(argparse.ArgumentParser):
    """argparse that never echoes a rejected value."""

    def __init__(self, *args, code, **kwargs):
        self.code = code
        # An abbreviation (--state-di) would slip past reject_repeated.
        kwargs["allow_abbrev"] = False
        super().__init__(*args, **kwargs)

    def error(self, message):  # noqa: ARG002 - message may contain values
        raise ToolError(self.code)


def reject_repeated(argv, options, code):
    """argparse keeps the last of a repeated option; refuse instead."""
    for option in options:
        count = sum(1 for token in argv if token == option or token.startswith(option + "="))
        require(count <= 1, code)


def canonical(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True,
                       allow_nan=False) + "\n").encode("ascii")


def strict_json(data, code):
    def pairs(items):
        result = {}
        for key, item in items:
            require(key not in result, code)
            result[key] = item
        return result

    def constant(_):
        raise ToolError(code)

    try:
        return json.loads(data.decode("ascii"), object_pairs_hook=pairs, parse_constant=constant)
    except (UnicodeError, ValueError):
        raise ToolError(code) from None


def state_directory(path, uid, code):
    """Explicit, owner-only directory; never created or derived here."""
    require(absolute(path), code)
    try:
        info = os.lstat(path)
    except OSError:
        raise ToolError(code) from None
    require(stat.S_ISDIR(info.st_mode) and info.st_uid == uid
            and stat.S_IMODE(info.st_mode) & 0o077 == 0, code)
    return path


def _fsync_directory(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


class Progress:
    """One root 0600 JSON record; created O_EXCL and replaced atomically."""

    def __init__(self, directory, name, uid, code):
        require(re.fullmatch(r"[a-z][a-z0-9-]{0,62}", name), code)
        self.directory, self.uid, self.code = directory, uid, code
        self.path = directory + "/" + name + ".json"
        self.lock_path = directory + "/" + name + ".lock"
        self._lock_fd = None

    def lock(self):
        try:
            fd = os.open(self.lock_path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
        except OSError:
            raise ToolError(self.code) from None
        try:
            info = os.fstat(fd)
            require(stat.S_ISREG(info.st_mode) and info.st_uid == self.uid
                    and stat.S_IMODE(info.st_mode) == 0o600 and info.st_nlink == 1, self.code)
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except OSError:
                raise ToolError(self.code) from None
        except BaseException:
            os.close(fd)
            raise
        self._lock_fd = fd

    def unlock(self):
        if self._lock_fd is not None:
            os.close(self._lock_fd)
            self._lock_fd = None

    def read(self):
        try:
            fd = os.open(self.path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        except FileNotFoundError:
            return None
        except OSError:
            raise ToolError(self.code) from None
        try:
            info = os.fstat(fd)
            require(stat.S_ISREG(info.st_mode) and info.st_uid == self.uid
                    and stat.S_IMODE(info.st_mode) == 0o600 and info.st_nlink == 1
                    and info.st_size <= PROGRESS_LIMIT, self.code)
            data = os.read(fd, PROGRESS_LIMIT + 1)
        finally:
            os.close(fd)
        value = strict_json(data, self.code)
        require(isinstance(value, dict) and canonical(value) == data, self.code)
        return value

    def _write(self, path, data):
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
        try:
            os.fchmod(fd, 0o600)
            view = memoryview(data)
            while view:
                view = view[os.write(fd, view):]
            os.fsync(fd)
        finally:
            os.close(fd)

    def create(self, value):
        data = canonical(value)
        require(len(data) <= PROGRESS_LIMIT, self.code)
        try:
            self._write(self.path, data)
            _fsync_directory(self.directory)
        except OSError:
            raise ToolError(self.code) from None
        return value

    def replace(self, previous, value):
        data = canonical(value)
        require(len(data) <= PROGRESS_LIMIT and self.read() == previous, self.code)
        temporary = self.path + "." + str(os.getpid()) + ".tmp"
        try:
            self._write(temporary, data)
            os.replace(temporary, self.path)
            _fsync_directory(self.directory)
        except OSError:
            try:
                os.unlink(temporary)
            except OSError:
                pass
            raise ToolError(self.code) from None
        return value

    def retire(self, previous, stamp):
        """Move a finished record aside without clobbering anything (link, then unlink)."""
        require(re.fullmatch(r"[0-9]{8}T[0-9]{6}Z", stamp or "") and self.read() == previous, self.code)
        target = self.path[:-len(".json")] + ".retired-" + stamp + ".json"
        try:
            os.link(self.path, target, follow_symlinks=False)
        except OSError:
            raise ToolError(self.code) from None
        try:
            os.unlink(self.path)
            _fsync_directory(self.directory)
        except OSError:
            raise ToolError(self.code) from None
        return target


def utc_stamp(now=None):
    value = datetime.datetime.now(datetime.timezone.utc) if now is None else now
    return value.strftime("%Y%m%dT%H%M%SZ")


def trusted_chain(root, relative, uid, code):
    """root and every component below it: no symlink, owned by uid, not g/o-writable."""
    require(absolute(root) and isinstance(relative, str) and relative
            and all(p not in ("", ".", "..") for p in relative.split("/")), code)
    path = root
    parts = relative.split("/")
    for index in range(len(parts) + 1):
        if index:
            path = path + "/" + parts[index - 1]
        try:
            info = os.lstat(path)
        except OSError:
            raise ToolError(code) from None
        last = index == len(parts)
        require((stat.S_ISREG(info.st_mode) if last else stat.S_ISDIR(info.st_mode))
                and info.st_uid == uid and info.st_mode & 0o022 == 0, code)
    return path


def load_module(name, root, relative, uid, code, *, siblings=()):
    """Load installed code by path, as OPS does, after the ownership check."""
    path = trusted_chain(root, relative, uid, code)
    directory = relative.rsplit("/", 1)[0]
    for sibling in siblings:
        trusted_chain(root, directory + "/" + sibling, uid, code)
    sys.dont_write_bytecode = True
    spec = importlib.util.spec_from_file_location(name, path)
    require(spec is not None and spec.loader is not None, code)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    try:
        spec.loader.exec_module(module)
    except Exception:
        sys.modules.pop(name, None)
        raise ToolError(code) from None
    return module


def restart_policy(value, code):
    """Exact two-key RestartPolicy dict from a container inspect value."""
    policy = (value.get("HostConfig") or {}).get("RestartPolicy") if isinstance(value, dict) else None
    require(isinstance(policy, dict) and set(policy) == {"Name", "MaximumRetryCount"}
            and policy["Name"] in RESTART_POLICIES
            and type(policy["MaximumRetryCount"]) is int and policy["MaximumRetryCount"] == 0, code)
    return {"Name": policy["Name"], "MaximumRetryCount": 0}


def restart_argument(policy, code):
    require(isinstance(policy, dict) and set(policy) == {"Name", "MaximumRetryCount"}
            and policy["Name"] in RESTART_POLICIES and policy["MaximumRetryCount"] == 0, code)
    return "--restart=" + policy["Name"]


def compose_ids(docker, project, service, code, timeout=15):
    """Every container (any state) of one Compose project service."""
    require(PROJECT.fullmatch(project or "") and re.fullmatch(r"[a-z][a-z0-9-]{0,62}", service), code)
    try:
        lines = docker.call(["ps", "--all", "--no-trunc", "--filter",
                             "label=com.docker.compose.project=" + project, "--filter",
                             "label=com.docker.compose.service=" + service,
                             "--format", "{{.ID}}"], timeout).splitlines()
    except Exception as exc:
        raise ToolError(code, cause_of(exc)) from None
    require(all(CONTAINER.fullmatch(line) for line in lines), code)
    return lines


def running_healthy(value):
    state = value.get("State") or {}
    return (state.get("Running") is True and not state.get("Restarting") and not state.get("Paused")
            and not state.get("OOMKilled") and not state.get("Dead") and not state.get("Error")
            and (state.get("Health") or {}).get("Status") == "healthy")


def exited_zero(value):
    state = value.get("State") or {}
    return (state.get("Running") is False and not state.get("Restarting") and not state.get("Paused")
            and not state.get("OOMKilled") and not state.get("Dead") and not state.get("Error")
            and state.get("Status") == "exited" and type(state.get("ExitCode")) is int
            and state.get("ExitCode") == 0)


def child_environment(extra=None):
    env = {"PATH": CHILD_PATH, "HOME": "/root", "LANG": "C.UTF-8", "PYTHONDONTWRITEBYTECODE": "1"}
    env.update(extra or {})
    return env


def _signal_group(process, signum):
    try:
        os.killpg(process.pid, signum)
    except (ProcessLookupError, PermissionError):
        pass


def run_child(argv, env, timeout, *, term_grace=600, kill_grace=30):
    """Bounded child in its own session; on timeout TERM the whole group first.

    A ``docker compose`` grandchild shares the group, so it is signalled too.
    Returns (returncode, stdout, stderr); returncode None means the outcome is
    unknown (timeout or spawn failure).  Output stays in memory.  Every wait,
    including the last one after SIGKILL, is bounded.
    """
    try:
        process = subprocess.Popen(argv, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, close_fds=True, start_new_session=True)
    except OSError:
        return None, b"", b""
    try:
        out, err = process.communicate(timeout=timeout)
        return process.returncode, out, err
    except subprocess.TimeoutExpired:
        pass
    _signal_group(process, signal.SIGTERM)
    try:
        out, err = process.communicate(timeout=term_grace)
        return None, out, err
    except subprocess.TimeoutExpired:
        pass
    _signal_group(process, signal.SIGKILL)
    try:
        out, err = process.communicate(timeout=kill_grace)
    except subprocess.TimeoutExpired:
        out, err = b"", b""
    return None, out, err


def same_generation(generation, recorded):
    """Exact, or the same container after an explicit ``docker start``.

    A new StartedAt with the same Id/Created is our own restart; RestartCount
    may stay or be reset to 0 by a manual start (Docker behavior not verified).
    """
    generation, recorded = list(generation), list(recorded)
    if generation[2] == recorded[2]:
        return generation == recorded
    return generation[:2] == recorded[:2] and generation[3] in (recorded[3], 0)


def restore_generation(docker, observe, recorded, policy, *, signalled, clock, sleep,
                       health_seconds, poll_seconds, codes):
    """Return one exact recorded generation to service with its recorded policy.

    ``observe()`` returns (inspect value, [id, created, started_at, restarts]) for
    the recorded container only and raises when the service resolves to anything
    else.  ``signalled(value)`` is True when a TERM may already have been sent to
    the still-running original start; such a process is never restarted.  Never
    creates, stops, kills or removes a container.  Returns "unchanged" (policy
    only, original start kept) or "started".
    """
    argument = restart_argument(policy, codes["policy"])
    identity = recorded[0]
    value, generation = observe()
    require(same_generation(generation, recorded), codes["changed"])

    def set_policy():
        try:
            docker.call(["update", argument, identity], 15)
        except Exception as exc:
            raise ToolError(codes["policy"], cause_of(exc)) from None
        current, after = observe()
        require(same_generation(after, recorded) and after[2] == generation[2], codes["changed"])
        require(restart_policy(current, codes["policy"]) == policy, codes["policy"])
        return current

    state = value.get("State") or {}
    if state.get("Running") is True:
        if generation[2] == recorded[2]:
            require(not signalled(value), codes["state"])
            require(running_healthy(set_policy()), codes["health"])
            return "unchanged"
        set_policy()  # an earlier restore already started this exact container
    else:
        # Only the exact stopped original is ever started; a restarted-then-exited
        # generation is left for the operator.
        require(generation == list(recorded) and state.get("Status") == "exited"
                and not state.get("Restarting") and not state.get("Paused")
                and not state.get("Dead") and not state.get("OOMKilled"), codes["state"])
        require((set_policy().get("State") or {}).get("Running") is False, codes["state"])
        try:
            docker.call(["start", identity], 30)
        except Exception as exc:
            raise ToolError(codes["start"], cause_of(exc)) from None
    deadline = clock() + health_seconds
    started = None
    while True:
        current, after = observe()
        require(same_generation(after, recorded) and after[2] != recorded[2]
                and (started is None or after[2] == started), codes["changed"])
        started = after[2]
        state = current.get("State") or {}
        require(state.get("Running") is True and not state.get("Restarting")
                and not state.get("Paused") and not state.get("OOMKilled")
                and not state.get("Dead") and not state.get("Error")
                and (state.get("Health") or {}).get("Status") in ("starting", "healthy"), codes["health"])
        require(restart_policy(current, codes["policy"]) == policy, codes["policy"])
        if running_healthy(current):
            return "started"
        require(clock() < deadline, codes["health"])
        sleep(poll_seconds)


def stdout_lines(data):
    try:
        text = data.decode("utf-8", errors="strict")
    except UnicodeError:
        return None
    return [line for line in text.splitlines() if line.strip()]


OPS_ENTRY = "owner-beta-daily-operations.py"
OPS_SIBLINGS = ("owner-beta-daily-release.py", "owner-beta-daily-base.py",
                "owner-beta-daily-runtime.py", "service-status-policy.py")
TOOL_ENTRY = "production-backup-tool.py"
TOOL_SIBLINGS = ("owner-beta-daily-release.py", "owner-beta-daily-backup.py")


class ReleaseHost:
    """Production adapter: the installed application release's own code.

    Ownership, manifest, generation and drain primitives come from
    ``<install-root>/releases/<application-commit>/scripts/ops/lib`` exactly as
    the installed wrapper loads them.  Docker is always the local daemon.
    """

    def __init__(self, install_root, application_commit, code):
        require(sys.platform == "linux" and os.geteuid() == 0, code)
        require(absolute(install_root) and is_commit(application_commit), code)
        self.uid, self.code = 0, code
        self.install_root, self.commit = install_root, application_commit
        self.release = install_root + "/releases/" + application_commit
        self.ops = load_module("reader_generation_release_operations", install_root,
                               "releases/" + application_commit + "/scripts/ops/lib/" + OPS_ENTRY,
                               0, code, siblings=OPS_SIBLINGS)
        self.runtime = self.ops.runtime
        self.docker = self.runtime.Docker()

    def validate_current(self):
        self.ops.policy.validate_current_release(self.install_root, self.release, self.commit)

    def manifest_images(self, commit):
        require(is_commit(commit), self.code)
        return self.ops.read_manifest(self.install_root + "/releases/" + commit, commit)

    def project(self):
        return self.ops.project_binding(self.commit, self.commit)[0]

    def ownership(self, project, commit, image):
        return self.ops.Ownership(project, commit, image)

    def owned(self, value, own, service, identity):
        return tuple(self.runtime.owned(value, own, service, identity))

    def global_cohort(self, own):
        self.ops.global_cohort(self.docker, own)

    def capture(self, own):
        return self.runtime.capture(self.docker, own)

    def drain(self, own, snapshot):
        return self.runtime.drain(self.docker, own, snapshot)

    def verify_tool(self, tool_commit):
        """The pinned maintenance package's own manifest check, bound to this release."""
        require(is_commit(tool_commit), self.code)
        relative = "maintenance/" + tool_commit + "/scripts/ops/lib/"
        tool = load_module("reader_generation_backup_tool_" + tool_commit, self.install_root,
                           relative + TOOL_ENTRY, 0, self.code, siblings=TOOL_SIBLINGS)
        from pathlib import Path
        return tool.read_manifest(Path(self.install_root) / "maintenance" / tool_commit, self.commit,
                                  install_root=Path(self.install_root))
