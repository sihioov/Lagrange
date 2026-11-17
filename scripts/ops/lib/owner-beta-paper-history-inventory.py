#!/usr/bin/env python3
"""Fail-closed pre-switch Owner-beta Paper history inventory (Owner Option A).

The reader guard accepts only an in-process `PaperHistoryInventory` minted by
`prove()` in this module and consumed once by the guard in the same process,
under the caller's held release lock.  No boolean, file, receipt or argv value
is inventory.  The outcome is ABSENT or PRESENT; every other state raises
`PaperHistoryInventoryError` and the guard maps it to
`PAPER_RELEASE_HISTORY_UNKNOWN`.

Database state A (`PRE_PAPER`) is proved by one fixed read-only SQL statement
sent through the existing Daily control one-shot of the current release's
db-migrate image.  State B (`PAPER_SCHEMA`) delegates presence to the P4
INSPECT inventory hosted by the current Paper-capable release.

This module must be loaded only under `MODULE_NAME`.  Handles are bound to that
single module object; a second copy of the file mints a different class, which
the guard rejects.
"""
from __future__ import annotations

import datetime
import fcntl
import hashlib
import importlib.util
import json
import os
import re
import stat
import sys
import time
import uuid
from pathlib import Path

MODULE_NAME = "owner_beta_paper_history_inventory"
ROOT = str(Path(__file__).resolve().parents[3])
SQL_RELATIVE = "deploy/db/owner-beta-paper-history-inventory.sql"
MIGRATIONS_RELATIVE = "migrations"
OUTPUT_PREFIX = "PAPER_HISTORY_INVENTORY: "
PRE_PAPER = "PRE_PAPER"
PAPER_SCHEMA = "PAPER_SCHEMA"
ABSENT = "ABSENT"
PRESENT = "PRESENT"
PRE_PAPER_MAX = 65
PAPER_SCHEMA_MIN = 69
MAX_AGE_NS = 300 * 1_000_000_000
MAX_SQL = 64 * 1024
MAX_MIGRATION = 4 * 1024 * 1024
MAX_MANIFEST = 64 * 1024
MAX_RECEIPT = 4096
LOCK_NAME = ".lagrange-release.lock"
PAPER_PREFIX = ".owner-beta-paper"
PAPER_DIR = ".owner-beta-paper"
FLOOR_NAME = ".owner-beta-paper-reader-floor"
LIFECYCLE_NAME = ".owner-beta-paper-lifecycle"
LIFECYCLE_MEMBERS = ("anchors", "queries", "selections", "staging")
EVIDENCE_NAME = "owner-beta-paper-evidence"
PAPER_RELEASE_DIRECTORY = "owner-beta-paper"
PAPER_BINDING_NAMES = frozenset({"process.json", "config.json"})
CAPABILITY_NAME = "capability.json"
# Deliberately not `.owner-beta-paper*`: a receipt can never satisfy a marker
# test or the unknown-Paper-name predicate below.
RECEIPT_DIRECTORY = ".lagrange-paper-history-receipts"
RECEIPT_SCHEMA = "owner-beta-paper-history-inventory-receipt-v1"
DISABLED_MODES = ("", "disabled")
CODES = frozenset({
    "PAPER_INVENTORY_ARGUMENT", "PAPER_INVENTORY_PRIVATE", "PAPER_INVENTORY_DEPENDENCY",
    "PAPER_INVENTORY_LOCK", "PAPER_INVENTORY_LOCAL", "PAPER_INVENTORY_SOURCE",
    "PAPER_INVENTORY_LINEAGE", "PAPER_INVENTORY_SQL", "PAPER_INVENTORY_GENERATION",
    "PAPER_INVENTORY_PROBE", "PAPER_INVENTORY_REPLAY", "PAPER_INVENTORY_STALE",
    "PAPER_INVENTORY_RECEIPT", "PAPER_INVENTORY_CLOSED", "PAPER_INVENTORY_CLOSE",
})
COMMIT = re.compile(r"[0-9a-f]{40}\Z")
CURRENT = re.compile(r"releases/([0-9a-f]{40})\Z")
MIGRATION_NAME = re.compile(r"([0-9]{4})_[A-Za-z0-9_]+\.up\.sql\Z")
LINE = re.compile(r"PAPER_HISTORY_INVENTORY: (PRE_PAPER|PAPER_SCHEMA) max=([1-9][0-9]{0,8}) "
                  r"lineage=([0-9a-f]{64})\Z")
BOOT_ID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\Z")
_DAILY_NAME = "_lagrange_owner_beta_paper_daily"
_OPS_NAME = "owner_beta_paper_operations"


class PaperHistoryInventoryError(Exception):
    """Closed, non-sensitive inventory failure.  Never carries provider text."""

    def __init__(self, code):
        if code not in CODES:
            code = "PAPER_INVENTORY_PRIVATE"
        super().__init__(code)
        self.code = code


