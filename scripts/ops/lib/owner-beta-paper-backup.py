#!/usr/bin/env python3
"""Owned Paper capture, private clone verification and immutable publication.

Source capture requires a live combined OFF owner. Standalone verification owns
only encrypted input and isolated clones. No snapshot token is persisted, and
no COMPLETE/latest exists until both clone lifecycles and all cleanup succeed.
"""
from __future__ import annotations

import contextlib
import ctypes
import dataclasses
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import platform
import re
import select
import shlex
import signal
import stat
import struct
import subprocess
import sys
import time
import uuid

_HELD_PAPER_SNAPSHOT_TOKEN = object()
_OWNERS = {}
_SPACES = {}
_RUNS = set()
_PACKAGES = set()
_PUBLICATIONS = set()


class PaperBackupError(Exception):
    """Finite source codes only; never include SQL, credentials or output."""


@dataclasses.dataclass(frozen=True)
class PaperHandledResult:
    """Caller routing result; never an archive or restoration authority."""
    status: str
    generation: str | None = None
    archive_sha256: str | None = None


def _require(value, code):
    if not value:
        raise PaperBackupError(code)


# The database image declares a volume at this path. Docker gives every container created from
# the image an anonymous volume there unless another mount covers the path, and reports it among
# the container's mounts. A tiny tmpfs covers the path, as in the Daily backup verifier, so that
# no volume is created or left behind and the reported mounts are exactly the binds asked for.
_IMAGE_VOLUME = "/var/lib/postgresql"
_VOLUME_COVER = "rw,nosuid,nodev,noexec,size=4k,mode=000"
# The verification guest keeps its data directory in a bind below that path and has to reach it.
_GUEST_VOLUME_COVER = "rw,nosuid,nodev,noexec,size=4k,mode=0711"


def _covered_mounts(value, cover, code):
    """Require the exact volume cover; return the other mounts Docker reports."""
    _require((value.get("HostConfig") or {}).get("Tmpfs") == {_IMAGE_VOLUME: cover}, code)
    mounts = value.get("Mounts") or []
    _require(type(mounts) is list, code)
    # Docker versions differ on whether a --tmpfs mount is also listed among the mounts.
    return [mount for mount in mounts
            if not (type(mount) is dict and mount.get("Type") == "tmpfs"
                    and mount.get("Destination") == _IMAGE_VOLUME)]


def _local(name, leaf):
    path = Path(__file__).with_name(leaf)
    module = sys.modules.get(name)
    if module is None:
        spec = importlib.util.spec_from_file_location(name, path)
        _require(spec is not None and spec.loader is not None, "PAPER_BACKUP_SOURCE")
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
    _require(Path(module.__file__).resolve() == path.resolve(), "PAPER_BACKUP_SOURCE")
    return module


def _operations(context):
    module = sys.modules.get(type(context).__module__)
    _require(module is not None and type(context) is getattr(module, "_PaperContext", None)
             and Path(module.__file__).resolve() == Path(__file__).with_name(
                 "owner-beta-paper-operations.py").resolve(), "PAPER_BACKUP_CONTEXT")
    return module


def _pgpass(secret, database):
    _require(type(secret) is bytes and secret and not any(c in secret for c in (b"\0", b"\r", b"\n"))
             and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,62}", database), "PAPER_BACKUP_SECRET")
    escaped = secret.replace(b"\\", b"\\\\").replace(b":", b"\\:")
    return b"postgres:5432:" + database.encode("ascii") + b":migration_owner:" + escaped + b"\n"


class _CaptureSpace:
    """Owned directories reserved before stop observations, never authority."""
    def __init__(self, context):
        self.context, self.ops = context, _operations(context)
        _require(context not in _SPACES and not self.ops._STOP_ATTEMPTS.get(context),
                 "PAPER_BACKUP_RESERVATION_ORDER")
        self.deadline = self.ops._ArchiveDeadline()  # setup is charged, before reservation I/O
        context.revalidate()
        self.owner = None
        self.closed = False
        self.selection = _local("owner_beta_paper_backup_selection", "owner-beta-paper-backup-selection.py")
        self.tree = self.selection._mkdir_private_tree(self.selection._backup_module().files)
        self.root = self.root_tree = None
        try:
            # journal._Tree is not a context manager; it only has descriptors to close.
            parent = self.ops.journal._Tree("/run")
            try:
                name = "lagrange-paper-capture-" + uuid.uuid4().hex
                os.mkdir(name, 0o700, dir_fd=parent.fd)
                os.fsync(parent.fd)
                self.root = Path("/run") / name
                self.root_tree = self.ops.journal._Tree(str(self.root))
            finally:
                parent.close()
            _SPACES[context] = self
        except BaseException:
            self.selection._dispose_private_tree(self.selection._backup_module().files, self.tree)
            raise

    def __enter__(self):
        return self

    def close(self):
        if self.closed:
            return
        _require(self.owner is None or self.owner._closed, "PAPER_BACKUP_CLEANUP_UNKNOWN")
        # Stop/archive owners must release their observations before removing
        # these directories changes any ancestor link count.
        _require(not any(not a.closed for a in self.ops._STOP_ATTEMPTS.get(self.context, {}).values()),
                 "PAPER_BACKUP_CLEANUP_UNKNOWN")
        self.selection._dispose_private_tree(self.selection._backup_module().files, self.tree)
        self.root_tree.revalidate()
        self.selection._remove_tree(self.root_tree.fd)
        self.root_tree.close()
        self.root.rmdir()
        self.closed = True
        _SPACES.pop(self.context, None)

    def __exit__(self, *error):
        try:
            self.close()
        except BaseException:
            if error[0] is None:
                raise


def reserve_capture(context):
    return _CaptureSpace(context)


def _sql_name(value):
    _require(type(value) is str and value and len(value.encode("utf-8")) <= 63
             and not any(ord(c) < 32 for c in value), "PAPER_BACKUP_CATALOG")
    return '"' + value.replace('"', '""') + '"'


def _sql_literal(value):
    _require(type(value) is str and not any(ord(c) < 32 for c in value),
             "PAPER_BACKUP_CATALOG")
    return "'" + value.replace("'", "''") + "'"


def _database_acl_sql(database, acl, owner):
    """Replay actual database ACL items, including grantor and grant options."""
    if acl is None:
        return ""  # CREATE DATABASE's null ACL is the original null ACL.
    _require(type(acl) is list, "PAPER_BACKUP_CATALOG")
    name = _sql_name(database)
    sql = ["REVOKE ALL ON DATABASE " + name + " FROM PUBLIC;",
           "REVOKE ALL ON DATABASE " + name + " FROM " + _sql_name(owner) + ";"]
    identifier = r'(?:"(?:[^"]|"")+"|[^=/"\s]+)'
    for item in acl:
        match = re.fullmatch(r"(" + identifier + r")?=([cCT*]*)/(" + identifier + r")", item)
        _require(match is not None, "PAPER_BACKUP_CATALOG")
        grantee, flags, grantor = match.groups()
        def unquote(text):
            return text[1:-1].replace('""', '"') if text.startswith('"') else text
        target = "PUBLIC" if grantee is None else _sql_name(unquote(grantee))
        index, seen = 0, set()
        while index < len(flags):
            code = flags[index]
            _require(code in "cCT" and code not in seen, "PAPER_BACKUP_CATALOG")
            seen.add(code); index += 1
            option = index < len(flags) and flags[index] == "*"
            index += int(option)
            privilege = {"c": "CONNECT", "C": "CREATE", "T": "TEMPORARY"}[code]
            sql.append("GRANT " + privilege + " ON DATABASE " + name + " TO " + target +
                       (" WITH GRANT OPTION" if option else "") + " GRANTED BY " +
                       _sql_name(unquote(grantor)) + ";")
    return "\n".join(sql) + "\n"


def _catalog_restore_sql(owner):
    """Derive clone initialization ONLY from the retained complete audit stream."""
    owner._check_spools()
    owner.audit.seek(0)
    owner.audit_module.decode(owner.audit)  # complete framing, all groups and EOF
    owner.audit.seek(0)
    roles, members, databases = [], [], []
    for raw in owner.audit:
        rec = owner.audit_module.parse(raw[:-1])
        # The audit parser uses Decimal for all JSON numbers; catalog SQL needs
        # canonical integer values, without accepting fractional role limits.
        rec = json.loads(owner.audit_module.canonical(rec))
        if len(rec) == 5 and rec[:2] == ["row", "schema_security"]:
            target = {"pg_roles": roles, "pg_auth_members": members,
                      "pg_database": databases}.get(rec[2])
            if target is not None:
                _require(len(target) < 4096, "PAPER_BACKUP_CATALOG")
                target.append(rec[4])
    _require(len(databases) == 1 and roles, "PAPER_BACKUP_CATALOG")
    database = databases[0]
    admins = sorted(r["rolname"] for r in roles if r["rolsuper"] is True and r["rolcanlogin"] is True)
    _require(admins and any(r["rolname"] == "migration_owner" and r["rolcanlogin"] is True
                           and r["rolsuper"] is False for r in roles), "PAPER_BACKUP_CATALOG")
    admin = database["owner"] if database["owner"] in admins else admins[0]
    names = {r["rolname"] for r in roles}
    _require(len(names) == len(roles) and database["owner"] in names, "PAPER_BACKUP_CATALOG")
    sql = ["SET standard_conforming_strings=on;"]
    flags = ("super", "inherit", "createrole", "createdb", "canlogin", "replication", "bypassrls")
    for role in roles:
        name = role["rolname"]
        _sql_name(name)
        _require(all(type(role["rol" + f]) is bool for f in flags)
                 and type(role["rolconnlimit"]) is int and role["rolconnlimit"] >= -1,
                 "PAPER_BACKUP_CATALOG")
        # PG18's reserved roles already exist in this independently pinned PG18
        # cluster. Exact catalog equality is still required by the native audit.
        if name.startswith("pg_"):
            _require(not role["rolsuper"] and not role["rolcanlogin"]
                     and role["rolconfig"] is None, "PAPER_BACKUP_CATALOG")
            continue
        if name != admin:
            sql.append("CREATE ROLE " + _sql_name(name) + ";")
        options = [("" if role["rol" + f] else "NO") +
                   {"super": "SUPERUSER", "canlogin": "LOGIN"}.get(f, f.upper()) for f in flags]
        sql.append("ALTER ROLE " + _sql_name(name) + " WITH " + " ".join(options) +
                   " CONNECTION LIMIT " + str(role["rolconnlimit"]) +
                   ("" if role["rolvaliduntil"] is None else
                    " VALID UNTIL " + _sql_literal(role["rolvaliduntil"])) + ";")
        for setting in role["rolconfig"] or []:
            key, sep, value = setting.partition("=")
            _require(sep and re.fullmatch(r"[a-z_][a-z0-9_.]*", key), "PAPER_BACKUP_CATALOG")
            sql.append("ALTER ROLE " + _sql_name(name) + " SET " + _sql_name(key) +
                       " TO " + _sql_literal(value) + ";")
    for member in members:
        _require(all(member[k] in names for k in ("role", "member", "grantor"))
                 and all(type(member[k]) is bool for k in ("admin_option", "inherit_option", "set_option")),
                 "PAPER_BACKUP_CATALOG")
        sql.append("GRANT " + _sql_name(member["role"]) + " TO " + _sql_name(member["member"]) +
                   " WITH ADMIN " + str(member["admin_option"]).upper() +
                   ", INHERIT " + str(member["inherit_option"]).upper() +
                   ", SET " + str(member["set_option"]).upper() +
                   " GRANTED BY " + _sql_name(member["grantor"]) + ";")
    name = owner.context.database["database"]
    _require(database["encoding"] == "UTF8" and database["allow_connections"] is True
             and type(database["connection_limit"]) is int, "PAPER_BACKUP_CATALOG")
    sql.append("CREATE DATABASE " + _sql_name(name) + " WITH TEMPLATE template0 OWNER " +
               _sql_name(database["owner"]) + " ENCODING 'UTF8' LC_COLLATE " +
               _sql_literal(database["collate"]) + " LC_CTYPE " + _sql_literal(database["ctype"]) +
               " CONNECTION LIMIT " + str(database["connection_limit"]) + ";")
    sql.append(_database_acl_sql(name, database["acl"], database["owner"]))
    owner._check_spools()
    return admin, ("\n".join(sql) + "\n").encode("utf-8")


class _ArchiveOriginal:
    """Named immutable file FD with stable ancestor generations, not mtimes."""
    def __init__(self, package, path, uid=0, gid=0, mode=0o400):
        self.ops, self.path = package.ops, str(path)
        self.tree = self.ops.journal._Tree(str(Path(path).parent))
        self.fd = None
        try:
            self.name = Path(path).name
            self.fd = os.open(self.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC,
                              dir_fd=self.tree.fd)
            info = self.ops._file(self.fd, mode, uid=uid, gid=gid)
            self.identity = self.ops._identity(info)
            self.size, self.sha256 = self.ops._hash_fd(self.fd)
            _require(self.size <= package.owner.maximum, "PAPER_BACKUP_LIMIT")
            self.revalidate(None)
        except BaseException:
            self.close()
            raise

    def revalidate(self, _files):
        self.tree.revalidate()
        _require(self.fd is not None and self.ops._identity(os.fstat(self.fd)) == self.identity
                 and self.ops._identity(os.stat(self.name, dir_fd=self.tree.fd,
                                               follow_symlinks=False)) == self.identity
                 and self.ops._hash_fd(self.fd) == (self.size, self.sha256),
                 "PAPER_BACKUP_ORIGINAL_CHANGED")

    def close(self):
        if self.fd is not None:
            os.close(self.fd); self.fd = None
        self.tree.close()


_PAPER_MEMBERS = tuple(name + ".enc" for name in (
    "postgres.dump", "files.bundle", "audit.stream", "dataset-rows.jsonl", "audit.json", "archive.json"))
_PAPER_FORMAT = "LAGRANGE_PAPER_COMBINED_BACKUP_V1"
_PAPER_SET = re.compile(r"backup-paper-[0-9]{8}T[0-9]{6}Z-[0-9a-f]{40}")


class _StoredPaperSet:
    """Retained encrypted originals. Native validation still consumes plaintext."""
    def __init__(self, storage, path, *, completed=True):
        self.storage, self.path, self.completed = storage, Path(path), completed
        self.tree = storage.ops.journal._Tree(str(path))
        self.files = {}
        self.closed = False
        self.publication=None
        try:
            expected = {*_PAPER_MEMBERS, "metadata.json"} | ({"COMPLETE"} if completed else set())
            _require(set(storage.daily.bounded_names(self.tree.fd,len(expected))) == expected, "PAPER_BACKUP_SET_MEMBERS")
            for name in sorted(expected):
                fd = os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=self.tree.fd)
                try:
                    info=storage.ops._file(fd,0o600,uid=0,gid=0)
                    _require(info.st_size <= (storage.maximum if name in _PAPER_MEMBERS else 65_536),"PAPER_BACKUP_LIMIT")
                    size,digest=storage.ops._hash_fd(fd)
                    self.files[name]=(fd,storage.ops._identity(info),size,digest)
                except BaseException:
                    os.close(fd)
                    raise
            metadata = self.read_small("metadata.json")
            self.metadata = storage.ops.journal._json(metadata, 65_536)
            value = self.metadata
            _require(type(value) is dict and set(value) == {"format","commit","created_at","encryption","files"}
                and value["format"] == _PAPER_FORMAT and storage.daily.install.is_commit(value["commit"])
                and (completed or value["commit"] == storage.context.commit)
                and value["encryption"] == "AES-256-CBC-PBKDF2-200000"
                and type(value["files"]) is dict and set(value["files"]) == set(_PAPER_MEMBERS)
                and type(value["created_at"]) is str
                and re.fullmatch(r"[0-9]{8}T[0-9]{6}Z",value["created_at"]), "PAPER_BACKUP_METADATA")
            try:
                self.created=storage.daily.datetime.datetime.strptime(value["created_at"],"%Y%m%dT%H%M%SZ")
            except ValueError:
                raise PaperBackupError("PAPER_BACKUP_METADATA") from None
            _require(not completed or (self.path.name == storage.prefix + value["created_at"] + "-" + value["commit"]
                and self.read_small("COMPLETE") == (_PAPER_FORMAT+"\n").encode()), "PAPER_BACKUP_SET_NAME")
            for name in _PAPER_MEMBERS:
                _require(value["files"][name] == self.files[name][3], "PAPER_BACKUP_CIPHER_DIGEST")
            self.revalidate()
        except BaseException:
            self.close()
            raise

    def read_small(self, name):
        fd, _identity, size, _digest = self.files[name]
        _require(size <= 65_536, "PAPER_BACKUP_METADATA")
        return os.pread(fd,size+1,0)

    def revalidate(self):
        _require(not self.closed, "PAPER_BACKUP_SET_CLOSED")
        self.tree.revalidate()
        expected = {*_PAPER_MEMBERS,"metadata.json"} | ({"COMPLETE"} if self.completed else set())
        _require(set(self.storage.daily.bounded_names(self.tree.fd,len(expected))) == expected, "PAPER_BACKUP_SET_MEMBERS")
        for name,(fd,identity,size,digest) in self.files.items():
            _require(self.storage.ops._identity(os.fstat(fd)) == identity
                and self.storage.ops._identity(os.stat(name,dir_fd=self.tree.fd,follow_symlinks=False)) == identity
                and self.storage.ops._hash_fd(fd) == (size,digest), "PAPER_BACKUP_SET_CHANGED")

    def close(self):
        if self.closed:
            return
        for fd,*_rest in self.files.values():
            os.close(fd)
        self.files.clear()
        self.tree.close()
        self.closed=True


class _PaperStorage:
    format, prefix, set_pattern, members = _PAPER_FORMAT, "backup-paper-", _PAPER_SET, _PAPER_MEMBERS
    def __init__(self, owner, values):
        self.owner, self.ops, self.context = owner, owner.ops, owner.context
        self.values, self.root, self.maximum = values, values["BACKUP_ROOT"], owner.maximum
        self.daily = owner.selection._backup_module()

    def sets(self):
        rows=[]
        with self.daily.install.root_directory(self.root) as root:
            for name in self.daily.bounded_names(root,10_000):
                if not name.startswith(self.prefix):
                    continue
                _require(_PAPER_SET.fullmatch(name), "PAPER_BACKUP_SET_NAME")
                value=_StoredPaperSet(self,Path(self.root)/name)
                try:
                    rows.append(dict(name=name,created=value.created,
                        size=sum(row[2] for row in value.files.values())))
                finally:
                    value.close()
        return sorted(rows,key=lambda row:(row["created"],row["name"]))

    def latest(self):
        rows=self.sets()
        _require(rows,"PAPER_BACKUP_NO_COMPLETE_SET")
        with self.daily.install.root_directory(self.root) as root:
            info=os.stat("latest",dir_fd=root,follow_symlinks=False)
            _require(stat.S_ISLNK(info.st_mode) and info.st_uid == 0
                and os.readlink("latest",dir_fd=root) == rows[-1]["name"],"PAPER_BACKUP_LATEST")
        return _StoredPaperSet(self,Path(self.root)/rows[-1]["name"])

    def publish(self, source, final, name):
        # Inventory is descriptive, not publication authority: the caller still
        # owns verified candidate custody, cleanup and both publication locks.
        candidate=_StoredPaperSet(self,source,completed=False)
        try:
            _require(name == self.prefix+candidate.metadata["created_at"]+"-"+candidate.metadata["commit"],
                "PAPER_BACKUP_SET_NAME")
            retained=sum(row["size"] for row in self.sets())
            # MAX_TOTAL_BYTES covers this entire backup root. The existing
            # Daily reader validates its five non-Paper formats and uses only
            # self.root; its grammar excludes Paper, so sizes cannot overlap.
            # Keep those rows out of Paper's separate latest/clock inventory.
            retained+=sum(row["size"] for row in self.daily.Storage.sets(self))
            candidate.revalidate()
            # All existing bytes (including metadata/COMPLETE) are measured.
            # The sole not-yet-written member is this exact fixed marker.
            size=sum(row[2] for row in candidate.files.values())+len((self.format+"\n").encode())
            _require(retained+size <= self.values["MAX_TOTAL_BYTES"],"PAPER_BACKUP_RETENTION_CAP")
        finally:
            candidate.close()
        # No Paper pruning policy exists: over-cap refusal leaves old sets and
        # latest intact, before the writer can create COMPLETE or rename a set.
        # Share the existing fsync/no-replace COMPLETE and atomic latest writer.
        self.daily.Storage.publish(self,source,final,name)


