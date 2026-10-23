#!/usr/bin/env python3
"""Offline daemon-stream fixtures; real pipe/reader processes, no Docker/base/provider."""
import contextlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import types
import unittest
from unittest import mock

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("lifecycle_driver_tested", ROOT / "scripts/ops/verify-owner-beta-daily-wholeflow.py")
driver = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = driver
spec.loader.exec_module(driver)
events = driver.lifecycle
NONCE = "lagrange-wholeflow-" + "a" * 32
IMAGE = "sha256:" + "b" * 64
SECRET = "DO_NOT_PERSIST_arbitrary_label_command_or_message"
POPEN = subprocess.Popen


def event(action, index=1, kind="container", **extra):
    role = "worker"
    attrs = {"name": f"{NONCE}-{role}-{index}", "image": IMAGE,
             events.LABEL: NONCE, events.SERVICE: role, "secret": SECRET, "command": SECRET}
    if kind == "network":
        attrs = {"name": NONCE + "-initial", "secret": SECRET}
    if action == "die":
        attrs["exitCode"] = "0"
    attrs.update(extra)
    return {"Type": kind, "Action": action, "scope": "local", "timeNano": 1000000000,
            "Actor": {"ID": f"{index:064x}", "Attributes": attrs}, "message": SECRET}


def line(value):
    return json.dumps(value).encode() + b"\n"


def objects(count=1):
    return [{"kind": "container", "id": f"{i:064x}", "name": f"{NONCE}-worker-{i}",
             "image": IMAGE, "role": "worker"} for i in range(1, count + 1)]


