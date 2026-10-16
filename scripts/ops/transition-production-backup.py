#!/usr/bin/env python3
"""Bounded replacement of the six already-installed production backup files.

This is a maintenance operation, separate from the no-clobber fresh installer.
The operator must stop the four exact backup units first. No mode starts, stops,
enables or disables a unit, calls Docker, or changes a database. Recovery restores
only this transaction's file identities and deliberately leaves timers inactive.
"""
from __future__ import annotations

import argparse
import contextlib
import datetime
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import stat
import subprocess
import sys
import time
import uuid

sys.dont_write_bytecode = True
ROOT = Path(__file__).absolute().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "backup_transition_files", ROOT / "scripts/ops/lib/owner-beta-daily-release.py")
assert SPEC and SPEC.loader
files = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(files)

UNITS = ("lagrange-production-backup.service", "lagrange-production-backup.timer",
         "lagrange-production-backup-verify.service", "lagrange-production-backup-verify.timer")
NAMES = ("helper", "config", *UNITS)
SCHEMA = "production-backup-transition-v1"
LIMIT = 131072
ENV = {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LANG": "C", "LC_ALL": "C",
       "SYSTEMD_PAGER": "cat", "SYSTEMD_COLORS": "0"}


class Refused(Exception):
    pass


def require(value):
    if not value:
        raise Refused("BACKUP_TRANSITION_REFUSED")


def digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def identity(info):
    # rename changes ctime. Dev/inode plus ownership, mode, link count, size,
    # mtime and separately checked bytes identify our staged file after rename.
    return list(files.file_identity(info)[:-1])


class Layout:
    """CLI uses only canonical production targets; tests supply an isolated root."""
    def __init__(self, release, config, prefix=Path("/")):
        self.release, self.config = Path(release), Path(config)
        self.prefix = prefix
        self.install = prefix / "opt/lagrange"
        self.targets = {"helper": prefix / "opt/lagrange/bin/run-production-backup.sh",
                        "config": prefix / "etc/lagrange/production-backup.conf",
                        **{n: prefix / "etc/systemd/system" / n for n in UNITS}}
        self.sources = {"helper": self.release / "scripts/ops/run-production-backup.sh",
                        "config": self.config,
                        **{n: self.release / "deploy/systemd" / n for n in UNITS}}
        self.locks = (self.install / ".lagrange-release.lock",
                      prefix / "var/lib/lagrange/backup-state/production-backup.lock")

    @staticmethod
    def mode(name):
        return 0o600 if name == "config" else 0o755 if name == "helper" else 0o644


def contract(data, now=None):
    value = files.json_bytes(data, LIMIT)
    require(isinstance(value, dict) and set(value) == {
        "schema", "new_commit", "config_source", "issued_at", "expires_at",
        "old", "new", "unit_file_states"})
    require(value["schema"] == SCHEMA and files.is_commit(value["new_commit"]))
    files.path_parts(value["config_source"])
    for key in ("old", "new"):
        require(isinstance(value[key], dict) and set(value[key]) == set(NAMES) and
                all(files.is_hash(v) for v in value[key].values()))
    require(isinstance(value["unit_file_states"], dict) and
            set(value["unit_file_states"]) == set(UNITS) and
            all(v in {"enabled", "disabled", "static"}
                for v in value["unit_file_states"].values()))
    times = []
    for key in ("issued_at", "expires_at"):
        require(isinstance(value[key], str) and
                re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z", value[key]))
        times.append(datetime.datetime.fromisoformat(value[key].replace("Z", "+00:00")))
    now = now or datetime.datetime.now(datetime.timezone.utc)
    require(times[0] <= now < times[1] and 0 < (times[1] - times[0]).total_seconds() <= 1800)
    return value


def binding(value):
    # A later recovery may carry a fresh bounded time window, but cannot change
    # any source, destination, release, unit-state or old-byte commitment.
    return {k: v for k, v in value.items() if k not in {"issued_at", "expires_at"}}


def capture(parent, name, mode):
    data = files.read_file(parent, name, 0, 0, mode, LIMIT)
    info = os.stat(name, dir_fd=parent, follow_symlinks=False)
    require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1 and info.st_uid == 0 and
            info.st_gid == 0 and stat.S_IMODE(info.st_mode) == mode and info.st_size == len(data))
    # Bind the data and identity in a second stable read before returning them.
    require(files.read_file(parent, name, 0, 0, mode, LIMIT) == data and
            identity(os.stat(name, dir_fd=parent, follow_symlinks=False)) == identity(info))
    return data, identity(info)


