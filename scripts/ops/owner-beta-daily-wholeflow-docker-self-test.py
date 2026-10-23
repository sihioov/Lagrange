#!/usr/bin/env python3
"""Actual hosted Docker create/inspect boundaries; no base, DB process or provider.

Requires Linux root and the fixed PostgreSQL image already used by prior policy
fixtures. Missing prerequisites fail; no skips. Never start these containers.
"""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import types
import unittest

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("wholeflow_docker_tested", ROOT / "scripts/ops/verify-owner-beta-daily-wholeflow.py")
driver = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = driver
spec.loader.exec_module(driver)


class WholeflowDockerMountTests(unittest.TestCase):
    @staticmethod
    def command(args, timeout=30):
        p = subprocess.run(["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", *args],
                           env=driver.ENV, capture_output=True, timeout=timeout, check=False)
        if p.returncode:
            raise RuntimeError("WHOLEFLOW_DOCKER_FIXTURE_COMMAND_FAILED")
        return p.stdout

    @classmethod
    def setUpClass(cls):
        if os.geteuid() != 0 or sys.platform != "linux":
            raise RuntimeError("WHOLEFLOW_DOCKER_FIXTURE_ROOT_REQUIRED")
        image = json.loads(cls.command(["image", "inspect", driver.POSTGRES]))[0]
        if driver.POSTGRES not in image["RepoDigests"] or image["Config"]["Volumes"] != {"/var/lib/postgresql": {}}:
            raise RuntimeError("WHOLEFLOW_DOCKER_FIXTURE_IMAGE_CONTRACT")
        cls.image = image["Id"]
        cls.extra_image = None
        cls.build = Path(tempfile.mkdtemp(prefix="wholeflow-mount-image-", dir="/var/lib"))
        cls.addClassCleanup(cls.cleanup_image)
        (cls.build / "Dockerfile").write_text("FROM " + driver.POSTGRES + '\nVOLUME ["/unexpected-fixture-volume"]\n')
        cls.command(["build", "--pull=false", "--network=none", "--iidfile", str(cls.build / "iid"), str(cls.build)], timeout=180)
        cls.extra_image = (cls.build / "iid").read_text().strip()
        if cls.extra_image == cls.image:
            raise RuntimeError("WHOLEFLOW_DOCKER_FIXTURE_DERIVED_ID")

    @classmethod
    def cleanup_image(cls):
        if cls.extra_image is not None:
            cls.command(["image", "rm", cls.extra_image])
        shutil.rmtree(cls.build)

    def setUp(self):
        self.volumes_before = set(self.command(["volume", "ls", "--quiet"]).splitlines())
        self.test = driver.WholeFlow(types.SimpleNamespace(timeout_seconds=1200, candidate="a" * 40))
        self.test.images = {"db-role-bootstrap": self.image, "db-migrate": self.image}
        self.test.root = Path(tempfile.mkdtemp(prefix=self.test.nonce + "-", dir="/var/lib"))
        self.addCleanup(self.cleanup_resources)
        self.secret = self.test.root / "synthetic-secret"
        self.secret.write_bytes(b"synthetic-fixture-only")
        self.secret.chmod(0o400)

    def cleanup_resources(self):
        self.test.cleanup()
        self.assertIsNone(self.test.root)
        self.assertEqual(set(self.command(["volume", "ls", "--quiet"]).splitlines()), self.volumes_before)
        self.assertFalse(self.command(["container", "ls", "--all", "--quiet", "--filter", "label=" + driver.LABEL + "=" + self.test.nonce]).strip())

    def check_client(self, service, entry):
        identity = self.test.create(service, self.image, 999, "none",
            [self.test.mount(self.secret, "/run/secrets/postgres_password")], {}, entry)
        value = json.loads(self.command(["container", "inspect", identity]))[0]
        self.assertEqual(value["State"]["Status"], "created")
        self.assertFalse(value["State"]["Running"])
        self.assertEqual(value["HostConfig"]["Tmpfs"]["/var/lib/postgresql"],
                         "ro,nosuid,nodev,noexec,size=1m,uid=999,gid=999,mode=0700")
        self.assertFalse(any(m["Type"] == "volume" for m in value["Mounts"]))
        self.assertEqual([(m["Destination"], m["Source"], m["RW"]) for m in value["Mounts"] if m["Type"] == "bind"],
                         [("/run/secrets/postgres_password", str(self.secret), False)])
        self.assertEqual(set(self.command(["volume", "ls", "--quiet"]).splitlines()), self.volumes_before)

    def test_bootstrap_inherited_volume_is_readonly_tmpfs_without_persistent_volume(self):
        self.check_client("bootstrap", "/usr/local/bin/lagrange-bootstrap-roles")

    def test_migrate_inherited_volume_is_readonly_tmpfs_without_persistent_volume(self):
        self.check_client("migrate", "/usr/local/bin/lagrange-migrate")

    def test_control_inherited_volume_is_readonly_tmpfs_without_persistent_volume(self):
        self.check_client("control", "/usr/local/bin/lagrange-daily-control")

    def test_postgres_data_remains_exact_private_writable_bind(self):
        data = self.test.root / "postgres-data"
        data.mkdir(mode=0o700)
        identity = self.test.create("postgres", self.image, 999, "none",
            [self.test.mount(data, "/var/lib/postgresql", False)], {}, "/usr/local/bin/docker-entrypoint.sh", ["postgres"])
        value = json.loads(self.command(["container", "inspect", identity]))[0]
        self.assertFalse(value["State"]["Running"])
        self.assertNotIn("/var/lib/postgresql", value["HostConfig"]["Tmpfs"])
        self.assertEqual([(m["Destination"], m["Source"], m["RW"]) for m in value["Mounts"] if m["Type"] == "bind"],
                         [("/var/lib/postgresql", str(data), True)])
        self.assertFalse(any(m["Type"] == "volume" for m in value["Mounts"]))

    def test_unexpected_inherited_volume_rejected_before_daemon_creation(self):
        with self.assertRaisesRegex(driver.fixture.FixtureError, "WHOLEFLOW_IMAGE_VOLUMES"):
            self.test.create("api", self.extra_image, 10001, "none", [], {}, "/not-executed")
        self.assertFalse(self.test.intents)
        self.assertFalse(self.test.uncertain_creations)
        self.assertFalse(self.command(["container", "ls", "--all", "--quiet", "--filter", "label=" + driver.LABEL + "=" + self.test.nonce]).strip())
        self.assertEqual(set(self.command(["volume", "ls", "--quiet"]).splitlines()), self.volumes_before)


if __name__ == "__main__":
    unittest.main(verbosity=2)
