#!/usr/bin/env python3
"""Internal Docker observation/drain primitives for the controlled daily release.

The caller supplies an already verified installed manifest/config binding.
Only backup recovery may restart the exact prior healthy daily containers.
This module never creates containers, changes DB state or contacts a provider.
Inspect bodies stay in memory; exceptions contain fixed diagnostic codes only.
"""
from __future__ import annotations

import dataclasses
import json
import os
import re
import subprocess
import sys
import time

SERVICES = ("owner-beta-daily-producer", "owner-beta-daily-admitter", "owner-beta-runner")
BINARY = "/usr/local/bin/owner-beta-runner"
PROJECT = re.compile(r"[a-z0-9][a-z0-9_-]{0,62}")
COMMIT = re.compile(r"[0-9a-f]{40}")
DIGEST = re.compile(r"sha256:[0-9a-f]{64}")
CONTAINER = re.compile(r"[0-9a-f]{64}")


class RuntimeErrorCode(Exception):
    """Fixed code only; never attach Docker output, environment or input."""


def require(condition, code="DAILY_RUNTIME_INVALID"):
    if not condition:
        raise RuntimeErrorCode(code)


def absolute(path):
    return (isinstance(path, str) and path.startswith("/") and path != "/"
            and all(part not in ("", ".", "..") for part in path[1:].split("/"))
            and not any(ord(c) < 32 or c in "\x7f" for c in path))


@dataclasses.dataclass(frozen=True)
class Binding:
    project: str
    commit: str
    image_id: str
    config_sha256: str
    release: str
    data_root: str
    artifact_root: str
    secret_root: str
    database: str
    capacity: str
    lease: str
    backoff: str

    def __post_init__(self):
        require(isinstance(self.project, str) and PROJECT.fullmatch(self.project))
        require(isinstance(self.commit, str) and COMMIT.fullmatch(self.commit)
                and self.commit != "0" * 40)
        for value in (self.image_id, self.config_sha256):
            require(isinstance(value, str) and DIGEST.fullmatch(value))
        require(all(absolute(v) for v in
                    (self.release, self.data_root, self.artifact_root, self.secret_root)))
        require(isinstance(self.database, str) and
                re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,62}", self.database))
        for value, maximum in ((self.capacity, 2**31 - 1), (self.lease, 2**31 - 1),
                               (self.backoff, 2**63 - 1)):
            require(isinstance(value, str) and re.fullmatch(r"[1-9][0-9]{0,18}", value)
                    and int(value) <= maximum)

    def mounts(self, service):
        require(service in SERVICES)
        shared = self.data_root + "/owner-beta-daily/"
        result = {
            "/run/owner-beta-daily/config.json":
                (self.release + "/.owner-beta-daily/config.json", False),
            "/run/owner-beta-daily/policy.json":
                (self.release + "/.owner-beta-daily/policy.json", False),
            "/data/owner-beta-artifacts":
                (self.artifact_root + "/historical-price-beta-root", False),
            "/data/owner-beta-daily/admitted": (shared + "admitted", False),
        }
        if service != "owner-beta-runner":
            role = "producer" if service == SERVICES[0] else "admitter"
            result["/data/owner-beta-artifacts"] = (shared + "historical-base/" + role, False)
            result.update({
                "/data/raw": (self.data_root + "/raw", False),
                "/data/owner-beta-daily/candidates":
                    (shared + "candidates", service == SERVICES[0]),
                "/data/owner-beta-daily/frontiers":
                    (shared + "frontiers", service == SERVICES[1]),
            })
            stage = "producer-state" if service == SERVICES[0] else "admitter-state"
            result["/data/owner-beta-daily/" + stage] = (shared + stage, True)
        if service == SERVICES[1]:
            result["/data/owner-beta-daily/admitted"] = (shared + "admitted", True)
            result["/run/secrets/db_daily_input_admitter_password"] = (
                self.secret_root + "/owner-beta-daily-admitter/db_daily_input_admitter_password",
                False)
        elif service == SERVICES[2]:
            result["/run/secrets/db_worker_password"] = (
                self.secret_root + "/owner-beta-runner/db_worker_password", False)
        return result


