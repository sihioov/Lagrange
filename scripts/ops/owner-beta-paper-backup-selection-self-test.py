#!/usr/bin/env python3
"""Actual Linux-root filesystem checks for retained Paper source originals.

The live installed-capability integration remains with the combined controller
owner; these tests exercise only this package's real descriptor-relative source
custody and fail closed unless run as Linux root.
"""
from __future__ import annotations

import fcntl
import importlib.util
import os
from pathlib import Path
import shutil
import stat
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
TEST_PATH = HERE / "test_owner_beta_paper_backup_selection.py"
TEST_SPEC = importlib.util.spec_from_file_location("paper_backup_selection_pure_tests", TEST_PATH)
TESTS = importlib.util.module_from_spec(TEST_SPEC)
sys.modules[TEST_SPEC.name] = TESTS
TEST_SPEC.loader.exec_module(TESTS)
FILES = TESTS.FILES
SELECTION = TESTS.SELECTION


class PaperBackupSelectionRootTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if sys.platform != "linux" or os.geteuid() != 0:
            raise RuntimeError(
                "PaperBackupSelectionRootTests requires an actual Linux root process; this class must not be skipped")
        parent = os.stat("/var/lib", follow_symlinks=False)
        if (not stat.S_ISDIR(parent.st_mode) or parent.st_uid != 0 or
                parent.st_mode & 0o7022):
            raise RuntimeError(
                "PaperBackupSelectionRootTests requires the protected /var/lib fixture parent")

    def setUp(self):
        self.temp = Path(tempfile.mkdtemp(prefix="lagrange-paper-selection-", dir="/var/lib"))
        os.chown(self.temp, 0, 0)
        os.chmod(self.temp, 0o700)
        self.addCleanup(shutil.rmtree, self.temp)

    def source_file(self, relative="source/original.json", data=b"{\"source\":true}\n",
                    mode=0o640):
        path = self.temp / relative
        path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        os.chown(path.parent, 0, 0)
        os.chmod(path.parent, 0o700)
        path.write_bytes(data)
        os.chown(path, 0, 0)
        os.chmod(path, mode)
        return path

    def test_retained_original_copies_exact_bytes_and_safe_metadata(self):
        path = self.source_file()
        target = self.temp / "selected"
        target.mkdir(mode=0o700)
        os.chown(target, 0, 0)
        os.chmod(target, 0o700)
        original = SELECTION._OriginalFile(FILES, str(path), uid=0, gid=0,
                                           mode=0o640, maximum=4096)
        self.addCleanup(original.close)
        class_fd = os.open(target, FILES.DIRECTORY)
        self.addCleanup(os.close, class_fd)
        original.copy_to(FILES, class_fd, "paper/input.json")
        copied = target / "paper" / "input.json"
        info = copied.stat(follow_symlinks=False)
        self.assertTrue(stat.S_ISREG(info.st_mode))
        self.assertEqual((info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode), info.st_nlink),
                         (0, 0, 0o640, 1))
        self.assertEqual(copied.read_bytes(), path.read_bytes())
        original.revalidate(FILES)

    def test_projection_swap_and_postseal_directory_mutation_reject(self):
        tree = SELECTION._mkdir_private_tree(FILES)
        self.addCleanup(SELECTION._dispose_private_tree, FILES, tree)
        path = self.source_file()
        original = SELECTION._OriginalFile(FILES, str(path), uid=0, gid=0,
                                           mode=0o640, maximum=4096)
        self.addCleanup(original.close)
        original.copy_to(FILES, tree["fd"], "projected.json")
        SELECTION._seal_private_tree(FILES, tree)
        original.revalidate(FILES)
        projection = Path(tree["path"]) / "projected.json"
        replacement = Path(tree["path"]) / "replacement.json"
        replacement.write_bytes(projection.read_bytes())
        os.chmod(replacement, 0o640)
        os.replace(replacement, projection)
        with self.assertRaises(SELECTION.PaperSelectionError):
            original.revalidate(FILES)
        with self.assertRaises(SELECTION.PaperSelectionError):
            SELECTION._check_private_tree(FILES, tree)

    def test_locked_empty_directory_is_observed_and_late_member_rejected(self):
        events = self.temp / "events"
        events.mkdir(mode=0o700)
        lock = os.open(self.temp / "control.lock", os.O_RDWR | os.O_CREAT | os.O_EXCL, 0o600)
        self.addCleanup(os.close, lock)
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        original = SELECTION._OriginalDirectory(FILES, str(events), maximum=64)
        self.addCleanup(original.close)
        self.assertEqual(original.members, ())
        original.revalidate(FILES)
        (events / "unexpected").write_bytes(b"late")
        with self.assertRaises(SELECTION.PaperSelectionError):
            original.revalidate(FILES)

    def test_retained_original_rejects_same_bytes_replacement_and_mutation(self):
        path = self.source_file()
        original = SELECTION._OriginalFile(FILES, str(path), uid=0, gid=0,
                                           mode=0o640, maximum=4096)
        self.addCleanup(original.close)
        replacement = self.temp / "replacement.json"
        replacement.write_bytes(path.read_bytes())
        os.chown(replacement, 0, 0)
        os.chmod(replacement, 0o640)
        os.replace(replacement, path)
        with self.assertRaises(SELECTION.PaperSelectionError):
            original.revalidate(FILES)

        changed = self.source_file("changed/original.json")
        second = SELECTION._OriginalFile(FILES, str(changed), uid=0, gid=0,
                                        mode=0o640, maximum=4096)
        self.addCleanup(second.close)
        changed.write_bytes(b"{\"source\":false}\n")
        with self.assertRaises(SELECTION.PaperSelectionError):
            second.revalidate(FILES)

    def test_source_open_rejects_symlink_and_hardlink_aliases(self):
        path = self.source_file()
        symlink = self.temp / "source" / "symlink.json"
        symlink.symlink_to(path.name)
        with self.assertRaises(SELECTION.PaperSelectionError):
            SELECTION._OriginalFile(FILES, str(symlink), uid=0, gid=0,
                                    mode=0o640, maximum=4096)
        linked = self.temp / "source" / "linked.json"
        os.link(path, linked)
        with self.assertRaises(FILES.SnapshotError):
            SELECTION._OriginalFile(FILES, str(path), uid=0, gid=0,
                                    mode=0o640, maximum=4096)

    def test_retained_parent_name_replacement_fails_even_if_file_is_unchanged(self):
        path = self.source_file()
        original = SELECTION._OriginalFile(FILES, str(path), uid=0, gid=0,
                                           mode=0o640, maximum=4096)
        self.addCleanup(original.close)
        parent = path.parent
        renamed = parent.with_name("renamed-source")
        os.rename(parent, renamed)
        with self.assertRaises(SELECTION.PaperSelectionError):
            original.revalidate(FILES)


if __name__ == "__main__":
    unittest.main()
