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
import tarfile
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
PRODUCER_BINS = ("api-server", "paper-runner", "owner-beta-paper-ops-native")


def producer_pair(bin_name: str, binary: bytes, source_commit: str = COMMIT) -> dict[str, bytes]:
    artifact = {
        "binary_mode": "0755", "binary_sha256": hashlib.sha256(binary).hexdigest(),
        "bin": bin_name, "cache_key": "1" * 64, "cargo_success": True,
        "compile_env": {"CARGO_BUILD_JOBS": "2", "CARGO_TARGET_DIR": "/cargo-target",
                        "RUSTFLAGS": "<unset>", "LAGRANGE_CODE_COMMIT": source_commit},
        "features": [], "format": "lagrange-rust-artifact-v1", "h_sha256": "2" * 64,
        "host_triple": "x86_64-unknown-linux-musl", "input_sha256": "3" * 64,
        "package": "api-server", "platform": "linux/amd64", "profile": "release",
        "recipe_sha256": "4" * 64, "source_commit": source_commit,
    }
    raw = release._canonical(artifact) + b"\n"
    return {"artifact.json": raw,
            "COMPLETE": (hashlib.sha256(raw).hexdigest() + "\n").encode("ascii")}


def producer_receipts(binary: bytes) -> dict[str, dict[str, bytes]]:
    return {name: producer_pair(name, binary) for name in PRODUCER_BINS}


