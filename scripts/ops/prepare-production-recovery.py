#!/usr/bin/env python3
"""Bounded pre-upgrade audit/capture/verification; never stop or resume a service.

Non-plan modes require a separately staged, protected, reviewed maintenance
payload and an exact fresh host contract. This is not the release installer.
Only the isolated verifier can start a container, and only with copied data.
"""
from __future__ import annotations

import argparse
import contextlib
import datetime
import fcntl
import hashlib
import importlib.util
import os
from pathlib import Path
import re
import signal
import stat
import subprocess
import sys
import time
import types

sys.dont_write_bytecode = True
ROOT = Path(__file__).absolute().parents[2]


def load(name, relative):
    spec = importlib.util.spec_from_file_location(name, ROOT / relative)
    if spec is None or spec.loader is None:
        raise RuntimeError("PREUPGRADE_SOURCE")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


recovery = load("preupgrade_maintenance_recovery", "scripts/ops/lib/production-preupgrade-recovery.py")
transition = load("preupgrade_maintenance_locks", "scripts/ops/transition-production-backup.py")
files, install, runtime = recovery.files, recovery.install, recovery.runtime
require = runtime.require
SCHEMA = "lagrange-preupgrade-maintenance-v3"
FORMAT = "LAGRANGE_PREUPGRADE_RECOVERY_V1"
LIMIT = 2 * 1024 * 1024
MEMBERS = {"files.bundle.enc", "state.json.enc", "metadata.json", "COMPLETE"}
PROPERTIES = ("LoadState", "ActiveState", "SubState", "FragmentPath", "DropInPaths",
              "NeedDaemonReload", "UnitFileState", "ActiveEnterTimestampMonotonic",
              "InactiveEnterTimestampMonotonic", "InvocationID", "Transient", "Job",
              "Triggers", "TriggeredBy", "OnFailure", "OnSuccess", "OnFailureOf", "OnSuccessOf")
SERVICE_PROPERTIES = ("ExecMainStartTimestampMonotonic", "ExecMainExitTimestampMonotonic",
                      "MainPID", "ControlPID", "Restart", "NRestarts", "ControlGroup", "Slice")
UNIT_NAME = re.compile(r"lagrange-[a-zA-Z0-9_.@-]+\.(service|timer|socket|path)")
PAYLOAD = (
    "scripts/ops/prepare-production-recovery.py", "scripts/ops/transition-production-backup.py",
    "scripts/ops/lib/production-preupgrade-recovery.py", "scripts/ops/lib/owner-beta-daily-backup.py",
    "scripts/ops/lib/owner-beta-simulation-operations.py",
    "scripts/ops/lib/owner-beta-simulation-release.py",
    "scripts/ops/lib/owner-beta-simulation-runtime.py",
    "scripts/ops/lib/owner-beta-daily-backup-files.py", "scripts/ops/lib/owner-beta-daily-operations.py",
    "scripts/ops/lib/owner-beta-daily-runtime.py", "scripts/ops/lib/owner-beta-daily-release.py",
    "scripts/ops/lib/owner-beta-daily-base.py",
    "scripts/ops/lib/service-status-policy.py",
    "deploy/db/preupgrade-recovery-verify.sh", "deploy/db/preupgrade-recovery-audit.sql")


def digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def utc():
    return datetime.datetime.now(datetime.timezone.utc)


def timestamp(value):
    require(isinstance(value, str) and re.fullmatch(
        r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z", value), "PREUPGRADE_TIME")
    return datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))


def overlaps(a, b):
    return a == b or a.startswith(b + "/") or b.startswith(a + "/")


def binding(value):
    # A later verify may have a fresh clock/boot bound, but cannot revise the
    # recovery point, bytes, old/tool revisions, owners or scope.
    return {k: v for k, v in value.items() if k not in {"issued_at", "expires_at", "boot_id"}}


def unit_properties(name):
    return PROPERTIES + (SERVICE_PROPERTIES if name.endswith(".service") else ())


