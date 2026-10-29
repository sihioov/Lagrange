"""Bounded maintenance primitives for the official simulation runner.

Callers hold the release lock through maintenance. Starting the worker requires
the explicit start operation; maintenance never removes or resumes services.
"""
import argparse
import contextlib
import dataclasses
import hashlib
import importlib.util
import json
import math
import os
import posixpath
import re
import stat
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def _local(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


release = _local("simulation_operations_release", "owner-beta-simulation-release.py")
runtime = _local("simulation_operations_runtime", "owner-beta-simulation-runtime.py")
daily = _local("simulation_operations_daily", "owner-beta-daily-operations.py")


class OperationError(Exception):
    pass


def require(condition, code="SIMULATION_OPERATION_REJECTED"):
    if not condition:
        raise OperationError(code) from None


def binding_from_release(project, release_path, commit):
    """Read the immutable config via C1 and retain its exact original bytes."""
    config, digest = release.verify_config(release_path, commit)
    install = daily.install
    with install.root_directory(release_path) as parent:
        with install.child_directory(parent, ".owner-beta-simulation", 0, 10001, 0o750) as directory:
            raw = install.read_file(directory, "config.json", 0, 10001, 0o440, 16384)
    require("sha256:" + hashlib.sha256(raw).hexdigest() == digest, "SIMULATION_OPERATION_CONFIG_CHANGED")
    binding = runtime.Binding(project, release_path, commit, raw)
    require(release.validate_config(raw, commit) == config, "SIMULATION_OPERATION_CONFIG_CHANGED")
    return binding


def _bound_evidence(parent, commit, receipt_data, complete_data):
    install = daily.install
    manifest = install.read_file(parent, ".lagrange-release-manifest", 0, 0, 0o600, 65536)
    with install.child_directory(parent, ".owner-beta-simulation", 0, 10001, 0o750) as directory:
        require(set(os.listdir(directory)) == {"config.json"}, "SIMULATION_OPERATION_EVIDENCE")
        config = install.read_file(directory, "config.json", 0, 10001, 0o440, 16384)
    release.validate_binding(config, manifest, receipt_data, complete_data, commit)


def stage_receipt(stage, receipt_source, complete_source, commit):
    """Retain separately validated sidecars without changing C1's closed directory."""
    install = daily.install
    install.require_root()
    receipt_data = install.read_root_file(receipt_source, gid=0, mode=0o444, maximum=65536)
    complete_data = install.read_root_file(complete_source, gid=0, mode=0o444, maximum=65)
    with install.root_directory(stage) as parent:
        _bound_evidence(parent, commit, receipt_data, complete_data)
        os.mkdir(".owner-beta-simulation-receipt", 0o700, dir_fd=parent)
        os.fsync(parent)
        fd = os.open(".owner-beta-simulation-receipt", install.DIR_FLAGS, dir_fd=parent)
        try:
            install.directory_info(fd, 0, 0, 0o700)
            install.create_file(fd, "artifact.json", receipt_data, 0, 0, 0o444)
            install.create_file(fd, "COMPLETE", complete_data, 0, 0, 0o444)
            os.fchmod(fd, 0o755)
            install.directory_info(fd, 0, 0, 0o755)
            named = os.stat(".owner-beta-simulation-receipt", dir_fd=parent, follow_symlinks=False)
            current = os.fstat(fd)
            require(stat.S_ISDIR(named.st_mode) and (named.st_dev, named.st_ino) ==
                    (current.st_dev, current.st_ino), "SIMULATION_OPERATION_EVIDENCE")
            os.fsync(fd)
            os.fsync(parent)
        finally:
            os.close(fd)


def verify_receipt(release_path, commit):
    install = daily.install
    install.require_root()
    with install.root_directory(release_path) as parent:
        with install.child_directory(parent, ".owner-beta-simulation-receipt", 0, 0, 0o755) as fd:
            require(set(os.listdir(fd)) == {"artifact.json", "COMPLETE"}, "SIMULATION_OPERATION_EVIDENCE")
            receipt_data = install.read_file(fd, "artifact.json", 0, 0, 0o444, 65536)
            complete_data = install.read_file(fd, "COMPLETE", 0, 0, 0o444, 65)
        _bound_evidence(parent, commit, receipt_data, complete_data)


ARCHIVE_EVIDENCE = (
    "simulation_config/config.json", "simulation_receipt/artifact.json",
    "simulation_receipt/COMPLETE", "simulation_manifest/.lagrange-release-manifest",
    "simulation_floor/.owner-beta-simulation-reader-floor",
    "simulation_reader/owner-beta-simulation-reader-v1.json",
    "simulation_approval/kis-historical-price-only-v3-approved-artifacts.json",
)

# Existing adopted V3 registry pin, also compiled into market-data and the
# simulation contract; this is not an operator-provided or invented input.
APPROVAL_REGISTRY_SHA256 = "sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707"
APPROVAL_NAME = "kis-historical-price-only-v3-approved-artifacts.json"


def approved_record(registry):
    require(type(registry) is bytes and "sha256:" + hashlib.sha256(registry).hexdigest() ==
            APPROVAL_REGISTRY_SHA256, "SIMULATION_OPERATION_APPROVAL")
    value = daily.install.json_bytes(registry, maximum=256 * 1024)
    require(type(value) is dict and set(value) == {"schema_id", "schema_version", "approved_artifacts"}
            and value["schema_id"] == "kis-historical-price-only-v3-approval-registry"
            and type(value["schema_version"]) is int and value["schema_version"] == 1
            and type(value["approved_artifacts"]) is list and len(value["approved_artifacts"]) == 1,
            "SIMULATION_OPERATION_APPROVAL")
    record = value["approved_artifacts"][0]
    require(type(record) is dict and all(daily.install.is_hash(record.get(key)) for key in
            ("candidate_content_sha256", "artifact_manifest_sha256", "bars_sha256")) and
            type(record.get("bars_size_bytes")) is int and record["bars_size_bytes"] > 0,
            "SIMULATION_OPERATION_APPROVAL")
    return record


def read_approval(release_path):
    data = daily.install.read_root_file(release_path + "/configs/evidence/" + APPROVAL_NAME,
                                       gid=0, mode=0o644, maximum=256 * 1024)
    return data, approved_record(data)


def verify_archived_evidence(evidence, binding):
    """Require the exact installed identity and both monotonic floor records."""
    require(type(evidence) is dict and set(evidence) == set(ARCHIVE_EVIDENCE) and
            all(type(value) is bytes for value in evidence.values()), "SIMULATION_OPERATION_BACKUP")
    config = evidence["simulation_config/config.json"]
    require(config == binding.config_data, "SIMULATION_OPERATION_BACKUP")
    require(evidence["simulation_floor/.owner-beta-simulation-reader-floor"] == release.FLOOR_BYTES and
            evidence["simulation_reader/owner-beta-simulation-reader-v1.json"] == release.READER_BYTES,
            "SIMULATION_OPERATION_BACKUP")
    approved_record(evidence["simulation_approval/" + APPROVAL_NAME])
    release.validate_binding(config, evidence["simulation_manifest/.lagrange-release-manifest"],
        evidence["simulation_receipt/artifact.json"], evidence["simulation_receipt/COMPLETE"], binding.commit)


def verify_simulation_archive(stream, limits, binding, *, profile):
    """Verify all archive bytes before accepting any retained evidence."""
    require(profile in ("simulation", "daily-simulation"), "SIMULATION_OPERATION_BACKUP")
    files = _local("simulation_operations_backup_files", "owner-beta-daily-backup-files.py")
    registry, approval = read_approval(binding.release)
    candidate = approval["candidate_content_sha256"][7:]
    artifact_manifest = "simulation_input/" + candidate + "/manifest.json"
    manifest, evidence = files.verify_archive(stream, limits, (*ARCHIVE_EVIDENCE, artifact_manifest), profile=profile)
    artifact_data = evidence.pop(artifact_manifest)
    require(evidence["simulation_approval/" + APPROVAL_NAME] == registry, "SIMULATION_OPERATION_APPROVAL")
    entries = {item["path"]: item for item in manifest["entries"]}
    expected_metadata = {
        "simulation_config": (0, 10001, 0o750),
        "simulation_receipt": (0, 0, 0o755),
        "simulation_config/config.json": (0, 10001, 0o440),
        "simulation_receipt/artifact.json": (0, 0, 0o444),
        "simulation_receipt/COMPLETE": (0, 0, 0o444),
        "simulation_manifest/.lagrange-release-manifest": (0, 0, 0o600),
        "simulation_floor/.owner-beta-simulation-reader-floor": (0, 0, 0o600),
        "simulation_reader/owner-beta-simulation-reader-v1.json": (0, 0, 0o644),
        "simulation_approval/" + APPROVAL_NAME: (0, 0, 0o644),
    }
    require(all(path in entries and entries[path]["kind"] ==
                ("directory" if path in ("simulation_config", "simulation_receipt") else "file") and
                (entries[path]["uid"], entries[path]["gid"], entries[path]["mode"]) == metadata
                for path, metadata in expected_metadata.items()), "SIMULATION_OPERATION_BACKUP")
    base = "simulation_input/" + candidate
    bars_path = base + "/bars.ndjson"
    require({p for p in entries if p.startswith("simulation_input/")} == {base, artifact_manifest, bars_path} and
            entries["simulation_input"]["uid"] == 10001 and entries[base]["kind"] == "directory" and
            entries[base]["uid"] == 10001 and entries[base]["mode"] == 0o700 and
            all(entries[path]["kind"] == "file" and entries[path]["uid"] == 10001 and
                entries[path]["mode"] == 0o600 for path in (artifact_manifest, bars_path)) and
            entries[bars_path]["sha256"] == approval["bars_sha256"] and
            entries[bars_path]["size"] == approval["bars_size_bytes"], "SIMULATION_OPERATION_APPROVAL")
    artifact = daily.install.json_bytes(artifact_data, maximum=2 * 1024 * 1024)
    require(type(artifact) is dict and artifact.get("candidate_content_sha256") == approval["candidate_content_sha256"]
            and artifact.get("manifest_sha256") == approval["artifact_manifest_sha256"],
            "SIMULATION_OPERATION_APPROVAL")
    canonical = json.dumps(artifact, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8")
    require(artifact_data == canonical + b"\n", "SIMULATION_OPERATION_APPROVAL")
    unsigned = dict(artifact)
    del unsigned["manifest_sha256"]
    unsigned_bytes = json.dumps(unsigned, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8")
    require("sha256:" + hashlib.sha256(unsigned_bytes).hexdigest() == approval["artifact_manifest_sha256"],
            "SIMULATION_OPERATION_APPROVAL")
    verify_archived_evidence(evidence, binding)
    return manifest


def overlay_environment(binding):
    """Derive every overlay value from the already verified immutable config."""
    config = release.validate_config(binding.config_data, binding.commit)
    limits = config["limits"]
    nano = limits["nano_cpus"]
    cpus = f"{nano // 1000000000}.{nano % 1000000000:09d}"
    return {
        "LAGRANGE_SIM_CONFIG_SHA256": binding.config_sha256,
        "LAGRANGE_SIM_RECEIPT_SHA256": binding.receipt_sha256.removeprefix("sha256:"),
        "LAGRANGE_SIM_RECEIPT_SHA256_PREFIXED": binding.receipt_sha256,
        "LAGRANGE_SIM_CONFIG_PATH": binding.release + "/.owner-beta-simulation/config.json",
        "LAGRANGE_SIM_HISTORICAL_ROOT": binding.historical_root,
        "LAGRANGE_SIM_MEMORY_BYTES": str(limits["memory_bytes"]),
        "LAGRANGE_SIM_CPUS": cpus,
        "LAGRANGE_SIM_PIDS_LIMIT": str(limits["pids_limit"]),
        "LAGRANGE_SIM_STOP_SECONDS": str(limits["stop_timeout_seconds"]),
        "LAGRANGE_SIM_HEALTH_TMPFS_BYTES": str(limits["health_tmpfs_bytes"]),
        "LAGRANGE_SIM_HEALTH_MAX_AGE_SECONDS": str(limits["health_max_age_seconds"]),
        "LAGRANGE_SIM_STARTUP_SECONDS": str(limits["startup_timeout_seconds"]),
    }


def validate_worker_secret(secret_root):
    require(daily.runtime.absolute(secret_root), "SIMULATION_OPERATION_SECRET")
    with daily.install.root_directory(secret_root) as parent:
        with daily.install.child_directory(parent, "owner-beta-simulation-runner", 10001, 10001, 0o750) as service:
            secret = daily.install.read_file(service, "db_worker_password", 10001, 10001, 0o440, 8192)
            require(bool(secret) and not any(c in secret for c in (b"\0", b"\r", b"\n")),
                    "SIMULATION_OPERATION_SECRET")


@contextlib.contextmanager
def input_guard(binding):
    """Hold the validated historical root through the explicit start."""
    files = _local("simulation_start_files", "owner-beta-daily-backup-files.py")
    info = os.stat(binding.historical_root, follow_symlinks=False)
    require(stat.S_ISDIR(info.st_mode) and info.st_uid == 10001 and
            info.st_gid in files.OWNERS and not info.st_mode & 0o7022, "SIMULATION_OPERATION_INPUT")
    root = files.Root("simulation_input", binding.historical_root, info.st_uid, info.st_gid,
                      stat.S_IMODE(info.st_mode), parent=input_parent_identity(binding.historical_root))
    with files.open_root(root) as descriptor:
        yield descriptor


def input_parent_identity(path):
    """Use Daily's service-owned artifact-parent exception only when observed."""
    info = os.stat(os.path.dirname(path), follow_symlinks=False)
    require(stat.S_ISDIR(info.st_mode), "SIMULATION_OPERATION_INPUT")
    if info.st_uid == 0:
        return None
    mode = stat.S_IMODE(info.st_mode)
    require(info.st_gid == 10001 and mode == 0o750 and not info.st_mode & 0o7000,
            "SIMULATION_OPERATION_INPUT")
    return (info.st_uid, info.st_gid, mode)


def verify_runtime_policy(docker, sim_docker, binding, secret_root, database, *, running=True):
    """Check the actual container beyond C2's process-only ownership binding."""
    require(type(database) is str and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,62}", database),
            "SIMULATION_OPERATION_DATABASE")
    validate_worker_secret(secret_root)
    before = runtime.capture(sim_docker, binding)
    require(before.generation is not None and before.running is running, "SIMULATION_OPERATION_POLICY")
    value = docker.object("container", before.generation[0], 15)
    config, host, state = value.get("Config"), value.get("HostConfig"), value.get("State")
    require(all(type(v) is dict for v in (config, host, state)), "SIMULATION_OPERATION_POLICY")
    limits = release.validate_config(binding.config_data, binding.commit)["limits"]
    require(host.get("ReadonlyRootfs") is True and host.get("Init") is True and
            host.get("Privileged") is False and host.get("AutoRemove") is False and
            host.get("CapDrop") == ["ALL"] and not host.get("CapAdd") and
            host.get("SecurityOpt") == ["no-new-privileges:true"] and
            not any(host.get(k) for k in ("Devices", "DeviceRequests", "PortBindings", "VolumesFrom",
                                          "PidMode", "UTSMode", "ExtraHosts", "Links", "LxcConf")) and
            host.get("IpcMode") == "private" and not config.get("ExposedPorts"),
            "SIMULATION_OPERATION_POLICY")
    require(host.get("Memory") == limits["memory_bytes"] and host.get("NanoCpus") == limits["nano_cpus"] and
            host.get("PidsLimit") == limits["pids_limit"] and
            config.get("StopTimeout") == limits["stop_timeout_seconds"] and
            host.get("RestartPolicy", {}).get("Name") == "no", "SIMULATION_OPERATION_LIMITS")
    tmpfs_path = "/run/owner-beta-simulation-health/records"
    require(host.get("Tmpfs") == {
        "/run/owner-beta-simulation": "uid=0,gid=10001,mode=0750,size=4096",
        tmpfs_path: "uid=10001,gid=10001,mode=0700,size=" + str(limits["health_tmpfs_bytes"]),
    }, "SIMULATION_OPERATION_TMPFS")
    secret_path = secret_root + "/owner-beta-simulation-runner/db_worker_password"
    expected_mounts = {
        "/run/owner-beta-simulation/config.json": (binding.release + "/.owner-beta-simulation/config.json", False),
        "/data/owner-beta-artifacts": (binding.historical_root, False),
        "/run/secrets/db_worker_password": (secret_path, False),
    }
    mounts = value.get("Mounts")
    require(type(mounts) is list, "SIMULATION_OPERATION_MOUNT")
    actual = {}
    for mount in mounts:
        require(type(mount) is dict, "SIMULATION_OPERATION_MOUNT")
        target = mount.get("Destination")
        if mount.get("Type") == "tmpfs" and target in (tmpfs_path, "/run/owner-beta-simulation"):
            continue
        require(type(target) is str and target not in actual and mount.get("Type") == "bind" and
                mount.get("Propagation") == "rprivate", "SIMULATION_OPERATION_MOUNT")
        actual[target] = (mount.get("Source"), mount.get("RW"))
    require(actual == expected_mounts, "SIMULATION_OPERATION_MOUNT")
    env = daily.runtime.environment(value)
    required = {
        "APP_ENV": "production", "OWNER_BETA_SIMULATION_MODE": "owner_only",
        "OWNER_BETA_ACCESS_MODE": "owner_only", "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1",
        "LAGRANGE_CODE_COMMIT": binding.commit, "OWNER_BETA_SIMULATION_CONFIG_SHA256": binding.config_sha256,
        "OWNER_BETA_SIMULATION_RECEIPT_SHA256": binding.receipt_sha256.removeprefix("sha256:"),
        "DB_HOST": "postgres", "DB_PORT": "5432", "DB_NAME": database, "DB_USER": "worker",
        "DB_PASSWORD_FILE": "/run/secrets/db_worker_password",
    }
    require(all(env.get(k) == v for k, v in required.items()) and set(env) <= set(required) | {"PATH", "LANG"},
            "SIMULATION_OPERATION_ENV")
    health = config.get("Healthcheck")
    require(type(health) is dict and health.get("Test") == [
        "CMD", "/usr/local/bin/owner-beta-simulation-runner", "--health", "--expected-commit", binding.commit,
        "--expected-config-sha256", binding.config_sha256, "--expected-receipt-sha256",
        binding.receipt_sha256.removeprefix("sha256:")] and
        health.get("Interval") == limits["health_max_age_seconds"] * 1000000000 and
        health.get("Timeout") == limits["startup_timeout_seconds"] * 1000000000 and
        health.get("StartPeriod") == limits["startup_timeout_seconds"] * 1000000000 and
        health.get("Retries") == 1 and (not running or state.get("Health", {}).get("Status") == "healthy"),
        "SIMULATION_OPERATION_HEALTH")
    network = daily.internal_network(docker, binding.project)
    endpoints = value.get("NetworkSettings", {}).get("Networks", {})
    require(type(endpoints) is dict and host.get("NetworkMode") in (network["Id"], network["Name"]),
            "SIMULATION_OPERATION_NETWORK")
    if running:
        require(len(endpoints) == 1 and next(iter(endpoints.values())).get("NetworkID") == network["Id"],
                "SIMULATION_OPERATION_NETWORK")
    else:
        # Created/stopped containers can be disconnected until start. Bind the
        # configured network now and require the actual endpoint after start.
        require(not endpoints or (len(endpoints) == 1 and network["Name"] in endpoints and
                type(endpoints[network["Name"]]) is dict and
                endpoints[network["Name"]].get("NetworkID") in ("", network["Id"])),
                "SIMULATION_OPERATION_NETWORK")
    after = runtime.observe_unchanged(sim_docker, binding, before)
    require(after.generation == before.generation and after.running is running, "SIMULATION_OPERATION_POLICY")
    return after


@dataclasses.dataclass(frozen=True)
class ApiGeneration:
    identity: str
    created: str
    started: str
    restarts: int


def _api_ids(docker, timeout):
    text = docker.call(["ps", "--all", "--no-trunc", "--filter",
                        "label=com.docker.compose.service=api-server", "--format", "{{.ID}}"], timeout)
    require(type(text) is str and "\r" not in text, "SIMULATION_OPERATION_API")
    rows = text.splitlines()
    require(len(rows) <= 1 and all(re.fullmatch(r"[0-9a-f]{64}", row) for row in rows),
            "SIMULATION_OPERATION_API")
    return rows


def _api_observe(docker, project, commit, image, identity, timeout):
    value = docker.object("container", identity, timeout)
    require(type(value) is dict and value.get("Id") == identity and value.get("Image") == image,
            "SIMULATION_OPERATION_API")
    config, host, state = value.get("Config"), value.get("HostConfig"), value.get("State")
    require(all(type(part) is dict for part in (config, host, state)), "SIMULATION_OPERATION_API")
    labels = config.get("Labels")
    require(type(labels) is dict and all(labels.get(k) == v for k, v in {
        "com.docker.compose.project": project, "com.docker.compose.service": "api-server",
        "com.docker.compose.oneoff": "False", "org.opencontainers.image.revision": commit,
    }.items()), "SIMULATION_OPERATION_API")
    executable = "/usr/local/bin/api-server"
    require(config.get("User") == "10001:10001" and config.get("Entrypoint") == [executable] and
            config.get("Cmd") in (None, []) and value.get("Path") == executable and value.get("Args") == [],
            "SIMULATION_OPERATION_API")
    mounts = value.get("Mounts")
    require(type(mounts) is list, "SIMULATION_OPERATION_API")
    seen = set()
    for mount in mounts:
        require(type(mount) is dict and type(mount.get("Destination")) is str, "SIMULATION_OPERATION_API")
        destination = mount["Destination"]
        require(destination.startswith("/") and posixpath.normpath(destination) == destination and destination not in seen and
                destination != "/" and executable != destination and
                not executable.startswith(destination.rstrip("/") + "/"), "SIMULATION_OPERATION_API")
        seen.add(destination)
    restart = host.get("RestartPolicy")
    require(type(restart) is dict and restart.get("Name") in ("no", "unless-stopped") and
            type(restart.get("MaximumRetryCount")) is int and restart["MaximumRetryCount"] == 0,
            "SIMULATION_OPERATION_API")
    require(all(type(state.get(k)) is bool for k in ("Running", "Restarting", "Paused", "OOMKilled"))
            and not state["Restarting"] and not state["Paused"] and not state["OOMKilled"]
            and state.get("Error") == "" and type(state.get("ExitCode")) is int,
            "SIMULATION_OPERATION_API")
    require((state["Running"] and state.get("Status") == "running") or
            (not state["Running"] and state.get("Status") in ("created", "exited") and
             state["ExitCode"] == 0), "SIMULATION_OPERATION_API")
    generation = ApiGeneration(identity, value.get("Created"), state.get("StartedAt"), value.get("RestartCount"))
    require(type(generation.created) is str and generation.created and type(generation.started) is str
            and generation.started and type(generation.restarts) is int and generation.restarts >= 0,
            "SIMULATION_OPERATION_API")
    return generation, state["Running"], restart["Name"]


def _api_image_capability(docker, commit, image, timeout):
    image_object = docker.object("image", image, timeout)
    image_config = image_object.get("Config") if type(image_object) is dict else None
    labels = image_config.get("Labels") if type(image_config) is dict else None
    require(type(image_object) is dict and image_object.get("Id") == image and type(labels) is dict and
            labels.get("org.opencontainers.image.revision") == commit and
            labels.get("org.lagrange.api.http-drain-contract") == "v1" and
            image_config.get("User") == "10001:10001" and
            image_config.get("Entrypoint") == ["/usr/local/bin/api-server"] and
            image_config.get("Cmd") in (None, []), "SIMULATION_OPERATION_API")


def _fence_api_process(docker, project, commit, image, timeout, *, clock=time.monotonic, expected=None):
    """Observe a clean process exit; this alone is not a completed API fence."""
    require(type(timeout) is int and timeout > 0, "SIMULATION_OPERATION_DEADLINE")
    start = clock()
    require(type(start) in (int, float) and math.isfinite(start), "SIMULATION_OPERATION_DEADLINE")
    deadline = start + timeout
    def remaining():
        now = clock()
        require(type(now) in (int, float) and math.isfinite(now) and deadline > now,
                "SIMULATION_OPERATION_DEADLINE")
        return min(15, deadline - now)
    require(type(project) is str and re.fullmatch(r"[a-z0-9][a-z0-9_-]{0,62}", project) and
            type(commit) is str and re.fullmatch(r"[0-9a-f]{40}", commit) and commit != "0" * 40 and
            type(image) is str and re.fullmatch(r"sha256:[0-9a-f]{64}", image), "SIMULATION_OPERATION_API")
    _api_image_capability(docker, commit, image, remaining())
    ids = _api_ids(docker, remaining())
    require(len(ids) == 1, "SIMULATION_OPERATION_API")
    identity = ids[0]
    generation, running, policy = _api_observe(docker, project, commit, image, identity, remaining())
    require(running and (expected is None or generation == expected), "SIMULATION_OPERATION_API")
    require(_api_ids(docker, remaining()) == [identity], "SIMULATION_OPERATION_API")
    again, running, current_policy = _api_observe(docker, project, commit, image, identity, remaining())
    require(again == generation and running and policy == current_policy, "SIMULATION_OPERATION_API")
    docker.call(["update", "--restart=no", identity], remaining())
    require(_api_ids(docker, remaining()) == [identity], "SIMULATION_OPERATION_API")
    again, running, policy = _api_observe(docker, project, commit, image, identity, remaining())
    require(again == generation and running and policy == "no", "SIMULATION_OPERATION_API")
    docker.call(["kill", "--signal=TERM", identity], remaining())
    now = clock()
    require(type(now) in (int, float) and math.isfinite(now) and deadline > now,
            "SIMULATION_OPERATION_DEADLINE")
    require(docker.call(["wait", identity], deadline - now) in ("0", "0\n"), "SIMULATION_OPERATION_API")
    verify_fence(docker, project, commit, image, generation, remaining())
    remaining()
    return generation


def fence_api(docker, project, commit, image, timeout, *, clock=time.monotonic, expected=None):
    """Require v1 on the pinned image, then TERM and await the exact API generation."""
    try:
        return _fence_api_process(docker, project, commit, image, timeout, clock=clock, expected=expected)
    except OperationError:
        raise
    except Exception:
        raise OperationError("SIMULATION_OPERATION_API") from None


def verify_fence(docker, project, commit, image, generation, timeout):
    ids = _api_ids(docker, timeout)
    require(type(generation) is ApiGeneration and ids == [generation.identity], "SIMULATION_OPERATION_API")
    observed, running, policy = _api_observe(docker, project, commit, image, generation.identity, timeout)
    require(observed == generation and not running and policy == "no", "SIMULATION_OPERATION_API")


# Read-only, fixed SQL. The existing Daily transport uses migration_owner,
# whose forced-RLS simulation policies require enumerating all visible users.
# No owner, provider message or result body crosses the response boundary.
SETTLEMENT_SQL = """BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL statement_timeout = '30s';
SET LOCAL lock_timeout = '5s';
SET LOCAL search_path = pg_catalog, pg_temp;
DO $audit$
DECLARE actor uuid;
BEGIN
 IF current_user <> session_user OR current_user <> 'migration_owner'
    OR pg_catalog.row_security_active('public.users'::regclass) THEN
   RAISE EXCEPTION 'SIMULATION_AUDIT_VISIBILITY';
 END IF;
 IF (SELECT count(*) FROM public._sqlx_migrations WHERE version IN (63,64) AND success) <> 2 THEN
   RAISE EXCEPTION 'SIMULATION_AUDIT_MIGRATIONS';
 END IF;
 FOR actor IN SELECT id FROM public.users ORDER BY id LOOP
   PERFORM pg_catalog.set_config('app.actor_user_id', actor::text, true);
   IF EXISTS (
     SELECT 1 FROM public.jobs j
     LEFT JOIN public.owner_beta_simulation_runs r ON r.job_id=j.id AND r.owner_user_id=j.owner_user_id
     WHERE j.owner_user_id=actor AND j.job_type='owner_beta_price_only_simulation'
       AND (r.id IS NULL OR j.status='RUNNING' OR j.locked_by IS NOT NULL OR j.locked_at IS NOT NULL
            OR (j.status='QUEUED' AND r.status <> 'PENDING')
            OR (j.status IN ('SUCCEEDED','FAILED','CANCELED') AND r.status <> j.status))
   ) OR EXISTS (
     SELECT 1 FROM public.owner_beta_simulation_runs r
     LEFT JOIN public.jobs j ON j.id=r.job_id AND j.owner_user_id=r.owner_user_id
     LEFT JOIN public.owner_beta_simulation_result_pairs p ON p.run_id=r.id AND p.owner_user_id=r.owner_user_id
     WHERE r.owner_user_id=actor AND (j.id IS NULL OR j.job_type <> 'owner_beta_price_only_simulation'
       OR (r.status='SUCCEEDED' AND p.run_id IS NULL) OR (r.status <> 'SUCCEEDED' AND p.run_id IS NOT NULL))
   ) OR EXISTS (
     SELECT 1 FROM public.job_attempts a JOIN public.jobs j ON j.id=a.job_id
     WHERE j.owner_user_id=actor AND j.job_type='owner_beta_price_only_simulation' AND a.outcome='RUNNING'
   ) THEN RAISE EXCEPTION 'SIMULATION_AUDIT_UNSETTLED'; END IF;
 END LOOP;
END $audit$;
SELECT 'SIMULATION_MAINTENANCE: SETTLED';
ROLLBACK;
"""

# A missing/failed query is UNKNOWN and must never be converted to EMPTY.
# Enumerating users is permitted only after the migration-owner visibility
# check, then each forced-RLS query uses that exact actor.
HISTORY_SQL = """BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL statement_timeout = '30s';
SET LOCAL lock_timeout = '5s';
SET LOCAL search_path = pg_catalog, pg_temp;
DO $audit$
DECLARE actor uuid; have_runs boolean; have_pairs boolean;
BEGIN
 IF current_user <> session_user OR current_user <> 'migration_owner'
    OR pg_catalog.to_regclass('public.users') IS NULL
    OR pg_catalog.to_regclass('public.jobs') IS NULL
    OR pg_catalog.to_regclass('public._sqlx_migrations') IS NULL
    OR pg_catalog.row_security_active('public.users'::regclass) THEN
   RAISE EXCEPTION 'SIMULATION_HISTORY_VISIBILITY';
 END IF;
 have_runs := pg_catalog.to_regclass('public.owner_beta_simulation_runs') IS NOT NULL;
 have_pairs := pg_catalog.to_regclass('public.owner_beta_simulation_result_pairs') IS NOT NULL;
 IF have_runs IS DISTINCT FROM have_pairs THEN RAISE EXCEPTION 'SIMULATION_HISTORY_SCHEMA'; END IF;
 IF EXISTS (SELECT 1 FROM public._sqlx_migrations WHERE version=63 AND success) IS DISTINCT FROM have_runs THEN
   RAISE EXCEPTION 'SIMULATION_HISTORY_SCHEMA';
 END IF;
 PERFORM pg_catalog.set_config('lagrange.simulation_history', 'EMPTY', true);
 FOR actor IN SELECT id FROM public.users ORDER BY id LOOP
   PERFORM pg_catalog.set_config('app.actor_user_id', actor::text, true);
   IF EXISTS (SELECT 1 FROM public.jobs j WHERE j.owner_user_id=actor
              AND j.job_type='owner_beta_price_only_simulation') THEN
     PERFORM pg_catalog.set_config('lagrange.simulation_history', 'PRESENT', true);
   END IF;
   IF have_runs THEN
     IF EXISTS (SELECT 1 FROM public.owner_beta_simulation_runs r WHERE r.owner_user_id=actor)
        OR EXISTS (SELECT 1 FROM public.owner_beta_simulation_result_pairs p WHERE p.owner_user_id=actor) THEN
       PERFORM pg_catalog.set_config('lagrange.simulation_history', 'PRESENT', true);
     END IF;
   END IF;
 END LOOP;
END $audit$;
SELECT 'SIMULATION_HISTORY: ' || pg_catalog.current_setting('lagrange.simulation_history');
ROLLBACK;
"""


def history_present(database, values):
    result = database.execute(values, HISTORY_SQL, {"SIMULATION_HISTORY: EMPTY", "SIMULATION_HISTORY: PRESENT"})
    require(result in ("SIMULATION_HISTORY: EMPTY", "SIMULATION_HISTORY: PRESENT"),
            "SIMULATION_OPERATION_HISTORY")
    return result == "SIMULATION_HISTORY: PRESENT"


# This audit emits only one digest, never row values. It deliberately includes
# all historical rows for the fixed job type and the owner/entitlement tables;
# a missing privilege, effective filtering or missing migration is failure.
SIMULATION_AUDIT_SQL = """BEGIN;
SET LOCAL statement_timeout = '30s';
SET LOCAL lock_timeout = '5s';
SET LOCAL search_path = pg_catalog, pg_temp;
SET LOCAL TimeZone = 'UTC';
SET LOCAL DateStyle = 'ISO, YMD';
SET LOCAL extra_float_digits = 3;
CREATE TEMP TABLE simulation_backup_hashes (kind text NOT NULL, digest text NOT NULL) ON COMMIT DROP;
DO $audit$
DECLARE actor uuid;
BEGIN
 IF current_user <> session_user OR current_user NOT IN ('migration_owner','postgres')
    OR pg_catalog.row_security_active('public.users'::regclass)
    OR pg_catalog.row_security_active('public.data_entitlements'::regclass)
    OR (SELECT count(*) FROM public._sqlx_migrations WHERE version IN (63,64) AND success) <> 2 THEN
   RAISE EXCEPTION 'SIMULATION_BACKUP_VISIBILITY';
 END IF;
 INSERT INTO pg_temp.simulation_backup_hashes
   SELECT 'users', pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(pg_catalog.to_jsonb(u)::text,'UTF8')),'hex') FROM public.users u;
 INSERT INTO pg_temp.simulation_backup_hashes
   SELECT 'entitlements', pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(pg_catalog.to_jsonb(e)::text,'UTF8')),'hex') FROM public.data_entitlements e;
 INSERT INTO pg_temp.simulation_backup_hashes
   SELECT 'migrations', pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(pg_catalog.to_jsonb(m)::text,'UTF8')),'hex')
     FROM public._sqlx_migrations m WHERE m.version IN (63,64);
 FOR actor IN SELECT id FROM public.users ORDER BY id LOOP
   PERFORM pg_catalog.set_config('app.actor_user_id', actor::text, true);
   IF EXISTS (
     SELECT 1 FROM public.owner_beta_simulation_runs r
     LEFT JOIN public.jobs j ON j.id=r.job_id AND j.owner_user_id=r.owner_user_id
     LEFT JOIN public.user_strategy_configs c ON c.id=r.strategy_config_id AND c.owner_user_id=r.owner_user_id
     LEFT JOIN public.owner_beta_simulation_result_pairs p ON p.run_id=r.id AND p.owner_user_id=r.owner_user_id
     LEFT JOIN public.job_attempts a ON a.id=r.published_attempt_id AND a.job_id=r.job_id
     WHERE r.owner_user_id=actor AND (j.id IS NULL OR c.id IS NULL OR
       j.job_type <> 'owner_beta_price_only_simulation' OR
       NOT public.owner_beta_simulation_version_namespace_matches(j.payload_json->>'schema',
           j.payload_json->>'contract_document_sha256',j.idempotency_key) OR
       NOT public.owner_beta_simulation_payload_matches(j.payload_json,r.id,r.strategy_config_id,
           r.evaluation_start,r.evaluation_end,r.candidate_content_sha256,r.artifact_manifest_sha256,
           r.stage5_manifest_sha256,r.action_manifest_sha256,r.approval_registry_sha256,
           r.contract_document_sha256,r.bars_sha256,r.strategy_id,r.strategy_version,
           r.strategy_config_json,r.strategy_config_sha256) OR
       (r.status='SUCCEEDED' AND (p.run_id IS NULL OR a.id IS NULL OR a.outcome <> 'SUCCEEDED')) OR
       (r.status <> 'SUCCEEDED' AND p.run_id IS NOT NULL) OR
       (p.run_id IS NOT NULL AND (p.illustrative_sha256 <> 'sha256:' ||
          pg_catalog.encode(pg_catalog.sha256(p.illustrative_canonical),'hex') OR
          p.zero_cost_sha256 <> 'sha256:' || pg_catalog.encode(pg_catalog.sha256(p.zero_cost_canonical),'hex'))))
   ) OR EXISTS (
     SELECT 1 FROM public.jobs j LEFT JOIN public.owner_beta_simulation_runs r
       ON r.job_id=j.id AND r.owner_user_id=j.owner_user_id
     WHERE j.owner_user_id=actor AND j.job_type='owner_beta_price_only_simulation' AND
       (r.id IS NULL OR j.status='RUNNING' OR j.locked_by IS NOT NULL OR j.locked_at IS NOT NULL OR
        (j.status='QUEUED' AND r.status <> 'PENDING') OR
        (j.status IN ('SUCCEEDED','FAILED','CANCELED') AND r.status <> j.status))
   ) OR EXISTS (
     SELECT 1 FROM public.job_attempts a JOIN public.jobs j ON j.id=a.job_id
     WHERE j.owner_user_id=actor AND j.job_type='owner_beta_price_only_simulation' AND a.outcome='RUNNING'
   ) THEN RAISE EXCEPTION 'SIMULATION_BACKUP_BINDING'; END IF;
   INSERT INTO pg_temp.simulation_backup_hashes
     SELECT 'configs', pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(pg_catalog.to_jsonb(c)::text,'UTF8')),'hex')
       FROM public.user_strategy_configs c WHERE c.owner_user_id=actor;
   INSERT INTO pg_temp.simulation_backup_hashes
     SELECT 'jobs', pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(pg_catalog.to_jsonb(j)::text,'UTF8')),'hex')
       FROM public.jobs j WHERE j.owner_user_id=actor AND j.job_type='owner_beta_price_only_simulation';
   INSERT INTO pg_temp.simulation_backup_hashes
     SELECT 'attempts', pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(pg_catalog.to_jsonb(a)::text,'UTF8')),'hex')
       FROM public.job_attempts a JOIN public.jobs j ON j.id=a.job_id
       WHERE j.owner_user_id=actor AND j.job_type='owner_beta_price_only_simulation';
   INSERT INTO pg_temp.simulation_backup_hashes
     SELECT 'runs', pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(pg_catalog.to_jsonb(r)::text,'UTF8')),'hex')
       FROM public.owner_beta_simulation_runs r WHERE r.owner_user_id=actor;
   INSERT INTO pg_temp.simulation_backup_hashes
     SELECT 'pairs', pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(pg_catalog.to_jsonb(p)::text,'UTF8')),'hex')
       FROM public.owner_beta_simulation_result_pairs p WHERE p.owner_user_id=actor;
 END LOOP;
END $audit$;
SELECT 'SIMULATION_AUDIT: sha256:' || pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(
 COALESCE(pg_catalog.string_agg(kind || ':' || digest, E'\\n' ORDER BY kind COLLATE "C",digest COLLATE "C"),''),'UTF8')),'hex')
 FROM pg_temp.simulation_backup_hashes;
ROLLBACK;
"""


class _SimulationAuditResult:
    def __contains__(self, value):
        return type(value) is str and re.fullmatch(r"SIMULATION_AUDIT: sha256:[0-9a-f]{64}", value) is not None


def simulation_audit(database, values):
    value = database.execute(values, SIMULATION_AUDIT_SQL, _SimulationAuditResult())
    require(value in _SimulationAuditResult(), "SIMULATION_OPERATION_DATABASE")
    return value.removeprefix("SIMULATION_AUDIT: ")


def _present(parent_path, name):
    with daily.install.root_directory(parent_path) as parent:
        try:
            os.stat(name, dir_fd=parent, follow_symlinks=False)
        except FileNotFoundError:
            return False
        return True


START_PENDING = ".owner-beta-simulation-start-pending"
HANDOFF_PENDING = ".owner-beta-simulation-release-handoff"
PENDING_LIMIT = 16384


def _pending_read(root, name):
    require(name in (START_PENDING, HANDOFF_PENDING), "SIMULATION_OPERATION_PENDING")
    with daily.install.root_directory(root) as parent:
        try:
            data = daily.install.read_file(parent, name, 0, 0, 0o600, PENDING_LIMIT)
        except FileNotFoundError:
            return None
    value = daily.install.json_bytes(data, maximum=PENDING_LIMIT)
    require(type(value) is dict and daily.install.canonical(value) == data,
            "SIMULATION_OPERATION_PENDING")
    return value


def _pending_write(root, name, value, *, previous=None):
    data = daily.install.canonical(value)
    require(len(data) <= PENDING_LIMIT, "SIMULATION_OPERATION_PENDING")
    with daily.install.root_directory(root) as parent:
        if previous is None:
            daily.install.create_file(parent, name, data, 0, 0, 0o600)
        else:
            original = daily.install.canonical(previous)
            require(daily.install.read_file(parent, name, 0, 0, 0o600, PENDING_LIMIT) == original,
                    "SIMULATION_OPERATION_PENDING")
            temporary = name + "." + str(os.getpid()) + ".tmp"
            daily.install.create_file(parent, temporary, data, 0, 0, 0o600)
            require(daily.install.read_file(parent, name, 0, 0, 0o600, PENDING_LIMIT) == original,
                    "SIMULATION_OPERATION_PENDING")
            os.replace(temporary, name, src_dir_fd=parent, dst_dir_fd=parent)
        os.fsync(parent)


def _pending_remove(root, name, expected):
    with daily.install.root_directory(root) as parent:
        require(daily.install.read_file(parent, name, 0, 0, 0o600, PENDING_LIMIT) ==
                daily.install.canonical(expected), "SIMULATION_OPERATION_PENDING")
        os.unlink(name, dir_fd=parent)
        os.fsync(parent)


def _no_pending(root):
    require(_pending_read(root, START_PENDING) is None and _pending_read(root, HANDOFF_PENDING) is None,
            "SIMULATION_OPERATION_PENDING")


class StartFailure(OperationError):
    def __init__(self, *, interrupted=False, outcome_unknown=False, cleanup_unknown=False):
        self.interrupted = interrupted
        self.outcome_unknown = outcome_unknown
        self.cleanup_unknown = cleanup_unknown
        super().__init__("SIMULATION_OPERATION_START_FAILED")


class DeadlineDocker:
    """Bound every C2 observation by the retained operation deadline."""
    def __init__(self, docker, deadline, clock=None):
        self.docker, self.deadline = docker, deadline
        self.clock = time.monotonic if clock is None else clock

    def _timeout(self, requested):
        remaining = self.deadline - self.clock()
        require(type(remaining) in (int, float) and math.isfinite(remaining) and remaining > 0,
                "SIMULATION_OPERATION_DEADLINE")
        return min(requested, remaining)

    def call(self, args, timeout=15):
        return self.docker.call(args, self._timeout(timeout))

    def object(self, kind, identity, timeout=15):
        return self.docker.object(kind, identity, self._timeout(timeout))

    def service_ids(self, timeout):
        return self.docker.service_ids(self._timeout(timeout))


def _floor_present(install_root):
    with daily.install.root_directory(install_root) as parent:
        try:
            value = daily.install.read_file(parent, release.FLOOR_NAME, 0, 0, 0o600, len(release.FLOOR_BYTES))
        except FileNotFoundError:
            return False
        require(value == release.FLOOR_BYTES, "SIMULATION_OPERATION_FLOOR")
        return True


def _configured(path, commit):
    config = _present(path, ".owner-beta-simulation")
    receipt = _present(path, ".owner-beta-simulation-receipt")
    require(config == receipt, "SIMULATION_OPERATION_CONFIG")
    if config:
        release.verify_config(path, commit)
        verify_receipt(path, commit)
    return config


def _current(install_root):
    with daily.install.root_directory(install_root) as parent:
        try:
            target = os.readlink("current", dir_fd=parent)
        except FileNotFoundError:
            return None
    require(type(target) is str and re.fullmatch(r"releases/[0-9a-f]{40}", target) and
            target[-40:] != "0" * 40, "SIMULATION_OPERATION_RELEASE")
    commit = target[-40:]
    path = install_root + "/" + target
    daily.policy.validate_current_release(install_root, path, commit)
    return path, commit


@dataclasses.dataclass(frozen=True)
class MaintenanceContext:
    install_root: str
    target_release: str
    target_commit: str
    source_release: str
    source_commit: str
    project: str
    source_images: dict
    values: dict
    binding: object
    history: bool
    required: bool
    legacy_api: object


def legacy_admission_disabled(docker, project, commit, image, *, no_current=False):
    """Bind an already-disabled API generation; this is not a drain proof."""
    ids = _api_ids(docker, 15)
    if not ids:
        require(no_current and not _api_ids(docker, 15), "SIMULATION_OPERATION_API")
        return None
    require(len(ids) == 1, "SIMULATION_OPERATION_API")
    value = docker.object("image", image, 15)
    config = value.get("Config") if type(value) is dict else None
    labels = config.get("Labels") if type(config) is dict else None
    require(type(value) is dict and value.get("Id") == image and type(labels) is dict and
            labels.get("org.opencontainers.image.revision") == commit and
            config.get("Entrypoint") == ["/usr/local/bin/api-server"] and
            config.get("Cmd") in (None, []) and config.get("User") == "10001:10001",
            "SIMULATION_OPERATION_API")
    generation, running, _ = _api_observe(docker, project, commit, image, ids[0], 15)
    observed = docker.object("container", ids[0], 15)
    env = daily.runtime.environment(observed)
    require(running and env.get("OWNER_BETA_ACCESS_MODE") == "disabled" and
            env.get("OWNER_BETA_PRICE_INPUT_MODE") == "disabled" and
            _api_ids(docker, 15) == ids, "SIMULATION_OPERATION_API")
    again, still_running, _ = _api_observe(docker, project, commit, image, ids[0], 15)
    require(again == generation and still_running, "SIMULATION_OPERATION_API")
    return generation


def inspect_transition(install_root, target_release, target_commit, docker, sim_docker, *, handoff=None,
                       allow_start_pending=False):
    """Prove absence or bind both releases before a maintenance mutation."""
    if handoff is None:
        _no_pending(install_root)
    elif not allow_start_pending:
        require(_pending_read(install_root, START_PENDING) is None, "SIMULATION_OPERATION_PENDING")
    require(daily.runtime.absolute(install_root) and daily.install.is_commit(target_commit) and
            target_release == install_root + "/releases/" + target_commit,
            "SIMULATION_OPERATION_RELEASE")
    # C1 subsequently validates the target's exact manifest and reader bytes.
    daily.read_manifest(target_release, target_commit)
    current = _current(install_root)
    if handoff is None:
        source_release, source_commit = current if current is not None else (target_release, target_commit)
    else:
        require(handoff["target_commit"] == target_commit and current is not None and
                current[1] in (handoff["source_commit"], handoff["target_commit"]),
                "SIMULATION_OPERATION_HANDOFF")
        source_commit = handoff["source_commit"]
        source_release = install_root + "/releases/" + source_commit
    source_images = daily.read_manifest(source_release, source_commit)
    project, _ = daily.project_binding(source_commit, target_commit)
    values, _ = daily.read_context(source_release, source_commit, "disable")
    target_values, _ = daily.read_context(target_release, target_commit, "disable")
    target_access = target_values.get("OWNER_BETA_ACCESS_MODE", "")
    target_input = target_values.get("OWNER_BETA_PRICE_INPUT_MODE", "")
    require(target_access in ("", "disabled", "owner_only") and
            target_input in ("", "disabled", "sealed_v1"), "SIMULATION_OPERATION_MODE")
    target_admits = target_access == "owner_only" and target_input == "sealed_v1"
    database = daily.Database(docker, project, source_commit, source_images["db-migrate"])
    floor = _floor_present(install_root)
    source_config = _configured(source_release, source_commit)
    target_config = _configured(target_release, target_commit)
    worker_ids = sim_docker.service_ids(15)
    require(type(worker_ids) is list and len(worker_ids) <= 1, "SIMULATION_OPERATION_SINGLETON")
    legacy_api = None
    if not (floor or source_config or target_config or worker_ids or target_admits):
        # A live API with admission enabled can create history immediately
        # after an empty snapshot. It is not positively inapplicable.
        legacy_api = legacy_admission_disabled(docker, project, source_commit,
                                               source_images["api-server"], no_current=current is None)
    present = history_present(database, values)
    required = bool(present or floor or source_config or target_config or worker_ids or target_admits)
    if not required:
        require(legacy_admission_disabled(docker, project, source_commit, source_images["api-server"],
                                          no_current=current is None) == legacy_api and
                sim_docker.service_ids(15) == [], "SIMULATION_OPERATION_GENERATION")
    release.reader_guard(install_root, target_release, target_commit, "disabled",
                         history_present=required, reserve=False)
    binding = binding_from_release(project, source_release, source_commit) if source_config else None
    require(not worker_ids or binding is not None, "SIMULATION_OPERATION_CONFIG")
    require(not present or binding is not None, "SIMULATION_OPERATION_CONFIG")
    require(not required or current is not None, "SIMULATION_OPERATION_SOURCE")
    return MaintenanceContext(install_root, target_release, target_commit, source_release, source_commit,
                              project, source_images, values, binding, present, required, legacy_api), database


def _worker_absent(sim_docker):
    require(sim_docker.service_ids(15) == [], "SIMULATION_OPERATION_SINGLETON")


def prepare_transition(context, database, docker, sim_docker, *, expected_api=None, worker_snapshot=None):
    """Fence and settle without ever starting or deleting a service."""
    if not context.required:
        _worker_absent(sim_docker)
        again = legacy_admission_disabled(docker, context.project, context.source_commit,
                                          context.source_images["api-server"],
                                          no_current=context.legacy_api is None)
        require(again == context.legacy_api and not history_present(database, context.values),
                "SIMULATION_OPERATION_HISTORY")
        return None, None
    image = context.source_images["api-server"]
    if context.binding is not None:
        return drain_for_maintenance(docker, sim_docker, database, context.values, context.binding, image,
                                     expected_api=expected_api, snapshot=worker_snapshot)
    # The target may first introduce a config while the source has no worker.
    # No config means no adopted timeout: this path must be supplied from the
    # target's validated protected limits rather than an operational default.
    target = binding_from_release(context.project, context.target_release, context.target_commit)
    _worker_absent(sim_docker)
    api = fence_api(docker, context.project, context.source_commit, image, target.stop_timeout_seconds,
                    expected=expected_api)
    _worker_absent(sim_docker)
    verify_fence(docker, context.project, context.source_commit, image, api, 15)
    require(database.execute(context.values, SETTLEMENT_SQL, {"SIMULATION_MAINTENANCE: SETTLED"}) ==
            "SIMULATION_MAINTENANCE: SETTLED", "SIMULATION_OPERATION_DATABASE")
    _worker_absent(sim_docker)
    verify_fence(docker, context.project, context.source_commit, image, api, 15)
    return api, None


def _manifest_digest(path):
    raw = daily.install.read_root_file(path + "/.lagrange-release-manifest", gid=0, mode=0o600,
                                       maximum=65536)
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def _binding_pins(binding):
    if binding is None:
        return None
    return {"image": binding.image_id, "config": binding.config_sha256, "receipt": binding.receipt_sha256}


def _handoff_read(root):
    value = _pending_read(root, HANDOFF_PENDING)
    if value is None:
        return None
    keys = {"schema", "phase", "project", "source_commit", "target_commit", "source_manifest",
            "target_manifest", "source_api_image", "target_api_image", "source_binding", "target_binding",
            "api_generation", "worker_generation", "worker_running", "worker_policy",
            "target_api_generation", "api_attempt"}
    require(set(value) == keys and value["schema"] == "owner-beta-simulation-handoff-v1" and
            value["phase"] in ("fencing", "prepared", "switched", "api-starting", "api-started", "worker-replacing") and
            daily.install.is_commit(value["source_commit"]) and daily.install.is_commit(value["target_commit"]) and
            type(value["project"]) is str and re.fullmatch(r"[a-z0-9][a-z0-9_-]{0,62}", value["project"]) and
            type(value["api_attempt"]) is str and re.fullmatch(r"[0-9a-f]{32}", value["api_attempt"]) and
            all(daily.install.is_hash(value[k]) for k in
                ("source_manifest", "target_manifest", "source_api_image", "target_api_image")),
            "SIMULATION_OPERATION_HANDOFF")
    for pins in (value["source_binding"], value["target_binding"]):
        require(pins is None or (type(pins) is dict and set(pins) == {"image", "config", "receipt"} and
                all(daily.install.is_hash(pins[k]) for k in pins)), "SIMULATION_OPERATION_HANDOFF")
    for key in ("api_generation", "worker_generation", "target_api_generation"):
        generation = value[key]
        require((key in ("worker_generation", "target_api_generation") and generation is None) or
                (type(generation) is list and len(generation) == 4 and
                 type(generation[0]) is str and re.fullmatch(r"[0-9a-f]{64}", generation[0]) and
                 all(type(generation[i]) is str and generation[i] for i in (1, 2)) and
                 type(generation[3]) is int and generation[3] >= 0), "SIMULATION_OPERATION_HANDOFF")
    require((value["target_api_generation"] is not None) ==
            (value["phase"] in ("api-started", "worker-replacing")) and
            (value["worker_generation"] is None or value["source_binding"] is not None) and
            (value["phase"] not in ("api-started", "worker-replacing") or value["worker_generation"] is not None) and
            (value["phase"] != "worker-replacing" or value["target_binding"] is not None) and
            type(value["worker_running"]) is bool and
            ((value["worker_generation"] is None and value["worker_policy"] is None and not value["worker_running"]) or
             (value["worker_generation"] is not None and value["worker_policy"] in ("no", "unless-stopped"))),
            "SIMULATION_OPERATION_HANDOFF")
    return value


def _handoff_make(context, api, worker):
    require(type(api) is ApiGeneration, "SIMULATION_OPERATION_HANDOFF")
    target_images = daily.read_manifest(context.target_release, context.target_commit)
    target_binding = (binding_from_release(context.project, context.target_release, context.target_commit)
                      if _configured(context.target_release, context.target_commit) else None)
    return {"schema": "owner-beta-simulation-handoff-v1", "phase": "prepared", "project": context.project,
            "source_commit": context.source_commit, "target_commit": context.target_commit,
            "source_manifest": _manifest_digest(context.source_release),
            "target_manifest": _manifest_digest(context.target_release),
            "source_api_image": context.source_images["api-server"], "target_api_image": target_images["api-server"],
            "source_binding": _binding_pins(context.binding), "target_binding": _binding_pins(target_binding),
            "api_attempt": os.urandom(16).hex(),
            "api_generation": [api.identity, api.created, api.started, api.restarts],
            "target_api_generation": None,
            "worker_generation": None if worker is None or worker.generation is None else list(worker.generation),
            "worker_running": False if worker is None else getattr(worker, "running", False),
            "worker_policy": (None if worker is None or worker.generation is None else
                              getattr(worker, "restart_policy", "no"))}


def _handoff_context(root, target, commit, docker, sim_docker, record, *, allow_start_pending=False):
    context, database = inspect_transition(root, target, commit, docker, sim_docker, handoff=record,
                                           allow_start_pending=allow_start_pending)
    target_images = daily.read_manifest(target, commit)
    target_binding = (binding_from_release(context.project, target, commit) if _configured(target, commit) else None)
    require(context.required and context.project == record["project"] and
            _manifest_digest(context.source_release) == record["source_manifest"] and
            _manifest_digest(target) == record["target_manifest"] and
            context.source_images["api-server"] == record["source_api_image"] and
            target_images["api-server"] == record["target_api_image"] and
            _binding_pins(context.binding) == record["source_binding"] and
            _binding_pins(target_binding) == record["target_binding"], "SIMULATION_OPERATION_HANDOFF")
    _api_image_capability(docker, context.source_commit, record["source_api_image"], 15)
    _api_image_capability(docker, commit, record["target_api_image"], 15)
    return context, database, target_binding


def _handoff_stopped(context, database, docker, sim_docker, record):
    generation = ApiGeneration(*record["api_generation"])
    verify_fence(docker, context.project, context.source_commit, record["source_api_image"], generation, 15)
    if record["worker_generation"] is None:
        _worker_absent(sim_docker)
    else:
        require(context.binding is not None, "SIMULATION_OPERATION_HANDOFF")
        verify_drained(sim_docker, context.binding, tuple(record["worker_generation"]))
    require(database.execute(context.values, SETTLEMENT_SQL, {"SIMULATION_MAINTENANCE: SETTLED"}) ==
            "SIMULATION_MAINTENANCE: SETTLED", "SIMULATION_OPERATION_DATABASE")
    verify_fence(docker, context.project, context.source_commit, record["source_api_image"], generation, 15)
    if record["worker_generation"] is None:
        _worker_absent(sim_docker)
    else:
        verify_drained(sim_docker, context.binding, tuple(record["worker_generation"]))


def _capture_fencing(context, docker, sim_docker):
    image = context.source_images["api-server"]
    _api_image_capability(docker, context.source_commit, image, 15)
    ids = _api_ids(docker, 15)
    require(len(ids) == 1, "SIMULATION_OPERATION_API")
    api, running, _ = _api_observe(docker, context.project, context.source_commit, image, ids[0], 15)
    require(running and _api_ids(docker, 15) == ids, "SIMULATION_OPERATION_API")
    if context.binding is None:
        _worker_absent(sim_docker)
        worker = None
    else:
        worker = runtime.capture(sim_docker, context.binding)
    again, running, _ = _api_observe(docker, context.project, context.source_commit, image, ids[0], 15)
    require(again == api and running and _api_ids(docker, 15) == ids, "SIMULATION_OPERATION_API")
    record = _handoff_make(context, api, worker)
    record["phase"] = "fencing"
    return record, worker


def _retry_fencing(context, database, docker, sim_docker, record):
    require(_current(context.install_root) == (context.source_release, context.source_commit),
            "SIMULATION_OPERATION_HANDOFF")
    expected = ApiGeneration(*record["api_generation"])
    ids = _api_ids(docker, 15)
    require(ids == [expected.identity], "SIMULATION_OPERATION_API")
    observed, running, _ = _api_observe(docker, context.project, context.source_commit,
                                         record["source_api_image"], ids[0], 15)
    require(observed == expected and running, "SIMULATION_OPERATION_API")
    if context.binding is None:
        _worker_absent(sim_docker)
        snapshot = None
    else:
        snapshot = runtime.capture(sim_docker, context.binding)
        generation = None if record["worker_generation"] is None else tuple(record["worker_generation"])
        require(snapshot.generation == generation and snapshot.running == record["worker_running"] and
                snapshot.restart_policy == record["worker_policy"], "SIMULATION_OPERATION_GENERATION")
    api, worker = prepare_transition(context, database, docker, sim_docker,
                                     expected_api=expected, worker_snapshot=snapshot)
    require(api == expected and (None if worker is None or worker.generation is None else list(worker.generation)) ==
            record["worker_generation"], "SIMULATION_OPERATION_GENERATION")
    updated = dict(record, phase="prepared")
    _pending_write(context.install_root, HANDOFF_PENDING, updated, previous=record)
    return updated


def start_simulation(context, docker, sim_docker, database, *, replacement=None):
    """Explicit start only; every Compose value derives from protected binding."""
    if replacement is None:
        _no_pending(context.install_root)
    else:
        require(_pending_read(context.install_root, START_PENDING) is None and
                _handoff_read(context.install_root) == replacement[3] and
                replacement[3]["phase"] == "api-started", "SIMULATION_OPERATION_PENDING")
    binding = context.binding
    require(context.required and binding is not None and context.source_release == context.target_release and
            context.source_commit == context.target_commit, "SIMULATION_OPERATION_CONFIG")
    values = context.values
    require(values["OWNER_BETA_ACCESS_MODE"] == "owner_only" and
            values["OWNER_BETA_PRICE_INPUT_MODE"] == "sealed_v1" and
            values["OWNER_BETA_PAPER_MODE"] in ("", "disabled") and values["POSTGRES_DB"],
            "SIMULATION_OPERATION_MODE")
    validate_worker_secret(values["LAGRANGE_RUNTIME_SECRET_DIR"])
    previous_binding = binding if replacement is None else replacement[0]
    initial = runtime.capture(sim_docker, previous_binding)
    if replacement is not None:
        require(initial.generation == tuple(replacement[2]), "SIMULATION_OPERATION_GENERATION")
    require(not initial.running and (initial.generation is None or initial.restart_policy == "no"),
            "SIMULATION_OPERATION_SINGLETON")
    require(database.execute(values, SETTLEMENT_SQL, {"SIMULATION_MAINTENANCE: SETTLED"}) ==
            "SIMULATION_MAINTENANCE: SETTLED", "SIMULATION_OPERATION_DATABASE")
    image = context.source_images["api-server"]
    _api_image_capability(docker, context.source_commit, image, 15)
    api_ids = _api_ids(docker, 15)
    require(len(api_ids) == 1, "SIMULATION_OPERATION_API")
    api_generation, running, _ = _api_observe(docker, context.project, context.source_commit, image, api_ids[0], 15)
    api_value = docker.object("container", api_ids[0], 15)
    api_env = daily.runtime.environment(api_value)
    require(running and _api_ids(docker, 15) == api_ids and
            api_env.get("OWNER_BETA_ACCESS_MODE") == "owner_only" and
            api_env.get("OWNER_BETA_PRICE_INPUT_MODE") == "sealed_v1" and
            api_env.get("LAGRANGE_CODE_COMMIT") == context.source_commit, "SIMULATION_OPERATION_API")
    release.reader_guard(context.install_root, context.target_release, context.target_commit,
                         "owner_only", history_present=True, reserve=True)
    config = release.validate_config(binding.config_data, binding.commit)
    startup = config["limits"]["startup_timeout_seconds"]
    if initial.generation is not None:
        old_values = values if replacement is None else replacement[1]
        stopped = verify_runtime_policy(docker, sim_docker, previous_binding, old_values["LAGRANGE_RUNTIME_SECRET_DIR"],
                                        old_values["POSTGRES_DB"], running=False)
        require(stopped.generation == initial.generation, "SIMULATION_OPERATION_GENERATION")
    env = dict(daily.ENVIRONMENT)
    env.update(overlay_environment(binding))
    env.update({"LAGRANGE_CODE_COMMIT": binding.commit, "LAGRANGE_RUNTIME_SECRET_DIR": values["LAGRANGE_RUNTIME_SECRET_DIR"],
                "POSTGRES_DB": values["POSTGRES_DB"], "COMPOSE_PARALLEL_LIMIT": "1", "COMPOSE_PROFILES": "",
                "RANGE_RAW_BATCH_ID": "compose-config-disabled"})
    guard = contextlib.ExitStack()
    guard.enter_context(input_guard(binding))
    try:
        fd, override = tempfile.mkstemp(prefix=".simulation-image-", dir=context.install_root)
    except BaseException:
        guard.close()
        raise
    pending = None
    owned_snapshot = None
    start_requested = False
    start_uncertain = False
    try:
        os.fchmod(fd, 0o600)
        attempt = os.urandom(16).hex()
        payload = ("services:\n  owner-beta-simulation-runner:\n    image: " + binding.image_id +
                   "\n    build: !reset null\n    labels:\n      org.lagrange.simulation-start-attempt: " +
                   attempt + "\n").encode("ascii")
        with os.fdopen(fd, "wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        fd = -1
        again, running, _ = _api_observe(docker, context.project, context.source_commit, image, api_ids[0], 15)
        require(again == api_generation and running and _api_ids(docker, 15) == api_ids,
                "SIMULATION_OPERATION_API")
        deadline = time.monotonic() + startup
        bounded = DeadlineDocker(sim_docker, deadline)
        bounded_api = DeadlineDocker(docker, deadline)
        pending = {"schema": "owner-beta-simulation-start-pending-v1", "project": binding.project,
                   "commit": binding.commit, "image": binding.image_id, "config": binding.config_sha256,
                   "api_image": image, "api_generation": [api_generation.identity, api_generation.created,
                                                             api_generation.started, api_generation.restarts],
                   "receipt": binding.receipt_sha256, "attempt": attempt,
                   "phase": "creating" if initial.generation is None or replacement is not None else "owned",
                   # The old stopped generation belongs to HANDOFF_PENDING.
                   # Until replacement capture succeeds, this target attempt
                   # has no generation, even when the source worker exists.
                   "generation": None if initial.generation is None or replacement is not None else list(initial.generation),
                   "primary_failure": False, "interrupted": False,
                   "start_uncertain": False, "cleanup_uncertain": False}
        _pending_write(context.install_root, START_PENDING, pending)
        if initial.generation is None or replacement is not None:
            if replacement is None:
                _worker_absent(sim_docker)
            else:
                runtime.observe_unchanged(bounded, previous_binding, initial)
                updated_handoff = dict(replacement[3], phase="worker-replacing")
                _pending_write(context.install_root, HANDOFF_PENDING, updated_handoff, previous=replacement[3])
                replacement = (*replacement[:3], updated_handoff)
            argv = ["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", "compose",
                "--env-file", context.source_release + "/deploy/compose/.env", "--project-name", context.project,
                "-f", context.source_release + "/deploy/compose/compose.yml", "-f",
                str(Path(__file__).with_name("owner-beta-simulation-runtime-overlay.yml")), "-f", override,
                "--profile", "owner-beta-simulation", "up", "--no-start"]
            if replacement is None:
                argv.append("--no-recreate")
            argv.extend(["--no-build", "--pull", "never", "--no-deps", "owner-beta-simulation-runner"])
            result = subprocess.run(argv, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                    stderr=subprocess.DEVNULL, timeout=bounded._timeout(startup), check=False)
            require(result.returncode == 0, "SIMULATION_OPERATION_START")
            initial = runtime.capture(bounded, binding)
            require(initial.generation is not None and not initial.running and initial.restart_policy == "no",
                    "SIMULATION_OPERATION_START")
            created = bounded_api.object("container", initial.generation[0])
            require(created.get("Config", {}).get("Labels", {}).get("org.lagrange.simulation-start-attempt") == attempt,
                    "SIMULATION_OPERATION_GENERATION")
            stopped = verify_runtime_policy(bounded_api, bounded, binding, values["LAGRANGE_RUNTIME_SECRET_DIR"],
                                            values["POSTGRES_DB"], running=False)
            require(stopped.generation == initial.generation, "SIMULATION_OPERATION_GENERATION")
        else:
            runtime.observe_unchanged(bounded, binding, initial)
        updated = dict(pending, phase="owned", generation=list(initial.generation))
        _pending_write(context.install_root, START_PENDING, updated, previous=pending)
        pending = updated
        again, running, _ = _api_observe(bounded_api, context.project, context.source_commit, image, api_ids[0],
                                          bounded._timeout(15))
        require(again == api_generation and running and _api_ids(bounded_api, bounded._timeout(15)) == api_ids,
                "SIMULATION_OPERATION_API")
        runtime.observe_unchanged(bounded, binding, initial)
        updated = dict(pending, phase="start-requested")
        _pending_write(context.install_root, START_PENDING, updated, previous=pending)
        pending = updated
        start_requested = True
        start_uncertain = True
        answer = bounded_api.call(["start", initial.generation[0]], bounded._timeout(startup))
        require(answer.strip() == initial.generation[0], "SIMULATION_OPERATION_START")
        start_uncertain = False
        while True:
            remaining = bounded._timeout(startup)
            observed = runtime.capture(bounded, binding)
            require(observed.running and observed.generation is not None and
                    observed.generation[0:2] == initial.generation[0:2] and
                    observed.generation[3] == initial.generation[3], "SIMULATION_OPERATION_GENERATION")
            owned_snapshot = observed
            state = bounded_api.object("container", observed.generation[0], remaining).get("State", {})
            health = state.get("Health", {}).get("Status")
            require(health in ("starting", "healthy"), "SIMULATION_OPERATION_HEALTH")
            if health == "healthy":
                break
            time.sleep(min(0.2, remaining))
        snapshot = verify_runtime_policy(bounded_api, bounded, binding, values["LAGRANGE_RUNTIME_SECRET_DIR"],
                                         values["POSTGRES_DB"])
        require(snapshot.generation == owned_snapshot.generation, "SIMULATION_OPERATION_GENERATION")
        again, running, _ = _api_observe(bounded_api, context.project, context.source_commit, image, api_ids[0], 15)
        require(again == api_generation and running and _api_ids(bounded_api, 15) == api_ids,
                "SIMULATION_OPERATION_API")
        # Revalidate the retained input path before accepting startup. A
        # changed path follows the same exact-generation drain on failure.
        guard.close()
        os.unlink(override)
        override = None
        if replacement is not None:
            _pending_remove(context.install_root, HANDOFF_PENDING, replacement[3])
        _pending_remove(context.install_root, START_PENDING, pending)
        pending = None
        return snapshot
    except BaseException as primary:
        cleanup_unknown = False
        interrupted = not isinstance(primary, Exception)
        try:
            if pending is not None and start_requested:
                cleanup_deadline = time.monotonic() + binding.stop_timeout_seconds
                cleanup_docker = DeadlineDocker(sim_docker, cleanup_deadline)
                observed = runtime.capture(cleanup_docker, binding)
                require(observed.generation is not None and
                        observed.generation[0:2] == initial.generation[0:2] and
                        observed.generation[3] == initial.generation[3], "SIMULATION_OPERATION_GENERATION")
                result = runtime.drain(cleanup_docker, binding, observed)
                verify_drained(cleanup_docker, binding, result.generation)
                require(database.execute(values, SETTLEMENT_SQL, {"SIMULATION_MAINTENANCE: SETTLED"}) ==
                        "SIMULATION_MAINTENANCE: SETTLED", "SIMULATION_OPERATION_DATABASE")
            elif pending is not None and pending["generation"] is None:
                # A failed create may still have created a container. Only the
                # protected per-attempt label can identify it; never start it
                # from this failure path.
                cleanup_deadline = time.monotonic() + binding.stop_timeout_seconds
                probe = DeadlineDocker(sim_docker, cleanup_deadline)
                captured = runtime.capture(probe, binding)
                if captured.generation is not None:
                    require(not captured.running and captured.restart_policy == "no", "SIMULATION_OPERATION_START")
                    value = docker.object("container", captured.generation[0], probe._timeout(15))
                    require(value.get("Config", {}).get("Labels", {}).get("org.lagrange.simulation-start-attempt") ==
                            pending["attempt"], "SIMULATION_OPERATION_GENERATION")
                    checked = verify_runtime_policy(DeadlineDocker(docker, cleanup_deadline), probe, binding,
                                                    values["LAGRANGE_RUNTIME_SECRET_DIR"], values["POSTGRES_DB"],
                                                    running=False)
                    require(checked.generation == captured.generation, "SIMULATION_OPERATION_GENERATION")
                    updated = dict(pending, phase="owned", generation=list(captured.generation))
                    _pending_write(context.install_root, START_PENDING, updated, previous=pending)
                    pending = updated
        except Exception:
            cleanup_unknown = True
        if pending is not None:
            try:
                failed = dict(pending, phase="failed", primary_failure=True, interrupted=interrupted,
                              start_uncertain=start_uncertain, cleanup_uncertain=cleanup_unknown)
                _pending_write(context.install_root, START_PENDING, failed, previous=pending)
                pending = failed
            except Exception:
                cleanup_unknown = True
        raise StartFailure(interrupted=interrupted, outcome_unknown=start_uncertain,
                           cleanup_unknown=cleanup_unknown) from None
    finally:
        active = sys.exc_info()[1]
        try:
            if fd >= 0:
                os.close(fd)
            if override is not None:
                os.unlink(override)
            guard.close()
        except BaseException:
            if isinstance(active, StartFailure):
                active.cleanup_unknown = True
                if pending is not None:
                    try:
                        updated = dict(pending, cleanup_uncertain=True)
                        _pending_write(context.install_root, START_PENDING, updated, previous=pending)
                    except Exception:
                        active.cleanup_unknown = True  # Existing pending record remains a rejection fence.
            else:
                raise StartFailure(interrupted=not isinstance(active, Exception) and active is not None,
                                   outcome_unknown=start_uncertain, cleanup_unknown=True) from None


def release_start_ready(context, database, docker, sim_docker):
    """Check an explicit operator start; this does not prove a past drain."""
    require(context.required and context.source_release == context.target_release and
            context.source_commit == context.target_commit, "SIMULATION_OPERATION_RELEASE")
    _api_image_capability(docker, context.target_commit, context.source_images["api-server"], 15)
    if context.binding is None:
        _worker_absent(sim_docker)
    else:
        worker = runtime.capture(sim_docker, context.binding)
        require(not worker.running and (worker.generation is None or worker.restart_policy == "no"),
                "SIMULATION_OPERATION_GENERATION")
        verify_drained(sim_docker, context.binding, worker.generation)
    ids = _api_ids(docker, 15)
    require(len(ids) <= 1, "SIMULATION_OPERATION_API")
    if ids:
        generation, running, restart = _api_observe(docker, context.project, context.source_commit,
                                                     context.source_images["api-server"], ids[0], 15)
        require(not running and restart == "no", "SIMULATION_OPERATION_API")
        again, running, restart = _api_observe(docker, context.project, context.source_commit,
                                                context.source_images["api-server"], ids[0], 15)
        require(again == generation and not running and restart == "no", "SIMULATION_OPERATION_API")
    require(_api_ids(docker, 15) == ids, "SIMULATION_OPERATION_API")
    # The inventory already proves no simulation history. This permits initial
    # migrations, before the simulation settlement tables exist.
    if context.history:
        require(database.execute(context.values, SETTLEMENT_SQL, {"SIMULATION_MAINTENANCE: SETTLED"}) ==
                "SIMULATION_MAINTENANCE: SETTLED", "SIMULATION_OPERATION_DATABASE")
    if context.binding is None:
        _worker_absent(sim_docker)
    else:
        verify_drained(sim_docker, context.binding, worker.generation)


def _switch_current(root, target_commit, expected):
    with daily.install.root_directory(root) as parent:
        require(_current(root) == expected, "SIMULATION_OPERATION_RELEASE")
        temporary = ".current.simulation." + target_commit + "." + str(os.getpid())
        os.symlink("releases/" + target_commit, temporary, dir_fd=parent)
        try:
            os.replace(temporary, "current", src_dir_fd=parent, dst_dir_fd=parent)
            os.fsync(parent)
        except BaseException:
            try:
                os.unlink(temporary, dir_fd=parent)
            except FileNotFoundError:
                pass
            raise


def _handoff_target_running(context, docker, sim_docker, record):
    ids = _api_ids(docker, 15)
    require(len(ids) == 1, "SIMULATION_OPERATION_API")
    generation, running, _ = _api_observe(docker, context.project, context.target_commit,
                                           record["target_api_image"], ids[0], 15)
    require(running and (context.source_commit == context.target_commit or
                         generation.identity != record["api_generation"][0]), "SIMULATION_OPERATION_API")
    if record["target_api_generation"] is not None:
        require(generation == ApiGeneration(*record["target_api_generation"]), "SIMULATION_OPERATION_GENERATION")
    target_values, _ = daily.read_context(context.target_release, context.target_commit, "disable")
    api_value = docker.object("container", ids[0], 15)
    env = daily.runtime.environment(api_value)
    require(api_value.get("Config", {}).get("Labels", {}).get("org.lagrange.api-release-attempt") ==
            record["api_attempt"] and api_value.get("State", {}).get("Health", {}).get("Status") == "healthy" and
            env.get("LAGRANGE_CODE_COMMIT") == context.target_commit and
            env.get("OWNER_BETA_ACCESS_MODE") == (target_values.get("OWNER_BETA_ACCESS_MODE") or "disabled") and
            env.get("OWNER_BETA_PRICE_INPUT_MODE") == (target_values.get("OWNER_BETA_PRICE_INPUT_MODE") or "disabled"),
            "SIMULATION_OPERATION_API")
    if record["worker_generation"] is None:
        _worker_absent(sim_docker)
    else:
        require(context.binding is not None, "SIMULATION_OPERATION_HANDOFF")
        verify_drained(sim_docker, context.binding, tuple(record["worker_generation"]))
    again, running, _ = _api_observe(docker, context.project, context.target_commit,
                                      record["target_api_image"], ids[0], 15)
    require(again == generation and running and _api_ids(docker, 15) == ids, "SIMULATION_OPERATION_API")
    return generation


def reconcile_start(root, target, commit, docker, sim_docker):
    """Explicitly fence and resolve only a recorded exact worker generation."""
    pending = _pending_read(root, START_PENDING)
    keys = {"schema", "project", "commit", "image", "config", "receipt", "api_image", "api_generation",
            "attempt", "phase", "generation",
            "primary_failure", "interrupted", "start_uncertain", "cleanup_uncertain"}
    require(type(pending) is dict and set(pending) == keys and
            pending["schema"] == "owner-beta-simulation-start-pending-v1" and pending["commit"] == commit and
            pending["phase"] in ("creating", "owned", "start-requested", "failed") and
            (pending["generation"] is None or
             (type(pending["generation"]) is list and len(pending["generation"]) == 4)) and
            all(type(pending[k]) is bool for k in
                ("primary_failure", "interrupted", "start_uncertain", "cleanup_uncertain")) and
            daily.install.is_hash(pending["api_image"]) and type(pending["api_generation"]) is list and
            len(pending["api_generation"]) == 4 and
            type(pending["api_generation"][0]) is str and re.fullmatch(r"[0-9a-f]{64}", pending["api_generation"][0]) and
            all(type(pending["api_generation"][i]) is str and pending["api_generation"][i] for i in (1, 2)) and
            type(pending["api_generation"][3]) is int and pending["api_generation"][3] >= 0 and
            type(pending["attempt"]) is str and re.fullmatch(r"[0-9a-f]{32}", pending["attempt"]),
            "SIMULATION_OPERATION_PENDING")
    require(_current(root) == (target, commit) and _configured(target, commit), "SIMULATION_OPERATION_RELEASE")
    images = daily.read_manifest(target, commit)
    project, _ = daily.project_binding(commit, commit)
    values, _ = daily.read_context(target, commit, "disable")
    binding = binding_from_release(project, target, commit)
    require(project == pending["project"] and binding.image_id == pending["image"] and
            binding.config_sha256 == pending["config"] and binding.receipt_sha256 == pending["receipt"] and
            images["api-server"] == pending["api_image"],
            "SIMULATION_OPERATION_PENDING")
    release.reader_guard(root, target, commit, "disabled", history_present=True, reserve=False)
    previous = _handoff_read(root)
    require(previous is None or (previous["phase"] in ("api-started", "worker-replacing") and
            previous["target_commit"] == commit), "SIMULATION_OPERATION_PENDING")
    if previous is not None:
        _handoff_context(root, target, commit, docker, sim_docker, previous, allow_start_pending=True)
        require(previous["target_api_generation"] == pending["api_generation"], "SIMULATION_OPERATION_HANDOFF")
        api_value = docker.object("container", pending["api_generation"][0], 15)
        require(api_value.get("Config", {}).get("Labels", {}).get("org.lagrange.api-release-attempt") ==
                previous["api_attempt"], "SIMULATION_OPERATION_HANDOFF")
    generation = pending["generation"]
    try:
        observed = runtime.capture(sim_docker, binding)
    except Exception:
        if generation is None and previous is not None:
            old_context, _, _ = _handoff_context(root, target, commit, docker, sim_docker, previous,
                                                  allow_start_pending=True)
            _handoff_target_running(old_context, docker, sim_docker, previous)
            restored = dict(previous, phase="api-started")
            _pending_write(root, HANDOFF_PENDING, restored, previous=previous)
            _pending_remove(root, START_PENDING, pending)
            return "RECONCILED_WORKER_STOPPED"
        raise
    if generation is None:
        require(not pending["start_uncertain"] and not observed.running, "SIMULATION_OPERATION_PENDING")
        if previous is not None and observed.generation == tuple(previous["worker_generation"]):
            old_context, _, _ = _handoff_context(root, target, commit, docker, sim_docker, previous,
                                                  allow_start_pending=True)
            _handoff_target_running(old_context, docker, sim_docker, previous)
            restored = dict(previous, phase="api-started")
            _pending_write(root, HANDOFF_PENDING, restored, previous=previous)
            _pending_remove(root, START_PENDING, pending)
            return "RECONCILED_WORKER_STOPPED"
        if observed.generation is None:
            require(previous is None, "SIMULATION_OPERATION_PENDING")
            _pending_remove(root, START_PENDING, pending)
            return "RECONCILED_WORKER_ABSENT"
        value = docker.object("container", observed.generation[0], 15)
        require(value.get("Config", {}).get("Labels", {}).get("org.lagrange.simulation-start-attempt") ==
                pending["attempt"] and observed.restart_policy == "no", "SIMULATION_OPERATION_GENERATION")
        checked = verify_runtime_policy(docker, sim_docker, binding, values["LAGRANGE_RUNTIME_SECRET_DIR"],
                                        values["POSTGRES_DB"], running=False)
        require(checked.generation == observed.generation, "SIMULATION_OPERATION_GENERATION")
        updated = dict(pending, phase="owned", generation=list(observed.generation))
        _pending_write(root, START_PENDING, updated, previous=pending)
        pending, generation = updated, updated["generation"]
    require(observed.generation is not None and observed.generation[0:2] == tuple(generation[0:2]) and
            observed.generation[3] == generation[3], "SIMULATION_OPERATION_GENERATION")
    api = fence_api(docker, project, commit, images["api-server"], binding.stop_timeout_seconds,
                    expected=ApiGeneration(*pending["api_generation"]))
    result = runtime.drain(sim_docker, binding, observed)
    verify_drained(sim_docker, binding, result.generation)
    verify_fence(docker, project, commit, images["api-server"], api, 15)
    database = daily.Database(docker, project, commit, images["db-migrate"])
    require(database.execute(values, SETTLEMENT_SQL, {"SIMULATION_MAINTENANCE: SETTLED"}) ==
            "SIMULATION_MAINTENANCE: SETTLED", "SIMULATION_OPERATION_DATABASE")
    verify_drained(sim_docker, binding, result.generation)
    verify_fence(docker, project, commit, images["api-server"], api, 15)
    context = MaintenanceContext(root, target, commit, target, commit, project, images, values,
                                 binding, True, True, None)
    handoff = _handoff_make(context, api, result)
    handoff["phase"] = "switched"
    _pending_write(root, HANDOFF_PENDING, handoff, previous=previous)
    _pending_remove(root, START_PENDING, pending)
    return "RECONCILED_OFF"


def transition(install_root, target_release, target_commit, lock_fd, *, action):
    """Hold the caller's release lock through fence, floor and atomic switch."""
    require(action in ("inspect", "drain", "start", "activate", "ready", "begin-release", "finish-release",
                       "reconcile-start", "reconcile-release"),
            "SIMULATION_OPERATION_ARGUMENT")
    daily.install.require_root()
    with daily.inherited_lock(install_root, lock_fd):
        docker, sim_docker = daily.runtime.Docker(), runtime.Docker()
        if action == "reconcile-start":
            return reconcile_start(install_root, target_release, target_commit, docker, sim_docker)
        record = _handoff_read(install_root)
        if record is not None:
            require(action in ("activate", "drain", "ready", "begin-release", "finish-release", "reconcile-release", "start"),
                    "SIMULATION_OPERATION_PENDING")
            context, database, target_binding = _handoff_context(install_root, target_release, target_commit,
                                                                  docker, sim_docker, record)
            current = _current(install_root)
            if action in ("activate", "drain") and record["phase"] == "fencing":
                record = _retry_fencing(context, database, docker, sim_docker, record)
            if action == "drain":
                require(record["phase"] == "prepared" and current == (target_release, target_commit),
                        "SIMULATION_OPERATION_PENDING")
                _handoff_stopped(context, database, docker, sim_docker, record)
                updated = dict(record, phase="switched")
                _pending_write(install_root, HANDOFF_PENDING, updated, previous=record)
                return "DRAINED_OFF"
            if action == "activate":
                require(record["phase"] == "prepared", "SIMULATION_OPERATION_PENDING")
                _handoff_stopped(context, database, docker, sim_docker, record)
                release.reader_guard(install_root, target_release, target_commit, "disabled",
                                     history_present=True, reserve=True)
                if current[1] == record["source_commit"]:
                    _switch_current(install_root, target_commit, current)
                else:
                    require(current[1] == target_commit, "SIMULATION_OPERATION_HANDOFF")
                updated = dict(record, phase="switched")
                _pending_write(install_root, HANDOFF_PENDING, updated, previous=record)
                return "ACTIVATED_OFF"
            require(current == (target_release, target_commit), "SIMULATION_OPERATION_HANDOFF")
            if action in ("ready", "begin-release"):
                require(record["phase"] == "switched", "SIMULATION_OPERATION_PENDING")
                _handoff_stopped(context, database, docker, sim_docker, record)
                if action == "begin-release":
                    updated = dict(record, phase="api-starting")
                    _pending_write(install_root, HANDOFF_PENDING, updated, previous=record)
                    return "RELEASE_START_BEGUN " + record["api_attempt"]
                return "RELEASE_START_READY"
            if action in ("finish-release", "reconcile-release"):
                require(record["phase"] == "api-starting", "SIMULATION_OPERATION_PENDING")
                if action == "reconcile-release":
                    try:
                        target_api = _handoff_target_running(context, docker, sim_docker, record)
                    except Exception:
                        # Retry is offered only when the exact original stopped
                        # generation and independent settlement still hold.
                        _handoff_stopped(context, database, docker, sim_docker, record)
                        updated = dict(record, phase="switched")
                        _pending_write(install_root, HANDOFF_PENDING, updated, previous=record)
                        return "RELEASE_RETRY_READY"
                else:
                    target_api = _handoff_target_running(context, docker, sim_docker, record)
                if record["worker_generation"] is None:
                    _pending_remove(install_root, HANDOFF_PENDING, record)
                else:
                    updated = dict(record, phase="api-started", target_api_generation=[target_api.identity,
                        target_api.created, target_api.started, target_api.restarts])
                    _pending_write(install_root, HANDOFF_PENDING, updated, previous=record)
                return "RELEASE_START_FINISHED" if action == "finish-release" else "RELEASE_START_OBSERVED"
            require(action == "start" and record["phase"] == "api-started" and target_binding is not None and
                    record["worker_generation"] is not None, "SIMULATION_OPERATION_PENDING")
            _handoff_target_running(context, docker, sim_docker, record)
            target_images = daily.read_manifest(target_release, target_commit)
            values, _ = daily.read_context(target_release, target_commit, "disable")
            target_context = dataclasses.replace(context, source_release=target_release, source_commit=target_commit,
                                                  source_images=target_images, values=values, binding=target_binding)
            target_database = daily.Database(docker, context.project, target_commit, target_images["db-migrate"])
            start_simulation(target_context, docker, sim_docker, target_database,
                             replacement=(context.binding, context.values, record["worker_generation"], record))
            return "STARTED"
        context, database = inspect_transition(install_root, target_release, target_commit, docker, sim_docker)
        if action == "inspect":
            return "MAINTENANCE_REQUIRED" if context.required else "LEGACY_OFF"
        if action == "start":
            start_simulation(context, docker, sim_docker, database)
            return "STARTED"
        if action == "ready":
            raise OperationError("SIMULATION_OPERATION_HANDOFF")
        require(action not in ("begin-release", "finish-release", "reconcile-release"), "SIMULATION_OPERATION_HANDOFF")
        if context.required:
            target_images = daily.read_manifest(target_release, target_commit)
            _api_image_capability(docker, target_commit, target_images["api-server"], 15)
            record, captured = _capture_fencing(context, docker, sim_docker)
            _pending_write(install_root, HANDOFF_PENDING, record)
            api, worker = prepare_transition(context, database, docker, sim_docker,
                                             expected_api=ApiGeneration(*record["api_generation"]),
                                             worker_snapshot=captured)
            require(api == ApiGeneration(*record["api_generation"]) and
                    (None if worker is None or worker.generation is None else list(worker.generation)) ==
                    record["worker_generation"], "SIMULATION_OPERATION_GENERATION")
            updated = dict(record, phase="prepared")
            _pending_write(install_root, HANDOFF_PENDING, updated, previous=record)
            record = updated
        else:
            api, worker = prepare_transition(context, database, docker, sim_docker)
        if action == "drain":
            if context.required:
                updated = dict(record, phase="switched")
                _pending_write(install_root, HANDOFF_PENDING, updated, previous=record)
            return "DRAINED_OFF" if context.required else "LEGACY_OFF"
        if context.required:
            verify_fence(docker, context.project, context.source_commit,
                         context.source_images["api-server"], api, 15)
            if context.binding is None:
                _worker_absent(sim_docker)
            else:
                observed = runtime.capture(sim_docker, context.binding)
                require(observed.generation == worker.generation and not observed.running and
                        (observed.generation is None or observed.restart_policy == "no"),
                        "SIMULATION_OPERATION_GENERATION")
        release.reader_guard(install_root, target_release, target_commit, "disabled",
                             history_present=context.required, reserve=True)
        if context.required:
            verify_fence(docker, context.project, context.source_commit,
                         context.source_images["api-server"], api, 15)
            if context.binding is None:
                _worker_absent(sim_docker)
            else:
                verify_drained(sim_docker, context.binding, worker.generation)
        else:
            require(legacy_admission_disabled(docker, context.project, context.source_commit,
                                              context.source_images["api-server"],
                                              no_current=context.legacy_api is None) == context.legacy_api,
                    "SIMULATION_OPERATION_GENERATION")
            _worker_absent(sim_docker)
        if context.required:
            _handoff_stopped(context, database, docker, sim_docker, record)
        previous = _current(install_root)
        require(previous is None or previous == (context.source_release, context.source_commit),
                "SIMULATION_OPERATION_RELEASE")
        _switch_current(install_root, target_commit, previous)
        if context.required:
            updated = dict(record, phase="switched")
            _pending_write(install_root, HANDOFF_PENDING, updated, previous=record)
        return "ACTIVATED_OFF" if context.required else "LEGACY_OFF_ACTIVATED"


def verify_drained(sim_docker, binding, generation):
    observed = runtime.capture(sim_docker, binding)
    require(observed.generation == generation and not observed.running and
            (observed.generation is None or observed.restart_policy == "no"),
            "SIMULATION_OPERATION_GENERATION")


def drain_for_maintenance(docker, sim_docker, database, values, binding, api_image, *, clock=time.monotonic,
                          expected_api=None, snapshot=None):
    """Fence API, drain captured generation, then independently audit the DB.

    The caller owns the release lock and must recheck the returned API fence
    until maintenance finishes. Failures never resume either service.
    """
    try:
        if snapshot is None:
            snapshot = runtime.capture(sim_docker, binding)
        api = fence_api(docker, binding.project, binding.commit, api_image,
                        binding.stop_timeout_seconds, clock=clock, expected=expected_api)
        result = runtime.drain(sim_docker, binding, snapshot, clock=clock)
        verify_drained(sim_docker, binding, result.generation)
        verify_fence(docker, binding.project, binding.commit, api_image, api, 15)
        require(database.execute(values, SETTLEMENT_SQL, {"SIMULATION_MAINTENANCE: SETTLED"}) ==
                "SIMULATION_MAINTENANCE: SETTLED", "SIMULATION_OPERATION_DATABASE")
        verify_drained(sim_docker, binding, result.generation)
        verify_fence(docker, binding.project, binding.commit, api_image, api, 15)
        return api, result
    except OperationError:
        raise
    except Exception:
        raise OperationError("SIMULATION_OPERATION_FAILED") from None


class _Arguments(argparse.ArgumentParser):
    def error(self, _message):
        self.exit(2, "SIMULATION_OPERATION_USAGE\n")


def verify_backup_tool(install_root, application_release, commit, tool_commit, manifest_hash):
    """Verify the selected tool with current, simulation-aware source code."""
    install = daily.install
    install.require_root()
    root, app = Path(install_root), Path(application_release)
    require(install.is_commit(commit) and install.is_commit(tool_commit) and
            install.is_hash(manifest_hash) and app == root / "releases" / commit,
            "SIMULATION_OPERATION_TOOL")
    tool = root / "maintenance" / tool_commit
    verifier = _local("simulation_backup_tool_verifier", "production-backup-tool.py")
    verifier.read_manifest(tool, commit, install_root=root, expected_hash=manifest_hash)
    # A member name alone does not prove that an older controller dispatches
    # simulation capture. Bind all Python controller dependencies to the
    # installed application's protected, simulation-aware code.
    libraries = sorted(name for name in verifier.REQUIRED
                       if name.startswith("scripts/ops/lib/") and name.endswith(".py"))
    require(libraries, "SIMULATION_OPERATION_TOOL")
    for name in libraries:
        app_bytes = install.read_root_file(str(app / name), gid=0, mode=0o644, maximum=verifier.MAX_FILE)
        tool_bytes = install.read_root_file(str(tool / name), gid=0, mode=0o644, maximum=verifier.MAX_FILE)
        require(hashlib.sha256(app_bytes).digest() == hashlib.sha256(tool_bytes).digest(),
                "SIMULATION_OPERATION_TOOL")


def main(argv=None):
    parser = _Arguments()
    commands = parser.add_subparsers(dest="command", required=True)
    stage = commands.add_parser("stage-receipt")
    stage.add_argument("--stage", required=True)
    stage.add_argument("--receipt-source", required=True)
    stage.add_argument("--complete-source", required=True)
    stage.add_argument("--commit", required=True)
    verify = commands.add_parser("verify-receipt")
    verify.add_argument("--release", required=True)
    verify.add_argument("--commit", required=True)
    maintenance = commands.add_parser("transition")
    maintenance.add_argument("--install-root", required=True)
    maintenance.add_argument("--target-release", required=True)
    maintenance.add_argument("--commit", required=True)
    maintenance.add_argument("--lock-fd", required=True)
    maintenance.add_argument("--action", choices=("inspect", "drain", "start", "activate", "ready",
                                                 "begin-release", "finish-release", "reconcile-start",
                                                 "reconcile-release"), required=True)
    tool = commands.add_parser("verify-backup-tool")
    tool.add_argument("--install-root", required=True)
    tool.add_argument("--application-release", required=True)
    tool.add_argument("--commit", required=True)
    tool.add_argument("--tool-commit", required=True)
    tool.add_argument("--manifest-hash", required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "stage-receipt":
            stage_receipt(args.stage, args.receipt_source, args.complete_source, args.commit)
        elif args.command == "verify-receipt":
            verify_receipt(args.release, args.commit)
        elif args.command == "verify-backup-tool":
            verify_backup_tool(args.install_root, args.application_release, args.commit,
                               args.tool_commit, args.manifest_hash)
        else:
            result = transition(args.install_root, args.target_release, args.commit, args.lock_fd,
                                action=args.action)
            print("SIMULATION_OPERATION: " + result)
            return 0
        print("SIMULATION_OPERATION_OK")
        return 0
    except StartFailure as failure:
        print("SIMULATION_OPERATION_START_FAILED", file=sys.stderr)
        if failure.interrupted:
            print("SIMULATION_OPERATION_INTERRUPTED", file=sys.stderr)
        if failure.outcome_unknown:
            print("SIMULATION_OPERATION_START_OUTCOME_UNKNOWN", file=sys.stderr)
        if failure.cleanup_unknown:
            print("SIMULATION_OPERATION_CLEANUP_UNCONFIRMED", file=sys.stderr)
        return 1
    except Exception:
        print("SIMULATION_OPERATION_REJECTED", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
