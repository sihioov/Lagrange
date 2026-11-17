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
import importlib.util
import io
import json
import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path

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
            value["State"].update(Running=False, Status="exited", ExitCode=self.exit_code,
                                  FinishedAt="2026-11-17T00:00:00.5Z")
            value["State"]["Health"]["Status"] = "unhealthy"
            return str(self.exit_code) + "\n"
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
            if flag in self.overrides:
                return self.overrides.pop(flag)()
            if flag == "--simulation-release-start":
                self.case.write_pending(tool.HANDOFF_PENDING, "api-started")
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

    def tearDown(self):
        self.temp.cleanup()

    def write_pending(self, name, phase):
        path = os.path.join(self.root, name)
        data = json.dumps({"phase": phase, "source_commit": APP, "target_commit": APP},
                          sort_keys=True).encode()
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
