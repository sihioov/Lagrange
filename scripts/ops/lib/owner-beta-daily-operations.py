#!/usr/bin/env python3
"""Internal implementation of the controlled compose-release daily operations."""
from __future__ import annotations

import argparse
import contextlib
import dataclasses
import fcntl
import hashlib
import importlib.util
import os
import re
import stat
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path


def load_local(name, filename):
    path = Path(__file__).parent / filename
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError("DAILY_OPERATION_SOURCE")
    value = importlib.util.module_from_spec(spec)
    sys.modules[name] = value
    spec.loader.exec_module(value)
    return value


sys.dont_write_bytecode = True

install = load_local("daily_install_operations", "owner-beta-daily-release.py")
runtime = load_local("daily_runtime_operations", "owner-beta-daily-runtime.py")
policy = load_local("daily_status_operations", "service-status-policy.py")
ROOT = str(Path(__file__).absolute().parents[3])
CONTEXT_KEYS = (
    "LAGRANGE_CODE_COMMIT", "LAGRANGE_DATA_DIR", "LAGRANGE_ARTIFACTS_DIR",
    "LAGRANGE_RUNTIME_SECRET_DIR", "POSTGRES_DB", "MAX_JOBS_PER_OWNER",
    "PRICE_BETA_LEASE_SECS", "PRICE_BETA_BACKOFF_SECS", "OWNER_BETA_DAILY_MODE",
    "OWNER_BETA_ACCESS_MODE", "OWNER_BETA_PRICE_INPUT_MODE", "OWNER_BETA_PAPER_MODE",
)
ENVIRONMENT = {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "HOME": "/root",
               "LANG": "C.UTF-8", "PYTHONDONTWRITEBYTECODE": "1"}
require = runtime.require


@dataclasses.dataclass(frozen=True)
class Ownership:
    project: str
    commit: str
    image_id: str

    def __post_init__(self):
        require(runtime.PROJECT.fullmatch(self.project) and
                install.is_commit(self.commit) and runtime.DIGEST.fullmatch(self.image_id),
                "DAILY_OPERATION_OWNERSHIP")


@contextlib.contextmanager
def release_lock(root):
    with install.root_directory(root) as directory:
        name = ".lagrange-release.lock"
        try:
            fd = os.open(name, os.O_RDWR | os.O_CREAT | os.O_EXCL |
                         os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=directory)
        except FileExistsError:
            fd = os.open(name, os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=directory)
        try:
            info = install.file_info(fd, 0, 0, 0o600)
            require(info.st_size == 0, "DAILY_OPERATION_LOCK")
            named = os.stat(name, dir_fd=directory, follow_symlinks=False)
            require((named.st_dev, named.st_ino) == (info.st_dev, info.st_ino),
                    "DAILY_OPERATION_LOCK")
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise runtime.RuntimeErrorCode("DAILY_OPERATION_BUSY") from None
            yield
            after = os.stat(name, dir_fd=directory, follow_symlinks=False)
            require((after.st_dev, after.st_ino) == (info.st_dev, info.st_ino),
                    "DAILY_OPERATION_LOCK")
        finally:
            os.close(fd)


def read_manifest(release, commit):
    path = release + "/.lagrange-release-manifest"
    data = install.read_root_file(path, gid=0, mode=0o600, maximum=64 * 1024)
    expected = hashlib.sha256(data).hexdigest()
    require(policy.validate_manifest(path, commit) == expected,
            "DAILY_OPERATION_MANIFEST")
    result = {}
    for row in data.decode("ascii").splitlines()[2:]:
        _, service, _, image, revision = row.split("|")
        require(revision == commit, "DAILY_OPERATION_MANIFEST")
        result[service] = image
    require(tuple(result) == policy.MANIFEST_SERVICES, "DAILY_OPERATION_MANIFEST")
    return result


