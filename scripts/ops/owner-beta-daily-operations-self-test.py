#!/usr/bin/env python3
"""Disposable root fixtures and injected failure ordering; no Docker/DB calls."""
from __future__ import annotations
import contextlib
import copy
import fcntl
import importlib.util
import io
import os
from pathlib import Path
import shutil
import sys
import tempfile
import types
import unittest
from unittest import mock

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "daily_operations_tested", ROOT / "scripts/ops/lib/owner-beta-daily-operations.py")
ops = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = ops
spec.loader.exec_module(ops)
COMMIT, PREVIOUS = "a" * 40, "b" * 40
IMAGE, HASH = "sha256:" + "c" * 64, "sha256:" + "d" * 64
SNAPSHOT = {"owner-beta-runner": {"generation": ("e" * 64, "created", "started", 0)}}


class DailyOperationsTests(unittest.TestCase):
    def setUp(self):
        if os.geteuid() != 0 or sys.platform != "linux":
            self.fail("actual root Linux fixture required")
        self.directory = Path(tempfile.mkdtemp(prefix="lagrange-daily-operations-", dir="/var/lib"))
        self.addCleanup(shutil.rmtree, self.directory)
        self.events = []
        self.values = {
            "LAGRANGE_CODE_COMMIT": COMMIT, "LAGRANGE_DATA_DIR": "/var/lib/lagrange/data",
            "LAGRANGE_ARTIFACTS_DIR": "/var/lib/lagrange/artifacts",
            "LAGRANGE_RUNTIME_SECRET_DIR": "/var/lib/lagrange/secrets", "POSTGRES_DB": "lagrange",
            "MAX_JOBS_PER_OWNER": "10", "PRICE_BETA_LEASE_SECS": "60",
            "PRICE_BETA_BACKOFF_SECS": "30", "OWNER_BETA_DAILY_MODE": "owner_only",
            "OWNER_BETA_ACCESS_MODE": "owner_only", "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1",
            "OWNER_BETA_PAPER_MODE": "disabled",
        }
        self.images = {k: IMAGE for k in ops.policy.MANIFEST_SERVICES}
        self.target = ops.Context("/opt/lagrange/releases/" + COMMIT, COMMIT,
                                  self.images, self.values.copy(), [])
        self.source = ops.Context("/opt/lagrange/releases/" + PREVIOUS, PREVIOUS,
                                  self.images, self.values.copy(), [])
        self.own = ops.Ownership("lagrange-station", PREVIOUS, IMAGE)
        self.binding = ops.runtime.Binding(
            "lagrange-station", COMMIT, IMAGE, HASH, self.target.release,
            self.values["LAGRANGE_DATA_DIR"], self.values["LAGRANGE_ARTIFACTS_DIR"],
            self.values["LAGRANGE_RUNTIME_SECRET_DIR"], "lagrange", "10", "60", "30")
        self.db = mock.Mock()
        self.db.control.side_effect = lambda r, c, v, a: self.events.append("db:" + a)
        self.db.status.return_value = "DISABLED"
        self.operation = ops.Operations(mock.Mock(), self.db, self.source, self.target,
                                        self.own, str(self.directory), "absent")

    @contextlib.contextmanager
    def activation(self, *, start_failure=False, observations=None):
        def event(name, result=None, failure=False):
            def invoke(*args, **kwargs):
                self.events.append(name)
                if failure:
                    raise ops.runtime.RuntimeErrorCode("INJECTED")
                return result
            return invoke
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(ops, "activation_binding",
                                                  return_value=(self.binding, {})))
            stack.enter_context(mock.patch.object(ops, "reject_ambient"))
            stack.enter_context(mock.patch.object(ops, "validate_activation_inputs",
                                                  side_effect=event("validate")))
            stack.enter_context(mock.patch.object(ops.runtime, "drain",
                                                  side_effect=event("drain")))
            stack.enter_context(mock.patch.object(ops.install, "check_state",
                                                  side_effect=event("state")))
            stack.enter_context(mock.patch.object(ops, "remove_stopped",
                                                  side_effect=event("remove")))
            stack.enter_context(mock.patch.object(ops, "start_target",
                                                  side_effect=event("start", failure=start_failure)))
            stack.enter_context(mock.patch.object(ops.runtime, "capture",
                                                  side_effect=event("capture", SNAPSHOT)))
            verifier = stack.enter_context(mock.patch.object(
                ops.runtime, "verify_running", side_effect=event("runtime", SNAPSHOT)))
            if observations is not None:
                verifier.side_effect = observations
            stack.enter_context(mock.patch.object(ops, "verify_api",
                                                  side_effect=event("api", ("api",))))
            yield

    def test_disable_database_failure_still_drains(self):
        self.db.control.side_effect = RuntimeError("unprinted")
        with mock.patch.object(ops.runtime, "drain") as drain:
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                self.operation.disable(SNAPSHOT)
            drain.assert_called_once_with(self.operation.docker, self.own, SNAPSHOT)

    def test_disable_context_failure_still_drains(self):
        self.source.values = None
        with mock.patch.object(ops, "read_context", side_effect=ValueError("unprinted")), \
                mock.patch.object(ops.runtime, "drain") as drain:
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                self.operation.disable(SNAPSHOT)
            drain.assert_called_once()
            self.db.control.assert_not_called()

    def test_disable_drain_failure_never_reports_success(self):
        with mock.patch.object(ops.runtime, "drain", side_effect=RuntimeError("unprinted")):
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                self.operation.disable(SNAPSHOT)
        self.assertEqual(self.events, ["db:disable"])

    def test_disable_ignores_activation_capacity_and_auth0(self):
        self.source.values["MAX_JOBS_PER_OWNER"] = ""
        with mock.patch.object(ops.runtime, "drain"), \
                mock.patch.object(ops, "validate_activation_inputs") as validation:
            self.operation.disable(SNAPSHOT)
        validation.assert_not_called()
        self.assertEqual(self.db.control.call_args.args[-1], "disable")

    def test_enable_disables_drains_checks_then_enables_last(self):
        with self.activation():
            self.assertEqual(self.operation.enable(SNAPSHOT), "ENABLED")
        self.assertEqual(self.events, ["validate", "db:disable", "drain", "state",
                                      "db:check-disabled", "remove", "start", "runtime", "api",
                                      "db:enable", "runtime", "api"])

    def test_partial_start_failure_disables_and_drains_target(self):
        with self.activation(start_failure=True):
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                self.operation.enable(SNAPSHOT)
        self.assertEqual(self.events[-4:], ["start", "db:disable", "capture", "drain"])
        self.assertNotIn("db:enable", self.events)

    def test_actual_policy_failure_never_enables(self):
        with self.activation(observations=[ops.runtime.RuntimeErrorCode("POLICY")]):
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                self.operation.enable(SNAPSHOT)
        self.assertNotIn("db:enable", self.events)
        self.assertEqual(self.events[-3:], ["db:disable", "capture", "drain"])

    def test_generation_change_after_enable_disables_and_drains(self):
        changed = copy.deepcopy(SNAPSHOT)
        changed["owner-beta-runner"]["generation"] = ("f" * 64, "created", "started", 0)
        with self.activation(observations=[SNAPSHOT, changed]):
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                self.operation.enable(SNAPSHOT)
        self.assertIn("db:enable", self.events)
        self.assertEqual(self.events[-3:], ["db:disable", "capture", "drain"])

    def test_cleanup_database_attempt_precedes_foreign_capture_refusal(self):
        with mock.patch.object(ops.runtime, "capture",
                               side_effect=ops.runtime.RuntimeErrorCode("FOREIGN")), \
                mock.patch.object(ops.runtime, "drain") as drain:
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                self.operation.cleanup_target(self.binding)
        self.assertEqual(self.events, ["db:disable"])
        drain.assert_not_called()

    def test_exact_running_enable_is_idempotent_without_restart(self):
        self.source.commit = COMMIT
        with self.activation():
            self.assertEqual(self.operation.enable(SNAPSHOT), "ALREADY_ENABLED")
        self.assertEqual(self.events, ["validate", "runtime", "api", "db:enable", "runtime", "api"])

    def test_repeated_enable_revocation_attempts_disable_and_drain(self):
        self.source.commit = COMMIT
        def control(release, commit, values, action):
            self.events.append("db:" + action)
            if action == "enable":
                raise ops.runtime.RuntimeErrorCode("REVOKED")
        self.db.control.side_effect = control
        with self.activation():
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                self.operation.enable(SNAPSHOT)
        self.assertEqual(self.events[-3:], ["db:disable", "capture", "drain"])

    def test_changed_data_binding_fails_before_stop(self):
        self.source.values["LAGRANGE_DATA_DIR"] = "/var/lib/foreign"
        with self.activation():
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                self.operation.enable(SNAPSHOT)
        self.assertEqual(self.events, ["validate"])

    def test_market_stream_api_restart_keeps_existing_confirmation_gate(self):
        self.target.overlays = ["-f", self.target.release + "/deploy/compose/compose.market-stream.yml"]
        with self.activation(), mock.patch.dict(os.environ, {}, clear=True):
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                self.operation.enable(SNAPSHOT)
        self.assertEqual(self.events, ["validate"])

    def test_approval_check_uses_verified_install_root_and_clean_environment(self):
        self.target.release = "/var/lib/controlled-install/releases/" + COMMIT
        approval = ("HISTORICAL_PRICE_BETA_APPROVAL status=ok operation=check "
                    "approval_registry_sha256=" + HASH + " approval_status=APPROVED "
                    "audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY "
                    "materialization_status=MATERIALIZED registration_status=UNREGISTERED "
                    "publication_status=NOT_PUBLISHED instrument_count=11 session_count=2452 bar_count=26972\n")
        results = [types.SimpleNamespace(returncode=0, stdout=b""),
                   types.SimpleNamespace(returncode=0, stdout=approval.encode())]
        with mock.patch.object(ops.subprocess, "run", side_effect=results) as run, \
                mock.patch.dict(os.environ, {"DOCKER_HOST": "tcp://untrusted.invalid:2375",
                                             "LAGRANGE_RELEASE_ROOT": "/untrusted"}, clear=True):
            ops.validate_activation_inputs(self.target)
        self.assertEqual(run.call_args.kwargs["env"], {
            **ops.ENVIRONMENT, "LAGRANGE_RELEASE_ROOT": "/var/lib/controlled-install"})
        self.assertEqual(run.call_args.args[0], [
            self.target.release + "/scripts/ops/kis-historical-price-beta-artifact.sh", "--approval-check"])

    def test_prepare_enabled_database_cannot_reset_state(self):
        self.db.status.return_value = "ENABLED"
        with mock.patch.object(ops, "stopped"), \
                mock.patch.object(ops, "activation_binding", return_value=(self.binding, {})), \
                mock.patch.object(ops.install, "provision_state") as provision:
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                self.operation.prepare(SNAPSHOT)
        provision.assert_not_called()
        self.db.control.assert_not_called()

    def test_first_off_release_guard_needs_no_database(self):
        with mock.patch.object(ops, "daily_present", return_value=False):
            self.assertEqual(ops.quiescence(self.operation, False), "LEGACY_OFF")
        self.db.status.assert_not_called()

    def test_process_guard_runs_before_any_database_query(self):
        with mock.patch.object(ops, "daily_present", return_value=True), \
                mock.patch.object(ops, "global_cohort"), \
                mock.patch.object(ops.runtime, "capture", return_value=SNAPSHOT), \
                mock.patch.object(ops, "stopped") as stopped:
            self.assertEqual(ops.quiescence(self.operation, True, check_database=False),
                             "PROCESSES_QUIESCENT")
        stopped.assert_called_once()
        self.db.status.assert_not_called()

    def test_enabled_database_blocks_full_release(self):
        self.db.status.return_value = "ENABLED"
        with mock.patch.object(ops, "daily_present", return_value=False), \
                mock.patch.object(ops, "global_cohort"), \
                mock.patch.object(ops.runtime, "capture", return_value={}), \
                mock.patch.object(ops, "stopped"), \
                mock.patch.object(ops, "read_context", return_value=(self.values, [])), \
                mock.patch.object(ops.install, "check_state") as check:
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                ops.quiescence(self.operation, True)
        check.assert_not_called()
        self.db.control.assert_not_called()

    def test_actual_release_lock_excludes_second_open_description(self):
        with ops.release_lock(str(self.directory)):
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                with ops.release_lock(str(self.directory)):
                    self.fail("second lock was accepted")
        with ops.release_lock(str(self.directory)):
            pass

    def test_inherited_lock_remains_held_after_guard_returns(self):
        path = self.directory / ".lagrange-release.lock"
        path.touch(mode=0o600)
        first = os.open(path, os.O_RDWR)
        second = os.open(path, os.O_RDWR)
        try:
            with ops.inherited_lock(str(self.directory), str(first)):
                pass
            with self.assertRaises(BlockingIOError):
                fcntl.flock(second, fcntl.LOCK_EX | fcntl.LOCK_NB)
        finally:
            os.close(first)
            os.close(second)

    def test_forged_inherited_descriptor_cannot_lock_another_file(self):
        (self.directory / ".lagrange-release.lock").touch(mode=0o600)
        other = self.directory / "other"
        other.touch(mode=0o600)
        descriptor = os.open(other, os.O_RDWR)
        try:
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                with ops.inherited_lock(str(self.directory), str(descriptor)):
                    self.fail("forged descriptor accepted")
        finally:
            os.close(descriptor)

    def context_fixture(self):
        release = self.directory / "releases" / COMMIT
        (release / "deploy/compose").mkdir(parents=True)
        lib = release / "scripts/ops/lib"
        lib.mkdir(parents=True)
        for name in ("dotenv.sh", "release-image-manifest.sh", "owner-beta-daily-release.sh"):
            shutil.copyfile(ROOT / "scripts/ops/lib" / name, lib / name)
        env = release / "deploy/compose/.env"
        env.write_text("".join(k + "=" + v + "\n" for k, v in self.values.items()))
        env.chmod(0o600)
        return release, env

    def test_real_context_emits_only_closed_nonsecret_fields(self):
        release, env = self.context_fixture()
        with env.open("a") as stream:
            stream.write("AUTH0_CLIENT_SECRET=synthetic-unprinted-value\n")
        values, overlays = ops.read_context(str(release), COMMIT, "disable")
        self.assertEqual(values, self.values)
        self.assertEqual(overlays, [])

    def test_real_context_rejects_shell_interpolation_without_execution(self):
        release, env = self.context_fixture()
        sentinel = self.directory / "must-not-exist"
        env.write_text(env.read_text() + "UNTRUSTED=$(touch " + str(sentinel) + ")\n")
        with self.assertRaises(ops.runtime.RuntimeErrorCode):
            ops.read_context(str(release), COMMIT, "disable")
        self.assertFalse(sentinel.exists())

    def test_absent_database_requires_pristine_filesystem_history(self):
        data = self.directory / "data"
        data.mkdir()
        config = ops.install.json_bytes(
            (ROOT / "tests/fixtures/owner-beta-daily-install/config.json").read_bytes())
        ops.pristine_state(str(data), config)
        ops.install.provision_state(str(data), config)
        ops.pristine_state(str(data), config)
        checkpoint = data / "owner-beta-daily/producer-state/checkpoint.json"
        value = ops.install.json_bytes(checkpoint.read_bytes())
        value.update(generation=1, last_started_unix_millis=1)
        checkpoint.write_bytes(ops.install.canonical(value))
        with self.assertRaises(ops.runtime.RuntimeErrorCode):
            ops.pristine_state(str(data), config)

    def test_absent_database_rejects_extra_admitted_leaf(self):
        data = self.directory / "data"
        data.mkdir()
        config = ops.install.json_bytes(
            (ROOT / "tests/fixtures/owner-beta-daily-install/config.json").read_bytes())
        ops.install.provision_state(str(data), config)
        (data / "owner-beta-daily/admitted/retained").write_bytes(b"synthetic")
        with self.assertRaises(ops.runtime.RuntimeErrorCode):
            ops.pristine_state(str(data), config)

    def manifest_fixture(self):
        release, _ = self.context_fixture()
        manifest = release / ".lagrange-release-manifest"
        manifest.write_text("LAGRANGE_RELEASE_MANIFEST_V2\ncommit|" + COMMIT + "\n" +
            "".join("image|" + service + "|lagrange-station-" + service + ":" + COMMIT +
                    "|" + IMAGE + "|" + COMMIT + "\n" for service in ops.policy.MANIFEST_SERVICES))
        manifest.chmod(0o600)
        return release, manifest

    def test_real_manifest_requires_canonical_service_and_revision_binding(self):
        release, manifest = self.manifest_fixture()
        self.assertEqual(ops.read_manifest(str(release), COMMIT), self.images)
        manifest.write_text(manifest.read_text().replace("image|api-server|", "image|foreign|"))
        with self.assertRaises(ops.policy.PolicyError):
            ops.read_manifest(str(release), COMMIT)

    def test_wrong_current_link_fails_before_daemon_access(self):
        release, _ = self.manifest_fixture()
        (self.directory / "current").symlink_to("releases/" + PREVIOUS)
        with mock.patch.object(ops, "ROOT", str(release)), \
                mock.patch.object(ops.runtime, "Docker") as docker, \
                contextlib.redirect_stderr(io.StringIO()):
            result = ops.main(["--operation", "disable", "--mode", "apply",
                               "--install-root", str(self.directory)])
        self.assertEqual(result, 1)
        docker.assert_not_called()

    def test_internal_mode_environment_is_rejected(self):
        with mock.patch.dict(os.environ, {"PRICE_BETA_DAILY_MODE": "owner_only"}, clear=True):
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                ops.reject_ambient(self.values)

    def test_unknown_argument_diagnostic_never_echoes_input(self):
        error = io.StringIO()
        with contextlib.redirect_stderr(error):
            self.assertEqual(ops.main(["--untrusted-secret-text"]), 1)
        self.assertEqual(error.getvalue(), "DAILY_OPERATION: FAILED\n")


if __name__ == "__main__":
    unittest.main(verbosity=2)