def _fail(code):
    raise PaperHistoryInventoryError(code)


def _require(condition, code):
    if not condition:
        _fail(code)


class _Phase:
    """Convert any failure inside one phase to that phase's closed code."""
    __slots__ = ("code",)

    def __init__(self, code):
        self.code = code

    def __enter__(self):
        return self

    def __exit__(self, kind, value, _traceback):
        if kind is None or not issubclass(kind, Exception):
            return False
        if isinstance(value, PaperHistoryInventoryError):
            return False
        raise PaperHistoryInventoryError(self.code) from None


# Patchable observation seams (time, boot, Docker client).
def _monotonic_ns():
    return time.monotonic_ns()


def _utc_now():
    return datetime.datetime.now(datetime.timezone.utc)


def _boot_id():
    with open("/proc/sys/kernel/random/boot_id", "rb") as handle:
        data = handle.read(64)
    value = data.decode("ascii").strip()
    _require(BOOT_ID.fullmatch(value) is not None and data == (value + "\n").encode("ascii"),
             "PAPER_INVENTORY_STALE")
    return value


def _docker(ops):
    return ops.daily.runtime.Docker()


def _this_module():
    module = sys.modules.get(MODULE_NAME)
    _require(__name__ == MODULE_NAME and module is not None
             and getattr(module, "PaperHistoryInventory", None) is PaperHistoryInventory,
             "PAPER_INVENTORY_PRIVATE")


def _daily():
    """The release module's shared Daily install helper copy (one per process)."""
    loaded = sys.modules.get(_DAILY_NAME)
    if loaded is not None:
        return loaded
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "owner-beta-daily-release.py")
    spec = importlib.util.spec_from_file_location(_DAILY_NAME, path)
    _require(spec is not None and spec.loader is not None, "PAPER_INVENTORY_DEPENDENCY")
    module = importlib.util.module_from_spec(spec)
    sys.modules[_DAILY_NAME] = module
    try:
        spec.loader.exec_module(module)
    except Exception:
        sys.modules.pop(_DAILY_NAME, None)
        _fail("PAPER_INVENTORY_DEPENDENCY")
    return module


def _ops():
    """Installed Paper operations module: DB identity helpers and the P4 probe."""
    loaded = sys.modules.get(_OPS_NAME)
    if loaded is not None:
        return loaded
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "owner-beta-paper-operations.py")
    spec = importlib.util.spec_from_file_location(_OPS_NAME, path)
    _require(spec is not None and spec.loader is not None, "PAPER_INVENTORY_DEPENDENCY")
    module = importlib.util.module_from_spec(spec)
    sys.modules[_OPS_NAME] = module
    try:
        spec.loader.exec_module(module)
    except Exception:
        sys.modules.pop(_OPS_NAME, None)
        _fail("PAPER_INVENTORY_DEPENDENCY")
    return module


def _is_commit(value):
    return type(value) is str and COMMIT.fullmatch(value) is not None and value != "0" * 40


def _canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"),
                      allow_nan=False).encode("utf-8")


def _validate_arguments(install_root, target_release, commit, *, staging):
    _require(type(install_root) is str and type(target_release) is str and _is_commit(commit)
             and type(staging) is bool and os.path.isabs(install_root)
             and os.path.normpath(install_root) == install_root and install_root != "/"
             and not install_root.endswith("/") and "\x00" not in install_root
             and "\x00" not in target_release, "PAPER_INVENTORY_ARGUMENT")
    valid = target_release == install_root + "/releases/" + commit
    if staging:
        prefix = install_root + "/releases/.staging." + commit + "."
        suffix = target_release[len(prefix):]
        valid = valid or (target_release.startswith(prefix) and suffix != ""
                          and "/" not in suffix and suffix not in (".", ".."))
    _require(valid, "PAPER_INVENTORY_ARGUMENT")


# ---------------------------------------------------------------------------
# Executing-code inputs: the fixed SQL text and the admissible output lines.
# The executing checkout is the code being run (and may be a user-owned
# checkout under sudo), so these are bounded no-follow reads of that same code,
# not protected root inputs.  The receipt records the SQL digest.
# ---------------------------------------------------------------------------
def _read_code_file(path, maximum):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        before = os.fstat(fd)
        _require(stat.S_ISREG(before.st_mode) and 0 < before.st_size <= maximum,
                 "PAPER_INVENTORY_LINEAGE")
        data = bytearray()
        while len(data) <= maximum:
            chunk = os.read(fd, min(65536, maximum + 1 - len(data)))
            if not chunk:
                break
            data.extend(chunk)
        after = os.fstat(fd)
        _require(len(data) == before.st_size and
                 (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) ==
                 (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns),
                 "PAPER_INVENTORY_LINEAGE")
        return bytes(data)
    finally:
        os.close(fd)