def read_context(release, commit, action):
    path = release + "/deploy/compose/.env"
    original = install.read_root_file(path, gid=0, mode=0o600, maximum=128 * 1024)
    result = subprocess.run(
        ["/bin/bash", "--noprofile", "--norc",
         ROOT + "/scripts/ops/lib/owner-beta-daily-context.sh", release, commit, action],
        env=ENVIRONMENT, stdin=subprocess.DEVNULL, capture_output=True, timeout=15, check=False)
    require(result.returncode == 0 and len(result.stdout) <= 32 * 1024,
            "DAILY_OPERATION_CONTEXT")
    require(install.read_root_file(path, gid=0, mode=0o600, maximum=128 * 1024) == original,
            "DAILY_OPERATION_CONTEXT")
    fields = result.stdout.decode("utf-8", errors="strict").split("\0")
    require(fields[-1] == "" and len(fields) % 2 == 1, "DAILY_OPERATION_CONTEXT")
    values, overlays = {}, []
    for index in range(0, len(fields) - 1, 2):
        key, value = fields[index:index + 2]
        if key == "COMPOSE_ARG":
            overlays.append(value)
        else:
            require(key in CONTEXT_KEYS and key not in values, "DAILY_OPERATION_CONTEXT")
            values[key] = value
    require(tuple(values) == CONTEXT_KEYS and values["LAGRANGE_CODE_COMMIT"] == commit,
            "DAILY_OPERATION_CONTEXT")
    for key in ("LAGRANGE_DATA_DIR", "LAGRANGE_ARTIFACTS_DIR", "LAGRANGE_RUNTIME_SECRET_DIR"):
        require(runtime.absolute(values[key]), "DAILY_OPERATION_CONTEXT")
    # Overlay argument grammar is a sequence returned by the existing fixed
    # selector. No caller-supplied path or option can enter Compose.
    allowed = {release + "/deploy/compose/" + name for name in
               ("compose.intraday.yml", "compose.intraday-operational.yml",
                "compose.market-stream.yml")}
    require(len(overlays) % 2 == 0 and len(overlays) <= 6 and
            all(overlays[n] == "-f" and overlays[n + 1] in allowed
                for n in range(0, len(overlays), 2)) and
            len(set(overlays[1::2])) == len(overlays[1::2]),
            "DAILY_OPERATION_CONTEXT")
    return values, overlays


def project_binding(source_commit, target_commit):
    if not policy.optional_trusted_file_exists(policy.POLICY_PATH, parent_mode=0o700):
        return "lagrange-station", "absent"
    data = policy.read_trusted_file(policy.POLICY_PATH, parent_mode=0o700)
    value = policy.validate_policy(policy.strict_json_loads(data))
    require(value["release_commit"] in {source_commit, target_commit},
            "DAILY_OPERATION_PROJECT")
    return value["compose_project"], value["research_mode"]


def activation_binding(release, commit, project, images, values):
    config, digest = install.verify_config(release, commit)
    require(values["OWNER_BETA_DAILY_MODE"] == "owner_only" and
            values["OWNER_BETA_ACCESS_MODE"] == "owner_only" and
            values["OWNER_BETA_PRICE_INPUT_MODE"] == "sealed_v1" and
            values["OWNER_BETA_PAPER_MODE"] in ("", "disabled"), "DAILY_OPERATION_MODE")
    # Capacity/lease/backoff must be explicitly present in the protected env.
    binding = runtime.Binding(
        project, commit, images["owner-beta-runner"], digest, release,
        values["LAGRANGE_DATA_DIR"], values["LAGRANGE_ARTIFACTS_DIR"],
        values["LAGRANGE_RUNTIME_SECRET_DIR"], values["POSTGRES_DB"] or "lagrange",
        values["MAX_JOBS_PER_OWNER"], values["PRICE_BETA_LEASE_SECS"],
        values["PRICE_BETA_BACKOFF_SECS"])
    return binding, config


def internal_network(docker, project):
    ids = docker.call(["network", "ls", "--no-trunc", "--filter",
                      "label=com.docker.compose.project=" + project, "--filter",
                      "label=com.docker.compose.network=owner-beta-db",
                      "--format", "{{.ID}}"]).splitlines()
    require(len(ids) == 1 and runtime.CONTAINER.fullmatch(ids[0]),
            "DAILY_OPERATION_NETWORK")
    network = docker.object("network", ids[0])
    labels = network.get("Labels") or {}
    require(network.get("Id") == ids[0] and network.get("Internal") is True and
            labels.get("com.docker.compose.project") == project and
            labels.get("com.docker.compose.network") == "owner-beta-db",
            "DAILY_OPERATION_NETWORK")
    postgres = []
    for identity in network.get("Containers", {}):
        value = docker.object("container", identity)
        endpoints = value.get("NetworkSettings", {}).get("Networks", {})
        endpoint = next((e for e in endpoints.values() if e.get("NetworkID") == ids[0]), {})
        if "postgres" in (endpoint.get("Aliases") or []):
            postgres.append(value)
    require(len(postgres) == 1, "DAILY_OPERATION_DATABASE_HOST")
    server = postgres[0]
    server_labels = server.get("Config", {}).get("Labels") or {}
    require(server_labels.get("com.docker.compose.project") == project and
            server_labels.get("com.docker.compose.service") == "postgres" and
            server.get("State", {}).get("Running") is True and
            server.get("Config", {}).get("Image") ==
            "postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a",
            "DAILY_OPERATION_DATABASE_HOST")
    return network


def migration_secret(secret_root):
    # The root ancestor ends before the deliberately UID999-owned service dir.
    with install.root_directory(secret_root) as parent:
        with install.child_directory(parent, "db-migrate", 999, 999, 0o750) as service:
            value = install.read_file(service, "db_migration_owner_password",
                                      999, 999, 0o400, 8192)
            require(value and not any(c in value for c in (b"\0", b"\r", b"\n")),
                    "DAILY_OPERATION_SECRET")
    return secret_root + "/db-migrate/db_migration_owner_password"


def database_name(values):
    name = values["POSTGRES_DB"] or "lagrange"
    require(re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,62}", name),
            "DAILY_OPERATION_DATABASE")
    return name


