#!/usr/bin/env python3
"""Private Paper capture resources. No archive publication or restore authority.

Only a registered, settled live COMBINED_BACKUP OFF can issue this owner. The
exported snapshot is never accepted as input or persisted. Every DB client is
created here, bound to its actual container/process/socket/backend generation,
and retained until independently absent. Reference closure remains a separate
required consumer of the real selected Snapshot and captured audit stream.
"""
from __future__ import annotations

import contextlib
import ctypes
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


class PaperBackupError(Exception):
    """Finite source codes only; never include SQL, credentials or output."""


def _require(value, code):
    if not value:
        raise PaperBackupError(code)


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
            with self.ops.journal._Tree("/run") as parent:
                name = "lagrange-paper-capture-" + uuid.uuid4().hex
                os.mkdir(name, 0o700, dir_fd=parent.fd)
                os.fsync(parent.fd)
                self.root = Path("/run") / name
                self.root_tree = self.ops.journal._Tree(str(self.root))
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
    def __init__(self, package, path, uid=0, gid=0):
        self.ops, self.path = package.ops, str(path)
        self.tree = self.ops.journal._Tree(str(Path(path).parent))
        self.fd = None
        try:
            self.name = Path(path).name
            self.fd = os.open(self.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC,
                              dir_fd=self.tree.fd)
            info = self.ops._file(self.fd, 0o400, uid=uid, gid=gid)
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


