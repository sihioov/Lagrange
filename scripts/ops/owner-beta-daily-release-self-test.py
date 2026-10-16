#!/usr/bin/env python3
"""Isolated real-UID installer tests; requires root, never production paths."""
from __future__ import annotations

import copy
import errno
import fcntl
import hashlib
import importlib.util
import json
import os
import select
import shutil
import signal
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "daily_release", ROOT / "scripts/ops/lib/owner-beta-daily-release.py")
assert SPEC and SPEC.loader
daily = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(daily)
FIXTURES = ROOT / "tests/fixtures/owner-beta-daily-install"
COMMIT = "a" * 40


class DailyReleaseTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if sys.platform != "linux" or os.geteuid() != 0:
            raise RuntimeError("DAILY_INSTALL_TEST_ENVIRONMENT_ERROR")
        cls.anchor = Path(tempfile.mkdtemp(prefix="lagrange-daily-install-", dir="/var/lib"))
        os.chmod(cls.anchor, 0o755)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.anchor)

    def setUp(self):
        self.root = Path(tempfile.mkdtemp(dir=self.anchor))
        os.chmod(self.root, 0o755)
        self.release = self.root / "release"
        self.release.mkdir(mode=0o755)
        self.data_root = self.root / "data"
        self.data_root.mkdir(mode=0o750)
        os.chown(self.data_root, 0, daily.READER_GROUP)
        self.config = json.loads((FIXTURES / "config.json").read_bytes())
        self.source = self.root / "source.json"
        self.source.write_bytes(daily.canonical(self.config))
        self.source.chmod(0o600)
        self.policy = self.root / "policy.json"
        self.policy.write_bytes((ROOT / "configs/evidence/owner-beta-daily-price-policy-v1.json").read_bytes())
        self.policy.chmod(0o644)
        self.install_root = self.root / "install"
        self.install_root.mkdir(mode=0o755)

    def tearDown(self):
        shutil.rmtree(self.root)

    def stage(self):
        return daily.stage_config(str(self.release), str(self.source), str(self.policy), COMMIT)

    def provision(self):
        daily.provision_state(str(self.data_root), self.config)
        return self.data_root / "owner-beta-daily"

    def reader(self, target=None):
        target = target or self.release
        path = target / "configs/evidence"
        path.mkdir(parents=True, mode=0o755)
        (path / "owner-beta-daily-reader-v1.json").write_bytes(daily.READER_BYTES)

    def child_errno(self, uid, operation):
        read_fd, write_fd = os.pipe()
        pid = os.fork()
        if pid == 0:
            os.close(read_fd)
            try:
                os.setgroups([daily.READER_GROUP])
                os.setgid(uid)
                os.setuid(uid)
                operation()
                result = 0
            except OSError as error:
                result = error.errno or 255
            except BaseException:
                result = 255
            try:
                os.write(write_fd, str(result).encode("ascii"))
            finally:
                os.close(write_fd)
                os._exit(0)
        os.close(write_fd)
        try:
            ready, _, _ = select.select([read_fd], [], [], 5)
            if not ready:
                os.kill(pid, signal.SIGKILL)
                self.fail("real UID child deadline exceeded")
            result = os.read(read_fd, 32)
        finally:
            os.close(read_fd)
            _, status = os.waitpid(pid, 0)
        self.assertEqual(status, 0)
        return int(result)

    def test_stage_preserves_exact_input_and_rejects_overwrite(self):
        before = self.source.read_bytes()
        self.stage()
        config, digest = daily.verify_config(str(self.release), COMMIT)
        self.assertEqual(config, self.config)
        self.assertEqual(digest, "sha256:" + hashlib.sha256(before).hexdigest())
        self.assertEqual(self.source.read_bytes(), before)
        target = self.release / daily.CONFIG_DIRECTORY / "config.json"
        self.assertEqual(target.read_bytes(), before)
        self.assertEqual((target.stat().st_uid, target.stat().st_gid, stat.S_IMODE(target.stat().st_mode)),
                         (0, daily.READER_GROUP, 0o440))
        with self.assertRaises(FileExistsError):
            self.stage()
        self.assertEqual(target.read_bytes(), before)

    def test_config_closed_schema_and_explicit_budget_fail_before_write(self):
        mutations = [
            lambda x: x.update(owner="00000000-0000-0000-0000-000000000000"),
            lambda x: x.update(unknown=True),
            lambda x: x.update(code_commit="b" * 40),
            lambda x: x.update(policy_sha256="sha256:" + "0" * 64),
            lambda x: x.update(raw_root="/data/owner-beta-daily/admitted"),
            lambda x: x.update(raw_owner_uid=True),
            lambda x: x.update(entitlement_reference="x" * 513),
            lambda x: x.update(entitlement_reference="has\ncontrol"),
            lambda x: x.update(entitlement_reference="has\u0085control"),
            lambda x: x.update(first_auto_session="2026-08-28"),
            lambda x: x.update(max_cycle_seconds=3601),
            lambda x: x.update(max_replay_admissions=0),
            lambda x: x.update(max_artifact_entries=20001),
            lambda x: x["scan"].update(max_source_file_bytes=67108865),
            lambda x: x["scan"].update(max_metadata_json_depth=33),
            lambda x: x["scan"].update(max_files=False),
            lambda x: x["scan"].update(max_open_descriptors=2**64),
            lambda x: x["scan"].update(extra=1),
        ]
        for index, mutate in enumerate(mutations):
            with self.subTest(case=index):
                value = copy.deepcopy(self.config)
                mutate(value)
                self.source.write_bytes(daily.canonical(value))
                with self.assertRaises(daily.DailyInstallError):
                    self.stage()
                self.assertFalse((self.release / daily.CONFIG_DIRECTORY).exists())

    def test_duplicate_json_and_policy_bytes_are_rejected_before_write(self):
        valid = self.source.read_bytes()
        self.source.write_bytes(valid[:-1] + b',"owner":"00000000-0000-4000-8000-000000000001"}')
        with self.assertRaises(daily.DailyInstallError):
            self.stage()
        self.source.write_bytes(valid)
        self.policy.write_bytes(self.policy.read_bytes() + b"\n")
        with self.assertRaises(daily.DailyInstallError):
            self.stage()
        self.assertFalse((self.release / daily.CONFIG_DIRECTORY).exists())

    def test_unsafe_external_file_link_mode_owner_and_fifo_are_rejected(self):
        content = self.source.read_bytes()
        for kind in ("symlink", "hardlink", "mode", "owner", "fifo"):
            with self.subTest(case=kind):
                self.source.unlink()
                if kind == "symlink":
                    self.source.symlink_to(self.policy)
                elif kind == "hardlink":
                    os.link(self.policy, self.source)
                    os.chmod(self.source, 0o600)
                elif kind == "fifo":
                    os.mkfifo(self.source, 0o600)
                else:
                    self.source.write_bytes(content)
                    self.source.chmod(0o640 if kind == "mode" else 0o600)
                    if kind == "owner":
                        os.chown(self.source, 10002, 10002)
                with self.assertRaises((daily.DailyInstallError, OSError)):
                    self.stage()
                self.assertFalse((self.release / daily.CONFIG_DIRECTORY).exists())

    def test_real_principals_have_only_their_write_roots(self):
        root = self.provision()
        def write(path):
            fd = os.open(path / "probe", os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            os.close(fd)
            os.unlink(path / "probe")
        for uid, allowed in ((10002, {"candidates", "producer-state"}),
                             (10003, {"admitted", "frontiers", "admitter-state"}),
                             (10001, set())):
            for name in (*daily.SHARED, *daily.STAGES):
                with self.subTest(uid=uid, root=name):
                    result = self.child_errno(uid, lambda p=root / name: write(p))
                    self.assertEqual(result, 0 if name in allowed else errno.EACCES)
        for uid in (10001, 10002, 10003):
            self.assertEqual(self.child_errno(uid, lambda: os.rename(root, self.data_root / "moved")),
                             errno.EACCES)
        for name, owner in daily.SHARED.items():
            file = root / name / "readable"
            file.write_bytes(b"fixture")
            os.chown(file, owner, daily.READER_GROUP)
            file.chmod(0o440)
            for uid in (10001, 10002, 10003):
                self.assertEqual(self.child_errno(uid, lambda p=file: p.read_bytes()), 0)

    def test_initial_checkpoints_match_shared_rust_fixtures(self):
        root = self.provision()
        for name, (stage, uid) in daily.STAGES.items():
            path = root / name
            self.assertEqual((path.stat().st_uid, path.stat().st_gid, stat.S_IMODE(path.stat().st_mode)),
                             (uid, uid, 0o700))
            for leaf in ("checkpoint.json", "poll.lock"):
                info = (path / leaf).stat()
                self.assertEqual((info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode), info.st_nlink),
                                 (uid, uid, 0o600, 1))
            self.assertEqual((path / "checkpoint.json").read_bytes(),
                             (FIXTURES / (stage.lower() + "-checkpoint.json")).read_bytes())
        daily.check_state(str(self.data_root), self.config)

    def test_existing_advanced_state_is_reused_without_reset(self):
        root = self.provision()
        file = root / "producer-state/checkpoint.json"
        value = json.loads(file.read_bytes())
        value.update(generation=7, last_started_unix_millis=123456, pending={
            "kind": "CANDIDATE", "content_sha256": "sha256:" + "1" * 64,
            "manifest_sha256": "sha256:" + "2" * 64, "parent_admission_sha256": None,
            "session": "2026-08-31", "producer_commit": COMMIT,
        })
        before = daily.canonical(value)
        file.write_bytes(before)
        self.assertEqual(daily.provision_state(str(self.data_root), self.config), "REUSED")
        self.assertEqual(file.read_bytes(), before)
        foreign = copy.deepcopy(self.config)
        foreign["owner"] = "00000000-0000-4000-8000-000000000002"
        with self.assertRaises(daily.DailyInstallError):
            daily.provision_state(str(self.data_root), foreign)
        self.assertEqual(file.read_bytes(), before)

    def test_partial_existing_state_is_not_repaired(self):
        root = self.data_root / "owner-beta-daily"
        root.mkdir(mode=0o750)
        os.chown(root, 0, daily.READER_GROUP)
        with self.assertRaises(daily.DailyInstallError):
            daily.provision_state(str(self.data_root), self.config)
        self.assertEqual(list(root.iterdir()), [])

    def test_busy_state_lock_is_bounded_and_preserves_checkpoint(self):
        root = self.provision()
        file = root / "producer-state/checkpoint.json"
        before = file.read_bytes()
        with (root / "producer-state/poll.lock").open("rb") as lock:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaisesRegex(daily.DailyInstallError, "^DAILY_INSTALL_BUSY$"):
                daily.check_state(str(self.data_root), self.config)
        self.assertEqual(file.read_bytes(), before)
        daily.check_state(str(self.data_root), self.config)

    def test_pending_crash_residue_is_left_for_runtime_recovery(self):
        root = self.provision()
        extra = root / "producer-state/.checkpoint.tmp"
        extra.write_bytes(b"unresolved")
        os.chown(extra, 10002, 10002)
        extra.chmod(0o600)
        with self.assertRaises(daily.DailyInstallError):
            daily.provision_state(str(self.data_root), self.config)
        self.assertEqual(extra.read_bytes(), b"unresolved")

    def test_no_replace_publication_preserves_existing_destination(self):
        with daily.root_directory(str(self.data_root)) as fd:
            os.mkdir("staged", dir_fd=fd)
            os.mkdir("existing", dir_fd=fd)
            before = os.stat("existing", dir_fd=fd)
            with self.assertRaises(FileExistsError):
                daily.rename_no_replace(fd, "staged", fd, "existing")
            after = os.stat("existing", dir_fd=fd)
            self.assertEqual((before.st_dev, before.st_ino), (after.st_dev, after.st_ino))
            self.assertTrue((self.data_root / "staged").is_dir())

    def test_uncertain_publication_does_not_delete_staging_or_create_final(self):
        with patch.object(daily, "rename_no_replace", side_effect=OSError(errno.EIO, "fixture")):
            with self.assertRaises(OSError):
                daily.provision_state(str(self.data_root), self.config)
        self.assertFalse((self.data_root / "owner-beta-daily").exists())
        stages = list(self.data_root.iterdir())
        self.assertEqual(len(stages), 1)
        self.assertEqual(stat.S_IMODE(stages[0].stat().st_mode), 0o700)
        for uid in (10001, 10002, 10003):
            self.assertEqual(self.child_errno(uid, lambda: list(stages[0].iterdir())), errno.EACCES)

    def test_reader_floor_blocks_old_reader_but_keeps_compatible_off(self):
        self.assertEqual(daily.reader_guard(str(self.install_root), str(self.release), "disabled"), "LEGACY_OFF")
        self.assertFalse((self.install_root / daily.FLOOR_NAME).exists())
        self.reader()
        daily.reader_guard(str(self.install_root), str(self.release), "owner_only")
        floor = self.install_root / daily.FLOOR_NAME
        before = floor.read_bytes()
        old = self.root / "old"
        old.mkdir(mode=0o755)
        with self.assertRaises(FileNotFoundError):
            daily.reader_guard(str(self.install_root), str(old), "disabled")
        self.assertEqual(daily.reader_guard(str(self.install_root), str(self.release), "disabled"), "DAILY_READER_REQUIRED")
        self.assertEqual(floor.read_bytes(), before)

    def test_corrupt_floor_never_becomes_absent_or_is_reset(self):
        floor = self.install_root / daily.FLOOR_NAME
        floor.write_bytes(b"partial")
        floor.chmod(0o600)
        self.reader()
        for mode in ("disabled", "owner_only"):
            with self.assertRaises(daily.DailyInstallError):
                daily.reader_guard(str(self.install_root), str(self.release), mode)
            self.assertEqual(floor.read_bytes(), b"partial")

    def test_root_ancestor_uid_mode_and_acl_are_authoritative(self):
        def acl():
            entries = [(1, 7, 0xffffffff), (2, 5, 10002), (4, 5, 0xffffffff),
                       (16, 5, 0xffffffff), (32, 0, 0xffffffff)]
            data = struct.pack("<I", 2) + b"".join(struct.pack("<HHI", *row) for row in entries)
            os.setxattr(self.data_root, "system.posix_acl_access", data)
        for mode in ("owner", "writable", "acl"):
            with self.subTest(case=mode):
                os.chown(self.data_root, 0, daily.READER_GROUP)
                self.data_root.chmod(0o750)
                if mode == "owner":
                    os.chown(self.data_root, 10002, daily.READER_GROUP)
                elif mode == "writable":
                    self.data_root.chmod(0o770)
                else:
                    acl()
                    self.assertEqual(stat.S_IMODE(self.data_root.stat().st_mode), 0o750)
                with self.assertRaises(daily.DailyInstallError):
                    daily.provision_state(str(self.data_root), self.config)
                self.assertFalse((self.data_root / "owner-beta-daily").exists())
                if mode == "acl":
                    os.removexattr(self.data_root, "system.posix_acl_access")

    def test_cli_failure_does_not_emit_configuration_or_paths(self):
        sentinel = "PRIVATE_DAILY_FIXTURE_SENTINEL"
        self.source.write_text('{"private":"' + sentinel + '"}')
        result = subprocess.run([
            sys.executable, str(ROOT / "scripts/ops/lib/owner-beta-daily-release.py"),
            "stage", "--stage", str(self.release), "--source", str(self.source),
            "--policy-source", str(self.policy), "--commit", COMMIT,
        ], capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, b"")
        self.assertEqual(result.stderr, b"DAILY_INSTALL_INVALID\n")
        self.assertNotIn(sentinel.encode(), result.stderr)
        self.assertNotIn(str(self.root).encode(), result.stderr)
        for arguments in ([sentinel], ["verify", "--unknown", sentinel], []):
            rejected = subprocess.run([
                sys.executable, str(ROOT / "scripts/ops/lib/owner-beta-daily-release.py"),
                *arguments,
            ], capture_output=True, timeout=5)
            self.assertEqual(rejected.returncode, 1)
            self.assertEqual(rejected.stdout, b"")
            self.assertEqual(rejected.stderr, b"DAILY_INSTALL_ARGUMENTS\n")
            self.assertNotIn(sentinel.encode(), rejected.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