class Docker:
    """Always the local daemon; ambient Docker host/context cannot redirect it."""
    def __init__(self):
        require(sys.platform == "linux" and os.geteuid() == 0, "DAILY_RUNTIME_ROOT")
        self.environment = {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "HOME": "/root",
                            "LANG": "C.UTF-8"}

    def call(self, args, timeout=15):
        try:
            result = subprocess.run(
                ["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", *args],
                env=self.environment, stdin=subprocess.DEVNULL, capture_output=True,
                timeout=max(0.01, timeout), check=False)
            require(result.returncode == 0 and len(result.stdout) <= 2 * 1024 * 1024,
                    "DAILY_RUNTIME_DOCKER")
            return result.stdout.decode("utf-8", errors="strict")
        except (OSError, UnicodeError, subprocess.TimeoutExpired):
            raise RuntimeErrorCode("DAILY_RUNTIME_DOCKER") from None

    def object(self, kind, identity, timeout=15):
        require(kind in ("container", "image", "network"))
        try:
            values = json.loads(self.call([kind, "inspect", identity], timeout))
            require(isinstance(values, list) and len(values) == 1
                    and isinstance(values[0], dict), "DAILY_RUNTIME_INSPECT")
            return values[0]
        except (ValueError, TypeError):
            raise RuntimeErrorCode("DAILY_RUNTIME_INSPECT") from None

    def service_ids(self, project, service, timeout=15):
        require(PROJECT.fullmatch(project) and service in SERVICES)
        lines = self.call(["ps", "--all", "--no-trunc", "--filter",
                          "label=com.docker.compose.project=" + project, "--filter",
                          "label=com.docker.compose.service=" + service,
                          "--format", "{{.ID}}"], timeout).splitlines()
        require(len(lines) <= 1 and all(CONTAINER.fullmatch(v) for v in lines),
                "DAILY_RUNTIME_AMBIGUOUS")
        return lines


def image_owned(docker, binding):
    value = docker.object("image", binding.image_id)
    config = value.get("Config") or {}
    labels = config.get("Labels") or {}
    require(value.get("Id") == binding.image_id and
            labels.get("org.opencontainers.image.revision") == binding.commit,
            "DAILY_RUNTIME_IMAGE")


def owned(value, binding, service, identity):
    labels = value.get("Config", {}).get("Labels") or {}
    require(CONTAINER.fullmatch(identity) and value.get("Id") == identity and
            value.get("Image") == binding.image_id and
            labels.get("org.opencontainers.image.revision") == binding.commit and
            labels.get("com.docker.compose.project") == binding.project and
            labels.get("com.docker.compose.service") == service and
            labels.get("com.docker.compose.oneoff") == "False", "DAILY_RUNTIME_OWNER")
    state = value.get("State", {})
    require(isinstance(value.get("Created"), str) and value["Created"] and
            isinstance(state.get("StartedAt"), str) and state["StartedAt"] and
            type(value.get("RestartCount")) is int and value["RestartCount"] >= 0,
            "DAILY_RUNTIME_GENERATION")
    return (identity, value["Created"], state["StartedAt"], value["RestartCount"])


def capture(docker, binding):
    image_owned(docker, binding)
    result = {}
    for service in SERVICES:
        ids = docker.service_ids(binding.project, service)
        if ids:
            value = docker.object("container", ids[0])
            result[service] = {"generation": owned(value, binding, service, ids[0]),
                               "value": value}
    return result


def environment(value):
    result = {}
    for row in value.get("Config", {}).get("Env") or []:
        require(isinstance(row, str) and "=" in row, "DAILY_RUNTIME_ENV")
        key, val = row.split("=", 1)
        require(key not in result, "DAILY_RUNTIME_ENV")
        result[key] = val
    return result