def checked_units(units, graph="reciprocal"):
    require(graph in {"reciprocal", "quiescent-observed"}, "PREUPGRADE_UNIT_GRAPH")
    require(isinstance(units, dict) and 4 <= len(units) <= 128 and
            set(transition.UNITS) <= set(units), "PREUPGRADE_UNITS")
    edges = {}
    for name, unit in units.items():
        require(UNIT_NAME.fullmatch(name) and isinstance(unit, dict) and
                set(unit) == {"category", *unit_properties(name)} and
                all(isinstance(v, str) and len(v) <= 16384 for v in unit.values()), "PREUPGRADE_UNITS")
        require(unit["LoadState"] == "loaded" and unit["DropInPaths"] == "" and
                unit["NeedDaemonReload"] == "no" and unit["Job"] == "" and
                unit["OnFailure"] == unit["OnSuccess"] == unit["OnFailureOf"] == unit["OnSuccessOf"] == "" and
                re.fullmatch(r"(?:[0-9a-f]{32})?", unit["InvocationID"]) and
                all(re.fullmatch(r"[0-9]{1,20}", unit[k]) for k in
                    ("ActiveEnterTimestampMonotonic", "InactiveEnterTimestampMonotonic")),
                "PREUPGRADE_UNITS")
        if unit["category"] == "stopped":
            require((unit["ActiveState"], unit["SubState"]) == ("inactive", "dead") and
                    unit["Transient"] == "no" and
                    unit["UnitFileState"] in {"enabled", "disabled", "static"} and
                    unit["FragmentPath"] == "/etc/systemd/system/" + name, "PREUPGRADE_UNITS")
        elif unit["category"] == "retained-disabled-failure":
            require(graph == "quiescent-observed" and name.endswith(".service") and
                    name not in transition.UNITS and unit["Transient"] == "no" and
                    unit["UnitFileState"] == "disabled" and
                    unit["FragmentPath"] == "/etc/systemd/system/" + name and
                    (unit["ActiveState"], unit["SubState"]) == ("failed", "failed") and
                    unit["Restart"] == "no" and unit["Triggers"] == "" and
                    re.fullmatch(r"[0-9a-f]{32}", unit["InvocationID"]), "PREUPGRADE_UNITS")
        else:
            require(unit["category"] == "retained-terminal" and name.endswith(".service") and
                    name not in transition.UNITS and unit["Transient"] == "yes" and
                    unit["UnitFileState"] == "transient" and
                    unit["FragmentPath"] == "/run/systemd/transient/" + name and
                    (unit["ActiveState"], unit["SubState"]) in
                    {("active", "exited"), ("failed", "failed"), ("inactive", "dead")} and
                    unit["Restart"] == "no" and unit["Triggers"] == unit["TriggeredBy"] == "" and
                    re.fullmatch(r"[0-9a-f]{32}", unit["InvocationID"]), "PREUPGRADE_UNITS")
        if name.endswith(".service"):
            require(unit["MainPID"] == unit["ControlPID"] == "0" and
                    unit["Restart"] in {"no", "always", "on-success", "on-failure", "on-abnormal",
                                         "on-abort", "on-watchdog"} and
                    unit["Slice"] == "system.slice" and
                    unit["ControlGroup"] in {"", "/system.slice/" + name} and
                    all(re.fullmatch(r"[0-9]{1,20}", unit[k]) for k in
                        ("ExecMainStartTimestampMonotonic", "ExecMainExitTimestampMonotonic", "NRestarts")),
                    "PREUPGRADE_UNITS")
            if unit["category"] in {"retained-terminal", "retained-disabled-failure"}:
                started = int(unit["ExecMainStartTimestampMonotonic"])
                exited = int(unit["ExecMainExitTimestampMonotonic"])
                # A failed transient unit can have no main-process timestamps.
                # Preserve that observation; do not reset it or invent an exit.
                # The detached graph, exact fragment pin and empty cgroup checks
                # still apply, including both native observations below.
                unstarted_failure = (
                    graph == "quiescent-observed" and unit["category"] == "retained-terminal" and
                    (unit["ActiveState"], unit["SubState"]) == ("failed", "failed") and
                    started == exited == 0 and unit["ActiveEnterTimestampMonotonic"] == "0" and
                    int(unit["InactiveEnterTimestampMonotonic"]) > 0 and unit["NRestarts"] == "0")
                require(0 < started <= exited or unstarted_failure, "PREUPGRADE_UNITS")
            if unit["category"] == "retained-disabled-failure":
                require(int(unit["ExecMainExitTimestampMonotonic"]) <=
                        int(unit["InactiveEnterTimestampMonotonic"]), "PREUPGRADE_UNITS")
        edges[name] = {}
        for key in ("Triggers", "TriggeredBy"):
            names = unit[key].split()
            require(len(names) <= 128 and len(names) == len(set(names)) and
                    all(n in units and n != name for n in names), "PREUPGRADE_UNIT_TRIGGER")
            edges[name][key] = set(names)
    # Preserve both actual property sets. The explicit quiescent mode checks
    # every reported edge without inventing a reverse observation. Its source
    # must still be inactive/dead with no job, and its destination has no PIDs
    # or populated cgroup. A retained persistent failure additionally requires
    # disabled activation sources. Hooks and references outside the full
    # inventory remain forbidden; retained transient histories stay detached.
    def edge(source, target):
        require(not source.endswith(".service") and target.endswith(".service") and
                units[source]["category"] == "stopped", "PREUPGRADE_UNIT_TRIGGER")
        if units[target]["category"] == "retained-disabled-failure":
            require(graph == "quiescent-observed" and
                    units[source]["UnitFileState"] == "disabled", "PREUPGRADE_UNIT_TRIGGER")
        else:
            require(units[target]["category"] == "stopped", "PREUPGRADE_UNIT_TRIGGER")
    for name, links in edges.items():
        for target in links["Triggers"]:
            edge(name, target)
            if graph == "reciprocal":
                require(name in edges[target]["TriggeredBy"], "PREUPGRADE_UNIT_TRIGGER")
        for source in links["TriggeredBy"]:
            edge(source, name)
            if graph == "reciprocal":
                require(name in edges[source]["Triggers"], "PREUPGRADE_UNIT_TRIGGER")