@contextlib.contextmanager
def locked(layout):
    with contextlib.ExitStack() as stack:
        held = []
        for path in layout.locks:
            parent = stack.enter_context(files.root_directory(str(path.parent)))
            fd = os.open(path.name, os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=parent)
            stack.callback(os.close, fd)
            before = files.file_info(fd, 0, 0, 0o600)
            require(before.st_size == 0)
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            require(identity(os.stat(path.name, dir_fd=parent, follow_symlinks=False)) == identity(before))
            held.append((parent, path.name, fd, identity(before)))
        try:
            yield
        finally:
            for parent, name, fd, before in held:
                require(identity(files.file_info(fd, 0, 0, 0o600)) == before and
                        identity(os.stat(name, dir_fd=parent, follow_symlinks=False)) == before)


class Native:
    def __init__(self, layout, value):
        self.layout, self.value = layout, value
        expires = datetime.datetime.fromisoformat(value["expires_at"].replace("Z", "+00:00"))
        self.deadline = time.monotonic() + min(
            300, (expires - datetime.datetime.now(datetime.timezone.utc)).total_seconds())

    def call(self, args, limit=30):
        remaining = min(limit, self.deadline - time.monotonic())
        require(remaining > 0)
        result = subprocess.run(args, env=ENV, stdin=subprocess.DEVNULL,
                                capture_output=True, timeout=remaining, check=False)
        require(result.returncode == 0 and len(result.stdout) <= LIMIT)
        return result.stdout

    def current(self):
        with files.root_directory(str(self.layout.install)) as parent:
            require(os.readlink("current", dir_fd=parent) == "releases/" + self.value["new_commit"])
        require(self.layout.release == self.layout.install / "releases" / self.value["new_commit"])

    def inactive(self, allow_reload=False):
        properties = ("LoadState", "ActiveState", "SubState", "FragmentPath",
                      "DropInPaths", "NeedDaemonReload", "UnitFileState")
        for name in UNITS:
            result = self.call(["/usr/bin/systemctl", "show", "--no-pager",
                                "--property=" + ",".join(properties), name], 10)
            values = {}
            for line in result.decode("utf-8", errors="strict").splitlines():
                key, sep, value = line.partition("=")
                require(sep and key in properties and key not in values)
                values[key] = value
            reload_state = values.get("NeedDaemonReload")
            require(reload_state in ({"no", "yes"} if allow_reload else {"no"}))
            require(values == {"LoadState": "loaded", "ActiveState": "inactive", "SubState": "dead",
                "FragmentPath": str(self.layout.targets[name]), "DropInPaths": "",
                "NeedDaemonReload": reload_state, "UnitFileState": self.value["unit_file_states"][name]})

    def reload(self):
        self.call(["/usr/bin/systemctl", "daemon-reload"])

    def check(self, installed):
        if installed:
            args = ["/bin/bash", "--noprofile", "--norc",
                    str(self.layout.release / "scripts/ops/install-production-backup.sh"),
                    "--check", "--config-source", str(self.layout.config)]
        else:
            args = ["/bin/bash", "--noprofile", "--norc", str(self.layout.sources["helper"]),
                    "--check", "--config-file", str(self.layout.config)]
        self.call(args, 60)


def save(parent, value):
    name = ".journal-" + uuid.uuid4().hex
    files.create_file(parent, name, files.canonical(value), 0, 0, 0o600)
    os.replace(name, "journal.json", src_dir_fd=parent, dst_dir_fd=parent)
    os.fsync(parent)


