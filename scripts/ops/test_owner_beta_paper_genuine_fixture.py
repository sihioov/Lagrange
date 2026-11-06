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
        owner.network_sent = owner.pg_sent = False
        close = mock.Mock()
        with mock.patch.object(owner, "resolve_helpers", side_effect=fixture.FixtureError("FIXTURE_BACKEND_ALIVE")):
            with self.assertRaises(fixture.FixtureError):
                owner.dispose(close)
        close.assert_not_called()

    def test_disposal_erases_only_after_proof_and_keeps_lock_until_last(self):
        owner = fixture.FixtureOwner.__new__(fixture.FixtureOwner)
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


if __name__ == "__main__":
    unittest.main()