def _inventory_sql(root=None):
    data = _read_code_file(os.path.join(root or ROOT, SQL_RELATIVE), MAX_SQL)
    try:
        text = data.decode("ascii")
    except UnicodeError:
        _fail("PAPER_INVENTORY_LINEAGE")
    return text, "sha256:" + hashlib.sha256(data).hexdigest()


def migration_checksums(root=None):
    """Return ((version, sha384-hex), ...) for the executing code, contiguous from 1.

    sqlx 0.9 (`deploy/db/Dockerfile`) stores `checksum` as SHA-384 of the
    up-file text when no ignored characters are configured.
    """
    directory = os.path.join(root or ROOT, MIGRATIONS_RELATIVE)
    entries = {}
    for name in os.listdir(directory):
        if not name.endswith(".up.sql"):
            continue
        match = MIGRATION_NAME.fullmatch(name)
        _require(match is not None, "PAPER_INVENTORY_LINEAGE")
        version = int(match.group(1))
        _require(version not in entries, "PAPER_INVENTORY_LINEAGE")
        entries[version] = name
    versions = sorted(entries)
    _require(versions == list(range(1, len(versions) + 1)) and len(versions) >= PAPER_SCHEMA_MIN,
             "PAPER_INVENTORY_LINEAGE")
    return tuple((version, hashlib.sha384(_read_code_file(
        os.path.join(directory, entries[version]), MAX_MIGRATION)).hexdigest())
        for version in versions)


def expected_lines(checksums):
    """Closed output set: state A for max 1..65, state B for max 69..(code max)."""
    lines, parts = set(), []
    for version, digest in checksums:
        parts.append(str(version) + ":" + digest)
        lineage = hashlib.sha256(",".join(parts).encode("ascii")).hexdigest()
        if version <= PRE_PAPER_MAX:
            lines.add(OUTPUT_PREFIX + PRE_PAPER + " max=" + str(version) + " lineage=" + lineage)
        if version >= PAPER_SCHEMA_MIN:
            lines.add(OUTPUT_PREFIX + PAPER_SCHEMA + " max=" + str(version) + " lineage=" + lineage)
    return frozenset(lines)


def _parse_line(line):
    match = LINE.fullmatch(line) if type(line) is str else None
    _require(match is not None, "PAPER_INVENTORY_SQL")
    state, maximum, lineage = match.group(1), int(match.group(2)), match.group(3)
    _require((state == PRE_PAPER and maximum <= PRE_PAPER_MAX) or
             (state == PAPER_SCHEMA and maximum >= PAPER_SCHEMA_MIN), "PAPER_INVENTORY_SQL")
    return state, maximum, lineage


# ---------------------------------------------------------------------------
# Local observations (§2.1).  All read-only and fd-relative.
# ---------------------------------------------------------------------------
def _dir_identity(value):
    return (value.st_dev, value.st_ino, value.st_uid, value.st_gid, value.st_mode)


def _named_directory(daily, parent, name, fd):
    named = os.stat(name, dir_fd=parent, follow_symlinks=False)
    opened = os.fstat(fd)
    _require(stat.S_ISDIR(named.st_mode) and
             (named.st_dev, named.st_ino) == (opened.st_dev, opened.st_ino),
             "PAPER_INVENTORY_LOCAL")


def _absent(parent, name):
    try:
        os.stat(name, dir_fd=parent, follow_symlinks=False)
    except FileNotFoundError:
        return True
    return False


def _lifecycle(daily, install_fd):
    """ABSENT, or the exact empty installer skeleton (lifecycle.prepare_storage)."""
    try:
        fd = os.open(LIFECYCLE_NAME, daily.DIR_FLAGS, dir_fd=install_fd)
    except FileNotFoundError:
        _require(_absent(install_fd, LIFECYCLE_NAME), "PAPER_INVENTORY_LOCAL")
        return "ABSENT", None
    try:
        root = _dir_identity(daily.directory_info(fd, uid=0, gid=0, mode=0o700))
        _named_directory(daily, install_fd, LIFECYCLE_NAME, fd)
        _require(tuple(sorted(os.listdir(fd))) == LIFECYCLE_MEMBERS, "PAPER_INVENTORY_LOCAL")
        members = []
        for name in LIFECYCLE_MEMBERS:
            child = os.open(name, daily.DIR_FLAGS, dir_fd=fd)
            try:
                members.append(_dir_identity(daily.directory_info(child, uid=0, gid=0, mode=0o700)))
                _named_directory(daily, fd, name, child)
                _require(os.listdir(child) == [], "PAPER_INVENTORY_LOCAL")
            finally:
                os.close(child)
        return "EMPTY_SKELETON", (root, tuple(members))
    finally:
        os.close(fd)


