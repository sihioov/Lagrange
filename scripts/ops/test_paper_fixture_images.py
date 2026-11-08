#!/usr/bin/env python3
"""Bounded fake-orchestration checks; no Docker, Cargo, DB, or product image runs."""

from __future__ import annotations

import json
import ast
import contextlib
import io
import hashlib
import os
import pathlib
import re
import stat
import subprocess
import sys
import tempfile
import time
import types
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/ops/build-paper-fixture-images.sh"
COMMIT = "1" * 40
MARKER = "exec /usr/bin/env -i PATH=/usr/bin:/bin:/usr/local/bin LANG=C.UTF-8 python3 - \"$@\" <<'PY'\n"
PRODUCER_PHASES = (
    "NATIVE_IDENTITY", "CURRENT_K", "REQUEST_K", "STALE_K",
    "PRIVATE_OUTPUT", "RECIPE", "ATTEMPT", "OUTPUT_PRODUCER",
    "OUTPUT_NATIVE", "VERIFY_PRODUCER", "NO_CLOBBER", "MOVE",
    "NATIVE_IDENTITY_COMPARE", "NATIVE_IDENTITY_INSTALL", "OUTPUT_PATH",
)
CAPTURE_ATTEMPT_PHASES = (
    "SHELL_VALIDATE", "PYTHON_SETUP", "HISTORY_VALIDATE", "PRIOR_ARCHIVE",
    "ACTIVE_LOG_OPEN", "CHILD_PENDING", "CHILD_RETURNED", "CHILD_SPAWN_ERROR",
    "LOG_SYNC", "COMPLETED_ARCHIVE", "COMPLETED_RECEIPT", "COMPLETE",
)


def extract_bash_function(source: str, name: str, next_name: str) -> str:
    start_match = re.search(rf"(?m)^{re.escape(name)}\(\) \{{\n", source)
    if start_match is None:
        raise AssertionError(f"missing bounded shell function {name}")
    end_match = re.search(rf"(?m)^{re.escape(next_name)}\(\) \{{\n", source[start_match.end():])
    if end_match is None:
        raise AssertionError(f"missing shell function boundary after {name}")
    return source[start_match.start():start_match.end() + end_match.start()]


