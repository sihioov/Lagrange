#!/usr/bin/env python3
"""Cold pre-upgrade recovery primitives; no production activation entry point.

The maintenance coordinator must separately bind a fresh all-writer shutdown,
the stopped PostgreSQL container/volume, the old release, and its migration
checksums. These primitives never stop/start a production container. A copied
cluster is booted only in a new networkless, disposable container.
"""
from __future__ import annotations

import contextlib
import hashlib
import importlib.util
import os
from pathlib import Path
import re
import shutil
import sys
import tempfile
import uuid

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    "preupgrade_backup_storage", Path(__file__).with_name("owner-beta-daily-backup.py"))
if spec is None or spec.loader is None:
    raise RuntimeError("PREUPGRADE_SOURCE")
backup = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = backup
spec.loader.exec_module(backup)
files, install, runtime = backup.files, backup.install, backup.runtime
require = runtime.require
PROFILE = "preupgrade"
PGROOT = "postgres/18/docker"
VERSION = PGROOT + "/PG_VERSION"
REQUIRED_DIRECTORIES = (
    "base", "global", "pg_wal", "pg_xact", "pg_multixact", "pg_subtrans", "pg_tblspc")
FORBIDDEN_LEAVES = (
    "postmaster.pid", "backup_label", "tablespace_map", "standby.signal", "recovery.signal")
AUDIT_KEYS = frozenset(("schema", "database", "server_version_num", "system_identifier",
                        "migrations", "catalog_sha256"))


def cluster_manifest(manifest, version):
    """Require an entire PG18 primary volume, including WAL and role catalogs.

    This establishes layout and bytes only. A missing pid file does not prove
    shutdown; the caller still needs actual container state and pg_controldata.
    """
    require(manifest.get("schema") == "lagrange-preupgrade-files-v1" and
            manifest.get("classes") == list(files.PREUPGRADE_CLASSES), "PREUPGRADE_PROFILE")
    entries = {entry["path"]: entry for entry in manifest["entries"]}
    require(len(entries) == len(manifest["entries"]) and version == b"18\n",
            "PREUPGRADE_CLUSTER_VERSION")
    for path in ("postgres", "postgres/18", PGROOT, *(PGROOT + "/" + d for d in REQUIRED_DIRECTORIES)):
        require(path in entries and entries[path]["kind"] == "directory", "PREUPGRADE_CLUSTER_LAYOUT")
    # The official root-started entrypoint can create PGDATA as root:root,
    # then chown only its user to postgres. With mode0700 its root group has
    # no access. Preserve that exact private layout without changing owners;
    # group access still requires the existing postgres:postgres0750 layout.
    require((entries[PGROOT]["uid"], entries[PGROOT]["gid"], entries[PGROOT]["mode"]) in
            {(999, 999, 0o700), (999, 999, 0o750), (999, 0, 0o700)},
            "PREUPGRADE_CLUSTER_OWNER")
    for path in (VERSION, PGROOT + "/global/pg_control"):
        require(path in entries and entries[path]["kind"] == "file" and
                (entries[path]["uid"], entries[path]["gid"]) == (999, 999) and
                entries[path]["size"] > 0, "PREUPGRADE_CLUSTER_LAYOUT")
    require(entries[VERSION]["size"] == len(version) and entries[VERSION]["sha256"] ==
            "sha256:" + hashlib.sha256(version).hexdigest(), "PREUPGRADE_CLUSTER_VERSION")
    for path in entries:
        if not path.startswith("postgres/"):
            continue
        require(path == "postgres/18" or path == PGROOT or path.startswith(PGROOT + "/"),
                "PREUPGRADE_CLUSTER_EXTRA")
        require(path not in {PGROOT + "/" + name for name in FORBIDDEN_LEAVES} and
                not path.startswith(PGROOT + "/pg_tblspc/"), "PREUPGRADE_CLUSTER_RECOVERY_STATE")
    return entries[PGROOT + "/global/pg_control"]["sha256"]


