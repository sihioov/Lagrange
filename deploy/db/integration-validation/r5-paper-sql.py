#!/usr/bin/env python3
"""Run the named R5 Paper SQL tests in one disposable, isolated PostgreSQL."""

from __future__ import annotations

import argparse
from collections.abc import Callable, Iterator
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import time
import uuid


ROOT = Path(__file__).resolve().parents[3]
IMAGE = "postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a"
CLOCK_PACKAGE_VERSION = "0.9.10-2.1"
TARGET = "migration_contract"
TEST_SECONDS = 900
CLEANUP_SECONDS = 120
COMMAND_TIMEOUT = 45 * 60
MAX_LOG_BYTES = 2 * 1024 * 1024
MAX_CLOCK_WAIT_BRIDGE_BYTES = 1024 * 1024
CLOCK_WAIT_BRIDGE_SOURCE_SHA256 = "ce33fdcb6f9324a5511e3ba86f3ab3647bd1ff99f9cf4b4011efa409bcb34eec"
CLOCK_WAIT_BRIDGE_NAME = "r5-paper-clock-wait.so"
FAILURE_WITNESS_MAX_ENTRIES = 16
FAILURE_WITNESS_MAX_COORDINATE = 1_000_000
TEST_WITNESS_MARKER = b"R5_PAPER_TEST_WITNESS:"
TEST_WITNESS_STAGES = frozenset(
    {
        "READY_SETUP", "READY_T1_TARGET", "READY_T1_SETTLE", "READY_T2_PREVIEW",
        "READY_T2_WAIT", "READY_T1_VALUE", "READY_TRIAL_APPLY", "READY_TRIAL_READBACK",
        "READY_REVOKE_RETRY", "APPLY_POOL_ACQUIRE", "APPLY_QUERY", "TERMINAL_UPDATE",
    }
)
TEST_WITNESS_OUTCOMES = frozenset(
    {
        "ENTERED", "POOL_PENDING", "QUERY_PENDING", "CLIENT_TIMEOUT", "UPDATE_OK",
        "DATABASE_ERROR", "NON_DATABASE_ERROR",
    }
)
TEST_WITNESS_OPTIONAL_KEYS = frozenset(
    {"sqlstate", "backend_pid", "blocker_pid", "advisory_blocked", "completed_before_unlock"}
)

PANIC_LOCATION_PATHS = (
    "tests/paper_0067.rs",
    "tests/migration_contract.rs",
    "tests/integration/migration-contract/tests/paper_0067.rs",
    "tests/integration/migration-contract/tests/migration_contract.rs",
)
_PANIC_LOCATION_PATH_PATTERN = "|".join(re.escape(path) for path in PANIC_LOCATION_PATHS)
_PANIC_LOCATION_RE = re.compile(
    rf"(?:(?<=/)|^)(?P<path>{_PANIC_LOCATION_PATH_PATTERN}):"
    r"(?P<line>[0-9]{1,7}):(?P<column>[0-9]{1,7})(?=[:\s]|$)",
    re.ASCII,
)
_RUST_PANIC_RE = re.compile(r"\bpanicked at\b", re.ASCII)
_PANICKED_AT_RE = re.compile(r"\bpanicked at\s+", re.ASCII)
_SQLX_DATABASE_DEBUG_RE = re.compile(
    r"\bDatabase\s*\(\s*PgDatabaseError\s*\{(?P<fields>[^}]{0,8192})\}",
    re.ASCII | re.DOTALL,
)
_SQLX_DATABASE_CODE_RE = re.compile(
    r"\bcode:\s*(?:Some\s*\(\s*)?[\"']([A-Z0-9]{5})[\"']",
    re.ASCII,
)
_SQLSTATE_MARKER_RE = re.compile(
    r"\bSQLSTATE\b\s*[:=]\s*[\"']?([A-Z0-9]{5})(?![A-Z0-9])",
    re.ASCII,
)
_FAILURE_CATEGORY_PATTERNS = (
    ("ROW_NOT_FOUND", re.compile(r"\bRowNotFound\b", re.ASCII)),
    ("COLUMN_DECODE", re.compile(r"\bColumnDecode\s*[({]", re.ASCII)),
    ("POOL_TIMED_OUT", re.compile(r"\bPoolTimedOut\b", re.ASCII)),
    ("IO", re.compile(r"\bIo\s*[({]", re.ASCII)),
    ("ASSERTION", re.compile(r"\bassertion(?:\s+`[^`\r\n]{1,256}`)?\s+failed\b", re.ASCII)),
)

BASELINE_TESTS = frozenset(
    {
        "paper_0067::paper_0067_enable_auto_publication_apply_and_revalidation",
        "paper_0067::paper_0067_source_contract_has_snapshot_fingerprint_and_no_trade_path",
        "paper_0067::paper_0067_catalog_reapply_empty_down",
        "paper_0067::paper_0067_calendar_agreement_conflict_coverage_and_observation",
        "paper_0067::paper_0067_configuration_consent_roles_rls_and_history_down",
        "paper_0067::paper_0067_daily_allocation_manual_auto_exclusions_cash_and_invalid_weights",
        "paper_0067::paper_0067_worker_actor_scope_restores_unset_empty_and_foreign_actor",
        "paper_0067::paper_0067_trade_then_unchanged_no_trade_settlement_valuation_atomic_replay",
        "paper_0067::paper_0067_settled_cross_midnight_recovery_values_once_without_reexecuting",
        "paper_0067::paper_0067_manual_apply_concurrency_tokens_stale_publication_and_revocation",
        "paper_0067::paper_0067_0068_sql_lifecycle_source_contract_is_closed",
        "paper_0067::paper_0067_0068_fence_reconcile_offline_readback_and_history_guard",
        "paper_0067::paper_0068_exact_receipt_canonical_roles_legacy_and_down",
        "paper_0067::paper_0068_exact_readback_live_backend_late_commit_and_cancel",
        "paper_0067::paper_0068_enable_reset_times_swaps_and_descriptive_readback",
        "paper_0067::paper_0068_reset_races_enable_control_lock",
        "paper_0067::paper_0068_exact_recovery_v1_v2_receipts",
    }
)
SOURCE_CONTRACT_TEST = "paper_0067::paper_0067_source_contract_has_snapshot_fingerprint_and_no_trade_path"
SOURCE_CONTRACT_TESTS = frozenset({
    SOURCE_CONTRACT_TEST,
    "paper_0067::paper_0067_0068_sql_lifecycle_source_contract_is_closed",
})
IGNORED_BASELINE_TESTS = BASELINE_TESTS - SOURCE_CONTRACT_TESTS


