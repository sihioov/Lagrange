#!/usr/bin/env python3
"""Closed local release/state preparation. Never starts a service or opens a DB."""
from __future__ import annotations

import argparse
import contextlib
import ctypes
import datetime
import errno
import fcntl
import hashlib
import json
import os
import re
import stat
import sys
import unicodedata
import uuid

CONFIG_LIMIT = 16 * 1024
POLICY_LIMIT = 256 * 1024
READER_GROUP = 10001
CONFIG_DIRECTORY = ".owner-beta-daily"
# Prepared canonical policy; operational activation still requires separate inputs.
POLICY_SHA256 = "sha256:e9149ef56170877e5e9646f1fff1c370b3a34bf2c8b8ee8272bc30bd5fa8779c"
READER_BYTES = b'{"schema":"owner-beta-daily-reader-v1","input_kind":"owner_beta_daily_etf11_price_only_v1","required_migration_versions":[60,61,62],"historical_v2_v3":true,"preserve_daily_history":true}'
FLOOR_BYTES = b'{"schema":"owner-beta-daily-reader-floor-v1","reader":"owner-beta-daily-reader-v1"}'
FLOOR_NAME = ".owner-beta-daily-reader-floor"
PATHS = {
    "policy_path": "/run/owner-beta-daily/policy.json",
    "historical_root": "/data/owner-beta-artifacts",
    "raw_root": "/data/raw",
    "candidate_root": "/data/owner-beta-daily/candidates",
    "admitted_root": "/data/owner-beta-daily/admitted",
    "frontier_root": "/data/owner-beta-daily/frontiers",
    "producer_state_root": "/data/owner-beta-daily/producer-state",
    "admitter_state_root": "/data/owner-beta-daily/admitter-state",
}
SCAN_KEYS = (
    "max_manifest_bytes", "max_manifest_records", "max_batches",
    "max_files_per_batch", "max_files", "max_batch_metadata_bytes",
    "max_source_file_bytes", "max_retained_bytes", "max_open_descriptors",
    "max_data_root_bytes", "max_data_root_components", "max_metadata_json_depth",
)
STAGES = {"producer-state": ("PRODUCER", 10002), "admitter-state": ("ADMITTER", 10003)}
SHARED = {"candidates": 10002, "admitted": 10003, "frontiers": 10003}
DIR_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
FILE_FLAGS = os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW | os.O_CLOEXEC


class DailyInstallError(Exception):
    """Only fixed diagnostic codes cross the command boundary."""


def reject(code="DAILY_INSTALL_INVALID"):
    raise DailyInstallError(code)


def require_root():
    if sys.platform != "linux" or os.geteuid() != 0 or sys.maxsize <= 2**32:
        reject("DAILY_INSTALL_PLATFORM")


