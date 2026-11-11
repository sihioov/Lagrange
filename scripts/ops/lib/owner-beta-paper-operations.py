#!/usr/bin/env python3
"""Retained Paper transport, live fencing and source-owned writer drain.

The controller may seal FENCED only after an independently opened exact readback
proves the original BEGIN result and current DRAINING state. The separate drain
observes launch sources, kernel generations and PostgreSQL before STOPPED. Only
the retained live handle can consume RECONCILE/OFF. Lifecycle actions use their separate retained host owner.
Callers must retain the returned contexts and inventories.
"""
from __future__ import annotations

import hashlib
import fcntl
import datetime
import importlib.util
import json
import os
import re
import selectors
import shutil
import signal
import socket
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
LIVE_ACTIONS = frozenset({"begin", "reconcile", "off", "restore-off", "configure", "enable", "recover"})
RESULT_SCHEMAS = {"begin": RESULT_SCHEMA, "reconcile": RESULT_SCHEMA,
                  "off": RESULT_SCHEMA, "restore-off": RESULT_SCHEMA, "inspect": RESULT_SCHEMA,
                  "readback": QUERY_RESULT_SCHEMA, "configure": RESULT_SCHEMA,
                  "enable": RESULT_SCHEMA, "recover": RESULT_SCHEMA, "observe-reset": QUERY_RESULT_SCHEMA}
_FENCED_CALLS = weakref.WeakKeyDictionary()
_STOP_ATTEMPTS = weakref.WeakKeyDictionary()


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


