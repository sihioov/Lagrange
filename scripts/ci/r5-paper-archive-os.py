#!/usr/bin/env python3
"""Required hosted archive filesystem fixtures; no public target/path selection."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import stat
import sys
import types

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/ci/r5-paper-archive-os.py"
HELPER = ROOT / "scripts/ci/r5-paper-native-os.py"
HELPER_SHA256 = "c35ecc989c6cf3929abcef1d8fde877a8e163b663d1c25a085af781e9ff4507a"
ARCHIVE_SOURCE = ROOT / "crates/market-data/src/paper_archive_sources/closure/tests.rs"
ARCHIVE_SOURCE_SHA256 = "fa691380798293dfd7706fb17b9f6c3928f081988489da360537888526d88ff6"
TESTS = ('paper_archive_sources::closure::tests::actual_root_codec_original_and_all_twenty_classes_match', 'paper_archive_sources::closure::tests::actual_root_same_bytes_named_replacement_is_not_the_original', 'paper_archive_sources::closure::tests::actual_root_omitted_pending_or_orphan_inventory_is_rejected', 'paper_archive_sources::closure::tests::actual_root_parent_replacement_is_checked_before_original_read', 'paper_archive_sources::closure::tests::actual_root_callback_error_cancellation_and_changed_provisional_bytes', 'paper_archive_sources::closure::tests::actual_root_codec_named_replacement_and_in_place_rewrite_are_rejected', 'paper_archive_sources::closure::tests::actual_root_wrong_mode_owner_hardlink_and_symlink_are_rejected', 'paper_archive_sources::closure::tests::actual_root_acl_is_rejected_even_with_unchanged_owner', 'paper_archive_sources::closure::tests::actual_root_callback_unwind_poison_and_all_owned_closes_remain_visible', 'paper_archive_sources::closure::tests::actual_root_late_close_failure_invalidates_provisional_member_and_retires_all_owners', 'paper_archive_sources::closure::tests::actual_root_old_v2_and_current_v3_require_their_own_retained_registry_original', 'paper_archive_sources::closure::tests::actual_root_retained_parquet_uses_existing_original_and_checked_close', 'paper_archive_sources::closure::tests::actual_root_retained_parquet_replacement_during_borrow_invalidates_values', 'paper_archive_sources::closure::tests::actual_root_retained_parquet_cancel_and_late_owner_close_failures_retire_every_owner', 'paper_archive_sources::closure::tests::actual_root_retained_parquet_callback_unwind_keeps_attempt_poisoned')
PREFIX = "lagrange-p5-source-consumer-test-"
ANCESTRY = (Path("/"), Path("/var"), Path("/var/lib"))
_PINNED_SOURCES = None
_PINNED_ELF_METADATA = None


def _source_identity(st):
    return {"device": st.st_dev, "inode": st.st_ino, "size": st.st_size,
            "mode": stat.S_IMODE(st.st_mode), "uid": st.st_uid, "gid": st.st_gid,
            "links": st.st_nlink, "mtime_ns": st.st_mtime_ns, "ctime_ns": st.st_ctime_ns}


def _read_source(path, expected_hash=None, owner=None, *, local_helper_import=False):
    """Use the original FD bytes for import, never hash then reopen executable code."""
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0))
    try:
        before = os.fstat(fd)
        allow_local = (local_helper_import and path == HELPER and os.geteuid() != 0
                       and os.environ.get("GITHUB_ACTIONS") != "true")
        if (not stat.S_ISREG(before.st_mode) or before.st_nlink != 1
                or not 0 < before.st_size <= 1024 * 1024
                or (before.st_mode & 0o022 and not allow_local)
                or (owner is not None and before.st_uid != owner)):
            raise ValueError("ARCHIVE_SOURCE_METADATA_INVALID")
        chunks = []
        remaining = before.st_size
        while remaining:
            block = os.read(fd, min(65536, remaining))
            if not block:
                raise ValueError("ARCHIVE_SOURCE_IDENTITY_DRIFT")
            chunks.append(block)
            remaining -= len(block)
        if os.read(fd, 1):
            raise ValueError("ARCHIVE_SOURCE_IDENTITY_DRIFT")
        data = b"".join(chunks)
        digest = hashlib.sha256(data).hexdigest()
        identity = _source_identity(before)
        if (identity != _source_identity(os.fstat(fd))
                or identity != _source_identity(os.stat(path, follow_symlinks=False))):
            raise ValueError("ARCHIVE_SOURCE_IDENTITY_DRIFT")
        if expected_hash is not None and digest != expected_hash:
            raise ValueError("ARCHIVE_SOURCE_CHECKPOINT_MISMATCH")
        return data, identity, digest
    finally:
        os.close(fd)


_helper_bytes, _HELPER_IDENTITY, _ = _read_source(HELPER, HELPER_SHA256, local_helper_import=True)
# Pinned original bytes are compiled once. No loader reopens a mutable helper.
native = types.ModuleType("r5_paper_archive_pinned_helper")
native.__file__ = str(HELPER)
sys.modules[native.__name__] = native
exec(compile(_helper_bytes, str(HELPER), "exec"), native.__dict__)
del _helper_bytes

ARCHIVE_ERRORS = frozenset({
    "ARCHIVE_SOURCE_METADATA_INVALID", "ARCHIVE_SOURCE_IDENTITY_DRIFT",
    "ARCHIVE_SOURCE_CHECKPOINT_MISMATCH", "ARCHIVE_ANCESTRY_INVALID",
    "ARCHIVE_ANCESTRY_IDENTITY_DRIFT", "ARCHIVE_NAMESPACE_NOT_EMPTY",
    "ARCHIVE_ROOT_UNEXPECTED_FAILURE",
})


def _source_pins():
    raw_uid = os.environ.get("SUDO_UID") if os.geteuid() == 0 else str(os.geteuid())
    if type(raw_uid) is not str or not raw_uid.isascii() or not raw_uid.isdigit():
        raise native.RunnerError("ROOT_DRIVER_PIN_INVALID")
    owner = int(raw_uid)
    result = []
    for path, expected in ((SCRIPT, None), (HELPER, HELPER_SHA256),
                           (ARCHIVE_SOURCE, ARCHIVE_SOURCE_SHA256)):
        native._no_symlink_path(path, start=ROOT)
        try:
            _, identity, digest = _read_source(path, expected, owner)
        except (OSError, ValueError) as exc:
            code = exc.args[0] if (type(exc) is ValueError and exc.args
                                  and type(exc.args[0]) is str and exc.args[0] in ARCHIVE_ERRORS) else "ARCHIVE_SOURCE_METADATA_INVALID"
            raise native.RunnerError(code) from None
        if path == HELPER and identity != _HELPER_IDENTITY:
            raise native.RunnerError("ARCHIVE_SOURCE_IDENTITY_DRIFT")
        result.append({"path": path.relative_to(ROOT).as_posix(),
                       "sha256": digest, "identity": identity})
    return result


def verify_archive_sources():
    global _PINNED_SOURCES
    current = _source_pins()
    if _PINNED_SOURCES is None:
        _PINNED_SOURCES = current
    elif current != _PINNED_SOURCES:
        raise native.RunnerError("ARCHIVE_SOURCE_IDENTITY_DRIFT")


def encode_identity(elf_identity):
    verify_archive_sources()
    if _PINNED_ELF_METADATA is None:
        raise native.RunnerError("TEST_ARTIFACT_IDENTITY_DRIFT")
    return {"elf": elf_identity, "elf_metadata": _PINNED_ELF_METADATA, "sources": _PINNED_SOURCES}


def decode_identity(packet):
    global _PINNED_SOURCES, _PINNED_ELF_METADATA
    if (type(packet) is not dict or set(packet) != {"elf", "elf_metadata", "sources"}
            or type(packet["elf"]) is not dict or type(packet["sources"]) is not list):
        raise native.RunnerError("ROOT_DRIVER_PIN_INVALID")
    metadata = packet["elf_metadata"]
    if (type(metadata) is not dict or set(metadata) != {"mtime_ns", "ctime_ns"}
            or any(type(v) is not int or v < 0 for v in metadata.values())):
        raise native.RunnerError("ROOT_DRIVER_PIN_INVALID")
    identity_keys = {"device", "inode", "size", "mode", "uid", "gid", "links", "mtime_ns", "ctime_ns"}
    for item in packet["sources"]:
        if (type(item) is not dict or set(item) != {"path", "sha256", "identity"}
                or type(item["path"]) is not str or type(item["sha256"]) is not str
                or native.HASH_RE.fullmatch(item["sha256"]) is None
                or type(item["identity"]) is not dict or set(item["identity"]) != identity_keys
                or any(type(v) is not int or v < 0 for v in item["identity"].values())):
            raise native.RunnerError("ROOT_DRIVER_PIN_INVALID")
    current = _source_pins()
    # Compare the whole fixed-path/typed identity packet; do not open input paths.
    if packet["sources"] != current:
        raise native.RunnerError("ARCHIVE_SOURCE_IDENTITY_DRIFT")
    _PINNED_SOURCES = current
    _PINNED_ELF_METADATA = metadata
    return packet["elf"]


_open_native_elf = native.open_pinned_elf


def open_archive_elf(path, target, expected_hash, expected_identity=None):
    """Add retained timestamp metadata without changing native7's identity schema."""
    global _PINNED_ELF_METADATA
    native._no_symlink_path(path, start=target)
    before = os.stat(path, follow_symlinks=False)
    metadata = {"mtime_ns": before.st_mtime_ns, "ctime_ns": before.st_ctime_ns}
    if _PINNED_ELF_METADATA is not None and metadata != _PINNED_ELF_METADATA:
        raise native.RunnerError("TEST_ARTIFACT_IDENTITY_DRIFT")
    fd, identity = _open_native_elf(path, target, expected_hash, expected_identity)
    try:
        after = os.fstat(fd)
        named = os.stat(path, follow_symlinks=False)
        if any({"mtime_ns": st.st_mtime_ns, "ctime_ns": st.st_ctime_ns} != metadata
               for st in (after, named)):
            raise native.RunnerError("TEST_ARTIFACT_IDENTITY_DRIFT")
        _PINNED_ELF_METADATA = metadata
        return fd, identity
    except BaseException:
        os.close(fd)
        raise