class FixtureError(Exception):
    """A fixed, safe error code; provider/process text is never surfaced."""

    def __init__(self, code: str, exit_status: int = 1):
        super().__init__(code)
        self.code = code
        if exit_status < 0:
            exit_status = min(255, 128 + abs(exit_status))
        self.exit_status = exit_status if 1 <= exit_status <= 255 else 1


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def write_json(path: Path, value: object) -> None:
    data = (json.dumps(value, sort_keys=True, indent=2) + "\n").encode()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)


def parse_artifact(lines: list[str]) -> Path:
    matches: set[str] = set()
    for line in lines:
        try:
            message = json.loads(line)
        except (TypeError, json.JSONDecodeError):
            continue
        if message.get("reason") != "compiler-artifact":
            continue
        target = message.get("target") or {}
        profile = message.get("profile") or {}
        executable = message.get("executable")
        if (
            target.get("name") == TARGET
            and target.get("kind") == ["test"]
            and profile.get("test") is True
            and isinstance(executable, str)
        ):
            matches.add(executable)
    if len(matches) != 1:
        raise FixtureError("TEST_ARTIFACT_MISSING_OR_AMBIGUOUS")
    artifact = Path(matches.pop())
    if not artifact.is_absolute() or not artifact.is_file():
        raise FixtureError("TEST_ARTIFACT_PATH_INVALID")
    with artifact.open("rb") as stream:
        if stream.read(4) != b"\x7fELF":
            raise FixtureError("TEST_ARTIFACT_NOT_ELF")
    return artifact


def parse_test_names(output: str) -> set[str]:
    names: set[str] = set()
    for line in output.splitlines():
        match = re.fullmatch(r"(.+): test", line.strip())
        if match and match.group(1).startswith("paper_0067::"):
            names.add(match.group(1))
    return names


def parse_test_inventory(output: str) -> list[str]:
    names = parse_test_names(output)
    if not BASELINE_TESTS.issubset(names):
        raise FixtureError("TEST_INVENTORY_BASELINE_MISSING")
    if not names:
        raise FixtureError("TEST_INVENTORY_EMPTY")
    return sorted(names)


def _parse_test_witness(value: object) -> dict[str, object] | None:
    if not isinstance(value, dict) or not {"stage", "outcome"}.issubset(value):
        return None
    if set(value) - ({"stage", "outcome"} | TEST_WITNESS_OPTIONAL_KEYS):
        return None
    stage = value["stage"]
    outcome = value["outcome"]
    if not isinstance(stage, str) or stage not in TEST_WITNESS_STAGES:
        return None
    if not isinstance(outcome, str) or outcome not in TEST_WITNESS_OUTCOMES:
        return None
    safe: dict[str, object] = {"stage": stage, "outcome": outcome}
    if "sqlstate" in value:
        code = value["sqlstate"]
        if (
            not isinstance(code, str)
            or not code.isascii()
            or re.fullmatch(r"[A-Z0-9]{5}", code, flags=re.ASCII) is None
        ):
            return None
        safe["sqlstate"] = code
    for key in ("backend_pid", "blocker_pid"):
        if key in value:
            pid = value[key]
            if type(pid) is not int or not (1 <= pid <= 2_147_483_647):
                return None
            safe[key] = pid
    for key in ("advisory_blocked", "completed_before_unlock"):
        if key in value:
            flag = value[key]
            if type(flag) is not bool:
                return None
            safe[key] = flag
    return safe


def validate_test_result(name: str, output: str, exit_code: int) -> None:
    if exit_code != 0:
        raise FixtureError("NAMED_TEST_EXIT_NONZERO")
    escaped = re.escape(name)
    if len(re.findall(rf"^test {escaped}\s+\.\.\. ok$", output, re.MULTILINE)) != 1:
        raise FixtureError("NAMED_TEST_RESULT_MISSING_OR_INVALID")
    summary = re.findall(
        r"^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;",
        output,
        re.MULTILINE,
    )
    if summary != [("1", "0", "0")]:
        raise FixtureError("NAMED_TEST_SUMMARY_INVALID")


