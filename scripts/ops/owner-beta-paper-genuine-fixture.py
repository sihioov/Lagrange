#!/usr/bin/env python3
"""Disposable selected-image Root/PG fixture. Import has no privileged effects."""
from __future__ import annotations

import argparse
import contextlib
import ctypes
import fcntl
import hashlib
import importlib.util
import json
import os
import pathlib
import re
import selectors
import shutil
import signal
import socket
import stat
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
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


class FixtureError(Exception):
    """Only closed diagnostic codes cross the fixture boundary."""


class FixtureNotRun(FixtureError):
    """Required hosted resources are absent; this is never a pass receipt."""


def require(value, code):
    if not value:
        raise FixtureError(code)


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


def validated_inputs(source_root, commit, manifest_path, deadline_ns):
    deadline = Deadline(deadline_ns)
    source = pathlib.Path(source_root).resolve(strict=True)
    require(source == ROOT and re.fullmatch("[0-9a-f]{40}", commit or "")
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
 'oid','public.begin_owner_beta_paper_operation_v1(bigint,jsonb,text)'::regprocedure::oid,
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
 'pid',pid,'start',backend_start::text,'addr',client_addr::text,'port',client_port,
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
        value = self.json("SELECT (SELECT json_build_object('event_id',fence_event_id,"
                          "'epoch',fence_epoch,'result_canonical',result_canonical,"
                          "'result_sha256',result_sha256) FROM public.owner_beta_paper_operations "
                          "WHERE generation='" + generation + "'::uuid);")
        if value is not None:
            require(type(value) is dict and set(value) ==
                    {"event_id", "epoch", "result_canonical", "result_sha256"}, "FIXTURE_RECEIPT_SHAPE")
            require(type(value["result_canonical"]) is str and
                    ops._digest(value["result_canonical"].encode("ascii")) == value["result_sha256"],
                    "FIXTURE_RECEIPT_DIGEST")
        return value

    def settle(self, expected, *, cap=10):
        """Wait for backend exit and real collector publication; never fill gaps."""
        until = time.monotonic() + self.owner.deadline.seconds(cap)
        while True:
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
            require(time.monotonic() < until, "FIXTURE_STATS_UNPROVEN")
            time.sleep(0.02)

    def barrier(self, phase, resource, result):
        context = resource._context if type(resource) is ops._NativeInvocation else resource
        if context not in self.owner.contexts:
            self.owner.contexts.append(context)
        if phase == "first":
            if type(resource) is ops._NativeInvocation and resource.verb == "begin" and not hasattr(self, "primary_invocation"):
                self.primary_invocation = resource
            return  # Retain actual objects before probe/dispatch, no fake result.
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
        self.observer = None
        self.contexts, self.tracees, self.threads, self.blockers = [], [], [], []
        self.helpers, self.helper_bindings = {}, {}
        self.cancel_event = threading.Event()
        self.original_custody_uncertain = False
        self.cleanup = {"containers": "PENDING", "backends": "PENDING", "inputs": "RETAINED",
                        "extractions": "NOT_STARTED"}
        self.extractions = P3ExtractionCustody(self)

    @staticmethod
    def argv(args):
        return ["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", *args]

    def command(self, args, payload=None, cap=30, cleanup=False, allow_failure=False):
        result = subprocess.run(self.argv(args), input=payload, capture_output=True,
                                env=ops._docker_client_environment(), check=False,
                                timeout=self.deadline.seconds(cap, cleanup=cleanup))
        require(allow_failure or result.returncode == 0, "FIXTURE_DOCKER_COMMAND")
        return result

    def output(self, args, **kwargs):
        return self.command(args, **kwargs).stdout.decode("ascii").strip()

    def setup(self, policy_root):
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
        with tarfile.open(archive) as stream:
            for member in stream:
                require(member.isfile() or member.isdir(), "FIXTURE_ARCHIVE_LINK")
                path = pathlib.PurePosixPath(member.name)
                require(not path.is_absolute() and ".." not in path.parts, "FIXTURE_ARCHIVE_PATH")
                stream.extract(member, staging, filter="data")
        for parent, directories, names in os.walk(staging):
            os.chmod(parent, 0o755)
            for name in names:
                p = pathlib.Path(parent) / name
                os.chmod(p, stat.S_IMODE(p.stat().st_mode) & ~0o022)
        secret_root = self.root / "secrets"
        secret_root.mkdir(mode=0o700)
        service = secret_root / "db-migrate"
        service.mkdir(mode=0o750)
        os.chown(service, 999, 999)
        password = uuid.uuid4().hex + uuid.uuid4().hex
        write_new(service / "db_migration_owner_password", password.encode(), 0o400, 999, 999)
        write_new(self.root / "postgres-secret", (uuid.uuid4().hex + uuid.uuid4().hex).encode(), 0o400, 999, 999)
        values = {"LAGRANGE_CODE_COMMIT": self.commit, "LAGRANGE_DATA_DIR": str(self.root / "data"),
                  "LAGRANGE_ARTIFACTS_DIR": str(self.root / "artifacts"),
                  "LAGRANGE_RUNTIME_SECRET_DIR": str(secret_root), "POSTGRES_DB": "lagrange",
                  "OWNER_BETA_DAILY_MODE": "disabled", "OWNER_BETA_PAPER_MODE": "disabled"}
        write_new(staging / "deploy/compose/.env",
                  "".join(k + "=" + v + "\n" for k, v in values.items()).encode())
        write_new(staging / ops.release.MANIFEST_NAME, self.manifest)
        require(ops.daily.policy.validate_manifest(str(staging / ops.release.MANIFEST_NAME), self.commit)
                == hashlib.sha256(self.manifest).hexdigest(), "FIXTURE_INSTALLED_MANIFEST")
        write_new(staging / ".lagrange-release", (self.commit + "\n").encode())
        manifest_sha = ops._digest(self.manifest)
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
                               "--mount", "type=bind,source=" + str(data) + ",target=/var/lib/postgresql/data",
                               ops.POSTGRES_IMAGE, "postgres", "-c", "shared_preload_libraries=pg_stat_statements",
                               "-c", "pg_stat_statements.track=all", "-c", "track_functions=pl"])
        require(re.fullmatch("[0-9a-f]{64}", self.pg), "FIXTURE_PG_ID")
        self.observer = PgObserver(self)
        until = time.monotonic() + self.deadline.seconds(90)
        while True:
            try:
                require(self.observer.sql("SELECT current_user||':'||session_user;") ==
                        b"lagrange:lagrange", "FIXTURE_PG_LOGIN")
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
        migrations = sorted((self.release / "migrations").glob("*.up.sql"))
        require(len(migrations) == 68, "FIXTURE_MIGRATION_INVENTORY")
        for path in migrations:
            script = path.read_text()
            if not script.startswith("-- no-transaction\n"):
                script = "BEGIN;\n" + script + "\nCOMMIT;"
            pg.sql(script, "migration_owner", cap=120)
            pg.sql("INSERT INTO public._sqlx_migrations(version,description,success,checksum,execution_time) "
                   "VALUES(%d,'fixture',true,decode('%s','hex'),0);" %
                   (int(path.name[:4]), hashlib.sha384(path.read_bytes()).hexdigest()), "migration_owner")
        pg.sql("CREATE EXTENSION pg_stat_statements;")
        images = ops.daily.read_manifest(str(self.release), self.commit)
        actual_values, overlays = ops.daily.read_context(str(self.release), self.commit, "disable")
        self.context = ops.daily.Context(str(self.release), self.commit, images, actual_values, overlays)
        with ops.open_context(self.context, self.lock_fd) as live:
            self.contexts.append(live)
            with live.probe_history() as inventory:
                require(inventory._configured is False and inventory._history is False,
                        "FIXTURE_INITIAL_NOT_EMPTY")

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
                except FixtureError:
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
        self.resolve_helpers()
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
                    and mounts.get("/var/lib/postgresql/data", {}).get("Source") == str(self.root / "pgdata"),
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
        close_custody()
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
            stages.update(context._retained_stages)
            stages.update(entry["stage"] for entry in context._input_stages.values())
            self.close_field(context, "_manifest_fd")
            if context.verified is not None:
                context.verified.close()
            context._release_tree.close()
            context.tree.close()
            self.close_field(context, "lock_fd")
        for stage in stages:
            if not stage.closed:
                stage.close(remove=True)
        require(faults.directory_identity(self.root) == self.root_id, "FIXTURE_ROOT_REPLACED")
        shutil.rmtree(self.root)
        if self.lock_fd is not None:
            os.close(self.lock_fd)
            self.lock_fd = None
        self.cleanup["inputs"] = "REMOVED_AFTER_DISPOSITION"