class _PaperPublication:
    def __init__(self, package):
        self.owner, self.storage = package.owner, _PaperStorage(package.owner,package.values)
        self.closed=self.published=False
        self.path=Path(self.storage.root)/(".paper-capture-"+uuid.uuid4().hex)
        self.tree=None
        self.retained=None
        self.archive_sha256=self.generation=None
        self.verified_package=None
        self.release_tree=None
        self.release_lock_fd=None
        package.revalidate()
        os.mkdir(self.path,0o700)
        self.tree=self.owner.ops.journal._Tree(str(self.path))
        try:
            hashes={}
            for name in _PAPER_MEMBERS:
                original=package.ciphertexts[name]
                original.revalidate(None)
                with os.fdopen(os.open(name,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW|os.O_CLOEXEC,
                                      0o600,dir_fd=self.tree.fd),"wb") as output:
                    offset=0
                    while chunk := os.pread(original.fd,1024*1024,offset):
                        package._reserve(len(chunk));output.write(chunk);offset+=len(chunk)
                    output.flush();os.fsync(output.fileno())
                original.revalidate(None)
                hashes[name]=original.sha256
            stamp=time.strftime("%Y%m%dT%H%M%SZ",time.gmtime())
            self.name=self.storage.prefix+stamp+"-"+self.storage.context.commit
            metadata=dict(format=_PAPER_FORMAT,commit=self.storage.context.commit,created_at=stamp,
                encryption="AES-256-CBC-PBKDF2-200000",files=hashes)
            self.owner.ops.journal._new_file(self.tree.fd,"metadata.json",self.owner.ops._canonical(metadata),0o600)
            os.fsync(self.tree.fd)
            original=next(v for v in package.originals if v.path == str(package.path/"archive.json"))
            original.revalidate(None)
            value=self.owner.ops.journal._json(os.pread(original.fd,original.size+1,0),65_536)
            self.archive_sha256,self.generation=original.sha256,value["generation"]
            self.release_tree=self.owner.ops.journal._Tree(self.owner.context.install_root)
            self.release_lock_fd=os.dup(self.owner.context.lock_fd)
            self.release_lock_identity=self.owner.context.lock_identity
            self.retained=_StoredPaperSet(self.storage,self.path,completed=False)
            self.retained.publication=self
            package.revalidate()
            _PUBLICATIONS.add(self)
        except BaseException:
            self.close()
            raise

    def require_verified(self):
        package=self.verified_package
        _require(self in _PUBLICATIONS and not self.closed and type(package) is _ArchivePackage
            and package.persisted is self.retained and package.owner is self.owner and package.closed
            and package.completed_destination is package.destination and package.destination.closed
            and package.guest.absent and all(reader.absent for reader in package.readers)
            and package.process is None and package.call is None,"PAPER_BACKUP_CLEANUP_UNKNOWN")
        self.retained.revalidate()

    def publish(self):
        self.require_verified()
        _require(self.owner._closed and self.owner.space.closed and self.owner.context._closed
            and self.owner._resources_closed() and self.owner.secret_sources is None
            and self.owner._snapshot is None and self.owner.selected is None,
            "PAPER_BACKUP_CLEANUP_UNKNOWN")
        context=self.owner.context
        _require(self.owner.ops.journal._check_lock(context.install_root,self.release_lock_fd,self.release_tree)
            == self.release_lock_identity,"PAPER_BACKUP_PUBLICATION_LOCK")
        self.owner.ops.daily.policy.validate_current_release(context.install_root,context.release_path,context.commit)
        self.retained.revalidate()
        final=Path(self.storage.root)/self.name
        self.storage.publish(self.path,final,self.name)
        # Renaming deliberately changes path ownership. The private persisted
        # bytes were already freshly verified; only close pre-rename handles.
        self.published=True
        self.retained.close();self.retained=None
        self.tree.close();self.tree=None
        self.path=final
        return None

    def close(self):
        if self.closed:
            return
        if self.retained is not None:
            self.retained.close();self.retained=None
        if self.tree is not None:
            self.tree.close();self.tree=None
        if self.release_lock_fd is not None:
            os.close(self.release_lock_fd);self.release_lock_fd=None
        if self.release_tree is not None:
            self.release_tree.close();self.release_tree=None
        # A prepared/renamed set is never silently removed after fsync/rename
        # uncertainty. It is finite diagnostic residue, not a COMPLETE success.
        self.closed=True
        _PUBLICATIONS.discard(self)


class _PackageAuditView:
    """Borrow only the independently retained clone-source stream for catalog setup."""
    def __init__(self, package):
        self.package = package
        self.context, self.audit_module = package.owner.context, package.owner.audit_module
        original = next(v for v in package.originals if v.path == str(package.path / "audit.stream"))
        original.revalidate(None)
        self.audit = os.fdopen(os.dup(original.fd), "rb")
    def _check_spools(self):
        for original in self.package.originals:
            original.revalidate(None)
    def close(self):
        self.audit.close()


