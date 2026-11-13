#!/usr/bin/env python3
"""Private, capability-bound source selection for Paper combined backups.

This module never opens a database, changes Paper state, or creates a release.
Its caller must retain the actual installed Paper context, OFF DrainedHandle,
and exported audit owner for the whole selected Snapshot lifetime. Filesystem
references are derived here from those capabilities and the existing source
validators; no caller roots or serialized verification flags are accepted.
"""
from __future__ import annotations

import hashlib
import itertools
import importlib.util
import os
import re
import stat
import sys
import uuid
from pathlib import Path


class PaperSelectionError(Exception):
    """Only fixed error codes cross this private boundary."""


_PAPER_CLASSES = (
    "base", "config", "curated", "daily", "floor", "paper_config",
    "paper_contract", "paper_evidence", "paper_floor", "paper_manifest",
    "paper_operations", "paper_reader", "raw", "simulation_approval",
    "simulation_config", "simulation_floor", "simulation_input",
    "simulation_manifest", "simulation_reader", "simulation_receipt",
)
_PAPER_PROJECTION_CLASSES = (
    "paper_config", "paper_contract", "paper_evidence", "paper_manifest",
    "paper_reader",
)


def _require(value, code="PAPER_SELECTION_INVALID"):
    if not value:
        raise PaperSelectionError(code)


def _file_module(limits, backup):
    module = sys.modules.get(type(limits).__module__)
    expected = Path(__file__).with_name("owner-beta-daily-backup-files.py").resolve()
    _require(module is not None and getattr(module, "Limits", None) is type(limits)
             and Path(getattr(module, "__file__", "")).resolve() == expected
             and backup.files is module, "PAPER_SELECTION_CODEC")
    _require(tuple(module.PAPER_CLASSES) == _PAPER_CLASSES,
             "PAPER_SELECTION_CLASSES")
    return module


def _backup_module():
    path = Path(__file__).with_name("owner-beta-daily-backup.py").resolve()
    found = [module for module in tuple(sys.modules.values())
             if getattr(module, "__file__", None) and
             Path(module.__file__).resolve() == path and
             callable(getattr(module, "roots", None)) and
             callable(getattr(module, "simulation_roots", None)) and
             hasattr(module, "files")]
    # The stop observer may have loaded this same immutable source under its
    # private name while the existing backup CLI is __main__. Use that exact
    # source-owned module and its codec consistently, never a caller's codec.
    retained = sys.modules.get("paper_stop_backup")
    if retained in found:
        return retained
    _require(len(found) <= 1, "PAPER_SELECTION_BACKUP_MODULE")
    if found:
        return found[0]
    spec = importlib.util.spec_from_file_location("paper_selection_daily_backup", path)
    _require(spec is not None and spec.loader is not None,
             "PAPER_SELECTION_BACKUP_MODULE")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _trusted_context(context, drained, held_audit):
    operations = sys.modules.get(type(context).__module__)
    _require(operations is not None and getattr(operations, "_PaperContext", None)
             is type(context) and getattr(operations, "DrainedHandle", None)
             is type(drained), "PAPER_SELECTION_CAPABILITY")
    _require(getattr(context, "_mode", None) == "live"
             and getattr(context, "_token", None) is getattr(operations, "_CONTEXT_TOKEN", None)
             and type(context.verified) is operations.release.VerifiedPaperRelease,
             "PAPER_SELECTION_CAPABILITY")
    attempt = drained._attempt
    _require(attempt.context is context and not attempt.closed and
             type(attempt.operation) is operations.journal.Operation and
             attempt.operation._journal._lock_id == context.lock_identity,
             "PAPER_SELECTION_SCOPE")
    root = attempt.operation._intent
    _require(root["purpose"] == "BACKUP" and root["scope"] == "COMBINED_BACKUP"
             and root["account_id"] is not None
             and root["generation"] == attempt.operation.generation
             and context.verified.process_sha256 is not None,
             "PAPER_SELECTION_SCOPE")
    _require(type(attempt.consumer) is operations._OffCommit
             and attempt.operation._last is not None
             and attempt.operation._last[2] == "OFF_COMMITTED"
             and attempt.operation._last[3]["generation"] == root["generation"],
             "PAPER_SELECTION_OFF_REQUIRED")
    # Entry remains bound to the original drain, but temporal authority now
    # belongs to C's actual once-issued archive resource (never an expired handle).
    _trusted_audit_owner(context, drained, attempt.operation, held_audit)
    attempt.operation.revalidate()
    attempt.originals.revalidate()
    context.verified.revalidate()
    return operations, attempt, attempt.operation


def _trusted_audit_owner(context, drained, operation, owner):
    """Require C's real held snapshot owner; there is no JSON-token fallback.

    C must load its new controller as ``owner_beta_paper_backup`` and define
    ``_HeldPaperSnapshot`` with private ``_token``, ``context``, ``drained``,
    ``operation``, ``generation``, ``revalidate()``, and
    ``_file_selection_requirements()`` members. The final method returns an
    immutable tuple of ``(class, sha256, size)`` identities decoded from the
    same retained completed audit stream. This accessor guides file selection;
    it never grants file or lifecycle authority.
    """
    module = sys.modules.get("owner_beta_paper_backup")
    expected = getattr(module, "_HeldPaperSnapshot", None) if module else None
    token = getattr(module, "_HELD_PAPER_SNAPSHOT_TOKEN", None) if module else None
    _require(expected is not None and token is not None and type(owner) is expected
             and getattr(owner, "_token", None) is token
             and owner.context is context and owner.drained is drained
             and owner.operation is operation
             and owner.generation == operation.generation,
             "PAPER_SELECTION_AUDIT_OWNER")
    owner.revalidate()
    requirements = owner._file_selection_requirements()
    _require(type(requirements) is tuple, "PAPER_SELECTION_AUDIT_REFERENCES")
    normalized, seen = [], set()
    for item in requirements:
        _require(type(item) is tuple and len(item) == 3, "PAPER_SELECTION_AUDIT_REFERENCES")
        name, digest, size = item
        _require(type(name) is str and type(digest) is str
                 and re.fullmatch(r"sha256:[0-9a-f]{64}", digest)
                 and type(size) is int and 0 <= size <= 2**50
                 and name in _paper_classes(), "PAPER_SELECTION_AUDIT_REFERENCES")
        key = (name, digest, size)
        _require(key not in seen, "PAPER_SELECTION_AUDIT_REFERENCES")
        seen.add(key)
        normalized.append(key)
    return owner, tuple(normalized)


