#!/usr/bin/env python3
"""Private fixtures for the actual attestation reader and checkpoint parser."""
import ast
import calendar
import copy
import datetime
import json
from pathlib import Path
import re
import subprocess
import tempfile
import time
import unittest

HELPER = Path(__file__).resolve().parent / "lib/release-build-layout.sh"
BODY = HELPER.read_text().split("release_build_layout_gate() {", 1)[1].split(
    "<<'PY'\n", 1)[1].split("\nPY\n", 1)[0]
NAMES = {"parse_container", "docker_timestamp", "next_state"}
FUNCTIONS = [n for n in ast.parse(BODY).body
             if isinstance(n, ast.FunctionDef) and n.name in NAMES]
assert {n.name for n in FUNCTIONS} == NAMES
SCOPE = {"calendar": calendar, "datetime": datetime, "json": json, "re": re}
exec(compile(ast.Module(body=FUNCTIONS, type_ignores=[]), str(HELPER), "exec"), SCOPE)
BETA = "lagrange-station-owner-beta-runner-1"
OWNER = "lagrange-station-owner-equity-v2-runner-1"
RESEARCH = "lagrange-station-research-worker-1"
COMMIT = "a" * 40


def stopped(name, ordinal):
    return dict(container_name=name, container_id=str(ordinal) * 64,
                image_id="sha256:" + str(ordinal + 3) * 64, status="exited",
                running=False, restarting=False, paused=False, dead=False,
                oom_killed=False, health_status="unhealthy", restart_count=0,
                exit_code=2 if name == RESEARCH else 0,
                started_at_utc="2020-01-01T00:00:00.000000000Z",
                finished_at_utc="2020-01-01T01:00:00.000000000Z")


def document(version=2):
    now = datetime.datetime.now(datetime.timezone.utc)
    value = dict(format=f"lagrange-build-drained-readers-attestation-v{version}",
                 scope="image-build-only", project="lagrange-station", target_commit=COMMIT,
                 observed_at_utc=(now-datetime.timedelta(seconds=5)).strftime("%Y-%m-%dT%H:%M:%SZ"),
                 expires_at_utc=(now+datetime.timedelta(hours=1)).strftime("%Y-%m-%dT%H:%M:%SZ"),
                 containers=[stopped(OWNER, 2), stopped(RESEARCH, 3)])
    if version == 2:
        value["containers"].insert(0, stopped(BETA, 1))
        value["disabled_daily"] = dict(
            enabled=False, running_jobs=0, activation_epoch=0,
            owner_user_id="11111111-1111-4111-8111-111111111111",
            policy_sha256="b"*64, source_release="c"*40,
            control_observation_sha256="d"*64)
    return value


def bound(value):
    with tempfile.TemporaryDirectory(prefix="r2-disabled-build-") as directory:
        path = Path(directory) / "attestation.json"
        with path.open("x") as handle:
            json.dump(value, handle, sort_keys=True, separators=(",", ":"))
            handle.write("\n")
        path.chmod(0o600)
        result = subprocess.run(
            ["bash", "-c", 'source "$1"; rbl_drained_readers_attestation_binding "$2" "$3"',
             "fixture", str(HELPER), str(path), COMMIT], capture_output=True)
        return result.returncode, result.stdout


def observed(item, **changes):
    item = dict(item, **changes)
    fields = [item["container_id"], str(item["running"]).lower(),
              str(item["restarting"]).lower(), str(item["oom_killed"]).lower(),
              item["health_status"], str(item["restart_count"]), "lagrange-station",
              item["image_id"], str(item["exit_code"]), item["status"],
              str(item["paused"]).lower(), str(item["dead"]).lower(),
              item["started_at_utc"], item["finished_at_utc"]]
    return ("\t".join(fields)+"\n").encode()


def drained(value):
    return {"binding": {"fields": value}, "project": value["project"],
            "observed_ns": SCOPE["docker_timestamp"](value["observed_at_utc"]),
            "by_name": {item["container_name"]: item for item in value["containers"]}}