class Transition:
    def __init__(self, layout, value, recovery, native):
        self.layout, self.value, self.recovery, self.native = layout, value, Path(recovery), native
        files.path_parts(str(self.recovery))
        require(self.recovery != self.layout.config and
                not any(self.recovery == p or self.recovery in p.parents or p in self.recovery.parents
                        for p in [*layout.targets.values(), *layout.sources.values(), *layout.locks]))

    @contextlib.contextmanager
    def parents(self):
        with contextlib.ExitStack() as stack:
            parents = {n: stack.enter_context(files.root_directory(str(p.parent)))
                       for n, p in self.layout.targets.items()}
            yield parents

    def sources(self):
        result = {}
        for name, path in self.layout.sources.items():
            data = files.read_root_file(str(path), gid=0, mode=self.layout.mode(name), maximum=LIMIT)
            require(digest(data) == self.value["new"][name])
            result[name] = data
        config = {}
        for line in result["config"].decode("ascii", errors="strict").splitlines():
            if not line or line.startswith("#"):
                continue
            key, sep, value = line.partition("=")
            require(sep and key not in config)
            config[key] = value
        expected = {
            "LAGRANGE_CODE_COMMIT": self.value["new_commit"],
            "COMPOSE_FILE": str(self.layout.release / "deploy/compose/compose.yml"),
            "COMPOSE_ENV_FILE": str(self.layout.release / "deploy/compose/.env"),
            "BACKUP_ROOT": str(self.layout.prefix / "srv/backups/lagrange"),
            "DATA_ROOT": str(self.layout.prefix / "var/lib/lagrange/data"),
            "KEY_FILE": str(self.layout.prefix / "etc/lagrange/secrets/backup_encryption_key"),
            "LOCK_FILE": str(self.layout.locks[1]),
            "METRICS_FILE": str(self.layout.prefix / "var/lib/lagrange/backup-state/production-backup.prom"),
        }
        require(all(config.get(k) == v for k, v in expected.items()) and
                self.layout.config not in self.layout.targets.values())
        return result

    def prepare(self, parents):
        self.native.current()
        self.native.inactive()
        new = self.sources()
        old, identities = {}, {}
        for name, parent in parents.items():
            old[name], identities[name] = capture(parent, self.layout.targets[name].name, self.layout.mode(name))
            require(digest(old[name]) == self.value["old"][name])
        self.native.check(False)
        self.native.current()
        self.native.inactive()
        return old, new, identities

    def preflight(self):
        with locked(self.layout), self.parents() as parents:
            self.prepare(parents)

    def restore(self, parents, journal_fd, journal):
        require(journal["binding"] == binding(self.value) and
                set(journal["entries"]) == set(NAMES) and
                re.fullmatch(r"[0-9a-f]{32}", journal["nonce"]))
        self.native.current()
        self.native.inactive(allow_reload=True)
        # Check every target before the first restoration. A foreign write is
        # never overwritten merely because another file needs recovery.
        states, old_bytes = {}, {}
        for name, parent in parents.items():
            entry = journal["entries"][name]
            index = NAMES.index(name)
            old_bytes[name] = files.read_file(journal_fd, f"old-{index}", 0, 0, 0o600, LIMIT)
            require(digest(old_bytes[name]) == self.value["old"][name])
            data, current = capture(parent, self.layout.targets[name].name, self.layout.mode(name))
            if current == entry["old_identity"] and digest(data) == self.value["old"][name]:
                states[name] = "old"
            elif current == entry.get("restored_identity") and digest(data) == self.value["old"][name]:
                states[name] = "old"
            else:
                require(current == entry.get("new_identity") and digest(data) == self.value["new"][name])
                states[name] = "new"
        for index, name in reversed(list(enumerate(NAMES))):
            parent, entry = parents[name], journal["entries"][name]
            target = self.layout.targets[name].name
            if states[name] == "new":
                temp = ".backup-restore-" + journal["nonce"] + "-" + str(index)
                try:
                    staged, info = capture(parent, temp, self.layout.mode(name))
                except FileNotFoundError:
                    files.create_file(parent, temp, old_bytes[name], 0, 0, self.layout.mode(name))
                    entry["restored_identity"] = identity(os.stat(temp, dir_fd=parent, follow_symlinks=False))
                    save(journal_fd, journal)
                else:
                    require(staged == old_bytes[name] and info == entry.get("restored_identity"))
                require(capture(parent, target, self.layout.mode(name))[1] == entry["new_identity"])
                os.replace(temp, target, src_dir_fd=parent, dst_dir_fd=parent)
                os.fsync(parent)
            stage = ".backup-upgrade-" + journal["nonce"] + "-" + str(index)
            try:
                staged, info = capture(parent, stage, self.layout.mode(name))
            except FileNotFoundError:
                continue
            require(info == entry.get("new_identity") and digest(staged) == self.value["new"][name])
            os.unlink(stage, dir_fd=parent)
            os.fsync(parent)
        self.native.reload()
        self.native.current()
        self.native.inactive()
        journal["state"] = "RECOVERED_FILES_TIMERS_INACTIVE"
        save(journal_fd, journal)

    def apply(self):
        with locked(self.layout), self.parents() as parents:
            old, new, identities = self.prepare(parents)
            with files.root_directory(str(self.recovery.parent)) as recovery_parent:
                os.mkdir(self.recovery.name, mode=0o700, dir_fd=recovery_parent)
                os.fsync(recovery_parent)
            with files.root_directory(str(self.recovery)) as recovery_fd:
                files.directory_info(recovery_fd, 0, 0, 0o700)
                for index, name in enumerate(NAMES):
                    files.create_file(recovery_fd, f"old-{index}", old[name], 0, 0, 0o600)
                journal = {"schema": SCHEMA, "binding": binding(self.value), "nonce": uuid.uuid4().hex,
                           "state": "STAGING", "entries": {n: {"old_identity": identities[n]}
                                                              for n in NAMES}}
                save(recovery_fd, journal)
                try:
                    for index, name in enumerate(NAMES):
                        parent = parents[name]
                        target = self.layout.targets[name].name
                        stage = ".backup-upgrade-" + journal["nonce"] + "-" + str(index)
                        require(capture(parent, target, self.layout.mode(name)) == (old[name], identities[name]))
                        files.create_file(parent, stage, new[name], 0, 0, self.layout.mode(name))
                        journal["entries"][name]["new_identity"] = identity(
                            os.stat(stage, dir_fd=parent, follow_symlinks=False))
                        save(recovery_fd, journal)
                    # Persist every new identity before the first destination
                    # changes. A crash during preparation leaves all old targets
                    # intact; an unrecorded staging file is never guessed away.
                    journal["state"] = "PREPARED"
                    save(recovery_fd, journal)
                    self.native.current()
                    self.native.inactive()
                    require(self.sources() == new)
                    for index, name in enumerate(NAMES):
                        parent = parents[name]
                        target = self.layout.targets[name].name
                        stage = ".backup-upgrade-" + journal["nonce"] + "-" + str(index)
                        require(capture(parent, target, self.layout.mode(name)) == (old[name], identities[name]))
                        require(capture(parent, stage, self.layout.mode(name)) ==
                                (new[name], journal["entries"][name]["new_identity"]))
                        os.replace(stage, target, src_dir_fd=parent, dst_dir_fd=parent)
                        os.fsync(parent)
                    self.native.reload()
                    self.native.current()
                    self.native.inactive()
                    require(self.sources() == new)
                    self.native.check(True)
                    for name, parent in parents.items():
                        data, info = capture(parent, self.layout.targets[name].name, self.layout.mode(name))
                        require(data == new[name] and info == journal["entries"][name]["new_identity"])
                    journal["state"] = "APPLIED_TIMERS_INACTIVE"
                    save(recovery_fd, journal)
                except BaseException:
                    # Even when rollback cannot complete, preserve the journal
                    # and propagate failure. Never claim successful recovery.
                    self.restore(parents, recovery_fd, journal)
                    raise

    def recover(self):
        with locked(self.layout), self.parents() as parents:
            self.native.current()
            # Files may differ from loaded units after a crash; reload is only
            # permitted after actual inactive state has been established.
            self.native.inactive(allow_reload=True)
            with files.root_directory(str(self.recovery)) as recovery_fd:
                files.directory_info(recovery_fd, 0, 0, 0o700)
                journal = files.json_bytes(files.read_file(recovery_fd, "journal.json", 0, 0, 0o600, LIMIT), LIMIT)
                require(journal.get("schema") == SCHEMA and journal.get("state") in {
                    "STAGING", "PREPARED", "APPLIED_TIMERS_INACTIVE", "RECOVERED_FILES_TIMERS_INACTIVE"})
                self.restore(parents, recovery_fd, journal)