def _paper_classes():
    return _PAPER_CLASSES


def _current_roots(paper_context, operations, backup, files):
    context = paper_context.context
    config, config_digest = operations.daily.install.verify_config(
        paper_context.release_path, paper_context.commit)
    values = paper_context.values
    required = ("LAGRANGE_DATA_DIR", "LAGRANGE_ARTIFACTS_DIR",
                "LAGRANGE_RUNTIME_SECRET_DIR", "POSTGRES_DB", "MAX_JOBS_PER_OWNER",
                "PRICE_BETA_LEASE_SECS", "PRICE_BETA_BACKOFF_SECS")
    _require(type(values) is dict and all(name in values for name in required),
             "PAPER_SELECTION_DAILY_BINDING")
    binding = operations.daily.runtime.Binding(
        paper_context.project, paper_context.commit,
        paper_context.images["owner-beta-runner"], config_digest,
        paper_context.release_path, values["LAGRANGE_DATA_DIR"],
        values["LAGRANGE_ARTIFACTS_DIR"], values["LAGRANGE_RUNTIME_SECRET_DIR"],
        values["POSTGRES_DB"] or "lagrange", values["MAX_JOBS_PER_OWNER"],
        values["PRICE_BETA_LEASE_SECS"], values["PRICE_BETA_BACKOFF_SECS"])
    daily_roots = tuple(backup.roots(binding, paper_context.install_root))
    simulation_binding = backup.simulation.binding_from_release(
        paper_context.project, paper_context.release_path, paper_context.commit)
    simulation_roots = tuple(backup.simulation_roots(simulation_binding, paper_context.install_root))
    by_name = {root.name: root for root in (*daily_roots, *simulation_roots)}
    required_names = set(files.CLASSES) | set(files.SIMULATION_EVIDENCE_CLASSES)
    _require(set(by_name) == required_names and len(by_name) == len(daily_roots) +
             len(simulation_roots), "PAPER_SELECTION_DAILY_CLASSES")
    return by_name


