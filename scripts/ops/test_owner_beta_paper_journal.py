#!/usr/bin/env python3
"""Disposable filesystem tests for the protected Paper journal core."""
from __future__ import annotations

import fcntl
import hashlib
import importlib.util
import json
import os
import pathlib
import shutil
import stat
import tempfile
import unittest
import uuid
from unittest import mock


SOURCE = pathlib.Path(__file__).with_name("lib") / "owner-beta-paper-journal.py"
SPEC = importlib.util.spec_from_file_location("owner_beta_paper_journal", SOURCE)
journal = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(journal)

def canonical(value):
    return json.dumps(value, ensure_ascii=True, sort_keys=True, separators=(",", ":")).encode("ascii")


def digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def uid_dir_info(fd, mode=None):
    """Test-local ownership adapter; all operations still use real Linux FDs."""
    info = os.fstat(fd)
    if (not stat.S_ISDIR(info.st_mode) or info.st_uid not in (0, 65534, os.geteuid())
            or info.st_gid not in (0, 65534, os.getegid())):
        journal._fail("JOURNAL_DIRECTORY")
    if mode is not None and stat.S_IMODE(info.st_mode) != mode:
        journal._fail("JOURNAL_DIRECTORY")
    journal._no_acl(fd)
    return info


def uid_file_info(fd, mode=None):
    """Test-local ownership adapter; production still requires root:root."""
    info = os.fstat(fd)
    if (not stat.S_ISREG(info.st_mode) or info.st_uid not in (0, 65534, os.geteuid())
            or info.st_gid not in (0, 65534, os.getegid()) or info.st_nlink != 1
            or (mode is not None and stat.S_IMODE(info.st_mode) != mode)):
        journal._fail("JOURNAL_FILE")
    journal._no_acl(fd)
    return info


def test_guards():
    return (mock.patch.object(journal, "_require_root", lambda: None),
            mock.patch.object(journal, "_dir_info", uid_dir_info),
            mock.patch.object(journal, "_file_info", uid_file_info))


