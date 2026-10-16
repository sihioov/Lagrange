#!/usr/bin/env python3
"""Disposable real-root file/archive fixtures; never opens a production path."""
from __future__ import annotations

import copy
import errno
import hashlib
import importlib.util
import io
import os
from pathlib import Path
import shutil
import struct
import sys
import tempfile
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

    def test_host_paths_and_unselected_floor_sibling_are_absent(self):
        self.put("floor/not-selected-secret", b"NOT_FOR_BACKUP")
        archive = self.archive()
        self.assertNotIn(str(self.directory).encode(), archive)
        self.assertNotIn(b"NOT_FOR_BACKUP", archive)
        files.verify_archive(io.BytesIO(archive), self.limits)

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


if __name__ == "__main__":
    unittest.main(verbosity=2)
