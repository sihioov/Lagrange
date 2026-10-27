#!/usr/bin/env python3
"""Synthetic stdlib tests for the protected simulation release helper."""
from __future__ import annotations

import contextlib
import hashlib
import importlib.util
import io
import json
import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts/ops/lib/owner-beta-simulation-release.py"
SPEC = importlib.util.spec_from_file_location("owner_beta_simulation_release_test", MODULE_PATH)
release = importlib.util.module_from_spec(SPEC)
assert SPEC is not None and SPEC.loader is not None
sys.modules[SPEC.name] = release
SPEC.loader.exec_module(release)

COMMIT = "a1" * 20


def manifest_bytes(manifest_format=release.MANIFEST_V3_FORMAT):
    services = release._manifest_services(manifest_format)
    rows = [manifest_format, "commit|" + COMMIT]
    images = {}
    for index, service in enumerate(services, start=1):
        image_id = "sha256:" + f"{index:064x}"
        images[service] = image_id
        rows.append(
            f"image|{service}|lagrange-station-{service}:{COMMIT}|{image_id}|{COMMIT}"
        )
    return ("\n".join(rows) + "\n").encode("ascii"), images


def binding_fixture(manifest_format=release.MANIFEST_V3_FORMAT):
    manifest, images = manifest_bytes(manifest_format)
    manifest_info = release.validate_manifest(manifest, COMMIT)
    source_input = "1" * 64
    recipe = "2" * 64
    binary = "3" * 64
    receipt = {
        "binary_mode": "0755",
        "binary_sha256": binary,
        "bin": release.BIN,
        "cache_key": "4" * 64,
        "cargo_success": True,
        "compile_env": release._expected_compile_env(COMMIT, source_input, recipe),
        "features": [],
        "format": release.RECEIPT_FORMAT,
        "h_sha256": "5" * 64,
        "host_triple": release.TARGET,
        "input_sha256": source_input,
        "package": release.PACKAGE,
        "platform": release.PLATFORM,
        "profile": release.PROFILE,
        "recipe_sha256": recipe,
        "source_commit": COMMIT,
    }
    receipt_data = json.dumps(receipt, separators=(",", ":")).encode("utf-8")
    receipt_digest = hashlib.sha256(receipt_data).hexdigest()
    complete = receipt_digest.encode("ascii") + b"\n"
    config = {
        "schema": release.SIMULATION_SCHEMA,
        "code_commit": COMMIT,
        "manifest_format": release.MANIFEST_V3_FORMAT,
        "manifest_sha256": manifest_info["sha256"],
        "image_id": images[release.BIN],
        "receipt_sha256": "sha256:" + receipt_digest,
        "binary_sha256": "sha256:" + binary,
        "source_input_sha256": "sha256:" + source_input,
        "recipe_sha256": "sha256:" + recipe,
        "build_origin": release.BUILD_ORIGIN,
        "package": release.PACKAGE,
        "bin": release.BIN,
        "target": release.TARGET,
        "profile": release.PROFILE,
        "toolchain": release.TOOLCHAIN,
        "owner": "11111111-1111-4111-8111-111111111111",
        "entitlement_reference": "synthetic-fixture",
        "entitlement_sha256": "sha256:" + "6" * 64,
        "historical_root": "/synthetic/history",
        "reader_uid": 10001,
        "reader_gid": 10001,
        "paths": dict(release._PATHS),
        "limits": {
            "memory_bytes": 1,
            "nano_cpus": 1,
            "health_tmpfs_bytes": 1,
            "pids_limit": 1,
            "startup_timeout_seconds": 1,
            "health_max_age_seconds": 1,
            "stop_timeout_seconds": 11,
        },
    }
    # The wire contract preserves valid source formatting instead of requiring
    # canonical config JSON. The staged bytes must remain exactly these bytes.
    config_data = (json.dumps(config, ensure_ascii=False, indent=1) + "\n").encode("utf-8")
    return {
        "config": config,
        "config_data": config_data,
        "manifest": manifest,
        "receipt": receipt,
        "receipt_data": receipt_data,
        "complete": complete,
        "images": images,
    }


def write_bytes(path, data, mode):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
    os.chmod(path, mode)
    return path