def contract(data, now=None):
    value = install.json_bytes(data, LIMIT)
    require(isinstance(value, dict) and set(value) == {
        "schema", "recovery_id", "tool_commit", "old_commit", "issued_at", "expires_at", "boot_id",
        "project", "database", "postgres", "postgres_ancestry", "roots", "writers", "units",
        "unit_graph", "pins", "migrations",
        "base_manifest_sha256", "secret_recovery_ref", "max_total_bytes", "min_free_bytes"},
        "PREUPGRADE_CONTRACT")
    require(value["schema"] == SCHEMA and all(install.is_commit(value[k]) for k in
            ("old_commit", "tool_commit")) and re.fullmatch(r"[0-9a-f]{32}", value["recovery_id"]) and
            re.fullmatch(r"[0-9a-f-]{36}", value["boot_id"]) and
            runtime.PROJECT.fullmatch(value["project"]) and
            re.fullmatch(r"[a-z][a-z0-9_]{0,62}", value["database"]), "PREUPGRADE_CONTRACT")
    issued, expires = timestamp(value["issued_at"]), timestamp(value["expires_at"])
    require(issued <= (now or utc()) < expires and 0 < (expires - issued).total_seconds() <= 1800,
            "PREUPGRADE_TIME")
    require(type(value["max_total_bytes"]) is int and 0 < value["max_total_bytes"] <= 2**40 and
            type(value["min_free_bytes"]) is int and value["min_free_bytes"] >= 1024**3,
            "PREUPGRADE_SPACE")
    require(install.is_hash(value["base_manifest_sha256"]) and
            re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_.:-]{0,127}", value["secret_recovery_ref"]),
            "PREUPGRADE_REFERENCE")
    roots = value["roots"]
    require(isinstance(roots, list) and len(roots) == 5 and all(isinstance(r, dict) and set(r) ==
            {"name", "path", "uid", "gid", "mode", "parent"} for r in roots), "PREUPGRADE_ROOTS")
    require(isinstance(value["postgres_ancestry"], list) and value["postgres_ancestry"] and
            all(isinstance(row, list) for row in value["postgres_ancestry"]), "PREUPGRADE_SOURCE_ANCESTRY")
    resolved = root_values(value)
    require(tuple(r.name for r in resolved) == files.PREUPGRADE_CLASSES and
            not any(overlaps(a.path, b.path) for i, a in enumerate(resolved) for b in resolved[i + 1:]),
            "PREUPGRADE_ROOTS")
    require(roots[0]["path"] == "/var/lib/lagrange/data/raw" and
            roots[1]["path"] == "/var/lib/lagrange/data/curated" and
            roots[3]["path"] == "/opt/lagrange/releases/" + value["old_commit"] and
            all((r["uid"], r["gid"]) == (10001, 10001) for r in roots[:2]) and
            (roots[3]["uid"], roots[3]["gid"]) == (0, 0), "PREUPGRADE_ROOTS")
    require(not any(overlaps(r.path, p) for r in resolved for p in
            ("/srv/backups/lagrange", "/etc/lagrange/secrets", "/opt/lagrange/maintenance")),
            "PREUPGRADE_ROOTS")
    pg = value["postgres"]
    require(isinstance(pg, dict) and set(pg) == {"id", "image_id", "created", "started_at",
            "restart_count", "volume", "mountpoint", "volume_created_at", "networks"} and
            runtime.CONTAINER.fullmatch(pg["id"]) and runtime.DIGEST.fullmatch(pg["image_id"]) and
            type(pg["restart_count"]) is int and pg["restart_count"] >= 0 and
            re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}", pg["volume"]) and
            pg["mountpoint"] == roots[4]["path"] and
            all(isinstance(pg[k], str) and 0 < len(pg[k]) <= 64 for k in
                ("created", "started_at", "volume_created_at")) and
            isinstance(pg["networks"], dict) and 1 <= len(pg["networks"]) <= 16 and
            all(isinstance(n, str) and runtime.CONTAINER.fullmatch(i) for n, i in pg["networks"].items()),
            "PREUPGRADE_POSTGRES")
    require(Path(pg["mountpoint"]).parts[-3:] == ("volumes", pg["volume"], "_data"),
            "PREUPGRADE_POSTGRES")
    require(isinstance(value["writers"], list) and len(value["writers"]) <= 256 and
            len({w["id"] for w in value["writers"]}) == len(value["writers"]) and
            all(set(w) == {"id", "created", "started_at", "finished_at", "restart_count", "policy_sha256"}
                and runtime.CONTAINER.fullmatch(w["id"]) and install.is_hash(w["policy_sha256"])
                and w["id"] != pg["id"] for w in value["writers"]), "PREUPGRADE_WRITERS")
    checked_units(value["units"], value["unit_graph"])
    tool = "/opt/lagrange/maintenance/" + value["tool_commit"]
    old = roots[3]["path"]
    mandatory = {tool + "/" + p for p in PAYLOAD} | {
        old + "/.lagrange-release", old + "/.lagrange-release-manifest", old + "/deploy/compose/.env",
        old + "/deploy/compose/compose.yml", "/etc/lagrange/production-backup.conf",
        "/opt/lagrange/bin/run-production-backup.sh", *[u["FragmentPath"] for u in value["units"].values()]}
    require(isinstance(value["pins"], dict) and mandatory <= set(value["pins"]) and
            len(value["pins"]) <= 256 and all(runtime.absolute(p) and install.is_hash(h)
                and not overlaps(p, "/etc/lagrange/secrets") for p, h in value["pins"].items()),
            "PREUPGRADE_PINS")
    require(isinstance(value["migrations"], list) and 1 <= len(value["migrations"]) <= 59,
            "PREUPGRADE_MIGRATIONS")
    for n, row in enumerate(value["migrations"], 1):
        require(set(row) == {"version", "checksum", "success"} and type(row["version"]) is int and
                row["version"] == n and row["success"] is True and
                re.fullmatch(r"[0-9a-f]{96}", row["checksum"]), "PREUPGRADE_MIGRATIONS")
    return value


def root_values(value):
    return [files.Root(r["name"], r["path"], r["uid"], r["gid"], r["mode"],
                       parent=None if r["parent"] is None else tuple(r["parent"]),
                       ancestry=tuple(tuple(row) for row in value["postgres_ancestry"])
                       if r["name"] == "postgres" else ()) for r in value["roots"]]


class Window:
    def __init__(self, value):
        self.issued, self.expires = timestamp(value["issued_at"]), timestamp(value["expires_at"])
        self.deadline = time.monotonic() + (self.expires - utc()).total_seconds()

    def remaining(self, maximum=1800):
        remaining = min(maximum, self.deadline - time.monotonic(), (self.expires - utc()).total_seconds())
        require(self.issued <= utc() and remaining > 0, "PREUPGRADE_EXPIRED")
        return remaining


def canonical_mounts(value):
    """Compare mount mappings by destination while retaining every field."""
    require(type(value) is list and len(value) <= 64 and
            all(type(m) is dict and type(m.get("Destination")) is str and
                runtime.absolute(m["Destination"]) for m in value),
            "PREUPGRADE_WRITER_MOUNTS")
    destinations = [m["Destination"] for m in value]
    require(len(destinations) == len(set(destinations)), "PREUPGRADE_WRITER_MOUNTS")
    return sorted(value, key=lambda m: m["Destination"])


def stopped_writer(value):
    state = value.get("State", {})
    require(state.get("Status") == "exited" and state.get("Running") is False and
            state.get("Paused") is False and state.get("Restarting") is False and
            state.get("Dead") is False and type(state.get("Pid")) is int and state.get("Pid") == 0 and
            type(value.get("RestartCount")) is int and value["RestartCount"] >= 0 and
            not state.get("OOMKilled") and not state.get("Error"), "PREUPGRADE_WRITER_ACTIVE")
    policy = {k: value.get(k) for k in ("Image", "Config", "HostConfig", "Mounts", "NetworkSettings")}
    policy["Mounts"] = canonical_mounts(policy["Mounts"])
    return {"id": value["Id"], "created": value["Created"], "started_at": state["StartedAt"],
            "finished_at": state["FinishedAt"], "restart_count": value["RestartCount"],
            "policy_sha256": digest(files.canonical(policy))}


