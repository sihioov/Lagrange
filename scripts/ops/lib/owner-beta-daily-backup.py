#!/usr/bin/env python3
"""Installed-release daily backup and guarded restore reconciliation.

Normal operations use compose-release. Explicit two-date checkpoints use the
pinned maintenance entrypoint. Plan is non-mutating; preflight
reads protected local inputs only. Apply requires the release and backup locks.
No provider request, account API or arbitrary Compose target is available.
"""
from __future__ import annotations

import argparse
import contextlib
import dataclasses
import datetime
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
import uuid

sys.dont_write_bytecode = True
import importlib.util


def local(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).parent / filename)
    if spec is None or spec.loader is None:
        raise RuntimeError("DAILY_BACKUP_SOURCE")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


ops = local("daily_backup_operations", "owner-beta-daily-operations.py")
files = local("daily_backup_files", "owner-beta-daily-backup-files.py")
simulation = local("daily_backup_simulation", "owner-beta-simulation-operations.py")
install, runtime = ops.install, ops.runtime
require = runtime.require
ROOT = str(Path(__file__).absolute().parents[3])
POSTGRES = "postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a"
FORMAT = "LAGRANGE_DAILY_BACKUP_V2"
SIMULATION_FORMAT = "LAGRANGE_SIMULATION_BACKUP_V1"
PREACTIVATION_FORMAT = "LAGRANGE_PREACTIVATION_BACKUP_V1"
CONFIG_KEYS = frozenset(("BACKUP_ROOT", "DATA_ROOT", "COMPOSE_FILE", "COMPOSE_ENV_FILE",
    "COMPOSE_PROJECT", "LAGRANGE_CODE_COMMIT", "KEY_FILE", "LOCK_FILE", "METRICS_FILE",
    "MAX_TOTAL_BYTES", "MIN_FREE_BYTES", "RETENTION_DAYS", "MIN_KEEP",
    "POSTGRES_SERVICE", "POSTGRES_IMAGE"))
TOOL_KEYS = frozenset(("DAILY_BACKUP_TOOL_COMMIT", "DAILY_BACKUP_TOOL_MANIFEST"))
SET_NAME = re.compile(r"backup-daily-[0-9]{8}T[0-9]{6}Z-[0-9a-f]{40}")
SIMULATION_SET_NAME = re.compile(r"backup-simulation-[0-9]{8}T[0-9]{6}Z-[0-9a-f]{40}")
INACTIVE_SIMULATION_FORMAT = "LAGRANGE_INACTIVE_DAILY_SIMULATION_BACKUP_V1"
INACTIVE_SIMULATION_SET_NAME = re.compile(r"backup-inactive-simulation-[0-9]{8}T[0-9]{6}Z-[0-9a-f]{40}")
PREACTIVATION_SET_NAME = re.compile(r"backup-preactivation-[0-9]{8}T[0-9]{6}Z-[0-9a-f]{40}")
MEMBERS = ("postgres.dump.enc", "files.bundle.enc", "state.json.enc")


def digest_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(files.CHUNK), b""):
            digest.update(chunk)
    return "sha256:" + digest.hexdigest()


def config_file(path, context, project, *, check_only=False):
    data = install.read_root_file(path, gid=0, mode=0o600, maximum=32768)
    values = {}
    for line in data.decode("ascii", errors="strict").splitlines():
        if not line or line.startswith("#"):
            continue
        key, sep, value = line.partition("=")
        require(sep and key in CONFIG_KEYS | TOOL_KEYS and key not in values and value and
                not any(c.isspace() or ord(c) < 32 or ord(c) == 127 for c in value),
                "DAILY_BACKUP_CONFIG")
        values[key] = value
    require(set(values) in (CONFIG_KEYS, CONFIG_KEYS | TOOL_KEYS) and
            values["COMPOSE_PROJECT"] == project and
            values["LAGRANGE_CODE_COMMIT"] == context.commit and
            values["DATA_ROOT"] == context.values["LAGRANGE_DATA_DIR"] and
            values["COMPOSE_FILE"] == context.release + "/deploy/compose/compose.yml" and
            values["COMPOSE_ENV_FILE"] == context.release + "/deploy/compose/.env" and
            values["POSTGRES_SERVICE"] == "postgres" and values["POSTGRES_IMAGE"] == POSTGRES,
            "DAILY_BACKUP_BINDING")
    if TOOL_KEYS <= values.keys():
        require(install.is_commit(values["DAILY_BACKUP_TOOL_COMMIT"]) and
                install.is_hash(values["DAILY_BACKUP_TOOL_MANIFEST"]), "DAILY_BACKUP_TOOL")
    for key in ("BACKUP_ROOT", "DATA_ROOT", "COMPOSE_FILE", "COMPOSE_ENV_FILE",
                "KEY_FILE", "LOCK_FILE", "METRICS_FILE"):
        require(runtime.absolute(values[key]) and "," not in values[key], "DAILY_BACKUP_PATH")
    for key, maximum in (("MAX_TOTAL_BYTES", 2**50), ("MIN_FREE_BYTES", 2**50),
                         ("RETENTION_DAYS", 3650), ("MIN_KEEP", 1000)):
        require(re.fullmatch(r"[1-9][0-9]{0,15}", values[key]) and
                int(values[key]) <= maximum, "DAILY_BACKUP_CONFIG")
        values[key] = int(values[key])
    backup = values["BACKUP_ROOT"]
    for source in (values["DATA_ROOT"], context.values["LAGRANGE_ARTIFACTS_DIR"],
                   context.values["LAGRANGE_RUNTIME_SECRET_DIR"], context.release):
        require(not (source == backup or source.startswith(backup + "/") or
                     backup.startswith(source + "/")), "DAILY_BACKUP_PATH")
    key = install.read_root_file(values["KEY_FILE"], gid=0, mode=0o600, maximum=64)
    require(re.fullmatch(rb"[0-9a-f]{64}", key), "DAILY_BACKUP_KEY")
    for directory in (backup, str(Path(values["LOCK_FILE"]).parent), str(Path(values["METRICS_FILE"]).parent)):
        probe = Path(directory)
        while check_only and not probe.exists() and not probe.is_symlink():
            probe = probe.parent
        with install.root_directory(str(probe)) as fd:
            if str(probe) == directory:
                install.directory_info(fd, 0, 0, 0o700)
    return values


@contextlib.contextmanager
def backup_lock(path):
    with install.root_directory(str(Path(path).parent)) as parent:
        name = Path(path).name
        try:
            descriptor = os.open(name, os.O_RDWR | os.O_CREAT | os.O_EXCL |
                                 os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=parent)
        except FileExistsError:
            descriptor = os.open(name, os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=parent)
        try:
            before = install.file_info(descriptor, 0, 0, 0o600)
            require(before.st_size == 0, "DAILY_BACKUP_LOCK")
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise runtime.RuntimeErrorCode("DAILY_BACKUP_BUSY") from None
            yield
            require(install.file_identity(os.stat(name, dir_fd=parent, follow_symlinks=False)) ==
                    install.file_identity(before), "DAILY_BACKUP_LOCK")
        finally:
            os.close(descriptor)


def roots(binding, install_root):
    with install.root_directory(install_root) as directory:
        info = os.fstat(directory)
        require(install.read_file(directory, install.FLOOR_NAME, 0, 0, 0o600) ==
                install.FLOOR_BYTES, "DAILY_BACKUP_READER")
    # provision-linux makes the artifact container directory service-owned,
    # while the historical leaf is UID10001. Preserve that existing identity;
    # do not recursively chown it or relax any other root-owned ancestor.
    artifact = Path(binding.artifact_root)
    with install.root_directory(str(artifact.parent)) as directory:
        fd = os.open(artifact.name, files.DIRECTORY, dir_fd=directory)
        try:
            parent = os.fstat(fd)
            files.checked(fd, directory=True, owners={parent.st_uid})
            require(parent.st_gid == 10001 and stat.S_IMODE(parent.st_mode) == 0o750,
                    "DAILY_BACKUP_BASE_PARENT")
            parent_identity = (parent.st_uid, parent.st_gid, stat.S_IMODE(parent.st_mode))
        finally:
            os.close(fd)
    return (
        files.Root("raw", binding.data_root + "/raw", 10001, 10001, 0o750),
        files.Root("curated", binding.data_root + "/curated", 10001, 10001, 0o750),
        files.Root("daily", binding.data_root + "/owner-beta-daily", 0, 10001, 0o750),
        files.Root("config", binding.release + "/.owner-beta-daily", 0, 10001, 0o750),
        files.Root("floor", install_root, 0, info.st_gid, stat.S_IMODE(info.st_mode),
                   install.FLOOR_NAME),
        files.Root("base", binding.artifact_root + "/historical-price-beta-root",
                   10001, 10001, 0o750, parent=parent_identity),
    )


def simulation_roots(binding, install_root):
    """Exact simulation classes; Snapshot reopens and rechecks every descriptor."""
    def protected_directory(path):
        with install.root_directory(path) as fd:
            value = os.fstat(fd)
            return value.st_uid, value.st_gid, stat.S_IMODE(value.st_mode)

    input_info = os.stat(binding.historical_root, follow_symlinks=False)
    require(stat.S_ISDIR(input_info.st_mode) and input_info.st_uid == 10001 and
            input_info.st_gid in files.OWNERS and not input_info.st_mode & 0o7022,
            "DAILY_BACKUP_SIMULATION_INPUT")
    floor_meta = protected_directory(install_root)
    release_meta = protected_directory(binding.release)
    reader_path = binding.release + "/configs/evidence"
    reader_meta = protected_directory(reader_path)
    # Inventory the entire bound historical tree. The archive verifier rejects
    # unapproved siblings rather than silently omitting historical material.
    simulation.read_approval(binding.release)
    return (
        files.Root("simulation_input", binding.historical_root, input_info.st_uid,
                   input_info.st_gid, stat.S_IMODE(input_info.st_mode),
                   parent=simulation.input_parent_identity(binding.historical_root)),
        files.Root("simulation_config", binding.release + "/.owner-beta-simulation", 0, 10001, 0o750),
        files.Root("simulation_floor", install_root, *floor_meta,
                   ".owner-beta-simulation-reader-floor"),
        files.Root("simulation_receipt", binding.release + "/.owner-beta-simulation-receipt", 0, 0, 0o755),
        files.Root("simulation_manifest", binding.release, *release_meta, ".lagrange-release-manifest"),
        files.Root("simulation_reader", reader_path, *reader_meta, "owner-beta-simulation-reader-v1.json"),
        files.Root("simulation_approval", reader_path, *reader_meta, simulation.APPROVAL_NAME),
    )


def simulation_snapshot(binding, install_root, limits, *, data_root, daily_roots=None):
    """Build the two explicit capture profiles without collecting host secrets."""
    if daily_roots is None:
        base = (files.Root("raw", data_root + "/raw", 10001, 10001, 0o750),
                files.Root("curated", data_root + "/curated", 10001, 10001, 0o750))
        profile = "simulation"
    else:
        base = tuple(daily_roots)
        require(tuple(item.name for item in base) == files.CLASSES, "DAILY_BACKUP_SIMULATION_ROOTS")
        profile = "daily-simulation"
    return files.Snapshot((*base, *simulation_roots(binding, install_root)), limits, profile=profile)


def bounded_names(descriptor, maximum):
    names = []
    with os.scandir(descriptor) as entries:
        for entry in entries:
            require(len(names) < maximum, "DAILY_BACKUP_DIRECTORY_LIMIT")
            names.append(entry.name)
    return sorted(names)