class _ArchivePackage:
    """Live captured/decrypted originals; never serializable archive authority."""
    def __init__(self, owner, limits, values, persisted=None):
        _require(type(owner) in {_HeldPaperSnapshot,_PersistedArchiveOwner} and _OWNERS.get(owner._custody) is owner
                 and owner._entered and not owner._closed, "PAPER_BACKUP_PRIVATE")
        owner.revalidate()
        _require((persisted is None and type(owner) is _HeldPaperSnapshot) or
            (type(persisted) is _StoredPaperSet and persisted.storage.owner is owner and
             (persisted.completed or (type(persisted.publication) is _PaperPublication
              and persisted.publication in _PUBLICATIONS and persisted.publication.retained is persisted
              and persisted.publication.owner is owner))),"PAPER_BACKUP_PRIVATE")
        _require(owner.package is None and type(values) is dict, "PAPER_BACKUP_PRIVATE")
        self.owner, self.ops, self.limits, self.values = owner, owner.ops, limits, values
        self.work = owner.root / ("restore-" + uuid.uuid4().hex)
        self.work_tree = None
        self.path = self.work / "archive"
        self.persisted = persisted
        self.tree = self.snapshot = self.guest = self.call = self.custody = None
        self.process = self.process_identity = None
        self.native_processes = {}
        self.readers = []
        self.destination = self.completed_destination = None
        self.originals = []
        self.ciphertexts = {}
        self.directories = []
        self.closed = False
        self.database = dict(owner.context.database, host="/run/postgresql")
        self.password = os.urandom(32).hex().encode("ascii")  # clone only; no source secret
        owner.package = self
        _PACKAGES.add(self)

    def __reduce__(self):
        raise PaperBackupError("PAPER_BACKUP_PRIVATE")

    def __copy__(self):
        raise PaperBackupError("PAPER_BACKUP_PRIVATE")

    def __deepcopy__(self, _memo):
        raise PaperBackupError("PAPER_BACKUP_PRIVATE")

    def _reserve(self, size=0):
        self.owner._custody.deadline.work_remaining()
        storage = self.owner.selection._backup_module()
        _require(storage.shutil.disk_usage(self.owner.root).free >=
                 self.values["MIN_FREE_BYTES"] + size, "PAPER_BACKUP_SPACE")

    def _command_file(self, args, target, source=None, seconds=900):
        """Existing crypto format, with the original capture clock and child custody."""
        self._reserve()
        fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
        try:
            with os.fdopen(fd, "wb") as output:
                self.process = subprocess.Popen(args, env=self.owner.selection._backup_module().ops.ENVIRONMENT,
                    stdin=source, stdout=output, stderr=subprocess.DEVNULL, close_fds=True)
                if self.process.poll() is None:
                    self.process_identity = self.ops._process_record(self.process.pid)
                while self.process.poll() is None:
                    self._reserve()
                    _require(os.fstat(output.fileno()).st_size <= self.owner.maximum, "PAPER_BACKUP_LIMIT")
                    time.sleep(min(0.05, self.owner._custody.deadline.work_remaining(0.05)))
                _require(self.process.returncode == 0 and os.fstat(output.fileno()).st_size <= self.owner.maximum,
                         "PAPER_BACKUP_CRYPTO")
                output.flush(); os.fsync(output.fileno())
        finally:
            if self.process is not None and self.process.poll() is None:
                self.process.terminate()
                try:
                    self.process.wait(timeout=self.owner._custody.deadline.remaining(5))
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait(timeout=self.owner._custody.deadline.remaining(5))
            if self.process is not None and self.process.poll() is not None:
                self.process = self.process_identity = None

    def prepare(self):
        owner, files = self.owner, self.owner.selection._backup_module().files
        _require(self in _PACKAGES and not self.closed and self.tree is None and self.guest is None,
                 "PAPER_BACKUP_PRIVATE")
        self._reserve()
        os.mkdir(self.work, 0o700)
        self.work_tree = self.ops.journal._Tree(str(self.work))
        os.mkdir(self.path, 0o700)
        self.tree = self.ops.journal._Tree(str(self.path))
        os.mkdir(self.path / "files", 0o700)
        os.mkdir(self.path / "coordination", 0o700)
        for name in ("files", "coordination"):
            fd = os.open(name, self.ops.journal.DIR_FLAGS, dir_fd=self.tree.fd)
            self.directories.append((self.tree.fd, name, fd, self.ops.journal._dir_identity(os.fstat(fd))))
        # Reserve every later directory before binding immutable parent identities.
        for leaf in ("clone-data", "clone-socket"):
            os.mkdir(self.work / leaf, 0o700); os.chown(self.work / leaf, 999, 999)
        # The enclosing capture directory is root0700. The helper has no DAC
        # override capability, so its readonly socket bind needs traversal.
        os.chmod(self.work / "clone-socket", 0o755)
        daily = owner.selection._backup_module()
        for name in ("files.bundle", "audit.stream", "postgres.dump", "dataset-rows.jsonl", "audit.json", "archive.json"):
            owner._check_spools()
            encrypted = owner.root / (name + ".enc")
            target = self.path / name if name != "postgres.dump" else self.work / "clone.dump"
            if self.persisted is None:
                daily._paper_crypt(self, owner.root / name, encrypted)
            else:
                self.persisted.revalidate()
                encrypted = self.persisted.path / (name + ".enc")
            cipher = _ArchiveOriginal(self, encrypted, mode=0o600)
            self.ciphertexts[name+".enc"] = cipher
            daily._paper_crypt(self, encrypted, target, decrypt=True)
            cipher.revalidate(None)
            fd = os.open(target, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
            try:
                if self.persisted is None:
                    _require(self.ops._hash_fd(fd) == owner.spools[name][2:], "PAPER_BACKUP_CRYPTO_CHANGED")
                os.fchmod(fd, 0o400); os.fsync(fd)
            finally:
                os.close(fd)
            self.originals.append(_ArchiveOriginal(self, target))
        with open(self.path / "files.bundle", "rb") as stream:
            manifest = files.restore_archive(stream, self.limits, str(self.path / "files"),
                                             self._reserve, profile="paper-combined")
        roots = []
        for name in files.PAPER_CLASSES:
            row = next(r for r in manifest["entries"] if r["path"] == name)
            roots.append(files.Root(name, str(self.path / "files" / name), row["uid"], row["gid"], row["mode"]))
        self.snapshot = files.Snapshot(roots, self.limits, profile="paper-combined")
        self.snapshot.__enter__()
        _require(self.snapshot.records == manifest["entries"], "PAPER_BACKUP_MATERIALIZED_CHANGED")
        parent = self.directories[0][2]
        for name, fd in self.snapshot.descriptors.items():
            self.directories.append((parent, name, os.dup(fd), self.ops.journal._dir_identity(os.fstat(fd))))
        os.fsync(self.tree.fd)
        self._validate_source_descriptors()
        self.guest = _PaperRestoreGuest(self)
        self.guest.start()
        self.revalidate()

    def _validate_source_descriptors(self):
        by_name={Path(original.path).name:original for original in self.originals}
        archive_original,audit_original=by_name["archive.json"],by_name["audit.json"]
        archive=self.ops.journal._json(os.pread(archive_original.fd,archive_original.size+1,0),65_536)
        audit=self.ops.journal._json(os.pread(audit_original.fd,audit_original.size+1,0),65_536)
        binding=dict(domain="lagrange-paper-snapshot-binding-v1",
            **{key:archive[key] for key in ("installation_id","generation","account_id","control_epoch")},
            database_dump_sha256=by_name["clone.dump"].sha256,
            audit_stream_sha256=by_name["audit.stream"].sha256,files_sha256=by_name["files.bundle"].sha256)
        _require(archive["database_dump_sha256"]==binding["database_dump_sha256"]
            and archive["files_sha256"]==binding["files_sha256"]
            and archive["database_snapshot_sha256"]==self.ops._digest(self.ops._canonical(binding))
            and archive["audit_sha256"]==audit_original.sha256
            and all(archive[key]==audit[key] for key in ("installation_id","generation","account_id","control_epoch")),
            "PAPER_BACKUP_DESCRIPTOR_BINDING")
        for original in by_name.values():
            original.revalidate(None)

    def revalidate(self):
        _require(self in _PACKAGES and not self.closed and self.owner.package is self
                 and _OWNERS.get(self.owner._custody) is self.owner, "PAPER_BACKUP_PRIVATE")
        self.owner.revalidate()
        if self.custody is not None:
            self.custody._check()
            action = self.owner.drained._attempt.consumer.actions[-1]
            _require(self.custody._intent == self.owner.operation._intent_bytes
                     and self.custody._action == action._binding_bytes
                     and self.custody._artifact == action._artifact_bytes
                     and self.custody._dispatch == action._dispatch_bytes,
                     "PAPER_BACKUP_CUSTODY")
        self.owner._custody.deadline.work_remaining()
        self.owner._check_spools()
        self.work_tree.revalidate()
        self.tree.revalidate()
        if self.persisted is not None:
            self.persisted.revalidate()
        _require(set(self.owner.selection._backup_module().bounded_names(self.tree.fd,7)) ==
                 {"files.bundle", "audit.stream", "archive.json", "audit.json", "dataset-rows.jsonl", "files", "coordination"}, "PAPER_BACKUP_ORIGINAL_CHANGED")
        for parent, name, fd, identity in self.directories:
            _require(self.ops.journal._dir_identity(os.fstat(fd)) == identity and
                     self.ops.journal._dir_identity(os.stat(name, dir_fd=parent,
                                                           follow_symlinks=False)) == identity,
                     "PAPER_BACKUP_ORIGINAL_CHANGED")
        files = self.owner.selection._backup_module().files
        for original in [*self.originals, *self.ciphertexts.values()]:
            original.revalidate(files)
        self.snapshot.verify_unchanged()
        _require(self.guest is not None, "PAPER_BACKUP_CLONE")
        self.guest.check(ready=True)

    def invoke(self):
        _require(self.call is None and self.custody is None, "PAPER_BACKUP_NATIVE_REPLAY")
        self.revalidate()
        old = self.owner.context.docker
        self.owner.context.docker = self.ops._ArchiveWorkDocker(old, self.owner._custody.deadline)
        primary = None
        try:
            if self.persisted is None:
                consumer = self.owner.drained._attempt.consumer
                action = consumer.actions[-1]
                self.custody = self.owner.operation._journal.open_custody(self.owner.generation, action.sequence)
                _require(self.custody._readonly and self.custody._action == action._binding_bytes
                         and self.custody._dispatch == action._dispatch_bytes
                         and self.custody._artifact == action._artifact_bytes, "PAPER_BACKUP_CUSTODY")
                self.call = self.owner.context._invoke_archive(self)
                self.call.__enter__()
                self.revalidate()
                self.call._retain_archive_terminal()
                _require(self.call.result["outcome"] == "REPLAY"
                         and self.call.result["error_code"] == "NONE", "PAPER_BACKUP_NATIVE_INCOMPLETE")
                original = self.ops.journal._json(self.call.readback_artifact(), self.ops.journal.MAX_NATIVE_EXACT)
                _require(original["status"] == "COMMITTED" and original["current_state"] == "OFF"
                         and original["current_epoch"] == consumer.floor
                         and original["event_id"] == consumer.event
                         and original["account_id"] == self.owner.operation._intent["account_id"]
                         and original["request_sha256"] == self.ops._digest(self.custody._sql)
                         and original["result_canonical"].encode("ascii") == consumer.off_result,
                         "PAPER_BACKUP_CLONE_READBACK")
                self.call.close()
                self.call = None
                self.custody.close()
                self.custody = None
            destination = _DestinationRestore(self)
            destination.start()
            destination.stop_and_restore_off()
            destination.compare_post_restore()
            destination.verify_role_visibility()
            self.completed_destination=destination
        except BaseException as error:
            primary = error
            raise
        finally:
            try:
                self.owner.context.docker=self.ops._StopDocker(old,self.owner._custody.deadline)
                if self.call is not None:
                    self.call.close()
                    self.call = None
            except BaseException:
                self.owner._poisoned = True
                if primary is None:
                    raise
            finally:
                self.owner.context.docker = old

    def close(self):
        if self.closed:
            return
        failure = None
        for resource in (self.call, *getattr(self, "readers", ()), getattr(self,"destination",None), self.guest):
            if resource is not None:
                try:
                    resource.close()
                except BaseException as error:
                    if failure is None:
                        failure = error
        if failure is not None or self.process is not None:
            self.owner._poisoned = True
            raise PaperBackupError("PAPER_BACKUP_CLEANUP_UNKNOWN") from None
        if self.custody is not None:
            self.custody.close(); self.custody = None
        if self.snapshot is not None:
            try:
                self.snapshot.__exit__(None, None, None)
            except BaseException as error:
                failure = error
            self.snapshot = None
        for original in [*self.originals, *self.ciphertexts.values()]:
            try:
                original.close()
            except BaseException as error:
                if failure is None:
                    failure = error
        self.originals.clear()
        self.ciphertexts.clear()
        for _parent, _name, fd, _identity in reversed(self.directories):
            try:
                os.close(fd)
            except BaseException as error:
                if failure is None:
                    failure = error
        self.directories.clear()
        if self.tree is not None:
            self.tree.close(); self.tree = None
        if self.work_tree is not None:
            self.work_tree.close(); self.work_tree = None
        self.password = None
        self.closed = True
        _PACKAGES.discard(self)
        if failure is not None:
            self.owner._poisoned = True
            raise failure


class _PaperRestoreGuest:
    """Exact owned networkless PG18 clone; no source cluster authentication."""
    __reduce__ = _ArchivePackage.__reduce__
    __copy__ = _ArchivePackage.__copy__
    __deepcopy__ = _ArchivePackage.__deepcopy__

    def __init__(self, package):
        _require(type(package) is _ArchivePackage and package in _PACKAGES, "PAPER_BACKUP_PRIVATE")
        self.package, self.owner, self.ops = package, package.owner, package.ops
        self.nonce = uuid.uuid4().hex
        self.name = "paper-archive-clone-" + self.nonce
        self.cid = self.created = self.process = self.socket_identity = self.socket_inode = None
        self.started = self.absent = False
        self.create_sent = False
        self.started_at = None
        self.directories = []
        self.cgroup_fd = self.cgroup_identity = None
        self.mounts = ()
        self.socket = self.package.work / "clone-socket"

    def start(self):
        _require(not self.create_sent and self.cid is None and not self.absent,
                 "PAPER_BACKUP_CLONE_UNKNOWN")
        owner = self.owner
        view = _PackageAuditView(self.package)
        try:
            admin, roles = _catalog_restore_sql(view)
        finally:
            view.close()
        self.admin = admin
        image = owner.context.docker.object("image", self.ops.POSTGRES_IMAGE)
        _require(self.ops.POSTGRES_IMAGE in image.get("RepoDigests", []) and
                 self.ops.runtime.DIGEST.fullmatch(image.get("Id", "")), "PAPER_BACKUP_IMAGE")
        self.image = image["Id"]
        for name, mode in (("clone-data", 0o700), ("clone-socket", 0o755)):
            fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                         dir_fd=self.package.work_tree.fd)
            self.directories.append((name, fd, self.ops.journal._dir_identity(os.fstat(fd))))
            info = os.fstat(fd)
            _require((info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode)) == (999, 999, mode),
                     "PAPER_BACKUP_CLONE_CHANGED")
        roles_path = self.package.work / "clone-roles.sql"
        fd = os.open(roles_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o400)
        with os.fdopen(fd, "wb") as stream:
            stream.write(roles); stream.flush(); os.fchown(stream.fileno(), 999, 999); os.fsync(stream.fileno())
        # The protected dump is mounted separately. A read-only fd copy with
        # guest ownership is necessary; production credentials are not mounted.
        dump = self.package.work / "guest.dump"
        original_dump = next(original for original in self.package.originals
                             if original.path == str(self.package.work / "clone.dump"))
        original_dump.revalidate(None)
        with os.fdopen(os.dup(original_dump.fd), "rb") as source, open(dump, "xb") as target:
            source.seek(0)
            while chunk := source.read(1024 * 1024):
                self.package._reserve(len(chunk)); target.write(chunk)
            target.flush(); os.fchown(target.fileno(), 999, 999); os.fchmod(target.fileno(), 0o400)
            os.fsync(target.fileno())
        files = owner.selection._backup_module().files
        for path in (roles_path, dump):
            self.package.originals.append(_ArchiveOriginal(self.package, path, uid=999, gid=999))
        copied_dump = self.package.originals[-1]
        _require((copied_dump.size, copied_dump.sha256) == (original_dump.size, original_dump.sha256),
                 "PAPER_BACKUP_CRYPTO_CHANGED")
        original_dump.revalidate(None)
        script = owner.selection._OriginalFile(files, owner.context.release_path +
                    "/deploy/db/paper-backup-verify.sh", uid=0, gid=0, maximum=64*1024)
        self.package.originals.append(script)
        self.mounts = ((str(self.package.work / "clone-data"), "/var/lib/postgresql/verify", False),
                       (str(self.socket), "/run/postgresql", False),
                       (str(roles_path), "/verify/roles.sql", True),
                       (str(dump), "/verify/postgres.dump", True),
                       (script.path, "/verify/entry.sh", True))
        args = ["create", "--name", self.name, "--label", "lagrange.paper.archive=" + self.nonce,
                "--user", "999:999", "--network", "none", "--read-only", "--cap-drop", "ALL",
                "--security-opt", "no-new-privileges", "--restart", "no", "--log-driver", "none",
                "--memory", "2147483648", "--cpus", "1",
                "--tmpfs", _IMAGE_VOLUME + ":" + _GUEST_VOLUME_COVER, "--entrypoint", "/bin/bash"]
        for source, target, readonly in self.mounts:
            args += ["--mount", "type=bind,source=" + source + ",target=" + target +
                     (",readonly" if readonly else "")]
        args += [self.image, "/verify/entry.sh", admin, self.package.database["database"]]
        # Name/nonce is retained before sending create. Lost response is never
        # permission to delete an object without independently matching it.
        self.create_sent = True
        cid = owner._docker(args).strip()
        _require(self.ops.runtime.CONTAINER.fullmatch(cid), "PAPER_BACKUP_CLONE_UNKNOWN")
        self.cid = cid
        value = owner.context.docker.object("container", cid)
        self.created = value.get("Created")
        self.check()
        owner._docker(["start", cid]); self.started = True
        while True:
            self.package._reserve()
            self.check()
            value = owner.context.docker.object("container", cid)
            _require(value.get("State", {}).get("Running") is True, "PAPER_BACKUP_CLONE_EXIT")
            started_at = value["State"].get("StartedAt")
            _require(type(started_at) is str and started_at and
                     (self.started_at is None or self.started_at == started_at),
                     "PAPER_BACKUP_CLONE_CHANGED")
            self.started_at = started_at
            try:
                info = (self.socket / ".s.PGSQL.5432").lstat()
            except FileNotFoundError:
                time.sleep(min(0.05, owner._custody.deadline.work_remaining(0.05)))
                continue
            record = self.ops._process_record(value["State"]["Pid"])
            fd = os.open(Path("/proc") / str(record["pid"]) / "root/usr/lib/postgresql/18/bin/postgres",
                         os.O_RDONLY | os.O_CLOEXEC)
            try:
                expected = self.ops._hash_fd(fd)
            finally:
                os.close(fd)
            if (record["executable"]["bytes"], record["executable"]["sha256"]) != expected:
                time.sleep(min(0.05, owner._custody.deadline.work_remaining(0.05)))
                continue  # bootstrap shell's socket is not serving readiness
            _require(stat.S_ISSOCK(info.st_mode) and info.st_uid == 999, "PAPER_BACKUP_CLONE_SOCKET")
            self.process, self.socket_identity = record, self.ops._identity(info)
            _require(self.cid in record["cgroup"], "PAPER_BACKUP_CLONE_CHANGED")
            group = Path("/sys/fs/cgroup") / record["cgroup"].lstrip("/")
            self.cgroup_fd = os.open(group, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
            self.cgroup_identity = self.ops.journal._dir_identity(os.fstat(self.cgroup_fd))
            self.check(ready=True)
            return

    def check(self, *, ready=False):
        _require(not self.absent and self.cid is not None, "PAPER_BACKUP_CLONE_UNKNOWN")
        for name, fd, identity in self.directories:
            _require(self.ops.journal._dir_identity(os.fstat(fd)) == identity and
                     self.ops.journal._dir_identity(os.stat(name, dir_fd=self.package.work_tree.fd,
                                                           follow_symlinks=False)) == identity,
                     "PAPER_BACKUP_CLONE_CHANGED")
        value = self.owner.context.docker.object("container", self.cid)
        config, host, state = (value.get(k) or {} for k in ("Config", "HostConfig", "State"))
        _require(value.get("Id") == self.cid and value.get("Created") == self.created
                 and value.get("Image") == self.image and value.get("Name") == "/" + self.name
                 and config.get("Labels", {}).get("lagrange.paper.archive") == self.nonce
                 and config.get("User") == "999:999" and config.get("Entrypoint") == ["/bin/bash"]
                 and config.get("Cmd") == ["/verify/entry.sh", self.admin, self.package.database["database"]]
                 and host.get("NetworkMode") == "none" and host.get("ReadonlyRootfs") is True
                 and host.get("Privileged") is False and host.get("CapDrop") == ["ALL"]
                 and not host.get("CapAdd") and host.get("RestartPolicy", {}).get("Name") == "no"
                 and host.get("AutoRemove") is False and not host.get("Binds")
                 and not host.get("PortBindings") and not host.get("Devices")
                 and not host.get("VolumesFrom")
                 and host.get("Memory") == 2147483648 and host.get("NanoCpus") == 1000000000
                 and host.get("LogConfig", {}).get("Type") == "none"
                 and host.get("SecurityOpt") in (["no-new-privileges"], ["no-new-privileges:true"])
                 and host.get("PidMode") in ("", "private") and host.get("IpcMode") in ("", "private")
                 and not host.get("DeviceRequests") and not host.get("PublishAllPorts")
                 and not host.get("Links")
                 and value.get("RestartCount") == 0 and not state.get("OOMKilled"),
                 "PAPER_BACKUP_CLONE_CHANGED")
        _require(self.started_at is None or state.get("StartedAt") == self.started_at,
                 "PAPER_BACKUP_CLONE_CHANGED")
        networks = value.get("NetworkSettings", {}).get("Networks") or {}
        _require(set(networks) <= {"none"} and all(not n.get("IPAddress") for n in networks.values()),
                 "PAPER_BACKUP_CLONE_CHANGED")
        self.ops._validate_mount_records(
            _covered_mounts(value, _GUEST_VOLUME_COVER, "PAPER_BACKUP_CLONE_CHANGED"), self.mounts)
        if ready:
            _require(state.get("Running") is True and self.process is not None
                     and self.ops._process_record(state["Pid"]) == self.process
                     and self.ops._identity((self.socket / ".s.PGSQL.5432").lstat()) == self.socket_identity,
                     "PAPER_BACKUP_CLONE_CHANGED")
            _require(self.cgroup_fd is not None and
                     self.ops.journal._dir_identity(os.fstat(self.cgroup_fd)) == self.cgroup_identity,
                     "PAPER_BACKUP_CLONE_CHANGED")
            proc = Path("/proc") / str(self.process["pid"])
            unix = [line.split() for line in (proc / "net/unix").read_text().splitlines()[1:]]
            matches = [r[6] for r in unix if len(r) == 8 and r[7] == "/run/postgresql/.s.PGSQL.5432"]
            owned = {os.readlink(p) for p in (proc / "fd").iterdir()}
            matches = [n for n in matches if "socket:[" + n + "]" in owned]
            _require(len(matches) == 1 and (self.socket_inode is None or self.socket_inode == matches[0]),
                     "PAPER_BACKUP_CLONE_SOCKET")
            self.socket_inode = matches[0]

    def observe_fence(self, destination):
        self.check(ready=True)
        reader = _GuestClient(self.package, self.admin)
        try:
            reader.start()
            raw = reader.exchange(("SELECT jsonb_build_object('observer',jsonb_build_object("
                "'pid',pg_backend_pid(),'session',session_user,'role',current_user,"
                "'superuser',(SELECT rolsuper FROM pg_roles WHERE rolname=current_user)),"
                "'prepared',(SELECT count(*) FROM pg_prepared_xacts),"
                "'backends',COALESCE((SELECT jsonb_agg(jsonb_build_object('pid',pid,"
                "'start_us',(extract(epoch FROM backend_start)*1000000)::bigint) ORDER BY pid) "
                "FROM pg_stat_activity WHERE backend_type='client backend' AND pid<>pg_backend_pid()),'[]'::jsonb),"
                "'fence',(SELECT jsonb_build_object('generation',o.generation,'installation_id',o.installation_id,"
                "'account_id',o.account_id,'epoch',c.control_epoch,'state',c.state,'event',o.fence_event_id) "
                "FROM public.owner_beta_paper_operations o JOIN public.owner_beta_paper_control c ON c.account_id=o.account_id "
                "WHERE o.generation=" + _sql_literal(destination.operation.generation) + "::uuid));\n").encode("ascii"))
            try:
                database = json.loads(raw)
            except (ValueError, UnicodeError):
                raise PaperBackupError("PAPER_BACKUP_GUEST_OBSERVATION") from None
            root = destination.operation._intent
            expected = dict(generation=root["generation"], installation_id=root["installation_id"],
                account_id=root["account_id"], epoch=destination.fence, state="DRAINING", event=destination.begin_event)
            _require(type(database) is dict and database.get("prepared") == 0
                     and database.get("backends") == [] and database.get("fence") == expected
                     and database.get("observer") == dict(pid=reader.backend["pid"], session=self.admin,
                                                          role=self.admin, superuser=True),
                     "PAPER_BACKUP_GUEST_WRITER")
            processes = []
            for pid, group in self.ops._proc_cgroups().items():
                if group == self.process["cgroup"]:
                    row = self.ops._process_record(pid)
                    _require(row["executable"] == self.process["executable"], "PAPER_BACKUP_GUEST_WRITER")
                    processes.append(row)
            _require(self.process in processes and reader.backend_process in processes and len(processes) <= 64,
                     "PAPER_BACKUP_GUEST_WRITER")
            value = self.owner.context.docker.object("container", self.cid)
            self.check(ready=True)
            sample = {"container": self.ops._container_generation(value),
                "postmaster": self.process, "processes": sorted(processes, key=lambda row: row["pid"]),
                "cgroup_identity": list(self.cgroup_identity), "socket_identity": list(self.socket_identity),
                "socket_inode": self.socket_inode, "mounts": [list(row) for row in self.mounts],
                "network": value["HostConfig"]["NetworkMode"], "restart": value["HostConfig"]["RestartPolicy"]["Name"],
                "database": database, "observer_backend": reader.backend_process,
                "observer_namespace_pids": reader.namespace_pids}
        finally:
            reader.close()
        sample["observer_terminal"] = reader.terminal
        self.check(ready=True)
        return sample

    def owns_process(self, pid):
        if self.process is None:
            return False
        self.check(ready=True)
        record = self.ops._process_record(pid)
        return record["cgroup"] == self.process["cgroup"] and (
            record == self.process or record["executable"] == self.process["executable"])

    def close(self):
        if self.absent:
            return
        if self.cid is None:
            if not self.create_sent:
                for _name, fd, _identity in self.directories:
                    os.close(fd)
                self.directories.clear()
                self.absent = True
                return
            # Creation may have succeeded despite a missing response. Retain
            # name/nonce and the whole capture owner for explicit recovery.
            raise PaperBackupError("PAPER_BACKUP_CLONE_UNKNOWN")
        self.check()
        value = self.owner.context.docker.object("container", self.cid)
        if value.get("State", {}).get("Running"):
            self.owner._cleanup_docker(["stop", "--time", "10", self.cid])
        finished = None
        for _ in range(2):
            self.check()
            state = self.owner.context.docker.object("container", self.cid)["State"]
            _require(state.get("Running") is False and state.get("Status") == "exited"
                     and type(state.get("ExitCode")) is int and state.get("FinishedAt"),
                     "PAPER_BACKUP_CLONE_UNKNOWN")
            _require(finished is None or finished == state["FinishedAt"], "PAPER_BACKUP_CLONE_CHANGED")
            finished = state["FinishedAt"]
            if self.process is not None:
                _require(self.cgroup_fd is not None and
                         self.ops.journal._dir_identity(os.fstat(self.cgroup_fd)) == self.cgroup_identity,
                         "PAPER_BACKUP_CLONE_UNKNOWN")
                group = Path("/sys/fs/cgroup") / self.process["cgroup"].lstrip("/")
                try:
                    named = group.lstat()
                except FileNotFoundError:
                    named = None
                if named is not None:
                    _require(self.ops.journal._dir_identity(named) == self.cgroup_identity,
                             "PAPER_BACKUP_CLONE_CHANGED")
                    fd = os.open("cgroup.procs", os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC,
                                 dir_fd=self.cgroup_fd)
                    try:
                        _require(not os.read(fd, 65537).strip(), "PAPER_BACKUP_CLONE_UNKNOWN")
                    finally:
                        os.close(fd)
                try:
                    start, _state = self.ops._process_generation(self.process["pid"])
                except FileNotFoundError:
                    start = None
                _require(start != self.process["start_ticks"], "PAPER_BACKUP_CLONE_UNKNOWN")
        self.owner._cleanup_docker(["rm", self.cid])
        _require(not self.owner._cleanup_docker(["ps", "--all", "--no-trunc", "--filter", "id=" + self.cid,
                                               "--format", "{{.ID}}"]).strip(), "PAPER_BACKUP_CLONE_UNKNOWN")
        self.absent = True
        for _name, fd, _identity in self.directories:
            os.close(fd)
        self.directories.clear()
        if self.cgroup_fd is not None:
            os.close(self.cgroup_fd); self.cgroup_fd = None


class _DumpGate:
    """Retain the real client at a kernel syscall stop until DB correlation.

    Arm on the already owned psql BEFORE writing its shell command. Fork/vfork
    events hold descendants before execution; exec events bind the actual pinned
    pg_dump ELF. At each outbound socket syscall, the client cannot send while
    the independent observer takes its fresh snapshot. PG18 ConnectDatabaseAhx
    issues SQL after ReadyForQuery; that send necessarily has a live backend.
    No protocol payload, password, client-reported PID or tracee memory is read.
    """
    SEIZE, INTERRUPT, DETACH, CONT, SYSCALL = 0x4206, 0x4207, 17, 7, 24
    GETEVENTMSG, GETINFO, WALL = 0x4201, 0x420E, 0x40000000
    OPTIONS = 1 | 2 | 4 | 8 | 16 | 64  # syscall/fork/vfork/clone/exec/exit
    SOCKET_CALLS = {1, 20, 44, 46, 307, 3, 48}  # x86_64 write[v]/send*/close/shutdown

    def __init__(self, client):
        _require(type(client) is _Client and client.dumping and not client.dump_backends
                 and client.dump_gate is None, "PAPER_BACKUP_DUMP_GATE")
        self.client, self.owner = client, client.owner
        self.rows, self.stopped, self.pending, self.signals = {}, set(), set(), {}
        self.group_stopped = {}
        self.syscall_stops = set()
        self.dump_pid = None
        self.closed = False
        self.libc = ctypes.CDLL(None, use_errno=True)
        self.libc.ptrace.restype = ctypes.c_long
        # Retain even a partially armed tracer. Disposal must settle this before
        # stopping/removing the container or releasing any original/lock.
        client.dump_gate = self
        _require(os.geteuid() == 0 and platform.machine() == "x86_64",
                 "PAPER_BACKUP_DUMP_GATE_PLATFORM")
        client.check()
        self.owner.revalidate()
        pid = client.process["pid"]
        _require({int(p.name) for p in (Path("/proc") / str(pid) / "task").iterdir()} == {pid},
                 "PAPER_BACKUP_DUMP_GATE_THREADS")
        self._ptrace(self.SEIZE, pid, self.OPTIONS)
        self.rows[pid] = dict(client.process)
        self._stop_all(False)
        self._identity(pid)
        self._resume(pid)

    def _ptrace(self, request, pid, data=0, address=0):
        ctypes.set_errno(0)
        result = self.libc.ptrace(ctypes.c_uint(request), ctypes.c_uint(pid),
                                 ctypes.c_void_p(address), ctypes.c_void_p(data))
        _require(result != -1, "PAPER_BACKUP_DUMP_GATE_KERNEL")
        return result

    def _identity(self, pid, *, executed=False):
        old = self.rows[pid]
        _require(type(old) is dict, "PAPER_BACKUP_DUMP_GATE_LOST")
        row = self.owner.ops._process_record(pid)
        _require(all(row[k] == old[k] for k in ("pid", "start_ticks", "parent", "cgroup")),
                 "PAPER_BACKUP_CLIENT_CHANGED")
        if executed:
            executable = row["executable"]
            _require(pid != self.client.process["pid"] and
                     (executable["bytes"], executable["sha256"]) in self.client.executables.values(),
                     "PAPER_BACKUP_UNOWNED_CHILD")
            self.rows[pid] = row
        else:
            _require(row == old, "PAPER_BACKUP_CLIENT_CHANGED")
        return row

    def _fork(self, parent):
        message = ctypes.c_ulong()
        self._ptrace(self.GETEVENTMSG, parent, ctypes.addressof(message))
        pid = message.value
        _require(pid > 1 and pid not in self.rows,
                 "PAPER_BACKUP_DUMP_GATE_CHILD")
        # The kernel reports this child while both parent and newborn are held.
        # Record custody BEFORE any subsequent validation can fail.
        self.pending.add(pid)
        self.rows[pid] = None
        _require(len(self.rows) <= 64, "PAPER_BACKUP_DUMP_GATE_CHILD")
        row = self.owner.ops._process_record(pid)
        self.rows[pid] = row
        _require(row["parent"] == parent and row["cgroup"] == self.client.process["cgroup"]
                 and row["executable"] == self.rows[parent]["executable"],
                 "PAPER_BACKUP_UNOWNED_CHILD")

    def _resume(self, pid):
        _require(pid not in self.group_stopped, "PAPER_BACKUP_DUMP_GATE_SIGNAL")
        self._ptrace(self.SYSCALL if pid == self.dump_pid else self.CONT,
                     pid, self.signals.get(pid, 0))
        self.signals.pop(pid, None)
        self.stopped.remove(pid)
        self.syscall_stops.discard(pid)

    def _event(self, pid, status, *, stopping=False):
        if os.WIFEXITED(status) or os.WIFSIGNALED(status):
            self.rows.pop(pid)
            self.pending.discard(pid)
            self.stopped.discard(pid)
            self.signals.pop(pid, None)
            self.group_stopped.pop(pid, None)
            self.syscall_stops.discard(pid)
            _require(stopping, "PAPER_BACKUP_DUMP_BACKEND_UNOBSERVED")
            return
        _require(os.WIFSTOPPED(status), "PAPER_BACKUP_DUMP_GATE_KERNEL")
        self.stopped.add(pid)
        self.pending.discard(pid)
        event, signum = status >> 16, os.WSTOPSIG(status)
        if event in (1, 2, 3):
            self._identity(pid)
            self._fork(pid)
        elif event == 4:
            row = self._identity(pid, executed=True)
            executable = row["executable"]
            if (executable["bytes"], executable["sha256"]) == self.client.executables[
                    "/usr/lib/postgresql/18/bin/pg_dump"]:
                _require(self.dump_pid is None, "PAPER_BACKUP_DUMP_GATE_CHILD")
                self.dump_pid = pid
        elif event == 6:  # pre-exit
            self._identity(pid)
        elif event == 128:
            # SEIZE reports a real group-stop with its stopping signal; our
            # INTERRUPT and an automatically attached newborn use SIGTRAP.
            # CONT or DETACH here can consume an external stop. Retain custody
            # and fail closed; signal injection is not proof it stayed stopped.
            if signum != signal.SIGTRAP:
                self.group_stopped[pid] = signum
            self._identity(pid)
            _require(signum == signal.SIGTRAP, "PAPER_BACKUP_DUMP_GATE_SIGNAL")
        elif event == 0 and signum == signal.SIGTRAP | 0x80:
            _require(pid == self.dump_pid, "PAPER_BACKUP_DUMP_GATE_KERNEL")
            self._identity(pid)
            self.syscall_stops.add(pid)
        elif event == 0:
            # Preserve real signal delivery, including timeout's TERM. Do not
            # swallow a job-control stop as if it were our INTERRUPT event.
            self._identity(pid)
            self.signals[pid] = signum
            _require(signum not in (signal.SIGSTOP, signal.SIGTSTP, signal.SIGTTIN, signal.SIGTTOU),
                     "PAPER_BACKUP_DUMP_GATE_SIGNAL")
        else:
            raise PaperBackupError("PAPER_BACKUP_DUMP_GATE_KERNEL")

    def _poll(self, *, stopping=False):
        for pid in tuple(self.rows):
            if pid in self.stopped:
                continue
            try:
                found, status = os.waitpid(pid, os.WNOHANG | self.WALL)
            except ChildProcessError:
                raise PaperBackupError("PAPER_BACKUP_DUMP_GATE_LOST") from None
            if found:
                _require(found == pid, "PAPER_BACKUP_DUMP_GATE_KERNEL")
                self._event(pid, status, stopping=stopping)
                return pid
        return None

    def _syscall(self, pid):
        buffer = ctypes.create_string_buffer(128)
        size = self._ptrace(self.GETINFO, pid, ctypes.addressof(buffer), 128)
        raw = buffer.raw[:size]
        _require(len(raw) >= 24 and struct.unpack_from("<I", raw, 4)[0] == 0xC000003E,
                 "PAPER_BACKUP_DUMP_GATE_ABI")
        if raw[0] == 2:  # syscall exit, NOT guessed entry/exit alternation
            _require(len(raw) >= 33, "PAPER_BACKUP_DUMP_GATE_ABI")
            return
        _require(raw[0] == 1 and len(raw) >= 80, "PAPER_BACKUP_DUMP_GATE_ABI")
        number, fd = struct.unpack_from("<QQ", raw, 24)
        if number in (60, 231):
            raise PaperBackupError("PAPER_BACKUP_DUMP_BACKEND_UNOBSERVED")
        # libpq's socket transport uses send/write; deny alternate async/zero-
        # copy close/transport surfaces until capture rather than assuming them.
        _require(number not in (40, 275, 425, 426, 427, 436), "PAPER_BACKUP_DUMP_GATE_ABI")
        if number in self.SOCKET_CALLS:
            self._socket_boundary(pid, fd, terminal=number in (3, 48))

    def _socket_boundary(self, pid, fd, *, terminal):
        row = self._identity(pid)
        try:
            target = os.readlink("/proc/%d/fd/%d" % (pid, fd))
        except FileNotFoundError:
            raise PaperBackupError("PAPER_BACKUP_DUMP_GATE_SOCKET") from None
        match = re.fullmatch(r"socket:\[([0-9]+)\]", target)
        if match is None:
            return
        before = self.owner.ops._tcp_endpoints(row)
        endpoints = [e for e in before
                     if e["socket_inode"] == int(match[1])]
        if not endpoints:  # DNS/unconnected startup socket is not DB evidence
            return
        _require(len(endpoints) == 1, "PAPER_BACKUP_DUMP_GATE_SOCKET")
        c = self.owner.context
        server = c.docker.object("container", c.database_generation[0])
        addresses = {n.get("IPAddress") for n in server["NetworkSettings"]["Networks"].values()}
        if endpoints[0]["peer"][0] not in addresses or endpoints[0]["peer"][1] != 5432:
            return
        # The full existing observer checks the exact container, source roots,
        # OFF originals, prepared transactions and ALL client backends. It alone
        # records the socket-correlated PID+backend_start; this tracer cannot.
        self._stop_all(False)
        self.owner.revalidate()
        self._identity(pid)
        _require(self.owner.ops._tcp_endpoints(row) == before and
                 os.readlink("/proc/%d/fd/%d" % (pid, fd)) == target,
                 "PAPER_BACKUP_DUMP_GATE_SOCKET")
        if self.client.dump_backends:
            _require(len(self.client.dump_backends) == 1 and
                     all(p == row and e == endpoints[0] for _b, p, e in self.client.dump_backends.values()),
                     "PAPER_BACKUP_CLIENT_CHANGED")
        else:
            _require(not terminal, "PAPER_BACKUP_DUMP_BACKEND_UNOBSERVED")

    def pump(self):
        if self.closed:
            return
        self.owner._custody.deadline.work_remaining()
        pid = self._poll()
        if pid is not None:
            self._release_or_capture()

    def _release_or_capture(self):
        if not self.client.dump_backends:
            for pid in tuple(self.syscall_stops):
                self._syscall(pid)
        if self.client.dump_backends:
            self._stop_all(False)
            self.owner.revalidate()  # still held, independent repeated identity
            self.close()
        else:
            for pid in tuple(self.stopped):
                self._resume(pid)

    def revalidate(self):
        # Freeze and consume fork/exec events before the ordinary one-second
        # guard enumerates children. No pre-exec child is admitted by timing.
        self._stop_all(False)
        self.owner.revalidate()
        self._release_or_capture()

    def _stop_all(self, cleanup):
        budget = (self.owner._custody.deadline.remaining if cleanup
                  else self.owner._custody.deadline.work_remaining)
        budget()
        requested = set()
        while set(self.rows) != self.stopped:
            budget()
            while self._poll(stopping=True) is not None:
                budget()
            for pid in set(self.rows) - self.stopped - self.pending - requested:
                self._ptrace(self.INTERRUPT, pid)
                requested.add(pid)
            if self._poll(stopping=True) is None:
                time.sleep(budget(0.001))

    def close(self):
        if self.closed:
            return
        # Original 920s clock only, including its reserved cleanup interval.
        # A failed detach retains this object and every unresolved kernel PID.
        _require(not self.group_stopped, "PAPER_BACKUP_DUMP_GATE_SIGNAL")
        self._stop_all(True)
        _require(not self.group_stopped, "PAPER_BACKUP_DUMP_GATE_SIGNAL")
        for pid in sorted(self.stopped, key=lambda p: p == self.client.process["pid"]):
            self.owner._custody.deadline.remaining()
            _require(self.rows[pid] is not None, "PAPER_BACKUP_DUMP_GATE_LOST")
            self._identity(pid)
            self._ptrace(self.DETACH, pid, self.signals.get(pid, 0))
            self.signals.pop(pid, None)
            self.stopped.remove(pid)
            self.rows.pop(pid)
            self.syscall_stops.discard(pid)
        self.closed = True


class _Client:
    """One source-created read process. Instances never come from a report."""
    def __init__(self, owner, kind):
        self.owner, self.kind = owner, kind
        self.name = "lagrange-paper-capture-" + uuid.uuid4().hex
        self.cid = self.created = self.generation = self.process = self.backend = None
        self.child = None
        self.sent = self.absent = self.ended = False
        self.command = None
        self.sockets = None
        self.attach = None
        self.executables, self.descendants, self.dump_backends = {}, {}, {}
        self.dumping = False
        self.dump_gate = None

    def policy(self, value):
        o, c = self.owner, self.owner.context
        config, host = value.get("Config") or {}, value.get("HostConfig") or {}
        labels = config.get("Labels") or {}
        _require(value.get("Id") == self.cid and value.get("Created") == self.created
                 and value.get("Image") == c.database_image_id
                 and config.get("Image") == c.database_image_id
                 and config.get("User") == "999:999"
                 and config.get("Entrypoint") == ["/usr/lib/postgresql/18/bin/psql"]
                 and config.get("Cmd") == self.command
                 and labels.get("io.lagrange.paper.capture") == self.name
                 and host.get("ReadonlyRootfs") is True and host.get("Privileged") is False
                 and host.get("RestartPolicy", {}).get("Name") == "no"
                 and host.get("NetworkMode") == c.network_id
                 and host.get("NanoCpus") == 1_000_000_000
                 and host.get("Memory") == 536_870_912
                 and host.get("CapDrop") == ["ALL"]
                 and host.get("SecurityOpt") == ["no-new-privileges"]
                 and not host.get("PortBindings") and not host.get("Binds")
                 and not host.get("Privileged") and not host.get("PidMode")
                 and host.get("LogConfig") == {"Type": "none", "Config": {}},
                 "PAPER_BACKUP_CLIENT_CHANGED")
        mounts = _covered_mounts(value, _VOLUME_COVER, "PAPER_BACKUP_CLIENT_CHANGED")
        _require(len(mounts) == 1 and type(mounts[0]) is dict and mounts[0].get("Type") == "bind"
                 and mounts[0].get("Source") == str(o.root / "pgpass")
                 and mounts[0].get("Destination") == "/run/pgpass"
                 and mounts[0].get("RW") is False, "PAPER_BACKUP_CLIENT_CHANGED")
        expected = {"PGPASSFILE": "/run/pgpass", "PGSERVICEFILE": "/dev/null",
                    "PGCONNECT_TIMEOUT": "10", "PGCLIENTENCODING": "UTF8",
                    "PGOPTIONS": "-c default_transaction_read_only=on -c statement_timeout=900000 -c lock_timeout=5000"}
        image = c.docker.object("image", c.database_image_id)
        base = o.ops.runtime.environment({"Config": image.get("Config") or {}})
        _require(o.ops.runtime.environment(value) == {**base, **expected}, "PAPER_BACKUP_CLIENT_CHANGED")
        networks = value.get("NetworkSettings", {}).get("Networks") or {}
        ids = {v.get("NetworkID") for v in networks.values()}
        if ids == {""} and (value.get("State") or {}).get("Status") == "created":
            # Docker allocates NetworkID at start. Until then the endpoint is bound by the
            # network's name, as for the native helper.
            network = c.docker.object("network", c.network_id)
            _require(network.get("Id") == c.network_id and set(networks) == {network.get("Name")},
                     "PAPER_BACKUP_CLIENT_CHANGED")
        else:
            _require(len(networks) == 1 and ids == {c.network_id}, "PAPER_BACKUP_CLIENT_CHANGED")

    def start(self):
        o, c = self.owner, self.owner.context
        self.command = ["-X", "-qAt", "-h", "postgres", "-U", "migration_owner", "-d",
                        c.database["database"], "-v", "ON_ERROR_STOP=1", "-v", "VERBOSITY=sqlstate"]
        self.sent = True  # unknown create is retained, never guessed by name
        value = o._docker(["create", "--interactive", "--name", self.name,
            "--label", "io.lagrange.paper.capture=" + self.name, "--network", c.network_id,
            "--restart", "no", "--read-only", "--user", "999:999", "--cap-drop", "ALL",
            "--security-opt", "no-new-privileges", "--cpus", "1", "--memory", "536870912", "--log-driver", "none",
            "--env", "PGPASSFILE=/run/pgpass", "--env", "PGSERVICEFILE=/dev/null",
            "--env", "PGCONNECT_TIMEOUT=10", "--env", "PGCLIENTENCODING=UTF8",
            "--env", "PGOPTIONS=-c default_transaction_read_only=on -c statement_timeout=900000 -c lock_timeout=5000",
            "--tmpfs", _IMAGE_VOLUME + ":" + _VOLUME_COVER,
            "--mount", "type=bind,source=" + str(o.root / "pgpass") + ",target=/run/pgpass,readonly",
            "--entrypoint", "/usr/lib/postgresql/18/bin/psql", c.database_image_id, *self.command]).strip()
        _require(re.fullmatch(r"[0-9a-f]{64}", value), "PAPER_BACKUP_CREATE_UNKNOWN")
        self.cid = value
        info = c.docker.object("container", self.cid)
        self.created = info.get("Created")
        self.policy(info)
        _require(info.get("State", {}).get("Status") == "created" and info.get("RestartCount") == 0,
                 "PAPER_BACKUP_CREATE_UNKNOWN")
        self.child = subprocess.Popen(o._argv(["start", "--attach", "--interactive", self.cid]),
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            env=o.ops._docker_client_environment(), close_fds=True)
        self.attach = o.ops._process_record(self.child.pid)
        until = time.monotonic() + o._custody.deadline.work_remaining(15)
        while True:
            info = c.docker.object("container", self.cid)
            self.policy(info)
            if info.get("State", {}).get("Status") in {"running", "exited"}:
                self.generation = o.ops._container_generation(info)
                break
            _require(time.monotonic() < until and self.child.poll() is None,
                     "PAPER_BACKUP_CLIENT_LOST")
            time.sleep(o._custody.deadline.work_remaining(0.05))
        # The only initial SQL is a real RR RO transaction and its own identity.
        raw = self.exchange(b"BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n"
            b"SET LOCAL statement_timeout='900s'; SET LOCAL lock_timeout='5s';\n"
            b"SELECT json_build_object('pid',pg_backend_pid(),'start_us',"
            b"(SELECT (extract(epoch FROM backend_start)*1000000)::bigint FROM pg_stat_activity "
            b"WHERE pid=pg_backend_pid()),'role',current_user,'session',session_user,"
            b"'readonly',current_setting('transaction_read_only'),'isolation',"
            b"current_setting('transaction_isolation'),'encoding',current_setting('client_encoding'),"
            b"'server_encoding',current_setting('server_encoding'),'set_role',current_setting('role'));\n")
        try:
            identity = json.loads(raw)
        except (ValueError, TypeError):
            raise PaperBackupError("PAPER_BACKUP_SESSION") from None
        _require(type(identity) is dict and set(identity) == {"pid", "start_us", "role", "session", "readonly",
                 "isolation", "encoding", "server_encoding", "set_role"}
                 and type(identity["pid"]) is int and identity["pid"] > 1
                 and type(identity["start_us"]) is int and identity["start_us"] > 0
                 and identity["role"] == identity["session"] == "migration_owner"
                 and identity["readonly"] == "on" and identity["isolation"] == "repeatable read"
                 and identity["encoding"] == identity["server_encoding"] == "UTF8"
                 and identity["set_role"] == "none", "PAPER_BACKUP_SESSION")
        info = c.docker.object("container", self.cid)
        self.policy(info)
        self.generation = o.ops._container_generation(info)
        self.process = o.ops._process_record(info["State"]["Pid"])
        _require(self.cid in self.process["cgroup"], "PAPER_BACKUP_CLIENT_CHANGED")
        self.sockets = o.ops._tcp_endpoints(self.process)
        server = c.docker.object("container", c.database_generation[0])
        ips = {n.get("IPAddress") for n in server["NetworkSettings"]["Networks"].values()}
        connections = [v for v in self.sockets if v["peer"][0] in ips and v["peer"][1] == 5432]
        _require(len(connections) == 1, "PAPER_BACKUP_SOCKET")
        self.backend = {"pid": identity["pid"], "start_us": identity["start_us"],
                        "role": "migration_owner", "addr": connections[0]["local"][0],
                        "port": connections[0]["local"][1]}
        # Real files from this immutable, independently pinned PG image. Never
        # execute its ELF on the host or infer identity from a binary name.
        for path in ("/bin/sh", "/usr/bin/timeout", "/usr/lib/postgresql/18/bin/pg_dump"):
            fd = os.open("/proc/%d/root%s" % (self.process["pid"], path), os.O_RDONLY | os.O_CLOEXEC)
            try:
                self.executables[path] = o.ops._hash_fd(fd)
            finally:
                os.close(fd)
        self.check()

    def child_processes(self):
        o = self.owner
        groups = o.ops._proc_cgroups()
        found = {}
        for pid, group in groups.items():
            if pid == self.process["pid"] or group != self.process["cgroup"]:
                continue
            row = o.ops._process_record(pid)
            gate = self.dump_gate
            inherited = (type(gate) is _DumpGate and not gate.closed and gate.rows.get(pid) == row
                         and row["executable"] == self.process["executable"])
            _require(self.dumping and (inherited or
                     (row["executable"]["bytes"], row["executable"]["sha256"]) in self.executables.values()),
                     "PAPER_BACKUP_UNOWNED_CHILD")
            prior = self.descendants.get(pid)
            _require(prior is None or prior["start_ticks"] == row["start_ticks"],
                     "PAPER_BACKUP_CLIENT_CHANGED")
            found[pid] = row
        for pid, row in found.items():
            chain, cursor = set(), row
            while cursor["parent"] != self.process["pid"]:
                _require(cursor["parent"] in found and cursor["parent"] not in chain,
                         "PAPER_BACKUP_UNOWNED_CHILD")
                chain.add(cursor["parent"]); cursor = found[cursor["parent"]]
        self.descendants.update(found)
        return found

    def check(self):
        o, c = self.owner, self.owner.context
        _require(not self.ended and not self.absent and self.child is not None
                 and self.child.poll() is None and self.generation is not None,
                 "PAPER_BACKUP_CLIENT_LOST")
        value = c.docker.object("container", self.cid)
        self.policy(value)
        _require(o.ops._container_generation(value) == self.generation
                 and value["State"].get("Running") is True
                 and o.ops._process_record(value["State"]["Pid"]) == self.process
                 and o.ops._process_record(self.child.pid) == self.attach
                 and o.ops._tcp_endpoints(self.process) == self.sockets,
                 "PAPER_BACKUP_CLIENT_CHANGED")

    def exchange(self, sql, destination=None, *, maximum=None):
        """Bounded simultaneous pipe write/drain; marker follows SQL completion."""
        o = self.owner
        _require(self.child is not None and self.child.poll() is None
                 and type(sql) is bytes and len(sql) <= 4 * 1024 * 1024,
                 "PAPER_BACKUP_CLIENT_LOST")
        marker = ("PAPER_CAPTURE_" + uuid.uuid4().hex).encode("ascii")
        payload = memoryview(sql + b"\n\\echo " + marker + b"\n")
        output = io.BytesIO() if destination is None else destination
        maximum = min(o.maximum, maximum if maximum is not None else (65_536 if destination is None else o.maximum))
        inp, out = self.child.stdin.fileno(), self.child.stdout.fileno()
        checked = time.monotonic()
        tail, total = b"", 0
        failure = None
        try:
            os.set_blocking(inp, False); os.set_blocking(out, False)
            gate = _DumpGate(self) if self.dumping else None
            while True:
                o._custody.deadline.work_remaining()
                if gate is not None:
                    gate.pump()
                reads, writes, _ = select.select([out], [inp] if payload else [], [],
                    o._custody.deadline.work_remaining(0.001 if gate is not None and not gate.closed else 1))
                if writes:
                    count = os.write(inp, payload[:65536]); payload = payload[count:]
                if reads:
                    part = os.read(out, 65536)
                    _require(part, "PAPER_BACKUP_CLIENT_LOST")
                    tail += part
                    # Keep only the possible final marker; audit data streams to
                    # the private spool, bounded by the configured byte limit.
                    suffix = b"\n" + marker + b"\n"
                    if tail == marker + b"\n" or tail.endswith(suffix):
                        data = tail[:-len(marker)-1]
                        _require(not payload, "PAPER_BACKUP_PROTOCOL")
                        _require(gate is None or (gate.closed and self.dump_backends),
                                 "PAPER_BACKUP_DUMP_BACKEND_UNOBSERVED")
                        total += len(data)
                        _require(total <= maximum, "PAPER_BACKUP_LIMIT")
                        output.write(data)
                        return output.getvalue() if destination is None else total
                    if len(tail) > len(suffix):
                        data, tail = tail[:-len(suffix)], tail[-len(suffix):]
                        total += len(data)
                        _require(total <= maximum, "PAPER_BACKUP_LIMIT")
                        output.write(data)
                _require(self.child.poll() is None, "PAPER_BACKUP_CLIENT_LOST")
                if self.backend is not None and time.monotonic() - checked >= 1:
                    if gate is not None and not gate.closed:
                        gate.revalidate()
                    else:
                        o.revalidate()
                    checked = time.monotonic()
        except BaseException as error:
            failure = error
            raise
        finally:
            cleanup_error = None
            for fd in (inp, out):
                try:
                    os.set_blocking(fd, True)
                except BaseException as error:
                    if cleanup_error is None:
                        cleanup_error = error
            try:
                if self.dump_gate is not None:
                    self.dump_gate.close()
                    self.dump_gate = None
            except BaseException as error:
                if cleanup_error is None:
                    cleanup_error = error
            if failure is None and cleanup_error is not None:
                raise cleanup_error

    def close(self):
        if self.absent:
            return
        o = self.owner
        if self.dump_gate is not None:
            self.dump_gate.close()
            self.dump_gate = None
        if not self.sent:
            self.absent = True
            return
        _require(self.cid is not None and self.created is not None, "PAPER_BACKUP_CLEANUP_UNKNOWN")
        value = o.context.docker.object("container", self.cid)
        self.policy(value)
        if self.generation is None:
            _require(value["State"].get("Status") == "created", "PAPER_BACKUP_CLEANUP_UNKNOWN")
        else:
            _require(o.ops._container_generation(value) == self.generation, "PAPER_BACKUP_CLEANUP_UNKNOWN")
            if value["State"].get("Running"):
                # Stop only this captured generation; never signal a role/PID set.
                o._cleanup_docker(["stop", "--time", "5", self.cid])
            first = o.ops._terminal_container(o.context.docker.object("container", self.cid), self.generation)
            second = o.ops._terminal_container(o.context.docker.object("container", self.cid), self.generation)
            _require(first == second and not any(self.cid in group for group in o.ops._proc_cgroups().values()),
                     "PAPER_BACKUP_CLEANUP_UNKNOWN")
        if self.child is not None:
            self.child.wait(timeout=o._custody.deadline.remaining(5))
            for stream in (self.child.stdin, self.child.stdout):
                stream.close()
        # The privileged source observer proves absence independently of psql.
        rows = o._custody.observer.database("observe")["backends"]
        if self.backend is not None:
            _require(not any((r["pid"], r["start_us"]) == (self.backend["pid"], self.backend["start_us"])
                             for r in rows), "PAPER_BACKUP_CLEANUP_UNKNOWN")
        _require(all((r["pid"], r["start_us"]) not in
                     {(v[0]["pid"], v[0]["start_us"]) for v in self.dump_backends.values()}
                     for r in rows), "PAPER_BACKUP_CLEANUP_UNKNOWN")
        self.ended = True
        o._cleanup_docker(["rm", self.cid])
        _require(o._cleanup_docker(["ps", "--all", "--no-trunc", "--filter", "id=" + self.cid,
                             "--format", "{{.ID}}"]).strip() == "", "PAPER_BACKUP_CLEANUP_UNKNOWN")
        self.absent = True


class _GuestClient(_Client):
    """Owned local-socket reader of the isolated clone, never the source DB."""
    def __init__(self, package, role):
        _require(type(package) is _ArchivePackage and package in _PACKAGES
                 and type(role) is str and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,62}", role),
                 "PAPER_BACKUP_PRIVATE")
        super().__init__(package.owner, "guest")
        self.package, self.role = package, role
        self.backend_process = None
        self.namespace_pids = None
        self.terminal = None
        self.environment = {"PGPASSFILE": "/dev/null", "PGSERVICEFILE": "/dev/null",
            "PGCONNECT_TIMEOUT": "10", "PGCLIENTENCODING": "UTF8",
            "PGOPTIONS": "-c default_transaction_read_only=on -c statement_timeout=900000 -c lock_timeout=5000"}
        package.readers.append(self)

    def policy(self, value):
        guest = self.package.guest
        guest.check(ready=True)
        config, host = value.get("Config") or {}, value.get("HostConfig") or {}
        _require(value.get("Id") == self.cid and value.get("Created") == self.created
                 and value.get("Image") == guest.image and config.get("Image") == guest.image
                 and config.get("User") == "999:999"
                 and config.get("Entrypoint") == ["/usr/lib/postgresql/18/bin/psql"]
                 and config.get("Cmd") == self.command
                 and config.get("Labels", {}).get("io.lagrange.paper.capture") == self.name
                 and host.get("NetworkMode") == "none" and host.get("ReadonlyRootfs") is True
                 and host.get("Privileged") is False and not host.get("CapAdd")
                 and host.get("CapDrop") == ["ALL"] and not host.get("GroupAdd")
                 and host.get("RestartPolicy", {}).get("Name") == "no"
                 and host.get("NanoCpus") == 1_000_000_000 and host.get("Memory") == 536_870_912
                 and host.get("SecurityOpt") in (["no-new-privileges"], ["no-new-privileges:true"])
                 and host.get("LogConfig", {}).get("Type") == "none"
                 and not host.get("PortBindings") and not host.get("Binds")
                 and not host.get("Devices") and not host.get("DeviceRequests")
                 and not host.get("VolumesFrom")
                 and not host.get("AutoRemove") and not host.get("PublishAllPorts")
                 and not host.get("Links") and host.get("PidMode") in ("", "private")
                 and host.get("IpcMode") in ("", "private") and value.get("RestartCount") == 0
                 and not value.get("State", {}).get("OOMKilled"), "PAPER_BACKUP_GUEST_CLIENT_CHANGED")
        self.owner.ops._validate_mount_records(
            _covered_mounts(value, _VOLUME_COVER, "PAPER_BACKUP_GUEST_CLIENT_CHANGED"),
            ((str(guest.socket), "/run/postgresql", True),))
        networks = value.get("NetworkSettings", {}).get("Networks") or {}
        _require(set(networks) <= {"none"} and all(not n.get("IPAddress") for n in networks.values()),
                 "PAPER_BACKUP_GUEST_CLIENT_CHANGED")
        image = self.owner.context.docker.object("image", guest.image)
        base = self.owner.ops.runtime.environment({"Config": image.get("Config") or {}})
        _require(self.owner.ops.runtime.environment(value) == {**base, **self.environment},
                 "PAPER_BACKUP_GUEST_CLIENT_CHANGED")

    def start(self):
        o, guest = self.owner, self.package.guest
        self.command = ["-X", "-qAt", "-h", "/run/postgresql", "-U", self.role, "-d",
                        self.package.database["database"], "-v", "ON_ERROR_STOP=1", "-v", "VERBOSITY=sqlstate"]
        self.sent = True
        args = ["create", "--interactive", "--name", self.name,
            "--label", "io.lagrange.paper.capture=" + self.name, "--network", "none",
            "--restart", "no", "--read-only", "--user", "999:999", "--cap-drop", "ALL",
            "--security-opt", "no-new-privileges", "--cpus", "1", "--memory", "536870912", "--log-driver", "none"]
        for key, value in self.environment.items():
            args.extend(("--env", key + "=" + value))
        args.extend(("--tmpfs", _IMAGE_VOLUME + ":" + _VOLUME_COVER,
                     "--mount", "type=bind,source=" + str(guest.socket) + ",target=/run/postgresql,readonly",
                     "--entrypoint", "/usr/lib/postgresql/18/bin/psql", guest.image, *self.command))
        cid = o._docker(args).strip()
        _require(re.fullmatch(r"[0-9a-f]{64}", cid), "PAPER_BACKUP_CREATE_UNKNOWN")
        self.cid = cid
        value = o.context.docker.object("container", cid)
        self.created = value.get("Created")
        self.policy(value)
        _require(value.get("State", {}).get("Status") == "created", "PAPER_BACKUP_CREATE_UNKNOWN")
        self.child = subprocess.Popen(o._argv(["start", "--attach", "--interactive", cid]),
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            env=o.ops._docker_client_environment(), close_fds=True)
        self.attach = o.ops._process_record(self.child.pid)
        until = time.monotonic() + o._custody.deadline.work_remaining(15)
        while True:
            value = o.context.docker.object("container", cid)
            self.policy(value)
            if value.get("State", {}).get("Running") is True:
                self.generation = o.ops._container_generation(value)
                self.process = o.ops._process_record(value["State"]["Pid"])
                _require(cid in self.process["cgroup"], "PAPER_BACKUP_GUEST_CLIENT_CHANGED")
                break
            _require(time.monotonic() < until and self.child.poll() is None, "PAPER_BACKUP_CLIENT_LOST")
            time.sleep(o._custody.deadline.work_remaining(0.05))
        raw = self.exchange(b"BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n"
            b"SET LOCAL search_path='pg_catalog, pg_temp';\n"
            b"SELECT json_build_object('pid',pg_backend_pid(),'start_us',"
            b"(SELECT (extract(epoch FROM backend_start)*1000000)::bigint FROM pg_stat_activity WHERE pid=pg_backend_pid()),"
            b"'role',current_user,'session',session_user,'readonly',current_setting('transaction_read_only'),"
            b"'isolation',current_setting('transaction_isolation'),'set_role',current_setting('role'));\n")
        try:
            identity = json.loads(raw)
        except (ValueError, UnicodeError):
            raise PaperBackupError("PAPER_BACKUP_SESSION") from None
        _require(type(identity) is dict and set(identity) == {"pid","start_us","role","session","readonly","isolation","set_role"}
                 and identity["role"] == identity["session"] == self.role and identity["readonly"] == "on"
                 and identity["isolation"] == "repeatable read" and identity["set_role"] == "none"
                 and type(identity["pid"]) is int and identity["pid"] > 1
                 and type(identity["start_us"]) is int and identity["start_us"] > 0, "PAPER_BACKUP_SESSION")
        matches = []
        for pid, group in o.ops._proc_cgroups().items():
            if group != guest.process["cgroup"]:
                continue
            try:
                status = (Path("/proc") / str(pid) / "status").read_text()
                line = next(line for line in status.splitlines() if line.startswith("NSpid:"))
                namespace_pids=[int(value) for value in line.split()[1:]]
                if namespace_pids[-1] == identity["pid"]:
                    record=o.ops._process_record(pid)
                    _require(namespace_pids[0]==record["pid"],"PAPER_BACKUP_GUEST_BACKEND")
                    matches.append((record,namespace_pids))
            except FileNotFoundError:
                continue
        _require(len(matches) == 1 and matches[0][0]["executable"] == guest.process["executable"],
                 "PAPER_BACKUP_GUEST_BACKEND")
        self.backend, self.backend_process, self.namespace_pids = identity, matches[0][0], matches[0][1]
        self.check()

    def check(self):
        o = self.owner
        _require(not self.absent and not self.ended and self.child is not None
                 and self.child.poll() is None and self.generation is not None,
                 "PAPER_BACKUP_CLIENT_LOST")
        value = o.context.docker.object("container", self.cid)
        self.policy(value)
        _require(o.ops._container_generation(value) == self.generation
                 and value["State"].get("Running") is True
                 and o.ops._process_record(value["State"]["Pid"]) == self.process
                 and o.ops._process_record(self.child.pid) == self.attach,
                 "PAPER_BACKUP_GUEST_CLIENT_CHANGED")
        if self.backend_process is not None:
            _require(o.ops._process_record(self.backend_process["pid"]) == self.backend_process,
                     "PAPER_BACKUP_GUEST_BACKEND")
        self.child_processes()  # no shell or descendant can be silently admitted

    def close(self):
        if self.absent:
            return
        o = self.owner
        if not self.sent:
            self.absent = True
            return
        _require(self.cid is not None and self.created is not None, "PAPER_BACKUP_CLEANUP_UNKNOWN")
        value = o.context.docker.object("container", self.cid)
        self.policy(value)
        if self.generation is None:
            _require(value.get("State", {}).get("Status") == "created", "PAPER_BACKUP_CLEANUP_UNKNOWN")
        else:
            _require(o.ops._container_generation(value) == self.generation, "PAPER_BACKUP_CLEANUP_UNKNOWN")
            if value["State"].get("Running"):
                o._cleanup_docker(["stop", "--time", "5", self.cid])
            first = o.ops._terminal_container(o.context.docker.object("container", self.cid), self.generation)
            second = o.ops._terminal_container(o.context.docker.object("container", self.cid), self.generation)
            _require(first == second and not any(self.cid in group for group in o.ops._proc_cgroups().values()),
                     "PAPER_BACKUP_CLEANUP_UNKNOWN")
        if self.child is not None:
            self.child.wait(timeout=o._custody.deadline.remaining(5))
            for stream in (self.child.stdin, self.child.stdout):
                stream.close()
        if self.backend_process is not None:
            until = time.monotonic() + o._custody.deadline.remaining(5)
            while True:
                try:
                    ticks, _ = o.ops._process_generation(self.backend_process["pid"])
                except FileNotFoundError:
                    ticks = None
                    break
                if ticks != self.backend_process["start_ticks"]:
                    break
                _require(time.monotonic() < until, "PAPER_BACKUP_CLEANUP_UNKNOWN")
                time.sleep(o._custody.deadline.remaining(0.05))
        if self.backend_process is not None:
            self.terminal = {"pid": self.backend_process["pid"], "start_ticks": self.backend_process["start_ticks"],
                             "observed_start_ticks": ticks}
        o._cleanup_docker(["rm", self.cid])
        _require(not o._cleanup_docker(["ps", "--all", "--no-trunc", "--filter", "id=" + self.cid,
                                       "--format", "{{.ID}}"]).strip(), "PAPER_BACKUP_CLEANUP_UNKNOWN")
        self.ended = self.absent = True