def _source_paper(daily, source_release):
    """Source `deploy/owner-beta-paper`: absent, or present without a binding."""
    with daily.root_directory(source_release) as release_fd:
        deploy_fd = os.open("deploy", daily.DIR_FLAGS, dir_fd=release_fd)
        try:
            daily.directory_info(deploy_fd, uid=0, gid=0)
            try:
                paper_fd = os.open(PAPER_RELEASE_DIRECTORY, daily.DIR_FLAGS, dir_fd=deploy_fd)
            except FileNotFoundError:
                _require(_absent(deploy_fd, PAPER_RELEASE_DIRECTORY), "PAPER_INVENTORY_LOCAL")
                return None
            try:
                identity = _dir_identity(daily.directory_info(paper_fd, uid=0, gid=10001, mode=0o750))
                _named_directory(daily, deploy_fd, PAPER_RELEASE_DIRECTORY, paper_fd)
                names = frozenset(os.listdir(paper_fd))
                # A source binding with no install-root journal is inconsistent:
                # it is never inferred as PRESENT, only UNKNOWN.
                _require(not (names & PAPER_BINDING_NAMES), "PAPER_INVENTORY_LOCAL")
                return identity, tuple(sorted(names))
            finally:
                os.close(paper_fd)
        finally:
            os.close(deploy_fd)


def _target_capable(daily, target_release):
    """Whether the target ships `deploy/owner-beta-paper/capability.json`.

    Same fd-relative, no-follow discipline as `_source_paper`; a target binding
    (process/config) is deliberately not examined here, the guard decides that
    first.  A symlink or non-regular capability entry is not capability.
    """
    with daily.root_directory(target_release) as release_fd:
        try:
            deploy_fd = os.open("deploy", daily.DIR_FLAGS, dir_fd=release_fd)
        except FileNotFoundError:
            _require(_absent(release_fd, "deploy"), "PAPER_INVENTORY_LOCAL")
            return False
        try:
            daily.directory_info(deploy_fd, uid=0, gid=0)
            try:
                paper_fd = os.open(PAPER_RELEASE_DIRECTORY, daily.DIR_FLAGS, dir_fd=deploy_fd)
            except FileNotFoundError:
                _require(_absent(deploy_fd, PAPER_RELEASE_DIRECTORY), "PAPER_INVENTORY_LOCAL")
                return False
            try:
                daily.directory_info(paper_fd, uid=0, gid=10001, mode=0o750)
                _named_directory(daily, deploy_fd, PAPER_RELEASE_DIRECTORY, paper_fd)
                try:
                    info = os.stat(CAPABILITY_NAME, dir_fd=paper_fd, follow_symlinks=False)
                except FileNotFoundError:
                    return False
                return stat.S_ISREG(info.st_mode)
            finally:
                os.close(paper_fd)
        finally:
            os.close(deploy_fd)


def _observe_local(daily, install_root, target_release):
    with daily.root_directory(install_root) as install_fd:
        root = os.fstat(install_fd)
        for name in (PAPER_DIR, FLOOR_NAME):
            _require(_absent(install_fd, name), "PAPER_INVENTORY_LOCAL")
        paper_names = tuple(sorted(name for name in os.listdir(install_fd)
                                   if name.startswith(PAPER_PREFIX)))
        # An unknown Paper-shaped install-root name is never "absent".
        _require(set(paper_names) <= {LIFECYCLE_NAME}, "PAPER_INVENTORY_LOCAL")
        lifecycle, lifecycle_identity = _lifecycle(daily, install_fd)
        try:
            link = os.readlink("current", dir_fd=install_fd)
        except FileNotFoundError:
            _fail("PAPER_INVENTORY_LOCAL")
        _require(stat.S_ISLNK(os.stat("current", dir_fd=install_fd, follow_symlinks=False).st_mode),
                 "PAPER_INVENTORY_LOCAL")
        match = CURRENT.fullmatch(link) if type(link) is str else None
        _require(match is not None and _is_commit(match.group(1)), "PAPER_INVENTORY_LOCAL")
    source_commit = match.group(1)
    source_release = install_root + "/" + link
    source_paper = _source_paper(daily, source_release)
    source_manifest = daily.read_root_file(source_release + "/.lagrange-release-manifest",
                                           gid=0, mode=0o600, maximum=MAX_MANIFEST)
    target_manifest = daily.read_root_file(target_release + "/.lagrange-release-manifest",
                                           gid=0, mode=0o600, maximum=MAX_MANIFEST)
    return {
        "install_identity": (root.st_dev, root.st_ino),
        "paper_names": paper_names,
        "lifecycle": lifecycle,
        "lifecycle_identity": lifecycle_identity,
        "current": link,
        "source_commit": source_commit,
        "source_release": source_release,
        "source_paper": source_paper,
        "source_capable": source_paper is not None and CAPABILITY_NAME in source_paper[1],
        "target_capable": _target_capable(daily, target_release),
        "source_manifest_sha256": "sha256:" + hashlib.sha256(source_manifest).hexdigest(),
        "target_manifest_sha256": "sha256:" + hashlib.sha256(target_manifest).hexdigest(),
    }


