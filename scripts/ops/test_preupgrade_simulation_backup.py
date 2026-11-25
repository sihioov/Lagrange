#!/usr/bin/env python3
"""Unprivileged tests for preupgrade-simulation-backup.py.

The real checked-in ``owner-beta-daily-operations.py`` / ``owner-beta-daily-runtime.py``
(ownership, global cohort, capture and drain) run against a fake Docker daemon.
systemctl, the pinned maintenance entrypoint and the installed wrapper are a fake
runner.  The state directory, progress file and pending files are real files in a
temporary tree owned by the test user.
"""
from __future__ import annotations

import copy
import contextlib
import importlib.util
import io
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
from pathlib import Path
from types import SimpleNamespace

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent


def _load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


tool = _load("preupgrade_simulation_backup_under_test", HERE / "preupgrade-simulation-backup.py")
ops = _load("preupgrade_test_daily_operations", HERE / "lib" / "owner-beta-daily-operations.py")
runtime = ops.runtime

APP = "a1" * 20
TOOL = "b2" * 20
OTHER = "c3" * 20
PROJECT = "lagrange-station"
BETA_IMAGE = "sha256:" + "c" * 64
OTHER_IMAGE = "sha256:" + "e" * 64
BETA_ID = "1" * 64
STARTED = "2026-11-16T01:02:03.123456789Z"
DB_DATE, CONFIG_DATE = "2026-10-21", "2026-10-22"
CONFIRM_VALUE = tool.EQUITY_CONFIRMATION[1]
VERIFY_OK = b"DAILY_BACKUP: INACTIVE_SIMULATION_VERIFIED_ISOLATED_DISABLED\n"
RELEASE_PASS = ("COMPOSE_RELEASE: PASS (immutable manifest image IDs bound; run post-backfill-health.sh "
                "--scope release --check to assert data readiness)")


def beta_container(identity=BETA_ID, image=BETA_IMAGE, revision=APP):
    return {
        "Id": identity, "Name": "/lagrange-station-owner-beta-runner-1",
        "Created": "2026-11-16T01:00:00.000000001Z", "RestartCount": 0, "Image": image,
        "State": {"Running": True, "Restarting": False, "Paused": False, "OOMKilled": False,
                  "Dead": False, "Error": "", "ExitCode": 0, "Status": "running",
                  "StartedAt": STARTED, "FinishedAt": "0001-01-01T00:00:00Z",
                  "Health": {"Status": "healthy"}},
        "Config": {"Labels": {"org.opencontainers.image.revision": revision,
                              "com.docker.compose.project": PROJECT,
                              "com.docker.compose.service": "owner-beta-runner",
                              "com.docker.compose.oneoff": "False"},
                   "Env": ["PRICE_BETA_DAILY_MODE=disabled"]},
        "HostConfig": {"RestartPolicy": {"Name": "unless-stopped", "MaximumRetryCount": 0}},
    }


class FakeDocker:
    def __init__(self, events):
        self.events = events
        self.containers = {BETA_ID: beta_container()}
        self.images = {BETA_IMAGE: APP, OTHER_IMAGE: OTHER}
        self.exit_code = 0
        self.wait_fails = False
        self.starts = 0

    def _fail(self):
        raise runtime.RuntimeErrorCode("DAILY_RUNTIME_DOCKER")

    def _ids(self, filters):
        result = []
        for identity, value in self.containers.items():
            labels = value["Config"]["Labels"]
            if all(labels.get(k) == v for k, v in filters):
                result.append(identity)
        return result

    def service_ids(self, project, service, timeout=15):
        return self._ids([("com.docker.compose.project", project), ("com.docker.compose.service", service)])

    def object(self, kind, identity, timeout=15):
        if kind == "image":
            if identity not in self.images:
                self._fail()
            return {"Id": identity, "Config": {"Labels": {"org.opencontainers.image.revision": self.images[identity]}}}
        if identity not in self.containers:
            self._fail()
        return copy.deepcopy(self.containers[identity])

    def call(self, args, timeout=15):
        args = list(args)
        if args[0] == "ps":
            filters = []
            for index, token in enumerate(args):
                if token == "--filter":
                    key, value = args[index + 1].removeprefix("label=").split("=", 1)
                    filters.append((key, value))
            return "".join(i + "\n" for i in self._ids(filters))
        self.events.append(("docker", args[0], *args[1:-1]))
        identity = args[-1]
        if identity not in self.containers:
            self._fail()
        value = self.containers[identity]
        if args[0] == "update":
            value["HostConfig"]["RestartPolicy"] = {"Name": args[1].split("=", 1)[1], "MaximumRetryCount": 0}
            return identity + "\n"
        if args[0] == "kill":
            return identity + "\n"
        if args[0] == "wait":
            if self.wait_fails:
                self._fail()
            for waited in args[1:]:
                state = self.containers[waited]["State"]
                state.update(Running=False, Status="exited", ExitCode=self.exit_code,
                             FinishedAt="2026-11-17T00:00:00.5Z")
                state["Health"]["Status"] = "unhealthy"
            return (str(self.exit_code) + "\n") * len(args[1:])
        if args[0] == "start":
            self.start(identity)
            return identity + "\n"
        self._fail()

    def start(self, identity):
        self.starts += 1
        state = self.containers[identity]["State"]
        state.update(Running=True, Status="running", ExitCode=0, StartedAt="2026-11-17T01:00:0%dZ" % self.starts)
        state["Health"]["Status"] = "healthy"


class FakeHost:
    def __init__(self, docker, *, images=None):
        self.docker = docker
        self.current_ok = True
        self.tool_ok = True
        self.images = images or {APP: {"owner-beta-runner": BETA_IMAGE}}

    def validate_current(self):
        if not self.current_ok:
            raise ValueError("POLICY_MISMATCH")

    def manifest_images(self, commit):
        if commit not in self.images:
            raise runtime.RuntimeErrorCode("DAILY_OPERATION_MANIFEST")
        return dict(self.images[commit])

    def project(self):
        return PROJECT

    def ownership(self, project, commit, image):
        return ops.Ownership(project, commit, image)

    def owned(self, value, own, service, identity):
        return runtime.owned(value, own, service, identity)

    def global_cohort(self, own):
        ops.global_cohort(self.docker, own)

    def capture(self, own):
        return runtime.capture(self.docker, own)

    def drain(self, own, snapshot):
        return runtime.drain(self.docker, own, snapshot, timeout=60)

    def verify_tool(self, commit):
        if not self.tool_ok:
            raise ValueError("BACKUP_TOOL_INVALID")
        return "sha256:" + "f" * 64


class FakeRunner:
    def __init__(self, case):
        self.case = case
        self.units = {tool.TIMER: ["loaded", "active"], tool.OBSERVER_SERVICE: ["loaded", "inactive"]}
        self.overrides = {}
        self.probe = ("owner_only", "off", "rest")  # 312 production: Owner V2 enabled
        self.pinned_env = []
        self.wrapper_env = []
        self.observe = None  # called with the wrapper flag before the fake acts

    def __call__(self, argv, env, timeout):
        argv = list(argv)
        events = self.case.events
        if argv[0] == tool.SYSTEMCTL:
            verb, unit = argv[1], argv[-1]
            if verb == "show":
                load, active = self.units[unit]
                return 0, ("LoadState=%s\nActiveState=%s\n" % (load, active)).encode(), b""
            events.append(("systemctl", verb, unit))
            self.units[unit][1] = "inactive" if verb == "stop" else "active"
            return 0, b"", b""
        if argv[0] == "/usr/bin/python3":
            key = (argv[argv.index("--operation") + 1], argv[argv.index("--mode") + 1])
            events.append(("pinned",) + key)
            self.pinned_env.append(dict(env))
            if key in self.overrides:
                return self.overrides.pop(key)()
            if key[1] == "plan":
                return 0, b"INACTIVE_SIMULATION_PLAN: preserve pinned inactive Daily dates\n", b""
            if key[1] == "preflight":
                return 0, b"DAILY_BACKUP: PREFLIGHT\n", b""
            if key[0] == "inactive-simulation-backup":
                self.case.write_pending(tool.HANDOFF_PENDING, "switched")
                return 0, b"DAILY_BACKUP: INACTIVE_SIMULATION_VERIFIED_DISABLED\n", b""
            return 0, b"DAILY_BACKUP: INACTIVE_SIMULATION_VERIFIED_ISOLATED_DISABLED\n", b""
        if argv[0] == "/bin/bash" and "-c" in argv:
            events.append(("probe",))
            if self.probe is None:
                return 3, b"", b""
            return 0, ("equity=%s\nquotes=%s\ntransport=%s\n" % self.probe).encode(), b""
        if argv[0] == "/bin/bash":
            flag = argv[-1]
            events.append(("wrapper", flag))
            self.wrapper_env.append(dict(env))
            if self.observe is not None:
                self.observe(flag)
            if flag in self.overrides:
                return self.overrides.pop(flag)()
            if flag == "--simulation-release-start":
                if self.case.worker_captured:
                    self.case.write_pending(tool.HANDOFF_PENDING, "api-started")
                else:
                    # finish-release removes a handoff that recorded no worker (SO:1538-1539).
                    os.unlink(os.path.join(self.case.root, tool.HANDOFF_PENDING))
                self.case.docker.start(BETA_ID)
                junk = ("NAME IMAGE " + BETA_ID + " " + BETA_IMAGE + "\n").encode()
                return 0, junk + (RELEASE_PASS + "\n").encode(), b"compose noise " + APP.encode() + b"\n"
            os.unlink(os.path.join(self.case.root, tool.HANDOFF_PENDING))
            return 0, b"SIMULATION_OPERATION: STARTED\n", b""
        raise AssertionError("unexpected child")


class Clock:
    def __init__(self):
        self.now = 1000.0

    def __call__(self):
        self.now += 1.0
        return self.now

    def sleep(self, seconds):
        self.now += seconds