class _ArchivePackage:
    """Live captured/decrypted originals; never serializable archive authority."""
    def __init__(self, owner, limits, values):
        _require(type(owner) is _HeldPaperSnapshot and _OWNERS.get(owner._custody) is owner
                 and owner._entered and not owner._closed, "PAPER_BACKUP_PRIVATE")
        owner.revalidate()
        _require(owner.package is None and type(values) is dict, "PAPER_BACKUP_PRIVATE")
        self.owner, self.ops, self.limits, self.values = owner, owner.ops, limits, values
        self.path = owner.root / "archive"
        self.tree = self.snapshot = self.guest = self.call = self.custody = None
        self.process = self.process_identity = None
        self.native_processes = {}
        self.originals = []
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
        os.mkdir(self.path, 0o700)
        self.tree = self.ops.journal._Tree(str(self.path))
        os.mkdir(self.path / "files", 0o700)
        os.mkdir(self.path / "coordination", 0o700)
        for name in ("files", "coordination"):
            fd = os.open(name, self.ops.journal.DIR_FLAGS, dir_fd=self.tree.fd)
            self.directories.append((self.tree.fd, name, fd, self.ops.journal._dir_identity(os.fstat(fd))))
        # Reserve every later directory before binding immutable parent identities.
        for leaf in ("clone-data", "clone-socket"):
            os.mkdir(owner.root / leaf, 0o700); os.chown(owner.root / leaf, 999, 999)
        # The enclosing capture directory is root0700. The helper has no DAC
        # override capability, so its readonly socket bind needs traversal.
        os.chmod(owner.root / "clone-socket", 0o755)
        daily = owner.selection._backup_module()
        for name in ("files.bundle", "audit.stream", "postgres.dump"):
            owner._check_spools()
            encrypted = owner.root / (name + ".enc")
            target = self.path / name if name != "postgres.dump" else owner.root / "clone.dump"
            daily._paper_crypt(self, owner.root / name, encrypted)
            daily._paper_crypt(self, encrypted, target, decrypt=True)
            fd = os.open(target, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
            try:
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
        self.guest = _PaperRestoreGuest(self)
        self.guest.start()
        self.revalidate()

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
        self.tree.revalidate()
        _require({entry.name for entry in os.scandir(self.tree.fd)} ==
                 {"files.bundle", "audit.stream", "files", "coordination"}, "PAPER_BACKUP_ORIGINAL_CHANGED")
        for parent, name, fd, identity in self.directories:
            _require(self.ops.journal._dir_identity(os.fstat(fd)) == identity and
                     self.ops.journal._dir_identity(os.stat(name, dir_fd=parent,
                                                           follow_symlinks=False)) == identity,
                     "PAPER_BACKUP_ORIGINAL_CHANGED")
        files = self.owner.selection._backup_module().files
        for original in self.originals:
            original.revalidate(files)
        self.snapshot.verify_unchanged()
        _require(self.guest is not None, "PAPER_BACKUP_CLONE")
        self.guest.check(ready=True)

    def invoke(self):
        _require(self.call is None and self.custody is None, "PAPER_BACKUP_NATIVE_REPLAY")
        self.revalidate()
        consumer = self.owner.drained._attempt.consumer
        action = consumer.actions[-1]
        self.custody = self.owner.operation._journal.open_custody(self.owner.generation, action.sequence)
        _require(self.custody._readonly and self.custody._action == action._binding_bytes
                 and self.custody._dispatch == action._dispatch_bytes
                 and self.custody._artifact == action._artifact_bytes, "PAPER_BACKUP_CUSTODY")
        self.call = self.owner.context._invoke_archive(self)
        old = self.owner.context.docker
        self.owner.context.docker = self.ops._ArchiveWorkDocker(old, self.owner._custody.deadline)
        primary = None
        try:
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
            # Even exact OFF readback is not the unfinished domain certificate.
            raise PaperBackupError("PAPER_BACKUP_REFERENCE_CLOSURE_INCOMPLETE")
        except BaseException as error:
            primary = error
            raise
        finally:
            try:
                self.call.close()
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
        for resource in (self.call, self.guest):
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
        for original in self.originals:
            try:
                original.close()
            except BaseException as error:
                if failure is None:
                    failure = error
        self.originals.clear()
        for _parent, _name, fd, _identity in reversed(self.directories):
            try:
                os.close(fd)
            except BaseException as error:
                if failure is None:
                    failure = error
        self.directories.clear()
        if self.tree is not None:
            self.tree.close(); self.tree = None
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
        self.socket = self.owner.root / "clone-socket"

    def start(self):
        _require(not self.create_sent and self.cid is None and not self.absent,
                 "PAPER_BACKUP_CLONE_UNKNOWN")
        owner = self.owner
        admin, roles = _catalog_restore_sql(owner)
        self.admin = admin
        image = owner.context.docker.object("image", self.ops.POSTGRES_IMAGE)
        _require(self.ops.POSTGRES_IMAGE in image.get("RepoDigests", []) and
                 self.ops.runtime.DIGEST.fullmatch(image.get("Id", "")), "PAPER_BACKUP_IMAGE")
        self.image = image["Id"]
        for name, mode in (("clone-data", 0o700), ("clone-socket", 0o755)):
            fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                         dir_fd=owner.root_tree.fd)
            self.directories.append((name, fd, self.ops.journal._dir_identity(os.fstat(fd))))
            info = os.fstat(fd)
            _require((info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode)) == (999, 999, mode),
                     "PAPER_BACKUP_CLONE_CHANGED")
        roles_path = owner.root / "clone-roles.sql"
        fd = os.open(roles_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o400)
        with os.fdopen(fd, "wb") as stream:
            stream.write(roles); stream.flush(); os.fchown(stream.fileno(), 999, 999); os.fsync(stream.fileno())
        # The protected dump is mounted separately. A read-only fd copy with
        # guest ownership is necessary; production credentials are not mounted.
        dump = owner.root / "guest.dump"
        original_dump = next(original for original in self.package.originals
                             if original.path == str(owner.root / "clone.dump"))
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
        self.mounts = ((str(owner.root / "clone-data"), "/var/lib/postgresql/verify", False),
                       (str(self.socket), "/run/postgresql", False),
                       (str(roles_path), "/verify/roles.sql", True),
                       (str(dump), "/verify/postgres.dump", True),
                       (script.path, "/verify/entry.sh", True))
        args = ["create", "--name", self.name, "--label", "lagrange.paper.archive=" + self.nonce,
                "--user", "999:999", "--network", "none", "--read-only", "--cap-drop", "ALL",
                "--security-opt", "no-new-privileges", "--restart", "no", "--log-driver", "none",
                "--memory", "2147483648", "--cpus", "1", "--entrypoint", "/bin/bash"]
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
                     self.ops.journal._dir_identity(os.stat(name, dir_fd=self.owner.root_tree.fd,
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
                 and not host.get("VolumesFrom") and not host.get("Tmpfs")
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
        self.ops._validate_mount_records(value.get("Mounts") or [], self.mounts)
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
        mounts = value.get("Mounts") or []
        _require(len(mounts) == 1 and mounts[0].get("Type") == "bind"
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
        _require(len(networks) == 1 and {v.get("NetworkID") for v in networks.values()} == {c.network_id},
                 "PAPER_BACKUP_CLIENT_CHANGED")

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

    def exchange(self, sql, destination=None):
        """Bounded simultaneous pipe write/drain; marker follows SQL completion."""
        o = self.owner
        _require(self.child is not None and self.child.poll() is None
                 and type(sql) is bytes and len(sql) <= 4 * 1024 * 1024,
                 "PAPER_BACKUP_CLIENT_LOST")
        marker = ("PAPER_CAPTURE_" + uuid.uuid4().hex).encode("ascii")
        payload = memoryview(sql + b"\n\\echo " + marker + b"\n")
        output = io.BytesIO() if destination is None else destination
        maximum = 65_536 if destination is None else o.maximum
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
        return self.context.docker.call(args, timeout=self._custody.deadline.remaining(15))

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
        for client in self.clients:
            if not client.absent:
                client.check()
                client.child_processes()

    def _seal_spool(self, name, stream):
        _require(name not in self.spools, "PAPER_BACKUP_SPOOL_CHANGED")
        files = self.selection._backup_module().files
        fd = stream.fileno()
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
        _require(name in {"audit.stream", "postgres.dump", "files.bundle"}, "PAPER_BACKUP_SPOOL_CHANGED")
        fd = os.open(name, os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                     0o600, dir_fd=self.root_tree.fd)
        return os.fdopen(fd, "r+b", buffering=0)

    def _owns_container(self, value):
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
                    # This is a real borrowed traversal. Pending has NO success or
                    # certificate API; partial graphs cannot publish an archive.
                    pending.visit_archive_members(lambda *_args: None)
                    pending.visit_database_references(lambda *_args: None)
                    self.revalidate()
                self._archive_capture(limits, storage_values)
                raise PaperBackupError("PAPER_BACKUP_REFERENCE_CLOSURE_INCOMPLETE")
            finally:
                self.selected = None

    def _archive_capture(self, limits, storage_values):
        package = _ArchivePackage(self, limits, storage_values)
        primary = None
        try:
            package.prepare()
            package.invoke()
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

    def _dump(self):
        """Actual PG18 dump importing the retained export; serial, finite, RO."""
        self.revalidate()
        client = self.exporter
        _require(not client.dumping and self._snapshot is not None, "PAPER_BACKUP_SNAPSHOT")
        status = "PAPER_DUMP_" + uuid.uuid4().hex
        command = ("/usr/bin/timeout --signal=TERM --kill-after=5s 900s "
            "/usr/lib/postgresql/18/bin/pg_dump --format=custom --no-password "
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


def run_locked(context, lock_fd, action, maximum, *, storage_values=None):
    """Existing backup caller prefix; only proved empty inventory returns None.

    Configured/unknown Paper never enters Daily fallback. Resource capture still
    stops at the incomplete reference consumer. UNKNOWN retains original FDs.
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
            for original in inventory._local.operations:
                original.revalidate()
                _require(original._last is not None and original._last[2] == "OFF_COMMITTED",
                         "PAPER_BACKUP_PRIOR_OPERATION_UNRESOLVED")
        _require(action == "backup", "PAPER_BACKUP_REFERENCE_CLOSURE_INCOMPLETE")
        projection = Path(__file__).resolve().parents[3] / "docs/contracts/owner-beta-paper-audit-projection-v1.json"
        _require(projection.is_file(), "PAPER_BACKUP_TOOL_COHORT_INCOMPLETE")
        run.space = reserve_capture(live)
        run.manager = ops.journal.open_journal(live.install_root, live.lock_fd, bootstrap=False)
        store = run.store = run.manager.__enter__()
        _require(store.epoch_floor == epoch, "PAPER_BACKUP_EPOCH")
        generation = str(uuid.uuid4())
        root = {"schema": "owner-beta-paper-operation-intent-v1", "installation_id": store.installation_id,
            "generation": generation, "account_id": account, "predecessor_generation": None,
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
        with open_snapshot(live, drained, maximum) as held:
            files = held.selection._backup_module().files
            held.capture(files.Limits(maximum), storage_values)
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