def isolated_policy(value, *, memory, mounts, tmpfs, environment):
    host, config = value["HostConfig"], value["Config"]
    require(host.get("Init") is True and host.get("ReadonlyRootfs") is True and
            host.get("Privileged") is False and host.get("CapDrop") == ["ALL"] and
            not host.get("CapAdd") and host.get("SecurityOpt") == ["no-new-privileges:true"] and
            host.get("IpcMode") == "private" and
            host.get("RestartPolicy", {}).get("Name") == "no" and
            host.get("Memory") == memory and host.get("NanoCpus") == 10**9 and
            host.get("Tmpfs") == tmpfs and not any(host.get(k) for k in (
                "Devices", "DeviceRequests", "PortBindings", "VolumesFrom", "PidMode",
                "UTSMode", "ExtraHosts", "Links", "LxcConf")), "DAILY_BACKUP_CONTAINER_POLICY")
    actual = {}
    for mount in value.get("Mounts", []):
        target = mount.get("Destination")
        if mount.get("Type") == "tmpfs" and target in tmpfs:
            continue
        require(target not in actual and mount.get("Type") == "bind" and
                mount.get("Propagation") == "rprivate", "DAILY_BACKUP_CONTAINER_MOUNTS")
        actual[target] = (mount.get("Source"), mount.get("RW"))
    require(actual == mounts and runtime.environment(value) == environment,
            "DAILY_BACKUP_CONTAINER_POLICY")


@contextlib.contextmanager
def bound_directory(parent, name):
    fd = os.open(name, files.DIRECTORY, dir_fd=parent)
    try:
        before = files.identity(files.checked(fd, directory=True, owners={10001}))
        require(files.identity(os.stat(name, dir_fd=parent, follow_symlinks=False)) == before,
                "DAILY_BACKUP_RAW_CHANGED")
        yield fd
        require(files.identity(os.fstat(fd)) == before and
                files.identity(os.stat(name, dir_fd=parent, follow_symlinks=False)) == before,
                "DAILY_BACKUP_RAW_CHANGED")
    finally:
        os.close(fd)


@contextlib.contextmanager
def raw_publication_fence(data_root):
    """Keep existing Raw publication locks and verify their identities."""
    with contextlib.ExitStack() as stack:
        raw = stack.enter_context(files.open_root(
            files.Root("raw", data_root + "/raw", 10001, 10001, 0o750)))
        manifests = stack.enter_context(bound_directory(raw, "manifests"))
        providers = bounded_names(manifests, 128)
        locks = []
        for provider in sorted(providers):
            require(re.fullmatch(r"provider=[a-z0-9][a-z0-9_-]{0,63}", provider), "DAILY_BACKUP_RAW")
            pfd = stack.enter_context(bound_directory(manifests, provider))
            markets = bounded_names(pfd, 128)
            for market in sorted(markets):
                require(re.fullmatch(r"market=[a-z0-9][a-z0-9_-]{0,63}", market), "DAILY_BACKUP_RAW")
                mfd = stack.enter_context(bound_directory(pfd, market))
                fd = os.open("commit.lock", files.REGULAR, dir_fd=mfd)
                stack.callback(os.close, fd)
                info = files.checked(fd, directory=False, owners={10001})
                require(info.st_size == 0 and len(locks) < 1024, "DAILY_BACKUP_LOCK_LIMIT")
                try:
                    fcntl.flock(fd, fcntl.LOCK_SH | fcntl.LOCK_NB)
                except BlockingIOError:
                    raise runtime.RuntimeErrorCode("DAILY_BACKUP_RAW_BUSY") from None
                locks.append((mfd, fd, files.identity(info)))
        yield
        for parent, fd, before in locks:
            require(files.identity(os.fstat(fd)) == before and
                    files.identity(os.stat("commit.lock", dir_fd=parent, follow_symlinks=False)) == before,
                    "DAILY_BACKUP_RAW_CHANGED")


@contextlib.contextmanager
def publication_fence(binding, config):
    """Keep Daily private locks and Raw publication locks in their existing order."""
    install.check_state(binding.data_root, config)
    with contextlib.ExitStack() as stack:
        daily = stack.enter_context(install.root_directory(binding.data_root + "/owner-beta-daily"))
        for name, (stage, uid) in install.STAGES.items():
            descriptor = stack.enter_context(install.child_directory(daily, name, uid, uid, 0o700))
            stack.enter_context(install.state_lock(descriptor, uid))
            install.validate_checkpoint(install.read_file(descriptor, "checkpoint.json", uid, uid, 0o600),
                                        stage, config)
        stack.enter_context(raw_publication_fence(binding.data_root))
        yield


