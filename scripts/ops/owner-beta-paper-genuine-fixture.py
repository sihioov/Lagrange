#!/usr/bin/env python3
"""Disposable selected-image Root/PG fixture. Import has no privileged effects."""
from __future__ import annotations

import argparse
import contextlib
import ctypes
import fcntl
import functools
import hashlib
import importlib.util
import json
import os
import pathlib
import re
import selectors
import shutil
import signal
import stat
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import types
import uuid

sys.dont_write_bytecode = True
ROOT = pathlib.Path(__file__).resolve().parents[2]


def load(name, path):
    if name in sys.modules:
        return sys.modules[name]
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


driver = load("paper_genuine_fixture_driver", ROOT / "scripts/ops/owner-beta-paper-operations-docker-self-test.py")
ops = driver.ops
faults = load("paper_genuine_fixture_faults", ROOT / "scripts/ops/lib/owner-beta-paper-fixture-faults.py")
BEGIN_SELECT = "selectpublic.begin_owner_beta_paper_operation_v1($1::bigint,$2::jsonb,$3::text)"
GIT_ENV = {"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C", "TZ": "UTC",
           "GIT_OPTIONAL_LOCKS": "0", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null"}
ROLES = ("migration_owner", "app", "worker", "admin", "research_writer",
         "audit_writer", "daily_input_admitter")
DEV_CODE_OVERLAY_MODE = "PAPER_GENUINE_FIXTURE_MODE=DEV_CODE_OVERLAY"


class FixtureError(Exception):
    """Only closed diagnostic codes cross the fixture boundary."""


class FixtureNotRun(FixtureError):
    """Required hosted resources are absent; this is never a pass receipt."""


def private_tree_witness(root, *, maximum=10000):
    """Bounded identity/hash witness; returns no file bytes or untrusted names."""
    root = pathlib.Path(root)
    require(stat.S_ISDIR(root.lstat().st_mode), "FIXTURE_ARCHIVE_WITNESS")
    digest, count = hashlib.sha256(), 0
    for parent, directories, names in os.walk(root, followlinks=False):
        directories.sort()
        for leaf in sorted([*directories, *names]):
            path = pathlib.Path(parent) / leaf
            info = path.lstat()
            count += 1
            require(count <= maximum and not stat.S_ISLNK(info.st_mode), "FIXTURE_ARCHIVE_WITNESS")
            record = [str(path.relative_to(root)), info.st_dev, info.st_ino,
                      info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode), info.st_size if path.is_file() else 0]
            if stat.S_ISREG(info.st_mode):
                require(info.st_nlink == 1 and info.st_size <= 2 * 1024**3, "FIXTURE_ARCHIVE_WITNESS")
                value = hashlib.sha256()
                with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC), "rb") as stream:
                    require((os.fstat(stream.fileno()).st_dev, os.fstat(stream.fileno()).st_ino) ==
                            (info.st_dev, info.st_ino), "FIXTURE_ARCHIVE_WITNESS")
                    while chunk := stream.read(1024 * 1024):
                        value.update(chunk)
                    end = os.fstat(stream.fileno())
                    require((end.st_size, end.st_mtime_ns, end.st_ctime_ns) ==
                            (info.st_size, info.st_mtime_ns, info.st_ctime_ns), "FIXTURE_ARCHIVE_WITNESS")
                record.append(value.hexdigest())
            else:
                require(stat.S_ISDIR(info.st_mode), "FIXTURE_ARCHIVE_WITNESS")
            digest.update(canonical(record) + b"\n")
    return {"entries": count, "sha256": digest.hexdigest()}


def complete_sets_witness(root):
    root = pathlib.Path(root)
    paper = driver._paper_backup()
    values = {}
    names = list(root.iterdir())
    require(len(names) <= 10000, "FIXTURE_ARCHIVE_WITNESS")
    for path in names:
        if paper._PAPER_SET.fullmatch(path.name) and (path / "COMPLETE").exists():
            values[path.name] = private_tree_witness(path, maximum=8)
    latest = root / "latest"
    require(not latest.exists() or latest.is_symlink(), "FIXTURE_ARCHIVE_WITNESS")
    return {"sets": values, "latest": os.readlink(latest) if latest.is_symlink() else None}


def require(value, code):
    if not value:
        raise FixtureError(code)


def run_simulation_operation(module, failure_code, operation, *args):
    try:
        return operation(*args)
    except module.OperationError:
        raise FixtureError(failure_code) from None


def pre_paper_api_role_passwords(observer):
    passwords = {role: uuid.uuid4().hex + uuid.uuid4().hex
                 for role in ("app", "admin", "audit_writer")}
    observer.sql("".join("ALTER ROLE " + role + " PASSWORD '" + password + "';"
                         for role, password in passwords.items()))
    return passwords


def wait_for_pre_paper_api_ready(docker, deadline, simulation_ops, project, commit, image, cid, generation):
    until = time.monotonic() + deadline.seconds(45)
    health = None
    while time.monotonic() < until:
        value = docker.object("container", cid, deadline.seconds(15))
        state = value.get("State") if type(value) is dict else None
        health = state.get("Health") if type(state) is dict else None
        require(type(value) is dict and value.get("Id") == cid and value.get("Image") == image and
                type(state) is dict and state.get("Running") is True and type(health) is dict and
                health.get("Status") in ("starting", "healthy"), "FIXTURE_PRE_PAPER_SOURCE_API")
        if health["Status"] == "healthy":
            break
        time.sleep(min(0.1, deadline.seconds(0.1)))
    require(type(health) is dict and health.get("Status") == "healthy" and
            run_simulation_operation(simulation_ops, "FIXTURE_PRE_PAPER_SOURCE_API",
                simulation_ops._api_ids, docker, deadline.seconds(15)) == [cid],
            "FIXTURE_PRE_PAPER_SOURCE_API")
    observed, running, _policy = run_simulation_operation(simulation_ops,
        "FIXTURE_PRE_PAPER_SOURCE_API", simulation_ops._api_observe,
        docker, project, commit, image, cid, deadline.seconds(15))
    require(observed == generation and running, "FIXTURE_PRE_PAPER_SOURCE_API")
    return observed


class Deadline:
    def __init__(self, value):
        require(type(value) is int and value > time.monotonic_ns() + 120_000_000_000,
                "FIXTURE_CLEANUP_RESERVE")
        self.value = value
        self.alarm_installed = False

    def seconds(self, cap=30, *, cleanup=False):
        left = (self.value - time.monotonic_ns()) / 1_000_000_000
        if not cleanup:
            left -= 120
        require(left > 0, "FIXTURE_DEADLINE")
        return min(cap, left)

    def action_phase(self):
        left = self.seconds(86400)
        if self.alarm_installed:
            signal.setitimer(signal.ITIMER_REAL, left)

    def cleanup_phase(self):
        if self.alarm_installed:
            signal.setitimer(signal.ITIMER_REAL, self.seconds(86400, cleanup=True))


@contextlib.contextmanager
def bounded_run(deadline):
    require(threading.current_thread() is threading.main_thread(), "FIXTURE_MAIN_THREAD")
    require(signal.getitimer(signal.ITIMER_REAL) == (0.0, 0.0), "FIXTURE_TIMER_ALREADY_OWNED")
    def expired(_signum, _frame):
        # The first cutoff leaves exactly the remaining job cleanup budget.
        deadline.cleanup_phase()
        raise FixtureError("FIXTURE_DEADLINE")
    previous = signal.signal(signal.SIGALRM, expired)
    deadline.alarm_installed = True
    try:
        deadline.action_phase()
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        deadline.alarm_installed = False
        signal.signal(signal.SIGALRM, previous)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


_DEV_TIMING_TARGETS = (
    ("_NativeInvocation", ("__enter__", "_prepare", "_prepare_inspect", "_run",
                           "_inspect_and_validate", "import_dispatch", "close")),
    ("_OffCommit", ("run", "refresh", "prepare", "resolve", "readback", "observe")),
    ("_PaperContext", ("revalidate",)),
    ("DrainedHandle", ("revalidate",)),
    ("_StopAttempt", ("run", "inventory", "check")),
    ("_HostStopObserver", ("capture", "units_now", "containers_now", "other_launch_sources",
                           "terminal_observation", "database")),
)
_DEV_TIMING = {}


def _timed(key_function, store):
    """Wrap a function so each call's wall time accumulates in store[key]; behaviour is unchanged."""
    def decorate(function):
        @functools.wraps(function)
        def wrapper(*args, **kwargs):
            started = time.monotonic_ns()
            try:
                return function(*args, **kwargs)
            finally:
                elapsed = (time.monotonic_ns() - started) // 1_000_000
                entry = store.setdefault(key_function(args), {"count": 0, "total_ms": 0, "max_ms": 0})
                entry["count"] += 1
                entry["total_ms"] += elapsed
                entry["max_ms"] = max(entry["max_ms"], elapsed)
        return wrapper
    return decorate


def _install_dev_timing():
    """Dev-mode only: time fixed ops methods. Idempotent; returns report()."""
    if _DEV_TIMING:
        return _DEV_TIMING["report"]
    store, missing, originals = {}, [], []
    for class_name, names in _DEV_TIMING_TARGETS:
        cls = getattr(ops, class_name, None)
        for name in names:
            original = vars(cls).get(name) if cls is not None else None
            if not isinstance(original, types.FunctionType):
                missing.append(class_name + "." + name)
                continue
            if class_name == "_NativeInvocation":
                def key_function(args, label=class_name + "." + name):
                    verb = getattr(args[0], "verb", None) if args else None
                    if not (isinstance(verb, str) and re.fullmatch("[a-z-]{1,24}", verb)):
                        verb = "unknown"
                    return label + "[" + verb + "]"
            else:
                def key_function(_args, label=class_name + "." + name):
                    return label
            setattr(cls, name, _timed(key_function, store)(original))
            originals.append((cls, name, original))

    def report():
        return {"calls": {key: dict(store[key]) for key in sorted(store)}, "missing": sorted(missing)}
    _DEV_TIMING.update(report=report, originals=originals, store=store)
    return report


def _uninstall_dev_timing():
    """Test-only: restore the original methods and forget the installation."""
    for cls, name, original in _DEV_TIMING.get("originals", ()):
        setattr(cls, name, original)
    _DEV_TIMING.clear()


def write_new(path, data, mode=0o600, uid=0, gid=0):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, mode)
    try:
        os.fchmod(fd, mode)
        os.fchown(fd, uid, gid)
        view = memoryview(data)
        while view:
            view = view[os.write(fd, view):]
        os.fsync(fd)
    finally:
        os.close(fd)


