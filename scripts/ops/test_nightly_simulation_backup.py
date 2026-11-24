from __future__ import annotations

import contextlib
import datetime as dt
from dataclasses import replace
import importlib.util
import io
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

def load_fixture(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


nightly = load_fixture("nightly_simulation_backup_test", "nightly-simulation-backup.py")
sequencer_fixture = load_fixture("nightly_preupgrade_fixture", "test_preupgrade_simulation_backup.py")


class NightlyTests(unittest.TestCase):
    def setUp(self):
        stack = contextlib.ExitStack()
        self.addCleanup(stack.close)
        root = Path(stack.enter_context(tempfile.TemporaryDirectory()))
        self.layout = nightly.Layout(str(root / "install"), str(root / "backup-state"),
                                     str(root / "nightly.json"), str(root / "release-manifests"))
        self.state = Path(self.layout.state_root)
        self.state.mkdir(mode=0o700)
        self.commit = "a" * 40
        self.tool = "b" * 40
        release = Path(self.layout.install_root) / "releases" / self.commit
        entry = release / nightly.SEQUENCER
        entry.parent.mkdir(parents=True, mode=0o700)
        entry.write_bytes(b"fixture sequencer\n")
        entry.chmod(0o644)
        (Path(self.layout.install_root)).chmod(0o700)
        (Path(self.layout.install_root) / "current").symlink_to("releases/" + self.commit)
        maintenance = Path(self.layout.config_root) / "maintenance.conf"
        maintenance.parent.mkdir(mode=0o700)
        maintenance.write_bytes(b"fixture maintenance\n")
        maintenance.chmod(0o600)
        self.config = {
            "schema": nightly.CONFIG_SCHEMA, "application_commit": self.commit, "tool_commit": self.tool,
            "config_file": str(maintenance), "config_sha256": nightly.digest(maintenance.read_bytes()),
            "expected_db_first_session": "2026-10-21", "expected_config_first_session": "2026-10-22",
        }
        self.write_config()
        self.now = dt.datetime(2026, 11, 21, 17, 15, tzinfo=dt.timezone.utc)
        self.date = "2026-11-22"
        self.run_path = self.state / self.date
        self.calls = []
        self.fail_preflight = False
        self.fail_apply = False
        self.status = ("active", "running", "running")
        uid, gid = os.geteuid(), os.getegid()
        real_directory, real_file, real_create = (nightly.install.directory_info,
                                                  nightly.install.file_info, nightly.install.create_file)

        @contextlib.contextmanager
        def directory(path):
            fd = os.open(path, nightly.install.DIR_FLAGS)
            try:
                real_directory(fd, uid=uid)
                yield fd
            finally:
                os.close(fd)

        # Test under an unprivileged temp root with real FDs, modes, ACLs and files.
        stack.enter_context(mock.patch.object(nightly.install, "root_directory", directory))
        stack.enter_context(mock.patch.object(nightly.install, "directory_info",
            side_effect=lambda fd, uid=0, gid=None, mode=None: real_directory(fd, os.geteuid(),
                None if gid is None else os.getegid(), mode)))
        stack.enter_context(mock.patch.object(nightly.install, "file_info",
            side_effect=lambda fd, uid=0, gid=None, mode=None: real_file(fd, os.geteuid(),
                None if gid is None else os.getegid(), mode)))
        stack.enter_context(mock.patch.object(nightly.install, "create_file",
            side_effect=lambda fd, name, data, uid, gid, mode: real_create(fd, name, data,
                os.geteuid(), os.getegid(), mode)))
        self.provided = dict(nightly.CONFIRMATION_VALUES)
        self.write_credentials()

    def test_combined_config_requires_matching_dates_and_its_final_control_receipt(self):
        self.config.update(schema=nightly.COMBINED_CONFIG_SCHEMA, backup_profile="daily-simulation")
        self.write_config()
        with self.assertRaises(nightly.rg.ToolError):
            nightly.read_config(self.layout)
        self.config["expected_db_first_session"] = self.config["expected_config_first_session"]
        self.write_config()
        protected = nightly.read_config(self.layout)
        arguments = nightly.child_arguments(self.layout, protected)
        self.assertEqual(arguments["backup_profile"], "daily-simulation")
        record = {"arguments": arguments, "nightly_completed": True,
                  "steps": {name: {"state": "done", "code": None} for name in nightly.STEPS}}
        self.assertFalse(nightly.complete(record))
        record["steps"][nightly.DAILY_CONTROL_STEP] = {"state": "done", "code": None}
        self.assertTrue(nightly.complete(record))

    def write_credentials(self):
        credentials = Path(self.layout.config_root) / "nightly-simulation" / self.commit
        credentials.mkdir(parents=True, mode=0o700, exist_ok=True)
        self.credential = credentials / nightly.CREDENTIAL
        self.credential.write_bytes(nightly.rg.canonical({"schema": nightly.ROLLOUT_SCHEMA,
            "application_commit": self.commit, "operation": "nightly-simulation-backup",
            "confirmations": self.provided}))
        self.credential.chmod(0o400)
        self.environment = {"CREDENTIALS_DIRECTORY": str(credentials),
                            nightly.CONFIRMATIONS[0]: "not-the-owner-value", "UNRELATED": "forbidden"}

    def write_config(self):
        config_path = Path(self.layout.config_file)
        config_path.write_bytes(nightly.rg.canonical(self.config))
        config_path.chmod(0o600)

    def child(self, argv, env, timeout, lock_fd):
        self.calls.append((argv, env, lock_fd))
        if "--preflight" in argv:
            if self.fail_preflight:
                return 1, b"", b"PREUPGRADE_BACKUP: FAILED PREUPGRADE_CONFIRMATION\n"
            timer, sim, v2 = self.status
            return 0, (f"PREUPGRADE_BACKUP: OBSERVER timer={timer}\n"
                       "PREUPGRADE_BACKUP: CONFIRMATIONS owner_equity_v2=required owner_market_stream=not_required\n"
                       f"PREUPGRADE_BACKUP: NIGHTLY_COHORT simulation={sim} owner_v2={v2}\n"
                       f"PREUPGRADE_BACKUP: NIGHTLY_UNTOUCHED_SHA256 {self.untouched()}\n"
                       "PREUPGRADE_BACKUP: PREFLIGHT\n").encode(), b""
        self.assertEqual(argv[4], str(Path(self.layout.install_root) / "releases" / self.commit / nightly.SEQUENCER))
        self.assertIn("--nightly", argv)
        self.assertIn("--apply", argv)
        self.assertEqual(argv[argv.index("--state-dir") + 1], str(self.run_path))
        self.assertEqual(argv[argv.index("--stopped-worker-from-commit") + 1], self.commit)
        self.assertEqual(env[nightly.CONFIRMATIONS[0]], self.provided[nightly.CONFIRMATIONS[0]])
        self.assertNotIn("UNRELATED", env)
        self.assertNotIn(nightly.CONFIRMATIONS[1], env)
        self.assertTrue((self.run_path / nightly.CLAIM).is_file())
        if self.fail_apply:
            return None, b"", b""
        self.write_progress()
        return 0, nightly.PASS_LINES[self.status[1] == "running"], b""

    def services(self):
        worker = {"running": True, "generation": ["f" * 64, "created", "started", 0],
                  "image": "sha256:" + "c" * 64, "restart_policy": {"Name": "always", "MaximumRetryCount": 0}}
        services = dict.fromkeys(nightly.SERVICES, worker)
        if self.status[1] == "absent":
            services["owner-beta-simulation-runner"] = None
        return services

    def untouched(self):
        services = self.services()
        return nightly.digest(nightly.rg.canonical({k: services[k] for k in nightly.UNTOUCHED}))

    def write_progress(self, *, steps=None, recovered=False):
        record = {"schema": nightly.PROGRESS_SCHEMA, "arguments": nightly.child_arguments(self.layout, self.config),
                  "steps": {name: {"state": "done", "code": None} for name in nightly.STEPS} if steps is None else steps,
                  "nightly": {"services": self.services(), "observer_timer_active": True}, "beta": None,
                  "observer": None}
        if steps is None:
            record.update(nightly_completed=True, simulation_worker_captured=self.status[1] == "running")
        if recovered:
            record["nightly_recovered"] = True
        (self.run_path / nightly.CHILD_LOCK).touch(mode=0o600)
        path = self.run_path / nightly.PROGRESS
        path.write_bytes(nightly.rg.canonical(record))
        path.chmod(0o600)

    def invoke(self, *args):
        out, err = io.StringIO(), io.StringIO()
        code = nightly.main(["--config-file", self.layout.config_file, *args], layout=self.layout,
                            runner=self.child, now=lambda: self.now, environ=self.environment, out=out, err=err)
        return code, out.getvalue(), err.getvalue()

    def test_preflight_readonly_and_days_use_asia_seoul(self):
        self.assertEqual(self.invoke("--preflight")[0], 0)
        self.assertEqual(list(self.state.iterdir()), [])
        self.assertEqual(self.invoke("--apply")[0], 0)
        self.assertTrue((self.run_path / nightly.RESULT).is_file())
        before = len(self.calls)
        self.assertIn("NIGHTLY_ALREADY_RAN", self.invoke("--apply")[2])
        self.assertEqual(len(self.calls), before)

    def test_missed_timer_window_fails_without_claim_or_service_changes(self):
        case = self.connect_sequencer()
        for local_time in ("02:14:59", "02:46:00.000001", "12:00:00"):
            with self.subTest(local_time=local_time):
                self.now = dt.datetime.fromisoformat(f"{self.date}T{local_time}+09:00")
                self.assertEqual(self.invoke("--apply"), (1, "", "NIGHTLY_BACKUP: FAILED NIGHTLY_OUTSIDE_WINDOW\n"))
                self.assertEqual(list(self.state.iterdir()), [])
                self.assertEqual(self.calls, [])
                self.assertEqual(case.mutations(), [])
        self.assertEqual(self.invoke("--preflight"), (0, "NIGHTLY_BACKUP: PREFLIGHT\n", ""))
        self.assertEqual(case.mutations(), [])

    def test_timer_start_accepts_last_in_window_and_next_day_after_a_miss(self):
        self.now = dt.datetime(2026, 11, 22, 3, tzinfo=dt.timezone.utc)
        self.assertIn("NIGHTLY_OUTSIDE_WINDOW", self.invoke("--apply")[2])
        self.now = dt.datetime(2026, 11, 22, 17, 46, tzinfo=dt.timezone.utc)
        self.date = "2026-11-23"
        self.run_path = self.state / self.date
        self.assertEqual(self.invoke("--apply"), (0, f"NIGHTLY_BACKUP: PASS date={self.date}\n", ""))

    def test_failed_child_blocks_new_dates_and_ack_never_retries_day(self):
        self.fail_apply = True
        self.assertNotEqual(self.invoke("--apply")[0], 0)
        self.now += dt.timedelta(days=1)
        self.assertIn("NIGHTLY_UNRESOLVED", self.invoke("--apply")[2])
        self.assertEqual(self.invoke("--acknowledge", self.date)[0], 0)
        self.now -= dt.timedelta(days=1)
        self.assertIn("NIGHTLY_ALREADY_RAN", self.invoke("--apply")[2])

    def test_progress_with_side_effects_needs_recovery_and_original_cohort(self):
        self.fail_apply = True
        self.assertNotEqual(self.invoke("--apply")[0], 0)
        steps = {"stop-beta": {"state": "failed", "code": "PREUPGRADE_BETA"}}
        self.write_progress(steps=steps)
        self.assertIn("NIGHTLY_RECOVERY_REQUIRED", self.invoke("--acknowledge", self.date)[2])
        steps["restore-observer"] = {"state": "recovered", "code": None}
        self.write_progress(steps=steps, recovered=True)
        self.status = ("active", "absent", "running")
        self.assertIn("NIGHTLY_RECOVERY_COHORT", self.invoke("--acknowledge", self.date)[2])
        self.status = ("active", "running", "running")
        self.assertEqual(self.invoke("--acknowledge", self.date)[0], 0)

    def test_empty_claim_directory_blocks_until_explicit_ack(self):
        self.run_path.mkdir(mode=0o700)
        self.assertIn("NIGHTLY_UNRESOLVED", self.invoke("--apply")[2])
        self.assertEqual(self.invoke("--acknowledge", self.date)[0], 0)
        self.assertIn("NIGHTLY_ALREADY_RAN", self.invoke("--apply")[2])

    def test_failed_preflight_can_be_disposed_without_child_recovery(self):
        self.fail_preflight = True
        self.assertIn("NIGHTLY_CHILD_PREFLIGHT", self.invoke("--apply")[2])
        self.assertEqual(len(self.calls), 1)
        self.fail_preflight = False
        self.now = dt.datetime(2026, 11, 22, 3, tzinfo=dt.timezone.utc)
        self.assertEqual(self.invoke("--acknowledge", self.date)[0], 0)

    def test_replaced_maintenance_config_or_current_refuse_live_child(self):
        config = Path(self.config["config_file"])
        config.write_bytes(b"different config\n")
        self.assertIn("NIGHTLY_CONFIG", self.invoke("--preflight")[2])
        self.assertEqual(self.calls, [])
        config.write_bytes(b"fixture maintenance\n")
        current = Path(self.layout.install_root) / "current"
        current.unlink()
        current.symlink_to("releases/" + self.tool)
        self.assertIn("NIGHTLY_CURRENT", self.invoke("--preflight")[2])
        self.assertEqual(self.calls, [])

    def test_absent_simulation_requires_sequencers_exact_skip_output(self):
        self.status = ("active", "absent", "running")
        self.assertEqual(self.invoke("--apply")[0], 0)

    def test_scheduler_lock_is_nonblocking(self):
        fd = os.open(self.state, nightly.install.DIR_FLAGS)
        try:
            nightly.lock(fd)
            self.assertIn("NIGHTLY_BUSY", self.invoke("--apply")[2])
            self.assertEqual(self.calls, [])
        finally:
            os.close(fd)

    def test_ack_does_not_race_an_attended_recovery(self):
        self.fail_apply = True
        self.assertNotEqual(self.invoke("--apply")[0], 0)
        self.write_progress(steps={})
        fd = os.open(self.run_path / nightly.CHILD_LOCK, nightly.install.FILE_FLAGS)
        try:
            nightly.lock(fd)
            before = len(self.calls)
            self.assertIn("NIGHTLY_BUSY", self.invoke("--acknowledge", self.date)[2])
            self.assertEqual(len(self.calls), before)
        finally:
            os.close(fd)
        self.assertEqual(self.invoke("--acknowledge", self.date)[0], 0)

    def test_missing_result_preserves_the_claim_and_blocks_tomorrow(self):
        self.assertEqual(self.invoke("--apply")[0], 0)
        (self.run_path / nightly.RESULT).unlink()
        self.now += dt.timedelta(days=1)
        self.assertIn("NIGHTLY_UNRESOLVED", self.invoke("--apply")[2])
        self.assertEqual(self.invoke("--acknowledge", self.date)[0], 0)
        self.assertTrue((self.run_path / nightly.CLAIM).is_file())
        self.assertTrue((self.run_path / nightly.PROGRESS).is_file())
        self.assertFalse((self.run_path / nightly.RESULT).exists())

    def test_ambient_confirmation_cannot_authorize_and_credential_commit_must_match(self):
        directory = self.environment.pop("CREDENTIALS_DIRECTORY")
        self.environment.update(self.provided)
        self.assertIn("NIGHTLY_CONFIRMATIONS", self.invoke("--apply")[2])
        self.assertEqual(len(self.calls), 1)
        self.assertNotIn(nightly.CONFIRMATIONS[0], self.calls[0][1])
        self.environment["CREDENTIALS_DIRECTORY"] = directory
        data = self.credential.read_bytes().replace(self.commit.encode(), self.tool.encode())
        self.credential.chmod(0o600)
        self.credential.write_bytes(data)
        self.credential.chmod(0o400)
        self.assertIn("NIGHTLY_CONFIRMATIONS", self.invoke("--acknowledge", self.date)[2])
        self.assertEqual(len(self.calls), 1)

    def test_subprocess_inherits_scheduler_lock(self):
        fd = os.open(self.state, nightly.install.DIR_FLAGS)
        try:
            nightly.lock(fd)
            script = "import os, sys; print(os.fstat(int(sys.argv[1])).st_ino)"
            argv = [sys.executable, "-I", "-S", "-B", "-c", script, str(fd)]
            code, out, err = nightly.run_child(argv, nightly.rg.child_environment(), 5, fd)
            self.assertEqual((code, out.strip(), err), (0, str(self.state.stat().st_ino).encode(), b""))
        finally:
            os.close(fd)

    def test_installed_wrapper_must_be_from_selected_release(self):
        with self.assertRaisesRegex(nightly.rg.ToolError, "NIGHTLY_CURRENT"):
            nightly.check_installed(self.layout, self.config)
        script = Path(self.layout.install_root) / "releases" / self.commit / nightly.SELF
        script.write_bytes(b"fixture scheduler\n")
        script.chmod(0o644)
        with mock.patch.object(nightly, "__file__", str(script)):
            nightly.check_installed(self.layout, self.config)

    def connect_sequencer(self):
        case = sequencer_fixture.NightlyTests()
        case.setUp()
        self.addCleanup(case.tearDown)
        self.layout = replace(self.layout, install_root=case.root)
        self.commit, self.tool = sequencer_fixture.APP, sequencer_fixture.TOOL
        self.config.update(application_commit=self.commit, tool_commit=self.tool)
        self.write_config()
        self.write_credentials()
        root = Path(case.root)
        (root / "current").symlink_to("releases/" + self.commit)
        entry = root / "releases" / self.commit / nightly.SEQUENCER
        entry.write_bytes(Path(sequencer_fixture.tool.__file__).read_bytes())
        entry.chmod(0o644)

        def sequencer(argv, env, _timeout, _lock_fd):
            self.calls.append((argv, env))
            out, err = io.StringIO(), io.StringIO()
            code = sequencer_fixture.tool.main(argv[5:], host_factory=lambda _: case.host,
                uid=os.geteuid(), runner=case.runner, environ=env, out=out, err=err,
                clock=case.clock, sleep=case.clock.sleep)
            return code, out.getvalue().encode(), err.getvalue().encode()

        self.child = sequencer
        return case

    def test_actual_sequencer_preflight_apply_skip_and_retained_progress(self):
        case = self.connect_sequencer()
        case.worker_captured = False
        case.docker.containers.pop(case.services[sequencer_fixture.tool.SIMULATION_RUNNER])
        case.stop(sequencer_fixture.tool.V2)
        self.assertEqual(self.invoke("--apply"), (0, f"NIGHTLY_BACKUP: PASS date={self.date}\n", ""))
        self.assertEqual(len(self.calls), 2)
        record = nightly.rg.strict_json((self.run_path / nightly.PROGRESS).read_bytes(), "TEST")
        self.assertTrue(record["nightly_completed"])
        self.assertFalse(record["simulation_worker_captured"])
        self.assertNotIn(("wrapper", "--simulation-start"), case.mutations())

    def test_actual_failed_verify_requires_attended_recovery_and_ack(self):
        case = self.connect_sequencer()
        case.fail_verify = True
        self.assertIn("NIGHTLY_CHILD_APPLY", self.invoke("--apply")[2])
        before = case.mutations()
        self.assertFalse(any(row[0] == "wrapper" for row in before))
        self.now += dt.timedelta(days=1)
        self.assertIn("NIGHTLY_UNRESOLVED", self.invoke("--apply")[2])
        self.assertNotEqual(self.invoke("--acknowledge", self.date)[0], 0)
        self.assertEqual(case.mutations(), before)
        args = [arg for arg in self.calls[1][0][5:] if arg != "--nightly"] + ["--step", "recover"]
        code, out, err = case.run_tool(args)
        self.assertEqual((code, out, err), (0, "PREUPGRADE_BACKUP: RECOVERED\n", ""))
        self.assertEqual(self.invoke("--acknowledge", self.date)[0], 0)
        result = nightly.rg.strict_json((self.run_path / nightly.RESULT).read_bytes(), "TEST")
        self.assertEqual(result["status"], "FAILED")
        self.assertTrue((self.run_path / nightly.ACK).is_file())

    def test_actual_apply_preflight_failure_is_acknowledgeable_without_recovery(self):
        case = self.connect_sequencer()
        child = self.child

        def fail_apply_preflight(argv, *args):
            if "--apply" in argv:
                case.children.overrides[("inactive-simulation-backup", "preflight")] = lambda: (1, b"", b"")
            return child(argv, *args)

        self.child = fail_apply_preflight
        self.assertIn("NIGHTLY_CHILD_APPLY", self.invoke("--apply")[2])
        self.assertEqual(case.mutations(), [])
        progress = (self.run_path / nightly.PROGRESS).read_bytes()
        record = nightly.rg.strict_json(progress, "TEST")
        self.assertEqual(record["steps"], {})
        self.assertEqual(record["nightly"], {})
        result_path = self.run_path / nightly.RESULT
        result = result_path.read_bytes()
        self.assertEqual(nightly.rg.strict_json(result, "TEST")["progress_sha256"], nightly.digest(progress))
        self.now += dt.timedelta(days=1)
        self.assertIn("NIGHTLY_UNRESOLVED", self.invoke("--apply")[2])
        self.assertEqual(self.invoke("--acknowledge", self.date)[0], 0)
        self.assertEqual(case.mutations(), [])
        self.assertEqual(result_path.read_bytes(), result)
        self.assertEqual((self.run_path / nightly.PROGRESS).read_bytes(), progress)

    def test_malformed_credential_preflight_is_durably_recorded_before_any_child(self):
        original = self.credential.read_bytes()
        self.credential.chmod(0o600)
        self.credential.write_bytes(b'{"bad":"shape"}\n')
        self.credential.chmod(0o400)
        self.assertIn("NIGHTLY_CONFIRMATIONS", self.invoke("--apply")[2])
        self.assertEqual(self.calls, [])
        self.assertTrue((self.run_path / nightly.RESULT).is_file())
        self.now += dt.timedelta(days=1)
        self.credential.chmod(0o600)
        self.credential.write_bytes(original)
        self.credential.chmod(0o400)
        self.assertIn("NIGHTLY_UNRESOLVED", self.invoke("--apply")[2])
        self.assertEqual(self.invoke("--acknowledge", self.date)[0], 0)

    def test_preflight_accepts_todays_settled_run_without_reopening_date(self):
        self.assertEqual(self.invoke("--apply")[0], 0)
        claim = (self.run_path / nightly.CLAIM).read_bytes()
        result = (self.run_path / nightly.RESULT).read_bytes()
        self.assertEqual(self.invoke("--preflight"), (0, "NIGHTLY_BACKUP: PREFLIGHT\n", ""))
        self.assertIn("NIGHTLY_ALREADY_RAN", self.invoke("--apply")[2])
        self.assertEqual((self.run_path / nightly.CLAIM).read_bytes(), claim)
        self.assertEqual((self.run_path / nightly.RESULT).read_bytes(), result)

    def test_preflight_accepts_todays_ack_and_refuses_unresolved_history(self):
        self.run_path.mkdir(mode=0o700)
        self.assertIn("NIGHTLY_UNRESOLVED", self.invoke("--preflight")[2])
        self.assertEqual(self.calls, [])
        self.assertEqual(self.invoke("--acknowledge", self.date)[0], 0)
        self.assertEqual(self.invoke("--preflight")[0], 0)
        self.assertIn("NIGHTLY_ALREADY_RAN", self.invoke("--apply")[2])
        (self.run_path / nightly.ACK).write_bytes(b"{}\n")
        self.assertIn("NIGHTLY_HISTORY", self.invoke("--preflight")[2])

    def test_ack_refuses_lost_or_rewritten_progress_even_if_only_hash_remains(self):
        case = self.connect_sequencer()
        case.fail_verify = True
        self.assertIn("NIGHTLY_CHILD_APPLY", self.invoke("--apply")[2])
        progress = self.run_path / nightly.PROGRESS
        record = nightly.rg.strict_json(progress.read_bytes(), "TEST")
        record["steps"] = {}
        progress.write_bytes(nightly.rg.canonical(record))
        before = len(self.calls)
        self.assertIn("NIGHTLY_RECOVERY_REQUIRED", self.invoke("--acknowledge", self.date)[2])
        progress.unlink()
        self.assertIn("NIGHTLY_RECOVERY_REQUIRED", self.invoke("--acknowledge", self.date)[2])
        (self.run_path / nightly.CHILD_LOCK).unlink()
        self.assertIn("NIGHTLY_RECOVERY_REQUIRED", self.invoke("--acknowledge", self.date)[2])
        self.assertEqual(len(self.calls), before)
        self.assertFalse((self.run_path / nightly.ACK).exists())

    def test_ack_refuses_ambiguous_child_lock_without_progress(self):
        self.fail_apply = True
        self.assertIn("NIGHTLY_CHILD_APPLY", self.invoke("--apply")[2])
        (self.run_path / nightly.CHILD_LOCK).touch(mode=0o600)
        self.assertIn("NIGHTLY_RECOVERY_REQUIRED", self.invoke("--acknowledge", self.date)[2])
        self.assertFalse((self.run_path / nightly.ACK).exists())
