#!/usr/bin/env python3
"""Retained Owner-beta Paper transport and live BEGIN-to-FENCED continuation.

The controller may seal FENCED only after an independently opened exact readback
proves the original BEGIN result and current DRAINING state. This does not prove
writer termination or provide STOPPED, OFF, restore, resume, or ENABLE authority.
Callers must retain the returned contexts and inventories.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import re
import selectors
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
import uuid
import weakref
from pathlib import Path


def _load_local(name, filename):
    path = Path(__file__).with_name(filename)
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError("PAPER_OPS_DEPENDENCY")
    module = sys.modules.get(name)
    if module is not None:
        return module
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


sys.dont_write_bytecode = True
journal = _load_local("owner_beta_paper_operations_journal", "owner-beta-paper-journal.py")
release = _load_local("owner_beta_paper_operations_release", "owner-beta-paper-release.py")
daily = _load_local("owner_beta_paper_operations_daily", "owner-beta-daily-operations.py")
runtime = _load_local("owner_beta_paper_operations_runtime", "owner-beta-daily-runtime.py")

NATIVE = "/usr/local/bin/owner-beta-paper-ops-native-bin"
PRIVATE = "/run/owner-beta-paper-ops"
OPERATION_ALIAS = PRIVATE + "/operation"
PAPER_STATE_ALIAS = PRIVATE + "/paper-state"
OUTPUT_ALIAS = PRIVATE + "/output"
DATABASE_ALIAS = PRIVATE + "/database.json"
POSTGRES_IMAGE = "postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a"
RESULT_SCHEMA = "owner-beta-paper-ops-result-v1"
QUERY_RESULT_SCHEMA = "owner-beta-paper-ops-query-result-v1"
INDEX_SCHEMA = "owner-beta-paper-operations-inspect-index-v1"
PAGE_SCHEMA = "owner-beta-paper-action-v1"
MAX_RESULT = 65_536
MAX_INDEX = 65_536
MAX_PAGE = 262_144
LOCK_NAME = ".lagrange-release.lock"
PAPER_DIR = ".owner-beta-paper"
READER_FLOOR = ".owner-beta-paper-reader-floor"
CONTAINER_LABEL = "io.lagrange.owner-beta-paper-ops.nonce"
# The native binary accepts only the three locale/time fields. The selected
# Alpine image's PATH and release commit are validated, then removed from the
# helper process with Docker's bare-key --env form.
SAFE_ENV = {"LANG", "LC_ALL", "TZ"}
NATIVE_ENV = {"LANG": "C", "LC_ALL": "C", "TZ": "UTC"}
IMAGE_ENV_NAMES = SAFE_ENV | {"PATH", "LAGRANGE_CODE_COMMIT"}
# Source-known PATH for the pinned Alpine runtime; the selected image is checked
# against it at inspection time (also used by the Daily runtime test fixture).
IMAGE_PATH = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
REMOVE_ENV = ("PATH", "HOSTNAME", "LAGRANGE_CODE_COMMIT", "TERM")
EXPECTED_RAW_ENV = ["LANG=C", "LC_ALL=C", "TZ=UTC", *REMOVE_ENV]
ENV_NAME = re.compile(r"[A-Za-z_][A-Za-z0-9_]*\Z")
OUTCOMES = {"OK", "REPLAY", "BLOCKED", "COMMIT_UNKNOWN"}
ERRORS = {
    "NONE", "WIRE_INVALID", "IDENTITY_CONFLICT", "STALE_EPOCH", "CLOCK_INVALID",
    "WRITER_UNKNOWN", "COMMIT_UNKNOWN", "READER_INCOMPATIBLE",
    "RECOVERY_AUTHORITY_UNAVAILABLE", "EVIDENCE_UNAVAILABLE", "INTERNAL_FAILURE",
}
RESULT_FIELDS = ("schema", "verb", "outcome", "event_id", "generation",
                 "control_epoch", "artifact_sha256", "error_code")
RESULT_SCHEMAS = {"begin": RESULT_SCHEMA, "inspect": RESULT_SCHEMA,
                  "readback": QUERY_RESULT_SCHEMA}
_FENCED_CALLS = weakref.WeakKeyDictionary()


class PaperOperationsError(Exception):
    """Fixed diagnostic code; never includes provider, Docker, or secret text."""

    def __init__(self, code="PAPER_OPS_INVALID"):
        self.code = code
        super().__init__(code)


def _fail(code="PAPER_OPS_INVALID"):
    raise PaperOperationsError(code)


def _require(condition, code="PAPER_OPS_INVALID"):
    if not condition:
        _fail(code)


def _root():
    _require(sys.platform == "linux" and os.geteuid() == 0 and sys.maxsize > 2**32,
             "PAPER_OPS_PLATFORM")


def _canonical(value):
    try:
        return json.dumps(value, ensure_ascii=True, sort_keys=True,
                          separators=(",", ":"), allow_nan=False).encode("ascii")
    except (TypeError, ValueError, UnicodeError, RecursionError):
        _fail("PAPER_OPS_WIRE")


def _pairs(items):
    result = {}
    for key, value in items:
        if key in result:
            _fail("PAPER_OPS_WIRE")
        result[key] = value
    return result


def _json(data, maximum):
    if type(data) is not bytes or not data or len(data) > maximum:
        _fail("PAPER_OPS_SIZE")
    try:
        value = json.loads(data.decode("ascii"), object_pairs_hook=_pairs,
                           parse_constant=lambda _value: _fail("PAPER_OPS_WIRE"),
                           parse_float=lambda _value: _fail("PAPER_OPS_WIRE"))
    except PaperOperationsError:
        raise
    except (UnicodeError, ValueError, TypeError, RecursionError):
        _fail("PAPER_OPS_WIRE")
    if _canonical(value) != data:
        _fail("PAPER_OPS_CANONICAL")
    return value


def _digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def _canonical_uuid_lf(value):
    if type(value) is not bytes or not value.endswith(b"\n") or value.count(b"\n") != 1:
        return False
    try:
        text = value[:-1].decode("ascii")
    except UnicodeError:
        return False
    return (journal.UUID_RE.fullmatch(text) is not None
            and text != "00000000-0000-0000-0000-000000000000")


def _identity(info):
    return (info.st_dev, info.st_ino, info.st_uid, info.st_gid,
            stat.S_IMODE(info.st_mode), info.st_nlink)


def _path_from_fd(fd):
    try:
        value = os.readlink("/proc/self/fd/%d" % fd)
    except OSError:
        _fail("PAPER_OPS_PATH")
    _require(value.startswith("/") and not value.endswith(" (deleted)") and
             not any(c in value for c in (",", "\x00", "\n", "\r")), "PAPER_OPS_PATH")
    return value


def _dir(fd, mode=0o700, *, uid=0, gid=0):
    value = os.fstat(fd)
    _require(stat.S_ISDIR(value.st_mode) and value.st_uid == uid and value.st_gid == gid
             and stat.S_IMODE(value.st_mode) == mode and value.st_mode & 0o022 == 0,
             "PAPER_OPS_DIRECTORY")
    daily.install.no_acl(fd)
    return value


def _file(fd, mode=0o400, *, uid=0, gid=0):
    value = os.fstat(fd)
    _require(stat.S_ISREG(value.st_mode) and value.st_uid == uid and value.st_gid == gid
             and stat.S_IMODE(value.st_mode) == mode and value.st_nlink == 1
             and value.st_mode & 0o022 == 0, "PAPER_OPS_FILE")
    daily.install.no_acl(fd)
    return value


def _check_named_directory(parent_fd, name, fd, identity):
    try:
        current = _identity(_dir(fd))
        named = _identity(os.stat(name, dir_fd=parent_fd, follow_symlinks=False))
    except OSError:
        _fail("PAPER_OPS_STAGE_CHANGED")
    _require(current == identity and named == identity, "PAPER_OPS_STAGE_CHANGED")


def _open_file(parent, name, mode=0o400, *, maximum=MAX_RESULT, uid=0, gid=0):
    _require(isinstance(name, str) and name not in ("", ".", "..") and "/" not in name,
             "PAPER_OPS_PATH")
    try:
        fd = os.open(name, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW | os.O_CLOEXEC,
                     dir_fd=parent)
    except OSError:
        _fail("PAPER_OPS_FILE")
    try:
        info = _file(fd, mode, uid=uid, gid=gid)
        _require(info.st_size <= maximum, "PAPER_OPS_SIZE")
        chunks = []
        offset = 0
        while offset <= maximum:
            chunk = os.pread(fd, min(65_536, maximum + 1 - offset), offset)
            if not chunk:
                break
            chunks.append(chunk)
            offset += len(chunk)
        data = b"".join(chunks)
        _require(len(data) <= maximum and os.pread(fd, 1, len(data)) == b"",
                 "PAPER_OPS_SIZE")
        named = os.stat(name, dir_fd=parent, follow_symlinks=False)
        _require(_identity(named) == _identity(os.fstat(fd)), "PAPER_OPS_CHANGED")
        return fd, data, _identity(info)
    except BaseException:
        os.close(fd)
        raise


def _ensure_output_mountpoint(directory_fd):
    try:
        os.mkdir("output", 0o700, dir_fd=directory_fd)
        os.fsync(directory_fd)
    except FileExistsError:
        pass
    output_fd = os.open("output", journal.DIR_FLAGS, dir_fd=directory_fd)
    try:
        _dir(output_fd)
        return _identity(os.fstat(output_fd))
    finally:
        os.close(output_fd)


def _read_migration_secret(secret_root):
    _require(type(secret_root) is str and secret_root.startswith("/"), "PAPER_OPS_SECRET")
    daily.migration_secret(secret_root)
    tree = journal._Tree(secret_root)
    service_fd = file_fd = None
    try:
        service_fd = os.open("db-migrate", journal.DIR_FLAGS, dir_fd=tree.fd)
        service_info = daily.install.directory_info(service_fd, uid=999, gid=999, mode=0o750)
        file_fd = os.open("db_migration_owner_password",
                          os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=service_fd)
        info = daily.install.file_info(file_fd, uid=999, gid=999, mode=0o400)
        _require(info.st_size <= 8192, "PAPER_OPS_SECRET")
        secret = os.pread(file_fd, 8193, 0)
        _require(0 < len(secret) <= 8192 and os.pread(file_fd, 1, len(secret)) == b"",
                 "PAPER_OPS_SECRET")
        source = {"tree": tree, "service_fd": service_fd, "file_fd": file_fd,
                  "identity": _identity(info), "service_identity": _identity(service_info),
                  "bytes": secret}
        tree, service_fd, file_fd = None, None, None
        return secret, (source,)
    except BaseException:
        if file_fd is not None:
            os.close(file_fd)
        if service_fd is not None:
            os.close(service_fd)
        if tree is not None:
            tree.close()
        raise


def _revalidate_secret_source(source):
    tree, service_fd, file_fd = source["tree"], source["service_fd"], source["file_fd"]
    tree.revalidate()
    daily.install.directory_info(service_fd, uid=999, gid=999, mode=0o750)
    daily.install.file_info(file_fd, uid=999, gid=999, mode=0o400)
    _require(_identity(os.fstat(service_fd)) == source["service_identity"]
             and _identity(os.fstat(file_fd)) == source["identity"]
             and os.pread(file_fd, len(source["bytes"]) + 1, 0) == source["bytes"]
             and _identity(os.stat("db-migrate", dir_fd=tree.fd,
                                  follow_symlinks=False)) == source["service_identity"]
             and _identity(os.stat("db_migration_owner_password", dir_fd=service_fd,
                                  follow_symlinks=False)) == source["identity"],
             "PAPER_OPS_SECRET_CHANGED")


def _write_file(parent, name, data, mode=0o400):
    _require(type(data) is bytes and len(data) <= MAX_RESULT, "PAPER_OPS_SIZE")
    fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                 mode, dir_fd=parent)
    try:
        os.fchmod(fd, mode)
        view = memoryview(data)
        while view:
            written = os.write(fd, view)
            if written <= 0:
                _fail("PAPER_OPS_WRITE")
            view = view[written:]
        os.fsync(fd)
        _file(fd, mode)
        os.fsync(parent)
        return _identity(os.fstat(fd))
    except BaseException:
        raise
    finally:
        os.close(fd)


def _hash_fd(fd, maximum=128 * 1024 * 1024):
    digest = hashlib.sha256()
    offset = 0
    while offset <= maximum:
        chunk = os.pread(fd, min(1024 * 1024, maximum + 1 - offset), offset)
        if not chunk:
            break
        digest.update(chunk)
        offset += len(chunk)
    _require(offset <= maximum and os.pread(fd, 1, offset) == b"", "PAPER_OPS_SIZE")
    return offset, "sha256:" + digest.hexdigest()


def _docker_client_environment():
    # Docker itself is invoked by absolute path with an explicit local socket.
    # In particular, bare --env KEY must not copy a host value into the helper.
    return dict(NATIVE_ENV)


def _validate_docker_client_environment(environment):
    _require(type(environment) is dict and environment == NATIVE_ENV
             and not set(environment) & {"PATH", "HOME", "HOSTNAME",
                                         "LAGRANGE_CODE_COMMIT", "TERM"},
             "PAPER_OPS_DOCKER_ENV")


def _image_environment(image_config, selected_commit):
    _require(type(image_config) is dict, "PAPER_OPS_IMAGE")
    env = image_config.get("Env")
    if env is None:
        env = []
    _require(type(env) is list, "PAPER_OPS_IMAGE_ENV")
    parsed = {}
    for item in env:
        _require(type(item) is str and "=" in item, "PAPER_OPS_IMAGE_ENV")
        name, value = item.split("=", 1)
        _require(ENV_NAME.fullmatch(name) is not None and name in IMAGE_ENV_NAMES
                 and name not in parsed and "\x00" not in value
                 and "\r" not in value and "\n" not in value,
                 "PAPER_OPS_IMAGE_ENV")
        if name == "PATH":
            _require(value == IMAGE_PATH, "PAPER_OPS_IMAGE_ENV")
        elif name == "LAGRANGE_CODE_COMMIT":
            _require(type(selected_commit) is str
                     and re.fullmatch(r"[0-9a-f]{40}", selected_commit) is not None
                     and value == selected_commit,
                     "PAPER_OPS_IMAGE_ENV")
        parsed[name] = value
    return parsed


def _effective_container_environment(image_env, raw_overrides, *, daemon_env,
                                     client_env, tty, links):
    """Model Docker/Moby env application for the inspected fixed config."""
    _require(type(image_env) is dict and type(raw_overrides) is list
             and type(daemon_env) is dict, "PAPER_OPS_CONTAINER_ENV")
    _validate_docker_client_environment(client_env)
    _require(tty is False and links in (None, []), "PAPER_OPS_CONTAINER_POLICY")
    effective = dict(daemon_env)
    effective.update(image_env)
    for item in raw_overrides:
        _require(type(item) is str, "PAPER_OPS_CONTAINER_ENV")
        if "=" in item:
            name, value = item.split("=", 1)
            _require(ENV_NAME.fullmatch(name) is not None and "\x00" not in value,
                     "PAPER_OPS_CONTAINER_ENV")
            effective[name] = value
        else:
            name = item
            _require(ENV_NAME.fullmatch(name) is not None and name not in client_env,
                     "PAPER_OPS_CONTAINER_ENV")
            effective.pop(name, None)
    return effective


def _native_env_args():
    return [argument for item in EXPECTED_RAW_ENV for argument in ("--env", item)]


def _validate_result(stdout, expected_verb, exit_status):
    _require(type(stdout) is bytes and len(stdout) <= MAX_RESULT + 1, "PAPER_OPS_RESULT")
    raw = stdout[:-1] if stdout.endswith(b"\n") else stdout
    _require(len(raw) <= MAX_RESULT and b"\n" not in raw and b"\r" not in raw,
             "PAPER_OPS_RESULT")
    try:
        value = json.loads(raw.decode("ascii"), object_pairs_hook=_pairs,
                           parse_constant=lambda _value: _fail("PAPER_OPS_RESULT"),
                           parse_float=lambda _value: _fail("PAPER_OPS_RESULT"))
    except PaperOperationsError:
        _fail("PAPER_OPS_RESULT")
    except (UnicodeError, ValueError, TypeError, RecursionError):
        _fail("PAPER_OPS_RESULT")
    _require(type(value) is dict and tuple(value) == RESULT_FIELDS
             and type(expected_verb) is str and expected_verb in RESULT_SCHEMAS
             and type(value["schema"]) is str
             and value["schema"] == RESULT_SCHEMAS[expected_verb]
             and type(value["verb"]) is str and value["verb"] == expected_verb
             and type(value["outcome"]) is str and value["outcome"] in OUTCOMES
             and type(value["error_code"]) is str and value["error_code"] in ERRORS,
             "PAPER_OPS_RESULT")
    try:
        producer_order = json.dumps(value, ensure_ascii=True, sort_keys=False,
                                    separators=(",", ":"), allow_nan=False).encode("ascii")
    except (TypeError, ValueError, UnicodeError, RecursionError):
        _fail("PAPER_OPS_RESULT")
    _require(producer_order == raw, "PAPER_OPS_RESULT")
    for key in ("event_id", "generation"):
        item = value[key]
        _require(item is None or (type(item) is str and journal.UUID_RE.fullmatch(item)),
                 "PAPER_OPS_RESULT")
    epoch = value["control_epoch"]
    _require(epoch is None or (type(epoch) is int and 0 <= epoch <= journal.MAX_I64),
             "PAPER_OPS_RESULT")
    artifact = value["artifact_sha256"]
    _require(artifact is None or (type(artifact) is str and journal.HASH_RE.fullmatch(artifact)),
             "PAPER_OPS_RESULT")
    expected_exit = {"OK": 0, "REPLAY": 0, "BLOCKED": 3, "COMMIT_UNKNOWN": 4}[value["outcome"]]
    _require(exit_status == expected_exit, "PAPER_OPS_RESULT")
    if value["outcome"] in {"OK", "REPLAY"}:
        _require(value["error_code"] == "NONE", "PAPER_OPS_RESULT")
    elif value["outcome"] == "COMMIT_UNKNOWN":
        _require(value["error_code"] == "COMMIT_UNKNOWN", "PAPER_OPS_RESULT")
    else:
        _require(value["error_code"] != "NONE", "PAPER_OPS_RESULT")
    return value


def _preserve_unknown_result(result, had_preexisting_dispatch):
    return (result.get("outcome") == "COMMIT_UNKNOWN"
            and (not had_preexisting_dispatch or result.get("event_id") is not None
                 or result.get("artifact_sha256") is not None))


def _validate_container_identity(value, cid, nonce, created, image, commit):
    config = value.get("Config") or {}
    labels = config.get("Labels") or {}
    _require(value.get("Id") == cid and value.get("Image") == image
             and value.get("Created") == created
             and labels.get(CONTAINER_LABEL) == nonce
             and labels.get("org.opencontainers.image.revision") == commit
             and type(value.get("RestartCount")) is int and value["RestartCount"] == 0,
             "PAPER_OPS_CONTAINER_IDENTITY")


def _validate_mount_records(actual_mounts, expected_mounts):
    expected = {(os.path.realpath(source), target, not readonly)
                for source, target, readonly in expected_mounts}
    _require(type(actual_mounts) is list and len(actual_mounts) == len(expected),
             "PAPER_OPS_MOUNT")
    actual = []
    for mount in actual_mounts:
        _require(type(mount) is dict and mount.get("Type") == "bind"
                 and type(mount.get("Source")) is str
                 and type(mount.get("Destination")) is str
                 and type(mount.get("RW")) is bool,
                 "PAPER_OPS_MOUNT")
        actual.append((mount["Source"], mount["Destination"], mount["RW"]))
    _require(set(actual) == expected and len(set(actual)) == len(actual), "PAPER_OPS_MOUNT")


def _validate_runtime_policy(config, host, verb, image_env, network_id, client_env):
    actual_env = config.get("Env")
    _require(type(actual_env) is list and all(type(item) is str for item in actual_env)
             and actual_env == EXPECTED_RAW_ENV,
             "PAPER_OPS_CONTAINER_ENV")
    _require(config.get("Entrypoint") == [NATIVE]
             and config.get("Cmd") == [verb]
             and config.get("User") == "0:0"
             and config.get("WorkingDir") == "/"
             and config.get("Tty") is False
             and config.get("Links") in (None, []),
             "PAPER_OPS_CONTAINER_POLICY")
    effective_env = _effective_container_environment(
        image_env, actual_env,
        daemon_env={"PATH": "<daemon-path>", "HOSTNAME": "<daemon-hostname>"},
        client_env=client_env, tty=config.get("Tty"), links=host.get("Links"))
    _require(effective_env == NATIVE_ENV, "PAPER_OPS_CONTAINER_ENV")
    _require(host.get("ReadonlyRootfs") is True and host.get("Privileged") is False
             and host.get("CapAdd") in (None, []) and host.get("CapDrop") == ["ALL"]
             and host.get("AutoRemove") is False
             and host.get("RestartPolicy", {}).get("Name") == "no"
             and host.get("LogConfig", {}).get("Type") == "none"
             and host.get("Memory") == 2_147_483_648
             and host.get("NanoCpus") == 1_000_000_000
             and host.get("NetworkMode") == network_id
             and host.get("Binds") in (None, []) and host.get("Tmpfs") in (None, {})
             and host.get("Devices") in (None, [])
             and host.get("DeviceRequests") in (None, [])
             and host.get("VolumesFrom") in (None, [])
             and host.get("PortBindings") in (None, {})
             and host.get("PublishAllPorts") is not True
             and host.get("PidMode") in ("", "private")
             and host.get("IpcMode") in ("", "private")
             and host.get("UTSMode") in ("", "private")
             and host.get("CgroupnsMode") in ("", "private")
             and host.get("Links") in (None, []),
             "PAPER_OPS_CONTAINER_POLICY")
    security = host.get("SecurityOpt") or []
    _require(security in (["no-new-privileges"], ["no-new-privileges:true"]),
             "PAPER_OPS_CONTAINER_POLICY")


def _manifest_and_context(context, tree):
    _require(type(context) is daily.Context, "PAPER_OPS_CONTEXT")
    release_path, commit = context.release, context.commit
    _require(type(release_path) is str and type(commit) is str
             and re.fullmatch(r"[0-9a-f]{40}", commit) and commit != "0" * 40,
             "PAPER_OPS_CONTEXT")
    install_root = str(Path(release_path).parent.parent)
    _require(install_root == str(Path(install_root)) and
             release_path == install_root + "/releases/" + commit,
             "PAPER_OPS_CONTEXT")
    _require(install_root == "/" + "/".join(tree.names), "PAPER_OPS_CONTEXT")
    images = daily.read_manifest(release_path, commit)
    project, _mode = daily.project_binding(commit, commit)
    values, overlays = daily.read_context(release_path, commit, "disable")
    _require(type(context.images) is dict and context.images == images
             and type(context.values) is dict and context.values == values
             and type(context.overlays) is list and context.overlays == overlays,
             "PAPER_OPS_CONTEXT")
    image = images.get("paper-scheduler")
    _require(type(image) is str and runtime.DIGEST.fullmatch(image), "PAPER_OPS_MANIFEST")
    return install_root, release_path, commit, images, values, overlays, project, image


def _database_image_id(docker):
    """Resolve the independently pinned upstream image, outside release V3."""
    value = docker.object("image", POSTGRES_IMAGE)
    identity, references = value.get("Id"), value.get("RepoDigests")
    _require(type(identity) is str and runtime.DIGEST.fullmatch(identity)
             and type(references) is list
             and all(type(item) is str for item in references)
             and POSTGRES_IMAGE in references, "PAPER_OPS_DATABASE_IMAGE")
    return identity


def _capture_database(docker, project, expected_image):
    network = daily.internal_network(docker, project)
    _require(network.get("Internal") is True, "PAPER_OPS_NETWORK")
    network_id = network.get("Id")
    _require(type(network_id) is str and runtime.CONTAINER.fullmatch(network_id),
             "PAPER_OPS_NETWORK")
    found = []
    for identity in (network.get("Containers") or {}):
        value = docker.object("container", identity)
        for endpoint in (value.get("NetworkSettings", {}).get("Networks") or {}).values():
            if endpoint.get("NetworkID") == network_id and "postgres" in (endpoint.get("Aliases") or []):
                found.append((identity, value))
    _require(len(found) == 1, "PAPER_OPS_DATABASE")
    identity, server = found[0]
    labels = server.get("Config", {}).get("Labels") or {}
    state = server.get("State") or {}
    _require(server.get("Id") == identity and state.get("Running") is True
             and server.get("Created") and state.get("StartedAt")
             and type(server.get("RestartCount")) is int and server["RestartCount"] >= 0
             and server.get("Image") == expected_image
             and labels.get("com.docker.compose.project") == project
             and labels.get("com.docker.compose.service") == "postgres",
             "PAPER_OPS_DATABASE")
    return network_id, (identity, server["Created"], state["StartedAt"],
                        server["RestartCount"], server.get("Image"))


class _Stage:
    """A named 0700 work directory, retained until the invocation is settled."""
    __slots__ = ("path", "fd", "identity", "closed", "tree", "name", "files", "directories")

    def __init__(self, prefix, *, root="/run"):
        self.tree = journal._Tree(root)
        self.path = None
        self.fd = None
        try:
            self.path = tempfile.mkdtemp(prefix=prefix, dir=root)
            self.name = os.path.basename(self.path)
            os.chmod(self.path, 0o700)
            self.fd = os.open(self.path, journal.DIR_FLAGS)
            self.identity = _identity(_dir(self.fd))
            self.closed = False
            self.files, self.directories = [], []
            self.revalidate()
        except BaseException:
            if self.fd is not None:
                os.close(self.fd)
            self.tree.close()
            if self.path is not None:
                shutil.rmtree(self.path, ignore_errors=True)
            raise

    def snapshot(self, *, mutable_children=()):
        _require(not self.closed and not self.files and not self.directories, "PAPER_OPS_STAGE")
        _require(type(mutable_children) in (tuple, set, frozenset)
                 and all(type(name) is str and name not in ("", ".", "..") and "/" not in name
                         for name in mutable_children), "PAPER_OPS_STAGE")

        def walk(parent, parent_path, parent_binding=None):
            names = sorted(entry.name for entry in os.scandir(parent))
            binding_parent, binding_name = parent_binding or (self.tree.fd, self.name)
            self.directories.append((parent, parent_path, tuple(names), _identity(_dir(parent)),
                                     binding_parent, binding_name))
            for name in names:
                _require(name not in (".", "..") and "/" not in name, "PAPER_OPS_STAGE")
                info = os.stat(name, dir_fd=parent, follow_symlinks=False)
                if stat.S_ISDIR(info.st_mode):
                    fd = os.open(name, journal.DIR_FLAGS, dir_fd=parent)
                    _dir(fd)
                    if parent_path == self.path and name in mutable_children:
                        self.directories.append((fd, os.path.join(parent_path, name), None,
                                                 _identity(os.fstat(fd)), parent, name))
                    else:
                        walk(fd, os.path.join(parent_path, name), (parent, name))
                else:
                    _require(stat.S_ISREG(info.st_mode), "PAPER_OPS_STAGE")
                    fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC,
                                 dir_fd=parent)
                    info = _file(fd, 0o400)
                    size, digest = _hash_fd(fd)
                    self.files.append((fd, parent, name, _identity(info), size, digest))

        walk(self.fd, self.path)
        self.revalidate()

    def revalidate(self):
        _require(not self.closed and _identity(_dir(self.fd)) == self.identity,
                 "PAPER_OPS_STAGE")
        self.tree.revalidate()
        _require(_identity(os.stat(self.name, dir_fd=self.tree.fd,
                                   follow_symlinks=False)) == self.identity,
                 "PAPER_OPS_STAGE")
        for fd, parent, name, identity, size, digest in self.files:
            now_size, now_digest = _hash_fd(fd)
            _require(_identity(os.fstat(fd)) == identity and
                     _identity(os.stat(name, dir_fd=parent, follow_symlinks=False)) == identity
                     and now_size == size and now_digest == digest,
                     "PAPER_OPS_STAGE_CHANGED")
        for fd, path, expected_names, identity, parent, name in self.directories:
            _check_named_directory(parent, name, fd, identity)
            if expected_names is not None:
                _require(tuple(sorted(entry.name for entry in os.scandir(fd))) == expected_names,
                         "PAPER_OPS_STAGE_CHANGED")

    def close(self, *, remove=False):
        if self.closed:
            return
        self.revalidate()
        if remove:
            shutil.rmtree(self.name, dir_fd=self.tree.fd)
            os.fsync(self.tree.fd)
        for fd, *_rest in reversed(self.files):
            os.close(fd)
        for fd, path, _names, _identity_value, _parent, _name in reversed(self.directories[1:]):
            os.close(fd)
        os.close(self.fd)
        self.tree.close()
        self.closed = True


class _PaperContext:
    __slots__ = ("_token", "context", "install_root", "release_path", "commit", "images",
                 "values", "overlays", "project", "image", "image_environment",
                 "manifest_sha256", "tree",
                 "lock_fd", "lock_identity", "verified", "docker", "network_id",
                 "database_generation", "database_image_id", "database", "secret_path", "_closed", "_mode",
                 "_env_bytes", "_manifest_bytes", "_manifest_fd", "_manifest_identity",
                 "_release_tree", "_invocations", "_inventories", "_input_stages",
                 "_retained_stages", "_begin_result_fds", "_initializing", "__weakref__")

    def __init__(self, token, context, release_lock_fd, *, live):
        if token is not _CONTEXT_TOKEN:
            _fail("PAPER_OPS_PRIVATE")
        self._initializing = True
        _root()
        _require(type(context) is daily.Context, "PAPER_OPS_CONTEXT")
        self._token, self.context, self._mode = token, context, "live" if live else "custody"
        self.release_path, self.commit = context.release, context.commit
        self.install_root = str(Path(self.release_path).parent.parent)
        self.tree = journal._Tree(self.install_root)
        self._release_tree = journal._Tree(self.release_path)
        # Keep our own descriptor to the caller's already-held open description.
        # Closing it never unlocks the caller's original descriptor.
        self.lock_fd = os.dup(release_lock_fd)
        self.lock_identity = journal._check_lock(self.install_root, self.lock_fd, self.tree)
        (self.install_root, self.release_path, self.commit, self.images, self.values,
         self.overlays, self.project, self.image) = _manifest_and_context(context, self.tree)
        self._manifest_fd, manifest, self._manifest_identity = _open_file(
            self._release_tree.fd, release.MANIFEST_NAME, 0o600,
            maximum=release.MAX_MANIFEST)
        self._manifest_bytes = manifest
        self.manifest_sha256 = _digest(manifest)
        self.verified = release.verify(self.install_root, self.release_path, self.commit) if live else None
        self.docker = runtime.Docker()
        # This is a new client owned by this context, not Daily's shared client.
        # Absolute Docker argv/socket make PATH and HOME unnecessary.
        self.docker.environment = _docker_client_environment()
        try:
            image_info = self.docker.object("image", self.image)
        except Exception:
            _fail("PAPER_OPS_IMAGE")
        config = image_info.get("Config") or {}
        labels = config.get("Labels") or {}
        _require(image_info.get("Id") == self.image
                 and labels.get("org.opencontainers.image.revision") == self.commit,
                 "PAPER_OPS_IMAGE")
        self.image_environment = _image_environment(config, self.commit)
        self.database_image_id = _database_image_id(self.docker)
        self.network_id, self.database_generation = _capture_database(
            self.docker, self.project, self.database_image_id)
        self.database = {"schema": "owner-beta-paper-ops-database-v1", "host": "postgres",
                         "port": 5432,
                         "database": self.values.get("POSTGRES_DB") or "lagrange"}
        _require(re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,62}", self.database["database"]) is not None,
                 "PAPER_OPS_DATABASE")
        self.secret_path = daily.migration_secret(self.values["LAGRANGE_RUNTIME_SECRET_DIR"])
        self._env_bytes = daily.install.read_root_file(
            self.release_path + "/deploy/compose/.env", gid=0, mode=0o600,
            maximum=128 * 1024)
        self._closed, self._invocations, self._inventories = False, set(), set()
        self._input_stages = {}
        self._retained_stages = set()
        self._begin_result_fds = {}
        self.revalidate()
        self._initializing = False

    def revalidate(self):
        _require(not self._closed, "PAPER_OPS_CLOSED")
        try:
            self.tree.revalidate()
            _validate_docker_client_environment(self.docker.environment)
            journal._check_lock(self.install_root, self.lock_fd, self.tree)
            _require(journal._identity(os.fstat(self.lock_fd)) == self.lock_identity,
                     "PAPER_OPS_LOCK")
            if self.verified is not None:
                self.verified.revalidate()
            current = daily.install.read_root_file(
                self.release_path + "/deploy/compose/.env", gid=0, mode=0o600,
                maximum=128 * 1024)
            _require(current == self._env_bytes, "PAPER_OPS_CONTEXT")
            self._release_tree.revalidate()
            _require(_identity(os.fstat(self._manifest_fd)) == self._manifest_identity
                     and os.pread(self._manifest_fd, len(self._manifest_bytes) + 1, 0) == self._manifest_bytes
                     and _identity(os.stat(release.MANIFEST_NAME, dir_fd=self._release_tree.fd,
                                          follow_symlinks=False)) == self._manifest_identity,
                     "PAPER_OPS_MANIFEST")
            _require(_database_image_id(self.docker) == self.database_image_id,
                     "PAPER_OPS_DATABASE_IMAGE")
            network_id, database_generation = _capture_database(
                self.docker, self.project, self.database_image_id)
            _require(network_id == self.network_id and
                     database_generation == self.database_generation,
                     "PAPER_OPS_DATABASE_GENERATION")
            image_info = self.docker.object("image", self.image)
            image_config = image_info.get("Config") or {}
            _require(image_info.get("Id") == self.image and
                     (image_config.get("Labels") or {}).get(
                         "org.opencontainers.image.revision") == self.commit and
                     _image_environment(image_config, self.commit) == self.image_environment,
                     "PAPER_OPS_IMAGE")
            current_project, _current_mode = daily.project_binding(self.commit, self.commit)
            _require(current_project == self.project, "PAPER_OPS_PROJECT")
            return None
        except PaperOperationsError:
            raise
        except Exception:
            _fail("PAPER_OPS_REVALIDATION")

    def invoke(self, verb, action_custody):
        self.revalidate()
        _require(type(verb) is str, "PAPER_OPS_VERB")
        if self._mode == "live":
            _require(verb == "begin" and type(action_custody) is journal.ActionCustody,
                     "PAPER_OPS_VERB")
            action_custody.revalidate()
            _require(action_custody._action["verb"] == "begin" and
                     action_custody._operation._intent["purpose"] in {"STOP", "BACKUP"},
                     "PAPER_OPS_ACTION")
            _validate_begin_custody(action_custody)
            invocation = _NativeInvocation(_INVOCATION_TOKEN, self, verb, action_custody)
        else:
            _require(verb == "readback" and type(action_custody) is journal.Custody
                     and action_custody._readonly, "PAPER_OPS_VERB")
            action_custody._check()
            invocation = _NativeInvocation(_INVOCATION_TOKEN, self, verb, action_custody)
        self._invocations.add(invocation)
        return invocation

    def fence_begin(self, invocation):
        """Append FENCED only after fresh exact COMMITTED readback and cleanup.

        This seals the BEGIN result in the operation journal; it is not evidence
        that writers have stopped and creates no DrainedHandle or later authority.
        """
        _require(self._mode == "live" and type(invocation) is _NativeInvocation
                 and invocation._token is _INVOCATION_TOKEN
                 and invocation._context is self and invocation.verb == "begin"
                 and invocation._entered is True and invocation._closed is True
                 and invocation._imported is True and invocation._preserve is False
                 and invocation._cid is None and invocation._child is None
                 and type(invocation._exit) is int and invocation._exit == 0
                 and invocation._cleanup_state == "RETAINED_FOR_ACTION"
                 and type(invocation._result) is dict
                 and type(invocation._result.get("outcome")) is str
                 and invocation._result.get("outcome") in {"OK", "REPLAY"}
                 and invocation._result.get("error_code") == "NONE",
                 "PAPER_OPS_BEGIN_UNSETTLED")
        _require(not self._invocations and not self._inventories and not self._retained_stages,
                 "PAPER_OPS_UNRESOLVED")
        action_custody = invocation.custody
        operation = action_custody._operation if type(action_custody) is journal.ActionCustody else None
        _require(operation is not None, "PAPER_OPS_ACTION")
        action, root, stage0_bytes, intent_stage, head = _validate_fence_begin_action(
            operation, action_custody)
        root_bytes = operation._intent_bytes
        entry = self._input_stages.get(action_custody)
        _require(type(entry) is dict and set(entry) == {"stage", "imports"}
                 and entry["stage"] is invocation._persistent_stage
                 and invocation._persistent_stage is not None
                 and type(entry["imports"]) is list
                 and bool(entry["imports"])
                 and all(type(item) is journal.Custody and item._closed
                         and item._action is action_custody
                         for item in entry["imports"]),
                 "PAPER_OPS_UNRESOLVED")
        for prior_action, prior_entry in self._input_stages.items():
            _require(type(prior_action) is journal.ActionCustody
                     and type(prior_entry) is dict and set(prior_entry) == {"stage", "imports"}
                     and type(prior_entry["imports"]) is list
                     and all(type(item) is journal.Custody and item._closed
                             and item._action is prior_action
                             for item in prior_entry["imports"]),
                     "PAPER_OPS_UNRESOLVED")
            if prior_action is not action_custody:
                _require(prior_action._closed, "PAPER_OPS_UNRESOLVED")
                prior_entry["stage"].revalidate()

        fence = _begin_fence_epoch(root)
        fenced_calls = _FENCED_CALLS.get(self)
        if fenced_calls is None:
            fenced_calls = weakref.WeakKeyDictionary()
            _FENCED_CALLS[self] = fenced_calls
        _require(self.verified is not None
                 and self.manifest_sha256 == root["release_manifest_sha256"]
                 and self.verified.manifest_sha256 == self.manifest_sha256,
                 "PAPER_OPS_RELEASE")
        self.revalidate()
        self.verified.revalidate()
        current_boot = _current_boot_id()
        _require(intent_stage["boot_id"] == current_boot, "PAPER_OPS_BOOT")
        operation._journal.revalidate()
        _require(operation._journal._lock_id == self.lock_identity,
                 "PAPER_OPS_LOCK")
        current_floor = operation._journal._read_floor()
        prior = fenced_calls.get(invocation)
        replay = prior is not None
        if replay:
            _require(type(prior) is tuple and len(prior) == 3
                     and type(prior[0]) is bytes and type(prior[1]) is str
                     and type(prior[2]) is bytes
                     and head[0] == 1 and head[2] == "FENCED"
                     and head[1] == prior[1] and head[3]["stage"] == "FENCED"
                     and journal._read_named(operation._fd, "00000001.json", 0o400,
                                             journal.MAX_SMALL) == prior[0]
                     and prior[2] == invocation._result_bytes
                     and current_floor <= fence,
                     "PAPER_OPS_STAGE")
        else:
            _require(head[0] == 0 and head[1] == action["stage_sha256"]
                     and head[2] == "INTENT" and head[3] == intent_stage
                     and current_floor == root["local_epoch_floor"],
                     "PAPER_OPS_STAGE")
        _require(current_floor >= root["local_epoch_floor"] and current_floor <= fence,
                 "PAPER_OPS_EPOCH")

        dispatch_bytes = action_custody._dispatch_bytes
        _require(type(dispatch_bytes) is bytes and dispatch_bytes
                 and action_custody._import_fd is not None,
                 "PAPER_OPS_IMPORT")
        output_fd = action_custody.borrow_output_fd()
        import_bytes = journal._read_named(action_custody._fd, "import.json", 0o400,
                                           journal.MAX_NATIVE_INDEX)
        output_dispatch = journal._read_named(output_fd, "dispatch.json", 0o400,
                                              journal.MAX_NATIVE_INDEX)
        _require(import_bytes == dispatch_bytes and output_dispatch == dispatch_bytes,
                 "PAPER_OPS_IMPORT")
        result_bytes = invocation._result_bytes
        result_value = _validate_begin_result_artifact(result_bytes, invocation,
                                                       operation, fence)
        result_hash = journal._sha(result_bytes)
        result_name = result_hash[7:] + ".json"
        # The original stays open from the BEGIN invocation through context close.
        # Reopening by name after closing it cannot detect reuse of the same inode.
        retained_fd = self._begin_result_fds.get(invocation)
        _require(type(retained_fd) is int, "PAPER_OPS_RESULT_CHANGED")
        result_fd = os.dup(retained_fd)
        result_identity = invocation._result_identity

        def revalidate_result_original():
            _file(result_fd, 0o400)
            _require(self._begin_result_fds.get(invocation) == retained_fd
                     and invocation._result_name == result_name
                     and invocation._result_parent == output_fd
                     and invocation._result_identity == result_identity
                     and _identity(os.fstat(result_fd)) == result_identity
                     and os.pread(result_fd, len(result_bytes) + 1, 0) == result_bytes
                     and _identity(os.stat(result_name, dir_fd=output_fd,
                                           follow_symlinks=False)) == result_identity,
                     "PAPER_OPS_RESULT_CHANGED")

        try:
            revalidate_result_original()
            historical = operation._journal.open_custody(operation.generation, 0)
            readback_context = None
            try:
                _require(type(historical) is journal.Custody and historical._readonly
                         and historical._intent == operation._intent_bytes
                         and historical._artifact is None
                         and historical._dispatch == dispatch_bytes,
                         "PAPER_OPS_IMPORT")
                request_bytes = historical._sql
                request = journal._json(request_bytes, journal.MAX_NATIVE_EXACT)
                _require(type(request) is dict and set(request) == {
                    "schema", "kind", "account_id", "args"
                } and request["schema"] == "owner-beta-paper-exact-request-v1"
                         and request["kind"] == "BEGIN"
                         and request["account_id"] == root["account_id"]
                         and type(request["args"]) is dict and set(request["args"]) == {
                             "expected_epoch", "intent_canonical", "request_key"
                         }
                         and type(request["args"]["expected_epoch"]) is int
                         and request["args"]["expected_epoch"] == root["expected_epoch"]
                         and request["args"]["intent_canonical"] == root_bytes.decode("ascii")
                         and request["args"]["request_key"] == root["generation"] + ":begin"
                         and journal._canonical(request) == request_bytes,
                         "PAPER_OPS_IMPORT")
                with open_custody_context(self.context, self.lock_fd) as readback_context:
                    _require(readback_context._mode == "custody"
                             and readback_context.release_path == self.release_path
                             and readback_context.commit == self.commit
                             and readback_context.image == self.image
                             and readback_context.manifest_sha256 == self.manifest_sha256
                             and readback_context._manifest_bytes == self._manifest_bytes
                             and readback_context.database_image_id == self.database_image_id
                             and readback_context.network_id == self.network_id
                             and readback_context.database_generation == self.database_generation,
                             "PAPER_OPS_READBACK_CONTEXT")
                    with readback_context.invoke("readback", historical) as readback:
                        response = readback.result
                        _require(type(response) is dict
                                 and tuple(response) == RESULT_FIELDS
                                 and response["schema"] == QUERY_RESULT_SCHEMA
                                 and response["verb"] == "readback"
                                 and response["outcome"] == "REPLAY"
                                 and response["error_code"] == "NONE"
                                 and response.get("generation") == operation.generation
                                 and response.get("event_id") == invocation._result["event_id"]
                                 and response.get("control_epoch") == fence,
                                 "PAPER_OPS_READBACK")
                        readback_bytes = readback.readback_artifact()
                        _require(response["artifact_sha256"] == journal._sha(readback_bytes),
                                 "PAPER_OPS_READBACK")
                    _require(readback._closed is True and readback._cleanup_state == "REMOVED"
                             and readback._cid is None and readback._child is None
                             and readback._preserve is False and readback._imported is True
                             and not readback_context._invocations,
                             "PAPER_OPS_READBACK_CLEANUP")
                    _validate_begin_readback(readback_bytes, request_bytes, result_bytes,
                                             operation, fence, invocation._result["event_id"])
                _require(readback_context._closed is True, "PAPER_OPS_READBACK_CONTEXT")
            finally:
                historical.close()
            _require(historical._closed is True, "PAPER_OPS_READBACK_CLEANUP")
            revalidate_result_original()

            self.revalidate()
            self.verified.revalidate()
            operation.revalidate()
            action_custody.revalidate()
            _require(operation._journal._lock_id == self.lock_identity,
                     "PAPER_OPS_LOCK")
            _require(operation._intent_bytes == root_bytes
                     and journal._read_named(operation._fd, "00000000.json", 0o400,
                                             journal.MAX_SMALL) == stage0_bytes
                     and action_custody._binding_bytes == journal._canonical(action)
                     and action_custody._dispatch_bytes == dispatch_bytes
                     and journal._read_named(action_custody._fd, "import.json", 0o400,
                                             journal.MAX_NATIVE_INDEX) == dispatch_bytes
                     and journal._read_named(output_fd, "dispatch.json", 0o400,
                                             journal.MAX_NATIVE_INDEX) == dispatch_bytes,
                     "PAPER_OPS_CUSTODY_CHANGED")
            _require(_current_boot_id() == current_boot
                     and intent_stage["boot_id"] == current_boot
                     and self.manifest_sha256 == root["release_manifest_sha256"]
                     and self.verified.manifest_sha256 == self.manifest_sha256,
                     "PAPER_OPS_CONTEXT_CHANGED")
            revalidate_result_original()
            current_floor = operation._journal._read_floor()
            _require(root["local_epoch_floor"] <= current_floor <= fence,
                     "PAPER_OPS_EPOCH")
            operation._journal.revalidate()
            if replay:
                _require(operation._last[0] == 1 and operation._last[2] == "FENCED"
                         and operation._last[1] == prior[1]
                         and operation._last[3]["artifact_sha256"] == result_hash,
                         "PAPER_OPS_STAGE")
                stage_bytes, stage_hash = prior[0], prior[1]
            else:
                _require(operation._last[0] == 0 and operation._last[2] == "INTENT"
                         and current_floor == root["local_epoch_floor"],
                         "PAPER_OPS_STAGE")
                operation._store_original(result_bytes, result_hash, journal.MAX_SMALL)
                _require(operation.read_original(result_hash) == result_bytes,
                         "PAPER_OPS_RESULT_CUSTODY")
                revalidate_result_original()
                stage = {
                    "schema": "owner-beta-paper-operation-stage-v1",
                    "installation_id": root["installation_id"],
                    "generation": root["generation"],
                    "account_id": root["account_id"],
                    "sequence": 1,
                    "previous_sha256": head[1],
                    "intent_sha256": operation.intent_sha256,
                    "stage": "FENCED",
                    "boot_id": current_boot,
                    "observed": _stage_observation(),
                    "control_epoch": fence,
                    "artifact_sha256": result_hash,
                    "event_id": invocation._result["event_id"],
                    "error_code": "NONE",
                }
                stage_bytes = journal._canonical(stage)
                stage_hash = operation.append(stage_bytes)
                fenced_calls[invocation] = (stage_bytes, stage_hash, result_bytes)
            revalidate_result_original()
            operation._journal.advance_floor(fence)
            self.revalidate()
            self.verified.revalidate()
            operation.revalidate()
            action_custody.revalidate()
            _require(operation._last[0] == 1 and operation._last[2] == "FENCED"
                     and operation._last[1] == stage_hash
                     and journal._read_named(operation._fd, "00000001.json", 0o400,
                                             journal.MAX_SMALL) == stage_bytes
                     and operation._journal._read_floor() == fence
                     and operation.read_original(result_hash) == result_bytes,
                     "PAPER_OPS_STAGE")
            revalidate_result_original()
            return stage_hash
        finally:
            os.close(result_fd)

    def probe_history(self):
        self.revalidate()
        _require(self._mode == "live", "PAPER_OPS_PROBE_CONTEXT")
        try:
            inventory = _HistoryInventory(_HISTORY_TOKEN, self)
            self._inventories.add(inventory)
            return inventory
        except PaperOperationsError:
            raise
        except Exception:
            _fail("PAPER_OPS_HISTORY_UNKNOWN")

    def _input_stage(self, action_custody):
        _require(self._mode == "live" and type(action_custody) is journal.ActionCustody,
                 "PAPER_OPS_ACTION")
        entry = self._input_stages.get(action_custody)
        if entry is None:
            entry = {"stage": _Stage("owner-beta-paper-input-"), "imports": []}
            self._input_stages[action_custody] = entry
            return entry["stage"], True
        entry["stage"].revalidate()
        return entry["stage"], False

    def _register_imported_custody(self, action_custody, custody):
        entry = self._input_stages.get(action_custody)
        _require(entry is not None and type(custody) is journal.Custody,
                 "PAPER_OPS_IMPORT")
        entry["imports"].append(custody)

    def _close_begin_results(self):
        retained = getattr(self, "_begin_result_fds", {})
        while retained:
            _invocation, fd = retained.popitem()
            os.close(fd)

    def close(self):
        if self._closed:
            return
        _require(not self._inventories, "PAPER_OPS_INVENTORY_OPEN")
        for invocation in tuple(self._invocations):
            invocation.close()
        _require(not self._invocations, "PAPER_OPS_UNRESOLVED")
        _require(not self._retained_stages, "PAPER_OPS_UNRESOLVED")
        for action_custody, entry in tuple(self._input_stages.items()):
            _require(action_custody._closed and
                     all(custody._closed for custody in entry["imports"]),
                     "PAPER_OPS_ACTION_OPEN")
            entry["stage"].close(remove=True)
        self._input_stages.clear()
        self._close_begin_results()
        self.revalidate()
        if self.verified is not None:
            self.verified.close()
            self.verified = None
        os.close(self._manifest_fd)
        self._manifest_fd = None
        self._release_tree.close()
        self.tree.close()
        os.close(self.lock_fd)
        self.lock_fd = None
        self._closed = True

    def __enter__(self):
        self.revalidate()
        return self

    def __exit__(self, *_exc):
        self.close()

    def __del__(self):
        # Construction can fail before open_context returns a manager. Close
        # only local descriptors here; never unlock the caller-owned release FD.
        try:
            self._close_begin_results()
        except OSError:
            pass
        for name in ("_manifest_fd",):
            fd = getattr(self, name, None)
            if isinstance(fd, int):
                try:
                    os.close(fd)
                except OSError:
                    pass
        for name in ("verified",):
            handle = getattr(self, name, None)
            if handle is not None:
                try:
                    handle.close()
                except Exception:
                    pass
        for name in ("_release_tree", "tree"):
            handle = getattr(self, name, None)
            if handle is not None:
                try:
                    handle.close()
                except Exception:
                    pass
        lock_fd = getattr(self, "lock_fd", None)
        if isinstance(lock_fd, int) and getattr(self, "_initializing", False):
            try:
                os.close(lock_fd)
            except OSError:
                pass


_CONTEXT_TOKEN = object()


def open_context(context, release_lock_fd):
    """Open a retained live BEGIN context from the installed, locked release."""
    try:
        return _PaperContext(_CONTEXT_TOKEN, context, release_lock_fd, live=True)
    except PaperOperationsError:
        raise
    except Exception:
        _fail("PAPER_OPS_CONTEXT")


def open_custody_context(context, release_lock_fd):
    """Open only the installed helper/DB transport for old-custody readback."""
    try:
        return _PaperContext(_CONTEXT_TOKEN, context, release_lock_fd, live=False)
    except PaperOperationsError:
        raise
    except Exception:
        _fail("PAPER_OPS_CONTEXT")


class _NativeInvocation:
    __slots__ = ("_token", "_context", "verb", "custody", "_stages", "_mounts",
                 "_container", "_nonce", "_created", "_cid", "_child", "_exit",
                 "_stdout", "_result", "_entered", "_closed", "_imported",
                 "_preserve", "_artifact_bytes", "_artifact_hash", "_failure",
                 "_probe_output_fd", "_probe_output_identity", "_paper_state_fd",
                 "_paper_state_identity", "_paper_state_files", "_secret_sources",
                 "_container_name", "_result_fd", "_result_identity", "_result_name",
                 "_result_bytes", "_result_parent", "_preexisting_dispatch_fd",
                 "_preexisting_dispatch_identity", "_preexisting_dispatch_bytes",
                 "_preexisting_dispatch_parent", "_cleanup_state", "_persistent_stage",
                 "__weakref__")

    def __init__(self, token, context, verb, custody):
        if token is not _INVOCATION_TOKEN:
            _fail("PAPER_OPS_PRIVATE")
        self._token, self._context, self.verb, self.custody = token, context, verb, custody
        self._stages, self._mounts = [], []
        self._container = None
        self._nonce = self._created = self._cid = self._child = self._exit = None
        self._stdout = self._result = None
        self._entered = self._closed = self._imported = self._preserve = False
        self._artifact_bytes = self._artifact_hash = self._failure = None
        self._probe_output_fd = self._probe_output_identity = None
        self._paper_state_fd = self._paper_state_identity = None
        self._paper_state_files = ()
        self._secret_sources = ()
        self._container_name = None
        self._result_fd = self._result_identity = self._result_name = None
        self._result_bytes = self._result_parent = None
        self._preexisting_dispatch_fd = self._preexisting_dispatch_identity = None
        self._preexisting_dispatch_bytes = self._preexisting_dispatch_parent = None
        self._cleanup_state = "NOT_STARTED"
        self._persistent_stage = None

    def _new_stage(self, prefix):
        stage = _Stage(prefix)
        self._stages.append(stage)
        return stage

    def _prepare(self):
        context = self._context
        context.revalidate()
        if self.verb == "inspect":
            _require(self._probe_output_fd is not None, "PAPER_OPS_OUTPUT")
            for stage in self._stages:
                stage.revalidate()
            return
        if self.verb == "begin":
            stage, stage_new = context._input_stage(self.custody)
            self._stages.append(stage)
            self._persistent_stage = stage
            # This must be first: ActionCustody enforces an empty destination.
            self.custody.stage_inputs(stage.fd)
            if stage_new:
                _write_file(stage.fd, "database.json", _canonical(context.database), 0o400)
        else:
            stage = self._new_stage("owner-beta-paper-input-")
            self.custody.stage_readback(stage.fd)
            _write_file(stage.fd, "database.json", _canonical(context.database), 0o400)
        secret_stage = self._new_stage("owner-beta-paper-secret-")
        secret, source_fds = _read_migration_secret(context.values["LAGRANGE_RUNTIME_SECRET_DIR"])
        self._secret_sources = source_fds
        _require(secret and not any(byte in secret for byte in (0, 10, 13)),
                 "PAPER_OPS_SECRET")
        _write_file(secret_stage.fd, "db_migration_owner_password", secret, 0o400)
        # Remove the transient Python reference before any Docker operation.
        secret = None
        mounts = []
        if self.verb == "begin":
            action = self.custody
            operation = action._operation
            operation_fd = operation.borrow_directory_fd()
            expected_operation = (context.install_root + "/" + PAPER_DIR + "/operations/" +
                                  operation.generation)
            operation_path = _path_from_fd(operation_fd)
            _require(operation_path == expected_operation, "PAPER_OPS_OPERATION")
            op_stat = _dir(operation_fd)
            named_parent = os.open(os.path.dirname(operation_path), journal.DIR_FLAGS)
            try:
                named = os.stat(os.path.basename(operation_path), dir_fd=named_parent,
                                follow_symlinks=False)
                _require(_identity(named) == _identity(op_stat), "PAPER_OPS_OPERATION")
            finally:
                os.close(named_parent)
            mounts.append((operation_path, OPERATION_ALIAS, True))
            state_path = context.install_root + "/" + PAPER_DIR
            state_fd = identity_fd = floor_fd = None
            try:
                state_fd = os.open(PAPER_DIR, journal.DIR_FLAGS, dir_fd=context.tree.fd)
                state_info = _dir(state_fd)
                identity_fd, identity_data, identity_id = _open_file(
                    state_fd, "installation-id", 0o600, maximum=64)
                floor_fd, floor_data, floor_id = _open_file(state_fd, "epoch-floor", 0o600, maximum=32)
                expected_identity = operation._intent["installation_id"].encode("ascii") + b"\n"
                expected_floor = str(operation._intent["local_epoch_floor"]).encode("ascii") + b"\n"
                _require(identity_data == expected_identity and _canonical_uuid_lf(identity_data)
                         and floor_data == expected_floor and
                         re.fullmatch(rb"(?:0|[1-9][0-9]{0,18})\n", floor_data),
                         "PAPER_OPS_PAPER_STATE")
            except BaseException:
                for fd in (identity_fd, floor_fd, state_fd):
                    if fd is not None:
                        os.close(fd)
                raise
            self._paper_state_fd = state_fd
            self._paper_state_identity = _identity(state_info)
            self._paper_state_files = tuple(self._paper_state_files) + (
                (identity_fd, "installation-id", identity_id, identity_data),
                (floor_fd, "epoch-floor", floor_id, floor_data),
            )
            mounts.append((state_path, PAPER_STATE_ALIAS, True))
            _require(stat.S_IMODE(state_info.st_mode) == 0o700, "PAPER_OPS_PAPER_STATE")
            _require(context.verified is not None, "PAPER_OPS_RELEASE")
            if stage_new:
                context.verified.stage_native_inputs(stage.fd)
                for alias in ("operation", "paper-state"):
                    try:
                        os.mkdir(alias, 0o700, dir_fd=stage.fd)
                    except FileExistsError:
                        _fail("PAPER_OPS_MOUNT")
            mounts.append((stage.path, PRIVATE, True))
        else:
            output_fd = self.custody.borrow_output_fd()
            self.custody._check()
            mounts.append((stage.path, PRIVATE, True))
        _ensure_output_mountpoint(stage.fd)
        os.fsync(stage.fd)
        stage.revalidate()
        mounts.append((secret_stage.path, "/run/secrets", True))
        if self.verb == "begin":
            output_fd = self.custody.borrow_output_fd()
            self.custody.revalidate()
            try:
                os.stat("dispatch.json", dir_fd=output_fd, follow_symlinks=False)
            except FileNotFoundError:
                pass
            else:
                dispatch_fd, dispatch_bytes, dispatch_identity = _open_file(
                    output_fd, "dispatch.json", 0o400, maximum=journal.MAX_NATIVE_INDEX)
                self._preexisting_dispatch_fd = dispatch_fd
                self._preexisting_dispatch_identity = dispatch_identity
                self._preexisting_dispatch_bytes = dispatch_bytes
                self._preexisting_dispatch_parent = output_fd
            mounts.append((_path_from_fd(output_fd), OUTPUT_ALIAS, False))
        elif self.verb == "readback":
            # Historical readback must use its original output and operation-bound bytes.
            mounts.append((_path_from_fd(output_fd), OUTPUT_ALIAS, False))
        self._mounts = tuple(mounts)
        if self.verb != "begin" or stage_new:
            stage.snapshot()
        secret_stage.snapshot()
        context.revalidate()

    def _prepare_inspect(self, stage, output_fd):
        context = self._context
        self._stages.append(stage)
        self._probe_output_fd = os.dup(output_fd)
        self._probe_output_identity = _identity(_dir(self._probe_output_fd))
        _ensure_output_mountpoint(stage.fd)
        secret_stage = self._new_stage("owner-beta-paper-secret-")
        secret, sources = _read_migration_secret(context.values["LAGRANGE_RUNTIME_SECRET_DIR"])
        self._secret_sources = sources
        _require(secret and not any(byte in secret for byte in (0, 10, 13)), "PAPER_OPS_SECRET")
        _write_file(secret_stage.fd, "db_migration_owner_password", secret, 0o400)
        secret = None
        os.fsync(stage.fd)
        stage.revalidate()
        self._mounts = (
            (stage.path, PRIVATE, True),
            (secret_stage.path, "/run/secrets", True),
            (_path_from_fd(self._probe_output_fd), OUTPUT_ALIAS, False),
        )
        stage.snapshot(mutable_children=("output",))
        secret_stage.snapshot()
        context.revalidate()

    def _create_args(self):
        context = self._context
        self._nonce = uuid.uuid4().hex
        self._container_name = "owner-beta-paper-ops-" + self._nonce
        args = ["create", "--name", self._container_name,
                "--label", CONTAINER_LABEL + "=" + self._nonce,
                "--user", "0:0", "--workdir", "/", "--entrypoint", NATIVE,
                "--restart", "no", "--read-only", "--cap-drop", "ALL",
                "--security-opt", "no-new-privileges", "--log-driver", "none",
                "--cpus", "1", "--memory", "2147483648", "--network", context.network_id]
        args.extend(_native_env_args())
        for source, target, readonly in self._mounts:
            _require(source.startswith("/") and target.startswith("/") and "," not in source,
                     "PAPER_OPS_MOUNT")
            mount = "type=bind,source=" + source + ",target=" + target
            if readonly:
                mount += ",readonly"
            args.extend(("--mount", mount))
        args.extend((context.image, self.verb))
        return args

    def _inspect_and_validate(self, *, started):
        context = self._context
        context.revalidate()
        value = context.docker.object("container", self._cid)
        config, host, state = value.get("Config") or {}, value.get("HostConfig") or {}, value.get("State") or {}
        _validate_container_identity(value, self._cid, self._nonce, self._created,
                                     context.image, context.commit)
        image_config = context.docker.object("image", context.image).get("Config") or {}
        image_env = _image_environment(image_config, context.commit)
        _require(image_env == context.image_environment, "PAPER_OPS_IMAGE_ENV")
        _validate_runtime_policy(config, host, self.verb, image_env,
                                 context.network_id, context.docker.environment)
        _validate_mount_records(value.get("Mounts") or [], self._mounts)
        networks = value.get("NetworkSettings", {}).get("Networks") or {}
        ids = {item.get("NetworkID") for item in networks.values()}
        _require(ids <= {context.network_id} and (not started or ids == {context.network_id}),
                 "PAPER_OPS_NETWORK")
        if started is True:
            _require(state.get("Running") is False and state.get("Status") == "exited"
                     and state.get("ExitCode") == self._exit,
                     "PAPER_OPS_CONTAINER_STATE")
        elif started is False:
            _require(state.get("Running") is False and state.get("Status") in {"created", "configured"},
                     "PAPER_OPS_CONTAINER_STATE")
        else:
            _require(state.get("Status") in {"running", "exited"}, "PAPER_OPS_CONTAINER_STATE")
        self._container = value
        return value

    def _run(self):
        context = self._context
        try:
            self._prepare()
            context.revalidate()
        except PaperOperationsError:
            raise
        except Exception:
            _fail("PAPER_OPS_INPUT")
        create_args = self._create_args()
        self._cleanup_state = "CREATE_SENT"
        try:
            cid = context.docker.call(create_args, timeout=30).strip()
        except Exception:
            self._preserve = True
            self._cleanup_state = "UNKNOWN"
            try:
                value = context.docker.object("container", self._container_name)
                if (value.get("Config", {}).get("Labels", {}).get(CONTAINER_LABEL) == self._nonce
                        and value.get("Image") == context.image):
                    self._cid = value.get("Id")
                    self._cleanup_state = "OWNED" if self._cid else "UNKNOWN"
                    self._created = value.get("Created")
                    if runtime.CONTAINER.fullmatch(self._cid or ""):
                        self._inspect_and_validate(started=None)
            except Exception:
                pass
            _fail("PAPER_OPS_CREATE_UNKNOWN")
        if runtime.CONTAINER.fullmatch(cid) is None:
            self._preserve = True
            self._cleanup_state = "UNKNOWN"
            _fail("PAPER_OPS_CREATE_UNKNOWN")
        self._cid = cid
        self._cleanup_state = "OWNED"
        try:
            created = context.docker.object("container", cid).get("Created")
            _require(type(created) is str and created, "PAPER_OPS_CONTAINER")
            self._created = created
            self._inspect_and_validate(started=False)
            context.revalidate()
            command = ["/usr/bin/docker", "--host", "unix:///var/run/docker.sock",
                       "start", "--attach", "--interactive", cid]
            child = subprocess.Popen(command, env=context.docker.environment,
                                     stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL, close_fds=True)
            self._child = child
            try:
                stdout = self._read_attached(child, timeout=90)
            except PaperOperationsError:
                self._reap_child(child)
                self._preserve = True
                self._failure = "PAPER_OPS_ATTACH_UNKNOWN"
                try:
                    self._inspect_and_validate(started=None)
                except Exception:
                    pass
                _fail(self._failure)
            finally:
                if child.stdout is not None:
                    child.stdout.close()
                if child.poll() is not None:
                    self._child = None
            self._stdout = stdout
            self._exit = child.returncode
            context.revalidate()
            container = context.docker.object("container", cid)
            state = container.get("State") or {}
            if state.get("Running") is True:
                self._preserve = True
                _fail("PAPER_OPS_CONTAINER_UNKNOWN")
            _require(state.get("Status") == "exited" and type(state.get("ExitCode")) is int,
                     "PAPER_OPS_CONTAINER_UNKNOWN")
            self._exit = state["ExitCode"]
            _require(child.returncode == self._exit, "PAPER_OPS_ATTACH_UNKNOWN")
            self._inspect_and_validate(started=True)
            self._result = _validate_result(stdout, self.verb, self._exit)
            self._capture_result_artifact()
            if _preserve_unknown_result(self._result,
                                        self._preexisting_dispatch_fd is not None):
                self._preserve = True
            context.revalidate()
            return None
        except PaperOperationsError:
            if self._cid is not None and self._failure is None:
                self._preserve = True
            raise
        except Exception:
            self._preserve = True
            _fail("PAPER_OPS_INVOCATION_UNKNOWN")

    @staticmethod
    def _reap_child(child):
        if child.poll() is None:
            try:
                child.send_signal(signal.SIGTERM)
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    _fail("PAPER_OPS_CHILD_UNKNOWN")
            except OSError:
                _fail("PAPER_OPS_CHILD_UNKNOWN")
        if child.poll() is None:
            _fail("PAPER_OPS_CHILD_UNKNOWN")

    def _read_attached(self, child, *, timeout):
        selector = selectors.DefaultSelector()
        output = bytearray()
        last_check = time.monotonic()
        deadline = last_check + timeout
        selector.register(child.stdout, selectors.EVENT_READ)
        try:
            while True:
                now = time.monotonic()
                if now >= deadline:
                    _fail("PAPER_OPS_ATTACH_TIMEOUT")
                events = selector.select(min(0.5, deadline - now))
                if events:
                    chunk = os.read(child.stdout.fileno(), 4096)
                    if not chunk:
                        break
                    output.extend(chunk)
                    if len(output) > MAX_RESULT + 1:
                        _fail("PAPER_OPS_ATTACH_SIZE")
                if time.monotonic() - last_check >= 5:
                    self._context.revalidate()
                    if self.verb == "begin":
                        self.custody.revalidate()
                    elif self.verb == "readback":
                        self.custody._check()
                    self._revalidate_paper_state()
                    last_check = time.monotonic()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                _fail("PAPER_OPS_ATTACH_TIMEOUT")
            return bytes(output)
        finally:
            selector.close()

    @property
    def result(self):
        _require(self._entered and self._result is not None, "PAPER_OPS_RESULT")
        return dict(self._result)

    def import_dispatch(self):
        _require(self.verb == "begin" and self._entered and self._result is not None,
                 "PAPER_OPS_IMPORT")
        self._revalidate()
        custody = self.custody.import_dispatch()
        try:
            self._revalidate()
            self._context._register_imported_custody(self.custody, custody)
            self._imported = True
            return custody
        except BaseException:
            custody.close()
            raise

    @property
    def cleanup(self):
        """Descriptive cleanup state only; it carries no Paper or DB authority."""
        stages = ("RETAINED" if self._preserve else "RETAINED_FOR_ACTION"
                  if self._persistent_stage is not None and self._closed else
                  "PENDING" if not self._closed else "REMOVED")
        return {"container": self._cleanup_state,
                "local_child_reaped": self._child is None,
                "stages": stages}

    def readback_artifact(self):
        _require(self.verb == "readback" and self._entered and self._result is not None,
                 "PAPER_OPS_READBACK")
        self._revalidate()
        _require(self._result_bytes is not None, "PAPER_OPS_READBACK")
        artifact_bytes = self._result_bytes
        self._revalidate()
        self._artifact_bytes, self._artifact_hash = artifact_bytes, self._result["artifact_sha256"]
        self._imported = True
        return self._result_bytes

    def _output_fd(self):
        if self.verb == "begin":
            return self.custody.borrow_output_fd()
        if self.verb == "readback":
            return self.custody.borrow_output_fd()
        return self._probe_output_fd

    def _capture_result_artifact(self):
        digest = self._result.get("artifact_sha256")
        if digest is None:
            return
        _require(type(digest) is str and journal.HASH_RE.fullmatch(digest), "PAPER_OPS_RESULT")
        parent = self._output_fd()
        name = digest[7:] + ".json"
        fd, data, identity = _open_file(parent, name, 0o400, maximum=MAX_PAGE)
        try:
            _require(_digest(data) == digest, "PAPER_OPS_RESULT")
            if self.verb == "begin":
                original = _json(data, MAX_PAGE)
                operation = self.custody._operation
                _require(type(original) is dict and set(original) == {
                    "schema", "action", "outcome", "control_epoch", "account_id", "data"
                } and original["schema"] == PAGE_SCHEMA and original["action"] == "BEGIN_OPERATION"
                  and original["outcome"] in {"OK", "REPLAY"}
                  and original["account_id"] == operation._intent["account_id"]
                  and type(original["data"]) is dict and set(original["data"]) == {
                      "generation", "operation_event_id", "intent_sha256", "fenced_epoch"
                  } and original["data"]["generation"] == operation.generation
                  and original["data"]["intent_sha256"] == operation.intent_sha256
                  and original["data"]["fenced_epoch"] == original["control_epoch"]
                  and original["data"]["operation_event_id"] == self._result["event_id"]
                  and original["control_epoch"] == self._result["control_epoch"],
                  "PAPER_OPS_RESULT")
            self._result_fd, self._result_bytes, self._result_identity = fd, data, identity
            self._result_name, self._result_parent = name, parent
        except BaseException:
            os.close(fd)
            raise

    def _revalidate(self):
        self._context.revalidate()
        if self.verb == "begin":
            self.custody.revalidate()
        elif self.verb == "readback":
            self.custody._check()
        elif self.verb == "inspect":
            _require(self._probe_output_fd is not None and
                     _identity(_dir(self._probe_output_fd)) == self._probe_output_identity,
                     "PAPER_OPS_OUTPUT")
        for stage in self._stages:
            stage.revalidate()
        self._revalidate_paper_state()
        for fd, name, identity, data in self._paper_state_files:
            _file(fd, 0o600, uid=0, gid=0)
            parent = self._paper_state_fd
            _require(parent is not None, "PAPER_OPS_PAPER_STATE_CHANGED")
            _require(_identity(os.fstat(fd)) == identity and os.pread(fd, len(data) + 1, 0) == data
                     and _identity(os.stat(name, dir_fd=parent, follow_symlinks=False)) == identity,
                     "PAPER_OPS_PAPER_STATE_CHANGED")
        for item in self._secret_sources:
            _revalidate_secret_source(item)
        if self._result_fd is not None:
            _require(_identity(os.fstat(self._result_fd)) == self._result_identity
                     and os.pread(self._result_fd, len(self._result_bytes) + 1, 0) == self._result_bytes
                     and _identity(os.stat(self._result_name, dir_fd=self._result_parent,
                                          follow_symlinks=False)) == self._result_identity,
                     "PAPER_OPS_RESULT_CHANGED")
        if self._preexisting_dispatch_fd is not None:
            _require(_identity(os.fstat(self._preexisting_dispatch_fd)) ==
                     self._preexisting_dispatch_identity
                     and os.pread(self._preexisting_dispatch_fd,
                                  len(self._preexisting_dispatch_bytes) + 1, 0) ==
                     self._preexisting_dispatch_bytes
                     and _identity(os.stat("dispatch.json",
                                           dir_fd=self._preexisting_dispatch_parent,
                                           follow_symlinks=False)) ==
                     self._preexisting_dispatch_identity,
                     "PAPER_OPS_DISPATCH_CHANGED")
        if self._cid is not None:
            self._inspect_and_validate(started=self._exit is not None)
        return None

    def _revalidate_paper_state(self):
        if self._paper_state_fd is None:
            return
        context = self._context
        _require(_identity(_dir(self._paper_state_fd)) == self._paper_state_identity,
                 "PAPER_OPS_PAPER_STATE_CHANGED")
        _require(_identity(os.stat(PAPER_DIR, dir_fd=context.tree.fd,
                                   follow_symlinks=False)) == self._paper_state_identity,
                 "PAPER_OPS_PAPER_STATE_CHANGED")

    def close(self):
        if self._closed:
            return
        if self._child is not None and self._child.poll() is None:
            self._reap_child(self._child)
            self._child = None
        if self._cid is not None:
            try:
                value = self._context.docker.object("container", self._cid)
            except Exception:
                self._preserve = True
                _fail("PAPER_OPS_CONTAINER_UNKNOWN")
            state = value.get("State") or {}
            if state.get("Running") is not False or state.get("Status") != "exited":
                self._preserve = True
                _fail("PAPER_OPS_CONTAINER_UNKNOWN")
            self._inspect_and_validate(started=True)
            if not self._imported:
                self._preserve = True
                _fail("PAPER_OPS_UNRESOLVED")
            else:
                self._revalidate()
                try:
                    self._context.docker.call(["rm", self._cid], timeout=15)
                    remaining = self._context.docker.call(
                        ["ps", "--all", "--no-trunc", "--filter", "id=" + self._cid,
                         "--format", "{{.ID}}"], timeout=15)
                    _require(remaining == "", "PAPER_OPS_CONTAINER_CLEANUP_UNKNOWN")
                except Exception:
                    self._preserve = True
                    _fail("PAPER_OPS_CONTAINER_CLEANUP_UNKNOWN")
                self._cid = None
                self._cleanup_state = "REMOVED"
        if self._preserve and not self._imported:
            _fail("PAPER_OPS_UNRESOLVED")
        if not self._preserve:
            for stage in reversed(self._stages):
                if stage is not self._persistent_stage:
                    stage.close(remove=True)
            self._cleanup_state = ("RETAINED_FOR_ACTION" if self._persistent_stage is not None
                                   else "REMOVED")
        else:
            for stage in self._stages:
                try:
                    stage.revalidate()
                except Exception:
                    pass
                if stage is not self._persistent_stage:
                    self._context._retained_stages.add(stage)
        if (self.verb == "begin" and self._context._mode == "live"
                and self._cleanup_state == "RETAINED_FOR_ACTION"
                and self._imported and not self._preserve and self._exit == 0
                and self._cid is None and self._child is None
                and self._result_fd is not None):
            # Transfer ownership without closing or reopening the original inode.
            _require(self not in self._context._begin_result_fds,
                     "PAPER_OPS_RESULT_CHANGED")
            self._context._begin_result_fds[self] = self._result_fd
            self._result_fd = None
        for fd in (self._probe_output_fd, self._paper_state_fd,
                   self._result_fd,
                   self._preexisting_dispatch_fd,
                   *(item[0] for item in self._paper_state_files),
                   *(item["file_fd"] for item in self._secret_sources),
                   *(item["service_fd"] for item in self._secret_sources)):
            if fd is not None:
                try:
                    os.close(fd)
                except OSError:
                    pass
        for item in self._secret_sources:
            item["tree"].close()
        self._closed = True
        self._context._invocations.discard(self)

    def __enter__(self):
        _require(not self._entered and not self._closed, "PAPER_OPS_INVOCATION")
        self._entered = True
        try:
            self._run()
            return self
        except BaseException:
            raise

    def __exit__(self, *_exc):
        self.close()


_INVOCATION_TOKEN = object()


class _HistoryInventory:
    __slots__ = ("_token", "_context", "_local", "_index", "_pages", "_history",
                 "_configured", "_closed", "_presence", "_probe_stage", "_snapshot")

    def __init__(self, token, context):
        if token is not _HISTORY_TOKEN:
            _fail("PAPER_OPS_PRIVATE")
        self._token, self._context = token, context
        self._local = _LocalMarkers(context)
        self._index, self._pages, self._history = None, (), None
        self._configured = None
        self._closed, self._presence, self._probe_stage, self._snapshot = False, None, None, None
        try:
            self._observe()
        except BaseException:
            if self._snapshot is not None:
                self._snapshot.close()
            try:
                self._local.close()
            except Exception:
                pass
            self._closed = True
            raise

    def _observe(self):
        self._context.revalidate()
        self._local.capture()
        invocation = _inspect_generation_null(self._context)
        self._probe_stage = invocation
        run = invocation.__enter__()
        snapshot, history, configured = _read_inventory(run, retain=True)
        run._imported = True
        self._snapshot = snapshot
        self._index = snapshot.index_bytes
        self._pages = tuple((page_hash, page_bytes)
                            for page_hash, page_bytes, _name, _identity_value, _fd in snapshot.pages)
        self._history, self._configured = history, configured
        index = _json(self._index, MAX_INDEX)
        self._local.check_database_epoch(index["control_epoch"], configured)
        local_presence = self._local.present
        self._presence = _history_presence(self._history, self._configured, local_presence)

    def revalidate(self):
        _require(not self._closed, "PAPER_OPS_CLOSED")
        self._context.revalidate()
        self._local.revalidate()
        _require(self._probe_stage is not None and self._snapshot is not None,
                 "PAPER_OPS_HISTORY_UNKNOWN")
        self._probe_stage._revalidate()
        self._snapshot.revalidate(self._probe_stage._probe_output_fd)
        invocation = _inspect_generation_null(self._context)
        try:
            run = invocation.__enter__()
            index, pages, history, configured = _read_inventory(run)
            run._imported = True
        finally:
            if not invocation._closed:
                invocation.close()
        _require(index == self._index and pages == self._pages and history == self._history
                 and configured == self._configured, "PAPER_OPS_HISTORY_DRIFT")
        self._local.check_database_epoch(_json(index, MAX_INDEX)["control_epoch"], configured)
        self._context.revalidate()
        self._local.revalidate()
        return None

    def _consume_presence(self, token):
        _require(token is _READER_CONSUMER and not self._closed, "PAPER_OPS_PRIVATE")
        self.revalidate()
        return self._presence

    def close(self):
        if not self._closed:
            self.revalidate()
            self._probe_stage.close()
            self._snapshot.close()
            self._local.close()
            self._closed = True
            self._context._inventories.discard(self)

    def __enter__(self):
        self.revalidate()
        return self

    def __exit__(self, *_exc):
        self.close()


_HISTORY_TOKEN = object()
_HISTORY_PRESENT = object()
_HISTORY_EMPTY = object()
_READER_CONSUMER = object()


class _InventorySnapshot:
    """Exact retained index/page files and their original directory names."""
    __slots__ = ("index_bytes", "index_name", "index_identity", "index_fd", "pages", "closed")

    def __init__(self, index_bytes, index_name, index_identity, index_fd, pages):
        self.index_bytes, self.index_name = index_bytes, index_name
        self.index_identity, self.index_fd = index_identity, index_fd
        self.pages, self.closed = tuple(pages), False

    def revalidate(self, output_fd):
        _require(not self.closed, "PAPER_OPS_CLOSED")
        _dir(output_fd)
        retained = [(self.index_fd, self.index_name, self.index_identity, self.index_bytes)]
        retained.extend((fd, name, identity, data) for _digest_value, data, name, identity, fd
                        in self.pages)
        for fd, name, identity, data in retained:
            _file(fd, 0o400)
            _require(_identity(os.fstat(fd)) == identity
                     and os.pread(fd, len(data) + 1, 0) == data
                     and _identity(os.stat(name, dir_fd=output_fd,
                                           follow_symlinks=False)) == identity,
                     "PAPER_OPS_HISTORY_DRIFT")
        expected_names = {self.index_name, *(page[2] for page in self.pages)}
        _require({entry.name for entry in os.scandir(output_fd)} == expected_names,
                 "PAPER_OPS_HISTORY_DRIFT")

    def close(self):
        if not self.closed:
            self.closed = True
            for fd in (self.index_fd, *(page[4] for page in self.pages)):
                try:
                    os.close(fd)
                except OSError:
                    pass


def _history_presence(db_history, configured, local_presence):
    _require(type(db_history) is bool and type(configured) is bool
             and type(local_presence) is bool, "PAPER_OPS_HISTORY_UNKNOWN")
    return _HISTORY_PRESENT if (db_history or configured or local_presence) else _HISTORY_EMPTY


def _validate_floor_epoch(floor_value, control_epoch, configured):
    _require(type(configured) is bool, "PAPER_OPS_HISTORY_UNKNOWN")
    if configured:
        _require(type(control_epoch) is int and floor_value is not None
                 and floor_value == control_epoch, "PAPER_OPS_HISTORY_UNKNOWN")
    elif floor_value is not None:
        _require((control_epoch is None and floor_value == 0)
                 or (type(control_epoch) is int and floor_value == control_epoch),
                 "PAPER_OPS_HISTORY_UNKNOWN")


def _open_existing_journal(context):
    """Open already named local history without creating a Paper state directory."""
    return journal.open_journal(context.install_root, context.lock_fd, bootstrap=False)


def _validate_begin_custody(custody):
    """Consume the checked sequence-zero BEGIN binding and its real INTENT stage."""
    _require(type(custody) is journal.ActionCustody, "PAPER_OPS_ACTION")
    operation = custody._operation
    root = operation._intent
    action = journal._validate_action(custody._binding_bytes, root, 0, None)
    _require(action == custody._action, "PAPER_OPS_ACTION")
    stage = operation._validate_action_stage(action)
    last = operation._last
    _require(type(stage) is dict and stage == custody._stage
             and last is not None and last[0] == 0
             and last[1] == action["stage_sha256"] and last[2] == "INTENT"
             and last[3] == stage and stage["sequence"] == 0
             and stage["stage"] == "INTENT"
             and stage["control_epoch"] == root["expected_epoch"],
             "PAPER_OPS_ACTION")
    purpose = root["purpose"]
    expected_scope = "PAPER_STOP" if purpose == "STOP" else "COMBINED_BACKUP"
    expected_key = root["generation"] + ":begin"
    _require(operation._bound is True and purpose in {"STOP", "BACKUP"}
             and action["verb"] == "begin" and action["sequence"] == 0
             and action["stage_sha256"] is not None
             and action["artifact_sha256"] is None
             and root["account_id"] is not None and root["artifact_sha256"] is None
             and root["scope"] == expected_scope
             and root["request_key"] == expected_key
             and action["request_key"] == expected_key
             and action["expected_epoch"] == root["expected_epoch"],
             "PAPER_OPS_ACTION")
    return action


def _begin_fence_epoch(intent):
    """Return the checked BEGIN fence derived from the immutable intent."""
    expected = intent.get("expected_epoch")
    floor = intent.get("local_epoch_floor")
    _require(type(expected) is int and type(floor) is int
             and 0 <= expected <= journal.MAX_I64 and 0 <= floor <= journal.MAX_I64,
             "PAPER_OPS_EPOCH")
    fence = max(expected, floor) + 1
    _require(fence <= journal.MAX_I64, "PAPER_OPS_EPOCH")
    return fence


def _current_boot_id():
    fd = None
    try:
        fd = os.open("/proc/sys/kernel/random/boot_id",
                     os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        raw = os.read(fd, 38)
        _require(_canonical_uuid_lf(raw), "PAPER_OPS_BOOT")
        return raw[:-1].decode("ascii")
    except PaperOperationsError:
        raise
    except Exception:
        _fail("PAPER_OPS_BOOT")
    finally:
        if fd is not None:
            os.close(fd)


def _stage_observation():
    try:
        values = {
            "realtime_ns": time.time_ns(),
            "monotonic_ns": time.monotonic_ns(),
            "boottime_ns": time.clock_gettime_ns(time.CLOCK_BOOTTIME),
        }
    except Exception:
        _fail("PAPER_OPS_CLOCK")
    _require(all(type(value) is int and 0 <= value <= journal.MAX_I64
                 for value in values.values()), "PAPER_OPS_CLOCK")
    return values


def _validate_fence_begin_action(operation, action_custody):
    """Reopen original R/A/INTENT without treating a later stage as authority."""
    _require(type(action_custody) is journal.ActionCustody and not action_custody._closed,
             "PAPER_OPS_ACTION")
    action_custody.revalidate()
    operation = action_custody._operation
    operation.revalidate()
    root = operation._intent
    action = journal._validate_action(action_custody._binding_bytes, root, 0, None)
    _require(action == action_custody._action and operation._bound is True
             and root["purpose"] in {"STOP", "BACKUP"}
             and root["account_id"] is not None
             and root["artifact_sha256"] is None
             and root["request_key"] == operation.generation + ":begin"
             and root["release_manifest_sha256"] is not None
             and action["verb"] == "begin" and action["sequence"] == 0
             and action["request_key"] == root["request_key"]
             and action["expected_epoch"] == root["expected_epoch"]
             and action["artifact_sha256"] is None
             and action["stage_sha256"] is not None,
             "PAPER_OPS_ACTION")
    if root["purpose"] == "STOP":
        _require(root["scope"] == "PAPER_STOP", "PAPER_OPS_ACTION")
    else:
        _require(root["scope"] == "COMBINED_BACKUP", "PAPER_OPS_ACTION")
    intent_bytes = journal._read_named(operation._fd, "00000000.json", 0o400,
                                       journal.MAX_SMALL)
    intent_stage = journal._validate_stage(intent_bytes, root, operation.intent_sha256,
                                           0, None)
    _require(journal._sha(intent_bytes) == action["stage_sha256"]
             and action_custody._stage == intent_stage
             and intent_stage["stage"] == "INTENT"
             and intent_stage["sequence"] == 0
             and intent_stage["control_epoch"] == root["expected_epoch"]
             and intent_stage["artifact_sha256"] is None
             and intent_stage["event_id"] is None
             and intent_stage["error_code"] == "NONE",
             "PAPER_OPS_ACTION")
    head = operation._last
    _require(head is not None, "PAPER_OPS_STAGE")
    return action, root, intent_bytes, intent_stage, head


def _validate_begin_result_artifact(data, invocation, operation, fence):
    result = invocation._result
    _require(type(result) is dict and tuple(result) == RESULT_FIELDS
             and result["schema"] == RESULT_SCHEMA and result["verb"] == "begin"
             and result["outcome"] in {"OK", "REPLAY"}
             and result["error_code"] == "NONE"
             and result["generation"] == operation.generation
             and type(result["event_id"]) is str
             and journal.UUID_RE.fullmatch(result["event_id"]) is not None
             and type(result["control_epoch"]) is int
             and result["control_epoch"] == fence
             and type(data) is bytes and 1 <= len(data) <= journal.MAX_SMALL
             and _digest(data) == result["artifact_sha256"],
             "PAPER_OPS_RESULT")
    value = journal._json(data, journal.MAX_SMALL)
    _require(type(value) is dict and set(value) == {
        "schema", "action", "outcome", "control_epoch", "account_id", "data"
    } and value["schema"] == PAGE_SCHEMA
             and value["action"] == "BEGIN_OPERATION"
             and type(value["outcome"]) is str
             and value["outcome"] in {"OK", "REPLAY"}
             and type(value["control_epoch"]) is int
             and value["control_epoch"] == fence
             and value["account_id"] == operation._intent["account_id"]
             and type(value["data"]) is dict and set(value["data"]) == {
                 "generation", "operation_event_id", "intent_sha256", "fenced_epoch"
             }
             and value["data"]["generation"] == operation.generation
             and value["data"]["operation_event_id"] == result["event_id"]
             and value["data"]["intent_sha256"] == operation.intent_sha256
             and type(value["data"]["fenced_epoch"]) is int
             and value["data"]["fenced_epoch"] == fence
             and result["control_epoch"] == fence,
             "PAPER_OPS_RESULT")
    return value


def _validate_begin_readback(data, request_bytes, result_bytes, operation, fence,
                             event_id):
    value = journal._json(data, journal.MAX_NATIVE_EXACT)
    _require(type(value) is dict and set(value) == {
        "schema", "kind", "request_sha256", "status", "account_id", "current_epoch",
        "current_state", "event_id", "event_epoch", "result_canonical",
        "result_sha256", "reset"
    } and value["schema"] == "owner-beta-paper-exact-readback-v1"
             and value["kind"] == "BEGIN"
             and value["request_sha256"] == journal._sha(request_bytes)
             and value["status"] == "COMMITTED"
             and value["account_id"] == operation._intent["account_id"]
             and type(value["current_epoch"]) is int and value["current_epoch"] == fence
             and value["current_state"] == "DRAINING"
             and value["event_id"] == event_id
             and type(value["event_epoch"]) is int and value["event_epoch"] == fence
             and type(value["result_canonical"]) is str
             and value["result_sha256"] == journal._sha(result_bytes)
             and value["reset"] is None,
             "PAPER_OPS_READBACK")
    try:
        _require(value["result_canonical"].encode("ascii") == result_bytes,
                 "PAPER_OPS_READBACK")
    except UnicodeError:
        _fail("PAPER_OPS_READBACK")
    return value


class _LocalMarkers:
    """Read-only local Paper markers, including retained absence and name evidence."""
    __slots__ = ("context", "tree", "root_fd", "state_fd", "ops_fd", "files", "dirs",
                 "entries", "present", "closed", "release_stage_fd", "journal_manager",
                 "journal_store", "operations", "callers_fd", "floor_value")

    def __init__(self, context):
        self.context = context
        self.tree = journal._Tree(context.install_root)
        self.root_fd = self.tree.fd
        self.state_fd = self.ops_fd = None
        self.files, self.dirs, self.entries = {}, {}, {}
        self.present, self.closed = False, False
        self.release_stage_fd = None
        self.journal_manager = self.journal_store = None
        self.operations = []
        self.callers_fd = None
        self.floor_value = None

    def capture(self):
        context = self.context
        self.tree.revalidate()
        self._capture_optional_file(self.root_fd, READER_FLOOR, 0o600, "reader_floor")
        if self.files["reader_floor"] is not None:
            value = _json(self.files["reader_floor"][3], 4096)
            _require(value == release.FLOOR_VALUE and
                     self.files["reader_floor"][3] == release.FLOOR_BYTES,
                     "PAPER_OPS_LOCAL_UNKNOWN")
            self.present = True
        try:
            self.state_fd = journal._open_child(self.root_fd, PAPER_DIR, 0o700)
        except FileNotFoundError:
            self.dirs["paper"] = None
        except Exception:
            _fail("PAPER_OPS_LOCAL_UNKNOWN")
        else:
            self.dirs["paper"] = (_identity(os.fstat(self.state_fd)), PAPER_DIR)
            names = {entry.name for entry in os.scandir(self.state_fd)}
            allowed = {"installation-id", "epoch-floor", "operations", "callers"}
            _require(names <= allowed and
                     {"installation-id", "epoch-floor", "operations"} <= names,
                     "PAPER_OPS_LOCAL_UNKNOWN")
            self.entries["paper_root"] = names
            self._capture_optional_file(self.state_fd, "installation-id", 0o600, "installation_id")
            self._capture_optional_file(self.state_fd, "epoch-floor", 0o600, "epoch_floor")
            if "installation-id" in names:
                self.present = True
                _require("epoch-floor" in names and "operations" in names, "PAPER_OPS_LOCAL_UNKNOWN")
                raw = self.files["installation_id"][3]
                _require(_canonical_uuid_lf(raw), "PAPER_OPS_LOCAL_UNKNOWN")
                floor = self.files["epoch_floor"][3]
                _require(re.fullmatch(rb"(?:0|[1-9][0-9]{0,18})\n", floor) is not None
                         and int(floor[:-1]) <= journal.MAX_I64, "PAPER_OPS_LOCAL_UNKNOWN")
                self.floor_value = int(floor[:-1])
            if "operations" in names:
                self.ops_fd = journal._open_child(self.state_fd, "operations", 0o700)
                self.dirs["operations"] = (_identity(os.fstat(self.ops_fd)), "operations")
                operation_entries = {}
                for entry in os.scandir(self.ops_fd):
                    info = os.stat(entry.name, dir_fd=self.ops_fd, follow_symlinks=False)
                    _require(stat.S_ISDIR(info.st_mode) and info.st_uid == 0 and info.st_gid == 0
                             and stat.S_IMODE(info.st_mode) == 0o700, "PAPER_OPS_LOCAL_UNKNOWN")
                    operation_entries[entry.name] = _identity(info)
                self.entries["operations"] = operation_entries
                self.present = self.present or bool(operation_entries)
                if "installation-id" in names:
                    self.journal_manager = _open_existing_journal(context)
                    self.journal_store = self.journal_manager.__enter__()
                    for generation in sorted(operation_entries):
                        operation = self.journal_store.open(generation)
                        operation.revalidate()
                        self.operations.append(operation)
            if "callers" in names:
                self.callers_fd = journal._open_child(self.state_fd, "callers", 0o700)
                self.dirs["callers"] = (_identity(os.fstat(self.callers_fd)), "callers")
                caller_names = {entry.name for entry in os.scandir(self.callers_fd)}
                _require(all(re.fullmatch(r"[0-9a-f]{64}\.json", name) for name in caller_names),
                         "PAPER_OPS_LOCAL_UNKNOWN")
                self.entries["callers"] = caller_names
                self.present = self.present or bool(caller_names)
        # The immutable process config is local configuration evidence. The
        # VerifiedPaperRelease handle independently pins it on the live path.
        stage_path = context.release_path + "/deploy/owner-beta-paper"
        try:
            with daily.install.root_directory(stage_path) as stage_fd:
                self.release_stage_fd = os.dup(stage_fd)
                _dir(self.release_stage_fd, 0o750, uid=0, gid=10001)
                self._capture_optional_file(self.release_stage_fd, "process.json", 0o440,
                                            "process", gid=10001)
                self._capture_optional_file(self.release_stage_fd, "config.json", 0o440,
                                            "config", gid=10001)
        except Exception:
            # A missing capability directory is UNKNOWN rather than empty.
            if not os.path.isdir(stage_path):
                _fail("PAPER_OPS_LOCAL_UNKNOWN")
            raise
        self.present = self.present or any(self.files.get(key) is not None for key in ("process", "config"))
        self.tree.revalidate()

    def _capture_optional_file(self, parent, name, mode, key, *, uid=0, gid=0):
        try:
            fd, data, identity = _open_file(parent, name, mode, maximum=128 * 1024,
                                            uid=uid, gid=gid)
        except PaperOperationsError as error:
            try:
                os.stat(name, dir_fd=parent, follow_symlinks=False)
            except FileNotFoundError:
                self.files[key] = None
                return
            raise error
        self.files[key] = (parent, name, fd, data, identity)

    def revalidate(self):
        _require(not self.closed, "PAPER_OPS_CLOSED")
        self.tree.revalidate()
        for key, item in self.files.items():
            if item is None:
                parent = self.root_fd if key == "reader_floor" else (
                    self.state_fd if key in {"installation_id", "epoch_floor"} else
                    self.release_stage_fd if key in {"process", "config"} else None)
                if parent is not None:
                    name = READER_FLOOR if key == "reader_floor" else key.replace("_", "-")
                    try:
                        os.stat(name, dir_fd=parent, follow_symlinks=False)
                    except FileNotFoundError:
                        pass
                    else:
                        _fail("PAPER_OPS_LOCAL_CHANGED")
                continue
            parent, name, fd, data, identity = item
            _require(_identity(os.fstat(fd)) == identity and
                     os.pread(fd, len(data) + 1, 0) == data and
                     _identity(os.stat(name, dir_fd=parent, follow_symlinks=False)) == identity,
                     "PAPER_OPS_LOCAL_CHANGED")
        if self.state_fd is None:
            try:
                os.stat(PAPER_DIR, dir_fd=self.root_fd, follow_symlinks=False)
            except FileNotFoundError:
                pass
            else:
                _fail("PAPER_OPS_LOCAL_CHANGED")
        else:
            journal._check_named_dir(self.root_fd, PAPER_DIR, self.state_fd)
            _require({entry.name for entry in os.scandir(self.state_fd)} ==
                     self.entries.get("paper_root", set()), "PAPER_OPS_LOCAL_CHANGED")
        if self.ops_fd is not None:
            journal._check_named_dir(self.state_fd, "operations", self.ops_fd)
            current = {entry.name: _identity(os.stat(entry.name, dir_fd=self.ops_fd,
                         follow_symlinks=False)) for entry in os.scandir(self.ops_fd)}
            _require(current == self.entries.get("operations", {}), "PAPER_OPS_LOCAL_CHANGED")
        if getattr(self, "callers_fd", None) is not None:
            journal._check_named_dir(self.state_fd, "callers", self.callers_fd)
            current = {entry.name for entry in os.scandir(self.callers_fd)}
            _require(current == self.entries.get("callers", set()), "PAPER_OPS_LOCAL_CHANGED")
        if self.journal_store is not None:
            self.journal_store.revalidate()
            _require(len(self.operations) == len(self.entries.get("operations", {})),
                     "PAPER_OPS_LOCAL_CHANGED")
            for operation in self.operations:
                operation.revalidate()
        if self.release_stage_fd is not None:
            info = _dir(self.release_stage_fd, 0o750, uid=0, gid=10001)
            _require((info.st_dev, info.st_ino, info.st_uid, info.st_gid, info.st_mode) ==
                     self.context.verified._identities[-1], "PAPER_OPS_LOCAL_CHANGED")
        return None

    def check_database_epoch(self, control_epoch, configured):
        _validate_floor_epoch(self.floor_value, control_epoch, configured)

    def close(self):
        if self.closed:
            return
        failures = []
        for item in self.files.values():
            if item is not None:
                try:
                    os.close(item[2])
                except OSError as error:
                    failures.append(error)
        for operation in self.operations:
            try:
                operation.close()
            except Exception as error:
                failures.append(error)
        if self.journal_store is not None:
            try:
                self.journal_manager.__exit__(None, None, None)
            except Exception as error:
                failures.append(error)
            self.journal_store = None
        for fd in (self.release_stage_fd, self.ops_fd, self.callers_fd, self.state_fd):
            if fd is not None:
                try:
                    os.close(fd)
                except OSError as error:
                    failures.append(error)
        try:
            self.tree.close()
        except Exception as error:
            failures.append(error)
        self.closed = True
        if failures:
            raise failures[0]


def _inspect_generation_null(context):
    temporary = _Stage("owner-beta-paper-inspect-")
    invocation = None
    output_fd = None
    try:
        generation = str(uuid.uuid4())
        installation = str(uuid.uuid4())
        intent = {
            "schema": "owner-beta-paper-operation-intent-v1",
            "installation_id": installation, "generation": generation,
            "account_id": None, "predecessor_generation": None,
            "request_key": "inspect-" + generation, "purpose": "INSPECT",
            "scope": "PAPER_STOP", "expected_epoch": 0, "local_epoch_floor": 0,
            "release_manifest_sha256": context.manifest_sha256,
            "artifact_sha256": None, "resume_target_id": None,
        }
        _write_file(temporary.fd, "request.json", _canonical(intent), 0o400)
        _write_file(temporary.fd, "database.json", _canonical(context.database), 0o400)
        os.mkdir("output", 0o700, dir_fd=temporary.fd)
        output_fd = os.open("output", journal.DIR_FLAGS, dir_fd=temporary.fd)
        _dir(output_fd)
        temporary.revalidate()
        invocation = _NativeInvocation(_INVOCATION_TOKEN, context, "inspect", None)
        invocation._prepare_inspect(temporary, output_fd)
        context._invocations.add(invocation)
        os.close(output_fd)
        output_fd = None
        return invocation
    except BaseException:
        if output_fd is not None:
            os.close(output_fd)
        if invocation is not None:
            invocation.close()
        else:
            temporary.close(remove=True)
        raise


def _read_inventory(invocation, *, retain=False):
    _require(invocation._result is not None and invocation._result["outcome"] == "OK"
             and invocation._result["error_code"] == "NONE", "PAPER_OPS_HISTORY_UNKNOWN")
    output_fd = invocation._probe_output_fd
    index_hash = invocation._result["artifact_sha256"]
    _require(type(index_hash) is str and journal.HASH_RE.fullmatch(index_hash),
             "PAPER_OPS_HISTORY_UNKNOWN")
    index_name = index_hash[7:] + ".json"
    index_fd, index_bytes, index_id = _open_file(output_fd, index_name, 0o400, maximum=MAX_INDEX)
    page_files = []
    try:
        _require(_digest(index_bytes) == index_hash, "PAPER_OPS_HISTORY_UNKNOWN")
        index = _json(index_bytes, MAX_INDEX)
        _require(type(index) is dict and set(index) == {
            "schema", "generation", "state", "control_epoch", "operation_event_id",
            "root_sha256", "pages"
        } and index["schema"] == INDEX_SCHEMA and index["generation"] is None
          and (index["state"] is None or (type(index["state"]) is str and index["state"] in {
              "OFF", "ENABLED", "DRAINING", "RESTORED_OFF", "BLOCKED"
          }))
          and (index["control_epoch"] is None or
               (type(index["control_epoch"]) is int and
                0 <= index["control_epoch"] <= journal.MAX_I64))
          and index["operation_event_id"] is None
          and type(index["root_sha256"]) is str
          and journal.HASH_RE.fullmatch(index["root_sha256"])
          and type(index["pages"]) is list and len(index["pages"]) == 1,
          "PAPER_OPS_HISTORY_UNKNOWN")
        _require(invocation._result["generation"] is None
                 and invocation._result["event_id"] == index["operation_event_id"]
                 and invocation._result["control_epoch"] == index["control_epoch"],
                 "PAPER_OPS_HISTORY_UNKNOWN")
        targets = []
        seen = set()
        for page_hash in index["pages"]:
            _require(type(page_hash) is str and journal.HASH_RE.fullmatch(page_hash)
                     and page_hash not in seen, "PAPER_OPS_HISTORY_UNKNOWN")
            seen.add(page_hash)
            name = page_hash[7:] + ".json"
            page_fd, page_bytes, page_id = _open_file(output_fd, name, 0o400, maximum=MAX_PAGE)
            page_files.append((page_hash, page_bytes, name, page_id, page_fd))
            _require(_digest(page_bytes) == page_hash, "PAPER_OPS_HISTORY_UNKNOWN")
            page = _json(page_bytes, MAX_PAGE)
            _validate_inspect_page(page)
            targets.extend(page["data"]["targets"])
        _require(_digest(_canonical(index["pages"])) == index["root_sha256"],
                 "PAPER_OPS_HISTORY_UNKNOWN")
        observed_names = {entry.name for entry in os.scandir(output_fd)}
        expected_names = {index_name, *(item[2] for item in page_files)}
        _require(observed_names == expected_names, "PAPER_OPS_HISTORY_UNKNOWN")
        for previous, current in zip(targets, targets[1:]):
            _require(previous["target_id"] < current["target_id"],
                     "PAPER_OPS_HISTORY_UNKNOWN")
        page = _json(page_files[0][1], MAX_PAGE)
        data = page["data"]
        _require(index["state"] == data["state"]
                 and index["control_epoch"] == data["control_epoch"]
                 and index["operation_event_id"] == data["operation_event_id"],
                 "PAPER_OPS_HISTORY_UNKNOWN")
        history, configured = data["history_present"], page["account_id"] is not None
        if retain:
            snapshot = _InventorySnapshot(index_bytes, index_name, index_id, index_fd, page_files)
            index_fd, page_files = None, []
            return snapshot, history, configured
        return (index_bytes, tuple((page_hash, raw)
                                   for page_hash, raw, _name, _identity_value, _fd in page_files),
                history, configured)
    finally:
        if index_fd is not None:
            os.close(index_fd)
        for _hash_value, _raw, _name, _identity_value, fd in page_files:
            os.close(fd)


def _validate_inspect_page(page):
    _require(type(page) is dict and set(page) == {
        "schema", "action", "outcome", "control_epoch", "account_id", "data"
    } and page["schema"] == PAGE_SCHEMA and page["action"] == "INSPECT_OPERATION"
      and page["outcome"] == "OK" and type(page["data"]) is dict
      and set(page["data"]) == {
          "generation", "state", "control_epoch", "history_present", "operation_event_id",
          "reconciliation", "targets", "next_target"
      } and page["data"]["generation"] is None
      and page["data"]["next_target"] is None
      and type(page["data"]["history_present"]) is bool
      and page["data"]["operation_event_id"] is None
      and page["data"]["reconciliation"] is None
      and page["data"]["targets"] == []
      and page["data"]["control_epoch"] == page["control_epoch"],
      "PAPER_OPS_HISTORY_UNKNOWN")
    account_id = page["account_id"]
    _require(account_id is None or
             (type(account_id) is str and journal.UUID_RE.fullmatch(account_id)),
             "PAPER_OPS_HISTORY_UNKNOWN")
    control_epoch = page["control_epoch"]
    _require(control_epoch is None or
             (type(control_epoch) is int and 0 <= control_epoch <= journal.MAX_I64),
             "PAPER_OPS_HISTORY_UNKNOWN")
    if page["data"]["state"] is not None:
        _require(type(page["data"]["state"]) is str and
                 page["data"]["state"] in {"OFF", "ENABLED", "DRAINING", "RESTORED_OFF", "BLOCKED"},
                 "PAPER_OPS_HISTORY_UNKNOWN")
    if account_id is None:
        _require(page["data"]["state"] is None and control_epoch is None
                 and page["data"]["history_present"] is False,
                 "PAPER_OPS_HISTORY_UNKNOWN")
    else:
        _require(page["data"]["state"] is not None and type(control_epoch) is int,
                 "PAPER_OPS_HISTORY_UNKNOWN")
