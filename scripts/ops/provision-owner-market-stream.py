#!/usr/bin/python3 -I
"""Provision a new WS domain, or validate its existing immutable identity.

Plan mode reads no installed configuration. Explicit operations require the
current root-owned immutable release, its pinned local image, and a stopped
runner. Only the reviewed Rust initializer writes domain state. An attempted
initialization is recorded durably before launching it and is never retried.
"""

from __future__ import annotations

import argparse
import contextlib
import fcntl
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import uuid
from dataclasses import dataclass


STATE = "kis-market-stream"
ANCHORS = "kis-market-stream-locks"
RECORD = ".kis-market-stream-install-v1.json"
BIN = "/usr/local/bin/kis-market-stream-state"
LABEL = "io.lagrange.market-stream.provision-operation"
COMMIT = re.compile(r"[0-9a-f]{40}")
IMAGE = re.compile(r"sha256:[0-9a-f]{64}")
CONTAINER = re.compile(r"[0-9a-f]{64}")
ENV = {"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C",
       "HOME": "/nonexistent", "DOCKER_CONFIG": "/nonexistent"}
DOCKER = ["/usr/bin/docker", "--host", "unix:///var/run/docker.sock"]


class ProvisionError(Exception):
    """Only closed error codes cross the command boundary."""


def require(condition: bool, code: str) -> None:
    if not condition:
        raise ProvisionError(code)


def canonical_path(value: str) -> Path:
    path = Path(value)
    require(value.startswith("/") and value != "/" and str(path) == value
            and not any(x in value for x in ("\x00", "\n", "\r", ","))
            and ".." not in path.parts, "ERR_PATH")
    return path


def trusted_directory(path: Path) -> None:
    canonical_path(str(path))
    for part in reversed((path, *path.parents)):
        info = part.lstat()
        require(stat.S_ISDIR(info.st_mode) and info.st_uid == 0
                and info.st_mode & 0o022 == 0, "ERR_TRUST")


def trusted_file(path: Path, protected: bool = False) -> None:
    trusted_directory(path.parent)
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_uid == 0
            and info.st_nlink == 1 and info.st_mode & 0o022 == 0
            and 0 < info.st_size <= 1_048_576, "ERR_TRUST")
    if protected:
        require(info.st_gid == 0 and stat.S_IMODE(info.st_mode) == 0o600, "ERR_TRUST")


def command(argv: list[str], timeout: int = 10) -> subprocess.CompletedProcess:
    try:
        result = subprocess.run(argv, env=ENV, stdin=subprocess.DEVNULL,
                                capture_output=True, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired):
        raise ProvisionError("ERR_PROCESS") from None
    require(len(result.stdout) <= 1_048_576 and len(result.stderr) <= 1_048_576,
            "ERR_PROCESS")
    return result


def successful(argv: list[str], timeout: int = 10) -> bytes:
    result = command(argv, timeout)
    require(result.returncode == 0, "ERR_PROCESS")
    return result.stdout


@dataclass(frozen=True)
class Context:
    commit: str
    slot: str
    generation: str
    parent: Path
    image: str


# Reuse the installed non-evaluating dotenv parser and strict complete V2
# manifest parser. No shell environment value selects or overrides an input.
READ_CONTEXT = r'''
set -euo pipefail
source "$1/scripts/ops/lib/dotenv.sh"
source "$1/scripts/ops/lib/release-image-manifest.sh"
dotenv_load "$1/deploy/compose/.env"
release_image_manifest_load "$1/.lagrange-release-manifest" "$2"
for key in LAGRANGE_CODE_COMMIT OWNER_INTRADAY_QUOTES_MODE KIS_READ_COORDINATION_MODE \
    KIS_MARKET_STREAM_CREDENTIAL_SLOT_ID KIS_READ_CREDENTIAL_GENERATION \
    LAGRANGE_RUNTIME_STATE_DIR LAGRANGE_DATA_DIR LAGRANGE_ARTIFACTS_DIR \
    LAGRANGE_RUNTIME_SECRET_DIR LAGRANGE_SECRET_SOURCE_DIR; do
    printf '%s\0' "$(dotenv_get "$key")"
done
printf '%s\0' "${RELEASE_IMAGE_MANIFEST_IDS[owner-equity-v2-runner]}"
'''