def empty_cgroup(name, window, root="/sys/fs/cgroup"):
    """Read a bounded, no-symlink system.slice subtree without changing it.

    Even an empty ControlGroup property does not skip the canonical path.
    Missing unit groups are allowed; missing parents or unreadable/changed
    descendants are not. The caller brackets this with exact unit generations.
    """
    require(UNIT_NAME.fullmatch(name) and name.endswith(".service"), "PREUPGRADE_CGROUP")
    count = 0

    def identity(info):
        require(info.st_uid == 0 and info.st_gid == 0 and not info.st_mode & 0o022,
                "PREUPGRADE_CGROUP")
        return info.st_dev, info.st_ino, info.st_mode

    def read(fd, filename):
        child = os.open(filename, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=fd)
        try:
            before = os.fstat(child)
            require(stat.S_ISREG(before.st_mode), "PREUPGRADE_CGROUP")
            install.no_acl(child)
            stamp = identity(before)
            data = os.read(child, 4097)
            require(len(data) <= 4096 and stamp == identity(os.fstat(child)) and
                    stamp == identity(os.stat(filename, dir_fd=fd, follow_symlinks=False)),
                    "PREUPGRADE_CGROUP")
            return data
        finally:
            os.close(child)

    def scan(fd, depth):
        nonlocal count
        window.remaining()
        count += 1
        require(count <= 128 and depth <= 16, "PREUPGRADE_CGROUP_LIMIT")
        install.no_acl(fd)
        stamp = identity(os.fstat(fd))
        names = sorted(recovery.backup.bounded_names(fd, 256))
        require(read(fd, "cgroup.procs") == read(fd, "cgroup.threads") == b"",
                "PREUPGRADE_CGROUP_ACTIVE")
        events = read(fd, "cgroup.events").splitlines()
        require(len(events) == 2 and set(events) == {b"populated 0", b"frozen 0"},
                "PREUPGRADE_CGROUP_ACTIVE")
        for child_name in names:
            window.remaining()
            info = os.stat(child_name, dir_fd=fd, follow_symlinks=False)
            require(stat.S_ISREG(info.st_mode) or stat.S_ISDIR(info.st_mode), "PREUPGRADE_CGROUP")
            if stat.S_ISDIR(info.st_mode):
                child = os.open(child_name, files.DIRECTORY, dir_fd=fd)
                try:
                    expected = identity(info)
                    require(identity(os.fstat(child)) == expected, "PREUPGRADE_CGROUP")
                    scan(child, depth + 1)
                    require(identity(os.stat(child_name, dir_fd=fd, follow_symlinks=False)) == expected,
                            "PREUPGRADE_CGROUP")
                finally:
                    os.close(child)
        require(names == sorted(recovery.backup.bounded_names(fd, 256)) and
                stamp == identity(os.fstat(fd)) and read(fd, "cgroup.procs") == b"" and
                read(fd, "cgroup.threads") == b"" and read(fd, "cgroup.events").splitlines() == events,
                "PREUPGRADE_CGROUP_CHANGED")

    with install.root_directory(root + "/system.slice") as parent:
        window.remaining()
        try:
            fd = os.open(name, files.DIRECTORY, dir_fd=parent)
        except FileNotFoundError:
            # A dangling symlink is not an absent group.
            require(name not in recovery.backup.bounded_names(parent, 4096), "PREUPGRADE_CGROUP")
            return
        try:
            expected = identity(os.fstat(fd))
            scan(fd, 0)
            require(expected == identity(os.stat(name, dir_fd=parent, follow_symlinks=False)),
                    "PREUPGRADE_CGROUP_CHANGED")
        finally:
            os.close(fd)