class Storage:
    format = FORMAT
    prefix = "backup-daily-"
    set_pattern = SET_NAME
    state_schema = "lagrange-daily-backup-state-v1"
    state_keys = frozenset(("schema", "database", "files_sha256", "config_sha256"))

    """Private staging, bounded encrypted sets and actual isolated restore."""
    def __init__(self, values, docker, context, binding, *, verifier_root=None):
        self.values, self.docker, self.context, self.binding = values, docker, context, binding
        self.verifier_root = verifier_root
        self.root = values["BACKUP_ROOT"]
        self.limits = files.Limits(values["MAX_TOTAL_BYTES"])

    def reserve(self, size=0):
        require(size >= 0 and shutil.disk_usage(self.root).free >=
                self.values["MIN_FREE_BYTES"] + size, "DAILY_BACKUP_SPACE")

    def require_no_verifier(self):
        # A create response or cleanup observation can be lost. Only bounded
        # reconciliation of the recorded resource may clear that uncertainty.
        with install.root_directory(self.root) as parent:
            require(not any(name.startswith("daily-verifier-")
                            for name in bounded_names(parent, 10000)),
                    "DAILY_BACKUP_VERIFY_PENDING")

    @contextlib.contextmanager
    def temporary(self, prefix):
        self.require_no_verifier()
        self.reserve()
        with install.root_directory(self.root) as parent:
            name = tempfile.mkdtemp(prefix=prefix, dir=self.root)
            os.chmod(name, 0o700)
            try:
                yield Path(name)
            finally:
                # Never remove a mounted copy while its owned verifier's
                # creation/removal remains uncertain. A malformed or unreadable
                # intent also raises before deletion rather than guessing.
                retained = False
                for leaf in bounded_names(parent, 10000):
                    if leaf.startswith("daily-verifier-"):
                        require(re.fullmatch(r"daily-verifier-[0-9a-f]{32}\.json", leaf),
                                "DAILY_BACKUP_VERIFY_OWNER")
                        intent = install.json_bytes(install.read_file(parent, leaf, 0, 0, 0o600))
                        require(isinstance(intent, dict) and isinstance(intent.get("staging"), str),
                                "DAILY_BACKUP_VERIFY_OWNER")
                        retained = retained or intent["staging"] == name
                if not retained:
                    shutil.rmtree(name)

    def command_file(self, args, target, source=None, seconds=900):
        """Capture no diagnostics. Bound duration, output size and free space."""
        self.reserve()
        descriptor = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL |
                             os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
        process = None
        try:
            with os.fdopen(descriptor, "wb") as output:
                process = subprocess.Popen(args, env=ops.ENVIRONMENT,
                    stdin=source if source is not None else subprocess.DEVNULL,
                    stdout=output, stderr=subprocess.DEVNULL)
                deadline = time.monotonic() + seconds
                while process.poll() is None:
                    self.reserve()
                    require(os.fstat(output.fileno()).st_size <= self.values["MAX_TOTAL_BYTES"] and
                            time.monotonic() < deadline, "DAILY_BACKUP_PROCESS_LIMIT")
                    time.sleep(0.1)
                require(process.returncode == 0 and
                        os.fstat(output.fileno()).st_size <= self.values["MAX_TOTAL_BYTES"],
                        "DAILY_BACKUP_PROCESS")
                output.flush()
                os.fsync(output.fileno())
        finally:
            if process is not None and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)

    def crypt(self, source, target, *, decrypt=False):
        # The key's bytes never enter argv, container env, metadata or output.
        with open(source, "rb") as stream:
            self.command_file(["/usr/bin/openssl", "enc", *( ["-d"] if decrypt else ["-salt"] ),
                "-aes-256-cbc", "-pbkdf2", "-iter", "200000", "-pass",
                "file:" + self.values["KEY_FILE"]], target, stream)

    def dump(self, target):
        network = ops.internal_network(self.docker, self.binding.project)
        servers = [self.docker.object("container", identity) for identity in network.get("Containers", {})]
        servers = [s for s in servers if (s.get("Config", {}).get("Labels") or {}).get(
            "com.docker.compose.service") == "postgres"]
        require(len(servers) == 1, "DAILY_BACKUP_DATABASE")
        server = servers[0]
        identity = server["Id"]
        generation = (server["Created"], server["State"]["StartedAt"], server["RestartCount"])
        require(runtime.CONTAINER.fullmatch(identity) and server["State"].get("Running") is True,
                "DAILY_BACKUP_DATABASE")
        # The installed cluster uses the existing lagrange administrator. OS
        # uid999 does not imply a database role named postgres. The pinned
        # server's local socket is the existing backup authority.
        # Both the server-side command and local client have finite deadlines.
        self.command_file(["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", "exec",
            "--user", "999:999", identity, "/usr/bin/env", "-i", "PATH=/usr/lib/postgresql/18/bin:/usr/bin:/bin",
            "PGHOST=/var/run/postgresql", "PGUSER=lagrange", "PGCONNECT_TIMEOUT=10",
            "PGPASSFILE=/dev/null", "PGSERVICEFILE=/dev/null",
            "PGOPTIONS=-c default_transaction_read_only=on -c statement_timeout=300000 -c lock_timeout=5000",
            "/usr/bin/timeout", "--signal=TERM", "--kill-after=5s", "900s",
            "pg_dump", "--format=custom", "--dbname=" + self.binding.database], target, seconds=920)
        after = self.docker.object("container", identity)
        require((after.get("Created"), after["State"].get("StartedAt"), after.get("RestartCount")) ==
                generation and after["State"].get("Running") is True and
                not after["State"].get("OOMKilled"), "DAILY_BACKUP_DATABASE_CHANGED")

    def restore_database(self, dump, directory, *, audit_mode="daily"):
        # Mount only this freshly decrypted dump, fixed scripts and a disposable
        # data directory. This container cannot reach the live DB/network/secret.
        verifier_root = self.verifier_root or self.context.release
        require(audit_mode in ("daily", "simulation", "combined"), "DAILY_BACKUP_VERIFY_POLICY")
        self.require_no_verifier()
        require(directory.parent == Path(self.root) and dump.parent == directory,
                "DAILY_BACKUP_VERIFY_PATH")
        with install.root_directory(self.root) as parent:
            with install.child_directory(parent, directory.name, 0, 0, 0o700):
                pass
        image = self.docker.object("image", POSTGRES)
        require(POSTGRES in image.get("RepoDigests", []) and
                runtime.DIGEST.fullmatch(image.get("Id", "")), "DAILY_BACKUP_VERIFY_IMAGE")
        image_environment = runtime.environment({"Config": image["Config"]})
        data = directory / "postgres"
        data.mkdir(mode=0o700)
        os.chown(data, 999, 999)
        dump.chmod(0o444)
        audit_path = verifier_root + "/deploy/db/owner-beta-daily-backup-audit.sql"
        if audit_mode != "daily":
            daily_sql = (install.read_root_file(audit_path, maximum=65536) if audit_mode == "combined" else b"")
            content = daily_sql + b"\n" + simulation.SIMULATION_AUDIT_SQL.encode("ascii")
            with install.root_directory(str(directory)) as parent:
                install.create_file(parent, "simulation-audit.sql", content, 0, 0, 0o444)
            audit_path = str(directory / "simulation-audit.sql")
        nonce = uuid.uuid4().hex
        labels = {"io.lagrange.daily-backup-verify": nonce,
                  "io.lagrange.daily-backup-project": self.binding.project}
        args = ["create", "--name", "lagrange-backup-verify-" + nonce, "--init",
            "--restart=no", "--network=none", "--read-only", "--user=999:999",
            "--cap-drop=ALL", "--security-opt=no-new-privileges:true", "--memory=512m", "--cpus=1",
            "--tmpfs", "/tmp:rw,nosuid,nodev,size=32m,uid=999,gid=999,mode=0700",
            "--tmpfs", "/var/lib/postgresql:rw,nosuid,nodev,noexec,size=4k,mode=000",
            "--mount", "type=bind,src=" + str(dump) + ",dst=/verify.dump,readonly",
            "--mount", "type=bind,src=" + str(data) + ",dst=/restore",
            "--mount", "type=bind,src=" + verifier_root + "/deploy/db/daily-backup-verify.sh,dst=/verify.sh,readonly",
            "--mount", "type=bind,src=" + audit_path + ",dst=/verify.sql,readonly",
            "--entrypoint=/bin/bash"]
        for key, value in labels.items():
            args += ["--label", key + "=" + value]
        name = "lagrange-backup-verify-" + nonce
        intent_name = "daily-verifier-" + nonce + ".json"
        intent = files.canonical({"schema": "lagrange-daily-verifier-v1", "name": name,
            "labels": labels, "image_id": image["Id"], "image": POSTGRES,
            "staging": str(directory)})
        with install.root_directory(self.root) as parent:
            install.create_file(parent, intent_name, intent, 0, 0, 0o600)
        identity = None
        created = None

        def owned_ids():
            result = self.docker.call(["ps", "--all", "--quiet", "--no-trunc", "--filter",
                "label=io.lagrange.daily-backup-verify=" + nonce, "--filter",
                "label=io.lagrange.daily-backup-project=" + self.binding.project,
                "--filter", "name=^/" + name + "$"]).splitlines()
            require(len(result) <= 1 and all(runtime.CONTAINER.fullmatch(i) for i in result),
                    "DAILY_BACKUP_VERIFY_OWNER")
            return result

        def observe():
            value = self.docker.object("container", identity)
            require(value.get("Id") == identity and value.get("Image") == image["Id"] and
                    value.get("Name") == "/" + name and
                    value["Config"].get("Image") == POSTGRES and
                    all((value["Config"].get("Labels") or {}).get(k) == v for k, v in labels.items()) and
                    (created is None or value.get("Created") == created), "DAILY_BACKUP_VERIFY_OWNER")
            return value

        try:
            result = self.docker.call([*args, POSTGRES, "/verify.sh"]).strip()
            require(runtime.CONTAINER.fullmatch(result), "DAILY_BACKUP_VERIFY_OWNER")
            identity = result
            value = observe()
            created = value["Created"]
            host = value["HostConfig"]
            require(value["Config"].get("User") == "999:999" and
                    value["Config"].get("Entrypoint") == ["/bin/bash"] and
                    value["Config"].get("Cmd") == ["/verify.sh"] and not host.get("GroupAdd") and
                    host.get("NetworkMode") == "none" and
                    set(value.get("NetworkSettings", {}).get("Networks", {})) <= {"none"},
                    "DAILY_BACKUP_VERIFY_POLICY")
            isolated_policy(value, memory=512 * 1024**2, environment=image_environment,
                mounts={"/verify.dump": (str(dump), False), "/restore": (str(data), True),
                        "/verify.sh": (verifier_root + "/deploy/db/daily-backup-verify.sh", False),
                        "/verify.sql": (audit_path, False)},
                tmpfs={"/tmp": "rw,nosuid,nodev,size=32m,uid=999,gid=999,mode=0700",
                       "/var/lib/postgresql": "rw,nosuid,nodev,noexec,size=4k,mode=000"})
            audit = directory / "restored-audit.json"
            self.command_file(["/usr/bin/docker", "--host", "unix:///var/run/docker.sock",
                               "start", "--attach", identity], audit, seconds=1800)
            value = observe()
            require(not value["State"].get("Running") and not value["State"].get("OOMKilled") and
                    not value["State"].get("Error") and value["State"].get("ExitCode") == 0 and
                    value.get("RestartCount") == 0, "DAILY_BACKUP_VERIFY_RESULT")
            data = install.read_root_file(str(audit), gid=0, mode=0o600, maximum=64 * 1024**2)
            rows = data.splitlines()
            if audit_mode == "daily":
                require(len(rows) == 1, "DAILY_BACKUP_VERIFY_RESULT")
                return install.json_bytes(rows[0], maximum=64 * 1024**2)
            require(len(rows) == (2 if audit_mode == "combined" else 1), "DAILY_BACKUP_VERIFY_RESULT")
            digest_line = rows[-1].decode("ascii", errors="strict")
            require(digest_line in simulation._SimulationAuditResult(), "DAILY_BACKUP_VERIFY_RESULT")
            digest = digest_line.removeprefix("SIMULATION_AUDIT: ")
            return (install.json_bytes(rows[0], maximum=64 * 1024**2), digest) if audit_mode == "combined" else digest
        finally:
            if identity is None:
                recovered = owned_ids()
                identity = recovered[0] if recovered else None
            if identity is not None:
                value = observe()
                if value["State"].get("Running"):
                    self.docker.call(["kill", "--signal=TERM", identity])
                    self.docker.call(["wait", identity], timeout=90)
                    value = observe()
                require(value["State"].get("Running") is False, "DAILY_BACKUP_VERIFY_CLEANUP")
                self.docker.call(["rm", "--volumes", identity])
                require(not owned_ids(), "DAILY_BACKUP_VERIFY_CLEANUP")
                with install.root_directory(self.root) as parent:
                    require(install.read_file(parent, intent_name, 0, 0, 0o600) == intent,
                            "DAILY_BACKUP_VERIFY_OWNER")
                    os.unlink(intent_name, dir_fd=parent)
                    os.fsync(parent)
            # An empty query after a lost create reply does not prove that the
            # daemon cannot finish creation later. Keep intent and staging.

    def backup_state(self, database, bundle_digest):
        return {"schema": self.state_schema, "database": database,
                "files_sha256": bundle_digest, "config_sha256": self.binding.config_sha256}

    def check_state(self, state, bundle_digest):
        require(isinstance(state, dict) and set(state) == self.state_keys and
                state["schema"] == self.state_schema and
                state["config_sha256"] == self.binding.config_sha256 and
                bundle_digest == state["files_sha256"], "DAILY_BACKUP_STATE")

    def check_restored_binding(self, restored, state):
        pass

    @contextlib.contextmanager
    def capture(self, snapshot, database):
        timestamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        name = self.prefix + timestamp + "-" + self.context.commit
        final = Path(self.root) / name
        require(not final.exists() and not final.is_symlink(), "DAILY_BACKUP_EXISTS")
        with self.temporary(".daily-capture-") as temporary:
            plain = temporary / "plain"
            plain.mkdir(mode=0o700)
            output = temporary / "set"
            output.mkdir(mode=0o700)
            self.dump(plain / "postgres.dump")
            with (plain / "files.bundle").open("xb") as stream:
                stream_path = plain / "files.bundle"
                stream_path.chmod(0o600)
                storage = self

                class Writer:
                    def write(self, chunk):
                        storage.reserve(len(chunk))
                        return stream.write(chunk)

                snapshot.write(Writer())
                stream.flush()
                os.fsync(stream.fileno())
            state = self.backup_state(database, digest_file(plain / "files.bundle"))
            (plain / "state.json").write_bytes(files.canonical(state))
            (plain / "state.json").chmod(0o600)
            for base in ("postgres.dump", "files.bundle", "state.json"):
                self.crypt(plain / base, output / (base + ".enc"))
            metadata = {"format": self.format, "commit": self.context.commit,
                        "created_at": timestamp, "encryption": "AES-256-CBC-PBKDF2-200000",
                        "files": {member: digest_file(output / member) for member in MEMBERS}}
            (output / "metadata.json").write_bytes(files.canonical(metadata))
            (output / "metadata.json").chmod(0o600)
            self.verify(output, completed=False)
            # The controller checks source DB equality and final snapshot state
            # before publication. Return the still-private prepared tree.
            yield output, final, name

    def verify(self, source, *, completed=True):
        with install.root_directory(str(source)) as parent:
            install.directory_info(parent, 0, 0, 0o700)
            expected = {*MEMBERS, "metadata.json"} | ({"COMPLETE"} if completed else set())
            require(set(bounded_names(parent, len(expected))) == expected, "DAILY_BACKUP_MEMBERS")
            if completed:
                require(install.read_file(parent, "COMPLETE", 0, 0, 0o600) == self.format.encode() + b"\n",
                        "DAILY_BACKUP_INCOMPLETE")
            metadata = install.json_bytes(install.read_file(parent, "metadata.json", 0, 0, 0o600))
            require(isinstance(metadata, dict) and set(metadata) ==
                    {"format", "commit", "created_at", "encryption", "files"} and
                    metadata["format"] == self.format and metadata["commit"] == self.context.commit and
                    metadata["encryption"] == "AES-256-CBC-PBKDF2-200000" and
                    isinstance(metadata["files"], dict) and set(metadata["files"]) == set(MEMBERS),
                    "DAILY_BACKUP_METADATA")
            stamp = datetime.datetime.strptime(metadata["created_at"], "%Y%m%dT%H%M%SZ")
            require(stamp.strftime("%Y%m%dT%H%M%SZ") == metadata["created_at"] and
                    (not completed or source.name == self.prefix + metadata["created_at"] +
                     "-" + metadata["commit"]), "DAILY_BACKUP_METADATA")
            for member in MEMBERS:
                fd = os.open(member, files.REGULAR, dir_fd=parent)
                try:
                    install.file_info(fd, 0, 0, 0o600)
                finally:
                    os.close(fd)
                require(digest_file(source / member) == metadata["files"][member], "DAILY_BACKUP_CIPHER_DIGEST")
            with self.temporary(".daily-verify-") as temporary:
                for member in MEMBERS:
                    self.crypt(source / member, temporary / member.removesuffix(".enc"), decrypt=True)
                state = install.json_bytes(install.read_root_file(str(temporary / "state.json"),
                    gid=0, mode=0o600, maximum=64 * 1024**2), maximum=64 * 1024**2)
                self.check_state(state, digest_file(temporary / "files.bundle"))
                restored = temporary / "files"
                restored.mkdir(mode=0o700)
                with (temporary / "files.bundle").open("rb") as stream:
                    manifest = files.restore_archive(stream, self.limits, str(restored), self.reserve)
                top = {e["path"]: e for e in manifest["entries"] if "/" not in e["path"]}
                restored_roots = [files.Root(name, str(restored / name), top[name]["uid"],
                                            top[name]["gid"], top[name]["mode"]) for name in files.CLASSES]
                with files.Snapshot(restored_roots, self.limits) as restored_snapshot:
                    require(restored_snapshot.manifest() == files.canonical(manifest), "DAILY_BACKUP_RESTORED_FILES")
                actual = self.restore_database(temporary / "postgres.dump", temporary)
                require(actual == state["database"], "DAILY_BACKUP_RESTORED_DATABASE")
                self.check_restored_binding(restored, state)
                return files.database_references(manifest, actual)

    def sets(self):
        result = []
        with install.root_directory(self.root) as parent:
            with os.scandir(parent) as entries:
                for entry in entries:
                    name = entry.name
                    matched = re.fullmatch(r"backup-(?:(daily|simulation|preactivation|inactive-simulation)-)?([0-9]{8}T[0-9]{6}Z)-([0-9a-f]{40})", name)
                    if matched is None:
                        continue
                    require(len(result) < 10000, "DAILY_BACKUP_SET_LIMIT")
                    daily, simulation_set = matched[1] == "daily", matched[1] == "simulation"
                    preactivation = matched[1] == "preactivation"
                    inactive = matched[1] == "inactive-simulation"
                    allowed = {*MEMBERS, "metadata.json", "COMPLETE"} if daily or simulation_set or preactivation or inactive else {
                        "postgres.dump.enc", "raw.tar.enc", "curated.tar.enc", "manifest.sha256",
                        "metadata", "VERIFIED", "COMPLETE"}
                    with install.child_directory(parent, name, 0, 0, 0o700) as directory:
                        require(set(os.listdir(directory)) == allowed, "DAILY_BACKUP_SET_MEMBERS")
                        marker = (INACTIVE_SIMULATION_FORMAT if inactive else PREACTIVATION_FORMAT if preactivation else FORMAT if daily else
                                  SIMULATION_FORMAT if simulation_set else "LAGRANGE_PRODUCTION_BACKUP_V1")
                        require(install.read_file(directory, "COMPLETE", 0, 0, 0o600) ==
                                marker.encode() + b"\n", "DAILY_BACKUP_SET_INCOMPLETE")
                        if not daily and not simulation_set and not preactivation and not inactive:
                            verified = install.read_file(directory, "VERIFIED", 0, 0, 0o600, 64)
                            require(re.fullmatch(rb"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z\n", verified),
                                    "DAILY_BACKUP_SET_INCOMPLETE")
                        size = 0
                        for member in allowed:
                            fd = os.open(member, files.REGULAR, dir_fd=directory)
                            try:
                                size += install.file_info(fd, 0, 0, 0o600).st_size
                            finally:
                                os.close(fd)
                    stamp = datetime.datetime.strptime(matched[2], "%Y%m%dT%H%M%SZ").replace(tzinfo=datetime.timezone.utc)
                    result.append({"name": name, "daily": daily, "simulation": simulation_set, "created": stamp,
                                   "size": size, "members": allowed})
                    if preactivation:
                        result[-1]["preactivation"] = True
                    if inactive:
                        result[-1]["inactive_simulation"] = True
        return sorted(result, key=lambda value: (value["created"], value["name"]))

    def publish(self, source, final, name, *, simulation_set=False):
        require(type(simulation_set) is bool and
                (SIMULATION_SET_NAME if simulation_set else self.set_pattern).fullmatch(name) and
                final == Path(self.root) / name, "DAILY_BACKUP_PUBLISH")
        previous = self.sets()
        if previous:
            prefix = "backup-simulation-" if simulation_set else self.prefix
            stamp = name[len(prefix):len(prefix) + 16]
            require(stamp > previous[-1]["created"].strftime("%Y%m%dT%H%M%SZ"), "DAILY_BACKUP_CLOCK")
        (source / "COMPLETE").write_bytes((SIMULATION_FORMAT if simulation_set else self.format).encode() + b"\n")
        (source / "COMPLETE").chmod(0o600)
        with install.root_directory(str(source)) as directory:
            for member in (*MEMBERS, "metadata.json", "COMPLETE"):
                fd = os.open(member, files.REGULAR, dir_fd=directory)
                try:
                    install.file_info(fd, 0, 0, 0o600)
                    os.fsync(fd)
                finally:
                    os.close(fd)
            os.fsync(directory)
        with install.root_directory(str(source.parent)) as origin, \
                install.root_directory(self.root) as parent:
            install.rename_no_replace(origin, source.name, parent, name)
            os.fsync(parent)
            try:
                info = os.stat("latest", dir_fd=parent, follow_symlinks=False)
                require(stat.S_ISLNK(info.st_mode) and info.st_uid == 0 and
                        re.fullmatch(r"backup-(?:(daily|simulation|preactivation|inactive-simulation)-)?[0-9]{8}T[0-9]{6}Z-[0-9a-f]{40}",
                                     os.readlink("latest", dir_fd=parent)), "DAILY_BACKUP_LATEST")
            except FileNotFoundError:
                pass
            temporary = ".latest-daily-" + uuid.uuid4().hex
            os.symlink(name, temporary, dir_fd=parent)
            try:
                os.replace(temporary, "latest", src_dir_fd=parent, dst_dir_fd=parent)
                os.fsync(parent)
            finally:
                try:
                    os.unlink(temporary, dir_fd=parent)
                except FileNotFoundError:
                    pass

    def prune(self):
        sets = self.sets()
        now = datetime.datetime.now(datetime.timezone.utc)
        while len(sets) > self.values["MIN_KEEP"]:
            total = sum(s["size"] for s in sets)
            first = sets[0]
            age = (now - first["created"]).total_seconds() / 86400
            if total <= self.values["MAX_TOTAL_BYTES"] and age <= self.values["RETENTION_DAYS"]:
                break
            with install.root_directory(self.root) as parent:
                # Reinspect the fixed flat set; never recursively delete caller paths.
                with install.child_directory(parent, first["name"], 0, 0, 0o700) as directory:
                    require(set(os.listdir(directory)) == first["members"], "DAILY_BACKUP_PRUNE_CHANGED")
                    for member in first["members"]:
                        fd = os.open(member, files.REGULAR, dir_fd=directory)
                        try:
                            install.file_info(fd, 0, 0, 0o600)
                        finally:
                            os.close(fd)
                    for member in first["members"]:
                        os.unlink(member, dir_fd=directory)
                os.rmdir(first["name"], dir_fd=parent)
                os.fsync(parent)
            sets.pop(0)
        total = sum(s["size"] for s in sets)
        require(total <= self.values["MAX_TOTAL_BYTES"], "DAILY_BACKUP_RETENTION_CAP")
        return len(sets), total

    def metrics(self, count, total, duration):
        target = Path(self.values["METRICS_FILE"])
        data = (f"lagrange_backup_last_success_timestamp_seconds {int(time.time())}\n"
                f"lagrange_backup_duration_seconds {int(duration)}\n"
                f"lagrange_backup_total_bytes {total}\n"
                f"lagrange_backup_retained_sets {count}\n").encode("ascii")
        with install.root_directory(str(target.parent)) as parent:
            try:
                install.read_file(parent, target.name, 0, 0, 0o600, 4096)
            except FileNotFoundError:
                pass
            name = ".backup-metrics-" + uuid.uuid4().hex
            install.create_file(parent, name, data, 0, 0, 0o600)
            os.replace(name, target.name, src_dir_fd=parent, dst_dir_fd=parent)
            os.fsync(parent)