class ArchiveNamespace:
    """Observe an initially empty exact namespace. Never delete guessed ownership."""

    def __init__(self):
        self.fd = None
        self.held = []

    @staticmethod
    def directory_identity(st):
        return (st.st_dev, st.st_ino, stat.S_IMODE(st.st_mode), st.st_uid, st.st_gid, st.st_nlink)

    def _open_ancestry(self):
        for path in ANCESTRY:
            native._no_symlink_path(path)
            fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_NOFOLLOW", 0))
            try:
                identity = self.directory_identity(os.fstat(fd))
            except BaseException:
                os.close(fd)
                raise
            self.held.append((path, fd, identity))
        self.fd = self.held[-1][1]

    def _check(self):
        if self.fd is None or len(self.held) != len(ANCESTRY):
            raise native.RunnerError("ARCHIVE_ANCESTRY_INVALID")
        try:
            for path, fd, identity in self.held:
                actual = os.fstat(fd)
                named = os.stat(path, follow_symlinks=False)
                if (self.directory_identity(actual) != identity
                        or self.directory_identity(named) != identity):
                    raise native.RunnerError("ARCHIVE_ANCESTRY_IDENTITY_DRIFT")
                if (not stat.S_ISDIR(actual.st_mode) or actual.st_uid != 0 or actual.st_gid != 0
                        or actual.st_mode & 0o022 or not native._no_acl(fd)):
                    raise native.RunnerError("ARCHIVE_ANCESTRY_INVALID")
            if any(name.startswith(PREFIX) for name in os.listdir(self.fd)):
                raise native.RunnerError("ARCHIVE_NAMESPACE_NOT_EMPTY")
        except OSError:
            raise native.RunnerError("ARCHIVE_ANCESTRY_INVALID") from None

    def _close(self):
        failed = False
        try:
            for _, fd, _ in reversed(self.held):
                try:
                    os.close(fd)
                except OSError:
                    failed = True
        finally:
            self.fd = None
            self.held = []
        if failed:
            raise native.RunnerError("ARCHIVE_ANCESTRY_INVALID")

    def create(self):
        try:
            self._open_ancestry()
            self._check()
        except BaseException:
            self._close()
            raise

    def before_test(self):
        self._check()

    def after_test(self):
        self._check()

    def cleanup(self):
        try:
            self._check()
        finally:
            self._close()