def canonical(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def pairs(values):
    result = {}
    for key, value in values:
        if key in result:
            reject()
        result[key] = value
    return result


def json_bytes(data, maximum=CONFIG_LIMIT):
    if len(data) > maximum:
        reject()
    try:
        return json.loads(data.decode("utf-8"), object_pairs_hook=pairs,
                          parse_constant=lambda _: reject())
    except (UnicodeError, ValueError, RecursionError):
        reject()


def is_hash(value):
    return isinstance(value, str) and re.fullmatch(r"sha256:[0-9a-f]{64}", value) is not None


def is_commit(value):
    return (isinstance(value, str) and re.fullmatch(r"[0-9a-f]{40}", value) is not None
            and value != "0" * 40)


def is_uuid(value):
    if not isinstance(value, str):
        return False
    try:
        parsed = uuid.UUID(value)
        return parsed.int != 0 and str(parsed) == value
    except ValueError:
        return False


def positive(value, maximum):
    return type(value) is int and 1 <= value <= maximum


def trading_date(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", value):
        reject()
    try:
        return datetime.date.fromisoformat(value)
    except ValueError:
        reject()


def validate_config(data, commit):
    value = json_bytes(data)
    keys = {"schema", "owner", "policy_sha256", *PATHS, "raw_owner_uid", "scan",
            "max_artifact_entries", "max_replay_admissions", "max_cycle_seconds",
            "code_commit", "entitlement_reference", "entitlement_sha256", "first_auto_session"}
    if not isinstance(value, dict) or set(value) != keys:
        reject()
    if (value["schema"] != "owner-beta-daily-process-v1" or not is_uuid(value["owner"])
            or value["policy_sha256"] != POLICY_SHA256
            or not is_commit(commit) or value["code_commit"] != commit
            or any(value[k] != expected for k, expected in PATHS.items())):
        reject()
    if type(value["raw_owner_uid"]) is not int or not 0 <= value["raw_owner_uid"] < 2**32 - 1:
        reject()
    ref = value["entitlement_reference"]
    if (not isinstance(ref, str) or not 1 <= len(ref) <= 512 or ref.strip() != ref
            or any(unicodedata.category(c) == "Cc" for c in ref)):
        reject()
    try:
        ref_bytes = ref.encode("utf-8")
    except UnicodeError:
        reject()
    if len(ref_bytes) > 4096 or not is_hash(value["entitlement_sha256"]):
        reject()
    if trading_date(value["first_auto_session"]) <= datetime.date(2026, 8, 28):
        reject()
    if not all(positive(value[k], cap) for k, cap in (
            ("max_artifact_entries", 20000), ("max_replay_admissions", 20000),
            ("max_cycle_seconds", 3600))):
        reject()
    scan = value["scan"]
    if not isinstance(scan, dict) or set(scan) != set(SCAN_KEYS):
        reject()
    # Operational budgets remain explicit. Bounded host-side grammar rejects
    # integer overflow; the Rust constructors remain authoritative at startup.
    for key in SCAN_KEYS:
        cap = (2**63 - 2) if key.endswith("_bytes") else (2**31 - 1)
        if key == "max_source_file_bytes":
            cap = 67108864
        if key == "max_metadata_json_depth":
            cap = 32
        if not positive(scan[key], cap):
            reject()
    return value


def path_parts(path):
    if (not isinstance(path, str) or not path.startswith("/") or path == "/"
            or path.endswith("/") or "\x00" in path
            or any(p in ("", ".", "..") for p in path.split("/")[1:])):
        reject("DAILY_INSTALL_PATH")
    return path.split("/")[1:]


def no_acl(fd):
    for name in ("system.posix_acl_access", "system.posix_acl_default"):
        try:
            os.getxattr(fd, name)
        except OSError as error:
            if error.errno in (errno.ENODATA, errno.ENOTSUP, errno.EOPNOTSUPP):
                continue
            raise
        reject("DAILY_INSTALL_ACL")


def directory_info(fd, uid=0, gid=None, mode=None):
    value = os.fstat(fd)
    if (not stat.S_ISDIR(value.st_mode) or value.st_uid != uid
            or (gid is not None and value.st_gid != gid)
            or value.st_mode & 0o022
            or (mode is not None and stat.S_IMODE(value.st_mode) != mode)):
        reject("DAILY_INSTALL_PATH")
    no_acl(fd)
    return value


@contextlib.contextmanager
def root_directory(path):
    # Retain every directory descriptor so the final check still addresses the
    # same chain. All ancestors, including '/', are root-owned and ACL-free.
    names = path_parts(path)
    descriptors = [os.open("/", DIR_FLAGS)]
    identities = []
    try:
        identities.append(directory_info(descriptors[0]))
        for name in names:
            child = os.open(name, DIR_FLAGS, dir_fd=descriptors[-1])
            descriptors.append(child)
            identities.append(directory_info(child))
        yield descriptors[-1]
        for index, fd in enumerate(descriptors):
            before = identities[index]
            after = directory_info(fd)
            if (before.st_dev, before.st_ino, before.st_uid, before.st_gid, before.st_mode) != (
                    after.st_dev, after.st_ino, after.st_uid, after.st_gid, after.st_mode):
                reject("DAILY_INSTALL_CHANGED")
            if index:
                named = os.stat(names[index - 1], dir_fd=descriptors[index - 1],
                                follow_symlinks=False)
                if (named.st_dev, named.st_ino) != (after.st_dev, after.st_ino):
                    reject("DAILY_INSTALL_CHANGED")
    finally:
        for fd in reversed(descriptors):
            os.close(fd)


def file_info(fd, uid, gid=None, mode=None):
    value = os.fstat(fd)
    if (not stat.S_ISREG(value.st_mode) or value.st_uid != uid or value.st_nlink != 1
            or (gid is not None and value.st_gid != gid) or value.st_mode & 0o022
            or (mode is not None and stat.S_IMODE(value.st_mode) != mode)):
        reject("DAILY_INSTALL_FILE")
    no_acl(fd)
    return value


def file_identity(value):
    return (value.st_dev, value.st_ino, value.st_uid, value.st_gid, value.st_mode,
            value.st_nlink, value.st_size, value.st_mtime_ns, value.st_ctime_ns)


def read_file(parent, name, uid=0, gid=None, mode=None, maximum=CONFIG_LIMIT):
    fd = os.open(name, FILE_FLAGS, dir_fd=parent)
    try:
        before = file_info(fd, uid, gid, mode)
        if before.st_size > maximum:
            reject("DAILY_INSTALL_SIZE")
        data = bytearray()
        while len(data) <= maximum:
            chunk = os.read(fd, min(65536, maximum + 1 - len(data)))
            if not chunk:
                break
            data.extend(chunk)
        after = file_info(fd, uid, gid, mode)
        named = os.stat(name, dir_fd=parent, follow_symlinks=False)
        if (len(data) != before.st_size or file_identity(before) != file_identity(after)
                or file_identity(after) != file_identity(named)):
            reject("DAILY_INSTALL_CHANGED")
        return bytes(data)
    finally:
        os.close(fd)


def read_root_file(path, gid=None, mode=None, maximum=CONFIG_LIMIT):
    parts = path_parts(path)
    parent = "/" + "/".join(parts[:-1])
    # Protected inputs never live directly at filesystem root.
    with root_directory(parent) as fd:
        return read_file(fd, parts[-1], gid=gid, mode=mode, maximum=maximum)


def create_file(parent, name, data, uid, gid, mode):
    fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                 0o600, dir_fd=parent)
    try:
        offset = 0
        while offset < len(data):
            written = os.write(fd, data[offset:])
            if written <= 0:
                reject("DAILY_INSTALL_WRITE")
            offset += written
        os.fchown(fd, uid, gid)
        os.fchmod(fd, mode)
        os.fsync(fd)
    finally:
        os.close(fd)
    os.fsync(parent)


def validate_policy(data):
    if len(data) > POLICY_LIMIT or "sha256:" + hashlib.sha256(data).hexdigest() != POLICY_SHA256:
        reject("DAILY_INSTALL_POLICY")
    value = json_bytes(data, POLICY_LIMIT)
    if canonical(value) != data:
        reject("DAILY_INSTALL_POLICY")
    return value


def stage_config(stage, source, policy_source, commit):
    require_root()
    data = read_root_file(source, gid=0, mode=0o600)
    config = validate_config(data, commit)
    policy = read_root_file(policy_source, maximum=POLICY_LIMIT)
    validate_policy(policy)
    with root_directory(stage) as parent:
        os.mkdir(CONFIG_DIRECTORY, 0o700, dir_fd=parent)
        fd = os.open(CONFIG_DIRECTORY, DIR_FLAGS, dir_fd=parent)
        try:
            create_file(fd, "config.json", data, 0, READER_GROUP, 0o440)
            create_file(fd, "policy.json", policy, 0, READER_GROUP, 0o440)
            os.fchown(fd, 0, READER_GROUP)
            os.fchmod(fd, 0o750)
            os.fsync(fd)
        finally:
            os.close(fd)
        os.fsync(parent)
    return config


def verify_config(release, commit):
    require_root()
    with root_directory(release + "/" + CONFIG_DIRECTORY) as fd:
        directory_info(fd, 0, READER_GROUP, 0o750)
        if set(os.listdir(fd)) != {"config.json", "policy.json"}:
            reject("DAILY_INSTALL_CONFIG")
        data = read_file(fd, "config.json", 0, READER_GROUP, 0o440)
        validate_policy(read_file(fd, "policy.json", 0, READER_GROUP, 0o440, POLICY_LIMIT))
        return validate_config(data, commit), "sha256:" + hashlib.sha256(data).hexdigest()


def checkpoint(stage, config):
    return canonical({
        "schema": "owner-beta-daily-checkpoint-v1", "stage": stage,
        "owner": config["owner"], "policy_sha256": config["policy_sha256"],
        "generation": 0, "last_started_unix_millis": None, "pending": None,
    })


def validate_checkpoint(data, stage, config):
    value = json_bytes(data)
    keys = {"schema", "stage", "owner", "policy_sha256", "generation",
            "last_started_unix_millis", "pending"}
    if (not isinstance(value, dict) or set(value) != keys
            or value["schema"] != "owner-beta-daily-checkpoint-v1"
            or value["stage"] != stage or value["owner"] != config["owner"]
            or value["policy_sha256"] != config["policy_sha256"]):
        reject("DAILY_INSTALL_STATE")
    generation = value["generation"]
    last = value["last_started_unix_millis"]
    pending = value["pending"]
    if (type(generation) is not int or not 0 <= generation <= 2**64 - 1
            or (last is not None and (type(last) is not int or not 0 <= last <= 2**63 - 1))
            or (generation == 0 and (last is not None or pending is not None))
            or (generation > 0 and last is None)):
        reject("DAILY_INSTALL_STATE")
    if pending is not None:
        if not isinstance(pending, dict):
            reject("DAILY_INSTALL_STATE")
        if stage == "ADMITTER":
            if set(pending) != {"kind", "receipt_id"} or pending["kind"] != "RECEIPT" or not is_uuid(pending["receipt_id"]):
                reject("DAILY_INSTALL_STATE")
        elif stage == "PRODUCER":
            if (set(pending) != {"kind", "content_sha256", "manifest_sha256",
                                 "parent_admission_sha256", "session", "producer_commit"}
                    or pending["kind"] != "CANDIDATE"
                    or not all(is_hash(pending[k]) for k in ("content_sha256", "manifest_sha256"))
                    or (pending["parent_admission_sha256"] is not None
                        and not is_hash(pending["parent_admission_sha256"]))
                    or not is_commit(pending["producer_commit"])):
                reject("DAILY_INSTALL_STATE")
            trading_date(pending["session"])
        else:
            reject("DAILY_INSTALL_STATE")
    # Field order is part of the Rust checkpoint byte contract.
    ordered = {k: value[k] for k in ("schema", "stage", "owner", "policy_sha256",
                                    "generation", "last_started_unix_millis", "pending")}
    if pending is not None:
        order = (("kind", "receipt_id") if stage == "ADMITTER" else
                 ("kind", "content_sha256", "manifest_sha256",
                  "parent_admission_sha256", "session", "producer_commit"))
        ordered["pending"] = {k: pending[k] for k in order}
    if canonical(ordered) != data:
        reject("DAILY_INSTALL_STATE")
    return value


@contextlib.contextmanager
def child_directory(parent, name, uid, gid, mode):
    fd = os.open(name, DIR_FLAGS, dir_fd=parent)
    try:
        before = directory_info(fd, uid, gid, mode)
        yield fd
        after = directory_info(fd, uid, gid, mode)
        named = os.stat(name, dir_fd=parent, follow_symlinks=False)
        if ((before.st_dev, before.st_ino) != (after.st_dev, after.st_ino)
                or (after.st_dev, after.st_ino) != (named.st_dev, named.st_ino)):
            reject("DAILY_INSTALL_CHANGED")
    finally:
        os.close(fd)


@contextlib.contextmanager
def state_lock(parent, uid):
    fd = os.open("poll.lock", FILE_FLAGS, dir_fd=parent)
    try:
        before = file_info(fd, uid, uid, 0o600)
        if before.st_size != 0:
            reject("DAILY_INSTALL_STATE")
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            reject("DAILY_INSTALL_BUSY")
        yield
        after = file_info(fd, uid, uid, 0o600)
        named = os.stat("poll.lock", dir_fd=parent, follow_symlinks=False)
        if file_identity(before) != file_identity(after) or file_identity(after) != file_identity(named):
            reject("DAILY_INSTALL_CHANGED")
    finally:
        os.close(fd)


def check_state_fd(parent, config):
    directory_info(parent, 0, READER_GROUP, 0o750)
    members = set(os.listdir(parent))
    if members not in (set(STAGES) | set(SHARED), set(STAGES) | set(SHARED) | {"historical-base"}):
        reject("DAILY_INSTALL_STATE")
    with contextlib.ExitStack() as stack:
        if "historical-base" in members:
            # Byte/pin verification belongs to the base-view gate before enable.
            # Preserve this immutable subtree during state checks and backup.
            views = stack.enter_context(child_directory(parent, "historical-base", 0, READER_GROUP, 0o750))
            if set(os.listdir(views)) != {"producer", "admitter"}:
                reject("DAILY_INSTALL_STATE")
            for name, uid in (("producer", 10002), ("admitter", 10003)):
                stack.enter_context(child_directory(views, name, uid, uid, 0o750))
        for name, uid in SHARED.items():
            stack.enter_context(child_directory(parent, name, uid, READER_GROUP, 0o750))
        for name, (stage, uid) in STAGES.items():
            fd = stack.enter_context(child_directory(parent, name, uid, uid, 0o700))
            stack.enter_context(state_lock(fd, uid))
            # A crash residue belongs to runtime recovery, never to an installer reset.
            if set(os.listdir(fd)) != {"poll.lock", "checkpoint.json"}:
                reject("DAILY_INSTALL_STATE")
            validate_checkpoint(read_file(fd, "checkpoint.json", uid, uid, 0o600),
                                stage, config)


def check_state(data_root, config):
    require_root()
    with root_directory(data_root + "/owner-beta-daily") as parent:
        check_state_fd(parent, config)


def rename_no_replace(source_fd, source, target_fd, target):
    function = getattr(ctypes.CDLL(None, use_errno=True), "renameat2", None)
    if function is None:
        reject("DAILY_INSTALL_PLATFORM")
    function.argtypes = (ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint)
    function.restype = ctypes.c_int
    if function(source_fd, os.fsencode(source), target_fd, os.fsencode(target), 1) != 0:
        raise OSError(ctypes.get_errno(), "DAILY_INSTALL_PUBLISH")
    os.fsync(source_fd)
    os.fsync(target_fd)


def provision_state(data_root, config):
    require_root()
    # Root-owned sibling of service-owned artifacts: no writable ancestor.
    with root_directory(data_root) as parent:
        try:
            os.stat("owner-beta-daily", dir_fd=parent, follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            with child_directory(parent, "owner-beta-daily", 0, READER_GROUP, 0o750) as current:
                check_state_fd(current, config)
            return "REUSED"
        stage_name = ".owner-beta-daily-stage-" + uuid.uuid4().hex
        os.mkdir(stage_name, 0o700, dir_fd=parent)
        stage = os.open(stage_name, DIR_FLAGS, dir_fd=parent)
        published = False
        try:
            # Keep prepared service-owned children behind a root-only staging
            # ancestor until the complete root is published with RENAME_NOREPLACE.
            os.fchown(stage, 0, 0)
            os.fchmod(stage, 0o700)
            directory_info(stage, 0, 0, 0o700)
            os.mkdir("root", 0o700, dir_fd=stage)
            root = os.open("root", DIR_FLAGS, dir_fd=stage)
            try:
                for name, uid in SHARED.items():
                    os.mkdir(name, 0o700, dir_fd=root)
                    fd = os.open(name, DIR_FLAGS, dir_fd=root)
                    try:
                        os.fchown(fd, uid, READER_GROUP)
                        os.fchmod(fd, 0o750)
                        os.fsync(fd)
                    finally:
                        os.close(fd)
                for name, (kind, uid) in STAGES.items():
                    os.mkdir(name, 0o700, dir_fd=root)
                    fd = os.open(name, DIR_FLAGS, dir_fd=root)
                    try:
                        create_file(fd, "poll.lock", b"", uid, uid, 0o600)
                        create_file(fd, "checkpoint.json", checkpoint(kind, config), uid, uid, 0o600)
                        os.fchown(fd, uid, uid)
                        os.fchmod(fd, 0o700)
                        os.fsync(fd)
                    finally:
                        os.close(fd)
                os.fchown(root, 0, READER_GROUP)
                os.fchmod(root, 0o750)
                os.fsync(root)
                check_state_fd(root, config)
            finally:
                os.close(root)
            os.fsync(stage)
            rename_no_replace(stage, "root", parent, "owner-beta-daily")
            published = True
        finally:
            os.close(stage)
            if published:
                os.rmdir(stage_name, dir_fd=parent)
                os.fsync(parent)
            # On failure retain the root-only unpublished staging tree. Do not
            # recursively delete or repair bytes after an uncertain publication.
        check_state(data_root, config)
        return "CREATED"


def reader_guard(install_root, target_release, mode, reserve=True):
    require_root()
    if mode not in ("disabled", "owner_only"):
        reject("DAILY_INSTALL_MODE")
    with root_directory(install_root) as parent:
        try:
            floor = read_file(parent, FLOOR_NAME, 0, 0, 0o600)
        except FileNotFoundError:
            floor = None
        if floor is not None and floor != FLOOR_BYTES:
            reject("DAILY_INSTALL_READER_FLOOR")
        if floor is None and mode == "disabled":
            return "LEGACY_OFF"
        reader = read_root_file(target_release + "/configs/evidence/owner-beta-daily-reader-v1.json")
        if reader != READER_BYTES:
            reject("DAILY_INSTALL_READER")
        if floor is None and reserve:
            # A durable conservative floor may remain after a later activation
            # failure. It is never removed just to permit an old-reader rollback.
            create_file(parent, FLOOR_NAME, FLOOR_BYTES, 0, 0, 0o600)
        return "DAILY_READER_REQUIRED"


def control_number(value, maximum):
    if (not isinstance(value, str) or len(value) > 19
            or re.fullmatch(r"[1-9][0-9]*", value) is None
            or int(value) > maximum):
        reject("DAILY_CONTROL_INPUT")
    return value


def render_control(release, commit, action, capacity=None, lease=None, backoff=None):
    # The official runtime caller owns DB execution and checks its final exit.
    # This helper only emits fixed SQL; it cannot connect, grant or start work.
    require_root()
    if action not in ("prepare", "check", "check-disabled", "enable", "disable"):
        reject("DAILY_CONTROL_INPUT")
    if action == "disable":
        if any(value is not None for value in (capacity, lease, backoff)):
            reject("DAILY_CONTROL_INPUT")
        capacity = lease = backoff = "NULL"
    else:
        capacity = control_number(capacity, 2**31 - 1)
        lease = control_number(lease, 2**31 - 1)
        backoff = control_number(backoff, 2**63 - 1)
    config, _ = verify_config(release, commit)
    policy = read_root_file(release + "/" + CONFIG_DIRECTORY + "/policy.json",
                           gid=READER_GROUP, mode=0o440, maximum=POLICY_LIMIT)
    validate_policy(policy)
    template = read_root_file(release + "/deploy/db/owner-beta-daily-control.sql",
                              maximum=64 * 1024).decode("ascii")
    replacements = {
        "__ACTION__": action, "__CONFIG_HEX__": canonical(config).hex(),
        "__POLICY_HEX__": policy.hex(), "__COMMIT__": commit,
        "__CAPACITY__": capacity, "__LEASE__": lease, "__BACKOFF__": backoff,
    }
    for token, value in replacements.items():
        if template.count(token) != 1:
            reject("DAILY_CONTROL_TEMPLATE")
        template = template.replace(token, value)
    if re.search(r"__[A-Z_]+__", template):
        reject("DAILY_CONTROL_TEMPLATE")
    return template


class ClosedArgumentParser(argparse.ArgumentParser):
    def error(self, message):
        # argparse's default error would echo untrusted argument values.
        reject("DAILY_INSTALL_ARGUMENTS")


def main(argv=None):
    parser = ClosedArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    control = commands.add_parser("render-control")
    control.add_argument("--release", required=True)
    control.add_argument("--commit", required=True)
    control.add_argument("--control-action", required=True,
                         choices=("prepare", "check", "check-disabled", "enable", "disable"))
    control.add_argument("--capacity")
    control.add_argument("--lease")
    control.add_argument("--backoff")
    prepare = commands.add_parser("stage")
    prepare.add_argument("--stage", required=True)
    prepare.add_argument("--source", required=True)
    prepare.add_argument("--policy-source", required=True)
    prepare.add_argument("--commit", required=True)
    for name in ("verify", "provision-state", "check-state"):
        command = commands.add_parser(name)
        command.add_argument("--release", required=True)
        command.add_argument("--commit", required=True)
        if name != "verify":
            command.add_argument("--data-root", required=True)
    for action in ("reader-guard", "reader-check"):
        guard = commands.add_parser(action)
        guard.add_argument("--install-root", required=True)
        guard.add_argument("--target-release", required=True)
        guard.add_argument("--mode", choices=("disabled", "owner_only"), required=True)
    try:
        args = parser.parse_args(argv)
        require_root()
        if args.action == "render-control":
            sys.stdout.write(render_control(args.release, args.commit, args.control_action,
                                            args.capacity, args.lease, args.backoff))
        elif args.action == "stage":
            stage_config(args.stage, args.source, args.policy_source, args.commit)
            print("DAILY_INSTALL_STAGE: PASS")
        elif args.action in ("reader-guard", "reader-check"):
            # Caller validates manifest/commit and config before this pre-switch
            # guard. This command never changes the current release symlink.
            print("DAILY_INSTALL_READER: " + reader_guard(
                args.install_root, args.target_release, args.mode,
                reserve=args.action == "reader-guard"))
        else:
            config, digest = verify_config(args.release, args.commit)
            if args.action == "verify":
                print(digest)
            elif args.action == "provision-state":
                print("DAILY_INSTALL_STATE: " + provision_state(args.data_root, config))
            else:
                check_state(args.data_root, config)
                print("DAILY_INSTALL_STATE: PASS")
        return 0
    except DailyInstallError as error:
        print(str(error), file=sys.stderr)
        return 1
    except (OSError, ValueError, TypeError, OverflowError, RecursionError):
        print("DAILY_INSTALL_IO_OR_INVALID", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