class Database:
    def __init__(self, docker, project, commit, image):
        self.docker, self.project, self.commit, self.image = docker, project, commit, image

    def execute(self, values, sql, expected, *, backup_audit=False):
        docker = self.docker
        require(isinstance(sql, str) and 0 < len(sql.encode("utf-8")) <= 128 * 1024,
                "DAILY_OPERATION_SQL")
        require(type(backup_audit) is bool, "DAILY_OPERATION_SQL")
        if backup_audit:
            # Only this fixed, bounded inventory may use a larger response.
            # Ordinary controls retain their exact one-line 4KiB/45s contract.
            fixed = install.read_root_file(
                ROOT + "/deploy/db/owner-beta-daily-backup-audit.sql",
                maximum=64 * 1024).decode("ascii")
            require(sql == fixed and expected is None, "DAILY_OPERATION_SQL")
        maximum = 64 * 1024**2 if backup_audit else 4096
        deadline = 180 if backup_audit else 45
        runtime.image_owned(docker, Ownership(self.project, self.commit, self.image))
        network = internal_network(docker, self.project)
        secret = migration_secret(values["LAGRANGE_RUNTIME_SECRET_DIR"])
        require("," not in secret, "DAILY_OPERATION_SECRET")
        name, nonce = "lagrange-daily-control-" + uuid.uuid4().hex, uuid.uuid4().hex
        labels = {"io.lagrange.daily-control": nonce,
                  "io.lagrange.daily-control-project": self.project,
                  "org.opencontainers.image.revision": self.commit}
        args = ["create", "--name", name, "--interactive", "--init", "--restart=no",
                "--read-only", "--user", "999:999", "--cap-drop=ALL",
                "--security-opt=no-new-privileges:true", "--memory=128m", "--cpus=1",
                "--network", network["Id"], "--tmpfs", "/tmp:rw,nosuid,nodev,size=16m",
                "--tmpfs", "/var/lib/postgresql:rw,nosuid,nodev,noexec,size=4k,mode=000",
                "--mount", "type=bind,src=" + secret +
                ",dst=/run/secrets/db_migration_owner_password,readonly",
                "--entrypoint", "/usr/local/bin/lagrange-daily-control"]
        for key, value in labels.items():
            args += ["--label", key + "=" + value]
        for key, value in {
            "DB_HOST": "postgres", "DB_PORT": "5432", "DB_USER": "migration_owner",
            "DB_NAME": database_name(values),
            "DB_PASSWORD_FILE": "/run/secrets/db_migration_owner_password",
        }.items():
            args += ["--env", key + "=" + value]
        identity = docker.call([*args, self.image]).strip()
        require(runtime.CONTAINER.fullmatch(identity), "DAILY_OPERATION_DATABASE")
        created = None

        def observation():
            value = docker.object("container", identity)
            config = value.get("Config") or {}
            require(value.get("Id") == identity and value.get("Image") == self.image and
                    all((config.get("Labels") or {}).get(k) == v for k, v in labels.items()) and
                    (created is None or value.get("Created") == created),
                    "DAILY_OPERATION_DATABASE_OWNER")
            return value

        try:
            value = observation()
            created = value.get("Created")
            require(isinstance(created, str) and created and
                    value["Config"].get("User") == "999:999" and
                    value["Config"].get("Entrypoint") ==
                    ["/usr/local/bin/lagrange-daily-control"] and
                    value["HostConfig"].get("ReadonlyRootfs") is True and
                    value["HostConfig"].get("RestartPolicy", {}).get("Name") == "no",
                    "DAILY_OPERATION_DATABASE_POLICY")
            host = value["HostConfig"]
            require(host.get("Privileged") is False and host.get("CapDrop") == ["ALL"] and
                    not host.get("CapAdd") and host.get("SecurityOpt") ==
                    ["no-new-privileges:true"] and host.get("Init") is True and
                    host.get("Memory") == 128 * 1024**2 and host.get("NanoCpus") == 10**9,
                    "DAILY_OPERATION_DATABASE_POLICY")
            mounts = value.get("Mounts", [])
            binds = [m for m in mounts if m.get("Type") == "bind"]
            require(len(binds) == 1 and binds[0].get("Source") == secret and
                    binds[0].get("Destination") == "/run/secrets/db_migration_owner_password"
                    and binds[0].get("RW") is False and all(
                        m.get("Type") == "bind" or (m.get("Type") == "tmpfs" and
                        m.get("Destination") in {"/tmp", "/var/lib/postgresql"}) for m in mounts),
                    "DAILY_OPERATION_DATABASE_POLICY")
            require(host.get("NetworkMode") in {network["Id"], network["Name"]},
                    "DAILY_OPERATION_DATABASE_POLICY")
            result = subprocess.run(
                ["/usr/bin/docker", "--host", "unix:///var/run/docker.sock",
                 "start", "--attach", "--interactive", identity],
                input=sql.encode("utf-8"), env=ENVIRONMENT, capture_output=True,
                timeout=deadline, check=False)
            value = observation()
            endpoints = value.get("NetworkSettings", {}).get("Networks", {})
            require(len(endpoints) == 1 and next(iter(endpoints.values())).get("NetworkID") ==
                    network["Id"], "DAILY_OPERATION_DATABASE_POLICY")
            require(result.returncode == 0 and len(result.stdout) <= maximum and
                    not value["State"].get("Running") and
                    value["State"].get("Status") == "exited" and
                    not value["State"].get("OOMKilled") and
                    not value["State"].get("Error") and value["State"].get("ExitCode") == 0 and
                    value.get("RestartCount") == 0,
                    "DAILY_OPERATION_DATABASE")
            rows = result.stdout.decode("ascii", errors="strict").splitlines()
            if backup_audit:
                require(len(rows) == 1, "DAILY_OPERATION_DATABASE_RESULT")
                value = install.json_bytes(rows[0].encode("ascii"), maximum=maximum)
                require(isinstance(value, dict) and
                        value.get("schema") == "lagrange-daily-backup-db-v1",
                        "DAILY_OPERATION_DATABASE_RESULT")
                return value
            require(len(rows) == 1 and rows[0] in expected, "DAILY_OPERATION_DATABASE_RESULT")
            return rows[0]
        finally:
            # A timed-out local Docker client does not prove psql stopped.
            # Only this unique, re-inspected one-shot is signalled/removed.
            value = observation()
            if value["State"].get("Running"):
                docker.call(["kill", "--signal=TERM", identity])
                docker.call(["wait", identity], timeout=25)
                value = observation()
            require(not value["State"].get("Running"), "DAILY_OPERATION_DATABASE_CLEANUP")
            docker.call(["rm", identity])

    def status(self, values):
        sql = install.read_root_file(
            ROOT + "/deploy/db/owner-beta-daily-control-status.sql",
            maximum=64 * 1024).decode("ascii")
        prefix = "DAILY_CONTROL_STATUS: "
        values_allowed = {"ABSENT_SCHEMA", "ABSENT_CONTROL", "DISABLED", "ENABLED"}
        result = self.execute(values, sql, {prefix + v for v in values_allowed})
        return result[len(prefix):]

    def backup_audit(self, values):
        sql = install.read_root_file(
            ROOT + "/deploy/db/owner-beta-daily-backup-audit.sql",
            maximum=64 * 1024).decode("ascii")
        return self.execute(values, sql, None, backup_audit=True)

    def restore_reservations(self, values):
        sql = install.read_root_file(
            ROOT + "/deploy/db/owner-beta-daily-reservation-restore.sql",
            maximum=64 * 1024).decode("ascii")
        # Repair is transactional; its larger statement limit does not relax
        # the existing transport timeout. A timeout is failure, never success.
        return self.execute(values, sql, {"DAILY_BACKUP_RESERVATIONS: VERIFIED"})

    def control(self, release, commit, values, action):
        numbers = () if action == "disable" else (
            values["MAX_JOBS_PER_OWNER"], values["PRICE_BETA_LEASE_SECS"],
            values["PRICE_BETA_BACKOFF_SECS"])
        sql = install.render_control(release, commit, action, *numbers)
        expected = "ENABLED" if action == "enable" else "DISABLED"
        return self.execute(values, sql, {"DAILY_CONTROL: " + expected})