def verify_policy(docker, binding, service, value, *, stopped_for_backup=False):
    identity = value.get("Id", "")
    owned(value, binding, service, identity)
    config, host, state = value["Config"], value["HostConfig"], value["State"]
    index = SERVICES.index(service)
    uid = (10002, 10003, 10001)[index]
    command = [["daily-producer"], ["daily-admitter"],
               ["--artifact-root", "/data/owner-beta-artifacts"]][index]
    require(config.get("User") == f"{uid}:{uid}" and
            config.get("Entrypoint") == [BINARY] and config.get("Cmd") == command,
            "DAILY_RUNTIME_COMMAND")
    if stopped_for_backup:
        require(state.get("Running") is False and state.get("Status") == "exited" and
                state.get("ExitCode") == 0 and not state.get("Error") and
                not state.get("Restarting") and not state.get("Paused") and
                not state.get("OOMKilled"), "DAILY_RUNTIME_STATE")
    else:
        require(state.get("Running") is True and not state.get("Restarting") and
                not state.get("Paused") and not state.get("OOMKilled") and
                state.get("Health", {}).get("Status") == "healthy", "DAILY_RUNTIME_STATE")
    require(host.get("ReadonlyRootfs") is True and host.get("Init") is True and
            host.get("Privileged") is False and host.get("CapDrop") == ["ALL"] and
            not host.get("CapAdd") and
            host.get("SecurityOpt") == ["no-new-privileges:true"] and
            host.get("GroupAdd", []) in ((["10001"] if index < 2 else []), None) and
            (index == 2 or host.get("GroupAdd") == ["10001"]),
            "DAILY_RUNTIME_PRIVILEGES")
    require(not any(host.get(k) for k in
                    ("Devices", "DeviceRequests", "PortBindings", "VolumesFrom",
                     "PidMode", "UTSMode", "ExtraHosts", "Links", "LxcConf")) and
            host.get("IpcMode") == "private" and not config.get("ExposedPorts"),
            "DAILY_RUNTIME_PRIVILEGES")
    require(host.get("Memory") == 1024**3 and host.get("NanoCpus") == 10**9 and
            host.get("RestartPolicy", {}).get("Name") ==
            ("no" if stopped_for_backup else "unless-stopped"),
            "DAILY_RUNTIME_LIMITS")
    require(host.get("Tmpfs") == {
        "/run/owner-beta-health": f"uid={uid},gid={uid},mode=0750"},
        "DAILY_RUNTIME_TMPFS")
    log = host.get("LogConfig", {})
    require(log.get("Type") == "json-file" and
            log.get("Config") == {"max-size": "10m", "max-file": "3", "tag": "{{.Name}}"},
            "DAILY_RUNTIME_LOG")
    require(config.get("Healthcheck", {}).get("Test") == ["CMD", BINARY, "healthcheck"],
            "DAILY_RUNTIME_HEALTHCHECK")
    expected = binding.mounts(service)
    actual = {}
    for mount in value.get("Mounts", []):
        target = mount.get("Destination")
        if mount.get("Type") == "tmpfs" and target == "/run/owner-beta-health":
            continue
        require(target not in actual and mount.get("Type") == "bind" and
                mount.get("Propagation") == "rprivate", "DAILY_RUNTIME_MOUNT")
        actual[target] = (mount.get("Source"), mount.get("RW"))
    require(actual == expected, "DAILY_RUNTIME_MOUNT")
    env = environment(value)
    required = {
        "APP_ENV": "production", "LAGRANGE_CODE_COMMIT": binding.commit,
        "PRICE_BETA_DAILY_MODE": "owner_only",
        "PRICE_BETA_DAILY_CONFIG_FILE": "/run/owner-beta-daily/config.json",
        "PRICE_BETA_DAILY_CONFIG_SHA256": binding.config_sha256,
        "PRICE_BETA_HEALTH_STATE_PATH": "/run/owner-beta-health/health.json",
        "PRICE_BETA_HEALTH_MAX_AGE_SECS": "180",
    }
    if index > 0:
        required.update(DB_HOST="postgres", DB_PORT="5432", DB_NAME=binding.database,
                        DB_USER="daily_input_admitter" if index == 1 else "worker",
                        DB_PASSWORD_FILE="/run/secrets/db_daily_input_admitter_password"
                        if index == 1 else "/run/secrets/db_worker_password")
    if index == 2:
        required.update(PRICE_BETA_POLL_SECS="2", PRICE_BETA_RECOVERY_SECS="30",
                        PRICE_BETA_HEARTBEAT_SECS="10", PRICE_BETA_LEASE_SECS=binding.lease,
                        PRICE_BETA_BACKOFF_SECS=binding.backoff,
                        PRICE_BETA_COMPUTE_TIMEOUT_SECS="300")
    require(all(env.get(k) == v for k, v in required.items()) and
            set(env) <= set(required) | {"PATH", "LANG"}, "DAILY_RUNTIME_ENV")
    networks = value.get("NetworkSettings", {}).get("Networks", {})
    if index == 0:
        require(host.get("NetworkMode") == "none" and set(networks) <= {"none"},
                "DAILY_RUNTIME_NETWORK")
    else:
        require(len(networks) == 1, "DAILY_RUNTIME_NETWORK")
        name = next(iter(networks))
        require(host.get("NetworkMode") == name, "DAILY_RUNTIME_NETWORK")
        network = docker.object("network", name)
        labels = network.get("Labels") or {}
        require(network.get("Internal") is True and
                labels.get("com.docker.compose.project") == binding.project and
                labels.get("com.docker.compose.network") == "owner-beta-db" and
                networks[name].get("NetworkID") == network.get("Id"),
                "DAILY_RUNTIME_NETWORK")


