#!/usr/bin/env python3
"""Closed daily Docker policy and drain failures; all Docker effects are fakes."""
from __future__ import annotations

import copy
import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "daily_runtime", ROOT / "scripts/ops/lib/owner-beta-daily-runtime.py")
assert SPEC and SPEC.loader
runtime = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runtime
SPEC.loader.exec_module(runtime)
COMMIT = "a" * 40
IMAGE = "sha256:" + "b" * 64


def binding():
    return runtime.Binding("daily-test", COMMIT, IMAGE, "sha256:" + "c" * 64,
                           "/var/lib/fixture/release", "/var/lib/fixture/data",
                           "/var/lib/fixture/artifacts", "/var/lib/fixture/secrets",
                           "lagrange", "10", "60", "30")


def container(context, service):
    index = runtime.SERVICES.index(service)
    uid = (10002, 10003, 10001)[index]
    env = {
        "PATH": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "APP_ENV": "production", "LAGRANGE_CODE_COMMIT": COMMIT,
        "PRICE_BETA_DAILY_MODE": "owner_only",
        "PRICE_BETA_DAILY_CONFIG_FILE": "/run/owner-beta-daily/config.json",
        "PRICE_BETA_DAILY_CONFIG_SHA256": context.config_sha256,
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
    network = context.project + "_owner-beta-db"
    return {
        "Id": str(index + 1) * 64, "Created": "2026-10-16T00:00:00Z",
        "RestartCount": 0, "Image": IMAGE,
        "State": {"Running": True, "Restarting": False, "Paused": False,
                  "OOMKilled": False, "Error": "", "ExitCode": 0,
                  "Status": "running", "StartedAt": "2026-10-16T00:01:00Z",
                  "Health": {"Status": "healthy"}},
        "Config": {"User": f"{uid}:{uid}", "Entrypoint": [runtime.BINARY],
                   "Cmd": [["daily-producer"], ["daily-admitter"],
                           ["--artifact-root", "/data/owner-beta-artifacts"]][index],
                   "Labels": {"org.opencontainers.image.revision": COMMIT,
                              "com.docker.compose.project": context.project,
                              "com.docker.compose.service": service,
                              "com.docker.compose.oneoff": "False"},
                   "Env": [k + "=" + v for k, v in env.items()],
                   "Healthcheck": {"Test": ["CMD", runtime.BINARY, "healthcheck"]}},
        "HostConfig": {"ReadonlyRootfs": True, "Init": True, "Privileged": False,
                       "CapDrop": ["ALL"], "CapAdd": None,
                       "SecurityOpt": ["no-new-privileges:true"],
                       "GroupAdd": ["10001"] if index < 2 else None,
                       "IpcMode": "private", "Memory": 1024**3, "NanoCpus": 10**9,
                       "RestartPolicy": {"Name": "unless-stopped"},
                       "Tmpfs": {"/run/owner-beta-health": f"uid={uid},gid={uid},mode=0750"},
                       "LogConfig": {"Type": "json-file",
                                     "Config": {"max-size": "10m", "max-file": "3",
                                                "tag": "{{.Name}}"}},
                       "NetworkMode": "none" if index == 0 else network},
        "Mounts": [{"Type": "bind", "Source": source, "Destination": target,
                    "RW": writable, "Propagation": "rprivate"}
                   for target, (source, writable) in context.mounts(service).items()],
        "NetworkSettings": {"Networks": {"none": {}} if index == 0 else
                            {network: {"NetworkID": "d" * 64}}},
    }


class FakeDocker:
    def __init__(self, context):
        self.context = context
        self.values = {service: container(context, service) for service in runtime.SERVICES}
        self.calls = []
        self.on_call = None
        self.network = {"Id": "d" * 64, "Internal": True,
                        "Labels": {"com.docker.compose.project": context.project,
                                   "com.docker.compose.network": "owner-beta-db"}}
        self.image = {"Id": IMAGE, "Config": {
            "Labels": {"org.opencontainers.image.revision": COMMIT}}}

    def object(self, kind, identity, timeout=15):
        if kind == "image":
            return copy.deepcopy(self.image)
        if kind == "network":
            return copy.deepcopy(self.network)
        for value in self.values.values():
            if value["Id"] == identity:
                return copy.deepcopy(value)
        raise runtime.RuntimeErrorCode("DAILY_RUNTIME_DOCKER")

    def service_ids(self, project, service, timeout=15):
        return [self.values[service]["Id"]] if service in self.values else []

    def call(self, args, timeout=15):
        self.calls.append(list(args))
        if self.on_call:
            self.on_call(args)
        if args[0] == "update":
            value = next(v for v in self.values.values() if v["Id"] == args[-1])
            value["HostConfig"]["RestartPolicy"]["Name"] = "no"
        if args[0] == "wait":
            for value in self.values.values():
                if value["Id"] in args[1:]:
                    value["State"].update(Running=False, Status="exited")
            return "0\n" * (len(args) - 1)
        return ""


class DailyRuntimeTests(unittest.TestCase):
    def setUp(self):
        self.context = binding()
        self.docker = FakeDocker(self.context)

    def rejected(self, action):
        with self.assertRaises(runtime.RuntimeErrorCode):
            action()

    def test_binding_rejects_unbounded_or_ambiguous_context(self):
        original = vars(self.context)
        for key, values in {
            "project": ["", "-wrong", "a\n", "a" * 64],
            "commit": ["0" * 40, "A" * 40, "a" * 41],
            "image_id": ["latest", "sha256:" + "A" * 64],
            "data_root": ["relative", "/data/../raw", "/data//raw", "/", "/data/"],
            "database": ["lagrange;select", "", "db\n"],
            "capacity": ["0", "01", "-1", "2147483648"],
            "backoff": ["9223372036854775808", "1;select"],
        }.items():
            for value in values:
                with self.subTest(key=key, value=value):
                    self.rejected(lambda: runtime.Binding(**{**original, key: value}))

    def test_all_three_distinct_principals_and_views_are_verified(self):
        self.assertEqual(set(runtime.verify_running(self.docker, self.context)),
                         set(runtime.SERVICES))
        producer = self.context.mounts(runtime.SERVICES[0])
        admitter = self.context.mounts(runtime.SERVICES[1])
        worker = self.context.mounts(runtime.SERVICES[2])
        self.assertTrue(producer["/data/owner-beta-daily/candidates"][1])
        self.assertFalse(producer["/data/owner-beta-daily/admitted"][1])
        self.assertFalse(admitter["/data/owner-beta-daily/candidates"][1])
        self.assertTrue(admitter["/data/owner-beta-daily/admitted"][1])
        self.assertTrue(all(not rw for _, rw in worker.values()))

    def test_foreign_image_or_project_is_rejected_before_any_mutation(self):
        for key in ("image", "project", "revision", "oneoff"):
            with self.subTest(key=key):
                self.docker = FakeDocker(self.context)
                value = self.docker.values[runtime.SERVICES[1]]
                if key == "image":
                    value["Image"] = "sha256:" + "f" * 64
                else:
                    field = {"project": "com.docker.compose.project",
                             "revision": "org.opencontainers.image.revision",
                             "oneoff": "com.docker.compose.oneoff"}[key]
                    value["Config"]["Labels"][field] = "wrong"
                self.rejected(lambda: runtime.capture(self.docker, self.context))
                self.assertEqual(self.docker.calls, [])

    def test_mutated_image_revision_is_rejected(self):
        self.docker.image["Config"]["Labels"]["org.opencontainers.image.revision"] = "f" * 40
        self.rejected(lambda: runtime.capture(self.docker, self.context))

    def test_command_uid_privilege_or_extra_mount_is_rejected(self):
        mutations = [
            lambda v: v["Config"].update(User="0:0"),
            lambda v: v["Config"].update(Cmd=["sh"]),
            lambda v: v["Config"].update(Entrypoint=["/bin/sh"]),
            lambda v: v["HostConfig"].update(Privileged=True),
            lambda v: v["HostConfig"].update(ReadonlyRootfs=False),
            lambda v: v["HostConfig"].update(CapAdd=["NET_ADMIN"]),
            lambda v: v["HostConfig"].update(SecurityOpt=[]),
            lambda v: v["HostConfig"].update(PidMode="host"),
            lambda v: v["HostConfig"].update(GroupAdd=["0"]),
            lambda v: v["HostConfig"].update(PortBindings={"8080/tcp": []}),
            lambda v: v["HostConfig"].update(Devices=[{"PathOnHost": "/dev/sda"}]),
            lambda v: v["Mounts"].append({"Type": "bind", "Source": "/", "Destination":
                                         "/host", "RW": True, "Propagation": "rprivate"}),
            lambda v: v["Mounts"][0].update(RW=True),
            lambda v: v["Config"].update(Healthcheck={"Test": ["CMD", "true"]}),
        ]
        for mutation in mutations:
            with self.subTest(mutation=mutations.index(mutation)):
                docker = FakeDocker(self.context)
                mutation(docker.values[runtime.SERVICES[0]])
                self.rejected(lambda: runtime.verify_running(docker, self.context))

    def test_producer_network_or_credential_and_duplicate_env_are_rejected(self):
        for extra in ["DATABASE_URL=sentinel-secret", "KIS_APP_KEY=sentinel-secret",
                      "PRICE_BETA_DAILY_MODE=owner_only"]:
            docker = FakeDocker(self.context)
            docker.values[runtime.SERVICES[0]]["Config"]["Env"].append(extra)
            self.rejected(lambda: runtime.verify_running(docker, self.context))
        self.docker.values[runtime.SERVICES[0]]["HostConfig"]["NetworkMode"] = "host"
        self.rejected(lambda: runtime.verify_running(self.docker, self.context))

    def test_internal_network_labels_identity_and_count_are_checked(self):
        for mutation in [
            lambda d: d.network.update(Internal=False),
            lambda d: d.network.update(Id="e" * 64),
            lambda d: d.network["Labels"].update({"com.docker.compose.project": "foreign"}),
            lambda d: d.values[runtime.SERVICES[1]]["NetworkSettings"]["Networks"].update(
                {"second-egress-network": {}}),
        ]:
            docker = FakeDocker(self.context)
            mutation(docker)
            self.rejected(lambda: runtime.verify_running(docker, self.context))

    def test_missing_process_or_unhealthy_process_is_not_running_acceptance(self):
        del self.docker.values[runtime.SERVICES[1]]
        self.rejected(lambda: runtime.verify_running(self.docker, self.context))
        for state in [{"Running": False}, {"Paused": True}, {"Restarting": True},
                      {"OOMKilled": True}, {"Health": {"Status": "starting"}}]:
            docker = FakeDocker(self.context)
            docker.values[runtime.SERVICES[2]]["State"].update(state)
            self.rejected(lambda: runtime.verify_running(docker, self.context))

    def test_drain_disables_all_restarts_then_signals_all_before_wait(self):
        snapshot = runtime.capture(self.docker, self.context)
        result = runtime.drain(self.docker, self.context, snapshot)
        self.assertEqual(result, {"stopped": 3, "signalled": 3, "previously_running": 3})
        self.assertEqual([x[0] for x in self.docker.calls],
                         ["update"] * 3 + ["kill"] * 3 + ["wait"])
        self.assertTrue(all(x[1] == "--signal=TERM" for x in self.docker.calls[3:6]))
        self.assertTrue(all(v["HostConfig"]["RestartPolicy"]["Name"] == "no"
                            for v in self.docker.values.values()))

    def test_owned_policy_failure_can_be_drained_without_accepting_it(self):
        self.docker.values[runtime.SERVICES[1]]["HostConfig"]["ReadonlyRootfs"] = False
        self.rejected(lambda: runtime.verify_running(self.docker, self.context))
        snapshot = runtime.capture(self.docker, self.context)
        self.assertEqual(runtime.drain(self.docker, self.context, snapshot)["stopped"], 3)

    def test_replaced_or_restarted_cohort_fails_before_any_mutation(self):
        for key, value in [("Id", "f" * 64), ("RestartCount", 1)]:
            docker = FakeDocker(self.context)
            snapshot = runtime.capture(docker, self.context)
            docker.values[runtime.SERVICES[1]][key] = value
            self.rejected(lambda: runtime.drain(docker, self.context, snapshot))
            self.assertEqual(docker.calls, [])

    def test_failure_after_restart_update_never_resets_or_forces_kill(self):
        snapshot = runtime.capture(self.docker, self.context)
        def fail_wait(args):
            if args[0] == "wait":
                raise runtime.RuntimeErrorCode("DAILY_RUNTIME_DOCKER")
        self.docker.on_call = fail_wait
        self.rejected(lambda: runtime.drain(self.docker, self.context, snapshot))
        self.assertEqual([x[0] for x in self.docker.calls],
                         ["update"] * 3 + ["kill"] * 3 + ["wait"])
        self.assertTrue(all("--signal=KILL" not in x and "stop" not in x
                            for x in self.docker.calls))
        self.assertTrue(all(v["HostConfig"]["RestartPolicy"]["Name"] == "no"
                            for v in self.docker.values.values()))

    def test_changed_generation_after_updates_is_not_signalled(self):
        snapshot = runtime.capture(self.docker, self.context)
        def restart(args):
            if args == ["update", "--restart=no", "3" * 64]:
                self.docker.values[runtime.SERVICES[0]]["RestartCount"] += 1
        self.docker.on_call = restart
        self.rejected(lambda: runtime.drain(self.docker, self.context, snapshot))
        self.assertEqual([x[0] for x in self.docker.calls],
                         ["update"] * 3 + ["kill"] * 2 + ["wait"])
        self.assertNotIn(["kill", "--signal=TERM", "1" * 64], self.docker.calls)

    def test_one_signal_failure_still_drains_other_owned_members(self):
        snapshot = runtime.capture(self.docker, self.context)
        def fail_first(args):
            if args == ["kill", "--signal=TERM", "1" * 64]:
                raise runtime.RuntimeErrorCode("DAILY_RUNTIME_DOCKER")
        self.docker.on_call = fail_first
        self.rejected(lambda: runtime.drain(self.docker, self.context, snapshot))
        self.assertEqual([x[0] for x in self.docker.calls],
                         ["update"] * 3 + ["kill"] * 3 + ["wait"])
        self.assertTrue(self.docker.values[runtime.SERVICES[0]]["State"]["Running"])
        self.assertFalse(self.docker.values[runtime.SERVICES[1]]["State"]["Running"])
        self.assertFalse(self.docker.values[runtime.SERVICES[2]]["State"]["Running"])

    def test_new_service_after_capture_and_failed_exit_are_not_stopped_success(self):
        del self.docker.values[runtime.SERVICES[1]]
        snapshot = runtime.capture(self.docker, self.context)
        self.docker.values[runtime.SERVICES[1]] = container(self.context, runtime.SERVICES[1])
        self.rejected(lambda: runtime.drain(self.docker, self.context, snapshot))
        self.assertEqual(self.docker.calls, [])
        self.docker = FakeDocker(self.context)
        self.docker.values[runtime.SERVICES[2]]["State"].update(
            Running=False, Status="exited", ExitCode=1)
        snapshot = runtime.capture(self.docker, self.context)
        self.rejected(lambda: runtime.drain(self.docker, self.context, snapshot))

    def test_absent_cohort_and_clean_already_stopped_are_idempotent(self):
        self.docker.values = {}
        self.assertEqual(runtime.drain(self.docker, self.context, {})["stopped"], 0)
        self.docker = FakeDocker(self.context)
        for value in self.docker.values.values():
            value["State"].update(Running=False, Status="exited")
        snapshot = runtime.capture(self.docker, self.context)
        self.assertEqual(runtime.drain(self.docker, self.context, snapshot)["signalled"], 0)
        self.assertEqual([x[0] for x in self.docker.calls], ["update"] * 3)

    def test_paused_or_expired_drain_does_not_mutate(self):
        self.docker.values[runtime.SERVICES[0]]["State"]["Paused"] = True
        snapshot = runtime.capture(self.docker, self.context)
        self.rejected(lambda: runtime.drain(self.docker, self.context, snapshot))
        self.rejected(lambda: runtime.drain(self.docker, self.context, snapshot, timeout=0))
        self.assertEqual(self.docker.calls, [])

    def test_docker_transport_pins_local_daemon_and_redacts_failures(self):
        with mock.patch.object(runtime.os, "geteuid", return_value=0):
            docker = runtime.Docker()
        with mock.patch.object(runtime.subprocess, "run") as call:
            call.return_value = subprocess.CompletedProcess([], 0, b"ok\n", b"")
            self.assertEqual(docker.call(["version"]), "ok\n")
            args, kwargs = call.call_args
            self.assertEqual(args[0][:3],
                             ["/usr/bin/docker", "--host", "unix:///var/run/docker.sock"])
            self.assertNotIn("DOCKER_HOST", kwargs["env"])
            self.assertNotIn("DOCKER_CONTEXT", kwargs["env"])
            call.return_value = subprocess.CompletedProcess(
                [], 1, b"sentinel-secret", b"sentinel-secret")
            with self.assertRaises(runtime.RuntimeErrorCode) as error:
                docker.call(["version"])
            self.assertEqual(str(error.exception), "DAILY_RUNTIME_DOCKER")
            call.side_effect = subprocess.TimeoutExpired("sentinel-secret", 1)
            with self.assertRaises(runtime.RuntimeErrorCode) as error:
                docker.call(["version"])
            self.assertNotIn("sentinel", str(error.exception))

    def test_nonroot_transport_guard_is_rejected(self):
        with mock.patch.object(runtime.os, "geteuid", return_value=10003):
            with self.assertRaisesRegex(runtime.RuntimeErrorCode, "^DAILY_RUNTIME_ROOT$"):
                runtime.Docker()


if __name__ == "__main__":
    unittest.main(verbosity=2)