class LifecycleTests(unittest.TestCase):
    def setUp(self):
        self.output = tempfile.TemporaryFile()
        self.addCleanup(self.output.close)
        self.recorder = events.Recorder(NONCE, self.output.fileno(), 30)
        self.addCleanup(self.recorder.close)

    def launch(self, container=b"", network=b"", *, exit_immediately=False):
        def spawn(args, **kwargs):
            data = container if "type=container" in args else network
            # Real child and OS pipe exercise continuous reads, EOF and drain.
            fd, path = tempfile.mkstemp(prefix="wholeflow-event-fixture-")
            with os.fdopen(fd, "wb") as output:
                output.write(data)
            self.addCleanup(os.unlink, path)
            code = "import sys,time,pathlib;sys.stdout.buffer.write(pathlib.Path(sys.argv[1]).read_bytes());sys.stdout.flush();"
            code += "sys.stderr.write(sys.argv[2]);sys.stderr.flush();"
            if not exit_immediately:
                code += "time.sleep(60)"
            return POPEN([sys.executable, "-c", code, path, SECRET], **kwargs)
        with mock.patch.object(events.subprocess, "Popen", side_effect=spawn):
            self.recorder.start()

    def read_output(self):
        self.output.seek(0)
        return self.output.read()

    def test_continuous_stream_retains_more_than_historical_256_events(self):
        data = b"".join(line(event(a, i)) for i in range(1, 81) for a in ("create", "start", "die", "destroy"))
        self.launch(data)
        result = self.recorder.finish(objects(80))
        self.assertEqual(result["status"], "COMPLETE")
        self.assertEqual(result["events"], 320)
        self.assertTrue(result["processes_drained"])
        self.assertNotIn(SECRET.encode(), self.read_output())
        self.assertEqual(result["sha256"], events.hashlib.sha256(self.read_output()).hexdigest())

    def test_container_and_network_use_separate_narrow_filters(self):
        container, network = self.recorder.command("container"), self.recorder.command("network")
        self.assertIn("label=" + events.LABEL + "=" + NONCE, container)
        self.assertIn("network=" + NONCE + "-initial", network)
        self.assertIn("network=" + NONCE + "-recovered", network)
        self.assertFalse(any(s.startswith("label=") for s in network))
        for command in (container, network):
            self.assertIn("--until", command)
            self.assertIn("unix:///var/run/docker.sock", command)
            self.assertIn("scope=local", command)

    def test_started_clients_are_not_readiness(self):
        self.launch()
        with self.assertRaisesRegex(events.EvidenceError, "WAIT_TIMEOUT"):
            self.recorder.wait_for("container", "1" * 64, "create", timeout=0.03)
        self.assertEqual(self.read_output(), b"")

    def test_each_stream_create_is_received_and_durable(self):
        self.launch(line(event("create")), line(event("create", 99, "network")))
        self.recorder.wait_for("network", f"{99:064x}", "create")
        self.recorder.wait_for("container", f"{1:064x}", "create")
        self.assertEqual(len(self.read_output().splitlines()), 2)

    def test_stream_eof_is_failure_without_reconnect(self):
        self.launch(exit_immediately=True)
        with self.assertRaisesRegex(events.EvidenceError, "STREAM_LOST"):
            self.recorder.wait_for("container", "1" * 64, "create", timeout=2)

    def test_partial_line_is_never_persisted_or_treated_as_ready(self):
        self.launch(line(event("create"))[:-1], exit_immediately=True)
        with self.assertRaisesRegex(events.EvidenceError, "STREAM_LOST"):
            self.recorder.wait_for("container", f"{1:064x}", "create", timeout=2)
        self.assertEqual(self.read_output(), b"")

    def test_incomplete_original_lifecycle_remains_incomplete(self):
        self.launch(line(event("destroy")))
        result = self.recorder.finish(objects())
        self.assertEqual(result["status"], "INCOMPLETE")
        self.assertEqual(result["code"], "WHOLEFLOW_EVENTS_INCOMPLETE")
        self.assertTrue(result["processes_drained"])

    def test_missing_exit_code_and_duplicate_keys_are_rejected(self):
        bad = event("die")
        del bad["Actor"]["Attributes"]["exitCode"]
        for raw in (line(bad), b'{"Type":"container","Type":"network"}'):
            with self.assertRaises(events.EvidenceError):
                events.sanitize(raw, NONCE, "container")

    def test_unknown_foreign_or_free_form_identity_never_crosses_boundary(self):
        for attr in ({"name": SECRET}, {"image": SECRET}, {events.SERVICE: SECRET},
                     {events.LABEL: "foreign"}, {"exitCode": SECRET}):
            with self.subTest(attr=next(iter(attr))), self.assertRaises(events.EvidenceError) as raised:
                events.sanitize(line(event("die", **attr)), NONCE, "container")
            self.assertNotIn(SECRET, str(raised.exception))

    def test_exact_images_and_full_order_are_required(self):
        rows = [events.sanitize(line(event(a)), NONCE, "container") for a in ("create", "start", "die", "destroy")]
        events.validate(rows, objects())
        for bad in (rows + rows[:1], rows[1:], rows[:2] + rows[3:], list(reversed(rows))):
            with self.assertRaises(events.EvidenceError):
                events.validate(bad, objects())
        changed = objects()
        changed[0]["image"] = "sha256:" + "c" * 64
        with self.assertRaisesRegex(events.EvidenceError, "BINDING"):
            events.validate(rows, changed)

    def test_oom_cannot_be_accepted_as_complete(self):
        rows = [events.sanitize(line(event(a)), NONCE, "container") for a in ("create", "start", "oom", "die", "destroy")]
        with self.assertRaisesRegex(events.EvidenceError, "INCOMPLETE"):
            events.validate(rows, objects())

    def test_oversized_line_fails_without_persisting_raw_output(self):
        self.launch(b"x" * (events.MAX_LINE + 1))
        with self.assertRaisesRegex(events.EvidenceError, "LIMIT"):
            self.recorder.wait_for("container", f"{1:064x}", "create", timeout=2)
        self.assertEqual(self.read_output(), b"")

    def test_event_count_and_total_wire_bound_fail_closed(self):
        data = b"".join(line(event(a)) for a in ("create", "start", "die", "destroy"))
        with mock.patch.object(events, "MAX_EVENTS", 2):
            self.launch(data)
            with self.assertRaisesRegex(events.EvidenceError, "LIMIT"):
                self.recorder.wait_for("container", f"{1:064x}", "destroy", timeout=2)
        self.recorder.close()
        self.recorder = events.Recorder(NONCE, self.output.fileno(), 30)
        self.addCleanup(self.recorder.close)
        with mock.patch.object(events, "MAX_WIRE_BYTES", 1):
            self.launch(data)
            with self.assertRaisesRegex(events.EvidenceError, "LIMIT"):
                self.recorder.wait_for("container", f"{1:064x}", "destroy", timeout=2)

    def test_deadline_does_not_become_a_successful_stream_end(self):
        self.recorder.end = time.monotonic() - 1
        with self.assertRaisesRegex(events.EvidenceError, "DEADLINE"):
            self.recorder.check()

    def test_write_failure_is_typed_and_does_not_announce_readiness(self):
        with mock.patch.object(events.os, "fsync", side_effect=OSError(SECRET)):
            self.launch(line(event("create")))
            with self.assertRaisesRegex(events.EvidenceError, "READ_FAILED"):
                self.recorder.wait_for("container", f"{1:064x}", "create", timeout=2)
        self.assertEqual(self.recorder.rows, [])


