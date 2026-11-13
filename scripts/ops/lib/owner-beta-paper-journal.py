#!/usr/bin/env python3
"""Protected, append-only local custody for owner-beta Paper operations.

This module stores evidence only.  It does not prove database state, stop a
writer, authorize a transition, or confer lifecycle authority.
"""
from __future__ import annotations

import contextlib
import datetime
import errno
import fcntl
import hashlib
import json
import os
import re
import stat
import sys
import uuid

MAX_SMALL = 65_536
MAX_NATIVE_EXACT = 1_048_576
MAX_NATIVE_EMBEDDED = 262_144
# A native-dispatch JSON string embeds its <=1 MiB exact request and repeats
# the <=64 KiB intent/artifact strings. Doubling escaped request bytes plus
# those repeated preimages and envelope overhead stays below this derived cap.
MAX_NATIVE_INDEX = 3 * 1_048_576
MAX_I64 = (1 << 63) - 1
PAPER_DIR = ".owner-beta-paper"
LOCK_NAME = ".lagrange-release.lock"
DIR_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
READ_FLAGS = os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW | os.O_CLOEXEC
UUID_RE = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\Z")
HASH_RE = re.compile(r"sha256:[0-9a-f]{64}\Z")
HEX_RE = re.compile(r"[0-9a-f]{64}\Z")
GEN_RE = re.compile(r"[0-9a-f-]{36}\Z")
STAGE_FILE_RE = re.compile(r"([0-9]{8,})\.json\Z")
TEMP_RE = re.compile(r"\..+\.tmp\.[0-9a-f-]{36}\Z")
ACTION_DIR_RE = re.compile(r"[0-9]{8,}\Z")
CALLER_KEYS = {"schema", "installation_id", "account_id", "caller_key", "generation",
               "root_intent_sha256", "command_canonical"}
ACTION_KEYS = {"schema", "installation_id", "generation", "root_intent_sha256", "sequence",
               "previous_binding_sha256", "verb", "request_key", "expected_epoch",
               "artifact_sha256", "stage_sha256"}
V2_DISPATCH_KEYS = {"schema", "root_intent_canonical", "root_intent_sha256", "action_canonical",
                    "action_sha256", "artifact_canonical", "artifact_sha256",
                    "sql_request_canonical", "sql_request_sha256"}
INTENT_KEYS = {
    "schema", "installation_id", "generation", "account_id", "predecessor_generation",
    "request_key", "purpose", "scope", "expected_epoch", "local_epoch_floor",
    "release_manifest_sha256", "artifact_sha256", "resume_target_id",
}
STAGE_KEYS = {
    "schema", "installation_id", "generation", "account_id", "sequence", "previous_sha256",
    "intent_sha256", "stage", "boot_id", "observed", "control_epoch", "artifact_sha256",
    "event_id", "error_code",
}
STAGES = ("INTENT", "FENCED", "STOPPED", "RECONCILED", "OFF_COMMITTED",
          "SNAPSHOT", "RESTORE_VERIFIED", "RESTORED_COMMITTED")
ERROR_CODES = {"NONE", "WIRE_INVALID", "IDENTITY_CONFLICT", "STALE_EPOCH", "CLOCK_INVALID",
               "WRITER_UNKNOWN", "COMMIT_UNKNOWN", "READER_INCOMPATIBLE",
               "RECOVERY_AUTHORITY_UNAVAILABLE", "EVIDENCE_UNAVAILABLE", "INTERNAL_FAILURE"}


class JournalError(Exception):
    """Failure without provider text or secret-bearing diagnostics."""
    def __init__(self, code="JOURNAL_INVALID"):
        self.code = code
        super().__init__(code)


def _fail(code="JOURNAL_INVALID"):
    raise JournalError(code)


def _require_root():
    if sys.platform != "linux" or os.geteuid() != 0 or sys.maxsize <= 2**32:
        _fail("JOURNAL_PLATFORM")


def _pairs(items):
    value = {}
    for key, item in items:
        if key in value:
            _fail("JOURNAL_WIRE")
        value[key] = item
    return value


def _parse_int(value):
    parsed = int(value)
    if parsed < -MAX_I64 - 1 or parsed > MAX_I64:
        _fail("JOURNAL_WIRE")
    return parsed


def _reject_number(_value):
    _fail("JOURNAL_WIRE")


def _depth(value, current=1):
    if current > 16:
        _fail("JOURNAL_DEPTH")
    if isinstance(value, dict):
        for key, item in value.items():
            if not isinstance(key, str):
                _fail("JOURNAL_WIRE")
            _depth(item, current + 1)
    elif isinstance(value, list):
        for item in value:
            _depth(item, current + 1)


def _canonical(value):
    try:
        return json.dumps(value, ensure_ascii=True, sort_keys=True,
                          separators=(",", ":"), allow_nan=False).encode("ascii")
    except (TypeError, ValueError, UnicodeError, RecursionError):
        _fail("JOURNAL_WIRE")


def _json(data, maximum=MAX_SMALL):
    if not isinstance(data, bytes) or not data or len(data) > maximum:
        _fail("JOURNAL_SIZE")
    try:
        data.decode("ascii")
        value = json.loads(data, object_pairs_hook=_pairs, parse_int=_parse_int,
                           parse_float=_reject_number, parse_constant=_reject_number)
    except JournalError:
        raise
    except (UnicodeError, ValueError, TypeError, RecursionError):
        _fail("JOURNAL_WIRE")
    _depth(value)
    if _canonical(value) != data:
        _fail("JOURNAL_CANONICAL")
    return value


def _exact(value, keys):
    if not isinstance(value, dict) or set(value) != set(keys):
        _fail("JOURNAL_WIRE")


def _uuid(value, nullable=False):
    if nullable and value is None:
        return
    if (not isinstance(value, str) or UUID_RE.fullmatch(value) is None
            or value == "00000000-0000-0000-0000-000000000000"):
        _fail("JOURNAL_IDENTITY")


def _hash(value, nullable=False):
    if nullable and value is None:
        return
    if not isinstance(value, str) or HASH_RE.fullmatch(value) is None:
        _fail("JOURNAL_HASH")


def _counter(value):
    if type(value) is not int or not 0 <= value <= MAX_I64:
        _fail("JOURNAL_COUNTER")


def _validate_intent(data):
    value = _json(data, MAX_SMALL)
    _exact(value, INTENT_KEYS)
    if value["schema"] != "owner-beta-paper-operation-intent-v1":
        _fail("JOURNAL_WIRE")
    _uuid(value["installation_id"])
    _uuid(value["generation"])
    _uuid(value["account_id"], nullable=True)
    _uuid(value["predecessor_generation"], nullable=True)
    _uuid(value["resume_target_id"], nullable=True)
    if value["generation"] == value["predecessor_generation"]:
        _fail("JOURNAL_IDENTITY")
    key = value["request_key"]
    if (not isinstance(key, str) or not 1 <= len(key) <= 128
            or re.fullmatch(r"[A-Za-z0-9._:-]+", key) is None):
        _fail("JOURNAL_WIRE")
    if (not isinstance(value["purpose"], str)
            or value["purpose"] not in {"CONFIGURE", "INSPECT", "ENABLE", "STOP", "BACKUP", "RESTORE", "RECOVERY"}):
        _fail("JOURNAL_WIRE")
    if not isinstance(value["scope"], str) or value["scope"] not in {"PAPER_STOP", "COMBINED_BACKUP"}:
        _fail("JOURNAL_WIRE")
    _counter(value["expected_epoch"])
    _counter(value["local_epoch_floor"])
    _hash(value["release_manifest_sha256"])
    _hash(value["artifact_sha256"], nullable=True)
    purpose, scope = value["purpose"], value["scope"]
    generation, account = value["generation"], value["account_id"]
    expected, floor = value["expected_epoch"], value["local_epoch_floor"]
    key = value["request_key"]
    if purpose == "CONFIGURE":
        if account is not None or expected != 0 or floor != expected or value["predecessor_generation"] is not None:
            _fail("JOURNAL_IDENTITY")
    elif purpose == "INSPECT":
        if (account is None and expected != 0) or floor != expected or value["predecessor_generation"] is not None:
            _fail("JOURNAL_IDENTITY")
    else:
        if account is None:
            _fail("JOURNAL_IDENTITY")
        if purpose in {"ENABLE", "RECOVERY"} and (floor != expected or value["predecessor_generation"] is not None):
            _fail("JOURNAL_IDENTITY")
        if purpose != "RESTORE" and floor != expected:
            _fail("JOURNAL_IDENTITY")
    required_scope = {"STOP": "PAPER_STOP", "BACKUP": "COMBINED_BACKUP",
                      "RESTORE": "COMBINED_BACKUP"}.get(purpose)
    if required_scope is not None and scope != required_scope:
        _fail("JOURNAL_IDENTITY")
    expected_keys = {
        "ENABLE": {generation + ":enable"},
        "RECOVERY": {generation + ":recover"},
        "STOP": {generation + ":begin", generation + ":off"},
        "BACKUP": {generation + ":begin", generation + ":off"},
        "RESTORE": {generation + ":begin", generation + ":restore-off"},
    }
    if purpose in expected_keys:
        valid_key = key in expected_keys[purpose]
        if purpose in {"STOP", "BACKUP", "RESTORE"}:
            valid_key = valid_key or re.fullmatch(re.escape(generation) + r":reconcile:[0-9]{1,18}", key) is not None
        if not valid_key:
            _fail("JOURNAL_IDENTITY")
    return value


def _command_projection(intent):
    return _canonical({key: value for key, value in intent.items()
                       if key not in {"generation", "request_key"}})


def _validate_bound_root(intent, caller_key=None):
    purpose, generation = intent["purpose"], intent["generation"]
    if purpose == "INSPECT":
        _fail("JOURNAL_IDENTITY")
    if purpose == "CONFIGURE":
        if intent["account_id"] is not None or intent["expected_epoch"] != 0 or intent["local_epoch_floor"] != 0:
            _fail("JOURNAL_IDENTITY")
        if intent["artifact_sha256"] is None or intent["request_key"] != caller_key:
            _fail("JOURNAL_IDENTITY")
        return
    if intent["account_id"] is None:
        _fail("JOURNAL_IDENTITY")
    if purpose in {"ENABLE", "RECOVERY"}:
        suffix = ":enable" if purpose == "ENABLE" else ":recover"
        if (intent["request_key"] != generation + suffix or intent["artifact_sha256"] is None
                or intent["local_epoch_floor"] != intent["expected_epoch"]
                or intent["predecessor_generation"] is not None):
            _fail("JOURNAL_IDENTITY")
        return
    if purpose not in {"STOP", "BACKUP", "RESTORE"} or intent["request_key"] != generation + ":begin":
        _fail("JOURNAL_IDENTITY")
    scope = "PAPER_STOP" if purpose == "STOP" else "COMBINED_BACKUP"
    if intent["scope"] != scope:
        _fail("JOURNAL_IDENTITY")
    if purpose in {"STOP", "BACKUP"}:
        if intent["artifact_sha256"] is not None or intent["local_epoch_floor"] != intent["expected_epoch"]:
            _fail("JOURNAL_IDENTITY")
    elif intent["artifact_sha256"] is None or intent["predecessor_generation"] is None:
        _fail("JOURNAL_IDENTITY")
    if intent["local_epoch_floor"] < intent["expected_epoch"]:
        _fail("JOURNAL_EPOCH")


def _caller_name(installation_id, account_id, caller_key):
    return _sha(_canonical({"installation_id": installation_id,
                            "account_id": account_id, "caller_key": caller_key}))[7:] + ".json"


def _validate_caller(data, install_id=None):
    value = _json(data, MAX_SMALL)
    _exact(value, CALLER_KEYS)
    if value["schema"] != "owner-beta-paper-caller-binding-v1":
        _fail("JOURNAL_WIRE")
    _uuid(value["installation_id"])
    _uuid(value["account_id"], nullable=True)
    _uuid(value["generation"])
    _hash(value["root_intent_sha256"])
    if install_id is not None and value["installation_id"] != install_id:
        _fail("JOURNAL_IDENTITY")
    key = value["caller_key"]
    if (not isinstance(key, str) or not 1 <= len(key) <= 128
            or re.fullmatch(r"[A-Za-z0-9._:-]+", key) is None):
        _fail("JOURNAL_WIRE")
    if not isinstance(value["command_canonical"], str):
        _fail("JOURNAL_WIRE")
    projection = value["command_canonical"].encode("ascii")
    parsed = _json(projection, MAX_SMALL)
    _exact(parsed, INTENT_KEYS - {"generation", "request_key"})
    if _canonical(parsed) != projection or parsed["installation_id"] != value["installation_id"] or parsed["account_id"] != value["account_id"]:
        _fail("JOURNAL_IDENTITY")
    return value