def elf_fixture(commit):
    """Build a small synthetic ELF fixture; its marker is not compile proof."""
    marker = commit.encode("ascii")
    names = b"\x00.rodata\x00.shstrtab\x00"
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
                     section_name_offset, 1, 0x2, 0,
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
        pairs = producer_receipts(elf_fixture(COMMIT))
        def extract_receipts(image_id, expected_bins):
            expected_image = ids["api-server"] if expected_bins == ("api-server",) else ids["paper-scheduler"]
            self.assertEqual(image_id, expected_image)
            return {name: pairs[name] for name in expected_bins}
        checked = release._actual_elfs(ids, COMMIT, extract, extract_receipts)
        self.assertEqual(calls, [
            (ids["api-server"], "/usr/local/bin/api-server"),
            (ids["paper-scheduler"], "/usr/local/bin/paper-runner-bin"),
            (ids["paper-scheduler"], "/usr/local/bin/owner-beta-paper-ops-native-bin"),
        ])
        binary_keys = ("api_binary_sha256", "worker_binary_sha256", "native_binary_sha256")
        self.assertTrue(all(checked[key][0].startswith(b"\x7fELF") for key in binary_keys))
        self.assertTrue(all(checked[key][1] == release._digest(checked[key][0]) for key in binary_keys))
        self.assertEqual(checked["producer_receipts"], pairs)
        with self.assertRaises(release.PaperReleaseError):
            release._actual_elfs(ids, COMMIT, lambda *_: elf_fixture("b2" * 20), extract_receipts)
        with self.assertRaises(release.PaperReleaseError):
            release._actual_elfs(ids, COMMIT, lambda *_: b"caller-supplied-label", extract_receipts)

    def test_producer_receipt_fields_and_complete_are_bound_to_exact_elf(self):
        binary = elf_fixture(COMMIT)
        valid = producer_pair("paper-runner", binary)
        self.assertEqual(release._producer_receipt_pair(
            "paper-runner", valid["artifact.json"], valid["COMPLETE"], COMMIT,
            hashlib.sha256(binary).hexdigest()), valid)
        mutations = (
            lambda value: value.update(source_commit="b2" * 20),
            lambda value: value.update(bin="api-server"),
            lambda value: value.update(package="job-queue"),
            lambda value: value["compile_env"].update(LAGRANGE_CODE_COMMIT="b2" * 20),
            lambda value: value["compile_env"].update(CARGO_BUILD_JOBS="99"),
            lambda value: value.update(binary_sha256="f" * 64),
            lambda value: value.update(recipe_sha256="F" * 64),
        )
        for mutate in mutations:
            value = json.loads(valid["artifact.json"])
            mutate(value)
            raw = release._canonical(value) + b"\n"
            complete = (hashlib.sha256(raw).hexdigest() + "\n").encode("ascii")
            with self.assertRaises(release.PaperReleaseError):
                release._producer_receipt_pair("paper-runner", raw, complete, COMMIT,
                                                hashlib.sha256(binary).hexdigest())
        with self.assertRaises(release.PaperReleaseError):
            release._producer_receipt_pair("paper-runner", valid["artifact.json"],
                                            b"0" * 64 + b"\n", COMMIT,
                                            hashlib.sha256(binary).hexdigest())

    def test_image_provenance_directory_is_closed_root_owned_and_bounded(self):
        expected = {"api-server": producer_pair("api-server", elf_fixture(COMMIT))}

        def archive_bytes(bin_names=("api-server",), root_mode=0o755, directory_mode=0o755,
                          artifact_mode=0o444, include_complete=True, extra=False,
                          symlink=False, mixed_bin=False):
            output = io.BytesIO()
            with tarfile.open(fileobj=output, mode="w") as archive:
                root = tarfile.TarInfo("owner-beta-paper-provenance/")
                root.type = tarfile.DIRTYPE
                root.mode = root_mode
                root.uid = root.gid = 0
                archive.addfile(root)
                for bin_name in bin_names:
                    directory = tarfile.TarInfo(f"owner-beta-paper-provenance/{bin_name}/")
                    directory.type = tarfile.DIRTYPE
                    directory.mode = directory_mode
                    directory.uid = directory.gid = 0
                    archive.addfile(directory)
                    pair = producer_pair(bin_name, elf_fixture(COMMIT))
                    for name in (("artifact.json", "COMPLETE") if include_complete else ("artifact.json",)):
                        path_bin = "other" if mixed_bin and name == "artifact.json" else bin_name
                        if symlink and name == "COMPLETE":
                            link = tarfile.TarInfo(f"owner-beta-paper-provenance/{path_bin}/COMPLETE")
                            link.type = tarfile.SYMTYPE
                            link.linkname = "artifact.json"
                            archive.addfile(link)
                            continue
                        item = tarfile.TarInfo(f"owner-beta-paper-provenance/{path_bin}/{name}")
                        item.uid = item.gid = 0
                        item.mode = artifact_mode if name == "artifact.json" else 0o444
                        content = pair[name]
                        item.size = len(content)
                        archive.addfile(item, io.BytesIO(content))
                if extra:
                    item = tarfile.TarInfo("owner-beta-paper-provenance/extra")
                    item.mode = 0o444
                    item.uid = item.gid = 0
                    item.size = 1
                    archive.addfile(item, io.BytesIO(b"x"))
            return output.getvalue()

        def run_with(archive_data):
            def fake_run(argv, **_kwargs):
                if argv[1] == "create":
                    return subprocess.CompletedProcess(argv, 0, b"abcdef123456\n", b"")
                if argv[1] == "cp":
                    return subprocess.CompletedProcess(argv, 0, archive_data, b"")
                return subprocess.CompletedProcess(argv, 0, b"", b"")
            return mock.patch.object(release.subprocess, "run", side_effect=fake_run)

        with run_with(archive_bytes()):
            self.assertEqual(release._extract_image_provenance("sha256:" + "1" * 64,
                                                               ("api-server",)), expected)
        for bad in (archive_bytes(root_mode=0o777), archive_bytes(directory_mode=0o777),
                    archive_bytes(artifact_mode=0o644),
                    archive_bytes(include_complete=False), archive_bytes(extra=True),
                    archive_bytes(symlink=True), archive_bytes(mixed_bin=True)):
            with run_with(bad), self.assertRaises(release.PaperReleaseError):
                release._extract_image_provenance("sha256:" + "1" * 64, ("api-server",))
        all_pairs = producer_receipts(elf_fixture(COMMIT))
        d7_pairs = {name: all_pairs[name] for name in ("paper-runner", "owner-beta-paper-ops-native")}
        with run_with(archive_bytes(bin_names=("paper-runner", "owner-beta-paper-ops-native"))):
            observed = release._extract_image_provenance(
                "sha256:" + "2" * 64, ("paper-runner", "owner-beta-paper-ops-native"))
        self.assertEqual(observed, d7_pairs)

    def test_source_builder_route_stays_buildable_but_missing_official_receipts_fail_closed(self):
        api_dockerfile = (ROOT / "crates/api-server/Dockerfile").read_text(encoding="utf-8")
        paper_dockerfile = (ROOT / "deploy/runtime/Dockerfile.paper-runner").read_text(encoding="utf-8")
        for dockerfile in (api_dockerfile, paper_dockerfile):
            self.assertIn("mkdir -p /build/target/release /build/provenance", dockerfile)
            self.assertIn("COPY --from=builder /build/provenance/ /usr/share/lagrange/owner-beta-paper-provenance/",
                          dockerfile)
            self.assertIn('ENV LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT"', dockerfile)
        _, ids = manifest_fixture()
        def extract_elf(_image, _path):
            return elf_fixture(COMMIT)
        def absent_receipt(_image, _bins):
            raise release.PaperReleaseError("PAPER_RELEASE_PRODUCER_MISSING")
        with self.assertRaises(release.PaperReleaseError):
            release._actual_elfs(ids, COMMIT, extract_elf, absent_receipt)

    def test_elf_target_format_is_checked_without_treating_literals_as_identity(self):
        data = elf_fixture(COMMIT)
        self.assertTrue(release._elf64_x86_64(data))
        self.assertTrue(release._elf64_x86_64(elf_fixture("b2" * 20)))
        wrong_machine = bytearray(data)
        wrong_machine[18:20] = (183).to_bytes(2, "little")
        self.assertFalse(release._elf64_x86_64(bytes(wrong_machine)))
        self.assertFalse(release._elf64_x86_64(b"caller-supplied-label"))

    def test_sources_fail_closed_when_final_audit_or_projection_is_missing(self):
        with tempfile.TemporaryDirectory(prefix="paper-release-source-") as temporary:
            base = Path(temporary)
            for relative in ("docs/contracts", "migrations", "deploy/db"):
                (base / relative).mkdir(parents=True)
            shutil_paths = {
                "docs/contracts/" + release.OPERATIONS_SCHEMA: ROOT / "docs/contracts" / release.OPERATIONS_SCHEMA,
                "docs/contracts/" + release.ARCHIVE_SCHEMA: ROOT / "docs/contracts" / release.ARCHIVE_SCHEMA,
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
                # Match the real Git archive layout. A root-level copy cannot
                # substitute for the checked-in docs/contracts source.
                schema = base / "docs/contracts" / release.OPERATIONS_SCHEMA
                original = schema.read_bytes()
                misplaced = base / release.OPERATIONS_SCHEMA
                schema.rename(misplaced)
                with self.assertRaises(release.PaperReleaseError) as caught:
                    release._source_values(daily, root_fd)
                self.assertEqual(caught.exception.code, "PAPER_RELEASE_SOURCE")
                misplaced.rename(schema)
                wrong = json.loads(original)
                wrong["$id"] = "wrong-schema.json"
                schema.write_text(json.dumps(wrong), encoding="utf-8")
                with self.assertRaises(release.PaperReleaseError) as caught:
                    release._source_values(daily, root_fd)
                self.assertEqual(caught.exception.code, "PAPER_RELEASE_SOURCE")
                schema.write_bytes(original)
                restored, _ = release._source_values(daily, root_fd)
                self.assertEqual(restored["operations_schema_sha256"], original)
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

    def test_protected_input_cli_preserves_failure_and_only_finite_daily_guard(self):
        class DailyError(Exception):
            pass

        args = ["stage", "--install-root", "/opt/lagrange", "--target-release",
                "/opt/lagrange/releases/.staging." + COMMIT + ".fixture", "--commit", COMMIT,
                "--manifest-sha256", "sha256:" + "1" * 64]
        invalid = ("SECRET_SENTINEL", "DAILY_INSTALL_PATH\nSECRET_SENTINEL",
                   "DAILY_INSTALL_PATН", "", None, True, 42)
        for guard in (*sorted(release._DAILY_GUARDS), *invalid):
            def stage(*_args):
                raise DailyError(guard)

            stderr = io.StringIO()
            with mock.patch.dict(sys.modules, {release._DAILY_NAME: types.SimpleNamespace(
                    DailyInstallError=DailyError)}), mock.patch.object(
                    release, "stage", release._boundary(stage)), contextlib.redirect_stderr(stderr):
                self.assertEqual(release.main(args), 1)
            lines = stderr.getvalue().splitlines()
            self.assertEqual(lines[0], "PAPER_RELEASE_PROTECTED_INPUT")
            if type(guard) is str and guard in release._DAILY_GUARDS:
                self.assertEqual(len(lines), 2)
                self.assertEqual(json.loads(lines[1].split(":", 1)[1]),
                                 {"stage": "STAGE", "daily_guard": guard})
            else:
                self.assertEqual(len(lines), 1)
            self.assertNotIn("SECRET_SENTINEL", stderr.getvalue())

    def test_protected_input_diagnostic_rejects_open_or_tampered_records(self):
        valid = {"stage": "STAGE", "daily_guard": "DAILY_INSTALL_PATH"}
        self.assertEqual(release._diagnostic(valid), valid)
        for value in (None, [], {**valid, "path": "SECRET_SENTINEL"},
                      {**valid, "stage": "SECRET_SENTINEL"},
                      {**valid, "daily_guard": "DAILY_INSTALL_SECRET_SENTINEL"}):
            self.assertIsNone(release._diagnostic(value))

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
            "producer_receipts_sha256": None,
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
                "docs/contracts/" + release.OPERATIONS_SCHEMA: sources["operations_schema_sha256"],
                "docs/contracts/" + release.ARCHIVE_SCHEMA: sources["archive_schema_sha256"],
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
            expected_receipts = producer_receipts(elf_data)
            provenance_root = paper / "provenance"
            provenance_root.mkdir(mode=0o700)
            os.chmod(provenance_root, 0o700)
            for bin_name, pair in expected_receipts.items():
                bin_directory = provenance_root / bin_name
                bin_directory.mkdir(mode=0o700)
                os.chmod(bin_directory, 0o700)
                for file_name, raw in pair.items():
                    (bin_directory / file_name).write_bytes(raw)
                    os.chmod(bin_directory / file_name, 0o444)
            (target / release.INSTALL_RECEIPT_NAME).write_bytes(release._install_receipt(
                COMMIT, manifest_sha, release._digest(cap_bytes), release._digest(projection),
                producer_receipts_sha256=release._producer_receipts_sha256(expected_receipts)))
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
            actual["producer_receipts"] = expected_receipts
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
                        "migration-0068.up.sql", "audit.sql", "provenance",
                    })
                    for name in os.listdir(operation_fd):
                        mode = stat.S_IMODE(os.stat(name, dir_fd=operation_fd,
                                                     follow_symlinks=False).st_mode)
                        self.assertEqual(mode, 0o700 if name == "provenance" else 0o400)
                    native_provenance = operation_directory / "provenance"
                    self.assertEqual(set(os.listdir(native_provenance)), set(PRODUCER_BINS))
                    for bin_name in PRODUCER_BINS:
                        bin_directory = native_provenance / bin_name
                        self.assertEqual(stat.S_IMODE(bin_directory.stat().st_mode), 0o700)
                        self.assertEqual(set(os.listdir(bin_directory)), {"artifact.json", "COMPLETE"})
                        for file_name in ("artifact.json", "COMPLETE"):
                            self.assertEqual((bin_directory / file_name).read_bytes(),
                                             expected_receipts[bin_name][file_name])
                            self.assertEqual(stat.S_IMODE((bin_directory / file_name).stat().st_mode),
                                             0o400)
                    staged_before = (native_provenance / "paper-runner/artifact.json").read_bytes()
                    with self.assertRaises(OSError):
                        handle.stage_native_inputs(operation_fd)
                    self.assertEqual((native_provenance / "paper-runner/artifact.json").read_bytes(),
                                     staged_before)
                    release_receipt = provenance_root / "paper-runner/artifact.json"
                    release_receipt.chmod(0o644)
                    release_receipt.write_bytes(b"substituted after validation")
                    with self.assertRaises(release.PaperReleaseError):
                        handle.revalidate()
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