class FixtureRunner:
    def __init__(self, source, commit, manifest, deadline, policy_root):
        self.source, self.commit, self.manifest = source, commit, manifest
        self.deadline, self.policy_root = deadline, policy_root
        self.results, self.cleanups, self.completed_begin = [], [], None

    @contextlib.contextmanager
    def fixture_case(self, purpose):
        require(purpose in ("STOP", "BACKUP", "RESTORE"), "FIXTURE_PURPOSE")
        self.deadline.action_phase()
        owner = FixtureOwner(self.source, self.commit, self.manifest, self.deadline)
        with owner.extractions:
            journal_manager = operation = action = None
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
                yield owner.context, owner.lock_fd, operation, action, owner.observer
            finally:
                self.deadline.cleanup_phase()
                # Resolve actual helper/backend resources before releasing journal
                # originals, secrets or the held release open description.
                def close_custody():
                    if action is not None:
                        action.close()
                    if operation is not None:
                        operation.close()
                    if journal_manager is not None:
                        try:
                            journal_manager.__exit__(None, None, None)
                        except ops.journal.JournalError as error:
                            if not owner.original_custody_uncertain or error.code != "JOURNAL_CHANGED":
                                raise
                owner.dispose(close_custody)
                self.cleanups.append({"case": purpose, **owner.cleanup})

    def positive(self, purpose):
        with self.fixture_case(purpose) as (context, lock_fd, operation, action, pg):
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
                                 "release": pg.owner.release_hashes, "producers": pg.owner.producer_hashes})

    def restore_negative(self):
        with self.fixture_case("RESTORE") as (context, lock_fd, operation, action, pg):
            before = pg.rejection_counts()
            driver.run_restore_no_sql_rejection(context, lock_fd, action, observer=pg.barrier)
            after = pg.rejection_counts()
            require(all(before[k] == after[k] == 0 for k in ("sql", "events", "receipts"))
                    and before["oid"] == after["oid"] and before["function"] == after["function"],
                    "FIXTURE_RESTORE_SQL")
            self.results.append({"case": "RESTORE_UNVERIFIED_ARCHIVE_REJECTED", "status": "PASS",
                                 "counters": after})

    def fault(self, kind):
        with self.fixture_case("STOP") as (context, lock_fd, operation, action, pg):
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


