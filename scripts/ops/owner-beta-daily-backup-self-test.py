#!/usr/bin/env python3
"""Disposable real-root file/archive fixtures; never opens a production path."""
from __future__ import annotations

import copy
import contextlib
import errno
import hashlib
import importlib.util
import io
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "daily_backup_files_tested", ROOT / "scripts/ops/lib/owner-beta-daily-backup-files.py")
files = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = files
spec.loader.exec_module(files)


class DailyBackupFilesTests(unittest.TestCase):
    def setUp(self):
        if os.geteuid() != 0 or sys.platform != "linux":
            self.fail("actual root Linux fixture required")
        self.directory = Path(tempfile.mkdtemp(prefix="lagrange-backup-files-", dir="/var/lib"))
        self.addCleanup(shutil.rmtree, self.directory)
        self.roots = []
        for name in files.CLASSES:
            path = self.directory / name
            path.mkdir(mode=0o750)
            owner = 10001 if name in {"raw", "curated", "base"} else 0
            os.chown(path, owner, 10001)
            self.roots.append(files.Root(name, str(path), owner, 10001, 0o750,
                                         "reader-floor" if name == "floor" else None))
        self.put("raw/source/manifest.jsonl", b'{"fixture":1}\n')
        self.put("raw/source/empty", b"")
        self.put("curated/data", b"curated\0fixture")
        self.put("daily/admitted/hash/manifest.json", b'{"fixture":"admitted"}', 10003)
        self.put("daily/producer-state/poll.lock", b"", 10002)
        self.put("config/config.json", b'{"fixture":"config"}')
        self.put("config/policy.json", b'{"fixture":"policy"}')
        self.put("floor/reader-floor", b"reader-v1")
        self.put("base/base.data", b"approved-base-fixture")
        self.limits = files.Limits(4 * 1024 * 1024, entries=200, manifest_bytes=65536)

    def put(self, name, content, uid=0):
        path = self.directory / name
        path.parent.mkdir(mode=0o750, parents=True, exist_ok=True)
        path.write_bytes(content)
        os.chmod(path, 0o640)
        os.chown(path, uid, 10001)
        return path

    def archive(self):
        stream = io.BytesIO()
        with files.Snapshot(self.roots, self.limits) as snapshot:
            snapshot.write(stream)
        return stream.getvalue()

    def changed_manifest(self, mutate, *, body=True):
        original = self.archive()
        offset = len(files.MAGIC)
        length = struct.unpack(">Q", original[offset:offset + 8])[0]
        value = files.validate_manifest(original[offset + 8:offset + 8 + length], self.limits)
        mutate(value)
        data = files.canonical(value)
        return files.MAGIC + struct.pack(">Q", len(data)) + data + (
            original[offset + 8 + length:] if body else b"")

    def database_fixture(self):
        # Byte-reference fixtures only, not actual admitted domain documents.
        def hash_bytes(data):
            return "sha256:" + hashlib.sha256(data).hexdigest()
        shutil.rmtree(self.directory / "daily/admitted/hash")
        admission, manifest = b'{"fixture":"admission"}', b'{"fixture":"manifest"}'
        bars, normalization = b'{"bar":1}\n', b'{"normalization":1}'
        observation = b"owner-beta-daily-source-observation-v1\n{}"
        admission_key, frontier_key = hash_bytes(admission), hash_bytes(observation)
        bodies = {
            "config/policy.json": b'{"fixture":"policy"}',
            "daily/admitted/" + admission_key[7:] + "/admission.json": admission,
            "daily/admitted/" + admission_key[7:] + "/manifest.json": manifest,
            "daily/admitted/" + admission_key[7:] + "/bars.ndjson": bars,
            "daily/admitted/" + admission_key[7:] + "/normalization.json": normalization,
            "daily/frontiers/" + frontier_key[7:] + "/observation.json": observation,
            "daily/frontiers/" + frontier_key[7:] + "/frontier.json": b'{"fixture":"record"}',
        }
        for name, body in bodies.items():
            path = self.put(name, body, 0 if name.startswith("config/") else 10003)
            path.chmod(0o440)
            if name.startswith("daily/"):
                os.chown(path.parent, 10003, 10001)
        database = {
            "schema": "lagrange-daily-backup-db-v1",
            "control": {"owner_user_id": "00000000-0000-4000-8000-000000000001",
                        "policy_sha256": hash_bytes(bodies["config/policy.json"]),
                        "activation_epoch": 1, "first_auto_session": "2026-08-29",
                        "trigger_version": "owner-beta-daily-trigger-v1", "enabled": False},
            "tables": [{"name": name, "row_count": 0, "sha256": hash_bytes(b"")}
                       for name in sorted(files.AUDIT_TABLES)],
            "run_count": 0,
            "admissions": [{"admission_sha256": admission_key,
                            "content_sha256": hash_bytes(b"content"),
                            "manifest_sha256": hash_bytes(manifest),
                            "parent_admission_sha256": None, "range_end": "2026-08-29",
                            "source_frontier_sha256": frontier_key,
                            "bars": {"file": "bars.ndjson", "sha256": hash_bytes(bars),
                                     "size_bytes": len(bars), "row_count": 1},
                            "normalization": {"file": "normalization.json",
                                              "sha256": hash_bytes(normalization),
                                              "size_bytes": len(normalization), "row_count": 1}}],
            "verified_frontiers": [frontier_key],
        }
        archive = self.archive()
        inventory, _ = files.verify_archive(io.BytesIO(archive), self.limits)
        return inventory, database

    def test_all_classes_bytes_modes_and_empty_files_round_trip(self):
        archive = self.archive()
        names = ("raw/source/manifest.jsonl", "raw/source/empty", "base/base.data")
        manifest, retained = files.verify_archive(io.BytesIO(archive), self.limits, names)
        self.assertEqual(manifest["classes"], list(files.CLASSES))
        self.assertEqual(retained, {name: (self.directory / name).read_bytes() for name in names})
        admission = next(e for e in manifest["entries"] if e["path"].endswith("hash/manifest.json"))
        self.assertEqual((admission["uid"], admission["gid"], admission["mode"]),
                         (10003, 10001, 0o640))

    def test_provisioned_service_owned_artifact_parent_is_bound_without_chown(self):
        parent = self.directory / "service-artifacts"
        parent.mkdir(mode=0o750)
        os.chown(parent, 14567, 10001)
        (self.directory / "base").rename(parent / "historical-price-beta-root")
        self.roots[-1] = files.Root("base", str(parent / "historical-price-beta-root"),
                                   10001, 10001, 0o750, parent=(14567, 10001, 0o750))
        with files.Snapshot(self.roots, self.limits) as snapshot:
            output = io.BytesIO()
            snapshot.write(output)
        self.assertEqual(parent.stat().st_uid, 14567)
        with self.assertRaises(files.SnapshotError):
            with files.Snapshot(self.roots, self.limits):
                os.chown(parent, 14568, 10001)

    def test_host_paths_and_unselected_floor_sibling_are_absent(self):
        self.put("floor/not-selected-secret", b"NOT_FOR_BACKUP")
        archive = self.archive()
        self.assertNotIn(str(self.directory).encode(), archive)
        self.assertNotIn(b"NOT_FOR_BACKUP", archive)
        files.verify_archive(io.BytesIO(archive), self.limits)

    def test_isolated_restore_recreates_all_bytes_and_metadata_before_reinventory(self):
        restored = self.directory / "restored"
        restored.mkdir(mode=0o700)
        manifest = files.restore_archive(io.BytesIO(self.archive()), self.limits, str(restored))
        roots = [files.Root(name, str(restored / name),
                            next(e["uid"] for e in manifest["entries"] if e["path"] == name),
                            10001, 0o750) for name in files.CLASSES]
        with files.Snapshot(roots, self.limits) as snapshot:
            self.assertEqual(files.canonical(manifest), snapshot.manifest())
        self.assertEqual((restored / "raw/source/manifest.jsonl").read_bytes(), b'{"fixture":1}\n')
        self.assertEqual((restored / "raw/source/empty").stat().st_size, 0)
        self.assertFalse((restored / "floor/not-selected-secret").exists())

    def test_restore_rejects_existing_destination_and_corruption_without_overwrite(self):
        restored = self.directory / "restored"
        restored.mkdir(mode=0o700)
        sentinel = restored / "keep"
        sentinel.write_bytes(b"existing")
        with self.assertRaises(files.SnapshotError):
            files.restore_archive(io.BytesIO(self.archive()), self.limits, str(restored))
        self.assertEqual(sentinel.read_bytes(), b"existing")
        sentinel.unlink()
        corrupt = bytearray(self.archive())
        corrupt[-1] ^= 1
        with self.assertRaises(files.SnapshotError):
            files.restore_archive(io.BytesIO(corrupt), self.limits, str(restored))

    def test_leaf_symlink_is_rejected_without_reading_target(self):
        outside = self.put("floor/not-selected-secret", b"SENTINEL_SECRET")
        (self.directory / "raw/link").symlink_to(outside)
        with self.assertRaises(files.SnapshotError):
            self.archive()

    def test_directory_symlink_is_rejected(self):
        (self.directory / "raw/link").symlink_to(self.directory / "config", target_is_directory=True)
        with self.assertRaises(files.SnapshotError):
            self.archive()

    def test_duplicate_hard_link_is_rejected(self):
        os.link(self.directory / "config/config.json", self.directory / "raw/link")
        with self.assertRaises(files.SnapshotError):
            self.archive()

    def test_fifo_is_rejected_before_open(self):
        os.mkfifo(self.directory / "raw/fifo", 0o600)
        with self.assertRaises(files.SnapshotError):
            self.archive()

    def test_group_writable_object_is_rejected(self):
        os.chmod(self.directory / "raw/source/manifest.jsonl", 0o660)
        with self.assertRaises(files.SnapshotError):
            self.archive()

    def test_foreign_owner_is_rejected(self):
        os.chown(self.directory / "raw/source/manifest.jsonl", 10004, 10001)
        with self.assertRaises(files.SnapshotError):
            self.archive()

    def test_protected_ancestor_symlink_is_rejected(self):
        path = self.directory / "alias"
        path.symlink_to(self.directory / "raw", target_is_directory=True)
        self.roots[0] = files.Root("raw", str(path), 10001, 10001, 0o750)
        with self.assertRaises(OSError):
            self.archive()

    def test_same_size_rewrite_between_inventory_and_copy_is_rejected(self):
        with self.assertRaises(files.SnapshotError):
            with files.Snapshot(self.roots, self.limits) as snapshot:
                path = self.directory / "base/base.data"
                path.write_bytes(b"X" * path.stat().st_size)
                snapshot.write(io.BytesIO())

    def test_name_replacement_during_open_read_is_rejected(self):
        original = files.read_chunks
        target = self.directory / "raw/source/manifest.jsonl"
        expected = target.stat().st_ino
        injected = []
        def replace(fd, *args, **kwargs):
            if os.fstat(fd).st_ino == expected and not injected:
                replacement = self.put("raw/source/replacement", target.read_bytes())
                replacement.replace(target)
                injected.append(True)
            return original(fd, *args, **kwargs)
        with mock.patch.object(files, "read_chunks", side_effect=replace):
            with self.assertRaises(files.SnapshotError):
                self.archive()
        self.assertEqual(injected, [True])

    def test_final_file_addition_after_copy_is_rejected(self):
        with self.assertRaises(files.SnapshotError):
            with files.Snapshot(self.roots, self.limits) as snapshot:
                snapshot.write(io.BytesIO())
                self.put("raw/new-publication", b"new")

    def test_root_name_replacement_is_rejected_at_scope_exit(self):
        with self.assertRaises(files.SnapshotError):
            with files.Snapshot(self.roots, self.limits) as snapshot:
                snapshot.write(io.BytesIO())
                raw = self.directory / "raw"
                raw.rename(self.directory / "saved-raw")
                raw.mkdir(mode=0o750)
                os.chown(raw, 10001, 10001)

    def test_snapshot_budget_is_enforced_before_unbounded_data(self):
        self.limits = files.Limits(1, entries=200, manifest_bytes=65536)
        with self.assertRaises(files.SnapshotError):
            self.archive()

    def test_entry_budget_includes_directories(self):
        self.limits = files.Limits(4096, entries=6, manifest_bytes=65536)
        with self.assertRaises(files.SnapshotError):
            self.archive()

    def test_archive_body_corruption_is_rejected(self):
        archive = bytearray(self.archive())
        archive[-1] ^= 1
        with self.assertRaises(files.SnapshotError):
            files.verify_archive(io.BytesIO(archive), self.limits)

    def test_archive_truncation_and_trailing_bytes_are_rejected(self):
        archive = self.archive()
        for invalid in (archive[:3], archive[:-1], archive + b"extra"):
            with self.subTest(length=len(invalid)):
                with self.assertRaises(files.SnapshotError):
                    files.verify_archive(io.BytesIO(invalid), self.limits)

    def test_oversized_header_rejected_before_reading_body(self):
        header = files.MAGIC + struct.pack(">Q", self.limits.manifest_bytes + 1)
        with self.assertRaises(files.SnapshotError):
            files.verify_archive(io.BytesIO(header), self.limits)

    def test_duplicate_path_and_parent_escape_are_rejected(self):
        def duplicate(value):
            value["entries"].insert(2, copy.deepcopy(value["entries"][1]))
        def escape(value):
            value["entries"][1]["path"] = "raw/../outside"
        for mutate in (duplicate, escape):
            with self.subTest(mutation=mutate.__name__):
                with self.assertRaises(files.SnapshotError):
                    files.verify_archive(io.BytesIO(self.changed_manifest(mutate)), self.limits)

    def test_missing_class_and_noncanonical_header_are_rejected(self):
        def remove_base(value):
            value["entries"] = [e for e in value["entries"]
                                if e["path"] != "base" and not e["path"].startswith("base/")]
        archive = self.changed_manifest(remove_base, body=False)
        with self.assertRaises(files.SnapshotError):
            files.verify_archive(io.BytesIO(archive), self.limits)
        value = files.MAGIC + struct.pack(">Q", 3) + b"{ }"
        with self.assertRaises(files.SnapshotError):
            files.verify_archive(io.BytesIO(value), self.limits)

    def test_requested_missing_evidence_cannot_pass(self):
        with self.assertRaises(files.SnapshotError):
            files.verify_archive(io.BytesIO(self.archive()), self.limits,
                                 ("daily/admitted/missing/manifest.json",))

    def test_actual_named_user_acl_is_rejected(self):
        # Linux POSIX ACL version 2; owner, named user, owning group, mask, other.
        acl = struct.pack("<I", 2) + b"".join(struct.pack("<HHI", tag, permission, uid)
            for tag, permission, uid in ((1, 6, 0xFFFFFFFF), (2, 4, 10004),
                                         (4, 4, 0xFFFFFFFF), (16, 4, 0xFFFFFFFF),
                                         (32, 0, 0xFFFFFFFF)))
        os.setxattr(self.directory / "config/config.json", "system.posix_acl_access", acl)
        with self.assertRaises(files.SnapshotError):
            self.archive()

    def test_metadata_budget_includes_long_member_names(self):
        for index in range(8):
            self.put("raw/" + str(index) + "x" * 200, b"")
        self.limits = files.Limits(4096, entries=200, manifest_bytes=1024)
        with self.assertRaises(files.SnapshotError):
            self.archive()

    def test_short_writes_preserve_complete_archive(self):
        class Short(io.BytesIO):
            def write(self, value):
                return super().write(value[:7])
        stream = Short()
        with files.Snapshot(self.roots, self.limits) as snapshot:
            snapshot.write(stream)
        files.verify_archive(io.BytesIO(stream.getvalue()), self.limits)

    def test_zero_write_never_claims_a_completed_capture(self):
        with files.Snapshot(self.roots, self.limits) as snapshot:
            with self.assertRaises(files.SnapshotError):
                snapshot.write(mock.Mock(write=mock.Mock(return_value=0)))

    def test_unknown_acl_capability_is_rejected(self):
        with mock.patch.object(files.os, "getxattr", side_effect=OSError(errno.ENOTSUP, "fixture")):
            with self.assertRaises(files.SnapshotError):
                self.archive()

    def test_database_references_match_all_files_and_observation_identity(self):
        inventory, database = self.database_fixture()
        self.assertEqual(files.database_references(inventory, database),
                         {"extra_admissions": [], "extra_frontiers": []})
        key = database["verified_frontiers"][0]
        record = next(e for e in inventory["entries"] if
                      e["path"] == "daily/frontiers/" + key[7:] + "/frontier.json")
        self.assertNotEqual(record["sha256"], key)

    def test_database_missing_referenced_member_is_blocked(self):
        inventory, database = self.database_fixture()
        for name in ("admission.json", "manifest.json", "bars.ndjson", "normalization.json",
                     "observation.json", "frontier.json"):
            with self.subTest(member=name):
                changed = copy.deepcopy(inventory)
                changed["entries"] = [e for e in changed["entries"]
                                      if not e["path"].endswith("/" + name)]
                with self.assertRaises(files.SnapshotError):
                    files.database_references(changed, database)

    def test_database_digest_size_mode_and_descriptor_escape_are_blocked(self):
        inventory, database = self.database_fixture()
        for field, replacement in (("sha256", "sha256:" + "f" * 64), ("size_bytes", 999),
                                    ("file", "../../outside")):
            with self.subTest(field=field):
                changed = copy.deepcopy(database)
                changed["admissions"][0]["bars"][field] = replacement
                with self.assertRaises(files.SnapshotError):
                    files.database_references(inventory, changed)
        changed = copy.deepcopy(inventory)
        member = next(e for e in changed["entries"] if e["path"].endswith("/admission.json"))
        member["mode"] = 0o640
        with self.assertRaises(files.SnapshotError):
            files.database_references(changed, database)

    def test_database_unknown_leaf_requires_reverification_instead_of_admission(self):
        inventory, database = self.database_fixture()
        original = database["admissions"][0]["admission_sha256"][7:]
        other_bytes = b'{"fixture":"orphaned admission"}'
        other = hashlib.sha256(other_bytes).hexdigest()
        old = self.directory / "daily/admitted" / original
        new = self.directory / "daily/admitted" / other
        shutil.copytree(old, new)
        os.chown(new, 10003, 10001)
        for child in new.iterdir():
            os.chown(child, 10003, 10001)
        (new / "admission.json").write_bytes(other_bytes)
        archive = self.archive()
        inventory, _ = files.verify_archive(io.BytesIO(archive), self.limits)
        self.assertEqual(files.database_references(inventory, database),
                         {"extra_admissions": ["sha256:" + other], "extra_frontiers": []})

    def test_database_duplicate_date_and_parent_cycles_are_blocked(self):
        inventory, database = self.database_fixture()
        changed = copy.deepcopy(database)
        second = copy.deepcopy(changed["admissions"][0])
        second["admission_sha256"] = "sha256:" + "f" * 64
        changed["admissions"].append(second)
        with self.assertRaises(files.SnapshotError):
            files.database_references(inventory, changed)
        for parent in ("sha256:" + "e" * 64, database["admissions"][0]["admission_sha256"]):
            with self.subTest(parent=parent):
                changed = copy.deepcopy(database)
                changed["admissions"][0]["parent_admission_sha256"] = parent
                with self.assertRaises(files.SnapshotError):
                    files.database_references(inventory, changed)

    def test_database_policy_and_enabled_state_must_match_protected_snapshot(self):
        inventory, database = self.database_fixture()
        for field, replacement in (("policy_sha256", "sha256:" + "f" * 64),
                                    ("policy_sha256", None), ("enabled", True),
                                    ("owner_user_id", None), ("activation_epoch", True),
                                    ("activation_epoch", -1), ("trigger_version", "other"),
                                    ("first_auto_session", "2026-02-30"),
                                    ("first_auto_session", "2026-08-28")):
            with self.subTest(field=field):
                changed = copy.deepcopy(database)
                changed["control"][field] = replacement
                with self.assertRaises(files.SnapshotError):
                    files.database_references(inventory, changed)

    def test_database_omitted_table_fingerprint_is_not_full_audit(self):
        inventory, database = self.database_fixture()
        database["tables"].pop()
        with self.assertRaises(files.SnapshotError):
            files.database_references(inventory, database)

    def test_database_frontier_digest_checks_observation_bytes(self):
        inventory, database = self.database_fixture()
        changed = copy.deepcopy(inventory)
        observation = next(e for e in changed["entries"] if e["path"].endswith("/observation.json"))
        observation["sha256"] = "sha256:" + "f" * 64
        with self.assertRaises(files.SnapshotError):
            files.database_references(changed, database)


class BackupResumeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Reuse only the closed inspection fixture; its test suite is not run.
        spec = importlib.util.spec_from_file_location(
            "daily_backup_runtime_fixture", ROOT / "scripts/ops/owner-beta-daily-runtime-self-test.py")
        cls.fixture = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.fixture)
        cls.runtime = cls.fixture.runtime

    def setUp(self):
        self.context = self.fixture.binding()
        fixture = self.fixture

        class Docker(fixture.FakeDocker):
            def call(self, args, timeout=15):
                result = super().call(args, timeout)
                if args[0] == "update":
                    value = next(v for v in self.values.values() if v["Id"] == args[-1])
                    value["HostConfig"]["RestartPolicy"]["Name"] = args[1].split("=", 1)[1]
                if args[0] == "start":
                    value = next(v for v in self.values.values() if v["Id"] == args[-1])
                    value["State"].update(Running=True, Status="running",
                                          StartedAt="2026-10-16T00:02:00Z")
                return result

        self.docker = Docker(self.context)
        self.before = self.runtime.verify_running(self.docker, self.context)
        self.runtime.drain(self.docker, self.context, self.before)
        self.docker.calls.clear()

    def resume(self):
        return self.runtime.resume_after_backup(self.docker, self.context, self.before)

    def test_verified_original_cohort_is_restarted_by_exact_id_only(self):
        result = self.resume()
        self.assertEqual(set(result), set(self.before))
        for service, entry in result.items():
            self.assertEqual(entry["generation"][:2], self.before[service]["generation"][:2])
            self.assertNotEqual(entry["generation"][2], self.before[service]["generation"][2])
        expected = [["update", "--restart=unless-stopped", self.before[s]["generation"][0]]
                    for s in self.runtime.SERVICES]
        expected += [["start", self.before[s]["generation"][0]] for s in self.runtime.SERVICES]
        self.assertEqual(self.docker.calls, expected)

    def test_partial_original_cohort_is_rejected_without_starting_anything(self):
        self.before.pop(self.runtime.SERVICES[1])
        with self.assertRaises(self.runtime.RuntimeErrorCode):
            self.resume()
        self.assertEqual(self.docker.calls, [])

    def test_prior_unhealthy_cohort_does_not_gain_resume_permission(self):
        self.before[self.runtime.SERVICES[2]]["value"]["State"]["Health"]["Status"] = "unhealthy"
        with self.assertRaises(self.runtime.RuntimeErrorCode):
            self.resume()
        self.assertEqual(self.docker.calls, [])

    def test_replaced_stopped_container_is_rejected_before_any_mutation(self):
        self.docker.values[self.runtime.SERVICES[1]]["Id"] = "f" * 64
        with self.assertRaises(self.runtime.RuntimeErrorCode):
            self.resume()
        self.assertEqual(self.docker.calls, [])

    def test_stopped_mount_or_uid_change_is_rejected_before_any_mutation(self):
        for field in ("mount", "uid"):
            with self.subTest(field=field):
                value = self.docker.values[self.runtime.SERVICES[2]]
                original = copy.deepcopy(value)
                if field == "mount":
                    value["Mounts"][0]["RW"] = True
                else:
                    value["Config"]["User"] = "0:0"
                with self.assertRaises(self.runtime.RuntimeErrorCode):
                    self.resume()
                self.assertEqual(self.docker.calls, [])
                self.docker.values[self.runtime.SERVICES[2]] = original

    def test_failed_drain_cannot_be_promoted_to_backup_resume(self):
        self.docker.values[self.runtime.SERVICES[0]]["State"]["ExitCode"] = 1
        with self.assertRaises(self.runtime.RuntimeErrorCode):
            self.resume()
        self.assertEqual(self.docker.calls, [])

    def test_restart_or_oom_during_resume_is_not_success(self):
        def mutate(args):
            if args[0] == "start":
                self.docker.values[self.runtime.SERVICES[1]]["RestartCount"] += 1
        self.docker.on_call = mutate
        with self.assertRaises(self.runtime.RuntimeErrorCode):
            self.resume()
        self.assertTrue(all(c[0] in {"update", "start"} for c in self.docker.calls))

    def test_health_deadline_is_bounded_and_never_accepts_starting_only(self):
        for value in self.docker.values.values():
            value["State"]["Health"]["Status"] = "starting"
        clock = [0.0]
        def advance(seconds):
            clock[0] += seconds
        with mock.patch.object(self.runtime.time, "monotonic", side_effect=lambda: clock[0]), \
                mock.patch.object(self.runtime.time, "sleep", side_effect=advance):
            with self.assertRaises(self.runtime.RuntimeErrorCode):
                self.runtime.resume_after_backup(self.docker, self.context, self.before, timeout=1)
        self.assertLessEqual(clock[0], 1)


class BackupControllerOrderingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location(
            "daily_backup_controller_tested", ROOT / "scripts/ops/lib/owner-beta-daily-backup.py")
        cls.backup = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = cls.backup
        spec.loader.exec_module(cls.backup)

    def setUp(self):
        self.events, self.failure = [], None
        self.pending = {"extra_admissions": [], "extra_frontiers": []}
        self.database = {"admissions": []}
        self.operation = mock.Mock()
        self.operation.target.images = {"api-server": "sha256:" + "a" * 64}
        self.storage = mock.Mock()
        self.controller = self.backup.Controller(self.operation, mock.Mock(), {}, self.storage, "/unused")
        self.controller.quiesce = mock.Mock(side_effect=lambda: self.event("quiesce", ({}, True, "api")))
        self.controller.fail_off = mock.Mock(side_effect=lambda: self.event("off"))
        self.controller.audit = mock.Mock(side_effect=lambda: self.event("audit", self.database))
        self.operation.database.restore_reservations.side_effect = lambda *a: self.event("repair")
        self.operation.database.control.side_effect = lambda *a: self.event("db:" + a[-1])
        self.storage.publish.side_effect = lambda *a: self.event("publish")
        self.storage.prune.side_effect = lambda: self.event("prune", (1, 10))
        self.storage.metrics.side_effect = lambda *a: self.event("metrics")

        @contextlib.contextmanager
        def fence(*args):
            self.event("fence")
            yield
            self.event("fence-close")

        @contextlib.contextmanager
        def inventory():
            self.event("snapshot")
            yield types.SimpleNamespace(manifest=lambda: b"{}")
            self.event("snapshot-final")

        @contextlib.contextmanager
        def capture(*args):
            self.event("isolated-verify")
            try:
                yield ("private", "final", "name")
            finally:
                self.event("staging-close")

        self.controller.inventory = inventory
        self.storage.capture = capture
        stack = contextlib.ExitStack()
        self.addCleanup(stack.close)
        stack.enter_context(mock.patch.object(self.backup, "publication_fence", side_effect=fence))
        stack.enter_context(mock.patch.object(self.backup.files, "database_references",
                                             side_effect=lambda *a: copy.deepcopy(self.pending)))
        stack.enter_context(mock.patch.object(self.backup.ops, "verify_api", return_value="api"))
        stack.enter_context(mock.patch.object(self.backup.ops, "snapshot_equal", return_value=True))
        stack.enter_context(mock.patch.object(self.backup.runtime, "resume_after_backup",
                                             side_effect=lambda *a: self.event("resume", {})))
        stack.enter_context(mock.patch.object(self.backup.runtime, "verify_running", return_value={}))
        self.native = stack.enter_context(mock.patch.object(self.backup, "reverify_once"))

    def event(self, name, result=None):
        self.events.append(name)
        if name == self.failure:
            raise RuntimeError("INJECTED_BACKUP_FAILURE")
        return copy.deepcopy(result)

    def test_verified_snapshot_publishes_only_after_final_source_checks_then_enables_last(self):
        self.assertEqual(self.controller.backup(), "VERIFIED_AND_RESUMED")
        self.assertEqual(self.events, ["quiesce", "fence", "snapshot", "audit", "isolated-verify",
            "audit", "snapshot-final", "fence-close", "publish", "prune", "staging-close",
            "resume", "db:enable", "metrics"])

    def test_initially_disabled_backup_does_not_start_any_process(self):
        self.controller.quiesce.return_value = ({}, False, None)
        self.controller.quiesce.side_effect = None
        self.assertEqual(self.controller.backup(), "VERIFIED_DISABLED")
        self.assertNotIn("resume", self.events)
        self.assertNotIn("db:enable", self.events)

    def test_orphan_snapshot_can_be_backed_up_but_cannot_resume(self):
        for key in self.pending:
            with self.subTest(key=key):
                self.pending = {"extra_admissions": [], "extra_frontiers": []}
                self.pending[key] = ["sha256:" + "a" * 64]
                self.events.clear()
                self.assertEqual(self.controller.backup(), "VERIFIED_DISABLED")
                self.assertIn("publish", self.events)
                self.assertNotIn("resume", self.events)

    def test_capture_and_source_final_failures_never_publish_or_resume(self):
        for stage in ("quiesce", "audit", "isolated-verify", "snapshot-final", "fence-close"):
            with self.subTest(stage=stage):
                self.failure, self.events = stage, []
                with self.assertRaises(RuntimeError):
                    self.controller.backup()
                self.assertEqual(self.events[-1], "off")
                self.assertNotIn("publish", self.events)
                self.assertNotIn("resume", self.events)

    def test_database_change_between_capture_and_final_audit_keeps_off(self):
        self.controller.audit.side_effect = [{"admissions": []}, {"admissions": [], "changed": True}]
        with self.assertRaises(self.backup.runtime.RuntimeErrorCode):
            self.controller.backup()
        self.assertEqual(self.events[-1], "off")
        self.assertNotIn("publish", self.events)

    def test_retention_resume_enable_or_metrics_failure_never_leaves_an_accepted_on_state(self):
        for stage in ("prune", "resume", "db:enable", "metrics"):
            with self.subTest(stage=stage):
                self.failure, self.events = stage, []
                with self.assertRaises(RuntimeError):
                    self.controller.backup()
                self.assertEqual(self.events[-1], "off")

    def test_original_reservation_repair_and_two_snapshots_do_not_enable(self):
        self.assertEqual(self.controller.reconcile(), "RECONCILED_DISABLED")
        self.assertLess(self.events.index("repair"), self.events.index("audit"))
        self.assertEqual(self.events.count("snapshot-final"), 2)
        self.native.assert_not_called()
        self.assertNotIn("resume", self.events)
        self.assertNotIn("db:enable", self.events)

    def test_native_success_without_actual_reconciliation_remains_pending(self):
        self.pending["extra_admissions"] = ["sha256:" + "a" * 64]
        with self.assertRaises(self.backup.runtime.RuntimeErrorCode):
            self.controller.reconcile()
        self.native.assert_called_once()
        self.assertEqual(self.events[-1], "off")

    def test_original_orphan_identity_must_match_the_native_post_state(self):
        key = "sha256:" + "a" * 64
        self.pending["extra_admissions"] = [key]
        def native(*args):
            self.pending["extra_admissions"] = []
            self.database = {"admissions": [{"admission_sha256": key}]}
        self.native.side_effect = native
        self.assertEqual(self.controller.reconcile(), "RECONCILED_DISABLED")
        self.assertNotIn("db:enable", self.events)

    def test_native_cannot_substitute_a_new_identity(self):
        self.pending["extra_admissions"] = ["sha256:" + "a" * 64]
        def native(*args):
            self.pending["extra_admissions"] = []
            self.database = {"admissions": [{"admission_sha256": "sha256:" + "b" * 64}]}
        self.native.side_effect = native
        with self.assertRaises(self.backup.runtime.RuntimeErrorCode):
            self.controller.reconcile()
        self.assertEqual(self.events[-1], "off")

    def test_frontier_only_residue_cannot_trigger_a_new_candidate_admission(self):
        self.pending["extra_frontiers"] = ["sha256:" + "a" * 64]
        with self.assertRaises(self.backup.runtime.RuntimeErrorCode):
            self.controller.reconcile()
        self.native.assert_not_called()
        self.assertEqual(self.events[-1], "off")


