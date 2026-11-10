#!/usr/bin/env python3
"""Pure source-selection contract checks; no installed capability is forged."""
from __future__ import annotations

import contextlib
import hashlib
import os
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


if __name__ == "__main__":
    unittest.main()