class _DestinationRestore:
    """Fresh clone-local R/A/E/Q custody. Source OFF is never a destination action."""
    def __init__(self, package):
        _require(type(package) is _ArchivePackage and package in _PACKAGES
                 and package.destination is None and package.call is None, "PAPER_BACKUP_PRIVATE")
        self.package, self.owner, self.ops = package, package.owner, package.ops
        self.path = self.package.work / "destination"
        self.tree = self.manager = self.store = self.operation = None
        self.lock_fd = None
        self.actions, self.imports, self.stages, self.receipts = [], [], [], {}
        self.active_custody = None
        self.head = self.begin_event = self.summary = None
        self.closed = False
        package.destination = self

    def revalidate(self):
        _require(not self.closed and self.package.destination is self and self.package in _PACKAGES,
                 "PAPER_BACKUP_PRIVATE")
        self.package.revalidate()
        if self.tree is not None:
            self.tree.revalidate()
        if self.store is not None:
            self.store.revalidate()
        if self.operation is not None:
            self.operation.revalidate()
        for stage in self.stages:
            stage.revalidate()

    def retain(self, value):
        raw = self.ops._canonical(value)
        digest = self.ops._digest(raw)
        self.operation._store_original(raw, digest, self.ops.journal.MAX_SMALL)
        return digest

    def stage(self, name, artifact=None, event=None, epoch=None):
        root = self.operation._intent
        value = dict(schema="owner-beta-paper-operation-stage-v1",
            installation_id=root["installation_id"], generation=root["generation"], account_id=root["account_id"],
            sequence=0 if self.head is None else self.head[0]+1,
            previous_sha256=None if self.head is None else self.head[1], intent_sha256=self.operation.intent_sha256,
            stage=name, boot_id=self.ops._current_boot_id(), observed=self.ops._stage_observation(),
            control_epoch=root["expected_epoch"] if epoch is None else epoch,
            artifact_sha256=artifact, event_id=event, error_code="NONE")
        digest = self.operation.append(self.ops._canonical(value))
        self.head = (value["sequence"], digest, value)

    def start(self):
        self.revalidate()
        os.mkdir(self.path, 0o700)
        self.tree = self.ops.journal._Tree(str(self.path))
        self.lock_fd = os.open(self.ops.journal.LOCK_NAME,
            os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=self.tree.fd)
        self.manager = self.ops.journal.open_journal(str(self.path), self.lock_fd, bootstrap=True)
        self.store = self.manager.__enter__()
        archive = next(item for item in self.package.originals if item.path == str(self.package.path / "archive.json"))
        archive.revalidate(None)
        raw = os.pread(archive.fd, archive.size+1, 0)
        self.archive = self.ops.journal._json(raw, self.ops.journal.MAX_SMALL)
        self.archive_sha256 = self.ops._digest(raw)
        source_control = self.archive["control_epoch"]
        self.store.advance_floor(max(source_control,self.owner._custody.floor))
        generation = str(uuid.uuid4())
        root = dict(schema="owner-beta-paper-operation-intent-v1", installation_id=self.store.installation_id,
            generation=generation, account_id=self.archive["account_id"],
            predecessor_generation=self.archive["generation"], request_key=generation+":begin",
            purpose="RESTORE", scope="COMBINED_BACKUP", expected_epoch=source_control,
            local_epoch_floor=self.store.epoch_floor, release_manifest_sha256=self.owner.context.manifest_sha256,
            artifact_sha256=self.archive_sha256, resume_target_id=None)
        _require(root["installation_id"] != self.archive["installation_id"], "PAPER_BACKUP_DESTINATION_IDENTITY")
        self.operation = self.store.bind_caller("restore:"+generation, self.ops._canonical(root))
        self.operation._store_original(raw, self.archive_sha256, self.ops.journal.MAX_SMALL)
        self.fence = max(source_control, self.store.epoch_floor)+1
        self.stage("INTENT")
        result = self.invoke("begin")
        self.begin_event = result["data"]["operation_event_id"]
        self.stage("FENCED", self.retain(result), self.begin_event, self.fence)
        self.store.advance_floor(self.fence)
        self.revalidate()

    def invoke(self, verb, artifact=None):
        self.revalidate()
        root = self.operation._intent
        sequence = len(self.actions)
        raw = None if artifact is None else self.ops._canonical(artifact)
        action = dict(schema="owner-beta-paper-action-binding-v1",installation_id=root["installation_id"],
            generation=root["generation"],root_intent_sha256=self.operation.intent_sha256,sequence=sequence,
            previous_binding_sha256=None if not self.actions else self.ops._digest(self.actions[-1]._binding_bytes),
            verb=verb,request_key=root["generation"]+(":reconcile:"+str(sequence) if verb == "reconcile" else ":"+verb),
            expected_epoch=root["expected_epoch"] if verb == "begin" else self.fence,
            artifact_sha256=None if raw is None else self.ops._digest(raw), stage_sha256=self.head[1])
        custody = self.operation.prepare_action(self.ops._canonical(action), raw)
        self.actions.append(custody)
        self.active_custody = custody
        call = self.owner.context._invoke_restore(self, verb, custody)
        self.package.call = call
        try:
            with call:
                imported = call.import_dispatch()
                imported.close()
        finally:
            if call._closed:
                self.package.call = None
        # Read exactly this durable Q on an independent connection. Never retry
        # an uncertain mutation or turn ACK loss into a new action/request key.
        historical = self.store.open_custody(root["generation"], sequence)
        self.imports.append(historical)
        self.active_custody = historical
        readback = self.owner.context._invoke_restore(self, "readback", historical)
        self.package.call = readback
        try:
            with readback:
                response = readback.result
                _require(response["outcome"] == "REPLAY" and response["error_code"] == "NONE",
                         "PAPER_BACKUP_RESTORE_UNKNOWN")
                value = self.ops.journal._json(readback.readback_artifact(), self.ops.journal.MAX_NATIVE_EXACT)
                expected_epoch = self.fence + int(verb == "restore-off")
                _require(value["status"] == "COMMITTED" and value["request_sha256"] == self.ops._digest(historical._sql)
                         and value["account_id"] == root["account_id"] and value["current_epoch"] == expected_epoch
                         and value["event_epoch"] == expected_epoch
                         and value["current_state"] == ("RESTORED_OFF" if verb == "restore-off" else "DRAINING")
                         and value["result_sha256"] == self.ops._digest(value["result_canonical"].encode("ascii")),
                         "PAPER_BACKUP_RESTORE_UNKNOWN")
                result = self.ops.journal._json(value["result_canonical"].encode("ascii"), self.ops.journal.MAX_SMALL)
                _require(result["outcome"] == "OK" and result["control_epoch"] == expected_epoch
                         and result["account_id"] == root["account_id"], "PAPER_BACKUP_RESTORE_UNKNOWN")
                if verb == "begin":
                    _require(result["action"] == "BEGIN_OPERATION"
                             and result["data"]["generation"] == root["generation"]
                             and result["data"]["intent_sha256"] == self.operation.intent_sha256
                             and result["data"]["operation_event_id"] == value["event_id"], "PAPER_BACKUP_RESTORE_UNKNOWN")
                else:
                    event, _summary = self.ops._maintenance_result(value["result_canonical"].encode("ascii"),
                        verb, self.operation, raw, self.fence)
                    _require(event == value["event_id"], "PAPER_BACKUP_RESTORE_UNKNOWN")
                self.receipts[verb] = value
        finally:
            if readback._closed:
                self.package.call = None
            historical.close()
        self.active_custody = None
        self.revalidate()
        return result

    def stop_and_restore_off(self):
        self.revalidate()
        first = self.package.guest.observe_fence(self)
        second = self.package.guest.observe_fence(self)
        proof = dict(schema="owner-beta-paper-guest-stopped-v1",first=first,second=second)
        proof_hash = self.retain(proof)
        backend_hash = self.retain(dict(first=first["database"],second=second["database"]))
        root = self.operation._intent
        rows = []
        for writer_class in ("API","PAPER_RUNNER","PAPER_DELIVERY","SOURCE_PUBLISHER","SOURCE_CHILD",
                             "DAILY","ADMITTER","RECOMMENDATION","SIMULATION","RAW_CURATED","OTHER_DB_WRITER"):
            identity = self.retain(dict(schema="owner-beta-paper-guest-class-v1",writer_class=writer_class,
                installation_id=root["installation_id"],generation=root["generation"],guest_sha256=proof_hash))
            rows.append(dict(writer_class=writer_class,kind="PROCESS",identity_sha256=identity,
                observation_sha256=proof_hash,backend_set_sha256=backend_hash,terminal="ABSENT_AT_FENCE"))
        inventory = dict(schema="owner-beta-paper-writer-inventory-v1",installation_id=root["installation_id"],
            generation=root["generation"],account_id=root["account_id"],scope=root["scope"],control_epoch=self.fence,
            fence_event_id=self.begin_event,release_manifest_sha256=root["release_manifest_sha256"],
            boot_id=self.ops._current_boot_id(),observed=self.ops._stage_observation(),
            launch_fence_sha256=proof_hash,backend_observation_sha256=backend_hash,
            writers=sorted(rows,key=lambda row:(row["identity_sha256"],row["writer_class"],row["kind"])))
        self.stage("STOPPED",self.retain(inventory),self.begin_event,self.fence)
        result = self.invoke("reconcile",inventory)
        self.summary = result["data"]["summary"]
        self.stage("RECONCILED",self.retain(self.summary),self.summary["observation_id"],self.fence)
        # BEGIN independently verified the archive before dispatch. Bind its
        # original archive object, as in the adopted frozen trace, with no SQL event.
        self.stage("RESTORE_VERIFIED",self.archive_sha256,None,self.fence)
        receipt = dict(schema="owner-beta-paper-restore-receipt-v1",installation_id=root["installation_id"],
            generation=root["generation"],account_id=root["account_id"],request_key=root["generation"]+":restore-off",
            scope=root["scope"],expected_epoch=root["expected_epoch"],resulting_epoch=self.fence,
            intent_sha256=self.operation.intent_sha256,stage_sha256=self.head[1],
            inventory_sha256=self.summary["inventory_sha256"],reconciliation_sha256=self.ops._digest(self.ops._canonical(self.summary)),
            observation_id=self.summary["observation_id"],release_manifest_sha256=root["release_manifest_sha256"],
            boot_id=self.ops._current_boot_id(),observed=self.ops._stage_observation(),
            source_installation_id=self.archive["installation_id"],source_generation=self.archive["generation"],
            source_epoch=self.archive["control_epoch"]-1,archive_sha256=self.archive_sha256,
            source_audit_sha256=self.archive["audit_sha256"],restored_audit_sha256=self.archive["audit_sha256"],
            destination_epoch_floor=root["local_epoch_floor"])
        result = self.invoke("restore-off",receipt)
        self.stage("RESTORED_COMMITTED",self.retain(result),result["data"]["event_id"],self.fence+1)
        self.store.advance_floor(self.fence+1)
        self.revalidate()

    def compare_post_restore(self):
        self.revalidate()
        audit = self.owner.audit_module
        reader = _GuestClient(self.package, "migration_owner")
        after_path = self.package.work / "after.audit"
        try:
            reader.start()
            with open(after_path, "x+b", buffering=0) as after:
                sql = self.owner.sources[0]
                sql.revalidate(self.owner.selection._backup_module().files)
                reader.exchange(os.pread(sql.fd, sql.size+1, 0), after)
                after.seek(0)
                audit.decode(after)
                source_dataset_original = next(item for item in self.package.originals
                    if item.path == str(self.package.path / "dataset-rows.jsonl"))
                source_dataset_original.revalidate(None)
                source_dataset_rows = [json.loads(line) for line in os.pread(source_dataset_original.fd,
                    source_dataset_original.size+1,0).splitlines()]
                after_dataset_rows = []
                for row in source_dataset_rows:
                    identity = row["id"]
                    query = ("SELECT jsonb_build_object('id',id::text,'dataset_id',dataset_id,'version',version,"
                        "'status',status,'manifest_sha256',manifest_sha256,'storage_path',storage_path)::text "
                        "FROM public.dataset_versions WHERE id=" + _sql_literal(identity) + "::uuid;\n").encode("ascii")
                    raw = reader.exchange(query, maximum=2*1024*1024)
                    _require(raw.endswith(b"\n") and raw.count(b"\n") == 1, "PAPER_BACKUP_DATASET_ROW")
                    try:
                        value = json.loads(raw)
                    except (ValueError, UnicodeError):
                        raise PaperBackupError("PAPER_BACKUP_DATASET_ROW") from None
                    _require(type(value) is dict and value.get("id") == identity, "PAPER_BACKUP_DATASET_ROW")
                    after_dataset_rows.append(value)
                after.flush(); os.fchmod(after.fileno(), 0o400); os.fsync(after.fileno())
                self.package.originals.append(_ArchiveOriginal(self.package, after_path))
                allowed = {"owner_beta_paper_control", "owner_beta_paper_events", "owner_beta_paper_operations",
                           "owner_beta_paper_operation_observations", "owner_beta_paper_recovery_grants"}
                def selected_rows(stream):
                    selected = {}
                    stream.seek(0)
                    for line in stream:
                        record = audit.parse(line[:-1])
                        if len(record) == 5 and record[0] == "row" and record[2] in allowed:
                            key = (record[2], record[3])
                            _require(key not in selected or selected[key] == record[4], "PAPER_BACKUP_AUDIT_ROW")
                            selected[key] = record[4]
                    return selected
                with open(self.package.path / "audit.stream", "rb") as source:
                    before = selected_rows(source)
                current = selected_rows(after)
                changes = {}
                for table, key in sorted(before.keys() | current.keys()):
                    old, new = before.get((table,key)), current.get((table,key))
                    if old != new:
                        changes.setdefault(table,{})[key] = dict(before=old,after=new)
                root = self.operation._intent
                expected = dict(generation=root["generation"],installation_id=root["installation_id"],
                    account_id=root["account_id"],source_generation=self.archive["generation"],
                    source_installation_id=self.archive["installation_id"],source_epoch=self.archive["control_epoch"]-1,
                    source_control_epoch=self.archive["control_epoch"],expected_epoch=root["expected_epoch"],
                    destination_epoch_floor=root["local_epoch_floor"],fence_epoch=self.fence,restore_epoch=self.fence+1,
                    archive_sha256=self.archive_sha256,source_audit_sha256=self.archive["audit_sha256"],
                    receipts=self.receipts,rows=changes)
                after.seek(0)
                with open(self.package.path / "audit.stream", "rb") as source:
                    audit.compare_restore_delta(source,after,expected,
                        source_dataset_rows=source_dataset_rows,after_dataset_rows=after_dataset_rows)
        finally:
            reader.close()
        self.revalidate()

    def verify_role_visibility(self):
        """Real local logins and SELECTs, using the audited rows as expectations.

        These expectations are the 0010 tenant policies, 0067 worker beta-target
        exclusion, and 0068 private-operation ACLs. No SET ROLE or role-name
        exception substitutes for a direct session or an observed denial.
        """
        self.revalidate()
        accounts, targets = {}, {}
        with open(self.package.work / "after.audit", "rb") as stream:
            self.owner.audit_module.decode(stream)
            stream.seek(0)
            for line in stream:
                row = self.owner.audit_module.parse(line[:-1])
                if len(row) == 5 and row[0] == "row" and row[2] in {"accounts", "pending_targets"}:
                    selected = accounts if row[2] == "accounts" else targets
                    _require(row[3] not in selected or selected[row[3]] == row[4], "PAPER_BACKUP_AUDIT_ROW")
                    selected[row[3]] = row[4]
        actors = sorted({row["owner_user_id"] for row in accounts.values()})
        _require(actors and len(actors) <= 10000, "PAPER_BACKUP_RLS_INVENTORY")
        stranger = str(uuid.uuid4())
        _require(stranger not in actors, "PAPER_BACKUP_RLS_INVENTORY")
        actors.append(stranger)
        def ids(reader, table):
            query = ("SELECT COALESCE(jsonb_agg(id::text ORDER BY id::text),'[]'::jsonb)::text "
                     "FROM public." + _sql_name(table) + ";\n").encode("ascii")
            raw = reader.exchange(query, maximum=2*1024*1024)
            try:
                value = json.loads(raw)
            except (ValueError, UnicodeError):
                raise PaperBackupError("PAPER_BACKUP_RLS_ROWS") from None
            _require(type(value) is list and all(type(v) is str for v in value), "PAPER_BACKUP_RLS_ROWS")
            return value
        def denied(reader, table):
            # psql exposes SQLSTATE, while the actual server error text remains
            # discarded. Roll back the failed SELECT inside the same RO snapshot.
            query = ("SAVEPOINT role_probe;\n\\set ON_ERROR_STOP off\nSELECT 1 FROM public."
                     + _sql_name(table) + ";\n\\echo :SQLSTATE\n\\set ON_ERROR_STOP on\n"
                     "ROLLBACK TO SAVEPOINT role_probe;\nRELEASE SAVEPOINT role_probe;\n").encode("ascii")
            _require(reader.exchange(query, maximum=2*1024*1024) == b"42501\n", "PAPER_BACKUP_RLS_DENIAL")
        for role in ("migration_owner", "app", "worker", "admin", "audit_writer", "research_writer"):
            reader = _GuestClient(self.package, role)
            try:
                reader.start()
                observed = json.loads(reader.exchange(b"SELECT jsonb_build_object('role',current_user,'session',session_user,"
                    b"'super',rolsuper,'bypass',rolbypassrls,'login',rolcanlogin)::text "
                    b"FROM pg_roles WHERE rolname=current_user;\n"))
                _require(observed == dict(role=role, session=role, super=False, bypass=False, login=True),
                         "PAPER_BACKUP_RLS_ROLE")
                if role in {"audit_writer", "research_writer"}:
                    denied(reader, "accounts")
                else:
                    for actor in actors:
                        reader.exchange(("SELECT set_config('app.actor_user_id',"+_sql_literal(actor)+",true);\n").encode("ascii"))
                        expected = sorted(row["id"] for row in accounts.values()
                            if role in {"worker", "admin"} or row["owner_user_id"] == actor)
                        _require(ids(reader,"accounts") == expected, "PAPER_BACKUP_RLS_ROWS")
                if role != "migration_owner":
                    denied(reader, "owner_beta_paper_operations")
                if role == "worker":
                    expected = sorted(row["id"] for row in targets.values() if row["beta_lineage_id"] is None)
                    _require(ids(reader,"pending_targets") == expected, "PAPER_BACKUP_RLS_ROWS")
                    denied(reader,"owner_beta_paper_control")
            finally:
                reader.close()
        self.revalidate()

    def close(self):
        if self.closed:
            return
        _require(self.package.call is None, "PAPER_BACKUP_CLEANUP_UNKNOWN")
        for custody in self.imports:
            custody.close()
        for action in self.actions:
            action.close()
        for stage in reversed(self.stages):
            stage.close(remove=True)
        if self.operation is not None:
            self.operation.close()
        if self.manager is not None:
            self.manager.__exit__(None,None,None)
        if self.lock_fd is not None:
            os.close(self.lock_fd)
        if self.tree is not None:
            self.tree.close()
        self.closed = True