def _observe_source(ops, local, target_release, commit):
    """Source release binding; both protected envs must keep Paper disabled."""
    daily_ops = ops.daily
    source_release, source_commit = local["source_release"], local["source_commit"]
    with _Phase("PAPER_INVENTORY_SOURCE"):
        images = daily_ops.read_manifest(source_release, source_commit)
        _require(type(images) is dict and type(images.get("db-migrate")) is str,
                 "PAPER_INVENTORY_SOURCE")
        project, _mode = daily_ops.project_binding(source_commit, commit)
        values, overlays = daily_ops.read_context(source_release, source_commit, "disable")
        target_values, _target_overlays = daily_ops.read_context(target_release, commit, "disable")
        _require(type(values) is dict and type(target_values) is dict and type(overlays) is list
                 and type(project) is str, "PAPER_INVENTORY_SOURCE")
    source_mode = values.get("OWNER_BETA_PAPER_MODE")
    target_mode = target_values.get("OWNER_BETA_PAPER_MODE")
    _require(source_mode in DISABLED_MODES and target_mode in DISABLED_MODES, "PAPER_INVENTORY_LOCAL")
    return {"images": dict(images), "project": project, "values": dict(values),
            "overlays": list(overlays), "source_mode": source_mode, "target_mode": target_mode}


def _evidence_root(values):
    data_dir = values.get("LAGRANGE_DATA_DIR")
    _require(type(data_dir) is str and data_dir.startswith("/") and "\x00" not in data_dir
             and all(part not in ("", ".", "..") for part in data_dir[1:].split("/")),
             "PAPER_INVENTORY_LOCAL")
    try:
        os.lstat(data_dir + "/" + EVIDENCE_NAME)
    except FileNotFoundError:
        return "ABSENT"
    # A pre-Paper database cannot own Paper KIS witness evidence.
    _fail("PAPER_INVENTORY_LOCAL")


def _database_observation(ops, docker, project):
    with _Phase("PAPER_INVENTORY_GENERATION"):
        image_id = ops._database_image_id(docker)
        network_id, generation = ops._capture_database(docker, project, image_id)
    _require(type(image_id) is str and type(network_id) is str and type(generation) is tuple
             and len(generation) == 5, "PAPER_INVENTORY_GENERATION")
    return image_id, network_id, generation


def _classify(ops, docker, source, local, sql, expected):
    """One fixed SQL statement bracketed by equal database container generations."""
    before = _database_observation(ops, docker, source["project"])
    with _Phase("PAPER_INVENTORY_SQL"):
        database = ops.daily.Database(docker, source["project"], local["source_commit"],
                                      source["images"]["db-migrate"])
        line = database.execute(source["values"], sql, expected)
    after = _database_observation(ops, docker, source["project"])
    _require(before == after, "PAPER_INVENTORY_GENERATION")
    _require(type(line) is str and line in expected, "PAPER_INVENTORY_SQL")
    state, maximum, lineage = _parse_line(line)
    return line, state, maximum, lineage, before


def _check_lock(daily, install_fd, lock_fd):
    _require(type(lock_fd) is int and lock_fd >= 3, "PAPER_INVENTORY_LOCK")
    with _Phase("PAPER_INVENTORY_LOCK"):
        info = daily.file_info(lock_fd, 0, 0, 0o600)
        _require(info.st_size == 0, "PAPER_INVENTORY_LOCK")
        named = os.stat(LOCK_NAME, dir_fd=install_fd, follow_symlinks=False)
        _require(stat.S_ISREG(named.st_mode) and
                 (named.st_dev, named.st_ino) == (info.st_dev, info.st_ino),
                 "PAPER_INVENTORY_LOCK")
        # Same inherited open file description: confirms the exclusive lock
        # is held without opening a second lock path.  Never LOCK_UN here.
        fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    return (info.st_dev, info.st_ino)


_MINT = object()


