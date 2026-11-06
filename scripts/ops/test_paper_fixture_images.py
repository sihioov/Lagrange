#!/usr/bin/env python3
"""Bounded fake-orchestration checks; no Docker, Cargo, DB, or product image runs."""

from __future__ import annotations

import json
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


if __name__ == "__main__":
    unittest.main()
