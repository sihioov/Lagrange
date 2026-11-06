#!/usr/bin/env python3
"""Prepare and verify an immutable OFF-capable Owner-beta Paper release.

This module is deliberately separate from activation. It never starts a service,
creates consent, or treats image labels as compile evidence.
"""
from __future__ import annotations

import contextlib
import dataclasses
import hashlib
import importlib.util
import io
import json
import os
import re
import stat
import subprocess
import sys
import tarfile
import uuid
from functools import wraps
from pathlib import Path

ROOT = str(Path(__file__).resolve().parents[3])
MANIFEST_NAME = ".lagrange-release-manifest"
MANIFEST_V3 = "LAGRANGE_RELEASE_MANIFEST_V3"
PAPER_DIRECTORY = "deploy/owner-beta-paper"
FLOOR_NAME = ".owner-beta-paper-reader-floor"
FLOOR_VALUE = {"schema": "owner-beta-paper-reader-floor-v1", "minimum": 1}
CAPABILITY_SCHEMA = "owner-beta-paper-reader-capability-v1"
INSTALL_RECEIPT_NAME = ".owner-beta-paper-install-receipt"
INSTALL_RECEIPT_SCHEMA = "owner-beta-paper-install-receipt-v1"
PROCESS_SCHEMA = "owner-beta-paper-process-v1"
OPERATIONS_SCHEMA = "owner-beta-paper-operations-v1.schema.json"
ARCHIVE_SCHEMA = "owner-beta-paper-operations-archive-v1.schema.json"
MIGRATION = "migrations/0068_owner_beta_paper_operations_v1.up.sql"
AUDIT = "deploy/db/owner-beta-paper-audit.sql"
PROJECTION = "docs/contracts/owner-beta-paper-audit-projection-v1.json"
PROJECTION_ID = "owner-beta-paper-audit-projection-v1.json"
FINANCIAL_CONTRACT = "docs/contracts/owner-beta-paper-v1.md"
MAX_MANIFEST = 64 * 1024
MAX_SOURCE = 1_048_576
MAX_PROCESS = 65_536
SHA = re.compile(r"sha256:[0-9a-f]{64}\Z")
COMMIT = re.compile(r"[0-9a-f]{40}\Z")
UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\Z")
CAPABILITY_KEYS = frozenset({
    "schema", "minimum", "code_commit", "release_manifest_sha256",
    "operations_schema_sha256", "archive_schema_sha256", "migration_0068_sha256",
    "audit_sql_sha256", "api_binary_sha256", "worker_binary_sha256", "native_binary_sha256",
})
PROCESS_KEYS = frozenset({
    "schema", "owner_user_id", "account_id", "strategy_config_id", "consent_event_id",
    "consent_version", "control_epoch", "daily_cohort_sha256",
    "financial_contract_sha256", "code_commit", "release_manifest_sha256",
    "api_binary_sha256", "worker_binary_sha256",
})
PATHS = {
    "operations_schema_sha256": OPERATIONS_SCHEMA,
    "archive_schema_sha256": ARCHIVE_SCHEMA,
    "migration_0068_sha256": MIGRATION,
    "audit_sql_sha256": AUDIT,
}

_DAILY_NAME = "_lagrange_owner_beta_paper_daily"


class PaperReleaseError(Exception):
    """Fixed, non-sensitive error boundary."""

    def __init__(self, code="PAPER_RELEASE_INVALID"):
        super().__init__(code)
        self.code = code


def _fail(code="PAPER_RELEASE_INVALID"):
    raise PaperReleaseError(code)


def _boundary(function):
    @wraps(function)
    def wrapped(*args, **kwargs):
        try:
            return function(*args, **kwargs)
        except PaperReleaseError:
            raise
        except FileNotFoundError:
            _fail("PAPER_RELEASE_MISSING")
        except OSError:
            _fail("PAPER_RELEASE_IO")
        except Exception as error:
            daily = sys.modules.get(_DAILY_NAME)
            daily_error = getattr(daily, "DailyInstallError", None) if daily else None
            if daily_error is not None and isinstance(error, daily_error):
                _fail("PAPER_RELEASE_PROTECTED_INPUT")
            _fail()
    return wrapped


def _daily():
    loaded = sys.modules.get(_DAILY_NAME)
    if loaded is not None:
        return loaded
    path = os.path.join(os.path.dirname(__file__), "owner-beta-daily-release.py")
    spec = importlib.util.spec_from_file_location(_DAILY_NAME, path)
    if spec is None or spec.loader is None:
        _fail("PAPER_RELEASE_DEPENDENCY")
    module = importlib.util.module_from_spec(spec)
    sys.modules[_DAILY_NAME] = module
    try:
        spec.loader.exec_module(module)
    except Exception:
        sys.modules.pop(_DAILY_NAME, None)
        _fail("PAPER_RELEASE_DEPENDENCY")
    return module


def _is_commit(value):
    return type(value) is str and COMMIT.fullmatch(value) is not None and value != "0" * 40


def _digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def _canonical(value):
    try:
        return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"),
                          allow_nan=False).encode("utf-8")
    except (TypeError, ValueError, UnicodeError, RecursionError):
        _fail()


FLOOR_BYTES = _canonical(FLOOR_VALUE)


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            _fail()
        result[key] = value
    return result


def _json(data, maximum):
    if type(data) is not bytes or not data or len(data) > maximum:
        _fail("PAPER_RELEASE_SIZE")
    try:
        value = json.loads(data.decode("utf-8", "strict"), object_pairs_hook=_pairs,
                           parse_constant=lambda _: _fail())
    except PaperReleaseError:
        raise
    except (UnicodeError, ValueError, RecursionError):
        _fail()
    if type(value) is not dict:
        _fail()
    return value