class PaperHistoryInventory:
    """Single-use in-process history evidence.  Its constructor is private."""
    __slots__ = (
        "outcome", "state", "install_root", "install_identity", "lock_identity",
        "source_commit", "target_commit", "target_release", "migration_max",
        "lineage_sha256", "database_generation", "network_id", "proved_monotonic_ns",
        "boot_id", "_staging", "_consumed", "_closed", "_lock_fd", "_local", "_source",
        "_line", "_sql", "_sql_sha256", "_expected", "_database_image_id", "_docker",
        "_evidence", "_ops_context", "_p4_inventory", "_p4_index_sha256", "_released",
    )

    def __init__(self, token, **values):
        if token is not _MINT:
            raise TypeError("PaperHistoryInventory has a private constructor")
        _require(set(values) == set(self.__slots__), "PAPER_INVENTORY_PRIVATE")
        for key, value in values.items():
            object.__setattr__(self, key, value)

    def __setattr__(self, _name, _value):
        raise AttributeError("PaperHistoryInventory is immutable")

    def __delattr__(self, _name):
        raise AttributeError("PaperHistoryInventory is immutable")

    def __reduce_ex__(self, _protocol):
        raise TypeError("PaperHistoryInventory cannot be copied or serialized")

    def __copy__(self):
        raise TypeError("PaperHistoryInventory cannot be copied")

    def __deepcopy__(self, _memo):
        raise TypeError("PaperHistoryInventory cannot be copied")

    def __repr__(self):
        return "PaperHistoryInventory(outcome=%s, state=%s)" % (self.outcome, self.state)

    def _set(self, name, value):
        object.__setattr__(self, name, value)

    def _require_open(self):
        _require(not self._closed, "PAPER_INVENTORY_CLOSED")

    def revalidate(self):
        """Rerun local capture, the SQL line and the generation check (and P4)."""
        self._require_open()
        _require(not self._released, "PAPER_INVENTORY_CLOSED")
        daily, ops = _daily(), _ops()
        with _Phase("PAPER_INVENTORY_LOCK"):
            with daily.root_directory(self.install_root) as install_fd:
                _require(self.lock_identity == _check_lock(daily, install_fd, self._lock_fd),
                         "PAPER_INVENTORY_LOCK")
        with _Phase("PAPER_INVENTORY_STALE"):
            _require(_boot_id() == self.boot_id, "PAPER_INVENTORY_STALE")
        with _Phase("PAPER_INVENTORY_LOCAL"):
            local = _observe_local(daily, self.install_root, self.target_release)
        _require(local == self._local, "PAPER_INVENTORY_LOCAL")
        source = _observe_source(ops, local, self.target_release, self.target_commit)
        _require(source == self._source, "PAPER_INVENTORY_SOURCE")
        line, state, maximum, lineage, database = _classify(
            ops, self._docker, source, local, self._sql, self._expected)
        _require(line == self._line and state == self.state and maximum == self.migration_max
                 and lineage == self.lineage_sha256, "PAPER_INVENTORY_SQL")
        _require(database == (self._database_image_id, self.network_id, self.database_generation),
                 "PAPER_INVENTORY_GENERATION")
        if self.state == PRE_PAPER:
            with _Phase("PAPER_INVENTORY_LOCAL"):
                _require(_evidence_root(source["values"]) == self._evidence, "PAPER_INVENTORY_LOCAL")
        else:
            with _Phase("PAPER_INVENTORY_PROBE"):
                self._p4_inventory.revalidate()
        return None

    def consume(self, install_root, target_release, commit):
        """Single use by the reader guard; returns "ABSENT" or "PRESENT"."""
        self._require_open()
        _require(not self._consumed, "PAPER_INVENTORY_REPLAY")
        self._set("_consumed", True)
        # A staging (pre-publication) proof is never authority for a switch.
        _require(not self._staging and install_root == self.install_root
                 and target_release == self.target_release and commit == self.target_commit,
                 "PAPER_INVENTORY_REPLAY")
        daily = _daily()
        with _Phase("PAPER_INVENTORY_REPLAY"):
            with daily.root_directory(install_root) as install_fd:
                root = os.fstat(install_fd)
                _require((root.st_dev, root.st_ino) == self.install_identity, "PAPER_INVENTORY_REPLAY")
                _require(_check_lock(daily, install_fd, self._lock_fd) == self.lock_identity,
                         "PAPER_INVENTORY_REPLAY")
        with _Phase("PAPER_INVENTORY_STALE"):
            _require(_boot_id() == self.boot_id, "PAPER_INVENTORY_STALE")
            age = _monotonic_ns() - self.proved_monotonic_ns
            _require(type(age) is int and 0 <= age <= MAX_AGE_NS, "PAPER_INVENTORY_STALE")
        self.revalidate()
        if self.outcome == PRESENT:
            # The guard reserves the floor next; the P4 local markers would
            # then refuse to close.  Release the probe before that happens.
            self._release_probe()
        return self.outcome

    def _release_probe(self):
        if self._released:
            return
        self._set("_released", True)
        failed = False
        for handle in (self._p4_inventory, self._ops_context):
            if handle is None:
                continue
            try:
                handle.close()
            except Exception:
                failed = True
        _require(not failed, "PAPER_INVENTORY_CLOSE")

    def close(self):
        """Close retained descriptors, the P4 inventory and its ops context."""
        if self._closed:
            return None
        self._set("_closed", True)
        failed = False
        try:
            self._release_probe()
        except Exception:
            failed = True
        try:
            os.close(self._lock_fd)
        except OSError:
            failed = True
        _require(not failed, "PAPER_INVENTORY_CLOSE")
        return None

    def __del__(self):
        try:
            closed = object.__getattribute__(self, "_closed")
            lock_fd = object.__getattribute__(self, "_lock_fd")
        except AttributeError:
            return
        if not closed:
            object.__setattr__(self, "_closed", True)
            try:
                os.close(lock_fd)
            except OSError:
                pass


