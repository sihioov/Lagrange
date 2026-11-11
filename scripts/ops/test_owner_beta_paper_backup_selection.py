#!/usr/bin/env python3
"""Pure source-selection contract checks; no installed capability is forged."""
from __future__ import annotations

import contextlib
import hashlib
import os
import stat
import subprocess
import tempfile
from unittest import mock
import importlib.util
import sys
import types
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
CODEC_TEST = HERE / "test_owner_beta_paper_backup_files.py"
CODEC_SPEC = importlib.util.spec_from_file_location("paper_selection_codec_tests", CODEC_TEST)
CODEC = importlib.util.module_from_spec(CODEC_SPEC)
sys.modules[CODEC_SPEC.name] = CODEC
CODEC_SPEC.loader.exec_module(CODEC)
FILES = CODEC.FILES
SELECTION_PATH = HERE / "lib" / "owner-beta-paper-backup-selection.py"
SELECTION_SPEC = importlib.util.spec_from_file_location("owner_beta_paper_backup_selection", SELECTION_PATH)
SELECTION = importlib.util.module_from_spec(SELECTION_SPEC)
sys.modules[SELECTION_SPEC.name] = SELECTION
SELECTION_SPEC.loader.exec_module(SELECTION)


class PaperBackupSelectionTests(unittest.TestCase):
    def test_selection_and_codec_share_the_exact_twenty_class_order(self):
        self.assertEqual(SELECTION._paper_classes(), CODEC.PAPER_CLASSES)
        self.assertEqual(SELECTION._PAPER_CLASSES, FILES.PAPER_CLASSES)
        self.assertEqual(len(SELECTION._paper_classes()), 20)
        self.assertEqual(len(set(SELECTION._paper_classes())), 20)

    def test_projection_roots_are_finite_and_exclude_source_lock_classes(self):
        self.assertEqual(SELECTION._PAPER_PROJECTION_CLASSES, (
            "paper_config", "paper_contract", "paper_evidence", "paper_manifest",
            "paper_reader"))
        self.assertTrue(set(SELECTION._PAPER_PROJECTION_CLASSES).issubset(
            set(SELECTION._paper_classes())))
        self.assertNotIn("raw", SELECTION._PAPER_PROJECTION_CLASSES)
        self.assertNotIn("paper_operations", SELECTION._PAPER_PROJECTION_CLASSES)

    def test_codec_binding_accepts_only_the_loaded_codec_and_canonical_limits(self):
        self.assertIs(SELECTION._file_module(CODEC.limits(),
                                            types.SimpleNamespace(files=FILES)), FILES)
        with self.assertRaises(SELECTION.PaperSelectionError):
            SELECTION._file_module(CODEC.limits(), types.SimpleNamespace(files=object()))

    def test_unowned_or_serialized_audit_evidence_never_mints_a_selection(self):
        with self.assertRaises(SELECTION.PaperSelectionError) as error:
            SELECTION._trusted_audit_owner(None, None, None, {})
        self.assertEqual(str(error.exception), "PAPER_SELECTION_AUDIT_OWNER")