@dataclasses.dataclass(frozen=True)
class SimulationStorageBinding:
    project: str
    database: str


def validate_simulation_state(state, binding, application_commit):
    config = simulation.release.validate_config(binding.config_data, application_commit)
    require(type(state) is dict and set(state) == {
        "schema", "profile", "application_commit", "tool_commit", "tool_manifest_sha256",
        "manifest_sha256", "config_sha256", "receipt_sha256", "database_sha256",
        "daily_database", "files_sha256"} and state["schema"] == "lagrange-simulation-backup-state-v1"
        and state["profile"] in ("simulation", "daily-simulation")
        and state["application_commit"] == application_commit and install.is_commit(state["tool_commit"])
        and state["manifest_sha256"] == config["manifest_sha256"] and state["config_sha256"] == binding.config_sha256
        and state["receipt_sha256"] == binding.receipt_sha256 and install.is_hash(state["database_sha256"])
        and install.is_hash(state["files_sha256"]) and
        (state["tool_manifest_sha256"] is None or install.is_hash(state["tool_manifest_sha256"])),
        "DAILY_BACKUP_STATE")
    combined = state["profile"] == "daily-simulation"
    require((type(state["daily_database"]) is dict) if combined else (state["daily_database"] is None),
            "DAILY_BACKUP_STATE")
    return combined


class SimulationStorage(Storage):
    """Same encrypted members and verifier, with a distinct simulation format."""
    format = SIMULATION_FORMAT
    prefix = "backup-simulation-"
    set_pattern = SIMULATION_SET_NAME

    def augment_simulation_state(self, state):
        return state

    def check_simulation_state(self, state):
        return validate_simulation_state(state, self.simulation_binding, self.context.commit)

    def __init__(self, values, docker, context, binding, *, tool_commit, tool_hash, verifier_root=None):
        require(bool(context.values["POSTGRES_DB"]), "DAILY_BACKUP_DATABASE")
        database = ops.database_name(context.values)
        super().__init__(values, docker, context, SimulationStorageBinding(binding.project, database),
                         verifier_root=verifier_root)
        self.simulation_binding = binding
        self.tool_commit = tool_commit
        self.tool_hash = tool_hash
        require(install.is_commit(tool_commit) and (tool_hash is None or install.is_hash(tool_hash)),
                "DAILY_BACKUP_TOOL")

    @contextlib.contextmanager
    def capture_simulation(self, snapshot, digest, daily_database, profile):
        require(profile in ("simulation", "daily-simulation") and install.is_hash(digest),
                "DAILY_BACKUP_SIMULATION_STATE")
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        name = self.prefix + stamp + "-" + self.context.commit
        final = Path(self.root) / name
        require(not final.exists() and not final.is_symlink(), "DAILY_BACKUP_EXISTS")
        with self.temporary(".simulation-capture-") as temporary:
            plain, output = temporary / "plain", temporary / "set"
            plain.mkdir(mode=0o700)
            output.mkdir(mode=0o700)
            self.dump(plain / "postgres.dump")
            with (plain / "files.bundle").open("xb") as stream:
                (plain / "files.bundle").chmod(0o600)
                storage = self
                class Writer:
                    def write(self, chunk):
                        storage.reserve(len(chunk))
                        return stream.write(chunk)
                snapshot.write(Writer())
                stream.flush()
                os.fsync(stream.fileno())
            config = simulation.release.validate_config(self.simulation_binding.config_data, self.context.commit)
            state = {"schema": "lagrange-simulation-backup-state-v1", "profile": profile,
                     "application_commit": self.context.commit, "tool_commit": self.tool_commit,
                     "tool_manifest_sha256": self.tool_hash, "manifest_sha256": config["manifest_sha256"],
                     "config_sha256": self.simulation_binding.config_sha256,
                     "receipt_sha256": self.simulation_binding.receipt_sha256,
                     "database_sha256": digest, "daily_database": daily_database,
                     "files_sha256": digest_file(plain / "files.bundle")}
            state = self.augment_simulation_state(state)
            (plain / "state.json").write_bytes(files.canonical(state))
            (plain / "state.json").chmod(0o600)
            for member in ("postgres.dump", "files.bundle", "state.json"):
                self.crypt(plain / member, output / (member + ".enc"))
            metadata = {"format": self.format, "commit": self.context.commit, "created_at": stamp,
                        "encryption": "AES-256-CBC-PBKDF2-200000",
                        "files": {member: digest_file(output / member) for member in MEMBERS}}
            (output / "metadata.json").write_bytes(files.canonical(metadata))
            (output / "metadata.json").chmod(0o600)
            self.verify_simulation(output, completed=False)
            yield output, final, name

    def verify_simulation(self, source, *, completed=True):
        with install.root_directory(str(source)) as parent:
            install.directory_info(parent, 0, 0, 0o700)
            expected = {*MEMBERS, "metadata.json"} | ({"COMPLETE"} if completed else set())
            require(set(bounded_names(parent, len(expected))) == expected, "DAILY_BACKUP_MEMBERS")
            if completed:
                require(install.read_file(parent, "COMPLETE", 0, 0, 0o600) == self.format.encode() + b"\n",
                        "DAILY_BACKUP_INCOMPLETE")
            meta = install.json_bytes(install.read_file(parent, "metadata.json", 0, 0, 0o600))
            require(type(meta) is dict and set(meta) == {"format", "commit", "created_at", "encryption", "files"}
                    and meta["format"] == self.format and meta["commit"] == self.context.commit
                    and meta["encryption"] == "AES-256-CBC-PBKDF2-200000" and type(meta["files"]) is dict
                    and set(meta["files"]) == set(MEMBERS), "DAILY_BACKUP_METADATA")
            stamp = datetime.datetime.strptime(meta["created_at"], "%Y%m%dT%H%M%SZ")
            require(stamp.strftime("%Y%m%dT%H%M%SZ") == meta["created_at"] and
                    (not completed or source.name == self.prefix + meta["created_at"] + "-" + meta["commit"]),
                    "DAILY_BACKUP_METADATA")
            for member in MEMBERS:
                fd = os.open(member, files.REGULAR, dir_fd=parent)
                try:
                    install.file_info(fd, 0, 0, 0o600)
                finally:
                    os.close(fd)
                require(digest_file(source / member) == meta["files"][member], "DAILY_BACKUP_CIPHER_DIGEST")
            with self.temporary(".simulation-verify-") as temporary:
                for member in MEMBERS:
                    self.crypt(source / member, temporary / member.removesuffix(".enc"), decrypt=True)
                state = install.json_bytes(install.read_root_file(str(temporary / "state.json"),
                    gid=0, mode=0o600, maximum=64 * 1024**2), maximum=64 * 1024**2)
                combined = self.check_simulation_state(state)
                require(digest_file(temporary / "files.bundle") == state["files_sha256"], "DAILY_BACKUP_STATE")
                source_tool_hash = state["tool_manifest_sha256"]
                if source_tool_hash is None:
                    require(state["tool_commit"] == self.context.commit, "DAILY_BACKUP_TOOL")
                else:
                    require(install.is_hash(source_tool_hash), "DAILY_BACKUP_TOOL")
                    root = Path(self.context.release).parent.parent
                    tool = local("simulation_backup_history_tool", "production-backup-tool.py")
                    tool.read_manifest(root / "maintenance" / state["tool_commit"], self.context.commit,
                                       install_root=root, expected_hash=source_tool_hash)
                with (temporary / "files.bundle").open("rb") as stream:
                    manifest = simulation.verify_simulation_archive(stream, self.limits, self.simulation_binding,
                                                                    profile=state["profile"])
                restored = temporary / "files"
                restored.mkdir(mode=0o700)
                with (temporary / "files.bundle").open("rb") as stream:
                    restored_manifest = files.restore_archive(stream, self.limits, str(restored), self.reserve,
                                                              profile=state["profile"])
                require(restored_manifest == manifest, "DAILY_BACKUP_RESTORED_FILES")
                top = {e["path"]: e for e in manifest["entries"] if "/" not in e["path"]}
                classes = files.profile_values(state["profile"])[0]
                restored_roots = [files.Root(name, str(restored / name), top[name]["uid"], top[name]["gid"],
                                            top[name]["mode"]) for name in classes]
                with files.Snapshot(restored_roots, self.limits, profile=state["profile"]) as restored_snapshot:
                    require(restored_snapshot.manifest() == files.canonical(manifest), "DAILY_BACKUP_RESTORED_FILES")
                actual = self.restore_database(temporary / "postgres.dump", temporary,
                                               audit_mode="combined" if combined else "simulation")
                if combined:
                    daily_database, sim_digest = actual
                    require(daily_database == state["daily_database"], "DAILY_BACKUP_RESTORED_DATABASE")
                    files.database_references(manifest, daily_database)
                else:
                    sim_digest = actual
                require(sim_digest == state["database_sha256"], "DAILY_BACKUP_RESTORED_DATABASE")
                self.check_restored_binding(restored, state)
                return state


