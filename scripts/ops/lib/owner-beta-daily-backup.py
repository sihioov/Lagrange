#!/usr/bin/env python3
"""Installed-release daily backup and guarded restore reconciliation.

Only compose-release calls this controller. Plan is non-mutating; preflight
reads protected local inputs only. Apply requires the release and backup locks.
No provider request, account API or arbitrary Compose target is available.
"""
from __future__ import annotations

import argparse
import contextlib
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
install, runtime = ops.install, ops.runtime
require = runtime.require
ROOT = str(Path(__file__).absolute().parents[3])
POSTGRES = "postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a"
FORMAT = "LAGRANGE_DAILY_BACKUP_V2"
CONFIG_KEYS = frozenset(("BACKUP_ROOT", "DATA_ROOT", "COMPOSE_FILE", "COMPOSE_ENV_FILE",
    "COMPOSE_PROJECT", "LAGRANGE_CODE_COMMIT", "KEY_FILE", "LOCK_FILE", "METRICS_FILE",
    "MAX_TOTAL_BYTES", "MIN_FREE_BYTES", "RETENTION_DAYS", "MIN_KEEP",
    "POSTGRES_SERVICE", "POSTGRES_IMAGE"))
SET_NAME = re.compile(r"backup-daily-[0-9]{8}T[0-9]{6}Z-[0-9a-f]{40}")
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
        require(sep and key in CONFIG_KEYS and key not in values and value and
                not any(c.isspace() or ord(c) < 32 or ord(c) == 127 for c in value),
                "DAILY_BACKUP_CONFIG")
        values[key] = value
    require(set(values) == CONFIG_KEYS and values["COMPOSE_PROJECT"] == project and
            values["LAGRANGE_CODE_COMMIT"] == context.commit and
            values["DATA_ROOT"] == context.values["LAGRANGE_DATA_DIR"] and
            values["COMPOSE_FILE"] == context.release + "/deploy/compose/compose.yml" and
            values["COMPOSE_ENV_FILE"] == context.release + "/deploy/compose/.env" and
            values["POSTGRES_SERVICE"] == "postgres" and values["POSTGRES_IMAGE"] == POSTGRES,
            "DAILY_BACKUP_BINDING")
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
def publication_fence(binding, config):
    """Keep private process locks and all existing Raw publication locks.

    New Raw control directories or uncommitted preparation are not hidden: the
    complete snapshot's before/after names and bytes still have to match.
    """
    install.check_state(binding.data_root, config)
    with contextlib.ExitStack() as stack:
        daily = stack.enter_context(install.root_directory(binding.data_root + "/owner-beta-daily"))
        for name, (stage, uid) in install.STAGES.items():
            descriptor = stack.enter_context(install.child_directory(daily, name, uid, uid, 0o700))
            stack.enter_context(install.state_lock(descriptor, uid))
            install.validate_checkpoint(install.read_file(descriptor, "checkpoint.json", uid, uid, 0o600),
                                        stage, config)
        raw = stack.enter_context(files.open_root(
            files.Root("raw", binding.data_root + "/raw", 10001, 10001, 0o750)))
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