class _HeldPaperSnapshot:
    def __init__(self, token, context, drained, maximum):
        _require(token is _HELD_PAPER_SNAPSHOT_TOKEN
                 and sys.modules.get("owner_beta_paper_backup") is sys.modules.get(__name__),
                 "PAPER_BACKUP_PRIVATE")
        self._token, self.context, self.drained = token, context, drained
        self.ops = _operations(context)
        _require(type(drained) is self.ops.DrainedHandle and drained._attempt.context is context
                 and type(maximum) is int and 0 < maximum <= 2**50, "PAPER_BACKUP_CONTEXT")
        self.operation = drained._attempt.operation
        self.generation, self.maximum = self.operation.generation, maximum
        self.clients, self.sources, self._requirements = [], [], None
        self.spools = {}
        self.root = self.root_tree = self.secret_sources = self.pgpass_identity = None
        self.exporter = self._snapshot = self.audit = self.description = self.selected = None
        self.package = None
        self.space = _SPACES.get(context)
        _require(type(self.space) is _CaptureSpace and not self.space.closed,
                 "PAPER_BACKUP_RESERVATION_REQUIRED")
        self.selection_tree = self.space.tree
        self.root, self.root_tree, self.selection = self.space.root, self.space.root_tree, self.space.selection
        self._selection_taken = self._selection_released = False
        self._closed = self._poisoned = self._entered = False
        self._custody = self.ops._open_archive_custody(drained)
        self._custody.bind(self)
        _OWNERS[self._custody] = self
        self.space.owner = self

    def __reduce__(self):
        raise PaperBackupError("PAPER_BACKUP_PRIVATE")

    def __reduce_ex__(self, _protocol):
        raise PaperBackupError("PAPER_BACKUP_PRIVATE")

    def __copy__(self):
        raise PaperBackupError("PAPER_BACKUP_PRIVATE")

    def __deepcopy__(self, _memo):
        raise PaperBackupError("PAPER_BACKUP_PRIVATE")

    @staticmethod
    def _argv(args):
        return ["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", *args]

    def _docker(self, args):
        return self.context.docker.call(args, timeout=self._custody.deadline.work_remaining(15))

    def _cleanup_docker(self, args):
        client=self.context.docker
        while isinstance(client,self.ops._StopDocker):
            client=client.base
        return client.call(args, timeout=self._custody.deadline.remaining(15))

    def _check_resources(self):
        _require(not self._closed and not self._poisoned and _OWNERS.get(self._custody) is self,
                 "PAPER_BACKUP_CLOSED")
        if self.root_tree is not None:
            self.root_tree.revalidate()
        if self.secret_sources is not None:
            for source in self.secret_sources:
                self.ops._revalidate_secret_source(source)
            info = os.stat("pgpass", dir_fd=self.root_tree.fd, follow_symlinks=False)
            _require(self.ops._identity(info) == self.pgpass_identity, "PAPER_BACKUP_SECRET_CHANGED")
            fd = os.open("pgpass", (os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW), dir_fd=self.root_tree.fd)
            try:
                expected = _pgpass(self.secret_sources[0]["bytes"], self.context.database["database"])
                _require(os.pread(fd, len(expected)+1, 0) == expected
                         and self.ops._identity(os.fstat(fd)) == self.pgpass_identity,
                         "PAPER_BACKUP_SECRET_CHANGED")
            finally:
                os.close(fd)
        self._check_spools()
        for source in self.sources:
            source.revalidate(self.selection._backup_module().files)
        if self.package is not None:
            for reader in self.package.readers:
                if not reader.absent and reader.process is not None:
                    reader.check()
        for client in self.clients:
            if not client.absent:
                client.check()
                client.child_processes()

    def _seal_spool(self, name, stream):
        _require(name not in self.spools, "PAPER_BACKUP_SPOOL_CHANGED")
        files = self.selection._backup_module().files
        fd = stream.fileno()
        os.fchmod(fd, 0o400)
        os.fsync(fd)
        info = os.fstat(fd)
        _require(info.st_size <= self.maximum, "PAPER_BACKUP_LIMIT")
        size, digest = self.ops._hash_fd(fd)
        _require(size == info.st_size and files.identity(os.fstat(fd)) == files.identity(info),
                 "PAPER_BACKUP_SPOOL_CHANGED")
        self.spools[name] = (os.dup(fd), files.identity(info), size, digest)
        self._check_spools()

    def _check_spools(self):
        if not self.spools:
            return
        files = self.selection._backup_module().files
        for name, (fd, identity, size, digest) in self.spools.items():
            _require(files.identity(os.fstat(fd)) == identity
                     and files.identity(os.stat(name, dir_fd=self.root_tree.fd, follow_symlinks=False)) == identity
                     and self.ops._hash_fd(fd) == (size, digest)
                     and files.identity(os.fstat(fd)) == identity, "PAPER_BACKUP_SPOOL_CHANGED")

    def _new_spool(self, name):
        _require(name in {"audit.stream", "postgres.dump", "files.bundle", "dataset-rows.jsonl", "audit.json", "archive.json"}, "PAPER_BACKUP_SPOOL_CHANGED")
        fd = os.open(name, os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                     0o600, dir_fd=self.root_tree.fd)
        return os.fdopen(fd, "r+b", buffering=0)

    def _owns_container(self, value):
        if self.package is not None:
            for reader in self.package.readers:
                if not reader.absent and value.get("Id") == reader.cid:
                    reader.check()
                    return True
        if self.package is not None and self.package.call is not None:
            call = self.package.call
            if value.get("Id") == call._cid:
                call._archive_observed_container(value)
                return True
        if self.package is not None and self.package.guest is not None:
            guest = self.package.guest
            if value.get("Id") == guest.cid:
                guest.check()
                return True
        for client in self.clients:
            if value.get("Id") == client.cid:
                client.check()
                return True
        return False

    def _owns_process(self, pid):
        if self.package is not None:
            package = self.package
            for reader in package.readers:
                if not reader.absent and reader.process is not None and pid in (reader.process["pid"], reader.child.pid):
                    reader.check()
                    return True
            call = package.call
            if call is not None and call._cid is not None:
                value = self.context.docker.object("container", call._cid)
                call._archive_observed_container(value)
                child = call._child
                native_pid = value.get("State", {}).get("Pid")
                if (pid == native_pid and value.get("State", {}).get("Running") is True) or (
                        child is not None and pid == child.pid and child.poll() is None):
                    row = self.ops._process_record(pid)
                    prior = package.native_processes.setdefault(pid, row)
                    _require(prior == row, "PAPER_BACKUP_NATIVE_CHANGED")
                    if pid == native_pid:
                        _require(call._cid in row["cgroup"], "PAPER_BACKUP_NATIVE_CHANGED")
                        fd = os.open("/proc/%d/root%s" % (pid, self.ops.NATIVE), os.O_RDONLY | os.O_CLOEXEC)
                        try:
                            _require(self.ops._hash_fd(fd) == (row["executable"]["bytes"],
                                                              row["executable"]["sha256"]),
                                     "PAPER_BACKUP_NATIVE_CHANGED")
                        finally:
                            os.close(fd)
                    return True
            if package.process is not None and pid == package.process.pid:
                _require(package.process_identity == self.ops._process_record(pid),
                         "PAPER_BACKUP_CRYPTO_CHANGED")
                return True
            if package.guest is not None and package.guest.owns_process(pid):
                return True
        for client in self.clients:
            if not client.absent:
                client.check()
                if pid in (client.process["pid"], client.child.pid) or pid in client.child_processes():
                    return True
        return False

    def _match_clients(self, rows):
        expected = [c.backend for c in self.clients if not c.absent]
        _require(all(type(row) is dict for row in expected), "PAPER_BACKUP_FOREIGN_CLIENT")
        remaining = list(rows)
        for row in expected:
            _require(row in remaining, "PAPER_BACKUP_CLIENT_LOST")
            remaining.remove(row)
        if not remaining:
            return
        candidates = []
        for client in self.clients:
            if not client.absent and client.dumping:
                for process in client.child_processes().values():
                    executable = process["executable"]
                    if (executable["bytes"], executable["sha256"]) == client.executables["/usr/lib/postgresql/18/bin/pg_dump"]:
                        for endpoint in self.ops._tcp_endpoints(process):
                            candidates.append((client, process, endpoint))
        server = self.context.docker.object("container", self.context.database_generation[0])
        ips = {n.get("IPAddress") for n in server["NetworkSettings"]["Networks"].values()}
        _require(len(remaining) <= 1, "PAPER_BACKUP_FOREIGN_CLIENT")  # serial pg_dump, no jobs
        for row in remaining:
            matches = [(c, p, e) for c, p, e in candidates if e["local"] == [row["addr"], row["port"]]
                       and e["peer"][0] in ips and e["peer"][1] == 5432]
            _require(row["role"] == "migration_owner" and len(matches) == 1, "PAPER_BACKUP_FOREIGN_CLIENT")
            client, process, endpoint = matches[0]
            observed = (row, process, endpoint)
            prior = client.dump_backends.get(row["pid"])
            _require(prior is None or prior == observed, "PAPER_BACKUP_CLIENT_CHANGED")
            client.dump_backends[row["pid"]] = observed

    def _resources_closed(self):
        return all(client.absent for client in self.clients) and (
            self.package is None or self.package.closed)

    def revalidate(self):
        try:
            self._custody.revalidate()
        except BaseException:
            self._poisoned = True
            raise

    def _take_selection_tree(self):
        self.revalidate()
        _require(not self._selection_taken and self.selection_tree is not None,
                 "PAPER_BACKUP_SELECTION_REPLAY")
        self._selection_taken = True
        return self.selection_tree

    def _release_selection_tree(self, tree):
        _require(self._selection_taken and not self._selection_released and tree is self.selection_tree,
                 "PAPER_BACKUP_SELECTION_REPLAY")
        self._selection_released = True

    def _file_selection_requirements(self):
        self.revalidate()
        _require(type(self._requirements) is tuple, "PAPER_BACKUP_AUDIT_INCOMPLETE")
        return self._requirements

    def __enter__(self):
        _require(not self._entered and not self._closed, "PAPER_BACKUP_CLOSED")
        self._entered = True
        try:
            self.revalidate()
            self.selection = _local("owner_beta_paper_backup_selection", "owner-beta-paper-backup-selection.py")
            self.audit_module = _local("owner_beta_paper_backup_audit", "owner-beta-paper-audit.py")
            secret, self.secret_sources = self.ops._read_migration_secret(
                self.context.values["LAGRANGE_RUNTIME_SECRET_DIR"])
            fd = os.open("pgpass", os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                         0o600, dir_fd=self.root_tree.fd)
            try:
                data = _pgpass(secret, self.context.database["database"])
                view = memoryview(data)
                while view:
                    size = os.write(fd, view); _require(size > 0, "PAPER_BACKUP_SECRET")
                    view = view[size:]
                os.fchown(fd, 999, 999); os.fchmod(fd, 0o600); os.fsync(fd)
                self.pgpass_identity = self.ops._identity(os.fstat(fd))
            finally:
                os.close(fd)
            os.fsync(self.root_tree.fd)
            secret = data = view = None
            self.exporter = _Client(self, "exporter")
            self.clients.append(self.exporter)  # cleanup custody precedes create
            self.exporter.start()
            self.revalidate()
            token = self.exporter.exchange(b"SELECT pg_export_snapshot();\n")
            _require(token.endswith(b"\n") and 1 < len(token) <= 4096 and token.count(b"\n") == 1
                     and b"\0" not in token and b"\r" not in token, "PAPER_BACKUP_SNAPSHOT")
            self._snapshot = token[:-1].decode("utf-8")
            files = self.selection._backup_module().files
            sql = self.selection._OriginalFile(files, self.context.release_path +
                "/deploy/db/owner-beta-paper-audit.sql", uid=0, gid=0, maximum=4*1024*1024)
            self.sources.append(sql)
            projection = self.selection._OriginalFile(files, self.context.release_path +
                "/docs/contracts/owner-beta-paper-audit-projection-v1.json", uid=0, gid=0, maximum=4*1024*1024)
            self.sources.append(projection)
            _require(os.pread(projection.fd, projection.size+1, 0) == self.audit_module.PROJECTION.read_bytes(),
                     "PAPER_BACKUP_PROJECTION")
            self.audit = self._new_spool("audit.stream")
            self.exporter.exchange(os.pread(sql.fd, sql.size+1, 0), self.audit)
            os.fsync(self.audit.fileno()); self.audit.seek(0)
            self.description = self.audit_module.decode(self.audit)
            self._seal_spool("audit.stream", self.audit)
            _require(self.description["projection_sha256"] == projection.sha256,
                     "PAPER_BACKUP_PROJECTION")
            # Source/stream correspondence, deliberately not full reference
            # closure: B must consume all29 tables and the actual selected files.
            self._requirements = (("paper_contract", projection.sha256, projection.size),
                                  ("paper_contract", sql.sha256, sql.size))
            self.audit.seek(0)
            self.revalidate()
            return self
        except BaseException:
            self._poisoned = True
            try:
                self.close()
            except BaseException:
                pass  # primary retained; custody remains UNKNOWN on cleanup loss
            raise

    def capture(self, limits, storage_values=None):
        """Consume selected originals under the actual exported transaction.

        No COMPLETE/latest or archive authority is produced by this phase.
        """
        self.revalidate()
        self._dump()  # actual same-snapshot DB consumer precedes file enumeration
        with self.selection.open_selected_paper_files(self.context, self.drained, self, limits) as selected:
            self.selected = selected
            try:
                with self._new_spool("files.bundle") as stream:
                    selected.snapshot.write(stream)
                    self._seal_spool("files.bundle", stream)
                selected.revalidate()
                refs = _local("owner_beta_paper_backup_references", "owner-beta-paper-backup-references.py")
                self.audit.seek(0)
                with refs.prepare_references(selected.snapshot, self.audit) as pending:
                    pending.visit_archive_members(lambda *_args: None)
                    pending.visit_database_references(lambda *_args: None)
                    self.revalidate()
                self.audit.seek(0)
                ids = refs.dataset_requirements(selected.snapshot, self.audit)
                rows = self._capture_dataset_rows(ids)
                data = refs.encode_dataset_rows(rows)
                with self._new_spool("dataset-rows.jsonl") as stream:
                    stream.write(data)
                    self._seal_spool("dataset-rows.jsonl", stream)
                self.audit.seek(0)
                source_files = refs.describe(selected.snapshot, self.audit, dataset_rows=rows)
                self._describe_archive(selected.snapshot, source_files, limits)
                self.revalidate()
                candidate = self._archive_capture(limits, storage_values)
                _require(type(candidate) is _PaperPublication and candidate.owner is self,
                         "PAPER_BACKUP_REFERENCE_CLOSURE_INCOMPLETE")
                return candidate
            finally:
                self.selected = None

    def _capture_dataset_rows(self, ids):
        """Only the already held exporter can read these source descriptors."""
        rows, total, previous = [], 0, None
        _require(type(ids) is tuple and len(ids) <= 1_000_000, "PAPER_BACKUP_DATASET_LIMIT")
        for identity in ids:
            _require(type(identity) is str and str(uuid.UUID(identity)) == identity
                     and (previous is None or previous < identity), "PAPER_BACKUP_DATASET_IDENTITY")
            self.revalidate()
            sql = ("SELECT jsonb_build_object('id',id::text,'dataset_id',dataset_id,"
                   "'version',version,'status',status,'manifest_sha256',manifest_sha256,"
                   "'storage_path',storage_path)::text FROM public.dataset_versions WHERE id="
                   + _sql_literal(identity) + "::uuid;\n").encode("ascii")
            # exchange's output limit applies while reading, before JSON decoding.
            output = io.BytesIO()
            self.exporter.exchange(sql, output, maximum=2 * 1024 * 1024)
            raw = output.getvalue()
            total += len(raw)
            _require(total <= 64 * 1024 * 1024 and raw.endswith(b"\n")
                     and raw.count(b"\n") == 1, "PAPER_BACKUP_DATASET_ROW")
            try:
                row = json.loads(raw)
            except (ValueError, UnicodeError):
                raise PaperBackupError("PAPER_BACKUP_DATASET_ROW") from None
            _require(type(row) is dict and row.get("id") == identity, "PAPER_BACKUP_DATASET_ROW")
            rows.append(row)
            previous = identity
        self.revalidate()
        return rows

    def _describe_archive(self, snapshot, source_files, limits):
        """Seal descriptors of source bytes; these grant no restore authority."""
        self.revalidate()
        root = self.operation._intent
        consumer = self.drained._attempt.consumer
        capability = self.context.verified
        capability.revalidate()
        audit = {"schema": "owner-beta-paper-audit-v1",
            "installation_id": root["installation_id"], "generation": root["generation"],
            "account_id": root["account_id"], "control_epoch": consumer.floor,
            "audit_sql_sha256": self.sources[0].sha256,
            "migration_0068_sha256": None,
            "groups": [{"group": row["name"], "sha256": row["sha256"],
                        "entries": row["row_count"], "bytes": row["byte_length"]}
                       for row in self.description["groups"]] + [source_files]}
        release = self.ops.release
        daily = release._daily()
        cap_bytes = daily.read_file(capability._fds[-1], "capability.json", uid=0, gid=0,
                                   mode=0o444, maximum=release.MAX_PROCESS)
        _require(self.ops._digest(cap_bytes) == capability.capability_sha256,
                 "PAPER_BACKUP_CAPABILITY_CHANGED")
        audit["migration_0068_sha256"] = release._json(cap_bytes, release.MAX_PROCESS)["migration_0068_sha256"]
        audit["groups"].sort(key=lambda row: row["group"])
        audit_bytes = self.ops._canonical(audit)
        binding = {"domain": "lagrange-paper-snapshot-binding-v1",
            "installation_id": root["installation_id"], "generation": root["generation"],
            "account_id": root["account_id"], "control_epoch": consumer.floor,
            "database_dump_sha256": self.spools["postgres.dump"][3],
            "audit_stream_sha256": self.spools["audit.stream"][3],
            "files_sha256": self.spools["files.bundle"][3]}
        archive = {k: binding[k] for k in ("installation_id", "generation", "account_id",
                                          "control_epoch", "database_dump_sha256", "files_sha256")}
        archive.update(schema="owner-beta-paper-archive-v1", profile="paper-combined",
            drain_receipt_sha256=self.ops._digest(consumer.actions[-1]._artifact_bytes),
            created_at=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            release_manifest_sha256=root["release_manifest_sha256"],
            capability_sha256=capability.capability_sha256,
            database_snapshot_sha256=self.ops._digest(self.ops._canonical(binding)),
            audit_sha256=self.ops._digest(audit_bytes),
            classes=self.selection._backup_module().files.class_records(snapshot.manifest(), limits))
        for name, value in (("audit.json", audit_bytes), ("archive.json", self.ops._canonical(archive))):
            _require(len(value) <= 65_536, "PAPER_BACKUP_DESCRIPTOR_LIMIT")
            with self._new_spool(name) as stream:
                stream.write(value)
                self._seal_spool(name, stream)
        capability.revalidate()
        self.revalidate()

    def _archive_capture(self, limits, storage_values, *, persisted=None):
        package = _ArchivePackage(self, limits, storage_values, persisted)
        primary = None
        try:
            package.prepare()
            package.invoke()
            if persisted is None:
                return _PaperPublication(package)
        except BaseException as error:
            primary = error
            raise
        finally:
            try:
                package.close()
            except BaseException:
                self._poisoned = True
                if primary is None:
                    raise

    def reconcile_daily(self, limits, storage_values):
        """Existing Daily repair under the already proved combined Paper OFF."""
        self.revalidate()
        daily=self.selection._backup_module()
        config,config_hash=self.ops.daily.install.verify_config(self.context.release_path,self.context.commit)
        v=self.context.values
        binding=daily.runtime.Binding(self.context.project,self.context.commit,
            self.context.images["owner-beta-runner"],config_hash,self.context.release_path,
            v["LAGRANGE_DATA_DIR"],v["LAGRANGE_ARTIFACTS_DIR"],v["LAGRANGE_RUNTIME_SECRET_DIR"],
            v["POSTGRES_DB"] or "lagrange",v["MAX_JOBS_PER_OWNER"],v["PRICE_BETA_LEASE_SECS"],v["PRICE_BETA_BACKOFF_SECS"])
        database=_bounded_daily_database(daily.ops,self.context,self._custody.deadline)
        operation=daily.ops.Operations(self.context.docker,database,self.context.context,self.context.context,
            binding,self.context.install_root,"absent")
        storage=daily.Storage(storage_values,self.context.docker,self.context.context,binding)
        controller=daily.Controller(operation,binding,config,storage,self.context.install_root)
        roots=self.selection._current_roots(self.context,self.ops,daily,daily.files)
        retained=[roots[name] for name in daily.files.SIMULATION_CLASSES]
        query=(b"SELECT jsonb_build_object('id',id::text,'dataset_id',dataset_id,'version',version,'status',status,"
            b"'manifest_sha256',manifest_sha256,'storage_path',storage_path)::text FROM public.dataset_versions ORDER BY id;\n")
        before_data=self.root/"repair-before-datasets.jsonl"
        with open(before_data,"x+b") as output:
            self.exporter.exchange(query,output,maximum=64*1024*1024)
            output.flush();os.fsync(output.fileno())
        before=self._audit_rows(self.audit)
        with daily.files.Snapshot(retained,limits,profile="simulation"):
            old=self.context.docker
            bounded=self.ops._ArchiveWorkDocker(old,self._custody.deadline)
            operation.docker=database.docker=bounded
            try:
                result=controller.reconcile_held(self)
            finally:
                operation.docker=database.docker=old
            _require(result=="RECONCILED_DISABLED","PAPER_BACKUP_RECONCILE")
            # One new RR snapshot on the SAME held, owned exporter connection.
            self.exporter.exchange(b"ROLLBACK;\nBEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n"
                b"SET LOCAL search_path='pg_catalog, pg_temp';\n")
            self._snapshot=None
            sql=self.sources[0]
            with open(self.root/"repair-after.audit","x+b") as after:
                self.exporter.exchange(os.pread(sql.fd,sql.size+1,0),after)
                after.seek(0);self.audit_module.decode(after)
                current=self._audit_rows(after)
            after_data=self.root/"repair-after-datasets.jsonl"
            with open(after_data,"x+b") as output:
                self.exporter.exchange(query,output,maximum=64*1024*1024)
                output.flush();os.fsync(output.fileno())
            _require(daily.digest_file(before_data)==daily.digest_file(after_data),"PAPER_BACKUP_RECONCILE_DATASET")
            _check_daily_repair_delta(before,current,controller._reconcile_evidence)
        self.revalidate()
        self.release_capture_clients()
        return PaperHandledResult("RECONCILED_DISABLED")

    def _audit_rows(self, stream):
        rows={}
        stream.seek(0)
        self.audit_module.decode(stream)
        stream.seek(0)
        for line in stream:
            record=self.audit_module.parse(line[:-1])
            if record[0]=="row":
                key=(record[2],record[3]);value=record[4]
            elif record[0]=="reference":
                key=(record[1],record[2]);value=record[3]
            else:
                continue
            _require(key not in rows or rows[key]==value,"PAPER_BACKUP_AUDIT_ROW")
            rows[key]=value
        return rows

    def release_capture_clients(self):
        """Prove absence without discarding the retained source/cipher originals."""
        self.revalidate()
        _require(self.selected is None and (not self._selection_taken or self._selection_released),
                 "PAPER_BACKUP_SELECTION_ACTIVE")
        if self.package is not None:
            _require(self.package.closed,"PAPER_BACKUP_CLEANUP_UNKNOWN")
        for client in reversed(self.clients):
            client.close()
        _require(self._resources_closed(),"PAPER_BACKUP_CLEANUP_UNKNOWN")
        self.revalidate()  # includes an independent zero-source-backend observation
        if self.secret_sources is not None:
            info=os.stat("pgpass",dir_fd=self.root_tree.fd,follow_symlinks=False)
            _require(self.ops._identity(info)==self.pgpass_identity,"PAPER_BACKUP_SECRET_CHANGED")
            os.unlink("pgpass",dir_fd=self.root_tree.fd)
            os.fsync(self.root_tree.fd)
            for source in self.secret_sources:
                os.close(source["file_fd"]);os.close(source["service_fd"]);source["tree"].close()
            self.secret_sources=None
        self._snapshot=None
        self.revalidate()

    def verify_candidate(self,candidate,limits,storage_values):
        _require(type(candidate) is _PaperPublication and candidate in _PUBLICATIONS
            and candidate.owner is self and candidate.retained.publication is candidate
            and not candidate.retained.completed and self._resources_closed()
            and self._snapshot is None and self.secret_sources is None,"PAPER_BACKUP_PRIVATE")
        candidate.retained.revalidate()
        if self.package is not None:
            _require(self.package.closed,"PAPER_BACKUP_CLEANUP_UNKNOWN")
        self.package=None
        self._archive_capture(limits,storage_values,persisted=candidate.retained)
        candidate.verified_package=self.package
        candidate.require_verified()
        self.revalidate()

    def verify_persisted(self, persisted, limits, storage_values):
        _require(type(persisted) is _StoredPaperSet and persisted.completed
            and persisted.storage.owner is self and self._resources_closed()
            and self._snapshot is None and self.secret_sources is None,"PAPER_BACKUP_PRIVATE")
        persisted.revalidate()
        if self.package is not None:
            _require(self.package.closed,"PAPER_BACKUP_CLEANUP_UNKNOWN")
        self.package=None
        self._archive_capture(limits,storage_values,persisted=persisted)
        persisted.revalidate()
        self.revalidate()
        _require(self._resources_closed(),"PAPER_BACKUP_CLEANUP_UNKNOWN")

    def _dump(self):
        """Actual PG18 dump importing the retained export; serial, finite, RO."""
        self.revalidate()
        client = self.exporter
        _require(not client.dumping and self._snapshot is not None, "PAPER_BACKUP_SNAPSHOT")
        status = "PAPER_DUMP_" + uuid.uuid4().hex
        command = ("/usr/bin/timeout --signal=TERM --kill-after=5s 900s "
            "/usr/lib/postgresql/18/bin/pg_dump --format=custom --no-password --enable-row-security "
            "--host=postgres --username=migration_owner --dbname=" + shlex.quote(self.context.database["database"]) +
            " --snapshot=" + shlex.quote(self._snapshot) +
            "; code=$?; printf '\\n" + status + "_%d\\n' \"$code\"")
        client.dumping = True
        try:
            with self._new_spool("postgres.dump") as dump:
                client.exchange(b"\\! " + command.encode("utf-8") + b"\n", dump)
                end = dump.tell()
                suffix = ("\n" + status + "_0\n").encode("ascii")
                _require(end > len(suffix) + 5, "PAPER_BACKUP_DUMP_FAILED")
                dump.seek(end-len(suffix))
                _require(dump.read() == suffix, "PAPER_BACKUP_DUMP_FAILED")
                dump.truncate(end-len(suffix)); dump.seek(0)
                _require(dump.read(5) == b"PGDMP", "PAPER_BACKUP_DUMP_FAILED")
                self._seal_spool("postgres.dump", dump)
            # psql cannot print our following marker before its shell wait/reap
            # completes; independently require no owned descendant/backend left.
            _require(bool(client.dump_backends), "PAPER_BACKUP_DUMP_BACKEND_UNOBSERVED")
            _require(not client.child_processes(), "PAPER_BACKUP_CLEANUP_UNKNOWN")
            rows = self._custody.observer.database("observe")["backends"]
            _require(all((row["pid"], row["start_us"]) not in
                         {(v[0]["pid"], v[0]["start_us"]) for v in client.dump_backends.values()}
                         for row in rows), "PAPER_BACKUP_CLEANUP_UNKNOWN")
        finally:
            client.dumping = False
        self.revalidate()

    def close(self):
        if self._closed:
            return
        _require(self.selected is None and (not self._selection_taken or self._selection_released),
                 "PAPER_BACKUP_SELECTION_ACTIVE")
        failure = None
        old = self.context.docker
        self.context.docker = self.ops._StopDocker(old, self._custody.deadline)
        try:
            if self.package is not None and not self.package.closed:
                try:
                    self.package.close()
                except BaseException as error:
                    failure = error
            for client in reversed(self.clients):
                try:
                    client.close()
                except BaseException as error:
                    if failure is None:
                        failure = error
        finally:
            self.context.docker = old
        if failure is not None:
            self._poisoned = True
            raise PaperBackupError("PAPER_BACKUP_CLEANUP_UNKNOWN") from None
        if self.audit is not None:
            self.audit.close(); self.audit = None
        for fd, _identity, _size, _digest in self.spools.values():
            os.close(fd)
        self.spools.clear()
        for source in reversed(self.sources):
            source.close()
        self.sources.clear()
        if self.secret_sources is not None:
            for source in self.secret_sources:
                os.close(source["file_fd"]); os.close(source["service_fd"]); source["tree"].close()
            self.secret_sources = None
        if self.root_tree is not None:
            self.root_tree.revalidate()
            self.selection._remove_tree(self.root_tree.fd)
            # The reservation retains the directory generation until the
            # original stop owner has released its ancestor observations.
        self._custody.close()
        self._snapshot = None
        self._closed = True
        _OWNERS.pop(self._custody, None)

    def __exit__(self, *error):
        if self._closed:
            return
        try:
            if error[0] is None:
                self.revalidate()
        finally:
            try:
                self.close()
            except BaseException:
                if error[0] is None:
                    raise