def _validate_process(value, commit, manifest_sha256, financial_contract_sha256=None):
    if type(value) is not dict or set(value) != PROCESS_KEYS:
        _fail("PAPER_RELEASE_PROCESS")
    if (value.get("schema") != PROCESS_SCHEMA or value.get("consent_version") != "owner-beta-paper-v1-draft-1"
            or not _is_commit(value.get("code_commit")) or value["code_commit"] != commit
            or value.get("release_manifest_sha256") != manifest_sha256
            or type(value.get("control_epoch")) is not int or value["control_epoch"] < 1):
        _fail("PAPER_RELEASE_PROCESS")
    for key in ("owner_user_id", "account_id", "strategy_config_id", "consent_event_id"):
        text = value.get(key)
        try:
            parsed = uuid.UUID(text) if type(text) is str else None
        except (ValueError, AttributeError):
            parsed = None
        if parsed is None or parsed.int == 0 or str(parsed) != text:
            _fail("PAPER_RELEASE_PROCESS")
    for key in ("daily_cohort_sha256", "financial_contract_sha256", "api_binary_sha256", "worker_binary_sha256"):
        if type(value.get(key)) is not str or SHA.fullmatch(value[key]) is None:
            _fail("PAPER_RELEASE_PROCESS")
    if (financial_contract_sha256 is not None
            and value["financial_contract_sha256"] != financial_contract_sha256):
        _fail("PAPER_RELEASE_PROCESS")
    return value


def _manifest(data, commit, expected_sha):
    # The deployed manifest is the closed line format from release-image-manifest.sh.
    try:
        lines = data.decode("ascii", "strict").splitlines()
    except (UnicodeError, AttributeError):
        _fail("PAPER_RELEASE_MANIFEST")
    if (not data.endswith(b"\n") or data.endswith(b"\n\n") or len(data) > MAX_MANIFEST
            or not _is_commit(commit) or _digest(data) != expected_sha
            or len(lines) != 15 or lines[0] != MANIFEST_V3 or lines[1] != "commit|" + commit):
        _fail("PAPER_RELEASE_MANIFEST")
    services = ("db-role-bootstrap", "db-migrate", "api-server", "web", "research-worker",
                "recommendation-runner", "candidate-runner", "owner-beta-runner",
                "owner-equity-v2-runner", "nt-backtest-worker-1", "nt-backtest-worker-2",
                "paper-scheduler", "owner-beta-simulation-runner")
    images = {}
    for line, service in zip(lines[2:], services, strict=True):
        fields = line.split("|")
        if (len(fields) != 5 or fields[0] != "image" or fields[1] != service
                or fields[2] != f"lagrange-station-{service}:{commit}"
                or not re.fullmatch(r"sha256:[0-9a-f]{64}", fields[3])
                or fields[4] != commit):
            _fail("PAPER_RELEASE_MANIFEST")
        images[service] = fields[3]
    return images


def _compiled_commit_in_elf(data, commit):
    """Require the Rust compile-time revision in an allocated read-only ELF section."""
    if type(data) is not bytes or not data.startswith(b"\x7fELF") or not _is_commit(commit):
        return False
    # The shipped image is linux/amd64. Refuse unknown ELF layouts rather than
    # searching arbitrary executable bytes for a caller-selected label.
    if len(data) < 64 or data[4:7] != b"\x02\x01\x01":
        return False
    try:
        import struct
        machine = struct.unpack_from("<H", data, 18)[0]
        section_offset = struct.unpack_from("<Q", data, 40)[0]
        section_size = struct.unpack_from("<H", data, 58)[0]
        section_count = struct.unpack_from("<H", data, 60)[0]
        string_index = struct.unpack_from("<H", data, 62)[0]
    except struct.error:
        return False
    if (machine != 62 or section_size < 64 or section_count < 2
            or section_count > 4096 or string_index >= section_count
            or section_offset > len(data)
            or section_count * section_size > len(data) - section_offset):
        return False
    sections = []
    for index in range(section_count):
        offset = section_offset + index * section_size
        try:
            name_offset = struct.unpack_from("<I", data, offset)[0]
            section_type = struct.unpack_from("<I", data, offset + 4)[0]
            flags = struct.unpack_from("<Q", data, offset + 8)[0]
            data_offset = struct.unpack_from("<Q", data, offset + 24)[0]
            size = struct.unpack_from("<Q", data, offset + 32)[0]
        except struct.error:
            return False
        if section_type != 8 and (data_offset > len(data) or size > len(data) - data_offset):
            return False
        sections.append((name_offset, section_type, flags, data_offset, size))
    _, strings_type, _, strings_offset, strings_size = sections[string_index]
    if (strings_type != 3 or strings_offset > len(data)
            or strings_size > len(data) - strings_offset):
        return False
    names = data[strings_offset:strings_offset + strings_size]
    marker = commit.encode("ascii")
    for name_offset, section_type, flags, offset, size in sections:
        if name_offset >= len(names):
            return False
        end = names.find(b"\x00", name_offset)
        if end < 0:
            return False
        section_name = names[name_offset:end]
        # SHF_ALLOC and not SHF_WRITE; Rust's option_env! constant is stored in
        # a read-only data section (commonly .rodata.str1.1).
        if (section_type != 8 and section_name.startswith(b".rodata")
                and (flags & 0x2) and not (flags & 0x1)):
            if marker in data[offset:offset + size]:
                return True
    return False