def bound_database(database, config):
    control = database.get("control", {})
    require(control.get("enabled") is False and control.get("owner_user_id") == config["owner"] and
            control.get("policy_sha256") == config["policy_sha256"] and
            control.get("first_auto_session") == config["first_auto_session"] and
            control.get("trigger_version") == "owner-beta-daily-trigger-v1", "DAILY_BACKUP_CONTROL_BINDING")


def preactivation_dates(expected_dates):
    require(isinstance(expected_dates, tuple) and len(expected_dates) == 2 and
            all(isinstance(d, str) and re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", d)
                for d in expected_dates), "PREACTIVATION_DATES")
    try:
        dates = tuple(datetime.date.fromisoformat(d) for d in expected_dates)
    except ValueError:
        raise runtime.RuntimeErrorCode("PREACTIVATION_DATES") from None
    require(datetime.date(2026, 8, 28) < dates[0] < dates[1], "PREACTIVATION_DATES")
    return expected_dates


def preactivation_date_binding(database, config, expected_dates):
    """Preserve an explicitly pinned discrepancy; never repair or activate it."""
    preactivation_dates(expected_dates)
    require(isinstance(database, dict) and set(database) == {
        "schema", "control", "tables", "run_count", "admissions", "verified_frontiers"} and
        database["schema"] == "lagrange-daily-backup-db-v1", "PREACTIVATION_DATABASE")
    control = database.get("control", {})
    require(isinstance(control, dict) and control.get("enabled") is False and
            type(control.get("activation_epoch")) is int and control["activation_epoch"] == 0 and
            control.get("owner_user_id") == config["owner"] and
            control.get("policy_sha256") == config["policy_sha256"] and
            control.get("trigger_version") == "owner-beta-daily-trigger-v1" and
            control.get("first_auto_session") == expected_dates[0] and
            config["first_auto_session"] == expected_dates[1] and
            type(database.get("run_count")) is int and database["run_count"] == 0 and
            database.get("admissions") == [] and database.get("verified_frontiers") == [],
            "PREACTIVATION_DATABASE")
    tables = database.get("tables")
    require(isinstance(tables, list) and len(tables) == len(files.AUDIT_TABLES),
            "PREACTIVATION_DATABASE")
    names = set()
    for row in tables:
        require(isinstance(row, dict) and set(row) == {"name", "row_count", "sha256"} and
                row["name"] in files.AUDIT_TABLES and row["name"] not in names and
                type(row["row_count"]) is int and 0 <= row["row_count"] <= 1100000 and
                install.is_hash(row["sha256"]), "PREACTIVATION_DATABASE")
        names.add(row["name"])
        if row["name"] == "owner_beta_daily_control":
            require(row["row_count"] == 1, "PREACTIVATION_DATABASE")
        elif row["name"] in files.DAILY_TABLES and row["name"] != "owner_beta_daily_poll_state":
            require(row["row_count"] == 0, "PREACTIVATION_HISTORY")
    return {"schema": "lagrange-preactivation-date-binding-v1",
            "database_first_auto_session": expected_dates[0],
            "configured_first_auto_session": expected_dates[1]}


PREACTIVATION_SCHEMA_SQL = """BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL statement_timeout = '30s';
SET LOCAL lock_timeout = '5s';
SET LOCAL search_path = pg_catalog, pg_temp;
DO $audit$
BEGIN
 IF current_user <> session_user OR current_user <> 'migration_owner'
    OR pg_catalog.row_security_active('public.users'::regclass)
    OR (SELECT count(*) FROM public._sqlx_migrations) <> 62
    OR (SELECT count(*) FROM public._sqlx_migrations WHERE version BETWEEN 1 AND 62 AND success) <> 62
    OR pg_catalog.to_regclass('public.owner_beta_simulation_runs') IS NOT NULL
    OR pg_catalog.to_regclass('public.owner_beta_simulation_result_pairs') IS NOT NULL
    OR (SELECT count(*) FROM (SELECT 1 FROM public.users LIMIT 10001) bounded) > 10000 THEN
   RAISE EXCEPTION 'PREACTIVATION_SCHEMA';
 END IF;
END $audit$;
SELECT 'PREACTIVATION_SCHEMA: PRE_R4';
ROLLBACK;
"""


def preactivation_schema(database, values):
    require(database.execute(values, PREACTIVATION_SCHEMA_SQL, {"PREACTIVATION_SCHEMA: PRE_R4"}) ==
            "PREACTIVATION_SCHEMA: PRE_R4" and not simulation.history_present(database, values),
            "PREACTIVATION_SCHEMA")


def preactivation_stopped_cohort(operation):
    # A legacy sealed worker can belong to an earlier immutable release. Its
    # separate trusted manifest supplies ownership, never a mutable label alone.
    own, docker = operation.ownership, operation.docker
    ops.global_cohort(docker, own)
    snapshot = runtime.capture(docker, own)
    result = {}
    for service, entry in snapshot.items():
        value = runtime.unchanged(docker, own, service, entry, 15)
        state = value["State"]
        require(not state.get("Running") and not state.get("Restarting") and
                not state.get("Paused") and not state.get("OOMKilled") and not state.get("Error") and
                state.get("Status") in {"created", "exited"} and state.get("ExitCode") == 0 and
                value["HostConfig"].get("RestartPolicy") == {"Name": "no", "MaximumRetryCount": 0},
                "PREACTIVATION_NOT_STOPPED")
        result[service] = tuple(entry["generation"])
    return result


class PreactivationStorage(Storage):
    """Distinct checkpoint format; ordinary Daily verification rejects it."""
    format = PREACTIVATION_FORMAT
    prefix = "backup-preactivation-"
    set_pattern = PREACTIVATION_SET_NAME
    state_schema = "lagrange-preactivation-backup-state-v1"
    state_keys = Storage.state_keys | {"date_binding"}

    def __init__(self, *args, config, expected_dates, **kwargs):
        super().__init__(*args, **kwargs)
        self.config, self.expected_dates = config, expected_dates

    def backup_state(self, database, bundle_digest):
        state = super().backup_state(database, bundle_digest)
        state["date_binding"] = preactivation_date_binding(database, self.config, self.expected_dates)
        return state

    def check_state(self, state, bundle_digest):
        super().check_state(state, bundle_digest)
        require(state["date_binding"] == preactivation_date_binding(
            state["database"], self.config, self.expected_dates), "PREACTIVATION_STATE")

    def check_restored_binding(self, restored, state):
        raw = install.read_root_file(str(restored / "config/config.json"), gid=10001,
                                     mode=0o440, maximum=install.CONFIG_LIMIT)
        config = install.validate_config(raw, self.context.commit)
        require("sha256:" + hashlib.sha256(raw).hexdigest() == state["config_sha256"] and
                preactivation_date_binding(state["database"], config, self.expected_dates) ==
                state["date_binding"], "PREACTIVATION_RESTORED_BINDING")


class InactiveDailySimulationStorage(SimulationStorage):
    """Full R4 history plus an unactivated Daily with two retained date pins."""
    format = INACTIVE_SIMULATION_FORMAT
    prefix = "backup-inactive-simulation-"
    set_pattern = INACTIVE_SIMULATION_SET_NAME

    def __init__(self, *args, daily_config, daily_config_sha256, expected_dates, **kwargs):
        super().__init__(*args, **kwargs)
        require(install.is_hash(daily_config_sha256), "PREACTIVATION_CONFIG")
        self.daily_config = daily_config
        self.daily_config_sha256 = daily_config_sha256
        self.expected_dates = expected_dates

    def augment_simulation_state(self, state):
        require(state["profile"] == "daily-simulation", "PREACTIVATION_SIMULATION_PROFILE")
        return dict(state, schema="lagrange-inactive-daily-simulation-state-v1",
            date_binding=preactivation_date_binding(state["daily_database"], self.daily_config,
                                                    self.expected_dates),
            daily_config_sha256=self.daily_config_sha256)

    def check_simulation_state(self, state):
        require(type(state) is dict and state.get("schema") ==
                "lagrange-inactive-daily-simulation-state-v1" and
                state.get("daily_config_sha256") == self.daily_config_sha256 and
                state.get("profile") == "daily-simulation", "PREACTIVATION_STATE")
        regular = {k: v for k, v in state.items() if k not in ("date_binding", "daily_config_sha256")}
        regular["schema"] = "lagrange-simulation-backup-state-v1"
        require(validate_simulation_state(regular, self.simulation_binding, self.context.commit) and
                state.get("date_binding") == preactivation_date_binding(
                    state["daily_database"], self.daily_config, self.expected_dates), "PREACTIVATION_STATE")
        return True

    def check_restored_binding(self, restored, state):
        raw = install.read_root_file(str(restored / "config/config.json"), gid=10001,
                                     mode=0o440, maximum=install.CONFIG_LIMIT)
        config = install.validate_config(raw, self.context.commit)
        require("sha256:" + hashlib.sha256(raw).hexdigest() == self.daily_config_sha256 and
                preactivation_date_binding(state["daily_database"], config, self.expected_dates) ==
                state["date_binding"], "PREACTIVATION_RESTORED_BINDING")