class Arguments(argparse.ArgumentParser):
    def error(self, message):
        raise Refused("BACKUP_TRANSITION_ARGUMENTS")


def main(argv=None):
    parser = Arguments(description=__doc__)
    parser.add_argument("--mode", choices=("plan", "preflight", "apply", "recover"), default="plan")
    parser.add_argument("--contract")
    parser.add_argument("--recovery-dir")
    try:
        args = parser.parse_args(argv)
        if args.mode == "plan":
            print("BACKUP_TRANSITION: PLAN six exact files; stopped units; private recovery; no timer activation")
            return 0
        files.require_root()
        os.umask(0o077)
        def terminated(signum, frame):
            raise Refused("BACKUP_TRANSITION_TERMINATED")
        signal.signal(signal.SIGTERM, terminated)
        signal.signal(signal.SIGINT, terminated)
        require(args.contract and args.recovery_dir)
        value = contract(files.read_root_file(args.contract, gid=0, mode=0o600, maximum=LIMIT))
        layout = Layout(ROOT, value["config_source"])
        require(ROOT == Path("/opt/lagrange/releases") / value["new_commit"])
        transition = Transition(layout, value, args.recovery_dir, Native(layout, value))
        getattr(transition, args.mode)()
        print("BACKUP_TRANSITION: " + {"preflight": "PREFLIGHT", "apply": "APPLIED_TIMERS_INACTIVE",
                                        "recover": "RECOVERED_FILES_TIMERS_INACTIVE"}[args.mode])
        return 0
    except Exception:
        print("BACKUP_TRANSITION: FAILED_RECOVERY_MAY_BE_REQUIRED", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
