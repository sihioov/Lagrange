#!/usr/bin/env python3
"""Hosted disposable daemon-event contract: /bin/true only, no application/DB/provider.

Uses the pre-pulled repository PostgreSQL image, covers its declared volume with
read-only tmpfs, and never builds/pulls. Not approved-base wholeflow acceptance.
"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import uuid

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("lifecycle_docker_driver", ROOT / "scripts/ops/verify-owner-beta-daily-wholeflow.py")
driver = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = driver
spec.loader.exec_module(driver)


def command(*args):
    result = subprocess.run(["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", *args],
                            env=driver.ENV, capture_output=True, timeout=30, check=False)
    if result.returncode:
        raise RuntimeError("LIFECYCLE_DOCKER_FIXTURE_COMMAND_FAILED")
    return result.stdout


class DockerLifecycleTests(unittest.TestCase):
    def test_actual_network_and_short_lived_container_events_are_complete(self):
        self.assertEqual(os.geteuid(), 0)
        self.assertEqual(sys.platform, "linux")
        image = json.loads(command("image", "inspect", driver.POSTGRES))[0]
        self.assertIn(driver.POSTGRES, image["RepoDigests"])
        self.assertEqual(image["Config"]["Volumes"], {"/var/lib/postgresql": {}})
        nonce = "lagrange-wholeflow-" + uuid.uuid4().hex
        intents, observed = [], []
        with tempfile.TemporaryFile() as output:
            recorder = driver.lifecycle.Recorder(nonce, output.fileno(), 120)
            try:
                recorder.start()
                name = nonce + "-initial"
                intents.append(("network", name))
                network = command("network", "create", "--internal", "--label", driver.LABEL + "=" + nonce, name).decode().strip()
                observed.append({"kind": "network", "id": network, "name": name})
                recorder.wait_for("network", network, "create")
                for index, role in enumerate(("worker", "producer", "admitter"), 1):
                    name = f"{nonce}-{role}-{index}"
                    intents.append(("container", name))
                    identity = command("create", "--name", name, "--label", driver.LABEL + "=" + nonce,
                                       "--label", driver.lifecycle.SERVICE + "=" + role, "--network", "none",
                                       "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges:true",
                                       "--pids-limit", "32", "--memory", "64m", "--cpus", "1", "--log-driver", "none",
                                       "--tmpfs", "/var/lib/postgresql:ro,nosuid,nodev,noexec,size=1m",
                                       "--pull", "never", "--entrypoint", "/bin/true", image["Id"]).decode().strip()
                    value = json.loads(command("container", "inspect", identity))[0]
                    self.assertFalse(any(m["Type"] == "volume" for m in value["Mounts"]))
                    self.assertEqual(value["Image"], image["Id"])
                    observed.append({"kind": "container", "id": identity, "name": name, "role": role, "image": image["Id"]})
                    recorder.wait_for("container", identity, "create")
                    command("start", "--attach", identity)
                    state = json.loads(command("container", "inspect", identity))[0]["State"]
                    self.assertFalse(state["Running"])
                    self.assertFalse(state["OOMKilled"])
                    self.assertEqual(state["ExitCode"], 0)
                    command("container", "rm", identity)
                command("network", "rm", network)
                result = recorder.finish(observed)
                self.assertEqual(result["status"], "COMPLETE", result["code"])
                self.assertEqual(result["events"], 14)
                self.assertTrue(result["processes_drained"] and result["readers_drained"])
                output.seek(0)
                saved = output.read()
                self.assertEqual(result["sha256"], driver.hashlib.sha256(saved).hexdigest())
                self.assertEqual([json.loads(row)["exitCode"] for row in saved.splitlines()
                                  if json.loads(row)["action"] == "die"], [0, 0, 0])
            finally:
                try:
                    # Resolve lost create replies by exact labelled intent; never prune.
                    for kind in ("container", "network"):
                        args = [kind, "ls", "--quiet", "--filter", "label=" + driver.LABEL + "=" + nonce]
                        if kind == "container":
                            args.append("--all")
                        for identity in command(*args).decode().split():
                            value = json.loads(command(kind, "inspect", identity))[0]
                            self.assertIn((kind, value["Name"].lstrip("/")), intents)
                            labels = value["Config"]["Labels"] if kind == "container" else value["Labels"]
                            self.assertEqual(labels[driver.LABEL], nonce)
                            if kind == "container":
                                self.assertFalse(any(m["Type"] == "volume" for m in value["Mounts"]))
                                command(kind, "rm", "--force", value["Id"])
                            else:
                                command(kind, "rm", value["Id"])
                        self.assertFalse(command(*args).strip())
                finally:
                    recorder.close()


if __name__ == "__main__":
    unittest.main(verbosity=2)