def reject_ambient(values):
    internal = ("OWNER_BETA_DAILY_MODE_FILE", "PRICE_BETA_DAILY_MODE",
                "PRICE_BETA_DAILY_CONFIG_FILE", "PRICE_BETA_DAILY_CONFIG_SHA256",
                "LAGRANGE_DAILY_VERIFIED_CONFIG_SHA256")
    require(not any(k in os.environ for k in internal), "DAILY_OPERATION_ENVIRONMENT")
    require(all(k not in os.environ or os.environ[k] == v
                for k, v in values.items()), "DAILY_OPERATION_ENVIRONMENT")


def global_cohort(docker, ownership):
    for service in runtime.SERVICES:
        ids = docker.call(["ps", "--all", "--no-trunc", "--filter",
                          "label=com.docker.compose.service=" + service,
                          "--format", "{{.ID}}"]).splitlines()
        require(all(runtime.CONTAINER.fullmatch(i) for i in ids),
                "DAILY_OPERATION_GLOBAL_OWNER")
        for identity in ids:
            value = docker.object("container", identity)
            labels = value.get("Config", {}).get("Labels") or {}
            if (service == "owner-beta-runner" and
                    labels.get("com.docker.compose.project") != ownership.project and
                    runtime.environment(value).get("PRICE_BETA_DAILY_MODE") != "owner_only"):
                continue
            runtime.owned(value, ownership, service, identity)