native.SCRIPT = SCRIPT
native.PACKAGE_NAME = "market-data"
native.TARGET_NAME = "market_data"
native.LIB_SOURCE = ROOT / "crates/market-data/src/lib.rs"
native.TESTS = TESTS
native.EVIDENCE_LEAF = "r5-paper-archive-os"
native.RESULT_SCHEMA = "r5-paper-archive-os-result-v1"
native.REPORT_SCHEMA = "r5-paper-archive-os-report-v1"
native.REPORT_NOT_RUN = ()
native.REPORT_SCOPE = "fifteen ignored hosted archive filesystem tests; no DB, provider, Origin, service, genuine restore or production authority"
native.SOURCE_CHECKPOINT_VALIDATOR = verify_archive_sources
native.ROOT_IDENTITY_ENCODER = encode_identity
native.ROOT_IDENTITY_DECODER = decode_identity
native.FIXTURE_FACTORY = ArchiveNamespace
native.open_pinned_elf = open_archive_elf
native.STRICT_NAMED_RESULT = True
native.ROOT_UNEXPECTED_ERROR = "ARCHIVE_ROOT_UNEXPECTED_FAILURE"
native.ROOT_FAILURE_ERROR_CODES = native.ROOT_FAILURE_ERROR_CODES | ARCHIVE_ERRORS
native.ROOT_FAILURE_CLEANUP_CODES = native.ROOT_FAILURE_CLEANUP_CODES | ARCHIVE_ERRORS


if __name__ == "__main__":
    raise SystemExit(native.main())
