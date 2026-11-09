#!/usr/bin/env python3
"""Actual Linux-root descriptor fixtures; never run by ordinary unittest discovery.

Coordinator may register this script in the existing mandatory hosted Root
command. No DB, service, provider, lifecycle or synthetic source authority. The
fixture storage is disposable below protected /var/lib and always cleaned up.
"""
from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import stat
import struct
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("paper_references_pure_fixtures",
    Path(__file__).with_name("test_owner_beta_paper_backup_references.py"))
PURE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = PURE
SPEC.loader.exec_module(PURE)
REF = PURE.ref
FILES = REF.files


class ProtectedReferenceRootTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if sys.platform != "linux" or os.geteuid() != 0:
            raise RuntimeError("REFERENCES_TEST_REQUIRES_REAL_LINUX_ROOT")
        info = os.stat("/var/lib", follow_symlinks=False)
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != 0 or info.st_mode & 0o7022:
            raise RuntimeError("REFERENCES_TEST_REQUIRES_PROTECTED_VAR_LIB")

    def setUp(self):
        self.temp = Path(tempfile.mkdtemp(prefix="lagrange-paper-references-", dir="/var/lib"))
        os.chown(self.temp, 0, 0)
        os.chmod(self.temp, 0o700)
        self.addCleanup(shutil.rmtree, self.temp)
        self.source = self.temp / "source"
        self.source.mkdir(mode=0o700)
        self.roots = []
        for name in FILES.PAPER_CLASSES:
            path = self.source / name
            path.mkdir(mode=0o700)
            self.roots.append(FILES.Root(name, str(path), 0, 0, 0o700))
        self.roots = tuple(self.roots)
        self.limits = PURE.limits(entries=1000, manifest=2 * 1024 * 1024)

    def write(self, relative, data, mode=0o600, uid=0, gid=0):
        path = self.source / relative
        path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        # mkdir(parents=True) uses the process umask for intermediate parents;
        # explicitly bind every disposable fixture component before Snapshot.
        for parent in (path.parent, *path.parent.parents):
            if parent == self.source:
                break
            os.chown(parent, 0, 0)
            os.chmod(parent, 0o700)
        path.write_bytes(data)
        os.chown(path, uid, gid)
        os.chmod(path, mode)
        return path

    def raw(self):
        body = b"opaque fixture bytes\x00\xff"
        row = PURE.raw_batch(body=body)
        prefix = "raw/provider=kis/market=kr/date=2026-11-06/batch=" + PURE.A
        self.write(prefix + "/bars.json", body)
        self.write(prefix + "/batch.json", json.dumps(row, indent=2).encode() + b"\n")
        self.write("raw/manifests/provider=kis/market=kr/manifest.jsonl",
                   json.dumps(row, separators=(",", ":")).encode() + b"\n")
        return row, prefix

    def snapshot(self):
        return FILES.Snapshot(self.roots, self.limits, profile="paper-combined")

    def test_actual_calendar_original_and_decoder_rows_then_retained_mutation(self):
        fixture = PURE.daily_calendar_fixture()
        prefix = fixture["prefix"]
        body_path = self.write(prefix + "/calendar.json", fixture["body"])
        self.write(prefix + "/batch.json", fixture["batch_bytes"])
        self.write("raw/manifests/provider=kis/market=kr/manifest.jsonl", fixture["line"])
        with self.snapshot() as snapshot:
            protected = REF._Protected(snapshot)
            references = REF._References(self.limits, PURE.DOC)
            references.decode(io.BytesIO(PURE.rows_stream(PURE.calendar_rows(fixture))))
            raw = REF._scan_raw(protected, references.budget)
            self.assertEqual(len(REF._calendar_sources(protected, references, raw)), 1)
            REF._provisional_root(protected, references)
            body_path.write_bytes(fixture["body"].replace(b'"N"', b'"Y"'))
            with self.assertRaises(Exception):
                REF._calendar_sources(protected, references, raw)
            self.assertTrue(protected.failed)
            with self.assertRaises(REF.ReferenceError):
                protected.finish()

    def test_actual_equivalent_manifest_lines_preserve_first_source_identity(self):
        fixture = PURE.daily_calendar_fixture()
        prefix = fixture["prefix"]
        self.write(prefix + "/calendar.json", fixture["body"])
        self.write(prefix + "/batch.json", fixture["batch_bytes"])
        second = json.dumps(fixture["batch"], separators=(", ", ": ")).encode() + b"\n"
        self.write("raw/manifests/provider=kis/market=kr/manifest.jsonl", fixture["line"] + second)
        with self.snapshot() as snapshot:
            protected = REF._Protected(snapshot)
            raw = REF._scan_raw(protected, REF._Budget(self.limits))
            original = raw.reference(fixture["file"])
            self.assertEqual(original["manifest_lines"], [[fixture["file"]["manifest_line_sha256"], 1],
                ["sha256:" + hashlib.sha256(second[:-1]).hexdigest(), 2]])
            self.assertEqual(REF._file_evidence(fixture["file"], raw), fixture["file"])
            protected.finish()

    def test_actual_nested_daily_originals_and_proof_replacement(self):
        originals, manifest, admission, _ = PURE.nested_daily_fixture()
        for path, data in originals.items():
            self.write(path, data, 0o440 if path.startswith("daily/") else 0o600,
                       10003 if path.startswith("daily/") else 0, 10001 if path.startswith("daily/") else 0)
        with self.snapshot() as snapshot:
            protected = REF._Protected(snapshot)
            raw = REF._scan_raw(protected, REF._Budget(self.limits))
            prefix = "daily/admitted/" + PURE.H[7:]
            REF._daily_nested(protected, prefix, manifest, admission, raw)
            target = self.source / prefix / "normalization.json"
            replacement = target.with_name("replacement")
            replacement.write_bytes(target.read_bytes())
            os.chown(replacement, 10003, 10001)
            os.chmod(replacement, 0o440)
            os.replace(replacement, target)
            with self.assertRaises(Exception):
                REF._daily_nested(protected, prefix, manifest, admission, raw)
            self.assertTrue(protected.failed)

    def test_real_snapshot_raw_closure_and_independent_provisional_root(self):
        row, prefix = self.raw()
        with self.snapshot() as snapshot:
            protected = REF._Protected(snapshot)
            refs = REF._References(self.limits, PURE.DOC)
            refs.decode(io.BytesIO(PURE.rows_stream()))
            raw = REF._scan_raw(protected, refs.budget)
            identity = {"provider": "kis", "market": "kr", "batch_id": PURE.A,
                        "file_name": "bars.json", "wire_sha256": row["files"][0]["content_hash"]}
            self.assertEqual(raw.reference(identity)["path"], prefix + "/bars.json")
            root = REF._provisional_root(protected, refs)
            # Independent class hash and group framing: no production framing,
            # canonical, class_records or accumulator in expected computation.
            manifest = json.loads(snapshot.manifest())
            expected = hashlib.sha256()
            size = count = 0
            for name in FILES.PAPER_CLASSES:
                class_hash = hashlib.sha256()
                entries = total = 0
                for entry in manifest["entries"]:
                    if entry["path"].startswith(name + "/"):
                        encoded = json.dumps(entry, sort_keys=True, ensure_ascii=True, separators=(",", ":")).encode("ascii")
                        class_hash.update(str(len(encoded)).encode("ascii") + b":" + encoded)
                        entries += 1
                        total += entry["size"]
                value = {"class": name, "sha256": "sha256:" + class_hash.hexdigest(), "entries": entries, "bytes": total}
                framed = PURE.independent_frame("source_class", name, value)
                expected.update(framed)
                size += len(framed)
                count += 1
            for entry in sorted(manifest["entries"], key=lambda e: e["path"].encode("utf-8")):
                framed = PURE.independent_frame("source_member", entry["path"].encode().hex(), entry)
                expected.update(framed)
                size += len(framed)
                count += 1
            self.assertEqual(root._description(), {"group": "source_files", "sha256": "sha256:" + expected.hexdigest(),
                                                  "entries": count, "bytes": size})
            protected.finish()
            with self.assertRaises(REF.ReferenceError):
                protected.read(prefix + "/bars.json")

    def test_complete_group_stays_closed_even_with_real_empty_roots_and_valid_stream(self):
        with self.snapshot() as snapshot:
            with self.assertRaisesRegex(REF.ReferenceError, "^REFERENCES_CLOSURE_INCOMPLETE$"):
                REF.describe(snapshot, io.BytesIO(PURE.rows_stream()))

    def test_named_file_replacement_and_finalization_rejected(self):
        row, prefix = self.raw()
        snapshot = self.snapshot()
        snapshot.__enter__()
        try:
            protected = REF._Protected(snapshot)
            target = self.source / prefix / "bars.json"
            replacement = target.with_name("replacement")
            replacement.write_bytes(target.read_bytes())
            os.chmod(replacement, 0o600)
            os.replace(replacement, target)
            with self.assertRaises(Exception):
                protected.hash_file(prefix + "/bars.json")
            self.assertTrue(protected.failed)
            with self.assertRaises(REF.ReferenceError):
                protected.finish()
        finally:
            snapshot.__exit__(RuntimeError, RuntimeError("fixture mutation"), None)

    def test_changed_content_restored_mtime_still_rejected(self):
        path = self.write("paper_config/process.json", b"original")
        snapshot = self.snapshot()
        snapshot.__enter__()
        try:
            protected = REF._Protected(snapshot)
            original = path.stat()
            path.write_bytes(b"mutated!")
            os.utime(path, ns=(original.st_atime_ns, original.st_mtime_ns))
            with self.assertRaises(Exception):
                protected.read("paper_config/process.json")
            self.assertTrue(protected.failed)
        finally:
            snapshot.__exit__(RuntimeError, RuntimeError("fixture mutation"), None)

    def test_root_directory_replaced_while_retained_fd_is_open(self):
        snapshot = self.snapshot()
        snapshot.__enter__()
        try:
            protected = REF._Protected(snapshot)
            root = self.source / "raw"
            root.rename(self.source / "old-raw")
            root.mkdir(mode=0o700)
            with self.assertRaises(Exception):
                protected.revalidate()
            self.assertTrue(protected.failed)
        finally:
            # Error cleanup closes retained descriptors; the consumer already
            # rejected the replacement before the codec context is unwound.
            snapshot.__exit__(RuntimeError, RuntimeError("fixture mutation"), None)

    def test_mode_owner_hardlink_symlink_and_real_acl_rejection(self):
        # Independent snapshots ensure no previous failure masks the next gate.
        for kind in ("mode", "owner", "hardlink", "symlink", "acl"):
            path = self.write("paper_config/test.json", b"{}")
            snapshot = self.snapshot()
            snapshot.__enter__()
            protected = REF._Protected(snapshot)
            try:
                if kind == "mode":
                    os.chmod(path, 0o640)  # safe-looking but changed exact metadata
                elif kind == "owner":
                    os.chown(path, 10001, 10001)
                elif kind == "hardlink":
                    os.link(path, self.temp / "second-link")
                elif kind == "symlink":
                    path.unlink()
                    path.symlink_to("/etc/passwd")
                else:
                    # Linux POSIX ACL xattr: user_obj, named user, group_obj,
                    # mask, other. A real successful setxattr is required.
                    acl = struct.pack("<I", 2) + b"".join(struct.pack("<HHI", tag, perm, ident) for tag, perm, ident in (
                        (1, 6, 0xffffffff), (2, 4, 10001), (4, 0, 0xffffffff),
                        (16, 4, 0xffffffff), (32, 0, 0xffffffff)))
                    os.setxattr(path, "system.posix_acl_access", acl)
                with self.subTest(kind=kind), self.assertRaises(Exception):
                    protected.read("paper_config/test.json")
                self.assertTrue(protected.failed)
            finally:
                snapshot.__exit__(RuntimeError, RuntimeError("fixture mutation"), None)
                if kind == "acl":
                    os.removexattr(path, "system.posix_acl_access")
                path.unlink()
                link = self.temp / "second-link"
                if link.exists():
                    link.unlink()

    def test_mutation_after_provisional_reference_and_truncated_stream(self):
        path = self.write("paper_config/process.json", b"original")
        raw = PURE.rows_stream({"accounts": [(PURE.A, PURE.account())]})
        class MutatingStream(io.BytesIO):
            def readline(self, maximum=-1):
                line = super().readline(maximum)
                if line.startswith(b'["reference"'):
                    path.write_bytes(b"mutated!")
                return line
        snapshot = self.snapshot()
        snapshot.__enter__()
        try:
            with self.assertRaises(REF.ReferenceError):
                REF.describe(snapshot, MutatingStream(raw))
        finally:
            snapshot.__exit__(RuntimeError, RuntimeError("fixture mutation"), None)
        with self.snapshot() as snapshot:
            with self.assertRaisesRegex(REF.ReferenceError, "^REFERENCES_REJECTED$"):
                REF.describe(snapshot, io.BytesIO(raw[:-1]))

    def test_uncommitted_extra_raw_member_and_mismatched_batch_rejected(self):
        row, prefix = self.raw()
        self.write(prefix + "/unexpected.json", b"opaque")
        with self.snapshot() as snapshot:
            protected = REF._Protected(snapshot)
            with self.assertRaisesRegex(REF.ReferenceError, "^REFERENCES_RAW_INVENTORY$"):
                REF._scan_raw(protected, REF._Budget(self.limits))
        (self.source / prefix / "unexpected.json").unlink()
        row["retrieved_at"] = "2026-11-07T07:30:00Z"
        self.write(prefix + "/batch.json", json.dumps(row).encode())
        with self.snapshot() as snapshot:
            with self.assertRaisesRegex(REF.ReferenceError, "^REFERENCES_RAW_MANIFEST_BATCH$"):
                REF._scan_raw(REF._Protected(snapshot), REF._Budget(self.limits))

    def test_actual_protected_daily_config_policy_and_floor_pins(self):
        repo = Path(__file__).resolve().parents[2]
        config = (repo / "tests/fixtures/owner-beta-daily-install/config.json").read_bytes()
        policy = (repo / "configs/evidence/owner-beta-daily-price-policy-v1.json").read_bytes()
        self.write("config/config.json", config, 0o440, 0, 10001)
        policy_path = self.write("config/policy.json", policy, 0o440, 0, 10001)
        floor_path = self.write("floor/" + REF.daily.FLOOR_NAME, REF.daily.FLOOR_BYTES)
        raw_path = self.source / "raw"
        os.chown(raw_path, 10001, 10001)
        self.roots = tuple(FILES.Root(r.name, r.path, 10001, 10001, r.mode)
                           if r.name == "raw" else r for r in self.roots)
        refs = REF._References(self.limits, PURE.DOC)
        refs.decode(io.BytesIO(PURE.rows_stream()))
        with self.snapshot() as snapshot:
            protected = REF._Protected(snapshot)
            REF._daily_config(protected, refs)
            REF._fixed_floors(protected)
            protected.finish()
        policy_path.write_bytes(policy + b"\n")
        with self.snapshot() as snapshot:
            with self.assertRaises(Exception):
                REF._daily_config(REF._Protected(snapshot), refs)
        policy_path.write_bytes(policy)
        floor_path.write_bytes(REF.daily.FLOOR_BYTES.replace(b"floor-v1", b"floor-v2"))
        with self.snapshot() as snapshot:
            with self.assertRaisesRegex(REF.ReferenceError, "^REFERENCES_FLOOR$"):
                REF._fixed_floors(REF._Protected(snapshot))

    def test_lock_authority_member_is_not_read_by_consumer(self):
        self.write("raw/commit.lock", b"")
        with self.snapshot() as snapshot:
            with self.assertRaisesRegex(REF.ReferenceError, "^REFERENCES_FORBIDDEN_MEMBER$"):
                REF._Protected(snapshot)


if __name__ == "__main__":
    unittest.main()
