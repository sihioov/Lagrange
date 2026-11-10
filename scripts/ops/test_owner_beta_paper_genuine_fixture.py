#!/usr/bin/env python3
"""Nonprivileged decision tests. None constitutes genuine fixture evidence."""
import importlib.util
import inspect
import contextlib
import io
import json
import tempfile
import types
import os
import pathlib
import struct
import subprocess
import sys
import time
import unittest
from unittest import mock

sys.dont_write_bytecode = True
PATH = pathlib.Path(__file__).with_name("owner-beta-paper-genuine-fixture.py")
SPEC = importlib.util.spec_from_file_location("genuine_fixture_unit", PATH)
fixture = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = fixture
SPEC.loader.exec_module(fixture)
faults = fixture.faults


class FixtureDecisionTests(unittest.TestCase):
    def test_capture_driver_accepts_only_exact_downstream_boundary_after_consumption(self):
        class CaptureError(Exception):
            pass
        for code in ("PAPER_BACKUP_REFERENCE_CLOSURE_INCOMPLETE", "PAPER_SELECTION_HISTORICAL_ORIGINAL_MISSING"):
            owner = mock.MagicMock()
            owner.__enter__.return_value = owner
            owner.capture.side_effect = CaptureError(code)
            owner.maximum, owner._closed, owner._snapshot = 12, True, None
            owner._resources_closed.return_value = True
            paper = types.SimpleNamespace(PaperBackupError=CaptureError, open_snapshot=mock.Mock(return_value=owner))
            with mock.patch.object(fixture.driver, "_paper_backup", return_value=paper):
                if code.endswith("CLOSURE_INCOMPLETE"):
                    fixture.driver.run_capture_prefix(object(), object())
                    owner.revalidate.assert_called_once()
                else:
                    with self.assertRaises(fixture.ops.PaperOperationsError):
                        fixture.driver.run_capture_prefix(object(), object())
            owner.capture.assert_called_once()
            owner.__exit__.assert_called_once()

    def test_fixture_cleanup_retries_actual_archive_owner_before_other_originals(self):
        owner = object.__new__(fixture.FixtureOwner)
        owner.cancel_event = mock.Mock()
        context = mock.Mock()
        owner.contexts = [context]
        archive = types.SimpleNamespace(closed=False,
            owner=types.SimpleNamespace(close=mock.Mock(side_effect=RuntimeError("component-unresolved"))))
        attempt = types.SimpleNamespace(archive=archive)
        with mock.patch.dict(fixture.ops._STOP_ATTEMPTS, {context: {object(): attempt}}, clear=True):
            with self.assertRaises(RuntimeError):
                owner.resolve_helpers()
        archive.owner.close.assert_called_once()
        self.assertFalse(archive.closed)

    def test_cleanup_preserves_primary_failure_and_does_not_claim_cleanup_success(self):
        primary = fixture.FixtureError("FIXTURE_FLOOR")
        disposal = fixture.FixtureError("FIXTURE_BACKEND_DISPOSAL_UNPROVEN")
        owner = types.SimpleNamespace(extractions=contextlib.nullcontext(),
            setup=mock.Mock(side_effect=primary), dispose=mock.Mock(side_effect=disposal))
        runner = fixture.FixtureRunner(pathlib.Path("/component"), "a" * 40, b"", mock.Mock(), None)
        with mock.patch.object(fixture, "FixtureOwner", return_value=owner):
            with self.assertRaises(fixture.FixtureError) as result:
                with runner.fixture_case("STOP"):
                    self.fail("failed setup must not yield")
        self.assertIs(result.exception, primary)
        self.assertIs(result.exception.__cause__, disposal)
        self.assertEqual(runner.cleanups, [])
        owner.dispose.assert_called_once()

    def test_cleanup_does_not_retry_prepared_work_or_observer_query_errors_as_backend_liveness(self):
        for code in ("FIXTURE_PREPARED_WORK", "FIXTURE_SQL_UNKNOWN", "FIXTURE_BACKEND_IDENTITY"):
            with self.subTest(code=code):
                owner = fixture.FixtureOwner.__new__(fixture.FixtureOwner)
                owner.tracees, owner.blockers, owner.contexts, owner.threads = [], [], [], []
                owner.cancel_event = mock.Mock()
                owner.helpers, owner.helper_bindings, owner.cleanup = {}, {}, {}
                owner.output = mock.Mock(return_value="")
                owner.deadline = types.SimpleNamespace(seconds=lambda *a, **k: 10)
                error = fixture.FixtureError(code)
                owner.observer = types.SimpleNamespace(absent=mock.Mock(side_effect=error))
                with mock.patch.object(fixture.time, "sleep") as sleep, \
                        mock.patch.object(fixture.time, "monotonic", side_effect=[0, 11]):
                    with self.assertRaises(fixture.FixtureError) as result:
                        owner.resolve_helpers()
                self.assertIs(result.exception, error)
                self.assertEqual(owner.observer.absent.call_count, 1)
                sleep.assert_not_called()
                self.assertNotIn("backends", owner.cleanup)

    def test_backend_liveness_still_requires_actual_second_absence(self):
        owner = fixture.FixtureOwner.__new__(fixture.FixtureOwner)
        owner.tracees, owner.blockers, owner.contexts, owner.threads = [], [], [], []
        owner.cancel_event = mock.Mock()
        owner.helpers, owner.helper_bindings, owner.cleanup = {}, {}, {}
        owner.output = mock.Mock(return_value="")
        owner.deadline = types.SimpleNamespace(seconds=lambda *a, **k: 10)
        owner.observer = types.SimpleNamespace(absent=mock.Mock(
            side_effect=[fixture.FixtureError("FIXTURE_BACKEND_ALIVE"), None]))
        with mock.patch.object(fixture.time, "sleep"):
            owner.resolve_helpers()
        self.assertEqual(owner.observer.absent.call_count, 2)
        self.assertEqual(owner.cleanup["backends"], "ABSENT_PREPARED_ZERO")

    def test_maintenance_counters_reject_null_and_extra_fields(self):
        observer = fixture.PgObserver(types.SimpleNamespace())
        good = dict.fromkeys(("reconcile_sql", "off_sql", "reconcile_function", "off_function",
                              "observations", "off_events"), 1)
        for value in ({**good, "off_function": None}, {**good, "off_function": True},
                      {**good, "caller_verified": True}):
            with self.subTest(value=value), mock.patch.object(observer, "json", return_value=value):
                with self.assertRaises(fixture.FixtureError):
                    observer.maintenance_snapshot()
        with mock.patch.object(observer, "json", return_value=good):
            self.assertEqual(observer.maintenance_snapshot(), good)

    def test_stop_writer_unknown_create_never_erases_custody(self):
        owner = object.__new__(fixture.FixtureOwner)
        owner.stop_writer, owner.stop_writer_sent = None, True
        owner.command = mock.Mock()
        with self.assertRaisesRegex(fixture.FixtureError, "FIXTURE_UNKNOWN_IDENTITY"):
            owner.dispose_stop_writer()
        owner.command.assert_not_called()
        self.assertTrue(owner.stop_writer_sent)

    def test_stop_writer_replaced_generation_is_not_stopped_or_removed(self):
        owner = object.__new__(fixture.FixtureOwner)
        cid, image = "a"*64, "sha256:"+"b"*64
        owner.stop_writer, owner.stop_writer_sent = (cid, {"old": True}, None), True
        owner.context = types.SimpleNamespace(images={"paper-scheduler": image})
        owner.project = "owned"
        owner.docker = mock.Mock()
        owner.docker.object.return_value = {"Image": image, "Config": {"Labels": {
            "com.docker.compose.project": "owned", "com.docker.compose.service": "paper-fixture-db-writer"}}}
        owner.command = mock.Mock()
        with mock.patch.object(fixture.ops, "_container_generation", return_value={"replacement": True}):
            with self.assertRaisesRegex(fixture.FixtureError, "FIXTURE_UNKNOWN_IDENTITY"):
                owner.dispose_stop_writer()
        owner.command.assert_not_called()
        self.assertEqual(owner.stop_writer[0], cid)

    def test_stop_writer_cleanup_reaps_before_remove_and_observes_backend_absence(self):
        owner = object.__new__(fixture.FixtureOwner)
        cid, image = "a"*64, "sha256:"+"b"*64
        child = mock.Mock()
        generation = {"captured": True}
        owner.stop_writer, owner.stop_writer_sent = (cid, generation, child), True
        owner.context = types.SimpleNamespace(images={"paper-scheduler": image})
        owner.project = "owned"
        value = {"Image": image, "State": {"Running": False}, "Config": {"Labels": {
            "com.docker.compose.project": "owned", "com.docker.compose.service": "paper-fixture-db-writer"}}}
        owner.docker, owner.observer, owner.deadline = mock.Mock(), mock.Mock(), mock.Mock()
        owner.docker.object.return_value = value
        owner.deadline.seconds.return_value = 1
        events = []
        child.wait.side_effect = lambda **kw: events.append("reap")
        owner.observer.absent.side_effect = lambda **kw: events.append("backend-absent")
        owner.command = mock.Mock(side_effect=lambda *a, **kw: events.append(a[0][0]))
        owner.output = mock.Mock(return_value="")
        with mock.patch.object(fixture.ops, "_container_generation", return_value=generation), \
                mock.patch.object(fixture.ops, "_terminal_container"):
            owner.dispose_stop_writer()
        self.assertEqual(events, ["reap", "backend-absent", "rm"])
        self.assertIsNone(owner.stop_writer)
        self.assertFalse(owner.stop_writer_sent)

    def observe_p3(self, owner):
        owner.extractions = fixture.P3ExtractionCustody(owner)
        owner.extractions.__enter__()
        self.addCleanup(owner.extractions.__exit__, None, None, None)
        return owner.extractions

    def test_deadline_preserves_real_cleanup_reserve(self):
        now = time.monotonic_ns()
        with self.assertRaises(fixture.FixtureError):
            fixture.Deadline(now + 119_000_000_000)
        deadline = fixture.Deadline(now + 180_000_000_000)
        self.assertGreater(deadline.seconds(100), 50)
        self.assertLessEqual(deadline.seconds(100), 60)
        self.assertEqual(deadline.seconds(10, cleanup=True), 10)

    def test_counter_missing_is_unknown_not_zero(self):
        known = {"sql": 1, "function": 1, "events": 1, "receipts": 1,
                 "oid": 42000, "catalog": 4, "prepared": 0}
        self.assertEqual(fixture.counter_values(known), known)
        for key in known:
            wrong = dict(known)
            wrong.pop(key)
            with self.subTest(key=key), self.assertRaises(fixture.FixtureError):
                fixture.counter_values(wrong)
        for key, value in (("sql", None), ("function", True), ("prepared", 1), ("catalog", 0)):
            with self.subTest(key=key), self.assertRaises(fixture.FixtureError):
                fixture.counter_values({**known, key: value})

    def test_duplicate_delta_and_reset_are_distinct(self):
        known = {"sql": 1, "function": 1, "events": 1, "receipts": 1,
                 "oid": 42000, "catalog": 4, "prepared": 0}
        self.assertEqual(fixture.counter_delta(known, dict(known)),
                         {"sql": 0, "function": 0, "events": 0, "receipts": 0})
        for after in ({**known, "sql": 0}, {**known, "oid": 42}):
            with self.assertRaises(fixture.FixtureError):
                fixture.counter_delta(known, after)

    def test_seed_uses_existing_contract_and_daily_fixture_literals(self):
        root = fixture.ROOT / "tests/integration/migration-contract/tests"
        policy = fixture.existing_json_macro(root / "migration_contract.rs", "daily_0060_policy")
        contract = fixture.existing_json_macro(root / "paper_0067.rs", "financial_contract")
        self.assertEqual(len(policy["instruments"]), 11)
        self.assertEqual(contract["cost_profile_id"], "KRX_ETF_DEFAULT")
        self.assertEqual(contract["cost_profile_version"], 1)
        self.assertEqual(contract["contract_sha256"],
                         "sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0")

    def test_observer_decodes_entry_and_exit_from_kernel_phase_not_parity(self):
        record = bytearray(80)
        record[0] = 1
        struct.pack_into("<I", record, 4, 0xC000003E)
        struct.pack_into("<Q", record, 24, 74)
        struct.pack_into("<6Q", record, 32, 9, 0, 0, 0, 0, 0)
        self.assertEqual(faults.syscall_record(bytes(record)), ("entry", 74, (9, 0, 0, 0, 0, 0)))
        record[0] = 2
        struct.pack_into("<q", record, 24, 0)
        record[32] = 0
        self.assertEqual(faults.syscall_record(bytes(record[:33])), ("exit", 0, False))
        struct.pack_into("<I", record, 4, 0)
        with self.assertRaises(faults.FaultError):
            faults.syscall_record(bytes(record))

    def test_observer_constructor_cannot_select_self_or_short_cid(self):
        for cid, pid in (("short", 123), ("a" * 64, os.getpid()), ("a" * 64, 1)):
            with self.assertRaises(faults.FaultError):
                faults.FaultObserver(cid, pid, 1, (1, 2), time.monotonic_ns() + 1_000_000_000)

    def test_detach_bookkeeping_never_sends_ptrace_kill(self):
        observer = faults.FaultObserver("a" * 64, os.getpid() + 100000, 1, (1, 2),
                                       time.monotonic_ns() + 1_000_000_000)
        observer.tids = {101, 102}
        observer.stopped = {101, 102}
        with mock.patch.object(observer, "_stop_all"), mock.patch.object(observer, "_ptrace") as ptrace:
            observer.close()
        self.assertTrue(observer.closed)
        self.assertEqual(observer.tids, set())
        self.assertEqual({call.args[0] for call in ptrace.call_args_list}, {observer.DETACH})

    def test_detach_failure_is_not_a_closed_observer(self):
        observer = faults.FaultObserver("a" * 64, os.getpid() + 100000, 1, (1, 2),
                                       time.monotonic_ns() + 1_000_000_000)
        observer.tids = {101}
        observer.stopped = {101}
        with mock.patch.object(observer, "_stop_all"), \
                mock.patch.object(observer, "_ptrace", side_effect=faults.FaultError("FAULT_PTRACE_1")), \
                mock.patch.object(pathlib.Path, "exists", return_value=True):
            with self.assertRaises(faults.FaultError):
                observer.close()
        self.assertFalse(observer.closed)
        self.assertEqual(observer.tids, {101})

    def test_result_fsync_token_requires_actual_content_hash(self):
        value = {"schema": "owner-beta-paper-action-v1", "action": "BEGIN_OPERATION", "outcome": "OK",
                 "control_epoch": 1, "account_id": "11111111-1111-4111-8111-111111111111",
                 "data": {"generation": "22222222-2222-4222-8222-222222222222",
                          "operation_event_id": "33333333-3333-4333-8333-333333333333",
                          "intent_sha256": "sha256:" + "a" * 64, "fenced_epoch": 1}}
        data = fixture.canonical(value)
        digest = fixture.ops._digest(data)
        self.assertEqual(faults.result_identity(data, digest[7:] + ".json"), (value, digest))
        with self.assertRaises(faults.FaultError):
            faults.result_identity(data, "f" * 64 + ".json")

    def test_fsync_result_parser_rejects_partial_or_wrong_epoch(self):
        for value in ({"schema": "owner-beta-paper-action-v1", "action": "BEGIN_OPERATION", "outcome": "OK"},
                      {"schema": "owner-beta-paper-action-v1", "action": "BEGIN_OPERATION", "outcome": "OK",
                       "control_epoch": True, "account_id": None, "data": {}}):
            data = fixture.canonical(value)
            with self.assertRaises(faults.FaultError):
                faults.result_identity(data, fixture.ops._digest(data)[7:] + ".json")

    def test_resume_detaches_and_holds_original_fd_until_supervisor_proof(self):
        observer = faults.FaultObserver("a" * 64, os.getpid() + 100000, 1, (1, 2),
                                       time.monotonic_ns() + 1_000_000_000)
        with tempfile.TemporaryFile() as original:
            fd = os.dup(original.fileno())
            observer.tids = observer.stopped = {101, 102}
            observer.token = types.SimpleNamespace(fd=fd)
            with mock.patch.object(observer, "_ptrace") as ptrace:
                observer.resume()
                self.assertEqual([c.args[0] for c in ptrace.call_args_list],
                                 [observer.DETACH, observer.DETACH])
                self.assertIsNone(observer.token)
                os.fstat(fd)
                with self.assertRaises(faults.FaultError):
                    observer.release_results()
                observer.close()
                os.fstat(fd)
                observer.release_results()
            with self.assertRaises(OSError):
                os.fstat(fd)

    def test_manifest_handoff_requires_exact_real_thirteen_rows(self):
        commit = "a" * 40
        rows = ["image|%s|lagrange-station-%s:%s|sha256:%064x|%s" %
                (service, service, commit, index + 1, commit)
                for index, service in enumerate(fixture.ops.daily.policy.MANIFEST_V3_SERVICES)]
        lines = ["LAGRANGE_RELEASE_MANIFEST_V3", "commit|" + commit, *rows]
        fixture.validate_v3_bytes(("\n".join(lines) + "\n").encode(), commit)
        invalid = [lines[:-1], [*lines, "image|postgres|postgres:latest|sha256:" + "f" * 64 + "|" + commit],
                   ["LAGRANGE_RELEASE_MANIFEST_V2", *lines[1:]],
                   [*lines[:2], rows[1], *rows[1:]],
                   [*lines[:-1], lines[-1].replace("sha256:", "sha512:")]]
        for candidate in invalid:
            with self.subTest(candidate=candidate[0]), self.assertRaises(fixture.FixtureError):
                fixture.validate_v3_bytes(("\n".join(candidate) + "\n").encode(), commit)

    def test_statistics_waits_for_backend_and_real_publication(self):
        observer = fixture.PgObserver(types.SimpleNamespace(deadline=types.SimpleNamespace(seconds=lambda cap: 1)))
        known = {"sql": 1, "function": 1, "events": 1, "receipts": 1,
                 "oid": 42000, "catalog": 4, "prepared": 0}
        with mock.patch.object(observer, "absent", side_effect=[fixture.FixtureError("FIXTURE_BACKEND_ALIVE"),
                                                               None, None]), \
                mock.patch.object(observer, "snapshot", side_effect=[{**known, "function": 0}, known]), \
                mock.patch.object(fixture.time, "sleep"):
            self.assertEqual(observer.settle({"sql": 1, "function": 1, "events": 1, "receipts": 1}), known)

    def test_statistics_excess_or_missing_never_becomes_zero_success(self):
        observer = fixture.PgObserver(types.SimpleNamespace(deadline=types.SimpleNamespace(seconds=lambda cap: 0)))
        with mock.patch.object(observer, "absent"), \
                mock.patch.object(observer, "snapshot", side_effect=fixture.FixtureError("FIXTURE_COUNTER_UNKNOWN")):
            with self.assertRaisesRegex(fixture.FixtureError, "FIXTURE_STATS_UNPROVEN"):
                observer.settle({"sql": 0})
        with mock.patch.object(observer, "absent"), mock.patch.object(observer, "snapshot", return_value={"sql": 2}):
            with self.assertRaisesRegex(fixture.FixtureError, "FIXTURE_EXCESS_CALLS"):
                observer.settle({"sql": 1})

    def test_unknown_create_retains_lock_inputs_and_custody(self):
        owner = fixture.FixtureOwner.__new__(fixture.FixtureOwner)
        owner.stop_writer, owner.stop_writer_sent = None, False
        owner.network_sent, owner.network = True, None
        owner.pg_sent, owner.pg = False, None
        with tempfile.TemporaryDirectory() as root:
            secret = pathlib.Path(root, "secret")
            secret.write_bytes(b"unit-only-sentinel")
            fd = os.open(secret, os.O_RDONLY)
            try:
                with mock.patch.object(owner, "resolve_helpers") as resolve:
                    close = mock.Mock()
                    with self.assertRaisesRegex(fixture.FixtureError, "FIXTURE_NETWORK_CREATE_UNKNOWN"):
                        owner.dispose(close)
                    close.assert_not_called()
                    resolve.assert_not_called()
                    self.assertEqual(secret.read_bytes(), b"unit-only-sentinel")
                    os.fstat(fd)
            finally:
                os.close(fd)

    def test_disposition_failure_does_not_close_originals(self):
        owner = fixture.FixtureOwner.__new__(fixture.FixtureOwner)
        owner.stop_writer, owner.stop_writer_sent = None, False
        owner.network_sent = owner.pg_sent = False
        close = mock.Mock()
        with mock.patch.object(owner, "resolve_helpers", side_effect=fixture.FixtureError("FIXTURE_BACKEND_ALIVE")):
            with self.assertRaises(fixture.FixtureError):
                owner.dispose(close)
        close.assert_not_called()

    def test_disposal_erases_only_after_proof_and_keeps_lock_until_last(self):
        owner = fixture.FixtureOwner.__new__(fixture.FixtureOwner)
        owner.stop_writer, owner.stop_writer_sent = None, False
        owner.network_sent = owner.pg_sent = False
        owner.network = owner.pg = owner.observer = None
        owner.contexts = []
        owner.cleanup = {"containers": "PENDING", "inputs": "RETAINED"}
        self.observe_p3(owner)
        with tempfile.TemporaryDirectory() as parent:
            owner.root = pathlib.Path(parent, "owned")
            owner.root.mkdir()
            owner.root_id = faults.directory_identity(owner.root)
            owner.lock_fd = os.open(owner.root / "lock", os.O_CREAT | os.O_RDWR, 0o600)
            fd = owner.lock_fd
            called = []
            def custody():
                self.assertEqual(called, ["PROVED"])
                self.assertTrue(owner.root.exists())
                os.fstat(fd)
                called.append("CUSTODY_CLOSED")
            with mock.patch.object(owner, "resolve_helpers", side_effect=lambda: called.append("PROVED")):
                owner.dispose(custody)
            self.assertEqual(called, ["PROVED", "CUSTODY_CLOSED"])
            self.assertFalse(owner.root.exists())
            self.assertEqual(owner.cleanup["inputs"], "REMOVED_AFTER_DISPOSITION")
            with self.assertRaises(OSError):
                os.fstat(fd)

    def test_cleanup_rejects_foreign_nonce_before_stop_or_remove(self):
        owner = fixture.FixtureOwner.__new__(fixture.FixtureOwner)
        owner.stop_writer, owner.stop_writer_sent = None, False
        owner.tracees, owner.blockers, owner.contexts, owner.threads = [], [], [], []
        owner.cancel_event = mock.Mock()
        owner.root = pathlib.Path("/unit-private-root")
        cid = "a" * 64
        owner.helpers = {cid: "b" * 32}
        owner.docker = mock.Mock()
        owner.docker.object.return_value = {"Id": cid, "Config": {"Labels": {fixture.ops.CONTAINER_LABEL: "c" * 32}}}
        with mock.patch.object(owner, "output", side_effect=["", cid]), mock.patch.object(owner, "command") as command:
            with self.assertRaisesRegex(fixture.FixtureError, "FIXTURE_FOREIGN_CLEANUP"):
                owner.resolve_helpers()
        command.assert_not_called()

    def test_failed_namespace_entry_closes_owned_descriptor_without_mount(self):
        policy = fixture.PrivatePolicy(pathlib.Path("/unit-source"))
        policy.libc = mock.Mock()
        policy.libc.unshare.return_value = -1
        with tempfile.TemporaryFile() as resource:
            fd = os.dup(resource.fileno())
            with mock.patch.object(fixture.os, "geteuid", return_value=0), \
                    mock.patch.object(fixture.os, "open", return_value=fd), \
                    mock.patch.object(fixture.threading, "active_count", return_value=1):
                with self.assertRaisesRegex(fixture.FixtureError, "FIXTURE_UNSHARE"):
                    policy.__enter__()
            with self.assertRaises(OSError):
                os.fstat(fd)
            policy.libc.mount.assert_not_called()
            policy.libc.setns.assert_not_called()

    def test_unavailable_cli_is_not_run_and_has_no_pass_receipt(self):
        output = io.StringIO()
        with mock.patch.object(fixture.os, "geteuid", return_value=12345), \
                mock.patch.object(fixture, "run") as run, contextlib.redirect_stdout(output):
            self.assertEqual(fixture.main([]), 77)
        run.assert_not_called()
        self.assertEqual(output.getvalue(), "PAPER_GENUINE_FIXTURE_NOT_RUN:HOSTED_ROOT_PG_FIXTURE_REQUIRED\n")

    def test_restore_unknown_function_statistic_is_preserved(self):
        owner = types.SimpleNamespace(docker=mock.Mock(), network="a" * 64)
        owner.docker.object.return_value = {"Containers": {}}
        observer = fixture.PgObserver(owner)
        actual = {"sql": 0, "function": None, "events": 0, "receipts": 0,
                  "oid": 42000, "catalog": 4, "prepared": 0}
        with mock.patch.object(observer, "_statistics", return_value=actual):
            counts = observer.rejection_counts()
            self.assertEqual(counts["function"], "UNKNOWN")
            self.assertEqual(counts["sql"], 0)
            with self.assertRaises(fixture.FixtureError):
                observer.snapshot()
        self.assertIsNone(actual["function"])

    def test_restore_rejection_counter_cannot_hide_a_helper(self):
        owner = types.SimpleNamespace(docker=mock.Mock(), network="a" * 64)
        owner.docker.object.side_effect = [{"Containers": {"b" * 64: {}}},
                                          {"Config": {"Labels": {fixture.ops.CONTAINER_LABEL: "c" * 32}}}]
        observer = fixture.PgObserver(owner)
        with mock.patch.object(observer, "_statistics", return_value={
                "sql": 0, "function": None, "events": 0, "receipts": 0,
                "oid": 42000, "catalog": 4, "prepared": 0}):
            with self.assertRaisesRegex(fixture.FixtureError, "FIXTURE_RESTORE_HELPER_CREATED"):
                observer.rejection_counts()

    def test_cancel_owned_client_kills_and_reaps_child_without_helper_claim(self):
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(20)"],
                                 stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                 stderr=subprocess.DEVNULL)
        try:
            deadline = fixture.Deadline(time.monotonic_ns() + 180_000_000_000)
            fixture.cancel_owned_attach(child, deadline)
            self.assertIsNotNone(child.poll())
            self.assertLess(child.returncode, 0)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=5)

    def test_deadline_alarm_reserves_cleanup_and_restores_previous_handler(self):
        deadline = fixture.Deadline(time.monotonic_ns() + 180_000_000_000)
        with mock.patch.object(fixture.signal, "getitimer", return_value=(0.0, 0.0)), \
                mock.patch.object(fixture.signal, "signal", return_value="previous") as handler, \
                mock.patch.object(fixture.signal, "setitimer") as timer:
            with fixture.bounded_run(deadline):
                action_seconds = timer.call_args.args[1]
                self.assertGreater(action_seconds, 50)
                self.assertLessEqual(action_seconds, 60)
                deadline.cleanup_phase()
                self.assertGreater(timer.call_args.args[1], 170)
            self.assertEqual(timer.call_args.args[1], 0)
            self.assertEqual(handler.call_args.args, (fixture.signal.SIGALRM, "previous"))
            self.assertFalse(deadline.alarm_installed)

    def test_unknown_network_removal_keeps_original_fds_and_inputs(self):
        owner = fixture.FixtureOwner.__new__(fixture.FixtureOwner)
        owner.stop_writer, owner.stop_writer_sent = None, False
        owner.network_sent, owner.pg_sent = True, False
        owner.network, owner.pg = "a" * 64, None
        owner.project = "paper-operations-fixture-unit"
        owner.contexts = []
        owner.cleanup = {"containers": "PENDING", "inputs": "RETAINED"}
        self.observe_p3(owner)
        owner.docker = mock.Mock()
        owner.docker.object.return_value = {"Id": owner.network, "Internal": True,
            "Labels": {"com.docker.compose.project": owner.project,
                       "com.docker.compose.network": "owner-beta-db"}}
        with tempfile.TemporaryDirectory() as root:
            owner.root = pathlib.Path(root)
            owner.root_id = faults.directory_identity(root)
            owner.lock_fd = os.open(owner.root / "lock", os.O_CREAT | os.O_RDWR, 0o600)
            close = mock.Mock()
            try:
                with mock.patch.object(owner, "resolve_helpers"), \
                        mock.patch.object(owner, "command", side_effect=fixture.FixtureError("FIXTURE_DOCKER_COMMAND")):
                    with self.assertRaises(fixture.FixtureError):
                        owner.dispose(close)
                close.assert_not_called()
                self.assertTrue(owner.root.exists())
                os.fstat(owner.lock_fd)
                self.assertEqual(owner.cleanup["inputs"], "RETAINED")
            finally:
                os.close(owner.lock_fd)

    def test_configure_hands_the_real_app_returned_id_to_public_configure(self):
        returned = "abcdef01-2345-4234-8234-0123456789ab"
        owner = fixture.FixtureOwner.__new__(fixture.FixtureOwner)
        owner.stop_writer, owner.stop_writer_sent = None, False
        owner.release = fixture.ROOT
        owner.observer = mock.Mock()
        owner.observer.json.side_effect = [returned, {"outcome": "OK", "control_epoch": 0,
                                                     "account_id": "12345678-1234-4234-8234-123456789abc",
                                                     "data": {"state": "OFF"}}]
        owner.configure()
        insert, configure = owner.observer.json.call_args_list
        self.assertEqual(insert.kwargs, {"role": "app"})
        columns = insert.args[0].split("INSERT INTO user_strategy_configs(", 1)[1].split(")", 1)[0].split(",")
        self.assertEqual(columns, ["owner_user_id", "strategy_id", "strategy_version", "config_json", "daily_enabled"])
        self.assertIn("RETURNING to_json(id)", insert.args[0])
        self.assertIn("SET LOCAL app.actor_user_id=", insert.args[0])
        self.assertEqual(configure.kwargs, {"role": "migration_owner"})
        self.assertIn("configure_owner_beta_paper_v1('" + returned + "'", configure.args[0])
        self.assertNotIn(returned, insert.args[0])
        self.assertEqual(owner.account, "12345678-1234-4234-8234-123456789abc")

    def test_invalid_app_returned_config_id_never_reaches_configure(self):
        for returned in (None, 1, True, {}, b"abcdef01-2345-4234-8234-0123456789ab",
                         "00000000-0000-0000-0000-000000000000",
                         "ABCDEF01-2345-4234-8234-0123456789AB",
                         " abcdef01-2345-4234-8234-0123456789ab", "';DROP TABLE users;--"):
            with self.subTest(returned=returned):
                owner = fixture.FixtureOwner.__new__(fixture.FixtureOwner)
                owner.stop_writer, owner.stop_writer_sent = None, False
                owner.release = fixture.ROOT
                owner.observer = mock.Mock()
                owner.observer.json.return_value = returned
                with self.assertRaisesRegex(fixture.FixtureError, "FIXTURE_CONFIG_ID"):
                    owner.configure()
                self.assertEqual(owner.observer.json.call_count, 1)
                self.assertEqual(owner.observer.json.call_args.kwargs, {"role": "app"})
                self.assertFalse(hasattr(owner, "account"))

    @contextlib.contextmanager
    def p3_resources(self, results):
        """Fake command observations only; no Root/Docker authority is created."""
        owner = fixture.FixtureOwner.__new__(fixture.FixtureOwner)
        owner.stop_writer, owner.stop_writer_sent = None, False
        commit = "a" * 40
        owner.manifest = ("LAGRANGE_RELEASE_MANIFEST_V3\ncommit|" + commit + "\n" + "".join(
            "image|%s|lagrange-station-%s:%s|sha256:%064x|%s\n" % (service, service, commit, i + 1, commit)
            for i, service in enumerate(fixture.ops.daily.policy.MANIFEST_V3_SERVICES))).encode()
        image, cid = "sha256:" + format(3, "064x"), "b" * 64
        generation = {"Id": cid, "Created": "2026-11-06T18:00:00.000000001Z", "Image": image,
                      "RestartCount": 0, "Config": {"Image": image},
                      "State": {"Status": "created", "Running": False, "Pid": 0}}
        owner.network_sent = owner.pg_sent = False
        owner.network = owner.pg = owner.observer = None
        owner.contexts = []
        owner.cleanup = {"containers": "PENDING", "backends": "NOT_STARTED", "inputs": "RETAINED",
                         "extractions": "NOT_STARTED"}
        owner.docker = mock.Mock()
        owner.docker.object.return_value = generation
        owner.output = mock.Mock(return_value=cid)
        owner.command = mock.Mock()
        real_run = subprocess.run
        commands = types.SimpleNamespace(run=mock.Mock(side_effect=results),
                                         PIPE=subprocess.PIPE, DEVNULL=subprocess.DEVNULL)
        with tempfile.TemporaryDirectory() as parent:
            owner.root = pathlib.Path(parent, "owned")
            owner.root.mkdir()
            owner.root_id = faults.directory_identity(owner.root)
            owner.lock_fd = os.open(owner.root / "lock", os.O_CREAT | os.O_RDWR, 0o600)
            (owner.root / "secret").write_bytes(b"unit-only-custody")
            try:
                with mock.patch.object(fixture.ops.release, "subprocess", commands):
                    with fixture.P3ExtractionCustody(owner) as custody:
                        owner.extractions = custody
                        with mock.patch.object(owner, "resolve_helpers"):
                            yield owner, custody, commands, image, cid, generation
                    self.assertIs(fixture.ops.release.subprocess, commands)
                self.assertIs(subprocess.run, real_run)
            finally:
                if owner.lock_fd is not None:
                    os.close(owner.lock_fd)

    def test_pending_p3_create_is_recorded_before_delegate_and_blocks_erasure(self):
        def interrupted(*args, **kwargs):
            self.assertEqual(custody.records[0]["state"], "CREATE_PENDING")
            self.assertEqual(owner.cleanup["extractions"], "UNKNOWN")
            raise fixture.FixtureError("FIXTURE_DEADLINE")
        with self.p3_resources(interrupted) as (owner, custody, commands, image, cid, generation):
            with self.assertRaisesRegex(fixture.FixtureError, "FIXTURE_DEADLINE"):
                fixture.ops.release._extract_image_elf(image, "/usr/local/bin/api-server")
            close = mock.Mock()
            with self.assertRaisesRegex(fixture.FixtureError, "FIXTURE_P3_CREATE_UNKNOWN"):
                owner.dispose(close)
            close.assert_not_called()
            owner.output.assert_not_called()
            owner.command.assert_not_called()
            self.assertTrue((owner.root / "secret").is_file())
            os.fstat(owner.lock_fd)
            self.assertEqual(owner.cleanup["extractions"], "UNKNOWN")
            self.assertNotEqual(owner.cleanup["containers"], "REMOVED")

    def test_failed_real_p3_extractors_keep_captured_generation_and_cleanup_unknown(self):
        for kind in ("elf", "provenance"):
            with self.subTest(kind=kind):
                results = [subprocess.CompletedProcess([], 0, b"b" * 64 + b"\n"),
                           subprocess.CompletedProcess([], 1, b""), subprocess.CompletedProcess([], 1)]
                with self.p3_resources(results) as (owner, custody, commands, image, cid, generation):
                    with self.assertRaises(fixture.ops.release.PaperReleaseError) as error:
                        if kind == "elf":
                            fixture.ops.release._extract_image_elf(image, "/usr/local/bin/api-server")
                        else:
                            fixture.ops.release._extract_image_provenance(image, ("api-server",))
                    self.assertEqual(error.exception.code, "PAPER_RELEASE_IMAGE_CLEANUP")
                    record = custody.records[0]
                    self.assertEqual((record["cid"], record["created"], record["image"]),
                                     (cid, generation["Created"], image))
                    close = mock.Mock()
                    with self.assertRaisesRegex(fixture.FixtureError, "FIXTURE_P3_CLEANUP_UNKNOWN"):
                        owner.dispose(close)
                    close.assert_not_called()
                    owner.command.assert_not_called()
                    os.fstat(owner.lock_fd)
                    self.assertTrue((owner.root / "secret").exists())
                    self.assertEqual(owner.cleanup["extractions"], "UNKNOWN")
                    self.assertNotEqual(owner.cleanup["containers"], "REMOVED")

    def test_captured_p3_cid_can_discharge_only_after_actual_terminal_absence_observation(self):
        results = [subprocess.CompletedProcess([], 0, b"b" * 64 + b"\n"),
                   subprocess.CompletedProcess([], 1, b""), subprocess.CompletedProcess([], 1)]
        with self.p3_resources(results) as (owner, custody, commands, image, cid, generation):
            with self.assertRaises(fixture.ops.release.PaperReleaseError):
                fixture.ops.release._extract_image_elf(image, "/usr/local/bin/api-server")
            with self.assertRaises(fixture.FixtureError):
                owner.dispose(mock.Mock())
            self.assertNotEqual(owner.cleanup["containers"], "REMOVED")
            owner.output.return_value = ""  # Explicit later fake actual-ID absence observation.
            fd = owner.lock_fd
            close = mock.Mock()
            owner.dispose(close)
            close.assert_called_once_with()
            self.assertEqual(owner.cleanup["extractions"], "ABSENT")
            self.assertFalse(owner.root.exists())
            with self.assertRaises(OSError):
                os.fstat(fd)
            owner.command.assert_not_called()  # Never delete by image/name or guess a missing CID.
            self.assertTrue(all(call.args[0][4] == "id=" + cid for call in owner.output.call_args_list))

    def test_p3_facade_preserves_exact_arguments_result_and_shared_subprocess(self):
        created = subprocess.CompletedProcess([], 0, b"b" * 64 + b"\n")
        removed = subprocess.CompletedProcess([], 0)
        with self.p3_resources([created, removed]) as (owner, custody, commands, image, cid, generation):
            argv, env = ["docker", "create", image], {"LANG": "C"}
            kwargs = {"stdin": subprocess.DEVNULL, "stdout": subprocess.PIPE,
                      "stderr": subprocess.DEVNULL, "check": False, "env": env}
            self.assertIs(custody.run(argv, **kwargs), created)
            call = commands.run.call_args
            self.assertIs(call.args[0], argv)
            self.assertIs(call.kwargs["env"], env)
            self.assertEqual(call.kwargs, kwargs)
            owner.output.return_value = ""
            self.assertIs(custody.run(["docker", "rm", "-f", cid], check=False), removed)
            custody.reconcile()
            self.assertEqual(owner.cleanup["extractions"], "ABSENT")
            self.assertIs(fixture.ops._PaperContext.__init__.__globals__["release"], fixture.ops.release)
            for name in ("extract", "extract_receipts"):
                function = inspect.signature(fixture.ops.release._actual_elfs).parameters[name].default
                self.assertIs(function.__globals__["subprocess"], custody)
            self.assertIs(fixture.subprocess, subprocess)

    def test_bad_p3_cid_cannot_be_reconstructed_from_image_or_native_labels(self):
        result = subprocess.CompletedProcess([], 0, b"not-a-container-id\n")
        with self.p3_resources([result]) as (owner, custody, commands, image, cid, generation):
            self.assertIs(custody.run(["docker", "create", image]), result)
            with self.assertRaisesRegex(fixture.FixtureError, "FIXTURE_P3_CREATE_UNKNOWN"):
                owner.dispose(mock.Mock())
            owner.output.assert_not_called()
            owner.docker.object.assert_not_called()
            owner.command.assert_not_called()
            os.fstat(owner.lock_fd)
            self.assertEqual(owner.cleanup["extractions"], "UNKNOWN")

    def test_p3_generation_change_blocks_cleanup_even_after_successful_rm_result(self):
        results = [subprocess.CompletedProcess([], 0, b"b" * 64 + b"\n"), subprocess.CompletedProcess([], 0)]
        with self.p3_resources(results) as (owner, custody, commands, image, cid, generation):
            custody.run(["docker", "create", image])
            owner.docker.object.return_value = {**generation, "Created": "2026-11-06T18:01:00Z"}
            custody.run(["docker", "rm", "-f", cid])
            with self.assertRaisesRegex(fixture.FixtureError, "FIXTURE_P3_IDENTITY_UNKNOWN"):
                owner.dispose(mock.Mock())
            self.assertNotEqual(owner.cleanup["containers"], "REMOVED")
            self.assertEqual(owner.cleanup["extractions"], "UNKNOWN")
            os.fstat(owner.lock_fd)

    def test_failed_p3_generation_capture_cannot_turn_absence_into_owned_proof(self):
        result = subprocess.CompletedProcess([], 0, b"b" * 64 + b"\n")
        with self.p3_resources([result]) as (owner, custody, commands, image, cid, generation):
            owner.docker.object.return_value = {**generation, "Image": "sha256:" + "f" * 64}
            custody.run(["docker", "create", image])
            owner.output.return_value = ""
            with self.assertRaisesRegex(fixture.FixtureError, "FIXTURE_P3_CREATE_UNKNOWN"):
                owner.dispose(mock.Mock())
            owner.output.assert_not_called()
            self.assertEqual(owner.cleanup["extractions"], "UNKNOWN")
            os.fstat(owner.lock_fd)

    def test_small_owned_attach_child_is_reaped_without_database_claim(self):
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(20)"],
                                 stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                 stderr=subprocess.DEVNULL)
        try:
            fixture.ops._NativeInvocation._reap_child(child)
            self.assertIsNotNone(child.poll())
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=5)
            child.stdout.close()

    def test_main_emits_closed_error_diagnostic_after_generic_marker(self):
        output = io.StringIO()
        args = ["--source-root", "/fixture/source", "--commit", "a" * 40,
                "--manifest-file", "/fixture/manifest", "--deadline-monotonic-ns", "1"]
        def fail_at_existing_guard(*_args):
            fixture.Deadline(time.monotonic_ns() + 1)
        with (mock.patch.object(fixture.os, "geteuid", return_value=0),
              mock.patch.object(fixture, "run", side_effect=fail_at_existing_guard),
              contextlib.redirect_stdout(output)):
            status = fixture.main(args)
        self.assertEqual(status, 1)
        lines = output.getvalue().splitlines()
        self.assertEqual(lines[0], "PAPER_GENUINE_FIXTURE_FAILED:OWNED_CASE_UNPROVEN")
        self.assertEqual(len(lines), 2)
        records = [line for line in lines if line.startswith("PAPER_GENUINE_FIXTURE_DIAGNOSTIC=")]
        self.assertEqual(len(records), 1, "the generic marker currently discards the closed failure context")
        value = json.loads(records[0].split("=", 1)[1])
        self.assertEqual(set(value), {"kind", "code", "line", "caller_line", "case", "errno"})
        self.assertEqual((value["kind"], value["code"], value["case"], value["errno"]),
                         ("FixtureError", "FIXTURE_CLEANUP_RESERVE", "UNKNOWN", 0))
        self.assertEqual(records[0].split("=", 1)[1], fixture.canonical(value).decode())
        self.assertGreater(value["line"], 0)
        self.assertGreater(value["caller_line"], 0)
        require_line = fixture.require.__code__.co_firstlineno
        deadline_line = fixture.Deadline.__init__.__code__.co_firstlineno
        self.assertLessEqual(require_line, value["line"])
        self.assertLessEqual(value["line"], require_line + 3)
        self.assertLessEqual(deadline_line, value["caller_line"])
        self.assertLessEqual(value["caller_line"], deadline_line + 10)

        with contextlib.redirect_stdout(output := io.StringIO()), \
             mock.patch.object(fixture.os, "geteuid", return_value=1000):
            status = fixture.main(args)
        self.assertEqual(status, 77)
        self.assertEqual(output.getvalue(),
                         "PAPER_GENUINE_FIXTURE_NOT_RUN:HOSTED_ROOT_PG_FIXTURE_REQUIRED\n")

        with contextlib.redirect_stdout(output := io.StringIO()), \
             mock.patch.object(fixture.os, "geteuid", return_value=0), \
             mock.patch.object(fixture, "run", side_effect=fixture.FixtureNotRun("private reason")):
            status = fixture.main(args)
        self.assertEqual(status, 77)
        self.assertEqual(output.getvalue(),
                         "PAPER_GENUINE_FIXTURE_NOT_RUN:HOSTED_ROOT_PG_FIXTURE_REQUIRED\n")

        result = {"status": "PASS", "marker": "existing-success-json"}
        with contextlib.redirect_stdout(output := io.StringIO()), \
             mock.patch.object(fixture.os, "geteuid", return_value=0), \
             mock.patch.object(fixture, "run", return_value=result) as run_mock:
            status = fixture.main(args)
        self.assertEqual(status, 0)
        self.assertEqual(output.getvalue(), fixture.canonical(result).decode() + "\n")
        run_mock.assert_called_once_with("/fixture/source", "a" * 40,
                                         "/fixture/manifest", 1)

    def test_failure_projection_is_closed_and_private(self):
        import ast

        tree = ast.parse(PATH.read_text())
        source_codes = set()
        for node in ast.walk(tree):
            if (isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
                    and node.func.id == "require" and len(node.args) >= 2
                    and isinstance(node.args[1], ast.Constant)
                    and type(node.args[1].value) is str
                    and node.args[1].value.startswith("FIXTURE_")):
                source_codes.add(node.args[1].value)
            if (isinstance(node, ast.Raise) and isinstance(node.exc, ast.Call)
                    and isinstance(node.exc.func, ast.Name) and node.exc.func.id == "FixtureError"
                    and node.exc.args and isinstance(node.exc.args[0], ast.Constant)
                    and type(node.exc.args[0].value) is str
                    and node.exc.args[0].value.startswith("FIXTURE_")):
                source_codes.add(node.exc.args[0].value)
        source_codes.discard("FIXTURE_SQL_")
        self.assertEqual(fixture._CLOSED_FIXTURE_CODES, frozenset(source_codes))
        self.assertEqual(len(source_codes), 119)
        self.assertNotIn("FIXTURE_SQL_", fixture._CLOSED_FIXTURE_CODES)
        self.assertNotIn("FIXTURE_SQL_57014", fixture._CLOSED_FIXTURE_CODES)

        secret = "DIAGNOSTIC-SECRET-SENTINEL-7f319"
        expected_keys = {"kind", "code", "line", "caller_line", "case", "errno"}
        def check_closed(error, *, kind=None, code="UNKNOWN", errno=0):
            projected = fixture._fixture_failure_diagnostic(error)
            self.assertEqual(set(projected), expected_keys)
            self.assertEqual(projected["code"], code)
            self.assertEqual(projected["errno"], errno)
            if kind is not None:
                self.assertEqual(projected["kind"], kind)
            self.assertEqual(type(projected["kind"]), str)
            self.assertEqual(type(projected["case"]), str)
            for key in ("line", "caller_line", "errno"):
                self.assertEqual(type(projected[key]), int)
                self.assertGreaterEqual(projected[key], 0)
                self.assertLessEqual(projected[key], 1_000_000)
            encoded = fixture.canonical(projected)
            self.assertNotIn(secret.encode(), encoded)
            return projected, encoded

        free_text = fixture.FixtureError(secret)
        free_text.case = secret
        free_text.code = secret
        free_text.command = secret
        free_text.__cause__ = free_text
        free_text.__context__ = free_text
        check_closed(free_text, kind="FixtureError")
        output = io.StringIO()
        def raise_secret(*_args):
            raise fixture.FixtureError(secret)
        with (mock.patch.object(fixture.os, "geteuid", return_value=0),
              mock.patch.object(fixture, "run", side_effect=raise_secret),
              contextlib.redirect_stdout(output)):
            self.assertEqual(fixture.main(["--source-root", "/fixture", "--commit", "c" * 40,
                                           "--manifest-file", "/manifest",
                                           "--deadline-monotonic-ns", "1"]), 1)
        self.assertNotIn(secret, output.getvalue())
        self.assertEqual(len(output.getvalue().splitlines()), 2)
        for args in ((), ("FIXTURE_DEADLINE", "extra"), ("FIXTURE_SQL_",),
                     ("FIXTURE_SQL_57014",), ("FIXTURE_OWNED_CASE_UNPROVEN",),
                     (type("DiagnosticString", (str,), {})("FIXTURE_DEADLINE"),)):
            malformed = fixture.FixtureError()
            malformed.args = args
            check_closed(malformed, kind="FixtureError")
        malformed = fixture.FixtureError()
        malformed.args = (["FIXTURE_DEADLINE"],)
        check_closed(malformed, kind="FixtureError")

        class PoisonFixtureError(fixture.FixtureError):
            def __getattribute__(self, _name):
                raise AssertionError(secret)
            def __str__(self):
                raise AssertionError(secret)
            def __repr__(self):
                raise AssertionError(secret)

        unknown = fixture._unknown_fixture_diagnostic()
        poisoned = PoisonFixtureError(secret)
        self.assertEqual(fixture._fixture_failure_diagnostic(poisoned), unknown)
        self.assertEqual(fixture._fixture_failure_diagnostic(object()), unknown)
        self.assertEqual(fixture._fixture_failure_diagnostic(True), unknown)
        self.assertEqual(fixture._fixture_failure_diagnostic(RuntimeError(secret)), unknown)
        self.assertEqual(fixture._fixture_failure_diagnostic(Exception(secret))["kind"], "UnexpectedError")

        for raw_errno, expected_errno in ((13, 13), (-1, 0), (4096, 0), (True, 0), (secret, 0)):
            error = OSError()
            error.errno = raw_errno
            check_closed(error, kind="OSError", errno=expected_errno)
        error = OSError(secret)
        foreign = {}
        try:
            exec(compile("raise OSError(13, '" + secret + "')", "foreign-diagnostic-frame.py", "exec"),
                 foreign)
        except OSError as caught:
            error = caught
        positions, encoded = check_closed(error, kind="OSError", errno=13)
        self.assertEqual((positions["line"], positions["caller_line"], positions["case"]),
                         (0, 0, "UNKNOWN"))
        self.assertNotIn(b"foreign-diagnostic-frame.py", encoded)

        def deep_failure(depth):
            if depth:
                return deep_failure(depth - 1)
            raise ValueError(secret)
        try:
            deep_failure(70)
        except ValueError as caught:
            positions, encoded = check_closed(caught, kind="ValueError")
        self.assertEqual((positions["line"], positions["caller_line"], positions["case"]),
                         (0, 0, "UNKNOWN"))
        self.assertNotIn(secret.encode(), encoded)

    def test_case_projection_requires_current_runner_method_frame(self):
        args = ["--source-root", "/fixture/source", "--commit", "b" * 40,
                "--manifest-file", "/fixture/manifest", "--deadline-monotonic-ns", "1"]

        @contextlib.contextmanager
        def stop_before_resources(*_args, **_kwargs):
            raise fixture.FixtureError("FIXTURE_DEADLINE")
            yield None

        def run_case(method, argument=None):
            runner = object.__new__(fixture.FixtureRunner)
            with mock.patch.object(fixture.FixtureRunner, "fixture_case", new=stop_before_resources):
                if method == "positive":
                    runner.positive(argument)
                elif method == "fault":
                    runner.fault(argument)
                else:
                    runner.restore_negative()

        def invoke_main(method, argument=None):
            output = io.StringIO()
            with (mock.patch.object(fixture.os, "geteuid", return_value=0),
                  mock.patch.object(fixture, "run", side_effect=lambda *_args: run_case(method, argument)),
                  contextlib.redirect_stdout(output)):
                status = fixture.main(args)
            self.assertEqual(status, 1)
            self.assertEqual(output.getvalue().splitlines()[0],
                             "PAPER_GENUINE_FIXTURE_FAILED:OWNED_CASE_UNPROVEN")
            record = [line for line in output.getvalue().splitlines()
                      if line.startswith("PAPER_GENUINE_FIXTURE_DIAGNOSTIC=")]
            self.assertEqual(len(record), 1)
            return json.loads(record[0].split("=", 1)[1])

        for purpose in ("STOP", "BACKUP"):
            with self.subTest(purpose=purpose):
                value = invoke_main("positive", purpose)
                self.assertEqual((value["kind"], value["code"], value["case"]),
                                 ("FixtureError", "FIXTURE_DEADLINE", purpose))
        with self.subTest(purpose="restore"):
            value = invoke_main("restore")
            self.assertEqual(value["case"], "RESTORE_UNVERIFIED_ARCHIVE_REJECTED")
        fault_cases = {"installation": "INSTALLATION", "floor": "FLOOR", "stage": "STAGE",
                       "result-delete": "RESULT_DELETE", "result-replace": "RESULT_REPLACE",
                       "attach-cancel": "ATTACH_CANCEL", "cleanup": "CLEANUP"}
        self.assertEqual(fixture._FIXTURE_DIAGNOSTIC_FAULT_CASES, fault_cases)
        for kind, expected in fault_cases.items():
            with self.subTest(kind=kind):
                value = invoke_main("fault", kind)
                self.assertEqual(value["case"], expected)
        for method, argument in (("positive", "OTHER"), ("fault", "cleanup-secret")):
            with self.subTest(method=method, argument=argument):
                self.assertEqual(invoke_main(method, argument)["case"], "UNKNOWN")

        def outside_runner(*_args):
            raise fixture.FixtureError("FIXTURE_DEADLINE")
        output = io.StringIO()
        with (mock.patch.object(fixture.os, "geteuid", return_value=0),
              mock.patch.object(fixture, "run", side_effect=outside_runner),
              contextlib.redirect_stdout(output)):
            self.assertEqual(fixture.main(args), 1)
        record = [line for line in output.getvalue().splitlines()
                  if line.startswith("PAPER_GENUINE_FIXTURE_DIAGNOSTIC=")]
        self.assertEqual(json.loads(record[0].split("=", 1)[1])["case"], "UNKNOWN")


if __name__ == "__main__":
    unittest.main()