def _extract_image_elf(image_id, path):
    """Read an executable from the exact local image ID with a disposable container."""
    created = subprocess.run(["docker", "create", image_id], stdin=subprocess.DEVNULL,
                             stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False)
    if created.returncode != 0:
        _fail("PAPER_RELEASE_IMAGE")
    container = created.stdout.decode("ascii", "strict").strip()
    if not re.fullmatch(r"[0-9a-f]{12,64}", container):
        _fail("PAPER_RELEASE_IMAGE")
    try:
        copied = subprocess.run(["docker", "cp", container + ":" + path, "-"],
                                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, check=False)
        if copied.returncode != 0:
            _fail("PAPER_RELEASE_ELF")
        try:
            with tarfile.open(fileobj=io.BytesIO(copied.stdout), mode="r:*") as archive:
                members = [item for item in archive.getmembers() if item.isfile()]
                if len(members) != 1 or members[0].name.rsplit("/", 1)[-1] != path.rsplit("/", 1)[-1]:
                    _fail("PAPER_RELEASE_ELF")
                stream = archive.extractfile(members[0])
                data = stream.read() if stream is not None else b""
        except (OSError, tarfile.TarError, ValueError):
            _fail("PAPER_RELEASE_ELF")
        if not data.startswith(b"\x7fELF"):
            _fail("PAPER_RELEASE_ELF")
        return data
    finally:
        removed = subprocess.run(["docker", "rm", "-f", container], stdin=subprocess.DEVNULL,
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
        if removed.returncode != 0:
            _fail("PAPER_RELEASE_IMAGE_CLEANUP")


def _actual_elfs(images, commit, extract=_extract_image_elf):
    binaries = {
        "api_binary_sha256": (images["api-server"], "/usr/local/bin/api-server"),
        "worker_binary_sha256": (images["paper-scheduler"], "/usr/local/bin/paper-runner-bin"),
        "native_binary_sha256": (images["paper-scheduler"], "/usr/local/bin/owner-beta-paper-ops-native-bin"),
    }
    found = {}
    for name, (image_id, path) in binaries.items():
        data = extract(image_id, path)
        if not _compiled_commit_in_elf(data, commit):
            _fail("PAPER_RELEASE_ELF_REVISION")
        found[name] = (data, _digest(data))
    return found


def _install_receipt(commit, manifest_sha256, capability_sha256, projection_sha256,
                     process_sha256=None):
    return _canonical({
        "schema": INSTALL_RECEIPT_SCHEMA,
        "code_commit": commit,
        "release_manifest_sha256": manifest_sha256,
        "capability_sha256": capability_sha256,
        "audit_projection_sha256": projection_sha256,
        "process_sha256": process_sha256,
    })


def _verify_install_receipt(daily, release_fd, commit, manifest_sha256,
                            capability_sha256, projection_sha256, process_sha256=None):
    data = daily.read_file(release_fd, INSTALL_RECEIPT_NAME, uid=0, gid=0,
                           mode=0o600, maximum=4096)
    expected = _install_receipt(commit, manifest_sha256, capability_sha256,
                                projection_sha256, process_sha256)
    if data != expected:
        _fail("PAPER_RELEASE_RECEIPT")
    return _digest(data)


def _read_source(daily, release_fd, relative, maximum=MAX_SOURCE):
    parts = relative.split("/")
    current = release_fd
    try:
        with contextlib.ExitStack() as stack:
            for part in parts[:-1]:
                current = stack.enter_context(daily.child_directory(current, part, 0, 0, 0o755))
            return daily.read_file(current, parts[-1], uid=0, gid=0, maximum=maximum)
    except PaperReleaseError:
        raise
    except OSError:
        _fail("PAPER_RELEASE_SOURCE")


def _source_values(daily, release_fd):
    files = {}
    for key, path in PATHS.items():
        files[key] = _read_source(daily, release_fd, path)
    operations_schema = _json(files["operations_schema_sha256"], MAX_SOURCE)
    archive_schema = _json(files["archive_schema_sha256"], MAX_SOURCE)
    if (operations_schema.get("$id") != OPERATIONS_SCHEMA
            or archive_schema.get("$id") != ARCHIVE_SCHEMA):
        _fail("PAPER_RELEASE_SOURCE")
    # P5 owns the projection and its physical audit SQL coupling. Require its
    # actual file to exist and parse before a capability can be emitted.
    projection = _read_source(daily, release_fd, PROJECTION)
    if _json(projection, MAX_SOURCE).get("$id") != PROJECTION_ID:
        _fail("PAPER_RELEASE_SOURCE")
    if not files["audit_sql_sha256"].strip() or not files["migration_0068_sha256"].strip():
        _fail("PAPER_RELEASE_SOURCE")
    return files, projection


def _write_new_file(parent, name, data, uid=0, gid=0, mode=0o444):
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0)
    fd = os.open(name, flags, mode, dir_fd=parent)
    try:
        os.fchown(fd, uid, gid)
        os.fchmod(fd, mode)
        view = memoryview(data)
        while view:
            written = os.write(fd, view)
            if written <= 0:
                _fail("PAPER_RELEASE_WRITE")
            view = view[written:]
        os.fsync(fd)
    finally:
        os.close(fd)


def _stage_directory(daily, release_fd):
    deploy_fd = os.open("deploy", daily.DIR_FLAGS, dir_fd=release_fd)
    try:
        daily.directory_info(deploy_fd, uid=0, gid=0)
        deploy_identity = os.fstat(deploy_fd)
        os.mkdir("owner-beta-paper", 0o700, dir_fd=deploy_fd)
        os.fsync(deploy_fd)
        directory_fd = os.open("owner-beta-paper", daily.DIR_FLAGS, dir_fd=deploy_fd)
        daily.directory_info(directory_fd, uid=0, gid=0, mode=0o700)
        return deploy_fd, directory_fd, deploy_identity
    except Exception:
        os.close(deploy_fd)
        raise


