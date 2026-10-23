#!/usr/bin/env python3
"""Disposable real-Docker fixtures, not the Rust approved-base whole flow.

Builds a tiny shell-process image from the already pinned runtime base, selects
the actual daily overlay and actual worker Compose stanza, and destroys only
the test's unique resources. No database, provider or production path is used.
"""
from __future__ import annotations

import copy
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
import uuid
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "daily_runtime_fixture", ROOT / "scripts/ops/lib/owner-beta-daily-runtime.py")
assert SPEC and SPEC.loader
runtime = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runtime
SPEC.loader.exec_module(runtime)
BASE_IMAGE = "alpine@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d"
COMMIT = "a" * 40
SCRIPT = """#!/bin/sh
if [ "$1" = healthcheck ]; then
    test -f /run/owner-beta-health/ready
    exit $?
fi
trap 'exit 0' TERM
: >/run/owner-beta-health/ready
while :; do
    sleep 1 &
    wait $!
done
"""


def command(argv, timeout=30, check=True):
    result = subprocess.run(argv, capture_output=True, timeout=timeout, check=False,
                            env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin",
                                 "HOME": "/root", "LANG": "C.UTF-8"})
    if check and result.returncode:
        # Never dump the daemon or shell output, even for a fixture failure.
        raise AssertionError("DAILY_DOCKER_FIXTURE_COMMAND")
    return result


def docker(*args, timeout=30, check=True):
    return command(["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", *args],
                   timeout, check)


class DailyDockerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if sys.platform != "linux" or os.geteuid() != 0:
            raise RuntimeError("DAILY_DOCKER_FIXTURE_ENVIRONMENT")
        cls.anchor = Path(tempfile.mkdtemp(prefix="lagrange-daily-docker-", dir="/var/lib"))
        cls.anchor.chmod(0o755)
        cls.addClassCleanup(shutil.rmtree, cls.anchor)
        build = cls.anchor / "build"
        build.mkdir(mode=0o755)
        (build / "runner").write_text(SCRIPT)
        (build / "Dockerfile").write_text(
            "FROM " + BASE_IMAGE + "\n"
            "LABEL org.opencontainers.image.revision=" + COMMIT + "\n"
            "ENV LAGRANGE_CODE_COMMIT=" + COMMIT + "\n"
            "COPY --chmod=0755 runner /usr/local/bin/owner-beta-runner\n"
            "USER 10001:10001\nSTOPSIGNAL SIGTERM\n"
            'ENTRYPOINT ["/usr/local/bin/owner-beta-runner"]\n')
        cls.tag = "lagrange-daily-fixture:" + uuid.uuid4().hex
        docker("pull", BASE_IMAGE, timeout=120)
        docker("build", "--network=none", "--pull=false", "--tag", cls.tag, str(build),
               timeout=120)
        cls.addClassCleanup(docker, "image", "rm", cls.tag)
        cls.image_id = json.loads(docker("image", "inspect", cls.tag).stdout)[0]["Id"]
        assert runtime.DIGEST.fullmatch(cls.image_id)

    def setUp(self):
        self.root = Path(tempfile.mkdtemp(dir=self.anchor))
        self.root.chmod(0o755)
        self.addCleanup(shutil.rmtree, self.root)
        self.project = "daily-" + uuid.uuid4().hex
        self.addCleanup(self.remove_owned)
        self.release = self.root / "release"
        self.compose_root = self.release / "deploy/compose"
        self.compose_root.mkdir(parents=True, mode=0o755)
        self.data = self.root / "data"
        self.artifacts = self.root / "artifacts"
        self.secrets = self.root / "secrets"
        for path in (self.data / "raw", self.artifacts / "historical-price-beta-root",
                     self.secrets / "owner-beta-runner",
                     self.secrets / "owner-beta-daily-admitter",
                     self.release / ".owner-beta-daily"):
            path.mkdir(parents=True, mode=0o755)
        (self.release / ".owner-beta-daily").chmod(0o750)
        os.chown(self.release / ".owner-beta-daily", 0, 10001)
        for name in ("config.json", "policy.json"):
            path = self.release / ".owner-beta-daily" / name
            path.write_text('{"fixture":true}\n')
            os.chown(path, 0, 10001)
            path.chmod(0o440)
        for service, uid, name in [
            ("owner-beta-runner", 10001, "db_worker_password"),
            ("owner-beta-daily-admitter", 10003, "db_daily_input_admitter_password"),
        ]:
            path = self.secrets / service / name
            path.write_text("fixture-only-never-connected")
            os.chown(path, uid, uid)
            path.chmod(0o400)
        shared = self.data / "owner-beta-daily"
        shared.mkdir(mode=0o750)
        os.chown(shared, 0, 10001)
        for role in ("producer", "admitter"):
            view = shared / "historical-base" / role
            view.mkdir(parents=True, mode=0o755)
            uid = 10002 if role == "producer" else 10003
            os.chown(view, uid, uid)
            view.chmod(0o750)
            probe = view / "fixed-synthetic-base"
            probe.write_bytes(b"same-fixed-synthetic-bytes\n")
            os.chown(probe, uid, uid)
            probe.chmod(0o600)
        for name, uid, gid, mode in [
            ("candidates", 10002, 10001, 0o750), ("admitted", 10003, 10001, 0o750),
            ("frontiers", 10003, 10001, 0o750), ("producer-state", 10002, 10002, 0o700),
            ("admitter-state", 10003, 10003, 0o700),
        ]:
            path = shared / name
            path.mkdir(mode=mode)
            os.chown(path, uid, gid)
        self.binding = runtime.Binding(
            self.project, COMMIT, self.image_id, "sha256:" + "c" * 64,
            str(self.release), str(self.data), str(self.artifacts), str(self.secrets),
            "lagrange", "10", "60", "30")
        original = yaml.safe_load((ROOT / "deploy/compose/compose.yml").read_bytes())
        worker = copy.deepcopy(original["services"]["owner-beta-runner"])
        # Only the image build/dependency launcher is substituted. The runtime
        # command, mounts, environment, healthcheck and limits are the real stanza.
        worker.pop("build")
        worker.pop("depends_on")
        base = {
            "services": {
                "owner-beta-runner": worker,
                "api-server": {"image": self.image_id},
                "postgres": {"image": self.image_id, "healthcheck": {"disable": True}},
                "db-migrate": {"image": self.image_id, "healthcheck": {"disable": True}},
            },
            "networks": {"owner-beta-db": {"internal": True}},
            "secrets": {"owner_beta_db_worker_password":
                        original["secrets"]["owner_beta_db_worker_password"]},
        }
        (self.compose_root / "base.json").write_text(json.dumps(base))
        (self.compose_root / "overlay.yml").write_bytes(
            (ROOT / "deploy/compose/compose.owner-beta-daily.yml").read_bytes())
        (self.compose_root / "images.json").write_text(json.dumps({
            "services": {name: {"image": self.image_id} for name in runtime.SERVICES}}))
        self.env = self.compose_root / ".env"
        self.env.write_text(
            "LAGRANGE_CODE_COMMIT=" + COMMIT + "\n"
            "LAGRANGE_DATA_DIR=" + str(self.data) + "\n"
            "LAGRANGE_ARTIFACTS_DIR=" + str(self.artifacts) + "\n"
            "LAGRANGE_RUNTIME_SECRET_DIR=" + str(self.secrets) + "\n"
            "LAGRANGE_DAILY_VERIFIED_CONFIG_SHA256=" + self.binding.config_sha256 + "\n")
        self.engine = runtime.Docker()

    def compose(self, *args, check=True):
        return docker("compose", "--project-name", self.project, "--env-file", str(self.env),
                      "-f", str(self.compose_root / "base.json"),
                      "-f", str(self.compose_root / "overlay.yml"),
                      "-f", str(self.compose_root / "images.json"), *args,
                      timeout=90, check=check)

    def remove_owned(self):
        ids = docker("ps", "-aq", "--filter",
                     "label=com.docker.compose.project=" + self.project).stdout.splitlines()
        # Destruction is limited to this test's unguessable project and image.
        for raw in ids:
            identity = raw.decode("ascii")
            require = json.loads(docker("container", "inspect", identity).stdout)[0]
            if require["Image"] != self.image_id:
                raise AssertionError("DAILY_DOCKER_FIXTURE_FOREIGN")
            docker("rm", "--force", identity)
        networks = docker("network", "ls", "-q", "--filter",
                          "label=com.docker.compose.project=" + self.project).stdout.splitlines()
        for raw in networks:
            docker("network", "rm", raw.decode("ascii"))
        if docker("ps", "-aq", "--filter",
                  "label=com.docker.compose.project=" + self.project).stdout.strip():
            raise AssertionError("DAILY_DOCKER_FIXTURE_CLEANUP")

    def start(self):
        self.compose("up", "--no-build", "--no-deps", "-d", *runtime.SERVICES)
        end = time.monotonic() + 65
        while time.monotonic() < end:
            snapshot = runtime.capture(self.engine, self.binding)
            if len(snapshot) == 3 and all(
                    v["value"]["State"].get("Health", {}).get("Status") == "healthy"
                    for v in snapshot.values()):
                return snapshot
            time.sleep(0.2)
        raise AssertionError("DAILY_DOCKER_FIXTURE_HEALTH")

    def shell(self, service, script):
        identity = self.engine.service_ids(self.project, service)[0]
        return docker("exec", identity, "/bin/sh", "-c", script, check=False)

    def test_actual_overlay_principals_mounts_and_networks_match(self):
        self.start()
        runtime.verify_running(self.engine, self.binding)
        for service, uid in zip(runtime.SERVICES, (10002, 10003, 10001)):
            result = self.shell(service, "id -u")
            self.assertEqual(result.returncode, 0)
            self.assertEqual(result.stdout.strip(), str(uid).encode())
            state = self.shell(service, "cat /proc/self/status").stdout.decode()
            self.assertRegex(state, r"(?m)^CapEff:\s+0000000000000000$")
            self.assertRegex(state, r"(?m)^NoNewPrivs:\s+1$")
            routes = self.shell(service, "cat /proc/net/route").stdout.decode().splitlines()[1:]
            self.assertTrue(all(row.split()[1] != "00000000" for row in routes))

    def test_actual_readonly_and_cross_principal_write_denials(self):
        self.start()
        runtime.verify_running(self.engine, self.binding)
        for service in runtime.SERVICES:
            for path in ("/root-escape", "/data/owner-beta-artifacts/change",
                         "/run/owner-beta-daily/config.json"):
                result = self.shell(service, "printf denied >" + path)
                self.assertNotEqual(result.returncode, 0)
        producer, admitter, worker = runtime.SERVICES
        self.assertEqual(self.shell(producer,
            "printf candidate >/data/owner-beta-daily/candidates/probe").returncode, 0)
        self.assertEqual(self.shell(admitter,
            "test \"$(cat /data/owner-beta-daily/candidates/probe)\" = candidate").returncode, 0)
        self.assertNotEqual(self.shell(admitter,
            "printf overwrite >/data/owner-beta-daily/candidates/probe").returncode, 0)
        self.assertEqual(self.shell(admitter,
            "printf admitted >/data/owner-beta-daily/admitted/probe").returncode, 0)
        self.assertNotEqual(self.shell(producer,
            "printf overwrite >/data/owner-beta-daily/admitted/probe").returncode, 0)
        self.assertEqual(self.shell(worker,
            "test \"$(cat /data/owner-beta-daily/admitted/probe)\" = admitted").returncode, 0)
        self.assertNotEqual(self.shell(worker,
            "printf overwrite >/data/owner-beta-daily/admitted/probe").returncode, 0)
        self.assertEqual((self.data / "owner-beta-daily/candidates/probe").stat().st_uid, 10002)
        self.assertEqual((self.data / "owner-beta-daily/admitted/probe").stat().st_uid, 10003)

    def test_actual_role_owned_base_mounts_allow_reads_and_deny_writes(self):
        self.start()
        runtime.verify_running(self.engine, self.binding)
        for service in runtime.SERVICES[:2]:
            read = self.shell(service, "cat /data/owner-beta-artifacts/fixed-synthetic-base")
            self.assertEqual((read.returncode, read.stdout), (0, b"same-fixed-synthetic-bytes\n"))
            denied = self.shell(service, "printf change >/data/owner-beta-artifacts/fixed-synthetic-base")
            self.assertNotEqual(denied.returncode, 0)

    def test_actual_sigterm_drain_preserves_generations_and_disables_restart(self):
        self.start()
        snapshot = runtime.verify_running(self.engine, self.binding)
        result = runtime.drain(self.engine, self.binding, snapshot, timeout=20)
        self.assertEqual(result, {"stopped": 3, "signalled": 3, "previously_running": 3})
        for service, entry in snapshot.items():
            value = self.engine.object("container", entry["generation"][0])
            self.assertEqual(runtime.owned(value, self.binding, service, value["Id"]),
                             entry["generation"])
            self.assertEqual(value["State"]["ExitCode"], 0)
            self.assertFalse(value["State"]["Running"])
            self.assertEqual(value["HostConfig"]["RestartPolicy"]["Name"], "no")
        self.assertEqual(runtime.drain(self.engine, self.binding, snapshot, timeout=20)
                         ["signalled"], 0)

    def test_actual_foreign_revision_is_never_stopped(self):
        path = self.compose_root / "images.json"
        override = json.loads(path.read_text())
        override["services"][runtime.SERVICES[1]]["labels"] = {
            "org.opencontainers.image.revision": "f" * 40}
        path.write_text(json.dumps(override))
        self.compose("up", "--no-build", "--no-deps", "-d", *runtime.SERVICES)
        with self.assertRaisesRegex(runtime.RuntimeErrorCode, "^DAILY_RUNTIME_OWNER$"):
            runtime.capture(self.engine, self.binding)
        for service in runtime.SERVICES:
            identity = self.engine.service_ids(self.project, service)[0]
            value = self.engine.object("container", identity)
            self.assertTrue(value["State"]["Running"])
            self.assertEqual(value["HostConfig"]["RestartPolicy"]["Name"], "unless-stopped")

    def test_actual_restarted_process_invalidates_captured_generation(self):
        self.start()
        snapshot = runtime.verify_running(self.engine, self.binding)
        identity = snapshot[runtime.SERVICES[0]]["generation"][0]
        docker("restart", "--time=10", identity)
        with self.assertRaisesRegex(runtime.RuntimeErrorCode, "^DAILY_RUNTIME_GENERATION$"):
            runtime.drain(self.engine, self.binding, snapshot, timeout=20)
        for service in runtime.SERVICES:
            value = self.engine.object("container", snapshot[service]["generation"][0])
            self.assertTrue(value["State"]["Running"])
            self.assertEqual(value["HostConfig"]["RestartPolicy"]["Name"], "unless-stopped")


if __name__ == "__main__":
    unittest.main(verbosity=2)
