#!/usr/bin/env python3
"""Validate local D-06 one-shot wiring and retain the independent attempt witness."""

import json
import os
from pathlib import Path
import re
import stat
import sys

PROJECT = "lagrange-station"
SERVICE = "research-action-range-raw"
TARGET = "2026-11-15"
RAW = "/var/lib/lagrange/data/raw"
NETWORK = "range-raw-egress"
MARKER = "r2-d06-calendar-20261115.claim"


def require(condition):
    if not condition:
        raise ValueError


def inspect_image(document, revision):
    [image] = document
    identity = image["Id"]
    config = image["Config"]
    require(re.fullmatch(r"sha256:[0-9a-f]{64}", identity))
    require(config["Labels"]["org.opencontainers.image.revision"] == revision)
    commits = [value for value in config["Env"] if value.startswith("LAGRANGE_CODE_COMMIT=")]
    require(commits == [f"LAGRANGE_CODE_COMMIT={revision}"])
    require(config["User"] == "10001:10001" and not config.get("Volumes"))
    return identity


def verify_compose(document, identity, revision, state_root, secret_root, generation, quote_mode):
    require(document["name"] == PROJECT)
    require(set(document["services"]) == {SERVICE})
    service = document["services"][SERVICE]
    require(set(service) <= {
        "build", "cap_drop", "command", "cpus", "entrypoint", "environment", "healthcheck", "image",
        "init", "logging", "mem_limit", "networks", "profiles", "read_only",
        "secrets", "security_opt", "tmpfs", "user", "volumes",
    })
    require(service["image"] == identity and service.get("command") is None)
    require(service["user"] == "10001:10001" and service["read_only"] is True)
    require(service["entrypoint"] == ["/usr/local/bin/kis-action-range-raw"])
    require(service["profiles"] == ["action-range-raw"])
    require(service["healthcheck"] == {"disable": True})
    require(service["cap_drop"] == ["ALL"])
    require(service["security_opt"] == ["no-new-privileges:true"])
    require(set(service["networks"]) == {NETWORK})
    require(service["networks"][NETWORK] in (None, {}))
    network = document["networks"][NETWORK]
    require(network["name"] == f"{PROJECT}_{NETWORK}" and network["driver"] == "bridge")
    require(not network.get("internal") and not network.get("external"))
    require(not network.get("driver_opts"))
    require(service["environment"] == {
        "LAGRANGE_CODE_COMMIT": revision,
        "RESEARCH_RAW_ROOT": "/data",
        "RESEARCH_ENTITLEMENT_REFERENCE": os.environ["RESEARCH_ENTITLEMENT_REFERENCE"],
        "KIS_APP_KEY_FILE": "/run/secrets/kis_app_key",
        "KIS_APP_SECRET_FILE": "/run/secrets/kis_app_secret",
        "KIS_READ_COORDINATION_MODE": "shared_required",
        "KIS_READ_CREDENTIAL_GENERATION": generation,
        "OWNER_INTRADAY_QUOTES_MODE": quote_mode,
    })
    volumes = service["volumes"]
    require(len(volumes) == 2)
    expected = {"/data/raw": RAW, "/run/lagrange/kis-read-coordination":
                f"{state_root}/kis-read-coordination"}
    for volume in volumes:
        require(volume["type"] == "bind" and not volume.get("read_only"))
        require(volume["source"] == expected.pop(volume["target"]))
        require(set(volume) <= {"type", "source", "target", "read_only", "bind"})
        binding = volume["bind"]
        require(set(binding) <= {"create_host_path", "propagation"})
        require(binding["create_host_path"] is False)
        require(binding.get("propagation", "rprivate") in {"", "rprivate"})
    require(not expected)
    expected_secrets = {
        f"research_range_raw_{name}": name for name in ("kis_app_key", "kis_app_secret")
    }
    require(len(service["secrets"]) == 2)
    for value in service["secrets"]:
        name = expected_secrets.pop(value["source"])
        require(value["target"] == name)
        require(str(value["uid"]) == str(value["gid"]) == "10001")
        mode = value["mode"]
        require((int(mode, 8) if isinstance(mode, str) else mode) == 0o440)
        require(document["secrets"][value["source"]]["file"] ==
                f"{secret_root}/research-range-raw/{name}")
    require(not expected_secrets)


def verify_network(document):
    [network] = document
    require(network["Name"] == f"{PROJECT}_{NETWORK}")
    require(network["Driver"] == "bridge" and network["Scope"] == "local")
    require(network["Internal"] is False and not network["Ingress"])
    require(not network["Options"])
    labels = network["Labels"]
    require(labels["com.docker.compose.project"] == PROJECT)
    require(labels["com.docker.compose.network"] == NETWORK)


def open_state(state_root):
    path = Path(state_root)
    require(path.is_absolute() and path.resolve(strict=True) == path)
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    info = os.fstat(fd)
    if (info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode)) != (0, 10001, 0o750):
        os.close(fd)
        raise ValueError
    return fd


def witness(state_root, revision, identity, reserve):
    directory = open_state(state_root)
    try:
        if not reserve:
            try:
                info = os.stat(MARKER, dir_fd=directory, follow_symlinks=False)
            except FileNotFoundError:
                return "unused"
            require(stat.S_ISREG(info.st_mode) and info.st_uid == 0 and info.st_nlink == 1)
            require(stat.S_IMODE(info.st_mode) == 0o600)
            return "consumed"
        flags = os.O_WRONLY | os.O_NOFOLLOW | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC
        with os.fdopen(os.open(MARKER, flags, 0o600, dir_fd=directory), "w") as claimed:
            json.dump({"target": TARGET, "revision": revision, "image_id": identity}, claimed)
            claimed.write("\n")
            claimed.flush()
            os.fsync(claimed.fileno())
        os.fsync(directory)
        return "consumed"
    finally:
        os.close(directory)


def check_report(report):
    require(report["target_date"] == TARGET and report["raw_only"] is True)
    require(report["outcome"] in {
        "ALREADY_COVERED", "VERIFIED_PRIOR_CAPTURE", "WOULD_CAPTURE_NO_RESERVATION"
    })
    return report["outcome"]


def main(argv):
    mode, *args = argv
    if mode == "image":
        print(inspect_image(json.load(sys.stdin), *args))
    elif mode == "compose":
        verify_compose(json.load(sys.stdin), *args)
    elif mode == "network":
        verify_network(json.load(sys.stdin))
    elif mode == "check-report":
        print(check_report(json.load(sys.stdin)))
    elif mode in {"inspect-witness", "consume-witness"}:
        print(witness(*args, mode == "consume-witness"))
    else:
        raise ValueError


if __name__ == "__main__":
    try:
        main(sys.argv[1:])
    except (KeyError, ValueError, TypeError, OSError, AttributeError):
        sys.exit("CALENDAR_REPAIR_BOUNDARY")