class Base(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        base = Path(self.temp.name)
        os.chmod(base, 0o700)
        self.root = str(base / "install")
        self.state = str(base / "state")
        for directory in (self.root + "/maintenance/" + TOOL + "/scripts/ops/lib",
                          self.root + "/releases/" + APP + "/scripts/ops/lib"):
            os.makedirs(directory)
        for path, mode in ((self.root + "/maintenance/" + TOOL + "/" + tool.ENTRYPOINT, 0o644),
                           (self.root + "/releases/" + APP + "/" + tool.WRAPPER, 0o755),
                           (self.root + "/releases/" + APP + "/scripts/ops/lib/dotenv.sh", 0o644),
                           (self.root + "/releases/" + APP + "/scripts/ops/lib/kis-read-compose.sh", 0o644)):
            Path(path).write_text("stub\n")
            os.chmod(path, mode)
        for current, dirs, _ in os.walk(base):
            for name in dirs:
                os.chmod(os.path.join(current, name), 0o755)
        os.mkdir(self.state, 0o700)
        os.chmod(self.state, 0o700)
        self.events = []
        self.docker = FakeDocker(self.events)
        self.host = FakeHost(self.docker)
        self.runner = FakeRunner(self)
        self.clock = Clock()
        self.environ = {"OWNER_EQUITY_V2_ROLLOUT_CONFIRM": CONFIRM_VALUE, "UNRELATED": "x"}
        self.outputs = []
        self.worker_captured = True  # what the fake backup capture records in the handoff

    def tearDown(self):
        self.temp.cleanup()

    def worker_fields(self, captured):
        if captured:
            return {"worker_generation": ["9" * 64, "2026-11-16T00:00:00.000000001Z", STARTED, 0],
                    "worker_policy": "no", "worker_running": False}
        return {"worker_generation": None, "worker_policy": None, "worker_running": False}

    def write_pending(self, name, phase, **record):
        path = os.path.join(self.root, name)
        if os.path.exists(path):
            os.unlink(path)
        fields = {"phase": phase, "source_commit": APP, "target_commit": APP,
                  **self.worker_fields(self.worker_captured), **record}
        data = json.dumps(fields, sort_keys=True).encode()
        tmp = path + ".tmp"
        fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        os.write(fd, data)
        os.close(fd)
        os.replace(tmp, path)

    def argv(self, mode="--apply", *extra):
        return [mode, "--install-root", self.root, "--application-commit", APP, "--tool-commit", TOOL,
                "--config-file", "/protected/backup.conf", "--expected-db-first-session", DB_DATE,
                "--expected-config-first-session", CONFIG_DATE, "--stopped-worker-from-commit", APP,
                "--state-dir", self.state, *extra]

    def run_tool(self, argv):
        out, err = io.StringIO(), io.StringIO()
        code = tool.main(argv, host_factory=lambda args: self.host, uid=os.geteuid(), runner=self.runner,
                         environ=self.environ, out=out, err=err, clock=self.clock, sleep=self.clock.sleep)
        self.outputs.append(out.getvalue() + err.getvalue())
        return code, out.getvalue(), err.getvalue()

    def progress(self):
        path = os.path.join(self.state, tool.PROGRESS_NAME + ".json")
        if not os.path.exists(path):
            return None
        return json.loads(Path(path).read_text())

    def mutations(self):
        return [e for e in self.events if e[0] in ("docker", "systemctl", "wrapper")
                or (e[0] == "pinned" and e[2] == "apply")]

    def steps(self):
        return {name: value["state"] for name, value in self.progress()["steps"].items()}

    def count(self, event):
        return sum(1 for e in self.events if e == event)

    def assertFailed(self, result, code):
        self.assertEqual(result[0], 1, result)
        self.assertIn("PREUPGRADE_BACKUP: FAILED " + code + "\n", result[2])

    def assertNoValues(self):
        text = "".join(self.outputs)
        for value in (APP, TOOL, BETA_ID, BETA_IMAGE, STARTED, DB_DATE, CONFIG_DATE, CONFIRM_VALUE,
                      self.root, self.state, "/protected/backup.conf", "compose noise"):
            self.assertNotIn(value, text)


class ArgumentTests(Base):
    def test_grammar_rejects_without_echoing_values(self):
        good = self.argv()
        cases = [
            [],
            good[1:],                                             # no mode
            ["--plan", "--apply"] + good[1:],                      # two modes
            good + ["--install-root", self.root],                  # repeated option
            good + ["--state-dir=" + self.state],                  # repeated, = form
            [a if a != "--state-dir" else "--state-di" for a in good],  # abbreviation
            good + ["--state-di", self.state],                     # abbreviation as repeat
            [a if a != APP else "A1" * 20 for a in good],         # upper-case commit
            [a if a != TOOL else "0" * 40 for a in good],         # all-zero commit
            [a if a != DB_DATE else "2026-02-30" for a in good],  # impossible date
            [a if a != CONFIG_DATE else DB_DATE for a in good],   # db date not before config
            [a if a != "/protected/backup.conf" else "relative.conf" for a in good],
            [a if a != self.state else self.state + "/../x" for a in good],
            self.argv("--preflight", "--step", "backup"),         # --step outside apply
            good + ["--step", "everything"],
            good + ["--unknown"],
        ]
        for argv in cases:
            with self.subTest(argv=len(argv)):
                result = self.run_tool(argv)
                self.assertFailed(result, "PREUPGRADE_ARGUMENTS")
        self.assertEqual(self.events, [])
        self.assertIsNone(self.progress())
        self.assertNotIn("everything", "".join(self.outputs))
        self.assertNoValues()

    def test_step_names_and_default(self):
        args = tool.parse(self.argv())
        self.assertEqual(args.step, "all")
        self.assertEqual(set(tool.GROUPS), {"pause-observer", "stop-beta", "backup", "verify",
                                            "restore-beta-policy", "resume", "recover-beta",
                                            "restore-observer", "recover", "all"})


class PlanPreflightTests(Base):
    def test_plan_runs_only_pinned_plan(self):
        code, out, _ = self.run_tool(self.argv("--plan"))
        self.assertEqual(code, 0)
        self.assertTrue(out.startswith("PREUPGRADE_BACKUP: PLAN\n"))
        self.assertEqual(self.events, [("pinned", "inactive-simulation-backup", "plan")])
        self.assertIsNone(self.progress())

    def test_plan_rejects_wrong_pinned_line(self):
        self.runner.overrides[("inactive-simulation-backup", "plan")] = lambda: (0, b"DAILY_BACKUP_PLAN: x\n", b"")
        self.assertFailed(self.run_tool(self.argv("--plan")), "PREUPGRADE_TOOL")

    def test_preflight_is_read_only(self):
        code, out, _ = self.run_tool(self.argv("--preflight"))
        self.assertEqual(code, 0, self.outputs)
        self.assertEqual(out, "PREUPGRADE_BACKUP: OBSERVER timer=active\n"
                              "PREUPGRADE_BACKUP: CONFIRMATIONS owner_equity_v2=required owner_market_stream=not_required\n"
                              "PREUPGRADE_BACKUP: PREFLIGHT\n")
        self.assertEqual(self.mutations(), [])
        self.assertEqual([e for e in self.events if e[0] == "pinned"],
                         [("pinned", "inactive-simulation-backup", "plan"),
                          ("pinned", "inactive-simulation-backup", "preflight")])
        self.assertIsNone(self.progress())
        self.assertNoValues()

    def test_preflight_rejects_wrong_lines_and_codes(self):
        for output in ((0, b"DAILY_BACKUP: PREFLIGHT\nextra\n", b""), (1, b"DAILY_BACKUP: PREFLIGHT\n", b""),
                       (0, b"", b""), (None, b"DAILY_BACKUP: PREFLIGHT\n", b"")):
            with self.subTest(output=output):
                self.runner.overrides[("inactive-simulation-backup", "preflight")] = lambda o=output: o
                self.assertFailed(self.run_tool(self.argv("--preflight")), "PREUPGRADE_TOOL")

    def test_pending_before_start_is_refused(self):
        for name in (tool.HANDOFF_PENDING, tool.START_PENDING):
            with self.subTest(name=name):
                self.write_pending(name, "switched")
                self.assertFailed(self.run_tool(self.argv("--preflight")), "PREUPGRADE_PENDING")
                self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_PENDING")
                os.unlink(os.path.join(self.root, name))
        self.assertEqual(self.mutations(), [])
        self.assertIsNone(self.progress())

    def test_worker_source_must_match_manifest(self):
        self.docker.containers[BETA_ID]["Config"]["Labels"]["org.opencontainers.image.revision"] = OTHER
        self.assertFailed(self.run_tool(self.argv("--preflight")), "PREUPGRADE_WORKER_SOURCE")
        self.docker.containers[BETA_ID] = beta_container(image=OTHER_IMAGE, revision=APP)
        self.assertFailed(self.run_tool(self.argv("--preflight")), "PREUPGRADE_WORKER_SOURCE")
        self.docker.containers[BETA_ID] = beta_container()
        self.host.images = {}
        self.assertFailed(self.run_tool(self.argv("--preflight")), "PREUPGRADE_WORKER_SOURCE")

    def test_preflight_requires_running_healthy_beta_and_current(self):
        self.docker.containers[BETA_ID]["State"]["Health"]["Status"] = "starting"
        self.assertFailed(self.run_tool(self.argv("--preflight")), "PREUPGRADE_BETA_STOP")
        self.docker.containers[BETA_ID] = beta_container()
        self.docker.containers[BETA_ID]["HostConfig"]["RestartPolicy"] = {"Name": "on-failure", "MaximumRetryCount": 3}
        self.assertFailed(self.run_tool(self.argv("--preflight")), "PREUPGRADE_BETA_STOP")
        self.docker.containers[BETA_ID] = beta_container()
        self.host.current_ok = False
        self.assertFailed(self.run_tool(self.argv("--preflight")), "PREUPGRADE_CURRENT")
        self.host.current_ok = True
        self.host.tool_ok = False
        self.assertFailed(self.run_tool(self.argv("--preflight")), "PREUPGRADE_TOOL")

    def test_preflight_rejects_failed_collector(self):
        self.runner.units[tool.OBSERVER_SERVICE] = ["loaded", "failed"]
        self.assertFailed(self.run_tool(self.argv("--preflight")), "PREUPGRADE_OBSERVER")
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_OBSERVER")
        self.assertEqual(self.mutations(), [])
        self.assertIsNone(self.progress())

    def test_state_directory_must_be_private(self):
        os.chmod(self.state, 0o750)
        self.assertFailed(self.run_tool(self.argv("--preflight")), "PREUPGRADE_PROGRESS")
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_PROGRESS")
        self.assertEqual(self.mutations(), [])


class ApplyTests(Base):
    EXPECTED = [
        ("systemctl", "stop", tool.TIMER),
        ("docker", "update", "--restart=no"),
        ("docker", "kill", "--signal=TERM"),
        ("docker", "wait"),
        ("pinned", "inactive-simulation-backup", "apply"),
        ("pinned", "inactive-simulation-verify", "apply"),
        ("docker", "update", "--restart=unless-stopped"),
        ("wrapper", "--simulation-release-start"),
        ("wrapper", "--simulation-start"),
        ("systemctl", "start", tool.TIMER),
    ]

    def test_full_sequence_order_and_restore_before_resume(self):
        code, out, err = self.run_tool(self.argv())
        self.assertEqual((code, out, err), (0, "PREUPGRADE_BACKUP: PASS\n", ""), self.outputs)
        self.assertEqual(self.mutations(), self.EXPECTED)
        events = self.mutations()
        self.assertLess(events.index(("docker", "update", "--restart=unless-stopped")),
                        events.index(("wrapper", "--simulation-release-start")))
        record = self.progress()
        self.assertTrue(all(record["steps"][n] == {"state": "done", "code": None} for n in tool.STEPS))
        self.assertEqual(record["observer"], {"timer_was_active": True})
        self.assertEqual(record["beta"]["restart_policy"], {"Name": "unless-stopped", "MaximumRetryCount": 0})
        self.assertTrue(record["beta_generation_reused"])
        info = os.stat(os.path.join(self.state, tool.PROGRESS_NAME + ".json"))
        self.assertEqual(stat.S_IMODE(info.st_mode), 0o600)
        self.assertFalse(os.path.exists(os.path.join(self.root, tool.HANDOFF_PENDING)))
        self.assertEqual(self.docker.containers[BETA_ID]["HostConfig"]["RestartPolicy"]["Name"], "unless-stopped")
        # Confirmations are passed through unchanged to the wrapper only, never synthesized.
        for env in self.runner.wrapper_env:
            self.assertEqual(env.get("OWNER_EQUITY_V2_ROLLOUT_CONFIRM"), CONFIRM_VALUE)
            self.assertNotIn("OWNER_MARKET_STREAM_ROLLOUT_CONFIRM", env)
            self.assertNotIn("UNRELATED", env)
            self.assertEqual(env["LAGRANGE_RELEASE_ROOT"], self.root)
        for env in self.runner.pinned_env:
            self.assertFalse(set(tool.CONFIRMATIONS) & set(env))
        # A completed run is idempotent and mutates nothing further.
        self.assertEqual(self.run_tool(self.argv())[0:2], (0, "PREUPGRADE_BACKUP: PASS\n"))
        self.assertEqual(self.mutations(), self.EXPECTED)
        self.assertNoValues()

    def test_observer_restored_only_if_it_was_active(self):
        self.runner.units[tool.TIMER] = ["loaded", "inactive"]
        self.assertEqual(self.run_tool(self.argv())[0], 0, self.outputs)
        self.assertFalse([e for e in self.events if e[0] == "systemctl"])
        self.assertEqual(self.progress()["observer"], {"timer_was_active": False})
        self.assertEqual(self.runner.units[tool.TIMER][1], "inactive")

    def test_observer_collector_must_finish(self):
        self.runner.units[tool.OBSERVER_SERVICE] = ["loaded", "activating"]  # never finishes
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_OBSERVER")
        self.assertEqual(self.mutations(), [("systemctl", "stop", tool.TIMER)])
        # Retryable: the originally recorded "active" survives the retry.
        self.runner.units[tool.OBSERVER_SERVICE] = ["loaded", "inactive"]
        self.assertEqual(self.run_tool(self.argv())[0], 0, self.outputs)
        self.assertEqual(self.mutations()[-1], ("systemctl", "start", tool.TIMER))
        self.assertEqual(self.runner.units[tool.TIMER][1], "active")

    def test_resume_by_explicit_steps(self):
        sequence = ["pause-observer", "stop-beta", "backup", "verify", "restore-beta-policy", "resume",
                    "restore-observer"]
        for index, step in enumerate(sequence):
            code, out, _ = self.run_tool(self.argv("--apply", "--step", step))
            self.assertEqual(code, 0, self.outputs)
            last = index == len(sequence) - 1
            self.assertEqual(out, "PREUPGRADE_BACKUP: " + ("PASS" if last else "STEP " + step) + "\n")
        self.assertEqual(self.mutations(), self.EXPECTED)

    def test_steps_out_of_order_or_repeated_are_refused(self):
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "stop-beta")), "PREUPGRADE_PROGRESS")
        self.assertIsNone(self.progress())
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "pause-observer"))[0], 0)
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "pause-observer")), "PREUPGRADE_PROGRESS")
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "backup")), "PREUPGRADE_PROGRESS")
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "resume")), "PREUPGRADE_PROGRESS")
        self.assertEqual(self.mutations(), [("systemctl", "stop", tool.TIMER)])

    def test_progress_bound_to_arguments(self):
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "pause-observer"))[0], 0)
        changed = [a if a != CONFIG_DATE else "2026-10-23" for a in self.argv()]
        self.assertFailed(self.run_tool(changed), "PREUPGRADE_PROGRESS")
        self.assertFailed(self.run_tool(self.argv("--preflight")), "PREUPGRADE_PROGRESS")

    def test_retryable_verify_resumes(self):
        self.runner.overrides[("inactive-simulation-verify", "apply")] = lambda: (
            1, b"", b"PREACTIVATION_BACKUP: FAILED PREACTIVATION_HANDOFF\nTraceback /secret/path\nDAILY_BACKUP: FAILED\n")
        code, _, err = self.run_tool(self.argv())
        self.assertEqual(code, 1)
        self.assertEqual(err, "PREACTIVATION_BACKUP: FAILED PREACTIVATION_HANDOFF\nDAILY_BACKUP: FAILED\n"
                              "PREUPGRADE_BACKUP: FAILED PREUPGRADE_VERIFY_STEP\n")
        self.assertEqual(self.progress()["steps"]["verify"]["state"], "failed")
        self.assertNotIn(("docker", "update", "--restart=unless-stopped"), self.mutations())
        self.assertEqual(self.run_tool(self.argv())[0:2], (0, "PREUPGRADE_BACKUP: PASS\n"))
        self.assertEqual([e for e in self.mutations() if e[0] == "pinned"],
                         [("pinned", "inactive-simulation-backup", "apply"),
                          ("pinned", "inactive-simulation-verify", "apply"),
                          ("pinned", "inactive-simulation-verify", "apply")])
        self.assertEqual(sum(1 for e in self.events if e[:2] == ("docker", "kill")), 1)

    def test_wrong_backup_line_is_final(self):
        for output in ((0, b"DAILY_BACKUP: INACTIVE_SIMULATION_VERIFIED_ISOLATED_DISABLED\n", b""),
                       (1, b"DAILY_BACKUP: INACTIVE_SIMULATION_VERIFIED_DISABLED\n", b"")):
            with self.subTest(output=output):
                self.tearDown()
                self.setUp()
                self.runner.overrides[("inactive-simulation-backup", "apply")] = lambda o=output: o
                self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_BACKUP_STEP")
                self.assertEqual(self.progress()["steps"]["backup"]["state"], "failed")
                # Never re-run, even on an explicit step request.
                self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_PROGRESS")
                self.assertFailed(self.run_tool(self.argv("--apply", "--step", "backup")), "PREUPGRADE_PROGRESS")
                self.assertEqual(sum(1 for e in self.events if e == ("pinned", "inactive-simulation-backup", "apply")), 1)
                # The observer can still be restored explicitly.
                self.assertEqual(self.run_tool(self.argv("--apply", "--step", "restore-observer"))[0:2],
                                 (0, "PREUPGRADE_BACKUP: STEP restore-observer\n"))
                self.assertEqual(self.runner.units[tool.TIMER][1], "active")

    def test_backup_without_handoff_is_rejected(self):
        self.runner.overrides[("inactive-simulation-backup", "apply")] = lambda: (
            0, b"DAILY_BACKUP: INACTIVE_SIMULATION_VERIFIED_DISABLED\n", b"")
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_BACKUP_STEP")

    def test_simulation_start_wrong_line_is_final(self):
        output = (0, b"SIMULATION_OPERATION: RECONCILED_OFF\n", b"SIMULATION_OPERATION_REJECTED\n")
        self.runner.overrides["--simulation-start"] = lambda: output
        code, _, err = self.run_tool(self.argv())
        self.assertEqual(code, 1)
        self.assertEqual(err, "SIMULATION_OPERATION_REJECTED\nPREUPGRADE_BACKUP: FAILED PREUPGRADE_RESUME_STEP\n")
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_PROGRESS")
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "resume")), "PREUPGRADE_PROGRESS")
        self.assertEqual(sum(1 for e in self.events if e == ("wrapper", "--simulation-start")), 1)
        self.assertNotIn(("systemctl", "start", tool.TIMER), self.mutations())

    def test_release_start_refused_before_handoff_moves_may_be_retried(self):
        # e.g. sudo dropped a rollout confirmation: the wrapper refuses at its gate,
        # the handoff stays "switched" and the Beta stays stopped.
        for output in ((2, b"", b"BLOCKED_EXTERNAL: owner_equity_v2_rollout_confirmation_required\n"),
                       (0, b"COMPOSE_RELEASE: FAILED\n", b"compose-release: simulation release start is not ready\n")):
            with self.subTest(output=output):
                self.tearDown()
                self.setUp()
                self.runner.overrides["--simulation-release-start"] = lambda o=output: o
                code, _, err = self.run_tool(self.argv())
                self.assertEqual(code, 1)
                self.assertEqual(err, output[2].decode() + "PREUPGRADE_BACKUP: FAILED PREUPGRADE_RESUME_STEP\n")
                self.assertEqual(self.progress()["steps"]["resume-release"]["state"], "failed")
                self.assertNotIn(("wrapper", "--simulation-start"), self.mutations())
                self.assertEqual(self.run_tool(self.argv())[0:2], (0, "PREUPGRADE_BACKUP: PASS\n"))
                self.assertEqual(sum(1 for e in self.events if e == ("wrapper", "--simulation-release-start")), 2)

    def test_release_start_not_retried_after_handoff_moved(self):
        def moved():
            self.write_pending(tool.HANDOFF_PENDING, "api-starting")
            return 1, b"", b"compose-release: simulation release handoff remains pending\n"
        self.runner.overrides["--simulation-release-start"] = moved
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_RESUME_STEP")
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_PENDING")
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "resume")), "PREUPGRADE_PENDING")
        self.assertEqual(sum(1 for e in self.events if e == ("wrapper", "--simulation-release-start")), 1)

    def test_wrong_verify_line_fails(self):
        for output in ((0, b"DAILY_BACKUP: INACTIVE_SIMULATION_VERIFIED_DISABLED\n", b""),
                       (0, VERIFY_OK + b"extra\n", b""), (1, VERIFY_OK, b""), (None, VERIFY_OK, b"")):
            with self.subTest(output=output):
                self.tearDown()
                self.setUp()
                self.runner.overrides[("inactive-simulation-verify", "apply")] = lambda o=output: o
                self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_VERIFY_STEP")
                self.assertNotIn(("docker", "update", "--restart=unless-stopped"), self.mutations())

    def test_out_of_order_observer_restore_does_not_consume_final_step(self):
        self.runner.units[tool.OBSERVER_SERVICE] = ["loaded", "activating"]
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_OBSERVER")  # collector never finished
        self.assertEqual(self.runner.units[tool.TIMER][1], "inactive")
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "restore-observer"))[0:2],
                         (0, "PREUPGRADE_BACKUP: STEP restore-observer\n"))
        self.assertEqual(self.runner.units[tool.TIMER][1], "active")
        self.assertEqual(self.progress()["steps"]["restore-observer"]["state"], "recovered")
        self.runner.units[tool.OBSERVER_SERVICE] = ["loaded", "inactive"]
        self.assertEqual(self.run_tool(self.argv())[0:2], (0, "PREUPGRADE_BACKUP: PASS\n"))
        self.assertEqual(self.runner.units[tool.TIMER][1], "active")
        self.assertEqual([e for e in self.mutations() if e[0] == "systemctl"],
                         [("systemctl", "stop", tool.TIMER), ("systemctl", "start", tool.TIMER),
                          ("systemctl", "stop", tool.TIMER), ("systemctl", "start", tool.TIMER)])

    def test_drain_exit_nonzero_is_never_retried(self):
        self.docker.exit_code = 1
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_BETA_STOP")
        record = self.progress()
        self.assertEqual(record["steps"]["stop-beta"], {"state": "failed", "code": "PREUPGRADE_BETA_STOP"})
        self.assertEqual(record["beta"]["restart_policy"], {"Name": "unless-stopped", "MaximumRetryCount": 0})
        self.docker.exit_code = 0
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_PROGRESS")
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "stop-beta")), "PREUPGRADE_PROGRESS")
        self.assertEqual(sum(1 for e in self.events if e[:2] == ("docker", "kill")), 1)
        self.assertFalse([e for e in self.events if e[0] == "pinned" and e[2] == "apply"])

    def test_daily_cohort_present_is_refused(self):
        other = beta_container(identity="2" * 64)
        other["Config"]["Labels"]["com.docker.compose.service"] = "owner-beta-daily-producer"
        self.docker.containers["2" * 64] = other
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_WORKER_SOURCE")
        self.assertEqual(self.mutations(), [])

    def test_changed_generation_between_steps(self):
        for step in ("pause-observer", "stop-beta"):
            self.assertEqual(self.run_tool(self.argv("--apply", "--step", step))[0], 0, self.outputs)
        changes = {
            "started": lambda v: v["State"].update(StartedAt="2026-11-17T09:09:09Z"),
            "restarts": lambda v: v.update(RestartCount=1),
            "running": lambda v: v["State"].update(Running=True, Status="running"),
            "policy": lambda v: v["HostConfig"].update(RestartPolicy={"Name": "always", "MaximumRetryCount": 0}),
        }
        original = copy.deepcopy(self.docker.containers[BETA_ID])
        for name, change in changes.items():
            with self.subTest(change=name):
                change(self.docker.containers[BETA_ID])
                self.assertFailed(self.run_tool(self.argv("--apply", "--step", "backup")), "PREUPGRADE_BETA_CHANGED")
                self.docker.containers[BETA_ID] = copy.deepcopy(original)
        replaced = copy.deepcopy(original)
        replaced["Id"] = "3" * 64
        self.docker.containers = {"3" * 64: replaced}
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "backup")), "PREUPGRADE_BETA_CHANGED")
        self.assertFalse([e for e in self.events if e[0] == "pinned" and e[2] == "apply"])
        self.assertIsNone(self.progress()["steps"].get("backup"))
        # After the original generation is back, the step proceeds.
        self.docker.containers = {BETA_ID: copy.deepcopy(original)}
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "backup"))[0], 0, self.outputs)

    def test_changed_generation_before_resume(self):
        for step in ("pause-observer", "stop-beta", "backup", "verify", "restore-beta-policy"):
            self.assertEqual(self.run_tool(self.argv("--apply", "--step", step))[0], 0, self.outputs)
        self.docker.containers[BETA_ID]["HostConfig"]["RestartPolicy"] = {"Name": "no", "MaximumRetryCount": 0}
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "resume")), "PREUPGRADE_BETA_CHANGED")
        self.assertFalse([e for e in self.events if e[0] == "wrapper"])

    def test_resume_requires_switched_handoff(self):
        for step in ("pause-observer", "stop-beta", "backup", "verify", "restore-beta-policy"):
            self.assertEqual(self.run_tool(self.argv("--apply", "--step", step))[0], 0, self.outputs)
        self.write_pending(tool.HANDOFF_PENDING, "fencing")
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "resume")), "PREUPGRADE_PENDING")
        os.unlink(os.path.join(self.root, tool.HANDOFF_PENDING))
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "resume")), "PREUPGRADE_PENDING")
        self.assertFalse([e for e in self.events if e[0] == "wrapper"])

    def test_resumed_beta_must_become_healthy_with_recorded_policy(self):
        def release():
            self.write_pending(tool.HANDOFF_PENDING, "api-started")
            self.docker.start(BETA_ID)
            self.docker.containers[BETA_ID]["State"]["Health"]["Status"] = "starting"
            return 0, (RELEASE_PASS + "\n").encode(), b""
        self.runner.overrides["--simulation-release-start"] = release
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_RESUME_STEP")
        # The health check is its own read-only step after the simulation start.
        self.assertEqual(self.steps()["resume-release"], "done")
        self.assertEqual(self.steps()["resume-simulation"], "done")
        self.assertEqual(self.steps()["verify-beta"], "failed")
        self.assertNotIn(("systemctl", "start", tool.TIMER), self.mutations())
        self.docker.containers[BETA_ID]["State"]["Health"]["Status"] = "healthy"
        self.assertEqual(self.run_tool(self.argv())[0:2], (0, "PREUPGRADE_BACKUP: PASS\n"))
        self.assertEqual(self.count(("wrapper", "--simulation-start")), 1)

    def test_progress_lock_rejects_concurrent_run(self):
        progress = tool.rg.Progress(self.state, tool.PROGRESS_NAME, os.geteuid(), "PREUPGRADE_PROGRESS")
        progress.lock()
        try:
            self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_PROGRESS")
        finally:
            progress.unlock()
        self.assertEqual(self.mutations(), [])


