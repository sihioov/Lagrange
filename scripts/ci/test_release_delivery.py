from contextlib import redirect_stdout
import io
import json
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

# unittest imports this file as a package module; pytest also collects it directly.
if __package__:
    from . import release_delivery as delivery
    from .test_ci_contract import SHA_ACTION, action_uses, load_workflow
else:
    import release_delivery as delivery
    from test_ci_contract import SHA_ACTION, action_uses, load_workflow


class ReleaseDeliveryTests(unittest.TestCase):
    commit = "a" * 40

    def setUp(self):
        self.run_info = {
            "head_repository": {"full_name": delivery.SOURCE}, "head_sha": self.commit,
            "head_branch": "main", "event": "push", "path": ".github/workflows/ci.yml",
            "status": "completed", "conclusion": "success", "run_attempt": 2,
        }
        self.jobs = {"total_count": 2, "jobs": [
            {"name": name, "status": "completed", "conclusion": "success", "head_sha": self.commit}
            for name in ("paper-image-integration", "required")
        ]}

    def test_requires_exact_ci_and_successful_jobs_in_current_attempt(self):
        with patch.object(delivery, "api", side_effect=[self.run_info, self.jobs]) as api:
            self.assertEqual(delivery.check_ci(self.commit, "123"), 2)
            self.assertIn("/attempts/2/jobs", api.call_args.args[0])
        # The exact same run retained the Paper pass when only another job was retried.
        with patch.object(delivery, "api", side_effect=[self.run_info,
                {"total_count": 1, "jobs": self.jobs["jobs"][1:]}, self.jobs]):
            self.assertEqual(delivery.check_ci(self.commit, "123"), 2)
        for key, value in (("head_sha", "b" * 40), ("head_branch", "feature"),
                           ("event", "pull_request"), ("path", "other.yml"),
                           ("head_repository", {"full_name": "other/repo"})):
            with self.subTest(key=key), patch.object(delivery, "api", return_value=self.run_info | {key: value}):
                with self.assertRaises(ValueError):
                    delivery.check_ci(self.commit, "123")
        for conclusion in ("skipped", "failure", "cancelled"):
            self.jobs["jobs"][0]["conclusion"] = conclusion
            with patch.object(delivery, "api", side_effect=[self.run_info, self.jobs]):
                with self.assertRaises(ValueError):
                    delivery.check_ci(self.commit, "123")

    def test_delivery_workflow_is_main_dispatch_only(self):
        _, workflow = load_workflow("release-delivery.yml")
        self.assertEqual(set(workflow["on"]), {"workflow_dispatch"})
        self.assertEqual(workflow["permissions"], {})
        job = workflow["jobs"]["delivery"]
        self.assertEqual(job["if"], "github.repository == 'sihioov/Lagrange' && github.ref == 'refs/heads/main'")
        self.assertEqual(job["environment"], "release-delivery")
        self.assertEqual(job["runs-on"], "ubuntu-24.04")
        actions = action_uses(workflow)
        self.assertEqual([use.split("@")[0] for use in actions],
                         ["actions/checkout", "actions/attest-build-provenance"])
        for use in actions:
            self.assertRegex(use, SHA_ACTION)
        self.assertEqual(job["steps"][0]["with"]["persist-credentials"], "false")

    @unittest.skipUnless(shutil.which("jq"), "the workflow's Docker store check requires jq")
    def test_docker_store_check_requires_the_containerd_driver_status_row(self):
        _, workflow = load_workflow("release-delivery.yml")
        prefix = "docker info --format '{{json .DriverStatus}}' | "
        (probe,) = [line.removeprefix(prefix) for step in workflow["jobs"]["delivery"]["steps"]
                    for line in step.get("run", "").splitlines() if line.startswith(prefix)]
        command = shlex.split(probe)
        self.assertEqual(command[:2], ["jq", "-e"])
        row = ["driver-type", "io.containerd.snapshotter.v1"]
        for status, accepted in (([row], True), ([["Backing Filesystem", "extfs"], row], True),
                                 ([], False), (None, False), (row, False),
                                 ([["other", row[1]], [row[0], "overlay2"]], False)):
            with self.subTest(status=status):
                result = subprocess.run(command, input=json.dumps(status), text=True, capture_output=True)
                self.assertEqual(result.returncode == 0, accepted, result.stderr)

    def test_public_or_mutable_store_rejected_before_upload(self):
        for private, enabled in ((False, True), (True, False)):
            repository = {"full_name": "test/store", "private": private,
                          "visibility": "private" if private else "public"}
            with patch.object(delivery, "api", side_effect=[repository, {"enabled": enabled}]):
                with self.assertRaises(ValueError):
                    delivery.private_store("test/store", writing=True)

    def test_publish_requires_matching_remote_digests_before_finalizing(self):
        names = {"release-bundle.json", "release.manifest", "d8-artifact.json",
                 "d8-COMPLETE", "images.tar.zst.part000000", *delivery.RESERVED}
        hashes = {name: "sha256:" + "a" * 64 for name in names}
        tag = delivery.release_tag(self.commit, 8, 1)
        assets = [{"name": name, "state": "uploaded", "digest": digest} for name, digest in hashes.items()]
        draft = {"tag_name": tag, "draft": True, "assets": assets}
        published = dict(draft, draft=False, immutable=True)
        with patch.object(delivery, "private_store"), patch.object(delivery, "inventory", return_value=hashes), \
                patch.object(delivery, "verify_record"), patch.object(delivery.subprocess, "run"):
            with patch.object(delivery, "api", side_effect=[{"id": 42}, draft, published]) as api:
                with redirect_stdout(io.StringIO()):
                    delivery.publish("test/store", Path("files"), self.commit, tag)
                self.assertIn("draft=false", api.call_args.args)
            assets[0]["digest"] = "sha256:" + "b" * 64
            with patch.object(delivery, "api", side_effect=[{"id": 42}, draft]) as api:
                with self.assertRaisesRegex(ValueError, "exact verified assets"):
                    delivery.publish("test/store", Path("files"), self.commit, tag)
                self.assertEqual(api.call_count, 2)

    def test_fetch_verifies_release_and_source_before_exposing_payload(self):
        with tempfile.TemporaryDirectory() as tmp:
            store = Path(tmp) / "assets"
            store.mkdir()
            chunk = "images.tar.zst.part000000"
            for name in ("release-bundle.json", "release.manifest", "d8-artifact.json", "d8-COMPLETE", chunk):
                (store / name).write_bytes(name.encode())
            tag = delivery.release_tag(self.commit, 8, 1)
            with patch.object(delivery, "check_ci", return_value=2):
                delivery.write_record(store, self.commit, 4, 8, 1)
            self.assertEqual(json.loads((store / delivery.RECORD).read_text())["ci_attempt"], 2)
            (store / delivery.SIGNATURE).write_bytes(b"signature fixture")
            release = {"immutable": True, "draft": False, "tag_name": tag, "assets": [
                {"name": key, "digest": value, "state": "uploaded"}
                for key, value in delivery.inventory(store).items()
            ]}
            output = Path(tmp) / "received"

            def gh(command, **kwargs):
                if command[1:3] == ["release", "download"]:
                    for file in store.iterdir():
                        (Path(command[-1]) / file.name).write_bytes(file.read_bytes())
                else:
                    self.assertEqual(command[1:3], ["attestation", "verify"])
                    self.assertIn("--deny-self-hosted-runners", command)
                    self.assertEqual(command[command.index("--source-digest") + 1], self.commit)
                    return delivery.SOURCE_REPOSITORY_ID + "\n"

            with patch.object(delivery, "private_store"), patch.object(delivery, "api", return_value=release), \
                    patch.object(delivery.subprocess, "run", side_effect=gh), \
                    patch.object(delivery.subprocess, "check_output", side_effect=gh):
                delivery.fetch("test/store", output, self.commit, tag)
                self.assertEqual(set(delivery.inventory(output / "bundle")), set(delivery.inventory(store)) - delivery.RESERVED)
                (store / chunk).write_bytes(b"corrupt")
                rejected = Path(tmp) / "corrupt"
                with self.assertRaisesRegex(ValueError, "digest mismatch"):
                    delivery.fetch("test/store", rejected, self.commit, tag)
                self.assertFalse(rejected.exists())
                # Even if the storage service reports the altered bytes, the signed inventory binds them.
                altered = next(asset for asset in release["assets"] if asset["name"] == chunk)
                altered["digest"] = delivery.inventory(store)[chunk]
                with self.assertRaisesRegex(ValueError, "authenticated build inventory"):
                    delivery.fetch("test/store", rejected, self.commit, tag)
                self.assertFalse(rejected.exists())


if __name__ == "__main__":
    unittest.main()