def run_rbl_produce_bin_case(case: str, state_root: pathlib.Path,
                             *, private_helper: bool = True,
                             fail_marker_write: bool = False):
    layout_path = ROOT / "scripts/ops/lib/release-build-layout.sh"
    layout = layout_path.read_text(encoding="utf-8")
    producer = extract_bash_function(layout, "rbl_produce_bin", "rbl_entry_hash")
    marker_start = re.search(r"(?m)^rbl_producer_phase_marker\(\) \{\n", layout)
    marker = (extract_bash_function(layout, "rbl_producer_phase_marker", "rbl_produce_bin")
              if marker_start else "")
    env = dict(os.environ)
    env.update({
        "TEST_CASE": case,
        "TEST_STATE_ROOT": str(state_root),
        "RELEASE_BUILD_LAYOUT_SOURCE_ROOT": str(state_root / "source"),
        "RELEASE_BUILD_LAYOUT_STATE_ROOT": str(state_root),
        "RELEASE_BUILD_LAYOUT_COMMIT": COMMIT,
        "RELEASE_BUILD_LAYOUT_CACHE_NAMESPACE": "fixture-cache",
        "RELEASE_BUILD_LAYOUT_HELPER_SHA256": "a" * 64,
        "RELEASE_BUILD_LAYOUT_CONFIG_SHA256": "b" * 64,
        "RBL_PLATFORM": "linux/amd64",
        "FAIL_MARKER_WRITE": "1" if fail_marker_write else "0",
    })
    if private_helper:
        env["PAPER_STATE_ROOT"] = str(state_root)
    else:
        env.pop("PAPER_STATE_ROOT", None)
    stderr_path = state_root / "logs/helper-server.stderr.log"
    stderr_path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    fd = os.open(stderr_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    script = r'''set +e
state=$TEST_STATE_ROOT
RBL_VERIFY_CALLS=0
RBL_PRIVATE_CALLS=0
rbl_validate_native_identity() { [ "$TEST_CASE" != NATIVE_IDENTITY ] || return 17; return 0; }
rbl_k_hash() { [ "$TEST_CASE" != CURRENT_K ] || return 19; printf 'current-k\n'; }
rbl_request_value() {
  if [ "$2" = k_sha256 ]; then
    [ "$TEST_CASE" != REQUEST_K ] || return 23
    if [ "$TEST_CASE" = STALE_K ]; then printf 'stale-k\n'; else printf 'current-k\n'; fi
  else printf 'fixture-value\n'; fi
}
rbl_verify_producer() {
  [ "$TEST_CASE" != VERIFY_PRODUCER ] || return 41
  if [ "$TEST_CASE" = NO_CLOBBER ]; then
    RBL_VERIFY_CALLS=$((RBL_VERIFY_CALLS + 1))
    [ "$RBL_VERIFY_CALLS" -ne 1 ] || return 1
  fi
  return 0
}
rbl_private_dir() {
  if [ "$TEST_CASE" = PRIVATE_OUTPUT ] && [ "$RBL_PRIVATE_CALLS" = 0 ]; then
    RBL_PRIVATE_CALLS=1; return 29
  fi
  RBL_PRIVATE_CALLS=1
  mkdir -p -- "$1" >/dev/null 2>&1
}
rbl_recipe_json() { [ "$TEST_CASE" != RECIPE ] || return 31; printf '{"package":"fixture-package","compile_commit":"present"}\n'; }
python3() {
  case "$*" in
    *'["package"]'*) printf 'fixture-package\n' ;;
    *'["compile_commit"]'*) printf 'present\n' ;;
    *) return 127 ;;
  esac
}
rbl_capture_attempt_log() {
  printf '%s\n' "$@" >"$TEST_STATE_ROOT/attempt-argv.txt"
  [ "$TEST_CASE" != ATTEMPT ] || return 37
  mkdir -p -- "$rbl_partial/.release-build" >/dev/null 2>&1 || return 90
  if [ "$TEST_CASE" != OUTPUT_PRODUCER ]; then
    mkdir -p -- "$rbl_partial/.release-build/producer/api-server" >/dev/null 2>&1 || return 91
  fi
  if [ "$TEST_CASE" != OUTPUT_NATIVE ]; then
    : >"$rbl_partial/.release-build/native-identity.json" || return 92
  fi
  return 0
}
rbl_tree_hash() { case "$1" in *'/.partial-'*) printf 'new-tree\n';; *) printf 'old-tree\n';; esac; }
rbl_die() { printf 'fixture-diagnostic\n' >&2 || :; return 1; }
if [ "$FAIL_MARKER_WRITE" = 1 ]; then
  printf() { [ "$1" != 'RBL_PRODUCE_BIN_PHASE|%s|%s\n' ] || return 88; builtin printf "$@"; }
fi
if [ "$TEST_CASE" = OUTPUT_PATH ]; then
  printf() { if [ "$1" = '%s\n' ] && [ "${2-}" = "$rbl_output" ]; then return 46; fi; builtin printf "$@"; }
fi
docker() { return 0; }
mv() { [ "$TEST_CASE" != MOVE ] || return 43; return 0; }
cmp() { [ "$TEST_CASE" != NATIVE_IDENTITY_COMPARE ] || return 44; return 0; }
install() { [ "$TEST_CASE" != NATIVE_IDENTITY_INSTALL ] || return 45; return 0; }
''' + marker + producer + r'''
if [ "$TEST_CASE" = NO_CLOBBER ] || [ "$TEST_CASE" = REUSE ]; then
  mkdir -p -- "$state/producers/D1/api-server" >/dev/null 2>&1
  : >"$state/producers/D1/api-server/old"
fi
if [ "$TEST_CASE" = NATIVE_IDENTITY_COMPARE ]; then : >"$state/native-identity.json"; fi
result=$(rbl_produce_bin D1 api-server "$state/context" "$state")
status=$?
printf 'RETURN=%s\n' "$status"
printf 'VALUE=%s\n' "$result"
'''
    with os.fdopen(fd, "wb") as stderr_handle:
        result = subprocess.run(["bash", "--noprofile", "--norc", "-c", script],
                                env=env, stdout=subprocess.PIPE, stderr=stderr_handle,
                                text=True, timeout=5, check=False)
    result.stderr = stderr_path.read_text(encoding="ascii")
    return result


def run_rbl_capture_attempt_case(case: str, state_root: pathlib.Path):
    layout_path = ROOT / "scripts/ops/lib/release-build-layout.sh"
    layout = layout_path.read_text(encoding="utf-8")
    marker = extract_bash_function(layout, "rbl_capture_attempt_marker", "rbl_capture_attempt_log")
    capture = extract_bash_function(layout, "rbl_capture_attempt_log", "rbl_produce_native")
    stderr_path = state_root / "logs/helper-server.stderr.log"
    stderr_path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    fd = os.open(stderr_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    if case == "PRECHILD":
        bad_history = state_root / "logs/attempt-history/producer-D1-api-server"
        bad_history.mkdir(mode=0o755, parents=True)
        bad_history.chmod(0o755)
    prefix = r'''set +e
PAPER_STATE_ROOT=$TEST_STATE_ROOT
export PAPER_STATE_ROOT
rbl_abs_path_ok() { [ "$1" = "$(realpath -m -- "$1")" ]; }
rbl_private_dir() { [ -d "$1" ] || mkdir -m 700 -- "$1"; }
rbl_die() { return 1; }
if [ "$TEST_CASE" = MARKER_WRITE ]; then
  printf() { case "$1" in 'RBL_CAPTURE_ATTEMPT|%s|%s\n') return 88;; esac; builtin printf "$@"; }
fi
'''
    suffix = r'''
label=producer-D1-api-server
[ "$TEST_CASE" != OUTSIDE ] || label=native-identity
if [ "$TEST_CASE" = CHILD1 ] || [ "$TEST_CASE" = MARKER_WRITE ]; then
  rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'exit 1'
elif [ "$TEST_CASE" = SPAWN ]; then
  rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" lagrange-no-such-producer-command
elif [ "$TEST_CASE" = POSTCHILD ]; then
  rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'rmdir "$1" && : > "$1"' fixture "$TEST_STATE_ROOT/logs/attempt-history/$label/objects"
else
  rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'exit 0'
fi
status=$?
builtin printf 'RETURN=%s\n' "$status"
'''
    script = prefix + marker + capture + suffix
    with os.fdopen(fd, "wb") as stderr_handle:
        result = subprocess.run(
            ["bash", "--noprofile", "--norc", "-c", script],
            env={**os.environ, "TEST_CASE": case, "TEST_STATE_ROOT": str(state_root)},
            stdout=subprocess.PIPE, stderr=stderr_handle, text=True, timeout=8, check=False,
        )
    result.stderr = stderr_path.read_text(encoding="utf-8")
    return result


def run_closed_child_capture_case(case: str, state_root: pathlib.Path):
    """Run the actual capture function with an inert shell child and private stderr."""
    layout = (ROOT / "scripts/ops/lib/release-build-layout.sh").read_text(encoding="utf-8")
    marker = extract_bash_function(layout, "rbl_capture_attempt_marker", "rbl_capture_attempt_log")
    capture = extract_bash_function(layout, "rbl_capture_attempt_log", "rbl_produce_native")
    stderr_path = state_root / "logs/helper-server.stderr.log"
    stderr_path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    fd = os.open(stderr_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    script = r'''set +e
PAPER_STATE_ROOT=$TEST_STATE_ROOT
export PAPER_STATE_ROOT
rbl_abs_path_ok() { [ "$1" = "$(realpath -m -- "$1")" ]; }
rbl_private_dir() { [ -d "$1" ] || mkdir -m 700 -- "$1"; }
rbl_die() { return 1; }
'''
    suffix = r'''
label=producer-D1-api-server
case "$TEST_CASE" in
  TYPED) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" cargo-json-invalid; exit 1' ;;
  VALUE_ERROR) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" "ValueError: cargo-json-invalid"; exit 1' ;;
  ALIAS) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" "release-build-layout: requested Cargo executable is missing"; exit 1' ;;
  DYNAMIC) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" "release-build-layout: Cargo build failed for api-server"; exit 1' ;;
  PREFIX) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" "#12 0.123 cargo-json-invalid"; exit 1' ;;
  PREFIX_ALIAS) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" "#12 0.123 release-build-layout: requested Cargo executable is missing"; exit 1' ;;
  NATIVE) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" native-identity-schema-invalid; exit 1' ;;
  FEATURE) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" feature-request-schema-invalid; exit 1' ;;
  METADATA) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" cargo-metadata-shape-invalid; exit 1' ;;
  BRACKETED) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" "#12 [0.123] cargo-json-invalid"; exit 1' ;;
  MISSING_ELAPSED) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" "#12 cargo-json-invalid"; exit 1' ;;
  LEADING_JUNK) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" "x #12 0.123 cargo-json-invalid"; exit 1' ;;
  BARE_ALIAS_CODE) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" builder-executable-missing; exit 1' ;;
  VALUE_ERROR_ALIAS_CODE) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" "ValueError: builder-executable-missing"; exit 1' ;;
  DYNAMIC_OTHER_BIN) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" "release-build-layout: Cargo build failed for paper-runner"; exit 1' ;;
  REPEATED) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" cargo-json-invalid cargo-json-invalid; exit 1' ;;
  AMBIGUOUS) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" cargo-json-invalid builder-binary-invalid; exit 1' ;;
  SUFFIX) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" "cargo-json-invalid:PRIVATE_CHILD_FAILURE_SENTINEL"; exit 1' ;;
  SUBSTRING) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" "PRIVATE_CHILD_FAILURE_SENTINEL cargo-json-invalid"; exit 1' ;;
  BAD_UTF8) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "\\377"; exit 1' ;;
  OVERSIZE) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" python3 -c 'import sys;sys.stdout.buffer.write(b"x"*1048577);sys.exit(1)' ;;
  SUCCESS) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" cargo-json-invalid; exit 0' ;;
  SPAWN) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" lagrange-no-such-producer-command ;;
  PRECHILD)
    mkdir -p -- "$TEST_STATE_ROOT/logs/attempt-history"
    chmod 755 -- "$TEST_STATE_ROOT/logs/attempt-history"
    bad="$TEST_STATE_ROOT/logs/attempt-history/$label"
    mkdir -m 755 -- "$bad" 2>/dev/null
    chmod 755 -- "$bad"
    rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'exit 1' ;;
  POSTCHILD) rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'rmdir "$1" && : > "$1"; printf "%s\n" cargo-json-invalid; exit 1' fixture "$TEST_STATE_ROOT/logs/attempt-history/$label/receipts" ;;
  SEQUENTIAL)
    rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" cargo-json-invalid; exit 1'
    first=$?
    rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" builder-binary-invalid; exit 1'
    second=$?
    builtin printf 'SEQUENTIAL=%s,%s\n' "$first" "$second"
    ;;
  MARKER_WRITE)
    rbl_capture_attempt_log "$TEST_STATE_ROOT" "$TEST_STATE_ROOT/logs/$label.log" "$label" bash -c 'printf "%s\n" cargo-json-invalid; exit 1' ;;
esac
status=$?
builtin printf 'RETURN=%s\n' "$status"
'''
    env = {**os.environ, "TEST_CASE": case, "TEST_STATE_ROOT": str(state_root)}
    if case == "MARKER_WRITE":
        (state_root / "sitecustomize.py").write_text(
            "import os\n_original_write=os.write\n"
            "def _write(fd,data):\n"
            "    if data.startswith(b'RBL_CAPTURE_CHILD_FAILURE|'): raise OSError('fixture')\n"
            "    return _original_write(fd,data)\n"
            "os.write=_write\n",
            encoding="ascii",
        )
        env["PYTHONPATH"] = str(state_root)
    with os.fdopen(fd, "wb") as stderr_handle:
        result = subprocess.run(["bash", "--noprofile", "--norc", "-c", script + marker + capture + suffix],
                                env=env, stdout=subprocess.PIPE, stderr=stderr_handle,
                                text=True, timeout=10, check=False)
    result.stderr = stderr_path.read_bytes()
    return result


def capture_python_module():
    layout = (ROOT / "scripts/ops/lib/release-build-layout.sh").read_text(encoding="utf-8")
    capture = extract_bash_function(layout, "rbl_capture_attempt_log", "rbl_produce_native")
    match = re.search(r"<<'PY'[^\n]*\n(.*?)\nPY\n}", capture, re.DOTALL)
    if match is None:
        raise AssertionError("missing actual capture Python heredoc")
    return ast.parse(match.group(1))


def load_completed_child_classifier():
    tree = capture_python_module()
    wanted = {"child_failure_codes", "child_failure_aliases", "child_failure_public_codes"}
    nodes = []
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(
                isinstance(target, ast.Name) and target.id in wanted for target in node.targets):
            nodes.append(node)
        elif isinstance(node, ast.FunctionDef) and node.name == "completed_child_failure_code":
            nodes.append(node)
    module = ast.Module(body=nodes, type_ignores=[])
    namespace = {"hashlib": hashlib, "os": os, "re": re, "stat": stat}
    exec(compile(module, "<extracted-completed-child-classifier>", "exec"), namespace)
    return namespace


def implementation():
    source = SCRIPT.read_text(encoding="utf-8")
    if not source.startswith("#!/usr/bin/env bash\nset -euo pipefail\n" + MARKER):
        raise AssertionError("embedded Python entrypoint changed")
    code = source.split(MARKER, 1)[1].rsplit("\nPY\n", 1)[0]
    module = types.ModuleType("paper_fixture_image_driver_under_test")
    sys.modules[module.__name__] = module
    exec(compile(code, str(SCRIPT), "exec"), module.__dict__)
    return module


DRIVER = implementation()


def source_plan():
    return DRIVER.expected_plan_from_layout(ROOT)


class FakeBackend:
    """In-memory orchestration fixture; observations are explicitly synthetic."""

    def __init__(self, fail_at=None, drift_at=None, bad_revision_at=None):
        self.events = []
        self.fail_at = fail_at
        self.drift_at = drift_at
        self.bad_revision_at = bad_revision_at
        self.actual_ids = {
            service: "sha256:" + f"{index + 1:064x}"
            for index, service in enumerate(DRIVER.SERVICES)
        }
        self.max_compose_active = 0
        self.compose_active = 0
        self.published = False

    def record(self, name, *values):
        self.events.append((name, *values))
        if self.fail_at == name:
            raise subprocess.CalledProcessError(7, [name])

    def make_context(self, recipe, state):
        self.record("context", recipe)
        return f"/fixture/context/{recipe}"

    def produce_native(self, context, state):
        self.record("native", context)

    def produce_bin(self, recipe, bin_name, context, state):
        self.record("bin", recipe, bin_name)

    def bundle_create(self, recipe, state):
        self.record("bundle-create", recipe)
        return f"/fixture/bundle/{recipe}"

    def bundle_verify(self, bundle, recipe):
        self.record("bundle-verify", recipe)

    def write_override(self, service, bundle, path):
        self.record("override", service, bundle, str(path))

    def prepare_none(self, service, commit, state):
        self.record("prepare-none", service)
        return "NONE"

    def compose_build(self, service, override):
        self.compose_active += 1
        self.max_compose_active = max(self.max_compose_active, self.compose_active)
        try:
            self.record("compose", service, override)
        finally:
            self.compose_active -= 1

    def verify_image(self, service, commit, state):
        self.record("verify", service)
        return {
            "service": service,
            "image_ref": f"lagrange-station-{service}:{commit}",
            "image_id": self.actual_ids[service],
            "revision": "0" * 40 if service == self.bad_revision_at else commit,
        }

    def publish_temp(self, images, path):
        self.record("reinspect-v3", str(path))
        for service in DRIVER.SERVICES:
            current_id = self.actual_ids[service]
            if service == self.drift_at:
                current_id = "sha256:" + "f" * 64
            if images[service]["image_id"] != current_id:
                raise DRIVER.BuildFailure("final-image-identity-drift")
        self.published = True


class FakeResourcePolicy:
    def __init__(self, reason=None):
        self.reason = reason

    def read_observation(self):
        return {
            "MemAvailable": 8 * 1024 * 1024,
            "SwapFree": 4 * 1024 * 1024,
            "memory_psi_some_avg10": 0.0,
            "memory_psi_full_avg10": 0.0,
        }

    def failure_reason(self, observation):
        return self.reason


def fake_resource_monitor(reason=None):
    monitor = DRIVER.ResourceMonitor.__new__(DRIVER.ResourceMonitor)
    now = time.monotonic_ns()
    monitor.policy = FakeResourcePolicy(reason)
    monitor.final_deadline_ns = now + DRIVER.BUILD_RESERVE_NS + 60_000_000_000
    monitor.build_deadline_ns = monitor.final_deadline_ns - DRIVER.BUILD_RESERVE_NS
    monitor.last_live_check_ns = 0
    monitor.oom_baseline = 7
    monitor.checks = []
    return monitor


def invoke_live_readline(monitor, base_ns, select_results):
    helper = object.__new__(DRIVER.HelperServer)
    helper.proc = types.SimpleNamespace(
        pid=100,
        stdout=types.SimpleNamespace(fileno=lambda: 77),
        poll=lambda: None,
    )
    helper.buffer = bytearray()
    with mock.patch.object(DRIVER.time, "monotonic_ns", return_value=base_ns):
        with mock.patch.object(DRIVER.select, "select", side_effect=select_results):
            with mock.patch.object(DRIVER.os, "read", return_value=b"READY\n"):
                return helper.readline(base_ns + 5_000_000_000, monitor, "produce_bin")


def synthetic_manifest_bytes():
    lines = ["LAGRANGE_RELEASE_MANIFEST_V3", f"commit|{COMMIT}"]
    for index, service in enumerate(DRIVER.SERVICES):
        lines.append(
            f"image|{service}|lagrange-station-{service}:{COMMIT}|"
            f"sha256:{index:064x}|{COMMIT}"
        )
    return ("\n".join(lines) + "\n").encode("ascii")


def write_private_file(path, data):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as handle:
        handle.write(data)
        handle.flush()
        os.fsync(handle.fileno())


class PaperFixtureImageTests(unittest.TestCase):
    def test_closed_failure_diagnostics_never_print_exception_payloads(self):
        sentinel = "PRIVATE_DIAGNOSTIC_SENTINEL"
        errors = [DRIVER.BuildFailure(sentinel),
                  DRIVER.BuildFailure("helper-action-failed:" + sentinel),
                  DRIVER.BuildFailure("command-failed:" + sentinel + ":1"),
                  OSError(13, sentinel, "/private/" + sentinel),
                  subprocess.CalledProcessError(7, [sentinel], output=sentinel, stderr=sentinel)]
        for error in errors:
            with self.subTest(kind=type(error).__name__):
                output = io.StringIO()
                with contextlib.redirect_stderr(output):
                    DRIVER.report_failure(error)
                self.assertNotIn(sentinel, output.getvalue())
                line = output.getvalue().splitlines()[1]
                value = json.loads(line.removeprefix("PAPER_FIXTURE_IMAGE_FAILURE "))
                self.assertEqual(set(value), {"kind", "stage", "reason", "line", "caller_line", "errno"})
                self.assertEqual(value["stage"], "UNKNOWN")
        self.assertEqual(DRIVER.failure_diagnostic(DRIVER.BuildFailure("commit-invalid"))["reason"],
                         "commit-invalid")
        self.assertEqual(DRIVER.failure_diagnostic(DRIVER.BuildFailure(
            "resource-policy-stop:memory-pressure-high"))["reason"], "resource-policy-stop:memory-pressure-high")
        for reason in DRIVER.CLOSED_HELPER_REASONS:
            self.assertEqual(DRIVER.failure_diagnostic(DRIVER.BuildFailure(
                "helper-action-failed:" + reason))["reason"], "helper-action-failed:" + reason)

    def test_cli_invalid_commit_exposes_closed_input_reason_before_any_heavy_work(self):
        result = subprocess.run(["bash", str(SCRIPT), "--source-root", str(ROOT),
            "--commit", "not-a-commit", "--state-root", "/private/PRIVATE_DIAGNOSTIC_SENTINEL",
            "--manifest-file", "/private/PRIVATE_DIAGNOSTIC_SENTINEL/manifest",
            "--deadline-monotonic-ns", str(time.monotonic_ns() + 300_000_000_000)],
            capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertNotIn("PRIVATE_DIAGNOSTIC_SENTINEL", result.stderr)
        self.assertEqual(result.stderr.splitlines()[0], "paper-fixture-image-build: BuildFailure")
        value = json.loads(result.stderr.splitlines()[1].removeprefix("PAPER_FIXTURE_IMAGE_FAILURE "))
        self.assertEqual((value["stage"], value["reason"]), ("INPUTS", "commit-invalid"))
        self.assertGreater(value["line"], 0)
        self.assertGreater(value["caller_line"], 0)
        physical = SCRIPT.read_text().splitlines()
        self.assertIn('raise BuildFailure("commit-invalid")', physical[value["line"] - 1])
        self.assertIn("validate_path_arguments(", physical[value["caller_line"] - 1])

    def test_current_compose_has_a_closed_disabled_fixture_environment(self):
        compose = (ROOT / "deploy/compose/compose.yml").read_text()
        values = DRIVER.disabled_env_values(compose,
            COMMIT, pathlib.Path("/var/tmp/lagrange-paper-fixture-build-unitprobe"), "unitprobe")
        required = set(re.findall(r"(?<!\$)\$\{([A-Za-z_][A-Za-z0-9_]*):\?", compose))
        self.assertTrue(required)
        self.assertEqual([name for name in sorted(required) if not values.get(name)], [])
        self.assertIsNone(re.fullmatch(r"[0-9a-f]{64}", values["RESEARCH_ENTITLEMENT_SHA256"]))
        self.assertEqual(values["LIVE_NODE_ACCOUNT_ID"], "")
        self.assertEqual(values["OWNER_BETA_PAPER_MODE"], "disabled")
        self.assertEqual(values["LIVE_NODE_MODE"], "disabled")

    def test_shared_helper_failure_reports_only_closed_opcode_and_plan_context(self):
        import base64
        sentinel = "PRIVATE_HELPER_ARGUMENT_SENTINEL"
        wire = b"ERR " + base64.b64encode(b"helper-failed")
        handlers = {"MAKE_CONTEXT": "make_context", "PRODUCE_NATIVE": "produce_native",
                    "PRODUCE_BIN": "produce_bin", "BUNDLE_CREATE": "bundle_create",
                    "BUNDLE_VERIFY": "bundle_verify", "WRITE_OVERRIDE": "write_override"}
        for opcode, method in handlers.items():
            with self.subTest(opcode=opcode):
                helper = object.__new__(DRIVER.HelperServer)
                helper.deadline_ns = 1
                helper.proc = types.SimpleNamespace(stdin=io.BytesIO())
                helper.readline = mock.Mock(return_value=wire)
                backend = FakeBackend()
                guards = []
                def fail(*args):
                    return helper.request(opcode, (sentinel,))
                with mock.patch.object(backend, method, side_effect=fail):
                    with self.assertRaises(DRIVER.BuildFailure) as raised:
                        DRIVER.execute_plan(source_plan(), backend, guards.append,
                                            COMMIT, pathlib.Path("/private/" + sentinel))
                value = DRIVER.failure_diagnostic(raised.exception)
                self.assertEqual(value["reason"], "helper-action-failed:helper-failed")
                self.assertEqual((value["helper_op"], value["build_service"], value["build_recipe"]),
                                 (opcode, "api-server", "D1"))
                if opcode == "PRODUCE_BIN":
                    self.assertEqual((value["producer_phase"], value["producer_status"]),
                                     ("UNKNOWN", "UNKNOWN"))
                else:
                    self.assertNotIn("producer_phase", value)
                    self.assertNotIn("producer_status", value)
                self.assertTrue(guards[-1].startswith("before:api-server:"))
                self.assertFalse(backend.published)
                self.assertEqual(helper.readline.call_count, 1)
                output = io.StringIO()
                with contextlib.redirect_stderr(output):
                    DRIVER.report_failure(raised.exception)
                self.assertNotIn(sentinel, output.getvalue())
                self.assertNotIn("/private/", output.getvalue())
                self.assertEqual(json.loads(output.getvalue().splitlines()[1].removeprefix(
                    "PAPER_FIXTURE_IMAGE_FAILURE ")), value)
        for payload in (sentinel, [sentinel], {"secret": sentinel}):
            forged = DRIVER.BuildFailure("helper-action-failed:helper-failed")
            forged.helper_op = payload
            forged.build_service = payload
            forged.build_recipe = payload
            self.assertEqual(set(DRIVER.failure_diagnostic(forged)),
                             {"kind", "stage", "reason", "line", "caller_line", "errno"})
            forged.helper_op = "MAKE_CONTEXT"
            self.assertEqual(DRIVER.failure_diagnostic(forged)["helper_op"], "MAKE_CONTEXT")
            self.assertNotIn(sentinel, json.dumps(DRIVER.failure_diagnostic(forged)))
        other = DRIVER.BuildFailure("commit-invalid")
        other.helper_op, other.build_service, other.build_recipe = "MAKE_CONTEXT", "api-server", "D1"
        self.assertEqual(set(DRIVER.failure_diagnostic(other)),
                         {"kind", "stage", "reason", "line", "caller_line", "errno"})

    def test_produce_bin_phase_uses_current_private_stderr_and_closed_diagnostics(self):
        import base64

        valid_marker = b"RBL_PRODUCE_BIN_PHASE|ATTEMPT|37\n"
        old_marker = b"RBL_PRODUCE_BIN_PHASE|CURRENT_K|19\n"
        cases = (
            ("current", b"prior helper diagnostic\n", "valid"),
            ("stale", old_marker, "none"),
            ("missing-offset", b"prior\n", "missing-offset"),
            ("replacement", b"prior\n", "replacement"),
            ("mode", b"prior\n", "mode"),
            ("hardlink", b"prior\n", "hardlink"),
            ("oversize", b"prior\n", "oversize"),
            ("non-ascii", b"prior\n", "non-ascii"),
            ("malformed", b"prior\n", "malformed"),
            ("forged-phase", b"prior\n", "forged-phase"),
            ("bad-status", b"prior\n", "bad-status"),
            ("changed-during-read", b"prior\n", "changed-during-read"),
        )

        def run_case(case, initial, mutation):
            with tempfile.TemporaryDirectory(prefix="producer-phase-read-") as tmp:
                root = pathlib.Path(tmp)
                log = root / "logs/helper-server.stderr.log"
                log.parent.mkdir(mode=0o700)
                log.write_bytes(initial)
                log.chmod(0o600)
                if mutation == "missing-offset":
                    log.chmod(0o644)
                info = log.stat()
                helper = object.__new__(DRIVER.HelperServer)
                helper._stderr_path = log
                helper._stderr_identity = (info.st_dev, info.st_ino)
                helper.deadline_ns = 1
                helper.proc = types.SimpleNamespace(stdin=io.BytesIO())
                wire = b"ERR " + base64.b64encode(b"helper-failed")

                def append(data):
                    with log.open("ab") as stream:
                        stream.write(data)
                        stream.flush()
                        os.fsync(stream.fileno())

                def reply(*_args):
                    if mutation == "valid":
                        append(valid_marker)
                    elif mutation == "replacement":
                        log.rename(root / "original.stderr")
                        log.write_bytes(valid_marker)
                        log.chmod(0o600)
                    elif mutation == "mode":
                        append(valid_marker)
                        log.chmod(0o644)
                    elif mutation == "hardlink":
                        append(valid_marker)
                        os.link(log, root / "stderr-link")
                    elif mutation == "oversize":
                        append(b"x" * 65537 + b"\n")
                    elif mutation == "non-ascii":
                        append(b"\xff\n")
                    elif mutation == "malformed":
                        append(b"RBL_PRODUCE_BIN_PHASE|ATTEMPT|037\n")
                    elif mutation == "forged-phase":
                        append(b"RBL_PRODUCE_BIN_PHASE|PRIVATE_DIAGNOSTIC_SENTINEL|37\n")
                    elif mutation == "bad-status":
                        append(b"RBL_PRODUCE_BIN_PHASE|ATTEMPT|256\n")
                    elif mutation == "changed-during-read":
                        append(valid_marker)
                    elif mutation == "missing-offset":
                        append(valid_marker)
                    return wire

                helper.readline = mock.Mock(side_effect=reply)
                backend = FakeBackend()

                def fail_producer(recipe, bin_name, context, state):
                    return helper.request(
                        "PRODUCE_BIN", (recipe, bin_name, context, str(state)),
                        cutoff_ns=2,
                    )

                read_patch = mock.patch.object(DRIVER.os, "pread")
                if mutation == "changed-during-read":
                    real_pread = os.pread

                    def read_then_change(fd, size, offset):
                        data = real_pread(fd, size, offset)
                        append(b"late helper write\n")
                        return data

                    read_patch = mock.patch.object(DRIVER.os, "pread", side_effect=read_then_change)
                with mock.patch.object(backend, "produce_bin", side_effect=fail_producer):
                    with (read_patch if mutation == "changed-during-read"
                          else contextlib.nullcontext()):
                        with self.assertRaises(DRIVER.BuildFailure) as raised:
                            DRIVER.execute_plan(
                                source_plan(), backend, lambda _label: None,
                                COMMIT, pathlib.Path("/fixture/state"),
                            )
                error = raised.exception
                diagnostic = DRIVER.failure_diagnostic(error)
                output = io.StringIO()
                with contextlib.redirect_stderr(output):
                    DRIVER.report_failure(error)
                self.assertEqual(helper.readline.call_count, 1)
                expected_wire = "PRODUCE_BIN " + " ".join(
                    DRIVER.b64_field(value) for value in (
                        "D1", "api-server", "/fixture/context/D1", "/fixture/state"
                    )
                ) + "\n"
                self.assertEqual(helper.proc.stdin.getvalue(), expected_wire.encode("ascii"))
                self.assertEqual(diagnostic["reason"], "helper-action-failed:helper-failed")
                self.assertEqual((diagnostic["helper_op"], diagnostic["build_service"],
                                  diagnostic["build_recipe"]), ("PRODUCE_BIN", "api-server", "D1"))
                self.assertNotIn("PRIVATE_DIAGNOSTIC_SENTINEL", output.getvalue())
                return diagnostic

        for name, initial, mutation in cases:
            with self.subTest(case=name):
                diagnostic = run_case(name, initial, mutation)
                if name == "current":
                    self.assertEqual((diagnostic["producer_phase"], diagnostic["producer_status"]),
                                     ("ATTEMPT", 37))
                else:
                    self.assertEqual((diagnostic["producer_phase"], diagnostic["producer_status"]),
                                     ("UNKNOWN", "UNKNOWN"))

        forged = DRIVER.BuildFailure("helper-action-failed:helper-failed")
        forged.helper_op, forged.build_service, forged.build_recipe = "PRODUCE_BIN", "api-server", "D1"
        for phase, status in (("PRIVATE_DIAGNOSTIC_SENTINEL", 37), ("ATTEMPT", 256),
                              (["ATTEMPT"], 37), ("ATTEMPT", True)):
            forged.producer_phase, forged.producer_status = phase, status
            value = DRIVER.failure_diagnostic(forged)
            self.assertEqual((value["producer_phase"], value["producer_status"]),
                             ("UNKNOWN", "UNKNOWN"))
            self.assertNotIn("PRIVATE_DIAGNOSTIC_SENTINEL", json.dumps(value))
        forged.helper_op = "MAKE_CONTEXT"
        forged.producer_phase, forged.producer_status = "ATTEMPT", 37
        self.assertNotIn("producer_phase", DRIVER.failure_diagnostic(forged))
        class ForgedOpcode:
            def __eq__(self, other):
                return other == "PRODUCE_BIN"
        forged.helper_op = ForgedOpcode()
        self.assertNotIn("producer_phase", DRIVER.failure_diagnostic(forged))
        forged.helper_op = "PRODUCE_BIN"
        forged.args = ("helper-action-failed:compose-build-failed",)
        self.assertNotIn("producer_phase", DRIVER.failure_diagnostic(forged))

    def test_saved_image_failure_exposes_only_current_bound_log_closed_codes(self):
        import base64
        sentinel = "PRIVATE_IMAGE_ERROR_SENTINEL"
        wire = b"ERR " + base64.b64encode(b"saved-image-verification-failed")
        with tempfile.TemporaryDirectory() as root:
            log = pathlib.Path(root) / "helper.stderr"
            for case in ("code", "payload", "old-only", "oversize", "mode", "replacement", "symlink"):
                with self.subTest(case=case):
                    if log.is_symlink() or log.exists():
                        log.unlink()
                    log.write_bytes(b"product-file-hash-invalid\n")
                    log.chmod(0o600)
                    info = log.stat()
                    helper = object.__new__(DRIVER.HelperServer)
                    helper.deadline_ns = 1
                    helper._stderr_path = log
                    helper._stderr_identity = (info.st_dev, info.st_ino)
                    helper.proc = types.SimpleNamespace(stdin=io.BytesIO())
                    def reply(*args):
                        if case == "replacement":
                            log.rename(pathlib.Path(root) / "old")
                            log.write_bytes(b"product-file-mode-invalid\n")
                            log.chmod(0o600)
                        elif case == "symlink":
                            log.unlink()
                            log.symlink_to(pathlib.Path(root) / "missing")
                        else:
                            data = {"code": b"product-file-mode-invalid\n",
                                    "payload": sentinel.encode() + b"\n",
                                    "old-only": b"", "oversize": b"x" * 65537,
                                    "mode": b"product-file-mode-invalid\n"}[case]
                            with log.open("ab") as stream:
                                stream.write(data)
                            if case == "mode":
                                log.chmod(0o644)
                        return wire
                    helper.readline = mock.Mock(side_effect=reply)
                    with self.assertRaises(DRIVER.BuildFailure) as raised:
                        helper.request("VERIFY_IMAGE", ("api-server",))
                    value = DRIVER.failure_diagnostic(raised.exception)
                    self.assertEqual(value["reason"], "helper-action-failed:saved-image-verification-failed")
                    self.assertEqual(value["image_service"], "api-server")
                    self.assertEqual(value["image_validator_reason"],
                                     "product-file-mode-invalid" if case == "code" else "UNKNOWN")
                    self.assertNotIn(sentinel, json.dumps(value))
                    output = io.StringIO()
                    with contextlib.redirect_stderr(output):
                        DRIVER.report_failure(raised.exception)
                    self.assertNotIn(sentinel, output.getvalue())
                    self.assertNotIn(str(log), output.getvalue())
                    self.assertEqual(helper.readline.call_count, 1)
                    self.assertEqual(helper.proc.stdin.getvalue(),
                                     b"VERIFY_IMAGE YXBpLXNlcnZlcg==\n")
            forged = DRIVER.BuildFailure("helper-action-failed:saved-image-verification-failed")
            forged.image_service = sentinel
            forged.image_validator_reason = sentinel
            self.assertNotIn(sentinel, json.dumps(DRIVER.failure_diagnostic(forged)))
            forged.image_service = "api-server"
            forged.image_validator_reason = [sentinel]
            self.assertEqual(DRIVER.failure_diagnostic(forged)["image_validator_reason"], "UNKNOWN")

    def test_helper_request_accepts_only_server_opcodes_before_writing(self):
        server_ops = set(re.findall(r"^    ([A-Z_]+)\)\s*$", DRIVER.BASH_SERVER, re.MULTILINE))
        self.assertIn("PLAN", server_ops)
        self.assertIn("QUIT", server_ops)
        helper = object.__new__(DRIVER.HelperServer)
        helper.deadline_ns = 1
        for op in ["PLAN", *sorted(server_ops - {"PLAN"})]:
            with self.subTest(op=op):
                stream = io.BytesIO()
                helper.proc = types.SimpleNamespace(stdin=stream)
                helper.readline = mock.Mock(return_value=b"OK b2s=")
                self.assertEqual(helper.request(op), "ok")
                self.assertEqual(stream.getvalue(), (op + "\n").encode("ascii"))
        self.assertEqual(DRIVER.HELPER_OPS, server_ops)
        self.assertIsNone(DRIVER.NAME_RE.fullmatch("PLAN"))
        self.assertIsNotNone(DRIVER.NAME_RE.fullmatch("api-server"))
        invalid = [(op, (), False) for op in ("", "plan", "UNKNOWN", "PLAN EXTRA", "PLAN\nQUIT", "PLÄN")]
        invalid += [("PLAN", ("x",) * 5, False), ("PLAN", (), True)]
        for op, args, missing_stdin in invalid:
            with self.subTest(op=op, args=args, missing_stdin=missing_stdin):
                stream = io.BytesIO()
                helper.proc = types.SimpleNamespace(stdin=None if missing_stdin else stream)
                helper.readline = mock.Mock(return_value=b"OK b2s=")
                with self.assertRaisesRegex(DRIVER.BuildFailure, "helper-request-invalid"):
                    helper.request(op, args)
                self.assertEqual(stream.getvalue(), b"")
                helper.readline.assert_not_called()

    def test_builder_driver_query_selects_one_exact_builder_before_bootstrap(self):
        builder = "paper-fixture-" + "a" * 32
        cases = [
            (f"foreign\tdocker\n{builder}\tdocker-container\n{builder}0\tunix:///fixture.sock", True),
            (f"foreign\tdocker-container\n{builder}\tdocker", False),
            ("foreign\tdocker-container", False),
            (f"{builder}\tdocker-container\n{builder}\tdocker-container", False),
            (f"{builder}\tdocker-container\textra", False),
            (f"{builder}\tdocker-container\n{builder}", False),
        ]
        deadline = time.monotonic_ns() + 300_000_000_000
        state = pathlib.Path("/fixture/state")
        args = types.SimpleNamespace(source_root=ROOT, commit=COMMIT, state_root=state,
            manifest_file=state / "manifest", deadline_monotonic_ns=deadline)
        for rows, valid in cases:
            with self.subTest(rows=rows):
                bootstraps = []

                def command(argv, label, env, command_state, command_deadline, output=False):
                    if label == "daemon-identity-before":
                        return "fixture-daemon"
                    if label == "buildx-ls-before-create":
                        return "foreign"
                    if label == "buildx-driver":
                        if argv[:3] == ["docker", "buildx", "inspect"]:
                            raise subprocess.CalledProcessError(125, argv)
                        self.assertEqual(argv, ["docker", "buildx", "ls", "--format",
                                                "{{.Name}}\t{{.DriverEndpoint}}"])
                        self.assertTrue(output)
                        return rows
                    if label == "buildx-bootstrap":
                        self.assertEqual(argv, ["docker", "buildx", "inspect", "--bootstrap", builder])
                        bootstraps.append(label)
                        raise RuntimeError("fixture-stop-before-helper")
                    return ""

                monitor = mock.Mock(build_deadline_ns=deadline - 120_000_000_000)
                cleanup = mock.Mock()
                with mock.patch.multiple(DRIVER,
                        parse_args=mock.Mock(return_value=args),
                        validate_path_arguments=mock.Mock(return_value=(ROOT, state, state / "manifest")),
                        assert_clean_source=mock.Mock(), source_evidence_hashes=mock.Mock(return_value={}),
                        create_private_state=mock.Mock(return_value=(state / ".env", "b" * 32)),
                        ResourceMonitor=mock.Mock(return_value=monitor),
                        process_environment=mock.Mock(return_value={}),
                        run_owned=mock.Mock(side_effect=command), cleanup_builder=cleanup), \
                        mock.patch.object(DRIVER.secrets, "token_hex", return_value="a" * 32):
                    error = RuntimeError if valid else DRIVER.BuildFailure
                    reason = "fixture-stop-before-helper" if valid else "ephemeral-buildx-driver-invalid"
                    with self.assertRaisesRegex(error, reason):
                        DRIVER.main([])
                self.assertEqual(bootstraps, ["buildx-bootstrap"] if valid else [])
                cleanup.assert_called_once_with(builder, {}, state, deadline - 15_000_000_000)

    def test_plan_uses_fixed_v3_order_and_none_allowlist(self):
        plan = source_plan()
        wire = "".join(
            f"{item.service}\t{item.recipe}\t{json.dumps(list(item.bins), separators=(',', ':'))}\n"
            for item in plan
        )
        parsed = DRIVER.parse_plan(wire)
        self.assertEqual(tuple(item.service for item in parsed), DRIVER.SERVICES)
        self.assertEqual(
            {item.service for item in parsed if item.recipe == "NONE"},
            {"db-role-bootstrap", "db-migrate", "web"},
        )
        DRIVER.assert_plan_matches_layout(parsed, ROOT)
        with self.assertRaises(DRIVER.BuildFailure):
            DRIVER.parse_plan(wire.replace("db-role-bootstrap", "postgres", 1))
        changed = list(parsed)
        changed[2] = DRIVER.ServicePlan("api-server", "D2", ("recommendation-runner",))
        with self.assertRaisesRegex(DRIVER.BuildFailure, "helper-plan-differs-from-checked-in-layout"):
            DRIVER.assert_plan_matches_layout(tuple(changed), ROOT)

    def test_source_hash_inventory_is_fixed_and_regular_file_bound(self):
        evidence = DRIVER.source_evidence_hashes(ROOT)
        self.assertEqual(set(evidence), set(DRIVER.SOURCE_EVIDENCE_FILES))
        raw = (ROOT / "scripts/ops/lib/release-build-layout.sh").read_bytes()
        self.assertEqual(
            evidence["scripts/ops/lib/release-build-layout.sh"]["sha256"],
            hashlib.sha256(raw).hexdigest(),
        )

    def test_actual_rbl_produce_bin_marks_failure_phases_and_preserves_return(self):
        expected = (
            ("NATIVE_IDENTITY", 17, 1), ("CURRENT_K", 19, 1),
            ("REQUEST_K", 23, 1), ("STALE_K", 1, 1),
            ("PRIVATE_OUTPUT", 29, 1), ("RECIPE", 31, 1),
            ("ATTEMPT", 37, 1), ("OUTPUT_PRODUCER", 1, 1),
            ("OUTPUT_NATIVE", 1, 1), ("VERIFY_PRODUCER", 41, 1),
            ("NO_CLOBBER", 1, 1), ("MOVE", 43, 1),
            ("NATIVE_IDENTITY_COMPARE", 44, 1),
            ("NATIVE_IDENTITY_INSTALL", 45, 1), ("OUTPUT_PATH", 46, 46),
        )
        self.assertEqual(tuple(phase for phase, _, _ in expected), PRODUCER_PHASES)
        for phase, original_status, expected_return in expected:
            with self.subTest(phase=phase):
                with tempfile.TemporaryDirectory(prefix="rbl-phase-") as tmp:
                    state_root = pathlib.Path(tmp)
                    result = run_rbl_produce_bin_case(phase, state_root)
                    if phase == "ATTEMPT":
                        captured = (state_root / "attempt-argv.txt").read_text().splitlines()
                        self.assertEqual(captured[:3], [
                            str(state_root),
                            str(state_root / "logs/producer-D1-api-server.log"),
                            "producer-D1-api-server",
                        ])
                        self.assertEqual(captured[3:6], ["docker", "buildx", "build"])
                        self.assertEqual(sum(arg.startswith("--build-arg=CARGO_BIN=")
                                             for arg in captured), 1)
                        self.assertIn("--build-arg=CARGO_BIN=api-server", captured)
                        self.assertEqual(captured[-1], f"{state_root}/context")
                self.assertEqual(result.returncode, 0, f"shell fixture runner failed for {phase}")
                self.assertIn(f"RETURN={expected_return}\n", result.stdout,
                              f"existing failure return changed for {phase}")
                marker_lines = [line for line in result.stderr.splitlines()
                                if line.startswith("RBL_PRODUCE_BIN_PHASE|")]
                self.assertEqual(marker_lines[-1:],
                                 [f"RBL_PRODUCE_BIN_PHASE|{phase}|{original_status}"],
                                 f"missing phase marker for {phase}")

        with tempfile.TemporaryDirectory(prefix="rbl-phase-no-helper-") as tmp:
            outside = run_rbl_produce_bin_case(
                "NATIVE_IDENTITY", pathlib.Path(tmp), private_helper=False
            )
        self.assertIn("RETURN=1\n", outside.stdout)
        self.assertFalse(any(line.startswith("RBL_PRODUCE_BIN_PHASE|")
                             for line in outside.stderr.splitlines()))

        with tempfile.TemporaryDirectory(prefix="rbl-phase-reuse-") as tmp:
            reused = run_rbl_produce_bin_case("REUSE", pathlib.Path(tmp))
        self.assertIn("RETURN=0\n", reused.stdout)
        self.assertIn(f"VALUE={tmp}/producers/D1/api-server\n", reused.stdout)
        self.assertFalse(any(line.startswith("RBL_PRODUCE_BIN_PHASE|")
                             for line in reused.stderr.splitlines()))

        with tempfile.TemporaryDirectory(prefix="rbl-phase-marker-failure-") as tmp:
            failed_marker = run_rbl_produce_bin_case(
                "NATIVE_IDENTITY", pathlib.Path(tmp), fail_marker_write=True
            )
        self.assertIn("RETURN=1\n", failed_marker.stdout)
        self.assertFalse(any(line.startswith("RBL_PRODUCE_BIN_PHASE|")
                             for line in failed_marker.stderr.splitlines()))

    def test_actual_capture_attempt_markers_preserve_child_and_archive_results(self):
        full = [
            "SHELL_VALIDATE", "PYTHON_SETUP", "HISTORY_VALIDATE", "PRIOR_ARCHIVE",
            "ACTIVE_LOG_OPEN", "CHILD_PENDING", "CHILD_RETURNED", "LOG_SYNC",
            "COMPLETED_ARCHIVE", "COMPLETED_RECEIPT", "COMPLETE",
        ]
        spawn_phases = full[:6] + ["CHILD_SPAWN_ERROR"] + full[7:]
        expected = {
            "SUCCESS": (0, full, "0"),
            "CHILD1": (1, full, "1"),
            "SPAWN": (127, spawn_phases, "SPAWN_ERROR"),
            "PRECHILD": (1, full[:3], "UNKNOWN"),
            "POSTCHILD": (1, full[:9], "0"),
            "MARKER_WRITE": (1, full[1:], "1"),
        }
        for case, (return_code, phases, child_status) in expected.items():
            with self.subTest(case=case), tempfile.TemporaryDirectory(prefix="capture-attempt-") as tmp:
                result = run_rbl_capture_attempt_case(case, pathlib.Path(tmp))
                self.assertEqual(result.returncode, 0, result.stderr[-500:])
                self.assertIn(f"RETURN={return_code}\n", result.stdout)
                records = [line.removeprefix("RBL_CAPTURE_ATTEMPT|").split("|")
                           for line in result.stderr.splitlines()
                           if line.startswith("RBL_CAPTURE_ATTEMPT|")]
                self.assertEqual([record[0] for record in records], phases)
                for record in records:
                    expected_status = ("UNKNOWN" if record[0] in full[:6] else child_status)
                    self.assertEqual(record[1], expected_status)
        with tempfile.TemporaryDirectory(prefix="capture-attempt-outside-") as tmp:
            outside = run_rbl_capture_attempt_case("OUTSIDE", pathlib.Path(tmp))
        self.assertFalse(any(line.startswith("RBL_CAPTURE_ATTEMPT|")
                             for line in outside.stderr.splitlines()))

    def test_capture_attempt_subphases_are_current_private_and_closed(self):
        import base64

        def sequence(child_status="0"):
            records = [(phase, "UNKNOWN") for phase in CAPTURE_ATTEMPT_PHASES[:6]]
            records.append(("CHILD_RETURNED", child_status))
            records.extend((phase, child_status) for phase in CAPTURE_ATTEMPT_PHASES[8:])
            return b"".join((f"RBL_CAPTURE_ATTEMPT|{phase}|{status}\n").encode("ascii")
                             for phase, status in records)

        wire = b"ERR " + base64.b64encode(b"helper-failed")

        def run_case(current, *, initial=b"PRIVATE_CAPTURE_SENTINEL\n",
                     producer=b"RBL_PRODUCE_BIN_PHASE|ATTEMPT|1\n", mutation=None):
            with tempfile.TemporaryDirectory(prefix="capture-evidence-") as tmp:
                root = pathlib.Path(tmp)
                log = root / "logs/helper-server.stderr.log"
                log.parent.mkdir(mode=0o700)
                log.write_bytes(initial)
                log.chmod(0o600)
                info = log.stat()
                helper = object.__new__(DRIVER.HelperServer)
                helper._stderr_path = log
                helper._stderr_identity = (info.st_dev, info.st_ino)
                helper.deadline_ns = 1
                helper.proc = types.SimpleNamespace(stdin=io.BytesIO())

                def reply(*_args):
                    with log.open("ab") as stream:
                        stream.write(current + producer)
                        stream.flush()
                        os.fsync(stream.fileno())
                    if mutation == "replacement":
                        log.rename(root / "old-stderr")
                        log.write_bytes(current + producer)
                        log.chmod(0o600)
                    elif mutation == "hardlink":
                        os.link(log, root / "stderr-hardlink")
                    elif mutation == "mode":
                        log.chmod(0o644)
                    elif mutation == "changed":
                        with log.open("ab") as stream:
                            stream.write(b"changed-after-response\n")
                    return wire

                helper.readline = mock.Mock(side_effect=reply)
                backend = FakeBackend()

                def fail_producer(recipe, bin_name, context, state):
                    return helper.request("PRODUCE_BIN", (recipe, bin_name, context, str(state)))

                with mock.patch.object(backend, "produce_bin", side_effect=fail_producer):
                    with self.assertRaises(DRIVER.BuildFailure) as raised:
                        DRIVER.execute_plan(source_plan(), backend, lambda _label: None,
                                            COMMIT, pathlib.Path("/fixture/state"))
                error = raised.exception
                diagnostic = DRIVER.failure_diagnostic(error)
                rendered = io.StringIO()
                with contextlib.redirect_stderr(rendered):
                    DRIVER.report_failure(error)
                self.assertNotIn("PRIVATE_CAPTURE_SENTINEL", rendered.getvalue())
                self.assertNotIn(str(log), rendered.getvalue())
                self.assertEqual(helper.readline.call_count, 1)
                self.assertEqual(helper.proc.stdin.getvalue().split(b" ", 1)[0], b"PRODUCE_BIN")
                return diagnostic, error, rendered.getvalue()

        valid, _, _ = run_case(sequence("1"))
        self.assertEqual((valid["producer_phase"], valid["attempt_phase"],
                          valid["attempt_child_status"]), ("ATTEMPT", "COMPLETE", 1))
        for child_status in ("0", "2", "-9"):
            diagnostic, _, _ = run_case(sequence(child_status))
            self.assertEqual((diagnostic["attempt_phase"], diagnostic["attempt_child_status"]),
                             ("UNKNOWN", "UNKNOWN"))
        postchild = sequence("0").removesuffix(b"RBL_CAPTURE_ATTEMPT|COMPLETE|0\n")
        diagnostic, _, _ = run_case(postchild)
        self.assertEqual((diagnostic["attempt_phase"], diagnostic["attempt_child_status"]),
                         ("COMPLETED_RECEIPT", 0))
        diagnostic, _, _ = run_case(sequence("-9"),
                                    producer=b"RBL_PRODUCE_BIN_PHASE|ATTEMPT|247\n")
        self.assertEqual((diagnostic["attempt_phase"], diagnostic["attempt_child_status"]),
                         ("COMPLETE", -9))
        spawn_records = [(phase, "UNKNOWN") for phase in CAPTURE_ATTEMPT_PHASES[:6]]
        spawn_records.append(("CHILD_SPAWN_ERROR", "SPAWN_ERROR"))
        spawn_records.extend((phase, "SPAWN_ERROR") for phase in CAPTURE_ATTEMPT_PHASES[8:])
        spawn = b"".join(f"RBL_CAPTURE_ATTEMPT|{p}|{s}\n".encode() for p, s in spawn_records)
        diagnostic, _, _ = run_case(spawn)
        self.assertEqual((diagnostic["attempt_phase"], diagnostic["attempt_child_status"]),
                         ("UNKNOWN", "UNKNOWN"))
        diagnostic, _, _ = run_case(spawn,
                                    producer=b"RBL_PRODUCE_BIN_PHASE|ATTEMPT|127\n")
        self.assertEqual((diagnostic["attempt_phase"], diagnostic["attempt_child_status"]),
                         ("COMPLETE", "SPAWN_ERROR"))
        prechild = b"RBL_CAPTURE_ATTEMPT|SHELL_VALIDATE|UNKNOWN\nRBL_CAPTURE_ATTEMPT|PYTHON_SETUP|UNKNOWN\nRBL_CAPTURE_ATTEMPT|HISTORY_VALIDATE|UNKNOWN\n"
        diagnostic, _, _ = run_case(prechild)
        self.assertEqual((diagnostic["attempt_phase"], diagnostic["attempt_child_status"]),
                         ("HISTORY_VALIDATE", "UNKNOWN"))

        invalid = (
            b"RBL_CAPTURE_ATTEMPT|SHELL_VALIDATE|UNKNOWN\nRBL_CAPTURE_ATTEMPT|NOPE|UNKNOWN\n",
            b"RBL_CAPTURE_ATTEMPT|SHELL_VALIDATE|UNKNOWN\nRBL_CAPTURE_ATTEMPT|PYTHON_SETUP|256\n",
            b"RBL_CAPTURE_ATTEMPT|SHELL_VALIDATE|UNKNOWN\nRBL_CAPTURE_ATTEMPT|HISTORY_VALIDATE|UNKNOWN\n",
            b"RBL_CAPTURE_ATTEMPT|SHELL_VALIDATE|UNKNOWN\nRBL_CAPTURE_ATTEMPT|PYTHON_SETUP|UNKNOWN\nRBL_CAPTURE_ATTEMPT|PYTHON_SETUP|UNKNOWN\n",
            b"RBL_CAPTURE_ATTEMPT|SHELL_VALIDATE|UNKNOWN\nRBL_CAPTURE_ATTEMPT|PYTHON_SETUP|037\n",
            b"RBL_CAPTURE_ATTEMPT|SHELL_VALIDATE|UNKNOWN\nRBL_CAPTURE_ATTEMPT|PYTHON_SETUP|UNKNOWN\xff\n",
        )
        for payload in invalid:
            with self.subTest(payload=payload[:48]):
                diagnostic, _, _ = run_case(payload)
                self.assertEqual((diagnostic["attempt_phase"], diagnostic["attempt_child_status"]),
                                 ("UNKNOWN", "UNKNOWN"))

        stale = sequence("1")
        diagnostic, _, _ = run_case(b"", initial=b"prior\n" + stale,
                                    producer=b"RBL_PRODUCE_BIN_PHASE|ATTEMPT|1\n")
        self.assertEqual((diagnostic["attempt_phase"], diagnostic["attempt_child_status"]),
                         ("UNKNOWN", "UNKNOWN"))
        diagnostic, _, _ = run_case(sequence("1"), producer=b"RBL_PRODUCE_BIN_PHASE|VERIFY_PRODUCER|1\n")
        self.assertNotIn("attempt_phase", diagnostic)
        diagnostic, _, _ = run_case(sequence("1"), producer=b"")
        self.assertNotIn("attempt_phase", diagnostic)
        for mutation in ("replacement", "hardlink", "mode", "changed"):
            with self.subTest(mutation=mutation):
                diagnostic, _, _ = run_case(sequence("1"), mutation=mutation)
                self.assertEqual(diagnostic.get("producer_phase"), "UNKNOWN")
                self.assertNotIn("attempt_phase", diagnostic)
        for current in (sequence("1") + b"x" * 65537 + b"\n",):
            diagnostic, _, _ = run_case(current)
            self.assertEqual(diagnostic.get("producer_phase"), "UNKNOWN")
            self.assertNotIn("attempt_phase", diagnostic)

        forged = DRIVER.BuildFailure("helper-action-failed:helper-failed")
        forged.helper_op, forged.producer_phase, forged.producer_status = "PRODUCE_BIN", "ATTEMPT", 1
        for phase, status in ((["COMPLETE"], 0), ("COMPLETE", True), ("COMPLETE", 256),
                              ("NO_PRIVATE_PHASE_SENTINEL", 0)):
            forged.attempt_phase, forged.attempt_child_status = phase, status
            value = DRIVER.failure_diagnostic(forged)
            self.assertEqual((value["attempt_phase"], value["attempt_child_status"]),
                             ("UNKNOWN", "UNKNOWN"))
            self.assertNotIn("NO_PRIVATE_PHASE_SENTINEL", json.dumps(value))
        for child_status in (0, 2, -9, "SPAWN_ERROR"):
            forged.attempt_phase, forged.attempt_child_status = "COMPLETE", child_status
            value = DRIVER.failure_diagnostic(forged)
            self.assertEqual((value["attempt_phase"], value["attempt_child_status"]),
                             ("UNKNOWN", "UNKNOWN"))
        forged.attempt_phase, forged.attempt_child_status = "COMPLETE", 0
        forged.helper_op = "MAKE_CONTEXT"
        self.assertNotIn("attempt_phase", DRIVER.failure_diagnostic(forged))
        forged.helper_op = "PRODUCE_BIN"
        forged.args = ("helper-action-failed:compose-build-failed",)
        self.assertNotIn("attempt_phase", DRIVER.failure_diagnostic(forged))

    def test_live_monitor_path_allows_active_compiler_and_labels_it_observed(self):
        monitor = fake_resource_monitor()
        active_compilers = [(12345, "cargo"), (12346, "rustc")]
        base_ns = monitor.build_deadline_ns - 10_000_000_000
        with mock.patch.object(DRIVER, "oom_kill_count", return_value=monitor.oom_baseline):
            with mock.patch.object(DRIVER, "compiler_pids", return_value=active_compilers):
                with mock.patch.object(DRIVER, "group_members", return_value=[100, 12345]) as children:
                    result = invoke_live_readline(
                        monitor, base_ns, [([], [], []), ([77], [], [])]
                    )
        self.assertEqual(result, b"READY")
        children.assert_not_called()
        observation = monitor.checks[0]
        self.assertEqual(observation["boundary"], "during:produce_bin")
        self.assertEqual(
            observation["compiler_processes"],
            [{"pid": 12345, "name": "cargo"}, {"pid": 12346, "name": "rustc"}],
        )
        self.assertFalse(observation["compiler_absence_required"])
        self.assertFalse(observation["helper_children_absence_required"])

    def test_idle_boundaries_still_reject_compilers_and_helper_children(self):
        monitor = fake_resource_monitor()
        base_ns = monitor.build_deadline_ns - 10_000_000_000
        with mock.patch.object(DRIVER.time, "monotonic_ns", return_value=base_ns):
            with mock.patch.object(DRIVER, "oom_kill_count", return_value=monitor.oom_baseline):
                with mock.patch.object(DRIVER, "compiler_pids", return_value=[(12345, "cargo")]):
                    with self.assertRaisesRegex(
                        DRIVER.BuildFailure, "compiler-process-present-at-boundary"
                    ):
                        monitor.check("after:producer", server_pid=100)
                with mock.patch.object(DRIVER, "compiler_pids", return_value=[]):
                    with mock.patch.object(DRIVER, "group_members", return_value=[100, 12345]):
                        with self.assertRaisesRegex(
                            DRIVER.BuildFailure, "owned-helper-has-unreaped-children"
                        ):
                            monitor.check("after:producer", server_pid=100)
                    with mock.patch.object(DRIVER, "group_members", return_value=[100]):
                        monitor.check("after:producer", server_pid=100)
        observation = monitor.checks[-1]
        self.assertEqual(observation["compiler_processes"], [])
        self.assertTrue(observation["compiler_absence_required"])
        self.assertTrue(observation["helper_children_absence_required"])

    def test_live_monitor_path_still_rejects_resource_oom_and_deadline_failures(self):
        cases = (
            (fake_resource_monitor("mem-available-below-floor"), "resource-policy-stop"),
            (fake_resource_monitor(), "kernel-oom-counter-changed"),
            (fake_resource_monitor(), "build-deadline-reserve-reached"),
        )
        for index, (monitor, expected) in enumerate(cases):
            with self.subTest(expected=expected):
                base_ns = monitor.build_deadline_ns - 10_000_000_000
                if expected == "build-deadline-reserve-reached":
                    monitor.build_deadline_ns = base_ns
                oom_count = monitor.oom_baseline + (1 if expected == "kernel-oom-counter-changed" else 0)
                with mock.patch.object(DRIVER, "oom_kill_count", return_value=oom_count):
                    with mock.patch.object(DRIVER, "compiler_pids", return_value=[(12345, "cargo")]):
                        with self.assertRaisesRegex(DRIVER.BuildFailure, expected):
                            invoke_live_readline(monitor, base_ns, [([], [], [])])

    def test_synthetic_producer_evidence_checks_exact_receipt_and_elf_binding(self):
        with tempfile.TemporaryDirectory(prefix="paper-fixture-receipts-", dir="/tmp") as tmp:
            state = pathlib.Path(tmp)
            for recipe, bins in DRIVER.RECEIPT_BINS.items():
                for index, bin_name in enumerate(sorted(bins)):
                    producer = state / "bundles" / recipe / ".release-build" / "producer" / bin_name
                    target = state / "bundles" / recipe / "target" / "release" / bin_name
                    producer.mkdir(parents=True, mode=0o700)
                    target.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
                    elf = b"\x7fELF" + recipe.encode() + bin_name.encode() + bytes([index])
                    target.write_bytes(elf)
                    os.chmod(target, 0o755)
                    artifact = {
                        "bin": bin_name, "source_commit": COMMIT,
                        "cargo_success": True, "binary_mode": "0755",
                        "compile_env": {"LAGRANGE_CODE_COMMIT": COMMIT},
                        "binary_sha256": hashlib.sha256(elf).hexdigest(),
                    }
                    artifact_raw = json.dumps(artifact, sort_keys=True).encode()
                    (producer / "artifact.json").write_bytes(artifact_raw)
                    (producer / "COMPLETE").write_bytes(
                        (hashlib.sha256(artifact_raw).hexdigest() + "\n").encode()
                    )
            evidence = DRIVER.producer_receipt_evidence(state, source_plan(), COMMIT)
            self.assertEqual(len(evidence), 4)
            self.assertEqual(
                {(row["recipe"], row["bin"]) for row in evidence},
                {(recipe, bin_name) for recipe, bins in DRIVER.RECEIPT_BINS.items()
                 for bin_name in bins},
            )
            self.assertTrue(all(row["source_commit"] == COMMIT for row in evidence))
            self.assertTrue(all(len(row["receipt_pair_sha256"]) == 64 for row in evidence))
            broken = state / "bundles" / "D7" / ".release-build" / "producer" / "paper-runner" / "COMPLETE"
            broken.write_bytes(b"changed\n")
            with self.assertRaisesRegex(DRIVER.BuildFailure, "producer-complete-binding-invalid"):
                DRIVER.producer_receipt_evidence(state, source_plan(), COMMIT)

    def test_private_interpolation_is_inert_and_has_no_provider_credentials(self):
        compose = (ROOT / "deploy/compose/compose.yml").read_text()
        with tempfile.TemporaryDirectory(prefix="paper-fixture-env-", dir="/tmp") as tmp:
            state = pathlib.Path(tmp) / "lagrange-paper-fixture-build-0123456789abcdef"
            values = DRIVER.disabled_env_values(compose, COMMIT, state, "0123456789abcdef")
        self.assertEqual(values["LAGRANGE_CODE_COMMIT"], COMMIT)
        self.assertEqual(values["LIVE_NODE_ACCOUNT_ID"], "")
        self.assertEqual(values["LIVE_NODE_MODE"], "disabled")
        self.assertEqual(values["API_INTERNAL_URL"], "http://127.0.0.1:1/prebuild-disabled")
        self.assertEqual(values["BACKTEST_MAX_QUEUED_BACKTESTS"], "0")
        self.assertEqual(values["OWNER_EQUITY_V2_ENTITLEMENT_SHA256"], "")
        self.assertTrue(all(value == "disabled" for key, value in values.items() if key.endswith("_MODE")))
        self.assertFalse(any("KIS" in key or "DART" in key or "APP_KEY" in key for key in values))
        self.assertFalse(any("TOKEN" in key or "PASSWORD" in key for key in values))
        environment = DRIVER.process_environment(
            pathlib.Path("/var/tmp/lagrange-paper-fixture-build-0123456789abcdef"),
            COMMIT, "paper-fixture-0123456789abcdef0123456789abcdef",
            pathlib.Path("/var/tmp/lagrange-paper-fixture-build-0123456789abcdef/.env"),
        )
        self.assertEqual(environment["CARGO_BUILD_JOBS"], "2")
        self.assertEqual(environment["COMPOSE_PARALLEL_LIMIT"], "1")
        self.assertFalse(any("KIS" in key or "DART" in key or "TOKEN" in key for key in environment))

    def test_shell_dispatch_has_build_only_docker_argv(self):
        server = DRIVER.BASH_SERVER
        syntax = subprocess.run(
            ["bash", "-n"], input=server, text=True, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, check=False,
        )
        self.assertEqual(syntax.returncode, 0, syntax.stderr)
        self.assertIn("release_build_layout_write_override", server)
        self.assertIn("release_build_layout_verify_image", server)
        self.assertIn("build --pull=false", server)
        self.assertNotRegex(server, r"docker\s+compose\s+(?:run|up|start|exec|restart)\b")
        self.assertNotIn("docker run", server)
        self.assertNotIn("docker exec", server)
        source = SCRIPT.read_text(encoding="utf-8")
        self.assertNotRegex(source, r"docker\s+compose\s+(?:run|up|start|exec|restart)\b")
        self.assertNotRegex(source, r"\b(?:systemctl|sudo)\b|docker\s+(?:run|exec)\b")
        self.assertLess(
            source.index('cleanup.push("buildx-builder", cleanup_ephemeral_builder)'),
            source.index('["docker", "buildx", "create"'),
        )

    def test_path_contract_requires_fresh_external_state_and_manifest_child(self):
        state = pathlib.Path("/var/tmp/lagrange-paper-fixture-build-0123456789abcdef")
        manifest = state / "release-images.manifest"
        result = DRIVER.validate_path_arguments(
            str(ROOT), COMMIT, str(state), str(manifest), time.monotonic_ns() + 600_000_000_000
        )
        self.assertEqual(result, (ROOT, state, manifest))
        with self.assertRaises(DRIVER.BuildFailure):
            DRIVER.validate_path_arguments(
                str(ROOT), "bad", str(state), str(manifest),
                time.monotonic_ns() + 600_000_000_000,
            )
        with self.assertRaises(DRIVER.BuildFailure):
            DRIVER.validate_path_arguments(
                str(ROOT), COMMIT, str(state), str(state.parent / "manifest"),
                time.monotonic_ns() + 600_000_000_000,
            )
        with self.assertRaises(DRIVER.DeadlineExceeded):
            DRIVER.validate_path_arguments(
                str(ROOT), COMMIT, str(state), str(manifest), time.monotonic_ns() + 1
            )
        with mock.patch.object(
            DRIVER.os.path, "lexists", side_effect=lambda path: str(path) == str(state)
        ):
            with self.assertRaisesRegex(DRIVER.BuildFailure, "state-or-manifest-already-exists"):
                DRIVER.validate_path_arguments(
                    str(ROOT), COMMIT, str(state), str(manifest),
                    time.monotonic_ns() + 600_000_000_000,
                )

    def test_fake_pipeline_is_ordered_and_builds_one_compose_service_at_a_time(self):
        plan = source_plan()
        backend = FakeBackend()
        guards = []
        images = DRIVER.execute_build(
            plan, backend, guards.append, COMMIT, pathlib.Path("/fixture/state"),
            pathlib.Path("/fixture/state/manifest.tmp"),
        )
        self.assertEqual(tuple(images), DRIVER.SERVICES)
        self.assertEqual(backend.max_compose_active, 1)
        self.assertTrue(backend.published)
        self.assertEqual(
            [event[1] for event in backend.events if event[0] in ("compose", "verify")],
            [value for service in DRIVER.SERVICES for value in (service, service)],
        )
        self.assertTrue(guards)
        expected_order = []
        for item in plan:
            if item.recipe == "NONE":
                self.assertIn(("prepare-none", item.service), backend.events)
                expected_order.append("prepare-none")
            else:
                self.assertIn(("native", f"/fixture/context/{item.recipe}"), backend.events)
                expected_order.extend(("context", "native", "context"))
                expected_order.extend("bin" for _ in item.bins)
                expected_order.extend(("bundle-create", "bundle-verify", "override"))
            expected_order.extend(("compose", "verify"))
        expected_order.append("reinspect-v3")
        self.assertEqual([event[0] for event in backend.events], expected_order)
        self.assertEqual(
            [(event[1], event[2]) for event in backend.events if event[0] == "bin"],
            [(item.recipe, bin_name) for item in plan if item.recipe != "NONE"
             for bin_name in item.bins],
        )
        self.assertEqual(backend.events[-1][0], "reinspect-v3")

    def test_fake_image_id_drift_blocks_manifest_candidate_acceptance(self):
        backend = FakeBackend(drift_at="paper-scheduler")
        with self.assertRaisesRegex(DRIVER.BuildFailure, "final-image-identity-drift"):
            DRIVER.execute_build(
                source_plan(), backend, lambda _: None, COMMIT,
                pathlib.Path("/fixture/state"), pathlib.Path("/fixture/state/manifest.tmp"),
            )
        self.assertFalse(backend.published)

    def test_partial_failure_and_bad_revision_never_reach_final_manifest_check(self):
        failed = FakeBackend(fail_at="compose")
        with self.assertRaises(subprocess.CalledProcessError):
            DRIVER.execute_build(
                source_plan(), failed, lambda _: None, COMMIT,
                pathlib.Path("/fixture/state"), pathlib.Path("/fixture/state/manifest.tmp"),
            )
        self.assertFalse(any(event[0] == "reinspect-v3" for event in failed.events))

        bad = FakeBackend(bad_revision_at="api-server")
        with self.assertRaisesRegex(DRIVER.BuildFailure, "verified-image-observation-invalid"):
            DRIVER.execute_build(
                source_plan(), bad, lambda _: None, COMMIT,
                pathlib.Path("/fixture/state"), pathlib.Path("/fixture/state/manifest.tmp"),
            )
        self.assertFalse(any(event[0] == "reinspect-v3" for event in bad.events))

    def test_deadline_and_nonzero_child_propagate_and_cleanup_runs_reverse_order(self):
        with tempfile.TemporaryDirectory(prefix="paper-fixture-child-", dir="/tmp") as tmp:
            state = pathlib.Path(tmp)
            (state / "logs").mkdir(mode=0o700)
            env = {"PATH": os.defpath, "HOME": tmp}
            with self.assertRaisesRegex(DRIVER.BuildFailure, "command-failed:fake-nonzero:7"):
                DRIVER.run_owned(
                    [sys.executable, "-c", "raise SystemExit(7)"],
                    "fake-nonzero", env, state, time.monotonic_ns() + 5_000_000_000,
                )
            with self.assertRaises(DRIVER.DeadlineExceeded):
                DRIVER.run_owned(
                    [sys.executable, "-c", "import time; time.sleep(20)"],
                    "fake-deadline", env, state, time.monotonic_ns() + 150_000_000,
                )
        proc = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(20)"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL, start_new_session=True,
        )
        DRIVER.stop_process_group(proc, grace=1.0)
        self.assertEqual(proc.poll() is None, False)
        self.assertEqual(DRIVER.group_members(proc.pid), [])
        order = []
        cleanup = DRIVER.CleanupStack()
        cleanup.push("first", lambda: order.append("first"))
        cleanup.push("second", lambda: order.append("second"))
        cleanup.close()
        self.assertEqual(order, ["second", "first"])
        failed_cleanup = DRIVER.CleanupStack()
        order = []
        failed_cleanup.push("first", lambda: order.append("first"))
        def fail_cleanup():
            order.append("second")
            raise RuntimeError("fixture-cleanup-failure")
        failed_cleanup.push("second", fail_cleanup)
        with self.assertRaises(DRIVER.CleanupFailure):
            failed_cleanup.close()
        self.assertEqual(order, ["second", "first"])
        cleanup.close()
        self.assertEqual(order, ["second", "first"])

    def test_synthetic_manifest_publish_is_private_and_no_replace(self):
        with tempfile.TemporaryDirectory(prefix="paper-fixture-manifest-", dir="/tmp") as tmp:
            state = pathlib.Path(tmp)
            candidate = state / "candidate"
            target = state / "manifest"
            data = synthetic_manifest_bytes()
            write_private_file(candidate, data)
            candidate_info = os.stat(candidate, follow_symlinks=False)
            digest = DRIVER.publish_no_replace(candidate, target, state, COMMIT)
            target_info = os.stat(target, follow_symlinks=False)
            self.assertEqual(digest, hashlib.sha256(data).hexdigest())
            self.assertFalse(os.path.lexists(candidate))
            self.assertEqual(target.read_bytes(), data)
            self.assertEqual((candidate_info.st_dev, candidate_info.st_ino),
                             (target_info.st_dev, target_info.st_ino))
            self.assertEqual(target_info.st_uid, os.geteuid())
            self.assertEqual(stat.S_IMODE(target_info.st_mode), 0o600)
            self.assertEqual(target_info.st_nlink, 1)

            candidate2 = state / "candidate2"
            write_private_file(candidate2, data)
            existing_info = os.stat(target, follow_symlinks=False)
            with self.assertRaisesRegex(DRIVER.BuildFailure, "manifest-target-already-exists"):
                DRIVER.publish_no_replace(candidate2, target, state, COMMIT)
            preserved = os.stat(target, follow_symlinks=False)
            self.assertEqual((preserved.st_dev, preserved.st_ino),
                             (existing_info.st_dev, existing_info.st_ino))
            self.assertEqual(target.read_bytes(), data)
            self.assertEqual(preserved.st_nlink, 1)
            self.assertTrue(os.path.lexists(candidate2))
            self.assertEqual(os.stat(candidate2, follow_symlinks=False).st_nlink, 1)

    def test_unlink_failure_rolls_back_only_own_manifest_link(self):
        with tempfile.TemporaryDirectory(prefix="paper-fixture-unlink-failure-", dir="/tmp") as tmp:
            state = pathlib.Path(tmp)
            candidate = state / "candidate"
            target = state / "manifest"
            write_private_file(candidate, synthetic_manifest_bytes())
            candidate_info = os.stat(candidate, follow_symlinks=False)
            real_unlink = os.unlink
            failed = False

            def fail_candidate_unlink(name, *args, **kwargs):
                nonlocal failed
                if name == candidate.name and kwargs.get("dir_fd") is not None and not failed:
                    failed = True
                    raise PermissionError("synthetic candidate unlink failure")
                return real_unlink(name, *args, **kwargs)

            with mock.patch.object(DRIVER.os, "unlink", side_effect=fail_candidate_unlink):
                with self.assertRaisesRegex(PermissionError, "candidate unlink failure"):
                    DRIVER.publish_no_replace(candidate, target, state, COMMIT)
            self.assertTrue(failed)
            self.assertTrue(os.path.lexists(candidate))
            self.assertFalse(os.path.lexists(target))
            retained = os.stat(candidate, follow_symlinks=False)
            self.assertEqual((retained.st_dev, retained.st_ino),
                             (candidate_info.st_dev, candidate_info.st_ino))
            self.assertEqual(retained.st_nlink, 1)

    def test_failure_after_candidate_retirement_removes_own_manifest(self):
        with tempfile.TemporaryDirectory(prefix="paper-fixture-post-unlink-failure-", dir="/tmp") as tmp:
            state = pathlib.Path(tmp)
            candidate = state / "candidate"
            target = state / "manifest"
            checks = state / "checks.json"
            write_private_file(candidate, synthetic_manifest_bytes())
            write_private_file(checks, b"private synthetic checks")
            checks_info = os.stat(checks, follow_symlinks=False)
            real_fsync = os.fsync
            calls = 0

            def fail_second_directory_fsync(fd):
                nonlocal calls
                calls += 1
                if calls == 2:
                    raise OSError("synthetic post-retirement fsync failure")
                return real_fsync(fd)

            with mock.patch.object(DRIVER.os, "fsync", side_effect=fail_second_directory_fsync):
                with self.assertRaisesRegex(OSError, "post-retirement fsync failure"):
                    DRIVER.publish_with_checks_cleanup(
                        candidate, target, state, COMMIT, checks,
                        (checks_info.st_dev, checks_info.st_ino),
                    )
            self.assertGreaterEqual(calls, 3)
            self.assertFalse(os.path.lexists(candidate))
            self.assertFalse(os.path.lexists(target))
            self.assertFalse(os.path.lexists(checks))

    def test_rollback_preserves_replaced_unrelated_destination(self):
        with tempfile.TemporaryDirectory(prefix="paper-fixture-foreign-target-", dir="/tmp") as tmp:
            state = pathlib.Path(tmp)
            candidate = state / "candidate"
            target = state / "manifest"
            write_private_file(candidate, synthetic_manifest_bytes())
            real_unlink = os.unlink
            replaced = False

            def replace_target_then_fail_candidate(name, *args, **kwargs):
                nonlocal replaced
                dirfd = kwargs.get("dir_fd")
                if name == candidate.name and dirfd is not None and not replaced:
                    replaced = True
                    real_unlink(target.name, dir_fd=dirfd)
                    fd = os.open(target.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                                 0o600, dir_fd=dirfd)
                    with os.fdopen(fd, "wb") as handle:
                        handle.write(b"unrelated destination")
                    raise PermissionError("synthetic candidate unlink failure")
                return real_unlink(name, *args, **kwargs)

            with mock.patch.object(DRIVER.os, "unlink", side_effect=replace_target_then_fail_candidate):
                with self.assertRaisesRegex(DRIVER.CleanupFailure, "publication-rollback-failed"):
                    DRIVER.publish_no_replace(candidate, target, state, COMMIT)
            self.assertTrue(replaced)
            self.assertTrue(os.path.lexists(candidate))
            self.assertEqual(target.read_bytes(), b"unrelated destination")
            target_info = os.stat(target, follow_symlinks=False)
            self.assertEqual(target_info.st_nlink, 1)
            self.assertEqual(stat.S_IMODE(target_info.st_mode), 0o600)

    def test_failed_publish_removes_only_its_unaccepted_checks(self):
        with tempfile.TemporaryDirectory(prefix="paper-fixture-checks-cleanup-", dir="/tmp") as tmp:
            state = pathlib.Path(tmp)
            candidate = state / "candidate"
            target = state / "manifest"
            checks = state / "checks.json"
            write_private_file(candidate, synthetic_manifest_bytes())
            write_private_file(target, b"pre-existing manifest")
            write_private_file(checks, b"private synthetic checks")
            target_info = os.stat(target, follow_symlinks=False)
            checks_info = os.stat(checks, follow_symlinks=False)
            with self.assertRaisesRegex(DRIVER.BuildFailure, "manifest-target-already-exists"):
                DRIVER.publish_with_checks_cleanup(
                    candidate, target, state, COMMIT, checks,
                    (checks_info.st_dev, checks_info.st_ino),
                )
            self.assertFalse(os.path.lexists(checks))
            self.assertTrue(os.path.lexists(candidate))
            preserved = os.stat(target, follow_symlinks=False)
            self.assertEqual((preserved.st_dev, preserved.st_ino),
                             (target_info.st_dev, target_info.st_ino))
            self.assertEqual(target.read_bytes(), b"pre-existing manifest")

    def test_child_failure_code_flows_from_current_complete_marker_to_public_report(self):
        import base64

        sentinel = "PRIVATE_CHILD_FAILURE_SENTINEL"
        with tempfile.TemporaryDirectory(prefix="child-failure-red-") as tmp:
            root = pathlib.Path(tmp)
            log = root / "logs/helper-server.stderr.log"
            log.parent.mkdir(mode=0o700)
            log.write_bytes((sentinel + "\n").encode())
            log.chmod(0o600)
            initial = log.stat()
            helper = object.__new__(DRIVER.HelperServer)
            helper._stderr_path = log
            helper._stderr_identity = (initial.st_dev, initial.st_ino)
            helper.deadline_ns = 1
            helper.proc = types.SimpleNamespace(stdin=io.BytesIO())
            phases = [
                (phase, "UNKNOWN") for phase in DRIVER.CAPTURE_ATTEMPT_PHASES[:6]
            ] + [("CHILD_RETURNED", "1")]
            phases += [(phase, "1") for phase in DRIVER.CAPTURE_ATTEMPT_PHASES[8:]]
            capture = b"".join(
                f"RBL_CAPTURE_ATTEMPT|{phase}|{status}\n".encode("ascii")
                for phase, status in phases
            )
            failure_code = b"RBL_CAPTURE_CHILD_FAILURE|cargo-json-invalid\n"
            producer = b"RBL_PRODUCE_BIN_PHASE|ATTEMPT|1\n"
            wire = b"ERR " + base64.b64encode(b"helper-failed")

            def reply(*_args):
                with log.open("ab") as stream:
                    stream.write(capture + failure_code + producer)
                    stream.flush()
                    os.fsync(stream.fileno())
                return wire

            helper.readline = mock.Mock(side_effect=reply)
            backend = FakeBackend()

            def fail_producer(recipe, bin_name, context, state):
                return helper.request("PRODUCE_BIN", (recipe, bin_name, context, str(state)))

            with mock.patch.object(backend, "produce_bin", side_effect=fail_producer):
                with self.assertRaises(DRIVER.BuildFailure) as raised:
                    DRIVER.execute_plan(source_plan(), backend, lambda _label: None,
                                        COMMIT, pathlib.Path("/fixture/state"))
            diagnostic = DRIVER.failure_diagnostic(raised.exception)
            rendered = io.StringIO()
            with contextlib.redirect_stderr(rendered):
                DRIVER.report_failure(raised.exception)
            self.assertNotIn(sentinel, rendered.getvalue())
            self.assertNotIn(str(log), rendered.getvalue())
            self.assertEqual(helper.readline.call_count, 1)
            self.assertEqual(diagnostic.get("attempt_failure_code"), "cargo-json-invalid")

    def test_capture_child_classifier_strips_only_canonical_buildkit_prefix_and_typed_codes(self):
        expected = {
            "TYPED": "cargo-json-invalid",
            "VALUE_ERROR": "cargo-json-invalid",
            "ALIAS": "builder-executable-missing",
            "DYNAMIC": "builder-cargo-build-failed",
            "PREFIX": "cargo-json-invalid",
            "PREFIX_ALIAS": "builder-executable-missing",
            "NATIVE": "native-identity-schema-invalid",
            "FEATURE": "feature-request-schema-invalid",
            "METADATA": "cargo-metadata-shape-invalid",
            "REPEATED": "cargo-json-invalid",
            "BRACKETED": "UNKNOWN",
            "MISSING_ELAPSED": "UNKNOWN",
            "LEADING_JUNK": "UNKNOWN",
            "BARE_ALIAS_CODE": "UNKNOWN",
            "VALUE_ERROR_ALIAS_CODE": "UNKNOWN",
            "DYNAMIC_OTHER_BIN": "UNKNOWN",
            "AMBIGUOUS": "UNKNOWN",
            "SUFFIX": "UNKNOWN",
            "SUBSTRING": "UNKNOWN",
            "BAD_UTF8": "UNKNOWN",
            "OVERSIZE": "UNKNOWN",
        }
        for case, code in expected.items():
            with self.subTest(case=case), tempfile.TemporaryDirectory(prefix="capture-code-boundary-") as tmp:
                result = run_closed_child_capture_case(case, pathlib.Path(tmp))
                self.assertEqual(result.returncode, 0, result.stderr[-300:])
                markers = [line for line in result.stderr.decode("ascii", "replace").splitlines()
                           if line.startswith("RBL_CAPTURE_CHILD_FAILURE|")]
                self.assertEqual(markers, ["RBL_CAPTURE_CHILD_FAILURE|" + code])

    def test_capture_child_classifier_requires_original_complete_file_identity(self):
        classifier = load_completed_child_classifier()["completed_child_failure_code"]

        def invoke(path, *, label="producer-D1-api-server", expected_digest=None,
                   expected_size=None, active_identity=None, info=None, status=1):
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
            try:
                opened = os.fstat(fd) if info is None else info
                digest = hashlib.sha256(path.read_bytes()).hexdigest() if expected_digest is None else expected_digest
                size = len(path.read_bytes()) if expected_size is None else expected_size
                identity = (opened.st_dev, opened.st_ino) if active_identity is None else active_identity
                os.lseek(fd, 2, os.SEEK_SET)
                before_offset = os.lseek(fd, 0, os.SEEK_CUR)
                code = classifier(fd, str(path), identity, opened, digest, size, status, label)
                self.assertEqual(os.lseek(fd, 0, os.SEEK_CUR), before_offset)
                return code
            finally:
                os.close(fd)

        with tempfile.TemporaryDirectory(prefix="child-code-fd-identity-") as tmp:
            root = pathlib.Path(tmp)
            data = b"cargo-json-invalid\n"
            source = root / "active.log"
            source.write_bytes(data)
            source.chmod(0o600)
            self.assertEqual(invoke(source), "cargo-json-invalid")
            original_info = source.stat(follow_symlinks=False)
            self.assertEqual(invoke(source, active_identity=(original_info.st_dev + 1,
                                                              original_info.st_ino)), "UNKNOWN")
            self.assertEqual(invoke(source, expected_digest="0" * 64), "UNKNOWN")
            self.assertEqual(invoke(source, expected_size=1048577), "UNKNOWN")
            self.assertEqual(invoke(source, label="native-identity"), "UNKNOWN")
            self.assertEqual(invoke(source, status=0), "UNKNOWN")

            linked = root / "hardlink.log"
            linked.write_bytes(data)
            linked.chmod(0o600)
            os.link(linked, root / "hardlink.other")
            self.assertEqual(invoke(linked), "UNKNOWN")

            changed = root / "changed.log"
            changed.write_bytes(data)
            changed.chmod(0o600)
            old_info = changed.stat(follow_symlinks=False)
            os.utime(changed, ns=(old_info.st_atime_ns, old_info.st_mtime_ns + 1_000_000))
            fd = os.open(changed, os.O_RDONLY | os.O_NOFOLLOW)
            try:
                now = os.fstat(fd)
                self.assertEqual(classifier(fd, str(changed), (old_info.st_dev, old_info.st_ino),
                    old_info, hashlib.sha256(data).hexdigest(), len(data), 1,
                    "producer-D1-api-server"), "UNKNOWN")
                os.fstat(fd)
            finally:
                os.close(fd)

            replaced = root / "replaced.log"
            replaced.write_bytes(data)
            replaced.chmod(0o600)
            replaced_fd = os.open(replaced, os.O_RDONLY | os.O_NOFOLLOW)
            try:
                before = os.fstat(replaced_fd)
                moved = root / "old.log"
                replaced.rename(moved)
                replaced.write_bytes(data)
                replaced.chmod(0o600)
                self.assertEqual(classifier(replaced_fd, str(replaced), (before.st_dev, before.st_ino),
                    before, hashlib.sha256(data).hexdigest(), len(data), 1,
                    "producer-D1-api-server"), "UNKNOWN")
            finally:
                os.close(replaced_fd)

            unsafe = root / "unsafe.log"
            unsafe.write_bytes(data)
            unsafe.chmod(0o644)
            self.assertEqual(invoke(unsafe), "UNKNOWN")

        with tempfile.TemporaryDirectory(prefix="child-code-read-error-") as tmp:
            path = pathlib.Path(tmp) / "active.log"
            path.write_bytes(b"cargo-json-invalid\n")
            path.chmod(0o600)
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
            try:
                info = os.fstat(fd)
                with mock.patch.object(os, "pread", side_effect=OSError("fixture")):
                    self.assertEqual(classifier(fd, str(path), (info.st_dev, info.st_ino), info,
                        hashlib.sha256(path.read_bytes()).hexdigest(), path.stat().st_size, 1,
                        "producer-D1-api-server"), "UNKNOWN")
            finally:
                os.close(fd)

    def test_capture_child_marker_only_follows_completed_nonzero_child_and_is_best_effort(self):
        expected = {
            "SUCCESS": [], "SPAWN": [], "PRECHILD": [], "POSTCHILD": [],
            "MARKER_WRITE": [],
        }
        for case, markers in expected.items():
            with self.subTest(case=case), tempfile.TemporaryDirectory(prefix="capture-code-lifecycle-") as tmp:
                result = run_closed_child_capture_case(case, pathlib.Path(tmp))
                self.assertEqual(result.returncode, 0, result.stderr[-300:])
                self.assertIn("RETURN=", result.stdout)
                actual = [line for line in result.stderr.decode("ascii", "replace").splitlines()
                          if line.startswith("RBL_CAPTURE_CHILD_FAILURE|")]
                self.assertEqual(actual, markers)
                if case in {"MARKER_WRITE", "POSTCHILD"}:
                    self.assertIn("RETURN=1\n", result.stdout)
                if case == "POSTCHILD":
                    self.assertIn("RBL_CAPTURE_ATTEMPT|COMPLETED_RECEIPT|1\n", result.stderr.decode("ascii", "replace"))
                    self.assertNotIn("RBL_CAPTURE_ATTEMPT|COMPLETE|1\n", result.stderr.decode("ascii", "replace"))
        with tempfile.TemporaryDirectory(prefix="capture-code-sequential-") as tmp:
            root = pathlib.Path(tmp)
            result = run_closed_child_capture_case("SEQUENTIAL", root)
            self.assertEqual(result.returncode, 0, result.stderr[-300:])
            markers = [line for line in result.stderr.decode("ascii").splitlines()
                       if line.startswith("RBL_CAPTURE_CHILD_FAILURE|")]
            self.assertEqual(markers, ["RBL_CAPTURE_CHILD_FAILURE|cargo-json-invalid",
                                       "RBL_CAPTURE_CHILD_FAILURE|builder-binary-invalid"])
            receipts = list((root / "logs/attempt-history/producer-D1-api-server/receipts").glob("*.json"))
            self.assertEqual(len(receipts), 2)

    def test_child_failure_code_source_provenance_is_derived_from_tracked_source(self):
        library = (ROOT / "scripts/ops/lib/release-build-layout.sh").read_text(encoding="utf-8")
        builder_source = SCRIPT.read_text(encoding="utf-8")
        names = (
            "rbl_validate_layout", "rbl_validate_feature_contract", "rbl_validate_native_identity",
            "rbl_compare_native_identity", "rbl_builder_native_identity",
            "rbl_validate_cargo_metadata_graph", "rbl_validate_d4_cargo_targets", "rbl_builder_guard",
            "rbl_builder_finish_guard", "rbl_builder_write_receipt", "rbl_request_compile_env_value",
            "rbl_builder_compile",
        )
        starts = list(re.finditer(r"(?m)^([A-Za-z_][A-Za-z0-9_]*)\(\) \{\n", library))
        typed = set()
        function_segments = {}
        for index, start in enumerate(starts):
            name = start.group(1)
            if name not in names:
                continue
            end = starts[index + 1].start() if index + 1 < len(starts) else len(library)
            segment = library[start.start():end]
            function_segments[name] = segment
            heredocs = re.findall(r"<<'PY'[^\n]*\n(.*?)\nPY(?:[^\n]*)", segment, re.DOTALL)
            for source in heredocs:
                tree = ast.parse(source)
                stack = [tree]
                while stack:
                    node = stack.pop()
                    if (isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and
                            node.func.id in {"SystemExit", "ValueError"} and node.args and
                            isinstance(node.args[0], ast.Constant) and
                            type(node.args[0].value) is str and
                            re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", node.args[0].value)):
                        typed.add(node.args[0].value)
                    stack.extend(ast.iter_child_nodes(node))
        self.assertEqual(set(function_segments), set(names))
        aliases = {
            "builder request/tool files are missing": "builder-request-tools-missing",
            "builder helper hash does not match host-supplied binding": "builder-helper-hash-binding-invalid",
            "builder layout hash does not match request binding": "builder-layout-hash-binding-invalid",
            "builder target platform is not linux/amd64": "builder-target-platform-invalid",
            "pinned Cargo metadata validation failed": "builder-pinned-cargo-metadata-failed",
            "target guard produced an empty reverse clean closure": "builder-target-guard-empty-clean-closure",
            "empty Cargo clean package": "builder-empty-clean-package",
            "target guard returned an unknown clean action": "builder-target-guard-action-invalid",
            "validated feature selection changed before Cargo": "builder-features-changed-before-cargo",
            "requested Cargo executable is missing": "builder-executable-missing",
            "requested Cargo executable is not executable": "builder-executable-not-executable",
        }
        self.assertEqual(len(typed), 130)
        tracked_producer_source = "\n".join(function_segments.values())
        for message in aliases:
            self.assertIn(message, tracked_producer_source)
        self.assertIn('rbl_die "Cargo build failed for $CARGO_BIN"',
                      function_segments["rbl_builder_compile"])
        public = typed | set(aliases.values()) | {"builder-cargo-build-failed"}
        self.assertEqual(len(public), 142)
        builder_tree = ast.parse(builder_source.split(MARKER, 1)[1].rsplit("\nPY\n", 1)[0])
        builder_codes = next(node.value for node in builder_tree.body
                              if isinstance(node, ast.Assign) and any(
                                  isinstance(target, ast.Name) and target.id == "CHILD_FAILURE_CODES"
                                  for target in node.targets))
        builder_code_text = ast.literal_eval(builder_codes.args[0].func.value)
        builder_codes = set(builder_code_text.split())
        self.assertEqual(builder_codes, public)
        self.assertEqual(set(aliases.values()) | {"builder-cargo-build-failed"},
                         builder_codes - typed)
        capture_tree = capture_python_module()
        capture_typed = next(node.value for node in capture_tree.body
                             if isinstance(node, ast.Assign) and any(
                                 isinstance(target, ast.Name) and target.id == "child_failure_codes"
                                 for target in node.targets))
        literals = {node.value for node in ast.walk(capture_typed)
                    if isinstance(node, ast.Constant) and type(node.value) is str}
        self.assertEqual(literals, typed)
        capture_codes = load_completed_child_classifier()
        self.assertEqual(capture_codes["child_failure_codes"], typed)
        self.assertEqual(capture_codes["child_failure_public_codes"], public)

    def test_child_failure_code_reader_uses_only_current_complete_request_window(self):
        import base64

        def attempt_records(status="1"):
            return [(phase, "UNKNOWN") for phase in CAPTURE_ATTEMPT_PHASES[:6]] + [
                ("CHILD_RETURNED", status)
            ] + [(phase, status) for phase in CAPTURE_ATTEMPT_PHASES[8:]]

        def run_case(case):
            sentinel = "PRIVATE_CHILD_CODE_SENTINEL"
            with tempfile.TemporaryDirectory(prefix="child-code-current-window-") as tmp:
                root = pathlib.Path(tmp)
                log = root / "logs/helper-server.stderr.log"
                log.parent.mkdir(mode=0o700)
                stale = b"RBL_CAPTURE_CHILD_FAILURE|cargo-json-invalid\n" if case == "stale" else b""
                log.write_bytes(stale + sentinel.encode() + b"\n")
                log.chmod(0o600)
                before = log.stat()
                helper = object.__new__(DRIVER.HelperServer)
                helper._stderr_path = log
                helper._stderr_identity = (before.st_dev, before.st_ino)
                helper.deadline_ns = 1
                helper.proc = types.SimpleNamespace(stdin=io.BytesIO())
                status = "SPAWN_ERROR" if case == "spawn" else ("0" if case == "zero" else "1")
                producer_status = 127 if status == "SPAWN_ERROR" else 1
                records = attempt_records(status) if status != "SPAWN_ERROR" else (
                    [(phase, "UNKNOWN") for phase in CAPTURE_ATTEMPT_PHASES[:6]] +
                    [("CHILD_SPAWN_ERROR", "SPAWN_ERROR")] +
                    [(phase, "SPAWN_ERROR") for phase in CAPTURE_ATTEMPT_PHASES[8:]])
                capture = b"".join(f"RBL_CAPTURE_ATTEMPT|{p}|{s}\n".encode()
                                    for p, s in records)
                code = b"RBL_CAPTURE_CHILD_FAILURE|cargo-json-invalid\n"
                if case == "missing" or case == "stale" or case in {"zero", "spawn"}:
                    child_markers = b""
                elif case == "duplicate":
                    child_markers = code + code
                elif case == "before-complete":
                    complete = b"RBL_CAPTURE_ATTEMPT|COMPLETE|1\n"
                    capture = capture.replace(complete, code + complete, 1)
                    child_markers = b""
                elif case == "unknown":
                    child_markers = b"RBL_CAPTURE_CHILD_FAILURE|PRIVATE_UNKNOWN_CODE\n"
                elif case == "malformed":
                    child_markers = b"RBL_CAPTURE_CHILD_FAILURE|cargo-json-invalid\xff\n"
                else:
                    child_markers = code
                if case != "before-complete":
                    current = capture + child_markers
                else:
                    current = capture
                final_phase = "VERIFY_PRODUCER" if case == "other-phase" else "ATTEMPT"
                producer = f"RBL_PRODUCE_BIN_PHASE|{final_phase}|{producer_status}\n".encode()
                reply = b"ERR " + base64.b64encode(b"helper-failed")

                def emit(*_args):
                    with log.open("ab") as stream:
                        stream.write(current + producer)
                        stream.flush()
                        os.fsync(stream.fileno())
                    return reply

                helper.readline = mock.Mock(side_effect=emit)
                backend = FakeBackend()

                def fail_producer(recipe, bin_name, context, state):
                    return helper.request("PRODUCE_BIN", (recipe, bin_name, context, str(state)))

                with mock.patch.object(backend, "produce_bin", side_effect=fail_producer):
                    with self.assertRaises(DRIVER.BuildFailure) as raised:
                        DRIVER.execute_plan(source_plan(), backend, lambda _label: None,
                                            COMMIT, pathlib.Path("/fixture/state"))
                diagnostic = DRIVER.failure_diagnostic(raised.exception)
                rendered = io.StringIO()
                with contextlib.redirect_stderr(rendered):
                    DRIVER.report_failure(raised.exception)
                self.assertNotIn(sentinel, rendered.getvalue())
                self.assertNotIn(str(log), rendered.getvalue())
                return diagnostic

        self.assertEqual(run_case("valid").get("attempt_failure_code"), "cargo-json-invalid")
        for case in ("missing", "duplicate", "before-complete", "unknown", "malformed", "stale"):
            with self.subTest(case=case):
                value = run_case(case)
                self.assertEqual((value["producer_phase"], value["producer_status"],
                                  value["attempt_phase"], value["attempt_child_status"]),
                                 ("ATTEMPT", 1, "COMPLETE", 1))
                self.assertEqual(value["attempt_failure_code"], "UNKNOWN")
        for case in ("zero", "spawn", "other-phase"):
            with self.subTest(case=case):
                self.assertNotIn("attempt_failure_code", run_case(case))
        forged = DRIVER.BuildFailure("helper-action-failed:helper-failed")
        forged.helper_op, forged.producer_phase, forged.producer_status = "PRODUCE_BIN", "ATTEMPT", 1
        forged.attempt_phase, forged.attempt_child_status = "COMPLETE", 1
        for malicious in ("PRIVATE_CHILD_CODE_SENTINEL", ["cargo-json-invalid"],
                          {"cargo-json-invalid": True}, True, 256):
            forged.attempt_failure_code = malicious
            value = DRIVER.failure_diagnostic(forged)
            self.assertEqual(value.get("attempt_failure_code"), "UNKNOWN")
            self.assertNotIn("PRIVATE_CHILD_CODE_SENTINEL", json.dumps(value))

    def test_source_inventory_matches_workspace_and_market_data_graph(self):
        import copy
        import posixpath
        import tomllib

        layout_path = ROOT / "deploy/build/release-build-layout.json"
        layout = json.loads(layout_path.read_text(encoding="utf-8"))
        root_manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        members = root_manifest["workspace"]["members"]

        def assert_member_binding(value, workspace_members):
            self.assertEqual(len(workspace_members), len(set(workspace_members)))
            member_set = set(workspace_members)
            package_dirs = value["source_inventory"]["package_dirs"]
            self.assertEqual(len(package_dirs), len(set(package_dirs)))
            self.assertEqual(set(package_dirs), member_set)
            package_paths = {record["path"] for record in value["packages"].values()}
            self.assertEqual(package_paths, member_set)
            self.assertEqual({record["path"] for record in value["packages"].values()},
                             set(package_dirs))

        def assert_manifest_graph(value, package_manifest, member_path):
            manifest = tomllib.loads(package_manifest.read_text(encoding="utf-8"))
            path_to_name = {record["path"]: name for name, record in value["packages"].items()}
            actual = set()
            dependency_tables = [manifest.get("dependencies", {}),
                                 manifest.get("build-dependencies", {})]
            for target in manifest.get("target", {}).values():
                dependency_tables.extend((target.get("dependencies", {}),
                                          target.get("build-dependencies", {})))
            for table in dependency_tables:
                for alias, record in table.items():
                    if isinstance(record, dict) and isinstance(record.get("path"), str):
                        resolved = posixpath.normpath(posixpath.join(
                            member_path, record["path"]))
                        self.assertIn(resolved, path_to_name)
                        actual.add(path_to_name[resolved])
            self.assertEqual(sorted(actual), value["packages"]["market-data"]["local_dependencies"])
            return manifest

        # This is intentionally workspace-derived, so a future Cargo member
        # makes the test fail until the source inventory adopts it.
        assert_member_binding(layout, members)
        self.assertIn("crates/r5-close-custody", members)
        market_manifest = assert_manifest_graph(
            layout, ROOT / "crates/market-data/Cargo.toml", "crates/market-data")
        custody_manifest = tomllib.loads(
            (ROOT / "crates/r5-close-custody/Cargo.toml").read_text(encoding="utf-8"))
        self.assertEqual(custody_manifest["package"]["name"], "r5-close-custody")
        self.assertIn("r5-close-custody", market_manifest["dependencies"])

        def dependency_closure(package, seen=None):
            seen = set() if seen is None else seen
            for dependency in layout["packages"][package]["local_dependencies"]:
                if dependency not in seen:
                    seen.add(dependency)
                    dependency_closure(dependency, seen)
            return seen

        for recipe in layout["recipes"].values():
            expected_clean = {recipe["package"]} | dependency_closure(recipe["package"])
            self.assertEqual(recipe["clean_packages"], sorted(expected_clean))

        def validate_real_layout(value):
            with tempfile.TemporaryDirectory(prefix="workspace-layout-validation-") as tmp:
                candidate = pathlib.Path(tmp) / "layout.json"
                candidate.write_text(json.dumps(value), encoding="utf-8")
                return subprocess.run(
                    ["bash", "--noprofile", "--norc", "-c",
                     '. "$1"; rbl_validate_layout "$2"', "_",
                     str(ROOT / "scripts/ops/lib/release-build-layout.sh"), str(candidate)],
                    cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                    text=True, timeout=5, check=False)

        self.assertEqual(validate_real_layout(layout).returncode, 0)
        missing = copy.deepcopy(layout)
        missing["source_inventory"]["package_dirs"].remove("crates/r5-close-custody")
        del missing["packages"]["r5-close-custody"]
        self.assertNotEqual(validate_real_layout(missing).returncode, 0)
        duplicate = copy.deepcopy(layout)
        duplicate["source_inventory"]["package_dirs"].append("crates/r5-close-custody")
        self.assertNotEqual(validate_real_layout(duplicate).returncode, 0)
        unexpected = copy.deepcopy(layout)
        unexpected["source_inventory"]["package_dirs"].append("crates/unapproved-member")
        self.assertNotEqual(validate_real_layout(unexpected).returncode, 0)
        for altered in (members[:-1], members + [members[0]],
                        members + ["crates/unapproved-member"]):
            with self.subTest(altered_member_count=len(altered)):
                with self.assertRaises(AssertionError):
                    assert_member_binding(layout, altered)
        removed_edge = copy.deepcopy(layout)
        removed_edge["packages"]["market-data"]["local_dependencies"].remove("r5-close-custody")
        with self.assertRaises(AssertionError):
            assert_manifest_graph(removed_edge, ROOT / "crates/market-data/Cargo.toml",
                                  "crates/market-data")

        # Exercise the real source hash and copy helpers on a small inert tree.
        with tempfile.TemporaryDirectory(prefix="workspace-member-copy-hash-") as tmp:
            root = pathlib.Path(tmp)
            fixture = root / "source"
            custody = fixture / "crates/r5-close-custody"
            custody.mkdir(parents=True)
            (fixture / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["crates/r5-close-custody"]\n', encoding="utf-8")
            (custody / "Cargo.toml").write_text(
                '[package]\nname = "r5-close-custody"\n', encoding="utf-8")
            lib = custody / "lib.rs"
            lib.write_bytes(b"pub fn custody_fixture() {}\n")
            fixture_layout = root / "layout.json"
            fixture_layout.write_text(json.dumps({"source_inventory": {
                "root_files": ["Cargo.toml"], "package_dirs": ["crates/r5-close-custody"],
                "external_compile_inputs": []}}), encoding="utf-8")
            helper = ROOT / "scripts/ops/lib/release-build-layout.sh"

            def source_hash():
                return subprocess.run(
                    ["bash", "--noprofile", "--norc", "-c",
                     '. "$1"; rbl_hash_source "$2" "$3" 0', "_",
                     str(helper), str(fixture), str(fixture_layout)], cwd=ROOT,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                    timeout=5, check=False)

            first_hash = source_hash()
            self.assertEqual(first_hash.returncode, 0, first_hash.stderr)
            destination = root / "copied/crates/r5-close-custody"
            copied = subprocess.run(
                ["bash", "--noprofile", "--norc", "-c",
                 '. "$1"; rbl_copy_entry "$2" "$3"', "_", str(helper),
                 str(custody), str(destination)], cwd=ROOT,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                timeout=5, check=False)
            self.assertEqual(copied.returncode, 0, copied.stderr)
            self.assertEqual((destination / "lib.rs").read_bytes(), lib.read_bytes())
            lib.write_bytes(b"pub fn custody_fixture() { true }\n")
            second_hash = source_hash()
            self.assertEqual(second_hash.returncode, 0, second_hash.stderr)
            self.assertNotEqual(first_hash.stdout.strip(), second_hash.stdout.strip())


if __name__ == "__main__":
    unittest.main()