class ReviewFindingTests(Base):
    RESUME_TAIL = [("docker", "update", "--restart=unless-stopped"), ("wrapper", "--simulation-release-start"),
                   ("wrapper", "--simulation-start"), ("systemctl", "start", tool.TIMER)]

    def fenced_backup_failure(self):
        def backup():
            self.write_pending(tool.HANDOFF_PENDING, "switched")  # API fenced, then a late failure
            return 1, b"", b"PREACTIVATION_BACKUP: FAILED PREACTIVATION_RETENTION_LIMIT\nDAILY_BACKUP: FAILED\n"
        self.runner.overrides[("inactive-simulation-backup", "apply")] = backup

    # Blocker 1
    def test_recover_after_backup_failed_with_switched_handoff(self):
        self.fenced_backup_failure()
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_BACKUP_STEP")
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_PROGRESS")
        before = len(self.mutations())
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "recover"))[0:3],
                         (0, "PREUPGRADE_BACKUP: RECOVERED\n", ""))
        self.assertEqual(self.mutations()[before:], self.RESUME_TAIL)
        steps = self.steps()
        self.assertEqual(steps["backup"], "failed")
        self.assertNotIn("verify", steps)
        self.assertEqual(steps["restore-observer"], "recovered")
        self.assertFalse(os.path.exists(os.path.join(self.root, tool.HANDOFF_PENDING)))
        self.assertEqual(self.runner.units[tool.TIMER][1], "active")
        # Idempotent, and a failed backup is never reported as PASS.
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "recover"))[0:2],
                         (0, "PREUPGRADE_BACKUP: RECOVERED\n"))
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_PROGRESS")
        self.assertEqual(len(self.mutations()), before + len(self.RESUME_TAIL))
        self.assertNotIn("PREUPGRADE_BACKUP: PASS", "".join(self.outputs))

    def test_explicit_resume_steps_after_repeated_verify_failure(self):
        self.runner.overrides[("inactive-simulation-verify", "apply")] = lambda: (1, b"", b"")
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_VERIFY_STEP")
        for step in ("restore-beta-policy", "resume"):
            code, out, _ = self.run_tool(self.argv("--apply", "--step", step))
            self.assertEqual((code, out), (0, "PREUPGRADE_BACKUP: STEP " + step + "\n"), self.outputs)
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "restore-observer"))[0:2],
                         (0, "PREUPGRADE_BACKUP: STEP restore-observer\n"))
        self.assertEqual(self.steps()["verify"], "failed")
        self.assertEqual(self.runner.units[tool.TIMER][1], "active")

    def test_recover_refuses_foreign_handoff_phase(self):
        self.runner.overrides[("inactive-simulation-backup", "apply")] = lambda: (
            self.write_pending(tool.HANDOFF_PENDING, "fencing"), (1, b"", b""))[1]
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_BACKUP_STEP")
        before = len(self.mutations())
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "recover")), "PREUPGRADE_PENDING")
        self.assertEqual(len(self.mutations()), before)

    # Major 2
    def test_wrapper_failure_after_api_start_continues_to_simulation(self):
        def release():
            self.write_pending(tool.HANDOFF_PENDING, "api-started")
            self.docker.start(BETA_ID)
            return 1, b"", b"compose-release: persistent service did not resolve to exactly one container\n"
        self.runner.overrides["--simulation-release-start"] = release
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_RESUME_STEP")
        self.assertEqual(self.steps()["resume-release"], "failed")
        self.assertEqual(self.run_tool(self.argv())[0:2], (0, "PREUPGRADE_BACKUP: RECOVERED\n"))
        self.assertEqual(self.count(("wrapper", "--simulation-release-start")), 1)
        self.assertEqual(self.count(("wrapper", "--simulation-start")), 1)
        self.assertEqual(self.steps()["resume-release"], "failed")
        self.assertEqual(self.steps()["verify-beta"], "done")
        self.assertEqual(self.runner.units[tool.TIMER][1], "active")

    def test_release_start_done_on_exit_zero_with_api_started(self):
        def release():
            self.write_pending(tool.HANDOFF_PENDING, "api-started")
            return 0, (RELEASE_PASS + "\n").encode(), b""  # Beta not started yet
        self.runner.overrides["--simulation-release-start"] = release
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_RESUME_STEP")
        self.assertEqual(self.steps()["resume-release"], "done")
        self.assertEqual(self.steps()["verify-beta"], "failed")

    # Major 3
    def test_recover_beta_after_failed_drain(self):
        self.docker.exit_code = 1
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_BETA_STOP")
        value = self.docker.containers[BETA_ID]
        self.assertEqual((value["State"]["Running"], value["HostConfig"]["RestartPolicy"]["Name"]), (False, "no"))
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "recover"))[0:3],
                         (0, "PREUPGRADE_BACKUP: RECOVERED\n", ""))
        self.assertEqual(self.mutations()[-3:], [("docker", "update", "--restart=unless-stopped"),
                                                 ("docker", "start"), ("systemctl", "start", tool.TIMER)])
        value = self.docker.containers[BETA_ID]
        self.assertTrue(value["State"]["Running"])
        self.assertEqual(value["HostConfig"]["RestartPolicy"], {"Name": "unless-stopped", "MaximumRetryCount": 0})
        self.assertEqual(self.steps()["recover-beta"], "done")
        self.assertEqual(self.steps()["stop-beta"], "failed")
        self.assertFalse([e for e in self.events if e[0] in ("wrapper",) or (e[0] == "pinned" and e[2] == "apply")])
        self.assertNoValues()

    def test_recover_beta_after_backup_failed_before_handoff(self):
        self.runner.overrides[("inactive-simulation-backup", "apply")] = lambda: (1, b"", b"DAILY_BACKUP: FAILED\n")
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_BACKUP_STEP")
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "recover"))[0:2],
                         (0, "PREUPGRADE_BACKUP: RECOVERED\n"))
        self.assertTrue(self.docker.containers[BETA_ID]["State"]["Running"])
        self.assertEqual(self.docker.starts, 1)

    def test_recover_beta_never_restarts_a_possibly_signalled_process(self):
        self.docker.wait_fails = True  # still running after TERM, policy already "no"
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_BETA_STOP")
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "recover")), "PREUPGRADE_BETA_RESTORE")
        self.assertEqual(self.docker.starts, 0)
        self.assertEqual(self.docker.containers[BETA_ID]["HostConfig"]["RestartPolicy"]["Name"], "no")
        # After it exits on its own, the retry restores and starts it.
        self.docker.containers[BETA_ID]["State"].update(Running=False, Status="exited", ExitCode=0)
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "recover"))[0:2],
                         (0, "PREUPGRADE_BACKUP: RECOVERED\n"))
        self.assertEqual(self.docker.starts, 1)

    def test_recover_beta_refused_with_handoff_or_after_resume(self):
        self.fenced_backup_failure()
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_BACKUP_STEP")
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "recover-beta")), "PREUPGRADE_PENDING")
        self.assertEqual(self.docker.starts, 0)

    # Major 4
    def test_worker_source_must_equal_application_commit(self):
        argv = [a if a != APP else OTHER for a in self.argv("--preflight")]
        argv[argv.index("--application-commit") + 1] = APP
        self.assertFailed(self.run_tool(argv), "PREUPGRADE_WORKER_SOURCE")
        argv[0] = "--apply"
        self.assertFailed(self.run_tool(argv), "PREUPGRADE_WORKER_SOURCE")
        self.assertEqual(self.mutations(), [])

    # Minor 5
    def test_run_child_kills_the_whole_group_and_is_bounded(self):
        import time as _time
        pidfile = os.path.join(self.temp.name, "grandchild.pid")
        script = ("import os,signal,subprocess,sys,time\n"
                  "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
                  "p=subprocess.Popen([sys.executable,'-c','import signal,time;"
                  "signal.signal(signal.SIGTERM, signal.SIG_IGN);time.sleep(60)'])\n"
                  "open(sys.argv[1],'w').write(str(p.pid))\n"
                  "time.sleep(60)\n")
        started = _time.monotonic()
        code, _, _ = tool.rg.run_child([sys.executable, "-c", script, pidfile], dict(os.environ), 1,
                                       term_grace=1, kill_grace=5)
        self.assertIsNone(code)
        self.assertLess(_time.monotonic() - started, 15)
        grandchild = int(Path(pidfile).read_text())
        deadline = _time.monotonic() + 5
        while _time.monotonic() < deadline:
            try:
                os.kill(grandchild, 0)
            except ProcessLookupError:
                break
            _time.sleep(0.05)
        else:
            self.fail("grandchild survived the group kill")

    # Minor 7
    def test_required_confirmation_must_be_present_and_exact(self):
        for environ in ({}, {"OWNER_EQUITY_V2_ROLLOUT_CONFIRM": "yes"}):
            with self.subTest(environ=sorted(environ)):
                self.environ = environ
                self.assertFailed(self.run_tool(self.argv("--preflight")), "PREUPGRADE_CONFIRMATION")
                self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_CONFIRMATION")
        self.assertEqual(self.mutations(), [])
        self.assertIsNone(self.progress())
        self.assertNotIn("yes\n", "".join(self.outputs))

    def test_confirmations_follow_the_release_modes(self):
        cases = {("disabled", "off", "rest"): ({}, "not_required", "not_required"),
                 ("", "owner_only", "rest"): ({}, "not_required", "not_required"),
                 ("disabled", "owner_only", "market_ws"):
                     ({tool.STREAM_CONFIRMATION[0]: tool.STREAM_CONFIRMATION[1]}, "not_required", "required")}
        for probe, (environ, equity, stream) in cases.items():
            with self.subTest(probe=probe):
                self.runner.probe, self.environ = probe, environ
                code, out, _ = self.run_tool(self.argv("--preflight"))
                self.assertEqual(code, 0, self.outputs)
                self.assertIn("CONFIRMATIONS owner_equity_v2=%s owner_market_stream=%s\n" % (equity, stream), out)
        self.runner.probe, self.environ = ("disabled", "owner_only", "market_ws"), {}
        self.assertFailed(self.run_tool(self.argv("--preflight")), "PREUPGRADE_CONFIRMATION")
        self.runner.probe = None
        self.assertFailed(self.run_tool(self.argv("--preflight")), "PREUPGRADE_CONFIRMATION")
        self.runner.probe = ("owner_only\nextra=1", "off", "rest")
        self.assertFailed(self.run_tool(self.argv("--preflight")), "PREUPGRADE_CONFIRMATION")

    def test_confirmation_rechecked_before_release_start(self):
        for step in ("pause-observer", "stop-beta", "backup", "verify", "restore-beta-policy"):
            self.assertEqual(self.run_tool(self.argv("--apply", "--step", step))[0], 0, self.outputs)
        self.environ = {}
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "resume")), "PREUPGRADE_CONFIRMATION")
        self.assertFalse([e for e in self.events if e[0] == "wrapper"])
        self.assertIsNone(self.progress()["steps"].get("resume-release"))

    # Minor 8
    def test_early_observer_restore_requires_a_new_pause_before_mutation(self):
        for step in ("pause-observer", "stop-beta"):
            self.assertEqual(self.run_tool(self.argv("--apply", "--step", step))[0], 0, self.outputs)
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "restore-observer"))[0], 0)
        self.assertEqual(self.runner.units[tool.TIMER][1], "active")
        self.assertNotIn("pause-observer", self.steps())
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "backup")), "PREUPGRADE_PROGRESS")
        self.assertFalse([e for e in self.events if e[0] == "pinned" and e[2] == "apply"])
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "pause-observer"))[0], 0)
        self.assertEqual(self.runner.units[tool.TIMER][1], "inactive")
        self.assertEqual(self.run_tool(self.argv())[0:2], (0, "PREUPGRADE_BACKUP: PASS\n"))
        self.assertEqual(self.runner.units[tool.TIMER][1], "active")

    # Minor 9
    def test_release_line_must_match_exactly(self):
        for line in (b"COMPOSE_RELEASE: PASS (something else)\n", (RELEASE_PASS + " extra\n").encode()):
            with self.subTest(line=line):
                self.tearDown()
                self.setUp()
                self.runner.overrides["--simulation-release-start"] = lambda l=line: (0, l, b"")
                self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_RESUME_STEP")
                self.assertEqual(self.steps()["resume-release"], "failed")