def verify_api(docker, binding, image):
    ids = docker.call(["ps", "--all", "--no-trunc", "--filter",
                      "label=com.docker.compose.project=" + binding.project, "--filter",
                      "label=com.docker.compose.service=api-server",
                      "--format", "{{.ID}}"]).splitlines()
    require(len(ids) == 1 and runtime.CONTAINER.fullmatch(ids[0]), "DAILY_OPERATION_API")
    value = docker.object("container", ids[0])
    api = Ownership(binding.project, binding.commit, image)
    generation = runtime.owned(value, api, "api-server", ids[0])
    env = runtime.environment(value)
    state, config = value["State"], value["Config"]
    require(state.get("Running") is True and not state.get("Restarting") and
            not state.get("Paused") and not state.get("OOMKilled") and
            state.get("Health", {}).get("Status") == "healthy" and
            config.get("User") == "10001:10001" and
            config.get("Entrypoint") == ["/usr/local/bin/api-server"],
            "DAILY_OPERATION_API")
    require(all(env.get(k) == v for k, v in {
        "OWNER_BETA_DAILY_MODE": "owner_only", "OWNER_BETA_ACCESS_MODE": "owner_only",
        "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1", "OWNER_BETA_PAPER_MODE": "disabled",
        "MAX_JOBS_PER_OWNER": binding.capacity, "LAGRANGE_CODE_COMMIT": binding.commit,
        "DB_HOST": "postgres", "DB_PORT": "5432", "DB_NAME": binding.database,
    }.items()), "DAILY_OPERATION_API")
    return generation


def stopped(docker, ownership, snapshot, allow_sealed_worker=False):
    for service, entry in snapshot.items():
        value = runtime.unchanged(docker, ownership, service, entry, 15)
        if (allow_sealed_worker and service == "owner-beta-runner" and
                runtime.environment(value).get("PRICE_BETA_DAILY_MODE", "disabled") == "disabled"):
            continue
        state = value["State"]
        require(not state.get("Running") and not state.get("Restarting") and
                not state.get("Paused") and not state.get("OOMKilled") and
                not state.get("Error") and state.get("ExitCode") == 0 and
                state.get("Status") in ("created", "exited") and
                value["HostConfig"].get("RestartPolicy", {}).get("Name") == "no",
                "DAILY_OPERATION_NOT_QUIESCENT")


def remove_stopped(docker, ownership, snapshot):
    stopped(docker, ownership, snapshot)
    for service, entry in snapshot.items():
        runtime.unchanged(docker, ownership, service, entry, 15)
        docker.call(["rm", entry["generation"][0]])
        require(not docker.service_ids(ownership.project, service), "DAILY_OPERATION_REMOVE")


def snapshot_equal(before, after):
    return (set(before) == set(after) and all(
        before[k]["generation"] == after[k]["generation"] for k in before))


@dataclasses.dataclass
class Context:
    release: str
    commit: str
    images: dict
    values: dict
    overlays: list