class SelectionConstructionTests(unittest.TestCase):
    """Unprivileged component checks; production ownership checks are unchanged."""
    def test_mkdir_then_seal_accounts_for_own_parent_mutation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "var/lib").mkdir(parents=True)
            real_open = os.open
            def opened(path, flags, *args, **kwargs):
                return real_open(root if path == "/" else path, flags, *args, **kwargs)
            with mock.patch.object(SELECTION.os, "open", side_effect=opened), \
                    mock.patch.object(FILES, "checked_parent", side_effect=os.fstat), \
                    mock.patch.object(FILES, "checked", side_effect=lambda fd, **kw: os.fstat(fd)):
                tree = SELECTION._mkdir_private_tree(FILES)
                try:
                    os.mkdir("paper_reader", 0o700, dir_fd=tree["fd"])
                    tree["class_fds"]["paper_reader"] = os.open("paper_reader", FILES.DIRECTORY, dir_fd=tree["fd"])
                    SELECTION._seal_private_tree(FILES, tree)
                    SELECTION._check_private_tree(FILES, tree)
                    with self.assertRaises(SELECTION.PaperSelectionError):
                        SELECTION._seal_private_tree(FILES, tree)
                    os.mkdir("unexpected", 0o700, dir_fd=tree["fd"])
                    with self.assertRaises(SELECTION.PaperSelectionError):
                        SELECTION._check_private_tree(FILES, tree)
                finally:
                    for fd in list(tree["class_fds"].values()) + tree["fds"]:
                        os.close(fd)

    def test_enter_failure_exits_snapshot_and_preserves_primary(self):
        owner = object.__new__(SELECTION._SelectedPaperFiles)
        owner._closed = owner._entered = False
        owner.snapshot = object.__new__(FILES.Snapshot)
        primary = SELECTION.PaperSelectionError("PAPER_SELECTION_HISTORICAL_ORIGINAL_MISSING")
        with mock.patch.object(SELECTION._SelectedPaperFiles, "revalidate"), \
                mock.patch.object(FILES.Snapshot, "__enter__"), \
                mock.patch.object(FILES.Snapshot, "__exit__") as exited, \
                mock.patch.object(SELECTION._SelectedPaperFiles, "_check_requirements", side_effect=primary), \
                mock.patch.object(SELECTION._SelectedPaperFiles, "close", side_effect=OSError()) as closed:
            with self.assertRaises(SELECTION.PaperSelectionError) as error:
                owner.__enter__()
            self.assertIs(error.exception, primary)
            self.assertFalse(owner._entered)
            exited.assert_called_once()
            closed.assert_called_once()

    def test_copy_parent_validation_failure_closes_opened_child(self):
        source = object.__new__(SELECTION._OriginalFile)
        with mock.patch.object(SELECTION._OriginalFile, "revalidate"), \
                mock.patch.object(SELECTION.os, "mkdir"), mock.patch.object(SELECTION.os, "fsync"), \
                mock.patch.object(SELECTION.os, "open", return_value=42), \
                mock.patch.object(SELECTION.os, "close") as closed, \
                mock.patch.object(FILES, "checked", side_effect=FILES.SnapshotError("BAD")):
            with self.assertRaises(FILES.SnapshotError):
                source.copy_to(FILES, 9, "one/two/file")
            closed.assert_called_once_with(42)

    def test_selected_exit_unwinds_snapshot_and_preserves_body_error(self):
        owner = object.__new__(SELECTION._SelectedPaperFiles)
        owner._entered = True
        snapshot = mock.MagicMock()
        snapshot.__exit__.side_effect = RuntimeError("secondary-snapshot")
        owner.snapshot = snapshot
        primary = ValueError("primary-body")
        with mock.patch.object(SELECTION._SelectedPaperFiles, "close", side_effect=RuntimeError("secondary-close")) as closed:
            self.assertFalse(owner.__exit__(type(primary), primary, None))
        self.assertFalse(owner._entered)
        snapshot.__exit__.assert_called_once_with(type(primary), primary, None)
        closed.assert_called_once()

    def test_original_constructor_validation_failure_closes_owned_child(self):
        with mock.patch.object(SELECTION.os, "open", side_effect=[41,42]), \
                mock.patch.object(SELECTION.os, "close") as closed, \
                mock.patch.object(FILES, "checked_parent", side_effect=[None, FILES.SnapshotError("BAD")]):
            with self.assertRaises(FILES.SnapshotError):
                SELECTION._OriginalFile(FILES, "/parent/file", uid=0, gid=0, maximum=4096)
        self.assertEqual(closed.call_args_list, [mock.call(42), mock.call(41)])


