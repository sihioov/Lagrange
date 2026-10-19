#!/usr/bin/env python3
"""Bounded, descriptor-relative file snapshots for the daily backup controller.

This internal module never activates services. Extraction requires a new empty,
protected root directory; no existing destination is overwritten. The
controller must hold the release, private-state and Raw publication locks. A
snapshot records every included directory/file, then requires identical bytes
and identities through capture. Concurrent ordinary-data changes fail closed.
Archive members use logical classes, never host paths or environment/secrets.
"""
from __future__ import annotations

import contextlib
import dataclasses
import datetime
import errno
import hashlib
import json
import os
import re
import stat
import struct

CLASSES = ("raw", "curated", "daily", "config", "floor", "base")
# An explicitly selected, distinct format for a stopped pre-upgrade cluster.
# The daily caller never accepts this format or the postgres UID implicitly.
PREUPGRADE_CLASSES = ("raw", "curated", "base", "config", "postgres")
DAILY_TABLES = tuple("owner_beta_daily_" + suffix for suffix in (
    "control", "admissions", "admission_events", "calendar_sources", "calendar_days",
    "source_events", "poll_receipts", "poll_state", "reservations", "publication_claims"))
AUDIT_TABLES = (*DAILY_TABLES, "jobs", "job_attempts", "owner_beta_recommendation_runs",
                "owner_beta_recommendation_items")
MAGIC = b"LAGRANGE_DAILY_FILES_V1\n"
PREUPGRADE_MAGIC = b"LAGRANGE_PREUPGRADE_FILES_V1\n"
OWNERS = frozenset((0, 10001, 10002, 10003))
DIRECTORY = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
REGULAR = os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW | os.O_CLOEXEC
CHUNK = 1024 * 1024


class SnapshotError(Exception):
    """Only a fixed code, never a path or data body."""


def require(condition, code="DAILY_BACKUP_FILES_INVALID"):
    if not condition:
        raise SnapshotError(code)


def profile_values(profile):
    require(profile in {"daily", "preupgrade"}, "BACKUP_FILES_PROFILE")
    if profile == "preupgrade":
        return PREUPGRADE_CLASSES, PREUPGRADE_MAGIC, "lagrange-preupgrade-files-v1"
    return CLASSES, MAGIC, "lagrange-daily-files-v1"


def profile_owners(profile, name):
    profile_values(profile)
    return OWNERS | {999} if profile == "preupgrade" and name == "postgres" else OWNERS


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=True, allow_nan=False).encode("ascii")


def identity(info):
    return (info.st_dev, info.st_ino, info.st_mode, info.st_uid, info.st_gid,
            info.st_nlink, info.st_size, info.st_mtime_ns, info.st_ctime_ns)


def no_acl(fd):
    for name in ("system.posix_acl_access", "system.posix_acl_default"):
        try:
            os.getxattr(fd, name)
        except OSError as error:
            if error.errno == errno.ENODATA:
                continue
            raise SnapshotError("DAILY_BACKUP_FILES_ACL") from None
        raise SnapshotError("DAILY_BACKUP_FILES_ACL")


def safe_parts(path):
    require(isinstance(path, str) and 0 < len(path.encode("utf-8")) <= 4096 and
            not any(ord(c) < 32 or ord(c) == 127 or c == "\\" for c in path))
    parts = path.split("/")
    require(len(parts) <= 64 and all(p not in ("", ".", "..") for p in parts))
    return parts


def checked(fd, *, directory, owners=OWNERS, groups=OWNERS, sticky_volume=False):
    value = os.fstat(fd)
    # The official PG18 volume root can be postgres:postgres1777. Only the
    # cold profile's exact root may carry that mode; open_root additionally
    # requires its host parent to be root:root0700. No descendant or daily
    # root inherits this exception.
    private_pg_volume = (sticky_volume and directory and
                         (value.st_uid, value.st_gid, stat.S_IMODE(value.st_mode)) == (999, 999, 0o1777))
    require((stat.S_ISDIR(value.st_mode) if directory else stat.S_ISREG(value.st_mode))
            and value.st_uid in owners and value.st_gid in groups and
            (not value.st_mode & 0o7022 or private_pg_volume) and (directory or value.st_nlink == 1),
            "DAILY_BACKUP_FILES_TYPE_OR_MODE")
    no_acl(fd)
    return value


