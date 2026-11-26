#!/usr/bin/env python3
"""Focused contract tests for the GitHub-hosted product image adapter."""

import importlib.util
import pathlib
import re
import stat
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
POLICY_PATH = ROOT / "scripts/ops/lib/hosted-release-build-policy.py"
BUILDER_PATH = ROOT / "scripts/ops/build-hosted-release.sh"
LAYOUT_PATH = ROOT / "scripts/ops/lib/release-build-layout.sh"


def load_policy():
    spec = importlib.util.spec_from_file_location("hosted_release_build_policy", POLICY_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


POLICY = load_policy()
COMMIT = "a" * 40
RUNNER_UID = 1001


def identity_fields():
    return {
        "GITHUB_ACTIONS": "true",
        "GITHUB_REPOSITORY": "sihioov/Lagrange",
        "GITHUB_REF": "refs/heads/main",
        "GITHUB_EVENT_NAME": "workflow_dispatch",
        "GITHUB_WORKFLOW_REF": (
            "sihioov/Lagrange/.github/workflows/release-delivery.yml@refs/heads/main"
        ),
        "GITHUB_JOB": "delivery",
        "GITHUB_SHA": COMMIT,
        "GITHUB_WORKSPACE": "/home/runner/work/Lagrange/Lagrange",
        "GITHUB_RUN_ID": "1234567890",
        "GITHUB_RUN_ATTEMPT": "1",
        "RUNNER_OS": "Linux",
        "RUNNER_ARCH": "X64",
        "RUNNER_ENVIRONMENT": "github-hosted",
        "SUDO_USER": "runner",
        "SUDO_UID": str(RUNNER_UID),
    }


def validate_identity(fields=None, **overrides):
    arguments = {
        "fields": identity_fields() if fields is None else fields,
        "os_release": {"ID": "ubuntu", "VERSION_ID": "24.04"},
        "machine": "x86_64",
        "commit": COMMIT,
        "source_root": "/home/runner/work/Lagrange/Lagrange",
        "source_owner_uid": RUNNER_UID,
        "runner_uid": RUNNER_UID,
        "remote_url": "https://github.com/sihioov/Lagrange",
    }
    arguments.update(overrides)
    return POLICY.validate_runner_identity(**arguments)


class HostedRunnerIdentityTests(unittest.TestCase):
    def test_exact_release_delivery_identity_is_accepted(self):
        result = validate_identity()
        self.assertEqual(result["source_commit"], COMMIT)
        self.assertEqual(result["runner_image"], "ubuntu-24.04")
        self.assertEqual(result["runner_architecture"], "linux/amd64")

    def test_identity_or_checkout_changes_fail_closed(self):
        variants = [
            ("GITHUB_EVENT_NAME", "push"),
            ("GITHUB_WORKFLOW_REF", "sihioov/Lagrange/.github/workflows/ci.yml@refs/heads/main"),
            ("GITHUB_JOB", "build"),
            ("RUNNER_ENVIRONMENT", "self-hosted"),
            ("RUNNER_ARCH", "ARM64"),
            ("SUDO_UID", "1002"),
            ("GITHUB_RUN_ID", "0"),
        ]
        for key, value in variants:
            with self.subTest(key=key, value=value):
                fields = identity_fields()
                fields[key] = value
                with self.assertRaises(ValueError):
                    validate_identity(fields=fields)

        for overrides in (
            {"os_release": {"ID": "ubuntu", "VERSION_ID": "22.04"}},
            {"machine": "aarch64"},
            {"source_owner_uid": RUNNER_UID + 1},
            {"remote_url": "https://github.com/other/repository"},
            {"source_root": "/tmp/checkout"},
            {"commit": "b" * 40},
        ):
            with self.subTest(overrides=overrides):
                with self.assertRaises(ValueError):
                    validate_identity(**overrides)

    def test_identity_field_set_is_closed(self):
        fields = identity_fields()
        fields["AWS_ACCESS_KEY_ID"] = "must-not-be-forwarded"
        with self.assertRaisesRegex(ValueError, "field-set"):
            validate_identity(fields=fields)


class HostedIsolationTests(unittest.TestCase):
    def test_os_release_reads_only_supported_assignment_fields(self):
        self.assertEqual(
            POLICY.parse_os_release(
                'NAME="Ubuntu"\nID=ubuntu\nID_LIKE=debian\nVERSION_ID="24.04"\n'
            ),
            {"ID": "ubuntu", "VERSION_ID": "24.04"},
        )
        for raw in (
            "ID=ubuntu\nVERSION_ID=24.04\nID=ubuntu\n",
            "ID=ubuntu linux\nVERSION_ID=24.04\n",
            "ID=ubuntu\n",
        ):
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                POLICY.parse_os_release(raw)

    def test_inventory_parsers_keep_names_and_reject_malformed_rows(self):
        self.assertEqual(
            POLICY.parse_systemd_inventory(
                b"docker.service enabled enabled\nlagrange-station-api.service disabled disabled\n"
            ),
            ["docker.service", "lagrange-station-api.service"],
        )
        self.assertEqual(
            POLICY.parse_docker_container_inventory(b"buildx_buildkit_default\npostgres\n"),
            ["buildx_buildkit_default", "postgres"],
        )
        with self.assertRaises(ValueError):
            POLICY.parse_systemd_inventory(b"No files found.\n")
        with self.assertRaises(ValueError):
            POLICY.parse_docker_container_inventory(b"unexpected container name\n")

    def test_production_install_cohorts_are_rejected(self):
        POLICY.reject_production_cohorts(
            systemd_units=["docker.service"],
            containers=["postgres", "buildx_buildkit_default"],
            production_root_present=False,
        )
        cases = (
            {"systemd_units": [], "containers": [], "production_root_present": True},
            {
                "systemd_units": ["lagrange-api.service"],
                "containers": [],
                "production_root_present": False,
            },
            {
                "systemd_units": [],
                "containers": ["lagrange-station-api-server-1"],
                "production_root_present": False,
            },
        )
        for case in cases:
            with self.subTest(case=case), self.assertRaises(ValueError):
                POLICY.reject_production_cohorts(**case)


class HostedBuilderContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = BUILDER_PATH.read_text(encoding="utf-8")
        cls.layout_text = LAYOUT_PATH.read_text(encoding="utf-8")

    def test_entrypoint_is_executable_and_accepts_only_commit_and_output(self):
        mode = stat.S_IMODE(BUILDER_PATH.stat().st_mode)
        self.assertTrue(mode & stat.S_IXUSR)
        self.assertIn("[ \"$#\" -eq 2 ]", self.text)
        self.assertIn("Usage: sudo scripts/ops/build-hosted-release.sh COMMIT OUTPUT_DIR", self.text)

    def test_official_batch_order_and_output_contract_are_fixed(self):
        batches = re.findall(r"^run_batch (B[1-5]) (.+)$", self.text, flags=re.MULTILINE)
        self.assertEqual(
            batches,
            [
                ("B1", "db-role-bootstrap db-migrate api-server"),
                ("B2", "web research-worker recommendation-runner"),
                ("B3", "candidate-runner owner-beta-runner owner-equity-v2-runner"),
                ("B4", "nt-backtest-worker-1 nt-backtest-worker-2"),
                ("B5", "paper-scheduler owner-beta-simulation-runner"),
            ],
        )
        self.assertIn('release_build_layout_init "$root" "$commit" "$state_root" product hosted', self.text)
        self.assertIn('release_build_layout_verify_image "$service" "$commit" "$image_id" "$state_root"', self.text)
        self.assertIn('discard_verified_archive "$image_id"', self.text)
        self.assertIn('validate(artifact,complete,os.environ["RELEASE_COMMIT"])', self.text)
        self.assertIn("$'d8\\nrelease.manifest'", self.text)
        self.assertIn('release_image_manifest_write "$temporary" "$commit"', self.text)
        self.assertIn('HOSTED_RELEASE_DISK boundary=', self.text)
        self.assertIn('report_disk_capacity run-start', self.text)
        self.assertIn('report_disk_capacity "batch:$batch"', self.text)
        self.assertIn('report_disk_capacity final', self.text)

    def test_only_synthetic_secret_free_compose_environment_is_passed(self):
        for required in (
            "env -i PATH=",
            "CARGO_BUILD_JOBS=2 COMPOSE_PARALLEL_LIMIT=1",
            "RESEARCH_APP_ENV=prebuild-disabled",
            "RESEARCH_FETCH_MODE=synthetic",
            "RESEARCH_ENTITLEMENT_REFERENCE=prebuild-disabled",
            "RESEARCH_ENTITLEMENT_SHA256=0000000000000000000000000000000000000000000000000000000000000000",
            "COMPOSE_PROFILES=",
            "DOCKER_CONFIG=\"$docker_config\"",
            "mkdir -m 0700 -- \"$docker_config\"",
        ):
            with self.subTest(required=required):
                self.assertIn(required, self.text)
        self.assertNotRegex(self.text, r"KIS_APP_(?:KEY|SECRET)|AUTH0_CLIENT_SECRET|AWS_SECRET_ACCESS_KEY")

    def test_hosted_receipt_binding_does_not_change_product_run_records(self):
        self.assertIn("rbl_execution_mode=${5:-production}", self.layout_text)
        self.assertIn('if os.environ["RBL_EXECUTION_MODE"] == "hosted":', self.layout_text)
        self.assertIn('value["execution_mode"] = "hosted"', self.layout_text)
        self.assertIn('value["hosted_policy_sha256"] = os.environ["RBL_HOSTED_POLICY_SHA256"]', self.layout_text)
        self.assertIn('"$state_root" product hosted', self.text)
        self.assertNotIn('"hosted_policy_sha256":rbl_hosted_policy_hash', self.layout_text)
        self.assertNotIn('"hosted-release"', self.layout_text)


if __name__ == "__main__":
    unittest.main()