class LifecycleOriginalTests(unittest.TestCase):
    def setUp(self):
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(mock.patch.object(subprocess, "Popen", side_effect=AssertionError("child forbidden")))
        self.stack.enter_context(mock.patch.object(os, "chown", side_effect=AssertionError("chown forbidden")))
        self.stack.enter_context(mock.patch.object(os, "fchown", side_effect=AssertionError("fchown forbidden")))
        # An unprivileged fixture models ownership; the production checks remain
        # exact. Real descriptors, names, bytes, modes and generation changes run.
        real_stat, real_fstat = os.stat, os.fstat
        def ownership(info):
            fields = {k: getattr(info, k) for k in dir(info) if k.startswith("st_")}
            fields.update(st_uid=0, st_gid=0)
            return types.SimpleNamespace(**fields)
        self.stack.enter_context(mock.patch.object(os, "stat", side_effect=lambda *a, **kw: ownership(real_stat(*a, **kw))))
        self.stack.enter_context(mock.patch.object(os, "fstat", side_effect=lambda *a, **kw: ownership(real_fstat(*a, **kw))))
        self.stack.enter_context(mock.patch.object(FILES, "checked_parent", side_effect=os.fstat))
        def checked(fd, *, directory, owners, groups):
            info = os.fstat(fd)
            if (not (stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode))
                    or info.st_uid not in owners or info.st_gid not in groups
                    or (not directory and info.st_nlink != 1)):
                raise FILES.SnapshotError("TYPE")
            return info
        self.stack.enter_context(mock.patch.object(FILES, "checked", side_effect=checked))
        self.temporary = self.stack.enter_context(tempfile.TemporaryDirectory())
        self.root = Path(self.temporary)
        self.install = self.root / "install"
        self.install.mkdir(mode=0o700)
        self.source = self.install / ".owner-beta-paper-lifecycle"
        self.source.mkdir(mode=0o700)
        for name in ("queries", "selections", "anchors", "staging"):
            (self.source / name).mkdir(mode=0o700)
        self.destination = self.root / "projection"
        self.destination.mkdir(mode=0o700)
        self.fd = os.open(self.destination, FILES.DIRECTORY)
        self.addCleanup(os.close, self.fd)
        self.sources = []
        self.addCleanup(lambda: [item.close() for item in reversed(self.sources)])
        self.query = self.source / "queries" / "00000000-0000-4000-8000-000000000001"

    def write(self, path, raw=b"{}", mode=0o400):
        path.write_bytes(raw)
        path.chmod(mode)
        return path

    def capture(self, **kw):
        SELECTION._lifecycle_originals(FILES, str(self.install), self.fd, self.sources,
                                      CODEC.limits(**kw), lambda: None)

    def test_actual_original_copy_prefixes_outputs_and_prior_boots(self):
        self.query.mkdir(mode=0o700)
        (self.query / "output").mkdir(mode=0o700)
        # Semantics are native's job: capture actual bytes, including producer
        # ordered stdout and interrupted prefixes, without rewriting documents.
        self.write(self.query / "request.json", b'{"schema":"R"}')
        self.write(self.query / "native-result.json", b'{"schema":"native","verb":"readback"}\n')
        large = b"x" * 70_000
        self.write(self.query / "sql-request.json", large)
        raw = b'{"actual":"immutable"}'
        name = hashlib.sha256(raw).hexdigest() + ".json"
        self.write(self.query / "output" / name, raw)
        self.write(self.source / "selections" / name, raw)
        self.write(self.source / "anchors" / name, raw)
        self.write(self.source / "staging" / "ephemeral", b"excluded")
        self.capture()
        self.assertEqual((self.destination / "lifecycle/queries" / self.query.name / "sql-request.json").read_bytes(), large)
        self.assertEqual((self.destination / "lifecycle/anchors" / name).read_bytes(), raw)
        self.assertFalse((self.destination / "lifecycle/staging").exists())
        for item in self.sources:
            item.revalidate(FILES)
        copied = self.destination / "lifecycle/anchors" / name
        copied.chmod(0o600)
        with self.assertRaises(SELECTION.PaperSelectionError):
            for item in self.sources:
                item.revalidate(FILES)

    def test_empty_query_and_request_only_are_retained(self):
        self.query.mkdir(mode=0o700)
        self.capture()
        self.assertTrue((self.destination / "lifecycle/queries" / self.query.name).is_dir())

    def test_unknown_names_modes_hashes_and_size_refuse(self):
        for mutant in ("unknown", "mode", "hash", "large", "symlink"):
            with self.subTest(mutant=mutant):
                # Independent captures need their own projected directory.
                for item in self.sources:
                    item.close()
                self.sources.clear()
                path = self.source / "selections" / ("a" * 64 + ".json")
                if path.exists() or path.is_symlink():
                    path.unlink()
                unknown = self.source / "unknown"
                if unknown.exists(): unknown.unlink()
                if mutant == "unknown": self.write(unknown)
                elif mutant == "mode": self.write(path, mode=0o600)
                elif mutant == "hash": self.write(path)
                elif mutant == "large": self.write(path, b"x" * 65_537)
                else: path.symlink_to(self.root / "missing")
                with self.assertRaises((SELECTION.PaperSelectionError, OSError)):
                    self.capture()

    def test_membership_contents_names_ancestors_and_bounds_revalidate(self):
        raw=b"{}"
        name=hashlib.sha256(raw).hexdigest()+".json"
        original=self.write(self.source / "anchors" / name,raw)
        self.capture()
        # Same inode mutation is observed through retained bytes/metadata.
        original.chmod(0o600)
        original.write_bytes(b"[]")
        with self.assertRaises(SELECTION.PaperSelectionError):
            for item in self.sources: item.revalidate(FILES)
        with self.assertRaises(SELECTION.PaperSelectionError):
            SELECTION._OriginalDirectory(FILES,str(self.source),maximum=3)
        renamed=self.source.with_name("old-lifecycle")
        self.source.rename(renamed)
        self.source.mkdir(mode=0o700)
        with self.assertRaises(SELECTION.PaperSelectionError):
            for item in self.sources: item.revalidate(FILES)

    def test_membership_late_insert_and_absent_root_are_bound(self):
        self.capture()
        self.write(self.source / "queries" / "unexpected")
        with self.assertRaises(SELECTION.PaperSelectionError):
            for item in self.sources: item.revalidate(FILES)

    def test_root_absence_name_replacement_and_global_capture_limits(self):
        for name in ("queries", "selections", "anchors", "staging"):
            (self.source / name).rmdir()
        self.source.rmdir()
        self.capture()
        self.assertFalse((self.destination / "lifecycle").exists())
        self.source.mkdir(mode=0o700)
        with self.assertRaises(SELECTION.PaperSelectionError):
            for item in self.sources: item.revalidate(FILES)

    def test_global_entry_and_byte_limit_and_original_name_replacement(self):
        self.query.mkdir(mode=0o700)
        (self.query / "output").mkdir(mode=0o700)
        self.write(self.query / "request.json", b"x" * 100)
        with self.assertRaises(SELECTION.PaperSelectionError):
            self.capture(entries=6)
        with self.assertRaises(SELECTION.PaperSelectionError):
            self.capture(total=50)
        source = SELECTION._OriginalFile(FILES,str(self.query / "request.json"),uid=0,gid=0,mode=0o400,maximum=100)
        self.sources.append(source)
        (self.query / "request.json").rename(self.query / "old")
        self.write(self.query / "request.json", b"x" * 100)
        with self.assertRaises(SELECTION.PaperSelectionError): source.revalidate(FILES)

    def test_fixed_shape_and_q_specific_limit(self):
        q=("queries",self.query.name)
        self.assertEqual(SELECTION._lifecycle_shape((*q,"sql-request.json"),False),1_048_576)
        self.assertEqual(SELECTION._lifecycle_shape((*q,"evidence.json"),False),65_536)
        self.assertFalse(SELECTION._lifecycle_shape(("staging",),True))
        self.assertFalse(SELECTION._lifecycle_shape(("queries","UPPER"),True))
        self.assertFalse(SELECTION._lifecycle_shape((*q,"output","dispatch.json"),False))


if __name__ == "__main__":
    unittest.main()