def checked_parent(fd):
    """Bind protected host ancestry independently of archived data identities.

    provision-linux.sh creates root:service-group0750 host parents. That group
    can be allocated by the host, unlike the fixed identities inside a data
    root. Root ownership, no group/other writes, no special bits and no ACL are
    still mandatory. open_root retains and rechecks the exact observed GID.
    """
    value = os.fstat(fd)
    require(stat.S_ISDIR(value.st_mode) and value.st_uid == 0 and
            not value.st_mode & 0o7022, "DAILY_BACKUP_FILES_TYPE_OR_MODE")
    no_acl(fd)
    return value


@dataclasses.dataclass(frozen=True)
class Limits:
    total_bytes: int
    entries: int = 200000
    manifest_bytes: int = 64 * 1024 * 1024

    def __post_init__(self):
        require(type(self.total_bytes) is int and 0 < self.total_bytes <= 2**50 and
                type(self.entries) is int and 6 <= self.entries <= 1000000 and
                type(self.manifest_bytes) is int and
                1024 <= self.manifest_bytes <= 64 * 1024 * 1024)


@dataclasses.dataclass(frozen=True)
class Root:
    name: str
    path: str
    uid: int
    gid: int
    mode: int
    leaf: str | None = None
    parent: tuple[int, int, int] | None = None
    # Explicit cold-source identities: / through the selected PostgreSQL root.
    # Restored/private trees and daily callers leave this empty.
    ancestry: tuple[tuple[int, int, int, int, int], ...] = ()

    def __post_init__(self):
        owners = OWNERS | {999} if self.name == "postgres" else OWNERS
        require(isinstance(self.ancestry, tuple), "PREUPGRADE_SOURCE_ANCESTRY")
        private_pg_volume = self.name == "postgres" and (self.uid, self.gid, self.mode) == (999, 999, 0o1777)
        require(self.name in {*CLASSES, *PREUPGRADE_CLASSES} and isinstance(self.path, str) and
                self.path.startswith("/") and type(self.uid) is int and self.uid in owners and
                type(self.gid) is int and self.gid in owners
                and type(self.mode) is int and
                ((0 < self.mode <= 0o777 and not self.mode & 0o022) or private_pg_volume))
        safe_parts(self.path[1:])
        if self.leaf is not None:
            require(len(safe_parts(self.leaf)) == 1)
        if self.parent is not None:
            require(self.name == "base" and len(self.parent) == 3 and
                    all(type(value) is int for value in self.parent) and
                    0 <= self.parent[0] < 2**32 - 1 and
                    self.parent[1:] == (10001, 0o750))
        if self.ancestry:
            require(self.name == "postgres" and self.leaf is None and self.parent is None and
                    isinstance(self.ancestry, tuple) and
                    len(self.ancestry) == len(safe_parts(self.path[1:])) + 1,
                    "PREUPGRADE_SOURCE_ANCESTRY")
            for index, entry in enumerate(self.ancestry):
                require(isinstance(entry, tuple) and len(entry) == 5 and
                        all(type(v) is int for v in entry) and
                        0 <= entry[0] < 2**64 and 0 < entry[1] < 2**64 and
                        all(0 <= v < 2**32 - 1 for v in entry[2:4]) and
                        0 < entry[4] <= 0o7777, "PREUPGRADE_SOURCE_ANCESTRY")
                if index < len(self.ancestry) - 1:
                    # This is an explicit trust in the fixed host's data-root
                    # owner, not a general permission for user-owned parents.
                    trusted_data = (index == 1 and self.path.startswith("/data/docker/volumes/") and
                                    entry[2:] == (1000, 1000, 0o755))
                    require((entry[2] == 0 and not entry[4] & 0o7022) or trusted_data,
                            "PREUPGRADE_SOURCE_ANCESTRY")
            require(self.ancestry[-1][2:] == (self.uid, self.gid, self.mode),
                    "PREUPGRADE_SOURCE_ANCESTRY")