def _validate_runtime_policy(config, host, verb, image_env, network_id, client_env, *, archive_restore_begin=False):
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
             and (host.get("CapAdd") == ["DAC_READ_SEARCH"] if archive_restore_begin
                  else host.get("CapAdd") in (None, [])) and host.get("CapDrop") == ["ALL"]
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
            # Own mountpoint directories are created before snapshot(). Their
            # creation changes st_nlink, not the directory generation. Frozen
            # snapshot names/metadata below still reject subsequent additions.
            self.identity = journal._dir_identity(_dir(self.fd))
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
        _require(not self.closed and journal._dir_identity(_dir(self.fd)) == self.identity,
                 "PAPER_OPS_STAGE")
        self.tree.revalidate()
        _require(journal._dir_identity(os.stat(self.name, dir_fd=self.tree.fd,
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
                 "_release_tree", "_lifecycle_stage_root", "_invocations", "_inventories", "_input_stages",
                 "_retained_stages", "_begin_result_fds", "_initializing", "__weakref__")

    def __init__(self, token, context, release_lock_fd, *, live, deadline=None):
        if token is not _CONTEXT_TOKEN:
            _fail("PAPER_OPS_PRIVATE")
        self._initializing = True
        self._lifecycle_stage_root = None
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
        if deadline is not None:
            self.docker = _StopDocker(self.docker, deadline)
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
            _require(verb in {"begin", "reconcile", "off"} and type(action_custody) is journal.ActionCustody,
                     "PAPER_OPS_VERB")
            action_custody.revalidate()
            _require(action_custody._action["verb"] == verb and
                     action_custody._operation._intent["purpose"] in {"STOP", "BACKUP"},
                     "PAPER_OPS_ACTION")
            if verb == "begin":
                _validate_begin_custody(action_custody)
            else:
                owners = [attempt.consumer for attempt in _STOP_ATTEMPTS.get(self, {}).values()
                          if getattr(attempt, "consumer", None) is not None
                          and attempt.consumer.action is action_custody]
                _require(len(owners) == 1 and owners[0].call is None,
                         "PAPER_OPS_ACTION")
                owners[0].attempt.check(stopped=True)
            invocation = _NativeInvocation(_INVOCATION_TOKEN, self, verb, action_custody)
        else:
            _require(verb == "readback" and type(action_custody) is journal.Custody
                     and action_custody._readonly, "PAPER_OPS_VERB")
            action_custody._check()
            invocation = _NativeInvocation(_INVOCATION_TOKEN, self, verb, action_custody)
        self._invocations.add(invocation)
        return invocation

    def _invoke_lifecycle(self, owner, verb, custody):
        """Only a registered action-specific lifecycle owner can enter here."""
        module = sys.modules.get("owner_beta_paper_lifecycle")
        _require(module is not None, "PAPER_OPS_PRIVATE")
        module._assert_call(owner, self, verb, custody)
        self.revalidate()
        invocation = _NativeInvocation(_INVOCATION_TOKEN, self, verb, custody)
        invocation._lifecycle_owner = owner
        owner.call = invocation
        self._invocations.add(invocation)
        return invocation

    def _invoke_archive(self, package):
        """Read the original committed OFF only on its live owned archive clone."""
        module = sys.modules.get("owner_beta_paper_backup")
        _require(module is not None and type(package) is getattr(module, "_ArchivePackage", None)
                 and package in module._PACKAGES and package.owner.context is self
                 and package.owner.package is package and package.call is None
                 and self._mode == "live", "PAPER_OPS_PRIVATE")
        package.revalidate()
        custody = package.custody
        _require(type(custody) is journal.Custody and custody._readonly,
                 "PAPER_OPS_ACTION")
        custody._check()
        invocation = _NativeInvocation(_INVOCATION_TOKEN, self, "readback", custody)
        invocation._archive_owner = package
        self._invocations.add(invocation)
        return invocation

    def _invoke_restore(self, destination, verb, custody):
        module = sys.modules.get("owner_beta_paper_backup")
        _require(module is not None and type(destination) is module._DestinationRestore
                 and destination.package in module._PACKAGES
                 and destination.package.destination is destination
                 and destination.package.owner.context is self
                 and destination.package.call is None and not destination.closed
                 and verb in {"begin", "reconcile", "restore-off", "readback"}, "PAPER_OPS_PRIVATE")
        destination.revalidate()
        if verb == "readback":
            _require(type(custody) is journal.Custody and custody._readonly
                     and custody._intent == destination.operation._intent_bytes, "PAPER_OPS_ACTION")
            custody._check()
        else:
            _require(type(custody) is journal.ActionCustody
                     and custody._operation is destination.operation
                     and custody._action["verb"] == verb, "PAPER_OPS_ACTION")
            custody.revalidate()
        invocation = _NativeInvocation(_INVOCATION_TOKEN, self, verb, custody)
        invocation._archive_owner = destination.package
        invocation._restore_owner = destination
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

    def drain(self, invocation):
        """Observe and stop a live FENCED generation; never imports stop authority.

        Every explicit attempt has one 90s deadline. Failed observations remain
        protected originals under the operation; they never append STOPPED.
        """
        _require(type(self) is _PaperContext and type(invocation) is _NativeInvocation
                 and invocation._context is self
                 and invocation in _FENCED_CALLS.get(self, {}),
                 "PAPER_OPS_BEGIN_UNSETTLED")
        prior = _STOP_ATTEMPTS.get(self, {}).get(invocation)
        if prior is not None and prior.handle is not None:
            prior.handle.revalidate()
            return prior.handle
        if prior is not None:
            prior.close()
        attempt = _StopAttempt(_STOP_TOKEN, self, invocation)
        _STOP_ATTEMPTS.setdefault(self, {})[invocation] = attempt
        return attempt.run()

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
        for attempt in _STOP_ATTEMPTS.get(self, {}).values():
            attempt.close()
        _STOP_ATTEMPTS.pop(self, None)
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
    """Open installed BEGIN transport and its handle-bound maintenance continuation."""
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
                 "_archive_owner", "_restore_owner", "_lifecycle_owner", "__weakref__")

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
        self._archive_owner = self._restore_owner = self._lifecycle_owner = None

    def _new_stage(self, prefix):
        if self._lifecycle_owner is not None:
            stage = _Stage(prefix, root=self._lifecycle_owner.stage_root)
        elif self._archive_owner is not None:
            stage = _Stage(prefix, root=str(self._archive_owner.owner.root))
        elif getattr(self._context, "_lifecycle_stage_root", None) is not None:
            stage = _Stage(prefix, root=self._context._lifecycle_stage_root)
        else:
            stage = _Stage(prefix)
        self._stages.append(stage)
        return stage

    def _prepare(self):
        context = self._context
        context.revalidate()
        if self._lifecycle_owner is not None:
            return self._lifecycle_owner.prepare(self)
        if self._archive_owner is not None:
            return self._prepare_archive()
        if self.verb == "inspect":
            _require(self._probe_output_fd is not None, "PAPER_OPS_OUTPUT")
            for stage in self._stages:
                stage.revalidate()
            return
        if self.verb in LIVE_ACTIONS:
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
        if self.verb in LIVE_ACTIONS:
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
                expected_floor = str(operation._intent["local_epoch_floor"] if self.verb == "begin"
                                     else _begin_fence_epoch(operation._intent)).encode("ascii") + b"\n"
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
        if self.verb in LIVE_ACTIONS:
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
        if self.verb not in LIVE_ACTIONS or stage_new:
            stage.snapshot()
        secret_stage.snapshot()
        context.revalidate()

    def _prepare_archive(self):
        package, context = self._archive_owner, self._context
        package.revalidate()
        destination = self._restore_owner
        _require(package.call is self and ((destination is None and self.verb == "readback"
                 and self.custody is package.custody) or (destination is package.destination
                 and destination.active_custody is self.custody)), "PAPER_OPS_PRIVATE")
        if destination is not None:
            destination.revalidate()
            if self.verb == "readback":
                self.custody._check()
            else:
                self.custody.revalidate()
        stage = self._new_stage("owner-beta-paper-archive-")
        if self.verb == "readback":
            self.custody.stage_readback(stage.fd)
        else:
            self.custody.stage_inputs(stage.fd)
            self._persistent_stage = stage
            destination.stages.append(stage)
        _write_file(stage.fd, "database.json", _canonical(package.database), 0o400)
        _require(context.verified is not None, "PAPER_OPS_RELEASE")
        context.verified.stage_native_inputs(stage.fd)
        for alias in ("operation", "paper-state", "archive"):
            os.mkdir(alias, 0o700, dir_fd=stage.fd)
        _ensure_output_mountpoint(stage.fd)
        operation = package.owner.operation if destination is None else destination.operation
        install_root = context.install_root if destination is None else str(destination.path)
        install_fd = context.tree.fd if destination is None else destination.tree.fd
        operation_path = _path_from_fd(operation.borrow_directory_fd())
        _require(operation_path == install_root + "/" + PAPER_DIR + "/operations/" +
                 operation.generation, "PAPER_OPS_OPERATION")
        state_fd = os.open(PAPER_DIR, journal.DIR_FLAGS, dir_fd=install_fd)
        self._paper_state_fd = state_fd
        self._paper_state_identity = _identity(_dir(state_fd))
        for name, expected in (("installation-id", operation._intent["installation_id"]),
                               ("epoch-floor", str(package.owner._custody.floor if destination is None else destination.store.epoch_floor))):
            fd, data, identity = _open_file(state_fd, name, 0o600, maximum=64)
            self._paper_state_files += ((fd, name, identity, data),)
            _require(data == expected.encode("ascii") + b"\n", "PAPER_OPS_PAPER_STATE")
        secret = self._new_stage("owner-beta-paper-clone-secret-")
        _write_file(secret.fd, "db_migration_owner_password", package.password, 0o400)
        output_fd = self.custody.borrow_output_fd()
        if self.verb in LIVE_ACTIONS:
            try:
                os.stat("dispatch.json", dir_fd=output_fd, follow_symlinks=False)
            except FileNotFoundError:
                pass
            else:
                fd, data, identity = _open_file(output_fd, "dispatch.json", 0o400,
                                                maximum=journal.MAX_NATIVE_INDEX)
                self._preexisting_dispatch_fd = fd
                self._preexisting_dispatch_identity = identity
                self._preexisting_dispatch_bytes = data
                self._preexisting_dispatch_parent = output_fd
        self._mounts = ((stage.path, PRIVATE, True),
            (operation_path, OPERATION_ALIAS, True),
            (install_root + "/" + PAPER_DIR, PAPER_STATE_ALIAS, True),
            (str(package.path), PRIVATE + "/archive", True),
            (str(package.guest.socket), "/run/postgresql", True),
            (secret.path, "/run/secrets", True),
            (_path_from_fd(output_fd), OUTPUT_ALIAS, False))
        os.fsync(stage.fd)
        # Only the empty mountpoint belongs to this stage. Archive tree owners
        # and bytes are checked by the retained codec/package and native intake.
        stage.snapshot(mutable_children=("archive",))
        secret.snapshot()
        package.revalidate()

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
        archive_restore_begin = self._archive_restore_begin()
        self._nonce = uuid.uuid4().hex
        self._container_name = "owner-beta-paper-ops-" + self._nonce
        args = ["create", "--name", self._container_name,
                "--label", CONTAINER_LABEL + "=" + self._nonce,
                "--user", "0:0", "--workdir", "/", "--entrypoint", NATIVE,
                "--restart", "no", "--read-only", "--cap-drop", "ALL",
                "--security-opt", "no-new-privileges", "--log-driver", "none",
                "--cpus", "1", "--memory", "2147483648", "--network",
                "none" if self._archive_owner is not None else context.network_id]
        if archive_restore_begin:
            args.extend(("--cap-add", "DAC_READ_SEARCH"))
        args.extend(_native_env_args())
        if self._lifecycle_owner is not None:
            self._lifecycle_owner.revalidate()
            for group in self._lifecycle_owner.groups:
                args.extend(("--group-add", group))
        if self._archive_owner is not None:
            # Preserve the codec's frozen class groups; group membership does
            # not grant access to 0600 files or 0700 directories.
            for group in (10001, 10002, 10003):
                args.extend(("--group-add", str(group)))
        for source, target, readonly in self._mounts:
            _require(source.startswith("/") and target.startswith("/") and "," not in source,
                     "PAPER_OPS_MOUNT")
            mount = "type=bind,source=" + source + ",target=" + target
            if readonly:
                mount += ",readonly"
            args.extend(("--mount", mount))
        args.extend((context.image, self.verb))
        return args

    def _archive_restore_begin(self):
        destination = self._restore_owner
        if destination is None or self.verb != "begin":
            return False
        package, context = self._archive_owner, self._context
        module = sys.modules.get("owner_beta_paper_backup")
        _require(module is not None and type(package) is module._ArchivePackage
                 and package in module._PACKAGES
                 and type(destination) is module._DestinationRestore
                 and package.destination is destination and destination.package is package
                 and package.owner.context is context and package.call is self
                 and self in context._invocations and not destination.closed
                 and destination.active_custody is self.custody
                 and type(self.custody) is journal.ActionCustody
                 and self.custody._operation is destination.operation
                 and self.custody._action.get("verb") == "begin"
                 and destination.operation._intent.get("purpose") == "RESTORE",
                 "PAPER_OPS_PRIVATE")
        return True

    def _inspect_and_validate(self, *, started):
        context = self._context
        context.revalidate()
        if self._archive_owner is not None:
            self._archive_owner.revalidate()
            if self._restore_owner is not None:
                self._restore_owner.revalidate()
                if self.verb == "readback":
                    self.custody._check()
                else:
                    self.custody.revalidate()
        value = context.docker.object("container", self._cid)
        config, host, state = value.get("Config") or {}, value.get("HostConfig") or {}, value.get("State") or {}
        _validate_container_identity(value, self._cid, self._nonce, self._created,
                                     context.image, context.commit)
        image_config = context.docker.object("image", context.image).get("Config") or {}
        image_env = _image_environment(image_config, context.commit)
        _require(image_env == context.image_environment, "PAPER_OPS_IMAGE_ENV")
        _validate_runtime_policy(config, host, self.verb, image_env,
                                 "none" if self._archive_owner is not None else context.network_id,
                                 context.docker.environment, archive_restore_begin=self._archive_restore_begin())
        _validate_mount_records(value.get("Mounts") or [], self._mounts)
        if self._lifecycle_owner is not None:
            self._lifecycle_owner.revalidate()
            _require((host.get("GroupAdd") or []) == list(self._lifecycle_owner.groups),
                     "PAPER_OPS_CONTAINER_POLICY")
        if self._archive_owner is not None:
            _require(host.get("GroupAdd") == ["10001", "10002", "10003"], "PAPER_OPS_CONTAINER_POLICY")
        networks = value.get("NetworkSettings", {}).get("Networks") or {}
        ids = {item.get("NetworkID") for item in networks.values()}
        if self._archive_owner is not None:
            _require(set(networks) <= {"none"} and all(not n.get("IPAddress") for n in networks.values()),
                     "PAPER_OPS_NETWORK")
        else:
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

    def _archive_observed_container(self, value):
        """Nonrecursive observer check of THIS registered readonly helper."""
        package, context = self._archive_owner, self._context
        _require(package is not None and package.call is self and self in context._invocations
                 and ((self._restore_owner is None and self.verb == "readback" and self.custody is package.custody)
                      or (self._restore_owner is package.destination and self.custody is package.destination.active_custody))
                 and self._cid is not None and self._created is not None,
                 "PAPER_OPS_PRIVATE")
        _validate_container_identity(value, self._cid, self._nonce, self._created,
                                     context.image, context.commit)
        _validate_runtime_policy(value.get("Config") or {}, value.get("HostConfig") or {},
            self.verb, context.image_environment, "none", context.docker.environment,
            archive_restore_begin=self._archive_restore_begin())
        _validate_mount_records(value.get("Mounts") or [], self._mounts)
        _require((value.get("HostConfig") or {}).get("GroupAdd") == ["10001", "10002", "10003"],
                 "PAPER_OPS_CONTAINER_POLICY")
        networks = value.get("NetworkSettings", {}).get("Networks") or {}
        _require(set(networks) <= {"none"} and all(not n.get("IPAddress") for n in networks.values()),
                 "PAPER_OPS_NETWORK")

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
            # A failure before native input access cannot establish that an
            # already durable dispatch rolled back. Require exact readback.
            if self._preexisting_dispatch_fd is not None and self._result["outcome"] == "BLOCKED":
                self._preserve = True
                _fail("PAPER_OPS_UNRESOLVED")
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
        outer = getattr(self._context.docker, "deadline", None)
        if outer is not None:
            deadline = min(deadline, outer.end / 1_000_000_000)
        if self._archive_owner is not None:
            deadline = min(deadline, self._archive_owner.owner._custody.deadline.end / 1_000_000_000 - 20)
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
                    if self._archive_owner is not None:
                        self._archive_owner.revalidate()
                    self._context.revalidate()
                    if self._lifecycle_owner is not None:
                        self._lifecycle_owner.revalidate()
                    elif self.verb in LIVE_ACTIONS:
                        self.custody.revalidate()
                    elif self.verb == "readback":
                        self.custody._check()
                    self._revalidate_paper_state()
                    last_check = time.monotonic()
            try:
                child.wait(timeout=5 if outer is None else outer.remaining(5))
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
        _require(self.verb in LIVE_ACTIONS and self._entered,
                 "PAPER_OPS_IMPORT")
        self._revalidate()
        custody = self.custody.import_dispatch()
        try:
            self._revalidate()
            if self._restore_owner is None:
                self._context._register_imported_custody(self.custody, custody)
            else:
                self._restore_owner.imports.append(custody)
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

    def _retain_archive_terminal(self):
        """Retain a readonly terminal response for disposal, never acceptance."""
        package = self._archive_owner
        _require(package is not None and package.call is self and self.verb == "readback"
                 and self._entered and self._child is None and self._result is not None
                 and type(self._exit) is int and type(self._stdout) is bytes,
                 "PAPER_OPS_UNRESOLVED")
        _require(self._result.get("outcome") != "COMMIT_UNKNOWN", "PAPER_OPS_UNRESOLVED")
        self._revalidate()
        _require(_validate_result(self._stdout, "readback", self._exit) == self._result,
                 "PAPER_OPS_RESULT_CHANGED")
        if self._result.get("artifact_sha256") is not None:
            self.readback_artifact()
        # Retaining actual helper bytes acknowledges receipt for resource
        # disposal only. Failure/UNKNOWN still cannot become archive authority.
        _write_file(package.owner.root_tree.fd, "archive-native-result.json", self._stdout, 0o400)
        module = sys.modules["owner_beta_paper_backup"]
        package.originals.append(module._ArchiveOriginal(
            package, package.owner.root / "archive-native-result.json"))
        self._imported = True

    def _output_fd(self):
        if self._lifecycle_owner is not None:
            self._lifecycle_owner.revalidate()
            return self._lifecycle_owner.output_fd
        if self.verb in LIVE_ACTIONS:
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
        fd, data, identity = _open_file(parent, name, 0o400,
            maximum=1_048_576 if self._lifecycle_owner is not None else MAX_PAGE)
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
        if self._archive_owner is not None:
            self._archive_owner.revalidate()
        if self._lifecycle_owner is not None:
            self._lifecycle_owner.revalidate()
        elif self.verb in LIVE_ACTIONS:
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
        parent = context.tree.fd if self._restore_owner is None else self._restore_owner.tree.fd
        _require(_identity(os.stat(PAPER_DIR, dir_fd=parent,
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
        if getattr(self, "_lifecycle_owner", None) is not None:
            # Original financial inputs outlive native exit/import and owned
            # container removal. Final checks precede temporary view deletion.
            self._lifecycle_owner.finish_inputs(preserve=self._preserve)
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
        if (self._restore_owner is None and self.verb in LIVE_ACTIONS and self._context._mode == "live"
                and (self._cleanup_state == "RETAINED_FOR_ACTION"
                     or self.verb in {"reconcile", "off"})
                and self._imported and (self.verb != "begin" or (not self._preserve and self._exit == 0))
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


# These are internal observation encodings, not new receipt/authority wires.
# Only the installed root observer constructs them; the frozen inventory refers
# to their protected, retained originals. No public parser mints a live handle.
_STOP_TOKEN = object()
_WRITER_CLASSES = frozenset({"API", "PAPER_RUNNER", "PAPER_DELIVERY",
    "SOURCE_PUBLISHER", "SOURCE_CHILD", "DAILY", "ADMITTER", "RECOMMENDATION",
    "SIMULATION", "RAW_CURATED", "OTHER_DB_WRITER"})
_PAPER_CLASSES = frozenset({"API", "PAPER_RUNNER", "PAPER_DELIVERY"})
_SERVICE_CLASSES = {
    "api-server": ("API", "PAPER_DELIVERY"),
    "paper-scheduler": ("PAPER_RUNNER", "PAPER_DELIVERY"),
    "owner-beta-daily-producer": ("DAILY", "RAW_CURATED"),
    "owner-beta-daily-admitter": ("ADMITTER",),
    "owner-beta-runner": ("DAILY", "RECOMMENDATION"),
    "recommendation-runner": ("RECOMMENDATION",),
    "candidate-runner": ("RAW_CURATED", "RECOMMENDATION"),
    "owner-beta-simulation-runner": ("SIMULATION",),
    "research-worker": ("RAW_CURATED", "DAILY"),
}


class _StopDeadline:
    def __init__(self):
        self.end = time.monotonic_ns() + 90_000_000_000

    def remaining(self, cap=15):
        left = (self.end - time.monotonic_ns()) / 1_000_000_000
        _require(left > 0, "PAPER_OPS_STOP_TIMEOUT")
        return min(cap, left)


class _StopDocker:
    """Charge even existing context revalidation to the original attempt."""
    def __init__(self, docker, deadline):
        self.base, self.deadline = docker, deadline
        self.environment = docker.environment

    def call(self, args, timeout=15):
        return self.base.call(args, timeout=self.deadline.remaining(timeout))

    def object(self, kind, identity):
        raw = self.call([kind, "inspect", identity])
        try:
            values = json.loads(raw, object_pairs_hook=_pairs)
        except (ValueError, TypeError):
            _fail("PAPER_OPS_WRITER_UNKNOWN")
        _require(type(values) is list and len(values) == 1 and type(values[0]) is dict,
                 "PAPER_OPS_WRITER_UNKNOWN")
        return values[0]


class _ArchiveWorkDocker(_StopDocker):
    """Work consumes the original capture clock, preserving its cleanup reserve."""
    def call(self, args, timeout=15):
        return self.base.call(args, timeout=self.deadline.work_remaining(timeout))


def _process_record(pid):
    """Kernel generation, executable and cgroup; argv/environ are never retained."""
    _require(type(pid) is int and pid > 1, "PAPER_OPS_PROCESS_UNKNOWN")
    root = Path("/proc") / str(pid)
    before = (root / "stat").read_text()
    fields = before[before.rfind(")") + 2:].split()
    _require(len(fields) > 19 and fields[0] not in {"Z", "X"},
             "PAPER_OPS_PROCESS_UNKNOWN")
    start, parent = int(fields[19]), int(fields[1])
    cgroup = (root / "cgroup").read_text()
    lines = cgroup.splitlines()
    _require(len(lines) == 1 and lines[0].startswith("0::/")
             and ".." not in lines[0].split("/"), "PAPER_OPS_CGROUP_UNKNOWN")
    fd = os.open(root / "exe", os.O_RDONLY | os.O_CLOEXEC)
    try:
        size, digest = _hash_fd(fd)
        executable = {"bytes": size, "sha256": digest,
                      "device": os.fstat(fd).st_dev, "inode": os.fstat(fd).st_ino}
    finally:
        os.close(fd)
    after = (root / "stat").read_text()
    _require(after[after.rfind(")") + 2:].split()[19] == str(start)
             and (root / "cgroup").read_text() == cgroup, "PAPER_OPS_PROCESS_CHANGED")
    return {"pid": pid, "start_ticks": start, "parent": parent,
            "cgroup": lines[0][3:], "executable": executable}


def _process_generation(pid):
    raw = (Path("/proc") / str(pid) / "stat").read_text()
    fields = raw[raw.rfind(")") + 2:].split()
    return int(fields[19]), fields[0]


def _tcp_endpoints(process):
    """Return actual owned TCP sockets, including both ends, without app-name."""
    pid = process["pid"]
    _require(_process_generation(pid)[0] == process["start_ticks"],
             "PAPER_OPS_PROCESS_CHANGED")
    root = Path("/proc") / str(pid)
    sockets = set()
    for entry in (root / "fd").iterdir():
        try:
            target = os.readlink(entry)
        except FileNotFoundError:
            continue
        match = re.fullmatch(r"socket:\[([0-9]+)\]", target)
        if match:
            sockets.add(match[1])
    found = []
    for name, family in (("tcp", socket.AF_INET), ("tcp6", socket.AF_INET6)):
        for row in (root / "net" / name).read_text().splitlines()[1:]:
            fields = row.split()
            if len(fields) < 10 or fields[9] not in sockets or fields[3] != "01":
                continue
            endpoints = []
            for field in fields[1:3]:
                address, port = field.split(":")
                raw = bytes.fromhex(address)
                raw = b"".join(raw[i:i+4][::-1] for i in range(0, len(raw), 4))
                endpoints.append([socket.inet_ntop(family, raw), int(port, 16)])
            found.append({"local": endpoints[0], "peer": endpoints[1],
                          "socket_inode": int(fields[9]), "pid": pid,
                          "start_ticks": process["start_ticks"]})
    _require(_process_generation(pid)[0] == process["start_ticks"],
             "PAPER_OPS_PROCESS_CHANGED")
    return sorted(found, key=lambda row: (row["local"], row["peer"], row["socket_inode"]))


def _proc_cgroups():
    result = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdecimal() or int(entry.name) <= 1:
            continue
        try:
            lines = (entry / "cgroup").read_text().splitlines()
        except FileNotFoundError:
            continue
        _require(len(lines) == 1 and lines[0].startswith("0::/"),
                 "PAPER_OPS_CGROUP_UNKNOWN")
        result[int(entry.name)] = lines[0][3:]
    return result


def _under(path, root):
    return path == root or path.startswith(root.rstrip("/") + "/")


def _container_generation(value):
    state = value.get("State") or {}
    result = {k: value.get(k) for k in ("Id", "Image", "Created", "RestartCount")}
    result["StartedAt"] = state.get("StartedAt")
    _require(runtime.CONTAINER.fullmatch(result["Id"] or "")
             and runtime.DIGEST.fullmatch(result["Image"] or "")
             and type(result["RestartCount"]) is int and result["RestartCount"] >= 0
             and all(type(result[k]) is str and result[k] for k in ("Created", "StartedAt")),
             "PAPER_OPS_WRITER_UNKNOWN")
    return result


def _terminal_container(value, generation):
    _require(_container_generation(value) == generation, "PAPER_OPS_WRITER_CHANGED")
    state = value.get("State") or {}
    _require(state.get("Running") is False and state.get("Status") == "exited"
             and state.get("Restarting") is False and state.get("Paused") is False
             and state.get("OOMKilled") is False and not state.get("Error")
             and type(state.get("ExitCode")) is int
             and type(state.get("FinishedAt")) is str
             and not state["FinishedAt"].startswith("0001-")
             and value.get("HostConfig", {}).get("RestartPolicy", {}).get("Name") == "no",
             "PAPER_OPS_WRITER_NOT_EXITED")
    times = []
    for stamp in (generation["Created"], generation["StartedAt"], state["FinishedAt"]):
        _require(re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]{1,9})?Z", stamp),
                 "PAPER_OPS_WRITER_UNKNOWN")
        try:
            times.append(datetime.datetime.fromisoformat(stamp.replace("Z", "+00:00")))
        except ValueError:
            _fail("PAPER_OPS_WRITER_UNKNOWN")
    _require(times == sorted(times) and 0 <= state["ExitCode"] <= 255,
             "PAPER_OPS_WRITER_UNKNOWN")
    return {"exit_code": state["ExitCode"], "finished_at": state["FinishedAt"],
            "restart": "no", "generation": generation}


class _ObservationOriginals:
    def __init__(self, operation):
        self.operation, self.files = operation, {}

    def put(self, value):
        data = _canonical(value)
        _require(len(data) <= journal.MAX_SMALL, "PAPER_OPS_SIZE")
        digest = _digest(data)
        if digest not in self.files:
            parent = self.operation.borrow_directory_fd()
            journal._immutable(parent, digest[7:] + ".json", data, 0o400)
            fd, actual, identity = _open_file(parent, digest[7:] + ".json")
            _require(actual == data, "PAPER_OPS_OBSERVATION_CHANGED")
            self.files[digest] = (fd, identity, data)
        self.revalidate()
        return digest

    def revalidate(self):
        parent = self.operation.borrow_directory_fd()
        for digest, (fd, identity, data) in self.files.items():
            _file(fd)
            _require(_identity(os.fstat(fd)) == identity
                     and _identity(os.stat(digest[7:] + ".json", dir_fd=parent,
                                           follow_symlinks=False)) == identity
                     and os.pread(fd, len(data) + 1, 0) == data,
                     "PAPER_OPS_OBSERVATION_CHANGED")

    def close(self):
        for fd, _identity_value, _data in self.files.values():
            os.close(fd)
        self.files.clear()


class _HostStopObserver:
    """Source-owned Linux/Docker/systemd/PG observer. No injected attestations."""
    def __init__(self, attempt):
        self.attempt, self.context, self.deadline = attempt, attempt.context, attempt.deadline
        self.containers, self.units, self.processes = {}, {}, {}
        self.pidfds, self.locks, self.source_bindings = {}, [], []
        self.cgroup_fds = {}
        self.unit_files = {}
        self.collector_locks, self.reaped = set(), set()
        self.initial = self.launch = None
        self.backends, self.backend_original = [], None
        self.roots = tuple(sorted({self.context.install_root,
            self.context.values["LAGRANGE_DATA_DIR"],
            self.context.values["LAGRANGE_ARTIFACTS_DIR"],
            self.context.values["LAGRANGE_RUNTIME_SECRET_DIR"]}))
        _require(all(runtime.absolute(p) for p in self.roots), "PAPER_OPS_PATH")

    def command(self, argv, payload=None, cap=15):
        self.deadline.remaining()
        try:
            result = subprocess.run(argv, input=payload, stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL, env=_docker_client_environment(),
                timeout=self.deadline.remaining(cap), check=False, close_fds=True)
        except (OSError, subprocess.TimeoutExpired):
            _fail("PAPER_OPS_STOP_TRANSPORT_UNKNOWN")
        self.deadline.remaining()
        _require(result.returncode == 0 and len(result.stdout) <= 2 * 1024 * 1024,
                 "PAPER_OPS_STOP_TRANSPORT_UNKNOWN")
        return result.stdout

    def systemctl(self, *args):
        return self.command(["/usr/bin/systemctl", "--no-pager", *args]).decode("utf-8")

    def units_now(self):
        # Inspect *all* service/timer sources, including loaded transient units;
        # service names alone do not establish authority or complete enumeration.
        names = set()
        for verb in ("list-unit-files", "list-units"):
            text = self.systemctl(verb, "--all", "--no-legend", "--plain", "--type=service,timer")
            for row in text.splitlines():
                fields = row.split()
                _require(fields and re.fullmatch(r"[A-Za-z0-9_.@:\\-]+\.(service|timer)", fields[0]),
                         "PAPER_OPS_LAUNCH_UNKNOWN")
                names.add(fields[0])
        _require(len(names) <= 4096, "PAPER_OPS_LAUNCH_UNKNOWN")
        selected, provisional_timers = {}, set()
        properties = ("Id", "FragmentPath", "DropInPaths", "ExecStart", "EnvironmentFiles",
            "ReadWritePaths", "WorkingDirectory", "ControlGroup", "MainPID", "InvocationID",
            "ActiveState", "SubState", "UnitFileState", "Triggers", "TriggeredBy")
        names = sorted(names)
        for offset in range(0, len(names), 64):
            raw = self.systemctl("show", "--all", "--property=" + ",".join(properties), *names[offset:offset+64])
            for block in raw.strip().split("\n\n"):
                fields = {}
                for line in block.splitlines():
                    key, sep, value = line.partition("=")
                    _require(sep and key in properties and key not in fields,
                             "PAPER_OPS_LAUNCH_UNKNOWN")
                    fields[key] = value
                if not fields:
                    continue
                _require({"Id", "FragmentPath", "ActiveState", "SubState", "UnitFileState"} <= set(fields),
                         "PAPER_OPS_LAUNCH_UNKNOWN")
                # Timer units have no Service.MainPID/ExecStart/ControlGroup
                # properties. Their lack is not a missing process observation.
                if fields["Id"].endswith(".service") and not fields["UnitFileState"].startswith("masked"):
                    _require({"MainPID", "ExecStart", "ControlGroup"} <= set(fields),
                             "PAPER_OPS_LAUNCH_UNKNOWN")
                fields = {key: fields.get(key, "") for key in properties}
                related = any(root in raw_field for root in self.roots for raw_field in fields.values())
                named = fields["Id"].startswith("lagrange-") or fields["Id"] == "paper-runner.service"
                is_timer = fields["Id"].endswith(".timer")
                if not related and not named and not is_timer:
                    continue
                # A second installation is not ours to stop, even with a familiar name.
                _require(related or is_timer, "PAPER_OPS_LAUNCH_AMBIGUOUS")
                if not related:
                    provisional_timers.add(fields["Id"])
                identity_fields = {k: fields[k] for k in properties if k not in {
                    "ActiveState", "SubState", "UnitFileState", "MainPID"}}
                record = {"name": fields["Id"], "identity": _digest(_canonical(identity_fields)),
                    "source_config": _digest(_canonical({k: fields[k] for k in properties
                        if k not in {"ActiveState", "SubState", "UnitFileState", "MainPID",
                                     "InvocationID", "ControlGroup"}})),
                    "cgroup": fields["ControlGroup"], "invocation": fields["InvocationID"],
                    "pid": int(fields["MainPID"] or "0"), "active": fields["ActiveState"],
                    "sub": fields["SubState"], "file_state": fields["UnitFileState"],
                    "triggers": sorted(fields["Triggers"].split()),
                    "triggered_by": sorted(fields["TriggeredBy"].split())}
                # Inspect protected unit originals; only their digests enter records.
                paths = [fields["FragmentPath"], *fields["DropInPaths"].split()]
                record["originals"] = []
                if fields["Id"] in provisional_timers:
                    record["pending_paths"] = paths
                    paths = []
                for path in paths:
                    if not path:
                        continue
                    data = daily.install.read_root_file(path, maximum=65_536)
                    if path not in self.unit_files:
                        tree = journal._Tree(str(Path(path).parent))
                        try:
                            mode = stat.S_IMODE(os.stat(path, follow_symlinks=False).st_mode)
                            _require(mode in {0o644, 0o600}, "PAPER_OPS_LAUNCH_UNKNOWN")
                            fd, original, identity = _open_file(tree.fd, Path(path).name, mode)
                            _require(original == data, "PAPER_OPS_LAUNCH_CHANGED")
                            self.unit_files[path] = (tree, fd, identity, original, mode)
                        except BaseException:
                            tree.close()
                            raise
                    tree, fd, identity, original, mode = self.unit_files[path]
                    tree.revalidate()
                    _require(data == original and _identity(os.fstat(fd)) == identity
                             and _identity(os.stat(Path(path).name, dir_fd=tree.fd,
                                                   follow_symlinks=False)) == identity,
                             "PAPER_OPS_LAUNCH_CHANGED")
                    record["originals"].append({"path": path, "sha256": _digest(data),
                                               "identity": list(identity)})
                    for match in re.finditer(rb"(?m)^Environment=KIS_DAILY_LOCK_FILE=(/[A-Za-z0-9/._-]+)$", data):
                        self.collector_locks.add(match[1].decode("ascii"))
                _require(record["originals"] or fields["Id"] in provisional_timers,
                         "PAPER_OPS_LAUNCH_UNKNOWN")
                record["classes"] = ["OTHER_DB_WRITER"]
                if "owner-beta-paper-evidence.py run --binding " in fields["ExecStart"]:
                    record["classes"] = ["SOURCE_PUBLISHER", "SOURCE_CHILD", "RAW_CURATED"]
                    match = re.search(r"--binding (/[^\s;{}]+)", fields["ExecStart"])
                    _require(match is not None, "PAPER_OPS_LAUNCH_UNKNOWN")
                    record["source_binding"] = match[1]
                elif "kis-" in fields["Id"]:
                    record["classes"] = ["DAILY", "RAW_CURATED"]
                selected[record["name"]] = record
        related = set(selected) - provisional_timers
        for name in provisional_timers:
            row = selected[name]
            if not (set(row["triggers"] + row["triggered_by"]) & related):
                del selected[name]
                continue
            for path in row.pop("pending_paths"):
                if not path:
                    continue
                if path not in self.unit_files:
                    tree = journal._Tree(str(Path(path).parent))
                    try:
                        fd, data, identity = _open_file(tree.fd, Path(path).name, 0o644)
                        self.unit_files[path] = (tree, fd, identity, data, 0o644)
                    except BaseException:
                        tree.close()
                        raise
                tree, fd, identity, data, mode = self.unit_files[path]
                tree.revalidate()
                _require(_identity(os.fstat(fd)) == identity
                         and _identity(os.stat(Path(path).name, dir_fd=tree.fd,
                                               follow_symlinks=False)) == identity
                         and os.pread(fd, len(data)+1, 0) == data, "PAPER_OPS_LAUNCH_CHANGED")
                row["originals"].append({"path": path, "sha256": _digest(data),
                                         "identity": list(identity)})
            _require(row["originals"], "PAPER_OPS_LAUNCH_UNKNOWN")
        # Every trigger of a selected unit must have been included as authority.
        for row in selected.values():
            _require(set(row["triggers"] + row["triggered_by"]) <= set(selected),
                     "PAPER_OPS_LAUNCH_UNKNOWN")
        return selected

    def containers_now(self):
        context = self.context
        rows = context.docker.call(["ps", "--all", "--no-trunc", "--format", "{{.ID}}"])
        ids = rows.splitlines()
        _require(len(ids) <= 4096 and len(ids) == len(set(ids))
                 and all(runtime.CONTAINER.fullmatch(cid) for cid in ids),
                 "PAPER_OPS_WRITER_UNKNOWN")
        selected = {}
        for cid in sorted(ids):
            self.deadline.remaining()
            value = context.docker.object("container", cid)
            labels = value.get("Config", {}).get("Labels") or {}
            mounts = value.get("Mounts") or []
            network_ids = {n.get("NetworkID") for n in
                           (value.get("NetworkSettings", {}).get("Networks") or {}).values()}
            related_mount = any(any(_under(m.get("Source", ""), root)
                                    or _under(root, m.get("Source", "/impossible"))
                                    for root in self.roots) for m in mounts)
            project = labels.get("com.docker.compose.project")
            related = project == context.project or related_mount or context.network_id in network_ids
            if not related:
                continue
            if self._owned_archive_container(value):
                continue
            _require(project == context.project, "PAPER_OPS_WRITER_AMBIGUOUS")
            if cid == context.database_generation[0]:
                continue
            service = labels.get("com.docker.compose.service")
            _require(type(service) is str and re.fullmatch(r"[a-z0-9][a-z0-9-]{0,62}", service),
                     "PAPER_OPS_WRITER_AMBIGUOUS")
            if value.get("Image") in context.images.values():
                _require(labels.get("org.opencontainers.image.revision") == context.commit,
                         "PAPER_OPS_WRITER_AMBIGUOUS")
            else:
                # Existing upstream Compose members are not invented V3 rows.
                # Bind their exact installed source pin and inspectable image ID.
                source = daily.install.read_root_file(context.release_path + "/deploy/compose/compose.yml",
                                                       maximum=release.MAX_SOURCE)
                path = context.release_path + "/deploy/compose/compose.yml"
                if path not in self.unit_files:
                    tree = journal._Tree(str(Path(path).parent))
                    try:
                        fd, original, identity = _open_file(tree.fd, Path(path).name, 0o644)
                        _require(original == source, "PAPER_OPS_LAUNCH_CHANGED")
                        self.unit_files[path] = (tree, fd, identity, original, 0o644)
                    except BaseException:
                        tree.close()
                        raise
                tree, fd, identity, original, _mode = self.unit_files[path]
                tree.revalidate()
                _require(source == original and _identity(os.fstat(fd)) == identity
                         and _identity(os.stat(Path(path).name, dir_fd=tree.fd,
                                               follow_symlinks=False)) == identity,
                         "PAPER_OPS_LAUNCH_CHANGED")
                match = re.search(rb"(?m)^  " + service.encode("ascii") +
                    rb":\n    image: ([A-Za-z0-9_./-]+@sha256:[0-9a-f]{64})(?: |\n)", source)
                _require(match is not None, "PAPER_OPS_WRITER_AMBIGUOUS")
                reference = match[1].decode("ascii")
                image = context.docker.object("image", reference)
                _require(image.get("Id") == value.get("Image")
                         and reference in (image.get("RepoDigests") or [])
                         and value.get("Config", {}).get("Image") == reference,
                         "PAPER_OPS_WRITER_AMBIGUOUS")
                self.attempt.originals.put({"compose_sha256": _digest(source),
                    "service": service, "upstream_reference": reference, "image_id": image["Id"]})
            classes = _SERVICE_CLASSES.get(service, ("OTHER_DB_WRITER",))
            # Shared roles are deliberately NOT excluded by application_name or
            # a mode flag. Without a proven read-only authority they require the
            # explicitly selected coherent combined operation.
            _require(self.attempt.scope == "COMBINED_BACKUP" or set(classes) <= _PAPER_CLASSES,
                     "PAPER_OPS_SCOPE_AMBIGUOUS")
            generation = _container_generation(value)
            _require(value.get("State", {}).get("Status") in {"running", "exited"},
                     "PAPER_OPS_WRITER_UNKNOWN")
            selected[cid] = {"generation": generation, "service": service,
                "classes": list(classes), "pid": value.get("State", {}).get("Pid"),
                "mounts": [{k: m.get(k) for k in ("Type", "Source", "Destination", "RW")}
                           for m in mounts],
                "policy": _digest(_canonical({k: value.get("HostConfig", {}).get(k)
                    for k in ("ReadonlyRootfs", "Privileged", "PidMode", "NetworkMode")})),
                "restart": value.get("HostConfig", {}).get("RestartPolicy", {}).get("Name")}
        return selected

    def _owned_archive_container(self, value):
        return False  # ordinary STOP has no read-client exception

    def _owned_archive_process(self, pid):
        return False

    def capture(self):
        self.context.revalidate()
        self.root_observation = self.observe_roots()
        self.scheduler_observation = self.other_launch_sources()
        self.units, self.containers = self.units_now(), self.containers_now()
        _require(self.attempt.scope == "COMBINED_BACKUP" or not self.units,
                 "PAPER_OPS_SCOPE_AMBIGUOUS")
        cgroups = _proc_cgroups()
        owners = {}
        for cid, container in self.containers.items():
            pid = container["pid"]
            if pid:
                _require(type(pid) is int and pid in cgroups and cid in cgroups[pid],
                         "PAPER_OPS_CGROUP_UNKNOWN")
                container["cgroup"] = cgroups[pid]
                self.retain_cgroup(container["cgroup"])
                owners[container["cgroup"]] = ("CONTAINER", cid)
            else:
                _require(cid not in "\n".join(cgroups.values()), "PAPER_OPS_CGROUP_UNKNOWN")
                container["cgroup"] = None
        for name, unit in self.units.items():
            _require(name.endswith(".timer") or unit["pid"] > 1
                     or unit["active"] in {"inactive", "failed"}, "PAPER_OPS_LAUNCH_PENDING")
            if unit["pid"]:
                _require(unit["cgroup"] and cgroups.get(unit["pid"]) == unit["cgroup"],
                         "PAPER_OPS_CGROUP_UNKNOWN")
                owners[unit["cgroup"]] = ("UNIT", name)
                self.retain_cgroup(unit["cgroup"])
            if "source_binding" in unit:
                source = _load_local("paper_stop_source", "owner-beta-paper-evidence.py")
                binding, raw = source.binding(unit["source_binding"])
                _require(binding["release_root"] == self.context.release_path,
                         "PAPER_OPS_LAUNCH_UNKNOWN")
                self.source_bindings.append((source, binding, raw, unit["source_binding"]))
                self.collector_locks.add(binding["lock_file"])
        for pid, group in cgroups.items():
            matches = [owner for path, owner in owners.items() if _under(group, path)]
            _require(len(matches) <= 1, "PAPER_OPS_WRITER_AMBIGUOUS")
            if not matches:
                continue
            process = _process_record(pid)
            process["owner"] = list(matches[0])
            process["sockets"] = _tcp_endpoints(process)
            _require(hasattr(os, "pidfd_open") and hasattr(signal, "pidfd_send_signal"),
                     "PAPER_OPS_PROCESS_UNKNOWN")
            self.pidfds[pid] = os.pidfd_open(pid, 0)
            _require(_process_generation(pid)[0] == process["start_ticks"],
                     "PAPER_OPS_PROCESS_CHANGED")
            self.processes[pid] = process
        self.reject_unowned(cgroups)
        self.verify_paper_elfs()
        self.backend_original = self.database("observe")
        self.backends = self.correlate(self.backend_original["backends"])
        self.initial = {"boot_id": self.attempt.boot, "manifest": self.context.manifest_sha256,
            "containers": {key: self.attempt.originals.put(value)
                           for key, value in self.containers.items()},
            "units": {key: self.attempt.originals.put(value) for key, value in self.units.items()},
            "writable_roots": self.root_observation,
            "other_launch_sources": self.scheduler_observation,
            "processes": [self.attempt.originals.put(self.processes[p]) for p in sorted(self.processes)],
            "backend_observation": self.backend_original}
        self.attempt.originals.put(self.initial)

    def retain_cgroup(self, group):
        _require(group.startswith("/") and ".." not in group.split("/"), "PAPER_OPS_CGROUP_UNKNOWN")
        if group in self.cgroup_fds:
            return
        path = Path("/sys/fs/cgroup") / group.lstrip("/")
        fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
        self.cgroup_fds[group] = (fd, _identity(os.fstat(fd)))

    def empty_cgroup(self, group):
        fd, identity = self.cgroup_fds[group]
        _require(_identity(os.fstat(fd))[:5] == identity[:5], "PAPER_OPS_CGROUP_UNKNOWN")
        path = Path("/sys/fs/cgroup") / group.lstrip("/")
        try:
            info = path.lstat()
        except FileNotFoundError:
            # A cgroup can be removed only after its processes have left. The
            # retained inode, process pidfds and separate /proc scan still bind it.
            return {"identity": list(identity), "removed": True}
        _require(_identity(info)[:5] == identity[:5], "PAPER_OPS_CGROUP_UNKNOWN")
        eventfd = os.open("cgroup.events", os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=fd)
        try:
            raw = os.read(eventfd, 4097)
        finally:
            os.close(eventfd)
        _require(len(raw) <= 4096, "PAPER_OPS_CGROUP_UNKNOWN")
        fields = dict(line.split() for line in raw.decode("ascii").splitlines())
        _require(fields.get("populated") == "0", "PAPER_OPS_CGROUP_NOT_EMPTY")
        return {"identity": list(identity), "removed": False, "populated": 0}

    def observe_roots(self):
        rows = []
        for root in self.roots:
            path = Path("/")
            chain, missing = [], []
            for name in Path(root).parts[1:]:
                path = path / name
                if missing:
                    missing.append(name)
                    continue
                try:
                    info = path.lstat()
                except FileNotFoundError:
                    missing.append(name)
                    continue
                _require(stat.S_ISDIR(info.st_mode) and info.st_uid in {0, 999, 10001, 10002, 10003}
                         and not info.st_mode & 0o022, "PAPER_OPS_ROOT_CHANGED")
                chain.append({"path": str(path), "identity": list(_identity(info))})
            rows.append({"root": root, "ancestors": chain, "missing": missing})
        return rows

    def other_launch_sources(self):
        """Reject unsanctioned cron/at launch authority instead of assuming none.

        These are not controller-managed launch sources. An installation-linked
        job must be removed through its owner before this operation can settle.
        No crontab, spool file, borrowed lock or unrelated job is edited here.
        """
        rows = []
        for selected in ("/etc/crontab", "/etc/cron.d", "/etc/cron.hourly", "/etc/cron.daily",
                         "/etc/cron.weekly", "/etc/cron.monthly", "/var/spool/cron/crontabs",
                         "/var/spool/cron/atjobs", "/var/spool/at"):
            root = Path(selected)
            try:
                info = root.lstat()
            except FileNotFoundError:
                rows.append({"path": selected, "absent": True})
                continue
            _require(not stat.S_ISLNK(info.st_mode), "PAPER_OPS_LAUNCH_UNKNOWN")
            paths = sorted(root.iterdir()) if stat.S_ISDIR(info.st_mode) else [root]
            _require(len(paths) <= 256, "PAPER_OPS_LAUNCH_UNKNOWN")
            members = []
            for path in paths:
                self.deadline.remaining()
                item = path.lstat()
                _require(stat.S_ISREG(item.st_mode) and item.st_nlink == 1
                         and item.st_size <= 65_536, "PAPER_OPS_LAUNCH_UNKNOWN")
                fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
                try:
                    data = os.pread(fd, 65_537, 0)
                    _require(_identity(os.fstat(fd)) == _identity(item)
                             and _identity(path.lstat()) == _identity(item), "PAPER_OPS_LAUNCH_CHANGED")
                finally:
                    os.close(fd)
                _require(not any(boundary.encode() in data for boundary in self.roots)
                         and not any(name in data for name in (b"owner-beta-", b"kis-daily-", b"compose-release")),
                         "PAPER_OPS_UNLISTED_LAUNCH")
                # Spool bodies/env/argv are never retained or hashed as evidence.
                members.append({"name": path.name, "identity": list(_identity(item)),
                                "mtime_ns": item.st_mtime_ns, "bytes": item.st_size})
            rows.append({"path": selected, "identity": list(_identity(info)), "members": members})
        return rows

    def reject_unowned(self, cgroups):
        # Include actual processes holding authority-root files/cwd, even when
        # neither a Compose label nor a conventional service name identifies them.
        dbpid = self.context.docker.object("container", self.context.database_generation[0])["State"]["Pid"]
        dbgroup = cgroups.get(dbpid)
        _require(dbgroup and self.context.database_generation[0] in dbgroup,
                 "PAPER_OPS_DATABASE_GENERATION")
        for pid, group in cgroups.items():
            self.deadline.remaining()
            if pid == os.getpid() or pid in self.processes or _under(group, dbgroup):
                continue
            if self._owned_archive_process(pid):
                continue
            root = Path("/proc") / str(pid)
            try:
                paths = [os.readlink(root / "cwd"), os.readlink(root / "exe")]
                for fd in (root / "fd").iterdir():
                    try:
                        target = os.readlink(fd)
                        flags = next(line.split()[1] for line in
                            (root / "fdinfo" / fd.name).read_text().splitlines()
                            if line.startswith("flags:"))
                        if int(flags, 8) & os.O_ACCMODE:
                            paths.append(target)
                    except FileNotFoundError:
                        continue
                argv = (root / "cmdline").read_bytes()
                _require(not any(any(_under(path, boundary) for boundary in self.roots)
                                 for path in paths)
                         and not any(boundary.encode() in argv for boundary in self.roots),
                         "PAPER_OPS_UNLISTED_WRITER")
            except (FileNotFoundError, ProcessLookupError):
                continue

    def verify_paper_elfs(self):
        checked = self.context.verified
        data = daily.install.read_file(checked._fds[-1], "capability.json", uid=0, gid=0,
                                       mode=0o444, maximum=release.MAX_PROCESS)
        _require(_digest(data) == checked.capability_sha256, "PAPER_OPS_WRITER_ELF")
        capability = _json(data, release.MAX_PROCESS)
        for cid, container in self.containers.items():
            if not container["pid"]:
                continue
            field = {"api-server": "api_binary_sha256",
                     "paper-scheduler": "worker_binary_sha256"}.get(container["service"])
            if field is not None:
                _require(any(p["owner"] == ["CONTAINER", cid]
                             and p["executable"]["sha256"] == capability[field]
                             for p in self.processes.values()), "PAPER_OPS_WRITER_ELF")

    def correlate(self, backends):
        endpoints = []
        db = self.context.docker.object("container", self.context.database_generation[0])
        server_ips = {n.get("IPAddress") for n in db["NetworkSettings"]["Networks"].values()}
        for process in self.processes.values():
            for endpoint in process["sockets"]:
                if endpoint["peer"][0] in server_ips and endpoint["peer"][1] == 5432:
                    endpoints.append((endpoint, process))
        captured = []
        for backend in backends:
            matches = [(endpoint, process) for endpoint, process in endpoints
                       if endpoint["local"] == [backend["addr"], backend["port"]]]
            _require(len(matches) == 1, "PAPER_OPS_BACKEND_UNCORRELATED")
            endpoint, process = matches[0]
            captured.append({**backend, "process": {"pid": process["pid"],
                "start_ticks": process["start_ticks"], "owner": process["owner"]},
                "socket": endpoint})
        return captured

    def database(self, action):
        """Fixed privileged LOCAL socket transport, never serving credentials.

        Each command completes its transaction before any OS wait. Captured PID
        and backend_start equality is repeated in the signaling SQL itself.
        The observer is excluded only by pg_backend_pid() in this exact source-
        owned read-only transaction, never by a caller's application_name.
        """
        _require(action in {"observe", "cancel", "terminate"}, "PAPER_OPS_DATABASE")
        self.context.revalidate()
        server = self.context.docker.object("container", self.context.database_generation[0])
        env = runtime.environment(server)
        role = self.context.values.get("POSTGRES_USER", "lagrange")
        _require(re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,62}", role)
                 and env.get("POSTGRES_USER") == role, "PAPER_OPS_DATABASE_PRINCIPAL")
        pid = server["State"]["Pid"]
        process = _process_record(pid)
        _require(server["Id"] in process["cgroup"], "PAPER_OPS_DATABASE_GENERATION")
        old = getattr(self, "database_process", None)
        _require(old is None or old == process, "PAPER_OPS_DATABASE_GENERATION")
        self.database_process = process
        control = self.attempt.operation._intent
        account, generation = control["account_id"], control["generation"]
        journal._uuid(account); journal._uuid(generation)
        signal_sql = ""
        if action != "observe":
            ids = [(row["pid"], row["start_us"]) for row in self.backends]
            _require(all(type(p) is int and p > 1 and type(s) is int and s > 0 for p, s in ids),
                     "PAPER_OPS_BACKEND_UNKNOWN")
            if ids:
                values = ",".join("(%d,%d)" % pair for pair in ids)
                function = "pg_cancel_backend" if action == "cancel" else "pg_terminate_backend"
                # Output from signal functions is not termination proof.
                signal_sql = ("SELECT count(pg_catalog." + function +
                    "(a.pid)) FROM pg_catalog.pg_stat_activity a JOIN (VALUES " + values +
                    ") v(pid,start_us) ON a.pid=v.pid AND "
                    "(extract(epoch FROM a.backend_start)*1000000)::bigint=v.start_us "
                    "WHERE a.datname=current_database() AND a.pid<>pg_backend_pid();\n")
        sql = ("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n"
            "SET LOCAL statement_timeout='5s'; SET LOCAL lock_timeout='5s';\n" + signal_sql +
            "SELECT json_build_object('observer',json_build_object('pid',pg_backend_pid(),"
            "'user',current_user,'session',session_user,'read_only',current_setting('transaction_read_only'),"
            "'role',current_setting('role'),'superuser',(SELECT rolsuper FROM pg_catalog.pg_roles "
            "WHERE rolname=current_user)), 'prepared',(SELECT count(*) FROM pg_catalog.pg_prepared_xacts),"
            "'unexpected',(SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname=current_database() "
            "AND backend_type NOT IN ('client backend','autovacuum worker')),"
            "'backends',(SELECT COALESCE(json_agg(json_build_object('pid',pid,"
            "'start_us',(extract(epoch FROM backend_start)*1000000)::bigint,'role',usename,"
            "'addr',host(client_addr),'port',client_port) ORDER BY pid), '[]'::json) "
            "FROM pg_catalog.pg_stat_activity WHERE datname=current_database() "
            "AND backend_type='client backend' AND pid<>pg_backend_pid()),"
            "'fence',(SELECT json_build_object('epoch',c.control_epoch,'state',c.state,"
            "'event',o.fence_event_id,'generation',o.generation) "
            "FROM public.owner_beta_paper_control c CROSS JOIN public.owner_beta_paper_operations o "
            "WHERE c.account_id='" + account + "'::uuid AND o.generation='" + generation +
            "'::uuid AND o.account_id=c.account_id AND o.fence_epoch=" + str(self.attempt.fence) + " "
            "AND o.installation_id='" + control["installation_id"] + "'::uuid "
            "AND o.release_manifest_sha256='" + self.context.manifest_sha256 +
            "' AND o.intent_sha256='" + self.attempt.operation.intent_sha256 + "'));\nCOMMIT;\n")
        output = self.command(["/usr/bin/docker", "--host", "unix:///var/run/docker.sock",
            "exec", "--interactive", "--user", "999:999", server["Id"],
            "psql", "-X", "-qAt", "-h", "/var/run/postgresql", "-U", role,
            "-d", self.context.database["database"], "-v", "ON_ERROR_STOP=1",
            "-v", "VERBOSITY=sqlstate"], sql.encode("ascii"))
        rows = output.splitlines()
        _require(len(rows) == (2 if signal_sql else 1), "PAPER_OPS_BACKEND_UNKNOWN")
        try:
            value = json.loads(rows[-1], object_pairs_hook=_pairs)
        except (ValueError, TypeError):
            _fail("PAPER_OPS_BACKEND_UNKNOWN")
        _require(type(value) is dict and set(value) == {"observer", "prepared", "unexpected", "backends", "fence"},
                 "PAPER_OPS_BACKEND_UNKNOWN")
        observer = value["observer"]
        _require(type(observer) is dict and set(observer) == {"pid", "user", "session", "read_only", "role", "superuser"}
                 and observer["user"] == observer["session"] == role
                 and observer["read_only"] == "on" and observer["role"] == "none"
                 and observer["superuser"] is True and type(observer["pid"]) is int,
                 "PAPER_OPS_DATABASE_PRINCIPAL")
        _require(type(value["prepared"]) is int and value["prepared"] == 0
                 and type(value["unexpected"]) is int and value["unexpected"] == 0,
                 "PAPER_OPS_PREPARED_UNKNOWN")
        committed = getattr(self.attempt, "consumer", None)
        epoch, state = ((self.attempt.fence, "DRAINING") if committed is None
                        else committed.database_state())
        _require(value["fence"] == {"epoch": epoch, "state": state,
                  "event": self.attempt.event, "generation": generation}, "PAPER_OPS_FENCE_CHANGED")
        _require(type(value["backends"]) is list and len(value["backends"]) <= 64,
                 "PAPER_OPS_BACKEND_UNKNOWN")
        seen = set()
        for row in value["backends"]:
            _require(type(row) is dict and set(row) == {"pid", "start_us", "role", "addr", "port"}
                     and type(row["pid"]) is int and row["pid"] > 1
                     and type(row["start_us"]) is int and row["start_us"] > 0
                     and type(row["role"]) is str and row["role"]
                     and row["pid"] not in seen, "PAPER_OPS_BACKEND_UNKNOWN")
            seen.add(row["pid"])
        _require(_process_record(pid) == process, "PAPER_OPS_DATABASE_GENERATION")
        self.context.revalidate()
        return value

    def disable_launches(self):
        captured = {cid: {k: v for k, v in row.items() if k != "cgroup"}
                    for cid, row in self.containers.items()}
        _require(self.units_now() == self.units and self.containers_now() == captured,
                 "PAPER_OPS_WRITER_CHANGED")
        for name in sorted(self.units):
            self.systemctl("disable", "--no-block", name)
        for name in sorted(self.units):
            if name.endswith(".timer"):
                self.systemctl("stop", "--no-block", name)
        for cid in self.containers:
            current = self.context.docker.object("container", cid)
            _require(_container_generation(current) == self.containers[cid]["generation"],
                     "PAPER_OPS_WRITER_CHANGED")
            self.context.docker.call(["update", "--restart=no", cid])
        while True:
            try:
                self.launch = self.launch_observation(waiting=True)
                break
            except PaperOperationsError as error:
                if error.code not in {"PAPER_OPS_LAUNCH_PENDING", "PAPER_OPS_WRITER_NOT_EXITED"}:
                    raise
                time.sleep(self.deadline.remaining(0.05))
        self.attempt.originals.put(self.launch)

    def launch_observation(self, *, waiting=False):
        units, containers = self.units_now(), self.containers_now()
        _require(set(units) == set(self.units) and set(containers) == set(self.containers),
                 "PAPER_OPS_LAUNCH_CHANGED")
        for name, current in units.items():
            original = self.units[name]
            _require(current["originals"] == original["originals"]
                     and current.get("source_config") == original.get("source_config")
                     and current["file_state"] in {"disabled", "static", "indirect", "masked", "masked-runtime"}
                     and current["triggers"] == original["triggers"]
                     and current["triggered_by"] == original["triggered_by"],
                     "PAPER_OPS_LAUNCH_CHANGED")
            if current["active"] in {"active", "activating", "deactivating"}:
                _require(current["invocation"] == original["invocation"]
                         and current["cgroup"] == original["cgroup"]
                         and current["pid"] in {0, original["pid"]},
                         "PAPER_OPS_LAUNCH_CHANGED")
                _require(waiting and not name.endswith(".timer"), "PAPER_OPS_WRITER_NOT_EXITED")
            else:
                _require(current["active"] in {"inactive", "failed"} and current["pid"] == 0,
                         "PAPER_OPS_LAUNCH_CHANGED")
        for cid, current in containers.items():
            original = self.containers[cid]
            _require(current["generation"] == original["generation"]
                     and current["mounts"] == original["mounts"]
                     and current["policy"] == original["policy"] and current["restart"] == "no",
                     "PAPER_OPS_WRITER_CHANGED")
        jobs = self.systemctl("list-jobs", "--no-legend", "--plain")
        for line in jobs.splitlines():
            parts = line.split()
            _require(not any(name in parts for name in self.units), "PAPER_OPS_LAUNCH_PENDING")
        return {"units": units, "containers": containers, "pending_owned_jobs": []}

    def signal(self):
        # Dispatch shutdown without taking collector/scheduler/publication locks.
        # The existing source SIGTERM handler revokes Current/OFD in finally.
        self.launch_observation(waiting=True)
        sent = set()
        for name in sorted(self.units):
            pid = self.units[name]["pid"]
            if pid and pid not in sent:
                process = self.processes[pid]
                _require(_process_generation(pid)[0] == process["start_ticks"],
                         "PAPER_OPS_PROCESS_CHANGED")
                # Keep the captured MainPID alive while submitting StopUnit.
                # This closes the automatic-restart race between a show reply
                # and a name-based stop. Signals use the retained pidfd only.
                # StopUnit suppresses Restart= even on failure; disable alone
                # would not do that. Never pause/wait under a collector lock.
                signal.pidfd_send_signal(self.pidfds[pid], signal.SIGSTOP)
                try:
                    while True:
                        ticks, state = _process_generation(pid)
                        _require(ticks == process["start_ticks"], "PAPER_OPS_PROCESS_CHANGED")
                        if state in {"T", "t"}:
                            break
                        time.sleep(self.deadline.remaining(0.01))
                    now = self.units_now()[name]
                    _require(now["pid"] == pid and now["invocation"] == self.units[name]["invocation"]
                             and now["cgroup"] == self.units[name]["cgroup"], "PAPER_OPS_LAUNCH_CHANGED")
                    self.systemctl("stop", "--no-block", name)
                finally:
                    # Even a failed stop dispatch must release our exact pause.
                    # pidfd never targets a replacement at the same numeric PID.
                    try:
                        signal.pidfd_send_signal(self.pidfds[pid], signal.SIGCONT)
                    except ProcessLookupError:
                        pass  # The retained generation exited; never signal a replacement.
                sent.add(pid)
        for cid, record in self.containers.items():
            value = self.context.docker.object("container", cid)
            _require(_container_generation(value) == record["generation"]
                     and value["HostConfig"]["RestartPolicy"]["Name"] == "no",
                     "PAPER_OPS_WRITER_CHANGED")
            if value["State"]["Running"]:
                self.context.docker.call(["kill", "--signal=TERM", cid])

    def wait_exit(self):
        while True:
            self.deadline.remaining()
            pending = False
            for pid, process in self.processes.items():
                selector = selectors.DefaultSelector()
                try:
                    selector.register(self.pidfds[pid], selectors.EVENT_READ)
                    exited = bool(selector.select(0))
                finally:
                    selector.close()
                if not exited:
                    _require(_process_generation(pid)[0] == process["start_ticks"],
                             "PAPER_OPS_PROCESS_CHANGED")
                    pending = True
                elif process["parent"] == os.getpid() and pid not in self.reaped:
                    # Only our captured child may be reaped. A foreign parent's
                    # wait status is never fabricated from a successful signal.
                    try:
                        found, _status = os.waitpid(pid, os.WNOHANG)
                    except ChildProcessError:
                        _fail("PAPER_OPS_CHILD_UNKNOWN")
                    _require(found == pid, "PAPER_OPS_CHILD_UNKNOWN")
                    self.reaped.add(pid)
            if not pending:
                break
            time.sleep(self.deadline.remaining(0.05))
        # Retain every container until the protected STOPPED stage is sealed.
        while True:
            try:
                self.terminal_observation()
                break
            except PaperOperationsError as error:
                if error.code not in {"PAPER_OPS_WRITER_NOT_EXITED", "PAPER_OPS_LAUNCH_PENDING"}:
                    raise
                time.sleep(self.deadline.remaining(0.05))

    def terminal_observation(self):
        _require(self.observe_roots() == self.root_observation, "PAPER_OPS_ROOT_CHANGED")
        _require(self.other_launch_sources() == self.scheduler_observation, "PAPER_OPS_LAUNCH_CHANGED")
        cgroups = _proc_cgroups()
        records = []
        for cid, row in self.containers.items():
            value = self.context.docker.object("container", cid)
            terminal = _terminal_container(value, row["generation"])
            _require(not any(cid in group or (row["cgroup"] and _under(group, row["cgroup"]))
                             for group in cgroups.values()), "PAPER_OPS_CGROUP_NOT_EMPTY")
            records.append({"kind": "CONTAINER", "identity_sha256": self.attempt.originals.put(row),
                            "terminal": terminal,
                            "cgroup": self.empty_cgroup(row["cgroup"]) if row["cgroup"] else None})
        for name, row in self.units.items():
            _require(not row["cgroup"] or not any(_under(group, row["cgroup"])
                         for group in cgroups.values()), "PAPER_OPS_CGROUP_NOT_EMPTY")
            records.append({"kind": "UNIT", "identity_sha256": self.attempt.originals.put(row),
                            "cgroup": self.empty_cgroup(row["cgroup"]) if row["cgroup"] else None})
        for pid, process in self.processes.items():
            if pid in cgroups:
                # Even PID reuse is an unknown observation, not exit of a new PID.
                _fail("PAPER_OPS_PROCESS_CHANGED")
        self.reject_unowned(cgroups)
        launches = self.launch_observation()
        return {"resources": records, "launches": {
            "units": {key: self.attempt.originals.put(value) for key, value in launches["units"].items()},
            "containers": {key: self.attempt.originals.put(value)
                           for key, value in launches["containers"].items()},
            "pending_owned_jobs": launches["pending_owned_jobs"]}}

    def publication_locks(self):
        # Publisher and children have already ended. Collector precedes scheduler.
        for path in sorted(self.collector_locks):
            self.acquire_lock(path, 0, 0, 0o600)
        for source, binding, raw, path in self.source_bindings:
            selected, current = source.binding(path)
            _require(current == raw and selected == binding, "PAPER_OPS_LAUNCH_CHANGED")
            self.acquire_lock(str(Path(binding["evidence_root"]) / ".control/lock"), 0, 0, 0o600)
            # The existing shutdown may fail to publish INVALID, but must still
            # close its OFD. Missing/invalid Current never becomes a stop proof.
            current_path = Path(binding["evidence_root"]) / "current" / source.CURRENT_LEAF
            data = source.read_file(current_path, 65_536, exact=(0, 10001, 0o440))
            current = source.decode(data, 65_536)
            source.validate_current(current)
            _require(current.get("status") == "INVALID", "PAPER_OPS_SOURCE_CURRENT_UNKNOWN")
            with source.opened(current_path, exact=(0, 10001, 0o440)) as current_fd:
                _require(hasattr(fcntl, "F_OFD_GETLK"), "PAPER_OPS_SOURCE_CURRENT_UNKNOWN")
                probe = source.OfdFlock(fcntl.F_WRLCK, os.SEEK_SET, 0, 0, 0)
                result = source.OfdFlock.from_buffer_copy(
                    fcntl.fcntl(current_fd, fcntl.F_OFD_GETLK, bytes(probe)))
                _require(result.type == fcntl.F_UNLCK, "PAPER_OPS_SOURCE_CURRENT_UNKNOWN")
            self.attempt.originals.put(current)
            self.attempt.originals.put({"source_binding_sha256": _digest(raw),
                                        "current_sha256": _digest(data), "status": "INVALID"})
        # Existing Daily/Raw lock order, no alternate lock names or force unlock.
        backup = _load_local("paper_stop_backup", "owner-beta-daily-backup.py")
        data_root = self.context.values["LAGRANGE_DATA_DIR"]
        daily_path = Path(data_root) / "owner-beta-daily"
        raw_path = Path(data_root) / "raw"
        if daily_path.exists():
            config, _hash = daily.install.verify_config(self.context.release_path, self.context.commit)
            # publication_fence consumes only data_root from the already checked
            # context; activation_binding would incorrectly try to activate Daily.
            self.data_root = data_root
            manager = backup.publication_fence(self, config)
        elif raw_path.exists():
            manager = backup.raw_publication_fence(data_root)
        else:
            _require(not daily_path.is_symlink() and not raw_path.is_symlink(), "PAPER_OPS_PATH")
            return
        self.deadline.remaining()
        self.publication_lease = manager.__enter__()
        self.locks.append(("manager", manager))
        self.deadline.remaining()

    def acquire_lock(self, path, uid, gid, mode):
        # Ancestors and exact inode are checked with the existing protected reader.
        source = _load_local("paper_stop_source", "owner-beta-paper-evidence.py")
        manager = source.opened(Path(path), write=True, exact=(uid, gid, mode))
        fd = manager.__enter__()
        self.locks.append(("manager", manager))
        while True:
            self.deadline.remaining()
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                time.sleep(self.deadline.remaining(0.05))

    def drain_backends(self):
        value = self.database("observe")
        captured = {(r["pid"], r["start_us"]) for r in self.backends}
        def same(rows):
            _require({(r["pid"], r["start_us"]) for r in rows} <= captured,
                     "PAPER_OPS_BACKEND_RECONNECTED")
        same(value["backends"])
        if value["backends"]:
            self.database("cancel")
            value = self.database("observe")
            same(value["backends"])
        if value["backends"]:
            self.database("terminate")
        while True:
            value = self.database("observe")
            same(value["backends"])
            if not value["backends"]:
                return value
            time.sleep(self.deadline.remaining(0.05))

    def close(self):
        failure = None
        while self.locks:
            _kind, manager = self.locks.pop()
            try:
                manager.__exit__(None, None, None)
            except Exception:
                failure = "PAPER_OPS_LOCK_CHANGED"
        for fd in self.pidfds.values():
            os.close(fd)
        self.pidfds.clear()
        for fd, _identity_value in self.cgroup_fds.values():
            os.close(fd)
        self.cgroup_fds.clear()
        for tree, fd, _identity_value, _data, _mode in self.unit_files.values():
            os.close(fd)
            tree.close()
        self.unit_files.clear()
        if failure:
            _fail(failure)


class _StopAttempt:
    def __init__(self, token, context, invocation):
        _require(token is _STOP_TOKEN and type(context) is _PaperContext,
                 "PAPER_OPS_PRIVATE")
        self.context, self.invocation = context, invocation
        self.deadline, self.handle, self.closed = _StopDeadline(), None, False
        self.operation = invocation.custody._operation
        self.originals = _ObservationOriginals(self.operation)
        self.observer = None
        self.stage_bytes = self.inventory_bytes = self.stage_hash = None
        self.consumer = None
        self.archive = None  # one issuance; closed/failed owners cannot renew
        self.root_bytes = self.operation._intent_bytes
        root = self.operation._intent
        self.scope = root["scope"]
        self.boot = _current_boot_id()
        self.fence = _begin_fence_epoch(root)
        self.event = invocation._result["event_id"]
        self.fenced = _FENCED_CALLS.get(context, {}).get(invocation)
        _require(self.fenced is not None, "PAPER_OPS_BEGIN_UNSETTLED")

    def check(self, *, stopped=False):
        self.deadline.remaining()
        self._check_custody(stopped=stopped)
        self.deadline.remaining()

    def _check_custody(self, *, stopped=False):
        """All original custody predicates, independent of the consuming clock."""
        c, call, operation = self.context, self.invocation, self.operation
        _require(not self.closed and c._mode == "live" and not c._closed
                 and c._token is _CONTEXT_TOKEN and c.verified is not None
                 and call._context is c and call._closed and call._entered
                 and call._imported and not call._preserve and call._cid is None
                 and call._child is None and call._exit == 0
                 and call._cleanup_state == "RETAINED_FOR_ACTION"
                 and (not c._invocations or (self.consumer is not None
                      and c._invocations == {self.consumer.call}) or
                      (self.archive is not None and
                       c._invocations == {self.archive.native_invocation()}))
                 and not c._inventories and (not c._retained_stages or self.consumer is not None)
                 and _FENCED_CALLS.get(c, {}).get(call) == self.fenced,
                 "PAPER_OPS_BEGIN_UNSETTLED")
        c.revalidate()
        call.custody.revalidate()
        entry = c._input_stages.get(call.custody)
        _require(entry is not None and entry["stage"] is call._persistent_stage
                 and entry["imports"] and all(i._closed for i in entry["imports"]),
                 "PAPER_OPS_UNRESOLVED")
        entry["stage"].revalidate()
        for action, pending in c._input_stages.items():
            _require(all(i._closed for i in pending["imports"])
                     and (action is call.custody or action._closed
                          or (self.consumer is not None and action in self.consumer.actions)),
                     "PAPER_OPS_UNRESOLVED")
        fd = c._begin_result_fds.get(call)
        _require(type(fd) is int and _identity(os.fstat(fd)) == call._result_identity
                 and os.pread(fd, len(call._result_bytes) + 1, 0) == call._result_bytes
                 and _identity(os.stat(call._result_name, dir_fd=call._result_parent,
                                       follow_symlinks=False)) == call._result_identity,
                 "PAPER_OPS_RESULT_CHANGED")
        operation.revalidate()
        root = operation._intent
        _require(operation._intent_bytes == self.root_bytes
                 and root["purpose"] in {"STOP", "BACKUP"}
                 and self.scope == ("PAPER_STOP" if root["purpose"] == "STOP" else "COMBINED_BACKUP")
                 and c.manifest_sha256 == root["release_manifest_sha256"]
                 and c.verified.manifest_sha256 == c.manifest_sha256
                 and operation._journal._lock_id == c.lock_identity
                 and operation._journal._read_floor() == (self.fence if self.consumer is None
                                                        else self.consumer.floor)
                 and _current_boot_id() == self.boot,
                 "PAPER_OPS_CONTEXT_CHANGED")
        original = journal._read_named(operation._fd, "00000001.json", 0o400, journal.MAX_SMALL)
        _require(original == self.fenced[0] and _digest(original) == self.fenced[1]
                 and _json(original, MAX_RESULT)["boot_id"] == self.boot,
                 "PAPER_OPS_STAGE")
        if self.consumer is not None:
            self.consumer.check()
        expected_sequence = 2 if stopped else 1
        expected_digest = self.stage_hash if stopped else self.fenced[1]
        expected_stage = "STOPPED" if stopped else "FENCED"
        if stopped and self.consumer is not None:
            expected_sequence, expected_digest, expected_stage = self.consumer.head
        _require(operation._last[0] == expected_sequence and operation._last[1] == expected_digest
                 and operation._last[2] == expected_stage,
                 "PAPER_OPS_STAGE")
        self.originals.revalidate()

    def inventory(self, first, second, backend):
        observer, root = self.observer, self.operation._intent
        launch_hash = self.originals.put(second["launches"])
        backend_hash = self.originals.put({"captured": observer.backends,
            "initial": observer.backend_original, "final": backend,
            "database_generation": self.context.database_generation})
        rows, covered = [], set()
        for kind, values in (("CONTAINER", observer.containers), ("UNIT", observer.units)):
            for name, identity in sorted(values.items()):
                classes = identity["classes"]
                identity_hash = self.originals.put({"kind": kind, "generation": identity,
                    "boot_id": self.boot, "manifest": self.context.manifest_sha256})
                observed_hash = self.originals.put({"identity_sha256": identity_hash,
                    "first": first, "second": second})
                backend_set = [b for b in observer.backends if b["process"]["owner"] == [kind, name]]
                set_hash = self.originals.put({"identity_sha256": identity_hash,
                    "backend_observation_sha256": backend_hash, "backends": backend_set})
                for writer_class in classes:
                    covered.add(writer_class)
                    rows.append({"writer_class": writer_class, "kind": kind,
                        "identity_sha256": identity_hash, "observation_sha256": observed_hash,
                        "backend_set_sha256": set_hash,
                        "terminal": "EXITED_REAPED" if identity["pid"] else "ABSENT_AT_FENCE"})
        required = _WRITER_CLASSES if self.scope == "COMBINED_BACKUP" else _PAPER_CLASSES
        for writer_class in sorted(required - covered):
            # Absence binds a complete actual capture plus both repeated scans,
            # disabled sources and DB observations; never SHA256(empty) as proof.
            identity_hash = self.originals.put({"writer_class": writer_class,
                "installation_id": root["installation_id"], "generation": root["generation"],
                "boot_id": self.boot, "manifest": self.context.manifest_sha256,
                "capture_sha256": self.originals.put(observer.initial)})
            observation_hash = self.originals.put({"identity_sha256": identity_hash,
                "first": first, "second": second, "launch_fence_sha256": launch_hash,
                "backend_observation_sha256": backend_hash})
            rows.append({"writer_class": writer_class, "kind": "UNIT",
                "identity_sha256": identity_hash, "observation_sha256": observation_hash,
                "backend_set_sha256": backend_hash, "terminal": "ABSENT_AT_FENCE"})
        rows.sort(key=lambda r: (r["identity_sha256"], r["writer_class"], r["kind"]))
        _require(len(rows) <= 64 and required <= {r["writer_class"] for r in rows}
                 and all(r["writer_class"] in _WRITER_CLASSES for r in rows), "PAPER_OPS_WRITER_INVENTORY")
        return {"schema": "owner-beta-paper-writer-inventory-v1",
            "installation_id": root["installation_id"], "generation": root["generation"],
            "account_id": root["account_id"], "scope": self.scope,
            "fence_event_id": self.event, "control_epoch": self.fence, "boot_id": self.boot,
            "observed": _stage_observation(), "release_manifest_sha256": self.context.manifest_sha256,
            "writers": rows, "launch_fence_sha256": launch_hash,
            "backend_observation_sha256": backend_hash}

    def run(self):
        original_docker = self.context.docker
        self.context.docker = _StopDocker(original_docker, self.deadline)
        try:
            self.check()
            self.observer = _HostStopObserver(self)
            self.observer.capture()
            self.check()
            self.observer.disable_launches()
            self.observer.signal()
            self.observer.wait_exit()
            self.check()
            self.observer.publication_locks()
            backend = self.observer.drain_backends()
            first = self.observer.terminal_observation()
            self.originals.put(first)
            self.check()
            second = self.observer.terminal_observation()
            self.originals.put(second)
            backend = self.observer.database("observe")
            _require(not backend["backends"], "PAPER_OPS_BACKEND_RECONNECTED")
            self.check()
            inventory = self.inventory(first, second, backend)
            self.inventory_bytes = _canonical(inventory)
            inventory_hash = self.originals.put(inventory)
            self.check()
            stage = dict(_json(self.fenced[0], MAX_RESULT))
            stage.update(sequence=2, previous_sha256=self.fenced[1], stage="STOPPED",
                         observed=_stage_observation(), artifact_sha256=inventory_hash)
            self.stage_bytes = _canonical(stage)
            self.stage_hash = self.operation.append(self.stage_bytes)
            self.check(stopped=True)
            self.handle = DrainedHandle(_STOP_TOKEN, self)
            return self.handle
        except BaseException:
            # Capture finite known identities before cleanup; no exception text,
            # ambient env, DB messages or caller assertions enter the journal.
            try:
                self.originals.put({"attempt": str(uuid.uuid4()), "status": "UNKNOWN",
                    "generation": self.operation.generation, "boot_id": self.boot,
                    "deadline_monotonic_ns": self.deadline.end,
                    "observed": _stage_observation(), "fenced_sha256": self.fenced[1],
                    "initial_sha256": self.originals.put(self.observer.initial)
                        if self.observer is not None and self.observer.initial is not None else None})
            except Exception:
                # Preserve the primary failure and all retained in-memory custody.
                # Failed durable observation recording never authorizes STOPPED.
                self.observation_write_unknown = True
            raise
        finally:
            self.context.docker = original_docker

    def close(self):
        archive = getattr(self, "archive", None)
        _require(archive is None or archive.closed, "PAPER_OPS_ARCHIVE_CLEANUP_UNKNOWN")
        if self.closed:
            return
        if self.consumer is not None:
            self.consumer.close()
        self.closed = True
        if self.handle is not None:
            self.handle._closed = True
        try:
            if self.observer is not None:
                self.observer.close()
        finally:
            self.originals.close()


class DrainedHandle:
    """Live lock/resource custody, not a receipt or a deserialization interface."""
    __slots__ = ("_attempt", "_closed")

    def __init__(self, token, attempt):
        _require(token is _STOP_TOKEN and type(attempt) is _StopAttempt
                 and not attempt.closed and attempt.stage_hash is not None
                 and _STOP_ATTEMPTS.get(attempt.context, {}).get(attempt.invocation) is attempt,
                 "PAPER_OPS_PRIVATE")
        self._attempt, self._closed = attempt, False

    def __reduce__(self):
        _fail("PAPER_OPS_PRIVATE")

    def __reduce_ex__(self, _protocol):
        _fail("PAPER_OPS_PRIVATE")

    def __copy__(self):
        _fail("PAPER_OPS_PRIVATE")

    def __deepcopy__(self, _memo):
        _fail("PAPER_OPS_PRIVATE")

    def revalidate(self):
        attempt = self._attempt
        _require(not self._closed and not attempt.closed and attempt.handle is self
                 and _STOP_ATTEMPTS.get(attempt.context, {}).get(attempt.invocation) is attempt,
                 "PAPER_OPS_CLOSED")
        old = attempt.context.docker
        attempt.context.docker = _StopDocker(old, attempt.deadline)
        try:
            attempt.check(stopped=True)
            attempt.observer.terminal_observation()
            _require(not attempt.observer.database("observe")["backends"],
                     "PAPER_OPS_BACKEND_RECONNECTED")
            attempt.check(stopped=True)
        except BaseException:
            self._closed = True
            raise
        finally:
            attempt.context.docker = old

    def close(self):
        self._closed = True
        self._attempt.close()

    def __enter__(self):
        self.revalidate()
        return self

    def __exit__(self, *_exc):
        self.close()


def _open_maintenance_reader(consumer):
    return _PaperContext(_CONTEXT_TOKEN, consumer.context.context, consumer.context.lock_fd,
                         live=False, deadline=consumer.attempt.deadline)


def _maintenance_result(data, verb, operation, artifact, fence):
    """Validate an original SQL result, never a transport's success assertion."""
    value = journal._json(data, journal.MAX_SMALL)
    epoch = fence if verb == "reconcile" else fence + 1
    _require(epoch <= journal.MAX_I64 and type(value) is dict and set(value) == {
        "schema", "action", "outcome", "control_epoch", "account_id", "data"
    } and value["schema"] == PAGE_SCHEMA and value["outcome"] == "OK"
             and value["action"] == ("RECONCILE_OPERATION" if verb == "reconcile" else "TRANSITION")
             and type(value["control_epoch"]) is int and value["control_epoch"] == epoch
             and value["account_id"] == operation._intent["account_id"]
             and type(value["data"]) is dict, "PAPER_OPS_RESULT")
    body = value["data"]
    if verb in {"off", "restore-off"}:
        _require(set(body) == {"event_id", "state"} and body["state"] == ("RESTORED_OFF" if verb == "restore-off" else "OFF"),
                 "PAPER_OPS_RESULT")
        journal._uuid(body["event_id"])
        return body["event_id"], None
    _require(set(body) == {"observation_id", "summary", "summary_sha256"}, "PAPER_OPS_RESULT")
    summary = body["summary"]
    _require(type(summary) is dict and set(summary) == {
        "schema", "installation_id", "generation", "account_id", "observation_id",
        "control_epoch", "inventory_sha256", "observed_at", "database_sha256",
        "ledger_sha256", "pending_sha256", "outbox_sha256", "jobs_sha256",
        "unresolved_commits", "live_claims", "held_targets", "quarantined_deliveries"
    } and summary["schema"] == "owner-beta-paper-reconciliation-v1", "PAPER_OPS_RECONCILIATION")
    for key in ("installation_id", "generation", "account_id"):
        _require(summary[key] == operation._intent[key], "PAPER_OPS_RECONCILIATION")
    journal._uuid(summary["observation_id"])
    for key in ("control_epoch", "unresolved_commits", "live_claims", "held_targets", "quarantined_deliveries"):
        journal._counter(summary[key])
    for key in ("inventory_sha256", "database_sha256", "ledger_sha256", "pending_sha256",
                "outbox_sha256", "jobs_sha256"):
        journal._hash(summary[key])
    stamp = summary["observed_at"]
    _require(type(stamp) is str and re.fullmatch(r"20[0-9]{2}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z", stamp),
             "PAPER_OPS_RECONCILIATION")
    try:
        datetime.datetime.strptime(stamp, "%Y-%m-%dT%H:%M:%SZ")
    except ValueError:
        _fail("PAPER_OPS_RECONCILIATION")
    _require(summary["control_epoch"] == fence and summary["inventory_sha256"] == _digest(artifact)
             and body["observation_id"] == summary["observation_id"]
             and body["summary_sha256"] == _digest(_canonical(summary))
             and summary["unresolved_commits"] == 0 and summary["live_claims"] == 0,
             "PAPER_OPS_RECONCILIATION")
    return body["observation_id"], _canonical(summary)


class _OffCommit:
    """A continuation of one registered live drain, never restored from disk."""
    def __init__(self, token, handle):
        _require(token is _STOP_TOKEN and type(handle) is DrainedHandle, "PAPER_OPS_PRIVATE")
        self.handle, self.attempt = handle, handle._attempt
        self.context, self.operation = self.attempt.context, self.attempt.operation
        self.head = self.operation._last[:3]
        self.floor = self.attempt.fence
        _require(self.head[2] == "STOPPED" and self.floor < journal.MAX_I64,
                 "PAPER_OPS_EPOCH")
        self.actions, self.originals, self.readers = [], [], []
        self.action = self.call = self.summary = self.off_result = self.event = None
        self.running = False
        try:
            for sequence in range(3):
                name = "%08d.json" % sequence
                data = journal._read_named(self.operation._fd, name, 0o400, journal.MAX_SMALL)
                self.retain(self.operation._fd, name, data,
                            self.operation._head_fd if sequence == 2 else None)
                self.retain(self.operation._fd, _digest(data)[7:] + ".json", data)
        except BaseException:
            self.close()
            raise

    def check(self):
        _require(self.attempt.consumer is self and self.attempt.handle is self.handle
                 and not self.handle._closed and not self.attempt.closed,
                 "PAPER_OPS_CLOSED")
        allowed = set() if self.call is None else set(self.call._stages)
        _require(self.context._retained_stages <= allowed, "PAPER_OPS_UNRESOLVED")
        for action in self.actions:
            action.revalidate()
        for parent, name, fd, identity, data in self.originals:
            journal._retained_file(parent, name, fd, identity, 0o400)
            _require(os.pread(fd, len(data) + 1, 0) == data, "PAPER_OPS_RESULT_CHANGED")
        # STOPPED and every appended continuation remain named, retained originals.
        _require(journal._read_named(self.operation._fd, "00000002.json", 0o400,
                                     journal.MAX_SMALL) == self.attempt.stage_bytes,
                 "PAPER_OPS_STAGE")

    def retain(self, parent, name, expected, source_fd=None):
        for prior_parent, prior_name, prior_fd, prior_identity, prior_data in self.originals:
            if (prior_name == name and journal._dir_identity(os.fstat(parent)) ==
                    journal._dir_identity(os.fstat(prior_parent))):
                journal._retained_file(prior_parent, name, prior_fd, prior_identity, 0o400)
                _require(prior_data == expected and os.pread(prior_fd, len(expected) + 1, 0) == expected
                         and (source_fd is None or journal._identity(os.fstat(source_fd)) == prior_identity),
                         "PAPER_OPS_RESULT_CHANGED")
                return expected
        fd, data, identity = _open_file(parent, name, 0o400, maximum=journal.MAX_NATIVE_INDEX)
        try:
            _require(data == expected and (source_fd is None or
                     _identity(os.fstat(source_fd)) == identity), "PAPER_OPS_RESULT_CHANGED")
            identity = journal._identity(os.fstat(fd))
            owned_parent = os.dup(parent)
        except BaseException:
            os.close(fd)
            raise
        self.originals.append((owned_parent, name, fd, identity, data))
        return data

    def database_state(self):
        if self.off_result is None:
            return self.attempt.fence, "DRAINING"
        # Only independently read COMMITTED original OFF bytes set this member.
        _maintenance_result(self.off_result, "off", self.operation,
                            self.actions[-1]._artifact_bytes, self.attempt.fence)
        return self.attempt.fence + 1, "OFF"

    def observe(self):
        self.attempt.check(stopped=True)
        self.attempt.observer.terminal_observation()
        _require(not self.attempt.observer.database("observe")["backends"],
                 "PAPER_OPS_BACKEND_RECONNECTED")
        self.attempt.check(stopped=True)

    def prepare(self, verb, evidence):
        self.observe()
        _require(self.action is None and self.call is None, "PAPER_OPS_UNRESOLVED")
        prior = self.attempt.invocation.custody if not self.actions else self.actions[-1]
        sequence = prior.sequence + 1
        _require(sequence <= 999_999_999_999_999_999, "PAPER_OPS_EPOCH")
        root = self.operation._intent
        binding = {"schema": "owner-beta-paper-action-binding-v1",
            "installation_id": root["installation_id"], "generation": root["generation"],
            "root_intent_sha256": self.operation.intent_sha256, "sequence": sequence,
            "previous_binding_sha256": _digest(prior._binding_bytes), "verb": verb,
            "request_key": root["generation"] + (":reconcile:%d" % sequence if verb == "reconcile" else ":off"),
            "expected_epoch": self.attempt.fence, "artifact_sha256": _digest(evidence),
            "stage_sha256": self.head[1]}
        self.operation._store_original(evidence, _digest(evidence), journal.MAX_SMALL)
        self.retain(self.operation._fd, _digest(evidence)[7:] + ".json", evidence)
        self.action = self.operation.prepare_action(_canonical(binding), evidence)
        self.actions.append(self.action)
        self.call = self.context.invoke(verb, self.action)
        # Enter exactly once. A lost response never creates a replacement call/key.
        try:
            self.call.__enter__()
        except PaperOperationsError:
            # Resolve only from fixed published dispatch and independent readback.
            # A missing dispatch or a surviving helper remains UNKNOWN below.
            pass

    def readback(self, action, *, current_epoch, current_state):
        historical = self.operation._journal.open_custody(self.operation.generation, action.sequence)
        try:
            _require(historical._readonly and historical._intent == self.operation._intent_bytes
                     and historical._action == action._binding_bytes
                     and historical._artifact == action._artifact_bytes
                     and historical._dispatch == action._dispatch_bytes, "PAPER_OPS_IMPORT")
            request = journal._json(historical._sql, journal.MAX_NATIVE_EXACT)
            journal._validate_bound_request(historical._sql, self.operation._intent,
                                            action._action, action._artifact_bytes)
            for pending in self.readers:
                if not pending._closed:
                    pending.close()  # Cleanup only; never reenter its SQL invocation.
                _require(pending._closed, "PAPER_OPS_READBACK_CLEANUP")
            reader = _open_maintenance_reader(self)
            self.readers.append(reader)
            with reader:
                _require(all(getattr(reader, key) == getattr(self.context, key) for key in (
                    "release_path", "commit", "image", "manifest_sha256", "_manifest_bytes",
                    "database_image_id", "network_id", "database_generation")), "PAPER_OPS_READBACK_CONTEXT")
                with reader.invoke("readback", historical) as read:
                    raw = read.readback_artifact()
                    wire = read.result
                    value = journal._json(raw, journal.MAX_NATIVE_EXACT)
                    _require(type(value) is dict and set(value) == {
                        "schema", "kind", "request_sha256", "status", "account_id", "current_epoch",
                        "current_state", "event_id", "event_epoch", "result_canonical", "result_sha256", "reset"
                    } and value["schema"] == "owner-beta-paper-exact-readback-v1"
                             and value["kind"] == request["kind"]
                             and value["request_sha256"] == _digest(historical._sql)
                             and value["status"] == "COMMITTED"
                             and value["account_id"] == self.operation._intent["account_id"]
                             and type(value["current_epoch"]) is int
                             and value["current_epoch"] == current_epoch
                             and value["current_state"] == current_state and value["reset"] is None
                             and type(value["result_canonical"]) is str, "PAPER_OPS_READBACK")
                    result = value["result_canonical"].encode("ascii")
                    event, summary = _maintenance_result(result, action._action["verb"],
                        self.operation, action._artifact_bytes, self.attempt.fence)
                    epoch = self.attempt.fence + (action._action["verb"] == "off")
                    _require(type(value["event_epoch"]) is int and value["event_epoch"] == epoch
                             and value["event_id"] == event and value["result_sha256"] == _digest(result)
                             and wire["outcome"] == "REPLAY" and wire["error_code"] == "NONE"
                             and wire["generation"] == self.operation.generation
                             and wire["event_id"] == event and wire["control_epoch"] == current_epoch
                             and wire["artifact_sha256"] == _digest(raw), "PAPER_OPS_READBACK")
                    self.retain(historical.borrow_output_fd(), _digest(raw)[7:] + ".json", raw,
                                read._result_fd)
                _require(read._closed and read._cleanup_state == "REMOVED" and not read._preserve
                         and read._cid is None and read._child is None and not reader._invocations,
                         "PAPER_OPS_READBACK_CLEANUP")
            _require(reader._closed, "PAPER_OPS_READBACK_CONTEXT")
            return result, event, summary
        finally:
            historical.close()

    def resolve(self):
        call, action = self.call, self.action
        _require(call is not None and action is not None and call.custody is action,
                 "PAPER_OPS_UNRESOLVED")
        if not call._closed:
            if call._cid is not None:
                value = call._inspect_and_validate(started=None)
                state = value.get("State") or {}
                _require(state.get("Running") is False and state.get("Status") == "exited"
                         and type(state.get("ExitCode")) is int, "PAPER_OPS_UNRESOLVED")
                call._exit = state["ExitCode"]
            _require(call._child is None and call._cleanup_state not in {"CREATE_SENT", "UNKNOWN"},
                     "PAPER_OPS_UNRESOLVED")
            imported = call.import_dispatch()
            imported.close()
            _index, originals = journal._validate_v2_dispatch(action._dispatch_bytes,
                self.operation._intent_bytes, action._binding_bytes, action._artifact_bytes)
            for digest, data, _limit in originals:
                self.retain(action.borrow_output_fd(), digest[7:] + ".json", data)
            call.close()
        _require(call._closed and call._cid is None and call._child is None and call._imported,
                 "PAPER_OPS_UNRESOLVED")
        self.attempt.check(stopped=True)
        verb = action._action["verb"]
        raw, event, summary = self.readback(action, current_epoch=self.attempt.fence + (verb == "off"),
                                           current_state="OFF" if verb == "off" else "DRAINING")
        # Never manufacture a missing native result from a hash or report. The
        # exact original must exist; a retained FD also rejects same-byte swaps.
        retained = self.context._begin_result_fds.get(call)
        if call._result_bytes is not None:
            _require(call._result_bytes == raw and type(retained) is int
                     and _identity(os.fstat(retained)) == call._result_identity,
                     "PAPER_OPS_RESULT_CHANGED")
        self.retain(action.borrow_output_fd(), _digest(raw)[7:] + ".json", raw, retained)
        if summary is not None:
            self.retain(action.borrow_output_fd(), _digest(summary)[7:] + ".json", summary)
        if call._result is not None:
            _require(call._result["generation"] in {None, self.operation.generation}
                     and call._result["event_id"] in {None, event}
                     and call._result["control_epoch"] in {None, self.attempt.fence + (verb == "off")}
                     and call._result["artifact_sha256"] in {None, _digest(raw)}, "PAPER_OPS_RESULT_CHANGED")
        # The source-owned helper has exited and its exact commit is independently
        # known. Dispose only its retained temporary stages; preserve action input.
        for stage in call._stages:
            if stage is not call._persistent_stage and not stage.closed:
                stage.close(remove=True)
            self.context._retained_stages.discard(stage)
        call._preserve = False
        call._cleanup_state = "RETAINED_FOR_ACTION"
        if verb == "off":
            self.off_result = raw
        self.observe()
        artifact = summary if verb == "reconcile" else action._artifact_bytes
        self.operation._store_original(artifact, _digest(artifact), journal.MAX_SMALL)
        self.retain(self.operation._fd, _digest(artifact)[7:] + ".json", artifact)
        self.check()
        stage = dict(self.operation._last[3])
        target = "RECONCILED" if verb == "reconcile" else "OFF_COMMITTED"
        if self.head[2] == target:
            # An error after sealing (for example before floor publication) is
            # resolved against this retained original, never a refreshed stage.
            _require(stage["artifact_sha256"] == _digest(artifact) and stage["event_id"] == event
                     and stage["control_epoch"] == self.attempt.fence + (verb == "off"),
                     "PAPER_OPS_STAGE")
            stage_bytes = _canonical(stage)
        else:
            stage.update(sequence=self.head[0] + 1, previous_sha256=self.head[1],
                         stage=target, observed=_stage_observation(),
                         control_epoch=self.attempt.fence + (verb == "off"),
                         artifact_sha256=_digest(artifact), event_id=event)
            stage_bytes = _canonical(stage)
            stage_hash = self.operation.append(stage_bytes)
            self.head = (stage["sequence"], stage_hash, stage["stage"])
        self.retain(self.operation._fd, "%08d.json" % stage["sequence"], stage_bytes)
        self.retain(self.operation._fd, _digest(stage_bytes)[7:] + ".json", stage_bytes)
        self.check()
        if verb == "off":
            self.operation._journal.advance_floor(self.attempt.fence + 1)
            self.floor = self.attempt.fence + 1
            self.event = event
        else:
            self.summary = summary
        self.action = self.call = None
        self.observe()

    def run(self):
        _require(not self.running, "PAPER_OPS_UNRESOLVED")
        self.running = True
        try:
            if self.event is not None:
                self.observe()
                raw, event, _summary = self.readback(self.actions[-1], current_epoch=self.floor,
                                                    current_state="OFF")
                _require(raw == self.off_result and event == self.event, "PAPER_OPS_RESULT_CHANGED")
                self.observe()
                return self.event
            if self.call is not None:
                self.resolve()  # Only original pending readback/cleanup, never dispatch again.
            if self.summary is None:
                self.prepare("reconcile", self.attempt.inventory_bytes)
                self.resolve()
            if self.event is None:
                self.observe()
                root, summary = self.operation._intent, journal._json(self.summary, journal.MAX_SMALL)
                drain = {"schema": "owner-beta-paper-drain-receipt-v1",
                    "installation_id": root["installation_id"], "generation": root["generation"],
                    "account_id": root["account_id"], "request_key": root["generation"] + ":off",
                    "scope": root["scope"], "expected_epoch": root["expected_epoch"],
                    "resulting_epoch": self.attempt.fence, "intent_sha256": self.operation.intent_sha256,
                    "stage_sha256": self.head[1], "inventory_sha256": summary["inventory_sha256"],
                    "reconciliation_sha256": _digest(self.summary), "observation_id": summary["observation_id"],
                    "release_manifest_sha256": root["release_manifest_sha256"], "boot_id": self.attempt.boot,
                    "observed": _stage_observation()}
                self.prepare("off", _canonical(drain))
                self.resolve()
            return self.event
        finally:
            self.running = False

    def close(self):
        _require(self.call is None or (self.call._closed and self.call._cid is None
                 and self.call._child is None), "PAPER_OPS_UNRESOLVED")
        _require(all(reader._closed for reader in self.readers), "PAPER_OPS_READBACK_CLEANUP")
        for action in self.actions:
            action.close()
        while self.originals:
            parent, _name, fd, _identity_value, _data = self.originals.pop()
            os.close(fd)
            os.close(parent)


def commit_off(handle):
    """Consume one live drain through exact RECONCILE/OFF; return the OFF UUID."""
    _require(type(handle) is DrainedHandle, "PAPER_OPS_PRIVATE")
    attempt = handle._attempt
    _require(type(attempt) is _StopAttempt and not handle._closed and not attempt.closed
             and attempt.handle is handle
             and _STOP_ATTEMPTS.get(attempt.context, {}).get(attempt.invocation) is attempt,
             "PAPER_OPS_CLOSED")
    old = attempt.context.docker
    attempt.context.docker = _StopDocker(old, attempt.deadline)
    try:
        if attempt.consumer is None:
            handle.revalidate()
            attempt.consumer = _OffCommit(_STOP_TOKEN, handle)
        return attempt.consumer.run()
    finally:
        attempt.context.docker = old


class _ArchiveDeadline:
    """One finite capture lease; does not mutate either original stop clock."""
    def __init__(self):
        self.end = time.monotonic_ns() + 920_000_000_000

    def remaining(self, cap=15):
        left = (self.end - time.monotonic_ns()) / 1_000_000_000
        _require(left > 0, "PAPER_OPS_ARCHIVE_TIMEOUT")
        return min(cap, left)

    def work_remaining(self, cap=15):
        # Keep the final 20 seconds inside the SAME original 920-second bound
        # for captured-client stop/reap/absence. This never renews authority.
        left = (self.end - time.monotonic_ns()) / 1_000_000_000 - 20
        _require(left > 0, "PAPER_OPS_ARCHIVE_TIMEOUT")
        return min(cap, left)


class _ArchiveObserver(_HostStopObserver):
    def __init__(self, custody):
        # Borrow the exact held locks/generations. Only the new observer's clock
        # differs; the original observer and StopDeadline remain untouched.
        self.__dict__.update(custody.attempt.observer.__dict__)
        self.custody, self.deadline = custody, custody.deadline

    def _owned_archive_container(self, value):
        return self.custody.owner._owns_container(value)

    def _owned_archive_process(self, pid):
        return self.custody.owner._owns_process(pid)

    def close(self):
        _fail("PAPER_OPS_BORROWED_LOCKS")


class _ArchiveCustody:
    """Source-owned transfer of temporal scope, retaining every OFF predicate."""
    def __init__(self, token, handle):
        _require(token is _STOP_TOKEN and type(handle) is DrainedHandle, "PAPER_OPS_PRIVATE")
        self.handle, self.attempt = handle, handle._attempt
        module = sys.modules.get("owner_beta_paper_backup")
        space = getattr(module, "_SPACES", {}).get(self.attempt.context)
        _require(module is not None and type(space) is getattr(module, "_CaptureSpace", None)
                 and not space.closed and type(space.deadline) is _ArchiveDeadline,
                 "PAPER_OPS_ARCHIVE_RESERVATION")
        self.deadline = space.deadline  # the single clock charged before reservation I/O
        self.owner = None
        self.closed = self.poisoned = False
        attempt = self.attempt
        _require(attempt.archive is None, "PAPER_OPS_ARCHIVE_REPLAY")
        handle.revalidate()  # ORIGINAL stop deadline, zero clients, full proof
        c = attempt.consumer
        _require(type(c) is _OffCommit and c.event is not None and c.off_result is not None
                 and c.head[2] == "OFF_COMMITTED" and c.call is None and c.action is None
                 and not c.running and all(r._closed for r in c.readers)
                 and attempt.operation._intent["purpose"] == "BACKUP"
                 and attempt.scope == "COMBINED_BACKUP", "PAPER_OPS_ARCHIVE_OFF")
        c.check()
        self.event, self.head, self.floor = c.event, c.head, c.floor
        self.observer = _ArchiveObserver(self)
        attempt.archive = self
        self.deadline.work_remaining()

    def __reduce__(self):
        _fail("PAPER_OPS_PRIVATE")

    def __copy__(self):
        _fail("PAPER_OPS_PRIVATE")

    def __deepcopy__(self, _memo):
        _fail("PAPER_OPS_PRIVATE")

    def bind(self, owner):
        module = sys.modules.get("owner_beta_paper_backup")
        _require(self.owner is None and module is not None
                 and type(owner) is getattr(module, "_HeldPaperSnapshot", None)
                 and owner._token is getattr(module, "_HELD_PAPER_SNAPSHOT_TOKEN", None)
                 and owner._custody is self, "PAPER_OPS_PRIVATE")
        self.owner = owner

    def native_invocation(self):
        """The one registered archive helper, not an arbitrary active-call waiver."""
        module = sys.modules.get("owner_beta_paper_backup")
        _require(module is not None and self.owner is not None
                 and module._OWNERS.get(self) is self.owner and self.owner._custody is self
                 and not self.closed and not self.poisoned, "PAPER_OPS_ARCHIVE_CLOSED")
        package = self.owner.package
        if package is None:
            return None
        _require(type(package) is module._ArchivePackage and package in module._PACKAGES
                 and package.owner is self.owner and not package.closed,
                 "PAPER_OPS_PRIVATE")
        call = package.call
        if call is not None:
            _require(type(call) is _NativeInvocation and call._archive_owner is package
                     and call._context is self.attempt.context and not call._closed, "PAPER_OPS_PRIVATE")
            if call._restore_owner is None:
                _require(call.verb == "readback" and call.custody is package.custody
                         and type(call.custody) is journal.Custody and call.custody._readonly, "PAPER_OPS_PRIVATE")
            else:
                destination = package.destination
                _require(type(destination) is module._DestinationRestore and call._restore_owner is destination
                         and call.custody is destination.active_custody
                         and call.verb in {"begin", "reconcile", "restore-off", "readback"}, "PAPER_OPS_PRIVATE")
        return call

    def revalidate(self):
        _require(not self.closed and not self.poisoned and self.owner is not None
                 and self.attempt.archive is self and self.attempt.handle is self.handle
                 and not self.handle._closed
                 and _STOP_ATTEMPTS.get(self.attempt.context, {}).get(self.attempt.invocation) is self.attempt,
                 "PAPER_OPS_ARCHIVE_CLOSED")
        context, attempt = self.attempt.context, self.attempt
        old = context.docker
        context.docker = _StopDocker(old, self.deadline)
        try:
            self.deadline.work_remaining()
            attempt._check_custody(stopped=True)
            _require((attempt.consumer.event, attempt.consumer.head, attempt.consumer.floor) ==
                     (self.event, self.head, self.floor), "PAPER_OPS_ARCHIVE_CHANGED")
            self.owner._check_resources()
            self.observer.terminal_observation()
            self.owner._match_clients(self.observer.database("observe")["backends"])
            attempt._check_custody(stopped=True)
            self.deadline.work_remaining()
        except BaseException:
            self.poisoned = True
            raise
        finally:
            context.docker = old

    def seal_snapshot(self, candidate):
        """Transition after source selection and every owned helper are released."""
        module=sys.modules.get("owner_beta_paper_backup")
        _require(module is not None and type(candidate) is module._PaperPublication
            and candidate.owner is self.owner and candidate.generation == self.attempt.operation.generation
            and self.owner._resources_closed() and self.owner.selected is None
            and self.owner._selection_released and self.owner.secret_sources is None
            and self.owner._snapshot is None and self.head[2] == "OFF_COMMITTED", "PAPER_OPS_ARCHIVE_OFF")
        self.revalidate()
        candidate.require_verified()
        owner,c,operation=self.owner,self.attempt.consumer,self.attempt.operation
        fd,_identity,size,digest=owner.spools["archive.json"]
        _require(digest == candidate.archive_sha256,"PAPER_OPS_ARCHIVE_CHANGED")
        raw=os.pread(fd,size+1,0)
        _require(_digest(raw)==digest,"PAPER_OPS_ARCHIVE_CHANGED")
        operation._store_original(raw,digest,journal.MAX_SMALL)
        value=dict(schema="owner-beta-paper-operation-stage-v1",installation_id=operation._intent["installation_id"],
            generation=operation.generation,account_id=operation._intent["account_id"],sequence=self.head[0]+1,
            previous_sha256=self.head[1],intent_sha256=operation.intent_sha256,stage="SNAPSHOT",
            boot_id=_current_boot_id(),observed=_stage_observation(),control_epoch=self.floor,
            artifact_sha256=digest,event_id=None,error_code="NONE")
        stage=_canonical(value)
        stage_hash=operation.append(stage)
        # This is the sole legitimate extension of the retained OFF head. A
        # later source observer checks the appended original and the same floor.
        c.head=self.head=(value["sequence"],stage_hash,"SNAPSHOT")
        c.retain(operation._fd,"%08d.json"%value["sequence"],stage)
        c.retain(operation._fd,stage_hash[7:]+".json",stage)
        self.revalidate()

    def close(self):
        _require(self.owner is not None and self.owner._resources_closed(),
                 "PAPER_OPS_ARCHIVE_CLEANUP_UNKNOWN")
        self.closed = True


def _open_archive_custody(handle):
    return _ArchiveCustody(_STOP_TOKEN, handle)


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
    temporary = _Stage("owner-beta-paper-inspect-",
                       root=getattr(context, "_lifecycle_stage_root", None) or "/run")
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


def main(argv=None):
    """Operator routing uses the separate per-action retained lifecycle owner."""
    lifecycle = _load_local("owner_beta_paper_lifecycle", "owner-beta-paper-lifecycle.py")
    return lifecycle.main(argv)


if __name__ == "__main__":
    sys.modules["owner_beta_paper_operations"] = sys.modules[__name__]
    try:
        raise SystemExit(main())
    except Exception as error:
        lifecycle = sys.modules.get("owner_beta_paper_lifecycle")
        if lifecycle is not None and isinstance(error, lifecycle.LifecycleError):
            print(error.code, file=sys.stderr)
            raise SystemExit(4 if error.code == "CLEANUP_UNKNOWN" else 3)
        print("PAPER_LIFECYCLE_REJECTED", file=sys.stderr)
        raise SystemExit(3)