def prove(install_root, target_release, commit, lock_fd, *, staging=False):
    """Mint ABSENT or PRESENT history evidence, or raise (UNKNOWN)."""
    with _Phase("PAPER_INVENTORY_ARGUMENT"):
        _validate_arguments(install_root, target_release, commit, staging=staging)
        _require(type(lock_fd) is int and lock_fd >= 3, "PAPER_INVENTORY_LOCK")
    _this_module()
    with _Phase("PAPER_INVENTORY_DEPENDENCY"):
        daily, ops = _daily(), _ops()
    with _Phase("PAPER_INVENTORY_LOCAL"):
        daily.require_root()
    with _Phase("PAPER_INVENTORY_STALE"):
        boot_id, started = _boot_id(), _monotonic_ns()
    retained = {"lock": None, "context": None, "p4": None}
    try:
        with _Phase("PAPER_INVENTORY_LOCK"):
            with daily.root_directory(install_root) as install_fd:
                lock_identity = _check_lock(daily, install_fd, lock_fd)
                retained["lock"] = os.dup(lock_fd)
        with _Phase("PAPER_INVENTORY_LOCAL"):
            local = _observe_local(daily, install_root, target_release)
        with _Phase("PAPER_INVENTORY_LINEAGE"):
            sql, sql_sha256 = _inventory_sql()
            expected = expected_lines(migration_checksums())
        source = _observe_source(ops, local, target_release, commit)
        with _Phase("PAPER_INVENTORY_SQL"):
            docker = _docker(ops)
        line, state, maximum, lineage, database = _classify(ops, docker, source, local, sql, expected)
        evidence, index_sha256, outcome = "NOT_CHECKED", None, None
        if state == PRE_PAPER:
            with _Phase("PAPER_INVENTORY_LOCAL"):
                evidence = _evidence_root(source["values"])
            outcome = ABSENT
        else:
            # State B: presence is the P4 INSPECT inventory hosted by the
            # current Paper-capable release, consumed as its reader.
            _require(local["source_capable"], "PAPER_INVENTORY_SOURCE")
            # A pre-Paper target over a Paper schema (a rollback past 0066-0069)
            # cannot read the Paper history it would strand: never ABSENT.
            _require(local["target_capable"], "PAPER_INVENTORY_LOCAL")
            with _Phase("PAPER_INVENTORY_PROBE"):
                context = ops.daily.Context(local["source_release"], local["source_commit"],
                                            dict(source["images"]), dict(source["values"]),
                                            list(source["overlays"]))
                retained["context"] = ops.open_context(context, retained["lock"])
                retained["p4"] = retained["context"].probe_history()
                retained["p4"].revalidate()
                presence = retained["p4"]._consume_presence(ops._READER_CONSUMER)
                index = retained["p4"]._index
                _require(type(index) is bytes and index, "PAPER_INVENTORY_PROBE")
                index_sha256 = "sha256:" + hashlib.sha256(index).hexdigest()
            if presence is ops._HISTORY_EMPTY:
                outcome = ABSENT
            elif presence is ops._HISTORY_PRESENT:
                outcome = PRESENT
            else:
                _fail("PAPER_INVENTORY_PROBE")
        image_id, network_id, generation = database
        inventory = PaperHistoryInventory(
            _MINT, outcome=outcome, state=state, install_root=install_root,
            install_identity=local["install_identity"], lock_identity=lock_identity,
            source_commit=local["source_commit"], target_commit=commit,
            target_release=target_release, migration_max=maximum, lineage_sha256=lineage,
            database_generation=generation, network_id=network_id,
            proved_monotonic_ns=started, boot_id=boot_id, _staging=staging, _consumed=False,
            _closed=False, _lock_fd=retained["lock"], _local=local, _source=source, _line=line,
            _sql=sql, _sql_sha256=sql_sha256, _expected=expected, _database_image_id=image_id,
            _docker=docker, _evidence=evidence, _ops_context=retained["context"],
            _p4_inventory=retained["p4"], _p4_index_sha256=index_sha256, _released=False)
        retained = None
        return inventory
    except BaseException:
        if retained is not None:
            for key in ("p4", "context"):
                if retained[key] is not None:
                    try:
                        retained[key].close()
                    except Exception:
                        pass
            if retained["lock"] is not None:
                try:
                    os.close(retained["lock"])
                except OSError:
                    pass
        raise