def _validate_action(data, intent, sequence, previous_digest):
    _validate_bound_root(intent, intent["request_key"] if intent["purpose"] == "CONFIGURE" else None)
    value = _json(data, MAX_SMALL)
    _exact(value, ACTION_KEYS)
    if value["schema"] != "owner-beta-paper-action-binding-v1":
        _fail("JOURNAL_WIRE")
    _uuid(value["installation_id"])
    _uuid(value["generation"])
    _hash(value["root_intent_sha256"])
    if (value["installation_id"] != intent["installation_id"]
            or value["generation"] != intent["generation"]):
        _fail("JOURNAL_IDENTITY")
    if value["root_intent_sha256"] != _sha(_canonical(intent)):
        _fail("JOURNAL_IDENTITY")
    if type(value["sequence"]) is not int or value["sequence"] != sequence:
        _fail("JOURNAL_SEQUENCE")
    _hash(value["previous_binding_sha256"], nullable=sequence == 0)
    if value["previous_binding_sha256"] != (None if sequence == 0 else previous_digest):
        _fail("JOURNAL_CHAIN")
    verb = value["verb"]
    if verb not in {"configure", "enable", "begin", "reconcile", "off", "restore-off", "recover"}:
        _fail("JOURNAL_WIRE")
    key = value["request_key"]
    if (not isinstance(key, str) or not 1 <= len(key) <= 128
            or re.fullmatch(r"[A-Za-z0-9._:-]+", key) is None):
        _fail("JOURNAL_WIRE")
    _counter(value["expected_epoch"])
    _hash(value["artifact_sha256"], nullable=True)
    _hash(value["stage_sha256"], nullable=True)
    purpose = intent["purpose"]
    maintenance = purpose in {"STOP", "BACKUP", "RESTORE"}
    if not maintenance:
        expected_verb = {"CONFIGURE": "configure", "ENABLE": "enable", "RECOVERY": "recover"}.get(purpose)
        if sequence != 0 or verb != expected_verb or value["stage_sha256"] is not None:
            _fail("JOURNAL_IDENTITY")
        if purpose == "CONFIGURE":
            expected_epoch = 0
        else:
            expected_epoch = intent["expected_epoch"]
        if value["expected_epoch"] != expected_epoch or value["artifact_sha256"] != intent["artifact_sha256"]:
            _fail("JOURNAL_IDENTITY")
        if value["artifact_sha256"] is None:
            _fail("JOURNAL_IDENTITY")
        expected_key = intent["request_key"] if purpose == "CONFIGURE" else intent["generation"] + (":enable" if purpose == "ENABLE" else ":recover")
        if key != expected_key:
            _fail("JOURNAL_IDENTITY")
        return value
    if verb not in {"begin", "reconcile", "off", "restore-off"} or intent["account_id"] is None:
        _fail("JOURNAL_IDENTITY")
    allowed_by_purpose = {
        "STOP": {"begin", "reconcile", "off"},
        "BACKUP": {"begin", "reconcile", "off"},
        "RESTORE": {"begin", "reconcile", "restore-off"},
    }
    if verb not in allowed_by_purpose[purpose]:
        _fail("JOURNAL_IDENTITY")
    if intent["purpose"] == "STOP" and intent["scope"] != "PAPER_STOP":
        _fail("JOURNAL_IDENTITY")
    if intent["purpose"] in {"BACKUP", "RESTORE"} and intent["scope"] != "COMBINED_BACKUP":
        _fail("JOURNAL_IDENTITY")
    if intent["request_key"] != intent["generation"] + ":begin":
        _fail("JOURNAL_IDENTITY")
    if intent["purpose"] in {"STOP", "BACKUP"} and intent["artifact_sha256"] is not None:
        _fail("JOURNAL_IDENTITY")
    if intent["purpose"] == "RESTORE" and (intent["artifact_sha256"] is None
                                               or intent["predecessor_generation"] is None):
        _fail("JOURNAL_IDENTITY")
    try:
        fence = max(intent["expected_epoch"], intent["local_epoch_floor"]) + 1
    except OverflowError:
        _fail("JOURNAL_EPOCH")
    if fence > MAX_I64:
        _fail("JOURNAL_EPOCH")
    expected_epoch = intent["expected_epoch"] if verb == "begin" else fence
    expected_key = {
        "begin": intent["generation"] + ":begin",
        "off": intent["generation"] + ":off",
        "restore-off": intent["generation"] + ":restore-off",
    }.get(verb)
    if verb == "reconcile":
        expected_key = "%s:reconcile:%d" % (intent["generation"], sequence)
        if (sequence == 0 or sequence > 999_999_999_999_999_999
                or key != expected_key):
            _fail("JOURNAL_IDENTITY")
    elif key != expected_key:
        _fail("JOURNAL_IDENTITY")
    if value["expected_epoch"] != expected_epoch:
        _fail("JOURNAL_IDENTITY")
    if verb == "begin":
        if sequence != 0 or value["artifact_sha256"] is not None:
            _fail("JOURNAL_IDENTITY")
    elif value["artifact_sha256"] is None:
        _fail("JOURNAL_IDENTITY")
    if value["stage_sha256"] is None:
        _fail("JOURNAL_IDENTITY")
    return value


def _validate_action_readback(data, intent, sequence):
    value = _json(data, MAX_SMALL)
    _exact(value, ACTION_KEYS)
    _hash(value["previous_binding_sha256"], nullable=sequence == 0)
    return _validate_action(data, intent, sequence, value["previous_binding_sha256"])


def _civil_date(value):
    if isinstance(value, str) and re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", value):
        try:
            datetime.date.fromisoformat(value)
        except ValueError:
            _fail("JOURNAL_DATE")
    elif isinstance(value, dict):
        for item in value.values():
            _civil_date(item)
    elif isinstance(value, list):
        for item in value:
            _civil_date(item)


def _validate_stage(data, intent, intent_digest, sequence, previous_digest):
    value = _json(data, MAX_SMALL)
    _exact(value, STAGE_KEYS)
    if value["schema"] != "owner-beta-paper-operation-stage-v1":
        _fail("JOURNAL_WIRE")
    _uuid(value["installation_id"])
    _uuid(value["generation"])
    _uuid(value["account_id"])
    _uuid(value["boot_id"])
    _uuid(value["event_id"], nullable=True)
    if (value["installation_id"] != intent["installation_id"]
            or value["generation"] != intent["generation"]
            or intent["account_id"] is None
            or value["account_id"] != intent["account_id"]):
        _fail("JOURNAL_IDENTITY")
    if type(value["sequence"]) is not int or value["sequence"] != sequence:
        _fail("JOURNAL_SEQUENCE")
    expected_previous = None if sequence == 0 else previous_digest
    if value["previous_sha256"] != expected_previous:
        _fail("JOURNAL_CHAIN")
    _hash(value["previous_sha256"], nullable=sequence == 0)
    _hash(value["intent_sha256"])
    if value["intent_sha256"] != intent_digest:
        _fail("JOURNAL_CHAIN")
    stage = value["stage"]
    if not isinstance(stage, str) or stage not in set(STAGES) | {"FAILED"}:
        _fail("JOURNAL_STAGE")
    observed = value["observed"]
    _exact(observed, {"realtime_ns", "monotonic_ns", "boottime_ns"})
    for counter in observed.values():
        _counter(counter)
    _counter(value["control_epoch"])
    _hash(value["artifact_sha256"], nullable=True)
    if not isinstance(value["error_code"], str) or value["error_code"] not in ERROR_CODES:
        _fail("JOURNAL_WIRE")
    _civil_date(value)
    if (stage == "FAILED") != (value["error_code"] != "NONE"):
        _fail("JOURNAL_STAGE")
    return value


def _advance(prior, current, intent=None):
    if prior is None:
        if current != "INTENT":
            _fail("JOURNAL_ORDER")
        return
    if prior == "FAILED" or prior == "RESTORED_COMMITTED":
        _fail("JOURNAL_ORDER")
    if prior == "SNAPSHOT" and current == "FAILED":
        _fail("JOURNAL_ORDER")
    if intent is not None and intent["purpose"] == "STOP" and prior == "OFF_COMMITTED":
        _fail("JOURNAL_ORDER")
    if current == "FAILED":
        return
    if prior == current == "RECONCILED":
        return
    if intent is not None:
        purpose = intent["purpose"]
        if purpose == "STOP" and (prior == "OFF_COMMITTED" or current in {"SNAPSHOT", "RESTORE_VERIFIED", "RESTORED_COMMITTED"}):
            _fail("JOURNAL_ORDER")
        if purpose == "BACKUP" and current in {"RESTORE_VERIFIED", "RESTORED_COMMITTED"}:
            _fail("JOURNAL_ORDER")
        if purpose == "RESTORE" and (current in {"OFF_COMMITTED", "SNAPSHOT"}
                                      or (prior == "RECONCILED" and current != "RESTORE_VERIFIED")
                                      or (prior == "RESTORE_VERIFIED" and current != "RESTORED_COMMITTED")):
            _fail("JOURNAL_ORDER")
    if prior not in STAGES or current not in STAGES:
        _fail("JOURNAL_ORDER")
    if (intent is not None and intent["purpose"] == "RESTORE"
            and prior == "RECONCILED" and current == "RESTORE_VERIFIED"):
        return
    index = STAGES.index(prior)
    if index + 1 >= len(STAGES) or STAGES[index + 1] != current:
        _fail("JOURNAL_ORDER")


def _sha(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def _identity(value):
    return (value.st_dev, value.st_ino, value.st_uid, value.st_gid, value.st_mode,
            value.st_nlink, value.st_size, value.st_mtime_ns, value.st_ctime_ns)


def _dir_identity(value):
    # Directory size/ctime/mtime change as journal children are published.
    # Ownership, permission bits, device and inode are the retained identity.
    return (value.st_dev, value.st_ino, value.st_uid, value.st_gid, value.st_mode)


def _check_named_dir(parent_fd, name, fd):
    identity = _dir_identity(os.fstat(fd))
    named = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    if _dir_identity(named) != identity:
        _fail("JOURNAL_CHANGED")


def _no_acl(fd):
    for name in ("system.posix_acl_access", "system.posix_acl_default"):
        try:
            os.getxattr(fd, name)
        except OSError as error:
            if error.errno in (errno.ENODATA, errno.ENOTSUP, errno.EOPNOTSUPP):
                continue
            raise
        _fail("JOURNAL_ACL")


def _dir_info(fd, mode=None):
    info = os.fstat(fd)
    if (not stat.S_ISDIR(info.st_mode) or info.st_uid != 0 or info.st_gid != 0
            or (mode is not None and stat.S_IMODE(info.st_mode) != mode)
            or (mode is None and info.st_mode & 0o022)):
        _fail("JOURNAL_DIRECTORY")
    _no_acl(fd)
    return info


def _file_info(fd, mode=None):
    info = os.fstat(fd)
    if (not stat.S_ISREG(info.st_mode) or info.st_uid != 0 or info.st_gid != 0
            or info.st_nlink != 1 or (mode is not None and stat.S_IMODE(info.st_mode) != mode)):
        _fail("JOURNAL_FILE")
    _no_acl(fd)
    return info


def _open_named(parent_fd, name, mode, maximum):
    fd = os.open(name, READ_FLAGS, dir_fd=parent_fd)
    try:
        before = _file_info(fd, mode)
        named_before = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
        if _identity(named_before) != _identity(before):
            _fail("JOURNAL_CHANGED")
        if before.st_size > maximum:
            _fail("JOURNAL_SIZE")
        chunks = bytearray()
        while len(chunks) <= maximum:
            chunk = os.pread(fd, min(65_536, maximum + 1 - len(chunks)), len(chunks))
            if not chunk:
                break
            chunks.extend(chunk)
        after = _file_info(fd, mode)
        named_after = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
        if (len(chunks) != before.st_size or _identity(before) != _identity(after)
                or _identity(after) != _identity(named_after)):
            _fail("JOURNAL_CHANGED")
        return fd, bytes(chunks), _identity(after)
    except BaseException:
        os.close(fd)
        raise


def _read_named(parent_fd, name, mode, maximum):
    fd, data, _identity_value = _open_named(parent_fd, name, mode, maximum)
    os.close(fd)
    return data


def _retained_file(parent_fd, name, fd, expected, mode):
    info = _file_info(fd, mode)
    named = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    if _identity(info) != expected or _identity(named) != expected:
        _fail("JOURNAL_CHANGED")
    return info


def _path_parts(path):
    if (not isinstance(path, str) or not path.startswith("/") or path == "/"
            or path.endswith("/") or "\x00" in path
            or any(part in ("", ".", "..") for part in path.split("/")[1:])):
        _fail("JOURNAL_PATH")
    return path.split("/")[1:]


def _read_prior_action_digest(actions_fd, sequence):
    if sequence == 0:
        return None
    prior_fd = _open_child(actions_fd, "%08d" % (sequence - 1), 0o700)
    try:
        raw = _read_named(prior_fd, "binding.json", 0o400, MAX_SMALL)
        return _sha(raw)
    finally:
        os.close(prior_fd)


def _validate_action_prefix(actions_fd, intent, sequence):
    previous = None
    selected = None
    reconcile_count = 0
    for current in range(sequence + 1):
        try:
            action_fd = _open_child(actions_fd, "%08d" % current, 0o700)
        except FileNotFoundError:
            _fail("JOURNAL_INCOMPLETE")
        try:
            raw = _read_named(action_fd, "binding.json", 0o400, MAX_SMALL)
            action = _validate_action(raw, intent, current, previous)
            if action["verb"] == "reconcile":
                reconcile_count += 1
                if action["request_key"] != "%s:reconcile:%d" % (intent["generation"], reconcile_count):
                    _fail("JOURNAL_IDENTITY")
            if action["artifact_sha256"] is not None:
                evidence = _read_named(action_fd, "evidence.json", 0o400, MAX_SMALL)
                if _sha(evidence) != action["artifact_sha256"]:
                    _fail("JOURNAL_CUSTODY")
            selected = action
            previous = _sha(raw)
        finally:
            os.close(action_fd)
    return selected, previous


def _next_action_sequence(operation_fd):
    try:
        actions_fd = _open_child(operation_fd, "actions", 0o700)
    except FileNotFoundError:
        return 0
    try:
        with os.scandir(actions_fd) as entries:
            return sum(1 for entry in entries)
    finally:
        os.close(actions_fd)


def _read_previous_action(operation_fd):
    sequence = _next_action_sequence(operation_fd)
    if sequence == 0:
        return None
    actions_fd = _open_child(operation_fd, "actions", 0o700)
    try:
        return _read_prior_action_digest(actions_fd, sequence)
    finally:
        os.close(actions_fd)


def _stage_previous(operation_fd, sequence):
    if sequence == 0:
        return None
    raw = _read_named(operation_fd, "%08d.json" % (sequence - 1), 0o400, MAX_SMALL)
    return _sha(raw)


class _Tree:
    def __init__(self, path):
        self.names = _path_parts(path)
        self.fds = [os.open("/", DIR_FLAGS)]
        self.ids = [_dir_identity(_dir_info(self.fds[0]))]
        try:
            for name in self.names:
                self.fds.append(os.open(name, DIR_FLAGS, dir_fd=self.fds[-1]))
                self.ids.append(_dir_identity(_dir_info(self.fds[-1])))
        except BaseException:
            self.close()
            raise

    @property
    def fd(self):
        return self.fds[-1]

    def revalidate(self):
        for index, fd in enumerate(self.fds):
            now = _dir_identity(_dir_info(fd))
            if now != self.ids[index]:
                _fail("JOURNAL_CHANGED")
            if index:
                named = os.stat(self.names[index - 1], dir_fd=self.fds[index - 1],
                                follow_symlinks=False)
                if _dir_identity(named) != now:
                    _fail("JOURNAL_CHANGED")

    def close(self):
        for fd in reversed(getattr(self, "fds", [])):
            try:
                os.close(fd)
            except OSError:
                pass
        self.fds = []


def _open_child(parent, name, mode, *, create=False):
    if create:
        try:
            os.mkdir(name, mode, dir_fd=parent)
            os.fsync(parent)
        except FileExistsError:
            pass
    fd = os.open(name, DIR_FLAGS, dir_fd=parent)
    try:
        _dir_info(fd, mode)
        named = os.stat(name, dir_fd=parent, follow_symlinks=False)
        if _dir_identity(named) != _dir_identity(os.fstat(fd)):
            _fail("JOURNAL_CHANGED")
        return fd
    except BaseException:
        os.close(fd)
        raise


def _new_file(parent, name, data, mode):
    """Publish immutable bytes through an fsynced temp + no-replace hard link."""
    temp = ".%s.tmp.%s" % (name.replace("/", "_"), str(uuid.uuid4()))
    fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                 mode, dir_fd=parent)
    try:
        os.fchmod(fd, mode)
        view = memoryview(data)
        while view:
            written = os.write(fd, view)
            if written <= 0:
                _fail("JOURNAL_WRITE")
            view = view[written:]
        os.fsync(fd)
        _file_info(fd, mode)
        os.fsync(parent)
        os.link(temp, name, src_dir_fd=parent, dst_dir_fd=parent, follow_symlinks=False)
        os.fsync(parent)
        os.unlink(temp, dir_fd=parent)
        os.fsync(parent)
    except BaseException:
        # An on-disk temp is an explicit incomplete marker.  Never clean it up
        # after a partial write/fsync/publication failure.
        raise
    finally:
        os.close(fd)