def verify_running(docker, binding):
    snapshot = capture(docker, binding)
    require(set(snapshot) == set(SERVICES), "DAILY_RUNTIME_MISSING")
    for service, entry in snapshot.items():
        verify_policy(docker, binding, service, entry["value"])
    return snapshot


def unchanged(docker, binding, service, entry, timeout):
    identity = entry["generation"][0]
    value = docker.object("container", identity, timeout)
    require(owned(value, binding, service, identity) == entry["generation"],
            "DAILY_RUNTIME_GENERATION")
    return value


def resume_after_backup(docker, binding, before, timeout=180):
    """Resume the exact previously healthy cohort; the DB remains disabled.

    Caller holds the release lock, has finished verification, and releases
    snapshot/private-state locks before entering. No API/market-stream/Compose
    mutation or new container is allowed. On failure the caller must disable
    and drain its current owned cohort; restart policy is never a success proof.
    """
    require(set(before) == set(SERVICES) and 0 < timeout <= 180,
            "DAILY_BACKUP_RESUME_COHORT")
    deadline = time.monotonic() + timeout

    def remaining():
        left = deadline - time.monotonic()
        require(left > 0, "DAILY_BACKUP_RESUME_DEADLINE")
        return min(15, left)

    image_owned(docker, binding)
    # Validate all originals before the first restart-policy mutation. A
    # partial, unhealthy or originally OFF installation must remain disabled.
    for service in SERVICES:
        entry = before[service]
        verify_policy(docker, binding, service, entry["value"])
        require(docker.service_ids(binding.project, service, remaining()) ==
                [entry["generation"][0]], "DAILY_RUNTIME_GENERATION")
        current = unchanged(docker, binding, service, entry, remaining())
        verify_policy(docker, binding, service, current, stopped_for_backup=True)
    for service in SERVICES:
        identity = before[service]["generation"][0]
        docker.call(["update", "--restart=unless-stopped", identity], remaining())
        current = unchanged(docker, binding, service, before[service], remaining())
        require(current["State"].get("Running") is False and
                current["HostConfig"].get("RestartPolicy", {}).get("Name") == "unless-stopped",
                "DAILY_BACKUP_RESUME_STATE")
    for service in SERVICES:
        docker.call(["start", before[service]["generation"][0]], remaining())
    started = {}
    while True:
        healthy = True
        for service in SERVICES:
            previous = before[service]["generation"]
            require(docker.service_ids(binding.project, service, remaining()) == [previous[0]],
                    "DAILY_RUNTIME_GENERATION")
            value = docker.object("container", previous[0], remaining())
            generation = owned(value, binding, service, previous[0])
            require(generation[:2] == previous[:2] and generation[3] == previous[3] and
                    generation[2] != previous[2] and
                    (service not in started or generation == started[service]),
                    "DAILY_RUNTIME_GENERATION")
            started[service] = generation
            state = value["State"]
            require(state.get("Running") is True and not state.get("Restarting") and
                    not state.get("Paused") and not state.get("OOMKilled") and
                    not state.get("Error"), "DAILY_BACKUP_RESUME_STATE")
            status = state.get("Health", {}).get("Status")
            require(status in {"starting", "healthy"}, "DAILY_BACKUP_RESUME_HEALTH")
            healthy = healthy and status == "healthy"
        if healthy:
            result = verify_running(docker, binding)
            require(all(result[s]["generation"] == started[s] for s in SERVICES),
                    "DAILY_RUNTIME_GENERATION")
            return result
        time.sleep(min(0.25, remaining()))