class Operations:
    def __init__(self, docker, database, source, target, ownership, install_root, research_mode):
        self.docker, self.database = docker, database
        self.source, self.target, self.ownership = source, target, ownership
        self.install_root, self.research_mode = install_root, research_mode

    def disable(self, snapshot, *, target=False):
        context = self.target if target else self.source
        own = (Ownership(self.ownership.project, context.commit,
                         context.images["owner-beta-runner"]) if target else self.ownership)
        failure = None
        try:
            # No capacity, Owner-rights or broader Auth0 validation on shutdown.
            if context.values is None:
                context.values, context.overlays = read_context(
                    context.release, context.commit, "disable")
            self.database.control(context.release, context.commit, context.values, "disable")
        except Exception:
            failure = "DAILY_OPERATION_DISABLE_DATABASE"
        try:
            runtime.drain(self.docker, own, snapshot)
        except Exception:
            failure = "DAILY_OPERATION_DISABLE_DRAIN"
        require(failure is None, failure or "DAILY_OPERATION_DISABLE")

    def cleanup_target(self, binding):
        failure = None
        try:
            self.database.control(self.target.release, self.target.commit,
                                  self.target.values, "disable")
        except Exception:
            failure = "DAILY_OPERATION_CLEANUP_DATABASE"
        try:
            current = runtime.capture(self.docker, binding)
            runtime.drain(self.docker, binding, current)
        except Exception:
            failure = "DAILY_OPERATION_CLEANUP_DRAIN"
        require(failure is None, failure or "DAILY_OPERATION_CLEANUP")

    def prepare(self, snapshot):
        stopped(self.docker, self.ownership, snapshot, allow_sealed_worker=True)
        target = self.target
        binding, config = activation_binding(
            target.release, target.commit, self.ownership.project, target.images, target.values)
        status = self.database.status(target.values)
        require(status in {"ABSENT_CONTROL", "DISABLED"},
                "DAILY_OPERATION_PREPARE_STATE")
        if status == "ABSENT_CONTROL":
            pristine_state(binding.data_root, config)
        install.reader_guard(self.install_root, target.release, "owner_only")
        install.provision_state(binding.data_root, config)
        self.database.control(target.release, target.commit, target.values, "prepare")

    def enable(self, snapshot):
        target = self.target
        binding, config = activation_binding(
            target.release, target.commit, self.ownership.project, target.images, target.values)
        reject_ambient(target.values)
        validate_activation_inputs(target)
        # Exact already-running activation is idempotent: no restart or epoch reset.
        if self.source.commit == target.commit:
            try:
                current = runtime.verify_running(self.docker, binding)
                api = verify_api(self.docker, binding, target.images["api-server"])
                require(snapshot_equal(snapshot, current), "DAILY_OPERATION_GENERATION")
            except runtime.RuntimeErrorCode:
                current = None
            if current is not None:
                try:
                    self.database.control(target.release, target.commit, target.values, "enable")
                    require(snapshot_equal(current, runtime.verify_running(self.docker, binding)),
                            "DAILY_OPERATION_GENERATION")
                    require(api == verify_api(self.docker, binding, target.images["api-server"]),
                            "DAILY_OPERATION_GENERATION")
                except Exception:
                    self.cleanup_target(binding)
                    raise
                return "ALREADY_ENABLED"
        if any(path.endswith("/compose.market-stream.yml") for path in target.overlays):
            require(os.environ.get("OWNER_MARKET_STREAM_ROLLOUT_CONFIRM") ==
                    "I_UNDERSTAND_OWNER_MARKET_STREAM_READ_ONLY_WS_CALLS",
                    "DAILY_OPERATION_MARKET_STREAM_CONFIRM")
        source_values = self.source.values
        if source_values is None:
            source_values, self.source.overlays = read_context(
                self.source.release, self.source.commit, "disable")
            self.source.values = source_values
        require(all(source_values[k] == target.values[k] for k in
                    ("LAGRANGE_DATA_DIR", "LAGRANGE_ARTIFACTS_DIR",
                     "LAGRANGE_RUNTIME_SECRET_DIR", "POSTGRES_DB")),
                "DAILY_OPERATION_RELEASE_BINDING")
        self.disable(snapshot)
        install.check_state(binding.data_root, config)
        self.database.control(target.release, target.commit, target.values, "check-disabled")
        remove_stopped(self.docker, self.ownership, snapshot)
        try:
            start_target(self.docker, target, binding, self.install_root, self.research_mode)
            current = runtime.verify_running(self.docker, binding)
            api = verify_api(self.docker, binding, target.images["api-server"])
            self.database.control(target.release, target.commit, target.values, "enable")
            require(snapshot_equal(current, runtime.verify_running(self.docker, binding)) and
                    api == verify_api(self.docker, binding, target.images["api-server"]),
                    "DAILY_OPERATION_GENERATION")
        except Exception:
            # Old members were removed only after a complete successful drain;
            # any remaining target member now has one unambiguous manifest owner.
            self.cleanup_target(binding)
            raise
        return "ENABLED"


def validate_activation_inputs(target):
    result = subprocess.run(
        ["/bin/bash", "--noprofile", "--norc",
         ROOT + "/scripts/ops/validate-production-config.sh", "--scope", "release",
         "--env-file", target.release + "/deploy/compose/.env"],
        env=ENVIRONMENT, stdin=subprocess.DEVNULL, capture_output=True,
        timeout=30, check=False)
    require(result.returncode == 0, "DAILY_OPERATION_PRODUCTION_CONFIG")
    result = subprocess.run(
        [target.release + "/scripts/ops/kis-historical-price-beta-artifact.sh", "--approval-check"],
        env={**ENVIRONMENT, "LAGRANGE_RELEASE_ROOT": str(Path(target.release).parents[1])},
        stdin=subprocess.DEVNULL, capture_output=True,
        timeout=180, check=False)
    require(result.returncode == 0 and re.fullmatch(
        rb"HISTORICAL_PRICE_BETA_APPROVAL status=ok operation=check "
        rb"approval_registry_sha256=sha256:[0-9a-f]{64} approval_status=APPROVED "
        rb"audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY "
        rb"materialization_status=MATERIALIZED registration_status=UNREGISTERED "
        rb"publication_status=NOT_PUBLISHED instrument_count=11 session_count=2452 bar_count=26972\n",
        result.stdout), "DAILY_OPERATION_APPROVED_BASE")