def installed_context(expected: str, entry: Path) -> Context:
    require(os.getuid() == 0 and os.geteuid() == 0, "ERR_ACTOR")
    require(not entry.is_symlink(), "ERR_TRUST")
    entry = entry.resolve(strict=True)
    release = entry.parents[2]
    require(entry == release / "scripts/ops/provision-owner-market-stream.py"
            and release.name == expected and release.parent.name == "releases",
            "ERR_RELEASE")
    root = release.parent.parent
    trusted_directory(release)
    link = root / "current"
    require(link.is_symlink() and os.readlink(link) == "releases/" + expected,
            "ERR_RELEASE")
    for relative in ("scripts/ops/provision-owner-market-stream.py",
                     "scripts/ops/lib/dotenv.sh", "scripts/ops/lib/release-image-manifest.sh"):
        trusted_file(release / relative)
    for relative in ("deploy/compose/.env", ".lagrange-release-manifest"):
        trusted_file(release / relative, protected=True)
    raw = successful(["/bin/bash", "--noprofile", "--norc", "-c", READ_CONTEXT,
                      "market-stream-context", str(release), expected])
    values = raw.split(b"\0")
    require(len(values) == 12 and values[-1] == b"", "ERR_CONFIG")
    try:
        commit, mode, coordination, slot, generation, parent, *paths, image = (
            x.decode("ascii") for x in values[:-1])
        valid_slot = str(uuid.UUID(slot)) == slot and uuid.UUID(slot).int != 0
    except (UnicodeError, ValueError):
        raise ProvisionError("ERR_CONFIG") from None
    require(commit == expected and mode == "off" and coordination == "shared_required"
            and valid_slot and re.fullmatch(r"[1-9][0-9]{0,19}", generation) is not None
            and int(generation) <= 2**64 - 1 and IMAGE.fullmatch(image) is not None,
            "ERR_CONFIG")
    parent = canonical_path(parent)
    for other in [str(root), *paths]:
        other = canonical_path(other)
        require(not parent.is_relative_to(other) and not other.is_relative_to(parent),
                "ERR_PATH")
    trusted_directory(parent)
    info = parent.lstat()
    require(info.st_gid == 10001 and stat.S_IMODE(info.st_mode) == 0o750, "ERR_PARENT")
    return Context(expected, slot, generation, parent, image)


def docker_json(args: list[str]) -> list:
    try:
        value = json.loads(successful(DOCKER + args))
    except (UnicodeError, ValueError):
        raise ProvisionError("ERR_DOCKER") from None
    require(type(value) is list, "ERR_DOCKER")
    return value


def check_image_and_runner(ctx: Context) -> None:
    images = docker_json(["image", "inspect", ctx.image])
    require(len(images) == 1 and images[0].get("Id") == ctx.image, "ERR_IMAGE")
    config = images[0].get("Config") or {}
    require((config.get("Labels") or {}).get("org.opencontainers.image.revision") == ctx.commit
            and config.get("User") == "10001:10001"
            and config.get("Entrypoint") == ["/usr/local/bin/owner-equity-v2-runner"],
            "ERR_IMAGE")
    identifiers = successful(DOCKER + ["ps", "--all", "--quiet", "--no-trunc",
        "--filter", "label=com.docker.compose.project=lagrange-station",
        "--filter", "label=com.docker.compose.service=owner-equity-v2-runner"]).splitlines()
    require(len(identifiers) <= 32, "ERR_RUNNER")
    for raw in identifiers:
        identifier = raw.decode("ascii")
        require(CONTAINER.fullmatch(identifier) is not None, "ERR_RUNNER")
        items = docker_json(["container", "inspect", identifier])
        require(len(items) == 1, "ERR_RUNNER")
        item = items[0]
        labels = (item.get("Config") or {}).get("Labels") or {}
        require(item.get("Id") == identifier
                and labels.get("com.docker.compose.project") == "lagrange-station"
                and labels.get("com.docker.compose.service") == "owner-equity-v2-runner"
                and (item.get("State") or {}).get("Status") in ("created", "exited")
                and (item.get("State") or {}).get("Running") is False, "ERR_RUNNER")


def initializer_argv(ctx: Context, operation: str, name: str, token: str) -> list[str]:
    initializing = operation == "initialize-new"
    require(operation in ("initialize-new", "validate-existing"), "ERR_OPERATION")
    args = DOCKER + ["create", "--pull=never", "--name", name, "--label", f"{LABEL}={token}",
        "--network=none", "--read-only", "--no-healthcheck", "--log-driver=none",
        "--cap-drop=ALL", "--security-opt=no-new-privileges", "--pids-limit=32",
        "--memory=128m", "--cpus=1", "--user", "0:0" if initializing else "10001:10001"]
    if initializing:
        args += ["--cap-add=CHOWN", "--cap-add=FOWNER", "--cap-add=DAC_OVERRIDE"]
    args += ["--mount", f"type=bind,src={ctx.parent},dst=/run/lagrange" + (
        "" if initializing else ",readonly"), "--env", "KIS_READ_CREDENTIAL_GENERATION=" + ctx.generation,
        "--entrypoint", BIN, ctx.image, operation, "--credential-slot-id", ctx.slot,
        "--credential-generation", ctx.generation]
    return args


