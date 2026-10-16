#!/usr/bin/env python3
"""Real private filesystem regressions; no production paths or Docker calls."""
from __future__ import annotations

import copy
import importlib.util
import io
import os
from pathlib import Path
import shutil
import struct
import sys
import tempfile
import types
import unittest
from unittest import mock

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "preupgrade_recovery_tested", ROOT / "scripts/ops/lib/production-preupgrade-recovery.py")
recovery = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = recovery
spec.loader.exec_module(recovery)
files = recovery.files
ERRORS = (files.SnapshotError, recovery.runtime.RuntimeErrorCode)


class PreupgradeRecoveryTests(unittest.TestCase):
    def setUp(self):
        if os.geteuid() != 0 or sys.platform != "linux":
            self.fail("actual root Linux fixture required")
        self.directory = Path(tempfile.mkdtemp(prefix="lagrange-preupgrade-files-", dir="/var/lib"))
        self.addCleanup(shutil.rmtree, self.directory)
        self.roots = []
        for name in files.PREUPGRADE_CLASSES:
            path = self.directory / name
            path.mkdir(mode=0o750)
            owner = 10001 if name in {"raw", "curated", "base"} else 0
            group = 10001 if owner else 0
            os.chown(path, owner, group)
            self.roots.append(files.Root(name, str(path), owner, group, 0o750))
        pgroot = self.directory / recovery.PGROOT
        pgroot.mkdir(mode=0o700, parents=True)
        os.chown(pgroot, 999, 999)
        for name in recovery.REQUIRED_DIRECTORIES:
            path = pgroot / name
            path.mkdir(mode=0o700)
            os.chown(path, 999, 999)
        self.put(recovery.VERSION, b"18\n", 999, 999, 0o600)
        self.put(recovery.PGROOT + "/global/pg_control", b"synthetic-control", 999, 999, 0o600)
        self.put(recovery.PGROOT + "/pg_wal/000000010000000000000001", b"synthetic-wal", 999, 999, 0o600)
        self.put("raw/manifest", b"synthetic-raw", 10001, 10001)
        self.put("curated/bars", b"synthetic-curated", 10001, 10001)
        self.put("base/approved", b"synthetic-base", 10001, 10001)
        self.put("config/env", b"SYNTHETIC_PROTECTED_CONFIG", 0, 0, 0o600)
        self.limits = files.Limits(1024 * 1024, entries=200, manifest_bytes=65536)

    def put(self, name, data, uid=0, gid=0, mode=0o640):
        path = self.directory / name
        path.write_bytes(data)
        path.chmod(mode)
        os.chown(path, uid, gid)
        return path

    def archive(self):
        target = io.BytesIO()
        with files.Snapshot(self.roots, self.limits, profile="preupgrade") as snapshot:
            snapshot.write(target)
        return target.getvalue()

    def inspect(self, data=None):
        manifest, retained = files.verify_archive(io.BytesIO(self.archive() if data is None else data),
            self.limits, (recovery.VERSION,), profile="preupgrade")
        recovery.cluster_manifest(manifest, retained[recovery.VERSION])
        return manifest

    @staticmethod
    def audit():
        return {"schema": "lagrange-preupgrade-db-v1", "database": "lagrange",
                "server_version_num": 180004, "system_identifier": "1234567890123456789",
                "migrations": [{"version": 1, "checksum": "b" * 96, "success": True}],
                "catalog_sha256": "sha256:" + "c" * 64}

    @staticmethod
    def container():
        return {"Id": "a" * 64, "Image": "sha256:" + "b" * 64, "Created": "created",
                "Config": {"Image": recovery.backup.POSTGRES},
                "RestartCount": 0, "State": {"Status": "exited", "Running": False,
                    "Paused": False, "Restarting": False, "Dead": False, "ExitCode": 0,
                    "OOMKilled": False, "Error": "", "Pid": 0, "StartedAt": "started",
                    "FinishedAt": "finished"},
                "Mounts": [{"Destination": "/var/lib/postgresql", "Type": "volume",
                    "Name": "fixture-volume", "Source": "/fixture-volume-data", "RW": True}]}

    def check_container(self, data):
        recovery.stopped_container(data, identity="a" * 64, image_id="sha256:" + "b" * 64,
            volume="fixture-volume", mountpoint="/fixture-volume-data",
            generation=("created", "started", "finished", 0))

    def test_cold_profile_round_trip_preserves_all_classes_wal_and_owners(self):
        data = self.archive()
        manifest = self.inspect(data)
        restored = self.directory / "restored"
        restored.mkdir(mode=0o700)
        self.assertEqual(files.restore_archive(io.BytesIO(data), self.limits, str(restored),
                         profile="preupgrade"), manifest)
        roots = []
        for name in files.PREUPGRADE_CLASSES:
            entry = next(e for e in manifest["entries"] if e["path"] == name)
            roots.append(files.Root(name, str(restored / name), entry["uid"], entry["gid"], entry["mode"]))
        with files.Snapshot(roots, self.limits, profile="preupgrade") as snapshot:
            self.assertEqual(snapshot.manifest(), files.canonical(manifest))
        wal = restored / recovery.PGROOT / "pg_wal/000000010000000000000001"
        self.assertEqual(wal.read_bytes(), b"synthetic-wal")
        self.assertEqual((wal.stat().st_uid, wal.stat().st_gid, wal.stat().st_mode & 0o777),
                         (999, 999, 0o600))

    def test_daily_reader_and_writer_reject_cold_profile_without_opt_in(self):
        with self.assertRaises(files.SnapshotError):
            files.Snapshot(self.roots, self.limits)
        with self.assertRaises(files.SnapshotError):
            files.verify_archive(io.BytesIO(self.archive()), self.limits)
        restored = self.directory / "restored"
        restored.mkdir(mode=0o700)
        with self.assertRaises(files.SnapshotError):
            files.restore_archive(io.BytesIO(self.archive()), self.limits, str(restored))
        self.assertEqual(list(restored.iterdir()), [])

    def test_official_sticky_volume_requires_exact_uid_and_private_host_parent(self):
        root = self.directory / "postgres"
        os.chown(root, 999, 999)
        root.chmod(0o1777)
        self.roots[-1] = files.Root("postgres", str(root), 999, 999, 0o1777)
        data = self.archive()
        manifest = self.inspect(data)
        restored = self.directory / "restored"
        restored.mkdir(mode=0o700)
        files.restore_archive(io.BytesIO(data), self.limits, str(restored), profile="preupgrade")
        info = (restored / "postgres").stat()
        self.assertEqual((info.st_uid, info.st_gid, info.st_mode & 0o7777), (999, 999, 0o1777))
        self.assertEqual(next(e for e in manifest["entries"] if e["path"] == "postgres")["mode"], 0o1777)
        self.directory.chmod(0o755)
        with self.assertRaises(files.SnapshotError):
            self.archive()
        self.directory.chmod(0o700)
        (root / "18").chmod(0o1777)
        with self.assertRaises(files.SnapshotError):
            self.archive()

    def test_cold_reader_rejects_daily_format_and_unknown_profile(self):
        parent = self.directory / "daily-profile"
        parent.mkdir(mode=0o700)
        roots = []
        for name in files.CLASSES:
            path = parent / name
            path.mkdir(mode=0o700)
            roots.append(files.Root(name, str(path), 0, 0, 0o700))
        stream = io.BytesIO()
        with files.Snapshot(roots, self.limits) as snapshot:
            snapshot.write(stream)
        files.verify_archive(io.BytesIO(stream.getvalue()), self.limits)
        for profile in ("preupgrade", "anything"):
            with self.subTest(profile=profile), self.assertRaises(files.SnapshotError):
                files.verify_archive(io.BytesIO(stream.getvalue()), self.limits, profile=profile)

    def test_postgres_uid_is_not_admitted_into_other_file_classes(self):
        for name in ("raw/manifest", "curated/bars", "base/approved", "config/env"):
            path = self.directory / name
            before = path.stat()
            for uid, gid in ((999, before.st_gid), (before.st_uid, 999)):
                with self.subTest(name=name, uid=uid, gid=gid):
                    os.chown(path, uid, gid)
                    with self.assertRaises(files.SnapshotError):
                        self.archive()
                    os.chown(path, before.st_uid, before.st_gid)

    def test_cold_roots_cannot_overlap_or_alias_a_class(self):
        changed = list(self.roots)
        changed[3] = files.Root("config", str(self.directory / "base/child"), 0, 0, 0o700)
        with self.assertRaises(files.SnapshotError):
            files.Snapshot(changed, self.limits, profile="preupgrade")
        changed[3] = files.Root("config", str(self.directory / "base"), 0, 0, 0o700)
        with self.assertRaises(files.SnapshotError):
            files.Snapshot(changed, self.limits, profile="preupgrade")

    def test_manifest_cannot_smuggle_postgres_uid_into_raw_on_restore(self):
        data = self.archive()
        start = len(files.PREUPGRADE_MAGIC)
        size = struct.unpack(">Q", data[start:start + 8])[0]
        value = self.inspect(data)
        next(e for e in value["entries"] if e["path"] == "raw/manifest")["uid"] = 999
        changed = files.canonical(value)
        data = files.PREUPGRADE_MAGIC + struct.pack(">Q", len(changed)) + changed + data[start + 8 + size:]
        with self.assertRaises(files.SnapshotError):
            files.verify_archive(io.BytesIO(data), self.limits, profile="preupgrade")

    def test_missing_wal_control_or_version_is_rejected(self):
        for leaf in ("pg_wal", "global/pg_control", "PG_VERSION"):
            path = self.directory / recovery.PGROOT / leaf
            moved = self.directory / "held"
            path.rename(moved)
            with self.subTest(leaf=leaf), self.assertRaises(ERRORS):
                self.inspect()
            moved.rename(path)

    def test_pid_recovery_markers_and_external_tablespaces_are_rejected(self):
        for name in recovery.FORBIDDEN_LEAVES:
            path = self.put(recovery.PGROOT + "/" + name, b"marker", 999, 999, 0o600)
            with self.subTest(name=name), self.assertRaises(ERRORS):
                self.inspect()
            path.unlink()
        path = self.put(recovery.PGROOT + "/pg_tblspc/12345", b"external", 999, 999, 0o600)
        with self.assertRaises(ERRORS):
            self.inspect()
        path.unlink()
        path.symlink_to(self.directory / "raw")
        with self.assertRaises(files.SnapshotError):
            self.archive()

    def test_unsupported_major_and_extra_volume_sibling_are_rejected(self):
        version = self.directory / recovery.VERSION
        version.write_bytes(b"17\n")
        with self.assertRaises(ERRORS):
            self.inspect()
        version.write_bytes(b"18\n")
        (self.directory / "postgres/17").mkdir(mode=0o700)
        with self.assertRaises(ERRORS):
            self.inspect()

    def test_concurrent_pgdata_change_fails_snapshot_scope(self):
        with self.assertRaises(files.SnapshotError):
            with files.Snapshot(self.roots, self.limits, profile="preupgrade"):
                (self.directory / recovery.PGROOT / "global/pg_control").write_bytes(b"changed-control")

    def test_corrupt_truncated_and_trailing_archives_fail_without_existing_overwrite(self):
        good = self.archive()
        for data in (good[:-1], good + b"extra", good[:-1] + bytes([good[-1] ^ 1])):
            with self.subTest(size=len(data)), self.assertRaises(files.SnapshotError):
                files.verify_archive(io.BytesIO(data), self.limits, profile="preupgrade")
        restored = self.directory / "restored"
        restored.mkdir(mode=0o700)
        sentinel = restored / "keep"
        sentinel.write_bytes(b"existing")
        with self.assertRaises(files.SnapshotError):
            files.restore_archive(io.BytesIO(good), self.limits, str(restored), profile="preupgrade")
        self.assertEqual(sentinel.read_bytes(), b"existing")

    def test_declared_size_entry_and_metadata_limits_remain_bounded(self):
        for limits in (files.Limits(3), files.Limits(1024 * 1024, entries=6),
                       files.Limits(1024 * 1024, manifest_bytes=1024)):
            with self.subTest(limits=limits), self.assertRaises(files.SnapshotError):
                with files.Snapshot(self.roots, limits, profile="preupgrade"):
                    self.fail("over-limit snapshot accepted")

    def test_stopped_container_requires_clean_exit_and_zero_pid(self):
        self.check_container(self.container())
        for key, value in (("Status", "running"), ("Running", True), ("Paused", True),
                           ("Restarting", True), ("Dead", True), ("ExitCode", 1),
                           ("OOMKilled", True), ("Error", "failure"), ("Pid", 1234)):
            changed = self.container()
            changed["State"][key] = value
            with self.subTest(key=key), self.assertRaises(ERRORS):
                self.check_container(changed)

    def test_stopped_container_identity_generation_and_mount_are_bound(self):
        for key, value in (("Id", "c" * 64), ("Image", "sha256:" + "d" * 64),
                           ("Created", "another"), ("RestartCount", 1)):
            changed = self.container()
            changed[key] = value
            with self.subTest(key=key), self.assertRaises(ERRORS):
                self.check_container(changed)
        for key in ("StartedAt", "FinishedAt"):
            changed = self.container()
            changed["State"][key] = "different"
            with self.subTest(key=key), self.assertRaises(ERRORS):
                self.check_container(changed)
        for key, value in (("Name", "foreign"), ("Type", "bind"), ("Source", "/foreign"), ("RW", False)):
            changed = self.container()
            changed["Mounts"][0][key] = value
            with self.subTest(key=key), self.assertRaises(ERRORS):
                self.check_container(changed)
        changed = self.container()
        changed["Mounts"].append({"Destination": "/var/lib/postgresql/18/docker/pg_wal"})
        with self.assertRaises(ERRORS):
            self.check_container(changed)
        changed = self.container()
        changed["Config"]["Image"] = "postgres:latest"
        with self.assertRaises(ERRORS):
            self.check_container(changed)

    def test_audit_checks_database_cluster_version_checksum_and_success(self):
        original = self.audit()
        def check(value):
            return recovery.checked_audit(value, database=original["database"],
                system_identifier=original["system_identifier"], migrations=original["migrations"])
        self.assertEqual(check(original), original)
        for key, value in (("database", "other"), ("system_identifier", "987654321"),
                           ("server_version_num", 170006), ("server_version_num", True),
                           ("schema", "unknown"), ("catalog_sha256", "bad"), ("extra", 1)):
            changed = copy.deepcopy(original)
            changed[key] = value
            with self.subTest(key=key), self.assertRaises(ERRORS):
                check(changed)
        changed = copy.deepcopy(original)
        changed["migrations"][0]["checksum"] = "d" * 96
        with self.assertRaises(ERRORS):
            check(changed)
        for version, success in ((60, True), (True, True), (1, False)):
            changed = copy.deepcopy(original)
            changed["migrations"][0].update(version=version, success=success)
            with self.subTest(version=version, success=success), self.assertRaises(ERRORS):
                recovery.checked_audit(changed, database=original["database"],
                    system_identifier=original["system_identifier"], migrations=changed["migrations"])

    def test_volume_must_be_local_exact_and_without_external_driver_options(self):
        good = {"Name": "fixture-volume", "Driver": "local", "Scope": "local",
                "Options": None, "Mountpoint": "/fixture/volume"}
        docker = mock.Mock()
        docker.call.return_value = files.canonical([good]).decode()
        self.assertEqual(recovery.volume_info(docker, "fixture-volume"), good)
        docker.call.assert_called_once_with(["volume", "inspect", "fixture-volume"])
        for key, value in (("Name", "other"), ("Driver", "remote"), ("Scope", "global"),
                           ("Options", {"device": "/other"}), ("Mountpoint", "/bad,option")):
            changed = dict(good)
            changed[key] = value
            docker.call.return_value = files.canonical([changed]).decode()
            with self.subTest(key=key), self.assertRaises(ERRORS):
                recovery.volume_info(docker, "fixture-volume")

    def test_verifier_rejects_live_path_before_any_docker_request(self):
        docker = mock.Mock()
        storage = recovery.ColdStorage({"BACKUP_ROOT": str(self.directory), "MAX_TOTAL_BYTES": 1024 * 1024},
            docker, types.SimpleNamespace(release="/fixture"), types.SimpleNamespace(database="lagrange"))
        with self.assertRaises(ERRORS):
            storage.verify_cluster(self.directory / "postgres", {})
        self.assertEqual(docker.mock_calls, [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
