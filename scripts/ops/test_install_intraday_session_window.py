"""Offline tests for the operational session-window installer seams."""

from __future__ import annotations

import datetime as dt
import hashlib
import importlib.util
import json
import os
import shutil
import tempfile
from pathlib import Path
import sys
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "ops" / "install-intraday-session-window.py"


def load_module():
    spec = importlib.util.spec_from_file_location("intraday_window_installer", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load installer module")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class InstallIntradaySessionWindowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.module = load_module()
        cls.now = dt.datetime(2026, 9, 14, 3, 0, tzinfo=dt.timezone.utc)

    def window_bytes(self, **changes):
        entry = {
            "date": "2026-09-14",
            "disposition": "REGULAR",
            "open_local": "09:00:00",
            "close_local": "15:30:00",
            "evidence_url": "https://global.krx.co.kr/contents/test",
            "evidence_retrieved_at": "2026-09-14T09:00:00+09:00",
            "evidence_sha256": "sha256:" + "a" * 64,
        }
        entry.update(changes.pop("entry", {}))
        document = {
            "schema_version": 1,
            "exchange": "KRX",
            "timezone": "Asia/Seoul",
            "entries": [entry],
        }
        document.update(changes)
        return json.dumps(document, separators=(",", ":")).encode()

    def test_valid_document_is_hashable_and_requires_current_date_entry(self):
        module = self.module
        raw = self.window_bytes()
        prepared = module.parse_window_document(raw, "2026-09-14", self.now)
        self.assertEqual(prepared.raw, raw)
        self.assertEqual(prepared.window_sha256, "sha256:" + hashlib.sha256(raw).hexdigest())
        self.assertEqual(prepared.requested_date, dt.date(2026, 9, 14))

    def test_parser_rejects_duplicate_nan_unknown_and_noncanonical_input(self):
        module = self.module
        with self.assertRaisesRegex(module.InstallerError, "JSON_INVALID"):
            module.parse_activation(b'{"schema_version":1,"schema_version":1,"window_sha256":"sha256:' + b"a" * 64 + b'"}')
        with self.assertRaisesRegex(module.InstallerError, "JSON_INVALID"):
            module._parse_json(b"NaN", module.MAX_WINDOW_BYTES)
        with self.assertRaisesRegex(module.InstallerError, "SCHEMA_INVALID"):
            module.parse_window_document(self.window_bytes(extra=True), "2026-09-14", self.now)
        with self.assertRaisesRegex(module.InstallerError, "EVIDENCE_INVALID"):
            module.parse_window_document(
                self.window_bytes(entry={"evidence_url": "https://global.krx.co.kr.evil/test"}),
                "2026-09-14",
                self.now,
            )

    def test_parser_rejects_future_or_wrong_kst_evidence_and_missing_date(self):
        module = self.module
        for timestamp in [
            "2026-09-14T12:00:01Z",
            "2026-09-13T14:59:59Z",
        ]:
            with self.assertRaisesRegex(module.InstallerError, "EVIDENCE_INVALID"):
                module.parse_window_document(
                    self.window_bytes(entry={"evidence_retrieved_at": timestamp}),
                    "2026-09-14",
                    self.now,
                )
        with self.assertRaisesRegex(module.InstallerError, "DATE_ENTRY_MISSING"):
            module.parse_window_document(
                self.window_bytes(
                    entry={
                        "date": "2026-09-13",
                        "evidence_retrieved_at": "2026-09-13T09:00:00+09:00",
                    }
                ),
                "2026-09-14",
                self.now,
            )
        with self.assertRaisesRegex(module.InstallerError, "DATE_NOT_CURRENT"):
            module.parse_window_document(self.window_bytes(), "2026-09-13", self.now)

    def test_activation_replay_and_filename_are_hash_pinned(self):
        module = self.module
        raw = self.window_bytes()
        prepared = module.parse_window_document(raw, "2026-09-14", self.now)
        activation = json.dumps(
            {"schema_version": 1, "window_sha256": prepared.window_sha256},
            separators=(",", ":"),
        ).encode()
        self.assertEqual(module.parse_activation(activation), prepared.window_sha256)
        self.assertEqual(
            module.window_filename(prepared.window_sha256),
            "windows-" + prepared.window_sha256[7:] + ".json",
        )
        with self.assertRaisesRegex(module.InstallerError, "HASH_INVALID"):
            module.window_filename("sha256:" + "A" * 64)

    def test_metadata_and_path_validation_are_exact_without_root_fixture(self):
        module = self.module
        directory = module.FileMetadata(True, False, False, 0, 10001, 0o750, 2, 0)
        module.validate_directory_metadata(directory)
        with self.assertRaisesRegex(module.InstallerError, "ROOT_METADATA_INVALID"):
            module.validate_directory_metadata(module.FileMetadata(True, True, False, 0, 10001, 0o750, 2, 0))
        file_metadata = module.FileMetadata(False, False, True, 0, 10001, 0o640, 1, 12)
        module.validate_file_metadata(file_metadata)
        with self.assertRaisesRegex(module.InstallerError, "FILE_METADATA_INVALID"):
            module.validate_file_metadata(module.FileMetadata(False, False, True, 0, 10001, 0o640, 2, 12))
        with self.assertRaisesRegex(module.InstallerError, "ROOT_PATH_BROAD"):
            module.validate_operational_paths(None, "/run")
        with self.assertRaisesRegex(module.InstallerError, "PATH_OVERLAP"):
            module.validate_operational_paths("/srv/runtime/input.json", "/srv/runtime")
        with self.assertRaisesRegex(module.InstallerError, "ROOT_PATH_INVALID"):
            module.validate_operational_paths(None, "/srv/../runtime")

    def test_apply_is_root_only_before_target_access(self):
        module = self.module
        raw = self.window_bytes()
        prepared = module.parse_window_document(raw, "2026-09-14", self.now)
        with patch.object(module.os, "geteuid", return_value=1000):
            with self.assertRaisesRegex(module.InstallerError, "ROOT_REQUIRED"):
                module.apply(prepared, "/srv/runtime")


@unittest.skipUnless(os.geteuid() == 0, "root metadata fixture requires root")
class AtomicInstallTests(InstallIntradaySessionWindowTests):
    def setUp(self):
        self.parent = Path(tempfile.mkdtemp(prefix="lagrange-window-qa-", dir="/run"))
        os.chown(self.parent, 0, 10001)
        os.chmod(self.parent, 0o750)
        self.target = self.parent / "intraday-session-windows"

    def tearDown(self):
        shutil.rmtree(self.parent)

    def test_activation_is_last_and_old_version_survives_interruption(self):
        module = self.module
        first = module.parse_window_document(self.window_bytes(), "2026-09-14", self.now)
        second = module.parse_window_document(
            self.window_bytes(entry={"evidence_sha256": "sha256:" + "b" * 64}),
            "2026-09-14", self.now,
        )
        module.apply(first, str(self.target))
        before = (self.target / "activation.json").read_bytes()
        module.apply(first, str(self.target))
        self.assertEqual((self.target / "activation.json").read_bytes(), before)
        with patch.object(module, "_replace_activation", side_effect=module.InstallerError("TEST_STOP")):
            with self.assertRaisesRegex(module.InstallerError, "TEST_STOP"):
                module.apply(second, str(self.target))
        self.assertEqual((self.target / "activation.json").read_bytes(), before)
        module.check(str(self.target), "2026-09-14", self.now)
        module.apply(second, str(self.target))
        module.check(str(self.target), "2026-09-14", self.now)
        self.assertEqual(module.parse_activation((self.target / "activation.json").read_bytes()), second.window_sha256)
        self.assertEqual((self.target / module.window_filename(first.window_sha256)).read_bytes(), first.raw)

    def test_unsafe_parent_or_hardlink_fails_without_activation_change(self):
        module = self.module
        prepared = module.parse_window_document(self.window_bytes(), "2026-09-14", self.now)
        module.apply(prepared, str(self.target))
        before = (self.target / "activation.json").read_bytes()
        link = self.parent / "extra-link"
        os.link(self.target / module.window_filename(prepared.window_sha256), link)
        with self.assertRaisesRegex(module.InstallerError, "FILE_METADATA_INVALID"):
            module.check(str(self.target), "2026-09-14", self.now)
        link.unlink()
        os.chmod(self.parent, 0o770)
        with self.assertRaisesRegex(module.InstallerError, "ROOT_PATH_UNSAFE"):
            module.apply(prepared, str(self.target))
        self.assertEqual((self.target / "activation.json").read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