SIMULATION_HOST_CONTROLLER_COMPATIBILITY = frozenset({
    # Immutable eb48/839 application controller -> reviewed host Compose repair.
    ("f9121455b7df37c0bf872e4c43404659778d73350f5a70c33acb620dbf404851",
     "1cdd70f0701443a1c442f74ca1f05223dd5efd01f636923633f4b0fafe6cff36"),
})


def inactive_simulation_tool(context, verifier_root):
    # This distinct entrypoint is the adopted two-date implementation. Every
    # other Python controller must match the immutable R4 application exactly.
    if verifier_root is None:
        return
    tool = local("inactive_simulation_package", "production-backup-tool.py")
    names = sorted(n for n in tool.REQUIRED if n.startswith("scripts/ops/lib/") and n.endswith(".py")
                   and n != "scripts/ops/lib/owner-beta-daily-backup.py")
    require(names, "PREACTIVATION_TOOL")
    for name in names:
        a = install.read_root_file(context.release + "/" + name, maximum=tool.MAX_FILE)
        b = install.read_root_file(verifier_root + "/" + name, maximum=tool.MAX_FILE)
        compatible = (name == "scripts/ops/lib/owner-beta-simulation-operations.py" and
                      (hashlib.sha256(a).hexdigest(), hashlib.sha256(b).hexdigest()) in
                      SIMULATION_HOST_CONTROLLER_COMPATIBILITY)
        require(a == b or compatible, "PREACTIVATION_TOOL_DEPENDENCY")


def inactive_simulation_run(operation, context, database, docker, sim_docker, backup, *,
                            tool_commit, tool_hash, verifier_root, action, expected_dates,
                            stopped_worker_commit):
    require(operation.required and operation.binding is not None and
            operation.source_release == operation.target_release, "PREACTIVATION_SIMULATION_CONFIG")
    inactive_simulation_tool(context, verifier_root)
    binding, config = simulation_daily_binding(context, operation.project, operation.install_root)
    require(binding is not None and install.is_commit(stopped_worker_commit), "PREACTIVATION_DAILY")
    prior_images = ops.read_manifest(operation.install_root + "/releases/" + stopped_worker_commit,
                                    stopped_worker_commit)
    ownership = ops.Ownership(operation.project, stopped_worker_commit, prior_images["owner-beta-runner"])
    daily_operation = ops.Operations(docker, database, context, context, ownership,
                                     operation.install_root, "absent")
    cohort = preactivation_stopped_cohort(daily_operation)
    storage = InactiveDailySimulationStorage(backup, docker, context, operation.binding,
        tool_commit=tool_commit, tool_hash=tool_hash, verifier_root=verifier_root,
        daily_config=config, daily_config_sha256=binding.config_sha256, expected_dates=expected_dates)
    require(backup["METRICS_FILE"].startswith(backup["BACKUP_ROOT"] + "/"), "PREACTIVATION_OUTPUT")
    storage.require_no_verifier()
    require(all(s.get("inactive_simulation") for s in storage.sets()), "PREACTIVATION_SEPARATE_ROOT")

    def daily_audit():
        require(preactivation_stopped_cohort(daily_operation) == cohort, "PREACTIVATION_COHORT_CHANGED")
        daily = database.backup_audit(context.values)
        preactivation_date_binding(daily, config, expected_dates)
        return daily

    def audit():
        return daily_audit(), simulation.simulation_audit(database, context.values)

    daily_before = daily_audit()  # All inactive-Daily conditions precede any signal.
    if action == "inactive-simulation-verify":
        ids = simulation._api_ids(docker, 15)
        require(len(ids) == 1, "PREACTIVATION_API")
        api, running, policy = simulation._api_observe(docker, operation.project, context.commit,
            context.images["api-server"], ids[0], 15)
        worker = simulation.runtime.capture(sim_docker, operation.binding)
        require(not running and policy == "no" and not worker.running, "PREACTIVATION_NOT_DRAINED")
        before = audit()
    else:
        require(action == "inactive-simulation-backup", "PREACTIVATION_ARGUMENTS")
        process = simulation.runtime.capture(sim_docker, operation.binding)
        if process.running:
            simulation.verify_runtime_policy(docker, sim_docker, operation.binding,
                operation.values["LAGRANGE_RUNTIME_SECRET_DIR"], ops.database_name(operation.values))
        # Retain ownership before signaling, using the same journal protocol
        # as an explicit release drain. A later explicit release-start can
        # validate this exact stopped generation; backup never resumes it.
        record, _ = simulation._capture_fencing(operation, docker, sim_docker)
        simulation._pending_write(operation.install_root, simulation.HANDOFF_PENDING, record)
        record = simulation._retry_fencing(operation, database, docker, sim_docker, record)
        simulation._handoff_stopped(operation, database, docker, sim_docker, record)
        api = simulation.ApiGeneration(*record["api_generation"])
        worker = simulation.runtime.capture(sim_docker, operation.binding)
        require((None if worker.generation is None else list(worker.generation)) ==
                record["worker_generation"] and not worker.running, "PREACTIVATION_NOT_DRAINED")
        updated = dict(record, phase="switched")
        simulation._pending_write(operation.install_root, simulation.HANDOFF_PENDING, updated,
                                  previous=record)
        require(api is not None and worker is not None, "PREACTIVATION_NOT_DRAINED")
        # Draining may complete a running simulation; observe its final history.
        before = audit()
    require(before[0] == daily_before, "PREACTIVATION_DAILY_CHANGED")

    def unchanged():
        require(audit() == before, "DAILY_BACKUP_SOURCE_DATABASE_CHANGED")
        simulation.verify_fence(docker, operation.project, context.commit,
                                context.images["api-server"], api, 15)
        simulation.verify_drained(sim_docker, operation.binding, worker.generation)

    if action == "inactive-simulation-verify":
        sets = storage.sets()
        require(sets, "PREACTIVATION_NO_COMPLETE_SET")
        storage.verify_simulation(Path(storage.root) / sets[-1]["name"])
        unchanged()
        return "INACTIVE_SIMULATION_VERIFIED_ISOLATED_DISABLED"
    started = time.monotonic()
    with contextlib.ExitStack() as staging:
        with publication_fence(binding, config):
            with simulation_snapshot(operation.binding, operation.install_root, storage.limits,
                    data_root=backup["DATA_ROOT"], daily_roots=roots(binding, operation.install_root)) as snapshot:
                require(audit() == before, "DAILY_BACKUP_SOURCE_DATABASE_CHANGED")
                files.database_references(json.loads(snapshot.manifest()), before[0])
                source, final, name = staging.enter_context(storage.capture_simulation(
                    snapshot, before[1], before[0], "daily-simulation"))
                unchanged()
        unchanged()
        storage.publish(source, final, name)
        sets = storage.sets()
        count, total = len(sets), sum(s["size"] for s in sets)
        require(total <= storage.values["MAX_TOTAL_BYTES"], "PREACTIVATION_RETENTION_LIMIT")
    storage.metrics(count, total, time.monotonic() - started)
    return "INACTIVE_SIMULATION_VERIFIED_DISABLED"


def reverify_once(docker, binding, config):
    """One native admission cycle. The caller must audit actual identities after it.

    This does not reset either poll clock or create replacement jobs. Busy,
    NotDue, Pending, or a remaining orphan never counts as reconciliation.
    """
    runtime.image_owned(docker, binding)
    image_environment = runtime.environment({"Config": docker.object("image", binding.image_id)["Config"]})
    require(set(image_environment) <= {"PATH", "LANG"}, "DAILY_BACKUP_RECONCILE_ENV")
    require(not docker.call(["ps", "--all", "--quiet", "--filter",
                "label=io.lagrange.daily-backup-reconcile-project=" + binding.project]).strip(),
            "DAILY_BACKUP_RECONCILE_ALREADY_PRESENT")
    network = ops.internal_network(docker, binding.project)
    with install.root_directory(binding.secret_root) as parent:
        with install.child_directory(parent, "owner-beta-daily-admitter", 10003, 10003, 0o750) as service:
            secret = install.read_file(service, "db_daily_input_admitter_password", 10003, 10003, 0o440, 8192)
            require(secret and not any(c in secret for c in (b"\0", b"\r", b"\n")), "DAILY_BACKUP_RECONCILE_SECRET")
    nonce = uuid.uuid4().hex
    labels = {"io.lagrange.daily-backup-reconcile": nonce,
              "io.lagrange.daily-backup-reconcile-project": binding.project,
              "org.opencontainers.image.revision": binding.commit}
    args = ["create", "--name", "lagrange-backup-reconcile-" + nonce, "--init", "--restart=no",
        "--read-only", "--user=10003:10003", "--group-add=10001", "--cap-drop=ALL",
        "--security-opt=no-new-privileges:true", "--memory=1g", "--cpus=1", "--log-driver=none",
        "--network", network["Id"], "--tmpfs", "/run/owner-beta-health:uid=10003,gid=10003,mode=0750",
        "--entrypoint", runtime.BINARY]
    for key, value in labels.items():
        args += ["--label", key + "=" + value]
    expected = binding.mounts(runtime.SERVICES[1])
    for target, (source, writable) in expected.items():
        require("," not in source, "DAILY_BACKUP_PATH")
        args += ["--mount", "type=bind,src=" + source + ",dst=" + target +
                 ("" if writable else ",readonly")]
    environment = {
        "APP_ENV": "production", "LAGRANGE_CODE_COMMIT": binding.commit,
        "PRICE_BETA_DAILY_MODE": "owner_only",
        "PRICE_BETA_DAILY_CONFIG_FILE": "/run/owner-beta-daily/config.json",
        "PRICE_BETA_DAILY_CONFIG_SHA256": binding.config_sha256,
        "PRICE_BETA_HEALTH_STATE_PATH": "/run/owner-beta-health/health.json",
        "PRICE_BETA_HEALTH_MAX_AGE_SECS": "180", "DB_HOST": "postgres", "DB_PORT": "5432",
        "DB_NAME": binding.database, "DB_USER": "daily_input_admitter",
        "DB_PASSWORD_FILE": "/run/secrets/db_daily_input_admitter_password",
    }
    for key, value in environment.items():
        args += ["--env", key + "=" + value]
    identity = docker.call([*args, binding.image_id, "daily-admitter", "--once"]).strip()
    require(runtime.CONTAINER.fullmatch(identity), "DAILY_BACKUP_RECONCILE_OWNER")
    created = None

    def observe():
        value = docker.object("container", identity)
        require(value.get("Id") == identity and value.get("Image") == binding.image_id and
                all((value["Config"].get("Labels") or {}).get(k) == v for k, v in labels.items()) and
                (created is None or value.get("Created") == created), "DAILY_BACKUP_RECONCILE_OWNER")
        return value

    try:
        value = observe()
        created = value["Created"]
        host = value["HostConfig"]
        require(value["Config"].get("User") == "10003:10003" and
                value["Config"].get("Entrypoint") == [runtime.BINARY] and
                value["Config"].get("Cmd") == ["daily-admitter", "--once"] and
                host.get("ReadonlyRootfs") is True and host.get("Privileged") is False and
                host.get("CapDrop") == ["ALL"] and host.get("GroupAdd") == ["10001"] and
                host.get("NetworkMode") in {network["Id"], network["Name"]}, "DAILY_BACKUP_RECONCILE_POLICY")
        isolated_policy(value, memory=1024**3, mounts=expected,
            tmpfs={"/run/owner-beta-health": "uid=10003,gid=10003,mode=0750"},
            environment={**image_environment, **environment})
        networks = value.get("NetworkSettings", {}).get("Networks", {})
        require(len(networks) == 1 and next(iter(networks.values())).get("NetworkID") == network["Id"] and
                host.get("LogConfig", {}).get("Type") == "none", "DAILY_BACKUP_RECONCILE_NETWORK")
        docker.call(["start", identity])
        result = docker.call(["wait", identity], timeout=config["max_cycle_seconds"] + 70).splitlines()
        value = observe()
        require(result == ["0"] and not value["State"].get("Running") and
                value["State"].get("ExitCode") == 0 and not value["State"].get("OOMKilled") and
                not value["State"].get("Error") and value.get("RestartCount") == 0,
                "DAILY_BACKUP_RECONCILE_EXECUTION")
    finally:
        value = observe()
        if value["State"].get("Running"):
            docker.call(["kill", "--signal=TERM", identity])
            docker.call(["wait", identity], timeout=90)
            value = observe()
        require(not value["State"].get("Running"), "DAILY_BACKUP_RECONCILE_CLEANUP")
        docker.call(["rm", identity])


