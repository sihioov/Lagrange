#!/usr/bin/env python3
"""Real Linux-root Paper file fixtures for the existing hosted backup step."""
from __future__ import annotations

import importlib.util
import io
import os
from pathlib import Path
import shutil
import stat
import struct
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "paper_backup_codec_tests", Path(__file__).with_name("test_owner_beta_paper_backup_files.py"))
CODEC_TESTS = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = CODEC_TESTS
SPEC.loader.exec_module(CODEC_TESTS)
FILES = CODEC_TESTS.FILES
PAPER_CLASSES = CODEC_TESTS.PAPER_CLASSES
PAPER_SCHEMA = CODEC_TESTS.PAPER_SCHEMA
EMPTY_SHA256 = CODEC_TESTS.EMPTY_SHA256
canonical = CODEC_TESTS.canonical
limits = CODEC_TESTS.limits
PaperCombinedCodecTests = CODEC_TESTS.PaperCombinedCodecTests


class PaperCombinedArchiveRootTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if sys.platform != "linux" or os.geteuid() != 0:
            raise RuntimeError("PaperCombinedArchiveRootTests requires an actual Linux root process; this class must not be skipped")
        parent = os.stat("/var/lib", follow_symlinks=False)
        if (not stat.S_ISDIR(parent.st_mode) or parent.st_uid != 0 or
                parent.st_mode & 0o7022):
            raise RuntimeError("PaperCombinedArchiveRootTests requires the protected /var/lib fixture parent")

    def setUp(self):
        self.temp = Path(tempfile.mkdtemp(prefix="lagrange-paper-combined-files-", dir="/var/lib"))
        os.chown(self.temp, 0, 0)
        os.chmod(self.temp, 0o700)
        self.addCleanup(shutil.rmtree, self.temp)
        self.limits = limits(total=1 << 20, entries=500, manifest=1 << 20)

    def source_roots(self, *, alias_base_input=False):
        index = getattr(self, "_source_index", 0) + 1
        self._source_index = index
        source = self.temp / ("source-%02d" % index)
        source.mkdir(mode=0o700)
        os.chown(source, 0, 0)
        os.chmod(source, 0o700)
        shared = source / "shared-base-input" if alias_base_input else None
        roots = []
        for name in PAPER_CLASSES:
            path = shared if alias_base_input and name in {"base", "simulation_input"} else source / name
            if not path.exists():
                path.mkdir(mode=0o700)
                os.chown(path, 0, 0)
                os.chmod(path, 0o700)
            roots.append(FILES.Root(name, str(path), 0, 0, 0o700))
            if name == "base":
                nested = path / "nested"
                nested.mkdir(mode=0o700)
                payload = nested / "snapshot.bin"
                payload.write_bytes(b"root-owned fixture data\x00\xff")
                os.chown(nested, 0, 0)
                os.chmod(nested, 0o700)
                os.chown(payload, 10001, 10001)
                os.chmod(payload, 0o440)
            elif name == "paper_config":
                payload = path / "projection.json"
                payload.write_bytes(b"{}\n")
                os.chown(payload, 10002, 10001)
                os.chmod(payload, 0o440)
        return tuple(roots)

    def make_archive(self, roots):
        output = io.BytesIO()
        with FILES.Snapshot(roots, self.limits, profile="paper-combined") as snapshot:
            snapshot.write(output)
            manifest_bytes = snapshot.manifest()
        return output.getvalue(), manifest_bytes

    def test_twenty_class_snapshot_verify_restore_snapshot_equality(self):
        roots = self.source_roots()
        archive, manifest_bytes = self.make_archive(roots)
        manifest, retained = FILES.verify_archive(io.BytesIO(archive), self.limits,
                                                   profile="paper-combined")
        self.assertEqual(retained, {})
        rows = FILES.class_records(canonical(manifest), self.limits, profile="paper-combined")
        self.assertEqual([row["class"] for row in rows], list(PAPER_CLASSES))
        self.assertEqual(rows[1], {"class": "config", "sha256": EMPTY_SHA256, "entries": 0, "bytes": 0})
        self.assertEqual(rows[0]["entries"], 2)
        self.assertGreater(rows[0]["bytes"], 0)
        self.assertEqual(manifest["schema"], PAPER_SCHEMA)
        self.assertEqual(manifest["classes"], list(PAPER_CLASSES))
        self.assertEqual(manifest_bytes, canonical(manifest))
        self.assertFalse(any(entry["uid"] == 999 or entry["gid"] == 999
                             for entry in manifest["entries"]))

        restored = self.temp / "restored"
        restored.mkdir(mode=0o700)
        os.chown(restored, 0, 0)
        os.chmod(restored, 0o700)
        decoded = FILES.restore_archive(io.BytesIO(archive), self.limits, str(restored),
                                        profile="paper-combined")
        self.assertEqual(decoded, manifest)
        restored_roots = tuple(FILES.Root(name, str(restored / name), 0, 0, 0o700)
                               for name in PAPER_CLASSES)
        roundtrip, restored_manifest_bytes = self.make_archive(restored_roots)
        restored_manifest, _ = FILES.verify_archive(io.BytesIO(roundtrip), self.limits,
                                                     profile="paper-combined")
        self.assertEqual(restored_manifest_bytes, manifest_bytes)
        self.assertEqual(restored_manifest, manifest)
        self.assertEqual(roundtrip, archive)

    def test_only_exact_base_simulation_input_alias_is_allowed(self):
        roots = self.source_roots(alias_base_input=True)
        with FILES.Snapshot(roots, self.limits, profile="paper-combined") as snapshot:
            snapshot.verify_unchanged()
        overlap = list(self.source_roots())
        base = overlap[0]
        overlap[1] = FILES.Root("config", base.path + "/child", 0, 0, 0o700)
        with self.assertRaises(FILES.SnapshotError):
            FILES.Snapshot(tuple(overlap), self.limits, profile="paper-combined")

    def test_source_mutation_and_name_replacement_fail_after_snapshot_open(self):
        for replacement in (False, True):
            with self.subTest(replacement=replacement):
                roots = self.source_roots()
                file_path = Path(roots[0].path) / "nested" / "snapshot.bin"
                with self.assertRaises(FILES.SnapshotError):
                    with FILES.Snapshot(roots, self.limits, profile="paper-combined") as snapshot:
                        if replacement:
                            saved = file_path.with_name("saved.bin")
                            file_path.rename(saved)
                            file_path.write_bytes(saved.read_bytes())
                            os.chown(file_path, 10001, 10001)
                            os.chmod(file_path, 0o440)
                        else:
                            file_path.write_bytes(file_path.read_bytes() + b"changed")
                        snapshot.verify_unchanged()

    def test_hardlink_symlink_unsafe_owner_and_acl_are_rejected(self):
        roots = self.source_roots()
        original = Path(roots[0].path) / "nested" / "snapshot.bin"
        os.link(original, original.with_name("hardlink.bin"))
        with self.assertRaises(FILES.SnapshotError):
            with FILES.Snapshot(roots, self.limits, profile="paper-combined"):
                pass

        roots = self.source_roots()
        original = Path(roots[0].path) / "nested" / "snapshot.bin"
        target = original.with_name("target.bin")
        original.rename(target)
        original.symlink_to(target.name)
        with self.assertRaises(FILES.SnapshotError):
            with FILES.Snapshot(roots, self.limits, profile="paper-combined"):
                pass

        roots = self.source_roots()
        original = Path(roots[0].path) / "nested" / "snapshot.bin"
        os.chown(original, 999, 999)
        with self.assertRaises(FILES.SnapshotError):
            with FILES.Snapshot(roots, self.limits, profile="paper-combined"):
                pass

        roots = self.source_roots()
        original = Path(roots[0].path) / "nested" / "snapshot.bin"
        acl = struct.pack("<I", 2) + b"".join(struct.pack("<HHI", tag, permission, uid)
            for tag, permission, uid in ((1, 6, 0xFFFFFFFF), (2, 4, 10004),
                                         (4, 4, 0xFFFFFFFF), (16, 4, 0xFFFFFFFF),
                                         (32, 0, 0xFFFFFFFF)))
        os.setxattr(original, "system.posix_acl_access", acl, follow_symlinks=False)
        self.assertEqual(os.getxattr(original, "system.posix_acl_access", follow_symlinks=False), acl)
        with self.assertRaises(FILES.SnapshotError):
            with FILES.Snapshot(roots, self.limits, profile="paper-combined"):
                pass

    def test_nonempty_restore_destination_is_never_overwritten(self):
        archive, _manifest = self.make_archive(self.source_roots())
        destination = self.temp / "nonempty"
        destination.mkdir(mode=0o700)
        os.chown(destination, 0, 0)
        os.chmod(destination, 0o700)
        marker = destination / "keep.bin"
        marker.write_bytes(b"keep exactly")
        os.chown(marker, 0, 0)
        os.chmod(marker, 0o400)
        before = (marker.stat().st_ino, marker.stat().st_mode, marker.read_bytes())
        with self.assertRaises(FILES.SnapshotError):
            FILES.restore_archive(io.BytesIO(archive), self.limits, str(destination),
                                  profile="paper-combined")
        after = (marker.stat().st_ino, marker.stat().st_mode, marker.read_bytes())
        self.assertEqual(after, before)
        self.assertEqual({entry.name for entry in destination.iterdir()}, {"keep.bin"})

    def test_corrupt_and_truncated_restore_fail_with_private_incomplete_staging(self):
        archive, _manifest = self.make_archive(self.source_roots())
        sentinel = self.temp / "outside-destination.bin"
        sentinel.write_bytes(b"untouched")
        os.chown(sentinel, 0, 0)
        os.chmod(sentinel, 0o400)
        broken_archives = (archive[:-1], archive[:-1] + bytes([archive[-1] ^ 1]))
        for index, broken in enumerate(broken_archives):
            with self.subTest(index=index):
                destination = self.temp / ("failed-restore-%d" % index)
                destination.mkdir(mode=0o700)
                os.chown(destination, 0, 0)
                os.chmod(destination, 0o700)
                with self.assertRaises(FILES.SnapshotError):
                    FILES.restore_archive(io.BytesIO(broken), self.limits, str(destination),
                                          profile="paper-combined")
                self.assertEqual(sentinel.read_bytes(), b"untouched")
                self.assertTrue(stat.S_IMODE(destination.stat().st_mode) == 0o700)



if __name__ == "__main__":
    unittest.main(verbosity=2)