class DailyBackupRoutingTests(unittest.TestCase):
    def setUp(self):
        if os.geteuid() != 0 or sys.platform != "linux":
            self.fail("actual root Linux fixture required")
        self.directory = Path(tempfile.mkdtemp(prefix="lagrange-backup-routing-", dir="/var/lib"))
        self.addCleanup(shutil.rmtree, self.directory)
        self.commit = "a" * 40
        self.install = self.directory / "install"
        self.release = self.install / "releases" / self.commit
        self.lib = self.release / "scripts/ops/lib"
        self.lib.mkdir(parents=True)
        shutil.copyfile(ROOT / "scripts/ops/compose-release.sh", self.lib.parent / "compose-release.sh")
        (self.lib.parent / "compose-release.sh").chmod(0o755)
        for name in ("dotenv.sh", "kis-read-compose.sh"):
            shutil.copyfile(ROOT / "scripts/ops/lib" / name, self.lib / name)
        (self.lib / "owner-beta-daily-backup.py").write_text(
            "import sys\nprint('FIXTURE_BACKUP_ARGS:' + '|'.join(sys.argv[1:]))\n")
        (self.install / "current").symlink_to("releases/" + self.commit)
        self.data = self.directory / "data"
        (self.data / "owner-beta-daily").mkdir(parents=True)
        self.config = self.directory / "backup.conf"
        values = {"BACKUP_ROOT": str(self.directory / "backups"), "DATA_ROOT": str(self.data),
                  "COMPOSE_FILE": str(self.release / "deploy/compose/compose.yml"),
                  "COMPOSE_ENV_FILE": str(self.release / "deploy/compose/.env"),
                  "COMPOSE_PROJECT": "fixture", "LAGRANGE_CODE_COMMIT": self.commit,
                  "KEY_FILE": str(self.directory / "key"), "LOCK_FILE": str(self.directory / "lock"),
                  "METRICS_FILE": str(self.directory / "metrics"), "MAX_TOTAL_BYTES": "1000000",
                  "MIN_FREE_BYTES": "1000", "RETENTION_DAYS": "7", "MIN_KEEP": "1",
                  "POSTGRES_SERVICE": "postgres", "POSTGRES_IMAGE": "fixture-controller-validates"}
        self.config.write_text("".join(key + "=" + value + "\n" for key, value in values.items()))
        self.config.chmod(0o600)
        self.environment = {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LANG": "C.UTF-8",
                            "PYTHONDONTWRITEBYTECODE": "1", "LAGRANGE_RELEASE_ROOT": str(self.install)}

    def invoke(self, path, *arguments):
        return subprocess.run(["/bin/bash", "--noprofile", "--norc", str(path), *arguments],
            env=self.environment, capture_output=True, timeout=10, check=False)

    def test_existing_timer_modes_route_to_current_release_without_legacy_fallback(self):
        for flag, operation, mode in (("--check", "backup", "preflight"),
                                      ("--run", "backup", "apply"),
                                      ("--verify-latest", "verify", "apply")):
            with self.subTest(flag=flag):
                result = self.invoke(ROOT / "scripts/ops/run-production-backup.sh", flag,
                                     "--config-file", str(self.config))
                self.assertEqual(result.returncode, 0)
                self.assertEqual(result.stdout.decode().strip(), "FIXTURE_BACKUP_ARGS:" +
                    "|".join(("--operation", operation, "--mode", mode, "--install-root", str(self.install),
                              "--config-file", str(self.config))))
                self.assertEqual(result.stderr, b"")

    def test_stale_config_or_missing_daily_helper_cannot_enter_legacy_backup(self):
        for failure in ("stale", "helper"):
            with self.subTest(failure=failure):
                if failure == "stale":
                    original = self.config.read_bytes()
                    self.config.write_bytes(original.replace(self.commit.encode(), b"b" * 40))
                else:
                    (self.lib / "owner-beta-daily-backup.py").unlink()
                result = self.invoke(ROOT / "scripts/ops/run-production-backup.sh", "--run",
                                     "--config-file", str(self.config))
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn(b"PRODUCTION_BACKUP_CHECK: PASS", result.stdout)
                if failure == "stale":
                    self.config.write_bytes(original)

    def test_reader_floor_prevents_legacy_fallback_even_without_daily_directory(self):
        (self.data / "owner-beta-daily").rmdir()
        (self.install / ".owner-beta-daily-reader-floor").touch(mode=0o600)
        result = self.invoke(ROOT / "scripts/ops/run-production-backup.sh", "--check",
                             "--config-file", str(self.config))
        self.assertEqual(result.returncode, 0)
        self.assertIn(b"FIXTURE_BACKUP_ARGS", result.stdout)

    def test_daily_backup_rejects_other_scope_from_commit_and_mixed_operation(self):
        wrapper = self.lib.parent / "compose-release.sh"
        for arguments in (("--scope", "backfill", "--daily-backup"),
                          ("--daily-backup", "--daily-from-commit", self.commit),
                          ("--daily-backup", "--daily-enable"),
                          ("--backup-config", str(self.config)),
                          ("--daily-reconcile", "--refresh-intraday")):
            with self.subTest(arguments=arguments):
                result = self.invoke(wrapper, *arguments)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn(b"FIXTURE_BACKUP_ARGS", result.stdout)


class DailyBackupRetentionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Import the controller without invoking any production entrypoint.
        BackupControllerOrderingTests.setUpClass()
        cls.backup = BackupControllerOrderingTests.backup

    def setUp(self):
        if os.geteuid() != 0 or sys.platform != "linux":
            self.fail("actual root Linux fixture required")
        self.directory = Path(tempfile.mkdtemp(prefix="lagrange-backup-retention-", dir="/var/lib"))
        self.addCleanup(shutil.rmtree, self.directory)
        self.values = {"BACKUP_ROOT": str(self.directory), "MAX_TOTAL_BYTES": 1048576,
                       "MIN_FREE_BYTES": 1024, "MIN_KEEP": 1, "RETENTION_DAYS": 1}
        self.storage = self.backup.Storage(self.values, mock.Mock(),
                                          types.SimpleNamespace(commit="a" * 40), mock.Mock())

    def make_set(self, day, *, daily):
        name = "backup-" + ("daily-" if daily else "") + f"200001{day:02d}T000000Z-" + "a" * 40
        path = self.directory / name
        path.mkdir(mode=0o700)
        names = {*self.backup.MEMBERS, "metadata.json", "COMPLETE"} if daily else {
            "postgres.dump.enc", "raw.tar.enc", "curated.tar.enc", "manifest.sha256",
            "metadata", "VERIFIED", "COMPLETE"}
        for name in names:
            (path / name).write_bytes(b"fixture-only")
            (path / name).chmod(0o600)
        (path / "COMPLETE").write_bytes((self.backup.FORMAT if daily else
            "LAGRANGE_PRODUCTION_BACKUP_V1").encode() + b"\n")
        if not daily:
            (path / "VERIFIED").write_bytes(b"2000-01-01T00:00:00Z\n")
        return path

    def test_retention_keeps_newest_verified_set_across_both_formats(self):
        old = self.make_set(1, daily=False)
        new = self.make_set(2, daily=True)
        (self.directory / "latest").symlink_to(new.name)
        count, total = self.storage.prune()
        self.assertEqual(count, 1)
        self.assertGreater(total, 0)
        self.assertFalse(old.exists())
        self.assertTrue(new.exists())
        self.assertEqual((self.directory / "latest").resolve(), new)

    def test_minimum_keep_over_disk_cap_fails_without_deleting_retained_set(self):
        path = self.make_set(1, daily=True)
        self.values["MAX_TOTAL_BYTES"] = 1
        with self.assertRaises(self.backup.runtime.RuntimeErrorCode):
            self.storage.prune()
        self.assertTrue(path.exists())

    def test_unexpected_or_linked_members_are_never_pruned_recursively(self):
        old = self.make_set(1, daily=False)
        newest = self.make_set(2, daily=True)
        external = self.directory / "external-sentinel"
        external.write_bytes(b"keep")
        (old / "unexpected").symlink_to(external)
        with self.assertRaises(self.backup.runtime.RuntimeErrorCode):
            self.storage.prune()
        self.assertTrue(old.exists() and newest.exists())
        self.assertEqual(external.read_bytes(), b"keep")

    def test_unverified_legacy_set_blocks_deletion_and_is_preserved(self):
        old = self.make_set(1, daily=False)
        self.make_set(2, daily=True)
        (old / "VERIFIED").unlink()
        with self.assertRaises(self.backup.runtime.RuntimeErrorCode):
            self.storage.prune()
        self.assertTrue(old.exists())


class ReconcileSecretTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location(
            "daily_reconcile_secret_tested", ROOT / "scripts/ops/lib/owner-beta-daily-backup.py")
        cls.backup = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = cls.backup
        spec.loader.exec_module(cls.backup)

    def setUp(self):
        if os.geteuid() != 0 or sys.platform != "linux":
            self.fail("actual root Linux fixture required")
        self.root = Path(tempfile.mkdtemp(prefix="lagrange-daily-secret-", dir="/var/lib"))
        self.addCleanup(shutil.rmtree, self.root)
        service = self.root / "owner-beta-daily-admitter"
        service.mkdir(mode=0o750)
        os.chown(service, 10003, 10003)
        self.secret = service / "db_daily_input_admitter_password"
        self.secret.write_bytes(b"synthetic-reconcile-secret")
        os.chown(self.secret, 10003, 10003)
        self.secret.chmod(0o440)
        self.binding = types.SimpleNamespace(secret_root=str(self.root), project="synthetic-reconcile",
            image_id="sha256:" + "a" * 64, commit="b" * 40,
            mounts=mock.Mock(side_effect=RuntimeError("TEST_NATIVE_BOUNDARY")))
        self.docker = mock.Mock()
        self.docker.object.return_value = {"Config": {"Env": []}}
        self.docker.call.return_value = ""

    def preflight(self):
        # Only Docker discovery is replaced. The production directory/file
        # reader and its real UID/GID/mode/link/ACL checks are unmodified.
        with mock.patch.object(self.backup.runtime, "image_owned"), \
                mock.patch.object(self.backup.ops, "internal_network", return_value={"Id": "c" * 64}):
            self.backup.reverify_once(self.docker, self.binding, {})

    def test_canonical_0440_serving_secret_passes_unmocked_preflight(self):
        with self.assertRaisesRegex(RuntimeError, "^TEST_NATIVE_BOUNDARY$"):
            self.preflight()
        self.binding.mounts.assert_called_once()
        self.assertTrue(all(call.args[0][0] == "ps" for call in self.docker.call.call_args_list))

    def test_secret_owner_mode_and_links_remain_rejected(self):
        for uid, gid, mode in ((10003, 10003, 0o400), (10003, 10003, 0o640),
                               (0, 10003, 0o440), (10003, 0, 0o440)):
            with self.subTest(uid=uid, gid=gid, mode=mode):
                os.chown(self.secret, uid, gid)
                self.secret.chmod(mode)
                with self.assertRaises(self.backup.install.DailyInstallError):
                    self.preflight()
                self.binding.mounts.assert_not_called()
        os.chown(self.secret, 10003, 10003)
        self.secret.chmod(0o440)
        other = self.root / "other"
        os.link(self.secret, other)
        with self.assertRaises(self.backup.install.DailyInstallError):
            self.preflight()
        self.secret.unlink()
        self.secret.symlink_to(other)
        with self.assertRaises(OSError):
            self.preflight()
        self.binding.mounts.assert_not_called()


if __name__ == "__main__":
    unittest.main(verbosity=2)