class Controller:
    def __init__(self, operations, binding, config, storage, install_root):
        self.operations, self.binding, self.config = operations, binding, config
        self.storage, self.install_root = storage, install_root

    def inventory(self):
        return files.Snapshot(roots(self.binding, self.install_root), self.storage.limits)

    def audit(self):
        value = self.operations.database.backup_audit(self.operations.target.values)
        bound_database(value, self.config)
        return value

    def quiesce(self):
        operation, binding = self.operations, self.binding
        ops.global_cohort(operation.docker, binding)
        before = runtime.capture(operation.docker, binding)
        state = operation.database.status(operation.target.values)
        require(state in {"ENABLED", "DISABLED"}, "DAILY_BACKUP_DATABASE_STATE")
        resume, api = False, None
        if state == "ENABLED":
            try:
                healthy = runtime.verify_running(operation.docker, binding)
                require(ops.snapshot_equal(before, healthy), "DAILY_RUNTIME_GENERATION")
                api = ops.verify_api(operation.docker, binding, operation.target.images["api-server"])
                resume = True
            except runtime.RuntimeErrorCode:
                pass
        operation.disable(before)
        operation.database.control(operation.target.release, operation.target.commit,
                                   operation.target.values, "check-disabled")
        return before, resume, api

    def fail_off(self):
        self.operations.cleanup_target(self.binding)

    def backup(self):
        started = time.monotonic()
        try:
            before, resume, api = self.quiesce()
            with contextlib.ExitStack() as staging:
                with publication_fence(self.binding, self.config):
                    with self.inventory() as snapshot:
                        database = self.audit()
                        pending = files.database_references(json.loads(snapshot.manifest()), database)
                        # A restorable snapshot can contain a sealed orphan.
                        # It cannot authorize resuming daily scheduling.
                        resume = resume and not any(pending.values())
                        source, final, name = staging.enter_context(self.storage.capture(snapshot, database))
                        require(database == self.audit(), "DAILY_BACKUP_SOURCE_DATABASE_CHANGED")
                    # Snapshot __exit__ verified exact bytes/names before the
                    # publication fence is released. Staging remains private.
                self.storage.publish(source, final, name)
                count, total = self.storage.prune()
            if resume:
                require(api == ops.verify_api(self.operations.docker, self.binding,
                                              self.operations.target.images["api-server"]),
                        "DAILY_BACKUP_API_CHANGED")
                running = runtime.resume_after_backup(self.operations.docker, self.binding, before)
                require(api == ops.verify_api(self.operations.docker, self.binding,
                                              self.operations.target.images["api-server"]),
                        "DAILY_BACKUP_API_CHANGED")
                self.operations.database.control(self.operations.target.release, self.operations.target.commit,
                                                  self.operations.target.values, "enable")
                require(ops.snapshot_equal(running, runtime.verify_running(self.operations.docker, self.binding)),
                        "DAILY_BACKUP_RESUME_CHANGED")
            self.storage.metrics(count, total, time.monotonic() - started)
            return "VERIFIED_AND_RESUMED" if resume else "VERIFIED_DISABLED"
        except BaseException:
            self.fail_off()
            raise

    def reconcile(self):
        # Maintenance never auto-enables. Permanent-key repair is allowed only
        # while every owned worker is stopped; later failure leaves it OFF.
        try:
            self.quiesce()
            with publication_fence(self.binding, self.config):
                with self.inventory() as snapshot:
                    self.operations.database.restore_reservations(self.operations.target.values)
                    database = self.audit()
                    pending = files.database_references(json.loads(snapshot.manifest()), database)
            expected_keys = {r["admission_sha256"] for r in database["admissions"]} | set(pending["extra_admissions"])
            if pending["extra_admissions"]:
                # Release the admitter's private lock before the native process
                # acquires it. No producer or worker is restarted here.
                reverify_once(self.operations.docker, self.binding, self.config)
            with publication_fence(self.binding, self.config):
                with self.inventory() as snapshot:
                    after = self.audit()
                    pending = files.database_references(json.loads(snapshot.manifest()), after)
                    require(not pending["extra_admissions"] and not pending["extra_frontiers"],
                            "DAILY_BACKUP_RECONCILE_PENDING")
                    require(expected_keys == {r["admission_sha256"] for r in after["admissions"]},
                            "DAILY_BACKUP_RECONCILE_IDENTITY")
            return "RECONCILED_DISABLED"
        except BaseException:
            self.fail_off()
            raise


class PreactivationController(Controller):
    """Observe an already OFF cohort; never invoke source repair or resume."""
    def __init__(self, *args, expected_dates, **kwargs):
        super().__init__(*args, **kwargs)
        self.expected_dates, self.cohort = expected_dates, None

    def audit(self):
        require(preactivation_stopped_cohort(self.operations) == self.cohort,
                "PREACTIVATION_COHORT_CHANGED")
        preactivation_schema(self.operations.database, self.operations.target.values)
        value = self.operations.database.backup_audit(self.operations.target.values)
        preactivation_date_binding(value, self.config, self.expected_dates)
        return value

    def quiesce(self):
        self.cohort = preactivation_stopped_cohort(self.operations)
        self.audit()

    def backup(self):
        # Deliberately separate from Controller.backup: no disable/repair/enable
        # or failure cleanup of the source is permitted for this operation.
        started = time.monotonic()
        self.quiesce()
        with contextlib.ExitStack() as staging:
            with publication_fence(self.binding, self.config):
                with self.inventory() as snapshot:
                    database = self.audit()
                    files.database_references(json.loads(snapshot.manifest()), database)
                    source, final, name = staging.enter_context(self.storage.capture(snapshot, database))
                    require(database == self.audit(), "DAILY_BACKUP_SOURCE_DATABASE_CHANGED")
            require(database == self.audit(), "DAILY_BACKUP_SOURCE_DATABASE_CHANGED")
            self.storage.publish(source, final, name)
            sets = self.storage.sets()
            count, total = len(sets), sum(s["size"] for s in sets)
            require(total <= self.storage.values["MAX_TOTAL_BYTES"], "PREACTIVATION_RETENTION_LIMIT")
        self.storage.metrics(count, total, time.monotonic() - started)
        return "PREACTIVATION_VERIFIED_DISABLED"


def preactivation_run(context, docker, backup, *, operation, install_root,
                      expected_dates, stopped_worker_commit, project, research_mode, verifier_root):
    simulation._no_pending(install_root)
    require(not simulation._floor_present(install_root) and
            not simulation._configured(context.release, context.commit) and
            simulation.runtime.Docker().service_ids(15) == [], "PREACTIVATION_SIMULATION")
    require(install.is_commit(stopped_worker_commit), "PREACTIVATION_WORKER_SOURCE")
    images = ops.read_manifest(install_root + "/releases/" + stopped_worker_commit,
                               stopped_worker_commit)
    own = ops.Ownership(project, stopped_worker_commit, images["owner-beta-runner"])
    binding, config = ops.activation_binding(context.release, context.commit,
                                             project, context.images, context.values)
    require(config["raw_owner_uid"] == 10001 and
            backup["METRICS_FILE"].startswith(backup["BACKUP_ROOT"] + "/"),
            "PREACTIVATION_OUTPUT")
    roots(binding, install_root)
    database = ops.Database(docker, project, context.commit, context.images["db-migrate"])
    operations = ops.Operations(docker, database, context, context, own, install_root, research_mode)
    storage = PreactivationStorage(backup, docker, context, binding, config=config,
        expected_dates=expected_dates, verifier_root=verifier_root)
    storage.require_no_verifier()
    sets = storage.sets()
    require(all(s.get("preactivation") is True for s in sets), "PREACTIVATION_SEPARATE_ROOT")
    controller = PreactivationController(operations, binding, config, storage, install_root,
                                        expected_dates=expected_dates)
    if operation == "preactivation-backup":
        return controller.backup()
    require(operation == "preactivation-verify" and sets, "PREACTIVATION_NO_COMPLETE_SET")
    controller.quiesce()
    before = controller.audit()
    storage.verify(Path(storage.root) / sets[-1]["name"])
    require(controller.audit() == before, "DAILY_BACKUP_SOURCE_DATABASE_CHANGED")
    return "PREACTIVATION_VERIFIED_ISOLATED_DISABLED"


class Arguments(argparse.ArgumentParser):
    def error(self, message):
        raise runtime.RuntimeErrorCode("DAILY_BACKUP_ARGUMENTS")


def verify_disabled_state(operation, binding, config):
    """Enable gate: no missing references, orphan identities or repair gaps.

    The caller already holds the release lock and has disabled/drained daily.
    This never repairs or starts anything. Reconciliation is a separate command.
    """
    with publication_fence(binding, config):
        with files.Snapshot(roots(binding, operation.install_root), files.Limits(2**50)) as snapshot:
            database = operation.database.backup_audit(operation.target.values)
            bound_database(database, config)
            pending = files.database_references(json.loads(snapshot.manifest()), database)
            require(not any(pending.values()), "DAILY_BACKUP_RECONCILE_REQUIRED")


def simulation_daily_binding(context, project, install_root):
    """Retain Daily history even when its current runtime mode is OFF."""
    values = context.values
    exists = (simulation._present(values["LAGRANGE_DATA_DIR"], "owner-beta-daily") or
              simulation._present(install_root, install.FLOOR_NAME) or
              simulation._present(context.release, ".owner-beta-daily"))
    if not exists:
        return None, None
    config, digest = install.verify_config(context.release, context.commit)
    require(simulation._present(install_root, install.FLOOR_NAME), "DAILY_BACKUP_READER")
    binding = runtime.Binding(project, context.commit, context.images["owner-beta-runner"], digest,
        context.release, values["LAGRANGE_DATA_DIR"], values["LAGRANGE_ARTIFACTS_DIR"],
        values["LAGRANGE_RUNTIME_SECRET_DIR"], ops.database_name(values), values["MAX_JOBS_PER_OWNER"],
        values["PRICE_BETA_LEASE_SECS"], values["PRICE_BETA_BACKOFF_SECS"])
    roots(binding, install_root)
    return binding, config