def _capability_files(capability):
    return {
        "capability.json": _canonical(capability),
        "operations.schema.json": None,
        "archive.schema.json": None,
        "migration-0068.up.sql": None,
        "audit.sql": None,
    }


@dataclasses.dataclass(frozen=True, init=False)
class VerifiedPaperRelease:
    """Verified release token. Its constructor is private to this module."""
    install_root: str
    target_release: str
    commit: str
    manifest_sha256: str
    capability_sha256: str
    install_receipt_sha256: str
    process_sha256: str | None
    _identity: tuple[int, int]
    _fds: tuple[int, ...]
    _names: tuple[str, ...]
    _identities: tuple[tuple[int, int, int, int, int], ...]
    _release_fd_index: int

    def __init__(self, token, **values):
        if token is not _VERIFIED:
            raise TypeError("VerifiedPaperRelease has a private constructor")
        for key, value in values.items():
            object.__setattr__(self, key, value)

    def revalidate(self):
        """Recheck retained directory descriptors and their named bindings."""
        daily = _daily()
        if not self._fds:
            _fail("PAPER_RELEASE_CLOSED")
        if len(self._fds) != len(self._names) + 1 or len(self._fds) != len(self._identities):
            _fail("PAPER_RELEASE_CHANGED")
        for index, fd in enumerate(self._fds):
            info = daily.directory_info(fd, uid=0, gid=(10001 if index == len(self._fds) - 1 else None))
            ident = (info.st_dev, info.st_ino, info.st_uid, info.st_gid, info.st_mode)
            if ident != self._identities[index]:
                _fail("PAPER_RELEASE_CHANGED")
            if index:
                named = os.stat(self._names[index - 1], dir_fd=self._fds[index - 1], follow_symlinks=False)
                if (named.st_dev, named.st_ino) != (info.st_dev, info.st_ino):
                    _fail("PAPER_RELEASE_CHANGED")
        release_fd = self._fds[self._release_fd_index]
        stage_fd = self._fds[-1]
        manifest = daily.read_file(release_fd, MANIFEST_NAME, uid=0, gid=0, mode=0o600,
                                   maximum=MAX_MANIFEST)
        capability_data = daily.read_file(stage_fd, "capability.json", uid=0, gid=0,
                                           mode=0o444, maximum=MAX_PROCESS)
        capability = _json(capability_data, MAX_PROCESS)
        if (_digest(manifest) != self.manifest_sha256
                or _digest(capability_data) != self.capability_sha256
                or _canonical(capability) != capability_data
                or capability.get("release_manifest_sha256") != self.manifest_sha256
                or capability.get("code_commit") != self.commit):
            _fail("PAPER_RELEASE_CHANGED")
        for key, name in (("operations_schema_sha256", "operations.schema.json"),
                          ("archive_schema_sha256", "archive.schema.json"),
                          ("migration_0068_sha256", "migration-0068.up.sql"),
                          ("audit_sql_sha256", "audit.sql")):
            source = daily.read_file(stage_fd, name, uid=0, gid=0, mode=0o444,
                                     maximum=MAX_SOURCE)
            if _digest(source) != capability.get(key):
                _fail("PAPER_RELEASE_CHANGED")
        for key, name in (("api_binary_sha256", "api-image.elf"),
                          ("worker_binary_sha256", "worker-image.elf"),
                          ("native_binary_sha256", "native-image.elf")):
            mode = 0o500 if name == "native-image.elf" else 0o400
            binary = daily.read_file(stage_fd, name, uid=0, gid=0, mode=mode,
                                     maximum=128 * 1024 * 1024)
            if (_digest(binary) != capability.get(key)
                    or not _compiled_commit_in_elf(binary, self.commit)):
                _fail("PAPER_RELEASE_CHANGED")
        projection = _read_source(daily, release_fd, PROJECTION)
        receipt_sha = _verify_install_receipt(
            daily, release_fd, self.commit, self.manifest_sha256,
            self.capability_sha256, _digest(projection), self.process_sha256)
        if receipt_sha != self.install_receipt_sha256:
            _fail("PAPER_RELEASE_CHANGED")
        if self.process_sha256 is not None:
            process = daily.read_file(stage_fd, "process.json", uid=0, gid=10001,
                                      mode=0o440, maximum=MAX_PROCESS)
            alias = daily.read_file(stage_fd, "config.json", uid=0, gid=10001,
                                    mode=0o440, maximum=MAX_PROCESS)
            if _digest(process) != self.process_sha256 or process != alias:
                _fail("PAPER_RELEASE_CHANGED")
            financial_contract = _read_source(daily, release_fd, FINANCIAL_CONTRACT)
            _validate_process(_json(process, MAX_PROCESS), self.commit,
                              self.manifest_sha256, _digest(financial_contract))
        return None

    def stage_native_inputs(self, operation_directory_fd):
        """Copy verified read-only native inputs into a caller-owned root0700 operation dir.

        The P4 caller retains its existing release lock. Files are created
        no-replace and remain root:root 0400 for the P2 fixed-input ABI.
        """
        self.revalidate()
        daily = _daily()
        daily.directory_info(operation_directory_fd, uid=0, gid=0, mode=0o700)
        stage_fd = self._fds[-1]
        sources = (
            ("capability.json", 0o444),
            ("api-image.elf", 0o400),
            ("worker-image.elf", 0o400),
            ("operations.schema.json", 0o444),
            ("archive.schema.json", 0o444),
            ("migration-0068.up.sql", 0o444),
            ("audit.sql", 0o444),
        )
        for name, mode in sources:
            data = daily.read_file(stage_fd, name, uid=0, gid=0, mode=mode,
                                  maximum=(128 * 1024 * 1024 if name.endswith(".elf") else MAX_SOURCE))
            daily.create_file(operation_directory_fd, name, data, 0, 0, 0o400)
        os.fsync(operation_directory_fd)
        self.revalidate()
        return None

    def close(self):
        descriptors = getattr(self, "_fds", ())
        object.__setattr__(self, "_fds", ())
        for fd in reversed(descriptors):
            try:
                os.close(fd)
            except OSError:
                pass

    def __del__(self):
        self.close()

    def __enter__(self):
        self.revalidate()
        return self

    def __exit__(self, *_exc):
        self.close()


