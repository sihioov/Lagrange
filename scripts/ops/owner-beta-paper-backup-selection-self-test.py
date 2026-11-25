#!/usr/bin/env python3
"""Actual Linux-root filesystem checks for retained Paper source originals.

The live installed-capability integration remains with the combined controller
owner; these tests exercise only this package's real descriptor-relative source
custody and fail closed unless run as Linux root.
"""
from __future__ import annotations

import contextlib
import fcntl
import importlib.util
import io
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

    def test_serving_originals_survive_actual_locked_selection_and_isolated_restore(self):
        spec = importlib.util.spec_from_file_location(
            "paper_serving_test_lifecycle",HERE / "lib/owner-beta-paper-lifecycle.py")
        lifecycle = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = lifecycle
        spec.loader.exec_module(lifecycle)
        serving = lifecycle._load("owner_beta_paper_serving", "owner-beta-paper-serving.py")
        j = lifecycle.journal
        generation = "00000000-0000-4000-8000-000000000501"
        account = "00000000-0000-4000-8000-000000000502"
        manifest = j._sha(b"release fixture")
        proposal = b'{"public_fixture":true}'
        example = {"Id":"a"*64,"Image":manifest,"Created":"2026-11-01T00:00:00Z",
                   "RestartCount":0,"StartedAt":"2026-11-01T00:00:01Z"}
        limits = FILES.Limits(8 * 1024 * 1024)
        for variant in ("configured", "attended"):
            with self.subTest(variant=variant), contextlib.ExitStack() as held:
                root = self.temp / variant
                root.mkdir(mode=0o700)
                lock = os.open(root / j.LOCK_NAME, os.O_RDWR|os.O_CREAT|os.O_EXCL, 0o600)
                held.callback(os.close, lock)
                lifecycle_root = root / lifecycle.STORE
                lifecycle_root.mkdir(mode=0o700)
                for name in ("queries", "anchors", "selections", "staging"):
                    (lifecycle_root / name).mkdir(mode=0o700)
                with j.open_journal(str(root), lock, bootstrap=True) as store:
                    intent = lifecycle._intent(store.installation_id, manifest, "ENABLE", account,
                                               0, proposal, generation=generation)
                    with contextlib.closing(store.bind_caller("public-fixture", j._canonical(intent))) as bound:
                        self.assertEqual(bound.generation,generation)
                with serving.Records(lifecycle,str(root),lock) as records:
                    body = dict(schema="owner-beta-paper-serving-intent-v1",enable_generation=generation,
                        account_id=account,expected_epoch=0,release_manifest_sha256=manifest,
                        process_sha256=j._sha(proposal),installed_env_sha256=j._sha(b"env"),
                        prior_api_generation=example,prior_proxy_generation=dict(example,Id="b"*64),
                        prior_daily_generations={})
                    records.write(generation, "serving-intent.json",body,create=True)
                    if variant == "attended":
                        records.write(generation,"serving-failed.json",
                            {"state":"UNKNOWN","fence":None,"enable_generation":generation})
                    else:
                        records.write(generation,"serving-not-started.json",
                            {"native_outcome":"BLOCKED","native_cleanup":"COMPLETE"})
                    self.assertEqual(records.read(generation,"serving-intent.json"),body)
                    originals = {}
                    journal_root = root / j.PAPER_DIR
                    operation = journal_root / "operations" / generation
                    for path in operation.iterdir():
                        if len(path.name) == 69:
                            raw = path.read_bytes()
                            self.assertEqual(path.name,j._sha(raw)[7:] + ".json")
                            at = "paper_operations/" + str(path.relative_to(journal_root))
                            originals[at] = raw
                    self.assertEqual(len(originals),2)
                    allocated = self.temp / (variant+"-projection")
                    allocated.mkdir(mode=0o700)
                    roots = []
                    for name in FILES.PAPER_CLASSES:
                        path = journal_root if name == "paper_operations" else allocated / name
                        if path != journal_root:
                            path.mkdir(mode=0o700)
                        roots.append(FILES.Root(name,str(path),0,0,0o700))
                    fd = os.open(allocated / "paper_evidence",FILES.DIRECTORY)
                    held.callback(os.close,fd)
                    sources = []
                    def close_sources():
                        for source in reversed(sources):
                            source.close()
                    held.callback(close_sources)
                    SELECTION._lifecycle_originals(FILES,str(root),fd,sources,limits,lambda: None)
                    self.assertEqual(list((allocated/"paper_evidence/lifecycle/queries").iterdir()),[])
                    stream = io.BytesIO()
                    with FILES.Snapshot(roots,limits,profile="paper-combined") as snapshot:
                        snapshot.write(stream)
                    _, retained = FILES.verify_archive(io.BytesIO(stream.getvalue()),limits,
                        retain=tuple(originals),profile="paper-combined")
                    self.assertEqual(retained,originals)
                    clone = self.temp / (variant+"-guest")
                    clone.mkdir(mode=0o700)
                    FILES.restore_archive(io.BytesIO(stream.getvalue()),limits,str(clone),profile="paper-combined")
                    for path, raw in originals.items():
                        self.assertEqual((clone / path).read_bytes(),raw)
                        self.assertEqual(stat.S_IMODE((clone / path).stat().st_mode),0o400)
                    first = next(iter(originals.values()))
                    changed = stream.getvalue().replace(first,b" " + first[1:],1)
                    with self.assertRaisesRegex(FILES.SnapshotError,"DAILY_BACKUP_FILES_DIGEST"):
                        FILES.verify_archive(io.BytesIO(changed),limits,profile="paper-combined")
                    missing = next(path for path in operation.iterdir() if len(path.name)==69
                                   and j._json(path.read_bytes())["phase"]=="intent")
                    missing.unlink()
                    with self.assertRaisesRegex(lifecycle.LifecycleError,"PAPER_SERVING_RECEIPT"):
                        records.read(generation,"serving-intent.json")

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
        # A replaced copied inode is rejected first by the shared file codec:
        # its retained descriptor is now unlinked (nlink=0), so checked() emits
        # this fixed snapshot error before selection can translate it.
        with self.assertRaisesRegex(FILES.SnapshotError,
                                   "^DAILY_BACKUP_FILES_TYPE_OR_MODE$"):
            original.revalidate(FILES)
        # The retained private-tree boundary independently detects the
        # post-seal directory identity change caused by replacing its member.
        with self.assertRaisesRegex(SELECTION.PaperSelectionError,
                                   "^PAPER_SELECTION_STAGE_REPLACED$"):
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