def ancestor_identity(info):
    return (info.st_dev, info.st_ino, info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode))


def root_component(root, fd, index, count):
    last = index == count
    boundary = root.parent if index == count - 1 else None
    trusted_data = (bool(root.ancestry) and index == 1 and
                    root.path.startswith("/data/docker/volumes/") and
                    root.ancestry[index][2:] == (1000, 1000, 0o755))
    if trusted_data:
        info = os.fstat(fd)
        require(stat.S_ISDIR(info.st_mode), "PREUPGRADE_SOURCE_ANCESTRY")
        no_acl(fd)
    else:
        groups = OWNERS | {999} if root.name == "postgres" else OWNERS
        info = (checked_parent(fd) if not last and boundary is None else
                checked(fd, directory=True, owners={root.uid} if last else {boundary[0]},
                        groups=groups if last else OWNERS,
                        sticky_volume=last and root.name == "postgres"))
    if root.ancestry:
        require(ancestor_identity(info) == root.ancestry[index], "PREUPGRADE_SOURCE_ANCESTRY")
    return info


@contextlib.contextmanager
def open_root(root):
    """Keep the complete protected parent chain until final name verification."""
    names = safe_parts(root.path[1:])
    fds, before = [], []
    try:
        fds.append(os.open("/", DIRECTORY))
        before.append(root_component(root, fds[0], 0, len(names)))
        for index, name in enumerate(names):
            fds.append(os.open(name, DIRECTORY, dir_fd=fds[-1]))
            last = index == len(names) - 1
            boundary = root.parent if index == len(names) - 2 else None
            info = root_component(root, fds[-1], index + 1, len(names))
            if last and root.mode == 0o1777:
                parent = before[-1]
                require((parent.st_uid, parent.st_gid, stat.S_IMODE(parent.st_mode)) == (0, 0, 0o700),
                        "PREUPGRADE_VOLUME_PARENT")
            if boundary:
                require((info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode)) == boundary)
            if last:
                require(info.st_gid == root.gid and stat.S_IMODE(info.st_mode) == root.mode)
            before.append(info)
        yield fds[-1]
        # Ancestor child lists may change independently; their identity, access
        # policy and binding may not. Included root contents are checked below.
        for index, fd in enumerate(fds):
            info = root_component(root, fd, index, len(names))
            original = before[index]
            require((info.st_dev, info.st_ino, info.st_uid, info.st_gid, info.st_mode) ==
                    (original.st_dev, original.st_ino, original.st_uid,
                     original.st_gid, original.st_mode), "DAILY_BACKUP_FILES_CHANGED")
            if index:
                named = os.stat(names[index - 1], dir_fd=fds[index - 1], follow_symlinks=False)
                require((named.st_dev, named.st_ino) == (info.st_dev, info.st_ino),
                        "DAILY_BACKUP_FILES_CHANGED")
    finally:
        for fd in reversed(fds):
            os.close(fd)


def read_chunks(fd, size, digest, destination=None):
    remaining = size
    while remaining:
        part = os.read(fd, min(CHUNK, remaining))
        require(part, "DAILY_BACKUP_FILES_CHANGED")
        remaining -= len(part)
        digest.update(part)
        if destination is not None:
            write_all(destination, part)
    require(not os.read(fd, 1), "DAILY_BACKUP_FILES_CHANGED")


def write_all(stream, value):
    view = memoryview(value)
    while view:
        written = stream.write(view)
        require(type(written) is int and 0 < written <= len(view),
                "DAILY_BACKUP_FILES_WRITE")
        view = view[written:]