_VERIFIED = object()


def _release_path(install_root, target_release, commit, *, staging=False):
    if (not isinstance(install_root, str) or not isinstance(target_release, str)
            or not _is_commit(commit) or not os.path.isabs(install_root)
            or os.path.normpath(install_root) != install_root or install_root.endswith("/")):
        _fail("PAPER_RELEASE_PATH")
    valid_target = target_release == install_root + "/releases/" + commit
    if staging:
        prefix = install_root + "/releases/.staging." + commit + "."
        valid_target = valid_target or (isinstance(target_release, str) and target_release.startswith(prefix)
                                        and "/" not in target_release[len(prefix):])
    if not valid_target:
        _fail("PAPER_RELEASE_PATH")


def _binding_files(process_binding):
    if process_binding is None:
        return None
    if type(process_binding) is bytes:
        value = _json(process_binding, MAX_PROCESS)
        if _canonical(value) != process_binding:
            _fail("PAPER_RELEASE_PROCESS")
    elif type(process_binding) is dict:
        value = process_binding
    else:
        _fail("PAPER_RELEASE_PROCESS")
    return value


@_boundary
def stage(install_root, target_release, commit, manifest_sha256, process_binding):
    """Stage capability/source files after validating final image and source bytes.

    `process_binding=None` prepares an OFF reader only. A supplied binding is
    copied as prepared config and remains inactive until separate operations.
    """
    _release_path(install_root, target_release, commit, staging=True)
    if not target_release.startswith(install_root + "/releases/.staging." + commit + "."):
        _fail("PAPER_RELEASE_IMMUTABLE")
    if type(manifest_sha256) is not str or SHA.fullmatch(manifest_sha256) is None:
        _fail("PAPER_RELEASE_MANIFEST")
    daily = _daily()
    daily.require_root()
    binding = _binding_files(process_binding)
    with daily.root_directory(install_root) as install_fd:
        with daily.root_directory(target_release) as release_fd:
            manifest_data = daily.read_file(release_fd, MANIFEST_NAME, uid=0, gid=0,
                                            mode=0o600, maximum=MAX_MANIFEST)
            images = _manifest(manifest_data, commit, manifest_sha256)
            sources, projection = _source_values(daily, release_fd)
            actual = _actual_elfs(images, commit)
            digests = {key: _digest(data) for key, data in sources.items()}
            capability = {
                "schema": CAPABILITY_SCHEMA, "minimum": 1, "code_commit": commit,
                "release_manifest_sha256": manifest_sha256,
                **digests,
                **{key: actual[key][1] for key in actual},
            }
            if set(capability) != CAPABILITY_KEYS:
                _fail("PAPER_RELEASE_CAPABILITY")
            if binding is not None:
                financial_contract = _read_source(daily, release_fd, FINANCIAL_CONTRACT)
                binding = _validate_process(binding, commit, manifest_sha256,
                                            _digest(financial_contract))
                if (binding["api_binary_sha256"] != capability["api_binary_sha256"]
                        or binding["worker_binary_sha256"] != capability["worker_binary_sha256"]):
                    _fail("PAPER_RELEASE_PROCESS")
            # Create the immutable subtree only after every input has passed.
            deploy_fd, paper_fd, deploy_identity = _stage_directory(daily, release_fd)
            try:
                files = {
                    "capability.json": _canonical(capability),
                    "operations.schema.json": sources["operations_schema_sha256"],
                    "archive.schema.json": sources["archive_schema_sha256"],
                    "migration-0068.up.sql": sources["migration_0068_sha256"],
                    "audit.sql": sources["audit_sql_sha256"],
                }
                for name, data in files.items():
                    _write_new_file(paper_fd, name, data)
                for name, key in (("api-image.elf", "api_binary_sha256"),
                                  ("worker-image.elf", "worker_binary_sha256")):
                    _write_new_file(paper_fd, name, actual[key][0], mode=0o400)
                _write_new_file(paper_fd, "native-image.elf",
                                actual["native_binary_sha256"][0], mode=0o500)
                if binding is not None:
                    process_bytes = _canonical(binding)
                    _write_new_file(paper_fd, "process.json", process_bytes, 0, 10001, 0o440)
                    # The immutable directory is mounted read-only at the
                    # runtime alias required by the Rust reader.
                    _write_new_file(paper_fd, "config.json", process_bytes, 0, 10001, 0o440)
                os.fchown(paper_fd, 0, 10001)
                os.fchmod(paper_fd, 0o750)
                daily.directory_info(paper_fd, uid=0, gid=10001, mode=0o750)
                after = os.fstat(paper_fd)
                named = os.stat("owner-beta-paper", dir_fd=deploy_fd, follow_symlinks=False)
                if (after.st_dev, after.st_ino) != (named.st_dev, named.st_ino):
                    _fail("PAPER_RELEASE_CHANGED")
                os.fsync(paper_fd)
                os.fsync(deploy_fd)
                os.fsync(release_fd)
                deploy_after = os.fstat(deploy_fd)
                deploy_named = os.stat("deploy", dir_fd=release_fd, follow_symlinks=False)
                if ((deploy_identity.st_dev, deploy_identity.st_ino) !=
                        (deploy_after.st_dev, deploy_after.st_ino) or
                        (deploy_after.st_dev, deploy_after.st_ino) !=
                        (deploy_named.st_dev, deploy_named.st_ino)):
                    _fail("PAPER_RELEASE_CHANGED")
            except Exception:
                # Never remove a partially created immutable stage. It is a
                # failed release tree and must be discarded before installation.
                raise
            finally:
                os.close(paper_fd)
                os.close(deploy_fd)
            capability_bytes = _canonical(capability)
            process_sha256 = None if binding is None else _digest(_canonical(binding))
            receipt_bytes = _install_receipt(
                commit, manifest_sha256, _digest(capability_bytes), _digest(projection),
                process_sha256)
            _write_new_file(release_fd, INSTALL_RECEIPT_NAME, receipt_bytes,
                            uid=0, gid=0, mode=0o600)
            os.fsync(release_fd)
            return capability