def simulation_backup_run(operation, context, database, docker, sim_docker, backup, *, tool_commit,
                          tool_hash, verifier_root, action):
    """Capture/verify only the fixed simulation-aware format, leaving services OFF."""
    require(operation.required and operation.binding is not None and
            operation.source_release == operation.target_release, "DAILY_BACKUP_SIMULATION_CONFIG")
    storage = SimulationStorage(backup, docker, context, operation.binding, tool_commit=tool_commit,
                                tool_hash=tool_hash, verifier_root=verifier_root)
    storage.require_no_verifier()
    if action == "verify":
        sets = [entry for entry in storage.sets() if entry["simulation"]]
        require(sets, "DAILY_BACKUP_NO_COMPLETE_SET")
        name = sets[-1]["name"]
        source_commit = name[-40:]
        if source_commit != context.commit:
            source_release = operation.install_root + "/releases/" + source_commit
            source_images = ops.read_manifest(source_release, source_commit)
            source_values, source_overlays = ops.read_context(source_release, source_commit, "disable")
            source_context = ops.Context(source_release, source_commit, source_images, source_values, source_overlays)
            simulation.release.reader_guard(operation.install_root, source_release, source_commit,
                                             "disabled", history_present=True, reserve=False)
            simulation.verify_receipt(source_release, source_commit)
            source_binding = simulation.binding_from_release(operation.project, source_release, source_commit)
            storage = SimulationStorage(backup, docker, source_context, source_binding, tool_commit=tool_commit,
                                        tool_hash=tool_hash, verifier_root=verifier_root)
        storage.verify_simulation(Path(storage.root) / name)
        return "SIMULATION_VERIFIED_ISOLATED"
    require(action == "backup", "DAILY_BACKUP_ARGUMENTS")
    started = time.monotonic()
    process = simulation.runtime.capture(sim_docker, operation.binding)
    if process.running:
        simulation.verify_runtime_policy(docker, sim_docker, operation.binding,
                                         operation.values["LAGRANGE_RUNTIME_SECRET_DIR"],
                                         ops.database_name(operation.values))
    api, worker = simulation.prepare_transition(operation, database, docker, sim_docker)
    require(api is not None and worker is not None, "DAILY_BACKUP_SIMULATION_STATE")
    simulation.release.reader_guard(operation.install_root, operation.target_release, operation.target_commit,
                                    "disabled", history_present=True, reserve=True)
    daily_binding, daily_config = simulation_daily_binding(context, operation.project, operation.install_root)
    if daily_binding is not None:
        _, research_mode = ops.project_binding(context.commit, context.commit)
        daily_operation = ops.Operations(docker, database, context, context, daily_binding,
                                         operation.install_root, research_mode)
        ops.global_cohort(docker, daily_binding)
        cohort = runtime.capture(docker, daily_binding)
        daily_operation.disable(cohort)
        database.control(context.release, context.commit, context.values, "check-disabled")
        def verify_daily_stopped():
            ops.global_cohort(docker, daily_binding)
            after = runtime.capture(docker, daily_binding)
            require(ops.snapshot_equal(cohort, after), "DAILY_BACKUP_SOURCE_CHANGED")
            ops.stopped(docker, daily_binding, after)
        verify_daily_stopped()
    profile = "daily-simulation" if daily_binding is not None else "simulation"
    daily_roots = roots(daily_binding, operation.install_root) if daily_binding is not None else None
    fence = publication_fence(daily_binding, daily_config) if daily_binding is not None else \
        raw_publication_fence(backup["DATA_ROOT"])
    with contextlib.ExitStack() as staging:
        with fence:
            with simulation_snapshot(operation.binding, operation.install_root, storage.limits,
                                     data_root=backup["DATA_ROOT"], daily_roots=daily_roots) as snapshot:
                digest = simulation.simulation_audit(database, context.values)
                daily_database = database.backup_audit(context.values) if daily_binding is not None else None
                if daily_binding is not None:
                    bound_database(daily_database, daily_config)
                    files.database_references(json.loads(snapshot.manifest()), daily_database)
                source, final, name = staging.enter_context(
                    storage.capture_simulation(snapshot, digest, daily_database, profile))
                require(simulation.simulation_audit(database, context.values) == digest,
                        "DAILY_BACKUP_SOURCE_DATABASE_CHANGED")
                if daily_binding is not None:
                    require(database.backup_audit(context.values) == daily_database,
                            "DAILY_BACKUP_SOURCE_DATABASE_CHANGED")
            simulation.verify_drained(sim_docker, operation.binding, worker.generation)
            simulation.verify_fence(docker, operation.project, operation.source_commit,
                                    operation.source_images["api-server"], api, 15)
            if daily_binding is not None:
                verify_daily_stopped()
                database.control(context.release, context.commit, context.values, "check-disabled")
        storage.publish(source, final, name, simulation_set=True)
        count, total = storage.prune()
    storage.metrics(count, total, time.monotonic() - started)
    return "SIMULATION_VERIFIED_DISABLED"


def main(argv=None):
    parser = Arguments(description=__doc__)
    parser.add_argument("--operation", choices=("backup", "verify", "reconcile",
                        "preactivation-backup", "preactivation-verify",
                        "inactive-simulation-backup", "inactive-simulation-verify"), required=True)
    parser.add_argument("--mode", choices=("plan", "preflight", "apply"), required=True)
    parser.add_argument("--install-root", required=True)
    parser.add_argument("--application-release")
    parser.add_argument("--config-file", default="/etc/lagrange/production-backup.conf")
    parser.add_argument("--expected-db-first-session")
    parser.add_argument("--expected-config-first-session")
    parser.add_argument("--stopped-worker-from-commit")
    previous = {}
    preactivation = False
    try:
        args = parser.parse_args(argv)
        preactivation = args.operation.startswith(("preactivation-", "inactive-simulation-"))
        expected_dates = (args.expected_db_first_session, args.expected_config_first_session)
        if preactivation:
            preactivation_dates(expected_dates)
            require(install.is_commit(args.stopped_worker_from_commit), "PREACTIVATION_WORKER_SOURCE")
        else:
            require(expected_dates == (None, None) and args.stopped_worker_from_commit is None,
                    "PREACTIVATION_ARGUMENTS")
        if args.mode == "plan":
            if args.operation.startswith("inactive-simulation-"):
                print("INACTIVE_SIMULATION_PLAN: preserve pinned inactive Daily dates and full R4 history; "
                      "exact API/worker drain; separate encrypted checkpoint and isolated restore; no Daily control write or resume")
                return 0
            print("PREACTIVATION_BACKUP_PLAN: preserve both pinned dates; already OFF and unactivated; "
                  "separate encrypted checkpoint; networkless restore; no source mutation or resume"
                  if preactivation else
                  "DAILY_BACKUP_PLAN: disabled consistent capture; encrypted full state; "
                  "networkless restore; same-identity reconciliation; exact prior-cohort resume")
            return 0
        install.require_root()
        os.umask(0o077)
        release = args.application_release or ROOT
        commit = Path(release).name
        require(install.is_commit(commit) and runtime.absolute(args.install_root), "DAILY_BACKUP_RELEASE")
        ops.policy.validate_current_release(args.install_root, release, commit)
        tool_hash = None
        if args.application_release:
            tool = local("daily_backup_tool", "production-backup-tool.py")
            tool_hash = tool.read_manifest(ROOT, commit, install_root=Path(args.install_root))
        images = ops.read_manifest(release, commit)
        project, research_mode = ops.project_binding(commit, commit)
        values, overlays = ops.read_context(release, commit, "enable")
        context = ops.Context(release, commit, images, values, overlays)
        ops.reject_ambient(values)
        backup = config_file(args.config_file, context, project, check_only=args.mode == "preflight")
        require((tool_hash is None and not (TOOL_KEYS & backup.keys())) or
                (tool_hash is not None and backup.get("DAILY_BACKUP_TOOL_COMMIT") == Path(ROOT).name
                 and backup.get("DAILY_BACKUP_TOOL_MANIFEST") == tool_hash), "DAILY_BACKUP_TOOL")
        if args.mode == "preflight":
            sim_config = simulation._configured(release, commit)
            sim_floor = simulation._floor_present(args.install_root)
            require(not args.operation.startswith("preactivation-") or not (sim_config or sim_floor),
                    "PREACTIVATION_SIMULATION")
            require(not args.operation.startswith("inactive-simulation-") or sim_config,
                    "PREACTIVATION_SIMULATION_CONFIG")
            if sim_config or sim_floor:
                require(sim_config, "DAILY_BACKUP_SIMULATION_CONFIG")
                simulation.release.reader_guard(args.install_root, release, commit, "disabled",
                                                 history_present=True, reserve=False)
            else:
                binding, config = ops.activation_binding(release, commit, project, images, values)
                require(config["raw_owner_uid"] == 10001, "DAILY_BACKUP_RAW_OWNER")
                roots(binding, args.install_root)
                install.check_state(binding.data_root, config)
            print("DAILY_BACKUP: PREFLIGHT")
            return 0
        def terminated(signum, frame):
            raise runtime.RuntimeErrorCode("DAILY_BACKUP_TERMINATED")
        for signum in (signal.SIGTERM, signal.SIGINT):
            previous[signum] = signal.signal(signum, terminated)
        with ops.release_lock(args.install_root), backup_lock(backup["LOCK_FILE"]):
            ops.policy.validate_current_release(args.install_root, release, commit)
            if tool_hash is not None:
                tool.read_manifest(ROOT, commit, install_root=Path(args.install_root),
                                   expected_hash=tool_hash)
            docker = runtime.Docker()
            if args.operation.startswith("preactivation-"):
                result = preactivation_run(context, docker, backup, operation=args.operation,
                    install_root=args.install_root, expected_dates=expected_dates,
                    stopped_worker_commit=args.stopped_worker_from_commit,
                    project=project, research_mode=research_mode,
                    verifier_root=ROOT if tool_hash is not None else None)
            else:
                sim_docker = simulation.runtime.Docker()
                sim_context, database = simulation.inspect_transition(args.install_root, release, commit,
                                                                      docker, sim_docker)
                if args.operation.startswith("inactive-simulation-"):
                    result = inactive_simulation_run(sim_context, context, database, docker, sim_docker, backup,
                        tool_commit=Path(ROOT).name, tool_hash=tool_hash, verifier_root=ROOT if tool_hash else None,
                        action=args.operation, expected_dates=expected_dates,
                        stopped_worker_commit=args.stopped_worker_from_commit)
                elif sim_context.required:
                    result = simulation_backup_run(sim_context, context, database, docker, sim_docker, backup,
                        tool_commit=Path(ROOT).name, tool_hash=tool_hash,
                        verifier_root=ROOT if tool_hash is not None else None, action=args.operation)
                else:
                    result = daily_backup_run(context, docker, database, backup, args, research_mode,
                                              project=project,
                                              verifier_root=ROOT if tool_hash is not None else None)
            ops.policy.validate_current_release(args.install_root, release, commit)
            if tool_hash is not None:
                tool.read_manifest(ROOT, commit, install_root=Path(args.install_root),
                                   expected_hash=tool_hash)
        print("DAILY_BACKUP: " + result)
        return 0
    except Exception as exc:
        # No source paths, owner IDs, SQL, provider bodies or secrets in output.
        if preactivation and isinstance(exc, (runtime.RuntimeErrorCode, files.SnapshotError)):
            code = str(exc)
            if re.fullmatch(r"(?:PREACTIVATION|DAILY_BACKUP|DAILY_OPERATION|DAILY_RUNTIME)_[A-Z0-9_]{1,100}", code):
                print("PREACTIVATION_BACKUP: FAILED " + code, file=sys.stderr)
        print("DAILY_BACKUP: FAILED", file=sys.stderr)
        return 1
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)


def daily_backup_run(context, docker, database, backup, args, research_mode, *, project, verifier_root):
    release, commit = context.release, context.commit
    images, values = context.images, context.values
    binding, config = ops.activation_binding(release, commit, project, images, values)
    require(config["raw_owner_uid"] == 10001, "DAILY_BACKUP_RAW_OWNER")
    roots(binding, args.install_root)
    operations = ops.Operations(docker, database, context, context, binding,
                                args.install_root, research_mode)
    storage = Storage(backup, docker, context, binding, verifier_root=verifier_root)
    storage.require_no_verifier()
    controller = Controller(operations, binding, config, storage, args.install_root)
    if args.operation == "verify":
        sets = [s for s in storage.sets() if s["daily"]]
        require(sets, "DAILY_BACKUP_NO_COMPLETE_SET")
        storage.verify(Path(storage.root) / sets[-1]["name"])
        return "VERIFIED_ISOLATED"
    if args.operation == "backup":
        return controller.backup()
    ops.validate_activation_inputs(context)
    return controller.reconcile()


if __name__ == "__main__":
    raise SystemExit(main())