def owned_container(ctx: Context, name: str, token: str) -> dict | None:
    # Enumeration distinguishes a genuinely absent container from a daemon or
    # inspect failure; the latter must never be reported as successful cleanup.
    raw = successful(DOCKER + ["ps", "--all", "--quiet", "--no-trunc",
                              "--filter", "name=^/" + name + "$"]).splitlines()
    if not raw:
        return None
    require(len(raw) == 1, "ERR_CLEANUP")
    identifier = raw[0].decode("ascii")
    require(CONTAINER.fullmatch(identifier) is not None, "ERR_CLEANUP")
    items = docker_json(["container", "inspect", identifier])
    require(len(items) == 1, "ERR_CLEANUP")
    item = items[0]
    require(item.get("Id") == identifier and item.get("Name") == "/" + name
            and item.get("Image") == ctx.image
            and ((item.get("Config") or {}).get("Labels") or {}).get(LABEL) == token,
            "ERR_CLEANUP")
    return item


def run_initializer(ctx: Context, operation: str, token: str) -> None:
    name = "lagrange-ws-provision-" + token + ("-init" if operation == "initialize-new" else "-check")
    try:
        identifier = successful(initializer_argv(ctx, operation, name, token)).strip().decode("ascii")
        require(CONTAINER.fullmatch(identifier) is not None, "ERR_DOCKER")
        created = owned_container(ctx, name, token)
        require(created is not None and created["Id"] == identifier, "ERR_DOCKER")
        output = successful(DOCKER + ["start", "--attach", identifier], timeout=60)
        item = owned_container(ctx, name, token)
        require(item is not None and item["Id"] == identifier
                and item.get("State", {}).get("Running") is False
                and item.get("State", {}).get("ExitCode") == 0, "ERR_INITIALIZER")
        require(output == (b"OK_INITIALIZED\n" if operation == "initialize-new"
                           else b"OK_VALIDATED\n"), "ERR_INITIALIZER")
    finally:
        item = owned_container(ctx, name, token)
        if item is not None:
            successful(DOCKER + ["rm", "--force", item["Id"]])
            require(owned_container(ctx, name, token) is None, "ERR_CLEANUP")


@contextlib.contextmanager
def parent_descriptor(ctx: Context):
    descriptor = os.open(ctx.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        info = os.fstat(descriptor)
        require(info.st_uid == 0 and info.st_gid == 10001
                and stat.S_IMODE(info.st_mode) == 0o750, "ERR_PARENT")
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield descriptor
    finally:
        os.close(descriptor)


def identity(info: os.stat_result) -> list[int]:
    return [info.st_dev, info.st_ino]


def metadata_at(parent: int, name: str, directory: bool, uid: int, gid: int, mode: int) -> dict:
    descriptor = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK |
                         (os.O_DIRECTORY if directory else 0), dir_fd=parent)
    try:
        info = os.fstat(descriptor)
        require((stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode))
                and info.st_uid == uid and info.st_gid == gid
                and stat.S_IMODE(info.st_mode) == mode
                and (directory or info.st_nlink == 1), "ERR_LAYOUT")
        return {"identity": identity(info), "size": info.st_size,
                "mtime_ns": info.st_mtime_ns, "ctime_ns": info.st_ctime_ns}
    finally:
        os.close(descriptor)


def layout(parent: int) -> tuple[dict, dict]:
    result = {}
    for name, uid, mode in ((STATE, 10001, 0o700), (ANCHORS, 0, 0o750)):
        result[name] = metadata_at(parent, name, True, uid, 10001, mode)["identity"]
    with contextlib.ExitStack() as stack:
        dirs = []
        for name in (STATE, ANCHORS):
            descriptor = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
            stack.callback(os.close, descriptor)
            require(identity(os.fstat(descriptor)) == result[name], "ERR_LAYOUT")
            dirs.append(descriptor)
        require(set(os.listdir(dirs[1])) == {"connection.lock", "state.lock"}, "ERR_LAYOUT")
        for name in ("connection.lock", "state.lock"):
            meta = metadata_at(dirs[1], name, False, 0, 10001, 0o440)
            require(meta["size"] == 0, "ERR_LAYOUT")
            result[name] = meta["identity"]
        require(result["connection.lock"] != result["state.lock"], "ERR_LAYOUT")
        state = metadata_at(dirs[0], "approval-state-v1.json", False, 10001, 10001, 0o600)
        require(0 < state["size"] <= 256 * 1024, "ERR_LAYOUT")
    return result, state