def safe_read_at(daily, parent, name, uid, gid, mode, maximum, calls):
    calls.append(("read_file", name, uid, gid, mode, maximum))
    fd = os.open(name, daily.FILE_FLAGS, dir_fd=parent)
    try:
        before = os.fstat(fd)
        if (not stat.S_ISREG(before.st_mode) or before.st_nlink != 1
                or stat.S_IMODE(before.st_mode) != mode
                or name in daily.bad_owner_names):
            raise daily.DailyInstallError("DAILY_INSTALL_FILE")
        if before.st_size > maximum:
            raise daily.DailyInstallError("DAILY_INSTALL_SIZE")
        chunks = []
        remaining = maximum + 1
        while remaining:
            chunk = os.read(fd, min(65536, remaining))
            if not chunk:
                break
            chunks.append(chunk)
            remaining -= len(chunk)
        data = b"".join(chunks)
        after = os.fstat(fd)
        try:
            named = os.stat(name, dir_fd=parent, follow_symlinks=False)
        except OSError:
            raise daily.DailyInstallError("DAILY_INSTALL_CHANGED") from None
        identity = lambda item: (item.st_dev, item.st_ino, item.st_mode, item.st_nlink,
                                 item.st_size, item.st_mtime_ns, item.st_ctime_ns)
        if len(data) != before.st_size or identity(before) != identity(after) or identity(after) != identity(named):
            raise daily.DailyInstallError("DAILY_INSTALL_CHANGED")
        return data
    finally:
        os.close(fd)


class FakeDaily:
    """Temp-only secure-file adapter with UID/GID metadata mocked explicitly."""

    def __init__(self, calls):
        self.real = release._daily()
        self.calls = calls
        self.DIR_FLAGS = self.real.DIR_FLAGS
        self.FILE_FLAGS = self.real.FILE_FLAGS
        self.DailyInstallError = self.real.DailyInstallError
        self.child_directory = self.real.child_directory
        self.bad_owner_names = set()

    def require_root(self):
        self.calls.append(("require_root",))

    @contextlib.contextmanager
    def root_directory(self, path):
        self.calls.append(("root_directory", str(path)))
        descriptors = [os.open("/", self.DIR_FLAGS)]
        try:
            for component in self.real.path_parts(path):
                child = os.open(component, self.DIR_FLAGS, dir_fd=descriptors[-1])
                if not stat.S_ISDIR(os.fstat(child).st_mode):
                    os.close(child)
                    raise self.DailyInstallError("DAILY_INSTALL_PATH")
                descriptors.append(child)
            yield descriptors[-1]
        finally:
            for fd in reversed(descriptors):
                os.close(fd)

    def read_root_file(self, path, *, gid, mode, maximum):
        self.calls.append(("read_root_file", str(path), gid, mode, maximum))
        fd = os.open(path, self.FILE_FLAGS)
        try:
            info = os.fstat(fd)
            if (not stat.S_ISREG(info.st_mode) or info.st_nlink != 1
                    or stat.S_IMODE(info.st_mode) != mode):
                raise self.DailyInstallError("DAILY_INSTALL_FILE")
            if info.st_size > maximum:
                raise self.DailyInstallError("DAILY_INSTALL_SIZE")
            chunks = []
            remaining = maximum + 1
            while remaining:
                chunk = os.read(fd, min(65536, remaining))
                if not chunk:
                    break
                chunks.append(chunk)
                remaining -= len(chunk)
            data = b"".join(chunks)
            if len(data) != info.st_size:
                raise self.DailyInstallError("DAILY_INSTALL_CHANGED")
            return data
        finally:
            os.close(fd)

    def read_file(self, parent, name, *, uid, gid, mode, maximum):
        return safe_read_at(self, parent, name, uid, gid, mode, maximum, self.calls)

    def create_file(self, parent, name, data, uid, gid, mode):
        self.calls.append(("create_file", name, uid, gid, mode, data))
        return self.real.create_file(parent, name, data, uid, gid, mode)

    def directory_info(self, fd, uid=0, gid=None, mode=None):
        self.calls.append(("directory_info", uid, gid, mode))
        info = os.fstat(fd)
        if (not stat.S_ISDIR(info.st_mode) or info.st_mode & 0o022
                or (mode is not None and stat.S_IMODE(info.st_mode) != mode)):
            raise self.DailyInstallError("DAILY_INSTALL_PATH")
        return info