class _OriginalFile:
    """One retained original FD plus every named ancestor used to reach it."""
    __slots__ = ("path", "parents", "parent_names", "fd", "name", "identity",
                 "size", "sha256", "uid", "gid", "mode", "maximum", "copies")

    def __init__(self, files, path, *, uid, gid, mode=None, maximum):
        self.path, self.parents, self.parent_names = path, [], []
        self.fd = None
        self.copies = []
        self.maximum = maximum
        _require(type(path) is str and path.startswith("/") and
                 not any(part in ("", ".", "..") for part in path[1:].split("/")),
                 "PAPER_SELECTION_SOURCE")
        parts = path[1:].split("/")
        current = os.open("/", files.DIRECTORY)
        self.parents.append(current)
        try:
            files.checked_parent(current)
            for name in parts[:-1]:
                child = os.open(name, files.DIRECTORY, dir_fd=current)
                self.parents.append(child)
                before = files.checked_parent(child)
                named = os.stat(name, dir_fd=current, follow_symlinks=False)
                _require(files.identity(before) == files.identity(named),
                         "PAPER_SELECTION_SOURCE_CHANGED")
                self.parent_names.append((current, child, name, files.identity(before)))
                current = child
            self.name = parts[-1]
            self.fd = os.open(self.name, files.REGULAR, dir_fd=current)
            info = files.checked(self.fd, directory=False, owners={uid}, groups={gid})
            self.uid, self.gid, self.mode = info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode)
            _require(info.st_size <= maximum and (mode is None or self.mode == mode),
                     "PAPER_SELECTION_SOURCE_METADATA")
            self.identity, self.size = files.identity(info), info.st_size
            self.sha256 = self._hash(files, info)
            self._check_name(files)
        except OSError:
            self.close()
            raise PaperSelectionError("PAPER_SELECTION_SOURCE_IO") from None
        except BaseException:
            self.close()
            raise

    def _hash(self, files, before):
        digest, offset = hashlib.sha256(), 0
        while offset < before.st_size:
            part = os.pread(self.fd, min(files.CHUNK, before.st_size - offset), offset)
            _require(part, "PAPER_SELECTION_SOURCE_CHANGED")
            digest.update(part)
            offset += len(part)
        _require(not os.pread(self.fd, 1, offset), "PAPER_SELECTION_SOURCE_CHANGED")
        after = files.checked(self.fd, directory=False, owners={self.uid}, groups={self.gid})
        _require(files.identity(before) == files.identity(after),
                 "PAPER_SELECTION_SOURCE_CHANGED")
        return "sha256:" + digest.hexdigest()

    def _check_name(self, files):
        named = os.stat(self.name, dir_fd=self.parents[-1], follow_symlinks=False)
        _require(files.identity(named) == self.identity, "PAPER_SELECTION_SOURCE_REPLACED")

    def revalidate(self, files):
        try:
            for parent, child, name, identity in self.parent_names:
                current = os.stat(name, dir_fd=parent, follow_symlinks=False)
                _require(files.identity(os.fstat(child)) == identity
                         and files.identity(current) == identity,
                         "PAPER_SELECTION_SOURCE_REPLACED")
            info = files.checked(self.fd, directory=False, owners={self.uid}, groups={self.gid})
            _require(files.identity(info) == self.identity and
                     files.identity(os.stat(self.name, dir_fd=self.parents[-1],
                                            follow_symlinks=False)) == self.identity and
                     self._hash(files, info) == self.sha256,
                     "PAPER_SELECTION_SOURCE_CHANGED")
            self._check_copies(files)
        except OSError:
            raise PaperSelectionError("PAPER_SELECTION_SOURCE_REPLACED") from None

    def copy_to(self, files, class_fd, relative):
        self.revalidate(files)
        parts = files.safe_parts(relative)
        current, opened, fd = class_fd, [], None
        try:
            for part in parts[:-1]:
                try:
                    os.mkdir(part, 0o700, dir_fd=current)
                    os.fsync(current)
                except FileExistsError:
                    pass
                child = os.open(part, files.DIRECTORY, dir_fd=current)
                opened.append(child)  # custody before validation can fail
                files.checked(child, directory=True, owners={0}, groups={0})
                current = child
            fd = os.open(parts[-1], os.O_RDWR | os.O_CREAT | os.O_EXCL |
                         os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=current)
            digest, offset = hashlib.sha256(), 0
            while offset < self.size:
                chunk = os.pread(self.fd, min(files.CHUNK, self.size - offset), offset)
                _require(chunk, "PAPER_SELECTION_SOURCE_CHANGED")
                view = memoryview(chunk)
                while view:
                    written = os.write(fd, view)
                    _require(written > 0, "PAPER_SELECTION_WRITE")
                    view = view[written:]
                digest.update(chunk)
                offset += len(chunk)
            _require("sha256:" + digest.hexdigest() == self.sha256,
                     "PAPER_SELECTION_SOURCE_CHANGED")
            created = os.fstat(fd)
            if (created.st_uid, created.st_gid) != (self.uid, self.gid):
                os.fchown(fd, self.uid, self.gid)
            os.fchmod(fd, self.mode)
            info = files.checked(fd, directory=False, owners={self.uid}, groups={self.gid})
            _require(info.st_size == self.size, "PAPER_SELECTION_WRITE")
            os.fsync(fd)
            parent = os.dup(current)
            route = []
            try:
                for node in [class_fd, *opened]:
                    route.append(os.dup(node))
            except BaseException:
                for node in route:
                    os.close(node)
                os.close(parent)
                raise
            self.copies.append({"parent": parent, "name": parts[-1], "fd": fd,
                "identity": files.identity(info), "parts": tuple(parts), "route": route,
                "births": tuple(_construction_identity(os.fstat(node)) for node in route), "sealed": None})
            fd = None
            os.fsync(current)
            self.revalidate(files)
        finally:
            if fd is not None:
                os.close(fd)
            for opened_fd in reversed(opened):
                os.close(opened_fd)

    def _check_copies(self, files):
        for copy in self.copies:
            parent, name, fd, identity = (copy[k] for k in ("parent", "name", "fd", "identity"))
            for index, node in enumerate(copy["route"]):
                current = os.fstat(node)
                _require(_construction_identity(current) == copy["births"][index],
                         "PAPER_SELECTION_COPY_CHANGED")
                if index:
                    named = os.stat(copy["parts"][index-1], dir_fd=copy["route"][index-1], follow_symlinks=False)
                    _require(files.identity(named) == files.identity(current), "PAPER_SELECTION_COPY_CHANGED")
                if copy["sealed"] is not None:
                    _require(files.identity(current) == copy["sealed"][index], "PAPER_SELECTION_COPY_CHANGED")
            info = files.checked(fd, directory=False, owners={self.uid}, groups={self.gid})
            _require(files.identity(info) == identity
                     and files.identity(os.stat(name, dir_fd=parent, follow_symlinks=False)) == identity,
                     "PAPER_SELECTION_COPY_CHANGED")
            offset, digest = 0, hashlib.sha256()
            while offset < self.size:
                part = os.pread(fd, min(files.CHUNK, self.size - offset), offset)
                _require(part, "PAPER_SELECTION_COPY_CHANGED")
                digest.update(part)
                offset += len(part)
            _require(not os.pread(fd, 1, offset)
                     and "sha256:" + digest.hexdigest() == self.sha256
                     and files.identity(os.fstat(fd)) == identity,
                     "PAPER_SELECTION_COPY_CHANGED")

    def seal_copies(self, files):
        self._check_copies(files)
        for copy in self.copies:
            _require(copy["sealed"] is None, "PAPER_SELECTION_STAGE_ALREADY_SEALED")
            copy["sealed"] = tuple(files.identity(os.fstat(fd)) for fd in copy["route"])
        self._check_copies(files)

    def close(self):
        while self.copies:
            copy = self.copies.pop()
            for fd in (copy["fd"], copy["parent"], *copy["route"]):
                try:
                    os.close(fd)
                except OSError:
                    pass
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None
        for fd in reversed(self.parents):
            try:
                os.close(fd)
            except OSError:
                pass
        self.parents.clear()