def _immutable(parent, name, data, mode):
    try:
        os.stat(name, dir_fd=parent, follow_symlinks=False)
    except FileNotFoundError:
        pass
    else:
        existing = _read_named(parent, name, mode, max(len(data), MAX_NATIVE_EXACT))
        if existing != data:
            _fail("JOURNAL_CONFLICT")
        return
    try:
        _new_file(parent, name, data, mode)
        return
    except FileExistsError:
        existing = _read_named(parent, name, mode, max(len(data), MAX_NATIVE_EXACT))
        if existing != data:
            _fail("JOURNAL_CONFLICT")


def _atomic_replace(parent, name, data, mode):
    temp = ".%s.tmp.%s" % (name, str(uuid.uuid4()))
    fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                 mode, dir_fd=parent)
    try:
        os.fchmod(fd, mode)
        view = memoryview(data)
        while view:
            n = os.write(fd, view)
            if n <= 0:
                _fail("JOURNAL_WRITE")
            view = view[n:]
        os.fsync(fd)
        os.fsync(parent)
        os.rename(temp, name, src_dir_fd=parent, dst_dir_fd=parent)
        os.fsync(parent)
    finally:
        os.close(fd)


def _check_lock(install_root, release_lock_fd, tree):
    if type(release_lock_fd) is not int or release_lock_fd < 3:
        _fail("JOURNAL_LOCK")
    info = _file_info(release_lock_fd, 0o600)
    if info.st_size != 0:
        _fail("JOURNAL_LOCK")
    named = os.stat(LOCK_NAME, dir_fd=tree.fd, follow_symlinks=False)
    if _identity(named) != _identity(info):
        _fail("JOURNAL_LOCK")
    # Same inherited open file description: confirms exclusive lock semantics
    # without opening or replacing a second lock pathname.  Never LOCK_UN here.
    try:
        fcntl.flock(release_lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        _fail("JOURNAL_LOCK")
    return _identity(info)


class ProtectedOutput:
    """Retained caller-selected invocation output directory; privately created."""
    __slots__ = ("_parent", "_name", "_fd", "_parent_id", "_id", "_closed")

    def __init__(self, token, parent_fd, name):
        if token is not _TOKEN:
            _fail("JOURNAL_PRIVATE")
        if (not isinstance(name, str) or name in ("", ".", "..") or "/" in name
                or "\x00" in name):
            _fail("JOURNAL_PATH")
        self._parent = os.dup(parent_fd)
        self._fd = None
        try:
            self._parent_id = _dir_identity(_dir_info(self._parent))
            self._name = name
            self._fd = os.open(name, DIR_FLAGS, dir_fd=self._parent)
            self._id = _dir_identity(_dir_info(self._fd, 0o700))
            self._closed = False
            self.revalidate()
        except BaseException:
            if self._fd is not None:
                os.close(self._fd)
            os.close(self._parent)
            raise

    def revalidate(self):
        if self._closed:
            _fail("JOURNAL_CLOSED")
        if (_dir_identity(_dir_info(self._parent)) != self._parent_id
                or _dir_identity(_dir_info(self._fd, 0o700)) != self._id
                or _dir_identity(os.stat(self._name, dir_fd=self._parent, follow_symlinks=False)) != self._id):
            _fail("JOURNAL_CHANGED")

    def close(self):
        if not self._closed:
            self._closed = True
            os.close(self._fd)
            os.close(self._parent)


_TOKEN = object()


def protected_output(parent_fd, name):
    """Retain a known root:root0700 invocation output directory by name."""
    return ProtectedOutput(_TOKEN, parent_fd, name)


class Journal:
    __slots__ = ("_tree", "_lock_fd", "_lock_id", "_root", "_ops", "_install_id",
                 "_floor", "_identity_fd", "_identity_id", "_floor_fd", "_floor_id",
                 "_closed", "_operations", "_callers")

    def __init__(self, token, tree, lock_fd, lock_id, root_fd, ops_fd, install_id, floor,
                 identity_fd, identity_id, floor_fd, floor_id, callers_fd=None):
        if token is not _TOKEN:
            _fail("JOURNAL_PRIVATE")
        self._tree, self._lock_fd, self._lock_id = tree, lock_fd, lock_id
        self._root, self._ops = root_fd, ops_fd
        self._install_id, self._floor, self._closed = install_id, floor, False
        self._identity_fd, self._identity_id = identity_fd, identity_id
        self._floor_fd, self._floor_id = floor_fd, floor_id
        self._operations = set()
        self._callers = callers_fd

    @property
    def installation_id(self):
        return self._install_id

    @property
    def epoch_floor(self):
        return self._floor

    def revalidate(self):
        self._check()
        return None

    def _check(self):
        if self._closed:
            _fail("JOURNAL_CLOSED")
        self._tree.revalidate()
        if _identity(_file_info(self._lock_fd, 0o600)) != self._lock_id:
            _fail("JOURNAL_LOCK")
        if _identity(os.stat(LOCK_NAME, dir_fd=self._tree.fd, follow_symlinks=False)) != self._lock_id:
            _fail("JOURNAL_LOCK")
        _retained_file(self._root, "installation-id", self._identity_fd,
                       self._identity_id, 0o600)
        _retained_file(self._root, "epoch-floor", self._floor_fd,
                       self._floor_id, 0o600)
        _dir_info(self._root, 0o700)
        _dir_info(self._ops, 0o700)
        if _dir_identity(os.stat(PAPER_DIR, dir_fd=self._tree.fd, follow_symlinks=False)) != _dir_identity(os.fstat(self._root)):
            _fail("JOURNAL_CHANGED")
        if _dir_identity(os.stat("operations", dir_fd=self._root, follow_symlinks=False)) != _dir_identity(os.fstat(self._ops)):
            _fail("JOURNAL_CHANGED")
        if self._callers is not None:
            _check_named_dir(self._root, "callers", self._callers)

    def _read_floor(self):
        _retained_file(self._root, "epoch-floor", self._floor_fd, self._floor_id, 0o600)
        raw = os.pread(self._floor_fd, 33, 0)
        _retained_file(self._root, "epoch-floor", self._floor_fd, self._floor_id, 0o600)
        if not re.fullmatch(rb"(?:0|[1-9][0-9]{0,18})\n", raw):
            _fail("JOURNAL_FLOOR")
        value = int(raw[:-1])
        if value > MAX_I64:
            _fail("JOURNAL_FLOOR")
        return value

    def advance_floor(self, value):
        self._check()
        _counter(value)
        current = self._read_floor()
        if value < current:
            _fail("JOURNAL_FLOOR_DECREMENT")
        # A floor is only a durable lower bound.  This method makes no claim
        # that PostgreSQL committed the corresponding epoch.
        if value > current:
            _atomic_replace(self._root, "epoch-floor", (str(value) + "\n").encode("ascii"), 0o600)
            new_fd, raw, new_id = _open_named(self._root, "epoch-floor", 0o600, 32)
            if raw != (str(value) + "\n").encode("ascii"):
                os.close(new_fd)
                _fail("JOURNAL_FLOOR")
            old_fd = self._floor_fd
            self._floor_fd, self._floor_id = new_fd, new_id
            os.close(old_fd)
            self._floor = value
        self._check()
        return self._floor

    def create(self, intent_canonical):
        self._check()
        if self._operations:
            _fail("JOURNAL_BUSY")
        intent = _validate_intent(intent_canonical)
        if intent["installation_id"] != self._install_id:
            _fail("JOURNAL_IDENTITY")
        generation = intent["generation"]
        self._check_operation_key(intent, generation)
        created = False
        try:
            os.mkdir(generation, 0o700, dir_fd=self._ops)
            os.fsync(self._ops)
            created = True
        except FileExistsError:
            pass
        operation_fd = os.open(generation, DIR_FLAGS, dir_fd=self._ops)
        intent_fd = None
        operation = None
        try:
            _dir_info(operation_fd, 0o700)
            _check_named_dir(self._ops, generation, operation_fd)
            if created:
                _immutable(operation_fd, "intent.json", intent_canonical, 0o400)
            else:
                current = _read_named(operation_fd, "intent.json", 0o400, MAX_SMALL)
                if current != intent_canonical:
                    _fail("JOURNAL_CONFLICT")
            intent_fd, stored_intent, intent_id = _open_named(
                operation_fd, "intent.json", 0o400, MAX_SMALL)
            if stored_intent != intent_canonical:
                _fail("JOURNAL_CONFLICT")
            self._check()
            operation = Operation(_TOKEN, self, operation_fd, generation, intent,
                                  intent_canonical, intent_fd, intent_id)
            operation._bound = self._has_caller_binding(generation, operation.intent_sha256)
            operation.revalidate()
            self._operations.add(operation)
            return operation
        except BaseException:
            if operation is not None:
                operation.close()
            else:
                if intent_fd is not None:
                    os.close(intent_fd)
                os.close(operation_fd)
            raise

    def _ensure_callers(self, create=False):
        self._check()
        if self._callers is None:
            self._callers = _open_child(self._root, "callers", 0o700, create=create)
        _check_named_dir(self._root, "callers", self._callers)
        self._scan_callers()
        return self._callers

    def _scan_callers(self):
        if self._callers is None:
            return
        with os.scandir(self._callers) as entries:
            names = sorted(entry.name for entry in entries)
        for name in names:
            if re.fullmatch(r"[0-9a-f]{64}\.json", name) is None:
                _fail("JOURNAL_INCOMPLETE")
            raw = _read_named(self._callers, name, 0o400, MAX_SMALL)
            value = _validate_caller(raw, self._install_id)
            if _caller_name(value["installation_id"], value["account_id"], value["caller_key"]) != name:
                _fail("JOURNAL_CUSTODY")

    def lookup_caller(self, caller_key, command_canonical):
        """Strict retained caller lookup; returns the mapped operation or None."""
        self._check()
        if (not isinstance(caller_key, str) or not 1 <= len(caller_key) <= 128
                or re.fullmatch(r"[A-Za-z0-9._:-]+", caller_key) is None):
            _fail("JOURNAL_WIRE")
        if not isinstance(command_canonical, bytes):
            _fail("JOURNAL_WIRE")
        projection = _json(command_canonical, MAX_SMALL)
        _exact(projection, INTENT_KEYS - {"generation", "request_key"})
        if projection["installation_id"] != self._install_id:
            _fail("JOURNAL_IDENTITY")
        if self._callers is None:
            return None
        self._scan_callers()
        name = _caller_name(self._install_id, projection["account_id"], caller_key)
        try:
            data = _read_named(self._callers, name, 0o400, MAX_SMALL)
        except FileNotFoundError:
            # A missing lookup is not permission to guess among retained roots.
            match = None
            with os.scandir(self._ops) as entries:
                for entry in entries:
                    _uuid(entry.name)
                    fd = os.open(entry.name, DIR_FLAGS, dir_fd=self._ops)
                    try:
                        _dir_info(fd, 0o700)
                        raw = _read_named(fd, "intent.json", 0o400, MAX_SMALL)
                        intent = _validate_intent(raw)
                        if _command_projection(intent) == command_canonical:
                            if match is not None:
                                _fail("JOURNAL_AMBIGUOUS_ROOT")
                            match = intent["generation"]
                    finally:
                        os.close(fd)
            return None if match is None else self.open(match)
        binding = _validate_caller(data, self._install_id)
        if _caller_name(binding["installation_id"], binding["account_id"], binding["caller_key"]) != name:
            _fail("JOURNAL_CUSTODY")
        if (binding["caller_key"] != caller_key or binding["account_id"] != projection["account_id"]
                or binding["command_canonical"].encode("ascii") != command_canonical):
            _fail("JOURNAL_CONFLICT")
        operation = self.open(binding["generation"])
        if (operation.intent_sha256 != binding["root_intent_sha256"]
                or _command_projection(operation._intent) != command_canonical):
            operation.close()
            _fail("JOURNAL_CUSTODY")
        return operation

    def _has_caller_binding(self, generation, intent_digest):
        if self._callers is None:
            return False
        self._scan_callers()
        with os.scandir(self._callers) as entries:
            for entry in entries:
                raw = _read_named(self._callers, entry.name, 0o400, MAX_SMALL)
                value = _validate_caller(raw, self._install_id)
                if value["generation"] == generation and value["root_intent_sha256"] == intent_digest:
                    return True
        return False

    def bind_caller(self, caller_key, root_intent_canonical):
        """Bind an idempotent caller key to one immutable root operation."""
        self._check()
        intent = _validate_intent(root_intent_canonical)
        if intent["installation_id"] != self._install_id:
            _fail("JOURNAL_IDENTITY")
        if (not isinstance(caller_key, str) or not 1 <= len(caller_key) <= 128
                or re.fullmatch(r"[A-Za-z0-9._:-]+", caller_key) is None):
            _fail("JOURNAL_WIRE")
        _validate_bound_root(intent, caller_key)
        projection = _command_projection(intent)
        existing = self.lookup_caller(caller_key, projection)
        if existing is not None:
            callers_fd = self._ensure_callers(create=True)
            name = _caller_name(self._install_id, existing._intent["account_id"], caller_key)
            try:
                _read_named(callers_fd, name, 0o400, MAX_SMALL)
            except FileNotFoundError:
                value = {"schema": "owner-beta-paper-caller-binding-v1",
                         "installation_id": self._install_id,
                         "account_id": existing._intent["account_id"], "caller_key": caller_key,
                         "generation": existing.generation,
                         "root_intent_sha256": existing.intent_sha256,
                         "command_canonical": projection.decode("ascii")}
                _immutable(callers_fd, name, _canonical(value), 0o400)
            return existing
        callers_fd = self._ensure_callers(create=True)
        name = _caller_name(self._install_id, intent["account_id"], caller_key)
        # Recheck an index that may have appeared between lookup and publication.
        try:
            prior = _validate_caller(_read_named(callers_fd, name, 0o400, MAX_SMALL), self._install_id)
        except FileNotFoundError:
            prior = None
        if prior is not None:
            if (prior["caller_key"] != caller_key or prior["command_canonical"] != projection.decode("ascii")):
                _fail("JOURNAL_CONFLICT")
            return self.open(prior["generation"])
        operation = self.create(root_intent_canonical)
        value = {"schema": "owner-beta-paper-caller-binding-v1",
                 "installation_id": self._install_id, "account_id": intent["account_id"],
                 "caller_key": caller_key, "generation": intent["generation"],
                 "root_intent_sha256": operation.intent_sha256,
                 "command_canonical": projection.decode("ascii")}
        binding = _canonical(value)
        _validate_caller(binding, self._install_id)
        try:
            _immutable(callers_fd, name, binding, 0o400)
            operation._bound = True
            operation.revalidate()
            self._check()
            _check_named_dir(self._root, "callers", callers_fd)
            return operation
        except BaseException:
            operation.close()
            raise

    def open_custody(self, generation, sequence):
        """Open immutable R/A/E/Q custody for descriptive readback, without a live chain."""
        self._check()
        _uuid(generation)
        _counter(sequence)
        op_fd = _open_child(self._ops, generation, 0o700)
        action_fd = output_fd = root_fd = binding_fd = evidence_fd = None
        try:
            root_fd, root_bytes, root_id = _open_named(op_fd, "intent.json", 0o400, MAX_SMALL)
            intent = _validate_intent(root_bytes)
            if intent["generation"] != generation or intent["installation_id"] != self._install_id:
                _fail("JOURNAL_IDENTITY")
            actions_fd = _open_child(op_fd, "actions", 0o700)
            action_name = "%08d" % sequence
            action_fd = _open_child(actions_fd, action_name, 0o700)
            binding_fd, action_bytes, action_id = _open_named(action_fd, "binding.json", 0o400, MAX_SMALL)
            action = _validate_action_readback(action_bytes, intent, sequence)
            if action_bytes != _read_named(action_fd, "binding.json", 0o400, MAX_SMALL):
                _fail("JOURNAL_CUSTODY")
            if action["root_intent_sha256"] != _sha(root_bytes):
                _fail("JOURNAL_IDENTITY")
            evidence = None
            if action["artifact_sha256"] is not None:
                evidence_fd, evidence_bytes, evidence_id = _open_named(action_fd, "evidence.json", 0o400, MAX_SMALL)
                if _sha(evidence_bytes) != action["artifact_sha256"]:
                    _fail("JOURNAL_CUSTODY")
                evidence = (evidence_fd, evidence_bytes, evidence_id)
                evidence_fd = None
            else:
                try:
                    os.stat("evidence.json", dir_fd=action_fd, follow_symlinks=False)
                except FileNotFoundError:
                    pass
                else:
                    _fail("JOURNAL_CUSTODY")
            output_fd = _open_child(action_fd, "output", 0o700)
            dispatch_bytes = _read_named(action_fd, "import.json", 0o400, MAX_NATIVE_INDEX)
            custody = _custody_from_v2(_TOKEN, self, generation, sequence, op_fd, actions_fd,
                                       action_fd, output_fd, root_fd, root_bytes, root_id,
                                       binding_fd, action_bytes, action_id, evidence,
                                       dispatch_bytes, readonly=True)
            op_fd = actions_fd = action_fd = output_fd = root_fd = binding_fd = None
            return custody
        except BaseException:
            raise
        finally:
            for fd in (evidence_fd, binding_fd, root_fd, output_fd, action_fd):
                if fd is not None:
                    os.close(fd)
            # actions_fd is scoped separately, and ownership moves on success.
            if 'actions_fd' in locals() and actions_fd is not None:
                os.close(actions_fd)
            if op_fd is not None:
                os.close(op_fd)

    def _check_operation_key(self, proposed, proposed_generation):
        """Stream old intents to enforce account/request-key immutability."""
        with os.scandir(self._ops) as entries:
            for entry in entries:
                name = entry.name
                if name == proposed_generation:
                    continue
                _uuid(name)
                fd = os.open(name, DIR_FLAGS, dir_fd=self._ops)
                try:
                    _dir_info(fd, 0o700)
                    _check_named_dir(self._ops, name, fd)
                    raw = _read_named(fd, "intent.json", 0o400, MAX_SMALL)
                    prior = _validate_intent(raw)
                    if prior["generation"] != name or prior["installation_id"] != self._install_id:
                        _fail("JOURNAL_IDENTITY")
                    _check_named_dir(self._ops, name, fd)
                    if (prior["account_id"] == proposed["account_id"]
                            and prior["request_key"] == proposed["request_key"]):
                        _fail("JOURNAL_CONFLICT")
                finally:
                    os.close(fd)

    def open(self, generation):
        self._check()
        if self._operations:
            _fail("JOURNAL_BUSY")
        _uuid(generation)
        fd = os.open(generation, DIR_FLAGS, dir_fd=self._ops)
        intent_fd = None
        op = None
        try:
            _dir_info(fd, 0o700)
            _check_named_dir(self._ops, generation, fd)
            intent_fd, intent_bytes, intent_id = _open_named(fd, "intent.json", 0o400, MAX_SMALL)
            intent = _validate_intent(intent_bytes)
            if intent["generation"] != generation or intent["installation_id"] != self._install_id:
                _fail("JOURNAL_IDENTITY")
            op = Operation(_TOKEN, self, fd, generation, intent, intent_bytes, intent_fd, intent_id)
            op._bound = self._has_caller_binding(generation, op.intent_sha256)
            op.revalidate()
            self._operations.add(op)
            return op
        except BaseException:
            if op is not None:
                op.close()
            else:
                if intent_fd is not None:
                    os.close(intent_fd)
                os.close(fd)
            raise

    def close(self):
        if not self._closed:
            self._closed = True
            for operation in tuple(self._operations):
                operation.close()
            os.close(self._ops)
            os.close(self._root)
            os.close(self._identity_fd)
            os.close(self._floor_fd)
            if self._callers is not None:
                os.close(self._callers)
            self._tree.close()


class Operation:
    __slots__ = ("_journal", "_fd", "_generation", "_intent", "_intent_bytes",
                 "_intent_digest", "_intent_fd", "_intent_id", "_head_fd", "_head_id",
                 "_head_name", "_original_fds", "_dispatch_hash", "_sql_request_hash",
                 "_closed", "_last", "_actions", "_bound")

    def __init__(self, token, journal, fd, generation, intent, intent_bytes, intent_fd, intent_id):
        if token is not _TOKEN:
            _fail("JOURNAL_PRIVATE")
        self._journal, self._fd = journal, fd
        self._generation, self._intent = generation, intent
        self._intent_bytes = intent_bytes
        self._intent_digest = _sha(intent_bytes)
        self._intent_fd, self._intent_id = intent_fd, intent_id
        self._head_fd = self._head_id = self._head_name = None
        self._original_fds = {}
        self._dispatch_hash = None
        self._sql_request_hash = None
        self._closed = False
        self._last = None
        self._actions = set()
        self._bound = False

    @property
    def generation(self):
        return self._generation

    @property
    def intent_sha256(self):
        return self._intent_digest

    def borrow_directory_fd(self):
        """Return the retained named operation directory while this operation is open."""
        self._check()
        return self._fd

    def _check(self):
        if self._closed:
            _fail("JOURNAL_CLOSED")
        self._journal._check()
        _dir_info(self._fd, 0o700)
        if _dir_identity(os.stat(self._generation, dir_fd=self._journal._ops,
                                 follow_symlinks=False)) != _dir_identity(os.fstat(self._fd)):
            _fail("JOURNAL_CHANGED")
        _retained_file(self._fd, "intent.json", self._intent_fd,
                       self._intent_id, 0o400)
        if self._head_fd is not None:
            _retained_file(self._fd, self._head_name, self._head_fd, self._head_id, 0o400)
        for content_hash, (content_fd, content_id) in self._original_fds.items():
            _retained_file(self._fd, content_hash[7:] + ".json", content_fd, content_id, 0o400)

    def _scan(self):
        """Stream and validate all sealed stages with constant history memory."""
        self._check()
        intent_bytes = _read_named(self._fd, "intent.json", 0o400, MAX_SMALL)
        if intent_bytes != self._intent_bytes:
            _fail("JOURNAL_CONFLICT")
        expected = 0
        previous_hash = None
        previous_stage = None
        last = None
        while True:
            name = "%08d.json" % expected
            try:
                data = _read_named(self._fd, name, 0o400, MAX_SMALL)
            except FileNotFoundError:
                break
            value = _validate_stage(data, self._intent, self._intent_digest, expected, previous_hash)
            self._validate_bound_stage(value, None if last is None else last[3])
            _advance(previous_stage, value["stage"], self._intent)
            previous_hash = _sha(data)
            sealed_copy = _read_named(self._fd, previous_hash[7:] + ".json", 0o400, MAX_SMALL)
            if sealed_copy != data:
                _fail("JOURNAL_CHAIN")
            previous_stage = value["stage"]
            last = (expected, previous_hash, previous_stage, value)
            expected += 1
        # Do not collect directory entries. Reject aliases, gaps, unknown files,
        # and temporary markers while retaining only the contiguous frontier.
        with os.scandir(self._fd) as entries:
            for entry in entries:
                name = entry.name
                if name == "intent.json" or (last is not None and name == "%08d.json" % last[0]):
                    continue
                if name == "actions":
                    actions_fd = _open_child(self._fd, "actions", 0o700)
                    try:
                        self._scan_action_bindings(actions_fd)
                    finally:
                        os.close(actions_fd)
                    continue
                match = STAGE_FILE_RE.fullmatch(name)
                if match:
                    numeric = int(match.group(1))
                    if name != "%08d.json" % numeric or numeric >= expected:
                        _fail("JOURNAL_INCOMPLETE")
                    continue
                if TEMP_RE.fullmatch(name):
                    _fail("JOURNAL_INCOMPLETE")
                if re.fullmatch(r"[0-9a-f]{64}\.json", name):
                    digest = "sha256:" + name[:-5]
                    data = _read_named(self._fd, name, 0o400, MAX_NATIVE_INDEX)
                    if _sha(data) != digest:
                        _fail("JOURNAL_CUSTODY")
                    continue
                _fail("JOURNAL_INCOMPLETE")
        self._check()
        if last is not None:
            head_name = "%08d.json" % last[0]
            if self._head_name != head_name:
                head_fd, head_bytes, head_id = _open_named(self._fd, head_name, 0o400, MAX_SMALL)
                if _sha(head_bytes) != last[1]:
                    os.close(head_fd)
                    _fail("JOURNAL_CHANGED")
                old_head = self._head_fd
                self._head_fd, self._head_id, self._head_name = head_fd, head_id, head_name
                if old_head is not None:
                    os.close(old_head)
        self._last = last
        return last

    def revalidate(self):
        last = self._scan()
        # None means intent is present but stage zero is not sealed; it is
        # incomplete, never a successful operation result.
        return None if last is None else last[2]

    def _validate_bound_stage(self, stage, previous):
        if not self._bound:
            return
        purpose = self._intent["purpose"]
        allowed = {
            "STOP": {"INTENT", "FENCED", "STOPPED", "RECONCILED", "OFF_COMMITTED", "FAILED"},
            "BACKUP": {"INTENT", "FENCED", "STOPPED", "RECONCILED", "OFF_COMMITTED", "SNAPSHOT", "FAILED"},
            "RESTORE": {"INTENT", "FENCED", "STOPPED", "RECONCILED", "RESTORE_VERIFIED", "RESTORED_COMMITTED", "FAILED"},
        }.get(purpose)
        if allowed is None or stage["stage"] not in allowed:
            _fail("JOURNAL_STAGE")
        if stage["stage"] == "FAILED":
            if previous is None or stage["control_epoch"] != previous["control_epoch"]:
                _fail("JOURNAL_EPOCH")
            return
        fence = max(self._intent["expected_epoch"], self._intent["local_epoch_floor"]) + 1
        if fence > MAX_I64:
            _fail("JOURNAL_EPOCH")
        expected = self._intent["expected_epoch"] if stage["stage"] == "INTENT" else fence
        if stage["stage"] in {"OFF_COMMITTED", "SNAPSHOT", "RESTORED_COMMITTED"}:
            if fence == MAX_I64:
                _fail("JOURNAL_EPOCH")
            expected = fence + 1
        if stage["control_epoch"] != expected:
            _fail("JOURNAL_EPOCH")

    def _scan_action_bindings(self, actions_fd):
        expected_sequence, previous_digest = 0, None
        reconcile_ordinal = 0
        with os.scandir(actions_fd) as entries:
            names = sorted(entry.name for entry in entries)
        for name in names:
            if ACTION_DIR_RE.fullmatch(name) is None or name != "%08d" % expected_sequence:
                _fail("JOURNAL_INCOMPLETE")
            action_fd = _open_child(actions_fd, name, 0o700)
            try:
                raw = _read_named(action_fd, "binding.json", 0o400, MAX_SMALL)
                action = _validate_action(raw, self._intent, expected_sequence, previous_digest)
                if action["verb"] == "reconcile":
                    reconcile_ordinal += 1
                    if action["request_key"] != "%s:reconcile:%d" % (self._generation, reconcile_ordinal):
                        _fail("JOURNAL_IDENTITY")
                previous_digest = _sha(raw)
                if action["artifact_sha256"] is not None:
                    evidence = _read_named(action_fd, "evidence.json", 0o400, MAX_SMALL)
                    if _sha(evidence) != action["artifact_sha256"]:
                        _fail("JOURNAL_CUSTODY")
                expected_files = {"binding.json", "output"}
                if action["artifact_sha256"] is not None:
                    expected_files.add("evidence.json")
                with os.scandir(action_fd) as action_names:
                    has_import = "import.json" in {entry.name for entry in action_names}
                if has_import:
                    expected_files.add("import.json")
                    output_check = _open_child(action_fd, "output", 0o700)
                    try:
                        if _read_named(action_fd, "import.json", 0o400, MAX_NATIVE_INDEX) != _read_named(output_check, "dispatch.json", 0o400, MAX_NATIVE_INDEX):
                            _fail("JOURNAL_CUSTODY")
                    finally:
                        os.close(output_check)
                with os.scandir(action_fd) as action_entries:
                    if {entry.name for entry in action_entries} != expected_files:
                        _fail("JOURNAL_INCOMPLETE")
                output_fd = _open_child(action_fd, "output", 0o700)
                os.close(output_fd)
            except FileNotFoundError:
                _fail("JOURNAL_INCOMPLETE")
            finally:
                os.close(action_fd)
            expected_sequence += 1

    def append(self, stage_canonical):
        self._check()
        if self._intent["purpose"] not in {"STOP", "BACKUP", "RESTORE"}:
            _fail("JOURNAL_STAGE")
        prior = self._scan()
        incoming = _json(stage_canonical, MAX_SMALL)
        _exact(incoming, STAGE_KEYS)
        incoming_sequence = incoming["sequence"]
        if type(incoming_sequence) is not int or incoming_sequence < 0:
            _fail("JOURNAL_SEQUENCE")
        if prior is not None and incoming_sequence <= prior[0]:
            name = "%08d.json" % incoming_sequence
            try:
                original = _read_named(self._fd, name, 0o400, MAX_SMALL)
            except FileNotFoundError:
                _fail("JOURNAL_CONFLICT")
            if original != stage_canonical:
                _fail("JOURNAL_CONFLICT")
            return _sha(original)
        sequence = 0 if prior is None else prior[0] + 1
        if incoming_sequence != sequence:
            _fail("JOURNAL_SEQUENCE")
        previous = None if prior is None else prior[1]
        value = _validate_stage(stage_canonical, self._intent, self._intent_digest, sequence, previous)
        self._validate_bound_stage(value, None if prior is None else prior[3])
        _advance(None if prior is None else prior[2], value["stage"], self._intent)
        _immutable(self._fd, _sha(stage_canonical)[7:] + ".json", stage_canonical, 0o400)
        _immutable(self._fd, "%08d.json" % sequence, stage_canonical, 0o400)
        self._check()
        self._scan()
        return _sha(stage_canonical)

    def prepare_action(self, action_canonical, artifact_canonical=None):
        """Seal one immutable action binding and its optional original evidence."""
        self._check()
        self._scan()
        if not isinstance(action_canonical, bytes):
            _fail("JOURNAL_WIRE")
        proposed = _json(action_canonical, MAX_SMALL)
        _exact(proposed, ACTION_KEYS)
        _counter(proposed["sequence"])
        sequence = proposed["sequence"]
        next_sequence = _next_action_sequence(self._fd)
        if sequence < next_sequence:
            existing = self.open_action(sequence)
            if (existing._binding_bytes != action_canonical
                    or existing._artifact_bytes != artifact_canonical):
                existing.close()
                _fail("JOURNAL_CONFLICT")
            return existing
        if sequence != next_sequence:
            _fail("JOURNAL_SEQUENCE")
        if next_sequence:
            actions_fd = _open_child(self._fd, "actions", 0o700)
            try:
                prior_fd = _open_child(actions_fd, "%08d" % (next_sequence - 1), 0o700)
                try:
                    try:
                        prior_import = _read_named(prior_fd, "import.json", 0o400, MAX_NATIVE_INDEX)
                        prior_output = _open_child(prior_fd, "output", 0o700)
                        try:
                            if prior_import != _read_named(prior_output, "dispatch.json", 0o400, MAX_NATIVE_INDEX):
                                _fail("JOURNAL_ACTION_PENDING")
                        finally:
                            os.close(prior_output)
                    except FileNotFoundError:
                        _fail("JOURNAL_ACTION_PENDING")
                finally:
                    os.close(prior_fd)
            finally:
                os.close(actions_fd)
        action_value = _validate_action(action_canonical, self._intent, sequence,
                                        _read_previous_action(self._fd))
        if artifact_canonical is not None:
            if not isinstance(artifact_canonical, bytes) or _sha(artifact_canonical) != action_value["artifact_sha256"]:
                _fail("JOURNAL_CUSTODY")
            _json(artifact_canonical, MAX_SMALL)
        elif action_value["artifact_sha256"] is not None:
            _fail("JOURNAL_CUSTODY")
        stage = self._validate_action_stage(action_value)
        actions_fd = _open_child(self._fd, "actions", 0o700, create=True)
        action_name = "%08d" % action_value["sequence"]
        created = False
        try:
            try:
                os.mkdir(action_name, 0o700, dir_fd=actions_fd)
                os.fsync(actions_fd)
                created = True
            except FileExistsError:
                pass
            action_fd = _open_child(actions_fd, action_name, 0o700)
            try:
                _immutable(action_fd, "binding.json", action_canonical, 0o400)
                if artifact_canonical is not None:
                    _immutable(action_fd, "evidence.json", artifact_canonical, 0o400)
                output_fd = _open_child(action_fd, "output", 0o700, create=True)
                custody = ActionCustody(_TOKEN, self, actions_fd, action_fd, output_fd,
                                        action_value, action_canonical, artifact_canonical,
                                        stage)
                self._actions.add(custody)
                self._scan()
                return custody
            except BaseException:
                os.close(action_fd)
                raise
        except BaseException:
            if created:
                # Preserve incomplete directories and files as an explicit recovery marker.
                pass
            os.close(actions_fd)
            raise

    def open_action(self, sequence):
        """Reopen an existing action for live work after validating its chain."""
        self._check()
        _counter(sequence)
        self._scan()
        actions_fd = _open_child(self._fd, "actions", 0o700)
        action_fd = output_fd = artifact_fd = None
        try:
            name = "%08d" % sequence
            action_fd = _open_child(actions_fd, name, 0o700)
            action_bytes = _read_named(action_fd, "binding.json", 0o400, MAX_SMALL)
            action = _validate_action(action_bytes, self._intent, sequence,
                                      _read_prior_action_digest(actions_fd, sequence))
            artifact = None
            if action["artifact_sha256"] is not None:
                artifact_fd, artifact, artifact_id = _open_named(action_fd, "evidence.json", 0o400, MAX_SMALL)
                if _sha(artifact) != action["artifact_sha256"]:
                    _fail("JOURNAL_CUSTODY")
                os.close(artifact_fd)
                artifact_fd = None
            stage = self._validate_action_stage(action)
            output_fd = _open_child(action_fd, "output", 0o700)
            custody = ActionCustody(_TOKEN, self, actions_fd, action_fd, output_fd,
                                    action, action_bytes, artifact, stage)
            self._actions.add(custody)
            actions_fd = action_fd = output_fd = None
            return custody
        finally:
            for fd in (artifact_fd, output_fd, action_fd, actions_fd):
                if fd is not None:
                    os.close(fd)

    def _validate_action_stage(self, action):
        if action["stage_sha256"] is None:
            return None
        last = self._scan()
        if last is None:
            _fail("JOURNAL_STAGE")
        stage_names = {"begin": {"INTENT"}, "reconcile": {"STOPPED", "RECONCILED"},
                       "off": {"RECONCILED"}, "restore-off": {"RESTORE_VERIFIED"}}
        selected = None
        for sequence in range(last[0] + 1):
            raw = _read_named(self._fd, "%08d.json" % sequence, 0o400, MAX_SMALL)
            if _sha(raw) == action["stage_sha256"]:
                selected = _validate_stage(raw, self._intent, self._intent_digest,
                                            sequence, None if sequence == 0 else _stage_previous(self._fd, sequence))
                break
        if selected is None or selected["stage"] not in stage_names.get(action["verb"], set()):
            _fail("JOURNAL_STAGE")
        if _sha(_read_named(self._fd, "%08d.json" % last[0], 0o400, MAX_SMALL)) != action["stage_sha256"]:
            _fail("JOURNAL_STAGE")
        expected_epoch = self._intent["expected_epoch"]
        if action["verb"] != "begin":
            expected_epoch = max(expected_epoch, self._intent["local_epoch_floor"]) + 1
            if expected_epoch > MAX_I64:
                _fail("JOURNAL_EPOCH")
        if selected["stage"] in {"OFF_COMMITTED", "SNAPSHOT", "RESTORED_COMMITTED"}:
            expected_epoch += 1
        if expected_epoch > MAX_I64 or selected["control_epoch"] != expected_epoch:
            _fail("JOURNAL_EPOCH")
        if action["verb"] != "begin" and selected["stage"] not in {"STOPPED", "RECONCILED", "RESTORE_VERIFIED"}:
            _fail("JOURNAL_STAGE")
        return selected

    def _store_original(self, data, expected_hash, maximum):
        if not isinstance(data, bytes) or len(data) > maximum or _sha(data) != expected_hash:
            _fail("JOURNAL_CUSTODY")
        name = expected_hash[7:] + ".json"
        _immutable(self._fd, name, data, 0o400)
        if expected_hash not in self._original_fds:
            fd, stored, identity = _open_named(self._fd, name, 0o400, maximum)
            if stored != data:
                os.close(fd)
                _fail("JOURNAL_CUSTODY")
            self._original_fds[expected_hash] = (fd, identity)
        return name

    def read_original(self, digest):
        self._check()
        _hash(digest)
        name = digest[7:] + ".json"
        if digest not in self._original_fds:
            if len(self._original_fds) >= 5:
                _fail("JOURNAL_CUSTODY_LIMIT")
            fd, data, identity = _open_named(self._fd, name, 0o400, MAX_NATIVE_INDEX)
            self._original_fds[digest] = (fd, identity)
        else:
            fd, identity = self._original_fds[digest]
            _retained_file(self._fd, name, fd, identity, 0o400)
            data = os.pread(fd, MAX_NATIVE_INDEX + 1, 0)
            _retained_file(self._fd, name, fd, identity, 0o400)
        if _sha(data) != digest:
            _fail("JOURNAL_CUSTODY")
        return data

    def import_native_dispatch(self, output, dispatch_sha256):
        """Import one caller-correlated content-addressed native custody index."""
        self._check()
        if not isinstance(output, ProtectedOutput):
            _fail("JOURNAL_PRIVATE")
        output.revalidate()
        _hash(dispatch_sha256)
        index_name = dispatch_sha256[7:] + ".json"
        try:
            index_bytes = _read_named(output._fd, index_name, 0o400, MAX_NATIVE_INDEX)
        except FileNotFoundError:
            _fail("JOURNAL_CUSTODY")
        if _sha(index_bytes) != dispatch_sha256:
            _fail("JOURNAL_CUSTODY")
        index = _json(index_bytes, MAX_NATIVE_INDEX)
        _exact(index, {"schema", "intent_canonical", "intent_sha256", "artifact_canonical",
                       "artifact_sha256", "sql_request_canonical", "sql_request_sha256"})
        if index["schema"] != "owner-beta-paper-native-dispatch-v1":
            _fail("JOURNAL_CUSTODY")
        if self._dispatch_hash is not None and self._dispatch_hash != dispatch_sha256:
            _fail("JOURNAL_CONFLICT")
        if not isinstance(index["intent_canonical"], str) or not isinstance(index["sql_request_canonical"], str):
            _fail("JOURNAL_CUSTODY")
        intent_bytes = index["intent_canonical"].encode("ascii")
        sql_bytes = index["sql_request_canonical"].encode("ascii")
        _hash(index["intent_sha256"])
        _hash(index["sql_request_sha256"])
        if _sha(intent_bytes) != index["intent_sha256"] or _sha(sql_bytes) != index["sql_request_sha256"]:
            _fail("JOURNAL_CUSTODY")
        if intent_bytes != self._intent_bytes:
            _fail("JOURNAL_IDENTITY")
        artifact = None
        artifact_bytes = None
        if index["artifact_canonical"] is None:
            if index["artifact_sha256"] is not None or self._intent["artifact_sha256"] is not None:
                _fail("JOURNAL_CUSTODY")
        else:
            if not isinstance(index["artifact_canonical"], str):
                _fail("JOURNAL_CUSTODY")
            artifact_bytes = index["artifact_canonical"].encode("ascii")
            _hash(index["artifact_sha256"])
            if (_sha(artifact_bytes) != index["artifact_sha256"]
                    or self._intent["artifact_sha256"] != index["artifact_sha256"]):
                _fail("JOURNAL_CUSTODY")
            artifact = _json(artifact_bytes, MAX_SMALL)
            if not isinstance(artifact, dict):
                _fail("JOURNAL_CUSTODY")
            _civil_date(artifact)
        self._validate_exact_request(sql_bytes, artifact, artifact_bytes)
        self._verify_output_object(output, index["intent_sha256"], intent_bytes, MAX_SMALL)
        if artifact_bytes is not None:
            self._verify_output_object(output, index["artifact_sha256"], artifact_bytes, MAX_SMALL)
        self._verify_output_object(output, index["sql_request_sha256"], sql_bytes, MAX_NATIVE_EXACT)
        # The custody index and every referenced preimage came from explicitly
        # named native hash files, not a directory search for JSON documents.
        for name, data, limit in (
            (index["intent_sha256"], intent_bytes, MAX_SMALL),
            (index["artifact_sha256"], artifact_bytes, MAX_SMALL),
            (index["sql_request_sha256"], sql_bytes, MAX_NATIVE_EXACT),
            (dispatch_sha256, index_bytes, MAX_NATIVE_INDEX),
        ):
            if name is not None:
                self._store_original(data, name, limit)
        self._dispatch_hash = dispatch_sha256
        self._sql_request_hash = index["sql_request_sha256"]
        output.revalidate()
        self._check()
        return Custody(_TOKEN, self, intent_bytes, artifact_bytes, sql_bytes,
                       dispatch_sha256, index_bytes, index["artifact_sha256"])

    def import_native_cleanup(self, output, cleanup_sha256):
        """Preserve native requires_drain as a descriptive, non-authoritative notice."""
        self._check()
        if not isinstance(output, ProtectedOutput) or self._dispatch_hash is None:
            _fail("JOURNAL_PRIVATE")
        output.revalidate()
        _hash(cleanup_sha256)
        try:
            data = _read_named(output._fd, cleanup_sha256[7:] + ".json", 0o400, MAX_SMALL)
        except FileNotFoundError:
            _fail("JOURNAL_CUSTODY")
        if _sha(data) != cleanup_sha256:
            _fail("JOURNAL_CUSTODY")
        value = _json(data, MAX_SMALL)
        _exact(value, {"schema", "generation", "sql_request_sha256", "commit_started",
                       "commit_acknowledged", "requires_drain", "result"})
        if (value["schema"] != "owner-beta-paper-native-cleanup-v1"
                or value["generation"] != self._generation
                or value["sql_request_sha256"] != self._sql_request_hash
                or type(value["commit_started"]) is not bool
                or type(value["commit_acknowledged"]) is not bool
                or (value["commit_acknowledged"] and not value["commit_started"])
                or value["requires_drain"] is not True):
            _fail("JOURNAL_CUSTODY")
        result = value["result"]
        _exact(result, {"schema", "verb", "outcome", "event_id", "generation",
                        "control_epoch", "artifact_sha256", "error_code"})
        if (result["schema"] != "owner-beta-paper-ops-result-v1"
                or result["verb"] != "enable"
                or result["outcome"] != "COMMIT_UNKNOWN"
                or result["error_code"] != "COMMIT_UNKNOWN"
                or result["generation"] != self._generation):
            _fail("JOURNAL_CUSTODY")
        _uuid(result["generation"])
        _uuid(result["event_id"], nullable=True)
        _counter(result["control_epoch"]) if result["control_epoch"] is not None else None
        _hash(result["artifact_sha256"], nullable=True)
        output.revalidate()
        self._store_original(data, cleanup_sha256, MAX_SMALL)
        self._check()
        return CleanupNotice(_TOKEN, cleanup_sha256, data)

    @staticmethod
    def _verify_output_object(output, object_hash, expected, maximum):
        _hash(object_hash)
        try:
            actual = _read_named(output._fd, object_hash[7:] + ".json", 0o400, maximum)
        except FileNotFoundError:
            _fail("JOURNAL_CUSTODY")
        if actual != expected or _sha(actual) != object_hash:
            _fail("JOURNAL_CUSTODY")

    def _validate_exact_request(self, data, artifact, artifact_bytes):
        request = _json(data, MAX_NATIVE_EXACT)
        _exact(request, {"schema", "kind", "account_id", "args"})
        if request["schema"] != "owner-beta-paper-exact-request-v1":
            _fail("JOURNAL_CUSTODY")
        account = request["account_id"]
        _uuid(account, nullable=True)
        if account != self._intent["account_id"]:
            _fail("JOURNAL_IDENTITY")
        kind = request["kind"]
        args = request["args"]
        maps = {
            "CONFIGURE": {"strategy_config_id", "account_name", "contract_canonical", "process_binding", "request_key"},
            "BEGIN": {"expected_epoch", "intent_canonical", "request_key"},
            "RECONCILE": {"generation", "expected_epoch", "inventory_sha256", "request_key"},
            "TRANSITION": {"expected_epoch", "transition", "evidence_canonical", "request_key"},
            "RESET": {"expected_epoch"},
        }
        if not isinstance(kind, str) or kind not in maps or not isinstance(args, dict):
            _fail("JOURNAL_CUSTODY")
        _exact(args, maps[kind])
        if (kind == "CONFIGURE") != (account is None):
            _fail("JOURNAL_CUSTODY")
        if kind != "CONFIGURE":
            _counter(args["expected_epoch"])
            if args["expected_epoch"] != self._intent["expected_epoch"]:
                _fail("JOURNAL_IDENTITY")
        if kind != "RESET" and args.get("request_key") != self._intent["request_key"]:
            _fail("JOURNAL_IDENTITY")
        purpose_by_kind = {
            "CONFIGURE": {"CONFIGURE"},
            "BEGIN": {"STOP", "BACKUP", "RESTORE"},
            "RECONCILE": {"STOP", "BACKUP", "RESTORE"},
        }
        if kind in purpose_by_kind and self._intent["purpose"] not in purpose_by_kind[kind]:
            _fail("JOURNAL_IDENTITY")
        if kind == "BEGIN":
            if self._intent["purpose"] not in {"STOP", "BACKUP", "RESTORE"}:
                _fail("JOURNAL_IDENTITY")
            if self._intent["request_key"] != self._generation + ":begin":
                _fail("JOURNAL_IDENTITY")
            if not isinstance(args["intent_canonical"], str):
                _fail("JOURNAL_CUSTODY")
            inner = args["intent_canonical"].encode("ascii")
            if inner != self._intent_bytes:
                _fail("JOURNAL_IDENTITY")
        elif kind == "RECONCILE":
            _uuid(args["generation"])
            _hash(args["inventory_sha256"])
            if (args["generation"] != self._generation or artifact is None
                    or args["inventory_sha256"] != _sha(artifact_bytes)
                    or re.fullmatch(re.escape(self._generation) + r":reconcile:[0-9]{1,18}",
                                    self._intent["request_key"]) is None):
                _fail("JOURNAL_IDENTITY")
        elif kind == "CONFIGURE":
            if artifact is None or args["process_binding"] is not None:
                _fail("JOURNAL_CUSTODY")
            if (set(artifact) != {"schema", "strategy_config_id", "account_name", "contract"}
                    or artifact.get("schema") != "owner-beta-paper-configure-operation-v1"):
                _fail("JOURNAL_CUSTODY")
            _uuid(args["strategy_config_id"])
            account_name = args["account_name"]
            if account_name != "Owner-beta Paper":
                _fail("JOURNAL_CUSTODY")
            if not isinstance(args["contract_canonical"], str):
                _fail("JOURNAL_CUSTODY")
            contract = _embedded(args["contract_canonical"], MAX_NATIVE_EMBEDDED)
            if (args["strategy_config_id"] != artifact.get("strategy_config_id")
                    or args["account_name"] != artifact.get("account_name")):
                _fail("JOURNAL_IDENTITY")
            expected_contract = {
                "contract_id": "owner-beta-paper-v1-draft-1",
                "contract_sha256": "sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0",
                "decision_sha256": "sha256:8ef602a35586d3e2ca532c5fbb5efc6f2e727eb25ebeb45a44dad0ea0d0ca394",
                "cost_profile_id": "KRX_ETF_DEFAULT", "cost_profile_version": 1,
            }
            if not isinstance(contract, dict) or set(contract) != set(expected_contract) or contract != expected_contract or contract != artifact.get("contract"):
                _fail("JOURNAL_IDENTITY")
        elif kind == "TRANSITION":
            if not isinstance(args["transition"], str):
                _fail("JOURNAL_CUSTODY")
            purpose_by_transition = {"ENABLE": "ENABLE", "DRAIN": "STOP", "OFF": "STOP",
                                     "RESTORED_OFF": "RESTORE", "RECOVERY": "RECOVERY"}
            if purpose_by_transition.get(args["transition"]) != self._intent["purpose"]:
                _fail("JOURNAL_IDENTITY")
            required_key = {"ENABLE": "enable", "OFF": "off", "RESTORED_OFF": "restore-off",
                            "RECOVERY": "recover"}.get(args["transition"])
            if required_key is not None and self._intent["request_key"] != self._generation + ":" + required_key:
                _fail("JOURNAL_IDENTITY")
            evidence_bytes, evidence = _embedded_pair(args["evidence_canonical"], MAX_NATIVE_EMBEDDED)
            if not isinstance(evidence, dict):
                _fail("JOURNAL_CUSTODY")
            if artifact is None:
                _fail("JOURNAL_CUSTODY")
            if args["transition"] == "ENABLE":
                _counter(evidence.get("expected_epoch"))
                _counter(evidence.get("new_epoch"))
                binding = evidence.get("process_binding")
                cohort = evidence.get("current_cohort")
                if not isinstance(binding, dict) or not isinstance(cohort, dict):
                    _fail("JOURNAL_CUSTODY")
                if (binding != artifact or evidence.get("process_binding_sha256") != _sha(artifact_bytes)
                        or evidence.get("expected_epoch") != self._intent["expected_epoch"]
                        or evidence.get("new_epoch") != self._intent["expected_epoch"] + 1):
                    _fail("JOURNAL_IDENTITY")
                if (cohort.get("release_manifest_sha256") != self._intent["release_manifest_sha256"]
                        or cohort.get("cohort_sha256") != artifact.get("daily_cohort_sha256")
                        or cohort.get("owner_user_id") != artifact.get("owner_user_id")
                        or cohort.get("strategy_config_id") != artifact.get("strategy_config_id")
                        or artifact.get("schema") != "owner-beta-paper-process-v1"
                        or artifact.get("control_epoch") != self._intent["expected_epoch"] + 1
                        or artifact.get("account_id") != self._intent["account_id"]
                        or artifact.get("release_manifest_sha256") != self._intent["release_manifest_sha256"]):
                    _fail("JOURNAL_IDENTITY")
                process_keys = {
                    "schema", "owner_user_id", "account_id", "strategy_config_id", "consent_event_id",
                    "consent_version", "control_epoch", "daily_cohort_sha256",
                    "financial_contract_sha256", "code_commit", "release_manifest_sha256",
                    "api_binary_sha256", "worker_binary_sha256",
                }
                if set(artifact) != process_keys or artifact.get("consent_version") != "owner-beta-paper-v1-draft-1":
                    _fail("JOURNAL_CUSTODY")
                for key in ("owner_user_id", "account_id", "strategy_config_id", "consent_event_id"):
                    _uuid(artifact.get(key))
                for key in ("daily_cohort_sha256", "api_binary_sha256", "worker_binary_sha256",
                            "financial_contract_sha256"):
                    _hash(artifact.get(key))
                if (not isinstance(artifact.get("code_commit"), str)
                        or re.fullmatch(r"[0-9a-f]{40}", artifact["code_commit"]) is None):
                    _fail("JOURNAL_CUSTODY")
            elif evidence_bytes != artifact_bytes:
                _fail("JOURNAL_IDENTITY")

    def close(self):
        if not self._closed:
            self._closed = True
            for action in tuple(self._actions):
                action.close()
            os.close(self._intent_fd)
            if self._head_fd is not None:
                os.close(self._head_fd)
            for content_fd, _identity_value in self._original_fds.values():
                os.close(content_fd)
            os.close(self._fd)
            self._journal._operations.discard(self)


def _embedded_pair(value, maximum):
    if not isinstance(value, str):
        _fail("JOURNAL_CUSTODY")
    data = value.encode("ascii")
    return data, _json(data, maximum)


def _embedded(value, maximum):
    return _embedded_pair(value, maximum)[1]


def _validate_v2_dispatch(data, root_bytes, action_bytes, artifact_bytes):
    value = _json(data, MAX_NATIVE_INDEX)
    _exact(value, V2_DISPATCH_KEYS)
    if value["schema"] != "owner-beta-paper-native-dispatch-v2":
        _fail("JOURNAL_CUSTODY")
    pairs = (("root_intent_canonical", "root_intent_sha256", root_bytes),
             ("action_canonical", "action_sha256", action_bytes),
             ("sql_request_canonical", "sql_request_sha256", None))
    originals = []
    parsed = {}
    for canonical_key, hash_key, expected in pairs:
        if not isinstance(value[canonical_key], str):
            _fail("JOURNAL_CUSTODY")
        raw = value[canonical_key].encode("ascii")
        _hash(value[hash_key])
        if _sha(raw) != value[hash_key] or (expected is not None and raw != expected):
            _fail("JOURNAL_IDENTITY")
        maximum = MAX_NATIVE_EXACT if canonical_key == "sql_request_canonical" else MAX_SMALL
        _json(raw, maximum)
        parsed[canonical_key] = value[canonical_key]
        originals.append((value[hash_key], raw, maximum))
    artifact_canonical, artifact_hash = value["artifact_canonical"], value["artifact_sha256"]
    if artifact_canonical is None:
        if artifact_hash is not None or artifact_bytes is not None:
            _fail("JOURNAL_CUSTODY")
        artifact = None
    else:
        if not isinstance(artifact_canonical, str) or not isinstance(artifact_hash, str):
            _fail("JOURNAL_CUSTODY")
        artifact = artifact_canonical.encode("ascii")
        _hash(artifact_hash)
        if _sha(artifact) != artifact_hash or artifact != artifact_bytes:
            _fail("JOURNAL_IDENTITY")
        _json(artifact, MAX_SMALL)
        originals.append((artifact_hash, artifact, MAX_SMALL))
    action = _json(parsed["action_canonical"].encode("ascii"), MAX_SMALL)
    if artifact_hash != action["artifact_sha256"]:
        _fail("JOURNAL_IDENTITY")
    parsed.update({"root_intent_sha256": value["root_intent_sha256"],
                   "action_sha256": value["action_sha256"],
                   "artifact_canonical": artifact_canonical, "artifact_sha256": artifact_hash})
    originals.append((_sha(data), data, MAX_NATIVE_INDEX))
    return parsed, originals


def _validate_bound_request(data, intent, action, artifact_bytes):
    request = _json(data, MAX_NATIVE_EXACT)
    _exact(request, {"schema", "kind", "account_id", "args"})
    if request["schema"] != "owner-beta-paper-exact-request-v1":
        _fail("JOURNAL_CUSTODY")
    account = request["account_id"]
    _uuid(account, nullable=True)
    if account != intent["account_id"]:
        _fail("JOURNAL_IDENTITY")
    maps = {
        "CONFIGURE": {"strategy_config_id", "account_name", "contract_canonical", "process_binding", "request_key"},
        "BEGIN": {"expected_epoch", "intent_canonical", "request_key"},
        "RECONCILE": {"generation", "expected_epoch", "inventory_sha256", "request_key"},
        "TRANSITION": {"expected_epoch", "transition", "evidence_canonical", "request_key"},
        "RESET": {"expected_epoch"},
    }
    kind, args = request["kind"], request["args"]
    if kind not in maps or not isinstance(args, dict):
        _fail("JOURNAL_CUSTODY")
    _exact(args, maps[kind])
    verb = action["verb"]
    expected_kind = {"configure": "CONFIGURE", "begin": "BEGIN", "reconcile": "RECONCILE",
                     "off": "TRANSITION", "restore-off": "TRANSITION",
                     "enable": "TRANSITION", "recover": "TRANSITION"}.get(verb)
    if kind != expected_kind or (kind == "CONFIGURE") != (account is None):
        _fail("JOURNAL_IDENTITY")
    if kind != "RESET" and args.get("request_key") != action["request_key"]:
        _fail("JOURNAL_IDENTITY")
    if kind != "CONFIGURE":
        _counter(args["expected_epoch"])
        if args["expected_epoch"] != action["expected_epoch"]:
            _fail("JOURNAL_IDENTITY")
    if verb == "begin":
        if args["intent_canonical"].encode("ascii") != _canonical(intent):
            _fail("JOURNAL_IDENTITY")
    elif verb == "reconcile":
        _uuid(args["generation"])
        _hash(args["inventory_sha256"])
        if args["generation"] != intent["generation"] or args["inventory_sha256"] != action["artifact_sha256"]:
            _fail("JOURNAL_IDENTITY")
    elif verb == "configure":
        if artifact_bytes is None or args["process_binding"] is not None:
            _fail("JOURNAL_CUSTODY")
        artifact = _json(artifact_bytes, MAX_SMALL)
        if (args["strategy_config_id"] != artifact.get("strategy_config_id")
                or args["account_name"] != artifact.get("account_name")
                or _embedded(args["contract_canonical"], MAX_NATIVE_EMBEDDED) != artifact.get("contract")):
            _fail("JOURNAL_IDENTITY")
    else:
        expected_transition = {"off": "OFF", "restore-off": "RESTORED_OFF",
                               "enable": "ENABLE", "recover": "RECOVERY"}[verb]
        if args["transition"] != expected_transition or artifact_bytes is None:
            _fail("JOURNAL_IDENTITY")
        evidence = args["evidence_canonical"].encode("ascii")
        decoded = _json(evidence, MAX_NATIVE_EMBEDDED)
        if verb in {"off", "restore-off", "recover"} and evidence != artifact_bytes:
            _fail("JOURNAL_IDENTITY")
        if verb == "enable":
            if (not isinstance(decoded, dict) or decoded.get("process_binding") != _json(artifact_bytes, MAX_SMALL)
                    or decoded.get("process_binding_sha256") != action["artifact_sha256"]):
                _fail("JOURNAL_IDENTITY")
    return request


def _custody_from_v2(token, journal, generation, sequence, op_fd, actions_fd,
                     action_fd, output_fd, root_fd, root_bytes, root_id,
                     binding_fd, action_bytes, action_id, evidence,
                     dispatch_bytes, readonly):
    intent = _validate_intent(root_bytes)
    action = _validate_action_readback(action_bytes, intent, sequence)
    if action_bytes != _canonical(action):
        _fail("JOURNAL_CUSTODY")
    artifact_bytes = None if evidence is None else evidence[1]
    parsed, originals = _validate_v2_dispatch(dispatch_bytes, root_bytes, action_bytes, artifact_bytes)
    _validate_bound_request(parsed["sql_request_canonical"].encode("ascii"), intent, action, artifact_bytes)
    if parsed["root_intent_sha256"] != _sha(root_bytes) or parsed["action_sha256"] != _sha(action_bytes):
        _fail("JOURNAL_CUSTODY")
    if parsed["artifact_sha256"] != action["artifact_sha256"]:
        _fail("JOURNAL_CUSTODY")
    import_bytes = _read_named(action_fd, "import.json", 0o400, MAX_NATIVE_INDEX)
    if import_bytes != dispatch_bytes:
        _fail("JOURNAL_CUSTODY")
    retained_files = [(root_fd, root_id, op_fd, "intent.json", 0o400, root_bytes),
                      (binding_fd, action_id, action_fd, "binding.json", 0o400, action_bytes)]
    if evidence is not None:
        retained_files.append((evidence[0], evidence[2], action_fd, "evidence.json", 0o400, evidence[1]))
    for digest, raw, maximum in originals:
        name = digest[7:] + ".json"
        fd, stored, identity = _open_named(output_fd, name, 0o400, maximum)
        if stored != raw:
            os.close(fd)
            _fail("JOURNAL_CUSTODY")
        retained_files.append((fd, identity, output_fd, name, 0o400, raw))
    import_fd, stored_import, import_id = _open_named(action_fd, "import.json", 0o400, MAX_NATIVE_INDEX)
    if stored_import != dispatch_bytes:
        os.close(import_fd)
        _fail("JOURNAL_CUSTODY")
    retained_files.append((import_fd, import_id, action_fd, "import.json", 0o400, dispatch_bytes))
    dispatch_fd, stored_dispatch, dispatch_id = _open_named(output_fd, "dispatch.json", 0o400, MAX_NATIVE_INDEX)
    if stored_dispatch != dispatch_bytes:
        os.close(dispatch_fd)
        _fail("JOURNAL_CUSTODY")
    retained_files.append((dispatch_fd, dispatch_id, output_fd, "dispatch.json", 0o400, dispatch_bytes))
    links = []
    for fd, parent, name in ((op_fd, journal._ops, generation),
                             (actions_fd, op_fd, "actions"),
                             (action_fd, actions_fd, "%08d" % sequence),
                             (output_fd, action_fd, "output")):
        links.append((fd, parent, name))
    custody = Custody(token, None, root_bytes, artifact_bytes,
                      parsed["sql_request_canonical"].encode("ascii"),
                      _sha(dispatch_bytes), dispatch_bytes, parsed["artifact_sha256"])
    custody._action = action_bytes
    custody._readonly = readonly
    custody._fds = tuple(retained_files)
    custody._ids = tuple((fd, _dir_identity(os.fstat(fd)), parent, name) for fd, parent, name in links)
    custody._generation, custody._sequence = generation, sequence
    custody._owned_dirs = (op_fd, actions_fd, action_fd, output_fd)
    return custody


class Custody:
    """Private descriptive originals imported from one native invocation."""
    __slots__ = ("_operation", "_intent", "_artifact", "_sql", "_dispatch_hash",
                 "_dispatch", "_artifact_hash", "_action", "_readonly", "_fds",
                 "_ids", "_owned_dirs", "_generation", "_sequence", "_closed")

    def __init__(self, token, operation, intent, artifact, sql, dispatch_hash, dispatch, artifact_hash):
        if token is not _TOKEN:
            _fail("JOURNAL_PRIVATE")
        self._operation = operation
        self._intent, self._artifact, self._sql = intent, artifact, sql
        self._dispatch_hash, self._dispatch, self._artifact_hash = dispatch_hash, dispatch, artifact_hash
        self._action = None
        self._readonly, self._fds, self._ids, self._owned_dirs = False, (), (), ()
        self._generation = operation.generation if operation is not None else None
        self._sequence, self._closed = None, False

    def _check(self):
        if self._closed:
            _fail("JOURNAL_CLOSED")
        if self._readonly:
            for fd, identity, parent, name, mode, expected in self._fds:
                _retained_file(parent, name, fd, identity, mode)
                if os.pread(fd, len(expected) + 1, 0) != expected:
                    _fail("JOURNAL_CUSTODY")
            for fd, identity, parent, name in self._ids:
                _check_named_dir(parent, name, fd)
            return
        self._operation._check()
        if self._action is not None:
            self._action.revalidate()

    def stage_readback(self, destination_fd):
        """Stage exact original request/evidence/SQL bytes into a fresh protected dir."""
        operation = self._operation
        self._check()
        directory = os.dup(destination_fd)
        try:
            _dir_info(directory, 0o700)
            with os.scandir(directory) as entries:
                if next(entries, None) is not None:
                    _fail("JOURNAL_DESTINATION")
            _immutable(directory, "request.json", self._intent, 0o400)
            if isinstance(self._action, ActionCustody):
                _immutable(directory, "action-binding.json", self._action._binding_bytes, 0o400)
            elif isinstance(self._action, bytes):
                _immutable(directory, "action-binding.json", self._action, 0o400)
            if self._artifact is not None:
                _immutable(directory, "evidence.json", self._artifact, 0o400)
            _immutable(directory, "sql-request.json", self._sql, 0o400)
            os.fsync(directory)
            _dir_info(directory, 0o700)
            self._check()
        finally:
            os.close(directory)

    def borrow_output_fd(self):
        """Return the original durable output FD, valid until this custody closes."""
        self._check()
        if isinstance(self._action, ActionCustody):
            return self._action.borrow_output_fd()
        if self._readonly and self._owned_dirs:
            return self._owned_dirs[-1]
        _fail("JOURNAL_CUSTODY")

    def close(self):
        if not self._closed:
            self._closed = True
            for item in self._fds:
                os.close(item[0])
            for fd in reversed(self._owned_dirs):
                os.close(fd)


class ActionCustody:
    """One retained immutable action and its private native output directory."""
    __slots__ = ("_operation", "_actions_fd", "_fd", "_output_fd", "_action",
                 "_binding_bytes", "_artifact_bytes", "_stage", "_binding_fd",
                 "_binding_id", "_evidence_fd", "_evidence_id", "_dispatch_fd",
                 "_dispatch_id", "_dispatch_bytes", "_import_fd", "_import_id", "_input_fd", "_input_ids", "_closed")

    def __init__(self, token, operation, actions_fd, action_fd, output_fd,
                 action, binding_bytes, artifact_bytes, stage):
        if token is not _TOKEN:
            _fail("JOURNAL_PRIVATE")
        self._operation, self._actions_fd, self._fd, self._output_fd = operation, actions_fd, action_fd, output_fd
        self._action, self._binding_bytes, self._artifact_bytes, self._stage = action, binding_bytes, artifact_bytes, stage
        self._binding_fd, binding_data, self._binding_id = _open_named(action_fd, "binding.json", 0o400, MAX_SMALL)
        if binding_data != binding_bytes:
            os.close(self._binding_fd)
            _fail("JOURNAL_CUSTODY")
        self._binding_id = _identity(os.fstat(self._binding_fd))
        self._evidence_fd = self._evidence_id = None
        if artifact_bytes is not None:
            self._evidence_fd, stored, self._evidence_id = _open_named(action_fd, "evidence.json", 0o400, MAX_SMALL)
            if stored != artifact_bytes:
                os.close(self._evidence_fd)
                _fail("JOURNAL_CUSTODY")
        self._dispatch_fd = self._dispatch_id = None
        self._dispatch_bytes = None
        self._import_fd = self._import_id = None
        self._input_fd, self._input_ids = None, ()
        self._closed = False

    @property
    def sequence(self):
        return self._action["sequence"]

    def borrow_output_fd(self):
        """Return the retained per-action output FD; valid until this custody closes."""
        self.revalidate()
        return self._output_fd

    def revalidate(self):
        if self._closed:
            _fail("JOURNAL_CLOSED")
        self._operation._check()
        _check_named_dir(self._operation._fd, "actions", self._actions_fd)
        _check_named_dir(self._actions_fd, "%08d" % self.sequence, self._fd)
        _check_named_dir(self._fd, "output", self._output_fd)
        _retained_file(self._fd, "binding.json", self._binding_fd, self._binding_id, 0o400)
        if os.pread(self._binding_fd, len(self._binding_bytes) + 1, 0) != self._binding_bytes:
            _fail("JOURNAL_CUSTODY")
        if self._evidence_fd is not None:
            _retained_file(self._fd, "evidence.json", self._evidence_fd, self._evidence_id, 0o400)
            if os.pread(self._evidence_fd, len(self._artifact_bytes) + 1, 0) != self._artifact_bytes:
                _fail("JOURNAL_CUSTODY")
        if self._dispatch_fd is not None:
            _retained_file(self._output_fd, "dispatch.json", self._dispatch_fd, self._dispatch_id, 0o400)
            if os.pread(self._dispatch_fd, len(self._dispatch_bytes) + 1, 0) != self._dispatch_bytes:
                _fail("JOURNAL_CUSTODY")
        if self._import_fd is not None:
            _retained_file(self._fd, "import.json", self._import_fd, self._import_id, 0o400)
            if os.pread(self._import_fd, len(self._dispatch_bytes) + 1, 0) != self._dispatch_bytes:
                _fail("JOURNAL_CUSTODY")
        else:
            try:
                imported = _read_named(self._fd, "import.json", 0o400, MAX_NATIVE_INDEX)
            except FileNotFoundError:
                imported = None
            if imported is not None:
                try:
                    dispatched = _read_named(self._output_fd, "dispatch.json", 0o400, MAX_NATIVE_INDEX)
                except FileNotFoundError:
                    _fail("JOURNAL_CUSTODY")
                if imported != dispatched:
                    _fail("JOURNAL_CUSTODY")
        if self._input_fd is not None:
            _dir_info(self._input_fd, 0o700)
            for name, fd, identity, expected in self._input_ids:
                _retained_file(self._input_fd, name, fd, identity, 0o400)
                if os.pread(fd, len(expected) + 1, 0) != expected:
                    _fail("JOURNAL_CUSTODY")

    def stage_inputs(self, destination_fd):
        """Materialize exact R/A/E originals in the fixed native input directory."""
        self.revalidate()
        directory = os.dup(destination_fd)
        try:
            _dir_info(directory, 0o700)
            if self._input_fd is not None:
                if _dir_identity(os.fstat(directory)) != _dir_identity(os.fstat(self._input_fd)):
                    _fail("JOURNAL_CONFLICT")
                self.revalidate()
                return
            with os.scandir(directory) as entries:
                if next(entries, None) is not None:
                    _fail("JOURNAL_DESTINATION")
            _immutable(directory, "request.json", self._operation._intent_bytes, 0o400)
            _immutable(directory, "action-binding.json", self._binding_bytes, 0o400)
            if self._artifact_bytes is not None:
                _immutable(directory, "evidence.json", self._artifact_bytes, 0o400)
            os.fsync(directory)
            retained = []
            for name in ("request.json", "action-binding.json", "evidence.json"):
                if name == "evidence.json" and self._artifact_bytes is None:
                    continue
                fd, data, identity = _open_named(directory, name, 0o400, MAX_SMALL)
                retained.append((name, fd, identity, data))
            self._input_fd = os.dup(directory)
            self._input_ids = tuple(retained)
            self.revalidate()
        finally:
            os.close(directory)

    def import_dispatch(self, output=None):
        """Import only this action's fixed dispatch.json; never scan/select by hash."""
        self.revalidate()
        output_fd = self._output_fd
        if output is not None:
            if not isinstance(output, ProtectedOutput):
                _fail("JOURNAL_PRIVATE")
            output.revalidate()
            if _dir_identity(os.fstat(output._fd)) != _dir_identity(os.fstat(output_fd)):
                _fail("JOURNAL_IDENTITY")
            output_fd = output._fd
        if self._dispatch_fd is None:
            self._dispatch_fd, self._dispatch_bytes, self._dispatch_id = _open_named(output_fd, "dispatch.json", 0o400, MAX_NATIVE_INDEX)
        try:
            dispatch = _read_named(output_fd, "dispatch.json", 0o400, MAX_NATIVE_INDEX)
        except FileNotFoundError:
            _fail("JOURNAL_CUSTODY")
        parsed, original_set = _validate_v2_dispatch(dispatch, self._operation._intent_bytes,
                                                     self._binding_bytes, self._artifact_bytes)
        _validate_bound_request(parsed["sql_request_canonical"].encode("ascii"),
                                self._operation._intent, self._action, self._artifact_bytes)
        for digest, raw, limit in original_set:
            name = digest[7:] + ".json"
            try:
                actual = _read_named(output_fd, name, 0o400, limit)
            except FileNotFoundError:
                _fail("JOURNAL_CUSTODY")
            if actual != raw:
                _fail("JOURNAL_CUSTODY")
        _immutable(self._fd, "import.json", dispatch, 0o400)
        os.fsync(self._fd)
        if self._import_fd is None:
            self._import_fd, _import_bytes, self._import_id = _open_named(self._fd, "import.json", 0o400, MAX_NATIVE_INDEX)
        self._dispatch_bytes = dispatch
        self.revalidate()
        action_custody = Custody(_TOKEN, self._operation, self._operation._intent_bytes,
                                 self._artifact_bytes,
                                 parsed["sql_request_canonical"].encode("ascii"),
                                 _sha(dispatch), dispatch, parsed["artifact_sha256"])
        action_custody._action = self
        action_custody._sequence = self.sequence
        return action_custody

    def close(self):
        if not self._closed:
            self._closed = True
            for fd in (self._binding_fd, self._evidence_fd, self._dispatch_fd, self._import_fd,
                       *(item[1] for item in self._input_ids), self._input_fd,
                       self._output_fd, self._fd, self._actions_fd):
                if fd is not None:
                    os.close(fd)
            self._operation._actions.discard(self)


class CleanupNotice:
    """Closed native failure metadata.  It never grants permission or resumption."""
    __slots__ = ("_sha256", "_canonical")

    def __init__(self, token, sha256, canonical_bytes):
        if token is not _TOKEN:
            _fail("JOURNAL_PRIVATE")
        self._sha256, self._canonical = sha256, canonical_bytes

    @property
    def requires_drain(self):
        return True

    @property
    def sha256(self):
        return self._sha256

    @property
    def canonical_bytes(self):
        return self._canonical


@contextlib.contextmanager
def open_journal(install_root, release_lock_fd, *, bootstrap=False):
    """Retain the install tree and borrowed held release FD for one journal use.

    Bootstrap is explicit.  A missing identity beside any floor or operation
    history is an error; foreign backups are never used to initialize identity.
    """
    _require_root()
    tree = _Tree(install_root)
    root_fd = ops_fd = identity_fd = floor_fd = callers_fd = None
    journal = None
    try:
        lock_id = _check_lock(install_root, release_lock_fd, tree)
        root_fd = _open_child(tree.fd, PAPER_DIR, 0o700, create=bootstrap)
        try:
            ops_fd = _open_child(root_fd, "operations", 0o700, create=bootstrap)
        except BaseException:
            os.close(root_fd)
            root_fd = None
            raise
        has_id = has_floor = False
        with os.scandir(root_fd) as root_entries:
            for entry in root_entries:
                if entry.name == "installation-id":
                    has_id = True
                elif entry.name == "epoch-floor":
                    has_floor = True
                elif entry.name == "callers":
                    callers_fd = _open_child(root_fd, "callers", 0o700)
                elif entry.name != "operations":
                    _fail("JOURNAL_INCOMPLETE")
        with os.scandir(ops_fd) as operation_entries:
            has_history = next(operation_entries, None) is not None
        if not has_id:
            if not bootstrap or has_floor or has_history:
                _fail("JOURNAL_IDENTITY_MISSING")
            installation_id = str(uuid.uuid4())
            _immutable(root_fd, "installation-id", (installation_id + "\n").encode("ascii"), 0o600)
        identity_fd, raw_id, identity_id = _open_named(root_fd, "installation-id", 0o600, 64)
        if not re.fullmatch(rb"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\n", raw_id):
            _fail("JOURNAL_IDENTITY")
        installation_id = raw_id[:-1].decode("ascii")
        _uuid(installation_id)
        if not has_floor:
            if not bootstrap or has_id or has_history:
                _fail("JOURNAL_FLOOR_MISSING")
            _immutable(root_fd, "epoch-floor", b"0\n", 0o600)
        floor_fd, floor_raw, floor_id = _open_named(root_fd, "epoch-floor", 0o600, 32)
        if not re.fullmatch(rb"(?:0|[1-9][0-9]{0,18})\n", floor_raw) or int(floor_raw[:-1]) > MAX_I64:
            _fail("JOURNAL_FLOOR")
        floor = int(floor_raw[:-1])
        journal = Journal(_TOKEN, tree, release_lock_fd, lock_id, root_fd, ops_fd,
                          installation_id, floor, identity_fd, identity_id, floor_fd, floor_id,
                          callers_fd)
        callers_fd = None
        journal._check()
        yield journal
        journal._check()
    except JournalError:
        raise
    except OSError as error:
        code = "JOURNAL_IO"
        if error.errno in (errno.ELOOP, errno.ENOTDIR, errno.ESTALE):
            code = "JOURNAL_PATH"
        raise JournalError(code) from None
    finally:
        if journal is not None:
            journal.close()
        else:
            if identity_fd is not None:
                os.close(identity_fd)
            if floor_fd is not None:
                os.close(floor_fd)
            if callers_fd is not None:
                os.close(callers_fd)
            if ops_fd is not None:
                os.close(ops_fd)
            if root_fd is not None:
                os.close(root_fd)
            tree.close()