class Snapshot:
    def __init__(self, roots, limits, *, profile="daily"):
        self.classes, self.magic, self.schema = profile_values(profile)
        self.profile = profile
        require(tuple(r.name for r in roots) == self.classes)
        require(len({r.path for r in roots}) == len(self.classes))
        if profile == "preupgrade":
            require(not any(a.path.startswith(b.path + "/") for a in roots for b in roots if a is not b),
                    "PREUPGRADE_ROOT_OVERLAP")
        self.roots, self.limits = tuple(roots), limits
        self.stack = None
        self.descriptors = {}
        self.records = None
        self.identities = None

    def __enter__(self):
        require(self.stack is None)
        self.stack = contextlib.ExitStack()
        try:
            for root in self.roots:
                self.descriptors[root.name] = self.stack.enter_context(open_root(root))
            self.records, self.identities = self.scan()
            return self
        except BaseException:
            self.stack.close()
            self.stack = None
            raise

    def __exit__(self, *error):
        try:
            if error[0] is None:
                self.verify_unchanged()
        finally:
            self.stack.__exit__(*error)
            self.stack = None

    def scan(self, destination=None, expected=None):
        require(self.stack is not None)
        records, identities, total, metadata = [], {}, 0, 128

        def visit(fd, path, directory, parent=None, name=None, only=None):
            nonlocal total, metadata
            require(len(records) < self.limits.entries and len(safe_parts(path)) <= 64,
                    "DAILY_BACKUP_FILES_LIMIT")
            owners = profile_owners(self.profile, path.split("/", 1)[0])
            sticky_volume = self.profile == "preupgrade" and path == "postgres"
            before = checked(fd, directory=directory, owners=owners, groups=owners, sticky_volume=sticky_volume)
            record = {"path": path, "kind": "directory" if directory else "file",
                      "uid": before.st_uid, "gid": before.st_gid,
                      "mode": stat.S_IMODE(before.st_mode),
                      "size": 0 if directory else before.st_size, "sha256": None}
            metadata += len(canonical(record)) + 76
            require(metadata <= self.limits.manifest_bytes, "DAILY_BACKUP_FILES_LIMIT")
            index = len(records)
            records.append(record)
            identities[path] = identity(before)
            if directory:
                names, name_bytes = [], 0
                if only is not None:
                    names.append(only)
                else:
                    with os.scandir(fd) as members:
                        for item in members:
                            require(len(names) + len(records) < self.limits.entries,
                                    "DAILY_BACKUP_FILES_LIMIT")
                            safe_parts(item.name)
                            name_bytes += len(item.name.encode("utf-8"))
                            require(name_bytes <= self.limits.manifest_bytes,
                                    "DAILY_BACKUP_FILES_LIMIT")
                            names.append(item.name)
                for child in sorted(names):
                    named = os.stat(child, dir_fd=fd, follow_symlinks=False)
                    is_dir = stat.S_ISDIR(named.st_mode)
                    require(is_dir or stat.S_ISREG(named.st_mode), "DAILY_BACKUP_FILES_TYPE_OR_MODE")
                    handle = os.open(child, DIRECTORY if is_dir else REGULAR, dir_fd=fd)
                    try:
                        require(identity(os.fstat(handle)) == identity(named),
                                "DAILY_BACKUP_FILES_CHANGED")
                        visit(handle, path + "/" + child, is_dir, fd, child)
                    finally:
                        os.close(handle)
            else:
                total += before.st_size
                require(0 <= before.st_size and total <= self.limits.total_bytes,
                        "DAILY_BACKUP_FILES_LIMIT")
                digest = hashlib.sha256()
                read_chunks(fd, before.st_size, digest, destination)
                record["sha256"] = "sha256:" + digest.hexdigest()
            after = checked(fd, directory=directory, owners=owners, groups=owners, sticky_volume=sticky_volume)
            require(identity(before) == identity(after), "DAILY_BACKUP_FILES_CHANGED")
            if parent is not None:
                require(identity(after) == identity(os.stat(name, dir_fd=parent, follow_symlinks=False)),
                        "DAILY_BACKUP_FILES_CHANGED")
            if expected is not None:
                require(index < len(expected) and expected[index] == record,
                        "DAILY_BACKUP_FILES_CHANGED")

        for root in self.roots:
            visit(self.descriptors[root.name], root.name, True, only=root.leaf)
        require(expected is None or len(expected) == len(records), "DAILY_BACKUP_FILES_CHANGED")
        return records, identities

    def manifest(self):
        require(self.records is not None)
        value = {"schema": self.schema, "classes": list(self.classes),
                 "entries": self.records}
        data = canonical(value)
        require(len(data) <= self.limits.manifest_bytes, "DAILY_BACKUP_FILES_LIMIT")
        return data

    def write(self, destination):
        data = self.manifest()
        write_all(destination, self.magic + struct.pack(">Q", len(data)) + data)
        records, identities = self.scan(destination, self.records)
        require(identities == self.identities and records == self.records,
                "DAILY_BACKUP_FILES_CHANGED")

    def verify_unchanged(self):
        records, identities = self.scan(expected=self.records)
        require(identities == self.identities and records == self.records,
                "DAILY_BACKUP_FILES_CHANGED")