class Native:
    def __init__(self, value, docker, window):
        self.value, self.docker, self.window = value, docker, window

    def call(self, args):
        result = subprocess.run(args, env=transition.ENV, stdin=subprocess.DEVNULL,
                                capture_output=True, timeout=self.window.remaining(30), check=False)
        require(result.returncode == 0 and len(result.stdout) <= LIMIT, "PREUPGRADE_OBSERVATION")
        return result.stdout

    def unit_names(self):
        names = set()
        for verb in ("list-unit-files", "list-units"):
            rows = self.call(["/usr/bin/systemctl", verb, "--all", "--plain", "--no-legend",
                              "--no-pager", "lagrange-*"]).decode().splitlines()
            require(len(rows) <= 128, "PREUPGRADE_UNITS")
            listed = set()
            for row in rows:
                require(row.split() and UNIT_NAME.fullmatch(row.split()[0]), "PREUPGRADE_UNITS")
                require(row.split()[0] not in listed, "PREUPGRADE_UNITS")
                listed.add(row.split()[0])
                names.add(row.split()[0])
        require(names == set(self.value["units"]), "PREUPGRADE_UNITS_CHANGED")
        return names

    def unit(self, name):
        properties = unit_properties(name)
        lines = self.call(["/usr/bin/systemctl", "show", "--all", "--no-pager",
            "--property=" + ",".join(properties), name]).decode().splitlines()
        actual = {}
        for line in lines:
            key, sep, val = line.partition("=")
            require(sep and key in properties and key not in actual, "PREUPGRADE_UNITS")
            actual[key] = val
        require(actual == {k: v for k, v in self.value["units"][name].items() if k != "category"},
                "PREUPGRADE_UNITS_CHANGED")

    def units(self):
        checked_units(self.value["units"], self.value["unit_graph"])
        names = self.unit_names()
        for name in sorted(names):
            self.window.remaining()
            self.unit(name)
            if name.endswith(".service"):
                empty_cgroup(name, self.window)
            self.unit(name)
        require(self.unit_names() == names, "PREUPGRADE_UNITS_CHANGED")

    def postgres(self, stopped):
        pg = self.value["postgres"]
        value = self.docker.object("container", pg["id"])
        state = value.get("State", {})
        require(value.get("Id") == pg["id"] and value.get("Image") == pg["image_id"] and
                value.get("Config", {}).get("Image") == recovery.backup.POSTGRES and
                (value.get("Created"), state.get("StartedAt"), value.get("RestartCount")) ==
                (pg["created"], pg["started_at"], pg["restart_count"]), "PREUPGRADE_DATABASE_CHANGED")
        labels = value.get("Config", {}).get("Labels") or {}
        require(labels.get("com.docker.compose.project") == self.value["project"] and
                labels.get("com.docker.compose.service") == "postgres" and
                labels.get("com.docker.compose.oneoff") == "False", "PREUPGRADE_DATABASE_CHANGED")
        networks = {k: v["NetworkID"] for k, v in value["NetworkSettings"]["Networks"].items()}
        require(networks == pg["networks"], "PREUPGRADE_DATABASE_CHANGED")
        volume = recovery.volume_info(self.docker, pg["volume"])
        require(volume["Mountpoint"] == pg["mountpoint"] and
                volume.get("CreatedAt") == pg["volume_created_at"], "PREUPGRADE_VOLUME_CHANGED")
        if stopped:
            generation = (pg["created"], pg["started_at"], state.get("FinishedAt"), pg["restart_count"])
            recovery.stopped_container(value, identity=pg["id"], image_id=pg["image_id"],
                volume=pg["volume"], mountpoint=pg["mountpoint"], generation=generation)
            return state["FinishedAt"]
        require(state.get("Status") == "running" and state.get("Running") is True and
                state.get("Paused") is False and state.get("Restarting") is False and
                state.get("Dead") is False and not state.get("OOMKilled") and not state.get("Error"),
                "PREUPGRADE_DATABASE_NOT_RUNNING")
        return None

    def writers(self):
        ids = self.docker.call(["ps", "--all", "--no-trunc", "--format", "{{.ID}}"]).splitlines()
        require(len(ids) <= 256 and len(ids) == len(set(ids)) and all(
            runtime.CONTAINER.fullmatch(i) for i in ids), "PREUPGRADE_WRITER_INVENTORY")
        selected = []
        for identity in ids:
            self.window.remaining()
            if identity == self.value["postgres"]["id"]:
                continue
            value = self.docker.object("container", identity)
            require(value.get("Id") == identity, "PREUPGRADE_WRITER_INVENTORY")
            same_project = (value.get("Config", {}).get("Labels") or {}).get(
                "com.docker.compose.project") == self.value["project"]
            network_ids = {v.get("NetworkID") for v in value.get("NetworkSettings", {}).get("Networks", {}).values()}
            shares_network = bool(network_ids & set(self.value["postgres"]["networks"].values()))
            shared_mount = any(runtime.absolute(m.get("Source")) and overlaps(m["Source"], r["path"])
                for m in value.get("Mounts", []) for r in self.value["roots"])
            # Host-network containers can reach a published/local PG socket;
            # fence them too, even when they lack project labels or data mounts.
            host_network = value.get("HostConfig", {}).get("NetworkMode") == "host"
            if same_project or shares_network or shared_mount or host_network:
                selected.append(stopped_writer(value))
        require(sorted(selected, key=lambda w: w["id"]) ==
                sorted(self.value["writers"], key=lambda w: w["id"]), "PREUPGRADE_WRITERS_CHANGED")

    def guard(self, stopped):
        self.window.remaining()
        require(Path("/proc/sys/kernel/random/boot_id").read_text().strip() == self.value["boot_id"],
                "PREUPGRADE_BOOT_CHANGED")
        with install.root_directory("/opt/lagrange") as parent:
            require(os.readlink("current", dir_fd=parent) == "releases/" + self.value["old_commit"],
                    "PREUPGRADE_RELEASE_CHANGED")
        for path, expected in self.value["pins"].items():
            self.window.remaining()
            mode = 0o600 if Path(path).name in {".env", ".lagrange-release", ".lagrange-release-manifest",
                                              "production-backup.conf"} else None
            require(digest(install.read_root_file(path, gid=0, mode=mode, maximum=16 * 1024**2)) == expected,
                    "PREUPGRADE_PIN_CHANGED")
        old = self.value["roots"][3]["path"]
        require(install.read_root_file(old + "/.lagrange-release", gid=0, mode=0o600).strip() ==
                self.value["old_commit"].encode(), "PREUPGRADE_RELEASE_CHANGED")
        with install.root_directory(old + "/migrations") as parent:
            names = recovery.backup.bounded_names(parent, 256)
            actual = []
            for name in sorted(names):
                matched = re.fullmatch(r"([0-9]{4})_[a-z0-9_]+\.up\.sql", name)
                if matched and int(matched[1]) <= len(self.value["migrations"]):
                    actual.append({"version": int(matched[1]), "success": True, "checksum":
                        hashlib.sha384(install.read_file(parent, name, 0, 0, maximum=16 * 1024**2)).hexdigest()})
            require(actual == self.value["migrations"], "PREUPGRADE_MIGRATION_BYTES")
        self.units()
        self.writers()
        # Pin the complete source directory chain for audit as well as capture.
        # No directory is created, chowned or chmodded by this read-only check.
        with files.open_root(root_values(self.value)[4]):
            pass
        finished = self.postgres(stopped)
        self.window.remaining()
        return finished