class _OriginalDirectory:
    """Actual locked directory inventory, including an explicit empty listing."""
    def __init__(self, files, path, *, maximum, membership=True):
        self.tree = None
        self.path = path
        self.maximum, self.membership = maximum, membership
        # Reuse descriptor-relative protected ancestry; no untrusted root list.
        self.parents, self.names = [], []
        try:
            current = os.open("/", files.DIRECTORY)
            self.parents.append(current)
            files.checked_parent(current)
            for part in files.safe_parts(path[1:]):
                fd = os.open(part, files.DIRECTORY, dir_fd=current)
                self.parents.append(fd)
                info = files.checked(fd, directory=True, owners={0}, groups={0, 10001})
                named = os.stat(part, dir_fd=current, follow_symlinks=False)
                _require(files.identity(info) == files.identity(named), "PAPER_SELECTION_SOURCE_REPLACED")
                self.names.append((current, part, fd, files.identity(info)))
                current = fd
            self.fd = current
            self.members = self._listing()
            _require(self.members is None or len(self.members) <= maximum, "PAPER_SELECTION_EVIDENCE_SIZE")
            self.revalidate(files)
        except BaseException:
            self.close()
            raise

    def _listing(self):
        if not self.membership:
            return None
        with os.scandir(self.fd) as entries:
            names = [entry.name for entry in itertools.islice(entries, self.maximum + 1)]
        _require(len(names) <= self.maximum, "PAPER_SELECTION_EVIDENCE_SIZE")
        return tuple(sorted(names))

    def revalidate(self, files):
        for parent, name, fd, identity in self.names:
            files.checked(fd, directory=True, owners={0}, groups={0, 10001})
            _require(files.identity(os.fstat(fd)) == identity
                     and files.identity(os.stat(name, dir_fd=parent, follow_symlinks=False)) == identity,
                     "PAPER_SELECTION_SOURCE_REPLACED")
        _require(self._listing() == self.members, "PAPER_SELECTION_EVIDENCE_CHANGED")

    def close(self):
        while self.parents:
            os.close(self.parents.pop())


def _source_events(files, source, binding, class_fd, prefix, sources, maximum):
    directory = _OriginalDirectory(files, binding["evidence_root"] + "/events", maximum=maximum)
    sources.append(directory)
    count = 0
    for slot in directory.members:
        _require(re.fullmatch(r"[0-9a-f]{64}", slot), "PAPER_SELECTION_EVIDENCE")
        chain = _OriginalDirectory(files, directory.path + "/" + slot, maximum=64)
        sources.append(chain)
        _require(chain.members == tuple("%06d.json" % i for i in range(len(chain.members)))
                 and chain.members, "PAPER_SELECTION_EVIDENCE")
        events = []
        for leaf in chain.members:
            count += 1
            _require(count <= maximum, "PAPER_SELECTION_EVIDENCE_SIZE")
            original = _OriginalFile(files, chain.path + "/" + leaf, uid=0, gid=10001,
                                     mode=0o440, maximum=source.SOURCE_LIMIT)
            sources.append(original)
            event = source.decode(os.pread(original.fd, original.size+1, 0))
            _require(event["slot_id"] == "sha256:" + slot, "PAPER_SELECTION_EVIDENCE")
            events.append(event)
            original.copy_to(files, class_fd, prefix + "/events/" + slot + "/" + leaf)
        source.verify_chain(events)  # partial/FAILED history remains evidence
        chain.revalidate(files)
    directory.revalidate(files)


def _lifecycle_shape(parts, directory):
    """Fixed descriptive namespace. Content semantics belong to native finish."""
    hashed = lambda n: re.fullmatch(r"[0-9a-f]{64}\.json", n) is not None
    canonical_id = lambda n: re.fullmatch(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", n) is not None
    if directory:
        return (not parts or (len(parts) == 1 and parts[0] in ("queries", "selections", "anchors"))
                or (len(parts) in (2, 3) and parts[0] == "queries" and canonical_id(parts[1])
                    and (len(parts) == 2 or parts[2] == "output")))
    if len(parts) == 2 and parts[0] in ("selections", "anchors") and hashed(parts[1]):
        return 65_536
    if len(parts) == 3 and parts[0] == "queries" and canonical_id(parts[1]):
        return {"request.json": 65_536, "evidence.json": 65_536,
                "action-binding.json": 65_536, "sql-request.json": 1_048_576,
                "native-result.json": 65_537, "artifact.json": 1_048_576}.get(parts[2], 0)
    if len(parts) == 4 and parts[0] == "queries" and canonical_id(parts[1]) and parts[2] == "output" and hashed(parts[3]):
        return 1_048_576
    return 0


def _lifecycle_originals(files, install_root, class_fd, sources, limits, check_time):
    """Copy locked sibling originals, retaining custody until selection closes.

    Root identity and bounded membership include staging's name, but ephemeral
    staging contents are never projected or interpreted as durable originals.
    """
    root_path = install_root + "/.owner-beta-paper-lifecycle"
    # Legacy installations predate this sibling. Retain the install directory
    # generation even for absence, so late creation cannot be silently missed.
    install = _OriginalDirectory(files, install_root, maximum=limits.entries)
    sources.append(install)
    if ".owner-beta-paper-lifecycle" not in install.members:
        return
    used_entries, used_bytes = 0, 0

    def directory(path, parts, *, project=True, membership=True):
        nonlocal used_entries
        check_time()
        used_entries += 1
        _require(used_entries <= limits.entries, "PAPER_SELECTION_EVIDENCE_SIZE")
        item = _OriginalDirectory(files, path, maximum=min(4096, limits.entries), membership=membership)
        sources.append(item)
        info = os.fstat(item.fd)
        _require((info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode)) == (0, 0, 0o700),
                 "PAPER_SELECTION_SOURCE_METADATA")
        if project:
            _require(_lifecycle_shape(parts, True), "PAPER_SELECTION_LIFECYCLE_NAME")
            current, opened = class_fd, []
            try:
                for part in ("lifecycle", *parts):
                    try:
                        os.mkdir(part, 0o700, dir_fd=current)
                        os.fsync(current)
                    except FileExistsError:
                        pass
                    child = os.open(part, files.DIRECTORY, dir_fd=current)
                    opened.append(child)
                    info = files.checked(child, directory=True, owners={0}, groups={0})
                    _require(stat.S_IMODE(info.st_mode) == 0o700, "PAPER_SELECTION_STAGE")
                    current = child
            finally:
                for fd in reversed(opened):
                    os.close(fd)
        return item

    root = directory(root_path, ())
    _require(root.members == ("anchors", "queries", "selections", "staging"), "PAPER_SELECTION_LIFECYCLE_NAME")
    directory(root_path + "/staging", ("staging",), project=False, membership=False)

    def walk(item, parts):
        nonlocal used_entries, used_bytes
        for name in item.members:
            check_time()
            child_parts = (*parts, name)
            info = os.stat(name, dir_fd=item.fd, follow_symlinks=False)
            if stat.S_ISDIR(info.st_mode):
                child = directory(item.path + "/" + name, child_parts)
                walk(child, child_parts)
            else:
                maximum = _lifecycle_shape(child_parts, False)
                _require(maximum and stat.S_ISREG(info.st_mode), "PAPER_SELECTION_LIFECYCLE_NAME")
                used_entries += 1
                used_bytes += info.st_size
                _require(used_entries <= limits.entries and used_bytes <= limits.total_bytes,
                         "PAPER_SELECTION_EVIDENCE_SIZE")
                original = _OriginalFile(files, item.path + "/" + name, uid=0, gid=0,
                                         mode=0o400, maximum=maximum)
                sources.append(original)
                if re.fullmatch(r"[0-9a-f]{64}\.json", name):
                    _require(name == original.sha256[7:] + ".json", "PAPER_SELECTION_LIFECYCLE_HASH")
                original.copy_to(files, class_fd, "/".join(("lifecycle", *child_parts)))
        item.revalidate(files)
    for name in ("queries", "selections", "anchors"):
        item = directory(root_path + "/" + name, (name,))
        walk(item, (name,))
    for item in sources:
        check_time()
        item.revalidate(files)


