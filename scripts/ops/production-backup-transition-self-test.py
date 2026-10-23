#!/usr/bin/env python3
"""Real-root filesystem transition fixtures; systemd commands are substituted.

Every destination is under a fresh /var/lib directory. No installed backup,
service, Docker daemon, database, provider or production config is accessed.
"""
from __future__ import annotations

import copy
import datetime
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import select
import shutil
import signal
import stat
import struct
import sys
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
ROOT = Path(__file__).absolute().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "backup_transition_tested", ROOT / "scripts/ops/transition-production-backup.py")
assert SPEC and SPEC.loader
transition = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(transition)
COMMIT = "a" * 40


class FakeNative:
    def __init__(self):
        self.calls = []
        self.active = False
        self.changed_current = False
        self.fail_check = False
        self.fail_reload = False

    def current(self):
        self.calls.append("current")
        transition.require(not self.changed_current)

    def inactive(self, allow_reload=False):
        self.calls.append(("inactive", allow_reload))
        transition.require(not self.active)

    def reload(self):
        self.calls.append("daemon-reload")
        if self.fail_reload:
            self.fail_reload = False
            raise transition.Refused()

    def check(self, installed):
        self.calls.append(("check", installed))
        transition.require(not (installed and self.fail_check))


class BackupTransitionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if sys.platform != "linux" or os.geteuid() != 0:
            raise RuntimeError("BACKUP_TRANSITION_ACTUAL_ROOT_REQUIRED")
        cls.anchor = Path(tempfile.mkdtemp(prefix="lagrange-backup-transition-", dir="/var/lib"))
        cls.anchor.chmod(0o700)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.anchor)

    def setUp(self):
        self.root = Path(tempfile.mkdtemp(dir=self.anchor))
        self.root.chmod(0o700)
        self.release = self.root / "opt/lagrange/releases" / COMMIT
        self.source_config = self.root / "new-config"
        self.layout = transition.Layout(self.release, self.source_config, self.root)
        self.old, self.new = {}, {}
        for name in transition.NAMES:
            self.old[name] = ("old fixture " + name + "\n").encode()
            self.new[name] = ("new fixture " + name + "\n").encode()
            for path, data in ((self.layout.targets[name], self.old[name]),
                               (self.layout.sources[name], self.new[name])):
                path.parent.mkdir(parents=True, exist_ok=True, mode=0o755)
                path.write_bytes(data)
                path.chmod(self.layout.mode(name))
        config = {
            "LAGRANGE_CODE_COMMIT": COMMIT,
            "COMPOSE_FILE": str(self.release / "deploy/compose/compose.yml"),
            "COMPOSE_ENV_FILE": str(self.release / "deploy/compose/.env"),
            "BACKUP_ROOT": str(self.root / "srv/backups/lagrange"),
            "DATA_ROOT": str(self.root / "var/lib/lagrange/data"),
            "KEY_FILE": str(self.root / "etc/lagrange/secrets/backup_encryption_key"),
            "LOCK_FILE": str(self.layout.locks[1]),
            "METRICS_FILE": str(self.root / "var/lib/lagrange/backup-state/production-backup.prom"),
        }
        self.new["config"] = ("\n".join(k + "=" + v for k, v in config.items()) + "\n").encode()
        self.source_config.write_bytes(self.new["config"])
        for path in self.layout.locks:
            path.parent.mkdir(parents=True, exist_ok=True, mode=0o755)
            path.write_bytes(b"")
            path.chmod(0o600)
        (self.layout.install / "current").symlink_to("releases/" + COMMIT)
        now = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0)
        self.value = {"schema": transition.SCHEMA, "new_commit": COMMIT,
            "config_source": str(self.source_config),
            "issued_at": now.strftime("%Y-%m-%dT%H:%M:%SZ"),
            "expires_at": (now + datetime.timedelta(minutes=20)).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "old": {n: transition.digest(b) for n, b in self.old.items()},
            "new": {n: transition.digest(b) for n, b in self.new.items()},
            "unit_file_states": {n: "enabled" for n in transition.UNITS}}
        self.native = FakeNative()
        self.recovery = self.root / "recovery"
        self.operation = self.operation_at(self.recovery)

    def tearDown(self):
        shutil.rmtree(self.root)

    def operation_at(self, path):
        return transition.Transition(self.layout, self.value, path, self.native)

    def assert_bytes(self, expected):
        for name, path in self.layout.targets.items():
            self.assertEqual(path.read_bytes(), expected[name])
            info = path.stat()
            self.assertEqual((info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode), info.st_nlink),
                             (0, 0, self.layout.mode(name), 1))

    def journal(self, path=None):
        return json.loads(((path or self.recovery) / "journal.json").read_bytes())

    def use_tool(self):
        tool = transition.load_tool()
        tool_commit = "c" * 40
        members = {name: (b"fixed tool fixture", 0o644) for name in tool.REQUIRED}
        members["scripts/ops/run-production-backup.sh"] = (self.new["helper"], 0o755)
        for name in transition.UNITS:
            members["deploy/systemd/" + name] = (self.new[name], 0o644)
        pin = tool.publish(members, tool_commit, COMMIT, install_root=self.layout.install)
        self.new["config"] += (
            "DAILY_BACKUP_TOOL_COMMIT=" + tool_commit + "\n"
            "DAILY_BACKUP_TOOL_MANIFEST=" + pin + "\n").encode()
        self.source_config.write_bytes(self.new["config"])
        self.value.update(schema=transition.TOOL_SCHEMA, tool_commit=tool_commit,
                          tool_manifest_sha256=pin)
        self.value["new"]["config"] = transition.digest(self.new["config"])
        self.layout = transition.Layout(self.release, self.source_config, self.root,
            tool=self.layout.install / "maintenance" / tool_commit)
        self.operation = self.operation_at(self.recovery)

    def test_tool_transition_preserves_application_and_recovers_exact_six_files(self):
        before = {p.relative_to(self.release).as_posix(): p.read_bytes()
                  for p in self.release.rglob("*") if p.is_file()}
        self.use_tool()
        self.assertEqual(transition.contract(transition.files.canonical(self.value)), self.value)
        transition.Native(self.layout, self.value).current()
        self.operation.apply()
        self.assert_bytes(self.new)
        self.assertEqual(self.journal()["schema"], transition.TOOL_SCHEMA)
        self.operation.recover()
        self.assert_bytes(self.old)
        self.assertEqual({p.relative_to(self.release).as_posix(): p.read_bytes()
                         for p in self.release.rglob("*") if p.is_file()}, before)
        self.assertEqual(os.readlink(self.layout.install / "current"), "releases/" + COMMIT)

    def test_tool_transition_rejects_changed_tool_and_wrong_current_application(self):
        self.use_tool()
        native = transition.Native(self.layout, self.value)
        native.current()
        self.value["tool_manifest_sha256"] = "sha256:" + "0" * 64
        with self.assertRaises(ValueError):
            native.current()
        (self.layout.install / "current").unlink()
        (self.layout.install / "current").symlink_to("releases/" + "d" * 40)
        with self.assertRaises(transition.Refused):
            native.current()
        self.assert_bytes(self.old)

    def test_tool_transition_refuses_config_mismatch_before_replacement(self):
        self.use_tool()
        self.new["config"] = self.new["config"].replace(b"DAILY_BACKUP_TOOL_COMMIT=" + b"c" * 40,
                                                       b"DAILY_BACKUP_TOOL_COMMIT=" + b"d" * 40)
        self.source_config.write_bytes(self.new["config"])
        self.value["new"]["config"] = transition.digest(self.new["config"])
        with self.assertRaises(transition.Refused):
            self.operation.apply()
        self.assert_bytes(self.old)
        self.assertFalse(self.recovery.exists())

    def test_tool_transition_failed_installed_check_restores_original_files(self):
        self.use_tool()
        self.native.fail_check = True
        with self.assertRaises(transition.Refused):
            self.operation.apply()
        self.assert_bytes(self.old)
        self.assertEqual(self.journal()["state"], "RECOVERED_FILES_TIMERS_INACTIVE")

    def test_preflight_checks_without_writes_or_service_mutations(self):
        self.operation.preflight()
        self.assert_bytes(self.old)
        self.assertFalse(self.recovery.exists())
        self.assertNotIn("daemon-reload", self.native.calls)
        self.assertEqual([x for x in self.native.calls if isinstance(x, tuple) and x[0] == "check"],
                         [("check", False)])

    def test_apply_replaces_all_six_and_preserves_private_recovery(self):
        self.operation.apply()
        self.assert_bytes(self.new)
        self.assertEqual(self.journal()["state"], "APPLIED_TIMERS_INACTIVE")
        self.assertEqual(stat.S_IMODE(self.recovery.stat().st_mode), 0o700)
        for index, name in enumerate(transition.NAMES):
            old = self.recovery / f"old-{index}"
            self.assertEqual(old.read_bytes(), self.old[name])
            self.assertEqual(stat.S_IMODE(old.stat().st_mode), 0o600)
        self.assertIn(("check", True), self.native.calls)
        self.assertEqual(self.native.calls.count("daemon-reload"), 1)

    def test_existing_recovery_directory_refuses_without_target_writes(self):
        self.recovery.mkdir()
        with self.assertRaises(FileExistsError):
            self.operation.apply()
        self.assert_bytes(self.old)

    def test_old_hash_mismatch_refuses_before_recovery_or_target_writes(self):
        self.value["old"]["helper"] = "sha256:" + "0" * 64
        with self.assertRaises(transition.Refused):
            self.operation.apply()
        self.assert_bytes(self.old)
        self.assertFalse(self.recovery.exists())

    def test_new_hash_mismatch_refuses_before_recovery_or_target_writes(self):
        self.source_config.write_bytes(b"changed source")
        with self.assertRaises(transition.Refused):
            self.operation.apply()
        self.assert_bytes(self.old)
        self.assertFalse(self.recovery.exists())

    def test_new_config_cannot_retain_an_old_release_binding(self):
        self.new["config"] = self.new["config"].replace(COMMIT.encode(), b"b" * 40)
        self.source_config.write_bytes(self.new["config"])
        self.value["new"]["config"] = transition.digest(self.new["config"])
        with self.assertRaises(transition.Refused):
            self.operation.apply()
        self.assert_bytes(self.old)
        self.assertFalse(self.recovery.exists())

    def test_active_unit_or_changed_release_refuses_without_writes(self):
        for attr in ("active", "changed_current"):
            with self.subTest(attr=attr):
                setattr(self.native, attr, True)
                with self.assertRaises(transition.Refused):
                    self.operation.apply()
                setattr(self.native, attr, False)
                self.assert_bytes(self.old)
                self.assertFalse(self.recovery.exists())

    def test_busy_release_and_backup_locks_refuse_without_writes(self):
        for path in self.layout.locks:
            with self.subTest(lock=path.name), path.open("rb") as held:
                fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
                with self.assertRaises(BlockingIOError):
                    self.operation.apply()
                self.assert_bytes(self.old)
                self.assertFalse(self.recovery.exists())

    def test_wrong_mode_symlink_hardlink_and_acl_are_rejected(self):
        target = self.layout.targets["config"]
        target.chmod(0o644)
        with self.assertRaises(Exception):
            self.operation.preflight()
        target.chmod(0o600)
        alias = self.root / "alias"
        os.link(target, alias)
        with self.assertRaises(Exception):
            self.operation.preflight()
        alias.unlink()
        target.rename(alias)
        target.symlink_to(alias)
        with self.assertRaises(Exception):
            self.operation.preflight()
        target.unlink()
        alias.rename(target)
        # Zero mask preserves mode0600 while the named-user ACL still exists.
        entries = [(1, 6, 0xffffffff), (2, 4, 10002), (4, 0, 0xffffffff),
                   (16, 0, 0xffffffff), (32, 0, 0xffffffff)]
        acl = struct.pack("<I", 2) + b"".join(struct.pack("<HHI", *row) for row in entries)
        os.setxattr(target, "system.posix_acl_access", acl)
        self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o600)
        with self.assertRaises(transition.files.DailyInstallError):
            self.operation.preflight()
        os.removexattr(target, "system.posix_acl_access")
        self.assert_bytes(self.old)
        self.assertFalse(self.recovery.exists())

    def test_each_replacement_failure_restores_only_owned_files(self):
        original = os.replace
        for failing in range(6):
            path = self.root / ("recovery-" + str(failing))
            def replace(source, target, **kwargs):
                if str(source).startswith(".backup-upgrade-") and str(source).endswith("-" + str(failing)):
                    raise OSError("fixture replacement failure")
                return original(source, target, **kwargs)
            with self.subTest(index=failing), mock.patch.object(transition.os, "replace", side_effect=replace):
                with self.assertRaises(OSError):
                    self.operation_at(path).apply()
            self.assert_bytes(self.old)
            self.assertEqual(self.journal(path)["state"], "RECOVERED_FILES_TIMERS_INACTIVE")

    def test_unrecorded_staging_failure_leaves_old_targets_and_requires_review(self):
        original = transition.files.create_file
        def create(parent, name, data, uid, gid, mode):
            original(parent, name, data, uid, gid, mode)
            if name.startswith(".backup-upgrade-") and name.endswith("-1"):
                raise OSError("fixture failure before identity journal")
        with mock.patch.object(transition.files, "create_file", side_effect=create):
            with self.assertRaises(transition.Refused):
                self.operation.apply()
        self.assert_bytes(self.old)
        self.assertEqual(self.journal()["state"], "STAGING")
        with self.assertRaises(transition.Refused):
            self.operation.recover()
        self.assert_bytes(self.old)

    def test_reload_and_installed_check_failures_restore_without_activation(self):
        for attr in ("fail_reload", "fail_check"):
            path = self.root / attr
            setattr(self.native, attr, True)
            with self.subTest(failure=attr), self.assertRaises(transition.Refused):
                self.operation_at(path).apply()
            setattr(self.native, attr, False)
            self.assert_bytes(self.old)
            self.assertEqual(self.journal(path)["state"], "RECOVERED_FILES_TIMERS_INACTIVE")

    def test_foreign_replacement_stops_rollback_before_any_restore(self):
        target = self.layout.targets["config"]
        foreign = self.root / "foreign"
        foreign.write_bytes(b"foreign operator bytes")
        foreign.chmod(0o600)
        def check(installed):
            if installed:
                os.replace(foreign, target)
                raise transition.Refused()
        with mock.patch.object(self.native, "check", side_effect=check):
            with self.assertRaises(transition.Refused):
                self.operation.apply()
        self.assertEqual(target.read_bytes(), b"foreign operator bytes")
        for name, path in self.layout.targets.items():
            if name != "config":
                self.assertEqual(path.read_bytes(), self.new[name])
        self.assertEqual(self.journal()["state"], "PREPARED")

    def test_recover_is_idempotent_and_keeps_timers_inactive(self):
        self.operation.apply()
        self.operation.recover()
        self.operation.recover()
        self.assert_bytes(self.old)
        self.assertEqual(self.journal()["state"], "RECOVERED_FILES_TIMERS_INACTIVE")

    def test_recovery_checks_all_old_copies_before_replacing_any_file(self):
        self.operation.apply()
        (self.recovery / "old-5").write_bytes(b"corrupt recovery copy")
        with self.assertRaises(transition.Refused):
            self.operation.recover()
        self.assert_bytes(self.new)

    def test_recovery_cannot_change_contract_or_source_commitments(self):
        self.operation.apply()
        self.value["old"]["helper"] = "sha256:" + "0" * 64
        with self.assertRaises(transition.Refused):
            self.operation.recover()
        self.assert_bytes(self.new)

    def test_recovery_refuses_identical_bytes_replaced_by_another_inode(self):
        self.operation.apply()
        target = self.layout.targets["helper"]
        other = self.root / "other-helper"
        other.write_bytes(self.new["helper"])
        other.chmod(0o755)
        os.replace(other, target)
        with self.assertRaises(transition.Refused):
            self.operation.recover()
        self.assert_bytes(self.new)

    def test_process_death_after_rename_recovers_from_persisted_journal(self):
        original = os.replace
        child = os.fork()
        if child == 0:
            def replace(source, target, **kwargs):
                original(source, target, **kwargs)
                if str(source).startswith(".backup-upgrade-") and str(source).endswith("-2"):
                    os._exit(71)
            try:
                with mock.patch.object(transition.os, "replace", side_effect=replace):
                    self.operation.apply()
            finally:
                os._exit(72)
        process = os.pidfd_open(child)
        try:
            ready, _, _ = select.select([process], [], [], 10)
            if not ready:
                os.kill(child, signal.SIGKILL)
                os.waitpid(child, 0)
                self.fail("transition fixture child exceeded deadline")
            waited, status = os.waitpid(child, 0)
        finally:
            os.close(process)
        self.assertEqual(waited, child)
        self.assertTrue(os.WIFEXITED(status))
        self.assertEqual(os.WEXITSTATUS(status), 71)
        self.assertEqual(self.layout.targets["helper"].read_bytes(), self.new["helper"])
        self.operation.recover()
        self.assert_bytes(self.old)

    def test_process_death_during_restoration_resumes_exact_original_bytes(self):
        self.operation.apply()
        original = os.replace
        child = os.fork()
        if child == 0:
            def replace(source, target, **kwargs):
                original(source, target, **kwargs)
                if str(source).startswith(".backup-restore-") and str(source).endswith("-3"):
                    os._exit(73)
            try:
                with mock.patch.object(transition.os, "replace", side_effect=replace):
                    self.operation.recover()
            finally:
                os._exit(74)
        process = os.pidfd_open(child)
        try:
            ready, _, _ = select.select([process], [], [], 10)
            if not ready:
                os.kill(child, signal.SIGKILL)
                os.waitpid(child, 0)
                self.fail("transition recovery child exceeded deadline")
            _, status = os.waitpid(child, 0)
        finally:
            os.close(process)
        self.assertTrue(os.WIFEXITED(status))
        self.assertEqual(os.WEXITSTATUS(status), 73)
        self.operation.recover()
        self.assert_bytes(self.old)

    def test_contract_requires_exact_files_bounded_utc_window_and_no_duplicates(self):
        encoded = json.dumps(self.value).encode()
        self.assertEqual(transition.contract(encoded), self.value)
        variants = []
        for key, value in (("expires_at", "2000-01-01T00:00:00Z"),
                           ("expires_at", "2099-01-01T00:00:00Z"),
                           ("config_source", "/etc/../tmp/config"), ("new_commit", "0" * 40)):
            invalid = copy.deepcopy(self.value)
            invalid[key] = value
            variants.append(json.dumps(invalid).encode())
        invalid = copy.deepcopy(self.value)
        invalid["old"]["foreign"] = "sha256:" + "b" * 64
        variants.append(json.dumps(invalid).encode())
        variants.append(b'{"schema":"x",' + encoded[1:])
        for data in variants:
            with self.subTest(data_shape=len(data)), self.assertRaises(Exception):
                transition.contract(data)

    def test_native_unit_checks_reject_running_dropins_stale_and_foreign_fragments(self):
        native = transition.Native(self.layout, self.value)
        native.current()
        def reply(args, limit=30):
            name = args[-1]
            values = {"LoadState": "loaded", "ActiveState": "inactive", "SubState": "dead",
                "FragmentPath": str(self.layout.targets[name]), "DropInPaths": "",
                "NeedDaemonReload": "no", "UnitFileState": "enabled"}
            values.update(changes)
            return ("\n".join(k + "=" + v for k, v in values.items()) + "\n").encode()
        changes = {}
        with mock.patch.object(native, "call", side_effect=reply):
            native.inactive()
            for changes in ({"ActiveState": "active"}, {"DropInPaths": "/foreign.conf"},
                            {"FragmentPath": "/foreign.service"}, {"NeedDaemonReload": "yes"}):
                with self.subTest(change=changes), self.assertRaises(transition.Refused):
                    native.inactive()
            changes = {"NeedDaemonReload": "yes"}
            native.inactive(allow_reload=True)

    def test_native_official_checks_have_fixed_read_only_arguments(self):
        native = transition.Native(self.layout, self.value)
        with mock.patch.object(native, "call", return_value=b"") as call:
            native.check(False)
            native.check(True)
        self.assertEqual(call.call_args_list[0].args[0], ["/bin/bash", "--noprofile", "--norc",
            str(self.layout.sources["helper"]), "--check", "--config-file", str(self.source_config)])
        self.assertEqual(call.call_args_list[1].args[0], ["/bin/bash", "--noprofile", "--norc",
            str(self.release / "scripts/ops/install-production-backup.sh"), "--check", "--config-source",
            str(self.source_config)])


if __name__ == "__main__":
    unittest.main(verbosity=2)