def stopped_container(value, *, identity, image_id, volume, mountpoint, generation):
    """Bind real Docker observations, including shutdown and restart history."""
    require(runtime.CONTAINER.fullmatch(identity) and value.get("Id") == identity and
            runtime.DIGEST.fullmatch(image_id) and value.get("Image") == image_id and
            value.get("Config", {}).get("Image") == backup.POSTGRES,
            "PREUPGRADE_CONTAINER_IDENTITY")
    state = value.get("State", {})
    require(state.get("Status") == "exited" and state.get("Running") is False and
            state.get("Paused") is False and state.get("Restarting") is False and
            state.get("Dead") is False and state.get("ExitCode") == 0 and
            state.get("OOMKilled") is False and not state.get("Error") and state.get("Pid") == 0,
            "PREUPGRADE_DATABASE_NOT_STOPPED")
    require(type(state.get("ExitCode")) is int and type(state.get("Pid")) is int and
            type(value.get("RestartCount")) is int and value["RestartCount"] >= 0,
            "PREUPGRADE_CONTAINER_CHANGED")
    require((value.get("Created"), state.get("StartedAt"), state.get("FinishedAt"),
             value.get("RestartCount")) == generation, "PREUPGRADE_CONTAINER_CHANGED")
    mounts = [m for m in value.get("Mounts", []) if m.get("Destination") == "/var/lib/postgresql"]
    require(len(mounts) == 1 and mounts[0].get("Type") == "volume" and
            mounts[0].get("Name") == volume and mounts[0].get("Source") == mountpoint and
            mounts[0].get("RW") is True, "PREUPGRADE_VOLUME_BINDING")
    # Nested or alternative data/WAL mounts would escape the captured volume.
    require(not any(m.get("Destination", "").startswith("/var/lib/postgresql/")
                    for m in value.get("Mounts", [])), "PREUPGRADE_VOLUME_BINDING")


def volume_info(docker, name):
    """Read one local Docker volume without broadening the daily Docker API."""
    require(isinstance(name, str) and re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}", name),
            "PREUPGRADE_VOLUME_NAME")
    result = install.json_bytes(docker.call(["volume", "inspect", name]).encode(), maximum=2 * 1024**2)
    require(isinstance(result, list) and len(result) == 1 and isinstance(result[0], dict),
            "PREUPGRADE_VOLUME_INSPECT")
    value = result[0]
    require(value.get("Name") == name and value.get("Driver") == "local" and
            value.get("Scope") == "local" and not value.get("Options") and
            runtime.absolute(value.get("Mountpoint")) and "," not in value["Mountpoint"],
            "PREUPGRADE_VOLUME_BINDING")
    return value


def checked_audit(data, *, database, system_identifier, migrations):
    require(isinstance(data, dict) and set(data) == AUDIT_KEYS and
            data["schema"] == "lagrange-preupgrade-db-v1" and data["database"] == database and
            type(data["server_version_num"]) is int and 180000 <= data["server_version_num"] < 190000 and
            data["system_identifier"] == system_identifier and
            isinstance(system_identifier, str) and re.fullmatch(r"[1-9][0-9]{0,19}", system_identifier) and
            isinstance(data["catalog_sha256"], str) and
            re.fullmatch(r"sha256:[0-9a-f]{64}", data["catalog_sha256"]), "PREUPGRADE_DATABASE_AUDIT")
    require(isinstance(migrations, list) and 1 <= len(migrations) <= 59 and
            data["migrations"] == migrations, "PREUPGRADE_MIGRATIONS")
    previous = 0
    for row in migrations:
        require(isinstance(row, dict) and set(row) == {"version", "checksum", "success"} and
                type(row["version"]) is int and previous < row["version"] <= 59 and
                row["success"] is True and isinstance(row["checksum"], str) and
                re.fullmatch(r"[0-9a-f]{96}", row["checksum"]), "PREUPGRADE_MIGRATIONS")
        previous = row["version"]
    return data