def exact(stream, size):
    value = bytearray()
    while len(value) < size:
        part = stream.read(min(CHUNK, size - len(value)))
        require(isinstance(part, bytes) and 0 < len(part) <= size - len(value),
                "DAILY_BACKUP_FILES_TRUNCATED")
        value.extend(part)
    return bytes(value)


def validate_manifest(data, limits, *, profile="daily"):
    classes, _, schema = profile_values(profile)
    try:
        value = json.loads(data)
        require(canonical(value) == data and set(value) == {"schema", "classes", "entries"}
                and value["schema"] == schema and value["classes"] == list(classes))
        entries = value["entries"]
        require(isinstance(entries, list) and len(classes) <= len(entries) <= limits.entries)
        seen, total, previous = {}, 0, None
        for record in entries:
            require(isinstance(record, dict) and set(record) ==
                    {"path", "kind", "uid", "gid", "mode", "size", "sha256"})
            parts = safe_parts(record["path"])
            require(parts[0] in classes and record["path"] not in seen)
            order = (classes.index(parts[0]), tuple(parts[1:]))
            require(previous is None or previous < order)
            previous = order
            owners = profile_owners(profile, parts[0])
            private_pg_volume = (profile == "preupgrade" and record["path"] == "postgres" and
                record["kind"] == "directory" and (record["uid"], record["gid"], record["mode"]) ==
                (999, 999, 0o1777))
            require(type(record["uid"]) is int and record["uid"] in owners and
                    type(record["gid"]) is int and record["gid"] in owners and
                    type(record["mode"]) is int and
                    ((0 <= record["mode"] <= 0o777 and not record["mode"] & 0o022) or private_pg_volume) and
                    type(record["size"]) is int and
                    record["size"] >= 0 and record["kind"] in {"file", "directory"})
            if len(parts) > 1:
                require(seen.get("/".join(parts[:-1])) == "directory")
            else:
                require(record["kind"] == "directory")
            if record["kind"] == "directory":
                require(record["size"] == 0 and record["sha256"] is None)
            else:
                require(isinstance(record["sha256"], str) and
                        re.fullmatch(r"sha256:[0-9a-f]{64}", record["sha256"]))
                total += record["size"]
                require(total <= limits.total_bytes, "DAILY_BACKUP_FILES_LIMIT")
            seen[record["path"]] = record["kind"]
        require(all(seen.get(name) == "directory" for name in classes))
        return value
    except (ValueError, TypeError, KeyError, UnicodeError, RecursionError):
        raise SnapshotError("DAILY_BACKUP_FILES_MANIFEST") from None


