"""Pure failure-contract tests for the R5 SQL fixture driver."""

import importlib.util
import json
import os
import re
import signal
import subprocess
import sys
from types import SimpleNamespace
from unittest import mock
from pathlib import Path
import tempfile
import unittest


DRIVER_PATH = Path(__file__).with_name("r5-paper-sql.py")
GUEST_SCRIPT_PATH = Path(__file__).with_name("r5-paper-sql-guest.sh")
SPEC = importlib.util.spec_from_file_location("r5_paper_sql", DRIVER_PATH)
driver = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(driver)


def extract_bounded_test_function():
    source = GUEST_SCRIPT_PATH.read_text(encoding="utf-8")
    match = re.search(r"(?ms)^run_bounded_test\(\) \(\n.*?^\)", source)
    if match is None:
        raise AssertionError("bounded test helper is missing or is not a subshell")
    return source, match.group(0)


class FixtureDriverContractTests(unittest.TestCase):
    def test_named_test_capture_keeps_progress_and_closed_witnesses_separate(self):
        source = GUEST_SCRIPT_PATH.read_text(encoding="utf-8")
        invocation = next(line for line in source.splitlines() if "elif run_bounded_test /input/tests" in line)
        self.assertIn("--show-output", invocation)
        self.assertNotIn("--nocapture", invocation)

    def test_captured_success_preserves_exact_result_and_first_closed_witness(self):
        name = "paper_0067::paper_0067_ready_auto_retry_denies_revoked_admission_after_earlier_valuation"
        item = {"stage": "READY_SETUP", "outcome": "ENTERED"}
        output = (
            f"running 1 test\ntest {name} ... ok\n\nsuccesses:\n\n"
            f"---- {name} stdout ----\n"
            f"R5_PAPER_TEST_WITNESS:{json.dumps(item)}\n\n"
            "test result: ok. 1 passed; 0 failed; 0 ignored; 27 filtered out; finished in 0.01s\n"
        )
        driver.validate_test_result(name, output, 0)
        self.assertEqual(driver.parse_failure_witness(output.encode())["test_witnesses"], [item])

    def test_file_size_limit_is_only_inside_bounded_test_subshell(self):
        source, helper = extract_bounded_test_function()
        limit_lines = re.findall(r"(?m)^[ \t]*ulimit\b[^\n]*$", source)
        self.assertEqual(limit_lines, ["    ulimit -f 4096 || exit 70"])
        self.assertIn(limit_lines[0], helper)
        self.assertNotRegex(source.replace(helper, ""), r"(?m)^[ \t]*ulimit\b")

    @unittest.skipUnless(os.name == "posix", "kernel RLIMIT_FSIZE regression requires POSIX")
    def test_bounded_test_subshell_inherits_only_the_test_file_limit(self):
        _source, helper = extract_bounded_test_function()
        self.assertIn('exec "$@"', helper)
        writer = """\
import errno
import os
import signal
import sys

signal.signal(signal.SIGXFSZ, lambda signum, frame: None)
fd = os.open(sys.argv[1], os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
remaining = 16 * 1024 * 1024
chunk = b"\\0" * 65536
try:
    while remaining:
        try:
            written = os.write(fd, chunk[:min(remaining, len(chunk))])
        except OSError as error:
            if error.errno == errno.EFBIG:
                sys.exit(23)
            raise
        remaining -= written
finally:
    os.close(fd)
"""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            before = root / "before.bin"
            bounded = root / "bounded.bin"
            after = root / "after.bin"
            script = "\n".join((
                "set -eu",
                helper,
                'limit_before=$(ulimit -f)',
                '"$PYTHON" -B -c "$WRITER_SCRIPT" "$BEFORE"',
                'before_size=$(wc -c < "$BEFORE")',
                "set +e",
                'run_bounded_test "$PYTHON" -B -c "$WRITER_SCRIPT" "$BOUNDED"',
                "bounded_status=$?",
                "set -e",
                'limit_after=$(ulimit -f)',
                '"$PYTHON" -B -c "$WRITER_SCRIPT" "$AFTER"',
                'after_size=$(wc -c < "$AFTER")',
                'bounded_size=$(wc -c < "$BOUNDED")',
                'printf "%s %s %s %s %s %s\\n" "$limit_before" "$limit_after" "$before_size" "$bounded_size" "$after_size" "$bounded_status"',
            ))
            child = subprocess.Popen(
                ["/bin/sh", "-eu", "-c", script],
                env={
                    **os.environ,
                    "PYTHON": sys.executable,
                    "WRITER_SCRIPT": writer,
                    "BEFORE": str(before),
                    "BOUNDED": str(bounded),
                    "AFTER": str(after),
                },
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            try:
                stdout, stderr = child.communicate(timeout=30)
            except subprocess.TimeoutExpired:
                child.kill()
                stdout, stderr = child.communicate()
                self.fail(f"bounded helper child timed out and was reaped: {stderr}")

            self.assertIsNotNone(child.returncode, "shell child must reach a terminal state")
            self.assertEqual(child.returncode, 0, stderr)
            (limit_before, limit_after, before_size, bounded_size, after_size,
             bounded_status) = stdout.split()
            before_size, bounded_size, after_size, bounded_status = map(
                int, (before_size, bounded_size, after_size, bounded_status)
            )
            self.assertEqual(limit_before, limit_after, "parent RLIMIT_FSIZE changed")
            self.assertEqual(before_size, 16 * 1024 * 1024)
            self.assertEqual(after_size, 16 * 1024 * 1024)
            self.assertGreater(bounded_status, 0, "bounded child unexpectedly wrote 16 MiB")
            self.assertGreater(bounded_size, 0)
            self.assertLessEqual(bounded_size, 4 * 1024 * 1024)


    def test_failure_witness_extracts_only_static_panic_paths_and_sqlstate(self):
        locations = (
            "/workspace/tests/paper_0067.rs:41:9",
            "/workspace/tests/migration_contract.rs:42:10",
            "/workspace/tests/integration/migration-contract/tests/paper_0067.rs:43:11",
            "/workspace/tests/integration/migration-contract/tests/migration_contract.rs:44:12",
            "/workspace/tests/unsupported.rs:45:13",
        )
        output = "\n".join(
            f"thread 'case' panicked at {location}: hidden panic payload"
            for location in locations
        ) + '\nDatabase(PgDatabaseError { severity: Error, code: Some("23505"), message: "SECRET-SQL-PAYLOAD" })\n'
        witness = driver.parse_failure_witness(output.encode("utf-8"))
        self.assertEqual(witness["categories"], ["DATABASE", "RUST_PANIC"])
        self.assertEqual(witness["sqlstates"], ["23505"])
        self.assertEqual(
            witness["panic_locations"],
            [
                {"path": "tests/paper_0067.rs", "line": 41, "column": 9},
                {"path": "tests/migration_contract.rs", "line": 42, "column": 10},
                {
                    "path": "tests/integration/migration-contract/tests/paper_0067.rs",
                    "line": 43,
                    "column": 11,
                },
                {
                    "path": "tests/integration/migration-contract/tests/migration_contract.rs",
                    "line": 44,
                    "column": 12,
                },
            ],
        )
        serialized = json.dumps(witness)
        for forbidden in ("/workspace", "hidden panic payload", "SECRET-SQL-PAYLOAD"):
            self.assertNotIn(forbidden, serialized)

    def test_failure_witness_handles_result_error_without_a_panic(self):
        output = b'Err(Database(PgDatabaseError { severity: Error, code: "23505", message: "private" }))'
        witness = driver.parse_failure_witness(output)
        self.assertEqual(witness["categories"], ["DATABASE"])
        self.assertEqual(witness["sqlstates"], ["23505"])
        self.assertEqual(witness["panic_locations"], [])
        self.assertNotIn("RUST_PANIC", witness["categories"])

        row_not_found = driver.parse_failure_witness(b"Err(RowNotFound)")
        self.assertEqual(row_not_found["categories"], ["ROW_NOT_FOUND"])
        self.assertEqual(row_not_found["panic_locations"], [])
        self.assertEqual(row_not_found["sqlstates"], [])

        typed_errors = driver.parse_failure_witness(
            b'Err(ColumnDecode("private")); PoolTimedOut; Io(OsError); assertion `left == right` failed'
        )
        self.assertEqual(
            typed_errors["categories"],
            ["COLUMN_DECODE", "POOL_TIMED_OUT", "IO", "ASSERTION"],
        )

    def test_failure_witness_discards_freeform_and_sentinel_payloads(self):
        sentinel = "SENTINEL-ACCOUNT-ROW-AND-SECRET-SQL"
        self.assertIsNone(driver.parse_failure_witness(
            f"{sentinel} SQLSTATE=bad!! /workspace/unsupported.rs:12:3".encode("utf-8")
        ))
        witness = driver.parse_failure_witness(
            f'SQLSTATE: 23505; message="{sentinel}"; query="SELECT * FROM private_rows"'.encode("utf-8")
        )
        self.assertEqual(witness["categories"], ["DATABASE"])
        self.assertEqual(witness["sqlstates"], ["23505"])
        serialized = json.dumps(witness)
        self.assertNotIn(sentinel, serialized)
        self.assertNotIn("SELECT", serialized)
        self.assertNotIn("private_rows", serialized)

    def test_failure_witness_bounds_deduplicates_and_tracks_truncation(self):
        records = []
        for index in range(1, 20):
            panic = f"thread 'case' panicked at tests/paper_0067.rs:{index}:7: payload"
            state = f"SQLSTATE: {index:05d}"
            records.extend((panic, panic, state, state))
        records.extend((
            "thread 'bad' panicked at tests/paper_0067.rs:0:7:",
            "thread 'bad' panicked at tests/paper_0067.rs:1000001:7:",
            "thread 'bad' panicked at tests/paper_0067.rs:10:0:",
        ))
        witness = driver.parse_failure_witness(
            "\n".join(records).encode("utf-8"), input_truncated=True
        )
        self.assertLessEqual(len(witness["categories"]), 16)
        self.assertLessEqual(len(witness["panic_locations"]), 16)
        self.assertLessEqual(len(witness["sqlstates"]), 16)
        self.assertEqual(len(witness["panic_locations"]), 16)
        self.assertEqual(len(witness["sqlstates"]), 16)
        self.assertEqual(len({
            (item["path"], item["line"], item["column"])
            for item in witness["panic_locations"]
        }), 16)
        self.assertEqual(len(set(witness["sqlstates"])), 16)
        self.assertTrue(witness["truncated"])

        capped = driver.parse_failure_witness(
            b"thread 'case' panicked at tests/paper_0067.rs:1:1:\n"
            + b"x" * driver.MAX_LOG_BYTES
            + b"\nSQLSTATE: 23505"
        )
        self.assertTrue(capped["truncated"])
        self.assertEqual(capped["sqlstates"], [])
        self.assertEqual(
            capped["panic_locations"],
            [{"path": "tests/paper_0067.rs", "line": 1, "column": 1}],
        )

    def test_test_witness_accepts_closed_stages_state_pids_and_booleans(self):
        for stage in sorted(driver.TEST_WITNESS_STAGES):
            item = {
                "stage": stage,
                "outcome": "DATABASE_ERROR",
                "sqlstate": "55P03",
                "backend_pid": 1,
                "blocker_pid": 2_147_483_647,
                "advisory_blocked": True,
                "completed_before_unlock": False,
            }
            output = driver.TEST_WITNESS_MARKER + json.dumps(item).encode("ascii") + b"\n"
            witness = driver.parse_failure_witness(output)
            self.assertEqual(witness["test_witnesses"], [item])

    def test_test_witness_rejects_open_schema_and_never_exports_marker_payload(self):
        secret = "SENTINEL-PRIVATE-ROW-AND-ERROR"
        invalid = [
            {"stage": "UNLISTED", "outcome": "ENTERED"},
            {"stage": "READY_SETUP", "outcome": "UNKNOWN"},
            {"stage": "READY_SETUP", "outcome": "ENTERED", "sql": secret},
            {"stage": "READY_SETUP", "outcome": "ENTERED", "sqlstate": "55p03"},
            {"stage": "READY_SETUP", "outcome": "ENTERED", "backend_pid": True},
            {"stage": "READY_SETUP", "outcome": "ENTERED", "blocker_pid": 0},
            {"stage": "READY_SETUP", "outcome": "ENTERED", "advisory_blocked": 1},
            {"stage": "READY_SETUP", "outcome": "ENTERED", "completed_before_unlock": "false"},
        ]
        output = b"".join(
            driver.TEST_WITNESS_MARKER + json.dumps(item).encode("ascii") + b"\n"
            for item in invalid
        )
        self.assertIsNone(driver.parse_failure_witness(output))
        self.assertNotIn(secret, json.dumps(driver.parse_failure_witness(output) or {}))

    def test_test_witnesses_are_unique_capped_and_respect_two_mib_scan(self):
        lines = []
        for index in range(20):
            item = {"stage": "READY_SETUP", "outcome": "ENTERED", "backend_pid": index + 1}
            lines.append(driver.TEST_WITNESS_MARKER + json.dumps(item).encode("ascii") + b"\n")
        bounded = driver.parse_failure_witness(b"".join(lines))
        self.assertEqual(len(bounded["test_witnesses"]), 16)
        self.assertTrue(bounded["truncated"])
        self.assertEqual(len({json.dumps(item, sort_keys=True) for item in bounded["test_witnesses"]}), 16)

        first_line = lines[0]
        filler = b"x" * max(0, driver.MAX_LOG_BYTES - len(first_line) - 1)
        after_cap = first_line + b"\n" + filler + b"\n" + lines[1]
        capped = driver.parse_failure_witness(after_cap)
        self.assertTrue(capped["truncated"])
        self.assertEqual(len(capped["test_witnesses"]), 1)
        self.assertEqual(capped["test_witnesses"][0]["backend_pid"], 1)

    def test_test_witness_no_marker_preserves_legacy_failure_shape(self):
        witness = driver.parse_failure_witness(
            b"Err(RowNotFound)\nthread 'case' panicked at tests/paper_0067.rs:9:2: payload"
        )
        self.assertEqual(
            witness,
            {
                "categories": ["ROW_NOT_FOUND", "RUST_PANIC"],
                "panic_locations": [
                    {"path": "tests/paper_0067.rs", "line": 9, "column": 2}
                ],
                "sqlstates": [],
                "truncated": False,
            },
        )

    def test_artifact_requires_exact_test_target_and_existing_elf(self):
        with tempfile.TemporaryDirectory() as directory:
            artifact = Path(directory) / "migration_contract-abc"
            artifact.write_bytes(b"\x7fELFfixture")
            message = {
                "reason": "compiler-artifact",
                "target": {"name": "other_integration", "kind": ["test"]},
                "profile": {"test": True},
                "executable": str(artifact),
            }
            with self.assertRaisesRegex(driver.FixtureError, "TEST_ARTIFACT_MISSING_OR_AMBIGUOUS"):
                driver.parse_artifact([json.dumps(message)])
            message["target"]["name"] = "migration_contract"
            self.assertEqual(driver.parse_artifact([json.dumps(message)]), artifact)

    def test_malformed_zero_passed_ignored_and_nonzero_outcomes_fail_closed(self):
        name = "paper_0067::paper_0067_case"
        valid = f"test {name} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out"
        driver.validate_test_result(name, valid, 0)
        invalid_results = [
            ("not a rust test result", 0),
            (f"test result: ok. 0 passed; 0 failed; 1 ignored;", 0),
            (f"test {name} ... ok\ntest result: ok. 0 passed; 0 failed; 0 ignored;", 0),
            (valid, 1),
            (f"test {name} ... FAILED\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;", 0),
        ]
        for output, exit_code in invalid_results:
            with self.subTest(output=output, exit_code=exit_code):
                with self.assertRaises(driver.FixtureError):
                    driver.validate_test_result(name, output, exit_code)

    def test_inventory_requires_original_baseline_but_accepts_new_named_cases(self):
        baseline = "\n".join(f"{name}: test" for name in sorted(driver.BASELINE_TESTS))
        selected = driver.parse_test_inventory(baseline + "\npaper_0067::future_regression: test\nother::ignored: test")
        self.assertEqual(len(selected), len(driver.BASELINE_TESTS) + 1)
        with self.assertRaisesRegex(driver.FixtureError, "TEST_INVENTORY_BASELINE_MISSING"):
            driver.parse_test_inventory("paper_0067::only_one: test")

    def test_ignored_sql_baseline_and_ordinary_source_guard_are_distinguished(self):
        ignored = "\n".join(f"{name}: test" for name in sorted(driver.IGNORED_BASELINE_TESTS))
        ordinary = "\n".join(f"{name}: test" for name in sorted(driver.SOURCE_CONTRACT_TESTS))
        self.assertTrue(driver.IGNORED_BASELINE_TESTS.issubset(driver.parse_test_names(ignored)))
        self.assertTrue(driver.SOURCE_CONTRACT_TESTS.issubset(driver.parse_test_names(ordinary)))
        self.assertTrue(driver.SOURCE_CONTRACT_TESTS.isdisjoint(driver.parse_test_names(ignored)))

    def test_inventory_requires_each_0068_checkpoint_case(self):
        required = {
            "paper_0067::paper_0067_0068_sql_lifecycle_source_contract_is_closed",
            "paper_0067::paper_0067_0068_fence_reconcile_offline_readback_and_history_guard",
            "paper_0067::paper_0068_exact_receipt_canonical_roles_legacy_and_down",
            "paper_0067::paper_0068_exact_readback_live_backend_late_commit_and_cancel",
            "paper_0067::paper_0068_enable_reset_times_swaps_and_descriptive_readback",
            "paper_0067::paper_0068_reset_races_enable_control_lock",
            "paper_0067::paper_0068_exact_recovery_v1_v2_receipts",
        }
        self.assertTrue(required.issubset(driver.BASELINE_TESTS))
        self.assertTrue((required - driver.SOURCE_CONTRACT_TESTS).issubset(driver.IGNORED_BASELINE_TESTS))
        for missing in required:
            with self.subTest(missing=missing):
                listing = "\n".join(f"{name}: test" for name in sorted(driver.BASELINE_TESTS - {missing}))
                with self.assertRaisesRegex(driver.FixtureError, "TEST_INVENTORY_BASELINE_MISSING"):
                    driver.parse_test_inventory(listing)

    def test_driver_refuses_non_github_environment(self):
        with self.assertRaisesRegex(driver.FixtureError, "GITHUB_ACTIONS_REQUIRED"):
            driver.require_ci_environment({"RUNNER_TEMP": "/tmp/runner", "CARGO_TARGET_DIR": "/tmp/runner/cargo"})
        with self.assertRaisesRegex(driver.FixtureError, "JOB_PRIVATE_PATHS_NOT_ABSOLUTE"):
            driver.require_ci_environment({"GITHUB_ACTIONS": "true", "RUNNER_TEMP": "relative", "CARGO_TARGET_DIR": "/tmp/cargo"})

    def test_failed_creator_intent_reconciles_and_removes_only_matching_resource(self):
        nonce = "a" * 32
        identity = "b" * 64

        class FakeDocker(driver.Docker):
            def __init__(self):
                self.removed = False
                self.running = True
                self.calls = []

            def call(self, args, *, timeout, allow_missing=False):
                self.calls.append(args)
                if args[:2] == ["container", "inspect"]:
                    reference = args[2]
                    if self.removed:
                        return None
                    return json.dumps([{
                        "Id": identity,
                        "Name": "/lagrange-r5-paper-" + nonce,
                        "Config": {"Labels": {"lagrange.r5-paper.nonce": nonce}},
                        "State": {"Running": self.running, "ExitCode": 125},
                    }])
                if args[0] == "stop":
                    self.running = False
                    return ""
                if args[0] == "rm":
                    self.removed = True
                    return ""
                raise AssertionError(args)

            def reconcile_intent(self, name, requested_nonce):
                self.calls.append(["reconcile", name, requested_nonce])
                return identity if requested_nonce == nonce else None

        fake = FakeDocker()
        original = driver.FixtureError("DOCKER_COMMAND_FAILED")
        recovered, error = driver.reconcile_failed_create(fake, "lagrange-r5-paper-" + nonce, nonce, original)
        self.assertEqual(recovered, identity)
        self.assertIs(error, original)
        secondary = []
        self.assertTrue(driver.close_owned_container(fake, recovered, "lagrange-r5-paper-" + nonce, nonce, secondary))
        self.assertTrue(fake.removed)
        self.assertEqual(secondary, [])
        self.assertIn(["reconcile", "lagrange-r5-paper-" + nonce, nonce], fake.calls)

    def test_child_failure_still_runs_owned_cleanup(self):
        nonce = "c" * 32
        identity = "d" * 64

        class FakeDocker(driver.Docker):
            def __init__(self):
                self.running = False
                self.removed = False

            def call(self, args, *, timeout, allow_missing=False):
                if args[:2] == ["container", "inspect"]:
                    if self.removed:
                        return None
                    return json.dumps([{
                        "Id": identity,
                        "Name": "/fixture",
                        "Config": {"Labels": {"lagrange.r5-paper.nonce": nonce}},
                        "State": {"Running": self.running, "ExitCode": 7},
                    }])
                if args[0] == "rm":
                    self.removed = True
                    return ""
                raise AssertionError(args)

        fake = FakeDocker()
        first = driver.FixtureError("NAMED_TEST_EXIT_NONZERO")
        secondary = []
        try:
            raise first
        except driver.FixtureError as observed:
            self.assertTrue(driver.close_owned_container(fake, identity, "fixture", nonce, secondary))
            self.assertIs(observed, first)
        self.assertTrue(fake.removed)
        self.assertIs(driver.first_error(first, secondary), first)

    def test_cleanup_and_log_failures_never_replace_first_error(self):
        primary = driver.FixtureError("NAMED_TEST_EXIT_NONZERO")
        self.assertIs(driver.first_error(primary, ["CONTAINER_LOG_COLLECTION_FAILED", "DOCKER_COMMAND_FAILED"]), primary)
        self.assertEqual(
            driver.first_error(None, ["CONTAINER_LOG_COLLECTION_FAILED", "DOCKER_COMMAND_FAILED"]).code,
            "CONTAINER_LOG_COLLECTION_FAILED",
        )

    def run_failed_fixture(self, root, *, inspect_failure=False,
                           interrupt_logs=None, interrupt_save=None):
        target = root / "cargo"
        target.mkdir()
        artifact = target / "migration_contract-fixture"
        artifact.write_bytes(b"\x7fELFfixture")
        artifact.chmod(0o555)
        library = root / "libfaketime.so.1"
        library.write_bytes(b"fixture-library")
        state = SimpleNamespace(removed=False, created=False, signaled=False)
        identity = "a" * 64

        class FakeDocker(driver.Docker):
            def call(self, args, **kwargs):
                if args[:2] == ["container", "inspect"]:
                    return None
                if args[0] == "create":
                    state.created = True
                    if inspect_failure:
                        raise driver.FixtureError("OWNED_COMMAND_TIMEOUT", exit_status=124)
                    return identity
                if args[0] == "start":
                    raise driver.FixtureError("START_FAILED")
                raise AssertionError(args)

            def assert_owned(self, *args):
                return {"state": {"Running": False}}

            def reconcile_intent(self, *args):
                if inspect_failure:
                    raise driver.FixtureError("CONTAINER_INSPECT_INVALID")
                return identity

            def cleanup(self, *args):
                state.removed = True

        def checked(argv, **kwargs):
            if argv[0] == "cargo":
                output = json.dumps({"reason": "compiler-artifact",
                    "target": {"name": "migration_contract", "kind": ["test"]},
                    "profile": {"test": True}, "executable": str(artifact)})
            elif argv[0] == str(artifact):
                if "--ignored" in argv:
                    names = driver.IGNORED_BASELINE_TESTS
                elif "--include-ignored" in argv:
                    names = driver.BASELINE_TESTS
                else:
                    names = driver.SOURCE_CONTRACT_TESTS
                output = "\n".join(f"{name}: test" for name in sorted(names))
            elif argv[:2] == ["docker", "pull"]:
                output = ""
            elif argv[:3] == ["docker", "image", "inspect"]:
                output = "[]"
            else:
                raise AssertionError(argv)
            return SimpleNamespace(stdout=output, stderr="", returncode=0)

        def command(argv, **kwargs):
            self.assertEqual(argv, ["docker", "logs", identity])
            if interrupt_logs is not None:
                os.kill(os.getpid(), interrupt_logs)
                state.signaled = True
            return SimpleNamespace(stdout="", stderr="", returncode=0)

        original_write = driver.write_json

        def write(path, value):
            if (interrupt_save is not None and not state.signaled
                    and state.removed and value.get("status") == "FAIL"):
                os.kill(os.getpid(), interrupt_save)
                state.signaled = True
            return original_write(path, value)

        environment = {"GITHUB_ACTIONS": "true", "RUNNER_TEMP": str(root),
                       "CARGO_TARGET_DIR": str(target)}
        with mock.patch.dict(os.environ, environment), \
                mock.patch.object(driver, "Docker", FakeDocker), \
                mock.patch.object(driver, "discover_clock_library",
                                  return_value=(library, "0.9.10-2.1", "f" * 64)), \
                mock.patch.object(driver, "compile_clock_wait_bridge",
                                  return_value=(root / "clock-wait.so", {"artifact_sha256": "a" * 64})), \
                mock.patch.object(driver, "verify_staged_clock_wait_bridge"), \
                mock.patch.object(driver, "checked", side_effect=checked), \
                mock.patch.object(driver, "command", side_effect=command), \
                mock.patch.object(driver, "parse_image_inspect", return_value={}), \
                mock.patch.object(driver, "write_json", side_effect=write):
            status = driver.run_fixture(root / "evidence", library)
        result = json.loads((root / "evidence/result.json").read_text())
        return status, result, state

    def test_create_error_stays_primary_when_intent_inspection_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            status, result, state = self.run_failed_fixture(
                Path(directory), inspect_failure=True)
            self.assertTrue(state.created)
            self.assertEqual(status, 124)
            self.assertEqual(result["error_code"], "OWNED_COMMAND_TIMEOUT")
            self.assertIn("CONTAINER_INSPECT_INVALID", result["secondary_error_codes"])
            self.assertEqual(result["container"]["removed"], "UNKNOWN")
            self.assertEqual(set(result["stages"].values()), {"REMOVED_CONFIRMED"})

    def test_first_interrupt_during_cleanup_preserves_error_and_finishes(self):
        def interrupt(_signum, _frame):
            raise KeyboardInterrupt

        for kind in (signal.SIGINT, signal.SIGTERM):
            with self.subTest(signal=kind), tempfile.TemporaryDirectory() as directory:
                previous = signal.signal(kind, interrupt)
                try:
                    status, result, state = self.run_failed_fixture(
                        Path(directory), interrupt_logs=kind)
                    self.assertIs(signal.getsignal(kind), interrupt)
                finally:
                    signal.signal(kind, previous)
                self.assertTrue(state.signaled)
                self.assertTrue(state.removed)
                self.assertEqual(status, 1)
                self.assertEqual(result["error_code"], "START_FAILED")
                self.assertIn("CLEANUP_INTERRUPTED", result["secondary_error_codes"])
                self.assertEqual(result["container"]["removed"], "CONFIRMED")
                self.assertEqual(set(result["stages"].values()), {"REMOVED_CONFIRMED"})

    def test_interrupt_during_final_save_is_recorded_before_handler_restore(self):
        def interrupt(_signum, _frame):
            raise KeyboardInterrupt

        with tempfile.TemporaryDirectory() as directory:
            previous = signal.signal(signal.SIGTERM, interrupt)
            try:
                status, result, state = self.run_failed_fixture(
                    Path(directory), interrupt_save=signal.SIGTERM)
                self.assertIs(signal.getsignal(signal.SIGTERM), interrupt)
            finally:
                signal.signal(signal.SIGTERM, previous)
            self.assertTrue(state.signaled)
            self.assertTrue(state.removed)
            self.assertEqual(status, 1)
            self.assertEqual(result["error_code"], "START_FAILED")
            self.assertIn("CLEANUP_INTERRUPTED", result["secondary_error_codes"])

    def test_driver_named_test_argv_uses_captured_output(self):
        artifact = Path("/tmp/migration_contract-test")
        argv = driver.test_argv(artifact, "paper_0067::example")
        self.assertEqual(argv, [str(artifact), "--exact", "paper_0067::example",
                                "--include-ignored", "--test-threads=1", "--show-output"])
        self.assertNotIn("--nocapture", argv)

    def test_clock_wait_bridge_compile_is_bounded_pinned_and_readonly_staged(self):
        captured = {}

        def fake_checked(argv, **kwargs):
            captured["argv"] = argv
            captured.update(kwargs)
            Path(argv[argv.index("-o") + 1]).write_bytes(b"\x7fELFfixture")
            return subprocess.CompletedProcess(argv, 0, "private compiler output", "")

        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.object(driver.shutil, "which", return_value="/usr/bin/cc"), \
                mock.patch.object(driver, "checked", side_effect=fake_checked):
            staged, evidence = driver.compile_clock_wait_bridge(Path(directory))
            self.assertEqual(captured["timeout"], 30)
            self.assertEqual(captured["cwd"], driver.ROOT)
            self.assertEqual(captured["code"], "CLOCK_WAIT_BRIDGE_COMPILE_FAILED")
            self.assertEqual(captured["argv"][1:6], ["-std=c11", "-O2", "-fPIC", "-shared", "-Wl,-z,relro,-z,now"])
            self.assertEqual(evidence["source_sha256"], driver.CLOCK_WAIT_BRIDGE_SOURCE_SHA256)
            self.assertEqual(evidence["source_sha256_after_compile"], driver.CLOCK_WAIT_BRIDGE_SOURCE_SHA256)
            self.assertEqual(evidence["artifact_sha256"], driver.sha256_file(staged))
            self.assertEqual(staged.stat().st_mode & 0o777, 0o444)
            self.assertNotIn("private compiler output", json.dumps(evidence))

    def test_clock_wait_bridge_missing_compiler_bad_output_and_source_mutation_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            with mock.patch.object(driver.shutil, "which", return_value=None):
                with self.assertRaisesRegex(driver.FixtureError, "CLOCK_WAIT_COMPILER_MISSING"):
                    driver.compile_clock_wait_bridge(Path(directory))
            with mock.patch.object(driver.shutil, "which", return_value="/usr/bin/cc"), \
                    mock.patch.object(driver, "checked", return_value=None):
                with self.assertRaisesRegex(driver.FixtureError, "CLOCK_WAIT_BRIDGE_OUTPUT_INVALID"):
                    driver.compile_clock_wait_bridge(Path(directory))
            def fake_checked(argv, **kwargs):
                Path(argv[argv.index("-o") + 1]).write_bytes(b"\x7fELFfixture")
                return subprocess.CompletedProcess(argv, 0, "", "")
            with mock.patch.object(driver.shutil, "which", return_value="/usr/bin/cc"), \
                    mock.patch.object(driver, "checked", side_effect=fake_checked), \
                    mock.patch.object(driver, "sha256_file", side_effect=[driver.CLOCK_WAIT_BRIDGE_SOURCE_SHA256, "0" * 64]):
                with self.assertRaisesRegex(driver.FixtureError, "CLOCK_WAIT_SOURCE_MUTATED_DURING_COMPILE"):
                    driver.compile_clock_wait_bridge(Path(directory))

    def test_staged_clock_wait_bridge_hash_mutation_fails_closed(self):
        def fake_checked(argv, **kwargs):
            Path(argv[argv.index("-o") + 1]).write_bytes(b"\x7fELFfixture")
            return subprocess.CompletedProcess(argv, 0, "", "")
        source = Path(driver.__file__).with_name("r5-paper-clock-wait.c")
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.object(driver.shutil, "which", return_value="/usr/bin/cc"), \
                mock.patch.object(driver, "checked", side_effect=fake_checked):
            staged, evidence = driver.compile_clock_wait_bridge(Path(directory))
            staged.chmod(0o644)
            staged.write_bytes(b"\x7fELFmutated")
            with self.assertRaisesRegex(driver.FixtureError, "CLOCK_WAIT_BRIDGE_STAGED_HASH_MISMATCH"):
                driver.verify_staged_clock_wait_bridge(source, staged, evidence)

    def test_clock_bridge_preload_is_limited_to_postgres_and_precedes_faketime(self):
        guest = GUEST_SCRIPT_PATH.read_text(encoding="utf-8")
        lines = guest.splitlines()
        bridge_check = next(i for i, line in enumerate(lines) if "/input/r5-paper-clock-wait.so ]" in line)
        initdb = next(i for i, line in enumerate(lines) if '"$pg/initdb"' in line)
        export = next(i for i, line in enumerate(lines) if line == "export LD_PRELOAD=/input/libfaketime.so.1")
        postgres = next(i for i, line in enumerate(lines) if '"$pg/postgres"' in line)
        ready = next(i for i, line in enumerate(lines) if '"$pg/pg_isready"' in line)
        self.assertLess(initdb, export)
        self.assertLess(export, postgres)
        self.assertLess(postgres, ready)
        self.assertIn("LD_PRELOAD=/input/r5-paper-clock-wait.so:/input/libfaketime.so.1", lines[postgres - 1])
        self.assertNotIn("r5-paper-clock-wait.so", lines[ready])
        self.assertIn("--show-output", next(line for line in lines if "elif run_bounded_test /input/tests" in line))

    def test_help_and_no_arguments_have_no_runtime_effects(self):
        self.assertEqual(driver.main([]), 0)


if __name__ == "__main__":
    unittest.main()