class DriverEvidenceGateTests(unittest.TestCase):
    def test_missing_container_create_event_blocks_attached_start(self):
        flow = driver.WholeFlow(types.SimpleNamespace(timeout_seconds=1200, candidate="a" * 40))
        flow.events = mock.Mock()
        flow.events.wait_for.side_effect = events.EvidenceError("WHOLEFLOW_EVENTS_WAIT_TIMEOUT")
        tmpfs = {"/tmp": "rw,nosuid,nodev,noexec,size=64m,mode=1777",
                 "/run/owner-beta-health": "uid=10001,gid=10001,mode=0750,size=1m"}
        value = {"Id": "1" * 64, "Image": IMAGE, "Config": {"User": "10001:10001",
                 "Labels": {driver.LABEL: flow.nonce}}, "HostConfig": {"ReadonlyRootfs": True,
                 "Privileged": False, "NetworkMode": "none", "CapDrop": ["ALL"], "PortBindings": {},
                 "LogConfig": {"Type": "none"}, "Tmpfs": tmpfs}, "Mounts": []}
        with mock.patch.object(flow, "inspect", side_effect=[{"Config": {"Volumes": None}}, value]), \
             mock.patch.object(flow, "command", return_value=b"owned") as command, mock.patch.object(flow, "intent"):
            with self.assertRaisesRegex(events.EvidenceError, "WAIT_TIMEOUT"):
                flow.once("worker", IMAGE, 10001, "none", [], {}, "/bin/true")
            self.assertEqual(command.call_count, 1)
            self.assertEqual(command.call_args.args[0][0], "create")

    def test_network_create_event_precedes_first_process(self):
        flow = driver.WholeFlow(types.SimpleNamespace(timeout_seconds=1200, candidate="a" * 40))
        flow.events = mock.Mock()
        flow.events.wait_for.side_effect = events.EvidenceError("WHOLEFLOW_EVENTS_WAIT_TIMEOUT")
        flow.prepare = mock.Mock()
        flow.intent = mock.Mock()
        flow.command = mock.Mock()
        flow.inspect = mock.Mock(return_value={"Internal": True, "Labels": {driver.LABEL: flow.nonce}, "Id": "1" * 64})
        with self.assertRaisesRegex(events.EvidenceError, "WAIT_TIMEOUT"):
            flow.database("initial", migrate=True)
        self.assertEqual(len(flow.command.call_args_list), 1)
        self.assertEqual(flow.command.call_args.args[0][:2], ["network", "create"])

    def test_collector_failure_blocks_commands_but_allows_owned_cleanup(self):
        flow = driver.WholeFlow(types.SimpleNamespace(timeout_seconds=1200, candidate="a" * 40))
        flow.events = mock.Mock()
        flow.events.check.side_effect = events.EvidenceError("WHOLEFLOW_EVENTS_STREAM_LOST")
        with mock.patch.object(driver.subprocess, "run", return_value=types.SimpleNamespace(returncode=0, stdout=b"", stderr=b"")) as run:
            with self.assertRaisesRegex(events.EvidenceError, "STREAM_LOST"):
                flow.command(["start", "container"])
            run.assert_not_called()
            flow.command(["container", "rm", "container"], cleanup=True)
            run.assert_called_once()

    def test_main_requires_lifecycle_completion_even_after_functional_pass_and_cleanup(self):
        @contextlib.contextmanager
        def root_directory(path):
            fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
            try:
                yield fd
            finally:
                os.close(fd)
        for status in ("COMPLETE", "INCOMPLETE"):
            with self.subTest(status=status), tempfile.TemporaryDirectory() as folder:
                order = []
                def execute(flow):
                    flow.events = mock.Mock()
                    flow.events.threads = []
                    flow.events.finish.side_effect = lambda _: (order.append("finish") or {"status": status})
                with mock.patch.object(driver.WholeFlow, "execute", execute), \
                     mock.patch.object(driver.WholeFlow, "cleanup", lambda _: order.append("cleanup")), \
                     mock.patch.object(driver.release, "root_directory", root_directory):
                    result = driver.main(["--execute", "--candidate", "a" * 40, "--image-manifest", "/unused",
                                          "--approved-base", "/unused", "--receipt", folder + "/result.json"])
                self.assertEqual(result, 0 if status == "COMPLETE" else 1)
                self.assertEqual(order, ["cleanup", "finish"])
                receipt = json.loads(Path(folder, "result.json").read_text())
                self.assertEqual(receipt["result"], "PASS" if status == "COMPLETE" else "FAIL")
                self.assertEqual(Path(folder, "result.json.events.jsonl").stat().st_mode & 0o777, 0o600)


if __name__ == "__main__":
    unittest.main(verbosity=2)