class Storage:
    """Private staging, bounded encrypted sets and actual isolated restore."""
    def __init__(self, values, docker, context, binding):
        self.values, self.docker, self.context, self.binding = values, docker, context, binding
        self.root = values["BACKUP_ROOT"]
        self.limits = files.Limits(values["MAX_TOTAL_BYTES"])

    def reserve(self, size=0):
        require(size >= 0 and shutil.disk_usage(self.root).free >=
                self.values["MIN_FREE_BYTES"] + size, "DAILY_BACKUP_SPACE")

    @contextlib.contextmanager
    def temporary(self, prefix):
        self.reserve()
        with install.root_directory(self.root):
            name = tempfile.mkdtemp(prefix=prefix, dir=self.root)
            os.chmod(name, 0o700)
            try:
                yield Path(name)
            finally:
                # Only this freshly created, privately owned staging tree.
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
        # The pinned server's local socket is the existing backup authority.
        # Both the server-side command and local client have finite deadlines.
        self.command_file(["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", "exec",
            "--user", "999:999", identity, "/usr/bin/env", "-i", "PATH=/usr/lib/postgresql/18/bin:/usr/bin:/bin",
            "PGHOST=/var/run/postgresql", "PGUSER=postgres", "PGCONNECT_TIMEOUT=10",
            "PGPASSFILE=/dev/null", "PGSERVICEFILE=/dev/null",
            "PGOPTIONS=-c default_transaction_read_only=on -c statement_timeout=300000 -c lock_timeout=5000",
            "/usr/bin/timeout", "--signal=TERM", "--kill-after=5s", "900s",
            "pg_dump", "--format=custom", "--dbname=" + self.binding.database], target, seconds=920)
        after = self.docker.object("container", identity)
        require((after.get("Created"), after["State"].get("StartedAt"), after.get("RestartCount")) ==
                generation and after["State"].get("Running") is True and
                not after["State"].get("OOMKilled"), "DAILY_BACKUP_DATABASE_CHANGED")

    def restore_database(self, dump, directory):
        # Mount only this freshly decrypted dump, fixed scripts and a disposable
        # data directory. This container cannot reach the live DB/network/secret.
        image = self.docker.object("image", POSTGRES)
        require(POSTGRES in image.get("RepoDigests", []) and
                runtime.DIGEST.fullmatch(image.get("Id", "")), "DAILY_BACKUP_VERIFY_IMAGE")
        image_environment = runtime.environment({"Config": image["Config"]})
        data = directory / "postgres"
        data.mkdir(mode=0o700)
        os.chown(data, 999, 999)
        dump.chmod(0o444)
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
            "--mount", "type=bind,src=" + self.context.release + "/deploy/db/daily-backup-verify.sh,dst=/verify.sh,readonly",
            "--mount", "type=bind,src=" + self.context.release + "/deploy/db/owner-beta-daily-backup-audit.sql,dst=/verify.sql,readonly",
            "--entrypoint=/bin/bash"]
        for key, value in labels.items():
            args += ["--label", key + "=" + value]
        identity = self.docker.call([*args, POSTGRES, "/verify.sh"]).strip()
        require(runtime.CONTAINER.fullmatch(identity), "DAILY_BACKUP_VERIFY_OWNER")
        created = None

        def observe():
            value = self.docker.object("container", identity)
            require(value.get("Id") == identity and value.get("Image") == image["Id"] and
                    value["Config"].get("Image") == POSTGRES and
                    all((value["Config"].get("Labels") or {}).get(k) == v for k, v in labels.items()) and
                    (created is None or value.get("Created") == created), "DAILY_BACKUP_VERIFY_OWNER")
            return value

        try:
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
                        "/verify.sh": (self.context.release + "/deploy/db/daily-backup-verify.sh", False),
                        "/verify.sql": (self.context.release + "/deploy/db/owner-beta-daily-backup-audit.sql", False)},
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
            require(len(data.splitlines()) == 1, "DAILY_BACKUP_VERIFY_RESULT")
            return install.json_bytes(data, maximum=64 * 1024**2)
        finally:
            value = observe()
            if value["State"].get("Running"):
                self.docker.call(["kill", "--signal=TERM", identity])
                self.docker.call(["wait", identity], timeout=90)
                value = observe()
            require(not value["State"].get("Running"), "DAILY_BACKUP_VERIFY_CLEANUP")
            self.docker.call(["rm", identity])

    @contextlib.contextmanager
    def capture(self, snapshot, database):
        timestamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        name = "backup-daily-" + timestamp + "-" + self.context.commit
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
            state = {"schema": "lagrange-daily-backup-state-v1", "database": database,
                     "files_sha256": digest_file(plain / "files.bundle"),
                     "config_sha256": self.binding.config_sha256}
            (plain / "state.json").write_bytes(files.canonical(state))
            (plain / "state.json").chmod(0o600)
            for base in ("postgres.dump", "files.bundle", "state.json"):
                self.crypt(plain / base, output / (base + ".enc"))
            metadata = {"format": FORMAT, "commit": self.context.commit,
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
                require(install.read_file(parent, "COMPLETE", 0, 0, 0o600) == FORMAT.encode() + b"\n",
                        "DAILY_BACKUP_INCOMPLETE")
            metadata = install.json_bytes(install.read_file(parent, "metadata.json", 0, 0, 0o600))
            require(isinstance(metadata, dict) and set(metadata) ==
                    {"format", "commit", "created_at", "encryption", "files"} and
                    metadata["format"] == FORMAT and metadata["commit"] == self.context.commit and
                    metadata["encryption"] == "AES-256-CBC-PBKDF2-200000" and
                    isinstance(metadata["files"], dict) and set(metadata["files"]) == set(MEMBERS),
                    "DAILY_BACKUP_METADATA")
            stamp = datetime.datetime.strptime(metadata["created_at"], "%Y%m%dT%H%M%SZ")
            require(stamp.strftime("%Y%m%dT%H%M%SZ") == metadata["created_at"] and
                    (not completed or source.name == "backup-daily-" + metadata["created_at"] +
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
                require(isinstance(state, dict) and set(state) ==
                        {"schema", "database", "files_sha256", "config_sha256"} and
                        state["schema"] == "lagrange-daily-backup-state-v1" and
                        state["config_sha256"] == self.binding.config_sha256 and
                        digest_file(temporary / "files.bundle") == state["files_sha256"],
                        "DAILY_BACKUP_STATE")
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
                return files.database_references(manifest, actual)

    def sets(self):
        result = []
        with install.root_directory(self.root) as parent:
            with os.scandir(parent) as entries:
                for entry in entries:
                    name = entry.name
                    matched = re.fullmatch(r"backup-(daily-)?([0-9]{8}T[0-9]{6}Z)-([0-9a-f]{40})", name)
                    if matched is None:
                        continue
                    require(len(result) < 10000, "DAILY_BACKUP_SET_LIMIT")
                    daily = matched[1] is not None
                    allowed = {*MEMBERS, "metadata.json", "COMPLETE"} if daily else {
                        "postgres.dump.enc", "raw.tar.enc", "curated.tar.enc", "manifest.sha256",
                        "metadata", "VERIFIED", "COMPLETE"}
                    with install.child_directory(parent, name, 0, 0, 0o700) as directory:
                        require(set(os.listdir(directory)) == allowed, "DAILY_BACKUP_SET_MEMBERS")
                        marker = FORMAT if daily else "LAGRANGE_PRODUCTION_BACKUP_V1"
                        require(install.read_file(directory, "COMPLETE", 0, 0, 0o600) ==
                                marker.encode() + b"\n", "DAILY_BACKUP_SET_INCOMPLETE")
                        if not daily:
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
                    result.append({"name": name, "daily": daily, "created": stamp,
                                   "size": size, "members": allowed})
        return sorted(result, key=lambda value: (value["created"], value["name"]))

    def publish(self, source, final, name):
        require(SET_NAME.fullmatch(name) and final == Path(self.root) / name, "DAILY_BACKUP_PUBLISH")
        previous = self.sets()
        if previous:
            stamp = name[len("backup-daily-"):len("backup-daily-") + 16]
            require(stamp > previous[-1]["created"].strftime("%Y%m%dT%H%M%SZ"), "DAILY_BACKUP_CLOCK")
        (source / "COMPLETE").write_bytes(FORMAT.encode() + b"\n")
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
                        re.fullmatch(r"backup-(daily-)?[0-9]{8}T[0-9]{6}Z-[0-9a-f]{40}",
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


def bound_database(database, config):
    control = database.get("control", {})
    require(control.get("enabled") is False and control.get("owner_user_id") == config["owner"] and
            control.get("policy_sha256") == config["policy_sha256"] and
            control.get("first_auto_session") == config["first_auto_session"] and
            control.get("trigger_version") == "owner-beta-daily-trigger-v1", "DAILY_BACKUP_CONTROL_BINDING")


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
            secret = install.read_file(service, "db_daily_input_admitter_password", 10003, 10003, 0o400, 8192)
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


def main(argv=None):
    parser = Arguments(description=__doc__)
    parser.add_argument("--operation", choices=("backup", "verify", "reconcile"), required=True)
    parser.add_argument("--mode", choices=("plan", "preflight", "apply"), required=True)
    parser.add_argument("--install-root", required=True)
    parser.add_argument("--config-file", default="/etc/lagrange/production-backup.conf")
    previous = {}
    try:
        args = parser.parse_args(argv)
        if args.mode == "plan":
            print("DAILY_BACKUP_PLAN: disabled consistent capture; encrypted full state; "
                  "networkless restore; same-identity reconciliation; exact prior-cohort resume")
            return 0
        install.require_root()
        os.umask(0o077)
        commit = Path(ROOT).name
        require(install.is_commit(commit) and runtime.absolute(args.install_root), "DAILY_BACKUP_RELEASE")
        ops.policy.validate_current_release(args.install_root, ROOT, commit)
        images = ops.read_manifest(ROOT, commit)
        project, research_mode = ops.project_binding(commit, commit)
        values, overlays = ops.read_context(ROOT, commit, "enable")
        context = ops.Context(ROOT, commit, images, values, overlays)
        binding, config = ops.activation_binding(ROOT, commit, project, images, values)
        ops.reject_ambient(values)
        backup = config_file(args.config_file, context, project, check_only=args.mode == "preflight")
        require(config["raw_owner_uid"] == 10001, "DAILY_BACKUP_RAW_OWNER")
        roots(binding, args.install_root)
        if args.mode == "preflight":
            install.check_state(binding.data_root, config)
            print("DAILY_BACKUP: PREFLIGHT")
            return 0
        def terminated(signum, frame):
            raise runtime.RuntimeErrorCode("DAILY_BACKUP_TERMINATED")
        for signum in (signal.SIGTERM, signal.SIGINT):
            previous[signum] = signal.signal(signum, terminated)
        with ops.release_lock(args.install_root), backup_lock(backup["LOCK_FILE"]):
            ops.policy.validate_current_release(args.install_root, ROOT, commit)
            docker = runtime.Docker()
            database = ops.Database(docker, project, commit, images["db-migrate"])
            operations = ops.Operations(docker, database, context, context, binding,
                                        args.install_root, research_mode)
            storage = Storage(backup, docker, context, binding)
            controller = Controller(operations, binding, config, storage, args.install_root)
            if args.operation == "verify":
                sets = [s for s in storage.sets() if s["daily"]]
                require(sets, "DAILY_BACKUP_NO_COMPLETE_SET")
                storage.verify(Path(storage.root) / sets[-1]["name"])
                result = "VERIFIED_ISOLATED"
            elif args.operation == "backup":
                result = controller.backup()
            else:
                ops.validate_activation_inputs(context)
                result = controller.reconcile()
            ops.policy.validate_current_release(args.install_root, ROOT, commit)
        print("DAILY_BACKUP: " + result)
        return 0
    except Exception:
        # No source paths, owner IDs, SQL, provider bodies or secrets in output.
        print("DAILY_BACKUP: FAILED", file=sys.stderr)
        return 1
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)


if __name__ == "__main__":
    raise SystemExit(main())
