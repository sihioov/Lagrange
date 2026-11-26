"""Focused checks for the offline VM inputs and Paper-OFF runtime evidence."""
from __future__ import annotations

import argparse
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import stat
from types import SimpleNamespace
import unittest
from unittest import mock

if __package__:
    from . import test_owner_beta_simulation_release as simulation_fixture
else:
    import test_owner_beta_simulation_release as simulation_fixture


MODULE = Path(__file__).with_name("verify-release-delivery-vm.py")
SPEC = importlib.util.spec_from_file_location("release_delivery_vm", MODULE)
vm = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(vm)
COMMIT = simulation_fixture.COMMIT
UPSTREAM = {
    "postgres": "sha256:" + "1" * 64,
    "research-schema-check": "sha256:" + "1" * 64,
    "research-raw-init": "sha256:" + "2" * 64,
    "reverse-proxy": "sha256:" + "3" * 64,
}
BUNDLE = vm.release_helper("release-delivery-bundle")
OVERRIDE = BUNDLE.compose_override([
    {"name": service, "image_id": UPSTREAM[target]}
    for service, target in (("postgres", "postgres"), ("reverse-proxy", "reverse-proxy"),
                            ("alpine", "research-raw-init"))
])


class ReleaseDeliveryVMTests(unittest.TestCase):
    def test_upstream_override_is_a_closed_id_mapping(self):
        self.assertEqual(vm.upstream_override(OVERRIDE), UPSTREAM)
        for invalid in (
            OVERRIDE + b"    privileged: true\n",
            OVERRIDE + b"  postgres:\n    image: sha256:" + b"1" * 64 + b"\n",
            OVERRIDE.replace(b"image: sha256:", b"image: nginx@sha256:"),
            OVERRIDE.replace(b"  postgres:", b"  prometheus:"),
            OVERRIDE.replace(b"    image: sha256:" + b"1" * 64, b"    image: sha256:" + b"4" * 64, 1),
        ):
            with self.subTest(invalid=invalid), self.assertRaisesRegex(vm.RehearsalError, "UPSTREAM_OVERRIDE"):
                vm.upstream_override(invalid)

    def test_validate_inputs_binds_real_v3_manifest_and_d8_receipt(self):
        fixture = simulation_fixture.binding_fixture()
        reader = mock.Mock()
        reader.root_directory.side_effect = lambda _path: contextlib.nullcontext(17)
        payloads = {
            "/protected/release.manifest": fixture["manifest"],
            "/protected/d8/artifact.json": fixture["receipt_data"],
            "/protected/d8/COMPLETE": fixture["complete"],
            "/protected/compose-offline.override.yml": OVERRIDE,
        }
        reader.read_root_file.side_effect = lambda path, **_kwargs: payloads[path]
        with (mock.patch.object(simulation_fixture.release, "_daily", return_value=reader),
              mock.patch.object(vm, "release_helper", return_value=simulation_fixture.release),
              mock.patch.object(vm, "inspect_image") as images):
            manifest, binding, override, upstream, receipt_hash = vm.validate_inputs("/protected", COMMIT)
        self.assertEqual(manifest, fixture["manifest"])
        self.assertEqual(binding["image_ids"], fixture["images"])
        self.assertEqual(receipt_hash, fixture["config"]["receipt_sha256"])
        self.assertEqual((override, upstream), (OVERRIDE, UPSTREAM))
        self.assertEqual(images.call_count, 16)
        images.assert_any_call(f"lagrange-station-owner-beta-simulation-runner:{COMMIT}",
                               fixture["images"]["owner-beta-simulation-runner"], COMMIT)
        reader.directory_info.assert_called_once_with(17, uid=0, gid=0, mode=0o700)

        payloads["/protected/d8/COMPLETE"] = b"0" * 64 + b"\n"
        with (mock.patch.object(simulation_fixture.release, "_daily", return_value=reader),
              mock.patch.object(vm, "release_helper", return_value=simulation_fixture.release),
              mock.patch.object(vm, "inspect_image")):
            with self.assertRaises(simulation_fixture.release.SimulationInstallError):
                vm.validate_inputs("/protected", COMMIT)

    def test_inspection_rejects_wrong_platform_identity_and_revision(self):
        identity = "sha256:" + "1" * 64
        with mock.patch.object(vm, "docker", return_value=f"{identity}|linux/amd64|{COMMIT}"):
            vm.inspect_image("ref", identity, COMMIT)
        for row, problem in ((f"{identity}|linux/arm64|{COMMIT}", "IMAGE_ID_OR_PLATFORM"),
                             (f"{identity}|linux/amd64|{'0' * 40}", "IMAGE_REVISION"),
                             (f"sha256:{'9' * 64}|linux/amd64|{COMMIT}", "IMAGE_ID_OR_PLATFORM")):
            with (self.subTest(row=row), mock.patch.object(vm, "docker", return_value=row),
                  self.assertRaisesRegex(vm.RehearsalError, problem)):
                vm.inspect_image("ref", identity, COMMIT)

    def test_guest_admission_refuses_non_root_before_any_process(self):
        with (mock.patch.object(vm.os, "geteuid", return_value=1000),
              mock.patch.object(vm, "call") as call,
              self.assertRaisesRegex(vm.RehearsalError, "ROOT_LINUX_AMD64_VM_REQUIRED")):
            vm.guest_preflight()
        call.assert_not_called()

    def delivery_args(self, *options):
        return vm.delivery_inputs(argparse.ArgumentParser(), [
            "--commit", COMMIT, "--import-dir", "/protected/output", *options,
        ])

    def test_connected_options_require_exact_release_identity_and_trust_root(self):
        self.assertIsNone(self.delivery_args().received)
        with self.assertRaisesRegex(vm.RehearsalError, "EXACT_OFFLINE_INPUTS_REQUIRED"):
            self.delivery_args("--received", "/protected/received")
        with self.assertRaisesRegex(vm.RehearsalError, "EXACT_RELEASE_RUN_REQUIRED"):
            self.delivery_args("--received", "/protected/received", "--trusted-root", "/etc/gh-root",
                               "--build-run", "01234", "--build-attempt", "2")
        with self.assertRaisesRegex(vm.RehearsalError, "CANONICAL_DELIVERY_PATH_REQUIRED"):
            self.delivery_args("--received", "//protected/received", "--trusted-root", "/etc/gh-root",
                               "--build-run", "1234", "--build-attempt", "2")

    def test_protected_inputs_refuse_untrusted_file_metadata(self):
        info = {"st_mode": stat.S_IFREG | 0o644, "st_uid": 0, "st_gid": 0, "st_nlink": 1, "st_size": 42}
        path = mock.Mock()
        path.lstat.return_value = SimpleNamespace(**info)
        vm.protected_file(path)
        for update in ({"st_mode": stat.S_IFLNK | 0o777}, {"st_mode": stat.S_IFREG | 0o666},
                       {"st_uid": 1000}, {"st_nlink": 2}, {"st_size": 0}):
            path.lstat.return_value = SimpleNamespace(**(info | update))
            with self.assertRaisesRegex(vm.RehearsalError, "UNTRUSTED_DELIVERY_INPUT"):
                vm.protected_file(path)
        with self.assertRaisesRegex(vm.RehearsalError, "DELIVERY_PATH_OVERLAP_OR_EXISTING_OUTPUT"):
            vm.check_offline_paths(Path("/protected/received"), Path("/protected/received/trust"),
                                   Path("/protected/output"))

    def test_dirty_checkout_blocks_measurement_and_import(self):
        args = self.delivery_args("--received", "/protected/received", "--trusted-root", "/etc/gh-root",
                                  "--build-run", "1234", "--build-attempt", "2")
        with (mock.patch.object(vm, "git", side_effect=[str(vm.ROOT).encode(), COMMIT.encode(), b"M source"]),
              mock.patch.object(vm, "GuestMeasurements") as measurements,
              mock.patch.object(vm, "verify_and_import") as import_release,
              self.assertRaisesRegex(vm.RehearsalError, "EXACT_CLEAN_CHECKOUT")):
            vm.accept_delivery(args, Path("/var/lib/fixture"), Path("/var/lib/docker"))
        measurements.assert_not_called()
        import_release.assert_not_called()

    def test_offline_workflow_checks_checkout_and_receipt_before_import_and_startup(self):
        args = self.delivery_args("--received", "/protected/received", "--trusted-root", "/etc/gh-root",
                                  "--build-run", "1234", "--build-attempt", "2")
        workspace = Path("/var/lib/fixture")
        commands = {}

        def invoke(argv, *, code, **options):
            order.append(code)
            commands[code] = (argv, options)
            if code == failed:
                raise vm.RehearsalError(code)
            if code == "GUEST_GITHUB_ATTESTATION_REQUIRED":
                return b"--custom-trusted-root --source-digest"
            return b""

        common = ["checkout", "start measurement", "paths", "GUEST_GITHUB_ATTESTATION_REQUIRED",
                  "DELIVERY_ATTESTATION", "DELIVERY_IMPORT", "DELIVERY_REIMPORT", "validate", "rehearse"]
        for failed in (None, "DELIVERY_ATTESTATION", "DELIVERY_IMPORT", "DELIVERY_REIMPORT"):
            order = []
            with (self.subTest(failed=failed), mock.patch.object(vm, "call", side_effect=invoke),
                  mock.patch.object(vm, "docker", return_value="") as docker,
                  mock.patch.object(vm, "check_checkout", side_effect=lambda _commit: order.append("checkout")),
                  mock.patch.object(vm, "check_offline_paths", side_effect=lambda *_args: order.append("paths")),
                  mock.patch.object(vm.shutil, "which", return_value=vm.GH),
                  mock.patch.object(vm, "GuestMeasurements") as measure,
                  mock.patch.object(vm, "validate_inputs", side_effect=lambda *_args: order.append("validate")),
                  mock.patch.object(vm, "rehearse", side_effect=lambda *_args: order.append("rehearse")),
                  contextlib.redirect_stdout(io.StringIO())):
                measure.return_value.thread.start.side_effect = lambda: order.append("start measurement")
                measure.return_value.finish.side_effect = lambda: order.append("finish measurement")
                if failed:
                    with self.assertRaisesRegex(vm.RehearsalError, failed):
                        vm.accept_delivery(args, workspace, Path("/var/lib/docker"))
                    self.assertEqual(order, common[:common.index(failed) + 1] + ["finish measurement"])
                else:
                    summary = vm.accept_delivery(args, workspace, Path("/var/lib/docker"))
                    self.assertIn("delivery_verification=offline imports=2", summary)
                    self.assertEqual(order, common + ["finish measurement"])
                    self.assertIn(Path("/protected/received"), measure.call_args.args)
            docker.assert_called_once_with("image", "ls", "--all", "--quiet")
        verifier, options = commands["DELIVERY_ATTESTATION"]
        self.assertEqual(verifier[3:], ["verify-offline", Path(args.received), COMMIT, "1234", "2",
                                        Path(args.trusted_root)])
        self.assertEqual(options["env"]["GH_CONFIG_DIR"], str(workspace / "gh-config"))
        self.assertNotIn("GH_TOKEN", options["env"])
        first, _options = commands["DELIVERY_IMPORT"]
        self.assertEqual(first[2:], ["import", COMMIT, Path(args.received) / "bundle", Path(args.import_dir)])
        self.assertEqual(first, commands["DELIVERY_REIMPORT"][0])

    def test_offline_acceptance_requires_an_empty_initial_image_inventory(self):
        args = self.delivery_args("--received", "/protected/received", "--trusted-root", "/etc/gh-root",
                                  "--build-run", "1234", "--build-attempt", "2")
        with (mock.patch.object(vm, "check_checkout"),
              mock.patch.object(vm, "docker", return_value="existing-image"),
              mock.patch.object(vm, "verify_and_import") as import_release,
              self.assertRaisesRegex(vm.RehearsalError, "GUEST_IMAGE_STORE_NOT_EMPTY")):
            vm.accept_delivery(args, Path("/var/lib/fixture"), Path("/var/lib/docker"))
        import_release.assert_not_called()

    def test_runtime_evidence_requires_exact_healthy_cohort_and_paper_off(self):
        identity = "sha256:" + "a" * 64
        images = {service: identity for service in vm.PERSISTENT}
        project = "fixture"
        rows = [{
            "Config": {"Labels": {"com.docker.compose.service": service, "com.docker.compose.project": project},
                       "Env": [f"{key}={value}" for key, value in vm.OFF.items()]},
            "State": {"Running": True, "Health": {"Status": "healthy"}, "OOMKilled": False},
            "Image": identity,
            "RestartCount": 0,
        } for service in vm.PERSISTENT]
        ids = "\n".join(str(index) for index in range(len(rows)))
        with (mock.patch.object(vm, "docker", side_effect=[ids, json.dumps(rows)]),
              contextlib.redirect_stdout(io.StringIO()) as evidence):
            vm.verify_running(project, images)
        self.assertEqual(evidence.getvalue().count("health=healthy restarts=0"), 4)

        api = rows[vm.PERSISTENT.index("api-server")]
        api["Config"]["Env"].remove("OWNER_BETA_PAPER_MODE=disabled")
        api["Config"]["Env"].append("OWNER_BETA_PAPER_MODE=owner_only")
        with (mock.patch.object(vm, "docker", side_effect=[ids, json.dumps(rows)]),
              contextlib.redirect_stdout(io.StringIO()),
              self.assertRaisesRegex(vm.RehearsalError, "API_OFF_STATE")):
            vm.verify_running(project, images)

    def test_fixture_env_declares_off_modes_and_only_synthetic_bindings(self):
        _, text = vm.fixture_environment(Path("/var/lib/isolated-fixture"), COMMIT, "fixture")
        env = dict(line.split("=", 1) for line in text.splitlines())
        self.assertEqual({key: env[key] for key in vm.OFF}, vm.OFF)
        self.assertEqual(env["LAGRANGE_CODE_COMMIT"], COMMIT)
        self.assertEqual(env["LAGRANGE_PGDATA_VOLUME"], "fixture-pgdata")
        self.assertTrue(env["AUTH0_DOMAIN"].endswith(".invalid"))
        self.assertTrue(env["LAGRANGE_DATA_DIR"].startswith("/var/lib/isolated-fixture/"))


if __name__ == "__main__":
    unittest.main()
