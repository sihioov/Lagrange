#!/usr/bin/env python3
"""Focused stdlib tests for the immutable Paper release binding."""
from __future__ import annotations

import hashlib
import contextlib
import io
import importlib.util
import json
import os
import stat
import struct
import subprocess
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
MODULE = ROOT / "scripts/ops/lib/owner-beta-paper-release.py"
SPEC = importlib.util.spec_from_file_location("owner_beta_paper_release_test", MODULE)
assert SPEC is not None and SPEC.loader is not None
release = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = release
SPEC.loader.exec_module(release)
COMMIT = "a1" * 20


def elf_fixture(commit, section_name=".rodata", section_flags=0x2):
    """Build a minimal linux/amd64 ELF fixture with a compile-identity section."""
    marker = commit.encode("ascii")
    names = b"\x00" + section_name.encode("ascii") + b"\x00.shstrtab\x00"
    section_name_offset = 1
    strings_name_offset = names.index(b".shstrtab")
    section_table_offset = 64
    data_offset = section_table_offset + 3 * 64
    strings_offset = data_offset + len(marker)
    result = bytearray(strings_offset + len(names))
    result[:16] = b"\x7fELF\x02\x01\x01" + bytes(9)
    struct.pack_into("<HHIQQQIHHHHHH", result, 16,
                     2, 62, 1, 0, 0, section_table_offset, 0,
                     64, 0, 0, 64, 3, 2)
    struct.pack_into("<IIQQQQIIQQ", result, section_table_offset + 64,
                     section_name_offset, 1, section_flags, 0,
                     data_offset, len(marker), 0, 0, 1, 0)
    struct.pack_into("<IIQQQQIIQQ", result, section_table_offset + 128,
                     strings_name_offset, 3, 0, 0,
                     strings_offset, len(names), 0, 0, 1, 0)
    result[data_offset:data_offset + len(marker)] = marker
    result[strings_offset:] = names
    return bytes(result)


def manifest_fixture():
    services = (
        "db-role-bootstrap", "db-migrate", "api-server", "web", "research-worker",
        "recommendation-runner", "candidate-runner", "owner-beta-runner",
        "owner-equity-v2-runner", "nt-backtest-worker-1", "nt-backtest-worker-2",
        "paper-scheduler", "owner-beta-simulation-runner",
    )
    rows = [release.MANIFEST_V3, "commit|" + COMMIT]
    ids = {}
    for index, service in enumerate(services, 1):
        image_id = "sha256:" + f"{index:064x}"
        ids[service] = image_id
        rows.append(f"image|{service}|lagrange-station-{service}:{COMMIT}|{image_id}|{COMMIT}")
    data = ("\n".join(rows) + "\n").encode("ascii")
    return data, ids


