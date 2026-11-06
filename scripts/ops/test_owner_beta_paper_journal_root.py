"""Non-root protocol/helper tests; these do not prove journal filesystem behavior."""
from __future__ import annotations

import ast
import contextlib
import importlib.util
import io
import json
import os
import signal
import stat
import tempfile
import time
import unittest
import uuid
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
RUNNER_PATH = ROOT / "scripts/ops/owner-beta-paper-journal-root-self-test.py"
SPEC = importlib.util.spec_from_file_location("paper_journal_root_self_test", RUNNER_PATH)
assert SPEC and SPEC.loader
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


def local_directory_check(fd, *, exact_mode=None):
    """Test-only ownership adapter for harmless tempfile cleanup protocol tests."""
    info = os.fstat(fd)
    if (not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid()
            or info.st_gid != os.getegid() or info.st_mode & 0o022
            or (exact_mode is not None and stat.S_IMODE(info.st_mode) != exact_mode)):
        raise RuntimeError("LOCAL_FIXTURE_DIRECTORY_UNSAFE")
    return info


def descriptor_count():
    return len(os.listdir("/proc/self/fd"))


def fork_fixture_child(payload=b"", *, delay=0, exit_code=0):
    """One fixed test child: optionally write bytes, close, delay, and exit."""
    read_fd, write_fd = os.pipe2(os.O_CLOEXEC)
    pid = os.fork()
    if pid == 0:
        os.close(read_fd)
        if payload:
            os.write(write_fd, payload)
        os.close(write_fd)
        if delay:
            time.sleep(delay)
        os._exit(exit_code)
    os.close(write_fd)
    runner._track_child(pid, read_fd)
    return pid, read_fd


