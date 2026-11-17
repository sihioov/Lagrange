#!/usr/bin/env python3
"""Unprivileged tests for build-reader-drain.py.

The real checked-in ``owner-beta-daily-runtime.owned`` and ``Ownership`` bind the
reader generation; Docker is a fake daemon.  State directory and progress file are
real files in a temporary tree owned by the test user.
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


tool = _load("build_reader_drain_under_test", HERE / "build-reader-drain.py")
ops = _load("reader_drain_test_daily_operations", HERE / "lib" / "owner-beta-daily-operations.py")
runtime = ops.runtime

APP = "a1" * 20
READER = "d4" * 20
OTHER = "c3" * 20
PROJECT = "lagrange-station"
IMAGE = "sha256:" + "d" * 64
READER_ID = "7" * 64
STARTED = "2026-11-16T05:06:07.987654321Z"
CREATED = "2026-11-16T05:00:00.000000001Z"


def reader_container(identity=READER_ID, revision=READER, name=tool.CONTAINER_NAME):
    return {
        "Id": identity, "Name": name, "Created": CREATED, "RestartCount": 0, "Image": IMAGE,
        "State": {"Running": True, "Restarting": False, "Paused": False, "OOMKilled": False,
                  "Dead": False, "Error": "", "ExitCode": 0, "Status": "running",
                  "StartedAt": STARTED, "FinishedAt": "0001-01-01T00:00:00Z",
                  "Health": {"Status": "healthy"}},
        "Config": {"Labels": {"org.opencontainers.image.revision": revision,
                              "com.docker.compose.project": PROJECT,
                              "com.docker.compose.service": tool.SERVICE,
                              "com.docker.compose.oneoff": "False"}},
        "HostConfig": {"RestartPolicy": {"Name": "unless-stopped", "MaximumRetryCount": 0}},
    }


class FakeDocker:
    def __init__(self):
        self.containers = {READER_ID: reader_container()}
        self.calls = []
        self.exit_code = 0
        self.wait_fails = False
        self.update_fails = False
        self.health_after_start = "healthy"
        self.starts = 0

    def _fail(self):
        raise runtime.RuntimeErrorCode("DAILY_RUNTIME_DOCKER")

    def object(self, kind, identity, timeout=15):
        if kind != "container" or identity not in self.containers:
            self._fail()
        return copy.deepcopy(self.containers[identity])

    def call(self, args, timeout=15):
        args = list(args)
        if args[0] == "ps":
            filters = [args[i + 1].removeprefix("label=").split("=", 1)
                       for i, t in enumerate(args) if t == "--filter"]
            return "".join(i + "\n" for i, v in self.containers.items()
                           if all(v["Config"]["Labels"].get(k) == val for k, val in filters))
        self.calls.append(args[:-1])
        identity = args[-1]
        if identity not in self.containers:
            self._fail()
        state = self.containers[identity]["State"]
        if args[0] == "update":
            if self.update_fails:
                self._fail()
            self.containers[identity]["HostConfig"]["RestartPolicy"] = {
                "Name": args[1].split("=", 1)[1], "MaximumRetryCount": 0}
        elif args[0] == "kill":
            pass
        elif args[0] == "wait":
            self.assert_timeout = timeout
            if self.wait_fails:
                self._fail()
            state.update(Running=False, Status="exited", ExitCode=self.exit_code,
                         FinishedAt="2026-11-17T00:00:00.5Z")
            state["Health"]["Status"] = "unhealthy"
            return str(self.exit_code) + "\n"
        elif args[0] == "start":
            self.starts += 1
            state.update(Running=True, Status="running", ExitCode=0,
                         StartedAt="2026-11-17T02:00:0%dZ" % self.starts)
            state["Health"]["Status"] = self.health_after_start
        else:
            self._fail()
        return identity + "\n"


class FakeHost:
    def __init__(self, docker):
        self.docker = docker
        self.current_ok = True
        self.images = {READER: {tool.SERVICE: IMAGE}, APP: {tool.SERVICE: "sha256:" + "9" * 64}}
        self.project_name = PROJECT

    def validate_current(self):
        if not self.current_ok:
            raise ValueError("POLICY_MISMATCH")

    def manifest_images(self, commit):
        if commit not in self.images:
            raise runtime.RuntimeErrorCode("DAILY_OPERATION_MANIFEST")
        return dict(self.images[commit])

    def project(self):
        return self.project_name

    def ownership(self, project, commit, image):
        return ops.Ownership(project, commit, image)

    def owned(self, value, own, service, identity):
        return runtime.owned(value, own, service, identity)


class Clock:
    def __init__(self):
        self.now = 50.0

    def __call__(self):
        self.now += 1.0
        return self.now

    def sleep(self, seconds):
        self.now += seconds


class Base(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.state = os.path.join(self.temp.name, "state")
        os.mkdir(self.state, 0o700)
        os.chmod(self.state, 0o700)
        self.docker = FakeDocker()
        self.host = FakeHost(self.docker)
        self.clock = Clock()
        self.outputs = []
        self.factory_calls = 0

    def tearDown(self):
        self.temp.cleanup()

    def factory(self, args):
        self.factory_calls += 1
        return self.host

    def argv(self, mode="--apply", reader=READER):
        return [mode, "--install-root", "/opt/lagrange", "--application-commit", APP,
                "--reader-from-commit", reader, "--state-dir", self.state]

    def run_tool(self, argv):
        out, err = io.StringIO(), io.StringIO()
        code = tool.main(argv, host_factory=self.factory, uid=os.geteuid(), out=out, err=err,
                         clock=self.clock, sleep=self.clock.sleep)
        self.outputs.append(out.getvalue() + err.getvalue())
        return code, out.getvalue(), err.getvalue()

    def progress(self):
        path = os.path.join(self.state, tool.PROGRESS_NAME + ".json")
        return json.loads(Path(path).read_text()) if os.path.exists(path) else None

    def assertFailed(self, result, code):
        self.assertEqual(result[0], 1, result)
        self.assertIn("READER_DRAIN: FAILED " + code + "\n", result[2])

    def assertNoValues(self):
        text = "".join(self.outputs)
        for value in (APP, READER, READER_ID, IMAGE, STARTED, CREATED, self.state, "/opt/lagrange"):
            self.assertNotIn(value, text)

    def verbs(self):
        return [c[0] for c in self.docker.calls]


class ArgumentTests(Base):
    def test_grammar(self):
        good = self.argv()
        cases = [[], good[1:], ["--apply", "--restore"] + good[1:], good + ["--state-dir", self.state],
                 good + ["--reader-from-commit=" + READER], [a if a != APP else APP.upper() for a in good],
                 [a if a != READER else "0" * 40 for a in good],
                 [a if a != self.state else "state" for a in good],
                 [a if a != "/opt/lagrange" else "/opt/lagrange/../x" for a in good],
                 good + ["--force"], [a if a != "--state-dir" else "--state-d" for a in good]]
        for argv in cases:
            with self.subTest(argv=len(argv)):
                self.assertFailed(self.run_tool(argv), "READER_DRAIN_ARGUMENTS")
        self.assertEqual(self.factory_calls, 0)
        self.assertEqual(self.docker.calls, [])
        self.assertNoValues()

    def test_plan_touches_nothing(self):
        code, out, _ = self.run_tool(self.argv("--plan"))
        self.assertEqual(code, 0)
        self.assertTrue(out.startswith("READER_DRAIN: PLAN\n"))
        self.assertEqual(self.factory_calls, 0)
        self.assertIsNone(self.progress())


class DrainTests(Base):
    def test_preflight_is_read_only(self):
        self.assertEqual(self.run_tool(self.argv("--preflight"))[0:2], (0, "READER_DRAIN: PREFLIGHT\n"))
        self.assertEqual(self.docker.calls, [])
        self.assertIsNone(self.progress())

    def test_drain_to_exit_zero_with_restart_no(self):
        self.assertEqual(self.run_tool(self.argv())[0:3], (0, "READER_DRAIN: DRAINED\n", ""))
        self.assertEqual(self.docker.calls, [["update", "--restart=no"], ["kill", "--signal=TERM"], ["wait"]])
        self.assertGreaterEqual(self.docker.assert_timeout, 16 * 60)
        value = self.docker.containers[READER_ID]
        self.assertEqual((value["State"]["Status"], value["State"]["ExitCode"], value["HostConfig"]["RestartPolicy"]),
                         ("exited", 0, {"Name": "no", "MaximumRetryCount": 0}))
        record = self.progress()
        self.assertEqual((record["phase"], record["signalled"]), ("drained", True))
        self.assertEqual(record["reader"], {"identity": READER_ID, "created": CREATED, "started_at": STARTED,
                                            "restart_count": 0, "image": IMAGE,
                                            "restart_policy": {"Name": "unless-stopped", "MaximumRetryCount": 0}})
        info = os.stat(os.path.join(self.state, tool.PROGRESS_NAME + ".json"))
        self.assertEqual(stat.S_IMODE(info.st_mode), 0o600)
        # A drained (not yet restored) cycle blocks a new one.
        self.assertFailed(self.run_tool(self.argv()), "READER_DRAIN_PROGRESS")
        self.assertFailed(self.run_tool(self.argv("--preflight")), "READER_DRAIN_PROGRESS")
        self.assertEqual(self.verbs().count("kill"), 1)
        self.assertNoValues()

    def test_exit_nonzero_fails_closed_without_retry(self):
        self.docker.exit_code = 1
        self.assertFailed(self.run_tool(self.argv()), "READER_DRAIN_EXIT")
        record = self.progress()
        self.assertEqual((record["phase"], record["code"]), ("drain-failed", "READER_DRAIN_EXIT"))
        self.docker.exit_code = 0
        self.assertFailed(self.run_tool(self.argv()), "READER_DRAIN_PROGRESS")
        self.assertEqual(self.verbs(), ["update", "kill", "wait"])
        # The precedent's recovery: explicit restore of that exact generation.
        self.assertEqual(self.run_tool(self.argv("--restore"))[0:2], (0, "READER_DRAIN: RESTORED\n"))
        self.assertEqual(self.verbs(), ["update", "kill", "wait", "update", "start"])
        self.assertEqual(self.docker.calls[3], ["update", "--restart=unless-stopped"])

    def test_wait_timeout_never_resignals_and_blocks_restore_while_running(self):
        self.docker.wait_fails = True
        self.assertFailed(self.run_tool(self.argv()), "READER_DRAIN_EXIT")
        self.assertEqual(self.progress()["phase"], "drain-failed")
        self.assertFailed(self.run_tool(self.argv("--restore")), "READER_RESTORE_STATE")
        self.assertEqual(self.verbs(), ["update", "kill", "wait"])
        # After it exits on its own the same generation can be restored.
        state = self.docker.containers[READER_ID]["State"]
        state.update(Running=False, Status="exited", ExitCode=0)
        self.assertEqual(self.run_tool(self.argv("--restore"))[0], 0, self.outputs)
        self.assertEqual(self.verbs().count("kill"), 1)

    def test_update_failure_before_signal_restores_policy_only(self):
        self.docker.update_fails = True
        self.assertFailed(self.run_tool(self.argv()), "READER_DRAIN_POLICY")
        self.assertFalse(self.progress()["signalled"])
        self.assertNotIn("kill", self.verbs())
        self.docker.update_fails = False
        self.assertEqual(self.run_tool(self.argv("--restore"))[0:2], (0, "READER_DRAIN: RESTORED\n"))
        self.assertNotIn("start", self.verbs())
        self.assertEqual(self.docker.containers[READER_ID]["HostConfig"]["RestartPolicy"]["Name"], "unless-stopped")

    def test_ownership_and_state_preconditions(self):
        cases = {
            "READER_DRAIN_OWNER": [
                lambda: self.docker.containers[READER_ID]["Config"]["Labels"].update(
                    {"org.opencontainers.image.revision": OTHER}),
                lambda: self.docker.containers[READER_ID].update(Image="sha256:" + "9" * 64),
                lambda: self.docker.containers[READER_ID].update(Name="/other-name"),
                lambda: self.docker.containers.update({"8" * 64: reader_container(identity="8" * 64)}),
                lambda: self.docker.containers.clear(),
                lambda: self.host.images.pop(READER),
                lambda: setattr(self.host, "project_name", "other-project"),
            ],
            "READER_DRAIN_STATE": [
                lambda: self.docker.containers[READER_ID]["State"]["Health"].update(Status="starting"),
                lambda: self.docker.containers[READER_ID]["State"].update(Running=False, Status="exited"),
                lambda: self.docker.containers[READER_ID]["State"].update(Restarting=True),
            ],
            "READER_DRAIN_POLICY": [
                lambda: self.docker.containers[READER_ID]["HostConfig"].update(
                    RestartPolicy={"Name": "on-failure", "MaximumRetryCount": 5}),
            ],
            "READER_DRAIN_CURRENT": [lambda: setattr(self.host, "current_ok", False)],
        }
        for code, changes in cases.items():
            for index, change in enumerate(changes):
                with self.subTest(code=code, index=index):
                    self.tearDown()
                    self.setUp()
                    change()
                    self.assertFailed(self.run_tool(self.argv("--preflight")), code)
                    self.assertFailed(self.run_tool(self.argv()), code)
                    self.assertEqual(self.docker.calls, [])
                    self.assertIsNone(self.progress())

    def test_state_directory_must_be_private(self):
        os.chmod(self.state, 0o755)
        self.assertFailed(self.run_tool(self.argv()), "READER_DRAIN_STATE_DIR")
        self.assertEqual(self.docker.calls, [])


class RestoreTests(Base):
    def drained(self):
        self.assertEqual(self.run_tool(self.argv())[0], 0, self.outputs)
        self.docker.calls.clear()

    def test_restore_exact_generation(self):
        self.drained()
        self.assertEqual(self.run_tool(self.argv("--restore"))[0:3], (0, "READER_DRAIN: RESTORED\n", ""))
        self.assertEqual(self.docker.calls, [["update", "--restart=unless-stopped"], ["start"]])
        value = self.docker.containers[READER_ID]
        self.assertTrue(value["State"]["Running"])
        self.assertEqual(value["State"]["Health"]["Status"], "healthy")
        self.assertEqual(self.progress()["phase"], "restored")
        self.assertFailed(self.run_tool(self.argv("--restore")), "READER_RESTORE_PROGRESS")
        self.assertEqual(self.docker.starts, 1)
        self.assertNoValues()

    def test_restore_accepts_restart_count_reset_by_manual_start(self):
        self.docker.containers[READER_ID]["RestartCount"] = 3
        self.drained()
        original = FakeDocker.call

        def call(docker, args, timeout=15):
            result = original(docker, args, timeout)
            if args[0] == "start":
                docker.containers[READER_ID]["RestartCount"] = 0
            return result
        self.docker.call = call.__get__(self.docker)
        self.assertEqual(self.run_tool(self.argv("--restore"))[0:2], (0, "READER_DRAIN: RESTORED\n"))

    def test_restore_rejects_other_restart_count_change(self):
        self.drained()
        original = FakeDocker.call

        def call(docker, args, timeout=15):
            result = original(docker, args, timeout)
            if args[0] == "start":
                docker.containers[READER_ID]["RestartCount"] = 4
            return result
        self.docker.call = call.__get__(self.docker)
        self.assertFailed(self.run_tool(self.argv("--restore")), "READER_RESTORE_CHANGED")

    def test_second_cycle_after_restore_moves_the_record_aside(self):
        self.drained()
        self.assertEqual(self.run_tool(self.argv("--restore"))[0], 0, self.outputs)
        first = self.progress()
        self.assertEqual(self.run_tool(self.argv("--preflight"))[0:2], (0, "READER_DRAIN: PREFLIGHT\n"))
        self.assertEqual(self.run_tool(self.argv())[0:2], (0, "READER_DRAIN: DRAINED\n"))
        retired = [n for n in os.listdir(self.state) if ".retired-" in n]
        self.assertEqual(len(retired), 1)
        self.assertRegex(retired[0], r"^build-reader-drain\.retired-[0-9]{8}T[0-9]{6}Z\.json$")
        path = os.path.join(self.state, retired[0])
        self.assertEqual(json.loads(Path(path).read_text()), first)
        self.assertEqual(stat.S_IMODE(os.stat(path).st_mode), 0o600)
        second = self.progress()
        self.assertEqual(second["phase"], "drained")
        self.assertNotEqual(second["reader"]["started_at"], first["reader"]["started_at"])
        self.assertEqual(self.verbs(), ["update", "start", "update", "kill", "wait"])  # calls after drained()

    def test_retire_never_clobbers(self):
        self.drained()
        self.assertEqual(self.run_tool(self.argv("--restore"))[0], 0, self.outputs)
        progress = tool.rg.Progress(self.state, tool.PROGRESS_NAME, os.geteuid(), "READER_DRAIN_PROGRESS")
        record = progress.read()
        stamp = "20261117T000000Z"
        occupied = os.path.join(self.state, tool.PROGRESS_NAME + ".retired-" + stamp + ".json")
        Path(occupied).write_text("keep")
        with self.assertRaises(tool.rg.ToolError):
            progress.retire(record, stamp)
        self.assertEqual(Path(occupied).read_text(), "keep")
        self.assertEqual(progress.read(), record)

    def test_only_a_restored_cycle_can_be_replaced(self):
        self.docker.exit_code = 1
        self.assertFailed(self.run_tool(self.argv()), "READER_DRAIN_EXIT")
        for mode in ("--apply", "--preflight"):
            self.assertFailed(self.run_tool(self.argv(mode)), "READER_DRAIN_PROGRESS")
        self.assertFalse([n for n in os.listdir(self.state) if ".retired-" in n])
        self.assertEqual(self.verbs().count("kill"), 1)

    def test_restore_without_drain_record(self):
        self.assertFailed(self.run_tool(self.argv("--restore")), "READER_RESTORE_PROGRESS")
        self.assertEqual(self.docker.calls, [])

    def test_restore_bound_to_recorded_arguments(self):
        self.drained()
        self.host.images[OTHER] = {tool.SERVICE: IMAGE}
        self.assertFailed(self.run_tool(self.argv("--restore", reader=OTHER)), "READER_RESTORE_PROGRESS")
        self.assertEqual(self.docker.calls, [])

    def test_changed_generation_is_never_started(self):
        cases = [
            lambda v: v.update(Created="2026-11-17T00:00:00Z"),
            lambda v: v.update(RestartCount=2),
        ]
        for index, change in enumerate(cases):
            with self.subTest(index=index):
                self.tearDown()
                self.setUp()
                self.drained()
                change(self.docker.containers[READER_ID])
                self.assertFailed(self.run_tool(self.argv("--restore")), "READER_RESTORE_CHANGED")
                self.assertEqual(self.docker.calls, [])
        self.tearDown()
        self.setUp()
        self.drained()
        replacement = reader_container(identity="8" * 64)
        self.docker.containers = {"8" * 64: replacement}
        self.assertFailed(self.run_tool(self.argv("--restore")), "READER_RESTORE_CHANGED")
        self.assertEqual(self.docker.calls, [])

    def test_restore_health_is_bounded(self):
        self.drained()
        self.docker.health_after_start = "starting"
        self.assertFailed(self.run_tool(self.argv("--restore")), "READER_RESTORE_HEALTH")
        self.assertEqual(self.progress()["phase"], "restoring")
        self.assertEqual(self.docker.starts, 1)
        # A re-run observes the already started generation; it never starts it twice.
        self.docker.containers[READER_ID]["State"]["Health"]["Status"] = "healthy"
        self.assertEqual(self.run_tool(self.argv("--restore"))[0], 0, self.outputs)
        self.assertEqual(self.docker.starts, 1)
        self.assertEqual(self.progress()["phase"], "restored")

    def test_restore_rejects_unhealthy_start(self):
        self.drained()
        self.docker.health_after_start = "unhealthy"
        self.assertFailed(self.run_tool(self.argv("--restore")), "READER_RESTORE_HEALTH")


if __name__ == "__main__":
    unittest.main()
