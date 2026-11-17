#!/usr/bin/env python3
"""Tests for the v1 drained-readers attestation generator.

The real bash validator runs unprivileged (it checks st_uid == geteuid()).
Docker is faked through the tool's injectable runner.
"""
from __future__ import annotations

import ast
import calendar
import contextlib
import datetime
import importlib.util
import io
import json
import os
import re
import stat
import subprocess
import tempfile
import time
import unittest
from pathlib import Path
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
TOOL_PATH = HERE / "build-drained-readers-attestation.py"
spec = importlib.util.spec_from_file_location("build_drained_readers_attestation", TOOL_PATH)
tool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tool)

HELPER = HERE / "lib/release-build-layout.sh"
BODY = HELPER.read_text().split("release_build_layout_gate() {", 1)[1].split(
    "<<'PY'\n", 1)[1].split("\nPY\n", 1)[0]
NAMES = {"parse_container", "docker_timestamp"}
FUNCTIONS = [n for n in ast.parse(BODY).body
             if isinstance(n, ast.FunctionDef) and n.name in NAMES]
assert {n.name for n in FUNCTIONS} == NAMES
GATE = {"calendar": calendar, "datetime": datetime, "json": json, "re": re}
exec(compile(ast.Module(body=FUNCTIONS, type_ignores=[]), str(HELPER), "exec"), GATE)

OWNER = "lagrange-station-owner-equity-v2-runner-1"
RESEARCH = "lagrange-station-research-worker-1"
COMMIT = "a" * 40


def line(name, **changes):
    item = dict(
        id=("2" if name == OWNER else "3") * 64, running="false", restarting="false",
        oom="false", health="absent", restarts="0", project="lagrange-station",
        image="sha256:" + ("5" if name == OWNER else "6") * 64,
        exit="2" if name == RESEARCH else "0", status="exited", paused="false",
        dead="false", started="2026-11-10T01:02:03.123456789Z",
        finished="2026-11-10T02:03:04.5Z")
    item.update(changes)
    order = ("id", "running", "restarting", "oom", "health", "restarts", "project", "image",
             "exit", "status", "paused", "dead", "started", "finished")
    return ("\t".join(item[key] for key in order) + "\n").encode()


class FakeDocker:
    def __init__(self, overrides=None, second=None):
        self.overrides = overrides or {}
        self.second = second or {}
        self.calls = []
        self.counts = {}

    def __call__(self, argv, **kwargs):
        self.calls.append(tuple(argv))
        name = argv[-1]
        count = self.counts[name] = self.counts.get(name, 0) + 1
        changes = dict(self.overrides.get(name, {}))
        if count > 1:
            changes.update(self.second.get(name, {}))
        return SimpleNamespace(returncode=0, stdout=line(name, **changes))


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="attestation-test-")
        self.addCleanup(self.tmp.cleanup)
        self.dir = os.path.realpath(self.tmp.name)
        os.chmod(self.dir, 0o700)
        self.output = os.path.join(self.dir, "attestation.json")

    def invoke(self, runner, mode="apply", lifetime=3600, fmt="v1", output=None):
        buffer = io.StringIO()
        try:
            tool.run(mode, COMMIT, output or self.output, lifetime, fmt, runner=runner, out=buffer)
        except tool.AttestationError as error:
            return error.code, buffer.getvalue()
        return None, buffer.getvalue()