def open_snapshot(context, drained, maximum):
    return _HeldPaperSnapshot(_HELD_PAPER_SNAPSHOT_TOKEN, context, drained, maximum)


class _PersistedArchiveCustody:
    """Read-only source context; authority is confined to owned private clones."""
    def __init__(self,owner,floor):
        self.owner,self.deadline,self.floor=owner,owner.space.deadline,floor
        self.closed=False
    def revalidate(self):
        o=self.owner
        _require(not self.closed and _OWNERS.get(self) is o and o.space.owner is o
            and not o._closed and not o._poisoned,"PAPER_BACKUP_CLOSED")
        self.deadline.work_remaining()
        o.context.revalidate()
        o._check_resources()
        if o.package is not None and not o.package.closed:
            call=o.package.call
            _require(not o.context._invocations or o.context._invocations=={call},"PAPER_BACKUP_PRIVATE")
    def close(self):
        _require(self.owner._resources_closed(),"PAPER_BACKUP_CLEANUP_UNKNOWN")
        self.closed=True


class _PersistedArchiveOwner:
    """Standalone verify opens no source exporter and dispatches no source action."""
    _argv=staticmethod(_HeldPaperSnapshot._argv)
    _docker=_HeldPaperSnapshot._docker
    _cleanup_docker=_HeldPaperSnapshot._cleanup_docker
    _check_resources=_HeldPaperSnapshot._check_resources
    _check_spools=_HeldPaperSnapshot._check_spools
    _owns_container=_HeldPaperSnapshot._owns_container
    _owns_process=_HeldPaperSnapshot._owns_process
    _resources_closed=_HeldPaperSnapshot._resources_closed
    revalidate=_HeldPaperSnapshot.revalidate
    _archive_capture=_HeldPaperSnapshot._archive_capture
    verify_persisted=_HeldPaperSnapshot.verify_persisted
    close=_HeldPaperSnapshot.close
    def __init__(self,context,space,maximum,floor):
        _require(type(space) is _CaptureSpace and _SPACES.get(context) is space
            and space.owner is None and type(floor) is int and floor>=0,"PAPER_BACKUP_PRIVATE")
        self.context,self.ops,self.space=context,_operations(context),space
        self.maximum=maximum
        self.root,self.root_tree,self.selection=space.root,space.root_tree,space.selection
        self.clients,self.sources,self.spools=[],[],{}
        self.package=self.selected=self.secret_sources=self.audit=self._snapshot=None
        self._selection_taken=self._selection_released=False
        self._closed=self._poisoned=False
        self._entered=True
        self.audit_module=_local("owner_beta_paper_backup_audit","owner-beta-paper-audit.py")
        self._custody=_PersistedArchiveCustody(self,floor)
        space.owner=self
        _OWNERS[self._custody]=self
        try:
            files=self.selection._backup_module().files
            for relative in ("/deploy/db/owner-beta-paper-audit.sql","/docs/contracts/owner-beta-paper-audit-projection-v1.json"):
                self.sources.append(self.selection._OriginalFile(files,context.release_path+relative,
                    uid=0,gid=0,maximum=4*1024*1024))
            projection=self.sources[1]
            _require(os.pread(projection.fd,projection.size+1,0)==self.audit_module.PROJECTION.read_bytes(),
                     "PAPER_BACKUP_PROJECTION")
            self.revalidate()
        except BaseException:
            self.close()
            raise


