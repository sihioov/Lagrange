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
    def activation(self, *, start_failure=False, observations=None, reconcile_failure=False):
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
            stack.enter_context(mock.patch.object(ops.base, "verify"))
            stack.enter_context(mock.patch.object(ops.runtime, "drain",
                                                  side_effect=event("drain")))
            stack.enter_context(mock.patch.object(ops.install, "check_state",
                                                  side_effect=event("state")))
            stack.enter_context(mock.patch.object(ops, "load_local", return_value=types.SimpleNamespace(
                verify_disabled_state=event("reconcile", failure=reconcile_failure))))
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
                                      "db:check-disabled", "reconcile", "remove", "start", "runtime", "api",
                                      "db:enable", "runtime", "api"])

    def test_invalid_base_views_fail_before_start_or_database_enable(self):
        with self.activation(), mock.patch.object(ops.base, "verify", side_effect=ValueError("FIXTURE")):
            with self.assertRaises(ValueError):
                self.operation.enable(SNAPSHOT)
        self.assertNotIn("start", self.events)
        self.assertNotIn("db:enable", self.events)

    def test_prepare_publishes_base_views_before_database_control(self):
        with mock.patch.object(ops, "stopped"), \
                mock.patch.object(ops, "activation_binding", return_value=(self.binding, {})), \
                mock.patch.object(ops.install, "reader_guard"), \
                mock.patch.object(ops.install, "provision_state"), \
                mock.patch.object(ops.base, "provision", side_effect=lambda *args: self.events.append("base")) as provision:
            self.operation.prepare(SNAPSHOT)
        provision.assert_called_once_with(self.binding.artifact_root + "/historical-price-beta-root",
                                          self.binding.data_root + "/owner-beta-daily")
        self.assertEqual(self.events, ["base", "db:prepare"])

    def test_base_copy_failure_does_not_prepare_database(self):
        with mock.patch.object(ops, "stopped"), \
                mock.patch.object(ops, "activation_binding", return_value=(self.binding, {})), \
                mock.patch.object(ops.install, "reader_guard"), \
                mock.patch.object(ops.install, "provision_state"), \
                mock.patch.object(ops.base, "provision", side_effect=ValueError("FIXTURE")):
            with self.assertRaises(ValueError):
                self.operation.prepare(SNAPSHOT)
        self.db.control.assert_not_called()

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
        self.db.status.return_value = "ENABLED"
        with self.activation():
            self.assertEqual(self.operation.enable(SNAPSHOT), "ALREADY_ENABLED")
        self.assertEqual(self.events, ["validate", "runtime", "api", "db:enable", "runtime", "api"])

    def test_repeated_enable_revocation_attempts_disable_and_drain(self):
        self.source.commit = COMMIT
        self.db.status.return_value = "ENABLED"
        def control(release, commit, values, action):
            self.events.append("db:" + action)
            if action == "enable":
                raise ops.runtime.RuntimeErrorCode("REVOKED")
        self.db.control.side_effect = control
        with self.activation():
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                self.operation.enable(SNAPSHOT)
        self.assertEqual(self.events[-3:], ["db:disable", "capture", "drain"])

    def test_unreconciled_restore_cannot_replace_or_start_containers(self):
        with self.activation(reconcile_failure=True):
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                self.operation.enable(SNAPSHOT)
        self.assertEqual(self.events, ["validate", "db:disable", "drain", "state",
                                      "db:check-disabled", "reconcile"])

    def test_running_containers_with_disabled_db_cannot_bypass_restore_gate(self):
        self.source.commit = COMMIT
        with self.activation(reconcile_failure=True):
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                self.operation.enable(SNAPSHOT)
        self.assertIn("reconcile", self.events)
        self.assertNotIn("db:enable", self.events)

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

    def test_release_lock_yields_the_held_descriptor_for_history_proofs(self):
        with ops.release_lock(str(self.directory)) as descriptor:
            self.assertIsInstance(descriptor, int)
            self.assertGreaterEqual(descriptor, 3)
            named = os.stat(self.directory / ".lagrange-release.lock")
            held = os.fstat(descriptor)
            self.assertEqual((held.st_dev, held.st_ino), (named.st_dev, named.st_ino))
            second = os.open(self.directory / ".lagrange-release.lock", os.O_RDWR)
            try:
                with self.assertRaises(BlockingIOError):
                    fcntl.flock(second, fcntl.LOCK_EX | fcntl.LOCK_NB)
            finally:
                os.close(second)

    def test_inherited_lock_yields_its_descriptor(self):
        path = self.directory / ".lagrange-release.lock"
        path.touch(mode=0o600)
        descriptor = os.open(path, os.O_RDWR)
        try:
            with ops.inherited_lock(str(self.directory), str(descriptor)) as yielded:
                self.assertEqual(yielded, descriptor)
        finally:
            os.close(descriptor)

    def test_operations_pass_their_held_lock_to_every_activation_binding(self):
        self.operation.lock_fd = 11
        with mock.patch.object(ops, "stopped"), \
                mock.patch.object(ops, "activation_binding", return_value=(self.binding, {})) as binding, \
                mock.patch.object(ops.install, "reader_guard"), \
                mock.patch.object(ops.install, "provision_state"), \
                mock.patch.object(ops.base, "provision"):
            self.operation.prepare(SNAPSHOT)
        self.assertEqual(binding.call_args.kwargs, {"lock_fd": 11})
        with self.activation(), mock.patch.object(ops, "activation_binding",
                                                  return_value=(self.binding, {})) as binding:
            self.operation.enable(SNAPSHOT)
        self.assertEqual(binding.call_args.kwargs, {"lock_fd": 11})

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
        manifest.write_text(ops.policy.MANIFEST_FORMAT + "\ncommit|" + COMMIT + "\n" +
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


class PaperHistoryActivationTests(unittest.TestCase):
    """activation_binding mints a Paper history proof only on a marker-less root.

    The Paper release module is a stand-in with the frozen interface: the
    marker path calls reader_guard(history_present=None, reserve=True); the
    no-marker path calls _reader_guard_with_inventory(root, release, commit,
    lock_fd, True), which owns the single inventory loader, maps every inventory
    failure to PAPER_RELEASE_HISTORY_UNKNOWN and closes the proof. The real
    prove/guard/close interplay through Daily is exercised in
    scripts/ops/test_owner_beta_paper_history_inventory.py (DailyActivationTests).
    """

    LOCK_FD = 9

    def setUp(self):
        self.directory = Path(tempfile.mkdtemp(prefix="lagrange-paper-history-activation-"))
        self.addCleanup(shutil.rmtree, self.directory)
        self.release = str(self.directory / "releases" / COMMIT)
        self.events = []
        self.values = {
            "OWNER_BETA_DAILY_MODE": "owner_only", "OWNER_BETA_ACCESS_MODE": "owner_only",
            "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1", "OWNER_BETA_PAPER_MODE": "disabled",
            "LAGRANGE_DATA_DIR": "/d", "LAGRANGE_ARTIFACTS_DIR": "/a",
            "LAGRANGE_RUNTIME_SECRET_DIR": "/s", "POSTGRES_DB": "lagrange",
            "MAX_JOBS_PER_OWNER": "10", "PRICE_BETA_LEASE_SECS": "60",
            "PRICE_BETA_BACKOFF_SECS": "30",
        }
        self.outcome = "ABSENT"
        self.unknown = False

        def guard(install_root, release, commit, history_present, reserve=True, *, inventory=None):
            self.events.append(("guard", history_present, reserve, inventory))
            floor = Path(install_root) / ".owner-beta-paper-reader-floor"
            if floor.exists() or Path(install_root, ".owner-beta-paper").exists():
                return "PAPER_READER_REQUIRED"
            raise ops.runtime.RuntimeErrorCode("PAPER_RELEASE_HISTORY_UNKNOWN")

        def guard_with_inventory(install_root, release, commit, lock_fd, reserve):
            self.events.append(("inventory", install_root, release, commit, lock_fd, reserve))
            if self.unknown:
                raise ops.runtime.RuntimeErrorCode("PAPER_RELEASE_HISTORY_UNKNOWN")
            if self.outcome == "PRESENT":
                (Path(install_root) / ".owner-beta-paper-reader-floor").write_bytes(b"floor")
                return "PAPER_READER_REQUIRED"
            return "LEGACY_OFF"

        self.paper_release = types.SimpleNamespace(
            reader_guard=guard, _reader_guard_with_inventory=guard_with_inventory)
        stack = contextlib.ExitStack()
        self.addCleanup(stack.close)
        stack.enter_context(mock.patch.object(ops.install, "verify_config",
                                              return_value=({}, "sha256:" + "1" * 64)))
        self.load = stack.enter_context(mock.patch.object(ops, "load_local", return_value=self.paper_release))

    def bind(self, **kwargs):
        return ops.activation_binding(self.release, COMMIT, "lagrange-station",
                                      {"owner-beta-runner": IMAGE}, self.values, **kwargs)

    def floor(self):
        return (self.directory / ".owner-beta-paper-reader-floor").exists()

    def test_daily_has_no_private_inventory_loader(self):
        # The release module's _history_inventory() is the only loader.
        self.assertFalse(hasattr(ops, "history_inventory_module"))
        self.assertFalse(hasattr(ops, "PAPER_HISTORY_INVENTORY_MODULE"))

    def test_no_markers_absent_inventory_passes_and_creates_no_floor(self):
        self.bind(lock_fd=self.LOCK_FD)
        self.assertEqual(self.events, [("inventory", str(self.directory), self.release, COMMIT,
                                        self.LOCK_FD, True)])
        self.load.assert_called_once_with("daily_paper_release_binding", "owner-beta-paper-release.py")
        self.assertFalse(self.floor())

    def test_no_markers_present_inventory_reserves_the_floor(self):
        self.outcome = "PRESENT"
        self.bind(lock_fd=self.LOCK_FD)
        self.assertEqual([event[0] for event in self.events], ["inventory"])
        self.assertTrue(self.floor())

    def test_unknown_inventory_fails_closed_and_creates_no_floor(self):
        self.unknown = True
        with self.assertRaises(ops.runtime.RuntimeErrorCode):
            self.bind(lock_fd=self.LOCK_FD)
        self.assertEqual([event[0] for event in self.events], ["inventory"])
        self.assertFalse(self.floor())

    def test_each_marker_keeps_the_existing_guard_call_and_never_proves(self):
        markers = (".owner-beta-paper", ".owner-beta-paper-reader-floor")
        for name in markers:
            with self.subTest(marker=name):
                del self.events[:]
                path = self.directory / name
                path.mkdir() if name == ".owner-beta-paper" else path.write_bytes(b"floor")
                try:
                    self.bind(lock_fd=self.LOCK_FD)
                finally:
                    shutil.rmtree(path) if path.is_dir() else path.unlink()
                self.assertEqual(self.events, [("guard", None, True, None)])
        for name in ("process.json", "config.json"):
            with self.subTest(release_binding=name):
                del self.events[:]
                binding = Path(self.release) / "deploy/owner-beta-paper"
                binding.mkdir(parents=True, exist_ok=True)
                (binding / name).write_bytes(b"{}")
                try:
                    with self.assertRaises(ops.runtime.RuntimeErrorCode):
                        self.bind(lock_fd=self.LOCK_FD)
                finally:
                    (binding / name).unlink()
                self.assertEqual(self.events, [("guard", None, True, None)])

    def test_dangling_marker_symlink_counts_as_a_marker(self):
        (self.directory / ".owner-beta-paper-reader-floor").symlink_to("absent-marker")
        with self.assertRaises(ops.runtime.RuntimeErrorCode):
            self.bind(lock_fd=self.LOCK_FD)
        self.assertEqual(self.events, [("guard", None, True, None)])

    def test_without_a_held_lock_there_is_no_proof_and_the_guard_stays_unknown(self):
        with self.assertRaises(ops.runtime.RuntimeErrorCode):
            self.bind()
        self.assertEqual(self.events, [("guard", None, True, None)])
        self.assertFalse(self.floor())

    def test_only_an_integer_descriptor_can_bind_a_proof(self):
        for value in (str(self.LOCK_FD), True, False, 2, -1, 9.0, object()):
            with self.subTest(lock_fd=value):
                del self.events[:]
                with self.assertRaises(ops.runtime.RuntimeErrorCode) as caught:
                    self.bind(lock_fd=value)
                self.assertEqual(str(caught.exception), "DAILY_OPERATION_LOCK")
                self.assertEqual(self.events, [])
                self.load.assert_not_called()
        self.assertFalse(self.floor())

    def test_activation_still_requires_the_daily_modes_after_a_proven_absence(self):
        self.values["OWNER_BETA_PAPER_MODE"] = "owner_only"
        with self.assertRaises(ops.runtime.RuntimeErrorCode):
            self.bind(lock_fd=self.LOCK_FD)
        self.assertEqual([event[0] for event in self.events], ["inventory"])


class FirstSessionOwnershipTests(unittest.TestCase):
    PROJECT = "lagrange-station"
    HISTORICAL_IMAGE = "sha256:" + "f" * 64
    MANUAL_ID = "1" * 64
    TARGET_ROOT = "/opt/lagrange/releases/" + COMMIT
    VALUES = {
        "LAGRANGE_DATA_DIR": "/protected/data", "LAGRANGE_ARTIFACTS_DIR": "/protected/artifacts",
        "LAGRANGE_RUNTIME_SECRET_DIR": "/protected/runtime-secrets", "POSTGRES_DB": "lagrange",
        "MAX_JOBS_PER_OWNER": "4", "PRICE_BETA_LEASE_SECS": "60", "PRICE_BETA_BACKOFF_SECS": "1",
    }

    def setUp(self):
        self.docker = mock.Mock()
        self.containers = {self.MANUAL_ID: self.container("owner-beta-runner", self.MANUAL_ID)}
        self.docker.call.side_effect = lambda argv: "\n".join(
            i for i, value in self.containers.items()
            if "label=com.docker.compose.service=" + value["Config"]["Labels"]["com.docker.compose.service"] in argv)
        self.docker.service_ids.side_effect = lambda project, service: [
            i for i, value in self.containers.items()
            if (value["Config"]["Labels"]["com.docker.compose.project"],
                value["Config"]["Labels"]["com.docker.compose.service"]) == (project, service)]
        self.docker.object.side_effect = lambda kind, identity, *args: copy.deepcopy(
            self.containers[identity] if kind == "container" else
            {"Id": IMAGE, "Config": {"Labels": {"org.opencontainers.image.revision": COMMIT}}})
        self.first = ops.load_local("first_session_runtime_test", "owner-beta-daily-first-session.py")

    def container(self, service, identity):
        return {"Id": identity, "Image": IMAGE, "Created": "created", "RestartCount": 0,
                "State": {"Running": True, "Status": "running", "StartedAt": "started",
                          "Restarting": False, "Paused": False, "OOMKilled": False, "Error": "",
                          "Health": {"Status": "healthy"}},
                "HostConfig": {"ReadonlyRootfs": True, "Init": True, "Privileged": False,
                               "AutoRemove": False, "CapDrop": ["ALL"],
                               "SecurityOpt": ["no-new-privileges:true"],
                               "RestartPolicy": {"Name": "unless-stopped"}},
                "Config": {"User": "10001:10001", "Entrypoint": [ops.runtime.BINARY],
                           "Cmd": ["--artifact-root", "/data/owner-beta-artifacts"],
                           "Env": ["PRICE_BETA_DAILY_MODE=disabled", "APP_ENV=production",
                                   "DB_USER=worker", "DB_PASSWORD_FILE=/run/secrets/db_worker_password"],
                           "Labels": {"org.opencontainers.image.revision": COMMIT,
                                      "com.docker.compose.project": self.PROJECT,
                                      "com.docker.compose.oneoff": "False",
                                      "com.docker.compose.service": service}},
                "Mounts": [
                    {"Type": "bind", "Destination": "/data/owner-beta-artifacts", "RW": False,
                     "Source": self.VALUES["LAGRANGE_ARTIFACTS_DIR"] + "/historical-price-beta-root"},
                    {"Type": "bind", "Destination": "/run/secrets/db_worker_password", "RW": False,
                     "Source": self.VALUES["LAGRANGE_RUNTIME_SECRET_DIR"] + "/owner-beta-runner/db_worker_password"},
                ]}

    def main_preflight(self, *, mode="daily"):
        def images(path, commit):
            return {"owner-beta-runner": IMAGE if commit == COMMIT else self.HISTORICAL_IMAGE,
                    "db-migrate": IMAGE}

        database = mock.Mock()
        database.status.return_value = "DISABLED"
        database.execute.return_value = "DAILY_FIRST_SESSION: PREFLIGHT"
        with (mock.patch.object(ops, "ROOT", self.TARGET_ROOT),
              mock.patch.object(ops.install, "require_root"),
              mock.patch.object(ops.install, "verify_config", return_value=({}, HASH)),
              mock.patch.object(ops.install, "reader_guard"),
              mock.patch.object(ops.policy, "validate_current_release"),
              mock.patch.object(ops, "read_manifest", side_effect=images),
              mock.patch.object(ops, "project_binding", return_value=(self.PROJECT, mode)) as project,
              mock.patch.object(ops.runtime, "Docker", return_value=self.docker),
              mock.patch.object(ops, "Database", return_value=database),
              mock.patch.object(ops, "release_lock", return_value=contextlib.nullcontext(9)),
              mock.patch.object(ops, "read_context", side_effect=lambda *args: (self.VALUES.copy(), [])),
              mock.patch.object(ops, "activation_binding", return_value=(None, {})),
              mock.patch.object(ops, "load_local", return_value=self.first),
              mock.patch.object(self.first, "render", return_value="synthetic fixture SQL") as render,
              mock.patch.object(self.first, "pristine_guard", return_value=contextlib.nullcontext()),
              mock.patch.object(self.first, "run", wraps=self.first.run) as run,
              contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO())):
            result = ops.main(["--operation", "rebind-first-session", "--mode", "preflight",
                               "--install-root", "/opt/lagrange", "--from-commit", PREVIOUS])
        return result, project, run, render, database

    def test_rebind_uses_current_runtime_and_historical_config(self):
        result, project, run, render, database = self.main_preflight()
        self.assertEqual(result, 0)
        project.assert_called_once_with(COMMIT, COMMIT)
        _, operation, snapshot = run.call_args.args
        self.assertEqual(operation.source.commit, PREVIOUS)
        self.assertEqual(operation.target.commit, COMMIT)
        self.assertEqual(operation.ownership, ops.Ownership(self.PROJECT, COMMIT, IMAGE))
        self.assertEqual(set(snapshot), {"owner-beta-runner"})
        render.assert_called_once_with(ops.install, operation.source, operation.target,
                                       operation.target.values, apply=False)
        database.execute.assert_called_once_with(operation.target.values, "synthetic fixture SQL",
                                                 {"DAILY_FIRST_SESSION: PREFLIGHT"})

    def test_rebind_rejects_missing_authenticated_project(self):
        result, _, run, _, database = self.main_preflight(mode="absent")
        self.assertEqual(result, 1)
        run.assert_not_called()
        database.execute.assert_not_called()

    def test_rebind_checks_closed_manual_runner_and_rejects_live_daily_writers(self):
        current = ops.Ownership(self.PROJECT, COMMIT, IMAGE)
        operation = types.SimpleNamespace(
            docker=self.docker, ownership=current, target=types.SimpleNamespace(values=self.VALUES))
        snapshot = ops.runtime.capture(self.docker, current)
        self.first.require_quiescent(ops, operation, snapshot)

        manual = self.containers[self.MANUAL_ID]
        invalid = copy.deepcopy(manual)
        invalid["Config"]["Env"].append("PRICE_BETA_DAILY_CONFIG_FILE=/run/owner-beta-daily/config.json")
        wrong_mode = copy.deepcopy(manual)
        wrong_mode["Config"]["Env"] = []
        writable = copy.deepcopy(manual)
        writable["Mounts"][0]["RW"] = True
        wrong_command = copy.deepcopy(manual)
        wrong_command["Config"]["Cmd"] = ["daily-producer"]
        variants = [invalid, wrong_mode, writable, wrong_command]
        for section, key, value in (
            ("State", "Health", {"Status": "unhealthy"}), ("State", "Paused", True),
            ("State", "Restarting", True), ("State", "OOMKilled", True),
            ("State", "Status", "exited"), ("HostConfig", "Privileged", True),
            ("HostConfig", "CapDrop", []), ("HostConfig", "CapAdd", ["NET_ADMIN"]),
            ("HostConfig", "SecurityOpt", []), ("Config", "User", "0:0"),
        ):
            variant = copy.deepcopy(manual)
            variant[section][key] = value
            variants.append(variant)
        for index in range(2):
            variant = copy.deepcopy(manual)
            variant["Mounts"][index]["Source"] = "/foreign/release/path"
            variants.append(variant)
        for variant in variants:
            self.containers[self.MANUAL_ID] = variant
            with self.assertRaisesRegex(ops.runtime.RuntimeErrorCode, "DAILY_OPERATION_NOT_QUIESCENT"):
                self.first.require_quiescent(ops, operation, snapshot)
        self.containers[self.MANUAL_ID] = manual

        stopped = copy.deepcopy(manual)
        stopped["State"].update(Running=False, Status="exited", Health={"Status": "unhealthy"}, ExitCode=0)
        stopped["HostConfig"]["RestartPolicy"]["Name"] = "no"
        self.containers[self.MANUAL_ID] = stopped
        self.first.require_quiescent(ops, operation, snapshot)
        self.containers[self.MANUAL_ID] = manual

        manual["RestartCount"] = 1
        with self.assertRaisesRegex(ops.runtime.RuntimeErrorCode, "DAILY_FIRST_SESSION_COHORT"):
            self.first.require_quiescent(ops, operation, snapshot)
        manual["RestartCount"] = 0

        daily_id = "2" * 64
        self.containers[daily_id] = self.container("owner-beta-daily-producer", daily_id)
        both = ops.runtime.capture(self.docker, current)
        with self.assertRaisesRegex(ops.runtime.RuntimeErrorCode, "DAILY_OPERATION_NOT_QUIESCENT"):
            self.first.require_quiescent(ops, operation, both)
        with self.assertRaisesRegex(ops.runtime.RuntimeErrorCode, "DAILY_FIRST_SESSION_COHORT"):
            self.first.require_quiescent(ops, operation, snapshot)

        del self.containers[daily_id]
        manual["Config"]["Labels"]["com.docker.compose.project"] = "foreign"
        with self.assertRaisesRegex(ops.runtime.RuntimeErrorCode, "DAILY_RUNTIME_OWNER"):
            self.first.require_quiescent(ops, operation, snapshot)


if __name__ == "__main__":
    unittest.main(verbosity=2)