@contextlib.contextmanager
def raw_fence(root):
    """Retain existing publication locks; never create an empty synthetic lock."""
    with contextlib.ExitStack() as stack:
        raw = stack.enter_context(files.open_root(root))
        manifests = stack.enter_context(recovery.backup.bound_directory(raw, "manifests"))
        locks = []
        for provider in sorted(recovery.backup.bounded_names(manifests, 128)):
            require(re.fullmatch(r"provider=[a-z0-9][a-z0-9_-]{0,63}", provider), "PREUPGRADE_RAW")
            pfd = stack.enter_context(recovery.backup.bound_directory(manifests, provider))
            for market in sorted(recovery.backup.bounded_names(pfd, 128)):
                require(re.fullmatch(r"market=[a-z0-9][a-z0-9_-]{0,63}", market), "PREUPGRADE_RAW")
                mfd = stack.enter_context(recovery.backup.bound_directory(pfd, market))
                fd = os.open("commit.lock", files.REGULAR, dir_fd=mfd)
                stack.callback(os.close, fd)
                info = files.checked(fd, directory=False, owners={10001})
                require(info.st_size == 0 and len(locks) < 1024, "PREUPGRADE_RAW")
                fcntl.flock(fd, fcntl.LOCK_SH | fcntl.LOCK_NB)
                locks.append((mfd, fd, files.identity(info)))
        try:
            yield
        finally:
            for parent, fd, before in locks:
                require(files.identity(os.fstat(fd)) == before and files.identity(
                    os.stat("commit.lock", dir_fd=parent, follow_symlinks=False)) == before, "PREUPGRADE_RAW_CHANGED")


@contextlib.contextmanager
def key_fence(path):
    with install.root_directory(str(Path(path).parent)) as parent:
        name = Path(path).name
        fd = os.open(name, files.REGULAR, dir_fd=parent)
        try:
            before = install.file_info(fd, 0, 0, 0o600)
            require(before.st_size == 64 and re.fullmatch(rb"[0-9a-f]{64}", os.read(fd, 65)), "PREUPGRADE_KEY")
            identity = files.identity(before)
            require(files.identity(os.stat(name, dir_fd=parent, follow_symlinks=False)) == identity,
                    "PREUPGRADE_KEY_CHANGED")
            try:
                yield
            finally:
                require(files.identity(os.fstat(fd)) == identity and
                        files.identity(os.stat(name, dir_fd=parent, follow_symlinks=False)) == identity,
                        "PREUPGRADE_KEY_CHANGED")
                install.file_info(fd, 0, 0, 0o600)
        finally:
            os.close(fd)


class Storage(recovery.ColdStorage):
    def __init__(self, value, docker, window):
        super().__init__({"BACKUP_ROOT": "/srv/backups/lagrange", "KEY_FILE":
            "/etc/lagrange/secrets/backup_encryption_key", "MAX_TOTAL_BYTES": value["max_total_bytes"],
            "MIN_FREE_BYTES": value["min_free_bytes"]}, docker, types.SimpleNamespace(release=str(ROOT)),
            types.SimpleNamespace(project=value["project"], database=value["database"]))
        self.value, self.window = value, window

    def reserve(self, size=0):
        self.window.remaining()
        super().reserve(size)

    def command_file(self, args, target, source=None, seconds=900):
        return super().command_file(args, target, source, seconds=self.window.remaining(seconds))

    def verify_set(self, source, *, completed=True):
        with install.root_directory(str(source)) as parent:
            install.directory_info(parent, 0, 0, 0o700)
            require(set(recovery.backup.bounded_names(parent, 4)) == MEMBERS -
                    (set() if completed else {"COMPLETE"}), "PREUPGRADE_MEMBERS")
            if completed:
                require(install.read_file(parent, "COMPLETE", 0, 0, 0o600) == (FORMAT + "\n").encode(),
                        "PREUPGRADE_INCOMPLETE")
            metadata = install.json_bytes(install.read_file(parent, "metadata.json", 0, 0, 0o600))
            require(set(metadata) == {"format", "binding_sha256", "files"} and metadata["format"] == FORMAT and
                    metadata["binding_sha256"] == digest(files.canonical(binding(self.value))) and
                    set(metadata["files"]) == {"files.bundle.enc", "state.json.enc"}, "PREUPGRADE_METADATA")
            for name, expected in metadata["files"].items():
                fd = os.open(name, files.REGULAR, dir_fd=parent)
                try:
                    info = install.file_info(fd, 0, 0, 0o600)
                    require(info.st_size <= self.values["MAX_TOTAL_BYTES"], "PREUPGRADE_SIZE")
                finally:
                    os.close(fd)
                require(recovery.backup.digest_file(source / name) == expected, "PREUPGRADE_CIPHER_DIGEST")
            with self.temporary(".preupgrade-verify-") as stage:
                for name in metadata["files"]:
                    self.crypt(source / name, stage / name.removesuffix(".enc"), decrypt=True)
                state = install.json_bytes(install.read_root_file(str(stage / "state.json"),
                    gid=0, mode=0o600, maximum=64 * 1024**2), maximum=64 * 1024**2)
                require(set(state) == {"schema", "binding", "audit", "files", "bundle_sha256", "finished_at"} and
                        state["schema"] == SCHEMA and state["binding"] == binding(self.value) and
                        recovery.backup.digest_file(stage / "files.bundle") == state["bundle_sha256"],
                        "PREUPGRADE_STATE")
                checked_receipt(state["audit"], self.value, fresh=False)
                restored = stage / "restored"
                restored.mkdir(mode=0o700)
                with (stage / "files.bundle").open("rb") as stream:
                    manifest = files.restore_archive(stream, self.limits, str(restored), self.reserve,
                                                     profile="preupgrade")
                require(manifest == state["files"], "PREUPGRADE_RESTORE_FILES")
                self.verify_cluster(restored, {"files": manifest, "database": state["audit"]["database"]})
                return state


def write_json(directory, name, value):
    with install.root_directory(str(directory)) as parent:
        install.create_file(parent, name, files.canonical(value), 0, 0, 0o600)