class Acceptance(Base):
    def test_apply_passes_real_validator_and_matches_canonical_form(self):
        fake = FakeDocker()
        code, text = self.invoke(fake)
        self.assertIsNone(code)
        self.assertEqual(text, "DRAINED_READERS_ATTESTATION: APPLIED format=v1 readers=2\n")
        info = os.lstat(self.output)
        self.assertEqual(stat.S_IMODE(info.st_mode), 0o600)
        self.assertEqual(info.st_nlink, 1)
        self.assertEqual(os.listdir(self.dir), ["attestation.json"])
        raw = Path(self.output).read_bytes()
        value = json.loads(raw)
        self.assertEqual(raw, tool.canonical(value))
        result = subprocess.run(
            ["bash", "-c", 'source "$1"; rbl_drained_readers_attestation_binding "$2" "$3"',
             "x", str(HELPER), self.output, COMMIT], capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([c["container_name"] for c in value["containers"]], [OWNER, RESEARCH])
        self.assertEqual(value["format"], "lagrange-build-drained-readers-attestation-v1")
        for stamp in (value["observed_at_utc"], value["expires_at_utc"]):
            self.assertRegex(stamp, r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{9}Z$")
        # Docker's raw strings are copied verbatim, not normalized.
        research = value["containers"][1]
        self.assertEqual(research["finished_at_utc"], "2026-11-10T02:03:04.5Z")
        self.assertEqual(research["health_status"], "absent")
        self.assertEqual(research["exit_code"], 2)
        # two reads per reader, observe-only docker verb
        self.assertEqual(len(fake.calls), 4)
        self.assertTrue(all(call[:3] == ("docker", "inspect", "--type") for call in fake.calls))

    def test_inspect_template_is_the_gate_template_with_fourteen_fields(self):
        template = tool._template()
        self.assertEqual(template.count("printf"), 13)
        fake = FakeDocker()
        self.invoke(fake)
        self.assertEqual(fake.calls[0][fake.calls[0].index("--format") + 1], template)

    def test_gate_parse_container_accepts_the_attested_records(self):
        self.assertIsNone(self.invoke(FakeDocker())[0])
        value = json.loads(Path(self.output).read_bytes())
        observed_ns = GATE["docker_timestamp"](value["observed_at_utc"])
        control = {"binding": {"fields": value}, "project": value["project"],
                   "observed_ns": observed_ns,
                   "by_name": {c["container_name"]: c for c in value["containers"]}}
        for name in (OWNER, RESEARCH):
            parsed = GATE["parse_container"](line(name), name, None, control, time.time_ns())
            self.assertEqual(parsed["status"], "exited")
            self.assertEqual(parsed["exit_code"], 2 if name == RESEARCH else 0)

    def test_preflight_prints_only_summary_and_writes_nothing(self):
        code, text = self.invoke(FakeDocker(), mode="preflight")
        self.assertIsNone(code)
        self.assertEqual(text.splitlines(), [
            "DRAINED_READERS_ATTESTATION: PREFLIGHT format=v1 readers=2",
            "reader=%s exit_code=0 status=exited" % OWNER,
            "reader=%s exit_code=2 status=exited" % RESEARCH])
        self.assertNotIn("2222", text)
        self.assertEqual(os.listdir(self.dir), [])

    def test_plan_does_not_call_docker(self):
        fake = FakeDocker()
        code, text = self.invoke(fake, mode="plan")
        self.assertIsNone(code)
        self.assertEqual(text, "DRAINED_READERS_ATTESTATION: PLAN format=v1 readers=2\n")
        self.assertEqual(fake.calls, [])


class Negatives(Base):
    def assertFails(self, expected, runner, **kwargs):
        code, _ = self.invoke(runner, **kwargs)
        self.assertEqual(code, expected)
        self.assertEqual(os.listdir(self.dir), [])

    def test_changed_generation_between_reads(self):
        self.assertFails("ATTESTATION_READER_CHANGED",
                         FakeDocker(second={RESEARCH: {"id": "9" * 64}}))
        self.assertFails("ATTESTATION_READER_CHANGED",
                         FakeDocker(second={OWNER: {"finished": "2026-11-10T02:03:05Z"}}))

    def test_existing_output(self):
        Path(self.output).write_text("keep")
        code, _ = self.invoke(FakeDocker())
        self.assertEqual(code, "ATTESTATION_OUTPUT_EXISTS")
        self.assertEqual(Path(self.output).read_text(), "keep")
        self.assertEqual(os.listdir(self.dir), ["attestation.json"])

    def test_group_writable_directory(self):
        os.chmod(self.dir, 0o770)
        self.assertFails("ATTESTATION_OUTPUT_PATH", FakeDocker())
        os.chmod(self.dir, 0o707)
        self.assertFails("ATTESTATION_OUTPUT_PATH", FakeDocker())

    def test_bad_output_paths(self):
        for path in ("relative.json", self.dir + "/../x.json", self.dir + "/a\nb", self.dir + "/"):
            with self.subTest(path=path):
                self.assertEqual(self.invoke(FakeDocker(), output=path)[0], "ATTESTATION_OUTPUT_PATH")

    def test_lifetime_bounds(self):
        self.assertFails("ATTESTATION_ARGUMENTS", FakeDocker(), lifetime=43201)
        self.assertFails("ATTESTATION_ARGUMENTS", FakeDocker(), lifetime=0)
        self.assertIsNone(self.invoke(FakeDocker(), lifetime=43200)[0])

    def test_reader_not_in_stopped_state(self):
        for changes in ({"status": "running", "running": "true"}, {"status": "paused", "paused": "true"},
                        {"status": "restarting", "restarting": "true"}, {"status": "dead", "dead": "true"},
                        {"oom": "true"}, {"project": "other"}):
            with self.subTest(changes=changes):
                self.assertFails("ATTESTATION_READER_STATE", FakeDocker({RESEARCH: changes}))

    def test_wrong_exit_code(self):
        self.assertFails("ATTESTATION_READER_STATE", FakeDocker({RESEARCH: {"exit": "0"}}))
        self.assertFails("ATTESTATION_READER_STATE", FakeDocker({RESEARCH: {"exit": "137"}}))
        self.assertFails("ATTESTATION_READER_STATE", FakeDocker({OWNER: {"exit": "2"}}))
        self.assertFails("ATTESTATION_READER_STATE", FakeDocker({OWNER: {"exit": "1"}}))

    def test_format_v2_is_unspecified(self):
        fake = FakeDocker()
        self.assertFails("ATTESTATION_V2_UNSPECIFIED", fake, fmt="v2")
        self.assertEqual(fake.calls, [])

    def test_docker_failures(self):
        def failing(argv, **kwargs):
            return SimpleNamespace(returncode=1, stdout=b"")

        def raising(argv, **kwargs):
            raise OSError("boom")

        def short(argv, **kwargs):
            return SimpleNamespace(returncode=0, stdout=b"a\tb\n")
        for runner in (failing, raising, short):
            self.assertFails("ATTESTATION_DOCKER", runner)

    def test_validator_rejects_plus_zero_offset_timestamp(self):
        # A raw Docker string the validator refuses must never be published.
        self.assertFails("ATTESTATION_VALIDATOR",
                         FakeDocker({OWNER: {"finished": "2026-11-10T02:03:04+00:00"}}))

    def test_cli_error_is_value_free(self):
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            status = tool._cli(["--apply", "--commit", COMMIT, "--output", self.output,
                                "--lifetime-seconds", "1", "--format", "v2"],
                               runner=FakeDocker())
        self.assertEqual((status, stderr.getvalue()), (1, "ATTESTATION_V2_UNSPECIFIED\n"))
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr), self.assertRaises(SystemExit) as context:
            tool._cli(["--apply"])
        self.assertEqual((context.exception.code, stderr.getvalue()), (2, "ATTESTATION_ARGUMENTS\n"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