def verify_archive(source, limits, retain=(), *, profile="daily"):
    """Verify every byte; retain only explicitly requested small evidence files.

    No archive path is ever sent to an extraction API or opened on the host.
    Retained evidence is bounded in aggregate by the manifest-size allowance.
    """
    _, magic, _ = profile_values(profile)
    require(exact(source, len(magic)) == magic)
    size = struct.unpack(">Q", exact(source, 8))[0]
    require(0 < size <= limits.manifest_bytes, "DAILY_BACKUP_FILES_LIMIT")
    manifest = validate_manifest(exact(source, size), limits, profile=profile)
    requested = set(retain)
    require(len(requested) == len(retain) and len(requested) <= limits.entries)
    for path in requested:
        safe_parts(path)
    result, retained = {}, 0
    for entry in manifest["entries"]:
        if entry["kind"] != "file":
            continue
        keep = entry["path"] in requested
        if keep:
            retained += entry["size"]
            require(retained <= limits.manifest_bytes, "DAILY_BACKUP_FILES_LIMIT")
        digest, chunks, remaining = hashlib.sha256(), [], entry["size"]
        while remaining:
            part = exact(source, min(remaining, CHUNK))
            remaining -= len(part)
            digest.update(part)
            if keep:
                chunks.append(part)
        require("sha256:" + digest.hexdigest() == entry["sha256"], "DAILY_BACKUP_FILES_DIGEST")
        if keep:
            result[entry["path"]] = b"".join(chunks)
    require(set(result) == requested, "DAILY_BACKUP_FILES_MISSING")
    require(source.read(1) == b"", "DAILY_BACKUP_FILES_TRAILING")
    return manifest, result


def restore_archive(source, limits, destination, reserve=lambda _: None, *, profile="daily"):
    """Restore into an empty root-owned 0700 staging directory only.

    The controller supplies a disposable staging path, never a live data root.
    Parents are descriptor-relative and remain private until all hashes and EOF
    pass. On failure the caller discards this incomplete isolated staging tree.
    """
    _, magic, _ = profile_values(profile)
    require(exact(source, len(magic)) == magic)
    length = struct.unpack(">Q", exact(source, 8))[0]
    require(0 < length <= limits.manifest_bytes, "DAILY_BACKUP_FILES_LIMIT")
    manifest = validate_manifest(exact(source, length), limits, profile=profile)
    with open_root(Root("floor", destination, 0, 0, 0o700)) as root:
        require(not os.listdir(root), "DAILY_BACKUP_RESTORE_NOT_EMPTY")

        @contextlib.contextmanager
        def parent(parts):
            with contextlib.ExitStack() as stack:
                current = root
                for part in parts:
                    fd = os.open(part, DIRECTORY, dir_fd=current)
                    stack.callback(os.close, fd)
                    checked(fd, directory=True, owners={0})
                    current = fd
                yield current

        directories = []
        for entry in manifest["entries"]:
            parts = safe_parts(entry["path"])
            with parent(parts[:-1]) as directory:
                if entry["kind"] == "directory":
                    os.mkdir(parts[-1], mode=0o700, dir_fd=directory)
                    directories.append(entry)
                    continue
                fd = os.open(parts[-1], os.O_WRONLY | os.O_CREAT | os.O_EXCL |
                             os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=directory)
                try:
                    digest, remaining = hashlib.sha256(), entry["size"]
                    while remaining:
                        chunk = exact(source, min(CHUNK, remaining))
                        reserve(len(chunk))
                        view = memoryview(chunk)
                        while view:
                            written = os.write(fd, view)
                            require(written > 0, "DAILY_BACKUP_FILES_WRITE")
                            view = view[written:]
                        digest.update(chunk)
                        remaining -= len(chunk)
                    require("sha256:" + digest.hexdigest() == entry["sha256"],
                            "DAILY_BACKUP_FILES_DIGEST")
                    os.fchown(fd, entry["uid"], entry["gid"])
                    os.fchmod(fd, entry["mode"])
                    os.fsync(fd)
                finally:
                    os.close(fd)
        require(source.read(1) == b"", "DAILY_BACKUP_FILES_TRAILING")
        # Bottom-up metadata restoration keeps every traversed parent private
        # and root-owned until there are no further children to create beneath it.
        for entry in reversed(directories):
            parts = safe_parts(entry["path"])
            with parent(parts[:-1]) as directory:
                fd = os.open(parts[-1], DIRECTORY, dir_fd=directory)
                try:
                    os.fchown(fd, entry["uid"], entry["gid"])
                    os.fchmod(fd, entry["mode"])
                    os.fsync(fd)
                finally:
                    os.close(fd)
        os.fsync(root)
    return manifest