FAILURE_CODES = frozenset({
    "PREUPGRADE_BOOT_CHANGED", "PREUPGRADE_RELEASE_CHANGED", "PREUPGRADE_PIN_CHANGED",
    "PREUPGRADE_MIGRATION_BYTES", "PREUPGRADE_UNITS", "PREUPGRADE_UNITS_CHANGED",
    "PREUPGRADE_CGROUP", "PREUPGRADE_CGROUP_CHANGED", "PREUPGRADE_WRITER_ACTIVE",
    "PREUPGRADE_WRITER_INVENTORY", "PREUPGRADE_WRITERS_CHANGED",
    "PREUPGRADE_SOURCE_ANCESTRY", "PREUPGRADE_VOLUME_PARENT", "PREUPGRADE_VOLUME_CHANGED",
    "PREUPGRADE_DATABASE_CHANGED", "PREUPGRADE_DATABASE_NOT_RUNNING",
    "PREUPGRADE_KEY", "PREUPGRADE_KEY_CHANGED", "PREUPGRADE_RAW",
    "PREUPGRADE_RAW_CHANGED", "PREUPGRADE_PENDING_CLEANUP", "PREUPGRADE_OBSERVATION",
    "PREUPGRADE_AUDIT_RESULT", "PREUPGRADE_AUDIT_RECEIPT", "PREUPGRADE_AUDIT_EXPIRED",
    "PREUPGRADE_DATABASE_AUDIT", "PREUPGRADE_MIGRATIONS", "PREUPGRADE_INTERRUPTED",
    "DAILY_RUNTIME_INVALID", "DAILY_BACKUP_SPACE", "DAILY_BACKUP_PROCESS",
    "DAILY_BACKUP_PROCESS_LIMIT", "DAILY_BACKUP_VERIFY_PENDING",
    "DAILY_BACKUP_FILES_TYPE_OR_MODE", "DAILY_BACKUP_FILES_ACL",
})


def failure_details(error):
    """Fixed codes and source locations only; never exception text or locals."""
    code = "UNKNOWN"
    if type(error) in {runtime.RuntimeErrorCode, files.SnapshotError,
                       install.DailyInstallError, RuntimeError}:
        if len(error.args) == 1 and type(error.args[0]) is str and error.args[0] in FAILURE_CODES:
            code = error.args[0]
    elif isinstance(error, OSError) and type(error.errno) is int and error.errno in {
            1, 2, 5, 9, 11, 13, 17, 20, 21, 28, 30, 40}:
        code = "OS_ERROR_" + str(error.errno)
    locations = []
    allowed = {str(ROOT / relative): relative for relative in PAYLOAD if relative.endswith(".py")}
    trace = error.__traceback__
    while trace is not None:
        relative = allowed.get(trace.tb_frame.f_code.co_filename)
        if relative is not None:
            locations.append({"source": relative, "line": trace.tb_lineno})
        trace = trace.tb_next
    return {"code": code, "locations": locations[-4:], "locationsTruncated": len(locations) > 4}


def record_failure(value, mode, error):
    """Best-effort exclusive diagnostic; failure never delays owed restoration."""
    if value is None:
        return
    try:
        receipt = {"schema": "lagrange-preupgrade-failure-v1", "mode": mode,
            "recovery_id": value["recovery_id"],
            "binding_sha256": digest(files.canonical(binding(value))),
            "observed_at": utc().strftime("%Y-%m-%dT%H:%M:%SZ"),
            "error": failure_details(error), "recoveryAccepted": False}
        write_json(Path("/srv/backups/lagrange"),
                   "preupgrade-failure-" + value["recovery_id"] + "-" + mode + ".json", receipt)
    except Exception:
        # Retain any existing/partial receipt; no overwrite, repair or retry.
        pass


def checked_receipt(receipt, value, *, fresh):
    require(isinstance(receipt, dict) and set(receipt) ==
            {"schema", "binding_sha256", "boot_id", "observed_at", "database"} and
            receipt["schema"] == SCHEMA and receipt["binding_sha256"] == digest(files.canonical(binding(value))),
            "PREUPGRADE_AUDIT_RECEIPT")
    when = timestamp(receipt["observed_at"])
    if fresh:
        require(receipt["boot_id"] == value["boot_id"] and timestamp(value["issued_at"]) <= when <= utc() <
                timestamp(value["expires_at"]), "PREUPGRADE_AUDIT_EXPIRED")
    database = receipt["database"]
    recovery.checked_audit(database, database=value["database"],
        system_identifier=database.get("system_identifier"), migrations=value["migrations"])
    return receipt


def audit_command(value):
    # The installed cluster was initialized with POSTGRES_USER=lagrange.
    # OS uid999 owns PostgreSQL; it does not imply a database role named postgres.
    return ["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", "exec", "-i",
        "--user", "999:999", value["postgres"]["id"], "/usr/bin/env", "-i",
        "PATH=/usr/lib/postgresql/18/bin:/usr/bin:/bin", "PGHOST=/var/run/postgresql", "PGUSER=lagrange",
        "PGCONNECT_TIMEOUT=10", "PGPASSFILE=/dev/null", "PGSERVICEFILE=/dev/null",
        "PGOPTIONS=-c default_transaction_read_only=on -c statement_timeout=120000 -c lock_timeout=5000",
        "/usr/bin/timeout", "--signal=TERM", "--kill-after=5s", "150s",
        "psql", "-X", "-q", "-t", "-A", "-v", "ON_ERROR_STOP=1", "--dbname=" + value["database"]]


def audit(value, storage, native):
    native.guard(False)
    with storage.temporary(".preupgrade-audit-") as stage:
        script = ROOT / "deploy/db/preupgrade-recovery-audit.sql"
        with script.open("rb") as source:
            storage.command_file(audit_command(value), stage / "audit.json", source, seconds=160)
        payload = install.read_root_file(str(stage / "audit.json"), gid=0, mode=0o600, maximum=LIMIT)
        require(len(payload.splitlines()) == 1, "PREUPGRADE_AUDIT_RESULT")
        receipt = {"schema": SCHEMA, "binding_sha256": digest(files.canonical(binding(value))),
            "boot_id": value["boot_id"], "observed_at": utc().strftime("%Y-%m-%dT%H:%M:%SZ"),
            "database": install.json_bytes(payload, LIMIT)}
        checked_receipt(receipt, value, fresh=True)
        native.guard(False)
        write_json(Path(storage.root), "preupgrade-audit-" + value["recovery_id"] + ".json", receipt)