def _application_context(context):
    ops = _local("owner_beta_paper_backup_operations", "owner-beta-paper-operations.py")
    values = ops.daily.Context(context.release, context.commit, context.images, context.values, context.overlays)
    return ops, values


def inspect_locked_install(install_root, release, commit, lock_fd):
    """Internal existing-shell adapter; no new CLI verb or history waiver."""
    ops = _local("owner_beta_paper_backup_operations", "owner-beta-paper-operations.py")
    _require(str(Path(release).parent.parent) == install_root, "PAPER_BACKUP_CONTEXT")
    ops.daily.policy.validate_current_release(install_root, release, commit)
    images = ops.daily.read_manifest(release, commit)
    values, overlays = ops.daily.read_context(release, commit, "enable")
    context = ops.daily.Context(release, commit, images, values, overlays)
    return inspect_presence(context, lock_fd)


def inspect_presence(context, lock_fd):
    """Existing locked backup dispatch consumes real native/local inventory."""
    ops, values = _application_context(context)
    with ops.open_context(values, lock_fd) as live:
        with live.probe_history() as inventory:
            inventory.revalidate()
            return "LEGACY_EMPTY" if inventory._presence is ops._HISTORY_EMPTY else "REQUIRED"


class _CaptureRun:
    """Retain original journal/context resources until clients are proved absent."""
    def __init__(self, live):
        self.live = live
        self.space = self.manager = self.store = self.operation = self.action = self.drained = None
        _RUNS.add(self)

    def close(self):
        if self not in _RUNS:
            return
        if self.space is not None and self.space.owner is not None:
            self.space.owner.close()  # failure retains EVERY later original/FD
        if self.drained is not None:
            self.drained.close()
        if self.action is not None:
            self.action.close()
        if self.operation is not None:
            self.operation.close()
        if self.manager is not None:
            self.manager.__exit__(None, None, None)
            self.manager = None
        self.live.close()
        if self.space is not None:
            self.space.close()
        _RUNS.remove(self)