def record_bytes(value: dict) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode("ascii")


def write_exclusive(parent: int, name: str, raw: bytes) -> None:
    descriptor = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=parent)
    try:
        os.fchmod(descriptor, 0o600)
        os.fchown(descriptor, 0, 0)
        with os.fdopen(os.dup(descriptor), "wb") as stream:
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
    finally:
        os.close(descriptor)
    os.fsync(parent)


def read_record(parent: int) -> dict:
    metadata_at(parent, RECORD, False, 0, 0, 0o600)
    descriptor = os.open(RECORD, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
    try:
        with os.fdopen(os.dup(descriptor), "rb") as stream:
            raw = stream.read(8193)
    finally:
        os.close(descriptor)
    try:
        value = json.loads(raw)
        require(len(raw) <= 8192 and type(value) is dict
                and record_bytes(value) == raw, "ERR_RECORD")
    except (UnicodeError, ValueError, TypeError):
        raise ProvisionError("ERR_RECORD") from None
    return value


def execute(ctx: Context, initializing: bool) -> None:
    check_image_and_runner(ctx)
    with parent_descriptor(ctx) as parent:
        token = uuid.uuid4().hex
        binding = {"slot": ctx.slot, "generation": ctx.generation,
                   "parent_identity": identity(os.fstat(parent))}
        if initializing:
            for name in (STATE, ANCHORS, RECORD):
                try:
                    os.stat(name, dir_fd=parent, follow_symlinks=False)
                except FileNotFoundError:
                    continue
                raise ProvisionError("ERR_ALREADY_ATTEMPTED")
            record = {"schema": 1, "status": "attempted", "binding": binding,
                      "commit": ctx.commit, "image": ctx.image, "operation_id": token}
            # This survives failed/uncertain initialization, even if both leaves
            # are later missing. Neither mode deletes or resets this record.
            write_exclusive(parent, RECORD, record_bytes(record))
            run_initializer(ctx, "initialize-new", token)
            anchors, state = layout(parent)
            run_initializer(ctx, "validate-existing", token)
            require(layout(parent) == (anchors, state), "ERR_LAYOUT_CHANGED")
            require(read_record(parent) == record, "ERR_RECORD")
            record.update(status="initialized", layout=anchors)
            temporary = RECORD + "." + token + ".tmp"
            write_exclusive(parent, temporary, record_bytes(record))
            os.replace(temporary, RECORD, src_dir_fd=parent, dst_dir_fd=parent)
            os.fsync(parent)
        else:
            record = read_record(parent)
            require(set(record) == {"schema", "status", "binding", "commit", "image", "operation_id", "layout"}
                    and record["schema"] == 1 and record["status"] == "initialized"
                    and record["binding"] == binding, "ERR_RECORD")
            anchors, state = layout(parent)
            require(record["layout"] == anchors, "ERR_LAYOUT_CHANGED")
            run_initializer(ctx, "validate-existing", token)
            require(layout(parent) == (anchors, state) and read_record(parent) == record,
                    "ERR_LAYOUT_CHANGED")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--plan", action="store_true")
    modes.add_argument("--initialize-new", action="store_true")
    modes.add_argument("--check", action="store_true")
    parser.add_argument("--expected-commit", required=True)
    args = parser.parse_args(argv)
    try:
        require(COMMIT.fullmatch(args.expected_commit) is not None
                and args.expected_commit != "0" * 40, "ERR_COMMIT")
        if not args.initialize_new and not args.check:
            print("PLAN_ONLY: current immutable release; mode off; stopped runner; pinned local image; "
                  "initialize-new once or validate-existing; no activation")
            return 0
        ctx = installed_context(args.expected_commit, Path(__file__))
        execute(ctx, args.initialize_new)
        print("OK_INITIALIZED" if args.initialize_new else "OK_VALIDATED")
        return 0
    except (ProvisionError, OSError, UnicodeError, ValueError, KeyError, TypeError) as error:
        print(str(error) if isinstance(error, ProvisionError) else "ERR_OPERATION", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