class ColdStorage(backup.Storage):
    """Reuse private staging/encryption; never route through daily DB tables."""

    @contextlib.contextmanager
    def temporary(self, prefix):
        self.reserve()
        require(prefix.startswith(".preupgrade-"), "PREUPGRADE_STAGING")
        with install.root_directory(self.root) as parent:
            name = tempfile.mkdtemp(prefix=prefix, dir=self.root)
            os.chmod(name, 0o700)
            try:
                yield Path(name)
            finally:
                # A failed/ambiguous Docker cleanup retains the copied mount
                # and its durable owned-resource intent for bounded recovery.
                keep = False
                for leaf in backup.bounded_names(parent, 10000):
                    if re.fullmatch(r"preupgrade-verifier-[0-9a-f]{32}\.json", leaf):
                        intent = install.json_bytes(install.read_file(parent, leaf, 0, 0, 0o600))
                        if intent.get("staging") == name:
                            keep = True
                if not keep:
                    shutil.rmtree(name)

    def verify_cluster(self, restored, expected):
        """Verify a newly restored private tree and boot only its PostgreSQL copy.

        The caller owns this fresh archive extraction, not a live volume. It
        must pass the protected staging parent returned by temporary(). The
        exact file inventory is checked again before any server is started.
        """
        restored = Path(restored)
        # A direct live-volume path or another backup root is not an input.
        require(restored.name == "restored" and restored.parent.parent == Path(self.root) and
                restored.parent.name.startswith(".preupgrade-"), "PREUPGRADE_RESTORE_SCOPE")
        with install.root_directory(str(restored.parent)) as parent:
            install.directory_info(parent, uid=0, gid=0, mode=0o700)
        require(isinstance(expected, dict) and set(expected) == {"files", "database"},
                "PREUPGRADE_RESTORE_EXPECTED")
        manifest = files.validate_manifest(files.canonical(expected["files"]), self.limits, profile=PROFILE)
        roots = []
        for name in files.PREUPGRADE_CLASSES:
            entry = next(e for e in manifest["entries"] if e["path"] == name)
            roots.append(files.Root(name, str(restored / name), entry["uid"], entry["gid"], entry["mode"]))
        with files.Snapshot(roots, self.limits, profile=PROFILE) as snapshot:
            require(snapshot.manifest() == files.canonical(manifest), "PREUPGRADE_RESTORE_FILES")
            pgfd = snapshot.descriptors["postgres"]
            handles = []
            try:
                for name in ("18", "docker"):
                    fd = os.open(name, files.DIRECTORY, dir_fd=pgfd if not handles else handles[-1])
                    handles.append(fd)
                version = install.read_file(handles[-1], "PG_VERSION", uid=999, gid=999, maximum=16)
                cluster_manifest(manifest, version)
            finally:
                for fd in reversed(handles):
                    os.close(fd)
        source_audit = expected["database"]
        require(isinstance(source_audit, dict), "PREUPGRADE_DATABASE_AUDIT")
        checked_audit(source_audit, database=self.binding.database,
                      system_identifier=source_audit.get("system_identifier"),
                      migrations=source_audit.get("migrations"))
        image = self.docker.object("image", backup.POSTGRES)
        require(backup.POSTGRES in image.get("RepoDigests", []) and
                runtime.DIGEST.fullmatch(image.get("Id", "")), "PREUPGRADE_VERIFY_IMAGE")
        environment = runtime.environment({"Config": image["Config"]})
        nonce = uuid.uuid4().hex
        labels = {"io.lagrange.preupgrade-verify": nonce,
                  "io.lagrange.preupgrade-project": self.binding.project}
        data = str(restored / "postgres")
        script = self.context.release + "/deploy/db/preupgrade-recovery-verify.sh"
        audit_script = self.context.release + "/deploy/db/preupgrade-recovery-audit.sql"
        args = ["create", "--name", "lagrange-preupgrade-verify-" + nonce, "--init",
                "--restart=no", "--network=none", "--read-only", "--user=999:999",
                "--cap-drop=ALL", "--security-opt=no-new-privileges:true", "--memory=512m", "--cpus=1",
                "--tmpfs", "/tmp:rw,nosuid,nodev,size=32m,uid=999,gid=999,mode=0700",
                "--tmpfs", "/var/lib/postgresql:rw,nosuid,nodev,noexec,size=4k,mode=000",
                "--mount", "type=bind,src=" + data + ",dst=/restore",
                "--mount", "type=bind,src=" + script + ",dst=/verify.sh,readonly",
                "--mount", "type=bind,src=" + audit_script + ",dst=/verify.sql,readonly",
                "--entrypoint=/bin/bash"]
        for key, value in labels.items():
            args += ["--label", key + "=" + value]
        require(re.fullmatch(r"[a-z][a-z0-9_]{0,62}", self.binding.database), "PREUPGRADE_DATABASE")
        name = "lagrange-preupgrade-verify-" + nonce
        intent_name = "preupgrade-verifier-" + nonce + ".json"
        intent = files.canonical({"schema": "lagrange-preupgrade-verifier-v1", "name": name,
            "labels": labels, "image_id": image["Id"], "image": backup.POSTGRES,
            "staging": str(restored.parent)})
        with install.root_directory(self.root) as parent:
            install.create_file(parent, intent_name, intent, 0, 0, 0o600)
        identity = None
        created = None

        def owned_ids():
            result = self.docker.call(["ps", "--all", "--quiet", "--no-trunc", "--filter",
                "label=io.lagrange.preupgrade-verify=" + nonce, "--filter",
                "label=io.lagrange.preupgrade-project=" + self.binding.project,
                "--filter", "name=^/" + name + "$"]).splitlines()
            require(len(result) <= 1 and all(runtime.CONTAINER.fullmatch(i) for i in result),
                    "PREUPGRADE_VERIFY_OWNER")
            return result

        def observe():
            value = self.docker.object("container", identity)
            require(value.get("Id") == identity and value.get("Image") == image["Id"] and
                    value.get("Name") == "/" + name and
                    value["Config"].get("Image") == backup.POSTGRES and
                    all((value["Config"].get("Labels") or {}).get(k) == v for k, v in labels.items()) and
                    (created is None or value.get("Created") == created), "PREUPGRADE_VERIFY_OWNER")
            return value

        try:
            result = self.docker.call([*args, backup.POSTGRES, "/verify.sh", self.binding.database]).strip()
            require(runtime.CONTAINER.fullmatch(result), "PREUPGRADE_VERIFY_OWNER")
            identity = result
            value = observe()
            created = value["Created"]
            host = value["HostConfig"]
            require(value["Config"].get("User") == "999:999" and
                    value["Config"].get("Entrypoint") == ["/bin/bash"] and
                    value["Config"].get("Cmd") == ["/verify.sh", self.binding.database] and
                    not host.get("GroupAdd") and host.get("NetworkMode") == "none" and
                    set(value.get("NetworkSettings", {}).get("Networks", {})) <= {"none"},
                    "PREUPGRADE_VERIFY_POLICY")
            backup.isolated_policy(value, memory=512 * 1024**2, environment=environment,
                mounts={"/restore": (data, True), "/verify.sh": (script, False),
                        "/verify.sql": (audit_script, False)},
                tmpfs={"/tmp": "rw,nosuid,nodev,size=32m,uid=999,gid=999,mode=0700",
                       "/var/lib/postgresql": "rw,nosuid,nodev,noexec,size=4k,mode=000"})
            target = restored.parent / "restored-audit.json"
            try:
                self.command_file(["/usr/bin/docker", "--host", "unix:///var/run/docker.sock",
                                   "start", "--attach", identity], target, seconds=300)
            except runtime.RuntimeErrorCode as error:
                # Only this exact owned clone's reserved script exit proves
                # native pg_controldata rejection. Never infer the stage from
                # an arbitrary command failure, timeout, or Docker error.
                if error.args == ("DAILY_BACKUP_PROCESS",):
                    state = observe()
                    result = state.get("State", {})
                    if (result.get("Running") is False and result.get("OOMKilled") is False
                            and not result.get("Error") and type(result.get("ExitCode")) is int
                            and result["ExitCode"] == 73 and state.get("RestartCount") == 0):
                        raise runtime.RuntimeErrorCode("PREUPGRADE_VERIFY_CONTROL") from None
                raise
            value = observe()
            require(not value["State"].get("Running") and not value["State"].get("OOMKilled") and
                    not value["State"].get("Error") and value["State"].get("ExitCode") == 0 and
                    value.get("RestartCount") == 0, "PREUPGRADE_VERIFY_RESULT")
            payload = install.read_root_file(str(target), gid=0, mode=0o600, maximum=1024 * 1024)
            require(len(payload.splitlines()) == 1, "PREUPGRADE_VERIFY_RESULT")
            actual = install.json_bytes(payload, maximum=1024 * 1024)
            checked_audit(actual, database=self.binding.database,
                          system_identifier=source_audit["system_identifier"],
                          migrations=source_audit["migrations"])
            require(actual == source_audit, "PREUPGRADE_RESTORE_AUDIT_MISMATCH")
            return actual
        finally:
            if identity is None:
                recovered = owned_ids()
                identity = recovered[0] if recovered else None
            if identity is not None:
                value = observe()
                if value["State"].get("Running"):
                    self.docker.call(["kill", "--signal=TERM", identity])
                    self.docker.call(["wait", identity], timeout=40)
                observe()
                self.docker.call(["rm", "--volumes", identity])
                require(not owned_ids(), "PREUPGRADE_VERIFY_CLEANUP")
                with install.root_directory(self.root) as parent:
                    require(install.read_file(parent, intent_name, 0, 0, 0o600) == intent,
                            "PREUPGRADE_VERIFY_OWNER")
                    os.unlink(intent_name, dir_fd=parent)
                    os.fsync(parent)
            # No response and no observed container is still ambiguous: the
            # daemon may finish creation later. Retain intent, fail and block
            # another invocation rather than assuming cleanup succeeded.