def write_receipt(inventory, outcome):
    """Write the root-only LEGACY_OFF evidence receipt; return its sha256.

    The receipt is evidence only.  No parser may turn it into an inventory.
    """
    _this_module()
    _require(type(inventory) is PaperHistoryInventory, "PAPER_INVENTORY_PRIVATE")
    _require(not inventory._closed and not inventory._released, "PAPER_INVENTORY_CLOSED")
    _require(outcome == "LEGACY_OFF" and inventory.outcome == ABSENT and inventory._consumed
             and not inventory._staging, "PAPER_INVENTORY_RECEIPT")
    daily = _daily()
    with _Phase("PAPER_INVENTORY_RECEIPT"):
        now = _utc_now()
        _require(type(now) is datetime.datetime and now.utcoffset() == datetime.timedelta(0),
                 "PAPER_INVENTORY_RECEIPT")
        local, source = inventory._local, inventory._source
        container_id, created, started_at, restart_count, image_id = inventory.database_generation
        value = {
            "schema": RECEIPT_SCHEMA,
            "outcome": "LEGACY_OFF",
            "inventory": ABSENT,
            "state": inventory.state,
            "install_root": inventory.install_root,
            "install_root_identity": list(inventory.install_identity),
            "release_lock_identity": list(inventory.lock_identity),
            "source_commit": inventory.source_commit,
            "target_commit": inventory.target_commit,
            "source_manifest_sha256": local["source_manifest_sha256"],
            "target_manifest_sha256": local["target_manifest_sha256"],
            "inventory_sql_sha256": inventory._sql_sha256,
            "migration_max": inventory.migration_max,
            "migration_lineage_sha256": inventory.lineage_sha256,
            "database": {
                "container_id": container_id, "created": created, "started_at": started_at,
                "restart_count": restart_count, "image_id": image_id,
                "network_id": inventory.network_id,
                "database": source["values"].get("POSTGRES_DB") or "lagrange",
            },
            "p4_index_sha256": inventory._p4_index_sha256,
            "local": {
                "paper_dir": False, "reader_floor": False, "lifecycle": local["lifecycle"],
                "source_binding": False,
                # Written only after the guard found no target process binding.
                "target_binding": False,
                "evidence_root": inventory._evidence,
                "source_paper_mode": source["source_mode"],
                "target_paper_mode": source["target_mode"],
            },
            "boot_id": inventory.boot_id,
            "observed_at": now.strftime("%Y-%m-%dT%H:%M:%S.%fZ"),
        }
        payload = _canonical(value) + b"\n"
        _require(len(payload) <= MAX_RECEIPT, "PAPER_INVENTORY_RECEIPT")
        # <UTC microsecond timestamp>-<target commit>-<8 lowercase hex>.json: sorts by
        # time and never collides within one second or between concurrent writers.
        suffix = uuid.uuid4().hex[:8]
        _require(re.fullmatch(r"[0-9a-f]{8}", suffix) is not None, "PAPER_INVENTORY_RECEIPT")
        name = (now.strftime("%Y%m%dT%H%M%S.%fZ") + "-" + inventory.target_commit + "-"
                + suffix + ".json")
        with daily.root_directory(inventory.install_root) as install_fd:
            root = os.fstat(install_fd)
            _require((root.st_dev, root.st_ino) == inventory.install_identity,
                     "PAPER_INVENTORY_RECEIPT")
            created_directory = False
            try:
                os.mkdir(RECEIPT_DIRECTORY, 0o700, dir_fd=install_fd)
                created_directory = True
            except FileExistsError:
                pass
            fd = os.open(RECEIPT_DIRECTORY, daily.DIR_FLAGS, dir_fd=install_fd)
            try:
                if created_directory:
                    os.fchmod(fd, 0o700)
                daily.directory_info(fd, uid=0, gid=0, mode=0o700)
                _named_directory(daily, install_fd, RECEIPT_DIRECTORY, fd)
                # O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC, root:root 0400, file and
                # directory fsync.  An existing name is never replaced.
                daily.create_file(fd, name, payload, 0, 0, 0o400)
                _named_directory(daily, install_fd, RECEIPT_DIRECTORY, fd)
            finally:
                os.close(fd)
            if created_directory:
                os.fsync(install_fd)
    return "sha256:" + hashlib.sha256(payload).hexdigest()