def drain(docker, binding, snapshot, timeout=3670):
    """Drain only a fully captured cohort, even if the caller's DB disable failed.

    Never reset restart policies on failure, never send SIGKILL, and never treat
    an unhealthy/foreign/replaced container as a successful graceful stop.
    """
    require(0 < timeout <= 3670, "DAILY_RUNTIME_DEADLINE")
    deadline = time.monotonic() + timeout
    def remaining():
        value = deadline - time.monotonic()
        require(value > 0, "DAILY_RUNTIME_DEADLINE")
        return value
    def control_timeout():
        return min(15, remaining())
    require(set(snapshot) <= set(SERVICES), "DAILY_RUNTIME_OWNER")
    initial = {}
    # Verify the complete cohort before the first mutation.
    for service in SERVICES:
        ids = docker.service_ids(binding.project, service, control_timeout())
        expected = [snapshot[service]["generation"][0]] if service in snapshot else []
        require(ids == expected, "DAILY_RUNTIME_GENERATION")
        if expected:
            value = unchanged(docker, binding, service, snapshot[service], control_timeout())
            require(not value["State"].get("Paused"), "DAILY_RUNTIME_PAUSED")
            initial[service] = value
    failed = False
    updated = {}
    for service, entry in snapshot.items():
        try:
            docker.call(["update", "--restart=no", entry["generation"][0]], control_timeout())
            updated[service] = entry
        except RuntimeErrorCode:
            failed = True
    waiting = []
    for service, entry in updated.items():
        try:
            value = unchanged(docker, binding, service, entry, control_timeout())
            require(value["HostConfig"].get("RestartPolicy", {}).get("Name") == "no",
                    "DAILY_RUNTIME_STATE")
            if value["State"].get("Running"):
                docker.call(["kill", "--signal=TERM", entry["generation"][0]], control_timeout())
                waiting.append(entry["generation"][0])
            else:
                require(not value["State"].get("Restarting"), "DAILY_RUNTIME_STATE")
        except RuntimeErrorCode:
            # Still attempt the other demonstrably owned members. No failed
            # restart update or changed generation is ever signalled.
            failed = True
    if waiting:
        try:
            rows = docker.call(["wait", *waiting], remaining()).splitlines()
            require(len(rows) == len(waiting) and all(row == "0" for row in rows),
                    "DAILY_RUNTIME_EXIT")
        except RuntimeErrorCode:
            failed = True
    # Re-list every service, including those absent at capture, before acceptance.
    for service in SERVICES:
        ids = docker.service_ids(binding.project, service, control_timeout())
        expected = [snapshot[service]["generation"][0]] if service in snapshot else []
        require(ids == expected, "DAILY_RUNTIME_GENERATION")
        if expected:
            value = unchanged(docker, binding, service, snapshot[service], control_timeout())
            state = value["State"]
            require(not state.get("Running") and not state.get("Restarting") and
                    not state.get("Paused") and not state.get("OOMKilled") and
                    not state.get("Error") and state.get("ExitCode") == 0 and
                    state.get("Status") in ("created", "exited") and
                    value["HostConfig"].get("RestartPolicy", {}).get("Name") == "no",
                    "DAILY_RUNTIME_EXIT")
    require(not failed, "DAILY_RUNTIME_INCOMPLETE")
    return {"stopped": len(snapshot), "signalled": len(waiting),
            "previously_running": sum(bool(v["State"].get("Running")) for v in initial.values())}