class _ExternalCurrent:
    """Retain a source-owned Current FD without copying its lock authority."""
    __slots__ = ("source", "path", "manager", "fd", "data", "identity")

    def __init__(self, files, source, binding, raw, path):
        _require(source.binding(path) == (binding, raw), "PAPER_SELECTION_EVIDENCE")
        current_path = str(Path(binding["evidence_root"]) / "current" / source.CURRENT_LEAF)
        self.source, self.path = source, current_path
        self.data = source.read_file(current_path, 65_536, exact=(0, 10001, 0o440))
        current = source.decode(self.data, 65_536)
        source.validate_current(current)
        _require(current.get("status") == "INVALID", "PAPER_SELECTION_EVIDENCE")
        self.manager, self.fd = source.opened(current_path, exact=(0, 10001, 0o440)), None
        try:
            self.fd = self.manager.__enter__()
            info = files.checked(self.fd, directory=False, owners={0}, groups={10001})
            _require(stat.S_IMODE(info.st_mode) == 0o440 and info.st_size == len(self.data),
                     "PAPER_SELECTION_EVIDENCE")
            self.identity = files.identity(info)
            _require(os.pread(self.fd, len(self.data) + 1, 0) == self.data,
                     "PAPER_SELECTION_EVIDENCE")
        except BaseException:
            self.manager.__exit__(*sys.exc_info())
            self.manager = self.fd = None
            raise

    def revalidate(self, files):
        data = self.source.read_file(self.path, 65_536, exact=(0, 10001, 0o440))
        with self.source.opened(self.path, exact=(0, 10001, 0o440)) as reopened:
            named_identity = files.identity(os.fstat(reopened))
        info = files.checked(self.fd, directory=False, owners={0}, groups={10001})
        _require(data == self.data and named_identity == self.identity
                 and files.identity(info) == self.identity
                 and os.pread(self.fd, len(self.data) + 1, 0) == self.data,
                 "PAPER_SELECTION_EVIDENCE_CHANGED")

    def close(self):
        if self.manager is not None:
            self.manager.__exit__(None, None, None)
            self.manager = None
            self.fd = None


def _construction_identity(info):
    # Creation can change nlink/size/timestamps. Retain the opened generation;
    # final full metadata is sealed once, after our exclusively owned writes.
    return (info.st_dev, info.st_ino, info.st_mode, info.st_uid, info.st_gid)


def _mkdir_private_tree(files):
    fds, names, births = [], [], []
    tree = None
    try:
        current = os.open("/", files.DIRECTORY)
        fds.append(current)
        files.checked_parent(current)
        for part in ("var", "lib"):
            fd = os.open(part, files.DIRECTORY, dir_fd=current)
            fds.append(fd)
            info = files.checked_parent(fd)
            _require(files.identity(os.stat(part, dir_fd=current, follow_symlinks=False)) ==
                     files.identity(info), "PAPER_SELECTION_STAGE_REPLACED")
            births.append(_construction_identity(info))
            names.append((current, part))
            current = fd
        leaf = ".lagrange-paper-selection-" + uuid.uuid4().hex
        os.mkdir(leaf, 0o700, dir_fd=current)
        os.fsync(current)
        fd = os.open(leaf, files.DIRECTORY, dir_fd=current)
        fds.append(fd)
        info = files.checked(fd, directory=True, owners={0}, groups={0})
        _require(stat.S_IMODE(info.st_mode) == 0o700, "PAPER_SELECTION_STAGE")
        births.append(_construction_identity(info))
        names.append((current, leaf))
        tree = {"fds": fds, "names": names, "births": births, "identities": None,
                "fd": fd, "path": "/var/lib/" + leaf, "name": leaf,
                "parent": current, "class_fds": {}, "classes": None, "removed": False}
        _check_private_tree(files, tree)
        return tree
    except BaseException:
        for fd in reversed(fds):
            os.close(fd)
        # A partially created path is not refreshed/reopened for deletion.
        raise