class NoCapturedWorkerTests(Base):
    """The capture recorded no simulation worker: resume only what was running."""
    EXPECTED = [m for m in ApplyTests.EXPECTED if m != ("wrapper", "--simulation-start")]
    PASS_SKIPPED = "PREUPGRADE_BACKUP: PASS simulation_worker=not_running_before_not_started\n"
    SKIP = tool.SKIP_LINE + "\n"

    def setUp(self):
        super().setUp()
        self.worker_captured = False

    def release_failed_after_api_start(self):
        # The wrapper removed the no-worker handoff, then failed: resume-simulation is next.
        def release():
            os.unlink(os.path.join(self.root, tool.HANDOFF_PENDING))
            self.docker.start(BETA_ID)
            return 1, b"", b"compose-release: persistent service did not resolve to exactly one container\n"
        self.runner.overrides["--simulation-release-start"] = release
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_RESUME_STEP")
        self.assertEqual(self.steps()["resume-release"], "failed")
        self.assertNotIn("resume-simulation", self.steps())

    def tamper(self, **changes):
        progress = tool.rg.Progress(self.state, tool.PROGRESS_NAME, os.geteuid(), "PREUPGRADE_PROGRESS")
        record = progress.read()
        progress.replace(record, {**{k: v for k, v in record.items() if k not in changes},
                                  **{k: v for k, v in changes.items() if v is not None}})

    def add_runner(self, state="exited"):
        runner = beta_container(identity="4" * 64)
        runner["Config"]["Labels"]["com.docker.compose.service"] = tool.SIMULATION_RUNNER
        runner["State"].update(Running=state == "running", Status=state)
        self.docker.containers["4" * 64] = runner

    def test_full_sequence_skips_simulation_start(self):
        seen = {}

        def observe(flag):
            if flag == "--simulation-release-start":
                record = self.progress()
                seen["decision"] = record.get(tool.WORKER_DECISION)
                seen["state"] = record["steps"]["resume-release"]["state"]
                seen["handoff"] = os.path.exists(os.path.join(self.root, tool.HANDOFF_PENDING))
        self.runner.observe = observe
        code, out, err = self.run_tool(self.argv())
        self.assertEqual((code, out, err), (0, self.SKIP + self.PASS_SKIPPED, ""), self.outputs)
        # Durably recorded, with the handoff still present, before the wrapper ran.
        self.assertEqual(seen, {"decision": False, "state": "started", "handoff": True})
        self.assertEqual(self.mutations(), self.EXPECTED)
        self.assertEqual(self.count(("wrapper", "--simulation-start")), 0)
        record = self.progress()
        self.assertTrue(all(record["steps"][n] == {"state": "done", "code": None} for n in tool.STEPS))
        self.assertIs(record[tool.WORKER_DECISION], False)
        self.assertFalse(os.path.exists(os.path.join(self.root, tool.HANDOFF_PENDING)))
        self.assertFalse(os.path.exists(os.path.join(self.root, tool.START_PENDING)))
        self.assertEqual(self.docker.service_ids(PROJECT, tool.SIMULATION_RUNNER), [])
        self.assertEqual(self.runner.units[tool.TIMER][1], "active")
        # A completed run is idempotent: same PASS field, no repeated skip line, no mutation.
        self.assertEqual(self.run_tool(self.argv())[0:3], (0, self.PASS_SKIPPED, ""))
        self.assertEqual(self.mutations(), self.EXPECTED)
        self.assertNoValues()

    def test_explicit_resume_group_skips(self):
        for step in ("pause-observer", "stop-beta", "backup", "verify", "restore-beta-policy"):
            self.assertEqual(self.run_tool(self.argv("--apply", "--step", step))[0], 0, self.outputs)
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "resume"))[0:2],
                         (0, self.SKIP + "PREUPGRADE_BACKUP: STEP resume\n"))
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "restore-observer"))[0:2],
                         (0, self.PASS_SKIPPED))
        self.assertEqual(self.mutations(), self.EXPECTED)

    def test_release_retry_keeps_the_recorded_decision(self):
        self.runner.overrides["--simulation-release-start"] = lambda: (
            2, b"", b"BLOCKED_EXTERNAL: owner_equity_v2_rollout_confirmation_required\n")
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_RESUME_STEP")
        self.assertIs(self.progress()[tool.WORKER_DECISION], False)
        # The handoff now says a worker was captured: the evidence changed, so refuse.
        self.worker_captured = True
        self.write_pending(tool.HANDOFF_PENDING, "switched")
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_WORKER_DECISION")
        self.assertEqual(self.count(("wrapper", "--simulation-release-start")), 1)
        self.worker_captured = False
        self.write_pending(tool.HANDOFF_PENDING, "switched")
        self.assertEqual(self.run_tool(self.argv())[0:3], (0, self.SKIP + self.PASS_SKIPPED, ""))

    def test_ambiguous_or_missing_capture_record_fails_closed_before_the_wrapper(self):
        for step in ("pause-observer", "stop-beta", "backup", "verify", "restore-beta-policy"):
            self.assertEqual(self.run_tool(self.argv("--apply", "--step", step))[0], 0, self.outputs)
        good = self.worker_fields(True)["worker_generation"]
        cases = {
            "keys missing": {"worker_generation": None, "worker_policy": None, "worker_running": None},
            "none with policy": {"worker_generation": None, "worker_policy": "no", "worker_running": False},
            "none while running": {"worker_generation": None, "worker_policy": None, "worker_running": True},
            "short generation": {"worker_generation": good[:3], "worker_policy": "no", "worker_running": False},
            "bad identity": {"worker_generation": ["x"] + good[1:], "worker_policy": "no", "worker_running": False},
            "bad restarts": {"worker_generation": good[:3] + [True], "worker_policy": "no", "worker_running": False},
            "bad policy": {"worker_generation": good, "worker_policy": "always", "worker_running": False},
            "string generation": {"worker_generation": "none", "worker_policy": None, "worker_running": False},
        }
        for name, fields in cases.items():
            with self.subTest(case=name):
                self.write_pending(tool.HANDOFF_PENDING, "switched", **fields)
                if name == "keys missing":
                    path = os.path.join(self.root, tool.HANDOFF_PENDING)
                    data = json.loads(Path(path).read_text())
                    for key in tool.WORKER_KEYS:
                        data.pop(key)
                    Path(path).write_text(json.dumps(data))
                self.assertFailed(self.run_tool(self.argv("--apply", "--step", "resume")),
                                  "PREUPGRADE_WORKER_DECISION")
                self.assertEqual(self.count(("wrapper", "--simulation-release-start")), 0)
                self.assertNotIn("resume-release", self.steps())
                self.assertNotIn(tool.WORKER_DECISION, self.progress())

    def test_missing_or_invalid_decision_record_fails_closed_at_resume_simulation(self):
        self.release_failed_after_api_start()
        for value in (None, "no", 0, 1):
            with self.subTest(value=value):
                self.tamper(**{tool.WORKER_DECISION: value})
                if value is None:
                    self.assertNotIn(tool.WORKER_DECISION, self.progress())
                self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_WORKER_DECISION")
                self.assertNotIn("resume-simulation", self.steps())
                self.assertEqual(self.count(("wrapper", "--simulation-start")), 0)
        # Nothing was consumed: with the record restored the step completes.
        self.tamper(**{tool.WORKER_DECISION: False})
        self.assertEqual(self.run_tool(self.argv())[0:3], (0, self.SKIP + "PREUPGRADE_BACKUP: RECOVERED\n", ""))
        self.assertEqual(self.count(("wrapper", "--simulation-start")), 0)

    def test_skip_refuses_when_a_runner_or_a_record_exists(self):
        self.release_failed_after_api_start()
        refusals = (
            ("runner exited", lambda: self.add_runner("exited"), "PREUPGRADE_SIMULATION_RUNNER"),
            ("runner running", lambda: self.add_runner("running"), "PREUPGRADE_SIMULATION_RUNNER"),
            ("start pending", lambda: self.write_pending(tool.START_PENDING, "started"), "PREUPGRADE_PENDING"),
            ("handoff", lambda: self.write_pending(tool.HANDOFF_PENDING, "api-started"), "PREUPGRADE_PENDING"),
        )
        for name, arrange, code in refusals:
            with self.subTest(case=name):
                arrange()
                self.assertFailed(self.run_tool(self.argv()), code)
                self.assertNotIn("resume-simulation", self.steps())
                self.assertEqual(self.count(("wrapper", "--simulation-start")), 0)
                self.assertNotIn(tool.SKIP_LINE, "".join(self.outputs))
                self.docker.containers.pop("4" * 64, None)
                for pending in (tool.START_PENDING, tool.HANDOFF_PENDING):
                    if os.path.exists(os.path.join(self.root, pending)):
                        os.unlink(os.path.join(self.root, pending))
        # Refusals left the step unjournaled, so it runs once the evidence is clean.
        self.assertEqual(self.run_tool(self.argv())[0:3], (0, self.SKIP + "PREUPGRADE_BACKUP: RECOVERED\n", ""))
        self.assertEqual(self.steps()["resume-simulation"], "done")
        self.assertEqual(self.count(("wrapper", "--simulation-start")), 0)
        self.assertEqual(self.run_tool(self.argv())[0:3], (0, "PREUPGRADE_BACKUP: RECOVERED\n", ""))

    def test_runner_listing_failure_fails_closed(self):
        self.release_failed_after_api_start()
        original = self.docker.call

        def broken(args, timeout=15):
            if list(args)[0] == "ps" and any(tool.SIMULATION_RUNNER in a for a in args):
                raise runtime.RuntimeErrorCode("DAILY_RUNTIME_DOCKER")
            return original(args, timeout)
        self.docker.call = broken
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_SIMULATION_RUNNER")
        self.assertNotIn("resume-simulation", self.steps())

    def test_recover_after_failed_backup_skips_simulation_start(self):
        def backup():
            self.write_pending(tool.HANDOFF_PENDING, "switched")
            return 1, b"", b"PREACTIVATION_BACKUP: FAILED PREACTIVATION_RETENTION_LIMIT\nDAILY_BACKUP: FAILED\n"
        self.runner.overrides[("inactive-simulation-backup", "apply")] = backup
        self.assertFailed(self.run_tool(self.argv()), "PREUPGRADE_BACKUP_STEP")
        before = len(self.mutations())
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "recover"))[0:3],
                         (0, self.SKIP + "PREUPGRADE_BACKUP: RECOVERED\n", ""))
        self.assertEqual(self.mutations()[before:],
                         [("docker", "update", "--restart=unless-stopped"),
                          ("wrapper", "--simulation-release-start"), ("systemctl", "start", tool.TIMER)])
        self.assertEqual(self.steps()["resume-simulation"], "done")
        self.assertEqual(self.steps()["restore-observer"], "recovered")
        self.assertIs(self.progress()[tool.WORKER_DECISION], False)
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "recover"))[0:3],
                         (0, "PREUPGRADE_BACKUP: RECOVERED\n", ""))
        self.assertNotIn("PASS", "".join(self.outputs))

    def test_recover_after_release_failure_uses_the_recorded_decision(self):
        self.release_failed_after_api_start()
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "recover"))[0:3],
                         (0, self.SKIP + "PREUPGRADE_BACKUP: RECOVERED\n", ""))
        self.assertEqual(self.steps()["verify-beta"], "done")
        self.assertEqual(self.count(("wrapper", "--simulation-start")), 0)