def validated_inputs(source_root, commit, manifest_path, deadline_ns, *, dev_code_overlay=False):
    deadline = Deadline(deadline_ns)
    source = pathlib.Path(source_root).resolve(strict=True)
    # DEV_CODE_OVERLAY relaxes only the code-root equality: this fixture code may come from a
    # later commit than the release source and images. Every other input check is unchanged.
    require((dev_code_overlay is True or source == ROOT) and re.fullmatch("[0-9a-f]{40}", commit or "")
            and commit != "0" * 40, "FIXTURE_SOURCE")
    def git(*args):
        process = subprocess.run(["/usr/bin/git", "--no-replace-objects", "-c", "core.fsmonitor=false",
                                  "-c", "safe.directory=" + str(source),
                                  "-C", str(source), *args], env=GIT_ENV, capture_output=True,
                                 timeout=deadline.seconds(15), check=False)
        require(process.returncode == 0, "FIXTURE_GIT")
        return process.stdout
    require(git("rev-parse", "HEAD").strip().decode() == commit
            and git("status", "--porcelain", "--untracked-files=normal") == b"",
            "FIXTURE_CANDIDATE_NOT_CLEAN")
    manifest = pathlib.Path(manifest_path)
    require(str(manifest).startswith("/var/tmp/lagrange-paper-fixture-build-")
            and manifest.is_absolute() and manifest.resolve() == manifest,
            "FIXTURE_MANIFEST_NAMESPACE")
    # The G1 handoff is intentionally under sticky /var/tmp. The production
    # trusted-path reader correctly rejects that ancestor. Capture its actual
    # private no-follow file here; the unchanged trusted manifest validator runs
    # after the exact bytes are installed below the new /var/lib fixture root.
    parent_fd = os.open(manifest.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    fd = None
    try:
        ops._dir(parent_fd)
        fd = os.open(manifest.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=parent_fd)
        info = ops._file(fd, 0o600)
        identity = ops._identity(info)
        data = os.pread(fd, 65537, 0)
        require(len(data) <= 65536 and ops._identity(os.fstat(fd)) == identity and
                ops._identity(os.stat(manifest.name, dir_fd=parent_fd, follow_symlinks=False)) == identity,
                "FIXTURE_MANIFEST_CHANGED")
        validate_v3_bytes(data, commit)
    finally:
        if fd is not None:
            os.close(fd)
        os.close(parent_fd)
    return source, data, deadline


def validate_v3_bytes(data, commit):
    """Private handoff syntax; the installed production reader remains required."""
    require(type(data) is bytes and data.endswith(b"\n") and b"\r" not in data,
            "FIXTURE_MANIFEST_V3")
    try:
        lines = data[:-1].decode("ascii").split("\n")
    except UnicodeError:
        raise FixtureError("FIXTURE_MANIFEST_V3") from None
    services = ops.daily.policy.MANIFEST_V3_SERVICES
    require(len(lines) == len(services) + 2 and lines[:2] ==
            ["LAGRANGE_RELEASE_MANIFEST_V3", "commit|" + commit], "FIXTURE_MANIFEST_V3")
    for service, line in zip(services, lines[2:]):
        fields = line.split("|")
        require(len(fields) == 5 and fields[:3] ==
                ["image", service, "lagrange-station-" + service + ":" + commit]
                and re.fullmatch("sha256:[0-9a-f]{64}", fields[3]) and fields[4] == commit,
                "FIXTURE_MANIFEST_V3")


def existing_json_macro(source, function):
    """Read only the two fixed existing Rust fixture literals; execute no Rust."""
    allowed = {"daily_0060_policy", "financial_contract"}
    require(function in allowed, "FIXTURE_SEED_SELECTOR")
    text = source.read_text()
    match = re.search(r"\bfn " + function + r"\([^)]*\)[^{]*\{", text)
    require(match is not None, "FIXTURE_SEED_SOURCE")
    begin = text.index("json!(", match.end()) + len("json!(")
    end = text.index("\n    })", begin) + len("\n    }") if function == "daily_0060_policy" else text.index("})", begin) + 1
    literal = text[begin:end].strip()
    literal = re.sub(r"daily_0060_hash_value\('([1-5])'\)",
                     lambda m: json.dumps("sha256:" + m[1] * 64), literal)
    try:
        value = json.loads(literal)
    except (ValueError, TypeError):
        raise FixtureError("FIXTURE_SEED_SOURCE") from None
    require(type(value) is dict, "FIXTURE_SEED_SOURCE")
    return value


class PrivatePolicy:
    """A private mount namespace changes no existing host policy bytes."""
    def __init__(self, source):
        self.source, self.original, self.target_id = source, None, None
        self.created = self.entered = self.bound = False
        self.libc = ctypes.CDLL(None, use_errno=True)

    def __enter__(self):
        require(os.geteuid() == 0 and threading.active_count() == 1, "FIXTURE_ROOT_NAMESPACE")
        self.original = os.open("/proc/self/ns/mnt", os.O_RDONLY | os.O_CLOEXEC)
        try:
            require(self.libc.unshare(0x20000) == 0, "FIXTURE_UNSHARE")
            self.entered = True
            require(self.libc.mount(None, b"/", None, 1 << 18 | 1 << 14, None) == 0,
                    "FIXTURE_PRIVATE_MOUNTS")
            target = pathlib.Path("/var/lib/lagrange")
            try:
                target.mkdir(mode=0o700)
                self.created = True
            except FileExistsError:
                require(target.is_dir() and not target.is_symlink(), "FIXTURE_POLICY_TARGET")
            self.target_id = faults.directory_identity(target)
            require(self.libc.mount(os.fsencode(self.source), os.fsencode(target), None, 4096, None) == 0,
                    "FIXTURE_POLICY_BIND")
            self.bound = True
            return self
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def __exit__(self, *_exc):
        if self.bound:
            require(self.libc.umount2(b"/var/lib/lagrange", 0) == 0, "FIXTURE_POLICY_UNMOUNT")
            self.bound = False
        if self.entered:
            require(self.libc.setns(self.original, 0x20000) == 0, "FIXTURE_NAMESPACE_RESTORE")
            self.entered = False
        if self.created:
            require(faults.directory_identity("/var/lib/lagrange") == self.target_id,
                    "FIXTURE_POLICY_TARGET_CHANGED")
            pathlib.Path("/var/lib/lagrange").rmdir()
            self.created = False
        if self.original is not None:
            os.close(self.original)
            self.original = None



SERVICE_STATUS_TARGET = pathlib.Path('/var/lib/lagrange/service-status')
_UNRESOLVED_POLICY_SCOPES = set()


def _service_status_evidence():
    path = ROOT / 'scripts/ops/lib/owner-beta-paper-evidence.py'
    module = load('paper_genuine_service_evidence', path)
    require(pathlib.Path(module.__file__).resolve() == path, 'FIXTURE_SOURCE')
    return module


def service_status_drop_body(policy):
    """One fixed private policy target, never a caller-selected mount target."""
    policy = pathlib.Path(policy)
    require(re.fullmatch('/[A-Za-z0-9/._-]+', str(policy)) is not None
        and '..' not in policy.parts, 'FIXTURE_POLICY_TARGET')
    return ('[Service]\nBindReadOnlyPaths=' + str(policy / 'service-status')
        + ':' + str(SERVICE_STATUS_TARGET) + '\n').encode('ascii')


def service_status_namespace(policy, pid):
    """Observe the actual leaf projection and unchanged parent in a real process."""
    require(type(pid) is int and pid > 1, 'FIXTURE_ROOT_NAMESPACE')
    policy = pathlib.Path(policy)
    service_status_drop_body(policy)  # same closed path grammar as the installed drop-in
    text = pathlib.Path(f'/proc/{pid}/mountinfo').read_text()
    matches = [line.split() for line in text.splitlines()
        if len(line.split()) > 6 and line.split()[4] == str(SERVICE_STATUS_TARGET)]
    require(len(matches) == 1 and 'ro' in matches[0][5].split(','), 'FIXTURE_ROOT_NAMESPACE')
    mounted = pathlib.Path(f'/proc/{pid}/root') / str(SERVICE_STATUS_TARGET).lstrip('/')
    for actual, original in ((mounted.parent, SERVICE_STATUS_TARGET.parent),
            (mounted, policy / 'service-status'),
            (mounted / 'policy.json', policy / 'service-status/policy.json')):
        require(ops._identity(os.stat(actual)) == ops._identity(os.stat(original)), 'FIXTURE_POLICY_TARGET_CHANGED')
    return mounted


class ServiceStatusScope:
    """Private actual-material namespace projection; original /var/lib/lagrange stays visible.

    Only the existing protected policy directory is shadowed. Actual Raw/base,
    config and shared credential-state originals keep their original paths and
    inodes. No host target is created and no host policy contents are changed.
    """
    def __init__(self, policy):
        self.policy = pathlib.Path(policy)
        self.stack = contextlib.ExitStack()
        self.entered = self.bound = False
        self.original = None
        self.pins = []
        self.libc = ctypes.CDLL(None, use_errno=True)

    def _check(self, value, operation):
        code = {'NAMESPACE': 'FIXTURE_ROOT_NAMESPACE', 'OWNER': 'FIXTURE_POLICY_TARGET',
            'CHANGED': 'FIXTURE_POLICY_TARGET_CHANGED', 'UNSHARE': 'FIXTURE_UNSHARE',
            'PRIVATE': 'FIXTURE_PRIVATE_MOUNTS', 'BIND': 'FIXTURE_POLICY_BIND',
            'READONLY': 'FIXTURE_POLICY_BIND', 'UNMOUNT': 'FIXTURE_POLICY_UNMOUNT',
            'RESTORE': 'FIXTURE_NAMESPACE_RESTORE'}[operation]
        require(value, code)

    def _retain_directory(self, fd, path):
        info = os.fstat(fd)
        identity = faults.directory_identity(path)
        self._check((info.st_dev, info.st_ino) == identity, 'CHANGED')
        self.pins.append((fd, pathlib.Path(path),
            (info.st_dev, info.st_ino, info.st_uid, info.st_gid, info.st_mode)))
        return identity

    def _revalidate_descriptors(self):
        for fd, path, identity in self.pins:
            info = os.fstat(fd)
            self._check((info.st_dev, info.st_ino, info.st_uid, info.st_gid, info.st_mode) == identity, 'CHANGED')
            # The host target is intentionally shadowed while bound; its named
            # identity is checked against the source and again after restoration.
            # Every other original name must still resolve to its held inode.
            if path != SERVICE_STATUS_TARGET:
                self._check(faults.directory_identity(path) == identity[:2], 'CHANGED')

    def __enter__(self):
        e = _service_status_evidence()
        self._check(os.geteuid() == 0 and threading.active_count() == 1, 'NAMESPACE')
        self._check(self.policy.is_absolute() and not self.policy.is_symlink(), 'OWNER')
        try:
            self.base = self.stack.enter_context(e.opened(SERVICE_STATUS_TARGET.parent, directory=True))
            self.base_id = self._retain_directory(self.base, SERVICE_STATUS_TARGET.parent)
            self.target = self.stack.enter_context(e.opened(SERVICE_STATUS_TARGET, directory=True, exact=(0, 0, 0o700)))
            self.target_id = self._retain_directory(self.target, SERVICE_STATUS_TARGET)
            self.parent = os.open(self.policy, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
            self.stack.callback(os.close, self.parent)
            parent_info = os.fstat(self.parent)
            e.no_acl(self.parent)
            self._check((parent_info.st_uid, parent_info.st_gid, parent_info.st_mode & 0o7777)
                == (0, 0, 0o700), 'OWNER')
            self._retain_directory(self.parent, self.policy)
            source = self.policy / 'service-status'
            # Bootstrap's owner-private temporary source is under sticky /var/tmp;
            # retain its actual descriptor before projecting the protected alias.
            self.source = os.open('service-status', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=self.parent)
            self.stack.callback(os.close, self.source)
            info = os.fstat(self.source)
            e.no_acl(self.source)
            self._check((info.st_uid, info.st_gid, info.st_mode & 0o7777) == (0, 0, 0o700), 'OWNER')
            self.source_id = self._retain_directory(self.source, source)
            self.original = os.open('/proc/self/ns/mnt', os.O_RDONLY | os.O_CLOEXEC)
            self.stack.callback(os.close, self.original)
            self._check(self.libc.unshare(0x20000) == 0, 'UNSHARE')
            self.entered = True
            self._check(self.libc.mount(None, b'/', None, 1 << 18 | 1 << 14, None) == 0, 'PRIVATE')
            self._check(self.libc.mount(os.fsencode(source), os.fsencode(SERVICE_STATUS_TARGET), None, 4096, None) == 0,
                    'BIND')
            self.bound = True
            self._check(self.libc.mount(None, os.fsencode(SERVICE_STATUS_TARGET), None, 4096 | 32 | 1, None) == 0,
                    'READONLY')
            self._check(faults.directory_identity(SERVICE_STATUS_TARGET) == self.source_id
                and faults.directory_identity(SERVICE_STATUS_TARGET.parent) == self.base_id, 'CHANGED')
            self._revalidate_descriptors()
            return self
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def _close_scope(self):
        self._revalidate_descriptors()
        if self.bound:
            self._check(faults.directory_identity(SERVICE_STATUS_TARGET) == self.source_id, 'CHANGED')
            self._check(self.libc.umount2(os.fsencode(SERVICE_STATUS_TARGET), 0) == 0, 'UNMOUNT')
            self.bound = False
        if self.entered:
            self._check(self.libc.setns(self.original, 0x20000) == 0, 'RESTORE')
            self.entered = False
        if hasattr(self, 'target_id'):
            self._check(faults.directory_identity(SERVICE_STATUS_TARGET) == self.target_id
                and faults.directory_identity(SERVICE_STATUS_TARGET.parent) == self.base_id, 'CHANGED')
        self.stack.close()

    def __exit__(self, *_exc):
        try:
            self._close_scope()
        except BaseException:
            _UNRESOLVED_POLICY_SCOPES.add(self)
            raise
        _UNRESOLVED_POLICY_SCOPES.discard(self)


def counter_values(value):
    required = {"sql", "function", "events", "receipts", "oid", "catalog", "prepared"}
    require(type(value) is dict and set(value) == required, "FIXTURE_COUNTER_SHAPE")
    require(all(type(value[k]) is int and 0 <= value[k] < 2**63 for k in required),
            "FIXTURE_COUNTER_UNKNOWN")
    require(value["oid"] > 0 and value["catalog"] > 0, "FIXTURE_COUNTER_UNAVAILABLE")
    require(value["prepared"] == 0, "FIXTURE_PREPARED_WORK")
    return value


def counter_delta(before, after):
    counter_values(before)
    counter_values(after)
    require(before["oid"] == after["oid"], "FIXTURE_FUNCTION_REPLACED")
    result = {key: after[key] - before[key] for key in ("sql", "function", "events", "receipts")}
    require(all(item >= 0 for item in result.values()), "FIXTURE_COUNTER_RESET")
    return result


class PgObserver:
    def __init__(self, owner):
        self.owner = owner
        self.generation = None
        self.backends = set()
        self.phase, self.created_ns = "NONE", time.monotonic_ns()

    def target(self, cleanup=False):
        network = self.owner.docker.object("network", self.owner.network)
        require(network.get("Id") == self.owner.network and network.get("Internal") is True
                and (network.get("Labels") or {}).get("com.docker.compose.project") == self.owner.project
                and (network.get("Labels") or {}).get("com.docker.compose.network") == "owner-beta-db",
                "FIXTURE_PG_NETWORK")
        value = self.owner.docker.object("container", self.owner.pg)
        state, config = value.get("State") or {}, value.get("Config") or {}
        networks = value.get("NetworkSettings", {}).get("Networks") or {}
        require(value.get("Id") == self.owner.pg and state.get("Running") is True
                and value.get("Image") == self.owner.pg_image
                and config.get("Image") == ops.POSTGRES_IMAGE
                and config.get("Labels", {}).get("com.docker.compose.project") == self.owner.project
                and config.get("Labels", {}).get("com.docker.compose.service") == "postgres"
                and any(e.get("NetworkID") == self.owner.network and
                        "postgres" in (e.get("Aliases") or []) for e in networks.values()),
                "FIXTURE_PG_IDENTITY")
        generation = (value["Created"], state["StartedAt"], value["RestartCount"], value["Image"])
        if self.generation is None:
            self.generation = generation
        require(generation == self.generation, "FIXTURE_PG_RESTART")
        self.owner.deadline.seconds(5, cleanup=cleanup)

    def sql(self, text, role="lagrange", cap=30, cleanup=False):
        require(role in ("lagrange", *ROLES), "FIXTURE_SQL_ROLE")
        self.target(cleanup)
        result = self.owner.command(
            ["exec", "--interactive", "--user", "999:999", self.owner.pg,
             "psql", "-X", "-qAt", "-U", role, "-d", "lagrange",
             "-v", "ON_ERROR_STOP=1", "-v", "VERBOSITY=sqlstate"],
            text.encode(), cap=cap, cleanup=cleanup, allow_failure=True)
        if result.returncode:
            match = re.search(rb"(?:ERROR|FATAL):\s+([A-Z0-9]{5})(?:\s|$)", result.stderr)
            raise FixtureError("FIXTURE_SQL_" + (match[1].decode() if match else "UNKNOWN"))
        self.target(cleanup)
        return result.stdout.strip()

    def json(self, text, **kwargs):
        try:
            return json.loads(self.sql(text, **kwargs))
        except (ValueError, UnicodeError):
            raise FixtureError("FIXTURE_SQL_SHAPE") from None

    def _statistics(self, cleanup=False):
        # COUNT/SUM cover the entire real statement catalog. No Python missing
        # field/transport error becomes zero. Actual collector/settings are gated.
        query = """
SELECT pg_stat_clear_snapshot();
SELECT json_build_object(
 'sql',(SELECT CASE WHEN count(*)=0 THEN 0 ELSE sum(calls)::bigint END
        FROM public.pg_stat_statements WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database())
          AND toplevel AND regexp_replace(lower(query),'[[:space:]]','','g')=
          'selectpublic.begin_owner_beta_paper_operation_v1($1::bigint,$2::jsonb,$3::text)'),
 'function',pg_stat_get_function_calls('public.begin_owner_beta_paper_operation_v1(bigint,jsonb,text)'::regprocedure::oid),
 'oid','public.begin_owner_beta_paper_operation_v1(bigint,jsonb,text)'::regprocedure::oid::bigint,
 'events',(SELECT count(*) FROM public.owner_beta_paper_events WHERE kind='DRAIN_STARTED'),
 'receipts',(SELECT count(*) FROM public.owner_beta_paper_operations WHERE request_canonical IS NOT NULL),
 'catalog',(SELECT count(*) FROM public.pg_stat_statements),
 'prepared',(SELECT count(*) FROM pg_prepared_xacts WHERE database=current_database()));
"""
        require(self.sql("SELECT current_setting('track_functions')||':'||current_setting('pg_stat_statements.track');",
                         cleanup=cleanup) == b"pl:all", "FIXTURE_COUNTER_SETTINGS")
        # A real successful observer statement is a collector positive control;
        # the later native positive case additionally requires BEGIN/function=1.
        self.sql("SELECT 1;", cleanup=cleanup)
        return self.json(query, cleanup=cleanup)

    def snapshot(self, cleanup=False):
        return counter_values(self._statistics(cleanup))

    def maintenance_snapshot(self):
        """Independent actual SELECT/function/event counts for the two new calls."""
        value = self.json("""SELECT pg_stat_clear_snapshot();
SELECT json_build_object(
 'reconcile_sql',(SELECT COALESCE(sum(calls),0)::bigint FROM public.pg_stat_statements
   WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND toplevel
   AND regexp_replace(lower(query),'[[:space:]]','','g') LIKE 'selectpublic.reconcile_owner_beta_paper_operation_v1(%'),
 'off_sql',(SELECT COALESCE(sum(calls),0)::bigint FROM public.pg_stat_statements
   WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND toplevel
   AND regexp_replace(lower(query),'[[:space:]]','','g') LIKE 'selectpublic.transition_owner_beta_paper_v1(%'),
 'reconcile_function',pg_stat_get_function_calls('public.reconcile_owner_beta_paper_operation_v1(uuid,bigint,text,text)'::regprocedure::oid),
 'off_function',pg_stat_get_function_calls('public.transition_owner_beta_paper_v1(bigint,text,jsonb,text)'::regprocedure::oid),
 'observations',(SELECT count(*) FROM public.owner_beta_paper_operation_observations),
 'off_events',(SELECT count(*) FROM public.owner_beta_paper_events WHERE kind='OFF'));
""")
        require(type(value) is dict and set(value) == {"reconcile_sql", "off_sql",
                "reconcile_function", "off_function", "observations", "off_events"},
                "FIXTURE_COUNTER_SHAPE")
        require(all(type(v) is int and v >= 0 for v in value.values()), "FIXTURE_COUNTER_UNKNOWN")
        return value

    def archive_source_witness(self):
        script = (self.owner.release / "deploy/db/owner-beta-paper-audit.sql").read_text()
        data = self.sql("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n" + script + "\nROLLBACK;",
                        role="migration_owner", cap=30)
        dataset = self.sql("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SELECT COALESCE(jsonb_agg("
            "jsonb_build_object('id',id,'dataset_id',dataset_id,'version',version,'status',status,"
            "'manifest_sha256',manifest_sha256,'storage_path',storage_path) ORDER BY id),'[]'::jsonb) "
            "FROM public.dataset_versions; ROLLBACK;", role="migration_owner")
        cids = self.owner.output(["ps", "-aq", "--no-trunc", "--filter",
                                  "label=com.docker.compose.project=" + self.owner.project]).splitlines()
        require(len(cids) <= 64, "FIXTURE_ARCHIVE_WITNESS")
        launch = []
        for cid in sorted(cids):
            value = self.owner.docker.object("container", cid)
            launch.append([value["Id"], value["Created"], value["Image"], value["RestartCount"],
                           value["State"]["Running"], value["State"]["StartedAt"]])
        return {"audit": hashlib.sha256(data).hexdigest(), "datasets": hashlib.sha256(dataset).hexdigest(),
                "journal": private_tree_witness(self.owner.root / ops.PAPER_DIR),
                "launch": hashlib.sha256(canonical(launch)).hexdigest()}

    def rejection_counts(self):
        # A fresh, never-called PL function can have no statistics entry. Keep
        # that explicit UNKNOWN. RESTORE's required zero helper/SELECT/event
        # proof uses the real complete statement catalog, not an invented zero
        # function counter. Positive native cases require all four counters.
        value = self._statistics()
        required = {"sql", "function", "events", "receipts", "oid", "catalog", "prepared"}
        require(type(value) is dict and set(value) == required, "FIXTURE_COUNTER_SHAPE")
        require(all(type(value[k]) is int and 0 <= value[k] < 2**63 for k in required - {"function"})
                and value["oid"] > 0 and value["catalog"] > 0 and value["prepared"] == 0,
                "FIXTURE_COUNTER_UNKNOWN")
        require(value["function"] is None or (type(value["function"]) is int and value["function"] >= 0),
                "FIXTURE_COUNTER_UNKNOWN")
        network = self.owner.docker.object("network", self.owner.network)
        for cid in network.get("Containers") or {}:
            container = self.owner.docker.object("container", cid)
            require(not (container.get("Config", {}).get("Labels") or {}).get(ops.CONTAINER_LABEL),
                    "FIXTURE_RESTORE_HELPER_CREATED")
        return {"sql": value["sql"], "events": value["events"], "receipts": value["receipts"],
                "function": "UNKNOWN" if value["function"] is None else value["function"],
                "oid": value["oid"]}

    def helper(self, invocation):
        cid = invocation._cid
        require(type(cid) is str and re.fullmatch("[0-9a-f]{64}", cid), "FIXTURE_HELPER_UNKNOWN")
        value = self.owner.docker.object("container", cid)
        require(value.get("Id") == cid and value.get("Image") == self.owner.context.images["paper-scheduler"]
                and (value.get("Config", {}).get("Labels") or {}).get(ops.CONTAINER_LABEL) == invocation._nonce,
                "FIXTURE_HELPER_IDENTITY")
        pid = value.get("State", {}).get("Pid")
        require(type(pid) is int and pid > 1, "FIXTURE_HELPER_PID")
        ticks = faults.start_ticks(pid)
        require(cid in pathlib.Path("/proc/%d/cgroup" % pid).read_text(), "FIXTURE_HELPER_CGROUP")
        endpoints = value.get("NetworkSettings", {}).get("Networks") or {}
        ips = {v.get("IPAddress") for v in endpoints.values() if v.get("NetworkID") == self.owner.network}
        sockets = set()
        for fd in pathlib.Path("/proc/%d/fd" % pid).iterdir():
            try:
                match = re.fullmatch(r"socket:\[([0-9]+)\]", os.readlink(fd))
                if match:
                    sockets.add(match[1])
            except FileNotFoundError:
                continue
        ports = set()
        for name in ("tcp", "tcp6"):
            for line in pathlib.Path("/proc/%d/net/%s" % (pid, name)).read_text().splitlines()[1:]:
                fields = line.split()
                if len(fields) > 9 and fields[9] in sockets and fields[2].endswith(":1538"):
                    ports.add(int(fields[1].split(":")[1], 16))
        self.owner.helpers[cid] = invocation._nonce
        sample_start = time.monotonic_ns()
        rows = self.json("""SELECT COALESCE(json_agg(json_build_object(
 'pid',pid,'start',backend_start::text,'addr',host(client_addr),'port',client_port,
 'blocked',cardinality(pg_blocking_pids(pid))>0,
 'age_ms',ceil(extract(epoch FROM clock_timestamp()-query_start)*1000)::bigint)), '[]'::json)
 FROM pg_stat_activity WHERE datname=current_database() AND usename='migration_owner'
 AND application_name='owner-beta-paper-ops-native';""")
        matches = [r for r in rows if r["addr"] in ips and r["port"] in ports]
        require(len(matches) == 1 and faults.start_ticks(pid) == ticks, "FIXTURE_BACKEND_CORRELATION")
        row = matches[0]
        self.backends.add((row["pid"], row["start"]))
        if row["blocked"]:
            require(type(row["age_ms"]) is int and 0 <= row["age_ms"] < 5000,
                    "FIXTURE_LOCK_WINDOW")
            self.blocked_deadline = sample_start + (5000 - row["age_ms"]) * 1_000_000
        return cid, pid, ticks, row["blocked"]

    def release_blocked(self, blocker):
        require(time.monotonic_ns() < self.blocked_deadline, "FIXTURE_LOCK_WINDOW")
        blocker.release()
        require(time.monotonic_ns() < self.blocked_deadline, "FIXTURE_LOCK_WINDOW")

    def absent(self, cleanup=False):
        for pid, start in self.backends:
            require(type(pid) is int and re.fullmatch(r"[0-9 :.+-]+", start),
                    "FIXTURE_BACKEND_IDENTITY")
            count = self.sql("SELECT count(*) FROM pg_stat_activity WHERE pid=%d AND backend_start='%s'::timestamptz;"
                             % (pid, start), cleanup=cleanup)
            require(count == b"0", "FIXTURE_BACKEND_ALIVE")
        require(self.sql("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() "
                         "AND usename='migration_owner' AND backend_type='client backend';",
                         cleanup=cleanup) == b"0", "FIXTURE_BACKEND_ALIVE")
        require(self.sql("SELECT count(*) FROM pg_prepared_xacts WHERE database=current_database();",
                         cleanup=cleanup) == b"0", "FIXTURE_PREPARED_WORK")

    def receipt(self, generation):
        require(type(generation) is str and re.fullmatch("[0-9a-f-]{36}", generation),
                "FIXTURE_RECEIPT_SELECTOR")
        # Without a row the scalar subquery is SQL NULL, which psql prints as an empty line.
        # JSON null keeps "no receipt" a value the caller can test.
        value = self.json("SELECT COALESCE((SELECT json_build_object('event_id',fence_event_id,"
                          "'epoch',fence_epoch,'result_canonical',result_canonical,"
                          "'result_sha256',result_sha256) FROM public.owner_beta_paper_operations "
                          "WHERE generation='" + generation + "'::uuid),'null'::json);")
        if value is not None:
            require(type(value) is dict and set(value) ==
                    {"event_id", "epoch", "result_canonical", "result_sha256"}, "FIXTURE_RECEIPT_SHAPE")
            require(type(value["result_canonical"]) is str and
                    ops._digest(value["result_canonical"].encode("ascii")) == value["result_sha256"],
                    "FIXTURE_RECEIPT_DIGEST")
        return value

    def settle(self, expected, *, cap=10):
        """Wait for backend exit and real collector publication; never fill gaps."""
        started = time.monotonic()
        until = started + self.owner.deadline.seconds(cap)
        rounds = 0
        while True:
            rounds, actual, last = rounds + 1, None, "NOT_EQUAL"
            try:
                self.absent()
                now = self.snapshot()
                actual = {k: now[k] for k in expected}
                require(all(actual[k] <= expected[k] for k in expected), "FIXTURE_EXCESS_CALLS")
                if actual == expected:
                    return now
            except FixtureError as error:
                if str(error) not in ("FIXTURE_BACKEND_ALIVE", "FIXTURE_COUNTER_UNKNOWN",
                                      "FIXTURE_COUNTER_UNAVAILABLE"):
                    raise
                last = str(error)
            expired = time.monotonic() >= until
            if expired:
                # Typed witness of the last round only: counters, one closed code, durations.
                print("PAPER_FIXTURE_SETTLE_UNPROVEN=" + canonical({
                    "phase": self.phase, "expected": expected, "actual": actual, "last": last,
                    "rounds": rounds,
                    "elapsed_ms": int((time.monotonic() - started) * 1000)}).decode(), flush=True)
                self._print_statistics_witness()
            require(not expired, "FIXTURE_STATS_UNPROVEN")
            time.sleep(0.02)

    def _print_statistics_witness(self):
        # Typed and best effort. Raw counters (a missing function statistic stays null), the
        # tracked call count of every public function, and the statement catalog rows that
        # mention the begin function. Names are source identifiers; no table values.
        try:
            raw = self._statistics()
            seen = self.json(
                "SELECT pg_stat_clear_snapshot(); SELECT json_build_object("
                "'functions',(SELECT COALESCE(json_object_agg(funcname,calls),'{}'::json) "
                "FROM pg_stat_user_functions WHERE schemaname='public'),"
                "'begin_statements',(SELECT COALESCE(json_agg(json_build_object("
                "'toplevel',toplevel,'calls',calls,"
                "'exact',regexp_replace(lower(query),'[[:space:]]','','g')='" + BEGIN_SELECT + "')),"
                "'[]'::json) FROM public.pg_stat_statements "
                "WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database()) "
                "AND lower(query) LIKE '%begin_owner_beta_paper_operation_v1%'));")
            functions, statements = seen.get("functions"), seen.get("begin_statements")
            print("PAPER_FIXTURE_STATISTICS_WITNESS=" + canonical({
                "raw": {key: raw.get(key) if type(raw.get(key)) is int else None
                        for key in ("sql", "function", "events", "receipts", "catalog", "prepared")},
                "functions": {name: count for name, count in functions.items()
                              if type(name) is str and re.fullmatch("[a-z0-9_]{1,63}", name)
                              and type(count) is int} if type(functions) is dict else None,
                "begin_statements": [{key: row.get(key) for key in ("toplevel", "calls", "exact")}
                                     for row in statements if type(row) is dict][:8]
                                    if type(statements) is list else None}).decode(), flush=True)
        except Exception:
            print("PAPER_FIXTURE_STATISTICS_WITNESS=UNAVAILABLE", flush=True)

    def barrier(self, phase, resource, result):
        # Typed timeline of the driver's phases; it calibrates the stop-attempt timing windows.
        self.phase = phase if type(phase) is str and re.fullmatch("[a-z-]{1,32}", phase) else "UNKNOWN"
        print("PAPER_FIXTURE_PHASE=" + canonical({
            "phase": self.phase,
            "ms": (time.monotonic_ns() - self.created_ns) // 1_000_000}).decode(), flush=True)
        context = (resource._context if type(resource) is ops._NativeInvocation else
                   resource._attempt.context if type(resource) is ops.DrainedHandle else resource)
        if context not in self.owner.contexts:
            self.owner.contexts.append(context)
        if phase == "first":
            if type(resource) is ops._NativeInvocation and resource.verb == "begin" and not hasattr(self, "primary_invocation"):
                self.primary_invocation = resource
            return  # Retain actual objects before probe/dispatch, no fake result.
        if phase == "stop-ready":
            if self.operation._intent["purpose"] == "BACKUP":
                self.owner.start_stop_writer()
            return
        now = self.settle({"sql": 1, "function": 1, "events": 1, "receipts": 1})
        if phase == "import":
            self.baseline = now
            self.original = {"generation": result["generation"], "event_id": result["event_id"],
                             "epoch": result["control_epoch"], "digest": result["artifact_sha256"]}
            self.original_hashes = {"root_intent_sha256": resource.custody._operation.intent_sha256,
                                    "action_sha256": ops._digest(resource.custody._binding_bytes),
                                    "dispatch_sha256": ops._digest(resource.custody._dispatch_bytes),
                                    "result_sha256": result["artifact_sha256"]}
        elif phase in ("duplicate", "readback"):
            require(counter_delta(self.baseline, now) ==
                    {"sql": 0, "function": 0, "events": 0, "receipts": 0},
                    "FIXTURE_DUPLICATE_OR_READBACK_WRITES")
        elif phase == "stopped":
            require(type(resource) is ops.DrainedHandle, "FIXTURE_UNKNOWN_IDENTITY")
            # No handle revalidation here: the driver's idempotent drain already made
            # the one full stop observation, and inventory1 must stay young.
            require(counter_delta(self.baseline, now) ==
                    {"sql": 0, "function": 0, "events": 0, "receipts": 0},
                    "FIXTURE_DUPLICATE_OR_READBACK_WRITES")
            negative = self.json("SELECT json_build_object('backends',(SELECT count(*) FROM "
                "pg_stat_activity WHERE datname=current_database() AND backend_type='client backend' "
                "AND pid<>pg_backend_pid()),'prepared',(SELECT count(*) FROM pg_prepared_xacts));")
            require(negative == {"backends": 0, "prepared": 0}, "FIXTURE_BACKEND_ALIVE")
            required = ops._WRITER_CLASSES if self.operation._intent["purpose"] == "BACKUP" else ops._PAPER_CLASSES
            require(required <= {w["writer_class"] for w in result["writers"]}
                    and result["generation"] == self.operation.generation
                    and self.operation.revalidate() == "STOPPED", "FIXTURE_UNKNOWN_IDENTITY")
            if self.owner.stop_writer is not None:
                cid, generation, child = self.owner.stop_writer
                value = self.owner.docker.object("container", cid)
                ops._terminal_container(value, generation)
                child.wait(timeout=self.owner.deadline.seconds(5))
                require(child.returncode is not None and any(w["writer_class"] == "OTHER_DB_WRITER"
                        and w["terminal"] == "EXITED_REAPED" for w in result["writers"]),
                        "FIXTURE_BACKEND_DISPOSAL_UNPROVEN")
                captured = resource._attempt.observer.backends
                require(any(row["process"]["owner"] == ["CONTAINER", cid] for row in captured),
                        "FIXTURE_BACKEND_CORRELATION")
            self.stop_receipt = {"stage_sha256": self.operation._last[1],
                "inventory_sha256": ops._digest(resource._attempt.inventory_bytes),
                "classes": sorted(required), "backend_negative": negative}
        elif phase in ("off", "off-replay"):
            require(type(resource) is ops.DrainedHandle, "FIXTURE_UNKNOWN_IDENTITY")
            resource.revalidate()
            until = time.monotonic() + self.owner.deadline.seconds(10)
            while True:
                try:
                    counts = self.maintenance_snapshot()
                    # reconcile:1 plus the refreshed reconcile:2 before the single OFF.
                    expected = {"reconcile_sql": 2, "reconcile_function": 2, "observations": 2,
                                "off_sql": 1, "off_function": 1, "off_events": 1}
                    require(all(counts[k] <= expected[k] for k in expected), "FIXTURE_EXCESS_CALLS")
                    if counts == expected:
                        break
                except FixtureError as error:
                    if error.args != ("FIXTURE_COUNTER_UNKNOWN",):
                        raise
                require(time.monotonic() < until, "FIXTURE_STATS_UNPROVEN")
                time.sleep(0.02)
            root = self.operation._intent
            ops.journal._uuid(result)
            value = self.json("SELECT json_build_object('event',e.id,'epoch',e.control_epoch,"
                "'payload',e.payload,'state',c.state,'current_epoch',c.control_epoch) "
                "FROM public.owner_beta_paper_events e JOIN public.owner_beta_paper_control c "
                "ON c.account_id=e.account_id WHERE e.id='" + result + "'::uuid AND e.kind='OFF' "
                "AND e.request_key='" + root["generation"] + ":off';")
            consumer = resource._attempt.consumer
            require(value == {"event": result, "epoch": resource._attempt.fence + 1,
                "payload": json.loads(consumer.actions[-1]._artifact_bytes), "state": "OFF",
                "current_epoch": resource._attempt.fence + 1}, "FIXTURE_ORIGINAL_SQL_RESULT")
            require(self.operation.revalidate() == "OFF_COMMITTED"
                    and self.operation._last[3]["event_id"] == result
                    and self.operation._journal._read_floor() == resource._attempt.fence + 1,
                    "FIXTURE_FLOOR")
            self.absent()
            if phase == "off":
                self.off_counts = counts
                self.stop_receipt["off"] = {"event_id": result, "stage_sha256": self.operation._last[1],
                    "epoch": resource._attempt.fence + 1, "counts": counts}
            else:
                require(counts == self.off_counts, "FIXTURE_DUPLICATE_OR_READBACK_WRITES")
            resource.revalidate()
        else:
            raise FixtureError("FIXTURE_OBSERVER_PHASE")


class RowLock:
    """One owned real server transaction, with retained backend PID/start."""
    def __init__(self, observer):
        self.observer, self.child = observer, None
        self.acquired = None

    def __enter__(self):
        owner = self.observer.owner
        self.observer.target()
        self.child = subprocess.Popen(owner.argv(
            ["exec", "--interactive", "--user", "999:999", owner.pg,
             "psql", "-X", "-qAt", "-U", "lagrange", "-d", "lagrange",
             "-v", "ON_ERROR_STOP=1"]), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, env=ops._docker_client_environment())
        owner.blockers.append(self)
        try:
            self.child.stdin.write(b"BEGIN; SELECT json_build_object('pid',pg_backend_pid(),"
                                   b"'start',backend_start::text) FROM pg_stat_activity "
                                   b"WHERE pid=pg_backend_pid();\n"
                                   b"SELECT 1 FROM public.owner_beta_paper_control WHERE id='etf11' FOR UPDATE;\n"
                                   b"\\echo FIXTURE_LOCKED\n")
            self.child.stdin.flush()
            selector = selectors.DefaultSelector()
            selector.register(self.child.stdout, selectors.EVENT_READ)
            output = b""
            try:
                until = time.monotonic() + owner.deadline.seconds(10)
                while b"FIXTURE_LOCKED\n" not in output:
                    require(self.child.poll() is None, "FIXTURE_LOCK_CHILD_EXIT")
                    require(time.monotonic() < until, "FIXTURE_LOCK_TIMEOUT")
                    if not selector.select(min(0.1, max(0, until-time.monotonic()))):
                        continue
                    output += os.read(self.child.stdout.fileno(), 4096)
                    require(len(output) < 8192, "FIXTURE_LOCK_OUTPUT")
            finally:
                selector.close()
            record = json.loads(output.splitlines()[0])
            require(type(record) is dict and set(record) == {"pid", "start"}
                    and type(record["pid"]) is int and type(record["start"]) is str,
                    "FIXTURE_LOCK_BACKEND")
            self.observer.backends.add((record["pid"], record["start"]))
            self.acquired = time.monotonic()
            return self
        except BaseException:
            self.release()
            raise

    def release(self):
        if self.child is None:
            return
        child, self.child = self.child, None
        try:
            if child.poll() is None:
                try:
                    child.stdin.write(b"ROLLBACK;\n\\q\n")
                    child.stdin.flush()
                except (BrokenPipeError, OSError):
                    pass
                child.stdin.close()
                try:
                    child.wait(timeout=self.observer.owner.deadline.seconds(5, cleanup=True))
                except subprocess.TimeoutExpired:
                    ops._NativeInvocation._reap_child(child)
                    raise FixtureError("FIXTURE_LOCK_RELEASE_UNPROVEN") from None
            require(child.returncode == 0, "FIXTURE_LOCK_RELEASE")
        finally:
            if child.poll() is None:
                ops._NativeInvocation._reap_child(child)
            for stream in (child.stdin, child.stdout):
                if stream is not None and not stream.closed:
                    stream.close()

    def __exit__(self, *_exc):
        self.release()


def cancel_owned_attach(child, deadline):
    require(child is not None and child.poll() is None, "FIXTURE_ATTACH_CHILD")
    # SIGTERM on Docker's attached CLI can be forwarded to the real helper.
    # Kill/reap only the owned CLI; inspect the helper/backend independently.
    child.kill()
    child.wait(timeout=deadline.seconds(5, cleanup=True))


class FixtureDocker(ops.runtime.Docker):
    def __init__(self, deadline):
        super().__init__()
        self.deadline = deadline
        self.environment = ops._docker_client_environment()

    def call(self, args, timeout=15):
        return super().call(args, self.deadline.seconds(timeout, cleanup=True))


class P3ExtractionCustody:
    """Observe only this fixture's real P3 commands, without changing their ABI.

    The P3 module reference is scoped; shared stdlib subprocess is never patched.
    Losing create's returned CID or its observed generation stays UNKNOWN. Only
    actual captured creation and terminal absence can discharge a record.
    """
    def __init__(self, owner):
        self.owner, self.original = owner, None
        self.records, self.lock = [], threading.RLock()
        self.images = set()
        manifest = getattr(owner, "manifest", None)
        if type(manifest) is bytes:
            self.images = {line.split(b"|")[3].decode("ascii") for line in manifest.splitlines()[2:]}

    def __enter__(self):
        require(self.original is None and not isinstance(ops.release.subprocess, P3ExtractionCustody),
                "FIXTURE_P3_OBSERVER_ALREADY_OWNED")
        self.original = ops.release.subprocess
        ops.release.subprocess = self
        return self

    def __exit__(self, *_exc):
        require(ops.release.subprocess is self, "FIXTURE_P3_OBSERVER_CHANGED")
        ops.release.subprocess = self.original
        self.original = None

    def __getattr__(self, name):
        return getattr(self.original, name)

    def _unknown(self):
        self.owner.cleanup["extractions"] = "UNKNOWN"
        self.owner.cleanup["containers"] = "UNKNOWN"

    def _capture(self, record, result):
        # Save actual returned identity before any further daemon observation.
        reference = result.stdout.decode("ascii", "strict").strip()
        require(re.fullmatch("[0-9a-f]{12,64}", reference), "FIXTURE_P3_CREATE_UNKNOWN")
        record["reference"] = reference
        require(record["image"] in self.images and re.fullmatch("sha256:[0-9a-f]{64}", record["image"]),
                "FIXTURE_P3_IMAGE_UNKNOWN")
        value = self.owner.docker.object("container", reference)
        cid, created = value.get("Id"), value.get("Created")
        state = value.get("State") or {}
        require(type(cid) is str and re.fullmatch("[0-9a-f]{64}", cid) and cid.startswith(reference)
                and type(created) is str and created
                and value.get("Image") == record["image"]
                and (value.get("Config") or {}).get("Image") == record["image"]
                and state.get("Status") == "created" and state.get("Running") is False
                and state.get("Pid") == 0 and type(value.get("RestartCount")) is int
                and value["RestartCount"] == 0, "FIXTURE_P3_GENERATION_UNKNOWN")
        record["cid"], record["created"] = cid, created
        record["state"] = "CAPTURED"

    def _absent(self, record):
        require(record["cid"] is not None and record["created"] is not None,
                "FIXTURE_P3_CREATE_UNKNOWN")
        found = self.owner.output(["ps", "--all", "--no-trunc", "--filter", "id=" + record["cid"],
                                   "--format", "{{.ID}}"], cleanup=True)
        require(found in ("", record["cid"]), "FIXTURE_P3_IDENTITY_UNKNOWN")
        if found:
            value = self.owner.docker.object("container", record["cid"])
            require((value.get("Id"), value.get("Created"), value.get("Image")) ==
                    (record["cid"], record["created"], record["image"]), "FIXTURE_P3_IDENTITY_UNKNOWN")
            return False
        record["state"] = "ABSENT"
        return True

    def run(self, *args, **kwargs):
        require(self.original is not None and ops.release.subprocess is self, "FIXTURE_P3_OBSERVER_CHANGED")
        argv = args[0] if args else kwargs.get("args")
        create = (type(argv) in (list, tuple) and len(argv) >= 2 and argv[:2] in
                  (["docker", "create"], ("docker", "create")))
        remove = (type(argv) in (list, tuple) and len(argv) == 4 and list(argv[:3]) == ["docker", "rm", "-f"])
        record = None
        with self.lock:
            if create:
                record = {"image": argv[2] if len(argv) == 3 else None, "reference": None,
                          "cid": None, "created": None, "state": "CREATE_PENDING"}
                self.records.append(record)
                self._unknown()
            elif remove:
                matches = [item for item in self.records if item["reference"] == argv[3]]
                if len(matches) == 1:
                    record = matches[0]
                    record["state"] = "REMOVE_PENDING"
                    self._unknown()
        try:
            result = self.original.run(*args, **kwargs)
        except BaseException:
            if record is not None:
                with self.lock:
                    record["state"] = "CREATE_UNKNOWN" if create else "REMOVE_UNKNOWN"
                    self._unknown()
            raise
        # Bookkeeping failures must not fabricate or replace P3 command output.
        # The exact real result is returned; disposal subsequently blocks on UNKNOWN.
        try:
            with self.lock:
                if create:
                    self._capture(record, result)
                elif remove and record is not None:
                    record["state"] = "REMOVE_UNKNOWN"
                    if result.returncode == 0:
                        self._absent(record)
                self._status()
        except BaseException as error:
            with self.lock:
                if record is not None:
                    record["state"] = "CREATE_UNKNOWN" if create else "REMOVE_UNKNOWN"
                self._unknown()
            if not isinstance(error, Exception) or str(error) in ("FIXTURE_DEADLINE", "FIXTURE_INTERRUPTED"):
                raise
        return result

    def _status(self):
        if all(item["state"] == "ABSENT" for item in self.records):
            self.owner.cleanup["extractions"] = "ABSENT" if self.records else "NOT_STARTED"
        else:
            self._unknown()

    def reconcile(self):
        """Read-only proof for captured CIDs; never guess a lost create by image/name."""
        require(self.original is not None and ops.release.subprocess is self, "FIXTURE_P3_OBSERVER_CHANGED")
        with self.lock:
            try:
                for record in self.records:
                    if record["state"] != "ABSENT":
                        require(self._absent(record), "FIXTURE_P3_CLEANUP_UNKNOWN")
                self._status()
            except BaseException:
                self._unknown()
                raise


class FixtureOwner:
    def __init__(self, source, commit, manifest, deadline):
        self.source, self.commit, self.manifest, self.deadline = source, commit, manifest, deadline
        self.root = pathlib.Path(tempfile.mkdtemp(prefix="lagrange-paper-operations-fixture-", dir="/var/lib"))
        self.root.chmod(0o700)
        self.root_id = faults.directory_identity(self.root)
        self.project = "paper-operations-fixture-" + self.root.name.rsplit("-", 1)[1]
        self.docker = FixtureDocker(deadline)
        self.pg_image = None
        self.network = self.pg = self.context = self.lock_fd = None
        self.network_sent = self.pg_sent = False
        self.source_api_nonce = self.source_api_image = self.source_api_commit = self.source_api_cid = None
        self.source_api_generation = None
        self.source_api_sent = self.source_api_disposed = False
        self.observer = None
        self.contexts, self.tracees, self.threads, self.blockers = [], [], [], []
        self.context_modules = {}
        self.helpers, self.helper_bindings = {}, {}
        self.stop_writer = None
        self.stop_writer_sent = False
        self.stop_writer_backend = None
        self.cancel_event = threading.Event()
        self.original_custody_uncertain = False
        self.cleanup = {"containers": "PENDING", "backends": "PENDING", "inputs": "RETAINED",
                        "extractions": "NOT_STARTED", "source_api": "NOT_STARTED"}
        self.extractions = P3ExtractionCustody(self)

    @staticmethod
    def argv(args):
        return ["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", *args]

    def start_stop_writer(self):
        """Real selected-image psql, real app authority and an owned TCP backend.

        This is an OTHER_DB_WRITER in combined scope, never an API/Paper ELF
        surrogate. The STOP case independently proves those classes absent.

        It runs under the Docker init process, as every installed writer service does. The
        controller requests termination with one TERM to the container and never escalates.
        A process that is PID 1 of its container and has no TERM handler never receives that
        signal, so psql started as PID 1 would outlive the whole stop attempt.
        """
        require(self.stop_writer is None and not self.stop_writer_sent, "FIXTURE_UNKNOWN_IDENTITY")
        password = uuid.uuid4().hex + uuid.uuid4().hex
        self.observer.sql("ALTER ROLE app PASSWORD '" + password + "';")
        secret = self.root / "stop-client.pgpass"
        write_new(secret, ("postgres:5432:lagrange:app:" + password + "\n").encode(),
                  mode=0o600, uid=10001, gid=10001)
        password = None
        self.stop_writer_sent = True
        cid = self.output(["create", "--init", "--interactive", "--name", self.project + "-stop-writer",
            "--label", "com.docker.compose.project=" + self.project,
            "--label", "com.docker.compose.service=paper-fixture-db-writer",
            "--network", self.network, "--restart", "no", "--read-only", "--user", "10001:10001",
            "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--cpus", "1",
            "--memory", "2147483648", "--env", "PGPASSFILE=/run/stop-client.pgpass",
            "--mount", "type=bind,source=" + str(secret) + ",target=/run/stop-client.pgpass,readonly",
            "--entrypoint", "/usr/bin/psql", self.context.images["paper-scheduler"],
            "-X", "-qAt", "-h", "postgres", "-U", "app", "-d", "lagrange",
            "-v", "ON_ERROR_STOP=1"])
        require(re.fullmatch("[0-9a-f]{64}", cid), "FIXTURE_UNKNOWN_IDENTITY")
        # Record before start so a start/attach error never loses resource custody.
        self.stop_writer = (cid, None, None)
        child = subprocess.Popen(self.argv(["start", "--attach", "--interactive", cid]),
            stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            env=ops._docker_client_environment())
        self.stop_writer = (cid, None, child)
        child.stdin.write(b"SELECT pg_sleep(300);\n")
        child.stdin.flush()
        until = time.monotonic() + self.deadline.seconds(15)
        while time.monotonic() < until:
            value = self.docker.object("container", cid)
            if value.get("State", {}).get("Running"):
                generation = ops._container_generation(value)
                self.stop_writer = (cid, generation, child)
                endpoints = value.get("NetworkSettings", {}).get("Networks") or {}
                ips = {v.get("IPAddress") for v in endpoints.values()}
                rows = self.observer.json("SELECT COALESCE(json_agg(json_build_object('pid',pid,"
                    "'start',backend_start::text,'addr',host(client_addr))), '[]'::json) "
                    "FROM pg_stat_activity WHERE datname=current_database() AND usename='app';")
                matching = [row for row in rows if row["addr"] in ips]
                if len(matching) == 1:
                    self.stop_writer_backend = (matching[0]["pid"], matching[0]["start"])
                    self.observer.backends.add(self.stop_writer_backend)
                    return
            require(child.poll() is None, "FIXTURE_ATTACH_CHILD")
            time.sleep(0.05)
        raise FixtureError("FIXTURE_BACKEND_CORRELATION")

    def dispose_stop_writer(self):
        require(not self.stop_writer_sent or self.stop_writer is not None, "FIXTURE_UNKNOWN_IDENTITY")
        if self.stop_writer is None:
            return
        cid, generation, child = self.stop_writer
        value = self.docker.object("container", cid)
        require(value.get("Image") == self.context.images["paper-scheduler"]
                and value.get("Config", {}).get("Labels", {}).get("com.docker.compose.project") == self.project
                and value.get("Config", {}).get("Labels", {}).get("com.docker.compose.service") ==
                "paper-fixture-db-writer", "FIXTURE_FOREIGN_CLEANUP")
        require(generation is not None and ops._container_generation(value) == generation,
                "FIXTURE_UNKNOWN_IDENTITY")
        state = value["State"]
        running = state["Running"] is True
        # Typed witness of what the controller's stop attempt left behind: whether the writer was
        # still running, its exit status, and whether it ran under the Docker init process.
        print("PAPER_FIXTURE_STOP_WRITER=" + canonical({
            "running": running,
            "exit": state.get("ExitCode") if type(state.get("ExitCode")) is int else None,
            "init": (value.get("HostConfig") or {}).get("Init") is True}).decode(), flush=True)
        if running:
            self.command(["stop", "--time", "5", cid], cleanup=True)
        ops._terminal_container(self.docker.object("container", cid), generation)
        if child is not None:
            child.wait(timeout=self.deadline.seconds(5, cleanup=True))
            if child.stdin is not None:
                child.stdin.close()
        if running:
            # The controller never reached its backend drain, and the session stays inside its
            # sleep after the client is gone. When the controller did stop the writer, the
            # absence check below remains the proof that it also drained the backend.
            self._terminate_stop_writer_backend()
        self.observer.absent(cleanup=True)
        self.command(["rm", cid], cleanup=True)
        require(self.output(["ps", "-aq", "--no-trunc", "--filter", "id=" + cid], cleanup=True) == "",
                "FIXTURE_HELPER_CLEANUP")
        self.stop_writer = None
        self.stop_writer_sent = False
        self.stop_writer_backend = None

    def _terminate_stop_writer_backend(self):
        backend = self.stop_writer_backend
        if backend is None:
            return
        pid, start = backend
        require(type(pid) is int and type(start) is str and re.fullmatch(r"[0-9 :.+-]+", start),
                "FIXTURE_BACKEND_IDENTITY")
        until = time.monotonic() + self.deadline.seconds(10, cleanup=True)
        while True:
            # Only the session this fixture started for its own writer: exact pid, start and role.
            alive = self.observer.sql(
                "SELECT count(pg_terminate_backend(pid)) FROM pg_stat_activity WHERE pid=%d "
                "AND backend_start='%s'::timestamptz AND usename='app';" % (pid, start), cleanup=True)
            if alive == b"0":
                return
            require(time.monotonic() < until, "FIXTURE_BACKEND_ALIVE")
            time.sleep(0.05)

    def command(self, args, payload=None, cap=30, cleanup=False, allow_failure=False):
        result = subprocess.run(self.argv(args), input=payload, capture_output=True,
                                env=ops._docker_client_environment(), check=False,
                                timeout=self.deadline.seconds(cap, cleanup=cleanup))
        require(allow_failure or result.returncode == 0, "FIXTURE_DOCKER_COMMAND")
        return result

    def output(self, args, **kwargs):
        return self.command(args, **kwargs).stdout.decode("ascii").strip()

    def _extract_archive(self, archive, target):
        """Owned git-archive bytes only: regular files and directories, no links or escapes."""
        with tarfile.open(archive) as stream:
            for member in stream:
                require(member.isfile() or member.isdir(), "FIXTURE_ARCHIVE_LINK")
                path = pathlib.PurePosixPath(member.name)
                require(not path.is_absolute() and ".." not in path.parts, "FIXTURE_ARCHIVE_PATH")
                stream.extract(member, target, filter="data")
        for parent, directories, names in os.walk(target):
            os.chmod(parent, 0o755)
            for name in names:
                p = pathlib.Path(parent) / name
                os.chmod(p, stat.S_IMODE(p.stat().st_mode) & ~0o022)

    def _release_env(self, commit):
        """The protected Compose env shape every fixture release uses; only the commit differs."""
        values = {"LAGRANGE_CODE_COMMIT": commit, "LAGRANGE_DATA_DIR": str(self.root / "data"),
                  "LAGRANGE_ARTIFACTS_DIR": str(self.root / "artifacts"),
                  "LAGRANGE_RUNTIME_SECRET_DIR": str(self.root / "secrets"), "POSTGRES_DB": "lagrange",
                  "OWNER_BETA_DAILY_MODE": "disabled", "OWNER_BETA_PAPER_MODE": "disabled"}
        return "".join(k + "=" + v + "\n" for k, v in values.items()).encode()

    def setup(self, policy_root, *, _prepare_release=None, migration_stop=70):
        # migration_stop=66 leaves the owned database at 0065 (pre-Paper). Such a database has no
        # INSPECT ABI, so the initial native history probe below is not run for it.
        require(migration_stop in (66, 70) and (migration_stop == 70 or _prepare_release is None),
                "FIXTURE_MIGRATION_INVENTORY")
        self.pg_image = ops._database_image_id(self.docker)
        self.lock_fd = os.open(self.root / ops.LOCK_NAME, os.O_CREAT | os.O_EXCL | os.O_RDWR |
                               os.O_CLOEXEC | os.O_NOFOLLOW, 0o600)
        fcntl.flock(self.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        releases = self.root / "releases"
        releases.mkdir(mode=0o755)
        staging = releases / (".staging." + self.commit + "." + uuid.uuid4().hex)
        staging.mkdir(mode=0o755)
        archive = self.root / "source.tar"
        with archive.open("xb") as output:
            child = subprocess.Popen(["/usr/bin/git", "--no-replace-objects", "-c", "core.fsmonitor=false",
                                      "-c", "safe.directory=" + str(self.source),
                                      "-C", str(self.source), "archive", "--format=tar", self.commit],
                                     stdout=output, stderr=subprocess.DEVNULL, env=GIT_ENV)
            try:
                child.wait(timeout=self.deadline.seconds(60))
            except BaseException:
                child.kill()
                child.wait(timeout=5)
                raise
            require(child.returncode == 0, "FIXTURE_ARCHIVE")
        self._extract_archive(archive, staging)
        secret_root = self.root / "secrets"
        secret_root.mkdir(mode=0o700)
        service = secret_root / "db-migrate"
        service.mkdir(mode=0o750)
        os.chown(service, 999, 999)
        password = uuid.uuid4().hex + uuid.uuid4().hex
        write_new(service / "db_migration_owner_password", password.encode(), 0o400, 999, 999)
        write_new(self.root / "postgres-secret", (uuid.uuid4().hex + uuid.uuid4().hex).encode(), 0o400, 999, 999)
        write_new(staging / "deploy/compose/.env", self._release_env(self.commit))
        write_new(staging / ops.release.MANIFEST_NAME, self.manifest)
        require(ops.daily.policy.validate_manifest(str(staging / ops.release.MANIFEST_NAME), self.commit)
                == hashlib.sha256(self.manifest).hexdigest(), "FIXTURE_INSTALLED_MANIFEST")
        write_new(staging / ".lagrange-release", (self.commit + "\n").encode())
        manifest_sha = ops._digest(self.manifest)
        if _prepare_release is not None:
            # A private implementation fixture may prepare Daily assets while
            # this exact owned tree is still unpublished. No Context exists and
            # the callback cannot substitute a selected installed release.
            self._setup_database(staging, password)
            password = None
            database_owner = (self.observer, self.pg, self.network, self.observer.generation)
            with ops.daily.install.root_directory(str(staging)) as stage_fd:
                original = faults.directory_identity(staging)
                require(self.context is None and not hasattr(self, "release"), "FIXTURE_CURRENT_UNEXPECTED")
                _prepare_release(self, staging)
                require(faults.directory_identity(staging) == original
                        and (os.fstat(stage_fd).st_dev, os.fstat(stage_fd).st_ino) == original[:2]
                        and self.context is None and not hasattr(self, "release")
                        and (self.observer, self.pg, self.network, self.observer.generation) == database_owner,
                        "FIXTURE_VERIFY_CHANGED_SOURCE")
                self.observer.target()
                os.fsync(stage_fd)
        ops.release.stage(str(self.root), str(staging), self.commit, manifest_sha, None)
        self.release = releases / self.commit
        staging.rename(self.release)
        fd = os.open(releases, os.O_RDONLY | os.O_DIRECTORY)
        os.fsync(fd)
        os.close(fd)
        checked = ops.release.verify(str(self.root), str(self.release), self.commit)
        try:
            paper = self.release / "deploy/owner-beta-paper"
            capability = ops._json((paper / "capability.json").read_bytes(), ops.MAX_RESULT)
            self.release_hashes = {"capability_sha256": checked.capability_sha256,
                                   "install_receipt_sha256": checked.install_receipt_sha256,
                                   "producer_receipts_sha256": checked.producer_receipts_sha256,
                                   **{key: capability[key] for key in
                                      ("api_binary_sha256", "worker_binary_sha256", "native_binary_sha256")}}
            self.producer_hashes = {name: {leaf: ops._digest((paper / "provenance" / name / leaf).read_bytes())
                                          for leaf in ("artifact.json", "COMPLETE")}
                                    for name in ("api-server", "paper-runner", "owner-beta-paper-ops-native")}
            checked.revalidate()
        finally:
            checked.close()
        policy = {"schema_version": "service-status.policy.v1", "profile": "owner_daily",
                  "release_commit": self.commit, "compose_project": self.project,
                  "owner_access": "owner_only", "quote_transport": "market_ws",
                  "window_source": "operational_v1", "research_mode": "daily",
                  "policy_installed_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                  "job_lease_seconds": {k: 60 for k in ("owner_equity_v2", "owner_beta_price_recommendation",
                                                       "candidate_compute", "recommendation", "backtest")}}
        ops.daily.policy.validate_policy(policy)
        path = policy_root / "service-status/policy.json"
        if path.exists():
            path.unlink()  # Owned namespace file only; no existing host policy.
        write_new(path, canonical(policy))
        if self.observer is None:
            self._setup_database(self.release, password, migration_stop=migration_stop)
        password = None
        images = ops.daily.read_manifest(str(self.release), self.commit)
        actual_values, overlays = ops.daily.read_context(str(self.release), self.commit, "disable")
        self.context = ops.daily.Context(str(self.release), self.commit, images, actual_values, overlays)
        if migration_stop != 70:
            return
        with ops.open_context(self.context, self.lock_fd) as live:
            self.contexts.append(live)
            with live.probe_history() as inventory:
                require(inventory._configured is False and inventory._history is False,
                        "FIXTURE_INITIAL_NOT_EMPTY")

    def _setup_database(self, migration_root, password, *, migration_stop=70):
        """Actual private DB owner; called once, never a prepared-state receipt.

        The release must carry exactly migrations 0001..0069. Versions below migration_stop are
        applied: 70 is the full schema, 66 is the pre-Paper 0065 schema.
        """
        require(migration_stop in (66, 70), "FIXTURE_MIGRATION_INVENTORY")
        require(self.observer is None and self.network is None and self.pg is None
                and not self.network_sent and not self.pg_sent, "FIXTURE_INITIAL_NOT_EMPTY")
        self.network_sent = True
        self.network = self.output(["network", "create", "--internal", "--label",
                                    "com.docker.compose.project=" + self.project, "--label",
                                    "com.docker.compose.network=owner-beta-db",
                                    self.project + "_owner-beta-db"])
        require(re.fullmatch("[0-9a-f]{64}", self.network), "FIXTURE_NETWORK_ID")
        data = self.root / "pgdata"
        data.mkdir(mode=0o700)
        os.chown(data, 999, 999)
        self.pg_sent = True
        self.pg = self.output(["run", "--detach", "--name", self.project + "-postgres",
                               "--network", self.network, "--network-alias", "postgres",
                               "--label", "com.docker.compose.project=" + self.project,
                               "--label", "com.docker.compose.service=postgres", "--restart", "no",
                               "--env", "POSTGRES_USER=lagrange", "--env", "POSTGRES_DB=lagrange",
                               "--env", "POSTGRES_PASSWORD_FILE=/run/test-secret",
                               "--mount", "type=bind,source=" + str(self.root / "postgres-secret") +
                               ",target=/run/test-secret,readonly",
                               "--mount", "type=bind,source=" + str(data) + ",target=/var/lib/postgresql",
                               ops.POSTGRES_IMAGE, "postgres", "-c", "shared_preload_libraries=pg_stat_statements",
                               "-c", "pg_stat_statements.track=all", "-c", "track_functions=pl"])
        require(re.fullmatch("[0-9a-f]{64}", self.pg), "FIXTURE_PG_ID")
        self.observer = PgObserver(self)
        until = time.monotonic() + self.deadline.seconds(90)
        while True:
            try:
                require(self.observer.sql("SELECT current_user||':'||session_user;") == b"lagrange:lagrange"
                        and self.observer.sql("SHOW listen_addresses;") == b"*", "FIXTURE_PG_LOGIN")
                break
            except FixtureError:
                require(time.monotonic() < until, "FIXTURE_PG_START_TIMEOUT")
                time.sleep(0.1)
        pg = self.observer
        pg.sql("".join("CREATE ROLE " + role + " LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE "
                       "NOREPLICATION NOBYPASSRLS;\n" for role in ROLES) +
               "GRANT CONNECT ON DATABASE lagrange TO " + ",".join(ROLES) + ";"
               "GRANT USAGE,CREATE ON SCHEMA public TO migration_owner;" +
               "GRANT USAGE ON SCHEMA public TO " + ",".join(ROLES[1:]) + ";" +
               "ALTER ROLE migration_owner PASSWORD '" + password + "';")
        password = None
        require(pg.sql("SELECT current_user||':'||session_user;", "migration_owner") ==
                b"migration_owner:migration_owner", "FIXTURE_MIGRATION_LOGIN")
        pg.sql("CREATE TABLE public._sqlx_migrations(version bigint PRIMARY KEY,description text NOT NULL,"
               "installed_on timestamptz NOT NULL DEFAULT now(),success boolean NOT NULL,"
               "checksum bytea NOT NULL,execution_time bigint NOT NULL);", "migration_owner")
        self._apply_migrations(migration_root, range(1, migration_stop))
        if migration_stop == 70:
            # Statement statistics are created after the last migration, as for every lifecycle
            # case. The pre-Paper case reads none and later applies 0066..0069 itself.
            pg.sql("CREATE EXTENSION pg_stat_statements;")

    def start_pre_paper_source_api(self, source_commit, image, target_image, derived):
        """Start the selected, test-derived D1 API against this fixture's private PG."""
        require(self.source_api_nonce is None and not self.source_api_sent and self.network is not None
                and self.observer is not None, "FIXTURE_PRE_PAPER_SOURCE_API")
        simulation_ops = load("paper_genuine_fixture_simulation_operations",
                              self.source / "scripts/ops/lib/owner-beta-simulation-operations.py")
        require(image is not None and derived.get(target_image) == image,
                "FIXTURE_PRE_PAPER_SOURCE_API")
        run_simulation_operation(simulation_ops, "FIXTURE_PRE_PAPER_SOURCE_API",
            simulation_ops._api_image_capability, self.docker, source_commit, image, self.deadline.seconds(15))
        network = self.docker.object("network", self.network, self.deadline.seconds(15))
        labels = network.get("Labels") if type(network) is dict else None
        network_name = network.get("Name") if type(network) is dict else None
        require(type(network) is dict and network.get("Id") == self.network and network.get("Internal") is True
                and type(labels) is dict and labels.get("com.docker.compose.project") == self.project
                and labels.get("com.docker.compose.network") == "owner-beta-db" and
                type(network_name) is str and run_simulation_operation(simulation_ops,
                    "FIXTURE_PRE_PAPER_SOURCE_API", simulation_ops._api_ids, self.docker,
                    self.deadline.seconds(15)) == [],
                "FIXTURE_PRE_PAPER_SOURCE_API")

        secrets = self.root / "source-api-secrets"
        secrets.mkdir(mode=0o700)
        passwords = pre_paper_api_role_passwords(self.observer)
        secret_paths = {}
        for role, filename in (("app", "db_app_password"), ("admin", "db_admin_password"),
                               ("audit_writer", "db_audit_password")):
            secret_paths[filename] = secrets / filename
            write_new(secret_paths[filename], passwords[role].encode("ascii"), 0o400, 10001, 10001)
        passwords = None
        for filename in ("cursor_secret", "auth0_client_secret"):
            secret_paths[filename] = secrets / filename
            write_new(secret_paths[filename], uuid.uuid4().hex.encode("ascii"), 0o400, 10001, 10001)
        artifacts = self.root / "source-api-artifacts"
        artifacts.mkdir(mode=0o750)
        os.chown(artifacts, 10001, 10001)
        artifacts.chmod(0o750)
        environment = {
            "LAGRANGE_CODE_COMMIT": source_commit, "APP_ENV": "production",
            "APP_HOST": "0.0.0.0", "APP_PORT": "8080",
            "DB_HOST": "postgres", "DB_PORT": "5432", "DB_NAME": "lagrange", "DB_USER": "app",
            "DB_PASSWORD_FILE": "/run/secrets/db_app_password",
            "ADMIN_DB_HOST": "postgres", "ADMIN_DB_PORT": "5432", "ADMIN_DB_NAME": "lagrange",
            "ADMIN_DB_USER": "admin", "ADMIN_DB_PASSWORD_FILE": "/run/secrets/db_admin_password",
            "AUDIT_DB_HOST": "postgres", "AUDIT_DB_PORT": "5432", "AUDIT_DB_NAME": "lagrange",
            "AUDIT_DB_USER": "audit_writer", "AUDIT_DB_PASSWORD_FILE": "/run/secrets/db_audit_password",
            "DB_APP_MAX_CONNECTIONS": "2", "DB_ADMIN_MAX_CONNECTIONS": "1",
            "DB_AUDIT_MAX_CONNECTIONS": "1", "CURSOR_SECRET_FILE": "/run/secrets/cursor_secret",
            "ARTIFACT_ROOT": "/data/artifacts", "SESSION_COOKIE_SECURE": "true",
            "AUTH0_DOMAIN": "paper-genuine-fixture.invalid", "AUTH0_CLIENT_ID": "synthetic-fixture",
            "AUTH0_REDIRECT_URI": "https://paper-genuine-fixture.invalid/auth/callback",
            "AUTH0_CLIENT_SECRET_FILE": "/run/secrets/auth0_client_secret",
            "RECOMMENDATION_DATASET_VERSION_ID": str(uuid.uuid5(uuid.NAMESPACE_URL,
                "lagrange-pre-paper-source-api:" + source_commit)),
            "RECOMMENDATION_DATASET_ID": "krx_eod_bars", "RECOMMENDATION_DATASET_VERSION": "fixture-unused",
            "RECOMMENDATION_CURATED_VERSION": "1", "RECOMMENDATION_DATASET_MANIFEST_SHA256": "f" * 64,
            "OWNER_BETA_ACCESS_MODE": "disabled", "OWNER_BETA_PRICE_INPUT_MODE": "disabled",
            "OWNER_BETA_PAPER_MODE": "disabled", "OWNER_BETA_DAILY_MODE": "disabled",
        }
        nonce = uuid.uuid4().hex
        self.source_api_nonce, self.source_api_image, self.source_api_commit = nonce, image, source_commit
        self.source_api_sent = True
        name = self.project + "-pre-paper-api-" + nonce[:12]
        mounts = []
        for filename, target in (("db_app_password", "/run/secrets/db_app_password"),
                                 ("db_admin_password", "/run/secrets/db_admin_password"),
                                 ("db_audit_password", "/run/secrets/db_audit_password"),
                                 ("cursor_secret", "/run/secrets/cursor_secret"),
                                 ("auth0_client_secret", "/run/secrets/auth0_client_secret")):
            mounts += ["--mount", "type=bind,source=" + str(secret_paths[filename]) + ",target=" + target + ",readonly"]
        mounts += ["--mount", "type=bind,source=" + str(artifacts) + ",target=/data/artifacts,readonly"]
        args = ["create", "--name", name,
            "--label", "com.docker.compose.project=" + self.project,
            "--label", "com.docker.compose.service=api-server",
            "--label", "com.docker.compose.oneoff=False",
            "--label", "com.docker.compose.container-number=1",
            "--label", REVISION_LABEL + "=" + source_commit,
            "--label", "io.lagrange.paper-fixture.source-api=" + nonce,
            "--network", self.network, "--network-alias", "api-server", "--restart", "unless-stopped",
            "--init", "--read-only", "--user", "10001:10001", "--cap-drop", "ALL",
            "--security-opt", "no-new-privileges", "--tmpfs", "/tmp:rw,nosuid,nodev,noexec,size=64m,mode=1777",
            "--log-driver", "none"]
        for key, value in environment.items():
            args += ["--env", key + "=" + value]
        cid = self.output([*args, *mounts, "--entrypoint", "/usr/local/bin/api-server", image])
        require(re.fullmatch(r"[0-9a-f]{64}", cid) is not None, "FIXTURE_PRE_PAPER_SOURCE_API")
        self.source_api_cid = cid
        self.command(["start", cid])
        require(run_simulation_operation(simulation_ops, "FIXTURE_PRE_PAPER_SOURCE_API",
                simulation_ops._api_ids, self.docker, self.deadline.seconds(15)) == [cid],
                "FIXTURE_PRE_PAPER_SOURCE_API")
        value = self.docker.object("container", cid, self.deadline.seconds(15))
        require(type(value) is dict, "FIXTURE_PRE_PAPER_SOURCE_API")
        config, host = value.get("Config"), value.get("HostConfig")
        container_labels = config.get("Labels") if type(config) is dict else None
        require(value.get("Id") == cid and value.get("Image") == image and type(container_labels) is dict and
                all(container_labels.get(key) == expected for key, expected in {
                    "com.docker.compose.project": self.project, "com.docker.compose.service": "api-server",
                    "com.docker.compose.oneoff": "False", REVISION_LABEL: source_commit,
                    "io.lagrange.paper-fixture.source-api": nonce}.items()) and type(host) is dict and
                host.get("PortBindings") in (None, {}) and host.get("PublishAllPorts") is False,
                "FIXTURE_PRE_PAPER_SOURCE_API")
        network_settings = value.get("NetworkSettings")
        networks = network_settings.get("Networks") if type(network_settings) is dict else None
        require(type(networks) is dict and set(networks) == {network_name} and
                type(networks[network_name]) is dict and
                networks[network_name].get("NetworkID") == self.network,
                "FIXTURE_PRE_PAPER_SOURCE_API")
        generation, running, _policy = run_simulation_operation(simulation_ops,
            "FIXTURE_PRE_PAPER_SOURCE_API", simulation_ops._api_observe,
            self.docker, self.project, source_commit, image, cid, self.deadline.seconds(15))
        env = simulation_ops.daily.runtime.environment(value)
        require(running and env.get("LAGRANGE_CODE_COMMIT") == source_commit and
                env.get("OWNER_BETA_ACCESS_MODE") == "disabled" and
                env.get("OWNER_BETA_PRICE_INPUT_MODE") == "disabled" and
                env.get("OWNER_BETA_PAPER_MODE") == "disabled", "FIXTURE_PRE_PAPER_SOURCE_API")
        generation = wait_for_pre_paper_api_ready(self.docker, self.deadline, simulation_ops,
            self.project, source_commit, image, cid, generation)
        self.source_api_generation = generation
        return generation

    def dispose_source_api(self):
        """Stop and remove only the source API carrying this fixture's nonce."""
        if self.source_api_disposed:
            return
        if self.source_api_nonce is None:
            self.cleanup["source_api"] = "NOT_STARTED"
            self.source_api_disposed = True
            return
        ids = self.output(["ps", "--all", "--no-trunc", "--filter",
            "label=io.lagrange.paper-fixture.source-api=" + self.source_api_nonce,
            "--format", "{{.ID}}"], cleanup=True).splitlines()
        require(len(ids) <= 1 and all(re.fullmatch(r"[0-9a-f]{64}", cid) for cid in ids),
                "FIXTURE_PRE_PAPER_SOURCE_API_CLEANUP")
        if not ids:
            require(self.source_api_generation is None and self.source_api_cid is None,
                    "FIXTURE_PRE_PAPER_SOURCE_API_CLEANUP")
            self.cleanup["source_api"] = "NOT_CREATED"
            self.source_api_disposed = True
            return
        cid = ids[0]
        require(self.source_api_cid is None or self.source_api_cid == cid,
                "FIXTURE_PRE_PAPER_SOURCE_API_CLEANUP")
        network = self.docker.object("network", self.network, self.deadline.seconds(15, cleanup=True))
        network_labels = network.get("Labels") if type(network) is dict else None
        network_name = network.get("Name") if type(network) is dict else None
        require(type(network) is dict and network.get("Id") == self.network and network.get("Internal") is True
                and type(network_labels) is dict and network_labels.get("com.docker.compose.project") == self.project
                and network_labels.get("com.docker.compose.network") == "owner-beta-db"
                and type(network_name) is str, "FIXTURE_PRE_PAPER_SOURCE_API_CLEANUP")
        value = self.docker.object("container", cid, self.deadline.seconds(15, cleanup=True))
        require(type(value) is dict, "FIXTURE_PRE_PAPER_SOURCE_API_CLEANUP")
        config, host, state = value.get("Config"), value.get("HostConfig"), value.get("State")
        labels = config.get("Labels") if type(config) is dict else None
        network_settings = value.get("NetworkSettings") if type(value) is dict else None
        networks = network_settings.get("Networks") if type(network_settings) is dict else None
        require(value.get("Id") == cid and value.get("Image") == self.source_api_image and
                type(config) is dict and type(host) is dict and type(state) is dict and
                type(labels) is dict and all(labels.get(key) == expected for key, expected in {
                    "com.docker.compose.project": self.project, "com.docker.compose.service": "api-server",
                    "com.docker.compose.oneoff": "False", REVISION_LABEL: self.source_api_commit,
                    "io.lagrange.paper-fixture.source-api": self.source_api_nonce}.items()) and
                config.get("User") == "10001:10001" and
                config.get("Entrypoint") == ["/usr/local/bin/api-server"] and config.get("Cmd") in (None, []) and
                value.get("Path") == "/usr/local/bin/api-server" and value.get("Args") == [] and
                host.get("PortBindings") in (None, {}) and host.get("PublishAllPorts") is False and
                type(networks) is dict and set(networks) == {network_name} and
                type(networks[network_name]) is dict and
                networks[network_name].get("NetworkID") == self.network,
                "FIXTURE_PRE_PAPER_SOURCE_API_CLEANUP")
        if self.source_api_generation is not None:
            require((value.get("Created"), state.get("StartedAt"), value.get("RestartCount")) ==
                    (self.source_api_generation.created, self.source_api_generation.started,
                     self.source_api_generation.restarts), "FIXTURE_PRE_PAPER_SOURCE_API_CLEANUP")
        if state.get("Running") is True:
            self.command(["stop", "--time", "45", cid], cap=60, cleanup=True)
            value = self.docker.object("container", cid, self.deadline.seconds(15, cleanup=True))
            config = value.get("Config") if type(value) is dict else None
            state = value.get("State") if type(value) is dict else None
            labels = config.get("Labels") if type(config) is dict else None
            require(type(value) is dict and value.get("Id") == cid and value.get("Image") == self.source_api_image
                    and type(labels) is dict and all(labels.get(key) == expected for key, expected in {
                        "com.docker.compose.project": self.project, "com.docker.compose.service": "api-server",
                        "com.docker.compose.oneoff": "False", REVISION_LABEL: self.source_api_commit,
                        "io.lagrange.paper-fixture.source-api": self.source_api_nonce}.items()) and
                    config.get("User") == "10001:10001" and
                    config.get("Entrypoint") == ["/usr/local/bin/api-server"] and
                    value.get("Path") == "/usr/local/bin/api-server" and value.get("Args") == [] and
                    (self.source_api_generation is None or (type(state) is dict and
                     (value.get("Created"), state.get("StartedAt"), value.get("RestartCount")) ==
                     (self.source_api_generation.created, self.source_api_generation.started,
                      self.source_api_generation.restarts))),
                    "FIXTURE_PRE_PAPER_SOURCE_API_CLEANUP")
        require(type(state) is dict and state.get("Running") is False and
                state.get("Status") in ("created", "exited"), "FIXTURE_PRE_PAPER_SOURCE_API_CLEANUP")
        self.command(["rm", cid], cleanup=True)
        absent = self.output(["ps", "-aq", "--no-trunc", "--filter",
            "label=io.lagrange.paper-fixture.source-api=" + self.source_api_nonce,
            "--format", "{{.ID}}"], cleanup=True)
        require(absent == "", "FIXTURE_PRE_PAPER_SOURCE_API_CLEANUP")
        self.cleanup["source_api"] = "REMOVED"
        self.source_api_disposed = True

    def _apply_migrations(self, migration_root, versions):
        """The existing sqlx-shaped loop for one contiguous version range of the release."""
        migrations = sorted((migration_root / "migrations").glob("*.up.sql"))
        require([int(path.name[:4]) for path in migrations] == list(range(1, 70))
                and type(versions) is range and versions.step == 1
                and 1 <= versions.start < versions.stop <= 70, "FIXTURE_MIGRATION_INVENTORY")
        pg = self.observer
        for path in migrations[versions.start - 1:versions.stop - 1]:
            script = path.read_text()
            if not script.startswith("-- no-transaction\n"):
                script = "BEGIN;\n" + script + "\nCOMMIT;"
            pg.sql(script, "migration_owner", cap=120)
            pg.sql("INSERT INTO public._sqlx_migrations(version,description,success,checksum,execution_time) "
                   "VALUES(%d,'fixture',true,decode('%s','hex'),0);" %
                   (int(path.name[:4]), hashlib.sha384(path.read_bytes()).hexdigest()), "migration_owner")

    def configure(self):
        pg = self.observer
        owner = str(uuid.uuid4())
        policy = existing_json_macro(self.release / "tests/integration/migration-contract/tests/migration_contract.rs",
                                     "daily_0060_policy")
        financial = existing_json_macro(self.release / "tests/integration/migration-contract/tests/paper_0067.rs",
                                        "financial_contract")
        pg.sql("BEGIN; SELECT set_config('app.actor_user_id','" + owner + "',true);"
               "INSERT INTO users(id,issuer,subject,email) VALUES('" + owner +
               "','https://daily.invalid','fixture-owner','fixture@example.invalid');"
               "INSERT INTO roles(id) VALUES('owner') ON CONFLICT DO NOTHING;"
               "INSERT INTO user_roles(user_id,role_id) VALUES('" + owner + "','owner');"
               "INSERT INTO strategies(id,display_name) VALUES('trend_following','Synthetic SQL strategy') ON CONFLICT DO NOTHING;" +
               "".join("INSERT INTO instruments(id,symbol,venue,currency) VALUES('%s','%s','KRX','KRW') "
                       "ON CONFLICT DO NOTHING;" % (s, s.split(".")[0]) for s in policy["instruments"]) +
               "INSERT INTO data_entitlements(contract_document_sha256,contract_reference,status,covered_datasets,"
               "covered_uses,effective_from,effective_until,managed_by) VALUES('" + "a" * 64 +
               "','daily-fixture','ACTIVE','[\"krx_eod_bars\"]','[\"recommendation\"]','2016-08-29',"
               "'2099-12-31','" + owner + "');"
               "INSERT INTO owner_beta_daily_control(owner_user_id,policy_sha256,policy_json,entitlement_reference,"
               "entitlement_sha256,first_auto_session,max_jobs_per_owner,lease_seconds,backoff_seconds) VALUES('" +
               owner + "','sha256:" + "b" * 64 + "','" + canonical(policy).decode() +
               "','daily-fixture','sha256:" + "a" * 64 + "','2026-08-29',4,60,1);"
               "UPDATE owner_beta_daily_control SET enabled=true; COMMIT;", "migration_owner")
        config = pg.json("BEGIN; SET LOCAL app.actor_user_id='" + owner + "';"
                         "INSERT INTO user_strategy_configs(owner_user_id,strategy_id,strategy_version,config_json,daily_enabled)"
                         " VALUES('" + owner + "','trend_following','1.0.0',"
                         "'{\"benchmark_instrument\":\"069500.KRX\",\"fast_ma\":50,\"slow_ma\":200}',true)"
                         " RETURNING to_json(id); COMMIT;", role="app")
        require(type(config) is str, "FIXTURE_CONFIG_ID")
        try:
            config_uuid = uuid.UUID(config)
        except (ValueError, TypeError, AttributeError):
            raise FixtureError("FIXTURE_CONFIG_ID") from None
        require(config_uuid.int != 0 and str(config_uuid) == config, "FIXTURE_CONFIG_ID")
        value = pg.json("SELECT public.configure_owner_beta_paper_v1('" + config +
                        "','SQL fixture','" + canonical(financial).decode() +
                        "'::jsonb,NULL::jsonb,'sql-config');", role="migration_owner")
        require(value["outcome"] == "OK" and value["control_epoch"] == 0
                and value["data"]["state"] == "OFF", "FIXTURE_CONFIGURE")
        self.account = value["account_id"]

    def resolve_helpers(self):
        """Prove original owned helper/process/backend disposition before FDs close."""
        self.cancel_event.set()
        intakes = getattr(self, "archive_intakes", ())
        for intake in intakes:
            require(type(intake) is driver.IntakeCancellation
                and intake.acceptance.owner is self, "FIXTURE_UNKNOWN_IDENTITY")
            intake.prepare_fixture_disposal()
        for context in tuple(self.contexts):
            context_ops = getattr(self, "context_modules", {}).get(context, ops)
            for attempt in context_ops._STOP_ATTEMPTS.get(context, {}).values():
                archive = getattr(attempt, "archive", None)
                if archive is not None and not archive.closed:
                    require(archive.owner is not None, "FIXTURE_UNKNOWN_IDENTITY")
                    archive.owner.close()  # actual owned clients, no role/PID waiver
                    require(archive.closed, "FIXTURE_BACKEND_DISPOSAL_UNPROVEN")
                consumer = getattr(attempt, "consumer", None)
                if consumer is not None:
                    for reader in consumer.readers:
                        if reader not in self.contexts:
                            self.contexts.append(reader)
            paper = sys.modules.get("owner_beta_paper_backup")
            space = None if paper is None else paper._SPACES.get(context)
            if space is not None and space.owner is not None and not space.owner._closed:
                # Standalone verify deliberately has no source stop attempt.
                # Its actual private owner still owns every clone/helper/FD.
                if not any(intake.package.owner is space.owner for intake in intakes):
                    space.owner.close()
                    require(space.owner._resources_closed(), "FIXTURE_BACKEND_DISPOSAL_UNPROVEN")
        for observer in reversed(self.tracees):
            try:
                observer.close()
            except (faults.FaultError, OSError):
                # Keep original FDs. An independently verified owned CID can
                # still be stopped, then detach/reap is required again below.
                pass
        for blocker in self.blockers:
            blocker.release()
        for context in self.contexts:
            for invocation in tuple(context._invocations):
                if invocation._child is not None:
                    invocation._reap_child(invocation._child)
                    invocation._child = None
        for thread in self.threads:
            thread.join(timeout=self.deadline.seconds(30, cleanup=True))
            require(not thread.is_alive(), "FIXTURE_OWNED_THREAD_ALIVE")
        for context in self.contexts:
            for invocation in tuple(context._invocations):
                if invocation._cid is not None:
                    self.helpers[invocation._cid] = invocation._nonce
                    output = [source for source, target, _readonly in invocation._mounts
                              if target == ops.OUTPUT_ALIAS]
                    require(len(output) == 1 and invocation._created, "FIXTURE_HELPER_BINDING_UNKNOWN")
                    self.helper_bindings[invocation._cid] = (invocation._created, output[0])
        # Also recover a helper whose driver failed before its first callback.
        ids = self.output(["ps", "--all", "--no-trunc", "--filter",
                           "label=" + ops.CONTAINER_LABEL, "--format", "{{.ID}}"], cleanup=True)
        for cid in ids.splitlines():
            value = self.docker.object("container", cid)
            mounts = value.get("Mounts") or []
            if any(m.get("Destination") == ops.OUTPUT_ALIAS and
                   str(m.get("Source", "")).startswith(str(self.root) + "/") for m in mounts):
                require(self.context is not None and value.get("Image") == self.context.images["paper-scheduler"],
                        "FIXTURE_FOREIGN_CLEANUP")
                nonce = (value.get("Config", {}).get("Labels") or {}).get(ops.CONTAINER_LABEL)
                require(type(nonce) is str and re.fullmatch("[0-9a-f]{32}", nonce),
                        "FIXTURE_FOREIGN_CLEANUP")
                self.helpers[cid] = nonce
                self.helper_bindings.setdefault(cid, (value.get("Created"), next(
                    m["Source"] for m in mounts if m.get("Destination") == ops.OUTPUT_ALIAS)))
        for cid, nonce in self.helpers.items():
            found = self.output(["ps", "--all", "--no-trunc", "--filter", "id=" + cid,
                                 "--format", "{{.ID}}"], cleanup=True)
            if found == "":
                continue
            require(found == cid, "FIXTURE_FOREIGN_CLEANUP")
            value = self.docker.object("container", cid)
            require(value.get("Id") == cid and (value.get("Config", {}).get("Labels") or {}).get(
                ops.CONTAINER_LABEL) == nonce, "FIXTURE_FOREIGN_CLEANUP")
            binding = self.helper_bindings.get(cid)
            require(binding is not None and binding[0] and value.get("Created") == binding[0]
                    and value.get("Image") == self.context.images["paper-scheduler"]
                    and any(m.get("Destination") == ops.OUTPUT_ALIAS and m.get("Source") == binding[1]
                            for m in (value.get("Mounts") or [])), "FIXTURE_FOREIGN_CLEANUP")
            if value.get("State", {}).get("Running"):
                self.command(["stop", "--time", "5", cid], cleanup=True)
            value = self.docker.object("container", cid)
            require(value.get("State", {}).get("Running") is False
                    and value.get("State", {}).get("Status") in ("created", "exited"),
                    "FIXTURE_HELPER_ALIVE")
            self.command(["rm", "--volumes", cid], cleanup=True)
            require(self.output(["ps", "-aq", "--no-trunc", "--filter", "id=" + cid],
                                cleanup=True) == "", "FIXTURE_HELPER_CLEANUP")
        for observer in reversed(self.tracees):
            observer.close()
        if self.observer is not None:
            until = time.monotonic() + self.deadline.seconds(10, cleanup=True)
            while True:
                try:
                    self.observer.absent(cleanup=True)
                    break
                except FixtureError as error:
                    if error.args != ("FIXTURE_BACKEND_ALIVE",):
                        raise
                    require(time.monotonic() < until, "FIXTURE_BACKEND_DISPOSAL_UNPROVEN")
                    time.sleep(0.02)
        for observer in self.tracees:
            observer.release_results()
        self.cleanup["backends"] = "ABSENT_PREPARED_ZERO" if self.observer else "NOT_STARTED"

    @staticmethod
    def close_field(handle, name):
        fd = getattr(handle, name, None)
        if isinstance(fd, int):
            os.close(fd)
            setattr(handle, name, None)

    def dispose(self, close_custody):
        """Supervisor disposal, preserving A's original uncertain status."""
        require(not self.network_sent or self.network is not None, "FIXTURE_NETWORK_CREATE_UNKNOWN")
        require(not self.pg_sent or self.pg is not None, "FIXTURE_PG_CREATE_UNKNOWN")
        if hasattr(self, "source_api_disposed"):
            self.dispose_source_api()
        self.dispose_stop_writer()
        self.resolve_helpers()
        for intake in getattr(self, "archive_intakes", ()):
            intake.dispose_retained_fixture()
        self.extractions.reconcile()
        if self.pg is not None:
            require(re.fullmatch("[0-9a-f]{64}", self.pg), "FIXTURE_PG_CREATE_UNKNOWN")
            value = self.docker.object("container", self.pg)
            config, state = value.get("Config") or {}, value.get("State") or {}
            mounts = {m.get("Destination"): m for m in value.get("Mounts") or []}
            require(value.get("Id") == self.pg and value.get("Image") == self.pg_image
                    and config.get("Image") == ops.POSTGRES_IMAGE
                    and (config.get("Labels") or {}).get("com.docker.compose.project") == self.project
                    and (config.get("Labels") or {}).get("com.docker.compose.service") == "postgres"
                    and mounts.get("/run/test-secret", {}).get("Source") == str(self.root / "postgres-secret")
                    and mounts.get("/var/lib/postgresql", {}).get("Source") == str(self.root / "pgdata"),
                    "FIXTURE_FOREIGN_CLEANUP")
            if self.observer is not None and self.observer.generation is not None:
                require((value["Created"], state["StartedAt"], value["RestartCount"], value["Image"])
                        == self.observer.generation, "FIXTURE_PG_RESTART")
            if state.get("Running") is True:
                self.command(["stop", "--time", "5", self.pg], cleanup=True)
            self.command(["rm", "--volumes", self.pg], cleanup=True)
            require(self.output(["ps", "-aq", "--no-trunc", "--filter", "id=" + self.pg],
                                cleanup=True) == "", "FIXTURE_PG_CLEANUP")
        if self.network is not None:
            require(re.fullmatch("[0-9a-f]{64}", self.network), "FIXTURE_NETWORK_CREATE_UNKNOWN")
            value = self.docker.object("network", self.network)
            require(value.get("Id") == self.network and value.get("Internal") is True
                    and (value.get("Labels") or {}).get("com.docker.compose.project") == self.project
                    and (value.get("Labels") or {}).get("com.docker.compose.network") == "owner-beta-db",
                    "FIXTURE_FOREIGN_CLEANUP")
            self.command(["network", "rm", self.network], cleanup=True)
            remaining = self.output(["network", "ls", "--no-trunc", "--filter", "id=" + self.network,
                                     "--format", "{{.ID}}"], cleanup=True)
            require(remaining == "", "FIXTURE_NETWORK_CLEANUP")
        self.cleanup["containers"] = "REMOVED"
        stages = set()
        for context in self.contexts:
            if context._closed:
                continue
            for inventory in tuple(context._inventories):
                # Native invocations are disposed below; do not run another
                # inventory/DB query after the server has been removed.
                if inventory._snapshot is not None:
                    inventory._snapshot.close()
                inventory._local.close()
            for invocation in tuple(context._invocations):
                stages.update(invocation._stages)
                for name in ("_probe_output_fd", "_paper_state_fd", "_result_fd",
                             "_preexisting_dispatch_fd"):
                    self.close_field(invocation, name)
                for fd, *_rest in invocation._paper_state_files:
                    os.close(fd)
                invocation._paper_state_files = ()
                for item in invocation._secret_sources:
                    os.close(item["file_fd"])
                    os.close(item["service_fd"])
                    item["tree"].close()
                invocation._secret_sources = ()
                # resolve_helpers has proved every original owned helper absent,
                # reaped clients and proved PostgreSQL backend/prepared absence.
                # This is fixture disposal only; no journal stage is advanced.
                invocation._cid = invocation._child = None
                invocation._closed = True
            stages.update(context._retained_stages)
            stages.update(entry["stage"] for entry in context._input_stages.values())
            context._close_begin_results()
            self.close_field(context, "_manifest_fd")
            if context.verified is not None:
                context.verified.close()
            context._release_tree.close()
            context.tree.close()
            self.close_field(context, "lock_fd")
            context._closed = True
        for context in self.contexts:
            context_ops = self.context_modules.get(context, ops)
            for attempt in context_ops._STOP_ATTEMPTS.get(context, {}).values():
                attempt.close()
            context_ops._STOP_ATTEMPTS.pop(context, None)
        paper = sys.modules.get("owner_beta_paper_backup")
        if paper is not None:
            for context in self.contexts:
                space = paper._SPACES.get(context)
                if space is not None:
                    space.close()
        close_custody()
        for stage in stages:
            if not stage.closed:
                stage.close(remove=True)
        for intake in getattr(self, "archive_intakes", ()):
            intake.remove_fixture_trees()
        require(faults.directory_identity(self.root) == self.root_id, "FIXTURE_ROOT_REPLACED")
        shutil.rmtree(self.root)
        if self.lock_fd is not None:
            os.close(self.lock_fd)
            self.lock_fd = None
        self.cleanup["inputs"] = "REMOVED_AFTER_DISPOSITION"


# Production-shaped pre-Paper installation: the Paper history absence proof (design 7.3).
PRE_PAPER_CASE = "PRE_PAPER_INSTALL"
BACKUP_RESTART_CASE = "BACKUP_RESTART"
# build-paper-fixture-images.sh --lifecycle-subset: only these two services are real images.
LIFECYCLE_SUBSET_BUILT = frozenset({"api-server", "paper-scheduler"})
LIFECYCLE_PLACEHOLDER_IMAGE_ID = "sha256:" + "a" * 64
# What the hosted job's `sudo env -i` hands an operator command; nothing else is inherited.
PRE_PAPER_ENV = {"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/root", "LC_ALL": "C", "LANG": "C",
                 "PYTHONDONTWRITEBYTECODE": "1"}
REVISION_LABEL = "org.opencontainers.image.revision"
DAILY_CONTROL_PROJECT_LABEL = "io.lagrange.daily-control-project"
HISTORY_INVENTORY_SQL = "deploy/db/owner-beta-paper-history-inventory.sql"
HISTORY_RECEIPT_DIR = ".lagrange-paper-history-receipts"
HISTORY_RECEIPT_SCHEMA = "owner-beta-paper-history-inventory-receipt-v1"
HISTORY_RECEIPT_KEYS = frozenset({
    "schema", "outcome", "inventory", "state", "install_root", "install_root_identity",
    "release_lock_identity", "source_commit", "target_commit", "source_manifest_sha256",
    "target_manifest_sha256", "inventory_sql_sha256", "migration_max", "migration_lineage_sha256",
    "database", "p4_index_sha256", "local", "boot_id", "observed_at"})
HISTORY_RECEIPT_DATABASE_KEYS = frozenset({"container_id", "created", "started_at", "restart_count",
                                           "image_id", "network_id", "database"})
HISTORY_RECEIPT_LOCAL_KEYS = frozenset({"paper_dir", "reader_floor", "lifecycle", "source_binding",
                                        "target_binding", "evidence_root", "source_paper_mode",
                                        "target_paper_mode"})
LIFECYCLE_SKELETON = ["anchors", "queries", "selections", "staging"]
MIGRATION_STATE_SQL = ("SELECT count(*)::text||':'||COALESCE(max(version),0)::text||':'||"
                       "CASE WHEN COALESCE(bool_and(success),false) THEN 't' ELSE 'f' END "
                       "FROM public._sqlx_migrations;")
# A sanity witness only; the installed inventory SQL is the authority for the predicate.
PAPER_CATALOG_SQL = ("SELECT (SELECT count(*) FROM pg_catalog.pg_class "
                     "WHERE relname LIKE 'owner\\_beta\\_paper\\_%')+(SELECT count(*) FROM pg_catalog.pg_proc "
                     "WHERE proname ~ '^paper_006[6-9]_' OR proname LIKE '%owner\\_beta\\_paper%');")
GUARD_UNKNOWN = b"PAPER_RELEASE_HISTORY_UNKNOWN"
_BACKUP_INVENTORY_PROGRAM = re.compile(
    r"paper_result=\$\(/usr/bin/python3 -I -S -B -c '([^']*)' \"\$paper_controller\" "
    r"\"\$daily_install_root\" \"\$guard_release\" \"\$code_commit\" \"\$compat_lock_fd\"\)")


def manifest_images(manifest):
    """service -> image ID of an already validated V3 manifest."""
    rows = [line.split("|") for line in manifest.decode("ascii").splitlines()[2:]]
    return {fields[1]: fields[3] for fields in rows}


def lifecycle_subset_manifest(manifest):
    """True only for the exact paper-lifecycle-fast subset; any other placeholder fails."""
    placeholders = {service for service, image in manifest_images(manifest).items()
                    if image == LIFECYCLE_PLACEHOLDER_IMAGE_ID}
    if not placeholders:
        return False
    require(placeholders == set(ops.daily.policy.MANIFEST_V3_SERVICES) - LIFECYCLE_SUBSET_BUILT,
            "FIXTURE_MANIFEST_V3")
    return True


def pre_paper_source_commit(commit):
    """A fixed foreign commit for the relabelled pre-Paper current release."""
    value = hashlib.sha1(b"lagrange-pre-paper-source\0" + commit.encode("ascii")).hexdigest()
    require(re.fullmatch("[0-9a-f]{40}", value) is not None and value not in (commit, "0" * 40),
            "FIXTURE_PRE_PAPER_SOURCE")
    return value


def relabel_dockerfile(base_ref, revision):
    """Config-only: no RUN, COPY or pull; only the revision label changes."""
    require(re.fullmatch(r"lagrange-station-[a-z0-9-]{1,64}:[0-9a-f]{40}", base_ref) is not None
            and re.fullmatch("[0-9a-f]{40}", revision) is not None, "FIXTURE_PRE_PAPER_IMAGE")
    return ("FROM " + base_ref + "\nLABEL " + REVISION_LABEL + "=" + revision + "\n").encode("ascii")


def pre_paper_manifest(commit, images):
    services = ops.daily.policy.MANIFEST_V3_SERVICES
    require(set(images) == set(services), "FIXTURE_PRE_PAPER_SOURCE")
    data = ("LAGRANGE_RELEASE_MANIFEST_V3\ncommit|" + commit + "\n" + "".join(
        "image|" + service + "|lagrange-station-" + service + ":" + commit + "|" + images[service]
        + "|" + commit + "\n" for service in services)).encode("ascii")
    validate_v3_bytes(data, commit)
    return data


def migration_lineage(directory, maximum):
    """The inventory's lineage term: sha256 of "<version>:<sha384 hex>" for 1..maximum."""
    paths = sorted(pathlib.Path(directory).glob("*.up.sql"))
    rows = [(int(path.name[:4]), path) for path in paths if int(path.name[:4]) <= maximum]
    require([version for version, _ in rows] == list(range(1, maximum + 1)), "FIXTURE_MIGRATION_INVENTORY")
    text = ",".join("%d:%s" % (version, hashlib.sha384(path.read_bytes()).hexdigest()) for version, path in rows)
    return hashlib.sha256(text.encode("ascii")).hexdigest()


def guard_argv(install_root, commit, lock_fd=None):
    """deploy-production-release.sh atomic_activate's Paper guard argv for a marker-free root."""
    argv = ["/usr/bin/python3", str(ROOT / "scripts/ops/lib/owner-beta-paper-release.py"), "reader-guard",
            "--install-root", install_root, "--target-release", install_root + "/releases/" + commit,
            "--commit", commit]
    if lock_fd is not None:
        require(type(lock_fd) is int and lock_fd >= 3, "FIXTURE_PRE_PAPER_LOCK")
        argv += ["--history-inventory", "--lock-fd", str(lock_fd)]
    return argv


def deploy_check_argv(install_root, commit):
    return ["/bin/bash", str(ROOT / "scripts/ops/deploy-production-release.sh"), "--check",
            "--commit", commit, "--install-root", install_root]


def deploy_apply_argv(source, root, commit, inputs, simulation_inputs):
    config, receipt, complete = (simulation_inputs / name for name in RELEASE_INPUT_FILES)
    return ["/bin/bash", str(source / "scripts/ops/deploy-production-release.sh"), "--apply",
            "--commit", commit, "--install-root", str(root),
            "--env-source", str(inputs / "release.env"),
            "--release-manifest", str(inputs / ops.release.MANIFEST_NAME),
            "--simulation-config-source", str(config),
            "--simulation-receipt-source", str(receipt),
            "--simulation-complete-source", str(complete)]


def require_apply_transition_refusal(result):
    require(result.returncode == 1 and result.stdout == b"" and result.stderr.splitlines() == [
                b"SIMULATION_OPERATION_REJECTED",
                b"deploy-production-release: simulation transition failed",
            ], "FIXTURE_PRE_PAPER_APPLY_TRANSITION")


def backup_inventory_argv(install_root, release, commit, lock_fd, *, text=None):
    """run-production-backup.sh's exact locked Paper history adapter, read from that script."""
    if text is None:
        text = (ROOT / "scripts/ops/run-production-backup.sh").read_text()
    programs = _BACKUP_INVENTORY_PROGRAM.findall(text)
    require(len(programs) == 1 and "module.inspect_locked_install(" in programs[0]
            and type(lock_fd) is int and lock_fd >= 3, "FIXTURE_PRE_PAPER_BACKUP")
    return ["/usr/bin/python3", "-I", "-S", "-B", "-c", programs[0],
            str(ROOT / "scripts/ops/lib/owner-beta-paper-backup.py"), install_root, release, commit, str(lock_fd)]


def run_owned(argv, *, env, timeout, pass_fds=(), reap=10):
    """One owned child session. An interruption kills and reaps the whole session group, so no
    grandchild keeps a release lock or a Docker client after the fixture moves on."""
    child = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             env=env, pass_fds=pass_fds, start_new_session=True)
    try:
        stdout, stderr = child.communicate(timeout=timeout)
    except BaseException as error:
        with contextlib.suppress(ProcessLookupError):
            os.killpg(child.pid, signal.SIGKILL)
        try:
            child.wait(timeout=reap)
        finally:
            for stream in (child.stdout, child.stderr):
                stream.close()
        if isinstance(error, subprocess.TimeoutExpired):
            raise FixtureError("FIXTURE_PRE_PAPER_COMMAND_TIMEOUT") from None
        raise
    return subprocess.CompletedProcess(argv, child.returncode, stdout, stderr)


def command_witness(step, result):
    """Typed: exit status and closed uppercase codes only, never messages, paths or values."""
    codes = sorted({token.decode("ascii") for token in re.findall(
        rb"\b(?:PAPER|DAILY|SIMULATION|JOURNAL|POLICY)_[A-Z0-9_]{1,80}\b", result.stderr)})[:8]
    print("PAPER_FIXTURE_PRE_PAPER_COMMAND=" + canonical(
        {"step": step, "exit": result.returncode, "codes": codes}).decode(), flush=True)


def guard_lines(stdout):
    return [line for line in stdout.splitlines() if line.startswith(b"PAPER_READER_GUARD:")]


def history_receipt_name_valid(name, commit):
    """`<UTC %Y%m%dT%H%M%S.%fZ>-<target commit>-<8 lowercase hex>.json`."""
    return type(name) is str and re.fullmatch(
        r"[0-9]{8}T[0-9]{6}\.[0-9]{6}Z-" + re.escape(commit) + r"-[0-9a-f]{8}\.json", name) is not None


def history_receipts(root, commit):
    """Retained receipt names and bytes; the directory and every file must be root-only."""
    path = pathlib.Path(root) / HISTORY_RECEIPT_DIR
    try:
        info = path.lstat()
    except FileNotFoundError:
        return {}
    require(stat.S_ISDIR(info.st_mode) and (info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode)) == (0, 0, 0o700),
            "FIXTURE_PRE_PAPER_RECEIPT")
    names = sorted(os.listdir(path))
    require(len(names) <= 64, "FIXTURE_PRE_PAPER_RECEIPT")
    result = {}
    for name in names:
        require(history_receipt_name_valid(name, commit), "FIXTURE_PRE_PAPER_RECEIPT")
        fd = os.open(path / name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        try:
            item = os.fstat(fd)
            require(stat.S_ISREG(item.st_mode) and (item.st_uid, item.st_gid, stat.S_IMODE(item.st_mode),
                    item.st_nlink) == (0, 0, 0o400, 1) and item.st_size <= 4096, "FIXTURE_PRE_PAPER_RECEIPT")
            result[name] = os.pread(fd, 4097, 0)
        finally:
            os.close(fd)
    return result


def history_receipt_value(data, expected):
    """Exact section-5 receipt of one LEGACY_OFF verdict. Evidence only, never an input."""
    require(type(expected) is dict and set(expected) == HISTORY_RECEIPT_KEYS - {"observed_at"},
            "FIXTURE_PRE_PAPER_RECEIPT")
    require(type(data) is bytes and 0 < len(data) <= 4096 and data.endswith(b"\n")
            and not data.endswith(b"\n\n"), "FIXTURE_PRE_PAPER_RECEIPT")
    try:
        value = json.loads(data[:-1].decode("utf-8"))
        encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"),
                             allow_nan=False).encode("utf-8")
    except (ValueError, UnicodeError, TypeError):
        raise FixtureError("FIXTURE_PRE_PAPER_RECEIPT") from None
    require(encoded == data[:-1] and type(value) is dict and set(value) == HISTORY_RECEIPT_KEYS
            and type(value["database"]) is dict and set(value["database"]) == HISTORY_RECEIPT_DATABASE_KEYS
            and type(value["local"]) is dict and set(value["local"]) == HISTORY_RECEIPT_LOCAL_KEYS
            and type(value["observed_at"]) is str
            and re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{6}Z",
                             value["observed_at"]) is not None, "FIXTURE_PRE_PAPER_RECEIPT")
    # Canonical bytes keep true/false/0 and list/dict shapes distinct.
    for key, wanted in expected.items():
        require(canonical(value[key]) == canonical(wanted), "FIXTURE_PRE_PAPER_RECEIPT")
    return value


def require_pre_paper_root(root):
    """No Paper marker: only the installer's exact empty lifecycle skeleton is Paper-shaped."""
    root = pathlib.Path(root)
    require({name for name in os.listdir(root) if name.startswith(".owner-beta-paper")}
            == {".owner-beta-paper-lifecycle"}, "FIXTURE_PRE_PAPER_ROOT")
    store = root / ".owner-beta-paper-lifecycle"
    members = sorted(os.listdir(store))
    require(members == LIFECYCLE_SKELETON and not any(os.listdir(store / name) for name in members),
            "FIXTURE_PRE_PAPER_ROOT")


@contextlib.contextmanager
def released_release_lock(owner):
    """deploy --check takes the release lock itself; the original description is kept and retaken."""
    identity = ops._identity(os.fstat(owner.lock_fd))[:2]
    fcntl.flock(owner.lock_fd, fcntl.LOCK_UN)
    try:
        yield
    finally:
        try:
            fcntl.flock(owner.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            raise FixtureError("FIXTURE_PRE_PAPER_LOCK") from None
        require(ops._identity(os.fstat(owner.lock_fd))[:2] == identity
                and ops._identity(os.stat(owner.root / ops.LOCK_NAME, follow_symlinks=False))[:2] == identity,
                "FIXTURE_PRE_PAPER_LOCK")


def relabel_source_images(owner, source_commit, derived):
    """One config-only build per distinct manifest image; derived records each ID before checks."""
    images = manifest_images(owner.manifest)
    work = owner.root / "pre-paper-relabel"
    work.mkdir(mode=0o700)
    result = {}
    for index, service in enumerate(ops.daily.policy.MANIFEST_V3_SERVICES):
        base = images[service]
        if base not in derived:
            # The exact configured tag the image build verified; FROM never names a bare ID.
            reference = "lagrange-station-" + service + ":" + owner.commit
            original = owner.docker.object("image", reference)
            base_config = original.get("Config") or {}
            require(original.get("Id") == base and (base_config.get("Labels") or {}).get(REVISION_LABEL)
                    == owner.commit, "FIXTURE_PRE_PAPER_IMAGE")
            context = work / ("%02d" % index)
            context.mkdir(mode=0o700)
            write_new(context / "Dockerfile", relabel_dockerfile(reference, source_commit))
            iid = context / "image-id"
            built = subprocess.run(owner.argv(["build", "--pull=false", "--network=none", "--iidfile", str(iid),
                                               str(context)]),
                                   stdin=subprocess.DEVNULL, capture_output=True, env=dict(ops.daily.ENVIRONMENT),
                                   timeout=owner.deadline.seconds(180), check=False)
            require(built.returncode == 0, "FIXTURE_PRE_PAPER_IMAGE")
            identity = iid.read_text().strip()
            require(re.fullmatch("sha256:[0-9a-f]{64}", identity) is not None and identity != base
                    and identity not in derived.values(), "FIXTURE_PRE_PAPER_IMAGE")
            derived[base] = identity
            # Config-only: the same layers, the new revision and every other original label.
            value = owner.docker.object("image", identity)
            labels = (value.get("Config") or {}).get("Labels") or {}
            require(value.get("Id") == identity and labels.get(REVISION_LABEL) == source_commit
                    and all(labels.get(key) == item for key, item in (base_config.get("Labels") or {}).items()
                            if key != REVISION_LABEL)
                    and (value.get("RootFS") or {}).get("Layers") == (original.get("RootFS") or {}).get("Layers"),
                    "FIXTURE_PRE_PAPER_IMAGE")
        result[service] = derived[base]
    return result


def install_pre_paper_source(owner, source_commit, images):
    """A published pre-Paper release S: archive content, no deploy/owner-beta-paper, own manifest."""
    releases = owner.root / "releases"
    release = releases / source_commit
    require(not os.path.lexists(release), "FIXTURE_PRE_PAPER_SOURCE")
    staging = releases / (".staging." + source_commit + "." + uuid.uuid4().hex)
    staging.mkdir(mode=0o755)
    owner._extract_archive(owner.root / "source.tar", staging)
    require(not os.path.lexists(staging / "deploy/owner-beta-paper"), "FIXTURE_PRE_PAPER_SOURCE")
    manifest = pre_paper_manifest(source_commit, images)
    write_new(staging / "deploy/compose/.env", owner._release_env(source_commit))
    write_new(staging / ops.release.MANIFEST_NAME, manifest)
    write_new(staging / ".lagrange-release", (source_commit + "\n").encode())
    staging.rename(release)
    fd = os.open(releases, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)
    # The production readers the inventory applies to its source accept S as a pre-Paper release.
    require(ops.daily.read_manifest(str(release), source_commit) == images, "FIXTURE_PRE_PAPER_SOURCE")
    values, _overlays = ops.daily.read_context(str(release), source_commit, "disable")
    require(values["OWNER_BETA_PAPER_MODE"] == "disabled", "FIXTURE_PRE_PAPER_SOURCE")
    ops.daily.runtime.image_owned(owner.docker, ops.daily.Ownership(owner.project, source_commit,
                                                                    images["db-migrate"]))
    return manifest


RELEASE_INPUT_FILES = ("simulation-config.json", "simulation-artifact.json", "simulation-COMPLETE")


def pre_paper_publication_witness(release, commit, simulation_inputs, env, manifest):
    simulation = load("paper_genuine_fixture_simulation_release",
                      ROOT / "scripts/ops/lib/owner-beta-simulation-release.py")
    expected = [
        (".lagrange-release", (commit + "\n").encode()),
        (ops.release.MANIFEST_NAME, manifest),
        ("deploy/compose/.env", env),
        (str(pathlib.Path(simulation.CONFIG_DIRECTORY) / simulation.CONFIG_NAME),
         (simulation_inputs / RELEASE_INPUT_FILES[0]).read_bytes()),
        (".owner-beta-simulation-receipt/artifact.json",
         (simulation_inputs / RELEASE_INPUT_FILES[1]).read_bytes()),
        (".owner-beta-simulation-receipt/COMPLETE",
         (simulation_inputs / RELEASE_INPUT_FILES[2]).read_bytes()),
    ]
    records = []
    for name, wanted in expected:
        path = release / name
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        try:
            before = os.fstat(fd)
            require(stat.S_ISREG(before.st_mode) and before.st_nlink == 1 and
                    before.st_size == len(wanted), "FIXTURE_PRE_PAPER_APPLY_PUBLICATION")
            with os.fdopen(os.dup(fd), "rb") as stream:
                actual = stream.read(len(wanted) + 1)
            after = os.fstat(fd)
            require(actual == wanted and (before.st_dev, before.st_ino, before.st_size,
                    before.st_mtime_ns, before.st_ctime_ns) ==
                    (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns),
                    "FIXTURE_PRE_PAPER_APPLY_PUBLICATION")
            records.append([name, before.st_dev, before.st_ino, before.st_size, hashlib.sha256(actual).hexdigest()])
        finally:
            os.close(fd)
    return {"identity": faults.directory_identity(release),
            "sha256": hashlib.sha256(canonical(records)).hexdigest()}


def pre_paper_handoff_witness(root, operations, expected):
    path = pathlib.Path(root) / operations.HANDOFF_PENDING
    try:
        named = path.lstat()
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    except OSError:
        raise FixtureError("FIXTURE_PRE_PAPER_HANDOFF") from None
    try:
        before = os.fstat(fd)
        require(stat.S_ISREG(named.st_mode) and stat.S_ISREG(before.st_mode) and
                (named.st_dev, named.st_ino) == (before.st_dev, before.st_ino) and
                (before.st_uid, before.st_gid, stat.S_IMODE(before.st_mode), before.st_nlink) ==
                (os.geteuid(), os.getegid(), 0o600, 1) and 0 < before.st_size <= operations.PENDING_LIMIT,
                "FIXTURE_PRE_PAPER_HANDOFF")
        with os.fdopen(os.dup(fd), "rb") as stream:
            data = stream.read(operations.PENDING_LIMIT + 1)
        after = os.fstat(fd)
        require(data == operations.daily.install.canonical(expected) and
                (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) ==
                (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns),
                "FIXTURE_PRE_PAPER_HANDOFF")
        return {"identity": [before.st_dev, before.st_ino], "size": len(data),
                "sha256": hashlib.sha256(data).hexdigest()}
    finally:
        os.close(fd)


def previous_release_configs(simulation, source_commit, source_manifest, images):
    """Synthetic protected simulation and Daily configs of S: {config directory: bytes}."""
    hashes = {key: "sha256:" + digit * 64 for key, digit in (("receipt_sha256", "1"), ("binary_sha256", "2"),
              ("source_input_sha256", "3"), ("recipe_sha256", "4"), ("entitlement_sha256", "5"))}
    config = {"schema": simulation.SIMULATION_SCHEMA, "code_commit": source_commit,
              "manifest_format": simulation.MANIFEST_V3_FORMAT, "manifest_sha256": ops._digest(source_manifest),
              "image_id": images[simulation.BIN], **hashes, "build_origin": simulation.BUILD_ORIGIN,
              "package": simulation.PACKAGE, "bin": simulation.BIN, "target": simulation.TARGET,
              "profile": simulation.PROFILE, "toolchain": simulation.TOOLCHAIN,
              "owner": "11111111-1111-4111-8111-111111111111", "entitlement_reference": "synthetic-pre-paper",
              "historical_root": "/data/owner-beta-artifacts", "reader_uid": 10001, "reader_gid": 10001,
              "paths": dict(simulation._PATHS),
              "limits": {"memory_bytes": 1, "nano_cpus": 1, "health_tmpfs_bytes": 1, "pids_limit": 1,
                         "startup_timeout_seconds": 1, "health_max_age_seconds": 1,
                         "stop_timeout_seconds": 60}}  # The source API has a 45-second Compose stop grace.
    install = ops.daily.install
    daily = {"schema": "owner-beta-daily-process-v1", "owner": config["owner"],
             "policy_sha256": install.POLICY_SHA256, **install.PATHS, "raw_owner_uid": 10001,
             "scan": {key: 1 for key in install.SCAN_KEYS}, "max_artifact_entries": 1,
             "max_replay_admissions": 1, "max_cycle_seconds": 1, "code_commit": source_commit,
             "entitlement_reference": "synthetic-pre-paper", "entitlement_sha256": hashes["entitlement_sha256"],
             "first_auto_session": "2026-08-31"}
    return {simulation.CONFIG_DIRECTORY: canonical(config), install.CONFIG_DIRECTORY: canonical(daily)}


def release_inputs_argv(install_root, commit, source_commit, inputs, output, *, daily):
    """simulation-release-inputs.py --apply: S's protected configs and C's real D8 receipt pair."""
    argv = ["/usr/bin/python3", "-I", "-B", str(ROOT / "scripts/ops/simulation-release-inputs.py"), "--apply",
            "--commit", commit, "--manifest", install_root + "/releases/" + commit + "/" + ops.release.MANIFEST_NAME,
            "--receipt", inputs + "/artifact.json", "--complete", inputs + "/COMPLETE",
            "--previous-release", install_root + "/releases/" + source_commit, "--output-dir", output]
    return argv + ["--daily"] if daily else argv


def dispose_daily_control_oneshots(owner, allowed_images):
    """Remove only this project's labelled migration-owner one-shots, before the network goes."""
    ids = owner.output(["ps", "--all", "--no-trunc", "--filter",
                        "label=" + DAILY_CONTROL_PROJECT_LABEL + "=" + owner.project,
                        "--format", "{{.ID}}"], cleanup=True).split()
    require(len(ids) <= 16, "FIXTURE_FOREIGN_CLEANUP")
    for cid in ids:
        require(re.fullmatch("[0-9a-f]{64}", cid) is not None, "FIXTURE_FOREIGN_CLEANUP")
        value = owner.docker.object("container", cid)
        labels = (value.get("Config") or {}).get("Labels") or {}
        require(value.get("Id") == cid and labels.get(DAILY_CONTROL_PROJECT_LABEL) == owner.project
                and re.fullmatch("[0-9a-f]{32}", labels.get("io.lagrange.daily-control") or "") is not None
                and value.get("Image") in allowed_images, "FIXTURE_FOREIGN_CLEANUP")
        if (value.get("State") or {}).get("Running"):
            owner.command(["kill", "--signal=TERM", cid], cleanup=True, allow_failure=True)
            owner.command(["wait", cid], cap=25, cleanup=True, allow_failure=True)
        owner.command(["rm", cid], cleanup=True)
        require(owner.output(["ps", "-aq", "--no-trunc", "--filter", "id=" + cid], cleanup=True) == "",
                "FIXTURE_PRE_PAPER_ONESHOT_CLEANUP")


def remove_relabelled_images(owner, derived):
    """Remove each derived image by ID, never forced; the shared base images stay untouched."""
    for identity in derived.values():
        listing = ["image", "ls", "--all", "--no-trunc", "--format", "{{.ID}}"]
        if identity in owner.output(listing, cleanup=True).split():
            owner.command(["image", "rm", identity], cleanup=True)
        require(identity not in owner.output(listing, cleanup=True).split(), "FIXTURE_PRE_PAPER_IMAGE_CLEANUP")


class FixtureRunner:
    def __init__(self, source, commit, manifest, deadline, policy_root):
        self.source, self.commit, self.manifest = source, commit, manifest
        self.deadline, self.policy_root = deadline, policy_root
        self.results, self.cleanups, self.completed_begin = [], [], None

    @contextlib.contextmanager
    def fixture_case(self, purpose, *, case=None):
        require(purpose in ("STOP", "BACKUP", "RESTORE"), "FIXTURE_PURPOSE")
        self.deadline.action_phase()
        owner = FixtureOwner(self.source, self.commit, self.manifest, self.deadline)
        with owner.extractions:
            journal_manager = operation = action = None
            primary = None

            def close_custody(*, disposing=False):
                nonlocal journal_manager, operation, action
                if action is not None:
                    action.close()
                    action = None
                if operation is not None:
                    operation.close()
                    operation = None
                if journal_manager is not None:
                    manager, journal_manager = journal_manager, None
                    try:
                        manager.__exit__(None, None, None)
                    except ops.journal.JournalError as error:
                        if not disposing or not owner.original_custody_uncertain or error.code != "JOURNAL_CHANGED":
                            raise

            try:
                owner.setup(self.policy_root)
                owner.configure()
                journal_manager = ops.journal.open_journal(str(owner.root), owner.lock_fd, bootstrap=True)
                store = journal_manager.__enter__()
                require(store.epoch_floor == 0, "FIXTURE_FLOOR")
                generation = str(uuid.uuid4())
                root = {"schema": "owner-beta-paper-operation-intent-v1", "installation_id": store.installation_id,
                        "generation": generation, "account_id": owner.account, "predecessor_generation": None,
                        "request_key": generation + ":begin", "purpose": purpose,
                        "scope": "PAPER_STOP" if purpose == "STOP" else "COMBINED_BACKUP",
                        "expected_epoch": 0, "local_epoch_floor": 0,
                        "release_manifest_sha256": ops._digest(self.manifest), "artifact_sha256": None,
                        "resume_target_id": None}
                if purpose == "RESTORE":
                    # Use an actual preceding native result identity, explicitly
                    # unverified as an archive. RESTORE must reject before use.
                    require(self.completed_begin is not None, "FIXTURE_RESTORE_PREDECESSOR")
                    root["artifact_sha256"] = self.completed_begin["digest"]
                    root["predecessor_generation"] = self.completed_begin["generation"]
                operation = store.bind_caller("fixture:" + generation, canonical(root))
                stage = {"schema": "owner-beta-paper-operation-stage-v1", "installation_id": store.installation_id,
                         "generation": generation, "account_id": owner.account, "sequence": 0,
                         "previous_sha256": None, "intent_sha256": operation.intent_sha256, "stage": "INTENT",
                         "boot_id": pathlib.Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
                         "observed": {"realtime_ns": time.time_ns(), "monotonic_ns": time.monotonic_ns(),
                                      "boottime_ns": time.clock_gettime_ns(time.CLOCK_BOOTTIME)},
                         "control_epoch": 0, "artifact_sha256": None, "event_id": None, "error_code": "NONE"}
                stage_hash = operation.append(canonical(stage))
                binding = {"schema": "owner-beta-paper-action-binding-v1", "installation_id": store.installation_id,
                           "generation": generation, "root_intent_sha256": operation.intent_sha256,
                           "sequence": 0, "previous_binding_sha256": None, "verb": "begin",
                           "request_key": generation + ":begin", "expected_epoch": 0,
                           "artifact_sha256": None, "stage_sha256": stage_hash}
                action = operation.prepare_action(canonical(binding))
                owner.observer.operation, owner.observer.action = operation, action
                yield owner.context, owner.lock_fd, operation, action, owner.observer, close_custody
            except BaseException as error:
                primary = error
                raise
            finally:
                self.deadline.cleanup_phase()
                # Resolve actual helper/backend resources before releasing journal
                # originals, secrets or the held release open description.
                try:
                    owner.dispose(lambda: close_custody(disposing=True))
                except BaseException as cleanup_error:
                    if primary is not None:
                        # Raising "from" replaces the primary's own context with the cleanup
                        # error. Keep the first error reachable for the failure details.
                        first = primary.__context__
                        if first is not None and first is not cleanup_error:
                            primary.fixture_first_context = first
                        raise primary.with_traceback(primary.__traceback__) from cleanup_error
                    raise
                self.cleanups.append({"case": purpose if case is None else case, **owner.cleanup})

    def positive(self, purpose):
        with self.fixture_case(purpose) as (context, lock_fd, operation, action, pg, _close_custody):
            # Hold a real row lock until the first actual BEGIN backend/socket
            # can be correlated. An attach client is never the DB proof.
            with RowLock(pg) as blocker:
                failures = []
                def release_when_observed():
                    try:
                        end = time.monotonic() + pg.owner.deadline.seconds(60)
                        while time.monotonic() < end and not pg.owner.cancel_event.is_set():
                            primary = getattr(pg, "primary_invocation", None)
                            if primary is not None:
                                try:
                                    observed = pg.helper(primary)[3]
                                except (FixtureError, OSError):
                                    observed = False
                                if observed:
                                    pg.release_blocked(blocker)
                                    return
                            time.sleep(0.02)
                        raise FixtureError("FIXTURE_BEGIN_BACKEND_TIMEOUT")
                    except BaseException as error:
                        failures.append(error)
                watcher = threading.Thread(target=release_when_observed)
                pg.owner.threads.append(watcher)
                watcher.start()
                try:
                    result = driver.run_genuine_fixture(context, lock_fd, action, observer=pg.barrier)
                finally:
                    watcher.join(timeout=pg.owner.deadline.seconds(10, cleanup=True))
                    require(not watcher.is_alive(), "FIXTURE_WATCHER_ALIVE")
                require(not failures, "FIXTURE_WATCHER_FAILURE")
            self.completed_begin = pg.original
            self.results.append({"case": purpose, "status": result["status"],
                                 "counters": pg.snapshot(), "originals": pg.original_hashes,
                                 "release": pg.owner.release_hashes, "producers": pg.owner.producer_hashes,
                                 "stop": pg.stop_receipt, "capture": result["capture"]})

    def restore_negative(self):
        with self.fixture_case("RESTORE") as (context, lock_fd, operation, action, pg, _close_custody):
            before = pg.rejection_counts()
            driver.run_restore_no_sql_rejection(context, lock_fd, action, observer=pg.barrier)
            after = pg.rejection_counts()
            require(all(before[k] == after[k] == 0 for k in ("sql", "events", "receipts"))
                    and before["oid"] == after["oid"] and before["function"] == after["function"],
                    "FIXTURE_RESTORE_SQL")
            self.results.append({"case": "RESTORE_UNVERIFIED_ARCHIVE_REJECTED", "status": "PASS",
                                 "counters": after})

    def backup_restart(self):
        """Real manually linked BEGIN from expired STOPPED, bounded before archive publication."""
        if lifecycle_subset_manifest(self.manifest):
            self.results.append({"case": BACKUP_RESTART_CASE, "status": "NOT_RUN",
                                 "reason": "LIFECYCLE_SUBSET_PLACEHOLDER_IMAGES"})
            return
        with self.fixture_case("BACKUP", case=BACKUP_RESTART_CASE) as resources:
            result = self._backup_restart_steps(*resources)
        self.results.append(result)

    def _backup_restart_steps(self, context, lock_fd, original, action, pg, close_custody):
        paper = driver._paper_backup()
        installed, _ = paper._application_context(context)
        owner = pg.owner
        end = None

        class RetainStopped(Exception):
            pass

        def stop_at_observed_stage(phase, resource, report):
            pg.barrier(phase, resource, report)
            if phase == "stopped":
                nonlocal end
                end = resource._attempt.deadline.end
                raise RetainStopped

        try:
            # This uses the existing driver's native BEGIN, exact readback and observed
            # retained writer shutdown. The fixture stops before its usual OFF action.
            driver.run_genuine_fixture(context, lock_fd, action, observer=stop_at_observed_stage)
        except RetainStopped:
            pass
        require(type(end) is int and original.revalidate() == "STOPPED", "FIXTURE_BACKUP_RESTART_STOPPED")
        now = time.monotonic_ns()
        if end > now:
            remaining = self.deadline.seconds(240)
            require((end - now) / 1e9 < remaining, "FIXTURE_DEADLINE")
        while (now := time.monotonic_ns()) < end:
            time.sleep(min((end - now) / 1e9, self.deadline.seconds(1)))
        require(now >= end, "FIXTURE_BACKUP_RESTART_STOPPED")
        self.deadline.seconds(180)
        pg.phase = "backup-restart"
        baseline = pg.settle({"sql": 1, "function": 1, "events": 1, "receipts": 1})
        predecessor = original.generation
        old_head = original._last
        old_receipt = pg.receipt(predecessor)
        fence = old_head[3]["control_epoch"]
        # The production successor advances the floor by replacing its inode.
        # Release the original journal while it can still revalidate its own floor.
        close_custody()
        readbacks, predecessors, reobserved = [], [], []
        audit_attempts = []
        capture_limit = 512

        def observe(phase, event, frame, value):
            if phase == "stopped-validation" and event == "call":
                reobserved.append(frame.f_locals["operation"].generation)
                return
            if phase == "capture-query" and event == "call" and frame.f_locals["destination"] is not None:
                audit_attempts.append(frame.f_locals["self"].backend is not None)
                return
            if event != "return" or value is None:
                return
            if phase in ("context", "custody") and value.install_root == str(owner.root):
                owner.contexts.append(value)
                owner.context_modules[value] = installed
            elif phase == "predecessor":
                predecessors.append(value)
            elif phase == "readback":
                call = frame.f_locals["self"]
                if call._context.install_root == str(owner.root) and call.result["generation"] == predecessor:
                    readbacks.append(installed.journal._json(value, installed.journal.MAX_NATIVE_EXACT))

        def expect_rejection(restart, code):
            before = private_tree_witness(owner.root / ops.PAPER_DIR)
            try:
                paper.run_locked(context, lock_fd, "backup", capture_limit, restart=restart)
            except paper.PaperBackupError as error:
                require(str(error) == code, "FIXTURE_BACKUP_RESTART_REJECTION")
            else:
                raise FixtureError("FIXTURE_BACKUP_RESTART_REJECTION")
            require(private_tree_witness(owner.root / ops.PAPER_DIR) == before,
                    "FIXTURE_BACKUP_RESTART_REJECTION")

        phases = {"context": installed.open_context, "custody": installed.open_custody_context,
                  "readback": installed._NativeInvocation.readback_artifact,
                  "predecessor": paper._latest_predecessor,
                  "stopped-validation": installed.verify_stopped_backup_restart,
                  "capture-query": paper._Client.exchange}
        with faults.ProductionPhases(phases, observe, self.deadline.value):
            key = "fixture-restart:" + predecessor
            expect_rejection((predecessor, "fixture:" + predecessor),
                             "PAPER_BACKUP_RESTART_KEY")
            expect_rejection((str(uuid.uuid4()), key), "PAPER_BACKUP_PRIOR_OPERATION_UNRESOLVED")
            require(counter_delta(baseline, pg.settle(
                {"sql": 1, "function": 1, "events": 1, "receipts": 1})) ==
                {"sql": 0, "function": 0, "events": 0, "receipts": 0},
                "FIXTURE_BACKUP_RESTART_REJECTION")
            try:
                # The exporter identity and snapshot token fit; the audit stream exceeds the
                # limit before file selection, after the linked BEGIN and OFF.
                paper.run_locked(context, lock_fd, "backup", capture_limit, restart=(predecessor, key))
            except paper.PaperBackupError as error:
                require(str(error) == "PAPER_BACKUP_LIMIT", "FIXTURE_BACKUP_RESTART_PREFIX")
            else:
                raise FixtureError("FIXTURE_BACKUP_RESTART_PREFIX")
            require(predecessors == reobserved == [predecessor] and len(readbacks) == 1
                    and audit_attempts == [True],
                    "FIXTURE_BACKUP_RESTART_PREFIX")
            proof = readbacks[0]
            require(proof["kind"] == "BEGIN" and proof["status"] == "COMMITTED"
                    and proof["current_epoch"] == fence and proof["current_state"] == "DRAINING"
                    and proof["event_id"] == old_receipt["event_id"]
                    and proof["result_sha256"] == old_receipt["result_sha256"],
                    "FIXTURE_BACKUP_RESTART_PREFIX")
            committed = pg.settle({"sql": 2, "function": 2, "events": 2, "receipts": 2})
            linked = pg.json("SELECT COALESCE(json_agg(json_build_object('generation',generation,"
                "'prior_state',prior_state,'expected',expected_epoch,'floor',local_epoch_floor,"
                "'fence',fence_epoch)),'[]'::json) FROM public.owner_beta_paper_operations "
                "WHERE intent->>'predecessor_generation'='" + predecessor + "';")
            require(type(linked) is list and len(linked) == 1, "FIXTURE_BACKUP_RESTART_PREFIX")
            successor = linked[0]["generation"]
            ops.journal._uuid(successor)
            require(successor != predecessor and linked[0] == {"generation": successor,
                    "prior_state": "DRAINING", "expected": fence, "floor": fence, "fence": fence + 1}
                    and pg.json("SELECT json_build_array(state,control_epoch) "
                                 "FROM public.owner_beta_paper_control;") == ["OFF", fence + 2],
                    "FIXTURE_BACKUP_RESTART_PREFIX")
            expect_rejection((predecessor, key), "PAPER_BACKUP_PRIOR_OPERATION_UNRESOLVED")
        final = pg.settle({"sql": 2, "function": 2, "events": 2, "receipts": 2})
        require(counter_delta(committed, final) == {"sql": 0, "function": 0, "events": 0, "receipts": 0}
                and pg.receipt(predecessor) == old_receipt
                and not any(run.live.install_root == str(owner.root) for run in paper._RUNS),
                "FIXTURE_BACKUP_RESTART_REJECTION")
        with ops.journal.open_journal(str(owner.root), lock_fd) as reader:
            require(reader.epoch_floor == fence + 2, "FIXTURE_BACKUP_RESTART_PREFIX")
            with contextlib.closing(reader.open(predecessor)) as retained:
                require(retained.revalidate() == "STOPPED" and retained._last == old_head,
                        "FIXTURE_BACKUP_RESTART_REJECTION")
            with contextlib.closing(reader.open(successor)) as current:
                require(current.revalidate() == "OFF_COMMITTED"
                        and current._intent["predecessor_generation"] == predecessor,
                        "FIXTURE_BACKUP_RESTART_PREFIX")
                successor_sha = current._last[1]
        return {"case": BACKUP_RESTART_CASE, "status": "PASS", "original_expired": True,
                "predecessor_readback": "COMMITTED", "linked_begin": 1, "successor": "OFF_COMMITTED",
                "archive": "NOT_PUBLISHED_LIMIT", "old_key_and_mismatch": "REJECTED_WITHOUT_BEGIN",
                "unfinished_replay": "REJECTED_WITHOUT_BEGIN", "original_sha256": old_head[1],
                "successor_sha256": successor_sha}

    def archive_acceptance(self, owner, values):
        """Private prepared-source continuation; not part of hosted ten-case G2.

        The caller must have used real producers and authorized original roots
        to populate this disposable FixtureOwner. No source-ready flag exists.
        """
        require(not os.path.lexists(owner.root / "native-source-owner.json"),
                "FIXTURE_PURPOSE")
        first, evidence = driver.run_archive_acceptance(owner, values, faults)
        published = complete_sets_witness(values["BACKUP_ROOT"])
        require(len(published["sets"]) == 1 and published["latest"] in published["sets"],
                "FIXTURE_ARCHIVE_PUBLICATION")
        before = owner.observer.archive_source_witness()
        _, verification = driver.run_archive_acceptance(owner, values, faults, action="verify")
        require(owner.observer.archive_source_witness() == before
                and complete_sets_witness(values["BACKUP_ROOT"]) == published,
                "FIXTURE_VERIFY_CHANGED_SOURCE")
        second, second_evidence = driver.run_archive_acceptance(owner, values, faults)
        require(second.generation != first.generation and second.archive_sha256 != first.archive_sha256,
                "FIXTURE_ARCHIVE_PREDECESSOR")
        previous = owner.observer.json("SELECT to_json(predecessor_generation) FROM public.owner_beta_paper_operations "
            "WHERE generation='" + second.generation + "';", role="migration_owner")
        require(previous == first.generation, "FIXTURE_ARCHIVE_PREDECESSOR")
        # The production caller must also reuse held Daily repair custody.
        repaired, repair_evidence = driver.run_archive_acceptance(owner, values, faults, action="reconcile")
        require(repaired.status == "RECONCILED_DISABLED", "FIXTURE_ARCHIVE_RECONCILE")
        self.results.append({"case": "ARCHIVE_LIFECYCLE", "status": "PASS", "synthetic": True,
            "owner_acceptance": False, "first": evidence, "verify": verification,
            "second": second_evidence, "reconcile": repair_evidence})
        for kind in driver.ArchiveAcceptance.FAULTS:
            self.archive_fault(owner, values, kind)

    def archive_fault(self, owner, values, kind):
        action = "backup" if kind in {"private-reverify", "publish-fsync", "publish-rename", "publish-latest"} else "verify"
        before = complete_sets_witness(values["BACKUP_ROOT"])
        counters = owner.observer.snapshot()
        source = owner.observer.archive_source_witness() if action == "verify" else None
        observed = driver.ArchiveAcceptance(owner, faults, kind)
        caught = None
        try:
            with observed:
                driver._paper_backup().run_locked(owner.context, owner.lock_fd, action,
                    values["MAX_TOTAL_BYTES"], storage_values=values)
        except (Exception, KeyboardInterrupt) as error:
            caught = error
        require(caught is not None and (observed.fired or kind == "intake-cancel") and not observed.publications,
                "FIXTURE_ARCHIVE_FAULT_UNPROVEN")
        expected_delta = dict.fromkeys(("sql", "function", "events", "receipts"), int(action == "backup"))
        settled = owner.observer.settle({key: counters[key] + value for key, value in expected_delta.items()})
        require(counter_delta(counters, settled) == expected_delta, "FIXTURE_CANCEL_RESUBMISSION")
        if kind in {"ciphertext", "missing-member", "changed-member", "dataset-version", "dataset-status",
                    "economics", "destination-identity", "destination-floor"}:
            require(observed.packages and all(row["zero_dispatch"] for row in observed.records.values()),
                    "FIXTURE_ARCHIVE_FAULT_UNPROVEN")
        after = complete_sets_witness(values["BACKUP_ROOT"])
        require(after["latest"] == before["latest"] and all(after["sets"].get(name) == value
                for name, value in before["sets"].items()), "FIXTURE_ARCHIVE_VALID_SET_CHANGED")
        if kind not in {"publish-fsync", "publish-rename", "publish-latest"}:
            require(not observed.snapshots and after == before, "FIXTURE_ARCHIVE_FAILED_PUBLICATION")
        # Rename/latest uncertainty may leave the publisher's COMPLETE residue;
        # it is never promoted to a success or silently removed by this fixture.
        if action == "verify":
            require(owner.observer.archive_source_witness() == source, "FIXTURE_VERIFY_CHANGED_SOURCE")
        if kind == "intake-cancel" and (observed.intake is None or not observed.intake.proven()):
            self.results.append({"case": "ARCHIVE_INTAKE_CANCEL", "status": "UNKNOWN",
                "reason": "OWNED_INTAKE_CORRELATION_OR_DISPOSITION_UNPROVEN"})
            return
        if kind == "intake-cancel":
            # Retain the actual observer for independent fixture teardown. Its
            # existence is not a production permission or a readiness input.
            if not hasattr(owner, "archive_intakes"):
                owner.archive_intakes = []
            owner.archive_intakes.append(observed.intake)
        self.results.append({"case": "ARCHIVE_" + kind.upper().replace("-", "_"), "status": "PASS",
                             "prior_set_preserved": True, "replayed": False})

    def fault(self, kind):
        with self.fixture_case("STOP") as (context, lock_fd, operation, action, pg, _close_custody):
            live = ops.open_context(context, lock_fd)
            pg.owner.contexts.append(live)
            invocation = live.invoke("begin", action)
            errors = []
            with RowLock(pg) as blocker:
                def enter():
                    try:
                        invocation.__enter__()
                    except BaseException as error:
                        errors.append(error)
                child = threading.Thread(target=enter)
                pg.owner.threads.append(child)
                child.start()
                until = time.monotonic() + pg.owner.deadline.seconds(60)
                while True:
                    require(time.monotonic() < until and child.is_alive(), "FIXTURE_BLOCKED_HELPER")
                    try:
                        cid, pid, ticks, blocked = pg.helper(invocation)
                        if blocked:
                            break
                    except (FixtureError, OSError):
                        pass
                    time.sleep(0.01)
                if kind in ("installation", "floor", "stage"):
                    target = (pg.owner.root / ops.PAPER_DIR / ("installation-id" if kind == "installation" else "epoch-floor")
                              if kind != "stage" else pathlib.Path(os.readlink(
                                  "/proc/self/fd/%d" % operation.borrow_directory_fd())) / "00000000.json")
                    pg.owner.original_custody_uncertain = True
                    saved = pg.owner.root / ("fault-held-" + kind)
                    target.rename(saved)
                    write_new(target, saved.read_bytes(), stat.S_IMODE(saved.stat().st_mode))
                    pg.release_blocked(blocker)
                    child.join(timeout=pg.owner.deadline.seconds(30))
                    require(not child.is_alive(), "FIXTURE_ATTACH_THREAD")
                    require(pg.receipt(operation.generation) is None, "FIXTURE_PRECOMMIT_EVENT")
                    target.unlink()
                    saved.rename(target)
                elif kind in ("result-delete", "result-replace"):
                    with faults.arm(cid, pid, ticks,
                                    faults.directory_identity(os.readlink("/proc/self/fd/%d" % action.borrow_output_fd())),
                                    min(pg.owner.deadline.value - 120_000_000_000,
                                        time.monotonic_ns() + 30_000_000_000)) as observer:
                        pg.owner.tracees.append(observer)
                        pg.release_blocked(blocker)
                        token = observer.wait_for_result_fsync()
                        token.revalidate()
                        committed = pg.receipt(operation.generation)
                        require(committed is not None and committed["result_canonical"].encode("ascii") == token.bytes
                                and committed["result_sha256"] == token.digest, "FIXTURE_COMMIT_NOT_OBSERVED")
                        output = pathlib.Path(os.readlink("/proc/self/fd/%d" % action.borrow_output_fd()))
                        target = output / token.leaf
                        target.unlink()
                        if kind == "result-replace":
                            write_new(target, token.bytes, 0o400)
                            require(target.stat().st_ino != token.identity[1], "FIXTURE_REPLACEMENT_INODE")
                        expected = token.value
                        observer.resume()
                    child.join(timeout=pg.owner.deadline.seconds(30))
                    require(not child.is_alive() and invocation._result is not None,
                            "FIXTURE_POSTCOMMIT_RESULT")
                    result = invocation._result
                    require(result["outcome"] == "COMMIT_UNKNOWN"
                            and result["event_id"] == expected["data"]["operation_event_id"]
                            and result["control_epoch"] == expected["control_epoch"]
                            and result["generation"] == operation.generation
                            and result["artifact_sha256"] == ops._digest(canonical(expected)),
                            "FIXTURE_UNKNOWN_IDENTITY")
                elif kind == "attach-cancel":
                    require(invocation._child is not None, "FIXTURE_ATTACH_CHILD")
                    cancel_owned_attach(invocation._child, pg.owner.deadline)
                    pg.release_blocked(blocker)
                    child.join(timeout=pg.owner.deadline.seconds(30))
                    require(not child.is_alive() and (errors or invocation._preserve),
                            "FIXTURE_FALSE_CLEAN_CANCEL")
                    until = time.monotonic() + pg.owner.deadline.seconds(20)
                    while True:
                        value = pg.owner.docker.object("container", cid)
                        require(value.get("Id") == cid and value.get("Created") == invocation._created
                                and (value.get("Config", {}).get("Labels") or {}).get(ops.CONTAINER_LABEL)
                                == invocation._nonce, "FIXTURE_HELPER_IDENTITY")
                        if value.get("State", {}).get("Running") is False:
                            require(value.get("State", {}).get("Status") == "exited",
                                    "FIXTURE_HELPER_DISPOSITION")
                            break
                        require(time.monotonic() < until, "FIXTURE_HELPER_DISPOSITION_UNPROVEN")
                        time.sleep(0.02)
                elif kind == "cleanup":
                    pg.release_blocked(blocker)
                    child.join(timeout=pg.owner.deadline.seconds(30))
                    require(not child.is_alive() and not errors, "FIXTURE_CLEANUP_BEGIN")
                    imported = invocation.import_dispatch()
                    imported.close()
                    invocation._inspect_and_validate(started=True)
                    pg.owner.command(["rm", cid])
                    try:
                        invocation.close()
                    except ops.PaperOperationsError:
                        pass
                    else:
                        raise FixtureError("FIXTURE_FALSE_REMOVED")
                    require(invocation.cleanup["container"] != "REMOVED", "FIXTURE_FALSE_REMOVED")
                else:
                    raise FixtureError("FIXTURE_FAULT_SELECTOR")
            pg.owner.resolve_helpers()
            pg.absent()
            receipt = pg.receipt(operation.generation)
            if kind in ("installation", "floor", "stage"):
                require(receipt is None, "FIXTURE_PRECOMMIT_RECEIPT")
                pg.settle({"sql": 1, "function": 1, "events": 0, "receipts": 0})
            elif kind in ("result-delete", "result-replace", "cleanup"):
                pg.settle({"sql": 1, "function": 1, "events": 1, "receipts": 1})
                require(receipt is not None, "FIXTURE_COMMITTED_RECEIPT")
                if kind.startswith("result"):
                    require(receipt["result_canonical"].encode("ascii") == token.bytes
                            and receipt["result_sha256"] == token.digest,
                            "FIXTURE_ORIGINAL_SQL_RESULT")
            else:
                counts = pg.snapshot()
                require(counts["sql"] <= 1 and counts["function"] <= 1 and
                        counts["events"] == counts["receipts"] <= 1,
                        "FIXTURE_CANCEL_RESUBMISSION")
            # Original fixed dispatch is retained, even after missing artifact
            # or lost client. Never re-dispatch to discover commit outcome.
            with ops.journal.open_journal(str(pg.owner.root), lock_fd, bootstrap=False) as history_store:
                # Import the exact existing dispatch with freshly opened journal
                # validators. Do not reuse drifted live inputs or submit BEGIN.
                retained_operation = history_store.open(operation.generation)
                retained_action = None
                try:
                    retained_action = retained_operation.open_action(0)
                    imported = retained_action.import_dispatch()
                    dispatch_before = bytes(retained_action._dispatch_bytes)
                    imported.close()
                finally:
                    if retained_action is not None:
                        retained_action.close()
                    retained_operation.close()
                historical = history_store.open_custody(operation.generation, 0)
                try:
                    capability = pathlib.Path(context.release) / "deploy/owner-beta-paper/capability.json"
                    retained = capability.with_name("capability.fixture-held")
                    capability.rename(retained)
                    current = pg.owner.root / "current"
                    require(not current.exists() and not current.is_symlink(), "FIXTURE_CURRENT_UNEXPECTED")
                    current.symlink_to("releases/unusable-fixture-current")
                    before_readback = pg.snapshot()
                    try:
                        reader = ops.open_custody_context(context, lock_fd)
                        pg.owner.contexts.append(reader)
                        readback = reader.invoke("readback", historical)
                        readback.__enter__()
                        value = ops._json(readback.readback_artifact(), ops.journal.MAX_NATIVE_EXACT)
                        require(value["kind"] == "BEGIN" and
                                readback.result["generation"] == operation.generation,
                                "FIXTURE_RECOVERY_GENERATION")
                        if receipt is not None:
                            require(value["status"] == "COMMITTED" and
                                    value["event_id"] == receipt["event_id"] and
                                    value["event_epoch"] == receipt["epoch"] and
                                    value["result_canonical"] == receipt["result_canonical"] and
                                    value["result_sha256"] == receipt["result_sha256"] and
                                    readback.result["outcome"] == "REPLAY",
                                    "FIXTURE_RECOVERY_READBACK")
                        else:
                            require(value["status"] == "ABSENT" and
                                    readback.result["outcome"] == "COMMIT_UNKNOWN" and
                                    value["result_canonical"] is None and value["event_id"] is None,
                                    "FIXTURE_RECOVERY_ABSENT")
                        require(historical._dispatch == dispatch_before,
                                "FIXTURE_RECOVERY_DISPATCH_CHANGED")
                        pg.absent()
                        require(counter_delta(before_readback, pg.snapshot()) ==
                                {"sql": 0, "function": 0, "events": 0, "receipts": 0},
                                "FIXTURE_READBACK_WRITES")
                        # Disposal is supervised after backend/process proof.
                        # An ABSENT readback remains uncertain, never clean rollback.
                    finally:
                        current.unlink()
                        retained.rename(capability)
                finally:
                    historical.close()
            self.results.append({"case": kind.upper().replace("-", "_"), "status": "PASS",
                                 "counters": pg.snapshot(),
                                 "dispatch_sha256": ops._digest(dispatch_before),
                                 "result_sha256": None if receipt is None else receipt["result_sha256"]})

    def pre_paper_install(self):
        """Design 7.3: the first Paper-capable release on a production-shaped pre-Paper root.

        Production --apply stages the real D8 inputs and publishes against the 0065 database.
        It first proves absent-API refusal, then retries from the same source with its real D1 API
        running. The fixture inspects the switched handoff and 0065/0069 guards without starting D8.
        """
        if lifecycle_subset_manifest(self.manifest):
            # paper-lifecycle-fast builds only api-server and paper-scheduler. Its db-migrate is a
            # placeholder, so no migration-owner one-shot can run. Recorded, never a pass.
            self.results.append({"case": PRE_PAPER_CASE, "status": "NOT_RUN",
                                 "reason": "LIFECYCLE_SUBSET_PLACEHOLDER_IMAGES"})
            return
        self.deadline.action_phase()
        owner = FixtureOwner(self.source, self.commit, self.manifest, self.deadline)
        derived, primary, result = {}, None, None
        with owner.extractions:
            try:
                result = self._pre_paper_steps(owner, derived)
            except BaseException as error:
                primary = error
                raise
            finally:
                self.deadline.cleanup_phase()
                try:
                    try:
                        allowed = set(derived.values()) | {manifest_images(self.manifest)["db-migrate"]}
                        dispose_daily_control_oneshots(owner, allowed)
                        owner.dispose_source_api()
                        remove_relabelled_images(owner, derived)
                    finally:
                        owner.dispose(lambda: None)
                except BaseException as cleanup_error:
                    if primary is not None:
                        first = primary.__context__
                        if first is not None and first is not cleanup_error:
                            primary.fixture_first_context = first
                        raise primary.with_traceback(primary.__traceback__) from cleanup_error
                    raise
                self.cleanups.append({"case": PRE_PAPER_CASE, **owner.cleanup,
                                      "relabelled_images": "REMOVED" if derived else "NOT_STARTED"})
        self.results.append(result)

    def _pre_paper_guard(self, owner, step, *, inventory=True):
        lock_fd = owner.lock_fd if inventory else None
        result = run_owned(guard_argv(str(owner.root), self.commit, lock_fd), env=PRE_PAPER_ENV,
                           timeout=self.deadline.seconds(180),
                           pass_fds=(lock_fd,) if inventory else ())
        command_witness(step, result)
        return result

    def _pre_paper_check(self, owner, step):
        with released_release_lock(owner):
            result = run_owned(deploy_check_argv(str(owner.root), self.commit), env=PRE_PAPER_ENV,
                               timeout=self.deadline.seconds(900))
        command_witness(step, result)
        require(result.returncode == 0 and result.stdout.splitlines()[-1:] ==
                [("PRODUCTION_RELEASE_CHECK: PASS commit=" + self.commit + " manifest=installed-v2").encode()],
                "FIXTURE_PRE_PAPER_CHECK")

    def _pre_paper_apply(self, owner, source_commit, source_manifest, simulation, inputs, simulation_inputs):
        root, commit = owner.root, self.commit
        target = root / "releases" / commit
        env = owner._release_env(commit)
        operations_path = self.source / "scripts/ops/lib/owner-beta-simulation-operations.py"
        operations = load("paper_genuine_fixture_simulation_operations", operations_path)
        for revision, manifest in ((source_commit, source_manifest), (commit, self.manifest)):
            operations._api_image_capability(owner.docker, revision, manifest_images(manifest)["api-server"],
                                             self.deadline.seconds(15))
        require(operations._api_ids(owner.docker, self.deadline.seconds(15)) == [],
                "FIXTURE_PRE_PAPER_APPLY_TRANSITION")
        write_new(inputs / "release.env", env)
        write_new(inputs / ops.release.MANIFEST_NAME, self.manifest)
        # The earlier inventory required a published fixture tree. Retire it so the installer
        # itself archives the clean candidate and publishes a fresh tree from these inputs.
        target.rename(inputs / "previous-fixture-target")
        try:
            with released_release_lock(owner):
                result = run_owned(deploy_apply_argv(self.source, root, commit, inputs, simulation_inputs),
                                   env=PRE_PAPER_ENV, timeout=self.deadline.seconds(900))
        finally:
            # The installer uses its production root mode. Restore the fixture's private mode.
            require(faults.directory_identity(root) == owner.root_id, "FIXTURE_ROOT_REPLACED")
            root.chmod(0o700)
        command_witness("apply-simulation-transition", result)
        require_apply_transition_refusal(result)
        require(os.readlink(root / "current") == "releases/" + source_commit
                and operations._api_ids(owner.docker, self.deadline.seconds(15)) == []
                and not any(os.path.lexists(root / name)
                            for name in (operations.START_PENDING, operations.HANDOFF_PENDING))
                and not any((root / "releases").glob(".staging.*")),
                "FIXTURE_PRE_PAPER_APPLY_TRANSITION")
        config_path = target / simulation.CONFIG_DIRECTORY / simulation.CONFIG_NAME
        receipt_dir = target / ".owner-beta-simulation-receipt"
        simulation.verify_config(str(target), commit)
        operations.verify_receipt(str(target), commit)
        require((target / ops.release.MANIFEST_NAME).read_bytes() == self.manifest
                and (target / ".lagrange-release").read_bytes() == (commit + "\n").encode()
                and (target / "deploy/compose/.env").read_bytes() == env
                and config_path.read_bytes() == (simulation_inputs / RELEASE_INPUT_FILES[0]).read_bytes()
                and (receipt_dir / "artifact.json").read_bytes() == (simulation_inputs / RELEASE_INPUT_FILES[1]).read_bytes()
                and (receipt_dir / "COMPLETE").read_bytes() == (simulation_inputs / RELEASE_INPUT_FILES[2]).read_bytes(),
                "FIXTURE_PRE_PAPER_APPLY_PUBLICATION")
        # Confirm the production transition can read the real PostgreSQL history and that it requires
        # maintenance. The apply CLI reports only a generic transition refusal.
        inspected = run_owned(["/usr/bin/python3", str(operations_path),
                               "transition", "--install-root", str(root), "--target-release", str(target),
                               "--commit", commit, "--lock-fd", str(owner.lock_fd), "--action", "inspect"],
                              env=PRE_PAPER_ENV, timeout=self.deadline.seconds(180), pass_fds=(owner.lock_fd,))
        command_witness("inspect-simulation-transition", inspected)
        require(inspected.returncode == 0 and inspected.stdout == b"SIMULATION_OPERATION: MAINTENANCE_REQUIRED\n",
                "FIXTURE_PRE_PAPER_APPLY_TRANSITION")

    def _pre_paper_apply_with_source_api(self, owner, source_commit, source_image, target_image, inputs,
                                         simulation_inputs, derived, refused_target_identity,
                                         refused_target_witness):
        """Retry the same installed target only after custody-checking the refused publication."""
        root, commit = owner.root, self.commit
        target = root / "releases" / commit
        held = inputs / "refused-published-target"
        operations_path = self.source / "scripts/ops/lib/owner-beta-simulation-operations.py"
        operations = load("paper_genuine_fixture_simulation_operations", operations_path)
        require(os.readlink(root / "current") == "releases/" + source_commit and
                faults.directory_identity(target) == refused_target_identity and
                pre_paper_publication_witness(target, commit, simulation_inputs, owner._release_env(commit),
                                              self.manifest) == refused_target_witness and
                not os.path.lexists(held) and
                not any(os.path.lexists(root / name) for name in
                        (operations.START_PENDING, operations.HANDOFF_PENDING)),
                "FIXTURE_PRE_PAPER_APPLY_SUCCESS")
        held_identity = refused_target_identity
        target.rename(held)
        require(not os.path.lexists(target) and faults.directory_identity(held) == held_identity and
                os.readlink(root / "current") == "releases/" + source_commit,
                "FIXTURE_PRE_PAPER_APPLY_SUCCESS")

        generation = owner.start_pre_paper_source_api(source_commit, source_image, target_image, derived)
        require(type(generation) is operations.ApiGeneration, "FIXTURE_PRE_PAPER_SOURCE_API")
        require((inputs / "release.env").read_bytes() == owner._release_env(commit) and
                (inputs / ops.release.MANIFEST_NAME).read_bytes() == self.manifest,
                "FIXTURE_PRE_PAPER_APPLY_SUCCESS")
        try:
            with released_release_lock(owner):
                result = run_owned(deploy_apply_argv(self.source, root, commit, inputs, simulation_inputs),
                                   env=PRE_PAPER_ENV, timeout=self.deadline.seconds(900))
        finally:
            require(faults.directory_identity(root) == owner.root_id, "FIXTURE_ROOT_REPLACED")
            root.chmod(0o700)
        command_witness("apply-source-api-transition", result)
        expected_line = ("PRODUCTION_RELEASE_APPLY: PASS commit=" + commit + " release=" + str(target) +
                         " manifest=installed-v2").encode()
        require(result.returncode == 0 and result.stdout.splitlines()[-1:] == [expected_line] and
                result.stderr == b"" and os.readlink(root / "current") == "releases/" + commit and
                faults.directory_identity(held) == held_identity and
                pre_paper_publication_witness(held, commit, simulation_inputs, owner._release_env(commit),
                                              self.manifest) == refused_target_witness and
                faults.directory_identity(target) != held_identity,
                "FIXTURE_PRE_PAPER_APPLY_SUCCESS")

        handoff = run_simulation_operation(operations, "FIXTURE_PRE_PAPER_HANDOFF",
                                          operations._handoff_read, str(root))
        target_binding = run_simulation_operation(operations, "FIXTURE_PRE_PAPER_HANDOFF",
            operations.binding_from_release, owner.project, str(target), commit)
        source_values, _ = ops.daily.read_context(str(root / "releases" / source_commit), source_commit, "disable")
        target_values, _ = ops.daily.read_context(str(target), commit, "disable")
        generation_fields = [generation.identity, generation.created, generation.started, generation.restarts]
        require(type(handoff) is dict and handoff.get("phase") == "switched" and
                handoff.get("project") == owner.project and handoff.get("source_commit") == source_commit and
                handoff.get("target_commit") == commit and
                handoff.get("source_manifest") == run_simulation_operation(operations,
                    "FIXTURE_PRE_PAPER_HANDOFF", operations._manifest_digest,
                    str(root / "releases" / source_commit)) and
                handoff.get("target_manifest") == run_simulation_operation(operations,
                    "FIXTURE_PRE_PAPER_HANDOFF", operations._manifest_digest, str(target)) and
                handoff.get("source_api_image") == source_image and handoff.get("target_api_image") == target_image and
                handoff.get("source_binding") is None and
                handoff.get("target_binding") == run_simulation_operation(operations,
                    "FIXTURE_PRE_PAPER_HANDOFF", operations._binding_pins, target_binding) and
                handoff.get("api_generation") == generation_fields and handoff.get("worker_generation") is None and
                handoff.get("worker_running") is False and handoff.get("worker_policy") is None and
                handoff.get("target_api_generation") is None and
                not os.path.lexists(root / operations.START_PENDING) and
                source_values["OWNER_BETA_PAPER_MODE"] == "disabled" and
                target_values["OWNER_BETA_PAPER_MODE"] == "disabled" and
                os.path.lexists(root / operations.HANDOFF_PENDING), "FIXTURE_PRE_PAPER_HANDOFF")
        run_simulation_operation(operations, "FIXTURE_PRE_PAPER_HANDOFF", operations.verify_fence,
            owner.docker, owner.project, source_commit, source_image, generation, self.deadline.seconds(15))
        api = owner.docker.object("container", generation.identity, self.deadline.seconds(15))
        api_state = api.get("State") if type(api) is dict else None
        require(type(api_state) is dict and api_state.get("Running") is False and
                api_state.get("Status") == "exited" and api_state.get("ExitCode") == 0 and
                run_simulation_operation(operations, "FIXTURE_PRE_PAPER_HANDOFF", operations._api_ids,
                    owner.docker, self.deadline.seconds(15)) == [generation.identity],
                "FIXTURE_PRE_PAPER_HANDOFF")
        require(owner.observer.sql(MIGRATION_STATE_SQL, "migration_owner") == b"65:65:t" and
                owner.observer.sql(PAPER_CATALOG_SQL, "migration_owner") == b"0" and
                not os.path.lexists(root / ".owner-beta-paper") and
                not os.path.lexists(root / ".owner-beta-paper-reader-floor"), "FIXTURE_PRE_PAPER_DATABASE")
        require_pre_paper_root(root)
        handoff_file_witness = pre_paper_handoff_witness(root, operations, handoff)
        return {
            "apply_with_source_api": "SWITCHED_OFF",
            "source_api": {"ready_before_apply": True, "selected_image": source_image,
                           "container_id": generation.identity, "stopped_exit_code": 0},
            "refused_target_witness_sha256": refused_target_witness["sha256"],
            "handoff": {"phase": handoff["phase"], "source_commit": source_commit,
                        "target_commit": commit, "sha256": ops._digest(canonical(handoff)),
                        "file_identity": handoff_file_witness["identity"],
                        "file_sha256": handoff_file_witness["sha256"]},
            "simulation_start": "NOT_RUN",
            "simulation_start_reason": "PRIVATE_INVESTOR_V3_BYTES_NOT_AVAILABLE_IN_GITHUB",
        }

    def _pre_paper_steps(self, owner, derived):
        commit, root = self.commit, owner.root
        install_root, target = str(root), root / "releases" / self.commit
        # 1. Owned PostgreSQL at 0065 (no Paper catalog object); real stage, publish and verify of C.
        owner.setup(self.policy_root, migration_stop=66)
        pg = owner.observer

        def pre_paper_database():
            require(pg.sql(MIGRATION_STATE_SQL, "migration_owner") == b"65:65:t"
                    and pg.sql(PAPER_CATALOG_SQL, "migration_owner") == b"0", "FIXTURE_PRE_PAPER_DATABASE")
        pre_paper_database()
        # 2. Production-shaped root: canonical Daily and simulation floors, no Paper marker, and a
        # relabelled pre-Paper current release S with its own manifest-selected images.
        simulation = load("paper_genuine_fixture_simulation_release",
                          ROOT / "scripts/ops/lib/owner-beta-simulation-release.py")
        operations = load("paper_genuine_fixture_simulation_operations",
                          ROOT / "scripts/ops/lib/owner-beta-simulation-operations.py")
        write_new(root / ops.daily.install.FLOOR_NAME, ops.daily.install.FLOOR_BYTES)
        write_new(root / simulation.FLOOR_NAME, simulation.FLOOR_BYTES)
        (root / "data").mkdir(mode=0o755)
        source_commit = pre_paper_source_commit(commit)
        images = relabel_source_images(owner, source_commit, derived)
        source_manifest = install_pre_paper_source(owner, source_commit, images)
        os.symlink("releases/" + source_commit, root / "current")
        require_pre_paper_root(root)
        require(history_receipts(root, commit) == {}, "FIXTURE_PRE_PAPER_RECEIPT")
        # 3. Pre-switch: the real guard CLI with deploy's argv, holding this fixture's lock.
        root_info, lock_info = root.lstat(), os.fstat(owner.lock_fd)
        server = owner.docker.object("container", owner.pg)
        expected = {
            "schema": HISTORY_RECEIPT_SCHEMA, "outcome": "LEGACY_OFF", "inventory": "ABSENT",
            "state": "PRE_PAPER", "install_root": install_root,
            "install_root_identity": [root_info.st_dev, root_info.st_ino],
            "release_lock_identity": [lock_info.st_dev, lock_info.st_ino],
            "source_commit": source_commit, "target_commit": commit,
            "source_manifest_sha256": ops._digest(source_manifest),
            "target_manifest_sha256": ops._digest(self.manifest),
            "inventory_sql_sha256": ops._digest((ROOT / HISTORY_INVENTORY_SQL).read_bytes()),
            "migration_max": 65, "migration_lineage_sha256": migration_lineage(ROOT / "migrations", 65),
            "database": {"container_id": owner.pg, "created": server["Created"],
                         "started_at": server["State"]["StartedAt"], "restart_count": server["RestartCount"],
                         "image_id": owner.pg_image, "network_id": owner.network, "database": "lagrange"},
            "p4_index_sha256": None,
            "local": {"paper_dir": False, "reader_floor": False, "lifecycle": "EMPTY_SKELETON",
                      "source_binding": False, "target_binding": False, "evidence_root": "ABSENT",
                      "source_paper_mode": "disabled", "target_paper_mode": "disabled"},
            "boot_id": pathlib.Path("/proc/sys/kernel/random/boot_id").read_text().strip()}
        guard = self._pre_paper_guard(owner, "guard")
        require(guard.returncode == 0 and guard.stdout.splitlines() == [b"PAPER_READER_GUARD: LEGACY_OFF"],
                "FIXTURE_PRE_PAPER_GUARD")
        require_pre_paper_root(root)
        receipts = history_receipts(root, commit)
        require(len(receipts) == 1, "FIXTURE_PRE_PAPER_RECEIPT")
        (receipt_name, receipt_bytes), = receipts.items()
        history_receipt_value(receipt_bytes, expected)
        handoff_record = handoff_witness = None

        def unchanged():
            # No floor, no journal and exactly the receipts recorded at this point.
            require_pre_paper_root(root)
            require(history_receipts(root, commit) == receipts, "FIXTURE_PRE_PAPER_RECEIPT")
            if handoff_witness is not None:
                current_handoff = run_simulation_operation(operations, "FIXTURE_PRE_PAPER_HANDOFF",
                    operations._handoff_read, str(root))
                require(current_handoff == handoff_record and
                        pre_paper_handoff_witness(root, operations, handoff_record) == handoff_witness,
                        "FIXTURE_PRE_PAPER_HANDOFF")

        def unknown(step):
            value = self._pre_paper_guard(owner, step)
            require(value.returncode != 0 and not guard_lines(value.stdout)
                    and value.stderr.splitlines()[:1] == [GUARD_UNKNOWN], "FIXTURE_PRE_PAPER_FAULT")
            unchanged()
        faults = []
        # Without --history-inventory there is still no absence evidence (design 7.4).
        bare = self._pre_paper_guard(owner, "guard-without-inventory", inventory=False)
        require(bare.returncode == 1 and not guard_lines(bare.stdout)
                and bare.stderr.splitlines()[:1] == [GUARD_UNKNOWN], "FIXTURE_PRE_PAPER_FAULT")
        unchanged()
        faults.append("NO_INVENTORY_UNKNOWN")
        # The simulation floor guard still runs after an ABSENT inventory: its target reader missing
        # is refused by that guard, not reported as an unknown history.
        reader = target / "configs/evidence/owner-beta-simulation-reader-v1.json"
        held = root / "pre-paper-held-simulation-reader"
        identity = ops._identity(reader.lstat())
        reader.rename(held)
        try:
            missing = self._pre_paper_guard(owner, "guard-simulation-reader-missing")
        finally:
            held.rename(reader)
        require(ops._identity(reader.lstat()) == identity, "FIXTURE_PRE_PAPER_FAULT")
        # The guard's fixed error boundary always prints one code first; an empty
        # stderr (e.g. a crash before the boundary) must not pass as "refused".
        refused = missing.stderr.splitlines()[:1]
        require(missing.returncode != 0 and not guard_lines(missing.stdout) and len(refused) == 1
                and re.fullmatch(rb"[A-Z][A-Z0-9_]{2,99}", refused[0]) is not None
                and refused != [GUARD_UNKNOWN], "FIXTURE_PRE_PAPER_FAULT")
        unchanged()
        faults.append("SIMULATION_READER_REFUSED")
        # A failed migration row is never absence. Committed so the guard's own session sees it.
        failed = hashlib.sha384(next((target / "migrations").glob("0066_*.up.sql")).read_bytes()).hexdigest()
        pg.sql("INSERT INTO public._sqlx_migrations(version,description,success,checksum,execution_time) "
               "VALUES(66,'fixture-failed',false,decode('" + failed + "','hex'),0);", "migration_owner")
        try:
            unknown("guard-failed-migration-row")
        finally:
            require(pg.sql("WITH removed AS (DELETE FROM public._sqlx_migrations WHERE version=66 AND NOT success "
                           "RETURNING 1) SELECT count(*) FROM removed;", "migration_owner") == b"1",
                    "FIXTURE_PRE_PAPER_DATABASE")
        faults.append("FAILED_MIGRATION_ROW_UNKNOWN")
        # A Paper catalog object without its migration rows is never absence either.
        pg.sql("CREATE TABLE public.owner_beta_paper_control();", "migration_owner")
        try:
            unknown("guard-paper-object-without-row")
        finally:
            pg.sql("DROP TABLE public.owner_beta_paper_control;", "migration_owner")
        faults.append("PAPER_OBJECT_WITHOUT_ROW_UNKNOWN")
        pre_paper_database()
        # 4b. While current is still S, run the inputs tool on C's real D8 receipt pair
        # (P3 extraction custody) and S's synthetic previous configs.
        runner_image = manifest_images(self.manifest)[simulation.BIN]
        pair = tuple(ops.release._extract_image_member(runner_image, simulation._PATHS[key], name, limit, 0o444)
                     for key, name, limit in (("receipt", "artifact.json", simulation.RECEIPT_LIMIT),
                                              ("complete", "COMPLETE", simulation.COMPLETE_BYTES)))
        previous = previous_release_configs(simulation, source_commit, source_manifest, images)
        for directory, data in previous.items():
            path = root / "releases" / source_commit / directory
            path.mkdir(mode=0o750)
            os.chown(path, 0, 10001)
            path.chmod(0o750)
            write_new(path / "config.json", data, 0o440, 0, 10001)
        inputs = root / "release-inputs"
        inputs.mkdir(mode=0o700)
        for name, data in zip(("artifact.json", "COMPLETE"), pair):
            write_new(inputs / name, data, 0o444)
        outputs = {}
        for with_daily in (False, True):
            output = inputs / ("daily" if with_daily else "simulation")
            output.mkdir(mode=0o700)
            output.chmod(0o700)
            ran = run_owned(release_inputs_argv(install_root, commit, source_commit, str(inputs), str(output),
                                                daily=with_daily), env=PRE_PAPER_ENV, timeout=self.deadline.seconds(60))
            command_witness("release-inputs-daily" if with_daily else "release-inputs", ran)
            names = RELEASE_INPUT_FILES + (("daily-config.json",) if with_daily else ())
            require(ran.returncode == 0 and ran.stdout == ("SIMULATION_RELEASE_INPUTS: PASS commit=" + commit
                    + " files=" + ",".join(names) + "\n").encode() and sorted(os.listdir(output)) == sorted(names),
                    "FIXTURE_PRE_PAPER_RELEASE_INPUTS")
            for name, mode in zip(names, (0o600, 0o444, 0o444, 0o600)):
                info = (output / name).lstat()
                require(stat.S_ISREG(info.st_mode) and (info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode),
                        info.st_nlink) == (0, 0, mode, 1), "FIXTURE_PRE_PAPER_RELEASE_INPUTS")
            require(((output / RELEASE_INPUT_FILES[1]).read_bytes(), (output / RELEASE_INPUT_FILES[2]).read_bytes())
                    == pair, "FIXTURE_PRE_PAPER_RELEASE_INPUTS")
            outputs[with_daily] = output
        written = {key: (path / RELEASE_INPUT_FILES[0]).read_bytes() for key, path in outputs.items()}
        old = json.loads(previous[ops.daily.install.CONFIG_DIRECTORY])
        require(written[False] == written[True] and json.loads((outputs[True] / "daily-config.json").read_bytes())
                == {**old, "code_commit": commit} and old["code_commit"] == source_commit,
                "FIXTURE_PRE_PAPER_RELEASE_INPUTS")
        # A previous release that is not current is refused before any input is read.
        other = root / "releases" / ("e" * 40)
        other.mkdir(mode=0o755)
        refused = run_owned(release_inputs_argv(install_root, commit, other.name, str(inputs),
                                                str(inputs / "refused"), daily=False),
                            env=PRE_PAPER_ENV, timeout=self.deadline.seconds(60))
        command_witness("release-inputs-not-current", refused)
        require(refused.returncode == 1 and refused.stdout == b"" and refused.stderr ==
                b"SIMULATION_RELEASE_INPUTS: FAIL code=SIMULATION_INPUTS_PREVIOUS_NOT_CURRENT\n"
                and not os.path.lexists(inputs / "refused"), "FIXTURE_PRE_PAPER_RELEASE_INPUTS")
        other.rmdir()
        for directory in previous:
            shutil.rmtree(root / "releases" / source_commit / directory)
        unchanged()
        self._pre_paper_apply(owner, source_commit, source_manifest, simulation, inputs, outputs[False])
        refused_target_witness = pre_paper_publication_witness(target, commit, outputs[False],
            owner._release_env(commit), self.manifest)
        refused_target_identity = refused_target_witness["identity"]
        apply_receipts = history_receipts(root, commit)
        require(apply_receipts.pop(receipt_name, None) == receipt_bytes and len(apply_receipts) == 1,
                "FIXTURE_PRE_PAPER_RECEIPT")
        apply_receipt_bytes, = apply_receipts.values()
        history_receipt_value(apply_receipt_bytes, expected)
        receipts.update(apply_receipts)
        unchanged()
        pre_paper_database()
        source_api_image = images["api-server"]
        target_api_images = [base for base, image in derived.items() if image == source_api_image]
        require(len(target_api_images) == 1, "FIXTURE_PRE_PAPER_SOURCE_API")
        switched = self._pre_paper_apply_with_source_api(owner, source_commit, source_api_image,
            target_api_images[0], inputs, outputs[False], derived, refused_target_identity,
            refused_target_witness)
        handoff_record = run_simulation_operation(operations, "FIXTURE_PRE_PAPER_HANDOFF",
            operations._handoff_read, str(root))
        handoff_witness = pre_paper_handoff_witness(root, operations, handoff_record)
        require(switched["handoff"]["file_identity"] == handoff_witness["identity"] and
                switched["handoff"]["file_sha256"] == handoff_witness["sha256"],
                "FIXTURE_PRE_PAPER_HANDOFF")
        switched_receipts = history_receipts(root, commit)
        require(all(switched_receipts.get(name) == data for name, data in receipts.items()) and
                len(switched_receipts) == len(receipts) + 1,
                "FIXTURE_PRE_PAPER_RECEIPT")
        added = {name: data for name, data in switched_receipts.items() if name not in receipts}
        require(len(added) == 1, "FIXTURE_PRE_PAPER_RECEIPT")
        switched_receipt_name, switched_receipt_bytes = next(iter(added.items()))
        history_receipt_value(switched_receipt_bytes, expected)
        receipts.update(added)
        unchanged()
        pre_paper_database()
        shutil.rmtree(inputs)
        # The real --apply already switched current to C. Keep that exact published link for
        # both schema guards and backup inventory.
        self._pre_paper_check(owner, "check-pre-paper")
        unchanged()
        # 6. compose-release's migrations 0066..0069, then --check (state B, native INSPECT hosted
        # by C) and the real backup adapter must agree: no floor, empty history.
        owner._apply_migrations(owner.release, range(66, 70))
        require(pg.sql(MIGRATION_STATE_SQL, "migration_owner") == b"69:69:t", "FIXTURE_PRE_PAPER_DATABASE")
        self._pre_paper_check(owner, "check-paper-schema")
        unchanged()
        backup = run_owned(backup_inventory_argv(install_root, str(target), commit, owner.lock_fd),
                           env=PRE_PAPER_ENV, timeout=self.deadline.seconds(300), pass_fds=(owner.lock_fd,))
        command_witness("backup-history-inventory", backup)
        require(backup.returncode == 0 and backup.stdout == b"LEGACY_EMPTY\n", "FIXTURE_PRE_PAPER_BACKUP")
        unchanged()
        require(pg.sql("SELECT count(*) FROM public.owner_beta_paper_control;", "migration_owner") == b"0",
                "FIXTURE_PRE_PAPER_DATABASE")
        return {"case": PRE_PAPER_CASE, "status": "PASS", "guard": "LEGACY_OFF",
                "receipt_sha256": hashlib.sha256(receipt_bytes).hexdigest(),
                "receipt_state": "PRE_PAPER", "receipt_migration_max": 65, "faults": faults,
                "checks": ["PRE_PAPER_0065_PASS", "PAPER_SCHEMA_0069_PASS"], "backup": "LEGACY_EMPTY",
                "relabelled_images": len(derived), "paper_reader_floor": False, "paper_control_rows": 0,
                "apply": "PUBLISHED_TRANSITION_REFUSED",
                "apply_receipt_sha256": hashlib.sha256(apply_receipt_bytes).hexdigest(),
                "apply_with_source_api_receipt_sha256": hashlib.sha256(switched_receipt_bytes).hexdigest(),
                "release_inputs": "PASS", "release_inputs_daily": "PASS", **switched}


_CLOSED_FIXTURE_CODES = frozenset({
    "FIXTURE_BACKUP_RESTART_PREFIX", "FIXTURE_BACKUP_RESTART_REJECTION", "FIXTURE_BACKUP_RESTART_STOPPED",
    "FIXTURE_ARCHIVE_WITNESS", "FIXTURE_ARCHIVE_PUBLICATION",
    "FIXTURE_VERIFY_CHANGED_SOURCE", "FIXTURE_ARCHIVE_PREDECESSOR", "FIXTURE_ARCHIVE_RECONCILE",
    "FIXTURE_ARCHIVE_FAULT_UNPROVEN", "FIXTURE_ARCHIVE_VALID_SET_CHANGED", "FIXTURE_ARCHIVE_FAILED_PUBLICATION",
    "FIXTURE_ARCHIVE", "FIXTURE_ARCHIVE_LINK", "FIXTURE_ARCHIVE_PATH",
    "FIXTURE_ATTACH_CHILD", "FIXTURE_ATTACH_THREAD", "FIXTURE_BACKEND_ALIVE",
    "FIXTURE_BACKEND_CORRELATION", "FIXTURE_BACKEND_DISPOSAL_UNPROVEN", "FIXTURE_BACKEND_IDENTITY",
    "FIXTURE_BEGIN_BACKEND_TIMEOUT", "FIXTURE_BLOCKED_HELPER", "FIXTURE_CANCEL_RESUBMISSION",
    "FIXTURE_CANDIDATE_NOT_CLEAN", "FIXTURE_CLEANUP_BEGIN", "FIXTURE_CLEANUP_RESERVE",
    "FIXTURE_COMMITTED_RECEIPT", "FIXTURE_COMMIT_NOT_OBSERVED", "FIXTURE_CONFIGURE",
    "FIXTURE_CONFIG_ID", "FIXTURE_COUNTER_RESET", "FIXTURE_COUNTER_SETTINGS", "FIXTURE_COUNTER_SHAPE",
    "FIXTURE_COUNTER_UNAVAILABLE", "FIXTURE_COUNTER_UNKNOWN", "FIXTURE_CURRENT_UNEXPECTED",
    "FIXTURE_DEADLINE", "FIXTURE_DOCKER_COMMAND", "FIXTURE_DUPLICATE_OR_READBACK_WRITES",
    "FIXTURE_EXCESS_CALLS", "FIXTURE_FALSE_CLEAN_CANCEL", "FIXTURE_FALSE_REMOVED",
    "FIXTURE_FAULT_SELECTOR", "FIXTURE_FLOOR", "FIXTURE_FOREIGN_CLEANUP", "FIXTURE_FUNCTION_REPLACED",
    "FIXTURE_GIT", "FIXTURE_HELPER_ALIVE", "FIXTURE_HELPER_BINDING_UNKNOWN", "FIXTURE_HELPER_CGROUP",
    "FIXTURE_HELPER_CLEANUP", "FIXTURE_HELPER_DISPOSITION", "FIXTURE_HELPER_DISPOSITION_UNPROVEN",
    "FIXTURE_HELPER_IDENTITY", "FIXTURE_HELPER_PID", "FIXTURE_HELPER_UNKNOWN",
    "FIXTURE_INITIAL_NOT_EMPTY", "FIXTURE_INSTALLED_MANIFEST", "FIXTURE_INTERRUPTED",
    "FIXTURE_LOCK_BACKEND", "FIXTURE_LOCK_CHILD_EXIT", "FIXTURE_LOCK_OUTPUT", "FIXTURE_LOCK_RELEASE",
    "FIXTURE_LOCK_RELEASE_UNPROVEN", "FIXTURE_LOCK_TIMEOUT", "FIXTURE_LOCK_WINDOW",
    "FIXTURE_MAIN_THREAD", "FIXTURE_MANIFEST_CHANGED", "FIXTURE_MANIFEST_NAMESPACE",
    "FIXTURE_MANIFEST_V3", "FIXTURE_MIGRATION_INVENTORY", "FIXTURE_MIGRATION_LOGIN",
    "FIXTURE_NAMESPACE_RESTORE", "FIXTURE_NETWORK_CLEANUP", "FIXTURE_NETWORK_CREATE_UNKNOWN",
    "FIXTURE_NETWORK_ID", "FIXTURE_OBSERVER_PHASE", "FIXTURE_ORIGINAL_SQL_RESULT",
    "FIXTURE_OWNED_THREAD_ALIVE", "FIXTURE_P3_CLEANUP_UNKNOWN", "FIXTURE_P3_CREATE_UNKNOWN",
    "FIXTURE_P3_GENERATION_UNKNOWN", "FIXTURE_P3_IDENTITY_UNKNOWN", "FIXTURE_P3_IMAGE_UNKNOWN",
    "FIXTURE_P3_OBSERVER_ALREADY_OWNED", "FIXTURE_P3_OBSERVER_CHANGED", "FIXTURE_PG_CLEANUP",
    "FIXTURE_PG_CREATE_UNKNOWN", "FIXTURE_PG_ID", "FIXTURE_PG_IDENTITY", "FIXTURE_PG_LOGIN",
    "FIXTURE_PG_NETWORK", "FIXTURE_PG_RESTART", "FIXTURE_PG_START_TIMEOUT", "FIXTURE_POLICY_BIND",
    "FIXTURE_POLICY_TARGET", "FIXTURE_POLICY_TARGET_CHANGED", "FIXTURE_POLICY_UNMOUNT",
    "FIXTURE_POSTCOMMIT_RESULT", "FIXTURE_PRECOMMIT_EVENT", "FIXTURE_PRECOMMIT_RECEIPT",
    "FIXTURE_PRE_PAPER_BACKUP", "FIXTURE_PRE_PAPER_CHECK", "FIXTURE_PRE_PAPER_COMMAND_TIMEOUT",
    "FIXTURE_PRE_PAPER_DATABASE", "FIXTURE_PRE_PAPER_FAULT", "FIXTURE_PRE_PAPER_GUARD",
    "FIXTURE_PRE_PAPER_IMAGE", "FIXTURE_PRE_PAPER_IMAGE_CLEANUP", "FIXTURE_PRE_PAPER_LOCK",
    "FIXTURE_PRE_PAPER_ONESHOT_CLEANUP", "FIXTURE_PRE_PAPER_RECEIPT", "FIXTURE_PRE_PAPER_ROOT",
    "FIXTURE_PRE_PAPER_SOURCE", "FIXTURE_PRE_PAPER_RELEASE_INPUTS",
    "FIXTURE_PRE_PAPER_APPLY_PUBLICATION", "FIXTURE_PRE_PAPER_APPLY_TRANSITION",
    "FIXTURE_PRE_PAPER_APPLY_SUCCESS", "FIXTURE_PRE_PAPER_HANDOFF", "FIXTURE_PRE_PAPER_SOURCE_API",
    "FIXTURE_PRE_PAPER_SOURCE_API_CLEANUP",
    "FIXTURE_PREPARED_WORK", "FIXTURE_PRIVATE_MOUNTS", "FIXTURE_PURPOSE", "FIXTURE_READBACK_WRITES",
    "FIXTURE_RECEIPT_DIGEST", "FIXTURE_RECEIPT_SELECTOR", "FIXTURE_RECEIPT_SHAPE",
    "FIXTURE_RECOVERY_ABSENT", "FIXTURE_RECOVERY_DISPATCH_CHANGED", "FIXTURE_RECOVERY_GENERATION",
    "FIXTURE_RECOVERY_READBACK", "FIXTURE_REPLACEMENT_INODE", "FIXTURE_RESTORE_HELPER_CREATED",
    "FIXTURE_RESTORE_PREDECESSOR", "FIXTURE_RESTORE_SQL", "FIXTURE_ROOT_NAMESPACE",
    "FIXTURE_ROOT_REPLACED", "FIXTURE_ROOT_REQUIRED", "FIXTURE_SEED_SELECTOR", "FIXTURE_SEED_SOURCE",
    "FIXTURE_SOURCE", "FIXTURE_SQL_ROLE", "FIXTURE_SQL_SHAPE", "FIXTURE_STATS_UNPROVEN",
    "FIXTURE_TIMER_ALREADY_OWNED", "FIXTURE_UNKNOWN_IDENTITY", "FIXTURE_UNSHARE",
    "FIXTURE_WATCHER_ALIVE", "FIXTURE_WATCHER_FAILURE",
})
_FIXTURE_DIAGNOSTIC_KINDS = {
    ops.runtime.RuntimeErrorCode: "RuntimeErrorCode",
    ops.PaperOperationsError: "PaperOperationsError",
    FixtureError: "FixtureError", KeyboardInterrupt: "KeyboardInterrupt", OSError: "OSError",
    TimeoutError: "TimeoutError", FileNotFoundError: "OSError", PermissionError: "OSError",
    ProcessLookupError: "OSError", BlockingIOError: "OSError", ConnectionError: "OSError",
    BrokenPipeError: "OSError", ConnectionAbortedError: "OSError",
    ConnectionRefusedError: "OSError", ConnectionResetError: "OSError",
    InterruptedError: "OSError", IsADirectoryError: "OSError", NotADirectoryError: "OSError",
    FileExistsError: "OSError", ValueError: "ValueError", TypeError: "TypeError",
    AssertionError: "AssertionError", Exception: "UnexpectedError",
}
_DIAGNOSTIC_DEPENDENCIES = (
    (ops, "PAPER_OPS"), (ops.runtime, "PAPER_RUNTIME"), (ops.daily, "DAILY_OPS"),
    (ops.daily.runtime, "DAILY_RUNTIME"), (ops.daily.install, "DAILY_INSTALL"),
    (ops.daily.policy, "POLICY"), (ops.release, "RELEASE"), (ops.journal, "JOURNAL"),
)

_FIXTURE_OPS_SOURCE = pathlib.Path(ops.__file__).resolve()
_FIXTURE_BACKUP_OPS_MODULE = "owner_beta_paper_backup_operations"
_FIXTURE_READBACK_STATUSES = frozenset(("COMMITTED", "ABSENT", "UNPROVEN", "RESET"))
_FIXTURE_READBACK_VERBS = {"RECONCILE": "reconcile", "TRANSITION": "off"}
_FIXTURE_OFF_COMMIT_READBACK_CODE = vars(ops._OffCommit)["readback"].__code__

_FIXTURE_TREE_DIRS = (pathlib.Path(__file__).resolve().parent,
                      pathlib.Path(__file__).resolve().parent / "lib")
_INNERMOST_FILE = re.compile(r"^[A-Za-z0-9_.-]{1,80}\.py$")


def _fixture_off_commit_source(frame):
    """Recognize the direct ops import or its same-source backup-module import."""
    try:
        if type(frame.f_globals) is not dict or type(frame.f_code) is not types.CodeType:
            return None
        if frame.f_globals is vars(ops) and frame.f_code is _FIXTURE_OFF_COMMIT_READBACK_CODE:
            return ops
        module = sys.modules.get(_FIXTURE_BACKUP_OPS_MODULE)
        source = vars(module).get("__file__") if type(module) is types.ModuleType else None
        if (type(module) is not types.ModuleType or vars(module) is not frame.f_globals
                or type(source) is not str or pathlib.Path(source).resolve() != _FIXTURE_OPS_SOURCE):
            return None
        cls = vars(module).get("_OffCommit")
        method = vars(cls).get("readback") if type(cls) is type else None
        return module if (type(method) is types.FunctionType and cls.__module__ == module.__name__
                          and frame.f_code is method.__code__
                          and pathlib.Path(frame.f_code.co_filename).resolve() == _FIXTURE_OPS_SOURCE) else None
    except (OSError, RuntimeError, TypeError, ValueError):
        return None


def _fixture_off_commit_readback(frame):
    local, value = frame.f_locals, frame.f_locals.get("value")
    if type(value) is not dict:
        return None
    request = local.get("request")
    kind = request.get("kind") if type(request) is dict else None
    verb = _FIXTURE_READBACK_VERBS.get(kind, "UNKNOWN") if type(kind) is str else "UNKNOWN"
    status = value.get("status")
    status = status if type(status) is str and status in _FIXTURE_READBACK_STATUSES else "UNKNOWN"
    state, epoch = local.get("current_state"), local.get("current_epoch")
    return {"status": status, "verb": verb,
            "state_match": type(state) is str and type(value.get("current_state")) is str
            and value["current_state"] == state,
            "epoch_match": type(epoch) is int and type(value.get("current_epoch")) is int
            and value["current_epoch"] == epoch}


def _dependency_literal_codes():
    # Freeze trusted module bytecode literals once at import. Runtime messages,
    # caller values and later monkeypatches cannot expand diagnostic vocabulary.
    codes, pending, seen = {"PAPER_OPS_INVALID"}, [], set()
    for module, _label in _DIAGNOSTIC_DEPENDENCIES:
        for value in vars(module).values():
            members = vars(value).values() if type(value) is type and value.__module__ == module.__name__ else (value,)
            for member in members:
                if type(member) in (staticmethod, classmethod):
                    member = member.__func__
                if type(member) is types.FunctionType and member.__module__ == module.__name__:
                    pending.append(member.__code__)
                elif type(member) is property:
                    pending.extend(fn.__code__ for fn in (member.fget, member.fset)
                                   if type(fn) is types.FunctionType and fn.__module__ == module.__name__)
    while pending:
        code = pending.pop()
        if code in seen:
            continue
        seen.add(code)
        for value in code.co_consts:
            if type(value) is types.CodeType:
                pending.append(value)
            elif type(value) is str and re.fullmatch(
                    r"(?:PAPER_OPS_|DAILY_|PAPER_RELEASE_|JOURNAL_|SERVICE_STATUS_|POLICY_)[A-Z0-9_]{1,100}", value):
                codes.add(value)
    return frozenset(codes)


_CLOSED_DEPENDENCY_CODES = _dependency_literal_codes()
_FIXTURE_DEPENDENCY_KINDS = {
    ops.runtime.RuntimeErrorCode: "RuntimeErrorCode",
    ops.PaperOperationsError: "PaperOperationsError",
    ops.daily.runtime.RuntimeErrorCode: "DailyRuntimeErrorCode",
    ops.daily.install.DailyInstallError: "DailyInstallError",
    ops.daily.policy.PolicyError: "PolicyError", ops.release.PaperReleaseError: "PaperReleaseError",
    ops.journal.JournalError: "JournalError",
}
for _kind, _label in _FIXTURE_DEPENDENCY_KINDS.items():
    _FIXTURE_DIAGNOSTIC_KINDS.setdefault(_kind, _label)

_FIXTURE_DIAGNOSTIC_ERRNO_TYPES = frozenset((
    OSError, TimeoutError, FileNotFoundError, PermissionError, ProcessLookupError, BlockingIOError,
    ConnectionError, BrokenPipeError, ConnectionAbortedError, ConnectionRefusedError,
    ConnectionResetError, InterruptedError, IsADirectoryError, NotADirectoryError, FileExistsError,
))
_FIXTURE_DIAGNOSTIC_FAULT_CASES = {
    "installation": "INSTALLATION", "floor": "FLOOR", "stage": "STAGE",
    "result-delete": "RESULT_DELETE", "result-replace": "RESULT_REPLACE",
    "attach-cancel": "ATTACH_CANCEL", "cleanup": "CLEANUP",
}
_FIXTURE_POSITIVE_CODE = FixtureRunner.positive.__code__
_FIXTURE_FAULT_CODE = FixtureRunner.fault.__code__
_FIXTURE_RESTORE_NEGATIVE_CODE = FixtureRunner.restore_negative.__code__
_FIXTURE_BACKUP_RESTART_CODES = (FixtureRunner.backup_restart.__code__, FixtureRunner._backup_restart_steps.__code__)
_FIXTURE_PRE_PAPER_CODES = (FixtureRunner.pre_paper_install.__code__, FixtureRunner._pre_paper_steps.__code__,
    FixtureRunner._pre_paper_apply_with_source_api.__code__, FixtureOwner.start_pre_paper_source_api.__code__,
    FixtureOwner.dispose_source_api.__code__, pre_paper_publication_witness.__code__,
    pre_paper_handoff_witness.__code__, pre_paper_api_role_passwords.__code__,
    wait_for_pre_paper_api_ready.__code__,
    run_simulation_operation.__code__)


def _unknown_fixture_diagnostic():
    return {"kind": "UNKNOWN", "code": "UNKNOWN", "line": 0,
            "caller_line": 0, "case": "UNKNOWN", "errno": 0}


def _fixture_failure_positions(traceback):
    line = caller_line = 0
    case = "UNKNOWN"
    frames = 0
    while traceback is not None and frames < 64:
        frame = traceback.tb_frame
        code = frame.f_code
        if code.co_filename == __file__:
            current_line = traceback.tb_lineno
            if type(current_line) is not int or not 0 < current_line <= 1_000_000:
                return 0, 0, "UNKNOWN"
            caller_line, line = line, current_line
            if code is _FIXTURE_POSITIVE_CODE:
                purpose = frame.f_locals.get("purpose")
                case = purpose if type(purpose) is str and purpose in ("STOP", "BACKUP") else "UNKNOWN"
            elif code is _FIXTURE_FAULT_CODE:
                kind = frame.f_locals.get("kind")
                case = (_FIXTURE_DIAGNOSTIC_FAULT_CASES.get(kind, "UNKNOWN")
                        if type(kind) is str else "UNKNOWN")
            elif code is _FIXTURE_RESTORE_NEGATIVE_CODE:
                case = "RESTORE_UNVERIFIED_ARCHIVE_REJECTED"
            elif code in _FIXTURE_BACKUP_RESTART_CODES:
                case = BACKUP_RESTART_CASE
            elif any(code is item for item in _FIXTURE_PRE_PAPER_CODES):
                case = PRE_PAPER_CASE
        traceback = traceback.tb_next
        frames += 1
    if traceback is not None:
        return 0, 0, "UNKNOWN"
    return line, caller_line, case


def _fixture_failure_diagnostic(error):
    try:
        kind = _FIXTURE_DIAGNOSTIC_KINDS.get(type(error))
        if kind is None:
            return _unknown_fixture_diagnostic()
        code = "UNKNOWN"
        if type(error) is FixtureError:
            args = error.args
            if (type(args) is tuple and len(args) == 1 and type(args[0]) is str
                    and args[0] in _CLOSED_FIXTURE_CODES):
                code = args[0]
        elif type(error) in _FIXTURE_DEPENDENCY_KINDS:
            args = error.args
            if (type(args) is tuple and len(args) == 1 and type(args[0]) is str
                    and args[0] in _CLOSED_DEPENDENCY_CODES):
                code = args[0]
        line, caller_line, case = _fixture_failure_positions(error.__traceback__)
        number = 0
        if type(error) in _FIXTURE_DIAGNOSTIC_ERRNO_TYPES:
            candidate = error.errno
            if type(candidate) is int and 0 <= candidate <= 4095:
                number = candidate
        return {"kind": kind, "code": code, "line": line,
                "caller_line": caller_line, "case": case, "errno": number}
    except BaseException:
        return _unknown_fixture_diagnostic()


_AUDIT_IDENTIFIER = re.compile(r"[a-z0-9_]{1,63}\Z")


def _audit_stream_position(frame):
    # Where the audit stream decoder stopped. Source identifiers and closed classes only: never
    # a key, a row value or a length.
    values = frame.f_locals

    def identifier(name):
        value = values.get(name)
        return value if type(value) is str and _AUDIT_IDENTIFIER.match(value) else "UNKNOWN"

    key, previous = values.get("key"), values.get("previous")
    if type(key) is not str:
        key_class = ("NULL" if "key" in values else "ABSENT") if key is None else "NOT_TEXT"
    else:
        key_class = "TEXT" if key.isascii() and key.isprintable() else "NOT_PRINTABLE_ASCII"
    order = "FIRST" if previous is None else "UNKNOWN"
    if type(key) is str and type(previous) is str:
        before, current = previous.encode("utf-8", "replace"), key.encode("utf-8", "replace")
        order = "ASCENDING" if before < current else "EQUAL" if before == current else "DESCENDING"
    return {"state": identifier("state"), "tag": identifier("tag"), "group": identifier("g"),
            "table": identifier("t"), "key": key_class, "order": order}


def _fixture_dependency_details(traceback):
    module, line, calls, seen, innermost, audit = "UNKNOWN", 0, [], set(), None, None
    off_commit_readback = None

    def collect(value, native_type=ops._NativeInvocation):
        if (type(value) not in (native_type, ops._PaperContext, FixtureOwner)
                or len(calls) >= 16 or len(seen) >= 64 or id(value) in seen):
            return
        seen.add(id(value))
        if type(value) is native_type:
            calls.append(value)
        elif type(value) is ops._PaperContext:
            current = getattr(value, "_invocations", ())
            if type(current) is set and len(current) <= 16:
                for call in current:
                    collect(call)
        elif type(value) is FixtureOwner:
            contexts = getattr(value, "contexts", ())
            if type(contexts) is list and len(contexts) <= 16:
                for context in contexts:
                    collect(context)

    for _index in range(64):
        if traceback is None:
            break
        frame = traceback.tb_frame
        readback_module = _fixture_off_commit_source(frame)
        if readback_module is not None:
            candidate = traceback.tb_lineno
            if type(candidate) is int and 0 < candidate <= 1_000_000:
                module, line = "PAPER_OPS", candidate
            local = frame.f_locals
            owner_type = vars(readback_module).get("_OffCommit")
            owner = local.get("self") if type(local) is dict else None
            if type(owner_type) is type and type(owner) is owner_type and type(local.get("value")) is dict:
                native_type = vars(readback_module).get("_NativeInvocation")
                if type(native_type) is type:
                    collect(owner.call, native_type)
                    collect(local.get("read"), native_type)
                off_commit_readback = _fixture_off_commit_readback(frame)
        if frame.f_code.co_name not in ("_fail", "_require", "fail", "require"):
            try:
                located = pathlib.Path(frame.f_code.co_filename).resolve()
            except (OSError, RuntimeError, ValueError):
                located = None
            candidate = traceback.tb_lineno
            if (located is not None and located.parent in _FIXTURE_TREE_DIRS
                    and _INNERMOST_FILE.fullmatch(located.name) is not None
                    and type(candidate) is int and 0 < candidate <= 1_000_000):
                innermost = {"file": located.name, "line": candidate}
                if located.name == "owner-beta-paper-audit.py" and frame.f_code.co_name == "decode":
                    try:
                        audit = _audit_stream_position(frame)
                    except Exception:
                        audit = None
        for dependency, label in _DIAGNOSTIC_DEPENDENCIES:
            if frame.f_code.co_filename == dependency.__file__ and frame.f_code.co_name not in (
                    "_fail", "_require", "fail", "require"):
                candidate = traceback.tb_lineno
                if type(candidate) is int and 0 < candidate <= 1_000_000:
                    module, line = label, candidate
        if frame.f_code.co_filename in (__file__, ops.__file__):
            for name in ("self", "owner", "live", "invocation", "call"):
                collect(frame.f_locals.get(name))
        traceback = traceback.tb_next
    native, unvalidated = [], []
    for call in calls:
        verb = getattr(call, "verb", None)
        try:
            result = ops._validate_result(call._stdout, verb, call._exit)
        except Exception:
            stdout = getattr(call, "_stdout", None)
            if type(stdout) is not bytes or len(stdout) > ops.MAX_RESULT + 1:
                continue
            stderr, exit_status = getattr(call, "_stderr", None), getattr(call, "_exit", None)
            summary = {"verb": verb if type(verb) is str and verb in ops.RESULT_SCHEMAS else "UNKNOWN",
                       "exit_status": exit_status if type(exit_status) is int and 0 <= exit_status <= 255 else "UNKNOWN",
                       "stdout_bytes": len(stdout), "stderr_bytes": len(stderr) if type(stderr) is bytes else "UNKNOWN"}
            try:
                raw = json.loads(stdout.decode("ascii"))
            except (ValueError, UnicodeError, RecursionError):
                raw = {}
            for name, allowed in (("schema", set(ops.RESULT_SCHEMAS.values())),
                                  ("outcome", ops.OUTCOMES), ("error_code", ops.ERRORS)):
                value = raw.get(name) if type(raw) is dict else None
                summary["observed_" + name] = value if type(value) is str and value in allowed else "UNKNOWN"
            if summary not in unvalidated:
                unvalidated.append(summary)
            continue
        fact = {name: result[name] for name in ("verb", "outcome", "error_code")}
        if fact not in native:
            native.append(fact)
    details = {"dependency_module": module, "dependency_line": line, "native": native,
               "unvalidated_output": unvalidated, "innermost": innermost}
    if off_commit_readback is not None:
        details["off_commit_readback"] = off_commit_readback
    if audit is not None:
        details["audit"] = audit
    return details


def _fixture_failure_details(error):
    # Descriptive only: bounded causes and revalidated original result bytes.
    # UUIDs, paths, messages, raw output and credentials are never projected.
    # Both links are followed, the explicit cause first. "raise primary from cleanup_error"
    # sets a cause and would otherwise hide the primary's own context, which is the first error.
    records, seen, pending = [], set(), [error]
    try:
        while pending and len(records) < 8:
            current = pending.pop(0)
            if not isinstance(current, BaseException) or id(current) in seen:
                continue
            seen.add(id(current))
            record = _fixture_failure_diagnostic(current)
            record.update(_fixture_dependency_details(current.__traceback__))
            records.append(record)
            pending.extend((current.__cause__, current.__context__,
                            getattr(current, "fixture_first_context", None)))
        return {"causes": records,
                "truncated": any(isinstance(item, BaseException) and id(item) not in seen
                                 for item in pending)}
    except Exception:
        return {"causes": records, "truncated": True}


def success_result(commit, manifest, runner, *, dev_code_overlay=False):
    result = {"status": "PASS", "commit": commit, "manifest_sha256": ops._digest(manifest),
              "cases": runner.results, "cleanup": runner.cleanups, "runtime": "GENUINE_SELECTED_IMAGE",
              "production": "CLOSED", "enable": False}
    if dev_code_overlay is True:
        # An iteration aid against an earlier commit's images; never a genuine pass.
        result.update(status="DEV_PASS", runtime="DEV_CODE_OVERLAY")
    return result


def run(source_root, commit, manifest_path, deadline_ns, *, dev_code_overlay=False):
    require(os.geteuid() == 0 and os.getegid() == 0 and sys.platform == "linux", "FIXTURE_ROOT_REQUIRED")
    if not pathlib.Path("/usr/bin/docker").is_file() or not pathlib.Path("/var/run/docker.sock").exists():
        raise FixtureNotRun("HOSTED_ROOT_PG_FIXTURE_REQUIRED")
    source, manifest, deadline = validated_inputs(source_root, commit, manifest_path, deadline_ns,
                                                  dev_code_overlay=dev_code_overlay)
    timing_report = _install_dev_timing() if dev_code_overlay is True else None
    try:
        with bounded_run(deadline):
            policy_root = pathlib.Path(tempfile.mkdtemp(prefix="lagrange-paper-fixture-policy-", dir="/var/tmp"))
            policy_root.chmod(0o700)
            (policy_root / "service-status").mkdir(mode=0o700)
            try:
                with PrivatePolicy(policy_root):
                    runner = FixtureRunner(source, commit, manifest, deadline, policy_root)
                    runner.positive("STOP")
                    runner.positive("BACKUP")
                    runner.restore_negative()
                    for case in ("installation", "floor", "stage", "result-delete",
                                 "result-replace", "attach-cancel", "cleanup"):
                        runner.fault(case)
                    runner.pre_paper_install()
                    runner.backup_restart()
                    return success_result(commit, manifest, runner, dev_code_overlay=dev_code_overlay)
            finally:
                deadline.cleanup_phase()
                shutil.rmtree(policy_root)
    finally:
        if timing_report is not None:
            print("PAPER_FIXTURE_DEV_TIMING=" + canonical(timing_report()).decode(), flush=True)


@contextlib.contextmanager
def termination_cleanup():
    previous = {}
    def interrupted(signum, _frame):
        # One cleanup attempt owns all children; a second signal cannot turn a
        # still-owned helper into clean disposal by interrupting that attempt.
        for number in previous:
            signal.signal(number, signal.SIG_IGN)
        raise FixtureError("FIXTURE_INTERRUPTED")
    try:
        for number in (signal.SIGTERM, signal.SIGHUP):
            previous[number] = signal.signal(number, interrupted)
        yield
    finally:
        for number, handler in previous.items():
            signal.signal(number, handler)


def main(argv=None):
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--source-root", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--manifest-file", required=True)
    parser.add_argument("--deadline-monotonic-ns", required=True, type=int)
    parser.add_argument("--dev-code-overlay", action="store_true")
    if "--dev-code-overlay" in (sys.argv[1:] if argv is None else argv):
        # First line of every outcome, including NOT_RUN, so no log reads as a genuine run.
        print(DEV_CODE_OVERLAY_MODE, flush=True)
    if os.geteuid() != 0:
        print("PAPER_GENUINE_FIXTURE_NOT_RUN:HOSTED_ROOT_PG_FIXTURE_REQUIRED")
        return 77
    args = parser.parse_args(argv)
    try:
        with termination_cleanup():
            if args.dev_code_overlay:
                result = run(args.source_root, args.commit, args.manifest_file, args.deadline_monotonic_ns,
                             dev_code_overlay=True)
            else:
                result = run(args.source_root, args.commit, args.manifest_file, args.deadline_monotonic_ns)
    except FixtureNotRun:
        print("PAPER_GENUINE_FIXTURE_NOT_RUN:HOSTED_ROOT_PG_FIXTURE_REQUIRED")
        return 77
    except (Exception, KeyboardInterrupt) as error:
        print("PAPER_GENUINE_FIXTURE_FAILED:OWNED_CASE_UNPROVEN")
        print("PAPER_GENUINE_FIXTURE_DIAGNOSTIC=" + canonical(
            _fixture_failure_diagnostic(error)).decode())
        print("PAPER_GENUINE_FIXTURE_DETAILS=" + canonical(_fixture_failure_details(error)).decode())
        return 1
    print(canonical(result).decode())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