def _check_private_tree(files, tree):
    for index, (parent, name) in enumerate(tree["names"]):
        fd = tree["fds"][index + 1]
        current = os.stat(name, dir_fd=parent, follow_symlinks=False)
        actual = os.fstat(fd)
        _require(_construction_identity(actual) == tree["births"][index]
                 and files.identity(current) == files.identity(actual),
                 "PAPER_SELECTION_STAGE_REPLACED")
        if tree["identities"] is not None:
            _require(files.identity(actual) == tree["identities"][index],
                     "PAPER_SELECTION_STAGE_REPLACED")
    for name, fd in tree["class_fds"].items():
        current = os.stat(name, dir_fd=tree["fd"], follow_symlinks=False)
        _require(files.identity(os.fstat(fd)) == files.identity(current),
                 "PAPER_SELECTION_STAGE_REPLACED")
        if tree["classes"] is not None:
            _require(files.identity(current) == tree["classes"][name],
                     "PAPER_SELECTION_STAGE_REPLACED")


def _seal_private_tree(files, tree):
    _require(tree["identities"] is None and tree["classes"] is None,
             "PAPER_SELECTION_STAGE_ALREADY_SEALED")
    _check_private_tree(files, tree)
    tree["identities"] = tuple(files.identity(os.fstat(fd)) for fd in tree["fds"][1:])
    tree["classes"] = {name: files.identity(os.fstat(fd)) for name, fd in tree["class_fds"].items()}
    _check_private_tree(files, tree)


def _remove_tree(fd):
    with os.scandir(fd) as entries:
        names = sorted(entry.name for entry in entries)
    for name in names:
        info = os.stat(name, dir_fd=fd, follow_symlinks=False)
        if stat.S_ISDIR(info.st_mode):
            child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                            dir_fd=fd)
            try:
                _remove_tree(child)
            finally:
                os.close(child)
            os.rmdir(name, dir_fd=fd)
        else:
            _require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1,
                     "PAPER_SELECTION_STAGE_CLEANUP")
            os.unlink(name, dir_fd=fd)
    os.fsync(fd)


def _dispose_private_tree(files, tree):
    """Remove only the retained named generation; close all FDs on every path."""
    failure = None
    try:
        # Authority has ended. Cleanup binds retained names/inodes/ownership;
        # sibling resource disposal may have changed ancestor timestamps/link
        # counts, which remain strict throughout selection and consumption.
        for index, (parent, name) in enumerate(tree["names"]):
            fd = tree["fds"][index+1]
            actual = os.fstat(fd)
            named = os.stat(name, dir_fd=parent, follow_symlinks=False)
            _require(_construction_identity(actual) == tree["births"][index]
                     and files.identity(named) == files.identity(actual), "PAPER_SELECTION_STAGE_REPLACED")
        _remove_tree(tree["fd"])
        named = os.stat(tree["name"], dir_fd=tree["parent"], follow_symlinks=False)
        _require(_construction_identity(named) == tree["births"][-1],
                 "PAPER_SELECTION_STAGE_REPLACED")
        os.rmdir(tree["name"], dir_fd=tree["parent"])
        os.fsync(tree["parent"])
        tree["removed"] = True
    except BaseException as error:
        failure = error
    finally:
        for fd in list(tree["class_fds"].values()) + list(reversed(tree["fds"])):
            try:
                os.close(fd)
            except OSError:
                pass
    if failure is not None:
        raise PaperSelectionError("PAPER_SELECTION_STAGE_CLEANUP") from None


class _SelectedPaperFiles:
    __slots__ = ("_token", "context", "drained", "held_audit", "operation",
                 "snapshot", "_files", "_sources", "_currents", "_tree",
                 "_entered", "_closed", "_requirements")

    def __init__(self, token, context, drained, held_audit, operation, snapshot,
                 files, sources, currents, tree, requirements):
        _require(token is _SELECTED_TOKEN, "PAPER_SELECTION_PRIVATE")
        self._token, self.context, self.drained = token, context, drained
        self.held_audit, self.operation, self.snapshot = held_audit, operation, snapshot
        self._files, self._sources, self._currents = files, sources, currents
        self._tree, self._requirements = tree, requirements
        self._entered, self._closed = False, False

    def __reduce__(self):
        raise PaperSelectionError("PAPER_SELECTION_PRIVATE")

    def __copy__(self):
        raise PaperSelectionError("PAPER_SELECTION_PRIVATE")

    def __deepcopy__(self, _memo):
        raise PaperSelectionError("PAPER_SELECTION_PRIVATE")

    def __enter__(self):
        files = sys.modules.get(self.snapshot.__class__.__module__)
        _require(not self._closed and not self._entered and files is not None
                 and type(self.snapshot) is getattr(files, "Snapshot", None),
                 "PAPER_SELECTION_SNAPSHOT")
        try:
            self.revalidate()
            self.snapshot.__enter__()
            self._entered = True
            self._check_requirements()
            return self
        except BaseException:
            primary = sys.exc_info()
            try:
                if self._entered:
                    try:
                        self.snapshot.__exit__(*primary)
                    finally:
                        self._entered = False
            except BaseException:
                pass  # preserve the failed entry's primary error
            try:
                self.close()
            except BaseException:
                pass
            raise

    def _check_requirements(self):
        manifest = self.snapshot.manifest()
        files = self.snapshot.__class__.__module__
        _require(isinstance(manifest, bytes), "PAPER_SELECTION_SNAPSHOT")
        value = sys.modules[files].validate_manifest(
            manifest, self.snapshot.limits, profile="paper-combined")
        present = {(entry["path"].split("/", 1)[0], entry.get("sha256"), entry["size"])
                   for entry in value["entries"] if entry["kind"] == "file"}
        for requirement in self._requirements:
            _require(requirement in present, "PAPER_SELECTION_HISTORICAL_ORIGINAL_MISSING")

    def revalidate(self):
        _require(not self._closed, "PAPER_SELECTION_CLOSED")
        self.held_audit.revalidate()
        self.context.revalidate()
        self.operation.revalidate()
        self.operation._journal.revalidate()
        self.held_audit.revalidate()
        _require(self.held_audit._file_selection_requirements() == self._requirements,
                 "PAPER_SELECTION_AUDIT_CHANGED")
        files = sys.modules[self.snapshot.__class__.__module__]
        for source in self._sources:
            source.revalidate(files)
        for current in self._currents:
            current.revalidate(files)
        _check_private_tree(files, self._tree)
        if self._entered:
            self.snapshot.verify_unchanged()
            self._check_requirements()

    def __exit__(self, *error):
        primary = error if error[0] is not None else None
        try:
            if primary is None:
                self.revalidate()
        except BaseException:
            primary = sys.exc_info()
        try:
            if self._entered:
                try:
                    self.snapshot.__exit__(*(primary or (None, None, None)))
                finally:
                    self._entered = False
        except BaseException:
            if primary is None:
                primary = sys.exc_info()
        try:
            self.close()
        except BaseException:
            if primary is None:
                primary = sys.exc_info()
        if error[0] is None and primary is not None:
            raise primary[1].with_traceback(primary[2])
        return False

    def close(self):
        if self._closed:
            return
        _require(not self._entered, "PAPER_SELECTION_STILL_ENTERED")
        files = sys.modules[self.snapshot.__class__.__module__]
        failure = None
        for source in reversed(self._sources):
            try:
                source.close()
            except BaseException:
                failure = failure or "PAPER_SELECTION_SOURCE_CLOSE"
        for current in reversed(self._currents):
            try:
                current.close()
            except BaseException:
                failure = failure or "PAPER_SELECTION_EVIDENCE_CLOSE"
        try:
            self.held_audit._release_selection_tree(self._tree)
        except BaseException:
            failure = failure or "PAPER_SELECTION_STAGE_CLEANUP"
        self._closed = True
        if failure:
            raise PaperSelectionError(failure)


