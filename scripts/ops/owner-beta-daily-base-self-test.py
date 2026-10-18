#!/usr/bin/env python3
"""Disposable actual-UID filesystem tests; no production base, DB or Docker.

Synthetic bytes replace only the fixed byte pins inside this test process.
This proves access/copy/restore boundaries, not real V3 approval or wholeflow.
"""
import errno
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
from unittest import mock

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("daily_base_tested", ROOT / "scripts/ops/lib/owner-beta-daily-base.py")
base = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = base
spec.loader.exec_module(base)


class DailyBaseTests(unittest.TestCase):
    def setUp(self):
        if sys.platform != "linux" or os.geteuid() != 0:
            self.fail("Linux root fixture required; no skips")
        self.root = Path(tempfile.mkdtemp(prefix="daily-base-fixture-", dir="/var/lib"))
        self.root.chmod(0o755)
        self.addCleanup(shutil.rmtree, self.root)
        self.source = self.root / "original"
        self.source.mkdir(mode=0o750)
        os.chown(self.source, 10001, 10001)
        self.daily = self.root / "daily"
        self.daily.mkdir(mode=0o750)
        os.chown(self.daily, 0, 10001)
        bars = b"synthetic-only-no-market-data\n"
        unsigned = {"candidate_content_sha256": "sha256:" + "a" * 64,
                    "bars": {"sha256": base.digest(bars), "size_bytes": len(bars)}}
        manifest_hash = base.digest(base.install.canonical(unsigned))
        self.values = {"manifest.json": base.install.canonical(dict(unsigned, manifest_sha256=manifest_hash)) + b"\n",
                       "bars.ndjson": bars}
        self.addCleanup(mock.patch.stopall)
        mock.patch.multiple(base, CANDIDATE="a" * 64, MANIFEST=manifest_hash,
                            BARS=base.digest(bars), BARS_SIZE=len(bars)).start()
        self.leaf = self.source / base.CANDIDATE
        self.leaf.mkdir(mode=0o700)
        os.chown(self.leaf, 10001, 10001)
        for name, data in self.values.items():
            path = self.leaf / name
            path.write_bytes(data)
            path.chmod(0o600)
            os.chown(path, 10001, 10001)
        self.before = self.fingerprint(self.source)

    @staticmethod
    def fingerprint(root):
        return {str(p.relative_to(root)): (base.files.identity(p.lstat()), p.read_bytes() if p.is_file() else None)
                for p in [root, *sorted(root.rglob("*"))]}

    def view(self, role):
        return self.daily / base.DIRECTORY / role

    def child_read(self, uid, path):
        pid = os.fork()
        if pid == 0:
            try:
                os.setgroups([10001])
                os.setgid(uid)
                os.setuid(uid)
                path.read_bytes()
                os._exit(0)
            except OSError as error:
                os._exit(error.errno)
            except BaseException:
                os._exit(255)
        _, status = os.waitpid(pid, 0)
        self.assertTrue(os.WIFEXITED(status))
        return os.WEXITSTATUS(status)

    def test_original_private_leaf_is_unreadable_to_daily_principals(self):
        for uid in (10002, 10003):
            self.assertEqual(self.child_read(uid, self.leaf / "bars.ndjson"), errno.EACCES)
        self.assertEqual(self.child_read(10001, self.leaf / "bars.ndjson"), 0)

    def test_exact_copies_are_readable_only_by_their_principals_and_source_unchanged(self):
        self.assertEqual(base.provision(self.source, self.daily), "CREATED")
        for role, uid in base.ROLES.items():
            root = self.view(role)
            for path, mode in ((root, 0o750), (root / base.CANDIDATE, 0o700)):
                info = path.stat()
                self.assertEqual((info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode)), (uid, uid, mode))
            for name, value in self.values.items():
                path = root / base.CANDIDATE / name
                info = path.stat()
                self.assertEqual((info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode), info.st_nlink),
                                 (uid, uid, 0o600, 1))
                self.assertEqual(path.read_bytes(), value)
                self.assertEqual(self.child_read(uid, path), 0)
                for other in ({10001, 10002, 10003} - {uid}):
                    self.assertEqual(self.child_read(other, path), errno.EACCES)
        self.assertEqual(self.fingerprint(self.source), self.before)

    def test_exact_reuse_preserves_every_destination_identity(self):
        base.provision(self.source, self.daily)
        before = self.fingerprint(self.daily)
        self.assertEqual(base.provision(self.source, self.daily), "REUSED")
        self.assertEqual(self.fingerprint(self.daily), before)
        base.verify(self.daily)

    def test_different_copy_is_rejected_without_repair(self):
        base.provision(self.source, self.daily)
        target = self.view("producer") / base.CANDIDATE / "bars.ndjson"
        target.write_bytes(b"tampered")
        before = self.fingerprint(self.daily)
        with self.assertRaises(Exception):
            base.provision(self.source, self.daily)
        self.assertEqual(self.fingerprint(self.daily), before)

    def test_wrong_source_owner_mode_links_or_members_are_rejected(self):
        path = self.leaf / "bars.ndjson"
        for failure in ("owner", "group", "mode", "link", "extra", "symlink"):
            with self.subTest(failure=failure):
                if failure == "owner": os.chown(path, 10002, 10001)
                elif failure == "group": os.chown(path, 10001, 10002)
                elif failure == "mode": path.chmod(0o640)
                elif failure == "link": os.link(path, self.root / "extra-link")
                elif failure == "extra": (self.leaf / "unexpected").write_bytes(b"")
                else:
                    path.rename(self.root / "saved-bars")
                    path.symlink_to(self.root / "saved-bars")
                with self.assertRaises(Exception): base.provision(self.source, self.daily)
                self.assertEqual(list(self.daily.iterdir()), [])
                if failure == "link": (self.root / "extra-link").unlink()
                elif failure == "extra": (self.leaf / "unexpected").unlink()
                elif failure == "symlink":
                    path.unlink()
                    (self.root / "saved-bars").rename(path)
                os.chown(path, 10001, 10001)
                path.chmod(0o600)

    def test_bad_manifest_or_bars_pin_never_publishes_views(self):
        for name in self.values:
            with self.subTest(name=name):
                path = self.leaf / name
                path.write_bytes(self.values[name] + b" ")
                with self.assertRaises(Exception): base.provision(self.source, self.daily)
                self.assertEqual(list(self.daily.iterdir()), [])
                path.write_bytes(self.values[name])

    def test_source_access_acl_is_rejected_before_any_copy(self):
        # Linux POSIX ACL xattr: owner rw, named UID10002 r, group none,
        # mask r, other none. The extended ACL itself is forbidden.
        acl = struct.pack("<I", 2) + b"".join(struct.pack("<HHI", tag, perm, uid)
            for tag, perm, uid in ((1, 6, 0xffffffff), (2, 4, 10002),
                                   (4, 0, 0xffffffff), (16, 4, 0xffffffff), (32, 0, 0xffffffff)))
        os.setxattr(self.leaf / "bars.ndjson", "system.posix_acl_access", acl)
        (self.leaf / "bars.ndjson").chmod(0o600)
        self.assertIn("system.posix_acl_access", os.listxattr(self.leaf / "bars.ndjson"))
        with self.assertRaises(Exception): base.provision(self.source, self.daily)
        self.assertEqual(list(self.daily.iterdir()), [])

    def test_partial_or_swapped_copy_tree_is_rejected_without_repair(self):
        base.provision(self.source, self.daily)
        for failure in ("missing", "extra", "wrong-role"):
            with self.subTest(failure=failure):
                root = self.view("producer")
                if failure == "missing": root.rename(self.root / "held-producer")
                elif failure == "extra": (root / "unexpected").write_bytes(b"")
                else: os.chown(root, 10003, 10003)
                before = self.fingerprint(self.daily)
                with self.assertRaises(Exception): base.provision(self.source, self.daily)
                with self.assertRaises(Exception): base.verify(self.daily)
                self.assertEqual(self.fingerprint(self.daily), before)
                if failure == "missing": (self.root / "held-producer").rename(root)
                elif failure == "extra": (root / "unexpected").unlink()
                else: os.chown(root, 10002, 10002)

    def test_publication_failure_retains_private_stage_and_blocks_retry(self):
        with mock.patch.object(base.install, "rename_no_replace", side_effect=OSError("FIXTURE")):
            with self.assertRaises(OSError): base.provision(self.source, self.daily)
        stages = list(self.daily.iterdir())
        self.assertEqual(len(stages), 1)
        self.assertTrue(stages[0].name.startswith(".historical-base-stage-"))
        self.assertEqual((stages[0].stat().st_uid, stat.S_IMODE(stages[0].stat().st_mode)), (0, 0o700))
        before = self.fingerprint(self.daily)
        with self.assertRaises(Exception): base.provision(self.source, self.daily)
        self.assertEqual(self.fingerprint(self.daily), before)

    def test_source_replacement_during_copy_is_not_accepted(self):
        publish = base.install.rename_no_replace
        def changed(*args):
            publish(*args)
            original = self.leaf / "bars.ndjson"
            original.rename(self.root / "old-bars")
            original.write_bytes(self.values["bars.ndjson"])
            os.chown(original, 10001, 10001)
            original.chmod(0o600)
        with mock.patch.object(base.install, "rename_no_replace", side_effect=changed):
            with self.assertRaises(Exception): base.provision(self.source, self.daily)

    def test_backup_restore_retains_copy_bytes_and_real_uid_access(self):
        base.provision(self.source, self.daily)
        roots = []
        for name in base.files.CLASSES:
            path = self.daily if name == "daily" else self.root / ("snapshot-" + name)
            if name != "daily": path.mkdir(mode=0o750)
            info = path.stat()
            roots.append(base.files.Root(name, str(path), info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode)))
        limits = base.files.Limits(2 * 1024**2)
        archive = io.BytesIO()
        with base.files.Snapshot(tuple(roots), limits) as snap:
            snap.write(archive)
            records = snap.records
        destination = self.root / "restored"
        destination.mkdir(mode=0o700)
        archive.seek(0)
        base.files.restore_archive(archive, limits, str(destination))
        base.verify(destination / "daily")
        # The outer private restore stage remains root-only until handed off.
        destination.chmod(0o755)
        for role, uid in base.ROLES.items():
            self.assertEqual(self.child_read(uid, destination / "daily" / base.DIRECTORY / role /
                                            base.CANDIDATE / "bars.ndjson"), 0)
        self.assertTrue(any(r["path"].startswith("daily/" + base.DIRECTORY + "/") for r in records))
        self.assertEqual(self.fingerprint(self.source), self.before)


class FixedPinsTests(unittest.TestCase):
    def test_copy_pins_equal_the_embedded_v3_registry_without_an_alternate_selector(self):
        record, = json.loads((ROOT / "configs/evidence/kis-historical-price-only-v3-approved-artifacts.json").read_bytes())["approved_artifacts"]
        self.assertEqual("sha256:" + base.CANDIDATE, record["candidate_content_sha256"])
        self.assertEqual(base.MANIFEST, record["artifact_manifest_sha256"])
        self.assertEqual(base.BARS, record["bars_sha256"])
        self.assertEqual(base.BARS_SIZE, record["bars_size_bytes"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