def _read_stage(daily, release_fd, commit, manifest_sha256):
    projection = _read_source(daily, release_fd, PROJECTION)
    if _json(projection, MAX_SOURCE).get("$id") != PROJECTION_ID:
        _fail("PAPER_RELEASE_SOURCE")
    deploy_fd = os.open("deploy", daily.DIR_FLAGS, dir_fd=release_fd)
    try:
        daily.directory_info(deploy_fd, uid=0, gid=0)
        paper_fd = os.open("owner-beta-paper", daily.DIR_FLAGS, dir_fd=deploy_fd)
        try:
            daily.directory_info(paper_fd, uid=0, gid=10001, mode=0o750)
            base_names = {"capability.json", "operations.schema.json", "archive.schema.json",
                          "migration-0068.up.sql", "audit.sql", "api-image.elf",
                          "worker-image.elf", "native-image.elf"}
            names = set(os.listdir(paper_fd))
            if names not in (base_names, base_names | {"process.json", "config.json"}):
                _fail("PAPER_RELEASE_FILES")
            capability_data = daily.read_file(paper_fd, "capability.json", uid=0, gid=0,
                                              mode=0o444, maximum=MAX_PROCESS)
            capability = _json(capability_data, MAX_PROCESS)
            if (_canonical(capability) != capability_data or set(capability) != CAPABILITY_KEYS
                    or capability["schema"] != CAPABILITY_SCHEMA or capability["minimum"] != 1):
                _fail("PAPER_RELEASE_CAPABILITY")
            if (capability["code_commit"] != commit or capability["release_manifest_sha256"] != manifest_sha256
                    or any(type(capability[k]) is not str or SHA.fullmatch(capability[k]) is None
                           for k in CAPABILITY_KEYS - {"schema", "minimum", "code_commit"})):
                _fail("PAPER_RELEASE_CAPABILITY")
            source_names = {
                "operations_schema_sha256": "operations.schema.json",
                "archive_schema_sha256": "archive.schema.json",
                "migration_0068_sha256": "migration-0068.up.sql", "audit_sql_sha256": "audit.sql",
            }
            for key, name in source_names.items():
                data = daily.read_file(paper_fd, name, uid=0, gid=0, mode=0o444, maximum=MAX_SOURCE)
                if _digest(data) != capability[key]:
                    _fail("PAPER_RELEASE_SOURCE")
            for key, name in (("api_binary_sha256", "api-image.elf"),
                              ("worker_binary_sha256", "worker-image.elf"),
                              ("native_binary_sha256", "native-image.elf")):
                mode = 0o500 if name == "native-image.elf" else 0o400
                data = daily.read_file(paper_fd, name, uid=0, gid=0, mode=mode,
                                       maximum=128 * 1024 * 1024)
                if (_digest(data) != capability[key]
                        or not _compiled_commit_in_elf(data, commit)):
                    _fail("PAPER_RELEASE_ELF")
            process_sha = None
            if "process.json" in names:
                data = daily.read_file(paper_fd, "process.json", uid=0, gid=10001,
                                       mode=0o440, maximum=MAX_PROCESS)
                if _canonical(_json(data, MAX_PROCESS)) != data:
                    _fail("PAPER_RELEASE_PROCESS")
                if daily.read_file(paper_fd, "config.json", uid=0, gid=10001,
                                   mode=0o440, maximum=MAX_PROCESS) != data:
                    _fail("PAPER_RELEASE_PROCESS")
                financial_contract = _read_source(daily, release_fd, FINANCIAL_CONTRACT)
                process = _validate_process(_json(data, MAX_PROCESS), commit, manifest_sha256,
                                            _digest(financial_contract))
                if (process["api_binary_sha256"] != capability["api_binary_sha256"]
                        or process["worker_binary_sha256"] != capability["worker_binary_sha256"]):
                    _fail("PAPER_RELEASE_PROCESS")
                process_sha = _digest(data)
            return (capability, _digest(capability_data), process_sha,
                    (os.fstat(paper_fd).st_dev, os.fstat(paper_fd).st_ino),
                    _digest(projection))
        finally:
            os.close(paper_fd)
    finally:
        os.close(deploy_fd)


@_boundary
def _open_verified_chain(daily, target_release):
    descriptors = [os.open("/", daily.DIR_FLAGS)]
    names = []
    identities = []
    try:
        identities.append(_directory_identity(daily, descriptors[0], gid=None))
        for part in target_release.split("/")[1:]:
            child = os.open(part, daily.DIR_FLAGS, dir_fd=descriptors[-1])
            descriptors.append(child)
            names.append(part)
            identities.append(_directory_identity(daily, child, gid=None))
            named = os.stat(part, dir_fd=descriptors[-2], follow_symlinks=False)
            if (named.st_dev, named.st_ino) != (identities[-1][0], identities[-1][1]):
                _fail("PAPER_RELEASE_CHANGED")
        return descriptors, names, identities
    except Exception:
        for fd in reversed(descriptors):
            os.close(fd)
        raise