def publish(directory, root, name):
    require(re.fullmatch(r"preupgrade-[0-9a-f]{32}", name), "PREUPGRADE_PUBLISH")
    with install.root_directory(str(directory)) as stage:
        install.create_file(stage, "COMPLETE", (FORMAT + "\n").encode(), 0, 0, 0o600)
    with install.root_directory(str(directory.parent)) as origin, install.root_directory(str(root)) as parent:
        install.rename_no_replace(origin, directory.name, parent, name)


def capture(value, storage, native):
    name = "preupgrade-" + value["recovery_id"]
    final = Path(storage.root) / name
    require(not final.exists() and not final.is_symlink(), "PREUPGRADE_EXISTS")
    receipt = checked_receipt(install.json_bytes(install.read_root_file(str(Path(storage.root) /
        ("preupgrade-audit-" + value["recovery_id"] + ".json")), gid=0, mode=0o600, maximum=LIMIT), LIMIT),
        value, fresh=True)
    finished = native.guard(True)
    with storage.temporary(".preupgrade-capture-") as stage, \
            files.Snapshot(root_values(value), storage.limits, profile="preupgrade") as snapshot:
        manifest = install.json_bytes(snapshot.manifest(), storage.limits.manifest_bytes)
        require(digest(files.canonical([r for r in manifest["entries"] if r["path"] == "base" or
                r["path"].startswith("base/")])) == value["base_manifest_sha256"], "PREUPGRADE_BASE_CHANGED")
        bundle = stage / "files.bundle"
        with bundle.open("xb") as stream:
            bundle.chmod(0o600)
            class Writer:
                def write(self, chunk):
                    storage.reserve(len(chunk))
                    return stream.write(chunk)
            snapshot.write(Writer())
            stream.flush()
            os.fsync(stream.fileno())
        state = {"schema": SCHEMA, "binding": binding(value), "audit": receipt, "files": manifest,
                 "bundle_sha256": recovery.backup.digest_file(bundle), "finished_at": finished}
        write_json(stage, "state.json", state)
        output = stage / "set"
        output.mkdir(mode=0o700)
        for member in ("files.bundle", "state.json"):
            storage.crypt(stage / member, output / (member + ".enc"))
        write_json(output, "metadata.json", {"format": FORMAT,
            "binding_sha256": digest(files.canonical(binding(value))), "files": {
                n: recovery.backup.digest_file(output / n) for n in ("files.bundle.enc", "state.json.enc")}})
        require(storage.verify_set(output, completed=False) == state, "PREUPGRADE_PREPARED_CHANGED")
        snapshot.verify_unchanged()
        require(native.guard(True) == finished, "PREUPGRADE_DATABASE_CHANGED")
        storage.window.remaining()
        publish(output, storage.root, name)
        # A post-publication failure leaves the uniquely named set for review;
        # never remove a recovery point or report successful completion then.
        require(native.guard(True) == finished, "PREUPGRADE_DATABASE_CHANGED")


class Arguments(argparse.ArgumentParser):
    def error(self, message):
        raise RuntimeError("PREUPGRADE_ARGUMENTS")


def main(argv=None):
    parser = Arguments(description=__doc__)
    parser.add_argument("--mode", choices=("plan", "audit", "capture", "verify"), default="plan")
    parser.add_argument("--contract")
    value = None
    try:
        args = parser.parse_args(argv)
        if args.mode == "plan":
            print("PREUPGRADE: PLAN fresh host contract; inactive writers; explicit DB stop; isolated clone; no restart")
            return 0
        install.require_root()
        os.umask(0o077)
        require(args.contract, "PREUPGRADE_CONTRACT")
        value = contract(install.read_root_file(args.contract, gid=0, mode=0o600, maximum=LIMIT))
        require(ROOT == Path("/opt/lagrange/maintenance") / value["tool_commit"], "PREUPGRADE_TOOL_LOCATION")
        window = Window(value)
        def interrupted(signum, frame):
            raise RuntimeError("PREUPGRADE_INTERRUPTED")
        for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGALRM):
            signal.signal(sig, interrupted)
        signal.setitimer(signal.ITIMER_REAL, window.remaining())
        docker = runtime.Docker()
        storage, native = Storage(value, docker, window), Native(value, docker, window)
        layout = transition.Layout(ROOT, Path("/etc/lagrange/production-backup.conf"))
        with transition.locked(layout), install.root_directory(storage.root) as backup_root, \
                key_fence(storage.values["KEY_FILE"]):
            install.directory_info(backup_root, 0, 0, 0o700)
            require(not any(n.startswith("preupgrade-verifier-") for n in
                    recovery.backup.bounded_names(backup_root, 10000)), "PREUPGRADE_PENDING_CLEANUP")
            if args.mode == "verify":
                # An old recovery point can be checked after activation. This
                # branch never claims the old live release is still current.
                for relative in PAYLOAD:
                    path = str(ROOT / relative)
                    require(digest(install.read_root_file(path, gid=0, maximum=16 * 1024**2)) ==
                            value["pins"][path], "PREUPGRADE_PIN_CHANGED")
                storage.verify_set(Path(storage.root) / ("preupgrade-" + value["recovery_id"]))
            else:
                with raw_fence(root_values(value)[0]):
                    (audit if args.mode == "audit" else capture)(value, storage, native)
            window.remaining()
        signal.setitimer(signal.ITIMER_REAL, 0)
        print("PREUPGRADE: " + {"audit": "AUDIT_WRITERS_REMAIN_STOPPED", "capture":
              "CAPTURE_VERIFIED_WRITERS_REMAIN_STOPPED", "verify": "EXISTING_POINT_VERIFIED"}[args.mode])
        return 0
    except Exception as error:
        signal.setitimer(signal.ITIMER_REAL, 0)
        if value is not None:
            record_failure(value, args.mode, error)
        print("PREUPGRADE: FAILED_WRITERS_MUST_REMAIN_STOPPED_INSPECT_OWNED_RESOURCES", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