class RootJournalSelfTestProtocol(unittest.TestCase):
    def setUp(self):
        self.assertFalse(runner.OWNED_CHILDREN, "prior owned child leaked into next test")
        runner.STOP_SIGNAL = None

    def tearDown(self):
        if runner.OWNED_CHILDREN:
            runner.reap_owned_children(timeout=2)
        self.assertFalse(runner.OWNED_CHILDREN, "owned child was not reaped")
        runner.STOP_SIGNAL = None

    def test_runner_and_companion_are_syntax_valid(self):
        for path in (RUNNER_PATH, Path(__file__)):
            ast.parse(path.read_text(encoding="utf-8"), filename=str(path))

    def test_fixture_name_is_fixed_and_nonce_closed(self):
        nonce = str(uuid.uuid4())
        self.assertEqual(runner.anchor_basename(nonce), runner.ANCHOR_PREFIX + nonce)
        for value in ("../x", "/tmp/x", "a" * 36, nonce.upper()):
            with self.subTest(value=value), self.assertRaises(ValueError):
                runner.anchor_basename(value)

    def test_public_arguments_fail_before_root_fixture_entry(self):
        output = io.StringIO()
        with mock.patch.object(runner, "run_self_test", side_effect=AssertionError("fixture touched")):
            with contextlib.redirect_stdout(output):
                result = runner.main(["--path", "/tmp/forbidden"])
        self.assertEqual(result, 2)
        self.assertIn('"code":"PUBLIC_ARGUMENTS_FORBIDDEN"', output.getvalue())

    def test_nonroot_cli_and_runtime_precondition_fail_closed(self):
        output = io.StringIO()
        with mock.patch.object(runner.os, "geteuid", return_value=1000), \
                mock.patch.object(runner, "run_self_test", side_effect=AssertionError("fixture touched")), \
                contextlib.redirect_stdout(output):
            result = runner.main([])
        self.assertEqual(result, 2)
        self.assertIn('"code":"ROOT_LINUX_REQUIRED"', output.getvalue())
        with mock.patch.object(runner.os, "geteuid", return_value=1000):
            result = runner.run_self_test()
        self.assertEqual((result["status"], result["cleanup"]), ("FAIL", "NOT_STARTED"))

    def test_cleanup_deletes_only_owned_anchor_and_preserves_sibling(self):
        with tempfile.TemporaryDirectory(prefix="paper-journal-root-protocol-") as parent:
            parent_path = Path(parent)
            anchor = parent_path / "owned-anchor"
            anchor.mkdir(mode=0o700)
            nested = anchor / "child"
            nested.mkdir(mode=0o700)
            (nested / "fixture").write_bytes(b"owned")
            sibling = parent_path / "preexisting-sibling"
            sibling.write_bytes(b"preserve")
            parent_fd = os.open(parent, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
            anchor_fd = os.open(anchor, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
            identity = runner.directory_identity(os.fstat(anchor_fd))
            try:
                with mock.patch.object(runner, "assert_root_directory", local_directory_check):
                    runner.cleanup_anchor(parent_fd, anchor.name, anchor_fd, identity)
                self.assertFalse(anchor.exists())
                self.assertEqual(sibling.read_bytes(), b"preserve")
            finally:
                os.close(anchor_fd)
                os.close(parent_fd)

    def test_cleanup_identity_drift_preserves_replacement_and_original(self):
        with tempfile.TemporaryDirectory(prefix="paper-journal-root-drift-") as parent:
            parent_path = Path(parent)
            anchor = parent_path / "owned-anchor"
            anchor.mkdir(mode=0o700)
            (anchor / "owned").write_bytes(b"fixture")
            parent_fd = os.open(parent, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
            anchor_fd = os.open(anchor, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
            identity = runner.directory_identity(os.fstat(anchor_fd))
            held = parent_path / "held-original"
            try:
                anchor.rename(held)
                anchor.mkdir(mode=0o700)
                (anchor / "replacement").write_bytes(b"preserve")
                with mock.patch.object(runner, "assert_root_directory", local_directory_check):
                    with self.assertRaisesRegex(RuntimeError, "CLEANUP_ANCHOR_IDENTITY"):
                        runner.cleanup_anchor(parent_fd, anchor.name, anchor_fd, identity)
                self.assertTrue((held / "owned").exists())
                self.assertEqual((anchor / "replacement").read_bytes(), b"preserve")
            finally:
                os.close(anchor_fd)
                os.close(parent_fd)

    def test_named_summary_requires_every_case_cleanup_and_descriptor_closure(self):
        self.assertEqual(tuple(name for name, _ in runner.CASES), runner.EXPECTED_CASES)
        passes = [{"case": name, "observed": "PASS"} for name, _ in runner.CASES]
        result = runner.result_summary(passes, "PASS", "CLOSED")
        self.assertEqual((result["status"], result["code"]), ("PASS", "ALL_CASES_PASS"))
        result = runner.result_summary(passes[:-1], "PASS", "CLOSED")
        self.assertEqual(result["status"], "FAIL")
        failed = [*passes[:-1], {"case": passes[-1]["case"], "observed": "FAIL", "code": "ACL_REQUIRED"}]
        self.assertEqual(runner.result_summary(failed, "PASS", "CLOSED")["status"], "FAIL")
        self.assertEqual(runner.result_summary(passes, "FAIL", "CLOSED")["status"], "FAIL")
        self.assertEqual(runner.result_summary(passes, "PASS", "LEAKED")["status"], "FAIL")

    def test_owned_child_is_reaped_and_timeout_is_typed(self):
        baseline_fds = descriptor_count()
        pid, read_fd = fork_fixture_child(b"FIXTURE_PASS\n")
        self.assertEqual(runner.wait_owned_child(pid, read_fd, timeout=2), "FIXTURE_PASS")
        self.assertNotIn(pid, runner.OWNED_CHILDREN)
        with self.assertRaises(ChildProcessError):
            os.waitpid(pid, os.WNOHANG)

        pid, read_fd = fork_fixture_child(delay=2)
        self.assertEqual(runner.wait_owned_child(pid, read_fd, timeout=0.05), "CHILD_TIMEOUT")
        self.assertNotIn(pid, runner.OWNED_CHILDREN)
        with self.assertRaises(ChildProcessError):
            os.waitpid(pid, os.WNOHANG)
        self.assertEqual(descriptor_count(), baseline_fds)

    def test_payload_then_live_child_obeys_shared_deadline(self):
        baseline_fds = descriptor_count()
        pid, read_fd = fork_fixture_child(b"FIXTURE_PASS\n", delay=3)
        started = time.monotonic()
        result = runner.wait_owned_child(pid, read_fd, timeout=0.12)
        elapsed = time.monotonic() - started
        self.assertEqual(result, "CHILD_TIMEOUT")
        self.assertLess(elapsed, 1.5)
        self.assertNotIn(pid, runner.OWNED_CHILDREN)
        with self.assertRaises(ChildProcessError):
            os.waitpid(pid, os.WNOHANG)
        self.assertEqual(descriptor_count(), baseline_fds)

    def test_signal_after_payload_reaps_before_fixture_cleanup(self):
        baseline_fds = descriptor_count()
        with tempfile.TemporaryDirectory(prefix="paper-journal-child-cleanup-") as parent:
            parent_path = Path(parent)
            anchor = parent_path / "anchor"
            anchor.mkdir(mode=0o700)
            (anchor / "owned-fixture").write_bytes(b"fixture")
            parent_fd = os.open(parent, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
            anchor_fd = os.open(anchor, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
            identity = runner.directory_identity(os.fstat(anchor_fd))
            pid, read_fd = fork_fixture_child(b"FIXTURE_PASS\n", delay=3)
            old_handler = signal.getsignal(signal.SIGTERM)
            signal.signal(signal.SIGTERM, runner._handle_signal)

            def cancel_after_payload():
                # The child wrote, closed its pipe, and is still alive here.
                self.assertEqual(os.waitpid(pid, os.WNOHANG), (0, 0))
                with mock.patch.object(runner, "assert_root_directory", local_directory_check):
                    with self.assertRaisesRegex(RuntimeError, "CLEANUP_CHILDREN_ACTIVE"):
                        runner.cleanup_anchor(parent_fd, anchor.name, anchor_fd, identity)
                self.assertTrue(anchor.exists(), "fixture cleaned before child reap")
                os.kill(os.getpid(), signal.SIGTERM)

            try:
                result = runner.wait_owned_child(pid, read_fd, timeout=2,
                                                 after_payload=cancel_after_payload)
                self.assertEqual(result, "CHILD_CANCELLED")
                self.assertEqual(runner.STOP_SIGNAL, signal.SIGTERM)
                self.assertNotIn(pid, runner.OWNED_CHILDREN)
                with self.assertRaises(ChildProcessError):
                    os.waitpid(pid, os.WNOHANG)
                with mock.patch.object(runner, "assert_root_directory", local_directory_check):
                    runner.cleanup_anchor(parent_fd, anchor.name, anchor_fd, identity)
                self.assertFalse(anchor.exists())
            finally:
                signal.signal(signal.SIGTERM, old_handler)
                runner.STOP_SIGNAL = None
                os.close(anchor_fd)
                os.close(parent_fd)
        self.assertEqual(descriptor_count(), baseline_fds)

    def test_missing_invalid_payload_and_nonzero_exit_are_rejected(self):
        cases = (
            (b"", 0, "CHILD_RESULT_INVALID"),
            (b"\n", 0, "CHILD_RESULT_INVALID"),
            (b"FIXTURE_PASS\n", 1, "CHILD_EXIT_INVALID"),
        )
        for payload, exit_code, expected in cases:
            with self.subTest(payload=payload, exit_code=exit_code):
                pid, read_fd = fork_fixture_child(payload, exit_code=exit_code)
                self.assertEqual(runner.wait_owned_child(pid, read_fd, timeout=2), expected)
                self.assertNotIn(pid, runner.OWNED_CHILDREN)
                with self.assertRaises(ChildProcessError):
                    os.waitpid(pid, os.WNOHANG)

    def test_context_exit_rejection_requires_body_and_closes_owned_fd(self):
        baseline_fds = descriptor_count()
        state = {}

        @contextlib.contextmanager
        def negative_context():
            fd = os.open("/dev/null", os.O_RDONLY | os.O_CLOEXEC)
            state["fd"] = fd
            try:
                yield fd
                raise runner.journal.JournalError("JOURNAL_LOCK")
            finally:
                os.close(fd)

        runner.expect_context_exit_error(
            negative_context(), "JOURNAL_LOCK", lambda _fd: state.__setitem__("body", "complete"))
        self.assertEqual(state["body"], "complete")
        with self.assertRaises(OSError):
            os.fstat(state["fd"])
        self.assertEqual(descriptor_count(), baseline_fds)

    def test_context_exit_helper_rejects_entry_and_wrong_exit_errors(self):
        @contextlib.contextmanager
        def rejects_on_entry():
            raise runner.journal.JournalError("JOURNAL_LOCK")
            yield None

        with self.assertRaisesRegex(runner.CaseFailure, "CONTEXT_REJECTED_BEFORE_BODY"):
            runner.expect_context_exit_error(rejects_on_entry(), "JOURNAL_LOCK", lambda _value: None)

        @contextlib.contextmanager
        def wrong_exit():
            yield None
            raise runner.journal.JournalError("JOURNAL_FILE")

        with self.assertRaisesRegex(runner.CaseFailure, "CONTEXT_EXIT_EXPECTED_JOURNAL_LOCK_GOT_JOURNAL_FILE"):
            runner.expect_context_exit_error(wrong_exit(), "JOURNAL_LOCK", lambda _value: None)

    def test_unexpected_failure_keeps_deepest_allowlisted_origin_only(self):
        for filename, relative in runner.TRACEBACK_PATH_ALLOWLIST.items():
            with self.subTest(source=relative):
                namespace = {"sentinel": RuntimeError("SECRET_SENTINEL_MESSAGE", "SECRET_SENTINEL_ARG")}
                source = "def outer():\n    inner()\ndef inner():\n    raise sentinel\n"
                exec(compile(source, filename, "exec"), namespace)
                try:
                    namespace["outer"]()
                except RuntimeError as error:
                    result = runner.unexpected_case_result("LINK_OWNER_MODE_AND_ANCESTOR_REJECTION", error)
                else:
                    self.fail("synthetic exception was not raised")
                self.assertEqual(result["observed"], "FAIL")
                self.assertEqual(result["code"], "UNEXPECTED_RUNTIMEERROR")
                self.assertEqual(set(result), {"case", "observed", "code", "origin"})
                self.assertEqual(result["origin"], {"path": relative, "line": 4})
                self.assertEqual(set(result["origin"]), {"path", "line"})
                self.assertIs(type(result["origin"]["line"]), int)
                self.assertLessEqual(result["origin"]["line"], runner.MAX_TRACEBACK_LINE)
                serialized = json.dumps(result)
                self.assertNotIn("SECRET_SENTINEL_MESSAGE", serialized)
                self.assertNotIn("SECRET_SENTINEL_ARG", serialized)
                summary = runner.result_summary(
                    [result if name == result["case"] else {"case": name, "observed": "PASS"}
                     for name, _function in runner.CASES],
                    "PASS", "CLOSED")
                self.assertEqual(summary["status"], "FAIL")

    def test_unallowlisted_origin_and_exception_secrets_are_omitted(self):
        namespace = {"message": "SECRET_SENTINEL_MESSAGE", "argument": "SECRET_SENTINEL_ARG"}
        source = "def fail():\n    raise RuntimeError(message, argument)\n"
        exec(compile(source, "/tmp/SECRET_SENTINEL_FILENAME.py", "exec"), namespace)
        try:
            namespace["fail"]()
        except RuntimeError as error:
            result = runner.unexpected_case_result("LINK_OWNER_MODE_AND_ANCESTOR_REJECTION", error)
        else:
            self.fail("synthetic exception was not raised")
        self.assertEqual(result, {
            "case": "LINK_OWNER_MODE_AND_ANCESTOR_REJECTION",
            "observed": "FAIL",
            "code": "UNEXPECTED_RUNTIMEERROR",
        })
        serialized = json.dumps(result)
        for sentinel in ("SECRET_SENTINEL_MESSAGE", "SECRET_SENTINEL_ARG", "SECRET_SENTINEL_FILENAME"):
            self.assertNotIn(sentinel, serialized)

    def test_traceback_witness_does_not_swallow_unrelated_errors(self):
        try:
            raise RuntimeError("original")
        except RuntimeError as error:
            with mock.patch.object(runner.Path, "resolve", side_effect=LookupError("unrelated")):
                with self.assertRaisesRegex(LookupError, "unrelated"):
                    runner.unexpected_case_result("LINK_OWNER_MODE_AND_ANCESTOR_REJECTION", error)


if __name__ == "__main__":
    unittest.main()