@contextlib.contextmanager
def patched_daily(calls):
    daily = release._daily()
    fake = FakeDaily(calls)
    with (mock.patch.object(release, "_daily", return_value=fake),
          mock.patch.object(daily, "directory_info", fake.directory_info)):
        yield daily, fake


def write_release(root, fixture, *, manifest=None, include_config=False):
    release_root = Path(root) / "releases" / COMMIT
    release_root.mkdir(parents=True)
    write_bytes(release_root / release.MANIFEST_NAME,
                fixture["manifest"] if manifest is None else manifest, 0o600)
    if include_config:
        config_dir = release_root / release.CONFIG_DIRECTORY
        config_dir.mkdir(mode=0o750)
        os.chmod(config_dir, 0o750)
        write_bytes(config_dir / release.CONFIG_NAME, fixture["config_data"], 0o440)
    return release_root


def write_capability(release_root, data=release.READER_BYTES, mode=0o644):
    evidence = Path(release_root) / "configs" / "evidence"
    evidence.mkdir(parents=True, mode=0o755, exist_ok=True)
    os.chmod(evidence.parent, 0o755)
    os.chmod(evidence, 0o755)
    return write_bytes(evidence / "owner-beta-simulation-reader-v1.json", data, mode)


class SimulationReleaseContractTests(unittest.TestCase):
    def assert_rejected(self, function):
        with self.assertRaises(release.SimulationInstallError) as raised:
            function()
        self.assertTrue(raised.exception.code.startswith("SIMULATION_INSTALL_"))
        self.assertEqual(str(raised.exception), raised.exception.code)

    def test_reader_json_file_is_exact_canonical_bytes_without_final_lf(self):
        path = ROOT / release.READER_PATH
        data = path.read_bytes()
        self.assertEqual(data, release.READER_BYTES)
        self.assertFalse(data.endswith(b"\n"))
        self.assertEqual(
            data,
            b'{"schema":"owner-beta-simulation-reader-v1","required_migration_versions":[63,64],'
            b'"input_versions":["owner-beta-price-only-simulation-job-v1",'
            b'"owner-beta-price-only-simulation-job-v2"],'
            b'"result_versions":["owner-beta-price-only-result-v1",'
            b'"owner-beta-price-only-result-v2"],"preserve_simulation_history":true}',
        )

    def test_config_accepts_noncanonical_original_bytes_and_exact_values(self):
        fixture = binding_fixture()
        parsed = release.validate_config(fixture["config_data"], COMMIT)
        self.assertEqual(parsed, fixture["config"])
        self.assertTrue(fixture["config_data"].endswith(b"\n"))

    def test_config_rejects_duplicates_unknown_missing_boolean_overflow_and_surrogate(self):
        fixture = binding_fixture()
        data = fixture["config_data"]
        duplicate = data[:-2] + b',"schema":"owner-beta-simulation-process-v1"}\n'
        self.assert_rejected(lambda: release.validate_config(duplicate, COMMIT))
        unknown_config = dict(fixture["config"], unexpected="x")
        self.assert_rejected(lambda: release.validate_config(json.dumps(unknown_config).encode(), COMMIT))
        missing_config = dict(fixture["config"])
        missing_config.pop("bin")
        self.assert_rejected(lambda: release.validate_config(json.dumps(missing_config).encode(), COMMIT))
        bool_config = dict(fixture["config"], reader_uid=True)
        self.assert_rejected(lambda: release.validate_config(json.dumps(bool_config).encode(), COMMIT))
        overflow_config = dict(fixture["config"])
        overflow_config["limits"] = dict(overflow_config["limits"], memory_bytes=2**63)
        self.assert_rejected(lambda: release.validate_config(json.dumps(overflow_config).encode(), COMMIT))
        surrogate = data.replace(b"synthetic-fixture", b"\\ud800hetic-fixture")
        self.assert_rejected(lambda: release.validate_config(surrogate, COMMIT))
        self.assert_rejected(lambda: release.validate_config(data + b"{}", COMMIT))
        self.assert_rejected(lambda: release.validate_config(data.replace(b"10001", b"NaN", 1), COMMIT))

    def test_config_checks_path_uuid_entitlement_and_integer_bounds(self):
        fixture = binding_fixture()
        for invalid_root in ("/", "/a/", "/a//b", "/a/./b", "/a/../b", "/a/\x01b"):
            invalid = dict(fixture["config"], historical_root=invalid_root)
            self.assert_rejected(lambda invalid=invalid: release.validate_config(
                json.dumps(invalid).encode(), COMMIT))
        invalid_owner = dict(fixture["config"], owner="00000000-0000-0000-0000-000000000000")
        self.assert_rejected(lambda: release.validate_config(json.dumps(invalid_owner).encode(), COMMIT))
        invalid_ref = dict(fixture["config"], entitlement_reference=" padded ")
        self.assert_rejected(lambda: release.validate_config(json.dumps(invalid_ref).encode(), COMMIT))
        for name, value in (("stop_timeout_seconds", 10), ("pids_limit", 2**31),
                            ("nano_cpus", 0), ("health_tmpfs_bytes", True)):
            invalid = dict(fixture["config"])
            invalid["limits"] = dict(invalid["limits"], **{name: value})
            self.assert_rejected(lambda invalid=invalid: release.validate_config(
                json.dumps(invalid).encode(), COMMIT))

    def test_manifest_accepts_exact_v2_and_v3_ordered_rows(self):
        for manifest_format, count in (("LAGRANGE_RELEASE_MANIFEST_V2", 12),
                                       ("LAGRANGE_RELEASE_MANIFEST_V3", 13)):
            data, images = manifest_bytes(manifest_format)
            parsed = release.validate_manifest(data, COMMIT)
            self.assertEqual(parsed["format"], manifest_format)
            self.assertEqual(parsed["commit"], COMMIT)
            self.assertEqual(len(parsed["services"]), count)
            self.assertEqual(parsed["image_ids"], images)
            self.assertEqual(parsed["sha256"], "sha256:" + hashlib.sha256(data).hexdigest())

    def test_manifest_rejects_bad_bytes_rows_order_commit_and_image_id(self):
        data, _images = manifest_bytes()
        cases = [data[:-1], data + b"\n", data.replace(b"\n", b"\r\n", 1),
                 data.replace(b"\n", b"\x00", 1), data + b"extra\n"]
        rows = data.decode().splitlines()
        rows[2], rows[3] = rows[3], rows[2]
        cases.append(("\n".join(rows) + "\n").encode())
        bad_image = data.replace(b"sha256:" + b"0" * 63 + b"1",
                                 b"sha256:" + b"A" * 64, 1)
        cases.append(bad_image)
        for invalid in cases:
            self.assert_rejected(lambda invalid=invalid: release.validate_manifest(invalid, COMMIT))
        self.assert_rejected(lambda: release.validate_manifest(data, "0" * 40))

    def test_receipt_and_complete_bind_all_four_sources(self):
        fixture = binding_fixture()
        self.assertEqual(release.validate_receipt(
            fixture["receipt_data"], fixture["complete"], fixture["config"]), fixture["receipt"])
        self.assertEqual(release.validate_binding(
            fixture["config_data"], fixture["manifest"], fixture["receipt_data"],
            fixture["complete"], COMMIT), fixture["config"])
        self.assert_rejected(lambda: release.validate_receipt(
            fixture["receipt_data"], fixture["complete"][:-1] + b"x", fixture["config"]))
        invalid_config = dict(fixture["config"], reader_uid=True)
        self.assert_rejected(lambda: release.validate_receipt(b"not json", b"", invalid_config))
        self.assert_rejected(lambda: release.validate_binding(
            fixture["config_data"], fixture["manifest"], fixture["receipt_data"], b"0" * 65, COMMIT))

    def test_receipt_rejects_extra_keys_hosted_origin_and_cross_bindings(self):
        fixture = binding_fixture()
        for mutation in (
            lambda value: value.update(extra="x"),
            lambda value: value.update(binary_sha256="f" * 64),
            lambda value: value.update(source_commit="b2" * 20),
            lambda value: value.update(input_sha256="f" * 64),
            lambda value: value.update(recipe_sha256="f" * 64),
            lambda value: value["compile_env"].update(WORKFLOW_RUN_ID="1"),
            lambda value: value.update(features=["default"]),
            lambda value: value.update(cargo_success="true"),
        ):
            receipt = json.loads(fixture["receipt_data"])
            mutation(receipt)
            raw = json.dumps(receipt, separators=(",", ":")).encode()
            digest = hashlib.sha256(raw).hexdigest().encode() + b"\n"
            config = dict(fixture["config"], receipt_sha256="sha256:" + digest[:-1].decode())
            self.assert_rejected(lambda raw=raw, digest=digest, config=config: release.validate_receipt(
                raw, digest, config))
        hosted = dict(fixture["config"], build_origin="hosted-run")
        self.assert_rejected(lambda: release.validate_config(json.dumps(hosted).encode(), COMMIT))
        config_manifest_mismatch = dict(fixture["config"], manifest_sha256="sha256:" + "f" * 64)
        raw_config = json.dumps(config_manifest_mismatch).encode()
        self.assert_rejected(lambda: release.validate_binding(
            raw_config, fixture["manifest"], fixture["receipt_data"], fixture["complete"], COMMIT))

    def test_stage_config_preserves_bytes_and_uses_exact_metadata_contract(self):
        fixture = binding_fixture()
        calls = []
        with tempfile.TemporaryDirectory() as temp:
            base = Path(temp)
            stage = base / "stage"
            stage.mkdir()
            write_bytes(stage / release.MANIFEST_NAME, fixture["manifest"], 0o600)
            source = write_bytes(base / "config.source", fixture["config_data"], 0o600)
            receipt = write_bytes(base / "artifact.json", fixture["receipt_data"], 0o444)
            complete = write_bytes(base / "COMPLETE", fixture["complete"], 0o444)
            chown_calls = []
            chmod_calls = []
            fsync_calls = []
            real_fchmod = os.fchmod
            with patched_daily(calls):
                with (mock.patch.object(os, "fchown", side_effect=lambda fd, uid, gid: chown_calls.append((uid, gid))),
                      mock.patch.object(os, "fchmod", side_effect=lambda fd, mode: (chmod_calls.append(mode), real_fchmod(fd, mode))[1]),
                      mock.patch.object(os, "fsync", side_effect=lambda fd: fsync_calls.append(
                          "file" if stat.S_ISREG(os.fstat(fd).st_mode) else "dir"))):
                    staged_config = release.stage_config(
                        str(stage), str(source), str(receipt), str(complete), COMMIT)
            self.assertEqual(staged_config, fixture["config"])
            staged = stage / release.CONFIG_DIRECTORY / release.CONFIG_NAME
            self.assertEqual(staged.read_bytes(), fixture["config_data"])
            self.assertEqual(stat.S_IMODE(staged.stat().st_mode), 0o440)
            self.assertEqual(stat.S_IMODE(staged.parent.stat().st_mode), 0o750)
            self.assertEqual(chown_calls, [(0, 10001), (0, 10001)])
            self.assertEqual(chmod_calls, [0o440, 0o750])
            self.assertEqual(fsync_calls, ["dir", "file", "dir", "dir", "dir"])
            self.assertEqual([c[2:4] for c in calls if c[0] == "read_root_file"],
                             [(0, 0o600), (0, 0o444), (0, 0o444)])
            self.assertEqual([(c[1], c[2], c[3], c[4]) for c in calls if c[0] == "create_file"],
                             [(release.CONFIG_NAME, 0, 10001, 0o440)])

    def test_stage_refuses_invalid_binding_before_creating_config_directory(self):
        fixture = binding_fixture()
        calls = []
        with tempfile.TemporaryDirectory() as temp:
            base = Path(temp)
            stage = base / "stage"
            stage.mkdir()
            write_bytes(stage / release.MANIFEST_NAME, fixture["manifest"], 0o600)
            source = write_bytes(base / "config.source", fixture["config_data"], 0o600)
            receipt = write_bytes(base / "artifact.json", fixture["receipt_data"], 0o444)
            complete = write_bytes(base / "COMPLETE", b"x" * 65, 0o444)
            with patched_daily(calls):
                self.assert_rejected(lambda: release.stage_config(
                    str(stage), str(source), str(receipt), str(complete), COMMIT))
            self.assertFalse((stage / release.CONFIG_DIRECTORY).exists())

    def test_stage_refuses_partial_existing_directory_without_repair(self):
        fixture = binding_fixture()
        calls = []
        with tempfile.TemporaryDirectory() as temp:
            base = Path(temp)
            stage = base / "stage"
            stage.mkdir()
            write_bytes(stage / release.MANIFEST_NAME, fixture["manifest"], 0o600)
            source = write_bytes(base / "config.source", fixture["config_data"], 0o600)
            receipt = write_bytes(base / "artifact.json", fixture["receipt_data"], 0o444)
            complete = write_bytes(base / "COMPLETE", fixture["complete"], 0o444)
            partial = stage / release.CONFIG_DIRECTORY
            partial.mkdir(mode=0o700)
            write_bytes(partial / "partial", b"partial", 0o600)
            with patched_daily(calls):
                self.assert_rejected(lambda: release.stage_config(
                    str(stage), str(source), str(receipt), str(complete), COMMIT))
            self.assertEqual((partial / "partial").read_bytes(), b"partial")

    def test_verify_config_checks_exact_inventory_modes_and_nofollow_files(self):
        fixture = binding_fixture()
        calls = []
        with tempfile.TemporaryDirectory() as temp:
            install = Path(temp) / "install"
            install.mkdir()
            target = write_release(install, fixture, include_config=True)
            with patched_daily(calls):
                config, digest = release.verify_config(str(target), COMMIT)
                self.assertEqual(config, fixture["config"])
                self.assertEqual(digest, "sha256:" + hashlib.sha256(fixture["config_data"]).hexdigest())
                extra = target / release.CONFIG_DIRECTORY / "extra"
                write_bytes(extra, b"x", 0o600)
                self.assert_rejected(lambda: release.verify_config(str(target), COMMIT))
                extra.unlink()
                config_path = target / release.CONFIG_DIRECTORY / release.CONFIG_NAME
                config_path.chmod(0o640)
                self.assert_rejected(lambda: release.verify_config(str(target), COMMIT))
                config_path.chmod(0o440)
                other = Path(temp) / "other"
                other.write_bytes(b"other")
                config_path.unlink()
                config_path.symlink_to(other)
                self.assert_rejected(lambda: release.verify_config(str(target), COMMIT))

    def test_verify_config_rejects_hardlink_and_symlink_directory(self):
        fixture = binding_fixture()
        calls = []
        with tempfile.TemporaryDirectory() as temp:
            install = Path(temp) / "install"
            install.mkdir()
            target = write_release(install, fixture, include_config=True)
            config_dir = target / release.CONFIG_DIRECTORY
            config_path = config_dir / release.CONFIG_NAME
            with patched_daily(calls):
                other = Path(temp) / "hardlink"
                os.link(config_path, other)
                self.assert_rejected(lambda: release.verify_config(str(target), COMMIT))
                other.unlink()
                config_dir.rename(target / "saved-config")
                config_dir.symlink_to(target / "saved-config", target_is_directory=True)
                self.assert_rejected(lambda: release.verify_config(str(target), COMMIT))

    def test_reader_guard_legacy_off_and_capability_transitions(self):
        fixture = binding_fixture()
        with tempfile.TemporaryDirectory() as temp:
            install = Path(temp) / "install"
            install.mkdir()
            target = write_release(install, fixture)
            v2, _ = manifest_bytes("LAGRANGE_RELEASE_MANIFEST_V2")
            target_manifest = target / release.MANIFEST_NAME
            target_manifest.write_bytes(v2)
            target_manifest.chmod(0o600)
            calls = []
            with patched_daily(calls):
                self.assertEqual(release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=False,
                    reserve=False), "LEGACY_OFF")
                target_manifest.write_bytes(fixture["manifest"])
                target_manifest.chmod(0o600)
                self.assertEqual(release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=False,
                    reserve=False), "LEGACY_OFF")
                self.assert_rejected(lambda: release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=True,
                    reserve=False))
                write_capability(target)
                self.assertEqual(release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=True,
                    reserve=False), "SIMULATION_READER_REQUIRED")
                self.assert_rejected(lambda: release.reader_guard(
                    str(install), str(target), COMMIT, "owner_only", history_present=False,
                    reserve=False))
                config_dir = target / release.CONFIG_DIRECTORY
                config_dir.mkdir(mode=0o750)
                os.chmod(config_dir, 0o750)
                write_bytes(config_dir / release.CONFIG_NAME, fixture["config_data"], 0o440)
                self.assertEqual(release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=False,
                    reserve=False), "SIMULATION_READER_REQUIRED")
                with mock.patch.object(release, "READER_BYTES", b"{}"):
                    self.assert_rejected(lambda: release.reader_guard(
                        str(install), str(target), COMMIT, "disabled", history_present=True,
                        reserve=False))

    def test_reader_guard_reserves_exact_floor_after_successful_checks(self):
        fixture = binding_fixture()
        calls = []
        with tempfile.TemporaryDirectory() as temp:
            install = Path(temp) / "install"
            install.mkdir()
            target = write_release(install, fixture, include_config=True)
            write_capability(target)
            chown_calls = []
            fsync_calls = []
            with patched_daily(calls):
                with (mock.patch.object(os, "fchown", side_effect=lambda fd, uid, gid: chown_calls.append((uid, gid))),
                      mock.patch.object(os, "fsync", side_effect=lambda fd: fsync_calls.append(
                          "file" if stat.S_ISREG(os.fstat(fd).st_mode) else "dir"))):
                    status = release.reader_guard(
                        str(install), str(target), COMMIT, "owner_only", history_present=False,
                        reserve=True)
            floor = install / release.FLOOR_NAME
            self.assertEqual(status, "SIMULATION_READER_REQUIRED")
            self.assertEqual(floor.read_bytes(), release.FLOOR_BYTES)
            self.assertEqual(stat.S_IMODE(floor.stat().st_mode), 0o600)
            self.assertEqual(chown_calls, [(0, 0)])
            self.assertEqual(fsync_calls, ["file", "dir"])
            self.assertEqual([(c[1], c[2], c[3], c[4], c[5])
                              for c in calls if c[0] == "create_file"],
                             [(release.FLOOR_NAME, 0, 0, 0o600, release.FLOOR_BYTES)])
            marker_reads = [index for index, call in enumerate(calls)
                            if call[:2] == ("read_file", "owner-beta-simulation-reader-v1.json")]
            floor_writes = [index for index, call in enumerate(calls)
                            if call[:2] == ("create_file", release.FLOOR_NAME)]
            self.assertEqual(len(marker_reads), 1)
            self.assertEqual(len(floor_writes), 1)
            self.assertLess(marker_reads[0], floor_writes[0])

    def test_reader_guard_reads_exact_target_capability_and_rejects_mutations(self):
        fixture = binding_fixture()
        calls = []
        with tempfile.TemporaryDirectory() as temp:
            install = Path(temp) / "install"
            install.mkdir()
            target = write_release(install, fixture)
            marker_path = target / release.READER_PATH
            with patched_daily(calls) as (_daily, fake):
                self.assert_rejected(lambda: release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=True,
                    reserve=True))
                self.assertFalse((install / release.FLOOR_NAME).exists())

                marker = write_capability(target, b"{}")
                self.assert_rejected(lambda: release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=True,
                    reserve=True))
                marker.unlink()

                marker = write_capability(target)
                fake.bad_owner_names.add(marker.name)
                self.assert_rejected(lambda: release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=True,
                    reserve=True))
                fake.bad_owner_names.clear()
                self.assertFalse((install / release.FLOOR_NAME).exists())

                marker.chmod(0o600)
                self.assert_rejected(lambda: release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=True,
                    reserve=True))
                marker.chmod(0o000)
                self.assert_rejected(lambda: release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=True,
                    reserve=True))
                marker.chmod(0o644)

                hardlink = Path(temp) / "capability-hardlink"
                os.link(marker, hardlink)
                self.assert_rejected(lambda: release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=True,
                    reserve=True))
                hardlink.unlink()

                marker.unlink()
                external = write_bytes(Path(temp) / "external-capability", release.READER_BYTES, 0o644)
                marker_path.symlink_to(external)
                self.assert_rejected(lambda: release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=True,
                    reserve=True))
                marker_path.unlink()

                write_capability(target)
                self.assertEqual(release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=True,
                    reserve=False), "SIMULATION_READER_REQUIRED")
                self.assertFalse((install / release.FLOOR_NAME).exists())
            self.assertIn(("read_file", marker_path.name, 0, 0, 0o644, len(release.READER_BYTES)), calls)

    def test_guard_does_not_reserve_floor_for_legacy_off(self):
        fixture = binding_fixture()
        v2, _ = manifest_bytes("LAGRANGE_RELEASE_MANIFEST_V2")
        calls = []
        with tempfile.TemporaryDirectory() as temp:
            install = Path(temp) / "install"
            install.mkdir()
            target = write_release(install, fixture, manifest=v2)
            with patched_daily(calls):
                self.assertEqual(release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=False,
                    reserve=True), "LEGACY_OFF")
                self.assertFalse((install / release.FLOOR_NAME).exists())
                self.assertEqual(release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=False,
                    reserve=True), "LEGACY_OFF")
                self.assertFalse((install / release.FLOOR_NAME).exists())
                self.assertFalse(any(call[0] == "create_file" for call in calls))

    def test_reader_guard_refuses_bad_floor_links_modes_and_v2_after_floor(self):
        fixture = binding_fixture()
        with tempfile.TemporaryDirectory() as temp:
            install = Path(temp) / "install"
            install.mkdir()
            target = write_release(install, fixture)
            write_capability(target)
            floor = install / release.FLOOR_NAME
            calls = []
            with patched_daily(calls):
                write_bytes(floor, b"partial", 0o600)
                self.assert_rejected(lambda: release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=False,
                    reserve=False))
                floor.unlink()
                target_manifest = target / release.MANIFEST_NAME
                v2, _ = manifest_bytes("LAGRANGE_RELEASE_MANIFEST_V2")
                target_manifest.write_bytes(v2)
                target_manifest.chmod(0o600)
                write_bytes(floor, release.FLOOR_BYTES, 0o600)
                self.assert_rejected(lambda: release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=False,
                    reserve=False))
                floor.unlink()
                target_manifest.write_bytes(fixture["manifest"])
                target_manifest.chmod(0o600)
                floor.write_bytes(release.FLOOR_BYTES)
                floor.chmod(0o644)
                self.assert_rejected(lambda: release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=False,
                    reserve=False))
                floor.chmod(0o600)
                hardlink = Path(temp) / "floor-hardlink"
                os.link(floor, hardlink)
                self.assert_rejected(lambda: release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=False,
                    reserve=False))
                hardlink.unlink()
                floor.unlink()
                floor.symlink_to(Path(temp) / "missing")
                self.assert_rejected(lambda: release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=False,
                    reserve=False))

    def test_reader_guard_detects_dangling_config_link_and_noncanonical_target(self):
        fixture = binding_fixture()
        calls = []
        with tempfile.TemporaryDirectory() as temp:
            install = Path(temp) / "install"
            install.mkdir()
            target = write_release(install, fixture)
            write_capability(target)
            (target / release.CONFIG_DIRECTORY).symlink_to(Path(temp) / "dangling",
                                                           target_is_directory=True)
            with patched_daily(calls):
                self.assert_rejected(lambda: release.reader_guard(
                    str(install), str(target), COMMIT, "disabled", history_present=False,
                    reserve=False))
                self.assert_rejected(lambda: release.reader_guard(
                    str(install), str(target) + "/../" + COMMIT, COMMIT,
                    "disabled", history_present=False, reserve=False))

    def test_cli_reader_check_never_reserves_floor(self):
        captured = []
        with mock.patch.object(release, "reader_guard", side_effect=lambda *a, **kw: captured.append(kw) or "LEGACY_OFF"), \
             mock.patch("builtins.print"):
            status = release.main([
                "reader-check", "--install-root", "/synthetic/install",
                "--target-release", "/synthetic/install/releases/" + COMMIT,
                "--commit", COMMIT, "--mode", "disabled",
            ])
        self.assertEqual(status, 0)
        self.assertEqual(captured, [{"history_present": False, "reserve": False}])

    def test_cli_usage_error_does_not_echo_untrusted_argument(self):
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            with self.assertRaises(SystemExit) as raised:
                release.main(["untrusted-private-value"])
        self.assertEqual(raised.exception.code, 2)
        self.assertEqual(stderr.getvalue(), "SIMULATION_INSTALL_USAGE\n")


if __name__ == "__main__":
    unittest.main()