def _directory_identity(daily, fd, gid=None):
    value = daily.directory_info(fd, uid=0, gid=gid)
    return (value.st_dev, value.st_ino, value.st_uid, value.st_gid, value.st_mode)


@_boundary
def verify(install_root, target_release, commit):
    """Verify an installed immutable Paper capability and return an opaque handle."""
    _release_path(install_root, target_release, commit)
    daily = _daily()
    daily.require_root()
    descriptors, names, identities = _open_verified_chain(daily, target_release)
    try:
        release_fd = descriptors[-1]
        release_fd_index = len(descriptors) - 1
        manifest_data = daily.read_file(release_fd, MANIFEST_NAME, uid=0, gid=0,
                                        mode=0o600, maximum=MAX_MANIFEST)
        manifest_sha = _digest(manifest_data)
        images = _manifest(manifest_data, commit, manifest_sha)
        capability, capability_sha, process_sha, identity, projection_sha = _read_stage(
            daily, release_fd, commit, manifest_sha)
        install_receipt_sha = _verify_install_receipt(
            daily, release_fd, commit, manifest_sha, capability_sha, projection_sha, process_sha)
        actual = _actual_elfs(images, commit)
        for key, (_, digest) in actual.items():
            if capability[key] != digest:
                _fail("PAPER_RELEASE_ELF")
        deploy_fd = os.open("deploy", daily.DIR_FLAGS, dir_fd=release_fd)
        descriptors.append(deploy_fd)
        names.append("deploy")
        identities.append(_directory_identity(daily, deploy_fd, gid=None))
        if (os.stat("deploy", dir_fd=release_fd, follow_symlinks=False).st_ino != identities[-1][1]
                or os.stat("deploy", dir_fd=release_fd, follow_symlinks=False).st_dev != identities[-1][0]):
            _fail("PAPER_RELEASE_CHANGED")
        stage_fd = os.open("owner-beta-paper", daily.DIR_FLAGS, dir_fd=deploy_fd)
        descriptors.append(stage_fd)
        names.append("owner-beta-paper")
        identities.append(_directory_identity(daily, stage_fd, gid=10001))
        named = os.stat("owner-beta-paper", dir_fd=deploy_fd, follow_symlinks=False)
        if (named.st_dev, named.st_ino) != (identities[-1][0], identities[-1][1]):
            _fail("PAPER_RELEASE_CHANGED")
        try:
            for key, path in PATHS.items():
                source = _read_source(daily, release_fd, path)
                if _digest(source) != capability[key]:
                    _fail("PAPER_RELEASE_SOURCE")
            if identity != (os.fstat(stage_fd).st_dev, os.fstat(stage_fd).st_ino):
                _fail("PAPER_RELEASE_CHANGED")
            handle = VerifiedPaperRelease(_VERIFIED, install_root=install_root,
                                          target_release=target_release, commit=commit,
                                          manifest_sha256=manifest_sha, capability_sha256=capability_sha,
                                          install_receipt_sha256=install_receipt_sha,
                                          process_sha256=process_sha, _identity=identity,
                                          _fds=tuple(descriptors), _names=tuple(names),
                                          _identities=tuple(identities),
                                          _release_fd_index=release_fd_index)
            handle.revalidate()
            descriptors = []
            return handle
        except Exception:
            raise
    finally:
        for fd in reversed(descriptors):
            os.close(fd)


def _floor_read(daily, install_fd):
    try:
        data = daily.read_file(install_fd, FLOOR_NAME, uid=0, gid=0, mode=0o600,
                               maximum=len(FLOOR_BYTES))
    except FileNotFoundError:
        return False
    if data != FLOOR_BYTES:
        _fail("PAPER_RELEASE_FLOOR")
    return True


def _target_has_process_binding(daily, target_release):
    with daily.root_directory(target_release) as release_fd:
        try:
            deploy_fd = os.open("deploy", daily.DIR_FLAGS, dir_fd=release_fd)
        except FileNotFoundError:
            return False
        try:
            daily.directory_info(deploy_fd, uid=0, gid=0)
            try:
                paper_fd = os.open("owner-beta-paper", daily.DIR_FLAGS, dir_fd=deploy_fd)
            except FileNotFoundError:
                return False
            try:
                daily.directory_info(paper_fd, uid=0, gid=10001, mode=0o750)
                names = set(os.listdir(paper_fd))
                return bool(names & {"process.json", "config.json"})
            finally:
                os.close(paper_fd)
        finally:
            os.close(deploy_fd)


def _history_required(install_root, install_fd, history_present, target_has_config):
    if history_present is not None and type(history_present) is not bool:
        _fail("PAPER_RELEASE_HISTORY_UNKNOWN")
    # Caller-provided positive evidence is accepted; false is not authority to
    # ignore protected on-disk indicators. A target config is itself an ON
    # request binding and therefore requires the capable reader.
    if history_present is True or target_has_config:
        return True
    for name in (".owner-beta-paper",):
        try:
            info = os.stat(name, dir_fd=install_fd, follow_symlinks=False)
        except FileNotFoundError:
            continue
        if not stat.S_ISDIR(info.st_mode):
            _fail("PAPER_RELEASE_HISTORY_UNKNOWN")
        daily = _daily()
        fd = os.open(name, daily.DIR_FLAGS, dir_fd=install_fd)
        try:
            opened = daily.directory_info(fd, uid=0, gid=0, mode=0o700)
            named = os.stat(name, dir_fd=install_fd, follow_symlinks=False)
            if (opened.st_dev, opened.st_ino) != (named.st_dev, named.st_ino):
                _fail("PAPER_RELEASE_HISTORY_UNKNOWN")
        finally:
            os.close(fd)
        return True
    if history_present is None:
        _fail("PAPER_RELEASE_HISTORY_UNKNOWN")
    # P3 has no protected database inventory ABI yet. A bare False (including
    # one supplied by a shell caller) cannot prove that Paper tables/journal
    # are absent. P4 may open this edge only with its retained inventory proof.
    _fail("PAPER_RELEASE_HISTORY_UNKNOWN")