def start_target(docker, target, binding, install_root, research_mode):
    for service in ("api-server", "owner-beta-runner"):
        runtime.image_owned(docker, Ownership(binding.project, target.commit, target.images[service]))
    if research_mode != "absent":
        result = subprocess.run(
            ["/usr/bin/python3", ROOT + "/scripts/ops/lib/service-status-policy.py",
             "startup", "--install-root", install_root, "--release-root", target.release,
             "--commit", target.commit], env=ENVIRONMENT, stdin=subprocess.DEVNULL,
            capture_output=True, timeout=30, check=False)
        require(result.returncode == 0 and result.stdout ==
                (research_mode + "|" + binding.project + "\n").encode("ascii"),
                "DAILY_OPERATION_PROJECT")
    descriptor, path = tempfile.mkstemp(prefix=".daily-image-override.", dir=install_root)
    try:
        with os.fdopen(descriptor, "w", encoding="ascii") as stream:
            stream.write("services:\n")
            for service in (*policy.MANIFEST_SERVICES, *runtime.SERVICES[:2]):
                canonical = "owner-beta-runner" if service in runtime.SERVICES[:2] else service
                stream.write(f"  {service}:\n    image: {target.images[canonical]}\n"
                             "    build: !reset null\n")
        require(stat.S_IMODE(os.stat(path).st_mode) == 0o600, "DAILY_OPERATION_OVERRIDE")
        args = ["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", "compose",
                "--project-name", binding.project, "--env-file",
                target.release + "/deploy/compose/.env", "-f",
                target.release + "/deploy/compose/compose.yml", *target.overlays, "-f",
                target.release + "/deploy/compose/compose.owner-beta-daily.yml"]
        if research_mode != "absent":
            args += ["-f", target.release + "/deploy/compose/compose.service-status.yml"]
        args += ["-f", path]
        env = {**ENVIRONMENT, "COMPOSE_PROFILES": "", "COMPOSE_PARALLEL_LIMIT": "1",
               "LAGRANGE_DAILY_VERIFIED_CONFIG_SHA256": binding.config_sha256}
        for command in (["config", "--quiet"],
                        ["up", "--no-build", "--no-deps", "--wait", "--wait-timeout", "180",
                         "api-server", *runtime.SERVICES]):
            result = subprocess.run([*args, *command], env=env, stdin=subprocess.DEVNULL,
                                    capture_output=True, timeout=240, check=False)
            require(result.returncode == 0, "DAILY_OPERATION_COMPOSE")
    finally:
        os.unlink(path)


def daily_present(docker):
    for service in runtime.SERVICES:
        ids = docker.call(["ps", "--all", "--no-trunc", "--filter",
                          "label=com.docker.compose.service=" + service,
                          "--format", "{{.ID}}"]).splitlines()
        require(all(runtime.CONTAINER.fullmatch(i) for i in ids), "DAILY_OPERATION_DISCOVERY")
        if service != "owner-beta-runner" and ids:
            return True
        for identity in ids:
            value = docker.object("container", identity)
            if runtime.environment(value).get("PRICE_BETA_DAILY_MODE") == "owner_only":
                return True
    return False


@contextlib.contextmanager
def inherited_lock(install_root, number):
    # The Bash caller retains this same open file description for the complete
    # normal release apply. A check followed by releasing the lock is insufficient.
    require(number is not None and re.fullmatch(r"[0-9]{1,5}", number),
            "DAILY_OPERATION_LOCK")
    descriptor = int(number)
    require(descriptor >= 3, "DAILY_OPERATION_LOCK")
    with install.root_directory(install_root) as parent:
        info = install.file_info(descriptor, 0, 0, 0o600)
        named = os.stat(".lagrange-release.lock", dir_fd=parent, follow_symlinks=False)
        require(info.st_size == 0 and install.file_identity(named) == install.file_identity(info),
                "DAILY_OPERATION_LOCK")
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield
        require(install.file_identity(os.stat(".lagrange-release.lock", dir_fd=parent,
                                             follow_symlinks=False)) ==
                install.file_identity(info), "DAILY_OPERATION_LOCK")
    # Do not LOCK_UN: the caller must retain the lock through its migrations/up.


def quiescence(operations, floor, *, check_database=True):
    docker, source, own = operations.docker, operations.source, operations.ownership
    present = daily_present(docker)
    if not floor and not present:
        return "LEGACY_OFF"
    global_cohort(docker, own)
    snapshot = runtime.capture(docker, own)
    stopped(docker, own, snapshot, allow_sealed_worker=True)
    if not check_database:
        return "PROCESSES_QUIESCENT"
    source.values, source.overlays = read_context(source.release, source.commit, "quiescent")
    status = operations.database.status(source.values)
    require(status in {"ABSENT_SCHEMA", "ABSENT_CONTROL", "DISABLED"},
            "DAILY_OPERATION_DATABASE_ENABLED")
    config, _ = install.verify_config(source.release, source.commit)
    if status in {"ABSENT_SCHEMA", "ABSENT_CONTROL"}:
        pristine_state(source.values["LAGRANGE_DATA_DIR"], config)
    else:
        install.check_state(source.values["LAGRANGE_DATA_DIR"], config)
    return "QUIESCENT"