class DisabledDailyBuild(unittest.TestCase):
    def test_original_two_reader_attestation_remains_valid(self):
        self.assertEqual(bound(document(1))[0], 0)

    def test_disabled_three_reader_binding_and_exact_checkpoint(self):
        value = document()
        code, raw = bound(value)
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(raw)["fields"], value)
        parsed = SCOPE["parse_container"](observed(value["containers"][0]), BETA,
                                          None, drained(value), time.time_ns())
        self.assertEqual(parsed["exit_code"], 0)
        self.assertEqual(parsed["status"], "exited")

    def test_v1_cannot_silently_acquire_disabled_daily(self):
        value = document()
        value["format"] = "lagrange-build-drained-readers-attestation-v1"
        self.assertNotEqual(bound(value)[0], 0)

    def test_v2_requires_explicit_disabled_control_observation(self):
        value = document()
        del value["disabled_daily"]
        self.assertNotEqual(bound(value)[0], 0)

    def test_enabled_running_or_malformed_control_is_rejected(self):
        for key, bad in (("enabled", True), ("enabled", 0), ("running_jobs", 1),
                         ("running_jobs", False), ("activation_epoch", -1),
                         ("activation_epoch", False), ("source_release", "0"*40),
                         ("control_observation_sha256", ""), ("policy_sha256", "bad"),
                         ("owner_user_id", "00000000-0000-0000-0000-000000000000")):
            with self.subTest(field=key, value=bad):
                value = document()
                value["disabled_daily"][key] = bad
                self.assertNotEqual(bound(value)[0], 0)

    def test_disabled_reader_requires_normal_zero_exit(self):
        for name in (BETA, OWNER):
            for bad in (1, 2, 137, 143):
                with self.subTest(name=name, exit=bad):
                    value = document()
                    next(item for item in value["containers"]
                         if item["container_name"] == name)["exit_code"] = bad
                    self.assertNotEqual(bound(value)[0], 0)

    def test_disabled_profile_cannot_omit_or_add_a_container(self):
        for action in ("omit", "extra", "duplicate"):
            value = document()
            if action == "omit":
                value["containers"].pop(0)
            else:
                item = copy.deepcopy(value["containers"][0])
                if action == "extra":
                    item["container_name"] = "lagrange-station-api-server-1"
                value["containers"].append(item)
            with self.subTest(action=action):
                self.assertNotEqual(bound(value)[0], 0)

    def test_each_actual_disabled_container_drift_fails(self):
        value = document()
        item = value["containers"][0]
        for key, bad in (("container_id", "f"*64), ("image_id", "sha256:"+"e"*64),
                         ("exit_code", 143), ("running", True), ("paused", True),
                         ("dead", True), ("oom_killed", True), ("restarting", True),
                         ("restart_count", 1), ("health_status", "healthy"),
                         ("started_at_utc", "2020-01-01T00:01:00Z"),
                         ("finished_at_utc", "2020-01-01T01:01:00Z")):
            with self.subTest(field=key):
                with self.assertRaises(ValueError):
                    SCOPE["parse_container"](observed(item, **{key: bad}), BETA,
                                            None, drained(value), time.time_ns())

    def test_ordinary_service_still_requires_live_health(self):
        with self.assertRaisesRegex(ValueError, "container-health-invalid"):
            SCOPE["parse_container"](observed(stopped("lagrange-station-api-server-1", 1)),
                                    "lagrange-station-api-server-1", None, drained(document()),
                                    time.time_ns())

    def test_disabled_container_cannot_start_and_stop_between_checkpoints(self):
        control = drained(document())
        selected = {name: SCOPE["parse_container"](observed(item), name, None,
                    control, time.time_ns()) for name, item in control["by_name"].items()}
        old = SCOPE["next_state"](None, "a"*32, 1, "origin", {}, selected, None, control)
        self.assertEqual(SCOPE["next_state"](old, "a"*32, 1, "origin", {},
                                            selected, None, control), old)
        changed = copy.deepcopy(selected)
        changed[BETA]["started_at_utc"] = "2020-01-01T00:02:00Z"
        with self.assertRaisesRegex(ValueError, "drained-readers-state-changed"):
            SCOPE["next_state"](old, "a"*32, 1, "origin", {}, changed, None, control)


if __name__ == "__main__":
    unittest.main(verbosity=2)