class CapturedWorkerTests(Base):
    def test_captured_worker_still_starts_the_simulation(self):
        seen = {}
        self.runner.observe = lambda flag: seen.setdefault(flag, self.progress().get(tool.WORKER_DECISION))
        code, out, err = self.run_tool(self.argv())
        self.assertEqual((code, out, err), (0, "PREUPGRADE_BACKUP: PASS\n", ""), self.outputs)
        self.assertEqual(seen["--simulation-release-start"], True)  # journaled with the started mark
        self.assertEqual(seen["--simulation-start"], True)
        self.assertEqual(self.mutations(), ApplyTests.EXPECTED)
        self.assertEqual(self.count(("wrapper", "--simulation-start")), 1)
        self.assertIs(self.progress()[tool.WORKER_DECISION], True)
        self.assertNotIn("SKIPPED", "".join(self.outputs))
        self.assertEqual(self.run_tool(self.argv())[0:2], (0, "PREUPGRADE_BACKUP: PASS\n"))


class NightlyTests(Base):
    def setUp(self):
        super().setUp()
        self.children = self.runner
        self.runner = self.run_nightly_child
        self.host.ops = SimpleNamespace(release_lock=lambda _: contextlib.nullcontext())
        self.pinned_entry = Path(self.root) / "maintenance" / TOOL / tool.ENTRYPOINT
        self.pinned_entry.write_text("def inactive_simulation_tool(context, verifier_root): pass\n")
        self.services = {tool.API: "4" * 64, tool.DATABASE: "5" * 64,
                         tool.V2: "6" * 64, tool.SIMULATION_RUNNER: "9" * 64}
        self.fail_verify = False
        self.interrupt_verify = False
        self.drift_after_verify = None
        self.fail_backup_at = None
        self.interrupt_api_resume = False
        self.interrupt_simulation_resume = False
        self.worker_policy_ok = True
        self.observed_worker = None
        self.bad_targets = False
        self.old_maintenance_plan = False
        self.configured_paper = False
        self.paper_proof = "sha256:" + "d" * 64
        self.paper_archive = None
        self.paper_archive_count = 0
        self.changed_paper_verify = False
        self.changed_paper_readback = False
        for digit, (service, identity) in enumerate(self.services.items(), 4):
            image = "sha256:" + format(digit, "x") * 64
            value = beta_container(identity, image)
            value["Config"]["Labels"]["com.docker.compose.service"] = service
            self.docker.containers[identity] = value
            self.host.images[APP][service] = image
        self.docker.containers[self.services[tool.SIMULATION_RUNNER]]["HostConfig"]["RestartPolicy"]["Name"] = "no"

    def generation(self, service):
        value = self.docker.containers[self.services[service]]
        return [value["Id"], value["Created"], value["State"]["StartedAt"], value["RestartCount"]]

    def stop(self, service):
        value = self.docker.containers[self.services[service]]
        value["State"].update(Running=False, Status="exited", ExitCode=0)
        value["HostConfig"]["RestartPolicy"]["Name"] = "no"

    def run_nightly_child(self, argv, env, timeout):
        if tool.NIGHTLY_API_ONLY in argv:
            daily_api = "--simulation-resume-daily-api" in argv
            self.assertIn("--simulation-release-start", argv)
            if "--plan" in argv:
                targets = b"api-server web" if self.bad_targets else b"api-server"
                prefix = b"SIMULATION_DAILY_API_RESUME_TARGETS: " if daily_api else b"SIMULATION_API_RESUME_TARGETS: "
                return 0, prefix + targets + b"\n", b""
            if daily_api and self.daily_enabled:
                self.assertEqual(argv[argv.index("--expected-nightly-daily-control") + 1],
                                 f"ENABLED:{self.daily_epoch}")
            else:
                self.assertNotIn("--expected-nightly-daily-control", argv)
            self.events.append(("wrapper", "--simulation-resume-daily-api" if daily_api else tool.NIGHTLY_API_ONLY))
            self.assertFalse(self.docker.containers[BETA_ID]["State"]["Running"])
            self.docker.start(self.services[tool.API])
            api = self.docker.containers[self.services[tool.API]]
            api["HostConfig"]["RestartPolicy"]["Name"] = "unless-stopped"
            api["Config"]["Labels"]["org.lagrange.api-release-attempt"] = self.backup_fields["api_attempt"]
            if self.worker_captured:
                self.write_pending(tool.HANDOFF_PENDING, "api-started", **self.backup_fields,
                                   target_api_generation=self.generation(tool.API))
            else:
                os.unlink(os.path.join(self.root, tool.HANDOFF_PENDING))
            if self.interrupt_api_resume:
                raise KeyboardInterrupt
            return 0, (tool.NIGHTLY_API_LINE + "\n").encode(), b""
        if argv[-1] == "--simulation-drain":
            self.events.append(("wrapper", "--simulation-drain"))
            pending = json.loads(Path(self.root, tool.HANDOFF_PENDING).read_text())
            if pending["phase"] == "fencing":
                self.assertTrue(self.docker.containers[self.services[tool.API]]["State"]["Running"])
                self.stop(tool.API)
                if self.worker_captured:
                    self.stop(tool.SIMULATION_RUNNER)
            else:
                self.assertEqual(pending["phase"], "prepared")
            self.write_pending(tool.HANDOFF_PENDING, "switched", **self.backup_fields)
            return 0, (tool.SIMULATION_DRAINED_LINE + "\n").encode(), b""
        if argv[-1] == "--simulation-start":
            self.docker.start(self.services[tool.SIMULATION_RUNNER])
            if self.interrupt_simulation_resume:
                self.children(argv, env, timeout)
                raise KeyboardInterrupt
        if "--operation" in argv and argv[argv.index("--mode") + 1] == "plan":
            if self.old_maintenance_plan:
                return self.children(argv, env, timeout)
            if "nightly-combined-backup" in argv:
                return 0, b"NIGHTLY_COMBINED_PLAN: fixture\n", b""
            return 0, (tool.PLAN_LINE + "fixture" + tool.NIGHTLY_PLAN_SUFFIX + "\n").encode(), b""
        if self.configured_paper and "--operation" in argv:
            operation, mode = argv[argv.index("--operation") + 1], argv[argv.index("--mode") + 1]
            if operation == "nightly-combined-backup" and mode == "preflight":
                self.children(argv, env, timeout)
                return 0, ("\n".join((tool.PREFLIGHT_LINE, tool.PAPER_PREFLIGHT_LINE,
                                      tool.PAPER_ORIGIN_PREFIX + self.paper_proof)) + "\n").encode(), b""
            if operation == "nightly-combined-paper-readback":
                self.assertEqual(mode, "readback")
                self.assertIn("--expected-nightly-paper-off", argv)
                self.assertEqual(argv[argv.index("--expected-nightly-paper-origin") + 1], self.paper_proof)
                self.assertEqual(argv[argv.index("--expected-nightly-daily-control") + 1],
                                 f"{'ENABLED' if self.daily_enabled else 'DISABLED'}:{self.daily_epoch}")
                self.assertTrue(self.docker.containers[self.services[tool.API]]["State"]["Running"])
                self.assertTrue(self.docker.containers[BETA_ID]["State"]["Running"])
                self.events.append(("pinned", operation, mode))
                return (1, b"", b"") if self.changed_paper_readback else (
                    0, (tool.PAPER_READBACK_LINE + "\n").encode(), b"")
        if "--operation" in argv and argv[argv.index("--mode") + 1] == "apply":
            operation = argv[argv.index("--operation") + 1]
            if operation.startswith("nightly-combined-"):
                self.assertEqual(argv[argv.index("--expected-nightly-daily-control") + 1],
                                 f"{'ENABLED' if self.daily_enabled else 'DISABLED'}:{self.daily_epoch}")
                self.assertTrue(all(not self.docker.containers[identity]["State"]["Running"]
                                    for name in runtime.SERVICES
                                    for identity in self.docker.service_ids(PROJECT, name)))
            if operation in ("inactive-simulation-backup", "nightly-combined-backup"):
                self.backup_fields = {"api_generation": self.generation(tool.API), "api_attempt": "a" * 32}
                if self.worker_captured:
                    self.backup_fields.update(worker_generation=self.generation(tool.SIMULATION_RUNNER),
                                              worker_running=True, worker_policy="no")
                if self.fail_backup_at in ("fencing", "partial-fencing"):
                    self.events.append(("pinned", operation, "apply"))
                    self.write_pending(tool.HANDOFF_PENDING, "fencing", **self.backup_fields)
                    if self.fail_backup_at == "partial-fencing":
                        self.stop(tool.API)
                    return 1, b"", b""
                self.stop(tool.API)
                if self.worker_captured:
                    self.stop(tool.SIMULATION_RUNNER)
                if self.fail_backup_at == "prepared":
                    self.events.append(("pinned", operation, "apply"))
                    self.write_pending(tool.HANDOFF_PENDING, "prepared", **self.backup_fields)
                    return 1, b"", b""
                result = self.children(argv, env, timeout)
                self.write_pending(tool.HANDOFF_PENDING, "switched", **self.backup_fields)
                if operation == "nightly-combined-backup":
                    if self.configured_paper:
                        self.assertIn("--expected-nightly-paper-off", argv)
                        self.paper_archive_count += 1
                        self.paper_archive = "sha256:" + f"{self.paper_archive_count:064x}"
                        return 0, ("\n".join((tool.PAPER_BACKUP_LINE, tool.PAPER_ARCHIVE_PREFIX + self.paper_archive,
                                            tool.PAPER_ORIGIN_PREFIX + self.paper_proof)) + "\n").encode(), b""
                    return 0, b"DAILY_BACKUP: NIGHTLY_COMBINED_VERIFIED_SOURCE_UNCHANGED\n", b""
                return result
            if operation in ("inactive-simulation-verify", "nightly-combined-verify"):
                if self.interrupt_verify:
                    raise KeyboardInterrupt
                result = self.children(argv, env, timeout)
                if self.drift_after_verify:
                    self.drift_after_verify()
                if self.fail_verify:
                    return 1, b"", b""
                if operation == "nightly-combined-verify":
                    if self.configured_paper:
                        self.assertIn("--expected-nightly-paper-off", argv)
                        digest = argv[argv.index("--expected-nightly-paper-archive") + 1]
                        self.assertEqual(digest, self.paper_archive)
                        result = "sha256:"+"3"*64 if self.changed_paper_verify else digest
                        return 0, ("\n".join((tool.PAPER_VERIFY_LINE, tool.PAPER_ARCHIVE_PREFIX + result,
                                            tool.PAPER_ORIGIN_PREFIX + self.paper_proof)) + "\n").encode(), b""
                    return 0, b"DAILY_BACKUP: NIGHTLY_COMBINED_VERIFIED_ISOLATED_DISABLED\n", b""
                return result
        return self.children(argv, env, timeout)

    def test_real_installed_wrapper_declares_exact_api_only_target(self):
        result = subprocess.run(["/bin/bash", str(HERE / "compose-release.sh"), "--scope", "release", "--plan",
                                 "--simulation-release-start", tool.NIGHTLY_API_ONLY],
                                env=tool.rg.child_environment(), capture_output=True, check=True)
        self.assertEqual(result.stdout, b"SIMULATION_API_RESUME_TARGETS: api-server\n")
        self.assertEqual(result.stderr, b"")

    def test_real_wrapper_daily_api_plan_keeps_exact_single_service_scope(self):
        args = ["/bin/bash", str(HERE / "compose-release.sh"), "--scope", "release", "--plan",
                "--simulation-release-start", tool.NIGHTLY_API_ONLY, "--simulation-resume-daily-api"]
        result = subprocess.run(args, env=tool.rg.child_environment(), capture_output=True, check=True)
        self.assertEqual(result.stdout, b"SIMULATION_DAILY_API_RESUME_TARGETS: api-server\n")
        refused = subprocess.run([item for item in args if item != tool.NIGHTLY_API_ONLY],
                                 env=tool.rg.child_environment(), capture_output=True)
        self.assertNotEqual(refused.returncode, 0)
        self.assertNotIn(b"RESUME_TARGETS", refused.stdout)
        for extra in (["--expected-nightly-daily-control", "ENABLED:4"],
                      ["--expected-nightly-daily-control", "DISABLED:4"]):
            refused = subprocess.run(args + extra, env=tool.rg.child_environment(), capture_output=True)
            self.assertNotEqual(refused.returncode, 0)
            self.assertNotIn(b"RESUME_TARGETS", refused.stdout)

    def combined_fixture(self, *, enabled):
        config_hash = "sha256:" + "7" * 64
        values = {
            "OWNER_BETA_DAILY_MODE": "owner_only", "OWNER_BETA_ACCESS_MODE": "owner_only",
            "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1", "OWNER_BETA_PAPER_MODE": "disabled",
            "LAGRANGE_DATA_DIR": "/protected/data", "LAGRANGE_ARTIFACTS_DIR": "/protected/artifacts",
            "LAGRANGE_RUNTIME_SECRET_DIR": "/protected/runtime-secrets", "POSTGRES_DB": "lagrange",
            "MAX_JOBS_PER_OWNER": "4", "PRICE_BETA_LEASE_SECS": "60", "PRICE_BETA_BACKOFF_SECS": "30",
        }
        self.daily_enabled = enabled
        self.daily_epoch = 4
        database = mock.Mock()
        database.nightly_state.side_effect = lambda *args: (self.daily_enabled, self.daily_epoch)

        def control(release, commit, env, action):
            self.events.append(("daily-control", action))
            enabled_after = action == "enable"
            self.daily_epoch += int(enabled_after != self.daily_enabled)
            self.daily_enabled = enabled_after

        database.control.side_effect = control
        self.daily_database = database
        self.daily_values = values
        self.host.release = self.root + "/releases/" + APP
        self.host.images[APP]["db-migrate"] = "sha256:" + "f" * 64
        self.host.runtime = runtime
        self.host.ops = SimpleNamespace(
            release_lock=lambda _: contextlib.nullcontext(), read_context=lambda *args: (values, []),
            reject_ambient=lambda _: None, project_binding=lambda *args: (PROJECT, "daily"),
            install=SimpleNamespace(verify_config=lambda *args: ({"first_auto_session": CONFIG_DATE}, config_hash)),
            database_name=ops.database_name, Context=ops.Context, Database=lambda *args: database,
            Operations=ops.Operations, global_cohort=ops.global_cohort, stopped=ops.stopped, verify_api=ops.verify_api,
            snapshot_equal=ops.snapshot_equal,
        )
        if not enabled:
            self.docker.containers[self.services[tool.API]]["Config"]["Env"] = ["OWNER_BETA_DAILY_MODE=disabled"]
            return
        binding = runtime.Binding(PROJECT, APP, BETA_IMAGE, config_hash, self.host.release,
            values["LAGRANGE_DATA_DIR"], values["LAGRANGE_ARTIFACTS_DIR"],
            values["LAGRANGE_RUNTIME_SECRET_DIR"], "lagrange", "4", "60", "30")
        self.daily_binding = binding
        network_name = PROJECT + "_owner-beta-db"
        network_id = "7" * 64
        inspect = self.docker.object
        self.docker.object = lambda kind, identity, *args: {
            "Id": network_id, "Internal": True,
            "Labels": {"com.docker.compose.project": PROJECT, "com.docker.compose.network": "owner-beta-db"},
        } if kind == "network" and identity == network_name else inspect(kind, identity, *args)
        api = self.docker.containers[self.services[tool.API]]
        api["Config"].update(User="10001:10001", Entrypoint=["/usr/local/bin/api-server"])
        api["Config"]["Env"] = [f"{key}={value}" for key, value in {
            **{key: values[key] for key in ("OWNER_BETA_DAILY_MODE", "OWNER_BETA_ACCESS_MODE",
               "OWNER_BETA_PRICE_INPUT_MODE", "OWNER_BETA_PAPER_MODE", "MAX_JOBS_PER_OWNER")},
            "LAGRANGE_CODE_COMMIT": APP, "DB_HOST": "postgres", "DB_PORT": "5432", "DB_NAME": "lagrange",
        }.items()]
        for index, (service, identity) in enumerate(zip(runtime.SERVICES, ("2" * 64, "3" * 64, BETA_ID))):
            uid = (10002, 10003, 10001)[index]
            value = beta_container(identity)
            value["Config"]["Labels"]["com.docker.compose.service"] = service
            value["Config"].update(User=f"{uid}:{uid}", Entrypoint=[runtime.BINARY],
                Cmd=(["daily-producer"], ["daily-admitter"], ["--artifact-root", "/data/owner-beta-artifacts"])[index],
                Healthcheck={"Test": ["CMD", runtime.BINARY, "healthcheck"]})
            env = {
                "APP_ENV": "production", "LAGRANGE_CODE_COMMIT": APP, "PRICE_BETA_DAILY_MODE": "owner_only",
                "PRICE_BETA_DAILY_CONFIG_FILE": "/run/owner-beta-daily/config.json",
                "PRICE_BETA_DAILY_CONFIG_SHA256": config_hash,
                "PRICE_BETA_HEALTH_STATE_PATH": "/run/owner-beta-health/health.json",
                "PRICE_BETA_HEALTH_MAX_AGE_SECS": "180",
            }
            if index:
                env.update(DB_HOST="postgres", DB_PORT="5432", DB_NAME="lagrange",
                           DB_USER="daily_input_admitter" if index == 1 else "worker",
                           DB_PASSWORD_FILE="/run/secrets/db_daily_input_admitter_password"
                           if index == 1 else "/run/secrets/db_worker_password")
            if index == 2:
                env.update(PRICE_BETA_POLL_SECS="2", PRICE_BETA_RECOVERY_SECS="30",
                           PRICE_BETA_HEARTBEAT_SECS="10", PRICE_BETA_LEASE_SECS="60",
                           PRICE_BETA_BACKOFF_SECS="30", PRICE_BETA_COMPUTE_TIMEOUT_SECS="300")
            value["Config"]["Env"] = [f"{key}={item}" for key, item in env.items()]
            value["HostConfig"].update(ReadonlyRootfs=True, Init=True, Privileged=False, CapDrop=["ALL"],
                SecurityOpt=["no-new-privileges:true"], GroupAdd=["10001"] if index < 2 else [], IpcMode="private",
                Memory=1024**3, NanoCpus=10**9, Tmpfs={"/run/owner-beta-health": f"uid={uid},gid={uid},mode=0750"},
                LogConfig={"Type": "json-file", "Config": {"max-size": "10m", "max-file": "3", "tag": "{{.Name}}"}},
                NetworkMode=network_name if index else "none")
            value["Mounts"] = [dict(Type="bind", Propagation="rprivate", Destination=destination,
                                    Source=source, RW=writable)
                               for destination, (source, writable) in binding.mounts(service).items()]
            value["NetworkSettings"] = {"Networks": {network_name: {"NetworkID": network_id}} if index else {}}
            self.docker.containers[identity] = value

    def combined_argv(self, *modes):
        argv = self.argv(*modes, "--backup-profile", "daily-simulation")
        argv[argv.index("--expected-db-first-session") + 1] = CONFIG_DATE
        return argv

    def test_two_combined_nights_resume_same_daily_cohort_last_at_original_epoch(self):
        self.combined_fixture(enabled=True)
        argv = self.combined_argv("--apply", "--nightly")
        for night in range(2):
            if night:
                # The real outer scheduler retains each date under a different
                # state directory. Preserve the first just as its journal does.
                state = Path(argv[argv.index("--state-dir") + 1])
                next_state = state.with_name("fixture-next-date")
                next_state.mkdir(mode=0o700)
                argv[argv.index("--state-dir") + 1] = str(next_state)
                self.state = str(next_state)
                self.events.clear()
            real_resume = runtime.resume_after_backup

            def observe_resume(*args):
                self.events.append(("resume", "daily-writers"))
                return real_resume(*args)

            with mock.patch.object(runtime, "resume_after_backup", side_effect=observe_resume) as resume:
                result = self.run_tool(argv)
                self.assertEqual(result[0], 0, result)
                resume.assert_called_once()
            self.assertEqual((self.daily_enabled, self.daily_epoch), (True, 4))
            verify = self.events.index(("pinned", "nightly-combined-verify", "apply"))
            api = self.events.index(("wrapper", "--simulation-resume-daily-api"))
            observer = self.events.index(("systemctl", "start", tool.TIMER))
            self.assertLess(verify, api)
            self.assertLess(api, observer)
            self.assertLess(observer, self.events.index(("resume", "daily-writers")))
            original_ids = ("2" * 64, "3" * 64, BETA_ID)
            for name, identity in zip(runtime.SERVICES, original_ids):
                self.assertEqual(self.docker.service_ids(PROJECT, name), [identity])
                self.assertTrue(self.docker.containers[identity]["State"]["Running"])
            self.assertFalse(any(event[0] == "daily-control" for event in self.events))
        self.assertTrue(self.progress()["nightly_completed"])
        self.assertEqual(self.progress()["nightly"]["daily_epoch"], 4)
        self.assertEqual(self.steps()[tool.DAILY_RESUME_STEP], "done")

    def test_combined_nightly_after_rebind_retains_off_and_manual_beta_only(self):
        self.combined_fixture(enabled=False)
        result = self.run_tool(self.combined_argv("--apply", "--nightly"))
        self.assertEqual(result[0], 0, result)
        self.assertFalse(self.daily_enabled)
        self.assertNotIn(("daily-control", "enable"), self.events)
        self.assertNotIn(("daily-control", "disable"), self.events)
        self.assertNotIn(("wrapper", "--simulation-resume-daily-api"), self.events)
        self.assertTrue(all(not self.docker.service_ids(PROJECT, service) for service in tool.DAILY_SERVICES))
        self.assertEqual(self.steps()[tool.DAILY_RESUME_STEP], "done")

    def test_two_configured_off_nights_require_distinct_archives_and_same_control_proof(self):
        self.configured_paper = True
        self.combined_fixture(enabled=True)
        argv = self.combined_argv("--apply", "--nightly", "--expected-nightly-paper-profile", "CONFIGURED_OFF")
        archives = []
        for night in range(2):
            if night:
                state = Path(argv[argv.index("--state-dir") + 1]).with_name("next-configured-night")
                state.mkdir(mode=0o700)
                argv[argv.index("--state-dir") + 1] = self.state = str(state)
                self.events.clear()
            result = self.run_tool(argv)
            self.assertEqual(result[0], 0, result)
            record = self.progress()
            self.assertTrue(record["nightly"]["paper_configured_off"])
            self.assertEqual(record["nightly"]["paper_preflight_origin"], self.paper_proof)
            self.assertTrue(record["nightly_paper_origin_unchanged"])
            self.assertEqual(record["nightly_paper_origin"], self.paper_proof)
            archives.append(record["nightly_paper_archive"])
            verify = self.events.index(("pinned", "nightly-combined-verify", "apply"))
            resume = self.events.index(("wrapper", "--simulation-resume-daily-api"))
            readback = self.events.index(("pinned", "nightly-combined-paper-readback", "readback"))
            self.assertLess(verify, resume)
            self.assertLess(resume, readback)
            self.assertEqual((self.daily_enabled, self.daily_epoch), (True, 4))
            self.daily_database.control.assert_not_called()
        self.assertNotEqual(*archives)

    def test_configured_off_nightly_with_unverified_digest_never_resumes_api(self):
        self.configured_paper = self.changed_paper_verify = True
        self.combined_fixture(enabled=False)
        result = self.run_tool(self.combined_argv("--apply", "--nightly"))
        self.assertEqual(result[0], 1)
        self.assertIn("PREUPGRADE_VERIFY_STEP", result[2])
        self.assertEqual(self.steps()["verify"], "failed")
        self.assertNotIn(("wrapper", tool.NIGHTLY_API_ONLY), self.events)
        self.assertFalse(self.daily_enabled)

    def test_configured_off_nightly_rejects_profile_drift_before_signalling(self):
        self.configured_paper = True
        self.combined_fixture(enabled=False)
        argv = self.combined_argv("--apply", "--nightly", "--expected-nightly-paper-profile", "LEGACY_EMPTY")
        result = self.run_tool(argv)
        self.assertEqual(result[0], 1)
        self.assertIn(tool.NIGHTLY_ERROR, result[2])
        self.assertEqual(self.progress()["steps"], {})
        self.assertTrue(self.docker.containers[BETA_ID]["State"]["Running"])

    def test_configured_off_nightly_rejects_origin_drift_before_signalling(self):
        self.configured_paper = True
        self.combined_fixture(enabled=False)
        argv = self.combined_argv("--apply", "--nightly", "--expected-nightly-paper-profile", "CONFIGURED_OFF",
                                  "--expected-nightly-paper-origin", "sha256:" + "0"*64)
        self.assertFailed(self.run_tool(argv), tool.NIGHTLY_ERROR)
        self.assertEqual(self.progress()["steps"], {})
        self.assertEqual(self.mutations(), [])

    def test_configured_off_recovery_requires_origin_or_proof_backup_never_started(self):
        self.configured_paper = self.changed_paper_readback = True
        self.combined_fixture(enabled=False)
        self.assertFailed(self.run_tool(self.combined_argv("--apply", "--nightly")), tool.NIGHTLY_ERROR)
        record = self.progress()
        self.assertEqual(record["steps"]["backup"]["state"], "done")
        self.assertEqual(record["nightly_paper_origin"], self.paper_proof)
        path = Path(self.state) / (tool.PROGRESS_NAME + ".json")
        recover = self.combined_argv("--apply", "--step", "recover")
        recovered_services = self.mutations()

        self.assertFailed(self.run_tool(recover), tool.NIGHTLY_ERROR)
        self.assertEqual(self.progress(), record)
        self.assertEqual(self.mutations(), recovered_services)

        self.changed_paper_readback = False
        uncertain = {key: value for key, value in record.items()
                     if key not in {"nightly_paper_archive", "nightly_paper_origin"}}
        uncertain["steps"] = {"backup": {"state": "started", "code": None}}
        path.write_bytes(tool.rg.canonical(uncertain))
        self.assertFailed(self.run_tool(recover), tool.NIGHTLY_ERROR)
        self.assertEqual(self.progress(), uncertain)

        before_archive = dict(uncertain, steps={"stop-beta": {"state": "failed", "code": tool.NIGHTLY_ERROR}})
        path.write_bytes(tool.rg.canonical(before_archive))
        readback = ("pinned", "nightly-combined-paper-readback", "readback")
        before_reads = self.events.count(readback)
        self.assertEqual(self.run_tool(recover)[0:2], (0, "PREUPGRADE_BACKUP: RECOVERED\n"))
        self.assertEqual(self.events.count(readback), before_reads + 1)
        self.assertNotIn("nightly_paper_origin", self.progress())
        self.assertTrue(self.progress()["nightly_recovered"])
        self.assertEqual(self.mutations(), recovered_services)

    def test_combined_nightly_preserves_the_original_daily_api_when_control_was_off(self):
        self.combined_fixture(enabled=True)
        self.daily_enabled = False
        self.docker.containers.pop("2" * 64)
        self.docker.containers.pop("3" * 64)
        self.docker.containers[BETA_ID] = beta_container()
        result = self.run_tool(self.combined_argv("--apply", "--nightly"))
        self.assertEqual(result[0], 0, result)
        self.assertIn(("wrapper", "--simulation-resume-daily-api"), self.events)
        self.assertTrue(self.progress()["nightly"]["daily_api"])
        self.assertFalse(self.daily_enabled)
        self.assertNotIn(("daily-control", "enable"), self.events)

    def test_combined_nightly_partial_resume_uncertainty_stops_originals_and_disables(self):
        self.combined_fixture(enabled=True)

        def lose_resume_ack(docker, binding, cohort):
            identity = cohort[runtime.SERVICES[0]]["generation"][0]
            docker.call(["start", identity])
            raise runtime.RuntimeErrorCode("FIXTURE_ACK_LOST")

        with mock.patch.object(runtime, "resume_after_backup", side_effect=lose_resume_ack):
            result = self.run_tool(self.combined_argv("--apply", "--nightly"))
        self.assertNotEqual(result[0], 0, result)
        self.assertEqual((self.daily_enabled, self.daily_epoch), (False, 5))
        self.assertFalse(self.progress().get("nightly_completed", False))
        self.assertEqual(self.steps()[tool.DAILY_RESUME_STEP], "failed")
        self.assertTrue(all(not self.docker.containers[key]["State"]["Running"]
                            for key in ("2" * 64, "3" * 64, BETA_ID)))

    def test_combined_nightly_isolated_verify_failure_stays_manual_and_off(self):
        self.combined_fixture(enabled=True)
        self.fail_verify = True
        result = self.run_tool(self.combined_argv("--apply", "--nightly"))
        self.assertNotEqual(result[0], 0, result)
        self.assertEqual((self.daily_enabled, self.daily_epoch), (False, 5))
        self.assertEqual(self.steps()["verify"], "failed")
        self.assertNotIn(("wrapper", "--simulation-resume-daily-api"), self.events)

    def test_combined_nightly_changed_epoch_refuses_to_resume_daily(self):
        self.combined_fixture(enabled=True)
        self.drift_after_verify = lambda: setattr(self, "daily_epoch", 6)
        with mock.patch.object(runtime, "resume_after_backup") as resume:
            result = self.run_tool(self.combined_argv("--apply", "--nightly"))
        self.assertNotEqual(result[0], 0, result)
        resume.assert_not_called()
        self.assertEqual((self.daily_enabled, self.daily_epoch), (False, 7))
        self.assertFalse(self.progress().get("nightly_completed", False))

    def test_actual_nightly_api_guard_requires_protected_stopped_daily_mode_and_epoch(self):
        self.combined_fixture(enabled=True)
        binding = self.daily_binding
        context = ops.Context(self.host.release, APP, self.host.images[APP], self.daily_values, [])
        operation = ops.Operations(self.docker, self.daily_database, context, context, binding,
                                   self.root, "daily", lock_fd=7)
        config = {"first_auto_session": CONFIG_DATE}
        paper = mock.Mock()
        paper.inspect_presence.return_value = "LEGACY_EMPTY"
        with mock.patch.object(ops, "read_context", return_value=(self.daily_values, [])), \
             mock.patch.object(ops, "load_local", return_value=paper), \
             mock.patch.object(ops, "activation_binding", return_value=(binding, config)), \
             mock.patch.object(ops.install, "check_state"):
            with self.assertRaisesRegex(runtime.RuntimeErrorCode, "DAILY_RUNTIME_STATE"):
                ops.guard_nightly_api(operation, (True, 4))
            runtime.drain(self.docker, binding, runtime.verify_running(self.docker, binding))
            self.assertEqual(ops.guard_nightly_api(operation, (True, 4)), "NIGHTLY_WRITERS_QUIESCENT")
            with self.assertRaisesRegex(runtime.RuntimeErrorCode, "DAILY_BACKUP_NIGHTLY_BINDING"):
                ops.guard_nightly_api(operation, (True, 5))
            paper.inspect_presence.return_value = "REQUIRED"
            with self.assertRaisesRegex(runtime.RuntimeErrorCode, "DAILY_BACKUP_NIGHTLY_BINDING"):
                ops.guard_nightly_api(operation, (True, 4))
        self.assertEqual((self.daily_enabled, self.daily_epoch), (True, 4))
        self.daily_database.control.assert_not_called()

    def test_combined_nightly_prepare_rejects_unhealthy_daily_before_mutation(self):
        self.combined_fixture(enabled=True)
        self.docker.containers["2" * 64]["State"]["Health"]["Status"] = "unhealthy"
        result = self.run_tool(self.combined_argv("--apply", "--nightly"))
        self.assertNotEqual(result[0], 0, result)
        self.assertTrue(self.daily_enabled)
        self.assertEqual(self.progress()["steps"], {})
        self.assertEqual(self.mutations(), [])

    def test_nightly_validates_preflight_and_durable_intent_before_pause(self):
        result = self.run_tool(self.argv("--preflight", "--nightly"))
        self.assertEqual(result[0], 0, result)
        self.assertIn("NIGHTLY_COHORT simulation=running owner_v2=running\n", result[1])
        self.assertRegex(result[1], r"NIGHTLY_UNTOUCHED_SHA256 sha256:[0-9a-f]{64}\n")
        self.assertIsNone(self.progress())
        self.assertEqual(self.mutations(), [])
        result = self.run_tool(self.argv("--apply", "--nightly", "--step", "all"))
        self.assertEqual(result[0], 0, result)
        self.assertTrue(self.progress()["nightly_completed"])
        mutations = self.mutations()
        verify = mutations.index(("pinned", "inactive-simulation-verify", "apply"))
        starts = [mutations.index(("wrapper", tool.NIGHTLY_API_ONLY)),
                  mutations.index(("wrapper", "--simulation-start")),
                  mutations.index(("docker", "start"))]
        self.assertTrue(all(start > verify for start in starts))
        self.assertNotIn(("wrapper", "--simulation-release-start"), mutations)
        self.assertEqual(self.progress()["nightly"]["services"][tool.API]["generation"][2], STARTED)
        self.assertNoValues()

    def test_nightly_never_starts_absent_worker_or_stopped_v2(self):
        self.worker_captured = False
        self.docker.containers.pop(self.services[tool.SIMULATION_RUNNER])
        self.stop(tool.V2)
        result = self.run_tool(self.argv("--apply", "--nightly"))
        self.assertEqual(result[0], 0, result)
        self.assertIsNone(self.progress()["nightly"]["services"][tool.SIMULATION_RUNNER])
        self.assertNotIn(("wrapper", "--simulation-start"), self.mutations())
        self.assertFalse(self.docker.containers[self.services[tool.V2]]["State"]["Running"])

    def test_nightly_blocks_stopped_simulation_absent_api_and_wider_wrapper_before_downtime(self):
        worker = copy.deepcopy(self.docker.containers[self.services[tool.SIMULATION_RUNNER]])
        self.stop(tool.SIMULATION_RUNNER)
        self.assertFailed(self.run_tool(self.argv("--preflight", "--nightly")), tool.NIGHTLY_ERROR)
        self.docker.containers[self.services[tool.SIMULATION_RUNNER]] = worker
        api = self.docker.containers.pop(self.services[tool.API])
        self.assertFailed(self.run_tool(self.argv("--preflight", "--nightly")), tool.NIGHTLY_ERROR)
        self.docker.containers[self.services[tool.API]] = api
        self.bad_targets = True
        self.assertFailed(self.run_tool(self.argv("--apply", "--nightly")), "PREUPGRADE_NIGHTLY_TARGETS")
        self.assertEqual(self.mutations(), [])
        self.assertEqual(self.progress()["steps"], {})
        self.assertEqual(self.progress()["nightly"], {})

    def test_failed_repeated_preflight_keeps_durable_no_effect_record(self):
        self.children.overrides[("inactive-simulation-backup", "preflight")] = lambda: (
            1, b"", b"PREACTIVATION_BACKUP: FAILED PREACTIVATION_DATABASE\n")
        self.assertFailed(self.run_tool(self.argv("--apply", "--nightly")), "PREUPGRADE_TOOL")
        record = self.progress()
        self.assertEqual((record["steps"], record["nightly"]), ({}, {}))
        self.assertEqual(self.mutations(), [])
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "recover")),
                         (0, "PREUPGRADE_BACKUP: RECOVERED\n", ""))
        self.assertEqual(self.progress(), record)
        self.assertEqual(self.mutations(), [])

    def check_recovery_for_backup_phase(self, phase):
        self.fail_backup_at = phase
        self.assertFailed(self.run_tool(self.argv("--apply", "--nightly")), "PREUPGRADE_BACKUP_STEP")
        self.assertEqual(self.steps()["backup"], "failed")
        self.assertEqual(self.count(("wrapper", "--simulation-drain")), 0)
        code, out, err = self.run_tool(self.argv("--apply", "--step", "recover"))
        self.assertEqual((code, out, err), (0, "PREUPGRADE_BACKUP: RECOVERED\n", ""))
        self.assertEqual(self.steps()["recover-handoff"], "done")
        self.assertTrue(self.progress()["nightly_recovered"])
        self.assertEqual(self.count(("wrapper", "--simulation-drain")), 1)
        self.assertEqual(self.count(("wrapper", tool.NIGHTLY_API_ONLY)), 1)
        self.assertNotIn(("wrapper", "--simulation-release-start"), self.mutations())

    def test_attended_recovery_finishes_prepared_handoff_using_guarded_drain(self):
        self.check_recovery_for_backup_phase("prepared")

    def test_attended_recovery_retries_fencing_when_nothing_has_changed(self):
        self.check_recovery_for_backup_phase("fencing")

    def test_attended_recovery_refuses_partial_fencing_without_service_changes(self):
        self.fail_backup_at = "partial-fencing"
        self.assertFailed(self.run_tool(self.argv("--apply", "--nightly")), "PREUPGRADE_BACKUP_STEP")
        before = list(self.mutations())
        pending = Path(self.root, tool.HANDOFF_PENDING)
        evidence = pending.read_bytes()
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "recover")), tool.NIGHTLY_FENCING_UNCERTAIN)
        self.assertEqual(self.mutations(), before)
        self.assertEqual(pending.read_bytes(), evidence)
        self.assertEqual(self.progress()["steps"]["recover-handoff"]["code"], tool.NIGHTLY_FENCING_UNCERTAIN)

    def test_attended_recovery_uses_completed_api_handoff_even_if_step_was_interrupted(self):
        self.interrupt_api_resume = True
        self.assertFailed(self.run_tool(self.argv("--apply", "--nightly")), "PREUPGRADE_INTERRUPTED")
        self.assertEqual(self.steps()["resume-release"], "started")
        self.assertEqual(self.run_tool(self.argv("--apply", "--step", "recover")),
                         (0, "PREUPGRADE_BACKUP: RECOVERED\n", ""))
        self.assertEqual(self.count(("wrapper", tool.NIGHTLY_API_ONLY)), 1)

    def test_attended_recovery_proves_completed_worker_start_without_starting_it_again(self):
        self.interrupt_simulation_resume = True
        self.assertFailed(self.run_tool(self.argv("--apply", "--nightly")), "PREUPGRADE_INTERRUPTED")
        self.assertEqual(self.steps()["resume-simulation"], "started")
        self.assertFalse(Path(self.root, tool.HANDOFF_PENDING).exists())

        def verify_worker(sequencer, generation):
            self.observed_worker = generation
            self.assertEqual(generation, self.generation(tool.SIMULATION_RUNNER))
            if not self.worker_policy_ok:
                raise tool.ToolError(tool.NIGHTLY_ERROR)

        self.docker.start(BETA_ID)
        self.children.units[tool.TIMER][1] = "active"
        previous = list(self.mutations())
        with mock.patch.object(tool.Sequencer, "verify_nightly_worker", verify_worker):
            self.worker_policy_ok = False
            self.assertFailed(self.run_tool(self.argv("--apply", "--step", "recover")), tool.NIGHTLY_ERROR)
            self.assertEqual(self.mutations(), previous)
            self.worker_policy_ok = True
            self.assertEqual(self.run_tool(self.argv("--apply", "--step", "recover")),
                             (0, "PREUPGRADE_BACKUP: RECOVERED\n", ""))
        self.assertEqual(self.mutations(), previous)
        self.assertEqual(self.steps()["resume-simulation"], "started")
        self.assertEqual(self.steps()["observe-simulation-resume"], "done")
        self.assertEqual(self.observed_worker, self.generation(tool.SIMULATION_RUNNER))

    def test_verify_failure_never_resumes_automatically_and_manual_recovery_is_narrow(self):
        self.fail_verify = True
        self.assertFailed(self.run_tool(self.argv("--apply", "--nightly")), "PREUPGRADE_VERIFY_STEP")
        self.assertEqual(self.steps()["verify"], "failed")
        self.assertFalse(any(row[0] == "wrapper" for row in self.mutations()))
        before = list(self.mutations())
        self.assertFailed(self.run_tool(self.argv("--apply", "--nightly")), "PREUPGRADE_PROGRESS")
        self.assertFailed(self.run_tool(self.argv("--apply", "--step", "resume")), "PREUPGRADE_PROGRESS")
        self.assertEqual(self.mutations(), before)
        result = self.run_tool(self.argv("--apply", "--step", "recover"))
        self.assertEqual(result[0:2], (0, "PREUPGRADE_BACKUP: RECOVERED\n"), result)
        self.assertEqual(self.steps()["verify"], "failed")
        self.assertTrue(self.progress()["nightly_recovered"])
        self.assertNotIn(("wrapper", "--simulation-release-start"), self.mutations())

    def test_drift_before_resume_leaves_services_stopped(self):
        self.drift_after_verify = lambda: self.docker.start(self.services[tool.V2])
        self.assertFailed(self.run_tool(self.argv("--apply", "--nightly")), tool.NIGHTLY_ERROR)
        self.assertEqual(self.steps()["restore-beta-policy"], "failed")
        self.assertFalse(any(row[0] == "wrapper" for row in self.mutations()))
        self.assertFalse(self.docker.containers[self.services[tool.API]]["State"]["Running"])

    def test_interruption_preserves_started_step_and_never_retries(self):
        self.interrupt_verify = True
        self.assertFailed(self.run_tool(self.argv("--apply", "--nightly")), "PREUPGRADE_INTERRUPTED")
        self.assertEqual(self.steps()["verify"], "started")
        observed = self.mutations()
        self.assertFailed(self.run_tool(self.argv("--apply", "--nightly")), "PREUPGRADE_PROGRESS")
        self.assertEqual(self.mutations(), observed)

    def test_only_nightly_apply_all_or_read_only_preflight_are_accepted(self):
        for args in (("--plan", "--nightly"), ("--apply", "--nightly", "--step", "recover"),
                     ("--apply", "--nightly", "--nightly")):
            with self.subTest(args=args):
                self.assertFailed(self.run_tool(self.argv(*args)), "PREUPGRADE_ARGUMENTS")
        self.assertEqual(self.mutations(), [])

    def test_pinned_controller_rejects_incompatible_application_before_downtime(self):
        self.pinned_entry.write_text(
            "def inactive_simulation_tool(context, verifier_root):\n"
            "    raise ValueError('PREACTIVATION_TOOL_DEPENDENCY')\n")
        result = self.run_tool(self.argv("--apply", "--nightly"))
        self.assertFailed(result, "PREUPGRADE_TOOL")
        self.assertIn("PREUPGRADE_BACKUP: CAUSE PREACTIVATION_TOOL_DEPENDENCY\n", result[2])
        self.assertEqual(self.mutations(), [])

    def test_rejects_previous_maintenance_controller_before_downtime(self):
        self.old_maintenance_plan = True
        self.assertFailed(self.run_tool(self.argv("--apply", "--nightly")), "PREUPGRADE_TOOL")
        self.assertEqual(self.mutations(), [])