_SELECTED_TOKEN = object()


def _open_selected_paper_files(context, drained, held_audit, limits):
    """Return a private owner exposing an entered exact ``files.Snapshot``.

    The lock, generation, operation and exported audit are borrowed and remain
    owned by their callers. This owner closes only its own source/staging FDs.
    The 90-second native stop deadline is never reset or extended.
    """
    _require(sys.platform == "linux" and os.geteuid() == 0,
             "PAPER_SELECTION_ROOT_REQUIRED")
    operations, attempt, operation = _trusted_context(context, drained, held_audit)
    held_audit, requirements = _trusted_audit_owner(
        context, drained, operation, held_audit)
    backup = _backup_module()
    files = _file_module(limits, backup)
    roots = _current_roots(context, operations, backup, files)
    tree = held_audit._take_selection_tree()
    sources, currents, class_fds = [], [], {}
    try:
        for name in _PAPER_PROJECTION_CLASSES:
            os.mkdir(name, 0o700, dir_fd=tree["fd"])
            os.fsync(tree["fd"])
            fd = os.open(name, files.DIRECTORY, dir_fd=tree["fd"])
            info = files.checked(fd, directory=True, owners={0}, groups={0})
            _require(stat.S_IMODE(info.st_mode) == 0o700, "PAPER_SELECTION_STAGE")
            class_fds[name] = fd
        tree["class_fds"] = class_fds

        verified, release = context.verified, operations.release
        release_fd = verified._fds[verified._release_fd_index]
        stage_fd = verified._fds[-1]
        release_path = verified.target_release
        stage_path = release_path + "/deploy/owner-beta-paper"
        daily = operations.daily.install

        def original(path, uid, gid, mode, maximum):
            source = _OriginalFile(files, path, uid=uid, gid=gid,
                                   mode=mode, maximum=maximum)
            sources.append(source)
            return source

        # The process binding/config alias are validated by VerifiedPaperRelease;
        # no environment file, deployment tree or secret directory is copied.
        for filename in ("process.json", "config.json"):
            data = daily.read_file(stage_fd, filename, uid=0, gid=10001,
                                  mode=0o440, maximum=release.MAX_PROCESS)
            _require(data, "PAPER_SELECTION_PROCESS_REQUIRED")
            src = original(stage_path + "/" + filename, 0, 10001, 0o440,
                           release.MAX_PROCESS)
            _require(src.sha256 == "sha256:" + hashlib.sha256(data).hexdigest(),
                     "PAPER_SELECTION_PROCESS_CHANGED")
            src.copy_to(files, class_fds["paper_config"], filename)

        # Installed immutable reader capability, ELFs and producer receipts.
        for filename, mode, maximum in (
                ("capability.json", 0o444, release.MAX_PROCESS),
                ("api-image.elf", 0o400, 128 * 1024 * 1024),
                ("worker-image.elf", 0o400, 128 * 1024 * 1024),
                ("native-image.elf", 0o500, 128 * 1024 * 1024)):
            src = original(stage_path + "/" + filename, 0, 0, mode, maximum)
            src.copy_to(files, class_fds["paper_reader"], filename)
        # 0069 is an internal trusted release original. The public capability
        # migration_0068 field retains its base-68 meaning.
        migration = original(stage_path + "/migration-0069.up.sql", 0, 0, 0o444, release.MAX_SOURCE)
        trusted_migration = original(release_path + "/" + release.RESTORE_MIGRATION,
                                     0, 0, None, release.MAX_SOURCE)
        _require(migration.sha256 == trusted_migration.sha256, "PAPER_SELECTION_MIGRATION")
        migration.copy_to(files, class_fds["paper_reader"], "migration-0069.up.sql")
        install_receipt = original(release_path + "/" + release.INSTALL_RECEIPT_NAME,
                                   0, 0, 0o600, 4096)
        install_receipt.copy_to(files, class_fds["paper_reader"],
                                "install-receipt.json")
        producer_roots = tuple(sorted(release._PRODUCERS))
        _require(producer_roots, "PAPER_SELECTION_PRODUCER")
        for producer in producer_roots:
            for filename in ("artifact.json", "COMPLETE"):
                src = original(stage_path + "/provenance/" + producer + "/" + filename,
                               0, 0, 0o444, release.MAX_PRODUCER_RECEIPT)
                src.copy_to(files, class_fds["paper_reader"],
                            "provenance/" + producer + "/" + filename)

        # The installed release manifest is already selected by the legacy
        # simulation_manifest class. Keep Paper's logical class disjoint by
        # projecting the same validated original into its own protected root.
        paper_manifest = original(release_path + "/" + release.MANIFEST_NAME,
                                  0, 0, None, release.MAX_MANIFEST)
        paper_manifest.copy_to(files, class_fds["paper_manifest"],
                               release.MANIFEST_NAME)

        # Frozen contract/schema/source originals, kept separate from the
        # installed reader copies above. Only validator-declared paths enter.
        contract_paths = {
            "financial-contract.md": release.FINANCIAL_CONTRACT,
            "audit-projection.json": release.PROJECTION,
            "operations.schema.json": release.PATHS["operations_schema_sha256"],
            "archive.schema.json": release.PATHS["archive_schema_sha256"],
            "migration-0068.up.sql": release.PATHS["migration_0068_sha256"],
            "audit.sql": release.PATHS["audit_sql_sha256"],
        }
        for destination, relative in contract_paths.items():
            uid, gid = 0, 0
            src = original(release_path + "/" + relative, uid, gid, None,
                           release.MAX_SOURCE)
            src.copy_to(files, class_fds["paper_contract"], destination)

        # Source Current originals are evidence only. Observer-held collector
        # and control locks remain in the borrowed DrainedHandle, outside bundle.
        observer = attempt.observer
        _require(observer is not None and type(observer.source_bindings) is list
                 and observer.source_bindings and type(observer.locks) is list
                 and any(type(item) is tuple and len(item) == 2 and item[0] == "manager"
                         for item in observer.locks),
                 "PAPER_SELECTION_EVIDENCE_REQUIRED")
        for index, entry in enumerate(observer.source_bindings):
            _require(type(entry) is tuple and len(entry) == 4,
                     "PAPER_SELECTION_EVIDENCE")
            source, binding, raw, path = entry
            current = _ExternalCurrent(files, source, binding, raw, path)
            currents.append(current)
            src = original(current.path, 0, 10001, 0o440, 65_536)
            _require(os.pread(src.fd, src.size+1, 0) == current.data, "PAPER_SELECTION_EVIDENCE_CHANGED")
            src.copy_to(files, class_fds["paper_evidence"], "current-%04d.json" % index)
            _source_events(files, source, binding, class_fds["paper_evidence"],
                           "source-%04d" % index, sources, limits.entries)

        _lifecycle_originals(files, context.install_root, class_fds["paper_evidence"],
                             sources, limits, held_audit.revalidate)

        # Direct roots retain the actual installed operation history, floor,
        # release manifest and the existing Daily/Raw/Curated histories.
        journal = operation._journal
        journal.revalidate()
        root_info = files.checked(journal._root, directory=True, owners={0}, groups={0})
        _require((root_info.st_uid, root_info.st_gid, stat.S_IMODE(root_info.st_mode)) ==
                 (0, 0, 0o700), "PAPER_SELECTION_JOURNAL")
        roots["paper_operations"] = files.Root(
            "paper_operations", context.install_root + "/.owner-beta-paper",
            0, 0, 0o700)
        with daily.root_directory(context.install_root) as install_fd:
            _require(release._floor_read(daily, install_fd),
                     "PAPER_SELECTION_FLOOR_REQUIRED")
            floor_info = files.checked_parent(install_fd)
            roots["paper_floor"] = files.Root(
                "paper_floor", context.install_root, floor_info.st_uid,
                floor_info.st_gid, stat.S_IMODE(floor_info.st_mode), release.FLOOR_NAME)
        for name in _PAPER_PROJECTION_CLASSES:
            roots[name] = files.Root(name, tree["path"] + "/" + name, 0, 0, 0o700)

        _require(set(roots) == set(files.PAPER_CLASSES), "PAPER_SELECTION_CLASSES")
        ordered_roots = tuple(roots[name] for name in files.PAPER_CLASSES)
        snapshot = files.Snapshot(ordered_roots, limits, profile="paper-combined")
        owner = _SelectedPaperFiles(_SELECTED_TOKEN, context, drained, held_audit,
                                    operation, snapshot, files, sources, currents,
                                    tree, requirements)
        for source in sources:
            if type(source) is _OriginalFile:
                source.seal_copies(files)
        _seal_private_tree(files, tree)
        return owner
    except BaseException:
        for source in reversed(sources):
            try:
                source.close()
            except BaseException:
                pass
        for current in reversed(currents):
            try:
                current.close()
            except Exception:
                pass
        try:
            held_audit._release_selection_tree(tree)
        except BaseException:
            pass  # reservation retains the private tree and primary error
        raise


def open_selected_paper_files(context, drained, held_audit, limits):
    """Typed public boundary for the private capability-bound selection."""
    try:
        return _open_selected_paper_files(context, drained, held_audit, limits)
    except PaperSelectionError:
        raise
    except OSError:
        raise PaperSelectionError("PAPER_SELECTION_IO") from None