def pristine_state(data_root, config):
    """Missing DB control is bootstrap only when no prior filesystem work exists."""
    with install.root_directory(data_root) as root:
        try:
            named = os.stat("owner-beta-daily", dir_fd=root, follow_symlinks=False)
        except FileNotFoundError:
            return
        require(stat.S_ISDIR(named.st_mode), "DAILY_OPERATION_HISTORY")
    install.check_state(data_root, config)
    with install.root_directory(data_root + "/owner-beta-daily") as parent:
        with contextlib.ExitStack() as stack:
            for name, uid in install.SHARED.items():
                fd = stack.enter_context(install.child_directory(parent, name, uid, 10001, 0o750))
                require(not os.listdir(fd), "DAILY_OPERATION_HISTORY")
            for name, (stage, uid) in install.STAGES.items():
                fd = stack.enter_context(install.child_directory(parent, name, uid, uid, 0o700))
                stack.enter_context(install.state_lock(fd, uid))
                require(install.read_file(fd, "checkpoint.json", uid, uid, 0o600) ==
                        install.checkpoint(stage, config), "DAILY_OPERATION_HISTORY")


class Arguments(argparse.ArgumentParser):
    def error(self, message):
        raise runtime.RuntimeErrorCode("DAILY_OPERATION_ARGUMENTS")


def main(argv=None):
    parser = Arguments(description=__doc__)
    parser.add_argument("--operation", choices=("prepare", "enable", "disable", "guard", "guard-processes"), required=True)
    parser.add_argument("--mode", choices=("plan", "preflight", "apply"), required=True)
    parser.add_argument("--install-root", required=True)
    parser.add_argument("--from-commit")
    parser.add_argument("--lock-fd")
    try:
        args = parser.parse_args(argv)
        require(args.operation in ("guard", "guard-processes") or args.lock_fd is None, "DAILY_OPERATION_ARGUMENTS")
        if args.mode == "plan":
            require(args.operation not in ("guard", "guard-processes"), "DAILY_OPERATION_ARGUMENTS")
            print("DAILY_OPERATION_PLAN: installed immutable binding; prepare disabled; "
                  "enable after actual cohort checks; disable database and drain")
            return 0
        install.require_root()
        require(runtime.absolute(args.install_root), "DAILY_OPERATION_ROOT")
        commit = Path(ROOT).name
        require(install.is_commit(commit), "DAILY_OPERATION_RELEASE")
        policy.validate_current_release(args.install_root, ROOT, commit)
        source_commit = args.from_commit or commit
        require(install.is_commit(source_commit), "DAILY_OPERATION_RELEASE")
        source_release = args.install_root + "/releases/" + source_commit
        target_images = read_manifest(ROOT, commit)
        source_images = target_images if source_commit == commit else read_manifest(source_release, source_commit)
        project, research_mode = project_binding(source_commit, commit)
        own = Ownership(project, source_commit, source_images["owner-beta-runner"])
        docker = runtime.Docker()
        database = Database(docker, project, commit, target_images["db-migrate"])
        target = Context(ROOT, commit, target_images, None, [])
        source = Context(source_release, source_commit, source_images, None, [])
        lock = (inherited_lock(args.install_root, args.lock_fd) if args.operation in ("guard", "guard-processes")
                else release_lock(args.install_root))
        with lock:
            policy.validate_current_release(args.install_root, ROOT, commit)
            operation = Operations(docker, database, source, target, own,
                                   args.install_root, research_mode)
            if args.operation in ("guard", "guard-processes"):
                require(args.mode == "apply", "DAILY_OPERATION_ARGUMENTS")
                with install.root_directory(args.install_root) as parent:
                    try:
                        floor = install.read_file(parent, install.FLOOR_NAME, 0, 0, 0o600)
                        require(floor == install.FLOOR_BYTES, "DAILY_OPERATION_READER")
                    except FileNotFoundError:
                        floor = None
                outcome = quiescence(operation, floor is not None,
                                     check_database=args.operation == "guard")
            else:
                global_cohort(docker, own)
                snapshot = runtime.capture(docker, own)
                if args.operation == "disable":
                    if args.mode == "apply":
                        operation.disable(snapshot)
                        outcome = "DISABLED_AND_DRAINED"
                    else:
                        source.values, source.overlays = read_context(source.release, source.commit, "disable")
                        database.status(source.values)
                        outcome = "PREFLIGHT"
                else:
                    target.values, target.overlays = read_context(ROOT, commit, args.operation)
                    reject_ambient(target.values)
                    binding, _ = activation_binding(ROOT, commit, project, target_images, target.values)
                    if args.mode == "preflight":
                        database.status(target.values)
                        if args.operation == "enable":
                            validate_activation_inputs(target)
                        outcome = "PREFLIGHT"
                    elif args.operation == "prepare":
                        operation.prepare(snapshot)
                        outcome = "PREPARED_DISABLED"
                    else:
                        outcome = operation.enable(snapshot)
                global_cohort(docker, Ownership(project, commit, target_images["owner-beta-runner"])
                              if args.operation == "enable" and args.mode == "apply" else own)
            policy.validate_current_release(args.install_root, ROOT, commit)
        print("DAILY_OPERATION: " + outcome)
        return 0
    except (runtime.RuntimeErrorCode, install.DailyInstallError):
        print("DAILY_OPERATION: FAILED", file=sys.stderr)
        return 1
    except Exception:
        # No input, SQL, Docker inspection, secret or provider text may escape.
        print("DAILY_OPERATION: FAILED", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