def _reserve_floor(daily, install_fd):
    try:
        daily.create_file(install_fd, FLOOR_NAME, FLOOR_BYTES, 0, 0, 0o600)
    except FileExistsError:
        if _floor_read(daily, install_fd) is not True:
            _fail("PAPER_RELEASE_FLOOR")
    os.fsync(install_fd)


@_boundary
def reader_guard(install_root, target_release, commit, history_present, reserve=True):
    """Enforce durable Paper, Daily and simulation reader floors before switch."""
    _release_path(install_root, target_release, commit)
    if (history_present is not None and type(history_present) is not bool) or type(reserve) is not bool:
        _fail("PAPER_RELEASE_HISTORY_UNKNOWN")
    daily = _daily()
    daily.require_root()
    with daily.root_directory(install_root) as install_fd:
        paper_floor = _floor_read(daily, install_fd)
        target_has_config = _target_has_process_binding(daily, target_release)
        required = paper_floor or _history_required(
            install_root, install_fd, history_present, target_has_config)
        # Database history/control/journal evidence is inventoried by P4. Until
        # then absent floor/config markers are UNKNOWN, never an OFF assertion.
        release = verify(install_root, target_release, commit) if required else None
        if required and release is None:
            _fail("PAPER_RELEASE_READER_REQUIRED")
        if required and reserve and not paper_floor:
            _reserve_floor(daily, install_fd)
        # Existing floors are conjunctive. Never reserve/replace the other
        # reader floors here; ask their existing guards to validate target.
        sim_path = os.path.join(install_root, ".owner-beta-simulation-reader-floor")
        daily_path = os.path.join(install_root, ".owner-beta-daily-reader-floor")
        if os.path.lexists(sim_path):
            sim = _simulation_release()
            sim.reader_guard(install_root, target_release, commit, "disabled",
                            history_present=True, reserve=False)
        if os.path.lexists(daily_path):
            daily_module = _daily_operations()
            daily_module.install.reader_guard(install_root, target_release, "disabled", reserve=False)
        if release is not None:
            release.close()
        return "PAPER_READER_REQUIRED" if required else "LEGACY_OFF"


def _simulation_release():
    path = os.path.join(os.path.dirname(__file__), "owner-beta-simulation-release.py")
    spec = importlib.util.spec_from_file_location("_lagrange_paper_sim_release", path)
    if spec is None or spec.loader is None:
        _fail("PAPER_RELEASE_DEPENDENCY")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    try:
        spec.loader.exec_module(module)
    except Exception:
        _fail("PAPER_RELEASE_DEPENDENCY")
    return module


def _daily_operations():
    path = os.path.join(os.path.dirname(__file__), "owner-beta-daily-operations.py")
    spec = importlib.util.spec_from_file_location("_lagrange_paper_daily_ops", path)
    if spec is None or spec.loader is None:
        _fail("PAPER_RELEASE_DEPENDENCY")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    try:
        spec.loader.exec_module(module)
    except Exception:
        _fail("PAPER_RELEASE_DEPENDENCY")
    return module


def main(argv=None):
    import argparse

    parser = argparse.ArgumentParser(prog="owner-beta-paper-release.py")
    commands = parser.add_subparsers(dest="command", required=True)
    stage_parser = commands.add_parser("stage")
    stage_parser.add_argument("--install-root", required=True)
    stage_parser.add_argument("--target-release", required=True)
    stage_parser.add_argument("--commit", required=True)
    stage_parser.add_argument("--manifest-sha256", required=True)
    verify_parser = commands.add_parser("verify")
    verify_parser.add_argument("--install-root", required=True)
    verify_parser.add_argument("--target-release", required=True)
    verify_parser.add_argument("--commit", required=True)
    guard_parser = commands.add_parser("reader-guard")
    guard_parser.add_argument("--install-root", required=True)
    guard_parser.add_argument("--target-release", required=True)
    guard_parser.add_argument("--commit", required=True)
    guard_parser.add_argument("--history-present", action="store_true",
                              help="protected inventory found prior Paper history/config/control/journal")
    guard_parser.add_argument("--no-reserve", action="store_true")
    try:
        args = parser.parse_args(argv)
        if args.command == "stage":
            stage(args.install_root, args.target_release, args.commit,
                  args.manifest_sha256, None)
        elif args.command == "verify":
            verify(args.install_root, args.target_release, args.commit)
        else:
            # This release CLI cannot prove database absence. Without positive
            # evidence, preserve UNKNOWN and fail closed; P4 supplies the exact
            # protected DB/history inventory before this edge can open.
            history_present = True if args.history_present else None
            reader_guard(args.install_root, args.target_release, args.commit,
                         history_present, reserve=not args.no_reserve)
        print("PAPER_RELEASE_OK")
        return 0
    except PaperReleaseError as error:
        print(error.code, file=sys.stderr)
        return 1
    except Exception:
        print("PAPER_RELEASE_INVALID", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