def database_references(manifest, database):
    """Bind verified archive entries to the fixed SQL audit's exact file pins.

    This proves byte/reference consistency, not independent source admission.
    Extra committed leaves are returned for the existing Rust admitter to
    reverify before enabling. A successful archive audit never admits them.
    """
    require(isinstance(database, dict) and set(database) == {
        "schema", "control", "tables", "run_count", "admissions", "verified_frontiers"}
        and database["schema"] == "lagrange-daily-backup-db-v1",
        "DAILY_BACKUP_DATABASE_SHAPE")
    control = database["control"]
    require(isinstance(control, dict) and set(control) == {
        "owner_user_id", "policy_sha256", "activation_epoch", "first_auto_session",
        "trigger_version", "enabled"} and control["enabled"] is False,
            "DAILY_BACKUP_DATABASE_ENABLED")
    require(isinstance(manifest, dict) and manifest.get("schema") == "lagrange-daily-files-v1"
            and isinstance(manifest.get("entries"), list))
    records = {entry["path"]: entry for entry in manifest["entries"]}
    require(len(records) == len(manifest["entries"]), "DAILY_BACKUP_DATABASE_SHAPE")

    def digest(value):
        require(isinstance(value, str) and re.fullmatch(r"sha256:[0-9a-f]{64}", value),
                "DAILY_BACKUP_DATABASE_SHAPE")
        return value

    digest(control["policy_sha256"])
    owner = control["owner_user_id"]
    require(isinstance(owner, str) and re.fullmatch(
                r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", owner)
            and owner != "00000000-0000-0000-0000-000000000000" and
            type(control["activation_epoch"]) is int and
            0 <= control["activation_epoch"] <= 2**63 - 1 and
            control["trigger_version"] == "owner-beta-daily-trigger-v1",
            "DAILY_BACKUP_DATABASE_SHAPE")
    first = control["first_auto_session"]
    require(isinstance(first, str) and re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", first),
            "DAILY_BACKUP_DATABASE_SHAPE")
    try:
        require(datetime.date.fromisoformat(first) > datetime.date(2026, 8, 28),
                "DAILY_BACKUP_DATABASE_SHAPE")
    except ValueError:
        raise SnapshotError("DAILY_BACKUP_DATABASE_SHAPE") from None

    require(type(database["run_count"]) is int and 0 <= database["run_count"] <= 100000 and
            isinstance(database["tables"], list) and len(database["tables"]) == len(AUDIT_TABLES),
            "DAILY_BACKUP_DATABASE_SHAPE")
    names = set()
    for table in database["tables"]:
        require(isinstance(table, dict) and set(table) == {"name", "row_count", "sha256"}
                and table["name"] in AUDIT_TABLES and table["name"] not in names and
                type(table["row_count"]) is int and 0 <= table["row_count"] <= 1100000,
                "DAILY_BACKUP_DATABASE_SHAPE")
        digest(table["sha256"])
        names.add(table["name"])

    def member(path, expected=None, size=None, *, uid=10003, mode=0o440):
        entry = records.get(path)
        require(entry is not None and entry["kind"] == "file" and
                entry["uid"] == uid and entry["gid"] == 10001 and entry["mode"] == mode,
                "DAILY_BACKUP_DATABASE_FILE")
        if expected is not None:
            require(entry["sha256"] == digest(expected), "DAILY_BACKUP_DATABASE_DIGEST")
        if size is not None:
            require(type(size) is int and size > 0 and entry["size"] == size,
                    "DAILY_BACKUP_DATABASE_SIZE")

    def leaves(base, names):
        result = {}
        for path, entry in records.items():
            if not path.startswith(base + "/"):
                continue
            parts = path[len(base) + 1:].split("/")
            require(len(parts) in (1, 2) and re.fullmatch(r"[0-9a-f]{64}", parts[0]),
                    "DAILY_BACKUP_DATABASE_LEAF")
            result.setdefault(parts[0], set())
            if len(parts) == 1:
                require(entry["kind"] == "directory" and entry["uid"] == 10003 and
                        entry["gid"] == 10001 and entry["mode"] == 0o750,
                        "DAILY_BACKUP_DATABASE_LEAF")
            else:
                member(path)
                result[parts[0]].add(parts[1])
        require(all(children == set(names) and base + "/" + key in records
                    for key, children in result.items()), "DAILY_BACKUP_DATABASE_LEAF")
        return result

    member("config/policy.json", control["policy_sha256"], uid=0)
    admitted = leaves("daily/admitted", ("admission.json", "manifest.json",
                                          "bars.ndjson", "normalization.json"))
    frontiers = leaves("daily/frontiers", ("frontier.json", "observation.json"))
    rows, frontier_keys = database["admissions"], database["verified_frontiers"]
    require(isinstance(rows, list) and len(rows) <= 20000 and isinstance(frontier_keys, list)
            and len(frontier_keys) <= 100000, "DAILY_BACKUP_DATABASE_LIMIT")
    dates, keys = set(), set()
    required_frontiers = {digest(key) for key in frontier_keys}
    require(len(required_frontiers) == len(frontier_keys), "DAILY_BACKUP_DATABASE_SHAPE")
    for row in rows:
        require(isinstance(row, dict) and set(row) == {
            "admission_sha256", "content_sha256", "manifest_sha256",
            "parent_admission_sha256", "range_end", "source_frontier_sha256",
            "bars", "normalization"}, "DAILY_BACKUP_DATABASE_SHAPE")
        key = digest(row["admission_sha256"])
        digest(row["content_sha256"])
        date = row["range_end"]
        require(isinstance(date, str) and re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", date),
                "DAILY_BACKUP_DATABASE_SHAPE")
        try:
            datetime.date.fromisoformat(date)
        except ValueError:
            raise SnapshotError("DAILY_BACKUP_DATABASE_SHAPE") from None
        require(key not in keys and date not in dates, "DAILY_BACKUP_DATABASE_CONFLICT")
        keys.add(key)
        dates.add(date)
        base = "daily/admitted/" + key[7:]
        require(key[7:] in admitted, "DAILY_BACKUP_DATABASE_MISSING")
        member(base + "/admission.json", key)
        member(base + "/manifest.json", row["manifest_sha256"])
        for field, filename in (("bars", "bars.ndjson"), ("normalization", "normalization.json")):
            descriptor = row[field]
            require(isinstance(descriptor, dict) and set(descriptor) ==
                    {"file", "sha256", "size_bytes", "row_count"} and descriptor["file"] == filename
                    and type(descriptor["row_count"]) is int and descriptor["row_count"] > 0,
                    "DAILY_BACKUP_DATABASE_SHAPE")
            member(base + "/" + filename, descriptor["sha256"], descriptor["size_bytes"])
        required_frontiers.add(digest(row["source_frontier_sha256"]))
    dates_by_key = {row["admission_sha256"]: row["range_end"] for row in rows}
    for row in rows:
        parent = row["parent_admission_sha256"]
        require(parent is None or (digest(parent) in keys and
                dates_by_key[parent] < row["range_end"]), "DAILY_BACKUP_DATABASE_PARENT")
    for key in required_frontiers:
        require(key[7:] in frontiers, "DAILY_BACKUP_DATABASE_MISSING")
        # The frontier identity hashes observation.json, not frontier.json.
        member("daily/frontiers/" + key[7:] + "/observation.json", key)
    return {"extra_admissions": sorted("sha256:" + key for key in admitted if "sha256:" + key not in keys),
            "extra_frontiers": sorted("sha256:" + key for key in frontiers
                                      if "sha256:" + key not in required_frontiers)}
