#!/usr/bin/env python3
"""Nonprivileged decision tests. None constitutes genuine fixture evidence."""
import importlib.util
import ast
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
import textwrap
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
    def test_database_start_uses_the_pinned_postgres18_volume_parent(self):
        class ReadyProbe(Exception):
            pass
        with tempfile.TemporaryDirectory() as root:
            owner = object.__new__(fixture.FixtureOwner)
            owner.root = pathlib.Path(root)
            owner.project = "paper-operations-fixture-unit"
            owner.network = owner.pg = owner.observer = None
            owner.network_sent = owner.pg_sent = False
            owner.deadline = mock.Mock()
            owner.deadline.seconds.return_value = 90
            owner.output = mock.Mock(side_effect=["a" * 64, "b" * 64])
            with mock.patch.object(fixture.os, "chown"), \
                    mock.patch.object(fixture, "PgObserver") as observer:
                observer.return_value.sql.side_effect = ReadyProbe
                with self.assertRaises(ReadyProbe):
                    owner._setup_database(pathlib.Path(root), "unlogged-fixture-password")
            launch = owner.output.call_args_list[1].args[0]
            self.assertIn("type=bind,source=" + root + "/pgdata,target=/var/lib/postgresql", launch)
            self.assertNotIn("type=bind,source=" + root + "/pgdata,target=/var/lib/postgresql/data", launch)
            self.assertEqual(launch[launch.index(fixture.ops.POSTGRES_IMAGE)], fixture.ops.POSTGRES_IMAGE)

    def test_database_setup_does_not_migrate_the_socket_only_init_server(self):
        class ReadyProbe(Exception):
            pass
        with tempfile.TemporaryDirectory() as root:
            owner = object.__new__(fixture.FixtureOwner)
            owner.root, owner.project = pathlib.Path(root), "paper-operations-fixture-unit"
            owner.network = owner.pg = owner.observer = None
            owner.network_sent = owner.pg_sent = False
            owner.deadline = mock.Mock()
            owner.deadline.seconds.return_value = 90
            owner.output = mock.Mock(side_effect=["a" * 64, "b" * 64])
            with mock.patch.object(fixture.os, "chown"), \
                    mock.patch.object(fixture.time, "sleep"), \
                    mock.patch.object(fixture, "PgObserver") as observer:
                observer.return_value.sql.side_effect = [b"lagrange:lagrange", b"", ReadyProbe]
                with self.assertRaises(ReadyProbe):
                    owner._setup_database(pathlib.Path(root), "unlogged-fixture-password")
            statements = [call.args[0] for call in observer.return_value.sql.call_args_list]
            self.assertEqual(statements[1], "SHOW listen_addresses;")
            self.assertFalse(any("CREATE ROLE" in statement for statement in statements))

    def test_context_guard_keeps_the_actual_closed_operations_code(self):
        with mock.patch.object(fixture.ops, "_root"):
            with self.assertRaises(fixture.ops.PaperOperationsError) as caught:
                with fixture.ops.open_context(object(), -1):
                    self.fail("invalid Context entered")
        result = fixture._fixture_failure_diagnostic(caught.exception)
        self.assertEqual(result["code"], "PAPER_OPS_CONTEXT")
        self.assertEqual(fixture._fixture_failure_diagnostic(
            fixture.ops.PaperOperationsError("PAPER_OPS_PRIVATE_SENTINEL_NOT_A_REAL_CODE"))["code"], "UNKNOWN")

    def test_failure_chain_keeps_validated_native_error_without_original_ids(self):
        call = object.__new__(fixture.ops._NativeInvocation)
        call.verb, call._exit = "inspect", 3
        raw = dict(zip(fixture.ops.RESULT_FIELDS, (
            fixture.ops.RESULT_SCHEMA, "inspect", "BLOCKED", None, None, None,
            None, "READER_INCOMPATIBLE"), strict=True))
        call._stdout = json.dumps(raw, separators=(",", ":")).encode()
        namespace = {"ops": fixture.ops}
        exec(compile("def observed_call(self):\n try:\n  ops._require(False, 'PAPER_OPS_RESULT')\n except Exception:\n  ops._require(False, 'PAPER_OPS_UNRESOLVED')\n",
                     fixture.ops.__file__, "exec"), namespace)
        try:
            namespace["observed_call"](call)
        except fixture.ops.PaperOperationsError as error:
            result = fixture._fixture_failure_details(error)
        self.assertFalse(result["truncated"])
        self.assertEqual([r["code"] for r in result["causes"]],
                         ["PAPER_OPS_UNRESOLVED", "PAPER_OPS_RESULT"])
        self.assertEqual(result["causes"][0]["dependency_module"], "PAPER_OPS")
        self.assertGreater(result["causes"][0]["dependency_line"], 0)
        self.assertEqual(result["causes"][0]["native"],
                         [{"verb": "inspect", "outcome": "BLOCKED", "error_code": "READER_INCOMPATIBLE"}])
        self.assertNotIn(b"artifact_sha256", fixture.canonical(result))
        self.assertNotIn(b"generation", fixture.canonical(result))

    def test_failure_chain_is_bounded_and_never_emits_free_text(self):
        secret = "PRIVATE_CHAIN_SENTINEL_NEVER_LOG"
        errors = [fixture.ops.PaperOperationsError("PAPER_OPS_CONTEXT") for _ in range(10)]
        for first, second in zip(errors, errors[1:]):
            first.__context__ = second
        errors[-1].__context__ = Exception(secret)
        result = fixture._fixture_failure_details(errors[0])
        self.assertTrue(result["truncated"])
        self.assertEqual(len(result["causes"]), 8)
        self.assertNotIn(secret.encode(), fixture.canonical(result))
        unknown = fixture._fixture_failure_details(Exception(secret))
        self.assertEqual(unknown["causes"][0]["code"], "UNKNOWN")
        self.assertEqual(unknown["causes"][0]["native"], [])
        self.assertNotIn(secret.encode(), fixture.canonical(unknown))

    def test_dependency_vocabulary_is_frozen_to_trusted_literal_codes(self):
        tree = ast.parse(pathlib.Path(fixture.ops.__file__).read_text())
        codes = {node.value for node in ast.walk(tree) if isinstance(node, ast.Constant)
                 and type(node.value) is str and node.value.startswith("PAPER_OPS_")
                 and node.value.replace("_", "").isalnum()}
        self.assertTrue(codes <= fixture._CLOSED_DEPENDENCY_CODES)
        self.assertGreater(len(codes), 100)
        self.assertNotIn("PAPER_OPS_PRIVATE_SENTINEL_NOT_A_REAL_CODE", fixture._CLOSED_DEPENDENCY_CODES)

    def test_unvalidated_native_bytes_expose_only_closed_fields_and_lengths(self):
        secret = "PRIVATE_NATIVE_SENTINEL_NEVER_LOG"
        call = object.__new__(fixture.ops._NativeInvocation)
        call.verb, call._exit = "inspect", 3
        call._stdout = json.dumps({"outcome": "BLOCKED", "error_code": secret,
                                  "schema": fixture.ops.RESULT_SCHEMA, "extra": secret}).encode()
        namespace = {"ops": fixture.ops}
        exec(compile("def observed_call(self):\n ops._require(False, 'PAPER_OPS_RESULT')\n",
                     fixture.ops.__file__, "exec"), namespace)
        try:
            namespace["observed_call"](call)
        except fixture.ops.PaperOperationsError as error:
            result = fixture._fixture_failure_details(error)
        self.assertEqual(result["causes"][0]["native"], [])
        raw = result["causes"][0]["unvalidated_output"][0]
        self.assertEqual(raw["observed_outcome"], "BLOCKED")
        self.assertEqual(raw["observed_error_code"], "UNKNOWN")
        self.assertEqual(raw["stdout_bytes"], len(call._stdout))
        self.assertNotIn(secret.encode(), fixture.canonical(result))

    def test_fresh_daemon_missing_database_image_keeps_closed_failure_context(self):
        sentinel = b"PRIVATE_DOCKER_STDERR_MUST_NOT_PRINT"
        deadline = fixture.Deadline(time.monotonic_ns() + 900_000_000_000)
        failed = subprocess.CompletedProcess([], 1, b"", sentinel)
        with mock.patch.object(fixture.os, "geteuid", return_value=0), \
                mock.patch.object(fixture.subprocess, "run", return_value=failed) as command:
            docker = fixture.FixtureDocker(deadline)
            with self.assertRaises(fixture.ops.runtime.RuntimeErrorCode) as caught:
                fixture.ops._database_image_id(docker)
        self.assertEqual(command.call_args.args[0][-3:],
                         ["image", "inspect", fixture.ops.POSTGRES_IMAGE])
        result = fixture._fixture_failure_diagnostic(caught.exception)
        self.assertEqual(result["kind"], "RuntimeErrorCode")
        self.assertEqual(result["code"], "DAILY_RUNTIME_DOCKER")
        self.assertNotIn(sentinel.decode(), json.dumps(result))
        for kind in (fixture.ops.runtime.RuntimeErrorCode, fixture.ops.PaperOperationsError):
            hidden = fixture._fixture_failure_diagnostic(kind(sentinel.decode()))
            self.assertEqual(hidden["code"], "UNKNOWN")
            self.assertNotIn(sentinel.decode(), json.dumps(hidden))

    def test_capture_driver_accepts_only_exact_downstream_boundary_after_consumption(self):
        # The old incomplete-archive guard no longer exists on the valid path.
        # This hosted fixture's concrete scope is actual held RR audit/dump.
        for broken_dump in (False, True):
            owner = mock.MagicMock()
            owner.__enter__.return_value = owner
            owner._snapshot = "in-memory-export-only"
            owner.exporter.backend = {"pid": 44}
            owner.description = {"groups": [{}] * 9}
            owner.spools = {"audit.stream": (5, (), 30, "sha256:" + "a" * 64),
                            "postgres.dump": (6, (), 50, "sha256:" + "b" * 64)}
            owner.ops._hash_fd.return_value = (50, "sha256:" + "b" * 64)
            def released():
                owner._snapshot = None
            def closed(*_args):
                released()
                owner._closed = True
            owner.release_capture_clients.side_effect = released
            owner.__exit__.side_effect = closed
            owner.maximum, owner._closed = 1000, False
            owner._snapshot = "in-memory-export-only"
            owner._resources_closed.return_value = True
            paper = types.SimpleNamespace(open_snapshot=mock.Mock(return_value=owner))
            with mock.patch.object(fixture.driver, "_paper_backup", return_value=paper), \
                    mock.patch.object(fixture.driver.os, "pread", return_value=b"BAD!!" if broken_dump else b"PGDMP"):
                if not broken_dump:
                    receipt = fixture.driver.run_capture_prefix(object(), object())
                    self.assertFalse(receipt["archive_acceptance"])
                    self.assertEqual(receipt["scope"], "HELD_SOURCE_RR_AUDIT_AND_DUMP")
                    owner.revalidate.assert_called_once()
                else:
                    with self.assertRaises(fixture.ops.PaperOperationsError):
                        fixture.driver.run_capture_prefix(object(), object())
            owner._dump.assert_called_once()
            owner.capture.assert_not_called()
            owner.__exit__.assert_called_once()

    def test_real_phase_observer_preserves_function_result_and_callable_identity(self):
        calls = []
        def actual(value):
            calls.append(value)
            return value + 1
        original = actual
        observed = []
        with faults.ProductionPhases({"actual": actual},
                lambda phase, event, frame, value: observed.append((phase, event, value)),
                time.monotonic_ns() + 1_000_000_000):
            self.assertEqual(actual(8), 9)
        self.assertIs(actual, original)
        self.assertEqual(calls, [8])
        self.assertEqual([(p, e) for p, e, _ in observed if e in {"call", "return"}],
                         [("actual", "call"), ("actual", "return")])
        self.assertIsNone(sys.getprofile())

    def test_phase_cancellation_cannot_return_success_and_releases_profile(self):
        body = []
        def actual():
            body.append("ran")
        def cancel(_phase, event, _frame, _value):
            if event == "call":
                raise KeyboardInterrupt()
        with self.assertRaises(KeyboardInterrupt):
            with faults.ProductionPhases({"actual": actual}, cancel, time.monotonic_ns() + 1_000_000_000):
                actual()
        self.assertEqual(body, [])
        self.assertIsNone(sys.getprofile())

    def test_archive_pre_intake_cancel_is_call_entry_only(self):
        evidence = object.__new__(fixture.driver.ArchiveAcceptance)
        evidence.fired, evidence.kind = False, "pre-intake-cancel"
        body = []
        def invoke():
            body.append("entered")
        def observe(phase, event, frame, result):
            evidence.inject(phase, event, None, None, frame, result)
        with self.assertRaises(KeyboardInterrupt):
            with faults.ProductionPhases({"invoke": invoke}, observe, time.monotonic_ns() + 1_000_000_000):
                invoke()
        self.assertEqual(body, [])
        self.assertTrue(evidence.fired)
        self.assertIn("intake-cancel", fixture.driver.ArchiveAcceptance.FAULTS)
        self.assertIn("pre-intake-cancel", fixture.driver.ArchiveAcceptance.FAULTS)

    def test_actual_intake_is_connected_only_to_private_fault_consumer(self):
        self.assertIn('intake-cancel', fixture.driver.ArchiveAcceptance.FAULTS)
        self.assertIn('self.archive_fault(owner, values, kind)', inspect.getsource(fixture.FixtureRunner.archive_acceptance))
        source = inspect.getsource(fixture.driver.ArchiveAcceptance.__enter__)
        self.assertIn('attached=production_ops._NativeInvocation._read_attached', source)
        self.assertNotIn('ARCHIVE_INTAKE_CANCEL', inspect.getsource(fixture.run))

    def test_cancelled_unimported_helper_is_unresolved_even_when_exited(self):
        call = types.SimpleNamespace(_closed=False, _child=None, _cid="owned-id",
            _context=types.SimpleNamespace(docker=types.SimpleNamespace(
                object=mock.Mock(return_value={"State": {"Running": False, "Status": "exited"}}))),
            _inspect_and_validate=mock.Mock(), _imported=False, _preserve=False)
        with self.assertRaises(fixture.ops.PaperOperationsError):
            fixture.ops._NativeInvocation.close(call)
        self.assertTrue(call._preserve)
        self.assertFalse(call._closed)
        call._inspect_and_validate.assert_called_once_with(started=True)

    def test_fsync_fault_is_actual_bad_descriptor_error_and_restores_same_open_file(self):
        evidence = object.__new__(fixture.driver.ArchiveAcceptance)
        evidence.kind, evidence.fired, evidence.saved_fd = "publish-fsync", False, None
        def publish(fd):
            os.fsync(fd)
        def observe(_phase, event, frame, function):
            evidence.publication_fault(event, frame, function)
        with tempfile.TemporaryFile() as stream:
            stream.write(b"synthetic-publisher-bytes");stream.flush()
            identity = (os.fstat(stream.fileno()).st_dev, os.fstat(stream.fileno()).st_ino)
            with self.assertRaises(OSError) as caught:
                with faults.ProductionPhases({"publish": publish}, observe, time.monotonic_ns() + 1_000_000_000):
                    publish(stream.fileno())
            self.assertEqual(caught.exception.errno, 9)
            self.assertTrue(evidence.fired)
            self.assertIsNone(evidence.saved_fd)
            self.assertEqual((os.fstat(stream.fileno()).st_dev, os.fstat(stream.fileno()).st_ino), identity)

    def test_real_retained_byte_and_inode_faults_restore_only_the_original(self):
        for kind in ("ciphertext", "missing-member", "changed-member"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as temporary:
                path = pathlib.Path(temporary) / "payload"
                path.write_bytes(b"synthetic-owned-original")
                descriptor = os.open(path, os.O_RDONLY)
                try:
                    inode = path.stat().st_ino
                    original = types.SimpleNamespace(path=str(path), fd=descriptor, size=path.stat().st_size,
                                                     revalidate=mock.Mock())
                    with mock.patch.object(faults.os, "fchown"):
                        effect = faults.RetainedMemberFault(original, kind)
                    if kind == "ciphertext":
                        self.assertNotEqual(path.read_bytes(), b"synthetic-owned-original")
                    elif kind == "missing-member":
                        self.assertFalse(path.exists())
                    else:
                        self.assertNotEqual(path.stat().st_ino, inode)
                        self.assertEqual(path.read_bytes(), b"synthetic-owned-original")
                    effect.close()
                    self.assertEqual(path.stat().st_ino, inode)
                    self.assertEqual(path.read_bytes(), b"synthetic-owned-original")
                finally:
                    os.close(descriptor)

    def test_stored_set_fault_consumes_retained_descriptor_not_a_forged_original(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "postgres.dump.enc"
            path.write_bytes(b"encrypted-fixture")
            descriptor = os.open(path, os.O_RDONLY)
            try:
                retained = types.SimpleNamespace(path=path.parent,
                    files={path.name: (descriptor, (), path.stat().st_size, "unused")}, revalidate=mock.Mock())
                effect = faults.RetainedMemberFault(retained, "ciphertext", member=path.name)
                retained.revalidate.assert_called_once_with()
                self.assertNotEqual(path.read_bytes(), b"encrypted-fixture")
                effect.close()
                self.assertEqual(path.read_bytes(), b"encrypted-fixture")
            finally:
                os.close(descriptor)

    def test_source_witness_detects_identity_change_and_refuses_symlink_member(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            path = root / "original"
            path.write_bytes(b"synthetic")
            before = fixture.private_tree_witness(root)
            path.rename(root / "saved")
            path.write_bytes(b"synthetic")
            (root / "saved").unlink()
            self.assertNotEqual(fixture.private_tree_witness(root), before)
            path.unlink()
            path.symlink_to("/never-read")
            with self.assertRaises(fixture.FixtureError):
                fixture.private_tree_witness(root)

    def test_hosted_ten_cases_do_not_depend_on_private_material_upload(self):
        source = inspect.getsource(fixture.run)
        tree = ast.parse(source)
        calls = [node.func.attr for node in ast.walk(tree)
                 if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
                 and isinstance(node.func.value, ast.Name) and node.func.value.id == "runner"]
        self.assertEqual(calls.count("positive"), 2)
        self.assertEqual(calls.count("restore_negative"), 1)
        self.assertEqual(calls.count("fault"), 1)  # existing seven-element loop
        self.assertNotIn("archive_acceptance", calls)
        self.assertEqual(set(fixture._FIXTURE_DIAGNOSTIC_FAULT_CASES),
                         {"installation", "floor", "stage", "result-delete", "result-replace", "attach-cancel", "cleanup"})

    def test_archive_profile_binds_actual_backup_operations_module(self):
        context = fixture.ops.daily.Context("/var/lib/lagrange-paper-operations-fixture-unit/releases/" + "a" * 40,
            "a" * 40, {}, {}, [])
        owner = types.SimpleNamespace(root=pathlib.Path("/var/lib/lagrange-paper-operations-fixture-unit"),
            context=context, deadline=types.SimpleNamespace(value=time.monotonic_ns() + 1_000_000_000))
        evidence = fixture.driver.ArchiveAcceptance(owner, faults)
        production, _context = evidence.paper._application_context(context)
        with evidence:
            self.assertIn(id(production.open_context.__code__), evidence.profile.codes)
            self.assertIn(id(production._ArchiveCustody.seal_snapshot.__code__), evidence.profile.codes)
            self.assertNotIn(id(fixture.ops.open_context.__code__), evidence.profile.codes)

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
        self.assertEqual(len(lines), 3)
        self.assertTrue(lines[2].startswith("PAPER_GENUINE_FIXTURE_DETAILS="))
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
        self.assertEqual(len(source_codes), 127)
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
        self.assertEqual(len(output.getvalue().splitlines()), 3)
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


class DevCodeOverlayTests(unittest.TestCase):
    """Decision tests only: the overlay relaxes one input check and never reads as a pass."""
    COMMIT = "c" * 40
    MANIFEST = "/fixture/outside-the-build-namespace/release-manifest"
    ARGS = ["--source-root", "/fixture/source", "--commit", "c" * 40,
            "--manifest-file", "/fixture/manifest", "--deadline-monotonic-ns", "1"]

    def validate(self, source, *, flag=None, head=None, status=b"", commit=None, git_status=0):
        calls = []
        def git(argv, **_kwargs):
            calls.append(argv)
            if argv[-2:] == ["rev-parse", "HEAD"]:
                output = ((head or self.COMMIT) + "\n").encode()
            else:
                self.assertEqual(argv[-3:], ["status", "--porcelain", "--untracked-files=normal"])
                output = status
            return subprocess.CompletedProcess(argv, git_status, output, b"")
        kwargs = {} if flag is None else {"dev_code_overlay": flag}
        with mock.patch.object(fixture.subprocess, "run", side_effect=git), \
                self.assertRaises(fixture.FixtureError) as caught:
            fixture.validated_inputs(str(source), commit or self.COMMIT, self.MANIFEST,
                                     time.monotonic_ns() + 300_000_000_000, **kwargs)
        return caught.exception.args[0], calls

    def test_flag_accepts_a_foreign_source_root_that_is_refused_without_it(self):
        with tempfile.TemporaryDirectory(prefix="dev-overlay-source-") as directory:
            foreign = pathlib.Path(directory).resolve()
            self.assertNotEqual(foreign, fixture.ROOT)
            for flag in (None, False, 1, "true"):
                with self.subTest(flag=flag):
                    self.assertEqual(self.validate(foreign, flag=flag), ("FIXTURE_SOURCE", []))
            # The next check after the source is the manifest namespace, so reaching it shows
            # that only the code-root equality was relaxed and git ran in the foreign root.
            code, calls = self.validate(foreign, flag=True)
            self.assertEqual(code, "FIXTURE_MANIFEST_NAMESPACE")
            self.assertEqual(len(calls), 2)
            for argv in calls:
                self.assertEqual(argv[argv.index("-C") + 1], str(foreign))
                self.assertIn("safe.directory=" + str(foreign), argv)
        for flag in (None, True):
            with self.subTest(code_root=True, flag=flag):
                self.assertEqual(self.validate(fixture.ROOT, flag=flag)[0],
                                 "FIXTURE_MANIFEST_NAMESPACE")

    def test_flag_keeps_commit_head_clean_tree_and_git_checks(self):
        with tempfile.TemporaryDirectory(prefix="dev-overlay-source-") as directory:
            foreign = pathlib.Path(directory)
            cases = {
                "dirty tracked file": ({"status": b" M scripts/ops/x.py\n"}, "FIXTURE_CANDIDATE_NOT_CLEAN"),
                "untracked file": ({"status": b"?? new.py\n"}, "FIXTURE_CANDIDATE_NOT_CLEAN"),
                "other HEAD": ({"head": "d" * 40}, "FIXTURE_CANDIDATE_NOT_CLEAN"),
                "git failure": ({"git_status": 128}, "FIXTURE_GIT"),
                "short commit": ({"commit": "c" * 39}, "FIXTURE_SOURCE"),
                "uppercase commit": ({"commit": "C" * 40}, "FIXTURE_SOURCE"),
                "zero commit": ({"commit": "0" * 40}, "FIXTURE_SOURCE"),
            }
            for name, (changes, expected) in cases.items():
                with self.subTest(case=name):
                    self.assertEqual(self.validate(foreign, flag=True, **changes)[0], expected)
            missing = pathlib.Path(directory) / "absent"
            with self.assertRaises(FileNotFoundError):
                fixture.validated_inputs(str(missing), self.COMMIT, self.MANIFEST,
                                         time.monotonic_ns() + 300_000_000_000,
                                         dev_code_overlay=True)

    def test_run_passes_the_flag_only_to_input_validation(self):
        for kwargs, expected in (({}, False), ({"dev_code_overlay": True}, True)):
            with self.subTest(kwargs=kwargs), \
                    mock.patch.object(fixture.os, "geteuid", return_value=0), \
                    mock.patch.object(fixture.os, "getegid", return_value=0), \
                    mock.patch.object(fixture.pathlib.Path, "is_file", return_value=True), \
                    mock.patch.object(fixture.pathlib.Path, "exists", return_value=True), \
                    mock.patch.object(fixture, "validated_inputs",
                                      side_effect=fixture.FixtureError("STOP")) as validated, \
                    self.assertRaises(fixture.FixtureError):
                fixture.run("/fixture/source", self.COMMIT, "/fixture/manifest", 1, **kwargs)
            validated.assert_called_once_with("/fixture/source", self.COMMIT, "/fixture/manifest", 1,
                                              dev_code_overlay=expected)

    def test_success_result_is_never_genuine_with_the_flag(self):
        runner = types.SimpleNamespace(results=[{"case": "STOP", "status": "PASS"}],
                                       cleanups=[{"containers": "REMOVED"}])
        genuine = fixture.success_result(self.COMMIT, b"manifest\n", runner)
        self.assertEqual(genuine, {"status": "PASS", "commit": self.COMMIT,
                                   "manifest_sha256": fixture.ops._digest(b"manifest\n"),
                                   "cases": runner.results, "cleanup": runner.cleanups,
                                   "runtime": "GENUINE_SELECTED_IMAGE", "production": "CLOSED",
                                   "enable": False})
        dev = fixture.success_result(self.COMMIT, b"manifest\n", runner, dev_code_overlay=True)
        self.assertEqual(dev, {**genuine, "status": "DEV_PASS", "runtime": "DEV_CODE_OVERLAY"})
        self.assertEqual(fixture.success_result(self.COMMIT, b"manifest\n", runner,
                                                dev_code_overlay=1), genuine)

    def test_main_prints_the_mode_line_first_and_forwards_the_flag(self):
        mode = "PAPER_GENUINE_FIXTURE_MODE=DEV_CODE_OVERLAY"
        result = {"status": "DEV_PASS", "runtime": "DEV_CODE_OVERLAY"}
        with contextlib.redirect_stdout(output := io.StringIO()), \
                mock.patch.object(fixture.os, "geteuid", return_value=0), \
                mock.patch.object(fixture, "run", return_value=result) as run_mock:
            self.assertEqual(fixture.main([*self.ARGS, "--dev-code-overlay"]), 0)
        self.assertEqual(output.getvalue().splitlines(), [mode, fixture.canonical(result).decode()])
        run_mock.assert_called_once_with("/fixture/source", self.COMMIT, "/fixture/manifest", 1,
                                         dev_code_overlay=True)

        with contextlib.redirect_stdout(output := io.StringIO()), \
                mock.patch.object(fixture.os, "geteuid", return_value=1000), \
                mock.patch.object(fixture, "run") as run_mock:
            self.assertEqual(fixture.main([*self.ARGS, "--dev-code-overlay"]), 77)
        run_mock.assert_not_called()
        self.assertEqual(output.getvalue().splitlines(),
                         [mode, "PAPER_GENUINE_FIXTURE_NOT_RUN:HOSTED_ROOT_PG_FIXTURE_REQUIRED"])

        with contextlib.redirect_stdout(output := io.StringIO()), \
                mock.patch.object(fixture.os, "geteuid", return_value=0), \
                mock.patch.object(fixture, "run", side_effect=fixture.FixtureError("FIXTURE_SOURCE")):
            self.assertEqual(fixture.main(["--dev-code-overlay", *self.ARGS]), 1)
        lines = output.getvalue().splitlines()
        self.assertEqual(lines[:2], [mode, "PAPER_GENUINE_FIXTURE_FAILED:OWNED_CASE_UNPROVEN"])
        self.assertEqual(len(lines), 4)

        # Without the flag no mode line appears and run keeps its four positional inputs.
        with contextlib.redirect_stdout(output := io.StringIO()), \
                mock.patch.object(fixture.os, "geteuid", return_value=0), \
                mock.patch.object(fixture, "run", return_value={"status": "PASS"}) as run_mock:
            self.assertEqual(fixture.main(self.ARGS), 0)
        self.assertEqual(output.getvalue(), fixture.canonical({"status": "PASS"}).decode() + "\n")
        run_mock.assert_called_once_with("/fixture/source", self.COMMIT, "/fixture/manifest", 1)
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()), \
                mock.patch.object(fixture.os, "geteuid", return_value=0), \
                self.assertRaises(SystemExit):
            fixture.main([*self.ARGS, "--dev-code-overlay=yes"])


class IntakeDecisionTests(unittest.TestCase):
    @staticmethod
    def packet(kind=20, *, seq=7, port=81, flags=2, inode=101, peer=202, attrs=None):
        if kind == 3:
            body = struct.pack('=i', 0)
        else:
            body = struct.pack('=BBBBIII', 1, 1, 1, 0, inode, 99, 10)
            body += struct.pack('=HHI', 8, 2, peer) if attrs is None else attrs
        return struct.pack('=IHHII', len(body)+16, kind, flags, seq, port) + body

    def new_intake(self):
        package = mock.Mock()
        acceptance = types.SimpleNamespace(faults=faults, paper=fixture.driver._paper_backup(),
            owner=mock.Mock(), fired=True, records={package: {'calls': {'begin': 1}, 'zero_dispatch': True}})
        intake = fixture.driver.IntakeCancellation(acceptance, package)
        intake.call = types.SimpleNamespace(_imported=False, _closed=False, _preserve=True,
            _context=mock.Mock(docker=mock.Mock()), _cid='a'*64)
        intake.helper = ({'Id': 'a'*64}, {'pid': 10, 'start_ticks': 30}, (), (), {101})
        intake.backend = (42, 700, {'pid': 20, 'start_ticks': 40})
        intake.correlated = intake.disposed = intake.clients_absent = True
        package.completed_destination, package.owner._poisoned = None, True
        package.ops._proc_cgroups.return_value = {}
        intake.bound = mock.Mock(return_value=1)
        return intake

    def test_linux_diag_request_matches_local_uapi(self):
        data = faults.unix_diag_request(7)
        self.assertEqual(struct.unpack_from('=IHHII', data), (40, 20, 0x301, 7, 0))
        self.assertEqual(struct.unpack_from('=BBHIIIII', data, 16),
                         (1, 0, 0, 0xffffffff, 0, 4, 0xffffffff, 0xffffffff))

    def test_linux_diag_stream_peer_and_done(self):
        rows, done = faults.unix_diag_packet(self.packet() + self.packet(3), 7, 81)
        self.assertEqual(rows, {101: 202})
        self.assertTrue(done)
        rows, done = faults.unix_diag_packet(self.packet(), 7, 81)
        self.assertFalse(done)

    def test_linux_diag_rejects_header_errors_and_incomplete_lengths(self):
        packets = [b'', self.packet(seq=8), self.packet(port=82), self.packet(flags=0x12),
            self.packet(kind=2), self.packet()[:-1], self.packet(3)+self.packet(),
            self.packet()+self.packet(), self.packet()+b'x', self.packet(inode=0)]
        for packet in packets:
            with self.subTest(packet=packet[:16]), self.assertRaises(faults.FaultError):
                faults.unix_diag_packet(packet, 7, 81)

    def test_linux_diag_rejects_duplicate_unknown_truncated_peer_attributes(self):
        for attrs in (struct.pack('=HH', 3, 2), struct.pack('=HH', 8, 2),
                struct.pack('=HHI', 8, 2, 4)*2, struct.pack('=HHI', 8, 8, 4),
                struct.pack('=HH', 4, 0x8002), struct.pack('=HH', 4, 2)):
            with self.subTest(attrs=attrs), self.assertRaises(faults.FaultError):
                faults.unix_diag_packet(self.packet(attrs=attrs), 7, 81)

    def test_peer_join_rejects_zero_multiple_and_duplicate_ownership(self):
        row, process = {'pid': 5, 'start_us': 91}, {'pid': 200, 'start_ticks': 50}
        self.assertEqual(faults.unique_unix_backend({101: 202}, [(row, process, {202})]),
                         (101, 202, row, process))
        for peers, candidates in (({}, []), ({101: 202}, []),
                ({101: 202}, [(row, process, {203})]),
                ({101: 202}, [(row, process, {202})]*2),
                ({101: 202, 102: 202}, [(row, process, {202})]),
                ({101: 202, 102: 999}, [(row, process, {202})])):
            with self.subTest(peers=peers), self.assertRaises(faults.FaultError):
                faults.unique_unix_backend(peers, candidates)

    def test_expired_probe_never_opens_namespace_or_spawns(self):
        with mock.patch.object(faults.os, 'open') as opened:
            with self.assertRaises(faults.FaultError):
                faults.unix_peers(2, 5, 'a'*64, time.monotonic_ns()-1)
            opened.assert_not_called()

    def test_probe_child_uses_retained_fd_and_strict_kernel_dump(self):
        info = types.SimpleNamespace(st_dev=4, st_ino=5)
        channel = mock.MagicMock()
        channel.getsockname.return_value = (81, 0)
        library = mock.Mock()
        library.setns.return_value = 0
        for sender, flags, good in (((0, 0), 0, True), ((55, 0), 0, False), ((0, 0), 32, False)):
            channel.recvmsg.return_value = (self.packet(seq=1)+self.packet(3, seq=1), [], flags, sender)
            with mock.patch.object(faults.ctypes, 'CDLL', return_value=library), \
                    mock.patch.object(faults.os, 'fstat', return_value=info), \
                    mock.patch.object(faults.os, 'stat', return_value=info), \
                    mock.patch.object(faults.os, 'close') as close, \
                    mock.patch.object(faults.socket, 'socket') as create:
                create.return_value.__enter__.return_value = channel
                if good:
                    value = faults._unix_probe_child(90, (4, 5), [101], time.monotonic_ns()+1_000_000_000)
                    self.assertEqual(json.loads(value), {'101': 202})
                else:
                    with self.assertRaises(faults.FaultError):
                        faults._unix_probe_child(90, (4, 5), [101], time.monotonic_ns()+1_000_000_000)
                close.assert_called_once_with(90)
                create.assert_called_once_with(faults.socket.AF_NETLINK, faults.socket.SOCK_RAW, 4)

    def test_inherited_work_and_cleanup_deadlines_are_not_renewed(self):
        intake = self.new_intake()
        intake.deadline.end = 900
        intake.deadline.work_remaining.return_value = 0.5
        intake.deadline.remaining.return_value = 0.75
        intake.acceptance.owner.deadline.value = 1000
        intake.acceptance.owner.deadline.seconds.return_value = 1
        bound = fixture.driver.IntakeCancellation.bound
        self.assertEqual(bound(intake, 2), 0.5)
        self.assertEqual(bound(intake, 2, cleanup=True), 0.75)
        self.assertEqual((intake.deadline.end, intake.acceptance.owner.deadline.value), (900, 1000))
        intake.deadline.work_remaining.side_effect = fixture.ops.PaperOperationsError('PAPER_OPS_ARCHIVE_TIMEOUT')
        with self.assertRaises(fixture.ops.PaperOperationsError):
            bound(intake, 2)

    def test_backend_resolves_pid_namespace_generation_parent_and_cgroup(self):
        intake = self.new_intake()
        guest = intake.package.guest
        guest.process = {'pid': 100, 'cgroup': '/owned', 'executable': {'inode': 9}}
        intake.blocker = types.SimpleNamespace(backend={'pid': 10, 'start_us': 1})
        intake.probe = types.SimpleNamespace(backend={'pid': 11, 'start_us': 2})
        rows = [{'pid': 10, 'start_us': 1}, {'pid': 11, 'start_us': 2},
            {'pid': 42, 'start_us': 3, 'active': True, 'audit': True, 'locked': True, 'age_ms': 25}]
        process = {'pid': 200, 'parent': 100, 'start_ticks': 10, 'executable': {'inode': 9}}
        intake.package.ops._proc_cgroups.return_value = {200: '/owned'}
        intake.package.ops._process_record.return_value = process
        with mock.patch.object(pathlib.Path, 'read_text', return_value='NSpid:\t200 42\n'), \
                mock.patch.object(faults, 'namespace_identity', return_value=(4, 5)), \
                mock.patch.object(faults, 'unix_socket_inodes', return_value={202}):
            self.assertEqual(intake.backend_candidates(rows), [(rows[-1], process, {202})])
            for key, bad in (('audit', False), ('locked', False), ('active', False), ('age_ms', 5000)):
                changed = dict(rows[-1], **{key: bad})
                with self.subTest(key=key), self.assertRaises(fixture.ops.PaperOperationsError):
                    intake.backend_candidates(rows[:-1]+[changed])
            process['parent'] = 999
            with self.assertRaises(fixture.ops.PaperOperationsError):
                intake.backend_candidates(rows)
            process['parent'] = 100
            intake.package.ops._process_record.side_effect = [process, dict(process, start_ticks=11)]
            with self.assertRaises(fixture.ops.PaperOperationsError):
                intake.backend_candidates(rows)

    def test_backend_rejects_unowned_extra_client_even_with_same_role(self):
        intake = self.new_intake()
        intake.blocker = types.SimpleNamespace(backend={'pid': 10, 'start_us': 1})
        intake.probe = types.SimpleNamespace(backend={'pid': 11, 'start_us': 2})
        rows = [{'pid': 10, 'start_us': 1}, {'pid': 11, 'start_us': 2},
                {'pid': 42, 'start_us': 3}, {'pid': 43, 'start_us': 4}]
        with self.assertRaises(fixture.ops.PaperOperationsError):
            intake.backend_candidates(rows)
        # PID alone cannot waive a client with a different backend_start.
        with self.assertRaises(fixture.ops.PaperOperationsError):
            intake.backend_candidates([{'pid': 10, 'start_us': 90}, rows[1], rows[2]])

    def test_final_gate_requires_every_disposition_and_poisoned_custody(self):
        for field in ('correlated', 'disposed', 'clients_absent'):
            intake = self.new_intake()
            setattr(intake, field, False)
            self.assertFalse(intake.proven(), field)
        for field, value in (('_imported', True), ('_closed', True), ('_preserve', False)):
            intake = self.new_intake()
            setattr(intake.call, field, value)
            self.assertFalse(intake.proven(), field)
        intake = self.new_intake()
        intake.package.owner._poisoned = False
        self.assertFalse(intake.proven())
        intake.package.owner._poisoned = True
        intake.acceptance.records[intake.package]['calls']['begin'] = 2
        self.assertFalse(intake.proven())

    def test_final_gate_rechecks_retained_processes_and_terminal_container(self):
        intake = self.new_intake()
        with mock.patch.object(faults, 'start_ticks', side_effect=FileNotFoundError):
            self.assertTrue(intake.proven())
        with mock.patch.object(faults, 'start_ticks', return_value=30):
            self.assertFalse(intake.proven())
        with mock.patch.object(faults, 'start_ticks', side_effect=FileNotFoundError):
            intake.package.ops._proc_cgroups.return_value = {10: '/docker/' + 'a'*64}
            self.assertFalse(intake.proven())
            intake.package.ops._proc_cgroups.return_value = {}
            intake.package.ops._terminal_container.side_effect = RuntimeError('changed')
            self.assertFalse(intake.proven())

    def test_independent_disposition_rejects_live_generation_and_unowned_backend(self):
        intake = self.new_intake()
        intake.blocker = types.SimpleNamespace(backend={'pid': 10, 'start_us': 1})
        intake.probe = types.SimpleNamespace(backend={'pid': 11, 'start_us': 2})
        clients = [{'pid': 10, 'start_us': 1}, {'pid': 11, 'start_us': 2}]
        intake.activity = mock.Mock(return_value=clients)
        intake.query = mock.Mock(return_value=0)
        with mock.patch.object(faults, 'start_ticks', side_effect=FileNotFoundError):
            intake.prove_disposition()
            intake.activity.return_value = clients+[{'pid': 90, 'start_us': 91}]
            with self.assertRaises(fixture.ops.PaperOperationsError):
                intake.prove_disposition()
            intake.activity.return_value = clients
            intake.query.return_value = 1
            with self.assertRaises(fixture.ops.PaperOperationsError):
                intake.prove_disposition()
        # A retained live PID cannot pass on empty backend rows or terminal CID alone.
        intake.query.return_value = 0
        with mock.patch.object(faults, 'start_ticks', return_value=30), \
                mock.patch.object(fixture.driver.time, 'monotonic', side_effect=[0, 3]):
            with self.assertRaises(fixture.ops.PaperOperationsError):
                intake.prove_disposition()

    def test_client_disposition_does_not_skip_second_client_on_failure(self):
        intake = self.new_intake()
        intake.blocker, intake.probe = mock.Mock(absent=False), mock.Mock(absent=False)
        intake.blocker.close.side_effect = RuntimeError('unproven')
        with self.assertRaises(RuntimeError):
            intake.close_clients()
        intake.probe.close.assert_called_once()
        self.assertFalse(intake.clients_absent)

    def test_observer_kills_only_after_peer_join_and_checks_before_release(self):
        for scenario in ('owned', 'unknown-peer', 'generation-race'):
            intake = self.new_intake()
            sequence = []
            call = intake.call
            call._created, call._child = 'creation', mock.Mock()
            call._child.poll.return_value = None
            call._archive_observed_container = mock.Mock()
            intake.binding = binding = (object(), b'original')
            intake.inputs = mock.Mock(return_value=binding)
            helper = intake.helper
            intake.helper_identity = mock.Mock(return_value=helper)
            if scenario == 'generation-race':
                intake.helper_identity.side_effect = [helper, (helper[0], dict(helper[1], start_ticks=31), (), (), {101})]
            intake.blocker = types.SimpleNamespace(backend={'pid': 10, 'start_us': 1})
            intake.probe = types.SimpleNamespace(backend={'pid': 11, 'start_us': 2})
            row = dict(pid=42, start_us=70, locked=True, age_ms=1)
            intake.activity = mock.Mock(return_value=[row])
            intake.backend_candidates = mock.Mock(return_value=[(row, helper[1], {202})])
            live = {'Created': 'creation', 'State': {'Running': True, 'Pid': 10}}
            exited = {'State': {'Running': False}}
            call._context.docker.object.side_effect = [live, exited, exited]
            intake.no_dispatch = mock.Mock(side_effect=lambda: sequence.append('no-dispatch'))
            intake.prove_disposition = mock.Mock(side_effect=lambda: sequence.append('absent'))
            intake.close_clients = mock.Mock(side_effect=lambda: sequence.append('close-clients'))
            intake.package.owner._cleanup_docker.side_effect = lambda _: sequence.append('kill')
            # Reset gates: construction's values must never stand in for observation.
            intake.correlated = intake.disposed = False
            intake.acceptance.fired = False
            with mock.patch.object(faults, 'unix_peers', return_value={101: 999 if scenario == 'unknown-peer' else 202}):
                if scenario == 'owned':
                    intake.observe(call, types.SimpleNamespace(f_locals={'child': call._child}))
                    self.assertEqual(sequence, ['no-dispatch', 'kill', 'absent', 'no-dispatch', 'close-clients'])
                    self.assertTrue(intake.correlated and intake.disposed and intake.acceptance.fired)
                else:
                    with self.assertRaises((faults.FaultError, fixture.ops.PaperOperationsError)):
                        intake.observe(call, types.SimpleNamespace(f_locals={'child': call._child}))
                    self.assertFalse(intake.correlated or intake.disposed or intake.acceptance.fired)
                    self.assertEqual(sequence[-1], 'close-clients')

    def test_zero_dispatch_rejects_durable_or_in_memory_dispatch_and_sql_effect(self):
        intake = self.new_intake()
        intake.binding = ('original',)
        intake.inputs = mock.Mock(return_value=intake.binding)
        intake.query = mock.Mock(return_value=0)
        with tempfile.TemporaryDirectory() as root:
            descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            try:
                action = types.SimpleNamespace(_dispatch_bytes=None, _output_fd=descriptor)
                intake.package.destination.actions = [action]
                intake.package.destination.operation.generation = '00000000-0000-0000-0000-000000000001'
                intake.no_dispatch()
                pathlib.Path(root, 'dispatch.json').write_bytes(b'uncertain')
                with self.assertRaises(fixture.ops.PaperOperationsError):
                    intake.no_dispatch()
                pathlib.Path(root, 'dispatch.json').unlink()
                action._dispatch_bytes = b'uncertain'
                with self.assertRaises(fixture.ops.PaperOperationsError):
                    intake.no_dispatch()
                action._dispatch_bytes = None
                intake.query.return_value = 1
                with self.assertRaises(fixture.ops.PaperOperationsError):
                    intake.no_dispatch()
            finally:
                os.close(descriptor)

    def test_fixture_disposal_revokes_registries_without_certifying_production_close(self):
        intake = self.new_intake()
        package, call, owner = intake.package, intake.call, intake.package.owner
        owner._closed, package.guest.absent = False, True
        owner._cleanup_docker.return_value = ''
        package.destination.imports, package.destination.actions, package.destination.stages = [], [], []
        package.destination.lock_fd = None
        package.custody = package.snapshot = None
        package.originals, package.ciphertexts, package.directories = [], {}, []
        owner.audit, owner.secret_sources, owner.spools, owner.sources = None, None, {}, []
        run = mock.Mock(live=call._context)
        intake.p = types.SimpleNamespace(_PACKAGES={package}, _OWNERS={owner._custody: owner},
            _SPACES={call._context: owner.space}, _RUNS={run})
        with mock.patch.object(faults, 'start_ticks', side_effect=FileNotFoundError):
            intake.dispose_retained_fixture()
        self.assertFalse(call._imported or call._closed or owner._closed)
        self.assertTrue(owner._poisoned and call._preserve)
        self.assertTrue(intake.fixture_handles_closed)
        owner._custody.close.assert_not_called()
        self.assertFalse(intake.fixture_disposed)
        self.assertFalse(intake.p._PACKAGES or intake.p._SPACES or intake.p._OWNERS or intake.p._RUNS)
        owner.space.selection._remove_tree.assert_not_called()
        intake.remove_fixture_trees()
        owner.space.root_tree.revalidate.assert_called_once()
        owner.space.selection._remove_tree.assert_called_once()
        self.assertTrue(intake.fixture_disposed)

    def test_fixture_disposal_rejects_cid_uncertainty_before_closing_originals(self):
        intake = self.new_intake()
        intake.package.owner._closed = False
        intake.package.guest.absent = True
        intake.package.owner._cleanup_docker.return_value = 'still-present'
        with self.assertRaises(fixture.ops.PaperOperationsError):
            intake.dispose_retained_fixture()
        intake.package.destination.operation.close.assert_not_called()
        intake.package.tree.close.assert_not_called()
        self.assertFalse(intake.fixture_handles_closed)

    def test_consumer_reports_unknown_until_intake_and_disposition_proven(self):
        for proven in (False, True):
            runner = object.__new__(fixture.FixtureRunner)
            runner.results = []
            owner = mock.Mock()
            counts = dict(sql=0, function=0, events=0, receipts=0, oid=1, catalog=1, prepared=0)
            owner.observer.snapshot.return_value = counts
            owner.observer.settle.return_value = counts
            owner.observer.archive_source_witness.return_value = {'same': True}
            observed = mock.MagicMock(fired=True, publications=[], snapshots=[])
            observed.intake.proven.return_value = proven
            paper = mock.Mock()
            paper.run_locked.side_effect = KeyboardInterrupt()
            with mock.patch.object(fixture.driver, 'ArchiveAcceptance', return_value=observed), \
                    mock.patch.object(fixture.driver, '_paper_backup', return_value=paper), \
                    mock.patch.object(fixture, 'complete_sets_witness', return_value={'sets': {}, 'latest': None}):
                runner.archive_fault(owner, {'BACKUP_ROOT': 'private', 'MAX_TOTAL_BYTES': 10}, 'intake-cancel')
            self.assertEqual(runner.results[0]['status'], 'PASS' if proven else 'UNKNOWN')
            self.assertEqual(runner.results[0]['case'], 'ARCHIVE_INTAKE_CANCEL')
            self.assertEqual(paper.run_locked.call_count, 1)


class BackendCorrelationSourceTests(unittest.TestCase):
    def test_backend_address_is_compared_without_the_netmask(self):
        # pg_stat_activity.client_addr is inet. A text cast appends the netmask length ("/32"),
        # which never equals the bare address Docker reports for the helper container, so the
        # watcher could not recognise the native backend and never released its row lock.
        source = pathlib.Path(PATH).read_text()
        self.assertNotIn("client_addr::text", source)
        self.assertIn("'addr',host(client_addr),'port',client_port", source)


@unittest.skipUnless(os.geteuid() == 0 and sys.platform == "linux", "hosted Root startup check")
class RootFixtureStartupTests(unittest.TestCase):
    def _assert_real_native_create_policy(self, owner, source, commit):
        # Create-only transport proof; this base image has no candidate ELF.
        bases = fixture.re.findall(r"^FROM (alpine:[^\s]+@sha256:[0-9a-f]{64})$",
                                  (source / "crates/api-server/Dockerfile").read_text(), fixture.re.M)
        self.assertEqual(len(bases), 1)
        owner.command(["pull", bases[0]], cap=60)
        image = owner.docker.object("image", bases[0])
        self.assertIn("alpine@" + bases[0].split("@", 1)[1], image["RepoDigests"])
        image_env = fixture.ops._image_environment(image["Config"], commit)
        context = types.SimpleNamespace(revalidate=lambda: None, network_id=owner.network,
            image=image["Id"], commit=commit, image_environment=image_env, docker=owner.docker)
        call = fixture.ops._NativeInvocation(fixture.ops._INVOCATION_TOKEN, context, "inspect", None)
        args = call._create_args()
        args[-2:-2] = ["--label", "org.opencontainers.image.revision=" + commit]
        cid = owner.output(args)
        self.assertRegex(cid, r"^[0-9a-f]{64}$")
        original = owner.docker.object("container", cid)
        try:
            call._cid, call._created = cid, original["Created"]
            # No patches: real CLI env merging, image/host policy, network and
            # container identity guards all consume the original inspect record.
            call._inspect_and_validate(started=False)
        finally:
            current = owner.docker.object("container", cid)
            fixture.ops._validate_container_identity(current, cid, call._nonce,
                original["Created"], image["Id"], commit)
            self.assertFalse(current["State"]["Running"])
            self.assertEqual(current["State"]["Status"], "created")
            owner.command(["rm", cid])
            self.assertEqual(owner.output(["ps", "-aq", "--no-trunc", "--filter", "id=" + cid]), "")

    def _assert_real_native_process_environment(self, owner, source, commit):
        # Observe the real container process environment under the production create args.
        # The runtime adds HOME=/root for uid 0; the native allowlist (protected.rs
        # env_entries_are_clean) accepts exactly this set.
        bases = fixture.re.findall(r"^FROM (alpine:[^\s]+@sha256:[0-9a-f]{64})$",
                                  (source / "crates/api-server/Dockerfile").read_text(), fixture.re.M)
        self.assertEqual(len(bases), 1)
        owner.command(["pull", bases[0]], cap=60)
        image = owner.docker.object("image", bases[0])
        image_env = fixture.ops._image_environment(image["Config"], commit)
        context = types.SimpleNamespace(revalidate=lambda: None, network_id=owner.network,
            image=image["Id"], commit=commit, image_environment=image_env, docker=owner.docker)
        call = fixture.ops._NativeInvocation(fixture.ops._INVOCATION_TOKEN, context, "inspect", None)
        args = call._create_args()
        args[-2:-2] = ["--label", "org.opencontainers.image.revision=" + commit]
        args[args.index("--entrypoint") + 1] = "/usr/bin/env"
        del args[-1]
        cid = owner.output(args)
        self.assertRegex(cid, r"^[0-9a-f]{64}$")
        try:
            printed = owner.output(["start", "--attach", cid])
        finally:
            owner.command(["rm", cid])
            self.assertEqual(owner.output(["ps", "-aq", "--no-trunc", "--filter", "id=" + cid]), "")
        pairs = [line.split("=", 1) for line in printed.splitlines() if line]
        keys = sorted(pair[0] for pair in pairs)
        home = next((pair[1] for pair in pairs if pair[0] == "HOME" and len(pair) == 2), None)
        print("PAPER_FIXTURE_OBSERVED_NATIVE_ENV=" + json.dumps(
            {"home": home, "keys": keys}, sort_keys=True, separators=(",", ":")), flush=True)
        self.assertEqual(keys, sorted([*fixture.ops.NATIVE_ENV, "HOME"]),
                         "observed env keys: %r" % (keys,))
        self.assertEqual(home, "/root")

    def _observe_native_container_preconditions(self, owner, source, commit, password):
        # What the native guards read inside the real container, observed under the production
        # create args with the three inspect mounts. The assertions mirror check_dir and
        # check_file_mode in protected.rs and allowed_caps in archive_sandbox.rs. When one of
        # those rules changes, change its mirror here in the same commit.
        bases = fixture.re.findall(r"^FROM (alpine:[^\s]+@sha256:[0-9a-f]{64})$",
                                  (source / "crates/api-server/Dockerfile").read_text(), fixture.re.M)
        self.assertEqual(len(bases), 1)
        owner.command(["pull", bases[0]], cap=60)
        image = owner.docker.object("image", bases[0])
        image_env = fixture.ops._image_environment(image["Config"], commit)
        context = types.SimpleNamespace(revalidate=lambda: None, network_id=owner.network,
            image=image["Id"], commit=commit, image_environment=image_env, docker=owner.docker)
        private, output = fixture.ops.PRIVATE, fixture.ops.OUTPUT_ALIAS
        directories = ["/", "/run", private, output, "/run/secrets"]
        files = [private + "/request.json", private + "/database.json",
                 "/run/secrets/db_migration_owner_password"]
        extra = ["/usr", "/usr/local", "/usr/local/bin"]
        stage = pathlib.Path(tempfile.mkdtemp(prefix="owner-beta-paper-observe-", dir="/run"))
        secret = pathlib.Path(tempfile.mkdtemp(prefix="owner-beta-paper-observe-secret-", dir="/run"))
        pgpass = pathlib.Path(tempfile.mkdtemp(prefix="owner-beta-paper-observe-pgpass-", dir="/run"))
        try:
            # Same shape as _inspect_generation_null and _prepare_inspect: a 0700 stage with two
            # 0400 files and an output directory, and a 0700 secret stage with one 0400 file.
            stage.chmod(0o700)
            secret.chmod(0o700)
            fixture.write_new(stage / "request.json", b"{}", 0o400)
            fixture.write_new(stage / "database.json", b"{}", 0o400)
            (stage / "output").mkdir(mode=0o700)
            fixture.write_new(secret / "db_migration_owner_password", b"observation-only", 0o400)
            call = fixture.ops._NativeInvocation(fixture.ops._INVOCATION_TOKEN, context, "inspect", None)
            call._mounts = ((str(stage), private, True), (str(secret), "/run/secrets", True),
                            (str(stage / "output"), output, False))

            def run(entrypoint, *arguments):
                args = call._create_args()
                args[-2:-2] = ["--label", "org.opencontainers.image.revision=" + commit]
                args[args.index("--entrypoint") + 1] = entrypoint
                args[-1:] = list(arguments)
                cid = owner.output(args)
                self.assertRegex(cid, r"^[0-9a-f]{64}$")
                try:
                    result = owner.command(["start", "--attach", cid], allow_failure=True)
                    return result.returncode, result.stdout.decode("ascii", "replace")
                finally:
                    owner.command(["rm", cid])
                    self.assertEqual(
                        owner.output(["ps", "-aq", "--no-trunc", "--filter", "id=" + cid]), "")

            stat_exit, stat_text = run("/bin/stat", "-c", "%n|%h|%u|%g|%a|%F",
                                       *directories, *files, *extra)
            observed = {}
            for line in stat_text.splitlines():
                parts = line.split("|")
                if len(parts) == 6:
                    observed[parts[0]] = {"links": int(parts[1]), "uid": int(parts[2]),
                                          "gid": int(parts[3]), "mode": parts[4], "type": parts[5]}
            status_exit, status_text = run("/bin/cat", "/proc/self/status")
            wanted = ("Uid", "Gid", "Groups", "CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb",
                      "NoNewPrivs", "Seccomp", "Seccomp_filters")
            process = {}
            for line in status_text.splitlines():
                key, _, value = line.partition(":")
                if key in wanted:
                    process[key] = " ".join(value.split())
            driver = owner.output(["info", "--format", "{{.Driver}}"])
            security = json.loads(owner.output(["info", "--format", "{{json .SecurityOptions}}"]))

            # The native connects as migration_owner over TCP on the internal network. Every other
            # fixture SQL call uses the server's Unix socket, so this login is otherwise unproven.
            pgpass.chmod(0o700)
            os.chown(pgpass, 999, 999)
            fixture.write_new(pgpass / "pgpass",
                ("postgres:5432:lagrange:migration_owner:" + password + "\n").encode(),
                0o400, 999, 999)
            sql = ("BEGIN; SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY; "
                   "SELECT public.inspect_owner_beta_paper_operation_v1(NULL::uuid,NULL::uuid,1); "
                   "COMMIT;")
            login = owner.command(["run", "--rm", "--network", owner.network, "--user", "999:999",
                "--read-only", "--cap-drop", "ALL", "--env", "PGPASSFILE=/run/observe/pgpass",
                "--mount", "type=bind,source=" + str(pgpass) + ",target=/run/observe,readonly",
                "--entrypoint", "psql", fixture.ops.POSTGRES_IMAGE, "-X", "-q", "-A", "-t",
                "-v", "ON_ERROR_STOP=1", "-v", "VERBOSITY=sqlstate",
                "-d", "host=postgres port=5432 dbname=lagrange user=migration_owner sslmode=prefer",
                "-c", sql], cap=60, allow_failure=True)
            page_keys = data_keys = None
            pages = [line for line in login.stdout.decode("utf-8", "replace").splitlines()
                     if line.startswith("{")]
            if login.returncode == 0 and len(pages) == 1:
                page = json.loads(pages[0])
                page_keys = sorted(page)
                if isinstance(page.get("data"), dict):
                    data_keys = sorted(page["data"])
            # Five-character tokens only (SQLSTATE, ERROR, FATAL); never the message text.
            tokens = sorted(set(fixture.re.findall(
                r"\b[0-9A-Z]{5}\b", login.stderr.decode("ascii", "replace"))))
            # The fixture later proves native calls through function and statement statistics.
            # Observe that one call of a SECURITY DEFINER PL function by a backend that has
            # exited is visible to a new session at once.
            published = owner.observer.json(
                "SELECT pg_stat_clear_snapshot(); SELECT json_build_object("
                "'function',pg_stat_get_function_calls("
                "'public.inspect_owner_beta_paper_operation_v1(uuid,uuid,integer)'::regprocedure::oid),"
                "'sql',(SELECT COALESCE(sum(calls),0)::bigint FROM public.pg_stat_statements "
                "WHERE toplevel AND regexp_replace(lower(query),'[[:space:]]','','g') "
                "LIKE 'selectpublic.inspect_owner_beta_paper_operation_v1(%'));")
            print("PAPER_FIXTURE_OBSERVED_FUNCTION_STATS=" + json.dumps(
                {key: published.get(key) if type(published.get(key)) is int else None
                 for key in ("function", "sql")}, sort_keys=True, separators=(",", ":")),
                flush=True)

            print("PAPER_FIXTURE_OBSERVED_NATIVE_PATHS=" + json.dumps(
                {"driver": driver, "paths": observed, "security": security, "stat_exit": stat_exit},
                sort_keys=True, separators=(",", ":")), flush=True)
            print("PAPER_FIXTURE_OBSERVED_NATIVE_PROCESS=" + json.dumps(
                {"cat_exit": status_exit, "status": process},
                sort_keys=True, separators=(",", ":")), flush=True)
            print("PAPER_FIXTURE_OBSERVED_DB_TCP=" + json.dumps(
                {"data_keys": data_keys, "exit": login.returncode, "page_keys": page_keys,
                 "tokens": tokens}, sort_keys=True, separators=(",", ":")), flush=True)

            self.assertEqual(stat_exit, 0)
            self.assertEqual(sorted(observed), sorted([*directories, *files, *extra]))
            rejected = []
            for path in directories:
                entry = observed[path]
                mode = int(entry["mode"], 8)
                # check_dir: uid 0, gid 0, at least two links, no group or other write, and
                # exactly 0700 for the operation directory and its output directory.
                # check_image_dir: the image's own `/` and `/run` are overlay directories merged
                # from several layers and may report one link.
                minimum = 1 if path in ("/", "/run") else 2
                if (entry["type"] != "directory" or entry["uid"] != 0 or entry["gid"] != 0
                        or entry["links"] < minimum or mode & 0o022
                        or (path in (private, output) and mode != 0o700)):
                    rejected.append(path)
            for path in files:
                entry = observed[path]
                # check_file_mode: regular, uid 0, gid 0, exactly 0400, one link.
                if (not entry["type"].startswith("regular") or entry["uid"] != 0
                        or entry["gid"] != 0 or entry["links"] != 1
                        or int(entry["mode"], 8) != 0o400):
                    rejected.append(path)
            self.assertEqual(rejected, [],
                             "paths the native directory or file guard rejects: %r" % (rejected,))
            self.assertEqual(status_exit, 0)
            zero = "0" * 16
            # allowed_caps(None): inheritable and ambient empty, effective equal to permitted. The
            # inspect create args add no capability, so that set is empty here.
            self.assertEqual((process.get("CapInh"), process.get("CapAmb")), (zero, zero))
            self.assertEqual((process.get("CapPrm"), process.get("CapEff")), (zero, zero))
            self.assertEqual((process.get("Uid"), process.get("Gid")), ("0 0 0 0", "0 0 0 0"))
            self.assertEqual(process.get("NoNewPrivs"), "1")
            self.assertEqual(login.returncode, 0, "psql exit; tokens %r" % (tokens,))
            self.assertIsNotNone(page_keys)
        finally:
            for path in (stage, secret, pgpass):
                fixture.shutil.rmtree(path, ignore_errors=True)

    def _observe_statistics_round(self, owner):
        # One settle round is absent() plus the statistics query. Each SQL call is a docker exec
        # wrapped in four docker inspects, so the real duration on this runner calibrates the
        # 10 s settle cap and the stop-attempt timing windows. Observation only.
        rounds, value = [], {}
        for _ in range(3):
            started = time.monotonic_ns()
            owner.observer.absent()
            value = owner.observer._statistics()
            rounds.append((time.monotonic_ns() - started) // 1_000_000)
        typed = {key: value.get(key) if type(value.get(key)) is int else None
                 for key in ("sql", "function", "events", "receipts", "catalog", "prepared")}
        print("PAPER_FIXTURE_OBSERVED_STATS_ROUND=" + json.dumps(
            {"backends": len(owner.observer.backends), "rounds_ms": rounds, "values": typed},
            sort_keys=True, separators=(",", ":")), flush=True)

    def _observe_host_units(self):
        # The stop attempt enumerates every systemd service and timer on the host through
        # _HostStopObserver.units_now. Run that real code against this runner's systemd.
        observer = fixture.ops._HostStopObserver
        stub = types.SimpleNamespace(
            deadline=types.SimpleNamespace(remaining=lambda cap=15: cap),
            roots=("/nonexistent-lagrange-fixture-root",), unit_files={}, collector_locks=set())
        stub.command = lambda argv, payload=None, cap=15: observer.command(stub, argv, payload, cap)
        stub.systemctl = lambda *args: observer.systemctl(stub, *args)
        names = templates = None
        try:
            listed = set()
            for verb in ("list-unit-files", "list-units"):
                text = stub.systemctl(verb, "--all", "--no-legend", "--plain", "--type=service,timer")
                listed.update(row.split()[0] for row in text.splitlines() if row.split())
            names = len(listed)
            templates = sum(1 for name in listed if "@." in name)
        except fixture.ops.PaperOperationsError:
            pass
        started = time.monotonic_ns()
        try:
            result, selected = "OK", len(observer.units_now(stub))
        except fixture.ops.PaperOperationsError as error:
            code = error.args[0] if error.args else None
            result = code if type(code) is str and fixture.re.fullmatch("[A-Z0-9_]{1,64}", code) else "UNKNOWN"
            selected = None
        print("PAPER_FIXTURE_OBSERVED_HOST_UNITS=" + json.dumps(
            {"elapsed_ms": (time.monotonic_ns() - started) // 1_000_000, "names": names,
             "result": result, "selected": selected, "templates": templates},
            sort_keys=True, separators=(",", ":")), flush=True)
        # No unit on a hosted runner belongs to this installation, so the enumeration must
        # complete and select nothing.
        self.assertEqual((result, selected), ("OK", 0))

    def test_root_handoff_namespace_and_pinned_database_image(self):
        # Real owned DB setup is included; synthetic manifest IDs grant no image authority.
        commit = subprocess.check_output(
            ["git", "-c", "safe.directory=" + str(fixture.ROOT), "-C", str(fixture.ROOT),
             "rev-parse", "HEAD"], text=True).strip()
        raw = ("LAGRANGE_RELEASE_MANIFEST_V3\ncommit|" + commit + "\n" + "".join(
            "image|" + service + "|lagrange-station-" + service + ":" + commit +
            "|sha256:" + "a" * 64 + "|" + commit + "\n"
            for service in fixture.ops.daily.policy.MANIFEST_V3_SERVICES)).encode()
        state = pathlib.Path(tempfile.mkdtemp(prefix="lagrange-paper-fixture-build-", dir="/var/tmp"))
        policy_root = pathlib.Path(tempfile.mkdtemp(prefix="lagrange-paper-startup-", dir="/var/tmp"))
        state.chmod(0o700)
        policy_root.chmod(0o700)
        policy = fixture.PrivatePolicy(policy_root)
        namespace = os.stat("/proc/self/ns/mnt").st_ino
        try:
            fixture.write_new(state / "release-manifest", raw)
            source, captured, deadline = fixture.validated_inputs(
                str(fixture.ROOT), commit, str(state / "release-manifest"),
                time.monotonic_ns() + 300_000_000_000)
            self.assertEqual((source, captured), (fixture.ROOT, raw))
            with policy:
                self.assertNotEqual(os.stat("/proc/self/ns/mnt").st_ino, namespace)
                identity = fixture.ops._database_image_id(fixture.FixtureDocker(deadline))
                self.assertRegex(identity, r"^sha256:[0-9a-f]{64}$")
                owner = fixture.FixtureOwner(source, commit, captured, deadline)
                owner.pg_image = identity
                with owner.extractions:
                    try:
                        fixture.write_new(owner.root / "postgres-secret",
                            fixture.uuid.uuid4().hex.encode(), 0o400, 999, 999)
                        password = fixture.uuid.uuid4().hex
                        owner._setup_database(source, password)
                        self.assertEqual(int(owner.observer.sql("SHOW server_version_num;")) // 10000, 18)
                        self.assertEqual(owner.observer.sql("SHOW data_directory;"),
                                         b"/var/lib/postgresql/18/docker")
                        self._assert_real_native_create_policy(owner, source, commit)
                        self._assert_real_native_process_environment(owner, source, commit)
                        self._observe_native_container_preconditions(
                            owner, source, commit, password)
                        self._observe_statistics_round(owner)
                        self._observe_host_units()
                        # Exercise the production network guard with actual
                        # created/exited Docker records before the long build.
                        # The pinned DB image grants no candidate/native authority;
                        # unrelated native image/mount validators are out of scope.
                        name = owner.project + "-network-probe"
                        cid = owner.output(["create", "--name", name, "--network", owner.network,
                            "--label", "com.docker.compose.project=" + owner.project,
                            "--user", "0:0", "--read-only", "--cap-drop", "ALL",
                            "--entrypoint", "/bin/true", "--mount", "type=bind,source=" +
                            str(owner.root / "pgdata") + ",target=/var/lib/postgresql,readonly",
                            fixture.ops.POSTGRES_IMAGE])
                        self.assertRegex(cid, r"^[0-9a-f]{64}$")
                        original = owner.docker.object("container", cid)
                        try:
                            context = types.SimpleNamespace(revalidate=lambda: None,
                                network_id=owner.network, image=identity, commit=commit,
                                image_environment={}, docker=owner.docker)
                            call = fixture.ops._NativeInvocation(
                                fixture.ops._INVOCATION_TOKEN, context, "inspect", None)
                            call._cid, call._nonce, call._created = cid, "probe", original["Created"]
                            with mock.patch.object(fixture.ops, "_validate_container_identity"), \
                                 mock.patch.object(fixture.ops, "_validate_runtime_policy"), \
                                 mock.patch.object(fixture.ops, "_validate_mount_records"), \
                                 mock.patch.object(fixture.ops, "_image_environment", return_value={}):
                                self.assertEqual(original["HostConfig"]["NetworkMode"], owner.network)
                                call._inspect_and_validate(started=False)
                                owner.command(["start", "--attach", cid])
                                call._exit = 0
                                call._inspect_and_validate(started=True)
                        finally:
                            current = owner.docker.object("container", cid)
                            self.assertEqual((current["Id"], current["Image"], current["Created"]),
                                (cid, identity, original["Created"]))
                            self.assertEqual(current["Config"]["Labels"]["com.docker.compose.project"],
                                             owner.project)
                            self.assertFalse(current["State"]["Running"])
                            self.assertIn(current["State"]["Status"], ("created", "exited"))
                            owner.command(["rm", cid])
                            self.assertEqual(owner.output(["ps", "-aq", "--no-trunc", "--filter", "id=" + cid]), "")
                    finally:
                        deadline.cleanup_phase()
                        owner.dispose(lambda: None)
                self.assertEqual(owner.cleanup["containers"], "REMOVED")
                self.assertEqual(owner.cleanup["backends"], "ABSENT_PREPARED_ZERO")
                self.assertEqual(owner.cleanup["inputs"], "REMOVED_AFTER_DISPOSITION")
                self.assertFalse(owner.root.exists())
            self.assertEqual(os.stat("/proc/self/ns/mnt").st_ino, namespace)
        finally:
            # Uncertain namespace custody keeps its source; it is never a success.
            if not policy.bound and not policy.entered:
                fixture.shutil.rmtree(policy_root)
                fixture.shutil.rmtree(state)


if __name__ == "__main__":
    unittest.main()