class HelperTests(Base):
    def test_trusted_chain_rejects_writable_or_symlinked_components(self):
        rg = tool.rg
        relative = "releases/" + APP + "/" + tool.WRAPPER
        self.assertTrue(rg.trusted_chain(self.root, relative, os.geteuid(), "X_CODE").endswith(tool.WRAPPER))
        os.chmod(self.root + "/releases", 0o775)
        with self.assertRaises(rg.ToolError):
            rg.trusted_chain(self.root, relative, os.geteuid(), "X_CODE")
        os.chmod(self.root + "/releases", 0o755)
        os.symlink(self.root + "/releases/" + APP, self.root + "/releases/link")
        with self.assertRaises(rg.ToolError):
            rg.trusted_chain(self.root, "releases/link/" + tool.WRAPPER, os.geteuid(), "X_CODE")
        with self.assertRaises(rg.ToolError):
            rg.trusted_chain(self.root, relative, os.geteuid() + 1, "X_CODE")

    def test_progress_record_is_canonical_and_private(self):
        rg = tool.rg
        progress = rg.Progress(self.state, "sample", os.geteuid(), "X_CODE")
        record = progress.create({"b": 1, "a": [1, 2]})
        self.assertEqual(progress.read(), record)
        with self.assertRaises(rg.ToolError):
            progress.create(record)
        newer = progress.replace(record, dict(record, c=True))
        with self.assertRaises(rg.ToolError):
            progress.replace(record, dict(record, d=True))
        self.assertEqual(progress.read(), newer)
        os.chmod(progress.path, 0o644)
        with self.assertRaises(rg.ToolError):
            progress.read()

    def test_restart_policy_allowlist(self):
        rg = tool.rg
        for policy in ({"Name": "on-failure", "MaximumRetryCount": 3}, {"Name": "unless-stopped"},
                       {"Name": "always", "MaximumRetryCount": 1}, {"Name": "", "MaximumRetryCount": 0}):
            with self.assertRaises(rg.ToolError):
                rg.restart_policy({"HostConfig": {"RestartPolicy": policy}}, "X_CODE")
        self.assertEqual(rg.restart_argument({"Name": "unless-stopped", "MaximumRetryCount": 0}, "X"),
                         "--restart=unless-stopped")


if __name__ == "__main__":
    unittest.main()