class JournalFixture(unittest.TestCase):
    def setUp(self):
        # Hosted checkout ancestors may carry ACLs unrelated to this fixture.
        # Keep real ACL checks on every opened ancestor and test node; place
        # only this owned, private test tree outside the checkout instead.
        self.base = pathlib.Path(tempfile.mkdtemp(prefix="lagrange-paper-journal-", dir="/tmp"))
        self.install = self.base / "install"
        self.install.mkdir(mode=0o700)
        self.lock_path = self.install / journal.LOCK_NAME
        self.lock_path.touch(mode=0o600)
        os.chmod(self.lock_path, 0o600)
        self.lock_fd = os.open(self.lock_path, os.O_RDWR | os.O_CLOEXEC)
        fcntl.flock(self.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        self.patches = test_guards()
        for patch in self.patches:
            patch.start()

    def tearDown(self):
        for patch in reversed(self.patches):
            patch.stop()
        try:
            fcntl.flock(self.lock_fd, fcntl.LOCK_UN)
            os.close(self.lock_fd)
        except OSError:
            pass
        shutil.rmtree(self.base, ignore_errors=True)

    def opened(self, bootstrap=True):
        return journal.open_journal(str(self.install), self.lock_fd, bootstrap=bootstrap)

    def intent(self, *, account="33333333-3333-4333-8333-333333333333", generation=None,
               artifact=None, purpose="STOP", request_key=None):
        generation = generation or str(uuid.uuid4())
        expected = 0 if purpose == "CONFIGURE" or (purpose == "INSPECT" and account is None) else 3
        default_key = {"ENABLE": generation + ":enable", "RECOVERY": generation + ":recover",
                       "STOP": generation + ":begin", "BACKUP": generation + ":begin",
                       "RESTORE": generation + ":begin"}.get(purpose, "request-1")
        return canonical({
            "schema": "owner-beta-paper-operation-intent-v1",
            "installation_id": self.installation_id,
            "generation": generation,
            "account_id": account,
            "predecessor_generation": None,
            "request_key": request_key or default_key,
            "purpose": purpose,
            "scope": "COMBINED_BACKUP" if purpose == "RESTORE" else "PAPER_STOP",
            "expected_epoch": expected,
            "local_epoch_floor": expected,
            "release_manifest_sha256": "sha256:" + "a" * 64,
            "artifact_sha256": artifact,
            "resume_target_id": None,
        })

    def stage(self, operation, name, sequence, previous, *, error="NONE", account=None):
        value = {
            "schema": "owner-beta-paper-operation-stage-v1",
            "installation_id": self.installation_id,
            "generation": operation.generation,
            "account_id": account or "33333333-3333-4333-8333-333333333333",
            "sequence": sequence,
            "previous_sha256": previous,
            "intent_sha256": operation.intent_sha256,
            "stage": name,
            "boot_id": "44444444-4444-4444-8444-444444444444",
            "observed": {"realtime_ns": sequence + 1, "monotonic_ns": sequence + 2, "boottime_ns": sequence + 3},
            "control_epoch": 3,
            "artifact_sha256": None,
            "event_id": None,
            "error_code": error,
        }
        return canonical(value)

    def open_operation(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            data = self.intent()
            operation = store.create(data)
            operation.close()
            return json.loads(data)["generation"], data


class CanonicalWireTests(unittest.TestCase):
    def test_canonical_parser_rejects_noncanonical_and_unsafe_numbers(self):
        for data in (b'{"a":1 }', b'{"a":1,"a":2}', b'{"a":1.0}', b'{"a":1e0}',
                     b'{"a":-0}', b'{"a":9223372036854775808}', b'{"a":NaN}'):
            with self.subTest(data=data), self.assertRaises(journal.JournalError):
                journal._json(data)
        deep = (b"[" * 17) + b"0" + (b"]" * 17)
        with self.assertRaises(journal.JournalError):
            journal._json(deep)

    @unittest.skipIf(os.geteuid() == 0, "production root gate passes in this process")
    def test_production_api_requires_root(self):
        with self.assertRaises(journal.JournalError):
            journal._require_root()

    def test_uuid_hash_counter_and_civil_date_are_closed(self):
        for call in (lambda: journal._uuid("00000000-0000-0000-0000-000000000000"),
                     lambda: journal._hash("sha256:" + "A" * 64),
                     lambda: journal._counter(True),
                     lambda: journal._counter(-1),
                     lambda: journal._civil_date({"date": "2026-02-30"})):
            with self.subTest(call=call), self.assertRaises(journal.JournalError):
                call()


class JournalFilesystemTests(JournalFixture):
    def test_bootstrap_floor_monotonicity_and_lock_lifetime(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            self.assertEqual(store.epoch_floor, 0)
            self.assertEqual(store.advance_floor(7), 7)
            self.assertEqual(store.advance_floor(7), 7)
            with self.assertRaises(journal.JournalError):
                store.advance_floor(6)
            with self.assertRaises(journal.JournalError):
                store.advance_floor(journal.MAX_I64 + 1)
        self.assertTrue(os.fstat(self.lock_fd).st_ino)
        other = os.open(self.lock_path, os.O_RDWR | os.O_CLOEXEC)
        try:
            with self.assertRaises(BlockingIOError):
                fcntl.flock(other, fcntl.LOCK_EX | fcntl.LOCK_NB)
        finally:
            os.close(other)

    def test_fresh_journal_requires_explicit_bootstrap(self):
        with self.assertRaises(journal.JournalError):
            with self.opened(bootstrap=False):
                pass

    def test_installation_missing_with_history_cannot_bootstrap(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            data = self.intent()
            op = store.create(data)
            op.close()
        os.unlink(self.install / journal.PAPER_DIR / "installation-id")
        with self.assertRaises(journal.JournalError):
            with self.opened(bootstrap=True):
                pass

    def test_floor_missing_after_identity_exists_cannot_rebootstrap(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
        os.unlink(self.install / journal.PAPER_DIR / "epoch-floor")
        with self.assertRaises(journal.JournalError):
            with self.opened(bootstrap=True):
                pass

    def test_retained_installation_identity_rejects_named_replacement(self):
        with self.assertRaises(journal.JournalError):
            with self.opened() as store:
                identity = self.install / journal.PAPER_DIR / "installation-id"
                replacement = self.base / "identity-copy"
                replacement.write_bytes(identity.read_bytes())
                os.chmod(replacement, 0o600)
                identity.unlink()
                replacement.rename(identity)
                store.revalidate()

    def test_create_idempotence_and_generation_conflict(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            original = self.intent()
            generation = json.loads(original)["generation"]
            op = store.create(original)
            op.close()
            same = store.create(original)
            self.assertIsNone(same.revalidate())
            same.close()
            altered = json.loads(original)
            altered["request_key"] = "changed"
            with self.assertRaises(journal.JournalError):
                store.create(canonical(altered))
            opened = store.open(generation)
            self.assertEqual(opened.generation, generation)
            opened.close()

    def test_account_request_key_cannot_move_to_new_generation(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            first = self.intent(account=None, purpose="INSPECT", request_key="stable-inspect-key")
            op = store.create(first)
            op.close()
            second = json.loads(first)
            second["generation"] = str(uuid.uuid4())
            with self.assertRaises(journal.JournalError):
                store.create(canonical(second))

    def test_stage_chain_order_refresh_and_hash_link(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            op = store.create(self.intent())
            first = self.stage(op, "INTENT", 0, None)
            first_hash = op.append(first)
            self.assertEqual(op.append(first), first_hash)
            second = self.stage(op, "FENCED", 1, first_hash)
            second_hash = op.append(second)
            stopped = self.stage(op, "STOPPED", 2, second_hash)
            stopped_hash = op.append(stopped)
            third = self.stage(op, "RECONCILED", 3, stopped_hash)
            third_hash = op.append(third)
            refresh_hash = op.append(self.stage(op, "RECONCILED", 4, third_hash))
            self.assertEqual(op.revalidate(), "RECONCILED")
            with self.assertRaises(journal.JournalError):
                op.append(self.stage(op, "SNAPSHOT", 5, refresh_hash))
            op.close()

    def test_configure_null_account_cannot_mint_stage(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            op = store.create(self.intent(account=None, purpose="CONFIGURE"))
            with self.assertRaises(journal.JournalError):
                op.append(self.stage(op, "INTENT", 0, None))
            op.close()

    def test_stage_gap_alias_fork_symlink_hardlink_and_mode_changes_fail(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            op = store.create(self.intent())
            # Sequence 1 without 0 is not adopted by directory scanning.
            gap = self.stage(op, "FENCED", 1, "sha256:" + "0" * 64)
            (self.install / journal.PAPER_DIR / "operations" / op.generation / "00000001.json").write_bytes(gap)
            os.chmod(self.install / journal.PAPER_DIR / "operations" / op.generation / "00000001.json", 0o400)
            with self.assertRaises(journal.JournalError):
                op.revalidate()
            (self.install / journal.PAPER_DIR / "operations" / op.generation / "00000001.json").unlink()
            first = self.stage(op, "INTENT", 0, None)
            first_hash = op.append(first)
            second = self.stage(op, "FENCED", 1, first_hash)
            op.append(second)
            op.close()
        with self.opened(bootstrap=False) as store:
            self.installation_id = store.installation_id
            generation = next((self.install / journal.PAPER_DIR / "operations").iterdir()).name
            op = store.open(generation)
            stage_path = self.install / journal.PAPER_DIR / "operations" / generation / "00000000.json"
            stage_path.unlink()
            stage_path.write_text("{}", encoding="ascii")
            os.chmod(stage_path, 0o400)
            with self.assertRaises(journal.JournalError):
                op.revalidate()
            op.close()

    def test_alias_and_unknown_torn_temp_are_incomplete(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            op = store.create(self.intent())
            op.append(self.stage(op, "INTENT", 0, None))
            directory = self.install / journal.PAPER_DIR / "operations" / op.generation
            (directory / "000000000.json").write_bytes((directory / "00000000.json").read_bytes())
            os.chmod(directory / "000000000.json", 0o400)
            with self.assertRaises(journal.JournalError):
                op.revalidate()
            (directory / "000000000.json").unlink()
            temp = directory / (".unknown.tmp." + str(uuid.uuid4()))
            temp.write_bytes(b"partial")
            with self.assertRaises(journal.JournalError):
                op.revalidate()
            temp.unlink()
            op.close()

    def test_no_replace_idempotent_originals_and_conflicts(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            op = store.create(self.intent())
            content = b'{"original":1}'
            hash_value = digest(content)
            op._store_original(content, hash_value, 1024)
            op._store_original(content, hash_value, 1024)
            self.assertEqual(op.read_original(hash_value), content)
            with self.assertRaises(journal.JournalError):
                op._store_original(b'{"changed":1}', hash_value, 1024)
            op.close()

    def test_retained_original_rejects_hardlink_and_symlink(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            op = store.create(self.intent())
            intent_path = self.install / journal.PAPER_DIR / "operations" / op.generation / "intent.json"
            hardlink = self.base / "hardlink"
            os.link(intent_path, hardlink)
            with self.assertRaises(journal.JournalError):
                op.revalidate()
            op.close()

    def test_retained_stage_rejects_mode_change(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            op = store.create(self.intent())
            op.append(self.stage(op, "INTENT", 0, None))
            stage_path = self.install / journal.PAPER_DIR / "operations" / op.generation / "00000000.json"
            os.chmod(stage_path, 0o600)
            with self.assertRaises(journal.JournalError):
                op.revalidate()
            op.close()

    def test_retained_original_rejects_symlink_replacement(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            op = store.create(self.intent())
            intent_path = self.install / journal.PAPER_DIR / "operations" / op.generation / "intent.json"
            target = self.base / "target"
            target.write_bytes(intent_path.read_bytes())
            os.chmod(target, 0o400)
            intent_path.unlink()
            intent_path.symlink_to(target)
            with self.assertRaises(journal.JournalError):
                op.revalidate()
            op.close()

    def test_supported_posix_acl_is_rejected(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            op = store.create(self.intent())
            intent_path = self.install / journal.PAPER_DIR / "operations" / op.generation / "intent.json"
            os.chmod(intent_path, 0o600)
            acl = (2).to_bytes(4, "little") + b"".join(
                tag.to_bytes(2, "little") + permissions.to_bytes(2, "little") + identity.to_bytes(4, "little")
                for tag, permissions, identity in (
                    (1, 6, 0xFFFFFFFF), (2, 4, 12345), (4, 0, 0xFFFFFFFF),
                    (16, 4, 0xFFFFFFFF), (32, 0, 0xFFFFFFFF)))
            try:
                os.setxattr(intent_path, "system.posix_acl_access", acl, follow_symlinks=False)
                os.chmod(intent_path, 0o400)
            except OSError as error:
                if error.errno in (getattr(os, "ENOTSUP", 95), getattr(os, "EOPNOTSUPP", 95), 1, 22, 95):
                    self.skipTest("POSIX ACL xattrs unavailable for this filesystem/user")
                raise
            with self.assertRaises(journal.JournalError):
                op.revalidate()
            op.close()

    def test_write_or_fsync_failure_leaves_incomplete_marker(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            op = store.create(self.intent())
            real_fsync = os.fsync
            calls = 0

            def failing_fsync(fd):
                nonlocal calls
                calls += 1
                if calls == 2:
                    raise OSError("simulated fsync failure")
                return real_fsync(fd)

            with mock.patch.object(journal.os, "fsync", failing_fsync):
                with self.assertRaises(OSError):
                    op.append(self.stage(op, "INTENT", 0, None))
            with self.assertRaises(journal.JournalError):
                op.revalidate()
            generation = op.generation
            op.close()
        with self.opened(bootstrap=False) as store:
            self.installation_id = store.installation_id
            with self.assertRaises(journal.JournalError):
                store.open(generation)

    def test_write_failure_leaves_incomplete_marker(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            op = store.create(self.intent())
            with mock.patch.object(journal.os, "write", side_effect=OSError("simulated write failure")):
                with self.assertRaises(OSError):
                    op.append(self.stage(op, "INTENT", 0, None))
            with self.assertRaises(journal.JournalError):
                op.revalidate()
            op.close()

    def test_epoch_floor_rename_failure_keeps_old_floor_and_blocks_restart(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            with mock.patch.object(journal.os, "rename", side_effect=OSError("simulated rename failure")):
                with self.assertRaises(OSError):
                    store.advance_floor(2)
            self.assertEqual(store.epoch_floor, 0)
            self.assertEqual((self.install / journal.PAPER_DIR / "epoch-floor").read_bytes(), b"0\n")
        with self.assertRaises(journal.JournalError):
            with self.opened(bootstrap=False):
                pass

    def test_publish_link_failure_leaves_incomplete_marker(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            op = store.create(self.intent())
            with mock.patch.object(journal.os, "link", side_effect=OSError("simulated publish failure")):
                with self.assertRaises(OSError):
                    op.append(self.stage(op, "INTENT", 0, None))
            with self.assertRaises(journal.JournalError):
                op.revalidate()
            op.close()

    def test_named_lock_replacement_blocks_retained_journal(self):
        with self.assertRaises(journal.JournalError):
            with self.opened() as store:
                self.installation_id = store.installation_id
                old = self.lock_path
                replacement = self.install / "replacement"
                replacement.touch(mode=0o600)
                os.chmod(replacement, 0o600)
                old.unlink()
                replacement.rename(old)
                store.revalidate()


class CustodyTests(JournalFixture):
    def _native_dispatch(self, intent_bytes):
        intent = json.loads(intent_bytes)
        sql = canonical({
            "schema": "owner-beta-paper-exact-request-v1",
            "kind": "BEGIN",
            "account_id": intent["account_id"],
            "args": {"expected_epoch": intent["expected_epoch"],
                     "intent_canonical": intent_bytes.decode("ascii"),
                     "request_key": intent["request_key"]},
        })
        fields = {
            "schema": "owner-beta-paper-native-dispatch-v1",
            "intent_canonical": intent_bytes.decode("ascii"),
            "intent_sha256": digest(intent_bytes),
            "artifact_canonical": None,
            "artifact_sha256": None,
            "sql_request_canonical": sql.decode("ascii"),
            "sql_request_sha256": digest(sql),
        }
        return sql, canonical(fields)

    def _make_output(self, files):
        parent = self.base / ("outputs-" + str(uuid.uuid4()))
        parent.mkdir(mode=0o700)
        os.chmod(parent, 0o700)
        output = parent / "invocation"
        output.mkdir(mode=0o700)
        os.chmod(output, 0o700)
        for data in files:
            (output / (digest(data)[7:] + ".json")).write_bytes(data)
            os.chmod(output / (digest(data)[7:] + ".json"), 0o400)
        parent_fd = os.open(parent, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
        protected = journal.protected_output(parent_fd, "invocation")
        os.close(parent_fd)
        return protected, output

    def test_exact_original_dispatch_import_and_readback_stage(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            intent_bytes = self.intent()
            sql, index = self._native_dispatch(intent_bytes)
            protected, output_path = self._make_output((intent_bytes, sql, index))
            try:
                op = store.create(intent_bytes)
                custody = op.import_native_dispatch(protected, digest(index))
                destination = self.base / "readback"
                destination.mkdir(mode=0o700)
                os.chmod(destination, 0o700)
                fd = os.open(destination, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
                try:
                    custody.stage_readback(fd)
                finally:
                    os.close(fd)
                self.assertEqual((destination / "request.json").read_bytes(), intent_bytes)
                self.assertEqual((destination / "sql-request.json").read_bytes(), sql)
                self.assertEqual(stat.S_IMODE((destination / "request.json").stat().st_mode), 0o400)
                self.assertEqual(op.read_original(digest(index)), index)
                self.assertEqual(op.read_original(digest(sql)), sql)
                op.close()
            finally:
                protected.close()

    def test_dispatch_hash_and_original_intent_mismatch_fail_closed(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            original = self.intent()
            sql, index = self._native_dispatch(original)
            protected, _ = self._make_output((original, sql, index))
            try:
                op = store.create(original)
                with self.assertRaises(journal.JournalError):
                    op.import_native_dispatch(protected, "sha256:" + "0" * 64)
                op.close()
            finally:
                protected.close()

    def test_missing_preimage_object_and_correlated_intent_mismatch_fail(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            original = self.intent()
            sql, index = self._native_dispatch(original)
            missing_output, output_path = self._make_output((original, sql, index))
            try:
                (output_path / (digest(sql)[7:] + ".json")).unlink()
                op = store.create(original)
                with self.assertRaises(journal.JournalError):
                    op.import_native_dispatch(missing_output, digest(index))
                op.close()
            finally:
                missing_output.close()

            corrupt_output, corrupt_path = self._make_output((original, sql, index))
            try:
                index_path = corrupt_path / (digest(index)[7:] + ".json")
                os.chmod(index_path, 0o600)
                index_path.write_bytes(index + b" ")
                os.chmod(index_path, 0o400)
                op = store.create(original)
                with self.assertRaises(journal.JournalError):
                    op.import_native_dispatch(corrupt_output, digest(index))
                op.close()
            finally:
                corrupt_output.close()

            foreign_intent = self.intent(purpose="BACKUP")
            foreign_sql, _ = self._native_dispatch(foreign_intent)
            foreign_index_value = {
                "schema": "owner-beta-paper-native-dispatch-v1",
                "intent_canonical": foreign_intent.decode("ascii"),
                "intent_sha256": digest(foreign_intent),
                "artifact_canonical": None,
                "artifact_sha256": None,
                "sql_request_canonical": foreign_sql.decode("ascii"),
                "sql_request_sha256": digest(foreign_sql),
            }
            foreign_index = canonical(foreign_index_value)
            other_output, _ = self._make_output((foreign_intent, foreign_sql, foreign_index))
            try:
                op = store.create(original)
                with self.assertRaises(journal.JournalError):
                    op.import_native_dispatch(other_output, digest(foreign_index))
                op.close()
            finally:
                other_output.close()

    def test_cleanup_requires_drain_is_preserved_across_readback_staging(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            artifact_value = {
                "schema": "owner-beta-paper-process-v1",
                "owner_user_id": "11111111-1111-4111-8111-111111111111",
                "account_id": "33333333-3333-4333-8333-333333333333",
                "strategy_config_id": "22222222-2222-4222-8222-222222222222",
                "consent_event_id": "66666666-6666-4666-8666-666666666666",
                "consent_version": "owner-beta-paper-v1-draft-1",
                "control_epoch": 4,
                "daily_cohort_sha256": "sha256:" + "c" * 64,
                "financial_contract_sha256": "sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0",
                "code_commit": "d" * 40,
                "release_manifest_sha256": "sha256:" + "a" * 64,
                "api_binary_sha256": "sha256:" + "e" * 64,
                "worker_binary_sha256": "sha256:" + "f" * 64,
            }
            artifact_bytes = canonical(artifact_value)
            artifact_hash = digest(artifact_bytes)
            intent_bytes = self.intent(purpose="ENABLE", artifact=artifact_hash)
            intent = json.loads(intent_bytes)
            evidence = {
                "expected_epoch": intent["expected_epoch"],
                "new_epoch": intent["expected_epoch"] + 1,
                "process_binding": artifact_value,
                "process_binding_sha256": artifact_hash,
                "current_cohort": {
                    "release_manifest_sha256": intent["release_manifest_sha256"],
                    "cohort_sha256": artifact_value["daily_cohort_sha256"],
                    "owner_user_id": artifact_value["owner_user_id"],
                    "strategy_config_id": artifact_value["strategy_config_id"],
                },
            }
            sql = canonical({
                "schema": "owner-beta-paper-exact-request-v1",
                "kind": "TRANSITION",
                "account_id": intent["account_id"],
                "args": {"expected_epoch": intent["expected_epoch"], "transition": "ENABLE",
                         "evidence_canonical": canonical(evidence).decode("ascii"),
                         "request_key": intent["request_key"]},
            })
            dispatch = canonical({
                "schema": "owner-beta-paper-native-dispatch-v1",
                "intent_canonical": intent_bytes.decode("ascii"),
                "intent_sha256": digest(intent_bytes),
                "artifact_canonical": artifact_bytes.decode("ascii"),
                "artifact_sha256": artifact_hash,
                "sql_request_canonical": sql.decode("ascii"),
                "sql_request_sha256": digest(sql),
            })
            dispatch_value = json.loads(dispatch)
            cleanup = canonical({
                "schema": "owner-beta-paper-native-cleanup-v1",
                "generation": json.loads(intent_bytes)["generation"],
                "sql_request_sha256": dispatch_value["sql_request_sha256"],
                "commit_started": True,
                "commit_acknowledged": True,
                "requires_drain": True,
                "result": {
                    "schema": "owner-beta-paper-ops-result-v1",
                    "verb": "enable",
                    "outcome": "COMMIT_UNKNOWN",
                    "event_id": "55555555-5555-4555-8555-555555555555",
                    "generation": json.loads(intent_bytes)["generation"],
                    "control_epoch": 4,
                    "artifact_sha256": "sha256:" + "b" * 64,
                    "error_code": "COMMIT_UNKNOWN",
                },
            })
            protected, _ = self._make_output((intent_bytes, artifact_bytes, sql, dispatch, cleanup))
            try:
                op = store.create(intent_bytes)
                custody = op.import_native_dispatch(protected, digest(dispatch))
                notice = op.import_native_cleanup(protected, digest(cleanup))
                self.assertTrue(notice.requires_drain)
                self.assertEqual(op.read_original(digest(cleanup)), cleanup)
                destination = self.base / "cleanup-readback"
                destination.mkdir(mode=0o700)
                os.chmod(destination, 0o700)
                fd = os.open(destination, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
                try:
                    custody.stage_readback(fd)
                finally:
                    os.close(fd)
                self.assertEqual(op.read_original(digest(cleanup)), cleanup)
                op.close()
            finally:
                protected.close()


if __name__ == "__main__":
    unittest.main()