def _check_daily_repair_delta(before,after,evidence):
    """Exact normalized row preservation around the existing narrow repair."""
    extras=set(evidence["extra_admissions"])
    expected=set(evidence["expected_admissions"])
    allowed={"owner_beta_daily_reservations","owner_beta_daily_admissions",
             "owner_beta_daily_poll_receipts","owner_beta_daily_poll_state"}
    def table(rows,name):
        return {key:value for (name_,key),value in rows.items() if name_==name}
    for key in before.keys()|after.keys():
        if before.get(key)==after.get(key):
            continue
        _require(key[0] in allowed and key in after,"PAPER_BACKUP_RECONCILE_DELTA")
    for name in ("owner_beta_daily_reservations","owner_beta_daily_admissions"):
        old,new=table(before,name),table(after,name)
        _require(all(new.get(key)==value for key,value in old.items()),"PAPER_BACKUP_RECONCILE_IDENTITY")
    admissions=table(after,"owner_beta_daily_admissions")
    _require({r["admission_sha256"] for r in admissions.values()}==expected
        and {r["admission_sha256"] for key,r in admissions.items() if ("owner_beta_daily_admissions",key) not in before}==extras,
        "PAPER_BACKUP_RECONCILE_IDENTITY")
    runs={row["id"]:row for row in table(before,"owner_beta_recommendation_runs").values()}
    for key,row in table(after,"owner_beta_daily_reservations").items():
        if ("owner_beta_daily_reservations",key) in before:
            continue
        run=runs.get(row["run_id"])
        _require(run is not None,"PAPER_BACKUP_RECONCILE_IDENTITY")
        fields={"owner_user_id":run["owner_user_id"],"strategy_config_id":run["strategy_config_id"],
            "session_date":run["as_of"],"trigger_version":run["daily_trigger_version"],"run_id":run["id"],
            "job_id":run["job_id"],"admission_sha256":run["daily_pins"]["admission_sha256"],
            "strategy_config_sha256":run["strategy_config_sha256"],"config_epoch":run["daily_config_epoch"],
            "activation_epoch":run["daily_activation_epoch"]}
        _require(set(row)==set(fields)|{"created_at"} and all(row[k]==v for k,v in fields.items())
            and type(row["created_at"]) is str and re.fullmatch(r"20[0-9]{2}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{6}Z",row["created_at"]),"PAPER_BACKUP_RECONCILE_IDENTITY")
    receipts=table(after,"owner_beta_daily_poll_receipts")
    changed={}
    for key,row in receipts.items():
        old=before.get(("owner_beta_daily_poll_receipts",key))
        if row==old:
            continue
        _require(extras and row["stage"]=="ADMISSION" and row["admission_sha256"] in extras
            and row["result_code"]=="VERIFIED" and row["completed_at"] is not None
            and row["event_sha256"] is None and row["completion_binding_sha256"] is not None,
            "PAPER_BACKUP_RECONCILE_RECEIPT")
        if old is not None:
            _require(old["completed_at"] is None and all(row[k]==old[k] for k in
                ("receipt_id","stage","owner_user_id","policy_sha256","activation_epoch","started_at")),
                "PAPER_BACKUP_RECONCILE_RECEIPT")
        changed[row["receipt_id"]]=row
    for key,row in table(after,"owner_beta_daily_poll_state").items():
        if row==before.get(("owner_beta_daily_poll_state",key)):
            continue
        receipt=changed.get(row["latest_receipt_id"])
        _require(receipt is not None and row["stage"]=="ADMISSION" and row["last_result_code"]=="VERIFIED"
            and row["last_completed_at"]==receipt["completed_at"]
            and row["verified_admission_sha256"]==receipt["admission_sha256"]
            and row["source_frontier_sha256"]==receipt["source_frontier_sha256"],"PAPER_BACKUP_RECONCILE_RECEIPT")


def _bounded_daily_database(module,live,deadline):
    class BoundedDatabase(module.Database):
        def execute(self,values,sql,expected,*,backup_audit=False):
            bound=180 if backup_audit else 45
            _require(deadline.work_remaining(bound)==bound,"PAPER_BACKUP_TIMEOUT")
            result=super().execute(values,sql,expected,backup_audit=backup_audit)
            deadline.work_remaining()
            return result
    return BoundedDatabase(live.docker,live.project,live.commit,live.images["owner-beta-runner"])


def _latest_predecessor(live, account, epoch, generations):
    """Read the actual SQL ordering; local history only constrains accepted IDs."""
    live.revalidate()
    ops = _operations(live)
    ops.journal._uuid(account)
    _require(type(epoch) is int and epoch >= 0 and type(generations) is set,"PAPER_BACKUP_EPOCH")
    for generation in generations:
        ops.journal._uuid(generation)
    database=_bounded_daily_database(ops.daily,live,_SPACES[live].deadline)
    sql=("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n"
         "SELECT CASE WHEN current_user='migration_owner' AND session_user=current_user "
         "AND current_setting('role')='none' AND current_setting('transaction_read_only')='on' "
         "AND EXISTS(SELECT 1 FROM public.owner_beta_paper_control WHERE account_id="+_sql_literal(account)+
         "::uuid AND control_epoch="+str(epoch)+") THEN COALESCE((SELECT generation::text "
         "FROM public.owner_beta_paper_operations WHERE account_id="+_sql_literal(account)+
         "::uuid ORDER BY created_at DESC,fence_epoch DESC,generation DESC LIMIT 1),'NONE') "
         "ELSE 'STALE' END;\nROLLBACK;\n")
    selected=database.execute(live.values,sql,generations|{"NONE"})
    _require(selected != "NONE" or not generations,"PAPER_BACKUP_PREDECESSOR")
    live.revalidate()
    return None if selected == "NONE" else selected


def run_locked(context, lock_fd, action, maximum, *, storage_values=None):
    """Existing backup caller prefix; only proved empty inventory returns None.

    Configured/unknown Paper never enters Daily fallback. Only consumed native
    validation, actual cleanup and persisted-set reverification return success.
    """
    _require(action in {"backup", "verify", "reconcile", "preflight"}, "PAPER_BACKUP_ACTION")
    ops, values = _application_context(context)
    run = _CaptureRun(ops.open_context(values, lock_fd))
    live = run.live
    primary = None
    try:
        with live.probe_history() as inventory:
            inventory.revalidate()
            if inventory._presence is ops._HISTORY_EMPTY:
                return None
            _require(inventory._configured, "PAPER_BACKUP_CONFIGURATION_UNKNOWN")
            page = ops._json(inventory._pages[0][1], ops.MAX_PAGE)
            account, epoch = page["account_id"], page["control_epoch"]
            if action == "preflight":
                inventory.revalidate()
                return PaperHandledResult("PREFLIGHT")
            generations=set()
            for original in (() if action=="verify" else inventory._local.operations):
                original.revalidate()
                _require(original._last is not None and original._last[2] in {"OFF_COMMITTED","SNAPSHOT"},
                         "PAPER_BACKUP_PRIOR_OPERATION_UNRESOLVED")
                _require(original._intent["account_id"] == account,"PAPER_BACKUP_PREDECESSOR")
                if original._last[2] == "SNAPSHOT":
                    _require(original._intent["purpose"] == "BACKUP", "PAPER_BACKUP_PRIOR_OPERATION_UNRESOLVED")
                    raw=original.read_original(original._last[3]["artifact_sha256"])
                    archived=ops.journal._json(raw,ops.journal.MAX_SMALL)
                    _require(archived.get("schema") == "owner-beta-paper-archive-v1"
                        and archived.get("generation") == original.generation
                        and archived.get("installation_id") == original._intent["installation_id"]
                        and archived.get("account_id") == account
                        and archived.get("control_epoch") == original._last[3]["control_epoch"],
                        "PAPER_BACKUP_PRIOR_OPERATION_UNRESOLVED")
                generations.add(original.generation)
        if action=="verify":
            run.space=reserve_capture(live)
            owner=_PersistedArchiveOwner(live,run.space,maximum,epoch)
            persisted=_PaperStorage(owner,storage_values).latest()
            try:
                limits=owner.selection._backup_module().files.Limits(maximum)
                owner.verify_persisted(persisted,limits,storage_values)
                owner.close()
                return PaperHandledResult("VERIFIED_ISOLATED")
            finally:
                persisted.close()
        _require(action in {"backup","verify","reconcile"}, "PAPER_BACKUP_ACTION")
        projection = Path(__file__).resolve().parents[3] / "docs/contracts/owner-beta-paper-audit-projection-v1.json"
        _require(projection.is_file(), "PAPER_BACKUP_TOOL_COHORT_INCOMPLETE")
        run.space = reserve_capture(live)
        predecessor=_latest_predecessor(live,account,epoch,generations)
        run.space.deadline.work_remaining()
        run.manager = ops.journal.open_journal(live.install_root, live.lock_fd, bootstrap=False)
        store = run.store = run.manager.__enter__()
        _require(store.epoch_floor == epoch, "PAPER_BACKUP_EPOCH")
        generation = str(uuid.uuid4())
        root = {"schema": "owner-beta-paper-operation-intent-v1", "installation_id": store.installation_id,
            "generation": generation, "account_id": account, "predecessor_generation": predecessor,
            "request_key": generation + ":begin", "purpose": "BACKUP", "scope": "COMBINED_BACKUP",
            "expected_epoch": epoch, "local_epoch_floor": epoch,
            "release_manifest_sha256": live.manifest_sha256, "artifact_sha256": None, "resume_target_id": None}
        operation = run.operation = store.bind_caller("backup:" + generation, ops._canonical(root))
        stage = {"schema": "owner-beta-paper-operation-stage-v1", "installation_id": store.installation_id,
            "generation": generation, "account_id": account, "sequence": 0, "previous_sha256": None,
            "intent_sha256": operation.intent_sha256, "stage": "INTENT", "boot_id": ops._current_boot_id(),
            "observed": ops._stage_observation(), "control_epoch": epoch, "artifact_sha256": None,
            "event_id": None, "error_code": "NONE"}
        stage_hash = operation.append(ops._canonical(stage))
        action_data = {"schema": "owner-beta-paper-action-binding-v1", "installation_id": store.installation_id,
            "generation": generation, "root_intent_sha256": operation.intent_sha256, "sequence": 0,
            "previous_binding_sha256": None, "verb": "begin", "request_key": generation + ":begin",
            "expected_epoch": epoch, "artifact_sha256": None, "stage_sha256": stage_hash}
        custody = run.action = operation.prepare_action(ops._canonical(action_data))
        invocation = live.invoke("begin", custody)
        with invocation:
            _require(invocation.result["outcome"] == "OK", "PAPER_BACKUP_BEGIN_UNKNOWN")
            imported = invocation.import_dispatch()
            imported.close()
        live.fence_begin(invocation)
        drained = run.drained = live.drain(invocation)
        ops.commit_off(drained)
        if action == "reconcile":
            repair_db=_bounded_daily_database(ops.daily,live,run.space.deadline)
            repair_db.control(live.release_path,live.commit,live.values,"disable")
            repair_db.control(live.release_path,live.commit,live.values,"check-disabled")
        with open_snapshot(live, drained, maximum) as held:
            files = held.selection._backup_module().files
            limits=files.Limits(maximum)
            if action == "reconcile":
                return held.reconcile_daily(limits,storage_values)
            candidate=persisted=None
            try:
                if action == "backup":
                    candidate=held.capture(limits, storage_values)
                    held.release_capture_clients()
                    held.verify_candidate(candidate,limits,storage_values)
                    held._custody.seal_snapshot(candidate)
                    # Finish every source/clone helper, original, staging tree,
                    # context and capture owner before COMPLETE/latest exists.
                    run.close()
                    candidate.publish()
                result=PaperHandledResult("BACKUP_COMPLETE" if action == "backup" else "VERIFIED_ISOLATED",
                    None if candidate is None else candidate.generation,
                    None if candidate is None else candidate.archive_sha256)
            finally:
                if persisted is not None:
                    persisted.close()
                if candidate is not None:
                    candidate.close()
        return result
    except BaseException as error:
        primary = error
        raise
    finally:
        try:
            run.close()
        except BaseException:
            if primary is None:
                raise
            # Preserve original finite failure; _RUNS retains unresolved custody.