class PaperReleaseTests(unittest.TestCase):
    def test_manifest_binds_exact_current_images_and_rejects_substitution(self):
        data, ids = manifest_fixture()
        parsed = release._manifest(data, COMMIT, release._digest(data))
        self.assertEqual(parsed, ids)
        changed = data.replace(b"lagrange-station-api-server:", b"lagrange-station-web:", 1)
        with self.assertRaises(release.PaperReleaseError):
            release._manifest(changed, COMMIT, release._digest(changed))
        with self.assertRaises(release.PaperReleaseError):
            release._manifest(data, COMMIT, "sha256:" + "0" * 64)

    def test_image_inspection_uses_real_elf_bytes_for_each_fixed_path(self):
        _, ids = manifest_fixture()
        calls = []
        def extract(image_id, path):
            calls.append((image_id, path))
            return elf_fixture(COMMIT)
        checked = release._actual_elfs(ids, COMMIT, extract)
        self.assertEqual(calls, [
            (ids["api-server"], "/usr/local/bin/api-server"),
            (ids["paper-scheduler"], "/usr/local/bin/paper-runner-bin"),
            (ids["paper-scheduler"], "/usr/local/bin/owner-beta-paper-ops-native-bin"),
        ])
        self.assertTrue(all(value[0].startswith(b"\x7fELF") for value in checked.values()))
        self.assertTrue(all(value[1] == release._digest(value[0]) for value in checked.values()))
        with self.assertRaises(release.PaperReleaseError):
            release._actual_elfs(ids, COMMIT, lambda *_: elf_fixture("b2" * 20))
        with self.assertRaises(release.PaperReleaseError):
            release._actual_elfs(ids, COMMIT, lambda *_: b"caller-supplied-label")

    def test_compile_revision_must_be_in_readonly_elf_data_section(self):
        self.assertTrue(release._compiled_commit_in_elf(elf_fixture(COMMIT), COMMIT))
        self.assertFalse(release._compiled_commit_in_elf(elf_fixture("b2" * 20), COMMIT))
        self.assertFalse(release._compiled_commit_in_elf(
            elf_fixture(COMMIT, section_name=".text", section_flags=0x6), COMMIT))
        self.assertFalse(release._compiled_commit_in_elf(
            elf_fixture(COMMIT, section_name=".rodata", section_flags=0x3), COMMIT))

    def test_sources_fail_closed_when_final_audit_or_projection_is_missing(self):
        with tempfile.TemporaryDirectory(prefix="paper-release-source-") as temporary:
            base = Path(temporary)
            for relative in ("docs/contracts", "migrations", "deploy/db"):
                (base / relative).mkdir(parents=True)
            shutil_paths = {
                release.OPERATIONS_SCHEMA: ROOT / "docs/contracts" / release.OPERATIONS_SCHEMA,
                release.ARCHIVE_SCHEMA: ROOT / "docs/contracts" / release.ARCHIVE_SCHEMA,
                release.MIGRATION: ROOT / release.MIGRATION,
            }
            for relative, source in shutil_paths.items():
                (base / relative).write_bytes(source.read_bytes())
            daily = release._daily()
            def fixture_directory_info(fd, *_args, **_kwargs):
                value = os.fstat(fd)
                if not __import__("stat").S_ISDIR(value.st_mode):
                    raise release.PaperReleaseError("PAPER_RELEASE_FIXTURE")
                return value
            def fixture_read_file(parent, name, maximum, **_kwargs):
                fd = os.open(name, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0), dir_fd=parent)
                try:
                    value = os.fstat(fd)
                    if not __import__("stat").S_ISREG(value.st_mode) or value.st_size > maximum:
                        raise release.PaperReleaseError("PAPER_RELEASE_FIXTURE")
                    return os.read(fd, maximum + 1)
                finally:
                    os.close(fd)
            root_fd = os.open(base, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
            with mock.patch.object(daily, "directory_info", side_effect=fixture_directory_info), \
                    mock.patch.object(daily, "read_file", side_effect=fixture_read_file):
                with self.assertRaises(release.PaperReleaseError):
                    release._source_values(daily, root_fd)
                (base / release.PROJECTION).write_bytes(
                    (ROOT / release.PROJECTION).read_bytes())
                with self.assertRaises(release.PaperReleaseError):
                    release._source_values(daily, root_fd)
                (base / release.AUDIT).write_text("-- fixture audit source\n", encoding="utf-8")
                values, projection = release._source_values(daily, root_fd)
                self.assertTrue(values["audit_sql_sha256"].startswith(b"-- fixture"))
                self.assertEqual(json.loads(projection), json.loads((ROOT / release.PROJECTION).read_bytes()))
            os.close(root_fd)

    def test_process_binding_is_closed_and_actual_revision_and_image_pins_match(self):
        process = {
            "schema": release.PROCESS_SCHEMA,
            "owner_user_id": "11111111-1111-4111-8111-111111111111",
            "account_id": "22222222-2222-4222-8222-222222222222",
            "strategy_config_id": "33333333-3333-4333-8333-333333333333",
            "consent_event_id": "44444444-4444-4444-8444-444444444444",
            "consent_version": "owner-beta-paper-v1-draft-1",
            "control_epoch": 1,
            "daily_cohort_sha256": "sha256:" + "1" * 64,
            "financial_contract_sha256": "sha256:" + "2" * 64,
            "code_commit": COMMIT,
            "release_manifest_sha256": "sha256:" + "3" * 64,
            "api_binary_sha256": "sha256:" + "4" * 64,
            "worker_binary_sha256": "sha256:" + "5" * 64,
        }
        self.assertEqual(release._validate_process(process, COMMIT, process["release_manifest_sha256"]), process)
        widened = {**process, "ambient_ready": True}
        with self.assertRaises(release.PaperReleaseError):
            release._validate_process(widened, COMMIT, process["release_manifest_sha256"])
        with self.assertRaises(release.PaperReleaseError):
            release._validate_process(process, COMMIT, "sha256:" + "0" * 64)
        with self.assertRaises(release.PaperReleaseError):
            release._validate_process(process, "b2" * 20, process["release_manifest_sha256"])
        with self.assertRaises(release.PaperReleaseError):
            release._validate_process(process, COMMIT, process["release_manifest_sha256"],
                                      "sha256:" + "0" * 64)

    def test_floor_and_unknown_inventory_do_not_downgrade(self):
        self.assertEqual(release.FLOOR_BYTES, release._canonical(release.FLOOR_VALUE))
        self.assertEqual(release.FLOOR_BYTES,
                         b'{"minimum":1,"schema":"owner-beta-paper-reader-floor-v1"}')
        self.assertEqual(set(release.FLOOR_VALUE), {"schema", "minimum"})
        target = "/opt/lagrange/releases/" + COMMIT
        for unknown in (None, 0, 1, "unknown"):
            with self.subTest(value=unknown), self.assertRaises(release.PaperReleaseError):
                release.reader_guard("/opt/lagrange", target, COMMIT, unknown)

    def test_reader_guard_cli_keeps_missing_db_inventory_unknown(self):
        target = "/opt/lagrange/releases/" + COMMIT
        with contextlib.redirect_stderr(io.StringIO()):
            result = release.main([
                "reader-guard", "--install-root", "/opt/lagrange",
                "--target-release", target, "--commit", COMMIT,
            ])
        self.assertEqual(result, 1)

    def test_daily_activation_passes_unknown_paper_inventory(self):
        path = ROOT / "scripts/ops/lib/owner-beta-daily-operations.py"
        spec = importlib.util.spec_from_file_location("daily_operations_paper_guard_test", path)
        daily_ops = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = daily_ops
        spec.loader.exec_module(daily_ops)
        paper_guard = mock.Mock(return_value="PAPER_READER_REQUIRED")
        paper_module = types.SimpleNamespace(reader_guard=paper_guard)
        values = {
            "LAGRANGE_CODE_COMMIT": COMMIT,
            "LAGRANGE_DATA_DIR": "/data/daily",
            "LAGRANGE_ARTIFACTS_DIR": "/data/artifacts",
            "LAGRANGE_RUNTIME_SECRET_DIR": "/run/secrets",
            "POSTGRES_DB": "lagrange",
            "MAX_JOBS_PER_OWNER": "10",
            "PRICE_BETA_LEASE_SECS": "60",
            "PRICE_BETA_BACKOFF_SECS": "30",
            "OWNER_BETA_DAILY_MODE": "owner_only",
            "OWNER_BETA_ACCESS_MODE": "owner_only",
            "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1",
            "OWNER_BETA_PAPER_MODE": "disabled",
        }
        release_path = "/opt/lagrange/releases/" + COMMIT
        with mock.patch.object(daily_ops.install, "verify_config", return_value=({}, "sha256:" + "1" * 64)), \
                mock.patch.object(daily_ops, "load_local", return_value=paper_module):
            daily_ops.activation_binding(release_path, COMMIT, "lagrange-station",
                                         {"owner-beta-runner": "sha256:" + "2" * 64}, values)
        paper_guard.assert_called_once_with("/opt/lagrange", release_path, COMMIT,
                                            history_present=None, reserve=True)

    def test_install_receipt_binds_projection_capability_manifest_and_commit(self):
        manifest = "sha256:" + "1" * 64
        capability = "sha256:" + "2" * 64
        projection = "sha256:" + "3" * 64
        receipt = json.loads(release._install_receipt(COMMIT, manifest, capability, projection))
        self.assertEqual(receipt, {
            "schema": release.INSTALL_RECEIPT_SCHEMA,
            "code_commit": COMMIT,
            "release_manifest_sha256": manifest,
            "capability_sha256": capability,
            "audit_projection_sha256": projection,
            "process_sha256": None,
        })
        self.assertNotEqual(release._install_receipt(COMMIT, manifest, capability, projection),
                            release._install_receipt(COMMIT, manifest, capability, "sha256:" + "4" * 64))

    def test_verify_revalidate_close_owns_real_temporary_descriptors(self):
        with tempfile.TemporaryDirectory(prefix="paper-verified-release-") as temporary:
            install = Path(temporary) / "opt" / "lagrange"
            target = install / "releases" / COMMIT
            paper = target / "deploy" / "owner-beta-paper"
            paper.mkdir(parents=True)
            os.chmod(install, 0o755)
            os.chmod(install / "releases", 0o755)
            os.chmod(target, 0o755)
            os.chmod(target / "deploy", 0o755)
            os.chmod(paper, 0o750)

            manifest, image_ids = manifest_fixture()
            manifest_sha = release._digest(manifest)
            (target / release.MANIFEST_NAME).write_bytes(manifest)
            os.chmod(target / release.MANIFEST_NAME, 0o600)
            sources = {
                "operations_schema_sha256": (ROOT / "docs/contracts" / release.OPERATIONS_SCHEMA).read_bytes(),
                "archive_schema_sha256": (ROOT / "docs/contracts" / release.ARCHIVE_SCHEMA).read_bytes(),
                "migration_0068_sha256": (ROOT / release.MIGRATION).read_bytes(),
                "audit_sql_sha256": b"-- fixture audit SQL\n",
            }
            projection = (ROOT / release.PROJECTION).read_bytes()
            release_sources = {
                release.OPERATIONS_SCHEMA: sources["operations_schema_sha256"],
                release.ARCHIVE_SCHEMA: sources["archive_schema_sha256"],
                release.MIGRATION: sources["migration_0068_sha256"],
                release.AUDIT: sources["audit_sql_sha256"],
                release.PROJECTION: projection,
            }
            for relative, contents in release_sources.items():
                path = target / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                for directory in path.parents:
                    if directory == target:
                        break
                    if target in directory.parents:
                        os.chmod(directory, 0o755)
                path.write_bytes(contents)
                os.chmod(path, 0o444)
            source_names = {
                "operations_schema_sha256": "operations.schema.json",
                "archive_schema_sha256": "archive.schema.json",
                "migration_0068_sha256": "migration-0068.up.sql",
                "audit_sql_sha256": "audit.sql",
            }
            for key, name in source_names.items():
                (paper / name).write_bytes(sources[key])
                os.chmod(paper / name, 0o444)
            elf_data = elf_fixture(COMMIT)
            for name in ("api-image.elf", "worker-image.elf", "native-image.elf"):
                (paper / name).write_bytes(elf_data)
                os.chmod(paper / name, 0o500 if name == "native-image.elf" else 0o400)
            capability = {
                "schema": release.CAPABILITY_SCHEMA,
                "minimum": 1,
                "code_commit": COMMIT,
                "release_manifest_sha256": manifest_sha,
                **{key: release._digest(value) for key, value in sources.items()},
                "api_binary_sha256": release._digest(elf_data),
                "worker_binary_sha256": release._digest(elf_data),
                "native_binary_sha256": release._digest(elf_data),
            }
            cap_bytes = release._canonical(capability)
            (paper / "capability.json").write_bytes(cap_bytes)
            os.chmod(paper / "capability.json", 0o444)
            (target / release.INSTALL_RECEIPT_NAME).write_bytes(release._install_receipt(
                COMMIT, manifest_sha, release._digest(cap_bytes), release._digest(projection)))
            os.chmod(target / release.INSTALL_RECEIPT_NAME, 0o600)

            daily = release._daily()

            # The fixture keeps actual openat descriptors/files and path identity;
            # only root ownership/ACL checks are replaced for portable stdlib CI.
            def fixture_directory_info(fd, uid=0, gid=None, mode=None):
                value = os.fstat(fd)
                if not stat.S_ISDIR(value.st_mode):
                    raise release.PaperReleaseError("PAPER_RELEASE_FIXTURE_METADATA")
                if mode is not None and stat.S_IMODE(value.st_mode) != mode:
                    raise release.PaperReleaseError("PAPER_RELEASE_FIXTURE_METADATA")
                return value

            def fixture_file_info(fd, uid, gid=None, mode=None):
                value = os.fstat(fd)
                if not stat.S_ISREG(value.st_mode) or value.st_nlink != 1:
                    raise release.PaperReleaseError("PAPER_RELEASE_FIXTURE_METADATA")
                if mode is not None and stat.S_IMODE(value.st_mode) != mode:
                    raise release.PaperReleaseError("PAPER_RELEASE_FIXTURE_METADATA")
                return value

            def fixture_create_file(parent, name, data, uid, gid, mode):
                fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL |
                             getattr(os, "O_NOFOLLOW", 0), 0o600, dir_fd=parent)
                try:
                    offset = 0
                    while offset < len(data):
                        offset += os.write(fd, data[offset:])
                    os.fchmod(fd, mode)
                    os.fsync(fd)
                finally:
                    os.close(fd)
                os.fsync(parent)

            actual = {
                key: (elf_data, release._digest(elf_data))
                for key in ("api_binary_sha256", "worker_binary_sha256", "native_binary_sha256")
            }
            with mock.patch.object(daily, "require_root"), \
                    mock.patch.object(daily, "directory_info", side_effect=fixture_directory_info), \
                    mock.patch.object(daily, "file_info", side_effect=fixture_file_info), \
                    mock.patch.object(daily, "create_file", side_effect=fixture_create_file), \
                    mock.patch.object(release, "_actual_elfs", return_value=actual):
                # Missing protected operation directory and floor do not prove
                # that the database contains no Paper history. In particular,
                # a caller-supplied false is not an inventory ABI.
                self.assertFalse((install / release.FLOOR_NAME).exists())
                self.assertFalse((install / ".owner-beta-paper").exists())
                self.assertFalse((install / ".owner-beta-paper/operations").exists())
                self.assertFalse((target / "deploy/owner-beta-paper/process.json").exists())
                with self.assertRaises(release.PaperReleaseError):
                    release.reader_guard(str(install), str(target), COMMIT, False)
                with self.assertRaises(release.PaperReleaseError):
                    release.reader_guard(str(install), str(target), COMMIT, None)
                self.assertEqual(release.reader_guard(str(install), str(target), COMMIT, True),
                                 "PAPER_READER_REQUIRED")
                floor = install / release.FLOOR_NAME
                self.assertEqual(floor.read_bytes(), release.FLOOR_BYTES)
                self.assertEqual(stat.S_IMODE(floor.stat().st_mode), 0o600)
                floor.write_bytes(b'{"schema":"owner-beta-paper-reader-floor-v1","minimum":1}')
                with self.assertRaises(release.PaperReleaseError):
                    release.reader_guard(str(install), str(target), COMMIT, False)
                floor.write_bytes(release.FLOOR_BYTES)
                os.chmod(floor, 0o600)
                handle = release.verify(str(install), str(target), COMMIT)
                self.assertIsInstance(handle, release.VerifiedPaperRelease)
                self.assertIsNone(handle.revalidate())
                operation_directory = Path(temporary) / "paper-ops"
                operation_directory.mkdir(mode=0o700)
                operation_fd = os.open(operation_directory, daily.DIR_FLAGS)
                try:
                    self.assertIsNone(handle.stage_native_inputs(operation_fd))
                    self.assertEqual(set(os.listdir(operation_fd)), {
                        "capability.json", "api-image.elf", "worker-image.elf",
                        "operations.schema.json", "archive.schema.json",
                        "migration-0068.up.sql", "audit.sql",
                    })
                    for name in os.listdir(operation_fd):
                        self.assertEqual(stat.S_IMODE(os.stat(name, dir_fd=operation_fd,
                                                               follow_symlinks=False).st_mode), 0o400)
                finally:
                    os.close(operation_fd)
                owned = set(handle._fds)
                self.assertGreater(len(owned), 5)
                handle.close()
                self.assertEqual(handle._fds, ())
                reused_fd = os.open("/dev/null", os.O_RDONLY)
                try:
                    self.assertIn(reused_fd, owned)
                    handle.close()
                    handle.__del__()
                    os.fstat(reused_fd)
                    with self.assertRaises(release.PaperReleaseError):
                        handle.revalidate()
                finally:
                    os.close(reused_fd)

                receipt = json.loads((target / release.INSTALL_RECEIPT_NAME).read_bytes())
                receipt["audit_projection_sha256"] = "sha256:" + "f" * 64
                (target / release.INSTALL_RECEIPT_NAME).write_bytes(release._canonical(receipt))
                with self.assertRaises(release.PaperReleaseError):
                    release.verify(str(install), str(target), COMMIT)

    def test_verified_handle_has_no_public_constructor(self):
        with self.assertRaises(TypeError):
            release.VerifiedPaperRelease(None, install_root="/opt/lagrange")

    def test_daily_shell_override_cannot_turn_paper_on(self):
        shell = r'''
set -e
source "$1"
declare -A DOTENV_VALUES=([OWNER_BETA_DAILY_MODE]=disabled [OWNER_BETA_PAPER_MODE]=disabled)
dotenv_has() { [[ -v DOTENV_VALUES[$1] ]]; }
OWNER_BETA_PAPER_MODE=owner_only
daily_release_parse_mode
'''
        result = subprocess.run(["bash", "-c", shell, "paper-shell-gate",
                                 str(ROOT / "scripts/ops/lib/owner-beta-daily-release.sh")],
                                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, check=False)
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