def parse_failure_witness(output: bytes, *, input_truncated: bool = False) -> dict[str, object] | None:
    """Extract only fixed failure metadata from an already-private bounded test log."""
    bounded_output = output[:MAX_LOG_BYTES]
    truncated = input_truncated or len(output) > MAX_LOG_BYTES
    text = bounded_output.decode("utf-8", errors="replace")

    test_witnesses: list[dict[str, object]] = []
    seen_test_witnesses: set[str] = set()
    for witness_line in bounded_output.splitlines():
        if not witness_line.startswith(TEST_WITNESS_MARKER):
            continue
        try:
            payload = json.loads(witness_line[len(TEST_WITNESS_MARKER):].decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            continue
        witness = _parse_test_witness(payload)
        if witness is None:
            continue
        encoded = json.dumps(witness, sort_keys=True, separators=(",", ":"))
        if encoded in seen_test_witnesses:
            continue
        if len(test_witnesses) >= FAILURE_WITNESS_MAX_ENTRIES:
            truncated = True
            continue
        seen_test_witnesses.add(encoded)
        test_witnesses.append(witness)

    panic_locations: list[dict[str, object]] = []
    seen_locations: set[tuple[str, int, int]] = set()
    for panic_line in text.splitlines():
        marker = _PANICKED_AT_RE.search(panic_line)
        if marker is None:
            continue
        for match in _PANIC_LOCATION_RE.finditer(panic_line[marker.end():]):
            line = int(match.group("line"))
            column = int(match.group("column"))
            if not (1 <= line <= FAILURE_WITNESS_MAX_COORDINATE):
                continue
            if not (1 <= column <= FAILURE_WITNESS_MAX_COORDINATE):
                continue
            location = (match.group("path"), line, column)
            if location in seen_locations:
                continue
            if len(panic_locations) >= FAILURE_WITNESS_MAX_ENTRIES:
                truncated = True
                continue
            seen_locations.add(location)
            panic_locations.append({"path": location[0], "line": line, "column": column})

    sqlstates: list[str] = []
    seen_sqlstates: set[str] = set()
    database_debug = False
    for match in _SQLX_DATABASE_DEBUG_RE.finditer(text):
        database_debug = True
        code_match = _SQLX_DATABASE_CODE_RE.search(match.group("fields"))
        if code_match is not None:
            code = code_match.group(1)
            if code not in seen_sqlstates:
                if len(sqlstates) >= FAILURE_WITNESS_MAX_ENTRIES:
                    truncated = True
                else:
                    seen_sqlstates.add(code)
                    sqlstates.append(code)
    for match in _SQLSTATE_MARKER_RE.finditer(text):
        code = match.group(1)
        if code not in seen_sqlstates:
            if len(sqlstates) >= FAILURE_WITNESS_MAX_ENTRIES:
                truncated = True
            else:
                seen_sqlstates.add(code)
                sqlstates.append(code)

    categories: list[str] = []
    if database_debug or sqlstates:
        categories.append("DATABASE")
    categories.extend(
        name for name, pattern in _FAILURE_CATEGORY_PATTERNS if pattern.search(text)
    )
    if _RUST_PANIC_RE.search(text):
        categories.append("RUST_PANIC")
    if len(categories) > FAILURE_WITNESS_MAX_ENTRIES:
        categories = categories[:FAILURE_WITNESS_MAX_ENTRIES]
        truncated = True

    if not categories and not panic_locations and not sqlstates and not test_witnesses:
        return None
    result = {
        "categories": categories,
        "panic_locations": panic_locations,
        "sqlstates": sqlstates,
        "truncated": truncated,
    }
    if test_witnesses:
        result["test_witnesses"] = test_witnesses
    return result


def require_ci_environment(environ: dict[str, str]) -> None:
    if environ.get("GITHUB_ACTIONS") != "true":
        raise FixtureError("GITHUB_ACTIONS_REQUIRED")
    runner_temp = environ.get("RUNNER_TEMP")
    target_dir = environ.get("CARGO_TARGET_DIR")
    if not runner_temp or not target_dir:
        raise FixtureError("JOB_PRIVATE_PATHS_REQUIRED")
    temp_input = Path(runner_temp)
    target_input = Path(target_dir)
    if not temp_input.is_absolute() or not target_input.is_absolute():
        raise FixtureError("JOB_PRIVATE_PATHS_NOT_ABSOLUTE")
    temp_path = temp_input.resolve()
    target_path = target_input.resolve()
    if temp_path not in target_path.parents:
        raise FixtureError("CARGO_TARGET_DIR_NOT_JOB_PRIVATE")


def command(argv: list[str], *, timeout: int, cwd: Path | None = None,
            env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    process: subprocess.Popen[str] | None = None

    def stop_owned_group() -> None:
        if process is None:
            return
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.communicate()

    try:
        process = subprocess.Popen(
            argv,
            cwd=cwd,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired as error:
            stop_owned_group()
            raise FixtureError("OWNED_COMMAND_TIMEOUT", exit_status=124) from error
        return subprocess.CompletedProcess(argv, process.returncode, stdout, stderr)
    except KeyboardInterrupt:
        stop_owned_group()
        raise
    except OSError as error:
        raise FixtureError("OWNED_COMMAND_START_FAILED") from error


def checked(argv: list[str], *, timeout: int, cwd: Path | None = None,
            env: dict[str, str] | None = None, code: str) -> subprocess.CompletedProcess[str]:
    result = command(argv, timeout=timeout, cwd=cwd, env=env)
    if result.returncode != 0:
        raise FixtureError(code, exit_status=result.returncode)
    return result


def compile_clock_wait_bridge(input_dir: Path) -> tuple[Path, dict[str, object]]:
    source = Path(__file__).with_name("r5-paper-clock-wait.c")
    if source.is_symlink() or not source.is_file():
        raise FixtureError("CLOCK_WAIT_SOURCE_INVALID")
    source_before = sha256_file(source)
    if source_before != CLOCK_WAIT_BRIDGE_SOURCE_SHA256:
        raise FixtureError("CLOCK_WAIT_SOURCE_HASH_MISMATCH")
    compiler = shutil.which("cc")
    if compiler is None or not Path(compiler).is_absolute():
        raise FixtureError("CLOCK_WAIT_COMPILER_MISSING")
    artifact = input_dir / CLOCK_WAIT_BRIDGE_NAME
    argv = [compiler, "-std=c11", "-O2", "-fPIC", "-shared",
            "-Wl,-z,relro,-z,now", str(source), "-o", str(artifact)]
    checked(argv, timeout=30, cwd=ROOT, code="CLOCK_WAIT_BRIDGE_COMPILE_FAILED")
    source_after = sha256_file(source)
    if source_after != source_before or source_after != CLOCK_WAIT_BRIDGE_SOURCE_SHA256:
        raise FixtureError("CLOCK_WAIT_SOURCE_MUTATED_DURING_COMPILE")
    if artifact.is_symlink() or not artifact.is_file():
        raise FixtureError("CLOCK_WAIT_BRIDGE_OUTPUT_INVALID")
    size = artifact.stat().st_size
    if size < 4 or size > MAX_CLOCK_WAIT_BRIDGE_BYTES:
        raise FixtureError("CLOCK_WAIT_BRIDGE_OUTPUT_INVALID")
    with artifact.open("rb") as stream:
        if stream.read(4) != b"\x7fELF":
            raise FixtureError("CLOCK_WAIT_BRIDGE_OUTPUT_INVALID")
    artifact.chmod(0o444)
    record = {
        "source_sha256": source_before,
        "source_sha256_after_compile": source_after,
        "artifact_sha256": sha256_file(artifact),
        "artifact_bytes": size,
        "artifact_mode": "0444",
        "argv": [Path(compiler).name, "-std=c11", "-O2", "-fPIC", "-shared",
                 "-Wl,-z,relro,-z,now", source.name, "-o", artifact.name],
        "timeout_seconds": 30,
    }
    return artifact, record


def verify_staged_clock_wait_bridge(source: Path, artifact: Path,
                                    record: dict[str, object]) -> None:
    if (source.is_symlink() or not source.is_file()
            or sha256_file(source) != CLOCK_WAIT_BRIDGE_SOURCE_SHA256
            or sha256_file(artifact) != record.get("artifact_sha256")
            or artifact.is_symlink() or not artifact.is_file()
            or artifact.stat().st_mode & 0o777 != 0o444):
        raise FixtureError("CLOCK_WAIT_BRIDGE_STAGED_HASH_MISMATCH")


def test_argv(artifact: Path, test: str) -> list[str]:
    return [str(artifact), "--exact", test, "--include-ignored", "--test-threads=1", "--show-output"]


def discover_clock_library(supplied: Path | None) -> tuple[Path, str, str]:
    version = checked(
        ["dpkg-query", "-W", "-f=${Version}", "libfaketime"],
        timeout=15,
        code="CLOCK_PACKAGE_QUERY_FAILED",
    ).stdout.strip()
    if version != CLOCK_PACKAGE_VERSION:
        raise FixtureError("CLOCK_PACKAGE_VERSION_MISMATCH")
    if supplied is None:
        listing = checked(
            ["dpkg-query", "-L", "libfaketime"],
            timeout=15,
            code="CLOCK_PACKAGE_FILES_FAILED",
        ).stdout.splitlines()
        candidates = [Path(item) for item in listing if Path(item).name == "libfaketime.so.1"]
        if len(candidates) != 1:
            raise FixtureError("CLOCK_LIBRARY_NOT_UNIQUE")
        supplied = candidates[0]
    if not supplied.is_absolute() or not supplied.is_file() or supplied.is_symlink():
        raise FixtureError("CLOCK_LIBRARY_PATH_INVALID")
    owner = checked(
        ["dpkg-query", "-S", str(supplied)],
        timeout=15,
        code="CLOCK_LIBRARY_PACKAGE_OWNER_MISSING",
    ).stdout
    if not any(line.endswith(": " + str(supplied)) for line in owner.splitlines()):
        raise FixtureError("CLOCK_LIBRARY_PACKAGE_OWNER_MISMATCH")
    return supplied.resolve(strict=True), version, sha256_file(supplied)


def parse_image_inspect(output: str) -> dict[str, object]:
    try:
        rows = json.loads(output)
        image = rows[0]
        repo_digests = image["RepoDigests"]
        platform = (image["Os"], image["Architecture"])
        image_id = image["Id"]
    except (ValueError, KeyError, IndexError, TypeError):
        raise FixtureError("POSTGRES_IMAGE_INSPECT_INVALID") from None
    if IMAGE not in repo_digests or platform != ("linux", "amd64") or not re.fullmatch(
        r"sha256:[0-9a-f]{64}", image_id
    ):
        raise FixtureError("POSTGRES_IMAGE_PIN_OR_PLATFORM_MISMATCH")
    return {"reference": IMAGE, "image_id": image_id, "platform": "linux/amd64"}


def inspect_container(docker: "Docker", reference: str) -> dict[str, object] | None:
    result = docker.call(["container", "inspect", reference], timeout=20, allow_missing=True)
    if result is None:
        return None
    try:
        rows = json.loads(result)
        item = rows[0]
        labels = item["Config"]["Labels"] or {}
        state = item["State"]
        identity = item["Id"]
        name = item["Name"].lstrip("/")
    except (ValueError, KeyError, IndexError, TypeError):
        raise FixtureError("CONTAINER_INSPECT_INVALID") from None
    return {"id": identity, "name": name, "labels": labels, "state": state}


def reconcile_failed_create(docker: "Docker", name: str, nonce: str,
                            original: FixtureError) -> tuple[str | None, FixtureError]:
    """Retain a failed create's exact intent and recover only its matching resource."""
    return docker.reconcile_intent(name, nonce), original


@contextmanager
def defer_cleanup_signals(on_interrupt: Callable[[], None]) -> Iterator[None]:
    """Finish exact owned cleanup/reporting before acting on cancellation."""
    signals = (signal.SIGINT, signal.SIGTERM)
    mask = signal.pthread_sigmask(signal.SIG_BLOCK, signals)
    previous = {kind: signal.getsignal(kind) for kind in signals}

    def defer(_signum: int, _frame: object) -> None:
        on_interrupt()

    try:
        for kind in signals:
            signal.signal(kind, defer)
    finally:
        signal.pthread_sigmask(signal.SIG_SETMASK, mask)
    try:
        yield
    finally:
        mask = signal.pthread_sigmask(signal.SIG_BLOCK, signals)
        try:
            for kind, handler in previous.items():
                signal.signal(kind, handler)
        finally:
            signal.pthread_sigmask(signal.SIG_SETMASK, mask)


def first_error(primary: FixtureError | None, secondary: list[str]) -> FixtureError | None:
    """A cleanup/logging failure may not replace an earlier task failure."""
    if primary is not None:
        return primary
    return FixtureError(secondary[0]) if secondary else None


def close_owned_container(docker: "Docker", identity: str, name: str,
                          nonce: str, secondary: list[str]) -> bool:
    try:
        docker.cleanup(identity, name, nonce)
        return True
    except FixtureError as error:
        secondary.append(error.code)
        return False


class Docker:
    cleanup_deadline: float | None = None

    def call(self, args: list[str], *, timeout: int,
             allow_missing: bool = False) -> str | None:
        if self.cleanup_deadline is not None:
            remaining = self.cleanup_deadline - time.monotonic()
            bounded_remaining = int(remaining) - 6
            if bounded_remaining < 1:
                raise FixtureError("CLEANUP_DEADLINE_EXCEEDED")
            timeout = min(timeout, bounded_remaining)
        result = command(["docker", *args], timeout=timeout)
        if result.returncode == 0:
            return result.stdout.strip()
        if allow_missing and result.returncode == 1:
            absent = "no such object" in result.stderr.lower() or "no such container" in result.stderr.lower()
            if absent:
                return None
        raise FixtureError("DOCKER_COMMAND_FAILED", exit_status=result.returncode)

    def reconcile_intent(self, name: str, nonce: str) -> str | None:
        item = inspect_container(self, name)
        if item is None:
            return None
        if (item["name"] != name
                or item["labels"].get("lagrange.r5-paper.nonce") != nonce
                or not re.fullmatch(r"[0-9a-f]{64}", str(item["id"]))):
            raise FixtureError("CONTAINER_INTENT_IDENTITY_MISMATCH")
        return str(item["id"])

    def assert_owned(self, identity: str, name: str, nonce: str) -> dict[str, object] | None:
        item = inspect_container(self, identity)
        if item is None:
            return None
        if (item["id"] != identity or item["name"] != name
                or item["labels"].get("lagrange.r5-paper.nonce") != nonce):
            raise FixtureError("CONTAINER_OWNERSHIP_MISMATCH")
        return item

    def cleanup(self, identity: str, name: str, nonce: str) -> None:
        item = self.assert_owned(identity, name, nonce)
        if item is None:
            return
        if item["state"].get("Running") is True:
            self.call(["stop", "--time", "20", identity], timeout=35)
        item = self.assert_owned(identity, name, nonce)
        if item is None or item["state"].get("Running") is True:
            raise FixtureError("CONTAINER_TERMINAL_STATE_UNPROVEN")
        self.call(["rm", identity], timeout=20)
        if self.assert_owned(identity, name, nonce) is not None:
            raise FixtureError("CONTAINER_REMOVAL_UNPROVEN")


def copy_bounded(source: Path, destination: Path, total: int) -> int:
    if not source.is_file() or source.is_symlink():
        return total
    destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    allowance = min(MAX_LOG_BYTES, max(0, MAX_LOG_BYTES * 8 - total))
    written = 0
    with source.open("rb") as incoming, destination.open("wb") as outgoing:
        while written < allowance:
            block = incoming.read(min(64 * 1024, allowance - written))
            if not block:
                break
            outgoing.write(block)
            written += len(block)
    os.chmod(destination, 0o600)
    return total + written


def read_receipt(path: Path, pattern: str) -> int | None:
    if not path.is_file() or path.is_symlink():
        return None
    value = path.read_text(encoding="ascii").strip()
    if not re.fullmatch(pattern, value):
        return None
    return int(value)


def new_report(nonce: str) -> dict[str, object]:
    return {
        "schema": "r5-paper-sql-fixture-result-v1",
        "nonce": nonce,
        "status": "RUNNING",
        "error_code": None,
        "selected_tests": [],
        "test_results": [],
        "container": {"intent": "NOT_STARTED", "terminal": "UNKNOWN", "removed": "UNKNOWN"},
        "postgres": {"exit_code": None, "reaped": "UNKNOWN"},
        "stages": {"input": "UNKNOWN", "output": "UNKNOWN"},
        "logs": "PRIVATE_BOUNDED",
    }


def run_fixture(evidence_dir: Path, supplied_clock: Path | None) -> int:
    require_ci_environment(dict(os.environ))
    if not evidence_dir.is_absolute():
        raise FixtureError("EVIDENCE_PATH_MUST_BE_ABSOLUTE")
    evidence_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    if evidence_dir.is_symlink() or not evidence_dir.is_dir():
        raise FixtureError("EVIDENCE_PATH_INVALID")
    runner_temp = Path(os.environ["RUNNER_TEMP"]).resolve(strict=True)
    evidence_resolved = evidence_dir.resolve(strict=True)
    if runner_temp not in evidence_resolved.parents:
        raise FixtureError("EVIDENCE_PATH_OUTSIDE_RUNNER_TEMP")
    os.chmod(evidence_dir, 0o700)
    nonce = uuid.uuid4().hex
    name = "lagrange-r5-paper-" + nonce
    report = new_report(nonce)
    result_path = evidence_dir / "result.json"
    work = evidence_dir / (".r5-paper-sql-" + nonce)
    work.mkdir(mode=0o700)
    input_dir = work / "input"
    output_dir = work / "output"
    private_logs = work / "logs"
    input_dir.mkdir(mode=0o700)
    output_dir.mkdir(mode=0o700)
    private_logs.mkdir(mode=0o700)
    docker = Docker()
    intent_written = False
    container_id: str | None = None
    primary_error: FixtureError | None = None
    secondary_errors: list[str] = []
    test_started: float | None = None
    selected: list[str] = []

    def save() -> None:
        write_json(result_path, report)

    save()
    try:
        library, clock_version, clock_hash = discover_clock_library(supplied_clock)
        report["clock"] = {"package_version": clock_version, "sha256": clock_hash}
        target_dir = Path(os.environ["CARGO_TARGET_DIR"]).resolve(strict=True)
        cargo_env = dict(os.environ)
        cargo_env.update({"CARGO_BUILD_JOBS": "2", "CARGO_TERM_COLOR": "never", "RUSTUP_AUTO_INSTALL": "0"})
        build_argv = [
            "cargo", "+1.97.1", "test", "--package", "migration-contract", "--test",
            "migration_contract", "--locked", "--no-run", "--message-format=json",
        ]
        build = checked(build_argv, timeout=COMMAND_TIMEOUT, cwd=ROOT, env=cargo_env,
                        code="RUST_TEST_BUILD_FAILED")
        (private_logs / "cargo.stdout.log").write_text(build.stdout[:MAX_LOG_BYTES])
        (private_logs / "cargo.stderr.log").write_text(build.stderr[:MAX_LOG_BYTES])
        (private_logs / "cargo.stdout.log").chmod(0o600)
        (private_logs / "cargo.stderr.log").chmod(0o600)
        artifact = parse_artifact(build.stdout.splitlines())
        try:
            artifact.relative_to(target_dir)
        except ValueError:
            raise FixtureError("TEST_ARTIFACT_OUTSIDE_JOB_TARGET") from None
        bridge_source = Path(__file__).with_name("r5-paper-clock-wait.c")
        bridge_artifact, bridge_record = compile_clock_wait_bridge(input_dir)
        verify_staged_clock_wait_bridge(bridge_source, bridge_artifact, bridge_record)
        report["clock_wait_bridge"] = bridge_record
        save()
        listing = checked([str(artifact), "--list", "--include-ignored"], timeout=30,
                          code="TEST_INVENTORY_COMMAND_FAILED")
        selected = parse_test_inventory(listing.stdout)
        ignored_listing = checked([str(artifact), "--list", "--ignored"], timeout=30,
                                  code="IGNORED_TEST_INVENTORY_COMMAND_FAILED")
        ordinary_listing = checked([str(artifact), "--list"], timeout=30,
                                   code="ORDINARY_TEST_INVENTORY_COMMAND_FAILED")
        ignored_names = parse_test_names(ignored_listing.stdout)
        ordinary_names = parse_test_names(ordinary_listing.stdout)
        if not IGNORED_BASELINE_TESTS.issubset(ignored_names):
            raise FixtureError("IGNORED_TEST_BASELINE_MISSING")
        if not SOURCE_CONTRACT_TESTS.issubset(ordinary_names) or SOURCE_CONTRACT_TESTS & ignored_names:
            raise FixtureError("ORDINARY_SOURCE_CONTRACT_TEST_MISSING")
        invocation = test_argv(artifact, "<FULL_QUALIFIED_NAME>")
        report["selected_tests"] = [{"name": test, "ignored": test in ignored_names,
            "argv": test_argv(artifact, test)}
            for test in selected]
        report["test_results"] = [{"name": test, "exit_code": None, "result": "UNKNOWN"}
                                   for test in selected]
        report["inventory_argv"] = [str(artifact), "--list", "--include-ignored"]
        report["ignored_inventory_argv"] = [str(artifact), "--list", "--ignored"]
        report["ordinary_inventory_argv"] = [str(artifact), "--list"]
        report["cargo_argv"] = build_argv
        report["test_argv_template"] = invocation
        report["artifact"] = {"target": TARGET, "sha256": sha256_file(artifact)}
        save()

        checked(["docker", "pull", IMAGE], timeout=180, code="POSTGRES_IMAGE_PULL_FAILED")
        image_result = checked(["docker", "image", "inspect", IMAGE], timeout=30,
                               code="POSTGRES_IMAGE_INSPECT_FAILED")
        report["image"] = parse_image_inspect(image_result.stdout)
        save()

        copied = ((artifact, input_dir / "tests", 0o555),
                  (Path(__file__).with_name("r5-paper-sql-guest.sh"), input_dir / "guest.sh", 0o444),
                  (library, input_dir / "libfaketime.so.1", 0o444))
        for source, destination, mode in copied:
            shutil.copyfile(source, destination)
            if sha256_file(source) != sha256_file(destination):
                raise FixtureError("STAGED_INPUT_HASH_MISMATCH")
            destination.chmod(mode)
        inventory = ("".join(test + "\n" for test in selected)).encode()
        inventory_path = input_dir / "selected-tests.txt"
        inventory_path.write_bytes(inventory)
        inventory_path.chmod(0o444)
        if inventory_path.read_bytes() != inventory:
            raise FixtureError("STAGED_TEST_INVENTORY_MISMATCH")
        report["test_inventory_sha256"] = hashlib.sha256(inventory).hexdigest()
        os.chmod(input_dir, 0o755)
        verify_staged_clock_wait_bridge(bridge_source, bridge_artifact, bridge_record)
        if any("," in str(path) for path in (input_dir, output_dir)):
            raise FixtureError("DOCKER_MOUNT_PATH_UNSUPPORTED")
        os.chmod(output_dir, 0o777)
        report["container"] = {"intent": "RECORDED", "name": name,
                               "terminal": "UNKNOWN", "removed": "UNKNOWN"}
        report["stages"] = {"input": "STAGED_AND_HASHED", "output": "UID999_PRIVATE_LEAF"}
        save()
        intent_written = True
        existing = inspect_container(docker, name)
        if existing is not None:
            raise FixtureError("CONTAINER_NAME_COLLISION")
        create_argv = [
            "create", "--name", name, "--label", "lagrange.r5-paper.nonce=" + nonce,
            "--network", "none", "--env", "R5_03_ISOLATED_NONCE=" + nonce,
            "--user", "999:999", "--read-only", "--cap-drop", "ALL",
            "--security-opt", "no-new-privileges", "--init", "--cpus", "2",
            "--memory", "1g", "--memory-swap", "1g", "--pids-limit", "128",
            "--log-opt", "max-size=2m", "--log-opt", "max-file=1",
            "--tmpfs", "/tmp:rw,nosuid,nodev,size=64m,mode=1777",
            "--tmpfs", "/var/lib/postgresql:rw,nosuid,nodev,size=512m,uid=999,gid=999,mode=0700",
            "--mount", f"type=bind,source={input_dir},target=/input,readonly",
            "--mount", f"type=bind,source={output_dir},target=/output",
            "--entrypoint", "/bin/sh", IMAGE, "/input/guest.sh",
        ]
        # Record any creator failure as primary before fallible intent recovery.
        created = docker.call(create_argv, timeout=30)
        if not isinstance(created, str) or not re.fullmatch(r"[0-9a-f]{64}", created):
            raise FixtureError("CONTAINER_CREATE_ID_INVALID")
        container_id = created
        item = docker.assert_owned(container_id, name, nonce)
        if item is None:
            raise FixtureError("CONTAINER_CREATE_NOT_INSPECTABLE")
        report["container"].update({"id": container_id, "intent": "RECONCILED"})
        docker.call(["start", container_id], timeout=30)
        test_started = time.monotonic()
        wait_result = command(["docker", "wait", container_id], timeout=TEST_SECONDS)
        elapsed = time.monotonic() - test_started
        if wait_result.returncode != 0 or not re.fullmatch(r"\d+", wait_result.stdout.strip()):
            raise FixtureError("CONTAINER_WAIT_FAILED")
        docker_exit = int(wait_result.stdout.strip())
        item = docker.assert_owned(container_id, name, nonce)
        if item is None or item["state"].get("Running") is True:
            raise FixtureError("CONTAINER_TERMINAL_STATE_UNPROVEN")
        if item["state"].get("ExitCode") != docker_exit or item["state"].get("OOMKilled") is not False:
            raise FixtureError("CONTAINER_EXIT_STATE_MISMATCH")
        report["container"].update({"terminal": "CONFIRMED", "exit_code": docker_exit,
                                    "elapsed_seconds": round(elapsed, 3)})
        report["test_results"] = []
        total_logs = 0
        for index, test in enumerate(selected, 1):
            log_path = output_dir / f"test-{index:02d}.log"
            exit_path = output_dir / f"test-{index:02d}-exit-code"
            destination = private_logs / f"test-{index:02d}.log"
            total_logs = copy_bounded(log_path, destination, total_logs)
            try:
                input_truncated = (
                    not log_path.is_file()
                    or log_path.is_symlink()
                    or not destination.is_file()
                    or log_path.stat().st_size > destination.stat().st_size
                )
            except OSError:
                input_truncated = True
            exit_code = read_receipt(exit_path, r"\d+")
            outcome = {"name": test, "exit_code": exit_code, "result": "UNKNOWN"}
            output_bytes: bytes | None = None
            try:
                if exit_code is None or not destination.is_file():
                    raise FixtureError("NAMED_TEST_RECEIPT_MISSING")
                output_bytes = destination.read_bytes()[:MAX_LOG_BYTES]
                output = output_bytes.decode("utf-8", errors="replace")
                validate_test_result(test, output, exit_code)
                outcome["result"] = "PASS"
            except FixtureError as error:
                outcome["result"] = "FAIL"
                outcome["error_code"] = error.code
                if output_bytes is not None:
                    try:
                        witness = parse_failure_witness(
                            output_bytes,
                            input_truncated=input_truncated,
                        )
                    except Exception:
                        witness = None
                    if witness is not None:
                        outcome["failure_witness"] = witness
            report["test_results"].append(outcome)
        pg_exit = read_receipt(output_dir / "pg-exit-code", r"\d+")
        reaped = (output_dir / "pg-reaped").is_file() and (output_dir / "pg-reaped").read_text().strip() == "true"
        report["postgres"] = {"exit_code": pg_exit, "reaped": "CONFIRMED" if reaped else "UNKNOWN"}
        for leaf in ("pg-init.log", "postgres.log"):
            total_logs = copy_bounded(output_dir / leaf, private_logs / leaf, total_logs)
        if any(item["result"] != "PASS" for item in report["test_results"]):
            failed_status = next((item["exit_code"] for item in report["test_results"]
                                  if item["result"] == "FAIL" and item["exit_code"] not in (None, 0)), 1)
            raise FixtureError("ONE_OR_MORE_NAMED_TESTS_FAILED", exit_status=failed_status)
        if len(report["test_results"]) != len(selected) or docker_exit != 0 or pg_exit != 0 or not reaped:
            raise FixtureError("FIXTURE_TERMINAL_RECEIPT_INVALID")
    except FixtureError as error:
        primary_error = error
    except KeyboardInterrupt:
        primary_error = FixtureError("INTERRUPTED", exit_status=130)
    except Exception as error:  # preserve typed result without exposing exception text
        primary_error = FixtureError("UNEXPECTED_DRIVER_FAILURE")
        primary_error.__cause__ = error
    finally:
        def note_cleanup_interrupt() -> None:
            nonlocal primary_error
            if primary_error is None:
                primary_error = first_error(None, secondary_errors)
                if primary_error is None:
                    primary_error = FixtureError("INTERRUPTED", exit_status=130)
                    return
            if "CLEANUP_INTERRUPTED" not in secondary_errors:
                secondary_errors.append("CLEANUP_INTERRUPTED")

        with defer_cleanup_signals(note_cleanup_interrupt):
            docker.cleanup_deadline = time.monotonic() + CLEANUP_SECONDS
            if intent_written and container_id is None:
                try:
                    if primary_error is None:
                        container_id = docker.reconcile_intent(name, nonce)
                    else:
                        container_id, _ = reconcile_failed_create(
                            docker, name, nonce, primary_error
                        )
                    if container_id is None:
                        report["container"]["intent"] = "NO_RESOURCE_CONFIRMED"
                except FixtureError as error:
                    secondary_errors.append(error.code)
                    report["container"]["intent"] = "UNRESOLVED"
            log_total = 0
            if output_dir.exists():
                output_names = ["pg-init.log", "postgres.log"] + [
                    f"test-{index:02d}.log" for index in range(1, len(selected) + 1)
                ]
                for output_name in output_names:
                    try:
                        log_total = copy_bounded(output_dir / output_name, private_logs / output_name, log_total)
                    except OSError:
                        secondary_errors.append("PRIVATE_LOG_CAPTURE_FAILED")
            if container_id is not None:
                try:
                    # Keep a bounded private copy of provider-free container logs.
                    log_timeout = min(20, int(docker.cleanup_deadline - time.monotonic()) - 6)
                    if log_timeout < 1:
                        raise FixtureError("CLEANUP_DEADLINE_EXCEEDED")
                    log_result = command(["docker", "logs", container_id], timeout=log_timeout)
                    (private_logs / "docker.stdout.log").write_text(log_result.stdout[:MAX_LOG_BYTES])
                    (private_logs / "docker.stderr.log").write_text(log_result.stderr[:MAX_LOG_BYTES])
                    (private_logs / "docker.stdout.log").chmod(0o600)
                    (private_logs / "docker.stderr.log").chmod(0o600)
                    if log_result.returncode != 0:
                        secondary_errors.append("CONTAINER_LOG_COLLECTION_FAILED")
                except FixtureError as error:
                    secondary_errors.append(error.code)
                except OSError:
                    secondary_errors.append("PRIVATE_LOG_CAPTURE_FAILED")
                if close_owned_container(docker, container_id, name, nonce, secondary_errors):
                    report["container"]["removed"] = "CONFIRMED"
                else:
                    report["container"]["removed"] = "UNKNOWN"
            else:
                report["container"]["removed"] = (
                    "NOT_CREATED"
                    if not intent_written or report["container"].get("intent") == "NO_RESOURCE_CONFIRMED"
                    else "UNKNOWN"
                )
            for path in (input_dir, output_dir):
                try:
                    if path.exists():
                        shutil.rmtree(path)
                    if path.exists():
                        raise OSError("stage remains")
                    report["stages"]["input" if path == input_dir else "output"] = "REMOVED_CONFIRMED"
                except OSError:
                    secondary_errors.append("STAGE_DISPOSITION_UNPROVEN")
                    report["stages"]["input" if path == input_dir else "output"] = "UNKNOWN"
            if work.exists():
                try:
                    # Logs remain in this mode-0700 directory; only staging leaves are removed.
                    os.chmod(work, 0o700)
                except OSError:
                    secondary_errors.append("PRIVATE_LOG_DIRECTORY_MODE_UNPROVEN")
            primary_error = first_error(primary_error, secondary_errors)
            # At most the first cancellation and one secondary marker can arrive
            # during save. Re-persist when either changes; handlers stay deferred.
            while True:
                observed_error = primary_error
                observed_secondary = tuple(secondary_errors)
                report["secondary_error_codes"] = secondary_errors
                report["error_code"] = primary_error.code if primary_error else None
                report["exit_code"] = primary_error.exit_status if primary_error else 0
                report["status"] = "PASS" if primary_error is None else "FAIL"
                save()
                if primary_error is observed_error and tuple(secondary_errors) == observed_secondary:
                    break
    return 0 if primary_error is None else primary_error.exit_status


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", type=Path)
    parser.add_argument("--clock-library", type=Path)
    args = parser.parse_args(argv)
    if args.evidence_dir is None:
        parser.print_help()
        return 0
    def interrupt(_signum: int, _frame: object) -> None:
        raise KeyboardInterrupt

    signal.signal(signal.SIGINT, interrupt)
    signal.signal(signal.SIGTERM, interrupt)
    try:
        return run_fixture(args.evidence_dir, args.clock_library)
    except FixtureError as error:
        print(json.dumps({"status": "FAIL", "error_code": error.code,
                          "exit_code": error.exit_status}, sort_keys=True), file=sys.stderr)
        return error.exit_status


if __name__ == "__main__":
    raise SystemExit(main())