def run(source_root, commit, manifest_path, deadline_ns):
    require(os.geteuid() == 0 and os.getegid() == 0 and sys.platform == "linux", "FIXTURE_ROOT_REQUIRED")
    if not pathlib.Path("/usr/bin/docker").is_file() or not pathlib.Path("/var/run/docker.sock").exists():
        raise FixtureNotRun("HOSTED_ROOT_PG_FIXTURE_REQUIRED")
    source, manifest, deadline = validated_inputs(source_root, commit, manifest_path, deadline_ns)
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
                return {"status": "PASS", "commit": commit, "manifest_sha256": ops._digest(manifest),
                        "cases": runner.results, "cleanup": runner.cleanups, "runtime": "GENUINE_SELECTED_IMAGE",
                        "production": "CLOSED", "enable": False}
        finally:
            deadline.cleanup_phase()
            shutil.rmtree(policy_root)


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
    if os.geteuid() != 0:
        print("PAPER_GENUINE_FIXTURE_NOT_RUN:HOSTED_ROOT_PG_FIXTURE_REQUIRED")
        return 77
    args = parser.parse_args(argv)
    try:
        with termination_cleanup():
            result = run(args.source_root, args.commit, args.manifest_file, args.deadline_monotonic_ns)
    except FixtureNotRun:
        print("PAPER_GENUINE_FIXTURE_NOT_RUN:HOSTED_ROOT_PG_FIXTURE_REQUIRED")
        return 77
    except (Exception, KeyboardInterrupt):
        print("PAPER_GENUINE_FIXTURE_FAILED:OWNED_CASE_UNPROVEN")
        return 1
    print(canonical(result).decode())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
